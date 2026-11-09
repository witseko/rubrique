//! React's JSX: HTML-like tags inside JavaScript and TSX.
//!
//! The scanner hands a `<` over here when it opens a tag. Tag names are
//! keywords, attribute names are names, `<`, `>`, `/`, `=` and the braces
//! around embedded code are markers, and text between tags stays plain.
//! Code in braces (`{count + 1}`) is scanned as JavaScript again, so it may
//! hold more tags.
//!
//! `<` is also less-than and, in TypeScript, opens generics, so a tag must
//! come where an expression starts (after `(`, `=`, `return` and so on) and
//! read as a whole tag up to its `>` before anything is labeled. A tag still
//! being typed stays plain until its `>` is there.

use super::{Kind, Scanner, is_word_char};

/// Elements nested deeper than this are left plain, so absurd nesting
/// cannot overflow the stack.
const MAX_DEPTH: usize = 64;

/// Bytes after which `<` starts an expression: `(<div>`, `= <A/>`,
/// `cond && <B/>`, `x ? <C/> : <D/>`, `() => <E/>`.
const EXPRESSION_BYTES: &[u8] = b"([{,;=:?!&|>}";

/// Words after which `<` starts an expression (`return <div/>`).
const EXPRESSION_WORDS: [&str; 4] = ["return", "yield", "default", "case"];

impl Scanner<'_> {
    /// Whether the `<` at `start` opens a JSX element.
    pub(super) fn starts_jsx(&self, start: usize) -> bool {
        self.rules.jsx
            && self.jsx_depth < MAX_DEPTH
            && starts_expression(self.text, start)
            && opening_tag(self.text, start).is_some()
    }

    /// An element at the current position, from its opening tag to its
    /// closing tag (or the end of the text).
    pub(super) fn jsx_element(&mut self) {
        let Some(self_closing) = opening_tag(self.text, self.position) else {
            return;
        };
        self.jsx_depth += 1;
        self.jsx_tag();
        if !self_closing {
            self.jsx_children();
        }
        self.jsx_depth -= 1;
    }

    /// An opening tag: `<`, the name, attributes, then `>` or `/>`.
    fn jsx_tag(&mut self) {
        self.mark(self.position + 1, Kind::Marker);
        let tag_name_end = name_end(self.text, self.position);
        self.mark(tag_name_end, Kind::Keyword);
        while let Some(character) = self.text[self.position..].chars().next() {
            let rest = &self.text[self.position..];
            if rest.starts_with("/>") || rest.starts_with('>') {
                let length = if rest.starts_with('>') { 1 } else { 2 };
                self.mark(self.position + length, Kind::Marker);
                return;
            }
            if character.is_alphabetic() {
                let end = name_end(self.text, self.position);
                self.mark(end, Kind::Name);
            } else if character == '=' {
                self.mark(self.position + 1, Kind::Marker);
            } else if character == '"' || character == '\'' {
                let end = quoted_end(self.text, self.position);
                self.mark(end, Kind::String);
            } else if character == '{' {
                self.jsx_code();
            } else {
                self.position += character.len_utf8();
            }
        }
    }

    /// What sits between an opening and a closing tag: plain text, child
    /// elements and code in braces. Ends after the closing tag.
    fn jsx_children(&mut self) {
        while let Some(character) = self.text[self.position..].chars().next() {
            let rest = &self.text[self.position..];
            if rest.starts_with("</") {
                self.mark(self.position + 2, Kind::Marker);
                let tag_name_end = name_end(self.text, self.position);
                self.mark(tag_name_end, Kind::Keyword);
                let close =
                    self.text[self.position..].find('>').map_or(self.text.len(), |offset| self.position + offset + 1);
                self.mark(close, Kind::Marker);
                return;
            }
            if character == '<' && self.jsx_depth < MAX_DEPTH && opening_tag(self.text, self.position).is_some() {
                self.jsx_element();
            } else if character == '{' {
                self.jsx_code();
            } else {
                self.position += character.len_utf8();
            }
        }
    }

    /// Code in braces, scanned as JavaScript up to the matching `}`.
    fn jsx_code(&mut self) {
        self.mark(self.position + 1, Kind::Marker);
        let mut depth = 0_usize;
        while let Some(&byte) = self.text.as_bytes().get(self.position) {
            match byte {
                b'}' if depth == 0 => {
                    self.mark(self.position + 1, Kind::Marker);
                    return;
                }
                b'}' => {
                    depth -= 1;
                    self.position += 1;
                }
                b'{' => {
                    depth += 1;
                    self.position += 1;
                }
                _ => self.step(),
            }
        }
    }
}

/// Whether `<` at `start` sits where an expression begins rather than
/// after a value (where it is less-than or a generic).
fn starts_expression(text: &str, start: usize) -> bool {
    let before = text[..start].trim_end();
    match before.as_bytes().last() {
        None => true,
        Some(byte) if EXPRESSION_BYTES.contains(byte) => true,
        Some(_) => {
            let word_start = before
                .char_indices()
                .rev()
                .find(|(_, character)| !is_word_char(*character))
                .map_or(0, |(index, character)| index + character.len_utf8());
            EXPRESSION_WORDS.contains(&&before[word_start..])
        }
    }
}

/// If a whole opening tag starts at `start`, whether it closes itself
/// (`<br />`). `None` for anything else: less-than, a generic like
/// `<T,>(x)` or `<T extends U>(x)`, or a tag not finished yet.
fn opening_tag(text: &str, start: usize) -> Option<bool> {
    let bytes = text.as_bytes();
    let name_start = start + 1;
    let first = text[name_start..].chars().next()?;
    // `<>` opens a fragment; any other tag starts with a letter.
    if first != '>' && !first.is_alphabetic() {
        return None;
    }
    let mut index = name_end(text, name_start);
    loop {
        match *bytes.get(index)? {
            b' ' | b'\t' | b'\n' | b'\r' | b'=' => index += 1,
            // A generic arrow function's type list is followed by `(`.
            b'>' => return (!text[index + 1..].trim_start().starts_with('(')).then_some(false),
            b'/' if bytes.get(index + 1) == Some(&b'>') => return Some(true),
            b'"' | b'\'' => index = quoted_end(text, index),
            b'{' => index = brace_end(text, index)?,
            byte if byte.is_ascii_alphabetic() => index = name_end(text, index),
            _ => return None,
        }
    }
}

/// The end of a tag or attribute name: letters, digits, `_`, `$`, `-`
/// (`aria-label`), `.` (`Menu.Item`) and `:` (`xlink:href`).
fn name_end(text: &str, start: usize) -> usize {
    text[start..]
        .find(|character: char| !(is_word_char(character) || matches!(character, '-' | '.' | ':')))
        .map_or(text.len(), |offset| start + offset)
}

/// Just past the quote that closes the attribute value opening at `start`
/// (JSX attribute strings have no escapes), or the end of the text.
fn quoted_end(text: &str, start: usize) -> usize {
    let quote = text.as_bytes()[start];
    text.as_bytes()[start + 1..].iter().position(|&byte| byte == quote).map_or(text.len(), |offset| start + offset + 2)
}

/// Just past the `}` matching the `{` at `start`, skipping strings; `None`
/// when it never closes.
fn brace_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut index = start;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            b'"' | b'\'' | b'`' => {
                index += 1;
                while bytes.get(index).is_some_and(|&inner| inner != byte) {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::{JAVASCRIPT, TSX, TYPESCRIPT, highlight};
    use crate::markdown::highlight::Kind;

    fn colored<'a>(text: &'a str, rules: &super::super::Rules) -> Vec<(&'a str, Kind)> {
        highlight(text, rules)
            .into_iter()
            .filter(|span| span.kind != Kind::Plain)
            .map(|span| (&text[span.range], span.kind))
            .collect()
    }

    #[test]
    fn tags_attributes_and_code() {
        let text = "return <Button kind=\"big\" onClick={() => go(1)}>Go if ready</Button>;";
        assert_eq!(
            colored(text, &JAVASCRIPT),
            vec![
                ("return", Kind::Keyword),
                ("<", Kind::Marker),
                ("Button", Kind::Keyword),
                ("kind", Kind::Name),
                ("=", Kind::Marker),
                ("\"big\"", Kind::String),
                ("onClick", Kind::Name),
                // Neighboring markers join into one span.
                ("={", Kind::Marker),
                ("1", Kind::Number),
                ("}>", Kind::Marker),
                ("</", Kind::Marker),
                ("Button", Kind::Keyword),
                (">", Kind::Marker),
            ]
        );
    }

    #[test]
    fn nested_elements_fragments_and_self_closing_tags() {
        let text = "const view = (<>{ok && <img src={url} />}<p>hi</p></>);";
        let kinds: Vec<&str> =
            colored(text, &JAVASCRIPT).into_iter().filter(|(_, kind)| *kind == Kind::Keyword).map(|(t, _)| t).collect();
        assert_eq!(kinds, ["const", "img", "p", "p"]);
    }

    #[test]
    fn less_than_and_generics_stay_code() {
        assert_eq!(colored("if (a < b) x = 1", &JAVASCRIPT), vec![("if", Kind::Keyword), ("1", Kind::Number)]);
        assert_eq!(colored("const f = <T,>(x: T) => x", &TSX), vec![("const", Kind::Keyword)]);
        assert_eq!(
            colored("const f = <T extends string>(x: T) => x", &TSX),
            vec![("const", Kind::Keyword), ("extends", Kind::Keyword), ("string", Kind::Keyword)]
        );
        assert_eq!(
            colored("let a = <div>", &JAVASCRIPT),
            vec![("let", Kind::Keyword), ("<", Kind::Marker), ("div", Kind::Keyword), (">", Kind::Marker)]
        );
    }

    #[test]
    fn plain_typescript_has_no_tags() {
        // `<Type>value` is a cast in a `.ts` file.
        assert_eq!(
            colored("let n = <number>value", &TYPESCRIPT),
            vec![("let", Kind::Keyword), ("number", Kind::Keyword)]
        );
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        let text = "x = ".to_string() + &"<a>".repeat(10_000);
        let spans = highlight(&text, &JAVASCRIPT);
        assert_eq!(spans.last().map(|span| span.range.end), Some(text.len()));
    }
}
