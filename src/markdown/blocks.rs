//! Block-level Markdown: headings, paragraphs, code, quotes, lists, tables,
//! breaks.
//!
//! Works line by line. Containers (quotes, list items) strip their markers
//! and parse their content recursively with the same function.

use super::inlines::{Inline, parse_inlines};

/// One block of the document.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// `level` is 1 to 6.
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    /// Fenced or indented code; `info` is the text after the opening fence.
    Code {
        info: String,
        code: String,
    },
    Quote(Vec<Block>),
    List(List),
    Table(Table),
    ThematicBreak,
    /// Blocks HTML centers or right-aligns (`align`, `text-align`,
    /// `<center>`).
    Aligned {
        align: ColumnAlign,
        blocks: Vec<Block>,
    },
    /// An HTML `<details>`: `summary` is always shown, `blocks` only once
    /// it is opened. `open` is how it starts.
    Details {
        summary: Vec<Inline>,
        open: bool,
        blocks: Vec<Block>,
    },
    /// An HTML block in a Markdown file (lines that start with a tag such
    /// as `<p align="center">`, up to a blank line), as the blocks the HTML
    /// reads as. The preview shows it but never rewrites it: its source is
    /// edited as text, like any block.
    Html(Vec<Block>),
}

/// A pipe table (GitHub style): a header row, a delimiter row that sets
/// each column's alignment, then rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// One entry per column.
    pub align: Vec<ColumnAlign>,
    /// Header cells, one per column.
    pub header: Vec<Vec<Inline>>,
    /// Body rows; each has exactly one cell per column.
    pub rows: Vec<Vec<Vec<Inline>>>,
}

/// How a column's cells are aligned, from the colons in the delimiter row.
/// HTML alignment (`align="center"`) uses it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Center,
    Right,
}

/// A bullet or ordered list.
#[derive(Clone, Debug, PartialEq)]
pub struct List {
    /// Start number for ordered lists, `None` for bullet lists.
    pub start: Option<u64>,
    pub items: Vec<ListItem>,
}

/// One list item and the blocks inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// `Some(checked)` for task items like `- [x] done`.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

/// Tab stops for leading indentation.
const TAB_WIDTH: usize = 4;

/// Indentation (in spaces) that makes a line indented code (CommonMark).
/// Below it, block markers like `#`, `>` and `-` still count.
const CODE_INDENT: usize = 4;

/// Deepest ATX heading level, `######`.
const MAX_HEADING_LEVEL: u8 = 6;

/// CommonMark allows at most this many digits in an ordered list number.
const MAX_LIST_NUMBER_DIGITS: usize = 9;

/// Deepest nesting of containers (quotes, list items) read as blocks;
/// content nested deeper stays as paragraph text, so a hostile file (a
/// line of thousands of `>`) cannot overflow the stack.
const MAX_BLOCK_DEPTH: usize = 32;

/// Parse a whole document.
pub fn parse_blocks(text: &str) -> Vec<Block> {
    let lines: Vec<String> = text.lines().map(expand_leading_tabs).collect();
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    parse_lines(&line_refs, 0)
}

/// A top-level block and where its source text is.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceBlock {
    pub block: Block,
    /// Byte range from the start of its first line to the end of its last
    /// line (line break not included).
    pub range: std::ops::Range<usize>,
}

/// Parse a whole document, remembering each top-level block's source text,
/// so the preview can edit one block in place.
pub fn parse_source_blocks(text: &str) -> Vec<SourceBlock> {
    let lines: Vec<String> = text.lines().map(expand_leading_tabs).collect();
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    // Start and end (without the line break) of every source line.
    let mut bounds = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']).len();
        bounds.push((offset, offset + content));
        offset += line.len();
    }
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < line_refs.len() {
        let (block, next) = parse_block(&line_refs, index, 0);
        if let Some(block) = block {
            // Blank lines a block swallowed at its end are not part of its text.
            let mut last = next.max(index + 1) - 1;
            while last > index && is_blank(line_refs[last]) {
                last -= 1;
            }
            let start = bounds.get(index).map_or(text.len(), |bound| bound.0);
            let end = bounds.get(last).map_or(text.len(), |bound| bound.1);
            blocks.push(SourceBlock { block, range: start..end });
        }
        index = next;
    }
    blocks
}

/// Parse lines into blocks, `depth` containers deep; used for the
/// document and for container content.
fn parse_lines(lines: &[&str], depth: usize) -> Vec<Block> {
    if depth > MAX_BLOCK_DEPTH {
        // Too deep to nest further: keep the text, markers and all, as one
        // paragraph rather than dropping it.
        let text = lines.join("\n");
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        return vec![Block::Paragraph(parse_inlines(text))];
    }
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let (block, next) = parse_block(lines, index, depth);
        blocks.extend(block);
        index = next;
    }
    blocks
}

/// Parse the block starting at `lines[index]`, `depth` containers deep;
/// returns it and the next index.
fn parse_block(lines: &[&str], index: usize, depth: usize) -> (Option<Block>, usize) {
    let line = lines[index];
    if is_blank(line) {
        return (None, index + 1);
    }
    if indentation(line) >= CODE_INDENT {
        return parse_indented_code(lines, index);
    }
    let trimmed = line.trim_start();
    if let Some(fence) = Fence::open(line) {
        return parse_fenced_code(lines, index, fence);
    }
    if let Some(start) = HtmlStart::of(trimmed) {
        return parse_html_block(lines, index, start);
    }
    if let Some(heading) = parse_atx_heading(trimmed) {
        return (Some(heading), index + 1);
    }
    if is_thematic_break(trimmed) {
        return (Some(Block::ThematicBreak), index + 1);
    }
    if trimmed.starts_with('>') {
        return parse_quote(lines, index, depth);
    }
    if let Some(marker) = ListMarker::parse(line) {
        return parse_list(lines, index, marker, depth);
    }
    if let Some((table, next)) = parse_table(lines, index) {
        return (Some(Block::Table(table)), next);
    }
    parse_paragraph(lines, index)
}

/// True when `line` begins a block that ends a running paragraph.
fn interrupts_paragraph(line: &str) -> bool {
    if indentation(line) >= CODE_INDENT {
        return false;
    }
    let trimmed = line.trim_start();
    let list_interrupts = ListMarker::parse(line)
        .is_some_and(|marker| marker.has_content && marker.number.is_none_or(|number| number == 1));
    // A lone tag (`<img ...>`) inside running text is inline HTML, as in
    // CommonMark; block tags and raw text elements start a block.
    let html_interrupts = HtmlStart::of(trimmed).is_some_and(|start| start != HtmlStart::LoneTag);
    Fence::open(line).is_some()
        || html_interrupts
        || parse_atx_heading(trimmed).is_some()
        || is_thematic_break(trimmed)
        || trimmed.starts_with('>')
        || list_interrupts
}

// ---------- headings, breaks, paragraphs ----------

/// `## Title ##` -> level 2 heading "Title".
fn parse_atx_heading(trimmed: &str) -> Option<Block> {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    let level = u8::try_from(hashes).ok().filter(|level| (1..=MAX_HEADING_LEVEL).contains(level))?;
    // `#` is one byte, so `hashes` is also a byte index.
    let rest = &trimmed[hashes..];
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let mut content = rest.trim();
    // Optional closing hashes, when separated by a space (or all hashes).
    let without_hashes = content.trim_end_matches('#');
    if without_hashes.is_empty() || without_hashes.ends_with(' ') {
        content = without_hashes.trim_end();
    }
    Some(Block::Heading { level, content: parse_inlines(content) })
}

/// `---`, `***`, `___`, spaces allowed in between.
fn is_thematic_break(trimmed: &str) -> bool {
    let marks: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && matches!(marks[0], '-' | '*' | '_') && marks.iter().all(|&c| c == marks[0])
}

/// A line of only `=` (level 1) or `-` (level 2) under paragraph text.
fn setext_level(line: &str) -> Option<u8> {
    if indentation(line) >= CODE_INDENT {
        return None;
    }
    let trimmed = line.trim();
    let all = |mark: char| !trimmed.is_empty() && trimmed.chars().all(|c| c == mark);
    if all('=') {
        Some(1)
    } else if all('-') {
        Some(2)
    } else {
        None
    }
}

/// Paragraph text up to a blank or interrupting line; a setext underline
/// turns it into a heading instead.
fn parse_paragraph(lines: &[&str], start: usize) -> (Option<Block>, usize) {
    let mut text_lines = vec![lines[start].trim_start()];
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        if let Some(level) = setext_level(line) {
            let content = parse_inlines(text_lines.join("\n").trim_end());
            return (Some(Block::Heading { level, content }), index + 1);
        }
        if is_blank(line) || interrupts_paragraph(line) {
            break;
        }
        // A delimiter row turns the line before it into a table header: the
        // table starts there, and the paragraph ends before it.
        if parse_table(lines, index - 1).is_some() {
            text_lines.pop();
            index -= 1;
            break;
        }
        text_lines.push(line.trim_start());
        index += 1;
    }
    if text_lines.is_empty() {
        return (None, index);
    }
    // Trailing spaces on the last line are not a hard break.
    let text = text_lines.join("\n");
    (Some(Block::Paragraph(parse_inlines(text.trim_end()))), index)
}

// ---------- tables ----------

/// A table starting at `lines[start]`: a header row, then a delimiter row
/// with the same number of cells, then rows until a blank line or another
/// block. Both first lines need a `|`, so `text` over `---` stays a heading.
fn parse_table(lines: &[&str], start: usize) -> Option<(Table, usize)> {
    let header_line = *lines.get(start)?;
    let delimiter_line = *lines.get(start + 1)?;
    if indentation(header_line) >= CODE_INDENT || !header_line.contains('|') || !delimiter_line.contains('|') {
        return None;
    }
    let align: Vec<ColumnAlign> =
        split_row(delimiter_line).iter().map(|cell| delimiter_align(cell)).collect::<Option<_>>()?;
    let header: Vec<Vec<Inline>> = split_row(header_line).iter().map(|cell| parse_inlines(cell)).collect();
    if header.len() != align.len() {
        return None;
    }
    let columns = align.len();
    let mut rows = Vec::new();
    let mut index = start + 2;
    while index < lines.len() {
        let line = lines[index];
        if is_blank(line) || interrupts_paragraph(line) {
            break;
        }
        let mut cells: Vec<Vec<Inline>> = split_row(line).iter().map(|cell| parse_inlines(cell)).collect();
        // Short rows are padded with empty cells; long rows are cut.
        cells.resize_with(columns, Vec::new);
        rows.push(cells);
        index += 1;
    }
    Some((Table { align, header, rows }, index))
}

/// The alignment a delimiter cell like `:---:` sets, or `None` when the cell
/// is not one.
pub(super) fn delimiter_align(cell: &str) -> Option<ColumnAlign> {
    let left = cell.starts_with(':');
    let right = cell.ends_with(':') && cell.len() > 1;
    let dashes = cell.trim_start_matches(':').trim_end_matches(':');
    if dashes.is_empty() || !dashes.bytes().all(|byte| byte == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => ColumnAlign::Center,
        (false, true) => ColumnAlign::Right,
        _ => ColumnAlign::Left,
    })
}

/// The cells of a table row: split at pipes that are not escaped, the
/// optional pipes at both ends dropped, every cell trimmed. `\|` is a pipe.
fn split_row(line: &str) -> Vec<String> {
    cell_spans(line).into_iter().map(|span| line[span].replace("\\|", "|").trim().to_string()).collect()
}

/// Byte spans in `line` of a row's cells as written (spaces and `\|`
/// escapes kept): between pipes that are not escaped, the optional pipes at
/// both ends dropped. The table editor replaces one cell through these.
pub(super) fn cell_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut index = 0;
    // Pipes and backslashes are ASCII, so byte positions are char boundaries.
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'|') => index += 2,
            b'|' => {
                spans.push(start..index);
                start = index + 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    spans.push(start..bytes.len());
    let is_blank_span = |span: &std::ops::Range<usize>| line[span.clone()].trim().is_empty();
    // A leading pipe leaves an empty first cell; a trailing one an empty last cell.
    if spans.first().is_some_and(is_blank_span) && line.trim().starts_with('|') {
        spans.remove(0);
    }
    if spans.len() > 1 && spans.last().is_some_and(is_blank_span) && line.trim().ends_with('|') {
        spans.pop();
    }
    spans
}

// ---------- code ----------

/// An opening code fence like ```` ```rust ```` or `~~~`.
struct Fence {
    /// Indentation of the opening fence, removed from the code lines too.
    indent: usize,
    /// `` ` `` or `~`.
    mark: char,
    /// Length of the opening run; the closing run must be at least as long.
    length: usize,
    /// Text after the fence, e.g. the language name.
    info: String,
}

impl Fence {
    /// Recognize an opening fence line.
    fn open(line: &str) -> Option<Fence> {
        let indent = indentation(line);
        let trimmed = &line[indent..];
        let mark = trimmed.chars().next().filter(|&c| c == '`' || c == '~')?;
        let length = trimmed.chars().take_while(|&c| c == mark).count();
        let info = trimmed[length..].trim();
        if indent >= CODE_INDENT || length < 3 || (mark == '`' && info.contains('`')) {
            return None;
        }
        Some(Fence { indent, mark, length, info: info.to_string() })
    }

    /// True when `line` is a closing fence for this opening fence.
    fn is_closed_by(&self, line: &str) -> bool {
        let trimmed = line.trim();
        indentation(line) < CODE_INDENT && trimmed.len() >= self.length && trimmed.chars().all(|c| c == self.mark)
    }
}

/// Code lines up to the closing fence (or the end of the document).
fn parse_fenced_code(lines: &[&str], start: usize, fence: Fence) -> (Option<Block>, usize) {
    let mut code_lines = Vec::new();
    let mut index = start + 1;
    // An unclosed fence runs to the end of the document.
    while index < lines.len() && !fence.is_closed_by(lines[index]) {
        code_lines.push(strip_indentation(lines[index], fence.indent));
        index += 1;
    }
    let code = code_lines.join("\n");
    (Some(Block::Code { info: fence.info, code }), (index + 1).min(lines.len()))
}

/// Consecutive indented (or blank) lines, without trailing blank lines.
fn parse_indented_code(lines: &[&str], start: usize) -> (Option<Block>, usize) {
    let mut code_lines = Vec::new();
    let mut index = start;
    while index < lines.len() && (is_blank(lines[index]) || indentation(lines[index]) >= CODE_INDENT) {
        code_lines.push(strip_indentation(lines[index], CODE_INDENT));
        index += 1;
    }
    while code_lines.last().is_some_and(|line| line.trim().is_empty()) {
        code_lines.pop();
    }
    (Some(Block::Code { info: String::new(), code: code_lines.join("\n") }), index)
}

// ---------- HTML blocks ----------

/// Tags that start an HTML block (CommonMark's list, plus `<picture>`,
/// common around README logos). Such a block runs to a blank line.
const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "caption",
    "center",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "picture",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];

/// Tags whose content may hold blank lines: their HTML block runs to the
/// line with the end tag.
const HTML_RAW_TAGS: &[&str] = &["pre", "script", "style", "textarea"];

/// How a line starts an HTML block, which decides where the block ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HtmlStart {
    /// `<pre>`, `<script>` and the like: up to the line with the end tag.
    Raw(&'static str),
    /// A block tag such as `<div>` or `</p>`: up to a blank line.
    BlockTag,
    /// A line that is only one tag of any other name (`<img src=...>`): up
    /// to a blank line. It does not end a running paragraph.
    LoneTag,
}

impl HtmlStart {
    /// How `trimmed` (a line without its indentation) starts an HTML block,
    /// if it does.
    fn of(trimmed: &str) -> Option<Self> {
        let after = trimmed.strip_prefix('<')?;
        let closing = after.starts_with('/');
        let after = after.strip_prefix('/').unwrap_or(after);
        let name_length = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(after.len());
        let name = after[..name_length].to_ascii_lowercase();
        let rest = &after[name_length..];
        // A tag name starts with a letter and ends at a space, `>` or `/`
        // (so `<https://...>` is an autolink, not a tag).
        let name_ends = rest.is_empty() || rest.starts_with([' ', '\t', '>', '/']);
        if !name.starts_with(|c: char| c.is_ascii_alphabetic()) || !name_ends {
            return None;
        }
        if let Some(raw) = HTML_RAW_TAGS.iter().find(|raw| **raw == name).filter(|_| !closing) {
            return Some(HtmlStart::Raw(raw));
        }
        if HTML_BLOCK_TAGS.contains(&name.as_str()) {
            return Some(HtmlStart::BlockTag);
        }
        // Only a whole tag on its own: its first `>` ends the line.
        let first_close = trimmed.find('>')?;
        (first_close + 1 == trimmed.trim_end().len()).then_some(HtmlStart::LoneTag)
    }
}

/// An HTML block from `lines[start]`, read with the HTML page reader. Its
/// lines are taken as they are, so nothing in them is lost when the block
/// is edited.
fn parse_html_block(lines: &[&str], start: usize, kind: HtmlStart) -> (Option<Block>, usize) {
    let mut end = start + 1;
    match kind {
        HtmlStart::Raw(name) => {
            let end_tag = format!("</{name}");
            let mut index = start;
            while index < lines.len() {
                end = index + 1;
                if lines[index].to_ascii_lowercase().contains(&end_tag) {
                    break;
                }
                index += 1;
            }
        }
        HtmlStart::BlockTag | HtmlStart::LoneTag => {
            while end < lines.len() && !is_blank(lines[end]) {
                end += 1;
            }
        }
    }
    let html = lines[start..end].join("\n");
    (Some(Block::Html(super::html_blocks::parse_html_blocks(&html))), end)
}

// ---------- block quotes ----------

/// Lines starting with `>` (plus lazy continuation lines), parsed as blocks.
fn parse_quote(lines: &[&str], start: usize, depth: usize) -> (Option<Block>, usize) {
    let mut inner: Vec<&str> = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('>').filter(|_| indentation(line) < CODE_INDENT) {
            inner.push(rest.strip_prefix(' ').unwrap_or(rest));
        } else if is_lazy_continuation(line, inner.last()) {
            // "Lazy" line: paragraph text continuing without the `>`.
            inner.push(trimmed);
        } else {
            break;
        }
        index += 1;
    }
    (Some(Block::Quote(parse_lines(&inner, depth + 1))), index)
}

/// A non-blank line continuing the paragraph on the previous (non-blank) line.
fn is_lazy_continuation(line: &str, previous: Option<&&str>) -> bool {
    let previous_has_text = previous.is_some_and(|previous| !is_blank(previous));
    previous_has_text && !is_blank(line) && !interrupts_paragraph(line)
}

// ---------- lists ----------

/// The `- ` or `3. ` that starts a list item.
#[derive(Clone, Copy)]
struct ListMarker {
    /// `None` for bullets, the item number for ordered lists.
    number: Option<u64>,
    /// `-`, `*`, `+` for bullets; `.` or `)` for ordered lists.
    mark: char,
    /// Column where the item's content starts; continuation lines must be
    /// indented at least this far.
    content_column: usize,
    /// Text follows the marker on the same line.
    has_content: bool,
}

impl ListMarker {
    /// Recognize a list marker at the start of `line`.
    fn parse(line: &str) -> Option<ListMarker> {
        let indent = indentation(line);
        if indent >= CODE_INDENT {
            return None;
        }
        let rest = &line[indent..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let (number, mark) = if digits == 0 {
            (None, rest.chars().next().filter(|c| matches!(c, '-' | '*' | '+'))?)
        } else {
            if digits > MAX_LIST_NUMBER_DIGITS {
                return None;
            }
            let mark = rest[digits..].chars().next().filter(|&c| c == '.' || c == ')')?;
            (Some(rest[..digits].parse::<u64>().ok()?), mark)
        };
        let marker_width = digits.max(1) + usize::from(digits > 0);
        let after_marker = &rest[marker_width..];
        if !(after_marker.is_empty() || after_marker.starts_with(' ')) {
            return None;
        }
        let spaces = after_marker.chars().take_while(|&c| c == ' ').count();
        let has_content = !after_marker.trim().is_empty();
        // Empty item or 5+ spaces (indented code inside the item): content
        // starts one space after the marker.
        let gap = if !has_content || spaces > CODE_INDENT { 1 } else { spaces };
        Some(ListMarker { number, mark, content_column: indent + marker_width + gap, has_content })
    }

    /// Items of one list share the bullet character or ordered delimiter.
    fn continues(&self, other: &ListMarker) -> bool {
        self.number.is_some() == other.number.is_some() && self.mark == other.mark
    }
}

/// Consecutive items with matching markers.
fn parse_list(lines: &[&str], start: usize, first_marker: ListMarker, depth: usize) -> (Option<Block>, usize) {
    let mut items = Vec::new();
    let mut marker = first_marker;
    let mut index = start;
    loop {
        let (item, next) = parse_list_item(lines, index, marker, depth);
        items.push(item);
        index = next;
        let next_marker = lines
            .get(index)
            .filter(|line| !is_thematic_break(line.trim_start()))
            .and_then(|line| ListMarker::parse(line))
            .filter(|next| next.continues(&first_marker));
        match next_marker {
            Some(next) => marker = next,
            None => break,
        }
    }
    let list = List { start: first_marker.number, items };
    (Some(Block::List(list)), index)
}

/// One item: its first line plus indented, blank, and lazy lines below it.
/// The item's blocks are one container deeper than its list.
fn parse_list_item(lines: &[&str], start: usize, marker: ListMarker, depth: usize) -> (ListItem, usize) {
    let first_line = lines[start].get(marker.content_column..).unwrap_or("");
    let mut item_lines = vec![first_line];
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        if is_blank(line) {
            item_lines.push("");
        } else if indentation(line) >= marker.content_column {
            item_lines.push(&line[marker.content_column..]);
        } else if ListMarker::parse(line).is_some_and(|next| next.continues(&marker)) {
            // A sibling item always ends this one, even `2.` which could
            // not interrupt a plain paragraph.
            break;
        } else if is_lazy_continuation(line, item_lines.last()) {
            item_lines.push(line.trim_start());
        } else {
            break;
        }
        index += 1;
    }
    while item_lines.len() > 1 && item_lines.last().is_some_and(|line| is_blank(line)) {
        item_lines.pop();
    }
    let (task, first_line) = split_task_box(item_lines[0]);
    item_lines[0] = first_line;
    (ListItem { task, blocks: parse_lines(&item_lines, depth + 1) }, index)
}

/// `[ ] text` -> (Some(false), "text"); `[x] text` -> (Some(true), "text").
fn split_task_box(line: &str) -> (Option<bool>, &str) {
    for (prefix, checked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(rest) = line.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with(' '))
        {
            return (Some(checked), rest.trim_start());
        }
    }
    (None, line)
}

// ---------- whitespace helpers ----------

/// Empty or only whitespace.
fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Number of leading spaces (tabs are already expanded).
fn indentation(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Remove up to `count` leading spaces.
fn strip_indentation(line: &str, count: usize) -> &str {
    let remove = indentation(line).min(count);
    &line[remove..]
}

/// Replace leading tabs with spaces up to the next tab stop.
fn expand_leading_tabs(line: &str) -> String {
    let mut expanded = String::new();
    let mut rest = line;
    while let Some(character) = rest.chars().next() {
        match character {
            ' ' => expanded.push(' '),
            '\t' => {
                let width = TAB_WIDTH - expanded.len() % TAB_WIDTH;
                expanded.extend(std::iter::repeat_n(' ', width));
            }
            _ => break,
        }
        rest = &rest[1..];
    }
    expanded.push_str(rest);
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str) -> Block {
        Block::Paragraph(parse_inlines(text))
    }

    fn heading(level: u8, text: &str) -> Block {
        Block::Heading { level, content: parse_inlines(text) }
    }

    fn code(info: &str, code: &str) -> Block {
        Block::Code { info: info.into(), code: code.into() }
    }

    fn item(blocks: Vec<Block>) -> ListItem {
        ListItem { task: None, blocks }
    }

    #[test]
    fn atx_headings() {
        assert_eq!(
            parse_blocks("# One\n###### Six ##\n####### seven\n#nospace"),
            vec![heading(1, "One"), heading(6, "Six"), paragraph("####### seven\n#nospace")]
        );
    }

    #[test]
    fn setext_headings() {
        assert_eq!(parse_blocks("Title\n=====\nSub\n---"), vec![heading(1, "Title"), heading(2, "Sub")]);
    }

    #[test]
    fn paragraphs_split_on_blank_lines() {
        assert_eq!(parse_blocks("a\nb\n\n\nc"), vec![paragraph("a\nb"), paragraph("c")]);
    }

    #[test]
    fn thematic_breaks() {
        assert_eq!(parse_blocks("***\n- - -\n___"), vec![Block::ThematicBreak; 3]);
    }

    #[test]
    fn fenced_code_keeps_content_verbatim() {
        assert_eq!(
            parse_blocks("```rust\nfn a() {}\n\n  # not heading\n```\nafter"),
            vec![code("rust", "fn a() {}\n\n  # not heading"), paragraph("after")]
        );
        assert_eq!(parse_blocks("~~~\nopen to end"), vec![code("", "open to end")]);
    }

    #[test]
    fn indented_code() {
        assert_eq!(
            parse_blocks("    let x = 1;\n\n    let y = 2;\n\ntext"),
            vec![code("", "let x = 1;\n\nlet y = 2;"), paragraph("text"),]
        );
    }

    #[test]
    fn indented_line_inside_paragraph_is_not_code() {
        assert_eq!(parse_blocks("a\n    b"), vec![paragraph("a\nb")]);
    }

    #[test]
    fn block_quote_with_nested_blocks_and_lazy_line() {
        assert_eq!(
            parse_blocks("> # Hi\n> text\nlazy\n>\n> > deep"),
            vec![Block::Quote(vec![heading(1, "Hi"), paragraph("text\nlazy"), Block::Quote(vec![paragraph("deep")]),])]
        );
    }

    #[test]
    fn bullet_list_with_nesting() {
        assert_eq!(
            parse_blocks("- a\n- b\n  - c\n  - d\n- e"),
            vec![Block::List(List {
                start: None,
                items: vec![
                    item(vec![paragraph("a")]),
                    item(vec![
                        paragraph("b"),
                        Block::List(List {
                            start: None,
                            items: vec![item(vec![paragraph("c")]), item(vec![paragraph("d")])],
                        }),
                    ]),
                    item(vec![paragraph("e")]),
                ],
            })]
        );
    }

    #[test]
    fn ordered_list_keeps_start_number() {
        let blocks = parse_blocks("3. three\n4. four");
        let Block::List(list) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(list.start, Some(3));
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn different_bullet_starts_new_list() {
        assert_eq!(parse_blocks("- a\n* b").len(), 2);
    }

    #[test]
    fn task_items() {
        let blocks = parse_blocks("- [ ] todo\n- [x] done\n- [link](u)");
        let Block::List(list) = &blocks[0] else { panic!("{blocks:?}") };
        let tasks: Vec<Option<bool>> = list.items.iter().map(|item| item.task).collect();
        assert_eq!(tasks, [Some(false), Some(true), None]);
        assert_eq!(list.items[0].blocks, vec![paragraph("todo")]);
    }

    #[test]
    fn list_item_with_blank_line_and_code() {
        let blocks = parse_blocks("1. step\n\n   ```\n   cmd\n   ```\n2. next");
        let Block::List(list) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(list.items[0].blocks, vec![paragraph("step"), code("", "cmd")]);
        assert_eq!(list.items.len(), 2);
    }

    fn cells(row: &[&str]) -> Vec<Vec<Inline>> {
        row.iter().map(|cell| parse_inlines(cell)).collect()
    }

    #[test]
    fn pipe_table_with_alignment() {
        let blocks = parse_blocks("| File | Size | Note |\n| :-- | --: | :-: |\n| a.md | 1 | *x* |\nb.md | 2\n");
        assert_eq!(
            blocks,
            vec![Block::Table(Table {
                align: vec![ColumnAlign::Left, ColumnAlign::Right, ColumnAlign::Center],
                header: cells(&["File", "Size", "Note"]),
                rows: vec![cells(&["a.md", "1", "*x*"]), cells(&["b.md", "2", ""])],
            })]
        );
    }

    #[test]
    fn table_ends_at_blank_line_or_block_and_keeps_escaped_pipes() {
        let blocks = parse_blocks("a | b\n--|--\n1 | x \\| y\n\nafter\n# H");
        let Block::Table(table) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(table.rows, vec![cells(&["1", "x | y"])]);
        assert_eq!(&blocks[1..], &[paragraph("after"), heading(1, "H")]);
    }

    #[test]
    fn table_can_follow_paragraph_text_directly() {
        let blocks = parse_blocks("Intro\n| a | b |\n| - | - |\n| 1 | 2 |");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], paragraph("Intro"));
        assert!(matches!(&blocks[1], Block::Table(table) if table.rows.len() == 1));
    }

    #[test]
    fn not_a_table_without_pipes_or_with_wrong_cell_count() {
        assert_eq!(parse_blocks("Title\n---"), vec![heading(2, "Title")]);
        assert_eq!(parse_blocks("a | b\n---"), vec![heading(2, "a | b")]);
        assert_eq!(parse_blocks("a | b\n| - |"), vec![paragraph("a | b\n| - |")]);
        assert_eq!(parse_blocks("a | b\n| -x- | - |"), vec![paragraph("a | b\n| -x- | - |")]);
    }

    #[test]
    fn number_without_marker_space_is_text() {
        assert_eq!(parse_blocks("2024.release"), vec![paragraph("2024.release")]);
        assert_eq!(parse_blocks("-not a list"), vec![paragraph("-not a list")]);
    }

    #[test]
    fn ordered_list_not_starting_at_one_does_not_interrupt_paragraph() {
        assert_eq!(parse_blocks("In 2024\n2. was good"), vec![paragraph("In 2024\n2. was good")]);
    }

    #[test]
    fn tabs_and_crlf() {
        assert_eq!(parse_blocks("\tcode\r\n\r\ntext\r\n"), vec![code("", "code"), paragraph("text")]);
    }

    #[test]
    fn empty_document() {
        assert_eq!(parse_blocks(""), vec![]);
        assert_eq!(parse_blocks("\n\n  \n"), vec![]);
    }

    #[test]
    fn partial_answer_still_parses() {
        // An AI answer cut off while it streams: open fence, list, link, bold.
        let text = "## Steps\n\n1. Run [it](https://e\n2. **Che\n\n```rust\nfn main() {\n    let x";
        assert_eq!(
            parse_blocks(text),
            vec![
                heading(2, "Steps"),
                Block::List(List {
                    start: Some(1),
                    items: vec![item(vec![paragraph("Run [it](https://e")]), item(vec![paragraph("**Che")])],
                }),
                code("rust", "fn main() {\n    let x"),
            ]
        );
        for end in 0..=text.len() {
            if text.is_char_boundary(end) {
                parse_blocks(&text[..end]);
            }
        }
    }

    #[test]
    fn source_blocks_know_their_text() {
        let text = "# Title\r\n\nFirst line\nsecond line\n\n- a\n- b\n\n```\ncode\n```\n";
        let blocks = parse_source_blocks(text);
        let sources: Vec<&str> = blocks.iter().map(|block| &text[block.range.clone()]).collect();
        assert_eq!(sources, ["# Title", "First line\nsecond line", "- a\n- b", "```\ncode\n```"]);
        assert_eq!(blocks.iter().map(|block| block.block.clone()).collect::<Vec<_>>(), parse_blocks(text));
    }

    /// The HTML block `source` should read as.
    fn html(source: &str) -> Block {
        Block::Html(crate::markdown::html_blocks::parse_html_blocks(source))
    }

    #[test]
    fn a_readme_header_is_one_html_block_up_to_the_blank_line() {
        let header =
            "<p align=\"center\">\n  <img src=\"logo.png\" width=\"120\">\n</p>\n<h1 align=\"center\">Tool</h1>";
        let blocks = parse_blocks(&format!("{header}\n\nSome *text*."));
        assert_eq!(blocks, [html(header), paragraph("Some *text*.")]);
        let Block::Html(inner) = &blocks[0] else { panic!("{blocks:?}") };
        assert!(matches!(&inner[0], Block::Aligned { align: ColumnAlign::Center, .. }));
        assert!(
            matches!(&inner[1], Block::Aligned { blocks, .. } if matches!(blocks[0], Block::Heading { level: 1, .. }))
        );
    }

    #[test]
    fn block_tags_end_a_paragraph_but_a_lone_tag_does_not() {
        assert_eq!(parse_blocks("text\n<div>box</div>"), [paragraph("text"), html("<div>box</div>")]);
        assert_eq!(parse_blocks("text\n<img src=a.png>"), [paragraph("text\n<img src=a.png>")]);
        assert_eq!(parse_blocks("<img src=a.png>\ntext"), [html("<img src=a.png>\ntext")]);
    }

    #[test]
    fn raw_text_blocks_run_to_their_end_tag_across_blank_lines() {
        assert_eq!(
            parse_blocks("<pre>\nline\n\nafter blank\n</pre>\nnext"),
            [html("<pre>\nline\n\nafter blank\n</pre>"), paragraph("next")]
        );
        // A script is read and dropped; nothing of it shows.
        assert_eq!(parse_blocks("<script>\nalert(1)\n\n</script>"), [Block::Html(Vec::new())]);
    }

    #[test]
    fn what_is_not_an_html_block() {
        for text in ["<https://example.com>", "<b>bold</b> text", "1 < 2", "<>", "    <div>indented</div>"] {
            assert!(!parse_blocks(text).iter().any(|block| matches!(block, Block::Html(_))), "{text}");
        }
    }

    #[test]
    fn html_blocks_keep_their_source_text_exactly() {
        let text = "# Title\n\n<details>\n<summary>More</summary>\n\nHidden **text**\n</details>\n\n\
                    <div align=\"center\">\r\n<b>x</b>\r\n</div>\n\nend\n";
        let blocks = parse_source_blocks(text);
        let sources: Vec<&str> = blocks.iter().map(|block| &text[block.range.clone()]).collect();
        assert_eq!(
            sources,
            [
                "# Title",
                "<details>\n<summary>More</summary>",
                "Hidden **text**",
                "</details>",
                "<div align=\"center\">\r\n<b>x</b>\r\n</div>",
                "end"
            ]
        );
        assert_eq!(blocks.iter().map(|block| block.block.clone()).collect::<Vec<_>>(), parse_blocks(text));
        // Every byte between and around the blocks is blank, so editing the
        // blocks one at a time loses nothing.
        let mut covered = 0;
        for block in &blocks {
            assert_eq!(text[covered..block.range.start].trim(), "");
            covered = block.range.end;
        }
        assert_eq!(text[covered..].trim(), "");
    }

    #[test]
    fn html_blocks_work_inside_quotes() {
        assert_eq!(parse_blocks("> <div>q</div>"), [Block::Quote(vec![html("<div>q</div>")])]);
    }

    #[test]
    fn deep_quotes_stay_text_past_the_limit() {
        let source = format!("{}deep text", ">".repeat(100_000));
        let mut blocks = parse_blocks(&source);
        let mut depth = 0;
        while let [Block::Quote(inner)] = blocks.as_slice() {
            depth += 1;
            blocks = inner.clone();
        }
        assert!(depth <= MAX_BLOCK_DEPTH + 1, "{depth}");
        let [Block::Paragraph(inlines)] = blocks.as_slice() else { panic!("{blocks:?}") };
        assert!(super::super::inlines::plain_text(inlines).ends_with(">deep text"));
    }

    #[test]
    fn deep_lists_stay_text_past_the_limit() {
        let source = format!("{}deep text", "- ".repeat(100_000));
        let mut blocks = parse_blocks(&source);
        let mut depth = 0;
        while let [Block::List(List { items, .. })] = blocks.as_slice()
            && let [ListItem { blocks: inner, .. }] = items.as_slice()
        {
            depth += 1;
            blocks = inner.clone();
        }
        assert!(depth <= MAX_BLOCK_DEPTH + 1, "{depth}");
        let [Block::Paragraph(inlines)] = blocks.as_slice() else { panic!("{blocks:?}") };
        assert!(super::super::inlines::plain_text(inlines).ends_with("- deep text"));
    }

    #[test]
    fn nesting_below_the_limit_is_unchanged() {
        let source = format!("{}text", "> ".repeat(MAX_BLOCK_DEPTH));
        let mut expected = paragraph("text");
        for _ in 0..MAX_BLOCK_DEPTH {
            expected = Block::Quote(vec![expected]);
        }
        assert_eq!(parse_blocks(&source), [expected]);
    }
}
