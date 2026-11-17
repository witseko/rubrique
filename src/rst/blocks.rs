//! Block-level reStructuredText: section titles, transitions, paragraphs
//! and literal blocks, block quotes, bullet and enumerated lists, field and
//! definition lists, line blocks, doctests and tables. Explicit markup
//! (`..` lines) is in `directives.rs`.
//!
//! Works line by line like the Markdown reader: containers (list items,
//! quotes, directive bodies) strip their indentation and parse their
//! content recursively with the same function.

use super::inlines::parse_inlines;
use super::lines::{adornment_char, body_end, dedent, indent, is_blank, is_transition_line, underlines};
use super::refs::Refs;
use super::tables::{self, RawTable};
use crate::markdown::blocks::{Block, ColumnAlign, List, ListItem, Table};
use crate::markdown::inlines::Inline;

/// Deepest nesting of containers read as blocks; content nested deeper
/// stays as paragraph text, so a hostile file cannot overflow the stack.
const MAX_BLOCK_DEPTH: usize = 32;

/// Deepest heading level the preview draws.
const MAX_HEADING_LEVEL: usize = 6;

/// Characters that start a bullet list item.
const BULLETS: &str = "*+-\u{2022}\u{2023}\u{2043}";

/// Most digits read as a list number, so the number fits a `u64`.
const MAX_LIST_NUMBER_DIGITS: usize = 9;

/// How a section title is adorned; the first style seen is level 1, the
/// next new one level 2, and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TitleStyle {
    character: char,
    overline: bool,
}

/// Reads one document's blocks.
#[derive(Debug)]
pub(super) struct Parser {
    pub(super) refs: Refs,
    title_styles: Vec<TitleStyle>,
    /// Language of `::` literal blocks, set by the `highlight` directive.
    pub(super) default_language: String,
}

/// Blocks read from a run of lines, and the index after them.
pub(super) type Parsed = (Vec<Block>, usize);

impl Parser {
    pub(super) fn new(refs: Refs) -> Parser {
        Parser { refs, title_styles: Vec::new(), default_language: String::new() }
    }

    /// Parse lines into blocks, `depth` containers deep.
    pub(super) fn parse_lines(&mut self, lines: &[&str], depth: usize) -> Vec<Block> {
        if depth > MAX_BLOCK_DEPTH {
            let text = lines.iter().map(|line| line.trim()).collect::<Vec<_>>().join(" ");
            return if text.trim().is_empty() { Vec::new() } else { vec![Block::Paragraph(vec![Inline::Text(text)])] };
        }
        let mut blocks = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            if is_blank(lines[index]) {
                index += 1;
                continue;
            }
            let (found, next) = self.parse_block(lines, index, depth);
            blocks.extend(found);
            index = next.max(index + 1);
        }
        blocks
    }

    /// Parse the inline text of a paragraph, heading or cell.
    pub(super) fn inlines(&mut self, text: &str) -> Vec<Inline> {
        parse_inlines(text, &mut self.refs)
    }

    /// Parse the block that starts at the non-blank `lines[index]`.
    fn parse_block(&mut self, lines: &[&str], index: usize, depth: usize) -> Parsed {
        let line = lines[index];
        if indent(line) > 0 {
            return self.block_quote(lines, index, depth);
        }
        if let Some(parsed) = self.section_title(lines, index) {
            return parsed;
        }
        if is_transition_line(line) && lines.get(index + 1).is_none_or(|next| is_blank(next)) {
            return (vec![Block::ThematicBreak], index + 1);
        }
        if line == ".." || line.starts_with(".. ") {
            return self.explicit_markup(lines, index, depth);
        }
        if line.starts_with("__ ") {
            // An anonymous target; `Refs` already has it.
            return (Vec::new(), body_end(lines, index + 1));
        }
        if let Some((table, next)) = tables::simple_table(lines, index).or_else(|| tables::grid_table(lines, index)) {
            return (self.table(table).into_iter().collect(), next);
        }
        self.list_like(lines, index, depth).unwrap_or_else(|| self.paragraph(lines, index))
    }

    /// Lists, field and definition lists, line blocks and doctests: the
    /// blocks recognized by how their first line starts.
    fn list_like(&mut self, lines: &[&str], index: usize, depth: usize) -> Option<Parsed> {
        let line = lines[index];
        if bullet_marker(line).is_some() {
            return Some(self.bullet_list(lines, index, depth));
        }
        if let Some(marker) = enumerator(line).filter(|_| starts_enumerated_list(lines, index)) {
            return Some(self.enumerated_list(lines, index, marker, depth));
        }
        if field_marker(line).is_some() {
            return Some(self.field(lines, index, depth));
        }
        if line == "|" || line.starts_with("| ") {
            return Some(self.line_block(lines, index));
        }
        if line.starts_with(">>>") {
            return Some(doctest(lines, index));
        }
        let next_is_definition = lines.get(index + 1).is_some_and(|next| !is_blank(next) && indent(next) > 0);
        if next_is_definition && !line.trim_end().ends_with("::") {
            return Some(self.definition(lines, index, depth));
        }
        None
    }

    /// An over- and underlined or underlined title at `lines[index]`.
    fn section_title(&mut self, lines: &[&str], index: usize) -> Option<Parsed> {
        let line = lines[index];
        let (title, style, next) = match (lines.get(index + 1), lines.get(index + 2)) {
            (Some(text), Some(under))
                if adornment_char(line).is_some() && !is_blank(text) && under.trim_end() == line.trim_end() =>
            {
                let character = adornment_char(line)?;
                (text.trim(), TitleStyle { character, overline: true }, index + 3)
            }
            (Some(under), _) if adornment_char(line).is_none() && underlines(line, under) => {
                let character = adornment_char(under)?;
                (line.trim(), TitleStyle { character, overline: false }, index + 2)
            }
            _ => return None,
        };
        let level = self.title_level(style);
        let content = self.inlines(title);
        Some((vec![Block::Heading { level, content }], next))
    }

    /// The heading level of a title style, by the order styles first appear.
    fn title_level(&mut self, style: TitleStyle) -> u8 {
        let position = self.title_styles.iter().position(|known| *known == style).unwrap_or_else(|| {
            self.title_styles.push(style);
            self.title_styles.len() - 1
        });
        u8::try_from((position + 1).min(MAX_HEADING_LEVEL)).unwrap_or(u8::MAX)
    }

    /// Indented lines (not after `::`): a quote of their blocks.
    fn block_quote(&mut self, lines: &[&str], index: usize, depth: usize) -> Parsed {
        let end = body_end(lines, index);
        let content = dedent(&lines[index..end]);
        (vec![Block::Quote(self.parse_lines(&content, depth + 1))], end)
    }

    /// Lines up to a blank or indented line. A paragraph ending in `::`
    /// introduces the literal block below it.
    fn paragraph(&mut self, lines: &[&str], index: usize) -> Parsed {
        let mut end = index;
        while end < lines.len() && !is_blank(lines[end]) && indent(lines[end]) == 0 {
            end += 1;
        }
        let text = lines[index..end].iter().map(|line| line.trim()).collect::<Vec<_>>().join(" ");
        let Some(before) = text.strip_suffix("::") else {
            return (vec![Block::Paragraph(self.inlines(&text))], end);
        };
        // `text::` shows `text:`, `text ::` shows `text`, a lone `::` nothing.
        let shown = if before.is_empty() || before.ends_with(char::is_whitespace) {
            before.trim_end().to_string()
        } else {
            format!("{before}:")
        };
        let mut blocks = Vec::new();
        if !shown.is_empty() {
            blocks.push(Block::Paragraph(self.inlines(&shown)));
        }
        let (code, next) = literal_block(lines, end);
        if let Some(code) = code {
            blocks.push(Block::Code { info: self.default_language.clone(), code });
        }
        (blocks, next)
    }

    /// Consecutive items with the same bullet character.
    fn bullet_list(&mut self, lines: &[&str], index: usize, depth: usize) -> Parsed {
        let Some(bullet) = bullet_marker(lines[index]).map(|(bullet, _)| bullet) else {
            return (Vec::new(), index + 1);
        };
        let mut items = Vec::new();
        let mut next = index;
        while let Some((marker, text_start)) = lines.get(next).and_then(|line| bullet_marker(line)) {
            if marker != bullet {
                break;
            }
            let (item, end) = self.list_item(lines, next, text_start, depth);
            items.push(item);
            next = skip_blank(lines, end);
        }
        (vec![Block::List(List { start: None, items })], next)
    }

    /// Consecutive items with the same enumerator format.
    fn enumerated_list(&mut self, lines: &[&str], index: usize, first: Enumerator, depth: usize) -> Parsed {
        let mut items = Vec::new();
        let mut next = index;
        while let Some(marker) = lines.get(next).and_then(|line| enumerator(line)) {
            if marker.format != first.format || (next > index && !starts_enumerated_list(lines, next)) {
                break;
            }
            let (item, end) = self.list_item(lines, next, marker.text_start, depth);
            items.push(item);
            next = skip_blank(lines, end);
        }
        (vec![Block::List(List { start: Some(first.number), items })], next)
    }

    /// The item whose marker line is `lines[index]` with its text at byte
    /// `text_start`, plus the indented lines below it.
    fn list_item(&mut self, lines: &[&str], index: usize, text_start: usize, depth: usize) -> (ListItem, usize) {
        let end = body_end(lines, index + 1);
        let mut content = vec![lines[index].get(text_start..).unwrap_or("").trim_start()];
        content.extend(dedent(&lines[index + 1..end]));
        let blocks = self.parse_lines(&content, depth + 1);
        (ListItem { task: None, blocks }, end)
    }

    /// One `:name: body` field, shown as a paragraph that starts with the
    /// name in bold.
    fn field(&mut self, lines: &[&str], index: usize, depth: usize) -> Parsed {
        let line = lines[index];
        let Some((name, body_start)) = field_marker(line) else {
            return (Vec::new(), index + 1);
        };
        let end = body_end(lines, index + 1);
        let mut content = vec![line[body_start..].trim()];
        content.extend(dedent(&lines[index + 1..end]));
        let name = vec![Inline::Strong(self.inlines(&format!("{name}:")))];
        let blocks = self.parse_lines(&content, depth + 1);
        (lead_with(name, blocks), end)
    }

    /// A term with its indented definition: the term in bold, the
    /// definition as an indented quote.
    fn definition(&mut self, lines: &[&str], index: usize, depth: usize) -> Parsed {
        let end = body_end(lines, index + 1);
        // `term : classifier` shows the term only.
        let term = lines[index].split(" : ").next().unwrap_or(lines[index]).trim();
        let term = vec![Inline::Strong(self.inlines(term))];
        let definition = self.parse_lines(&dedent(&lines[index + 1..end]), depth + 1);
        (vec![Block::Paragraph(term), Block::Quote(definition)], end)
    }

    /// `| line` lines, kept as written with a break after each. An
    /// indented line continues the line above it.
    fn line_block(&mut self, lines: &[&str], index: usize) -> Parsed {
        let mut texts: Vec<String> = Vec::new();
        let mut next = index;
        while let Some(line) = lines.get(next) {
            if *line == "|" {
                texts.push(String::new());
            } else if let Some(text) = line.strip_prefix("| ") {
                texts.push(text.to_string());
            } else if let Some(last) = texts.last_mut().filter(|_| !is_blank(line) && indent(line) > 0) {
                last.push(' ');
                last.push_str(line.trim());
            } else {
                break;
            }
            next += 1;
        }
        let mut content = Vec::new();
        for (position, text) in texts.iter().enumerate() {
            if position > 0 {
                content.push(Inline::LineBreak);
            }
            content.extend(self.inlines(text));
        }
        (vec![Block::Paragraph(content)], next)
    }

    /// A table from cell text. A table without a header row uses its first
    /// row as the header, since the preview always draws one.
    pub(super) fn table(&mut self, raw: RawTable) -> Option<Block> {
        let RawTable { header, rows } = raw;
        let header = header.into_iter().reduce(|mut merged, row| {
            append_row(&mut merged, &row);
            merged
        });
        let header = header.map(|row| row.iter().map(|cell| self.inlines(cell)).collect());
        let rows = rows.iter().map(|row| row.iter().map(|cell| self.inlines(cell)).collect()).collect();
        table_from_cells(header, rows)
    }
}

/// A table block from cells, every row padded to the widest one. Without a
/// header the first row becomes it; with no rows at all there is no table.
pub(super) fn table_from_cells(header: Option<Vec<Vec<Inline>>>, mut rows: Vec<Vec<Vec<Inline>>>) -> Option<Block> {
    let mut header = match header {
        Some(header) => header,
        None if rows.is_empty() => return None,
        None => rows.remove(0),
    };
    let columns = rows.iter().map(Vec::len).chain([header.len()]).max().unwrap_or(0);
    header.resize(columns, Vec::new());
    for row in &mut rows {
        row.resize(columns, Vec::new());
    }
    Some(Block::Table(Table { align: vec![ColumnAlign::Left; columns], header, rows }))
}

/// `inlines` at the front of the first block when it is a paragraph,
/// otherwise as a paragraph of their own before the blocks.
pub(super) fn lead_with(mut inlines: Vec<Inline>, mut blocks: Vec<Block>) -> Vec<Block> {
    if let Some(Block::Paragraph(first)) = blocks.first_mut() {
        if !first.is_empty() {
            inlines.push(Inline::Text(" ".to_string()));
        }
        inlines.append(first);
        *first = inlines;
    } else {
        blocks.insert(0, Block::Paragraph(inlines));
    }
    blocks
}

/// Add a second header row's text to the first, cell by cell.
fn append_row(merged: &mut Vec<String>, row: &[String]) {
    if merged.len() < row.len() {
        merged.resize(row.len(), String::new());
    }
    for (cell, text) in merged.iter_mut().zip(row) {
        if !cell.is_empty() && !text.is_empty() {
            cell.push(' ');
        }
        cell.push_str(text);
    }
}

/// The first non-blank line at or after `index`.
fn skip_blank(lines: &[&str], mut index: usize) -> usize {
    while index < lines.len() && is_blank(lines[index]) {
        index += 1;
    }
    index
}

/// The literal block after a `::` paragraph ending at `index`: indented
/// lines, or unindented lines that all start with the same punctuation
/// (a quoted literal block). Returns its text and the index after it.
fn literal_block(lines: &[&str], index: usize) -> (Option<String>, usize) {
    let start = skip_blank(lines, index);
    let Some(first) = lines.get(start) else {
        return (None, index);
    };
    if indent(first) > 0 {
        let end = body_end(lines, start);
        return (Some(dedent(&lines[start..end]).join("\n")), end);
    }
    let Some(quote) = first.chars().next().filter(char::is_ascii_punctuation) else {
        return (None, index);
    };
    let end = start + lines[start..].iter().take_while(|line| line.starts_with(quote)).count();
    (Some(lines[start..end].join("\n")), end)
}

/// `>>>` lines up to a blank line, as Python code.
fn doctest(lines: &[&str], index: usize) -> Parsed {
    let end = index + lines[index..].iter().take_while(|line| !is_blank(line)).count();
    (vec![Block::Code { info: "python".to_string(), code: lines[index..end].join("\n") }], end)
}

/// The bullet of a bullet item line and the byte where its text starts.
fn bullet_marker(line: &str) -> Option<(char, usize)> {
    let bullet = line.chars().next().filter(|first| BULLETS.contains(*first))?;
    let rest = &line[bullet.len_utf8()..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((bullet, line.len() - rest.trim_start().len()))
}

/// How an enumerated item writes its number: `1.`, `1)` or `(1)`, with
/// digits, letters or `#`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EnumeratorFormat {
    parentheses: Parentheses,
    letters: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parentheses {
    /// `1.`
    Period,
    /// `1)`
    Closing,
    /// `(1)`
    Both,
}

/// An enumerated item's marker: its number, format and where its text starts.
#[derive(Clone, Copy, Debug)]
struct Enumerator {
    number: u64,
    format: EnumeratorFormat,
    text_start: usize,
}

/// The enumerator at the start of `line`, followed by a space or the end.
fn enumerator(line: &str) -> Option<Enumerator> {
    let (inner_start, parentheses) =
        if line.starts_with('(') { (1, Parentheses::Both) } else { (0, Parentheses::Period) };
    let label_length = line[inner_start..].chars().take_while(char::is_ascii_alphanumeric).count();
    let label_length = if label_length == 0 && line[inner_start..].starts_with('#') { 1 } else { label_length };
    let label = &line[inner_start..inner_start + label_length];
    let closing = line[inner_start + label_length..].chars().next()?;
    let parentheses = match (parentheses, closing) {
        (Parentheses::Both, ')') => Parentheses::Both,
        (Parentheses::Period, '.') => Parentheses::Period,
        (Parentheses::Period, ')') => Parentheses::Closing,
        _ => return None,
    };
    let after = inner_start + label_length + 1;
    let rest = &line[after..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let (number, letters) = enumerator_number(label)?;
    let text_start = line.len() - rest.trim_start().len();
    Some(Enumerator { number, format: EnumeratorFormat { parentheses, letters }, text_start })
}

/// The number of an enumerator label: digits, one letter (`a` is 1) or
/// `#` (numbered automatically, from 1).
fn enumerator_number(label: &str) -> Option<(u64, bool)> {
    if label == "#" {
        return Some((1, false));
    }
    if label.len() <= MAX_LIST_NUMBER_DIGITS && label.chars().all(|digit| digit.is_ascii_digit()) {
        return Some((label.parse().ok()?, false));
    }
    let mut chars = label.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next().is_some() {
        return None;
    }
    Some((u64::from(letter.to_ascii_lowercase()) - u64::from('a') + 1, true))
}

/// True when the enumerated line at `index` starts an item rather than a
/// sentence (`A. Smith wrote`): the line after it is blank, indented, the
/// end, or another item of the same format.
fn starts_enumerated_list(lines: &[&str], index: usize) -> bool {
    let Some(next) = lines.get(index + 1) else {
        return true;
    };
    if is_blank(next) || indent(next) > 0 {
        return true;
    }
    match (enumerator(lines[index]), enumerator(next)) {
        (Some(this), Some(other)) => this.format == other.format,
        _ => false,
    }
}

/// The name of a `:name: body` field line and the byte where its body starts.
pub(super) fn field_marker(line: &str) -> Option<(&str, usize)> {
    let rest = line.strip_prefix(':')?;
    let close = rest.find(':')?;
    let name = &rest[..close];
    let after = &rest[close + 1..];
    let valid = !name.is_empty() && !name.starts_with(' ') && !name.contains('`');
    (valid && (after.is_empty() || after.starts_with(' '))).then_some((name, close + 2))
}

#[cfg(test)]
mod tests {
    use crate::markdown::blocks::{Block, List, ListItem};
    use crate::markdown::inlines::Inline;
    use crate::rst::parse_rst;

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_string())
    }

    fn paragraph(value: &str) -> Block {
        Block::Paragraph(vec![text(value)])
    }

    fn heading(level: u8, value: &str) -> Block {
        Block::Heading { level, content: vec![text(value)] }
    }

    fn item(value: &str) -> ListItem {
        ListItem { task: None, blocks: vec![paragraph(value)] }
    }

    #[test]
    fn title_levels_follow_the_order_styles_appear() {
        let text = "=====\nTitle\n=====\n\nPart\n----\n\nSection\n=======\n\nSub\n---\n";
        assert_eq!(
            parse_rst(text),
            [heading(1, "Title"), heading(2, "Part"), heading(3, "Section"), heading(2, "Sub")]
        );
    }

    #[test]
    fn paragraphs_join_lines_and_transitions_split_them() {
        assert_eq!(
            parse_rst("one\ntwo\n\n----\n\nthree"),
            [paragraph("one two"), Block::ThematicBreak, paragraph("three")]
        );
    }

    #[test]
    fn literal_blocks_after_double_colon() {
        let code = |code: &str| Block::Code { info: String::new(), code: code.to_string() };
        assert_eq!(
            parse_rst("Example::\n\n    a\n      b\n\nafter"),
            [paragraph("Example:"), code("a\n  b"), paragraph("after")]
        );
        assert_eq!(parse_rst("Example ::\n\n  x"), [paragraph("Example"), code("x")]);
        assert_eq!(parse_rst("::\n\n  x"), [code("x")]);
        assert_eq!(parse_rst("Quoted::\n\n> a\n> b"), [paragraph("Quoted:"), code("> a\n> b")]);
    }

    #[test]
    fn block_quotes_are_indented() {
        assert_eq!(
            parse_rst("intro\n\n   quoted\n   text"),
            [paragraph("intro"), Block::Quote(vec![paragraph("quoted text")])]
        );
    }

    #[test]
    fn bullet_lists_with_nested_content() {
        let nested = Block::List(List { start: None, items: vec![item("inner")] });
        assert_eq!(
            parse_rst("- one\n  continued\n\n  - inner\n\n- two\n\nafter"),
            [
                Block::List(List {
                    start: None,
                    items: vec![ListItem { task: None, blocks: vec![paragraph("one continued"), nested] }, item("two")]
                }),
                paragraph("after")
            ]
        );
    }

    #[test]
    fn enumerated_lists_and_sentences_that_look_like_them() {
        assert_eq!(
            parse_rst("3. three\n4. four"),
            [Block::List(List { start: Some(3), items: vec![item("three"), item("four")] })]
        );
        assert_eq!(
            parse_rst("(a) first\n(b) second"),
            [Block::List(List { start: Some(1), items: vec![item("first"), item("second")] })]
        );
        assert_eq!(parse_rst("A. Smith wrote\nthis book."), [paragraph("A. Smith wrote this book.")]);
    }

    #[test]
    fn field_and_definition_lists() {
        assert_eq!(
            parse_rst(":Author: Me\n:Version: 1.0\n\nterm : type\n   The definition."),
            [
                Block::Paragraph(vec![Inline::Strong(vec![text("Author:")]), text(" "), text("Me")]),
                Block::Paragraph(vec![Inline::Strong(vec![text("Version:")]), text(" "), text("1.0")]),
                Block::Paragraph(vec![Inline::Strong(vec![text("term")])]),
                Block::Quote(vec![paragraph("The definition.")]),
            ]
        );
    }

    #[test]
    fn line_blocks_keep_line_breaks() {
        assert_eq!(
            parse_rst("| first\n| second\n  continued"),
            [Block::Paragraph(vec![text("first"), Inline::LineBreak, text("second continued")])]
        );
    }

    #[test]
    fn doctests_are_python_code() {
        assert_eq!(
            parse_rst(">>> 1 + 1\n2"),
            [Block::Code { info: "python".to_string(), code: ">>> 1 + 1\n2".to_string() }]
        );
    }

    #[test]
    fn tables_without_header_use_the_first_row() {
        let blocks = parse_rst("==  ==\na   b\nc   d\n==  ==");
        let [Block::Table(table)] = blocks.as_slice() else { panic!("expected one table: {blocks:?}") };
        assert_eq!(table.header, [vec![text("a")], vec![text("b")]]);
        assert_eq!(table.rows, [vec![vec![text("c")], vec![text("d")]]]);
    }

    #[test]
    fn deep_nesting_stops_at_the_limit() {
        let deep: String = (0..200).map(|level| "  ".repeat(level) + "- x\n").collect();
        assert_eq!(parse_rst(&deep).len(), 1);
    }
}
