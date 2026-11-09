//! Find XML syntax in raw text so it can be colored.
//!
//! Forgiving, like the other highlighters: it never fails, it only labels
//! byte ranges. Tag names are keywords, attribute names are names, attribute
//! values are strings; `<`, `>`, `=`, entities (`&amp;`), declarations
//! (`<?xml ?>`, `<!DOCTYPE>`) and the CDATA brackets are markers; comments
//! are comments and the inside of CDATA is code. Element text stays plain.
//!
//! HTML uses the same scanner with Angular's template syntax on top:
//! `{{ value }}` interpolation is a template, binding attributes
//! (`[value]`, `(click)`, `[(ngModel)]`, `*ngIf`, `#ref`) are names, and
//! control flow blocks (`@if`, `@for`, `@switch`) are keywords. The text of
//! `<script>` and `<style>` elements stays plain, so a `<` in code there
//! opens no tag.

use crate::markdown::highlight::{Kind, Span, Spans};

/// Label every byte of `text`. Spans are in order, touch each other, never
/// empty, and neighbors never share a kind.
pub fn highlight(text: &str) -> Vec<Span> {
    scan(text, false)
}

/// Label every byte of HTML `text`, Angular templates included.
pub fn highlight_html(text: &str) -> Vec<Span> {
    scan(text, true)
}

fn scan(text: &str, html: bool) -> Vec<Span> {
    let mut scanner = Scanner { text, out: Spans::default(), position: 0, html };
    while scanner.position < text.len() {
        scanner.step();
    }
    scanner.out.to(text.len(), Kind::Plain);
    scanner.out.spans
}

const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";
/// Angular's control flow blocks, written `@if (...) { ... }`.
const ANGULAR_BLOCKS: [&str; 12] =
    ["if", "else", "for", "empty", "switch", "case", "default", "defer", "placeholder", "loading", "error", "let"];
/// HTML elements whose text is code or CSS, not markup.
const RAW_TEXT_ELEMENTS: [&str; 2] = ["script", "style"];

/// Walks the text once, front to back, stopping only on char boundaries.
struct Scanner<'a> {
    text: &'a str,
    out: Spans,
    position: usize,
    /// HTML with Angular templates rather than plain XML.
    html: bool,
}

impl Scanner<'_> {
    /// Read one construct (or one plain char) at the current position.
    fn step(&mut self) {
        let rest = &self.text[self.position..];
        if rest.starts_with("<!--") {
            let end = self.end_after(self.position + 4, "-->");
            self.mark(end, Kind::Comment);
        } else if rest.starts_with(CDATA_OPEN) {
            self.mark(self.position + CDATA_OPEN.len(), Kind::Marker);
            let close = find_from(self.text, self.position, CDATA_CLOSE).unwrap_or(self.text.len());
            self.mark(close, Kind::Code);
            self.mark((close + CDATA_CLOSE.len()).min(self.text.len()), Kind::Marker);
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            let end = self.end_after(self.position + 2, ">");
            self.mark(end, Kind::Marker);
        } else if rest.starts_with("</") && rest[2..].chars().next().is_some_and(is_name_start) {
            self.mark(self.position + 2, Kind::Marker);
            self.tag();
        } else if rest.starts_with('<') && rest[1..].chars().next().is_some_and(is_name_start) {
            self.mark(self.position + 1, Kind::Marker);
            let name_start = self.position;
            let opened = self.tag();
            let name = &self.text[name_start..name_end(self.text, name_start)];
            if self.html && opened && RAW_TEXT_ELEMENTS.iter().any(|raw| raw.eq_ignore_ascii_case(name)) {
                self.position = raw_text_end(self.text, self.position, name);
            }
        } else if rest.starts_with('&') && entity_length(rest).is_some() {
            let end = self.position + entity_length(rest).unwrap_or(1);
            self.mark(end, Kind::Marker);
        } else if self.html && rest.starts_with("{{") {
            let end = self.end_after(self.position + 2, "}}");
            self.mark(end, Kind::Template);
        } else if let Some(length) = self.html.then(|| angular_block_length(rest)).flatten() {
            self.mark(self.position + length, Kind::Keyword);
        } else {
            self.position += rest.chars().next().map_or(1, char::len_utf8);
        }
    }

    /// The inside of a tag, from its name to its closing `>` or `/>`.
    /// Returns whether it ended with `>` (so an element's text follows).
    fn tag(&mut self) -> bool {
        let end = name_end(self.text, self.position);
        self.mark(end, Kind::Keyword);
        while let Some(character) = self.text[self.position..].chars().next() {
            let rest = &self.text[self.position..];
            if rest.starts_with("/>") || rest.starts_with('>') {
                let opened = rest.starts_with('>');
                self.mark(self.position + if opened { 1 } else { 2 }, Kind::Marker);
                return opened;
            }
            if rest.starts_with('<') {
                // A broken tag: let the next one start fresh.
                return false;
            }
            if is_name_start(character) || (self.html && is_binding_start(character)) {
                let end =
                    if self.html { binding_end(self.text, self.position) } else { name_end(self.text, self.position) };
                self.mark(end, Kind::Name);
            } else if character == '=' {
                self.mark(self.position + 1, Kind::Marker);
            } else if character == '"' || character == '\'' {
                let quote = if character == '"' { "\"" } else { "'" };
                let end = self.end_after(self.position + 1, quote);
                self.mark_value(end);
            } else {
                self.position += character.len_utf8();
            }
        }
        false
    }

    /// An attribute value up to `end`: a string, with any `{{ ... }}` in
    /// HTML labeled as a template.
    fn mark_value(&mut self, end: usize) {
        while self.html && self.position < end {
            let Some(open) = find_from(self.text, self.position, "{{").filter(|&open| open < end) else {
                break;
            };
            self.mark(open, Kind::String);
            let close = find_from(self.text, open + 2, "}}").map_or(end, |close| (close + 2).min(end));
            self.mark(close, Kind::Template);
        }
        self.mark(end, Kind::String);
    }

    /// Just past the next `close` at or after `from`, or the end of the text.
    fn end_after(&self, from: usize, close: &str) -> usize {
        find_from(self.text, from, close).map_or(self.text.len(), |found| found + close.len())
    }

    /// Label plain text up to the current position, then up to `end` as
    /// `kind`, and move on to `end`.
    fn mark(&mut self, end: usize, kind: Kind) {
        self.out.to(self.position, Kind::Plain);
        self.out.to(end, kind);
        self.position = end;
    }
}

fn is_name_start(character: char) -> bool {
    character.is_alphabetic() || character == '_' || character == ':'
}

fn is_name_char(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | ':' | '-' | '.')
}

/// The end of the name that starts at `start`.
fn name_end(text: &str, start: usize) -> usize {
    text[start..].find(|character| !is_name_char(character)).map_or(text.len(), |offset| start + offset)
}

/// Angular binding attributes start with `[`, `(`, `*`, `#` or `@`.
fn is_binding_start(character: char) -> bool {
    matches!(character, '[' | '(' | '*' | '#' | '@')
}

/// The end of an HTML attribute name, Angular bindings included
/// (`[(ngModel)]`, `(click)`, `[class.active]`, `*ngFor`).
fn binding_end(text: &str, start: usize) -> usize {
    text[start..]
        .find(|character| !(is_name_char(character) || is_binding_start(character) || matches!(character, ']' | ')')))
        .map_or(text.len(), |offset| start + offset)
}

/// The length of an Angular block keyword like `@if` or `@else if` at the
/// start of `text`; `None` for other `@` text such as an email address.
fn angular_block_length(text: &str) -> Option<usize> {
    let word = text.strip_prefix('@')?;
    let length = word.find(|character: char| !character.is_ascii_alphabetic()).unwrap_or(word.len());
    if !ANGULAR_BLOCKS.contains(&&word[..length]) {
        return None;
    }
    let else_if = &word[..length] == "else"
        && word[length..].trim_start().starts_with("if")
        && !word[length..].trim_start()[2..].starts_with(|character: char| character.is_alphanumeric());
    if else_if {
        let spaces = word[length..].len() - word[length..].trim_start().len();
        return Some(1 + length + spaces + 2);
    }
    Some(1 + length)
}

/// Where the text of a `<script>` or `<style>` element that starts at
/// `from` ends: at its closing tag (any case), or the end of the text.
fn raw_text_end(text: &str, from: usize, name: &str) -> usize {
    let mut search = from;
    while let Some(found) = find_from(text, search, "</") {
        let after = found + 2;
        if text.get(after..after + name.len()).is_some_and(|candidate| candidate.eq_ignore_ascii_case(name)) {
            return found;
        }
        search = after;
    }
    text.len()
}

/// The length of an entity like `&amp;` or `&#x41;` at the start of `text`.
fn entity_length(text: &str) -> Option<usize> {
    let semicolon = text.find(';')?;
    let body = &text[1..semicolon];
    let valid = !body.is_empty() && body.chars().all(|character| character.is_alphanumeric() || character == '#');
    valid.then_some(semicolon + 1)
}

/// Where `needle` next appears at or after `from`.
fn find_from(text: &str, from: usize, needle: &str) -> Option<usize> {
    text.get(from..)?.find(needle).map(|offset| from + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colored(text: &str) -> Vec<(&str, Kind)> {
        highlight(text).into_iter().map(|span| (&text[span.range], span.kind)).collect()
    }

    #[test]
    fn tags_attributes_and_text() {
        assert_eq!(
            colored("<a href=\"x\">Tom &amp; me</a>"),
            vec![
                ("<", Kind::Marker),
                ("a", Kind::Keyword),
                (" ", Kind::Plain),
                ("href", Kind::Name),
                ("=", Kind::Marker),
                ("\"x\"", Kind::String),
                (">", Kind::Marker),
                ("Tom ", Kind::Plain),
                ("&amp;", Kind::Marker),
                (" me", Kind::Plain),
                ("</", Kind::Marker),
                ("a", Kind::Keyword),
                (">", Kind::Marker),
            ]
        );
    }

    #[test]
    fn declarations_comments_cdata_and_self_closing() {
        let text = "<?xml version=\"1.0\"?>\n<!-- a\nb --><svg:g/><![CDATA[<x>]]>";
        assert_eq!(
            colored(text),
            vec![
                ("<?xml version=\"1.0\"?>", Kind::Marker),
                ("\n", Kind::Plain),
                ("<!-- a\nb -->", Kind::Comment),
                ("<", Kind::Marker),
                ("svg:g", Kind::Keyword),
                ("/><![CDATA[", Kind::Marker),
                ("<x>", Kind::Code),
                ("]]>", Kind::Marker),
            ]
        );
    }

    #[test]
    fn stray_less_than_and_ampersand_stay_plain() {
        assert_eq!(colored("a < b & c"), vec![("a < b & c", Kind::Plain)]);
    }

    fn colored_html(text: &str) -> Vec<(&str, Kind)> {
        highlight_html(text)
            .into_iter()
            .filter(|span| span.kind != Kind::Plain)
            .map(|span| (&text[span.range], span.kind))
            .collect()
    }

    #[test]
    fn angular_bindings_and_interpolation() {
        let text = "<li *ngFor=\"let n of notes\" [class.done]=\"n.done\" (click)=\"open(n)\" title=\"Note {{n.id}}\">{{ n.title }}</li>";
        assert_eq!(
            colored_html(text),
            vec![
                ("<", Kind::Marker),
                ("li", Kind::Keyword),
                ("*ngFor", Kind::Name),
                ("=", Kind::Marker),
                ("\"let n of notes\"", Kind::String),
                ("[class.done]", Kind::Name),
                ("=", Kind::Marker),
                ("\"n.done\"", Kind::String),
                ("(click)", Kind::Name),
                ("=", Kind::Marker),
                ("\"open(n)\"", Kind::String),
                ("title", Kind::Name),
                ("=", Kind::Marker),
                ("\"Note ", Kind::String),
                ("{{n.id}}", Kind::Template),
                ("\"", Kind::String),
                (">", Kind::Marker),
                ("{{ n.title }}", Kind::Template),
                ("</", Kind::Marker),
                ("li", Kind::Keyword),
                (">", Kind::Marker),
            ]
        );
    }

    #[test]
    fn angular_control_flow_blocks() {
        let text = "@if (user) { hi } @else if (x) { } @else { } mail me@example.com";
        assert_eq!(
            colored_html(text),
            vec![("@if", Kind::Keyword), ("@else if", Kind::Keyword), ("@else", Kind::Keyword)]
        );
    }

    #[test]
    fn script_text_opens_no_tags() {
        let text = "<script>if (a<b) {}</SCRIPT><p>";
        let tags: Vec<&str> =
            colored_html(text).into_iter().filter(|(_, kind)| *kind == Kind::Keyword).map(|(tag, _)| tag).collect();
        assert_eq!(tags, ["script", "SCRIPT", "p"]);
    }

    #[test]
    fn xml_has_no_angular_syntax() {
        assert_eq!(colored("{{ x }} @if"), vec![("{{ x }} @if", Kind::Plain)]);
    }

    #[test]
    fn unfinished_input_is_covered() {
        for text in [
            "<",
            "<a",
            "<a b=\"",
            "<!--",
            "<![CDATA[x",
            "&amp",
            "<a\n<b>",
            "<é ü='ö'>",
            "{{",
            "@else",
            "@else i",
            "<script>",
            "<a b=\"{{",
        ] {
            for spans in [highlight(text), highlight_html(text)] {
                assert_eq!(spans.last().map_or(0, |span| span.range.end), text.len(), "{text:?}");
            }
        }
    }
}
