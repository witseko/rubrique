//! Color Markdown, YAML, Helm template and code syntax (Python, JavaScript,
//! JSON, XML) in the editor and in preview code blocks with the current
//! theme, and lay plain text out with the same line height.

use eframe::egui::cache::{ComputerMut, FrameCache};
use eframe::egui::emath::OrderedFloat;
use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, Context, FontId, Stroke, TextFormat, TextStyle, Ui};

use std::path::Path;

use crate::format::Language;
use crate::highlight::{code, xml};
use crate::markdown::highlight::{Kind, Span};
use crate::theme::style::EDITOR_LINE_HEIGHT;

/// Which syntax the editor colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Syntax {
    Markdown,
    /// YAML, with Helm (Go template) actions.
    Yaml,
    Python,
    JavaScript,
    Json,
    Xml,
}

impl Syntax {
    /// The syntax of a file, from its name; `None` for plain text.
    pub fn of(path: &Path) -> Option<Syntax> {
        if crate::explorer::tree::is_markdown_path(path) {
            Some(Syntax::Markdown)
        } else if crate::helm::is_yaml_path(path) {
            Some(Syntax::Yaml)
        } else {
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            match extension.as_str() {
                "py" | "pyw" | "pyi" => Some(Syntax::Python),
                "js" | "mjs" | "cjs" | "jsx" => Some(Syntax::JavaScript),
                _ => Syntax::of_format(Language::from_path(path)?),
            }
        }
    }

    /// The syntax of a fenced code block from its info string (` ```python `);
    /// `None` when it is not one the editor colors.
    pub fn from_code_info(info: &str) -> Option<Syntax> {
        let name = info.split_whitespace().next()?.to_ascii_lowercase();
        match name.as_str() {
            "python" | "py" | "python3" => Some(Syntax::Python),
            "javascript" | "js" | "jsx" | "mjs" | "cjs" | "node" => Some(Syntax::JavaScript),
            _ => Syntax::of_format(Language::from_code_info(info)?),
        }
    }

    /// JSON and XML share their file names and info strings with Format.
    fn of_format(language: Language) -> Option<Syntax> {
        match language {
            Language::Json => Some(Syntax::Json),
            Language::Xml => Some(Syntax::Xml),
            Language::Sql => None,
        }
    }

    fn spans(self, text: &str) -> Vec<Span> {
        match self {
            Syntax::Markdown => crate::markdown::highlight::highlight(text),
            Syntax::Yaml => crate::helm::highlight::highlight(text),
            Syntax::Python => code::highlight(text, &code::PYTHON),
            Syntax::JavaScript => code::highlight(text, &code::JAVASCRIPT),
            Syntax::Json => code::highlight(text, &code::JSON),
            Syntax::Xml => xml::highlight(text),
        }
    }
}

/// Theme colors and the editor font, read from the current style.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct HighlightStyle {
    font: FontId,
    line_height: OrderedFloat<f32>,
    text: Color32,
    strong: Color32,
    weak: Color32,
    accent: Color32,
    code_background: Color32,
    /// Code strings, in the theme's warning color.
    string: Color32,
    /// Code numbers, in the theme's error color.
    number: Color32,
}

impl HighlightStyle {
    /// The style for `ui`'s theme.
    pub fn from_ui(ui: &Ui) -> Self {
        let visuals = ui.visuals();
        let font = TextStyle::Monospace.resolve(ui.style());
        HighlightStyle {
            line_height: OrderedFloat(editor_line_height(&font)),
            font,
            text: visuals.text_color(),
            strong: visuals.strong_text_color(),
            weak: visuals.weak_text_color(),
            accent: visuals.hyperlink_color,
            code_background: visuals.code_bg_color,
            string: visuals.warn_fg_color,
            number: visuals.error_fg_color,
        }
    }

    /// The same colors with another monospace size and line height (preview
    /// code blocks).
    pub fn with_size(self, size: f32, line_height: f32) -> Self {
        HighlightStyle { font: FontId::monospace(size), line_height: OrderedFloat(line_height), ..self }
    }

    /// How to draw one kind of text. Every kind keeps the monospace font, so
    /// columns stay lined up, and the editor's line height.
    fn format(&self, kind: Kind) -> TextFormat {
        let plain = TextFormat {
            font_id: self.font.clone(),
            color: self.text,
            line_height: Some(self.line_height.0),
            ..TextFormat::default()
        };
        match kind {
            Kind::Plain => plain,
            Kind::Marker | Kind::Url => TextFormat { color: self.weak, ..plain },
            Kind::Heading | Kind::LinkText | Kind::Key | Kind::Keyword => TextFormat { color: self.accent, ..plain },
            Kind::Strong | Kind::Name => TextFormat { color: self.strong, ..plain },
            Kind::String => TextFormat { color: self.string, ..plain },
            Kind::Number => TextFormat { color: self.number, ..plain },
            Kind::Emphasis => TextFormat { italics: true, ..plain },
            Kind::Strikethrough => TextFormat { color: self.weak, strikethrough: Stroke::new(1.0, self.weak), ..plain },
            Kind::Code => TextFormat { background: self.code_background, ..plain },
            Kind::Template => TextFormat { color: self.strong, background: self.code_background, ..plain },
            Kind::Quote | Kind::Comment => TextFormat { color: self.weak, italics: true, ..plain },
        }
    }
}

/// The height of one line of `font` in the source editor and the block
/// editor: taller than the font's own rows, so lines of text breathe.
pub fn editor_line_height(font: &FontId) -> f32 {
    (font.size * EDITOR_LINE_HEIGHT).round()
}

/// The layout of `text`, colored for `syntax` (plain when `None`), reused
/// while the text and theme stay the same.
pub fn highlighted(ctx: &Context, text: &str, style: &HighlightStyle, syntax: Option<Syntax>) -> LayoutJob {
    ctx.memory_mut(|memory| memory.caches.cache::<HighlightCache>().get((text, style, syntax)).clone())
}

type HighlightCache = FrameCache<LayoutJob, Highlighter>;

/// Builds layouts for the cache.
#[derive(Default)]
struct Highlighter;

impl ComputerMut<(&str, &HighlightStyle, Option<Syntax>), LayoutJob> for Highlighter {
    fn compute(&mut self, (text, style, syntax): (&str, &HighlightStyle, Option<Syntax>)) -> LayoutJob {
        layout_job(text, style, syntax)
    }
}

/// One text section per highlighted span; plain text is one section.
fn layout_job(text: &str, style: &HighlightStyle, syntax: Option<Syntax>) -> LayoutJob {
    let mut job = LayoutJob::default();
    match syntax {
        Some(syntax) => {
            for span in syntax.spans(text) {
                job.append(&text[span.range], 0.0, style.format(span.kind));
            }
        }
        None => job.append(text, 0.0, style.format(Kind::Plain)),
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_from_file_names() {
        assert_eq!(Syntax::of(Path::new("app/main.py")), Some(Syntax::Python));
        assert_eq!(Syntax::of(Path::new("web/App.JSX")), Some(Syntax::JavaScript));
        assert_eq!(Syntax::of(Path::new("package.json")), Some(Syntax::Json));
        assert_eq!(Syntax::of(Path::new("pom.xml")), Some(Syntax::Xml));
        assert_eq!(Syntax::of(Path::new("notes.md")), Some(Syntax::Markdown));
        assert_eq!(Syntax::of(Path::new("query.sql")), None);
        assert_eq!(Syntax::of(Path::new("Makefile")), None);
    }

    #[test]
    fn syntax_from_code_info() {
        assert_eq!(Syntax::from_code_info("python"), Some(Syntax::Python));
        assert_eq!(Syntax::from_code_info("JS title=\"a\""), Some(Syntax::JavaScript));
        assert_eq!(Syntax::from_code_info("jsonc"), Some(Syntax::Json));
        assert_eq!(Syntax::from_code_info("svg"), Some(Syntax::Xml));
        assert_eq!(Syntax::from_code_info("sql"), None);
        assert_eq!(Syntax::from_code_info(""), None);
    }
}
