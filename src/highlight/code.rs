//! Find Python, JavaScript, TypeScript, Rust and JSON syntax in raw text so
//! it can be colored.
//!
//! One small scanner reads them all, steered by a `Rules` value per
//! language. Like the Markdown and YAML highlighters it is forgiving: it
//! never fails, it only labels byte ranges. It knows comments, strings,
//! numbers, keywords and the names that `def`, `class` and `function`
//! define. It does not parse, so it does not know JavaScript regular
//! expressions: a quote inside one can color the rest of its line as a
//! string (strings with plain quotes stop at the end of the line, so the
//! damage stays on that line).

use crate::markdown::highlight::{Kind, Span, Spans};

mod jsx;

/// What a language looks like to the scanner.
#[expect(clippy::struct_excessive_bools, reason = "each flag is an independent language feature, not a state")]
pub struct Rules {
    /// Starts a comment that runs to the end of the line.
    line_comment: &'static str,
    /// Opens and closes a comment that may span lines.
    block_comment: Option<(&'static str, &'static str)>,
    /// Python's `'''` and `"""` strings, which may span lines.
    triple_quotes: bool,
    /// Quote characters whose strings may span lines (JavaScript's backtick).
    multiline_quotes: &'static [u8],
    /// Python's string prefixes (`r"..."`, `f'...'`, `rb"..."` and so on).
    string_prefixes: bool,
    /// Python's `@decorator` lines.
    decorators: bool,
    /// A string followed by `:` is an object key (JSON).
    string_keys: bool,
    /// React's JSX tags inside the code (see `jsx.rs`).
    jsx: bool,
    /// Rust's `'a` lifetimes, which open no string.
    lifetimes: bool,
    /// Rust's `#[derive(Debug)]` and `#![...]` attributes.
    attributes: bool,
    /// Rust's macro calls, `println!`, colored as names.
    macros: bool,
    keywords: &'static [&'static str],
    /// More keywords on top of `keywords` (TypeScript's, on top of JavaScript's).
    extra_keywords: &'static [&'static str],
    /// Keywords whose next word is the name being defined.
    definers: &'static [&'static str],
}

pub const PYTHON: Rules = Rules {
    line_comment: "#",
    block_comment: None,
    triple_quotes: true,
    multiline_quotes: &[],
    string_prefixes: true,
    decorators: true,
    string_keys: false,
    jsx: false,
    lifetimes: false,
    attributes: false,
    macros: false,
    keywords: &[
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
        "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
    ],
    extra_keywords: &[],
    definers: &["def", "class"],
};

pub const JAVASCRIPT: Rules = Rules {
    line_comment: "//",
    block_comment: Some(("/*", "*/")),
    triple_quotes: false,
    multiline_quotes: b"`",
    string_prefixes: false,
    decorators: false,
    string_keys: false,
    jsx: true,
    lifetimes: false,
    attributes: false,
    macros: false,
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "null",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    extra_keywords: &[],
    definers: &["function", "class"],
};

/// TypeScript: JavaScript plus type keywords, the built-in type names and
/// decorators. `type` is a keyword but does not name the next word, since
/// `type` is also a common property name (`{ type: "a" }`).
pub const TYPESCRIPT: Rules = Rules {
    decorators: true,
    extra_keywords: &[
        "abstract",
        "accessor",
        "any",
        "as",
        "asserts",
        "bigint",
        "boolean",
        "declare",
        "enum",
        "implements",
        "infer",
        "interface",
        "is",
        "keyof",
        "module",
        "namespace",
        "never",
        "number",
        "object",
        "override",
        "private",
        "protected",
        "public",
        "readonly",
        "satisfies",
        "string",
        "symbol",
        "type",
        "unique",
        "unknown",
    ],
    definers: &["function", "class", "interface", "enum", "namespace"],
    // Plain `.ts` files have no JSX, and `<Type>value` is a cast there.
    jsx: false,
    ..JAVASCRIPT
};

/// TypeScript with JSX (`.tsx`).
pub const TSX: Rules = Rules { jsx: true, ..TYPESCRIPT };

/// Rust. Its strings may span lines; `r"..."` and `b"..."` prefixes join
/// their string. The built-in types are keywords, like TypeScript's.
pub const RUST: Rules = Rules {
    line_comment: "//",
    block_comment: Some(("/*", "*/")),
    triple_quotes: false,
    multiline_quotes: b"\"",
    string_prefixes: true,
    decorators: false,
    string_keys: false,
    jsx: false,
    lifetimes: true,
    attributes: true,
    macros: true,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
        "Self", "static", "struct", "super", "trait", "true", "type", "union", "unsafe", "use", "where", "while",
        "yield",
    ],
    extra_keywords: &[
        "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "f32", "f64",
    ],
    definers: &["fn", "struct", "enum", "trait", "type", "mod", "union"],
};

/// JSON, with the `//` and `/* */` comments that JSONC files allow (they
/// cannot appear in plain JSON outside strings, so they cost nothing there).
pub const JSON: Rules = Rules {
    line_comment: "//",
    block_comment: Some(("/*", "*/")),
    triple_quotes: false,
    multiline_quotes: &[],
    string_prefixes: false,
    decorators: false,
    string_keys: true,
    jsx: false,
    lifetimes: false,
    attributes: false,
    macros: false,
    keywords: &["true", "false", "null"],
    extra_keywords: &[],
    definers: &[],
};

/// Label every byte of `text` by `rules`. Spans are in order, touch each
/// other, never empty, and neighbors never share a kind.
pub fn highlight(text: &str, rules: &Rules) -> Vec<Span> {
    Scanner { text, rules, out: Spans::default(), position: 0, define_next: false, jsx_depth: 0 }.run()
}

/// Walks the text once, front to back. Every position it stops at is a char
/// boundary: it only steps over whole chars or ASCII bytes.
struct Scanner<'a> {
    text: &'a str,
    rules: &'a Rules,
    out: Spans,
    position: usize,
    /// The last word was `def`, `class` or `function`: the next word is a name.
    define_next: bool,
    /// How many JSX elements the current position is inside.
    jsx_depth: usize,
}

impl Scanner<'_> {
    fn run(mut self) -> Vec<Span> {
        while self.position < self.text.len() {
            self.step();
        }
        self.out.to(self.text.len(), Kind::Plain);
        self.out.spans
    }

    /// Read one token (or one plain char) at the current position.
    fn step(&mut self) {
        let start = self.position;
        let rest = &self.text[start..];
        let byte = rest.as_bytes()[0];
        if self.rules.attributes && (rest.starts_with("#[") || rest.starts_with("#![")) {
            self.mark(bracket_end(self.text, start), Kind::Name);
        } else if self.rules.lifetimes && byte == b'\'' && is_lifetime(rest) {
            self.mark(word_end(self.text, start + 1), Kind::Name);
        } else if rest.starts_with(self.rules.line_comment) {
            self.mark(line_end(self.text, start), Kind::Comment);
        } else if let Some((open, close)) = self.rules.block_comment.filter(|(open, _)| rest.starts_with(open)) {
            let end =
                find_from(self.text, start + open.len(), close).map_or(self.text.len(), |found| found + close.len());
            self.mark(end, Kind::Comment);
        } else if is_quote(byte, self.rules) {
            self.string(start);
        } else if byte.is_ascii_digit() || (byte == b'.' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_digit)) {
            self.mark(number_end(self.text, start), Kind::Number);
        } else if self.rules.decorators && byte == b'@' && rest[1..].chars().next().is_some_and(is_word_start) {
            let end = dotted_name_end(self.text, start + 1);
            self.mark(end, Kind::Name);
        } else if rest.chars().next().is_some_and(is_word_start) {
            self.word(start);
        } else if byte == b'<' && self.starts_jsx(start) {
            self.define_next = false;
            self.jsx_element();
        } else {
            // Plain punctuation or space. A definer keyword only names the
            // word right after it (`function (` is anonymous).
            if !byte.is_ascii_whitespace() {
                self.define_next = false;
            }
            self.position += rest.chars().next().map_or(1, char::len_utf8);
        }
    }

    /// A word: a keyword, a defined name, a string prefix, or plain.
    fn word(&mut self, start: usize) {
        let end = word_end(self.text, start);
        let word = &self.text[start..end];
        let next = self.text.as_bytes().get(end).copied();
        if self.rules.string_prefixes && is_string_prefix(word) && next.is_some_and(|byte| is_quote(byte, self.rules)) {
            // The prefix is part of the string: `f"..."` is one string.
            self.out.to(start, Kind::Plain);
            self.position = end;
            self.string(start);
            return;
        }
        let kind = if self.define_next {
            self.define_next = false;
            Kind::Name
        } else if self.rules.macros && next == Some(b'!') && self.text.as_bytes().get(end + 1) != Some(&b'=') {
            // `println!(...)` is a macro call; `a != b` is not.
            self.mark(end + 1, Kind::Name);
            return;
        } else if self.rules.keywords.contains(&word) || self.rules.extra_keywords.contains(&word) {
            self.define_next = self.rules.definers.contains(&word);
            Kind::Keyword
        } else {
            Kind::Plain
        };
        self.mark(end, kind);
    }

    /// A string whose opening quote is at the current position; `start` is
    /// where its label begins (before any prefix).
    fn string(&mut self, start: usize) {
        let quote_at = self.position;
        let quote = self.text.as_bytes()[quote_at];
        let triple = [quote; 3];
        let end = if self.rules.triple_quotes && self.text.as_bytes()[quote_at..].starts_with(&triple) {
            closing_quote(self.text, quote_at + 3, &triple, true)
        } else {
            let spans_lines = self.rules.multiline_quotes.contains(&quote);
            closing_quote(self.text, quote_at + 1, &[quote], spans_lines)
        };
        let kind = if self.rules.string_keys && followed_by_colon(self.text, end) { Kind::Key } else { Kind::String };
        self.out.to(start, Kind::Plain);
        self.out.to(end, kind);
        self.position = end;
        self.define_next = false;
    }

    /// Label plain text up to the current position, then up to `end` as
    /// `kind`, and move on to `end`.
    fn mark(&mut self, end: usize, kind: Kind) {
        self.out.to(self.position, Kind::Plain);
        self.out.to(end, kind);
        self.position = end;
    }
}

fn is_quote(byte: u8, rules: &Rules) -> bool {
    byte == b'"' || byte == b'\'' || rules.multiline_quotes.contains(&byte)
}

/// Python's string prefixes: `r`, `b`, `f`, `u` and the pairs `rb`, `br`,
/// `rf`, `fr`, in any case.
fn is_string_prefix(word: &str) -> bool {
    matches!(word.to_ascii_lowercase().as_str(), "r" | "b" | "f" | "u" | "rb" | "br" | "rf" | "fr")
}

fn is_word_start(character: char) -> bool {
    character.is_alphabetic() || character == '_' || character == '$'
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// The end of the word that starts at `start`.
fn word_end(text: &str, start: usize) -> usize {
    text[start..].find(|character| !is_word_char(character)).map_or(text.len(), |offset| start + offset)
}

/// Whether `text` starts with a Rust lifetime (`'a`, `'static`) rather than
/// a char literal (`'a'`).
fn is_lifetime(text: &str) -> bool {
    if !text[1..].chars().next().is_some_and(is_word_start) {
        return false;
    }
    let end = word_end(text, 1);
    text.as_bytes().get(end) != Some(&b'\'')
}

/// Just past the `]` that closes the first `[` at or after `start`
/// (counting nested brackets), or the end of the line when it never closes.
fn bracket_end(text: &str, start: usize) -> usize {
    let line_end = line_end(text, start);
    let mut depth = 0_usize;
    for (offset, byte) in text.as_bytes()[start..line_end].iter().enumerate() {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return start + offset + 1;
                }
            }
            _ => {}
        }
    }
    line_end
}

/// The end of a dotted name like `app.route` (a decorator).
fn dotted_name_end(text: &str, start: usize) -> usize {
    text[start..]
        .find(|character| !is_word_char(character) && character != '.')
        .map_or(text.len(), |offset| start + offset)
}

/// The end of a number: digits, letters (hex digits, `e`, `n`, `j`
/// suffixes), `_` separators and `.`, plus the sign of an exponent (`1e-5`).
fn number_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let hex = bytes[start..].starts_with(b"0x") || bytes[start..].starts_with(b"0X");
    let mut end = start;
    while let Some(&byte) = bytes.get(end) {
        let exponent_sign = !hex && matches!(byte, b'+' | b'-') && matches!(bytes[end - 1], b'e' | b'E');
        if !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || exponent_sign) {
            break;
        }
        end += 1;
    }
    end
}

/// Where the string ends: just past the closing `quote` (backslash escapes
/// skipped). Without one, at the end of the line, or of the text when
/// `spans_lines` is set.
fn closing_quote(text: &str, from: usize, quote: &[u8], spans_lines: bool) -> usize {
    let bytes = text.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            // Never step past the end, and never onto a newline that a
            // one-line string must stop at.
            b'\\' if bytes.get(index + 1).is_some_and(|&next| spans_lines || next != b'\n') => index += 2,
            b'\n' if !spans_lines => return index,
            _ if bytes[index..].starts_with(quote) => return index + quote.len(),
            _ => index += 1,
        }
    }
    // An escape can jump one past the end; land on a char boundary anyway.
    bytes.len()
}

/// True when the next character after `position`, past spaces and tabs, is `:`.
fn followed_by_colon(text: &str, position: usize) -> bool {
    text[position..].trim_start_matches([' ', '\t']).starts_with(':')
}

/// The end of the line that `position` is on (before its newline).
fn line_end(text: &str, position: usize) -> usize {
    text[position..].find('\n').map_or(text.len(), |offset| position + offset)
}

/// Where `needle` next appears at or after `from`.
fn find_from(text: &str, from: usize, needle: &str) -> Option<usize> {
    text.get(from..)?.find(needle).map(|offset| from + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labeled pieces of `text`, skipping plain ones.
    fn colored<'a>(text: &'a str, rules: &Rules) -> Vec<(&'a str, Kind)> {
        highlight(text, rules)
            .into_iter()
            .filter(|span| span.kind != Kind::Plain)
            .map(|span| (&text[span.range], span.kind))
            .collect()
    }

    /// Spans cover the whole text, in order, with no gaps or repeats.
    fn assert_covers(text: &str, rules: &Rules) {
        let spans = highlight(text, rules);
        let mut end = 0;
        for pair in spans.windows(2) {
            assert_ne!(pair[0].kind, pair[1].kind, "{text:?}");
        }
        for span in &spans {
            assert_eq!(span.range.start, end, "{text:?}");
            assert!(span.range.end > span.range.start, "{text:?}");
            end = span.range.end;
        }
        assert_eq!(end, text.len(), "{text:?}");
    }

    #[test]
    fn python_keywords_names_strings_numbers_comments() {
        let text = "@app.route('/')\ndef hello(n=1.5e-3):  # greet\n    return f\"hi {n}\" if n else None\n";
        assert_eq!(
            colored(text, &PYTHON),
            vec![
                ("@app.route", Kind::Name),
                ("'/'", Kind::String),
                ("def", Kind::Keyword),
                ("hello", Kind::Name),
                ("1.5e-3", Kind::Number),
                ("# greet", Kind::Comment),
                ("return", Kind::Keyword),
                ("f\"hi {n}\"", Kind::String),
                ("if", Kind::Keyword),
                ("else", Kind::Keyword),
                ("None", Kind::Keyword),
            ]
        );
    }

    #[test]
    fn python_triple_quoted_strings_span_lines() {
        let text = "x = \"\"\"one\n'two'\n\"\"\"\ny = 2";
        assert_eq!(colored(text, &PYTHON), vec![("\"\"\"one\n'two'\n\"\"\"", Kind::String), ("2", Kind::Number)]);
    }

    #[test]
    fn words_containing_keywords_stay_plain() {
        assert_eq!(colored("format = classify(inner)", &PYTHON), vec![]);
        assert_eq!(colored("const r2 = rb", &JAVASCRIPT), vec![("const", Kind::Keyword)]);
    }

    #[test]
    fn javascript_comments_templates_and_functions() {
        let text = "/* a\nb */ function add(a, b) { // sum\n  return `${a}\n${b}` + 'x' + 0xFF;\n}";
        assert_eq!(
            colored(text, &JAVASCRIPT),
            vec![
                ("/* a\nb */", Kind::Comment),
                ("function", Kind::Keyword),
                ("add", Kind::Name),
                ("// sum", Kind::Comment),
                ("return", Kind::Keyword),
                ("`${a}\n${b}`", Kind::String),
                ("'x'", Kind::String),
                ("0xFF", Kind::Number),
            ]
        );
    }

    #[test]
    fn typescript_types_interfaces_and_decorators() {
        let text = "@Component\nexport interface Note { title: string; type: 'a' }\nenum Kind {}";
        assert_eq!(
            colored(text, &TYPESCRIPT),
            vec![
                ("@Component", Kind::Name),
                ("export", Kind::Keyword),
                ("interface", Kind::Keyword),
                ("Note", Kind::Name),
                ("string", Kind::Keyword),
                ("type", Kind::Keyword),
                ("'a'", Kind::String),
                ("enum", Kind::Keyword),
                ("Kind", Kind::Name),
            ]
        );
        assert_eq!(colored("let x: string", &JAVASCRIPT), vec![("let", Kind::Keyword)]);
    }

    #[test]
    fn rust_items_lifetimes_attributes_and_macros() {
        let text = "#[derive(Debug)]\npub fn first<'a>(s: &'a str) -> char {\n    println!(\"{s}\");\n    if a != b { 'x' } else { '\\n' }\n}";
        assert_eq!(
            colored(text, &RUST),
            vec![
                ("#[derive(Debug)]", Kind::Name),
                ("pub", Kind::Keyword),
                ("fn", Kind::Keyword),
                ("first", Kind::Name),
                ("'a", Kind::Name),
                ("'a", Kind::Name),
                ("str", Kind::Keyword),
                ("char", Kind::Keyword),
                ("println!", Kind::Name),
                ("\"{s}\"", Kind::String),
                ("if", Kind::Keyword),
                ("'x'", Kind::String),
                ("else", Kind::Keyword),
                ("'\\n'", Kind::String),
            ]
        );
    }

    #[test]
    fn anonymous_function_names_nothing() {
        assert_eq!(
            colored("const f = function (x) { x }", &JAVASCRIPT),
            vec![("const", Kind::Keyword), ("function", Kind::Keyword)]
        );
    }

    #[test]
    fn unclosed_plain_string_stops_at_line_end() {
        let text = "s = 'open\nnext = 1";
        assert_eq!(colored(text, &JAVASCRIPT), vec![("'open", Kind::String), ("1", Kind::Number)]);
    }

    #[test]
    fn json_keys_values_and_literals() {
        let text = "{\"name\" : \"Ada\", \"age\": 36, \"ok\": true, \"tags\": [null, -1.5]}";
        assert_eq!(
            colored(text, &JSON),
            vec![
                ("\"name\"", Kind::Key),
                ("\"Ada\"", Kind::String),
                ("\"age\"", Kind::Key),
                ("36", Kind::Number),
                ("\"ok\"", Kind::Key),
                ("true", Kind::Keyword),
                ("\"tags\"", Kind::Key),
                ("null", Kind::Keyword),
                ("1.5", Kind::Number),
            ]
        );
    }

    #[test]
    fn escaped_quotes_stay_inside_strings() {
        assert_eq!(colored(r#""a\"b": 1"#, &JSON), vec![(r#""a\"b""#, Kind::Key), ("1", Kind::Number)]);
    }

    #[test]
    fn spans_cover_odd_input() {
        for text in ["", "'", "\"\\", "x = '\\", "/*", "@", "1e", "é = 'ü' # ö", "f'", "\"\"\"\\", "`\\\n"] {
            for rules in [&PYTHON, &JAVASCRIPT, &TYPESCRIPT, &TSX, &RUST, &JSON] {
                assert_covers(text, rules);
            }
        }
    }
}
