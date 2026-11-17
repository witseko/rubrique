//! Explicit markup: lines that start with `..`. Directives become blocks
//! (code, admonitions, images, tables, raw HTML); footnotes show their
//! label; targets, substitution definitions and comments show nothing.

use super::blocks::{Parsed, Parser, field_marker, lead_with, table_from_cells};
use super::lines::{body_end, dedent, indent, is_blank};
use super::tables::{RawTable, csv_fields};
use crate::markdown::blocks::{Block, ColumnAlign};
use crate::markdown::html_blocks::parse_html_blocks;
use crate::markdown::inlines::{HtmlStyle, ImageSize, Inline, Script};

/// Directives that change nothing a single-file preview can show: tables
/// of contents, index entries, page metadata, includes of other files.
const HIDDEN_DIRECTIVES: &[&str] = &[
    "contents",
    "toctree",
    "index",
    "meta",
    "title",
    "sectnum",
    "section-numbering",
    "header",
    "footer",
    "default-role",
    "role",
    "include",
    "literalinclude",
    "target-notes",
    "tabularcolumns",
    "highlight",
];

/// Admonitions shown as a quote under their name.
const ADMONITIONS: &[&str] =
    &["note", "tip", "hint", "important", "warning", "caution", "danger", "attention", "error", "todo"];

/// A directive: `.. name:: argument`, its `:option: value` lines, then
/// its content.
struct Directive<'a> {
    /// Lowercase, without a Sphinx domain (`py:function` is `function`).
    name: String,
    argument: &'a str,
    options: Vec<(&'a str, &'a str)>,
    content: Vec<&'a str>,
}

impl Directive<'_> {
    /// The value of option `name`, if given.
    fn option(&self, name: &str) -> Option<&str> {
        self.options.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| *value)
    }
}

impl Parser {
    /// The explicit markup block at `lines[index]` (which starts with `..`).
    pub(super) fn explicit_markup(&mut self, lines: &[&str], index: usize, depth: usize) -> Parsed {
        let rest = lines[index].get(2..).unwrap_or("").trim_start();
        let end = body_end(lines, index + 1);
        let body = dedent(&lines[index + 1..end]);
        if rest.starts_with('_') || rest.starts_with('|') {
            // Targets and substitution definitions; `Refs` already has them.
            return (Vec::new(), end);
        }
        if let Some((label, text)) = footnote_label(rest) {
            return (self.footnote(label, text, &body, depth), end);
        }
        let Some((name, argument)) = directive_head(rest) else {
            // A comment.
            return (Vec::new(), end);
        };
        let directive = read_directive(name, argument, &body);
        (self.directive(&directive, depth), end)
    }

    /// A footnote or citation: its label in superscript, then its text.
    fn footnote(&mut self, label: &str, text: &str, body: &[&str], depth: usize) -> Vec<Block> {
        let shown = self.refs.footnote_definition_label(label);
        let mut content = vec![text];
        content.extend(body);
        let blocks = self.parse_lines(&content, depth + 1);
        lead_with(vec![superscript(&shown)], blocks)
    }

    fn directive(&mut self, directive: &Directive, depth: usize) -> Vec<Block> {
        let name = directive.name.as_str();
        match name {
            "code" | "code-block" | "sourcecode" => vec![code_block(directive)],
            "math" => vec![math_block(directive)],
            "admonition" | "topic" | "sidebar" => self.admonition(directive.argument, &directive.content, depth),
            "seealso" => self.admonition("See also", &with_argument(directive), depth),
            "versionadded" | "versionchanged" | "deprecated" | "versionremoved" => self.version_note(directive, depth),
            "image" => aligned(directive, vec![Block::Paragraph(vec![self.image(directive)])]),
            "figure" => {
                let mut blocks = vec![Block::Paragraph(vec![self.image(directive)])];
                blocks.extend(self.parse_lines(&directive.content, depth + 1));
                aligned(directive, blocks)
            }
            "raw" if directive.argument.split_whitespace().any(|format| format.eq_ignore_ascii_case("html")) => {
                vec![Block::Html(parse_html_blocks(&directive.content.join("\n")))]
            }
            "raw" => Vec::new(),
            "rubric" => vec![Block::Paragraph(vec![Inline::Strong(self.inlines(directive.argument))])],
            "list-table" => self.captioned(directive, |parser| parser.list_table(directive, depth)),
            "csv-table" => self.captioned(directive, |parser| parser.csv_table(directive)),
            "table" => {
                self.captioned(directive, |parser| parser.parse_lines(&directive.content, depth + 1).into_iter().next())
            }
            _ if ADMONITIONS.contains(&name) => self.admonition(&title_case(name), &with_argument(directive), depth),
            _ if HIDDEN_DIRECTIVES.contains(&name) => {
                if name == "highlight" {
                    self.default_language = directive.argument.trim().to_string();
                }
                Vec::new()
            }
            // Others (`only`, `container`, `class`, ...) show their content.
            _ => self.parse_lines(&directive.content, depth + 1),
        }
    }

    /// A quote that starts with `title` in bold.
    fn admonition(&mut self, title: &str, content: &[&str], depth: usize) -> Vec<Block> {
        let mut blocks = Vec::new();
        if !title.trim().is_empty() {
            blocks.push(Block::Paragraph(vec![Inline::Strong(self.inlines(title.trim()))]));
        }
        blocks.extend(self.parse_lines(content, depth + 1));
        vec![Block::Quote(blocks)]
    }

    /// `.. versionadded:: 2.1 explanation`: an italic label, then the rest.
    fn version_note(&mut self, directive: &Directive, depth: usize) -> Vec<Block> {
        let (version, explanation) =
            directive.argument.trim().split_once(' ').unwrap_or((directive.argument.trim(), ""));
        let label = match directive.name.as_str() {
            "versionadded" => "New in version",
            "versionchanged" => "Changed in version",
            "versionremoved" => "Removed in version",
            _ => "Deprecated since version",
        };
        let mut content = vec![explanation.trim()];
        content.extend(&directive.content);
        let blocks = self.parse_lines(&content, depth + 1);
        let ending = if blocks.is_empty() { "." } else { ":" };
        let label = Inline::Emphasis(vec![Inline::Text(format!("{label} {version}{ending}"))]);
        lead_with(vec![label], blocks)
    }

    /// The image of an `image` or `figure` directive, a link when it has a
    /// `:target:`.
    fn image(&mut self, directive: &Directive) -> Inline {
        let image = Inline::Image {
            alt: directive.option("alt").unwrap_or_default().to_string(),
            url: directive.argument.split_whitespace().collect(),
            size: ImageSize {
                width: directive.option("width").and_then(pixels),
                height: directive.option("height").and_then(pixels),
            },
        };
        let target = directive.option("target").and_then(|target| match target.strip_suffix('_') {
            Some(name) => self.refs.target(name.trim_matches('`')).map(str::to_string),
            None => Some(target.to_string()),
        });
        match target {
            Some(url) => Inline::Link { content: vec![image], url },
            None => image,
        }
    }

    /// A table directive's title in bold above the table it builds.
    fn captioned(&mut self, directive: &Directive, build: impl FnOnce(&mut Parser) -> Option<Block>) -> Vec<Block> {
        let mut blocks = Vec::new();
        if !directive.argument.trim().is_empty() {
            blocks.push(Block::Paragraph(vec![Inline::Strong(self.inlines(directive.argument.trim()))]));
        }
        blocks.extend(build(self));
        blocks
    }

    /// `list-table`: a bullet list of rows, each a bullet list of cells.
    fn list_table(&mut self, directive: &Directive, depth: usize) -> Option<Block> {
        let blocks = self.parse_lines(&directive.content, depth + 1);
        let Some(Block::List(list)) = blocks.into_iter().next() else {
            return None;
        };
        let mut rows: Vec<Vec<Vec<Inline>>> = list
            .items
            .into_iter()
            .map(|row| match row.blocks.into_iter().next() {
                Some(Block::List(cells)) => cells.items.into_iter().map(|cell| cell_inlines(cell.blocks)).collect(),
                _ => Vec::new(),
            })
            .collect();
        let header_rows = header_rows(directive);
        let header = (header_rows > 0 && !rows.is_empty()).then(|| rows.remove(0));
        table_from_cells(header, rows)
    }

    /// `csv-table`: comma-separated rows, with a `:header:` row or
    /// `:header-rows:`. Rows from `:file:` or `:url:` are not read.
    fn csv_table(&mut self, directive: &Directive) -> Option<Block> {
        let mut rows: Vec<Vec<String>> =
            directive.content.iter().filter(|line| !is_blank(line)).map(|line| csv_fields(line)).collect();
        let header = match directive.option("header") {
            Some(header) => vec![csv_fields(header)],
            None => rows.drain(..header_rows(directive).min(rows.len())).collect(),
        };
        self.table(RawTable { header, rows })
    }
}

/// The label and the text after it in `[label] text`, when the label has
/// no spaces (a bracketed comment does).
fn footnote_label(rest: &str) -> Option<(&str, &str)> {
    let (label, text) = rest.strip_prefix('[')?.split_once(']')?;
    let valid = !label.is_empty() && !label.contains(char::is_whitespace);
    valid.then(|| (label, text.trim()))
}

/// The name and argument of `name:: argument`, or `None` for a comment.
fn directive_head(rest: &str) -> Option<(&str, &str)> {
    let (name, argument) = rest.split_once("::")?;
    let valid_name = !name.is_empty()
        && name.chars().all(|character| character.is_ascii_alphanumeric() || "-_:.+".contains(character));
    (valid_name && (argument.is_empty() || argument.starts_with(' '))).then(|| (name, argument.trim()))
}

/// Split a directive's dedented body into its options and content.
fn read_directive<'a>(name: &str, argument: &'a str, body: &[&'a str]) -> Directive<'a> {
    let name = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();
    let mut options = Vec::new();
    let mut position = 0;
    while let Some((key, value_start)) = body.get(position).and_then(|line| field_marker(line)) {
        options.push((key, body[position][value_start..].trim()));
        position += 1;
        // An option's value may go on over more indented lines; they are skipped.
        while body.get(position).is_some_and(|line| !is_blank(line) && indent(line) > 0) {
            position += 1;
        }
    }
    while body.get(position).is_some_and(|line| is_blank(line)) {
        position += 1;
    }
    Directive { name, argument, options, content: body[position.min(body.len())..].to_vec() }
}

/// An admonition's argument is the start of its text (`.. note:: Read
/// this.`), so it goes before the content.
fn with_argument<'a>(directive: &Directive<'a>) -> Vec<&'a str> {
    let mut content = Vec::new();
    if !directive.argument.is_empty() {
        content.push(directive.argument);
        if !directive.content.is_empty() {
            content.push("");
        }
    }
    content.extend(&directive.content);
    content
}

/// `code-block:: python`: the language is the first word of the argument.
fn code_block(directive: &Directive) -> Block {
    let info = directive.argument.split_whitespace().next().unwrap_or_default().to_string();
    Block::Code { info, code: directive.content.join("\n") }
}

/// `math`: the formula in the argument or the content, as code.
fn math_block(directive: &Directive) -> Block {
    let code = if directive.content.is_empty() { directive.argument.to_string() } else { directive.content.join("\n") };
    Block::Code { info: "math".to_string(), code }
}

/// The blocks centered or right-aligned when the directive's `:align:`
/// asks; otherwise the blocks as they are.
fn aligned(directive: &Directive, blocks: Vec<Block>) -> Vec<Block> {
    let align = match directive.option("align").map(str::to_ascii_lowercase).as_deref() {
        Some("center") => ColumnAlign::Center,
        Some("right") => ColumnAlign::Right,
        _ => return blocks,
    };
    vec![Block::Aligned { align, blocks }]
}

/// A size in pixels (`200` or `200px`); other units are left to the image.
fn pixels(value: &str) -> Option<u16> {
    value.trim().trim_end_matches("px").trim().parse().ok()
}

/// `:header-rows:` as a number, 0 when absent.
fn header_rows(directive: &Directive) -> usize {
    directive.option("header-rows").and_then(|rows| rows.trim().parse().ok()).unwrap_or(0)
}

/// A table cell's blocks as one line of inlines.
fn cell_inlines(blocks: Vec<Block>) -> Vec<Inline> {
    let mut inlines = Vec::new();
    for block in blocks {
        let part = match block {
            Block::Paragraph(content) => content,
            Block::Code { code, .. } => vec![Inline::Code(code)],
            _ => continue,
        };
        if !inlines.is_empty() {
            inlines.push(Inline::Text(" ".to_string()));
        }
        inlines.extend(part);
    }
    inlines
}

fn superscript(text: &str) -> Inline {
    let style = HtmlStyle { script: Some(Script::Superscript), ..HtmlStyle::default() };
    Inline::Styled { style, content: vec![Inline::Text(text.to_string())] }
}

/// `note` as `Note`.
fn title_case(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::markdown::blocks::{Block, ColumnAlign};
    use crate::markdown::inlines::{ImageSize, Inline};
    use crate::rst::parse_rst;

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_string())
    }

    fn paragraph(value: &str) -> Block {
        Block::Paragraph(vec![text(value)])
    }

    fn strong(value: &str) -> Block {
        Block::Paragraph(vec![Inline::Strong(vec![text(value)])])
    }

    #[test]
    fn code_blocks_keep_language_and_indentation() {
        assert_eq!(
            parse_rst(".. code-block:: python\n   :linenos:\n\n   def f():\n       return 1\n\nafter"),
            [
                Block::Code { info: "python".to_string(), code: "def f():\n    return 1".to_string() },
                paragraph("after")
            ]
        );
    }

    #[test]
    fn highlight_sets_the_language_of_literal_blocks() {
        assert_eq!(
            parse_rst(".. highlight:: rust\n\nCode::\n\n   let x = 1;"),
            [paragraph("Code:"), Block::Code { info: "rust".to_string(), code: "let x = 1;".to_string() }]
        );
    }

    #[test]
    fn admonitions_are_quotes_under_their_name() {
        assert_eq!(parse_rst(".. note:: Read this."), [Block::Quote(vec![strong("Note"), paragraph("Read this.")])]);
        assert_eq!(
            parse_rst(".. warning::\n\n   Careful.\n\n   Really."),
            [Block::Quote(vec![strong("Warning"), paragraph("Careful."), paragraph("Really.")])]
        );
        assert_eq!(
            parse_rst(".. admonition:: Custom title\n\n   Body."),
            [Block::Quote(vec![strong("Custom title"), paragraph("Body.")])]
        );
    }

    #[test]
    fn images_with_options_and_figures() {
        let image = Inline::Image {
            alt: "Logo".to_string(),
            url: "logo.png".to_string(),
            size: ImageSize { width: Some(200), height: None },
        };
        assert_eq!(
            parse_rst(".. image:: logo.png\n   :alt: Logo\n   :width: 200px\n   :target: https://x.org"),
            [Block::Paragraph(vec![Inline::Link { content: vec![image.clone()], url: "https://x.org".to_string() }])]
        );
        assert_eq!(
            parse_rst(".. figure:: logo.png\n   :alt: Logo\n   :width: 200\n   :align: center\n\n   The caption."),
            [Block::Aligned {
                align: ColumnAlign::Center,
                blocks: vec![Block::Paragraph(vec![image]), paragraph("The caption.")]
            }]
        );
    }

    #[test]
    fn comments_targets_and_hidden_directives_show_nothing() {
        let text = ".. a comment\n   more comment\n\n.. _target: https://x.org\n\n.. contents::\n   :depth: 2\n\n..\n   block comment\n\nkept";
        assert_eq!(parse_rst(text), [paragraph("kept")]);
    }

    #[test]
    fn unknown_directives_show_their_content() {
        assert_eq!(parse_rst(".. only:: html\n\n   Shown."), [paragraph("Shown.")]);
    }

    #[test]
    fn footnotes_show_their_label() {
        let blocks = parse_rst("Fact [#]_.\n\n.. [#] The source.");
        let Block::Paragraph(footnote) = &blocks[1] else { panic!("expected a paragraph: {blocks:?}") };
        assert!(matches!(&footnote[0], Inline::Styled { content, .. } if content == &[text("1")]));
        assert_eq!(footnote[1..], [text(" "), text("The source.")]);
    }

    #[test]
    fn list_and_csv_tables() {
        let list = ".. list-table:: Title\n   :header-rows: 1\n\n   * - A\n     - B\n   * - 1\n     - 2";
        let blocks = parse_rst(list);
        assert_eq!(blocks[0], strong("Title"));
        let Block::Table(table) = &blocks[1] else { panic!("expected a table: {blocks:?}") };
        assert_eq!(table.header, [vec![text("A")], vec![text("B")]]);
        assert_eq!(table.rows, [vec![vec![text("1")], vec![text("2")]]]);

        let csv = ".. csv-table::\n   :header: \"Name\", \"Notes\"\n\n   a, \"x, y\"";
        let blocks = parse_rst(csv);
        let [Block::Table(table)] = blocks.as_slice() else { panic!("expected a table: {blocks:?}") };
        assert_eq!(table.header, [vec![text("Name")], vec![text("Notes")]]);
        assert_eq!(table.rows, [vec![vec![text("a")], vec![text("x, y")]]]);
    }

    #[test]
    fn raw_html_is_read_by_the_html_reader() {
        let blocks = parse_rst(".. raw:: html\n\n   <p>hi</p><script>x</script>");
        assert_eq!(blocks, [Block::Html(vec![paragraph("hi")])]);
        assert_eq!(parse_rst(".. raw:: latex\n\n   \\LaTeX"), []);
    }

    #[test]
    fn version_notes() {
        assert_eq!(
            parse_rst(".. versionadded:: 2.1 The *x* option."),
            [Block::Paragraph(vec![
                Inline::Emphasis(vec![text("New in version 2.1:")]),
                text(" "),
                text("The "),
                Inline::Emphasis(vec![text("x")]),
                text(" option."),
            ])]
        );
    }
}
