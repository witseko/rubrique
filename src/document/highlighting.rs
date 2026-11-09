//! Color Markdown, YAML, Helm template and code syntax (Python, JavaScript,
//! TypeScript, Rust, JSON, XML, HTML) in the editor and in preview code blocks with the current
//! theme, and lay plain text out with the same line height.

use eframe::egui::cache::{ComputerMut, FrameCache};
use eframe::egui::emath::OrderedFloat;
use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, Context, FontId, Stroke, TextFormat, TextStyle, Ui};

use std::path::Path;

use crate::format::Language;
use crate::highlight::{code, xml};
use crate::markdown::highlight::{Kind, Span};
use crate::theme::code_colors::{CodeColors, SyntaxColors, installed_code_colors};
use crate::theme::style::EDITOR_LINE_HEIGHT;

/// Which syntax the editor colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Syntax {
    Markdown,
    /// YAML, with Helm (Go template) actions.
    Yaml,
    Python,
    JavaScript,
    TypeScript,
    /// TypeScript with React's JSX tags.
    Tsx,
    Rust,
    Json,
    Xml,
    /// HTML, with Angular's template syntax.
    Html,
}

impl Syntax {
    /// The syntax of a file, from its name; `None` for plain text.
    pub fn of(path: &Path) -> Option<Syntax> {
        if crate::explorer::tree::is_markdown_path(path) {
            Some(Syntax::Markdown)
        } else if crate::helm::is_yaml_path(path) {
            Some(Syntax::Yaml)
        } else if crate::markdown::html::is_html_path(path) {
            Some(Syntax::Html)
        } else {
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            match extension.as_str() {
                "py" | "pyw" | "pyi" => Some(Syntax::Python),
                "js" | "mjs" | "cjs" | "jsx" => Some(Syntax::JavaScript),
                "ts" | "mts" | "cts" => Some(Syntax::TypeScript),
                "tsx" => Some(Syntax::Tsx),
                "rs" => Some(Syntax::Rust),
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
            "typescript" | "ts" | "mts" | "cts" => Some(Syntax::TypeScript),
            "tsx" => Some(Syntax::Tsx),
            "rust" | "rs" => Some(Syntax::Rust),
            "html" | "htm" | "angular" => Some(Syntax::Html),
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
            Syntax::TypeScript => code::highlight(text, &code::TYPESCRIPT),
            Syntax::Tsx => code::highlight(text, &code::TSX),
            Syntax::Rust => code::highlight(text, &code::RUST),
            Syntax::Json => code::highlight(text, &code::JSON),
            Syntax::Xml => xml::highlight(text),
            Syntax::Html => xml::highlight_html(text),
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
    /// Keywords, strings, comments and the other kinds the Code colors setting changes.
    syntax: SyntaxColors,
}

impl HighlightStyle {
    /// The style for `ui`'s theme and the installed code colors.
    pub fn from_ui(ui: &Ui) -> Self {
        Self::with_code_colors(ui, installed_code_colors(ui.ctx()))
    }

    /// The style for `ui`'s theme with `code_colors` (the settings cards
    /// preview each choice).
    pub fn with_code_colors(ui: &Ui, code_colors: CodeColors) -> Self {
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
            syntax: SyntaxColors::new(code_colors, visuals),
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
        let syntax = &self.syntax;
        let comment_italics = syntax.italic_comments;
        match kind {
            Kind::Plain => plain,
            Kind::Marker | Kind::Url => TextFormat { color: self.weak, ..plain },
            Kind::LinkText => TextFormat { color: self.accent, ..plain },
            Kind::Heading => TextFormat { color: syntax.heading, ..plain },
            Kind::Key => TextFormat { color: syntax.key, ..plain },
            Kind::Keyword => TextFormat { color: syntax.keyword, ..plain },
            Kind::Strong => TextFormat { color: self.strong, ..plain },
            Kind::Name => TextFormat { color: syntax.name, ..plain },
            Kind::String => TextFormat { color: syntax.string, ..plain },
            Kind::Number => TextFormat { color: syntax.number, ..plain },
            Kind::Emphasis => TextFormat { italics: true, ..plain },
            Kind::Strikethrough => TextFormat { color: self.weak, strikethrough: Stroke::new(1.0, self.weak), ..plain },
            Kind::Code => TextFormat { color: syntax.code, background: self.code_background, ..plain },
            Kind::Template => TextFormat { color: syntax.template, background: self.code_background, ..plain },
            Kind::Quote => TextFormat { color: syntax.quote, italics: comment_italics, ..plain },
            Kind::Comment => TextFormat { color: syntax.comment, italics: comment_italics, ..plain },
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
        assert_eq!(Syntax::of(Path::new("web/app.ts")), Some(Syntax::TypeScript));
        assert_eq!(Syntax::of(Path::new("web/View.tsx")), Some(Syntax::Tsx));
        assert_eq!(Syntax::of(Path::new("src/main.rs")), Some(Syntax::Rust));
        assert_eq!(Syntax::of(Path::new("app/list.component.html")), Some(Syntax::Html));
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
        assert_eq!(Syntax::from_code_info("typescript"), Some(Syntax::TypeScript));
        assert_eq!(Syntax::from_code_info("tsx"), Some(Syntax::Tsx));
        assert_eq!(Syntax::from_code_info("rust"), Some(Syntax::Rust));
        assert_eq!(Syntax::from_code_info("html"), Some(Syntax::Html));
        assert_eq!(Syntax::from_code_info("jsonc"), Some(Syntax::Json));
        assert_eq!(Syntax::from_code_info("svg"), Some(Syntax::Xml));
        assert_eq!(Syntax::from_code_info("sql"), None);
        assert_eq!(Syntax::from_code_info(""), None);
    }

    /// The color of the first section of `text` as Python, with the code
    /// colors installed in a dark egui context.
    fn first_python_color(code_colors: Option<CodeColors>, text: &str) -> Color32 {
        let ctx = Context::default();
        ctx.set_visuals(eframe::egui::Visuals::dark());
        if let Some(choice) = code_colors {
            crate::theme::code_colors::install_code_colors(&ctx, choice);
        }
        let mut color = Color32::PLACEHOLDER;
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            let job = layout_job(text, &HighlightStyle::from_ui(ui), Some(Syntax::Python));
            color = job.sections[0].format.color;
        });
        output.textures_delta.clear();
        color
    }

    #[test]
    fn keywords_follow_the_installed_code_colors() {
        let accent = eframe::egui::Visuals::dark().hyperlink_color;
        assert_eq!(first_python_color(None, "def f(): pass"), accent);
        assert_eq!(first_python_color(Some(CodeColors::Theme), "def f(): pass"), accent);
        assert_eq!(first_python_color(Some(CodeColors::VsCode), "def f(): pass"), Color32::from_rgb(0x56, 0x9c, 0xd6));
    }

    #[test]
    fn vs_code_comments_are_not_italic() {
        let ctx = Context::default();
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            let theme = HighlightStyle::with_code_colors(ui, CodeColors::Theme);
            let vs_code = HighlightStyle::with_code_colors(ui, CodeColors::VsCode);
            assert!(theme.format(Kind::Comment).italics);
            assert!(!vs_code.format(Kind::Comment).italics);
        });
        output.textures_delta.clear();
    }
}
