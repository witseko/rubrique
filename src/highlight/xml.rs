//! Find XML syntax in raw text so it can be colored.
//!
//! Forgiving, like the other highlighters: it never fails, it only labels
//! byte ranges. Tag names are keywords, attribute names are names, attribute
//! values are strings; `<`, `>`, `=`, entities (`&amp;`), declarations
//! (`<?xml ?>`, `<!DOCTYPE>`) and the CDATA brackets are markers; comments
//! are comments and the inside of CDATA is code. Element text stays plain.

use crate::markdown::highlight::{Kind, Span, Spans};

/// Label every byte of `text`. Spans are in order, touch each other, never
/// empty, and neighbors never share a kind.
pub fn highlight(text: &str) -> Vec<Span> {
    let mut scanner = Scanner { text, out: Spans::default(), position: 0 };
    while scanner.position < text.len() {
        scanner.step();
    }
    scanner.out.to(text.len(), Kind::Plain);
    scanner.out.spans
}

const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";

/// Walks the text once, front to back, stopping only on char boundaries.
struct Scanner<'a> {
    text: &'a str,
    out: Spans,
    position: usize,
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
            self.tag();
        } else if rest.starts_with('&') && entity_length(rest).is_some() {
            let end = self.position + entity_length(rest).unwrap_or(1);
            self.mark(end, Kind::Marker);
        } else {
            self.position += rest.chars().next().map_or(1, char::len_utf8);
        }
    }

    /// The inside of a tag, from its name to its closing `>` or `/>`.
    fn tag(&mut self) {
        let end = name_end(self.text, self.position);
        self.mark(end, Kind::Keyword);
        while let Some(character) = self.text[self.position..].chars().next() {
            let rest = &self.text[self.position..];
            if rest.starts_with("/>") || rest.starts_with('>') {
                let length = if rest.starts_with('>') { 1 } else { 2 };
                self.mark(self.position + length, Kind::Marker);
                return;
            }
            if rest.starts_with('<') {
                // A broken tag: let the next one start fresh.
                return;
            }
            if is_name_start(character) {
                let end = name_end(self.text, self.position);
                self.mark(end, Kind::Name);
            } else if character == '=' {
                self.mark(self.position + 1, Kind::Marker);
            } else if character == '"' || character == '\'' {
                let quote = if character == '"' { "\"" } else { "'" };
                let end = self.end_after(self.position + 1, quote);
                self.mark(end, Kind::String);
            } else {
                self.position += character.len_utf8();
            }
        }
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

    #[test]
    fn unfinished_input_is_covered() {
        for text in ["<", "<a", "<a b=\"", "<!--", "<![CDATA[x", "&amp", "<a\n<b>", "<é ü='ö'>"] {
            let spans = highlight(text);
            assert_eq!(spans.last().map_or(0, |span| span.range.end), text.len(), "{text:?}");
        }
    }
}
