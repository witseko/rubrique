//! Read an HTML page into the preview's blocks, so an `.html` file can be
//! shown in the preview like a Markdown file.
//!
//! This is not a browser. It knows the tags that have a Markdown
//! counterpart (headings, paragraphs, bold, italic, links, images, lists,
//! quotes, code, tables and rules), a few that set the look of text
//! (`<mark>`, `<u>`, `<sup>`, `<sub>`, `<font color>`), `<details>` and
//! alignment (`align`, `<center>`), and keeps only the text of the others.
//! Of the CSS, only the text styles of `style` attributes are read (color,
//! background color, weight, italic, underline and strikethrough, text
//! alignment, image width and height); style sheets and scripts are
//! dropped and nothing runs or loads, so any file is safe to show. Broken
//! markup is read leniently, the way browsers do: unclosed paragraphs and
//! list items end where the next one starts, and stray end tags are
//! ignored.

use super::blocks::{Block, ColumnAlign, List, ListItem, SourceBlock, Table};
use super::css;
use super::inlines::{HtmlStyle, ImageSize, Inline, Script};
use crate::theme::palette::Rgb;

/// Deepest element nesting kept. Deeper tags are ignored (their text
/// stays), so a hostile file cannot overflow the stack while it is read or
/// drawn.
const MAX_DEPTH: usize = 64;

/// Longest entity name looked up (`&hellip;` is 6), so a stray `&` does
/// not scan far ahead for a `;`.
const MAX_ENTITY_LENGTH: usize = 10;

/// Most columns one table cell spans (`colspan`), so a hostile file cannot
/// make a table millions of cells wide.
const MAX_COLSPAN: usize = 64;

/// The highlighter yellow of `<mark>`, as browsers draw it.
const MARK_COLOR: Rgb = Rgb(0xff, 0xff, 0x00);

/// What `<details>` shows when it has no `<summary>`, as browsers do.
const DEFAULT_SUMMARY: &str = "Details";

/// Elements whose `align` places them (or that align through their
/// column), rather than aligning their text.
const NOT_TEXT_ALIGNED: &[&str] = &["caption", "table", "tbody", "td", "tfoot", "th", "thead", "tr"];

/// Elements that never have content or an end tag.
const VOID_ELEMENTS: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Elements whose content is raw text up to their end tag. None of it is
/// page text, so the tokenizer skips them whole.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "textarea", "title"];

/// Elements dropped with everything inside: page metadata, and widgets or
/// embedded pictures that have no text form.
const HIDDEN_ELEMENTS: &[&str] =
    &["button", "canvas", "datalist", "head", "iframe", "math", "object", "select", "svg", "template"];

/// Elements that start a block of their own. Headings, paragraphs, lists,
/// quotes, code, rules and tables become that block; the rest are
/// containers whose content becomes blocks.
const BLOCK_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
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
    "hgroup",
    "hr",
    "html",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
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

/// Parse an HTML page into blocks for the preview.
pub fn parse_html_blocks(html: &str) -> Vec<Block> {
    blocks_of(&build_tree(tokenize(html)))
}

/// The page's blocks for the preview cache. The HTML preview never edits a
/// block in place, so each block's range is simply the whole file.
pub fn html_source_blocks(html: &str) -> Vec<SourceBlock> {
    parse_html_blocks(html).into_iter().map(|block| SourceBlock { block, range: 0..html.len() }).collect()
}

// ---------- tokens ----------

/// One piece of the page: a tag or the text between tags (entities decoded).
#[derive(Debug, PartialEq)]
enum Token {
    Start { name: String, attributes: Vec<(String, String)>, self_closing: bool },
    End(String),
    Text(String),
}

/// A start or end tag read by `parse_tag`.
pub(super) struct Tag {
    /// Lowercase tag name.
    pub(super) name: String,
    /// Lowercase attribute names with their decoded values.
    pub(super) attributes: Vec<(String, String)>,
    /// Ends in `/>`.
    pub(super) self_closing: bool,
    /// Bytes from the name to the closing `>`, included.
    pub(super) length: usize,
}

/// Split the page into tags and text. Comments, doctypes and the content of
/// raw text elements (scripts, styles) are left out. A `<` that does not
/// start a tag is text.
fn tokenize(html: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut text_start = 0;
    let mut at = 0;
    // A `<` after the last `>` cannot start a complete tag, so the rest is
    // text. Stopping there keeps a page full of unclosed `<a` from being
    // scanned to its end once per `<`.
    let last_close = html.rfind('>').unwrap_or(0);
    while let Some(offset) = html[at..].find('<') {
        let open = at + offset;
        if open > last_close {
            break;
        }
        let Some((token, end)) = markup_at(html, open) else {
            at = open + 1;
            continue;
        };
        push_text(&mut tokens, &html[text_start..open]);
        tokens.extend(token);
        at = end;
        text_start = end;
    }
    push_text(&mut tokens, &html[text_start..]);
    tokens
}

/// Read the markup starting at the `<` at `open`. Returns the token (none
/// for comments and skipped elements) and where the markup ends, or `None`
/// when the `<` is plain text.
fn markup_at(html: &str, open: usize) -> Option<(Option<Token>, usize)> {
    let rest = &html[open..];
    if let Some(comment) = rest.strip_prefix("<!--") {
        let end = comment.find("-->").map_or(html.len(), |end| open + "<!--".len() + end + "-->".len());
        return Some((None, end));
    }
    if rest.starts_with("<!") || rest.starts_with("<?") {
        let end = rest.find('>').map_or(html.len(), |end| open + end + 1);
        return Some((None, end));
    }
    let closing = rest.starts_with("</");
    let name_start = if closing { 2 } else { 1 };
    if !rest[name_start..].starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let tag = parse_tag(&rest[name_start..])?;
    let end = open + name_start + tag.length;
    if closing {
        return Some((Some(Token::End(tag.name)), end));
    }
    if RAW_TEXT_ELEMENTS.contains(&tag.name.as_str()) {
        return Some((None, skip_raw_text(html, end, &tag.name)));
    }
    let token = Token::Start { name: tag.name, attributes: tag.attributes, self_closing: tag.self_closing };
    Some((Some(token), end))
}

/// Read a tag from its name to its `>`. `None` when the page ends first.
fn parse_tag(text: &str) -> Option<Tag> {
    let bytes = text.as_bytes();
    let name_end = find_byte(bytes, 0, |byte| byte.is_ascii_whitespace() || byte == b'>' || byte == b'/');
    let name = text[..name_end].to_ascii_lowercase();
    let mut attributes = Vec::new();
    let mut self_closing = false;
    let mut at = name_end;
    loop {
        at = skip_spaces(bytes, at);
        match *bytes.get(at)? {
            b'>' => return Some(Tag { name, attributes, self_closing, length: at + 1 }),
            b'/' => {
                self_closing = true;
                at += 1;
                continue;
            }
            _ => self_closing = false,
        }
        // Every byte here is part of a name or an `=`, so `at` always moves on.
        let attribute_end =
            find_byte(bytes, at, |byte| byte.is_ascii_whitespace() || matches!(byte, b'=' | b'>' | b'/'));
        let attribute = text[at..attribute_end].to_ascii_lowercase();
        at = skip_spaces(bytes, attribute_end);
        let mut value = String::new();
        if bytes.get(at) == Some(&b'=') {
            let (raw, next) = attribute_value(text, skip_spaces(bytes, at + 1))?;
            value = decode_entities(raw);
            at = next;
        }
        attributes.push((attribute, value));
    }
}

/// An attribute value starting at `at`, quoted or not, and where it ends.
fn attribute_value(text: &str, at: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    let first = *bytes.get(at)?;
    if first == b'"' || first == b'\'' {
        let length = text[at + 1..].find(char::from(first))?;
        return Some((&text[at + 1..at + 1 + length], at + length + 2));
    }
    let end = find_byte(bytes, at, |byte| byte.is_ascii_whitespace() || byte == b'>');
    Some((&text[at..end], end))
}

/// Where the raw text element `name`, whose start tag ends at `from`, ends:
/// just past its end tag, or the end of the page.
fn skip_raw_text(html: &str, from: usize, name: &str) -> usize {
    let mut at = from;
    while let Some(offset) = html[at..].find("</") {
        let name_start = at + offset + 2;
        let candidate = html.as_bytes().get(name_start..name_start + name.len());
        if candidate.is_some_and(|candidate| candidate.eq_ignore_ascii_case(name.as_bytes())) {
            return html[name_start..].find('>').map_or(html.len(), |end| name_start + end + 1);
        }
        at = name_start;
    }
    html.len()
}

/// Index of the first byte from `from` on that matches, or the length.
fn find_byte(bytes: &[u8], from: usize, matches: impl Fn(u8) -> bool) -> usize {
    bytes[from..].iter().position(|&byte| matches(byte)).map_or(bytes.len(), |offset| from + offset)
}

/// Index of the first byte from `at` on that is not whitespace.
fn skip_spaces(bytes: &[u8], at: usize) -> usize {
    find_byte(bytes, at, |byte| !byte.is_ascii_whitespace())
}

/// Add the text between two tags, if any.
fn push_text(tokens: &mut Vec<Token>, text: &str) {
    if !text.is_empty() {
        tokens.push(Token::Text(decode_entities(text)));
    }
}

/// Replace character references (`&amp;`, `&#233;`, `&#xE9;` and the common
/// named ones) with their characters. Unknown ones stay as written.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        // Only the next few bytes can hold the `;`, so text full of `&`
        // is still read in one pass.
        let decoded = after
            .bytes()
            .take(MAX_ENTITY_LENGTH + 1)
            .position(|byte| byte == b';')
            .and_then(|end| entity_char(&after[..end]).map(|character| (character, end)));
        if let Some((character, end)) = decoded {
            out.push(character);
            rest = &after[end + 1..];
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The character an entity name (the part between `&` and `;`) stands for.
fn entity_char(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => number.parse().ok(),
        }?;
        return char::from_u32(code).filter(|&character| character != '\0');
    }
    let character = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "deg" => '\u{b0}',
        "middot" => '\u{b7}',
        "times" => '\u{d7}',
        "laquo" => '\u{ab}',
        "raquo" => '\u{bb}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "bull" => '\u{2022}',
        "hellip" => '\u{2026}',
        "euro" => '\u{20ac}',
        _ => return None,
    };
    Some(character)
}

// ---------- element tree ----------

/// A node of the element tree: an element or text (entities decoded,
/// whitespace as written).
#[derive(Debug)]
enum Node {
    Element(Element),
    Text(String),
}

/// An element with its attributes and content.
#[derive(Debug)]
struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    /// The value of attribute `name` (lowercase), if the element has it.
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }
}

/// Nest the tokens into a tree, closing what the page leaves open. Returns
/// the top-level nodes.
fn build_tree(tokens: Vec<Token>) -> Vec<Node> {
    // The root is never popped: every index used below is at least 1.
    let mut stack = vec![Element { name: String::new(), attributes: Vec::new(), children: Vec::new() }];
    for token in tokens {
        match token {
            Token::Text(text) => current(&mut stack).children.push(Node::Text(text)),
            Token::Start { name, attributes, self_closing } => {
                open_element(&mut stack, Element { name, attributes, children: Vec::new() }, self_closing);
            }
            Token::End(name) => {
                // A stray end tag (nothing of that name open) is ignored.
                if let Some(index) = stack.iter().rposition(|open| open.name == name).filter(|&index| index > 0) {
                    pop_through(&mut stack, index);
                }
            }
        }
    }
    pop_through(&mut stack, 1);
    stack.pop().map(|root| root.children).unwrap_or_default()
}

/// The innermost open element.
fn current(stack: &mut [Element]) -> &mut Element {
    stack.last_mut().expect("the root element is never popped")
}

/// Add a start tag's element to the tree: void and self-closing ones as an
/// empty child, the rest as the new innermost open element.
fn open_element(stack: &mut Vec<Element>, element: Element, self_closing: bool) {
    close_implied(stack, &element.name);
    if self_closing || VOID_ELEMENTS.contains(&element.name.as_str()) {
        current(stack).children.push(Node::Element(element));
    } else if stack.len() <= MAX_DEPTH {
        stack.push(element);
    }
    // Deeper than MAX_DEPTH the tag is dropped; its text still lands in
    // the innermost element kept.
}

/// Close the elements a new `name` tag ends without an end tag: an open
/// paragraph before any block, the previous list item, row or cell, and an
/// open `<head>` when the body starts.
fn close_implied(stack: &mut Vec<Element>, name: &str) {
    if BLOCK_ELEMENTS.contains(&name)
        && let Some(index) = open_paragraph(stack)
    {
        pop_through(stack, index);
    }
    match name {
        "li" => close_open(stack, &["li"], &["ul", "ol", "menu"]),
        "dt" | "dd" => close_open(stack, &["dt", "dd"], &["dl"]),
        "tr" => close_open(stack, &["tr"], &["table"]),
        "td" | "th" => close_open(stack, &["td", "th"], &["tr", "table"]),
        "thead" | "tbody" | "tfoot" => close_open(stack, &["thead", "tbody", "tfoot"], &["table"]),
        "body" => close_open(stack, &["head"], &[]),
        _ => {}
    }
}

/// Index of an open `<p>` with only inline elements inside it, if any.
fn open_paragraph(stack: &[Element]) -> Option<usize> {
    for index in (1..stack.len()).rev() {
        let name = stack[index].name.as_str();
        if name == "p" {
            return Some(index);
        }
        if BLOCK_ELEMENTS.contains(&name) {
            return None;
        }
    }
    None
}

/// Close the innermost open element named in `targets`, unless one named
/// in `boundaries` is open inside it.
fn close_open(stack: &mut Vec<Element>, targets: &[&str], boundaries: &[&str]) {
    for index in (1..stack.len()).rev() {
        let name = stack[index].name.as_str();
        if targets.contains(&name) {
            pop_through(stack, index);
            return;
        }
        if boundaries.contains(&name) {
            return;
        }
    }
}

/// Close the open element at `index` and everything inside it, adding each
/// to its parent. `index` is at least 1, so the root stays.
fn pop_through(stack: &mut Vec<Element>, index: usize) {
    while stack.len() > index.max(1) {
        let Some(element) = stack.pop() else {
            return;
        };
        current(stack).children.push(Node::Element(element));
    }
}

// ---------- blocks ----------

/// The blocks for a run of nodes. Inline content between blocks (text
/// straight inside a `<div>`, say) becomes a paragraph of its own.
fn blocks_of(nodes: &[Node]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut loose = Vec::new();
    for node in nodes {
        match node {
            Node::Element(element) if BLOCK_ELEMENTS.contains(&element.name.as_str()) => {
                push_paragraph(&mut blocks, std::mem::take(&mut loose));
                push_element_blocks(&mut blocks, element);
            }
            _ => push_inlines(&mut loose, node),
        }
    }
    push_paragraph(&mut blocks, loose);
    blocks
}

/// Add a paragraph of `inlines`, unless it is only whitespace.
fn push_paragraph(blocks: &mut Vec<Block>, inlines: Vec<Inline>) {
    let inlines = tidy(inlines);
    if !inlines.is_empty() {
        blocks.push(Block::Paragraph(inlines));
    }
}

/// Add the blocks for one block element, with the text style and the
/// alignment it sets.
fn push_element_blocks(blocks: &mut Vec<Block>, element: &Element) {
    let mut own = Vec::new();
    push_element_content(&mut own, element);
    let mut look = TextLook::of(element);
    // A block's background is a box behind it, which the preview does not
    // draw; tinting each run of its text instead would read as highlighting.
    look.html.background = None;
    if !look.is_plain() {
        for block in &mut own {
            look.apply_to_block(block);
        }
    }
    match block_align(element) {
        Some(align) if !own.is_empty() => blocks.push(Block::Aligned { align, blocks: own }),
        _ => blocks.extend(own),
    }
}

/// The alignment a block element sets for its text: `text-align` in its
/// style, else its `align` attribute; `<center>` centers.
fn block_align(element: &Element) -> Option<ColumnAlign> {
    if NOT_TEXT_ALIGNED.contains(&element.name.as_str()) {
        return None;
    }
    let from_style = element.attribute("style").and_then(|style| css::property(style, "text-align"));
    let from_attribute = element.attribute("align");
    let center = (element.name == "center").then_some(ColumnAlign::Center);
    from_style.and_then(css::parse_align).or_else(|| from_attribute.and_then(css::parse_align)).or(center)
}

/// Add the blocks for what one block element holds, before its style.
fn push_element_content(blocks: &mut Vec<Block>, element: &Element) {
    let children = &element.children;
    match element.name.as_str() {
        "p" => push_paragraph(blocks, inlines_of(children)),
        "ul" | "menu" => push_list(blocks, element, None),
        "ol" => {
            let start = element.attribute("start").and_then(|start| start.trim().parse().ok()).unwrap_or(1);
            push_list(blocks, element, Some(start));
        }
        "blockquote" => {
            let inner = blocks_of(children);
            if !inner.is_empty() {
                blocks.push(Block::Quote(inner));
            }
        }
        "pre" => blocks.push(code_block(element)),
        "hr" => blocks.push(Block::ThematicBreak),
        "table" => push_table(blocks, element),
        "details" => push_details(blocks, element),
        name => match heading_level(name) {
            Some(level) => {
                let content = tidy(inlines_of(children));
                if !content.is_empty() {
                    blocks.push(Block::Heading { level, content });
                }
            }
            // A container: its content as blocks.
            None => blocks.extend(blocks_of(children)),
        },
    }
}

/// Add a `<details>`: its first `<summary>` (or "Details") over the rest
/// of its content, which shows once it is opened.
fn push_details(blocks: &mut Vec<Block>, details: &Element) {
    let children = &details.children;
    let summary_at =
        children.iter().position(|node| matches!(node, Node::Element(element) if element.name == "summary"));
    let (summary, inner) = match summary_at {
        Some(index) => {
            let summary = match &children[index] {
                Node::Element(summary) => tidy(inlines_of(&summary.children)),
                Node::Text(_) => Vec::new(),
            };
            let mut inner = blocks_of(&children[..index]);
            inner.extend(blocks_of(&children[index + 1..]));
            (summary, inner)
        }
        None => (Vec::new(), blocks_of(children)),
    };
    let summary = if summary.is_empty() { vec![Inline::Text(DEFAULT_SUMMARY.to_string())] } else { summary };
    blocks.push(Block::Details { summary, open: details.attribute("open").is_some(), blocks: inner });
}

/// 1 to 6 for `h1` to `h6`.
fn heading_level(name: &str) -> Option<u8> {
    let level = name.strip_prefix('h')?.parse().ok()?;
    (1..=6).contains(&level).then_some(level)
}

/// Add a list (`start` is the first number of an ordered list) when it has
/// items. Content outside any `<li>` becomes an item of its own.
fn push_list(blocks: &mut Vec<Block>, list: &Element, start: Option<u64>) {
    let mut items = Vec::new();
    for node in &list.children {
        match node {
            Node::Element(item) if item.name == "li" => items.push(list_item(item)),
            other => {
                let blocks = blocks_of(std::slice::from_ref(other));
                if !blocks.is_empty() {
                    items.push(ListItem { task: None, blocks });
                }
            }
        }
    }
    if !items.is_empty() {
        blocks.push(Block::List(List { start, items }));
    }
}

/// A list item; one that starts with a checkbox is a task.
fn list_item(item: &Element) -> ListItem {
    let first = item.children.iter().position(|node| !is_blank_text(node));
    let checkbox = first.filter(|&index| matches!(&item.children[index], Node::Element(input) if is_checkbox(input)));
    let task = checkbox.map(|index| match &item.children[index] {
        Node::Element(input) => input.attribute("checked").is_some(),
        Node::Text(_) => false,
    });
    let rest = checkbox.map_or(&item.children[..], |index| &item.children[index + 1..]);
    ListItem { task, blocks: blocks_of(rest) }
}

/// True for `<input type="checkbox">`.
fn is_checkbox(element: &Element) -> bool {
    element.name == "input" && element.attribute("type").is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
}

/// True for text that is only whitespace.
fn is_blank_text(node: &Node) -> bool {
    matches!(node, Node::Text(text) if text.trim().is_empty())
}

/// A `<pre>` as a code block, its language from a `language-x` (or
/// `lang-x`) class on the `<pre>` or the `<code>` inside it.
fn code_block(pre: &Element) -> Block {
    let text = text_content(&pre.children);
    // A line break right after `<pre>` is not part of the content.
    let text = text.strip_prefix("\r\n").or_else(|| text.strip_prefix('\n')).unwrap_or(&text);
    let code = text.trim_end_matches(['\n', '\r']).to_string();
    let inner_code = pre.children.iter().find_map(|node| match node {
        Node::Element(element) if element.name == "code" => Some(element),
        _ => None,
    });
    let info = code_language(pre).or_else(|| inner_code.and_then(code_language)).unwrap_or_default();
    Block::Code { info, code }
}

/// The language named by an element's `language-x` or `lang-x` class.
fn code_language(element: &Element) -> Option<String> {
    element
        .attribute("class")?
        .split_whitespace()
        .find_map(|class| class.strip_prefix("language-").or_else(|| class.strip_prefix("lang-")))
        .map(str::to_string)
}

/// Add a table (after its caption, as a paragraph). The first row is the
/// header; short rows are padded with empty cells.
fn push_table(blocks: &mut Vec<Block>, table: &Element) {
    for node in &table.children {
        if let Node::Element(caption) = node
            && caption.name == "caption"
        {
            push_paragraph(blocks, inlines_of(&caption.children));
        }
    }
    let mut rows = Vec::new();
    collect_rows(&table.children, &mut rows);
    let rows: Vec<Vec<Option<&Element>>> = rows.into_iter().map(row_cells).filter(|cells| !cells.is_empty()).collect();
    let Some((header, body)) = rows.split_first() else {
        return;
    };
    let columns = rows.iter().map(Vec::len).max().unwrap_or_default();
    let align = (0..columns)
        .map(|column| header.get(column).copied().flatten().map_or(ColumnAlign::Left, cell_align))
        .collect();
    blocks.push(Block::Table(Table {
        align,
        header: row_content(header, columns),
        rows: body.iter().map(|row| row_content(row, columns)).collect(),
    }));
}

/// The `<tr>` elements of a table, inside `<thead>`, `<tbody>` and
/// `<tfoot>` too (but not in tables nested in cells).
fn collect_rows<'a>(nodes: &'a [Node], rows: &mut Vec<&'a Element>) {
    for node in nodes {
        let Node::Element(element) = node else {
            continue;
        };
        match element.name.as_str() {
            "tr" => rows.push(element),
            "thead" | "tbody" | "tfoot" => collect_rows(&element.children, rows),
            _ => {}
        }
    }
}

/// The `<td>` and `<th>` cells of a row, one entry per column: a cell that
/// spans several columns (`colspan`) is followed by `None` for each column
/// it covers, so the cells after it stay in their columns.
fn row_cells(row: &Element) -> Vec<Option<&Element>> {
    let mut cells = Vec::new();
    for node in &row.children {
        if let Node::Element(cell) = node
            && (cell.name == "td" || cell.name == "th")
        {
            let span = cell.attribute("colspan").and_then(|span| span.trim().parse().ok()).unwrap_or(1);
            cells.push(Some(cell));
            cells.extend(std::iter::repeat_n(None, span.clamp(1, MAX_COLSPAN) - 1));
        }
    }
    cells
}

/// The content of each cell of a row, padded to `columns` cells.
fn row_content(cells: &[Option<&Element>], columns: usize) -> Vec<Vec<Inline>> {
    let mut content: Vec<Vec<Inline>> = cells.iter().map(|cell| cell.map(cell_content).unwrap_or_default()).collect();
    content.resize_with(columns, Vec::new);
    content
}

/// A cell's inline content in the text style the cell sets. As for other
/// blocks, a cell's background is a box the preview does not draw.
fn cell_content(cell: &Element) -> Vec<Inline> {
    let mut look = TextLook::of(cell);
    look.html.background = None;
    look.apply(tidy(inlines_of(&cell.children)))
}

/// A header cell's alignment, from its `text-align` style or `align`
/// attribute.
fn cell_align(cell: &Element) -> ColumnAlign {
    let from_attribute = || cell.attribute("align").and_then(css::parse_align);
    cell.attribute("style")
        .and_then(|style| css::property(style, "text-align"))
        .and_then(css::parse_align)
        .or_else(from_attribute)
        .unwrap_or(ColumnAlign::Left)
}

/// All the text inside `nodes`, whitespace as written, with `<br>` as a
/// line break. Used for code, which keeps its spacing.
fn text_content(nodes: &[Node]) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            Node::Text(piece) => text.push_str(piece),
            Node::Element(element) if element.name == "br" => text.push('\n'),
            Node::Element(element) if HIDDEN_ELEMENTS.contains(&element.name.as_str()) => {}
            Node::Element(element) => text.push_str(&text_content(&element.children)),
        }
    }
    text
}

// ---------- inlines ----------

/// The inline content of `nodes`, before `tidy`.
fn inlines_of(nodes: &[Node]) -> Vec<Inline> {
    let mut inlines = Vec::new();
    for node in nodes {
        push_inlines(&mut inlines, node);
    }
    inlines
}

/// Add the inline content of one node, in the look its element sets.
fn push_inlines(out: &mut Vec<Inline>, node: &Node) {
    match node {
        Node::Text(text) => out.push(Inline::Text(collapse_whitespace(text))),
        Node::Element(element) => {
            let mut content = Vec::new();
            push_element_inlines(&mut content, element);
            out.extend(TextLook::of(element).apply(content));
        }
    }
}

/// Add the inline content of one element, before its look. Unknown
/// elements add their content; a block element met inside a paragraph or
/// cell adds its text, set off by spaces.
fn push_element_inlines(out: &mut Vec<Inline>, element: &Element) {
    let children = &element.children;
    match element.name.as_str() {
        "br" => out.push(Inline::LineBreak),
        "code" | "kbd" | "samp" | "tt" => out.push(Inline::Code(collapse_whitespace(&text_content(children)))),
        "img" => {
            if let Some(url) = non_empty(element.attribute("src")) {
                let alt = element.attribute("alt").unwrap_or_default().to_string();
                out.push(Inline::Image { alt, url, size: image_size(element) });
            }
        }
        // The preview shows a video for an image whose address names one.
        "video" => {
            if let Some(url) = media_source(element) {
                let alt = element.attribute("title").unwrap_or_default().to_string();
                out.push(Inline::Image { alt, url, size: ImageSize::default() });
            }
        }
        // A voice memo or other sound: a link to the file.
        "audio" => {
            if let Some(url) = media_source(element) {
                let label = non_empty(element.attribute("title")).unwrap_or_else(|| url.clone());
                out.push(Inline::Link { content: vec![Inline::Text(label)], url });
            }
        }
        name if HIDDEN_ELEMENTS.contains(&name) => {}
        name if BLOCK_ELEMENTS.contains(&name) => {
            out.push(Inline::Text(" ".to_string()));
            out.extend(inlines_of(children));
            out.push(Inline::Text(" ".to_string()));
        }
        _ => out.extend(wrap_in_tag(element, inlines_of(children))),
    }
}

/// `content` as the inline tag `element` shows it: bold, italic,
/// strikethrough or a link; other tags show it as it is.
fn wrap_in_tag(element: &Element, content: Vec<Inline>) -> Vec<Inline> {
    match element.name.as_str() {
        "strong" | "b" => vec![Inline::Strong(content)],
        "em" | "i" | "cite" | "dfn" | "var" => vec![Inline::Emphasis(content)],
        "s" | "del" | "strike" => vec![Inline::Strikethrough(content)],
        "a" => match non_empty(element.attribute("href")) {
            Some(url) => vec![Inline::Link { content, url }],
            None => content,
        },
        _ => content,
    }
}

/// A start tag met in Markdown text, read from just after its `<`. `None`
/// when the text does not start with a whole tag.
pub(super) fn parse_start_tag(text: &str) -> Option<Tag> {
    if !text.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    parse_tag(text)
}

/// An HTML tag met in Markdown text, as inlines in the tag's look.
/// `content` is the Markdown read between its start and end tags (none for
/// `<br>` and `<img>`).
pub(super) fn inline_tag(name: String, attributes: Vec<(String, String)>, content: Vec<Inline>) -> Vec<Inline> {
    let element = Element { name, attributes, children: Vec::new() };
    let inner = match element.name.as_str() {
        "code" | "kbd" | "samp" | "tt" => vec![Inline::Code(super::inlines::plain_text(&content))],
        "br" | "img" => {
            let mut inner = Vec::new();
            push_element_inlines(&mut inner, &element);
            inner
        }
        _ => wrap_in_tag(&element, content),
    };
    TextLook::of(&element).apply(inner)
}

/// The size an `<img>` asks for, from its `width` and `height` attributes
/// or the same properties in its style (the style wins, as in CSS).
fn image_size(image: &Element) -> ImageSize {
    let style = image.attribute("style").unwrap_or_default();
    let dimension = |name: &str| pixels(css::property(style, name)).or_else(|| pixels(image.attribute(name)));
    ImageSize { width: dimension("width"), height: dimension("height") }
}

/// A length in pixels, such as `200` or `200px`. Percentages, other units
/// and zero are `None`: the image then keeps its own size.
fn pixels(value: Option<&str>) -> Option<u16> {
    let value = value?.trim();
    let number = value.strip_suffix("px").unwrap_or(value).trim();
    number.parse().ok().filter(|&pixels| pixels > 0)
}

/// How an element sets its text: the Markdown styles it turns on and the
/// look only HTML has.
#[derive(Debug, Default)]
struct TextLook {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    html: HtmlStyle,
}

impl TextLook {
    /// The look an element's tag and its `style` and `color` attributes
    /// give its text. Tags with a Markdown style of their own (`<b>`,
    /// `<em>`) get it in `push_element_inlines` instead.
    fn of(element: &Element) -> Self {
        let mut look = TextLook::default();
        match element.name.as_str() {
            "mark" => look.html.background = Some(MARK_COLOR),
            "u" | "ins" => look.html.underline = true,
            "sup" => look.html.script = Some(Script::Superscript),
            "sub" => look.html.script = Some(Script::Subscript),
            "font" => look.html.color = element.attribute("color").and_then(css::parse_color),
            _ => {}
        }
        if let Some(style) = element.attribute("style") {
            look.read_style(style);
        }
        look
    }

    /// Take the text properties of a `style` attribute; the others (sizes,
    /// fonts, margins, borders) are not drawn.
    fn read_style(&mut self, style: &str) {
        for (property, value) in css::declarations(style) {
            let value = value.to_ascii_lowercase();
            match property.as_str() {
                "color" => self.html.color = css::parse_color(&value).or(self.html.color),
                // `background` may also hold a picture or a gradient: only a
                // plain color counts.
                "background-color" | "background" => {
                    self.html.background = css::parse_color(&value).or(self.html.background);
                }
                "font-weight" => self.bold = is_bold_weight(&value),
                "font-style" => self.italic = value == "italic" || value == "oblique",
                "text-decoration" | "text-decoration-line" => {
                    self.html.underline = value.contains("underline");
                    self.strikethrough = value.contains("line-through");
                }
                _ => {}
            }
        }
    }

    /// True when the look changes nothing.
    fn is_plain(&self) -> bool {
        !self.bold && !self.italic && !self.strikethrough && self.html.is_plain()
    }

    /// `inlines` in this look.
    fn apply(&self, inlines: Vec<Inline>) -> Vec<Inline> {
        if inlines.is_empty() || self.is_plain() {
            return inlines;
        }
        let mut inlines = inlines;
        if !self.html.is_plain() {
            inlines = vec![Inline::Styled { style: self.html, content: inlines }];
        }
        if self.strikethrough {
            inlines = vec![Inline::Strikethrough(inlines)];
        }
        if self.italic {
            inlines = vec![Inline::Emphasis(inlines)];
        }
        if self.bold {
            inlines = vec![Inline::Strong(inlines)];
        }
        inlines
    }

    /// Set all the text in `block` (nested blocks and table cells too) in
    /// this look, as a block element's style reaches the text inside it.
    fn apply_to_block(&self, block: &mut Block) {
        let restyle = |content: &mut Vec<Inline>| *content = self.apply(std::mem::take(content));
        match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => restyle(content),
            Block::Quote(blocks) | Block::Aligned { blocks, .. } | Block::Html(blocks) => {
                for block in blocks {
                    self.apply_to_block(block);
                }
            }
            Block::Details { summary, blocks, .. } => {
                restyle(summary);
                for block in blocks {
                    self.apply_to_block(block);
                }
            }
            Block::List(list) => {
                list.items.iter_mut().flat_map(|item| &mut item.blocks).for_each(|block| self.apply_to_block(block));
            }
            Block::Table(table) => {
                table.header.iter_mut().chain(table.rows.iter_mut().flatten()).for_each(restyle);
            }
            Block::Code { .. } | Block::ThematicBreak => {}
        }
    }
}

/// True for a `font-weight` that reads as bold: `bold`, `bolder`, or 600
/// and up.
fn is_bold_weight(value: &str) -> bool {
    value == "bold" || value == "bolder" || value.parse::<u16>().is_ok_and(|weight| weight >= 600)
}

/// The address of an `<audio>` or `<video>`: its `src`, or the first
/// `<source>` inside it.
fn media_source(element: &Element) -> Option<String> {
    let from_source = || {
        element.children.iter().find_map(|node| match node {
            Node::Element(source) if source.name == "source" => source.attribute("src"),
            _ => None,
        })
    };
    non_empty(element.attribute("src").or_else(from_source))
}

/// An attribute value, trimmed, when it is not blank.
fn non_empty(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

/// Turn every run of whitespace into one space, as a browser shows text.
/// Non-breaking spaces stay.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for character in text.chars() {
        if character.is_ascii_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(character);
            in_space = false;
        }
    }
    out
}

/// Make inline content read as a browser shows it: neighboring text
/// joined with one space where the pieces meet, empty pieces dropped, and
/// no spaces at the ends or around line breaks.
fn tidy(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut inlines = join_text(inlines);
    trim_edges(&mut inlines);
    inlines
}

/// Join neighboring text pieces (at every level) and drop empty ones.
fn join_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for inline in inlines {
        let inline = match inline {
            Inline::Strong(inner) => Inline::Strong(join_text(inner)),
            Inline::Emphasis(inner) => Inline::Emphasis(join_text(inner)),
            Inline::Strikethrough(inner) => Inline::Strikethrough(join_text(inner)),
            Inline::Link { content, url } => Inline::Link { content: join_text(content), url },
            Inline::Styled { style, content } => Inline::Styled { style, content: join_text(content) },
            other => other,
        };
        match (out.last_mut(), inline) {
            (Some(Inline::Text(previous)), Inline::Text(text)) => {
                let text = if previous.ends_with(' ') { text.trim_start_matches(' ') } else { text.as_str() };
                previous.push_str(text);
            }
            (_, Inline::Text(text)) if text.is_empty() => {}
            (
                _,
                Inline::Strong(inner)
                | Inline::Emphasis(inner)
                | Inline::Strikethrough(inner)
                | Inline::Styled { content: inner, .. },
            ) if inner.is_empty() => {}
            (_, inline) => out.push(inline),
        }
    }
    out
}

/// Trim spaces at the start and end, and around line breaks; drop the
/// text left empty and trailing line breaks.
fn trim_edges(inlines: &mut Vec<Inline>) {
    for index in 0..inlines.len() {
        let starts_line = index == 0 || matches!(inlines[index - 1], Inline::LineBreak);
        let ends_line = index + 1 == inlines.len() || matches!(inlines[index + 1], Inline::LineBreak);
        if let Inline::Text(text) = &mut inlines[index] {
            if ends_line {
                text.truncate(text.trim_end_matches(' ').len());
            }
            if starts_line {
                *text = text.trim_start_matches(' ').to_string();
            }
        }
    }
    inlines.retain(|inline| !matches!(inline, Inline::Text(text) if text.is_empty()));
    while matches!(inlines.last(), Some(Inline::LineBreak)) {
        inlines.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_string())
    }

    fn paragraph(value: &str) -> Block {
        Block::Paragraph(vec![text(value)])
    }

    #[test]
    fn headings_paragraphs_and_emphasis() {
        let blocks = parse_html_blocks(
            "<!doctype html><html><head><title>T</title><style>p{color:red}</style></head>\n\
             <body><h1>Title</h1>\n<p>Some <b>bold</b> and <em>soft</em>\n   text.</p></body></html>",
        );
        assert_eq!(
            blocks,
            [
                Block::Heading { level: 1, content: vec![text("Title")] },
                Block::Paragraph(vec![
                    text("Some "),
                    Inline::Strong(vec![text("bold")]),
                    text(" and "),
                    Inline::Emphasis(vec![text("soft")]),
                    text(" text."),
                ]),
            ]
        );
    }

    #[test]
    fn scripts_comments_and_hidden_elements_are_dropped() {
        let blocks = parse_html_blocks(
            "<p>a<!-- a comment <p>not this</p> --></p><script>if (a < b) { x = '</p>' }</script>\
             <SCRIPT type=module>alert(1)</SCRIPT><svg><text>no</text></svg><p>b</p>",
        );
        assert_eq!(blocks, [paragraph("a"), paragraph("b")]);
    }

    #[test]
    fn unclosed_paragraphs_and_items_close_themselves() {
        let blocks = parse_html_blocks("<p>one<p>two<ul><li>a<li>b</ul><div>loose <i>text</i></div>");
        assert_eq!(
            blocks,
            [
                paragraph("one"),
                paragraph("two"),
                Block::List(List {
                    start: None,
                    items: vec![
                        ListItem { task: None, blocks: vec![paragraph("a")] },
                        ListItem { task: None, blocks: vec![paragraph("b")] },
                    ],
                }),
                Block::Paragraph(vec![text("loose "), Inline::Emphasis(vec![text("text")])]),
            ]
        );
    }

    #[test]
    fn ordered_lists_tasks_and_nesting() {
        let blocks = parse_html_blocks(
            "<ol start='3'><li><input type=checkbox checked> done</li>\
             <li><input type=\"checkbox\">todo<ul><li>inner</li></ul></li></ol>",
        );
        let inner =
            Block::List(List { start: None, items: vec![ListItem { task: None, blocks: vec![paragraph("inner")] }] });
        assert_eq!(
            blocks,
            [Block::List(List {
                start: Some(3),
                items: vec![
                    ListItem { task: Some(true), blocks: vec![paragraph("done")] },
                    ListItem { task: Some(false), blocks: vec![paragraph("todo"), inner] },
                ],
            })]
        );
    }

    #[test]
    fn links_images_and_line_breaks() {
        let blocks = parse_html_blocks(
            "<p><a href=\"notes/a.html\">A <code>link</code></a><br>\n<img src=pics/cat.png alt=\"A cat\"/>\
             <a name=anchor>plain</a></p>",
        );
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                Inline::Link {
                    content: vec![text("A "), Inline::Code("link".to_string())],
                    url: "notes/a.html".to_string()
                },
                Inline::LineBreak,
                Inline::Image { alt: "A cat".to_string(), url: "pics/cat.png".to_string(), size: ImageSize::default() },
                text("plain"),
            ])]
        );
    }

    #[test]
    fn audio_becomes_a_link_and_video_an_image() {
        let blocks = parse_html_blocks(
            "<audio controls src=\"memos/a.wav\" title=\"Tom's memo\"></audio>\n\
             <video controls><source src=\"clip.mp4\"></video>",
        );
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                Inline::Link { content: vec![text("Tom's memo")], url: "memos/a.wav".to_string() },
                text(" "),
                Inline::Image { alt: String::new(), url: "clip.mp4".to_string(), size: ImageSize::default() },
            ])]
        );
    }

    #[test]
    fn code_keeps_its_spacing_and_language() {
        let blocks = parse_html_blocks(
            "<pre>\n<code class=\"language-python\">def f():\n    return 1 &lt; 2\n</code></pre><hr>\
             <blockquote><p>quoted</p></blockquote>",
        );
        assert_eq!(
            blocks,
            [
                Block::Code { info: "python".to_string(), code: "def f():\n    return 1 < 2".to_string() },
                Block::ThematicBreak,
                Block::Quote(vec![paragraph("quoted")]),
            ]
        );
    }

    #[test]
    fn tables_get_a_header_alignment_and_padding() {
        let blocks = parse_html_blocks(
            "<table><caption>Prices</caption><thead><tr><th>Item<th align=right>Cost</thead>\
             <tbody><tr><td>Tea<td style=\"color: red\">2</tr><tr><td>Cake</tr></tbody></table>",
        );
        assert_eq!(
            blocks,
            [
                paragraph("Prices"),
                Block::Table(Table {
                    align: vec![ColumnAlign::Left, ColumnAlign::Right],
                    header: vec![vec![text("Item")], vec![text("Cost")]],
                    rows: vec![
                        vec![vec![text("Tea")], vec![styled(red(), vec![text("2")])]],
                        vec![vec![text("Cake")], vec![]],
                    ],
                }),
            ]
        );
    }

    fn red() -> HtmlStyle {
        HtmlStyle { color: Some(Rgb(0xff, 0, 0)), ..HtmlStyle::default() }
    }

    fn styled(style: HtmlStyle, content: Vec<Inline>) -> Inline {
        Inline::Styled { style, content }
    }

    #[test]
    fn style_attributes_set_the_look_of_text() {
        let blocks = parse_html_blocks(
            "<p><span style=\"color: #f00; font-weight: 700\">hot</span> \
             <span style='font-style: italic; text-decoration: underline line-through'>x</span> \
             <font color=red>old</font> <span style=\"font-size: 40px\">big</span></p>",
        );
        let underline = HtmlStyle { underline: true, ..HtmlStyle::default() };
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                Inline::Strong(vec![styled(red(), vec![text("hot")])]),
                text(" "),
                Inline::Emphasis(vec![Inline::Strikethrough(vec![styled(underline, vec![text("x")])])]),
                text(" "),
                styled(red(), vec![text("old")]),
                text(" big"),
            ])]
        );
    }

    #[test]
    fn mark_underline_and_scripts() {
        let blocks = parse_html_blocks("<p><mark>m</mark><u>u</u><ins>i</ins>H<sub>2</sub>O x<sup>2</sup></p>");
        let look = |style: HtmlStyle, value: &str| styled(style, vec![text(value)]);
        let underline = HtmlStyle { underline: true, ..HtmlStyle::default() };
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                look(HtmlStyle { background: Some(MARK_COLOR), ..HtmlStyle::default() }, "m"),
                look(underline, "u"),
                look(underline, "i"),
                text("H"),
                look(HtmlStyle { script: Some(Script::Subscript), ..HtmlStyle::default() }, "2"),
                text("O x"),
                look(HtmlStyle { script: Some(Script::Superscript), ..HtmlStyle::default() }, "2"),
            ])]
        );
    }

    #[test]
    fn a_block_style_reaches_its_text_but_not_its_background() {
        let blocks = parse_html_blocks(
            "<div style=\"color: red; background: yellow\"><p>a</p><ul><li>b</li></ul><pre>code</pre></div>",
        );
        assert_eq!(
            blocks,
            [
                Block::Paragraph(vec![styled(red(), vec![text("a")])]),
                Block::List(List {
                    start: None,
                    items: vec![ListItem {
                        task: None,
                        blocks: vec![Block::Paragraph(vec![styled(red(), vec![text("b")])])]
                    }],
                }),
                Block::Code { info: String::new(), code: "code".to_string() },
            ]
        );
    }

    #[test]
    fn alignment_from_attributes_styles_and_center() {
        let blocks = parse_html_blocks(
            "<h1 align=\"center\">Title</h1><center>c</center>\
             <p align=center style=\"text-align: right\">r</p><div align=\"center\"><p align=left>l</p></div>\
             <p align=\"top\">plain</p>",
        );
        assert_eq!(
            blocks,
            [
                Block::Aligned {
                    align: ColumnAlign::Center,
                    blocks: vec![Block::Heading { level: 1, content: vec![text("Title")] }]
                },
                Block::Aligned { align: ColumnAlign::Center, blocks: vec![paragraph("c")] },
                Block::Aligned { align: ColumnAlign::Right, blocks: vec![paragraph("r")] },
                Block::Aligned {
                    align: ColumnAlign::Center,
                    blocks: vec![Block::Aligned { align: ColumnAlign::Left, blocks: vec![paragraph("l")] }]
                },
                paragraph("plain"),
            ]
        );
    }

    #[test]
    fn details_keep_their_summary_and_open_state() {
        let blocks = parse_html_blocks(
            "<details open><summary>More <b>info</b></summary><p>hidden</p></details>\
             <details>loose text</details>",
        );
        assert_eq!(
            blocks,
            [
                Block::Details {
                    summary: vec![text("More "), Inline::Strong(vec![text("info")])],
                    open: true,
                    blocks: vec![paragraph("hidden")],
                },
                Block::Details {
                    summary: vec![text(DEFAULT_SUMMARY)],
                    open: false,
                    blocks: vec![paragraph("loose text")]
                },
            ]
        );
    }

    #[test]
    fn image_sizes_in_pixels() {
        let blocks = parse_html_blocks(
            "<img src=a.png width=\"200\"><img src=b.png height=20px style=\"width: 50px\">\
             <img src=c.png width=\"50%\" height=0>",
        );
        let image = |url: &str, width, height| Inline::Image {
            alt: String::new(),
            url: url.to_string(),
            size: ImageSize { width, height },
        };
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                image("a.png", Some(200), None),
                image("b.png", Some(50), Some(20)),
                image("c.png", None, None),
            ])]
        );
    }

    #[test]
    fn spanning_cells_keep_the_cells_after_them_in_their_columns() {
        let blocks = parse_html_blocks(
            "<table><tr><th colspan=2>Name<th align=right>Cost</tr>\
             <tr><td>a<td>b<td>1</tr></table>",
        );
        let Block::Table(table) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(table.align, [ColumnAlign::Left, ColumnAlign::Left, ColumnAlign::Right]);
        assert_eq!(table.header, [vec![text("Name")], vec![], vec![text("Cost")]]);
        assert_eq!(table.rows[0], [vec![text("a")], vec![text("b")], vec![text("1")]]);
        let blocks = parse_html_blocks("<table><tr><td colspan=99999>wide</tr></table>");
        let Block::Table(table) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(table.header.len(), MAX_COLSPAN);
    }

    #[test]
    fn entities_decode_and_unknown_ones_stay() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#233;&#xE9; &bogus; & x"),
            "a & b <c> \u{e9}\u{e9} &bogus; & x"
        );
        assert_eq!(decode_entities("&#0; &#xFFFFFFFF;"), "&#0; &#xFFFFFFFF;");
    }

    #[test]
    fn plain_text_and_stray_markup_survive() {
        assert_eq!(parse_html_blocks("just text, 1 < 2 and </b> stray"), [paragraph("just text, 1 < 2 and stray")]);
        assert_eq!(parse_html_blocks("<p>cut off <a href=\"x"), [paragraph("cut off <a href=\"x")]);
        assert_eq!(parse_html_blocks(""), []);
    }

    #[test]
    fn raw_text_ends_only_at_its_own_end_tag() {
        assert_eq!(parse_html_blocks("<style>a</b></Style ><p>after"), [paragraph("after")]);
        assert_eq!(parse_html_blocks("<p>before<script>never closed"), [paragraph("before")]);
    }

    #[test]
    fn a_missing_head_end_tag_does_not_hide_the_body() {
        assert_eq!(parse_html_blocks("<head><meta charset=utf-8><body><p>shown"), [paragraph("shown")]);
    }

    #[test]
    fn deep_nesting_is_capped() {
        let html = format!("{}deep{}", "<blockquote>".repeat(10_000), "</blockquote>".repeat(10_000));
        let blocks = parse_html_blocks(&html);
        let mut depth = 0;
        let mut block = &blocks[0];
        while let Block::Quote(inner) = block {
            depth += 1;
            block = &inner[0];
        }
        assert_eq!(block, &paragraph("deep"));
        assert!(depth <= MAX_DEPTH);
    }

    #[test]
    fn source_blocks_cover_the_whole_file() {
        let html = "<p>a</p><p>b</p>";
        let blocks = html_source_blocks(html);
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().all(|block| block.range == (0..html.len())));
    }
}
