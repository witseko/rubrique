//! Color Markdown, YAML, Helm template and code syntax (Python, JavaScript,
//! TypeScript, Rust, JSON, XML, HTML, SQL) in the editor and in preview code blocks with the current
//! theme, and lay plain text out with the same line height, colored by the
//! user's text highlight rules.

use eframe::egui::cache::{ComputerMut, FrameCache};
use eframe::egui::emath::OrderedFloat;
use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, Context, FontId, Stroke, TextFormat, TextStyle, Ui};

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::format::Language;
use crate::highlight::text_rules::{CompiledRules, HighlightRule, RuleColor};
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
    Sql,
}

/// The text highlight rules color this file: plain text (no code colors),
/// or a Markdown file, whose preview colors its plain text.
pub fn takes_text_highlights(path: &Path) -> bool {
    crate::explorer::tree::is_markdown_path(path) || Syntax::of(path).is_none()
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
                _ => Language::from_path(path).map(Syntax::of_format),
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
            _ => Language::from_code_info(info).map(Syntax::of_format),
        }
    }

    /// JSON, XML and SQL share their file names and info strings with Format.
    fn of_format(language: Language) -> Syntax {
        match language {
            Language::Json => Syntax::Json,
            Language::Xml => Syntax::Xml,
            Language::Sql => Syntax::Sql,
        }
    }

    fn spans(self, text: &str) -> Vec<Span> {
        match self {
            // Fenced code blocks are colored as the language their info
            // string names, as in the preview.
            Syntax::Markdown => crate::markdown::highlight::highlight_with_code(text, |info, code| {
                Some(Syntax::from_code_info(info)?.spans(code))
            }),
            Syntax::Yaml => crate::helm::highlight::highlight(text),
            Syntax::Python => code::highlight(text, &code::PYTHON),
            Syntax::JavaScript => code::highlight(text, &code::JAVASCRIPT),
            Syntax::TypeScript => code::highlight(text, &code::TYPESCRIPT),
            Syntax::Tsx => code::highlight(text, &code::TSX),
            Syntax::Rust => code::highlight(text, &code::RUST),
            Syntax::Json => code::highlight(text, &code::JSON),
            Syntax::Xml => xml::highlight(text),
            Syntax::Html => xml::highlight_html(text),
            Syntax::Sql => code::highlight(text, &code::SQL),
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
    /// The text highlight rules, for plain text.
    text_rules: Arc<CompiledRules>,
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
            text_rules: installed_text_rules(ui.ctx()),
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

impl HighlightStyle {
    /// The color of text matched by a highlight rule.
    pub fn rule_color(&self, color: RuleColor) -> Color32 {
        let syntax = &self.syntax;
        match color {
            RuleColor::Keyword => syntax.keyword,
            RuleColor::String => syntax.string,
            RuleColor::Number => syntax.number,
            RuleColor::Name => syntax.name,
            RuleColor::Key => syntax.key,
            RuleColor::Comment => syntax.comment,
            RuleColor::Custom(rgb) => Color32::from_rgb(rgb.0, rgb.1, rgb.2),
        }
    }
}

impl HighlightStyle {
    /// The pieces of `text` the text highlight rules color, in order, with
    /// their colors. The rules are tried again only when the text or the
    /// rules change, so the preview can ask for every paragraph every frame.
    pub fn rule_spans(&self, ctx: &Context, text: &str) -> Vec<(Range<usize>, Color32)> {
        if self.text_rules.source().is_empty() {
            return Vec::new();
        }
        let found =
            ctx.memory_mut(|memory| memory.caches.cache::<RuleSpanCache>().get((text, &*self.text_rules)).clone());
        found.iter().map(|(range, color)| (range.clone(), self.rule_color(*color))).collect()
    }

    /// How many pieces of `text` each installed rule colors, by its place in
    /// the rules. Empty while no rules are installed (highlights off).
    pub fn rule_match_counts(&self, text: &str) -> Vec<usize> {
        self.text_rules.match_counts(text)
    }
}

/// The rule matches of a text, reused while the text and the rules stay the same.
type RuleSpanCache = FrameCache<Arc<Vec<(Range<usize>, RuleColor)>>, RuleSpanFinder>;

/// Finds the rule matches for the cache.
#[derive(Default)]
struct RuleSpanFinder;

impl ComputerMut<(&str, &CompiledRules), Arc<Vec<(Range<usize>, RuleColor)>>> for RuleSpanFinder {
    fn compute(&mut self, (text, rules): (&str, &CompiledRules)) -> Arc<Vec<(Range<usize>, RuleColor)>> {
        Arc::new(rules.spans(text))
    }
}

/// Where the compiled text highlight rules are kept in egui's memory.
fn text_rules_id() -> eframe::egui::Id {
    eframe::egui::Id::unique("rubrique-text-rules")
}

/// Make `rules` color plain text from the next frame on. They are compiled
/// only when they differ from the installed ones, so this is cheap to call
/// every frame the rules may have changed.
pub fn install_text_rules(ctx: &Context, rules: &[HighlightRule]) {
    if installed_text_rules(ctx).source() == rules {
        return;
    }
    let compiled = Arc::new(CompiledRules::new(rules));
    ctx.data_mut(|data| data.insert_temp(text_rules_id(), compiled));
    ctx.request_repaint();
}

/// The rules installed with [`install_text_rules`] (none before that).
fn installed_text_rules(ctx: &Context) -> Arc<CompiledRules> {
    ctx.data(|data| data.get_temp(text_rules_id())).unwrap_or_default()
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
        None => append_plain_text(&mut job, text, style),
    }
    job
}

/// Plain text with the pieces the text highlight rules match in their colors.
fn append_plain_text(job: &mut LayoutJob, text: &str, style: &HighlightStyle) {
    let plain = style.format(Kind::Plain);
    let mut done = 0;
    for (range, color) in style.text_rules.spans(text) {
        if done < range.start {
            job.append(&text[done..range.start], 0.0, plain.clone());
        }
        job.append(&text[range.clone()], 0.0, TextFormat { color: style.rule_color(color), ..plain.clone() });
        done = range.end;
    }
    // Always at least one section, so an empty text keeps its line height.
    if done < text.len() || job.sections.is_empty() {
        job.append(&text[done..], 0.0, plain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_and_markdown_take_text_highlights() {
        assert!(takes_text_highlights(Path::new("logs/server.log")));
        assert!(takes_text_highlights(Path::new("notes/todo.txt")));
        assert!(takes_text_highlights(Path::new("README")));
        assert!(takes_text_highlights(Path::new("notes/ideas.md")));
        assert!(!takes_text_highlights(Path::new("src/main.rs")));
        assert!(!takes_text_highlights(Path::new("package.json")));
    }

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
        assert_eq!(Syntax::of(Path::new("query.sql")), Some(Syntax::Sql));
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
        assert_eq!(Syntax::from_code_info("sql"), Some(Syntax::Sql));
        assert_eq!(Syntax::from_code_info("PostgreSQL"), Some(Syntax::Sql));
        assert_eq!(Syntax::from_code_info(""), None);
    }

    #[test]
    fn markdown_colors_fenced_code_by_its_language() {
        let text = "# Notes\n```python\ndef f(): pass\n```\n";
        let kind_of = |piece: &str| {
            let at = text.find(piece).unwrap();
            Syntax::Markdown.spans(text).into_iter().find(|span| span.range.contains(&at)).map(|span| span.kind)
        };
        assert_eq!(kind_of("def"), Some(Kind::Keyword));
        assert_eq!(kind_of("f()"), Some(Kind::Name));
        assert_eq!(kind_of("```python"), Some(Kind::Marker));
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
    fn plain_text_is_colored_by_the_installed_rules() {
        let ctx = Context::default();
        ctx.set_visuals(eframe::egui::Visuals::dark());
        let rules = [HighlightRule::new("IP", r"[0-9]+(?:\.[0-9]+){3}", RuleColor::Number)];
        install_text_rules(&ctx, &rules);
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            let style = HighlightStyle::from_ui(ui);
            let job = layout_job("ping 10.0.0.1 now", &style, None);
            let pieces: Vec<&str> = job
                .sections
                .iter()
                .map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0])
                .collect();
            assert_eq!(pieces, ["ping ", "10.0.0.1", " now"]);
            assert_eq!(job.sections[1].format.color, ui.visuals().error_fg_color);
            assert_eq!(job.sections[0].format.color, ui.visuals().text_color());
            // Code keeps its own colors: the rules are for plain text only.
            let code = layout_job("x = 10.0.0.1", &style, Some(Syntax::Python));
            assert!(code.sections.iter().all(|section| section.format.color != Color32::PLACEHOLDER));
            assert_eq!(layout_job("", &style, None).sections.len(), 1);
        });
        output.textures_delta.clear();
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
