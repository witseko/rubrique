//! Decode RTF body text while ignoring markup and embedded objects.
//!
//! RTF is plain text: `{` and `}` open and close groups, `\word123 ` is a
//! control word with an optional number, and `\'e9` is one byte in the
//! current code page. Groups such as the font table, pictures and field
//! instructions are "destinations" that hold no body text, so their content
//! is skipped.

mod codepage;

use super::{MAX_TEXT_BYTES, limit_text};

/// Deepest group nesting read; real documents stay far below it.
const MAX_DEPTH: usize = 1_000;
/// Code page when the document names none.
const DEFAULT_CODEPAGE: u16 = 1252;

/// Destinations whose text is not part of the body.
const SKIPPED_DESTINATIONS: &[&str] = &[
    "colortbl",
    "datafield",
    "datastore",
    "filetbl",
    "fldinst",
    "footer",
    "footerf",
    "footerl",
    "footerr",
    "footnote",
    "generator",
    "header",
    "headerf",
    "headerl",
    "headerr",
    "info",
    "latentstyles",
    "listoverridetable",
    "listtable",
    "object",
    "objdata",
    "pict",
    "revtbl",
    "rsidtbl",
    "stylesheet",
    "themedata",
    "xmlnstbl",
];

/// Read text runs in document order and stop at the preview output limit.
pub(super) fn text(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if !bytes.starts_with(b"{\\rtf") {
        return Err("Cannot read this RTF document: it does not start with {\\rtf.".into());
    }
    Reader::new(bytes).read().map_err(|error| format!("Cannot read this RTF document: {error}"))
}

/// What the text of a group is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Body,
    FontTable,
    Skipped,
}

/// State that a group inherits from its parent and drops when it closes.
#[derive(Clone, Copy, Debug)]
struct Group {
    destination: Destination,
    codepage: u16,
    /// Fallback characters after each `\u` (the `\uc` value).
    unicode_fallback: usize,
}

/// A pass over the document bytes.
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    groups: Vec<Group>,
    text: String,
    /// The `\ansicpg` code page, used where a font names none.
    document_codepage: u16,
    /// Code page of each font number, from the font table.
    fonts: Vec<(i32, Option<u16>)>,
    /// The font being described in the font table.
    font_in_table: Option<i32>,
    /// Fallback characters still to skip after a `\u`.
    fallback_left: usize,
    /// The first half of a UTF-16 surrogate pair from `\u`.
    high_surrogate: Option<u16>,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            groups: Vec::new(),
            text: String::new(),
            document_codepage: DEFAULT_CODEPAGE,
            fonts: Vec::new(),
            font_in_table: None,
            fallback_left: 0,
            high_surrogate: None,
        }
    }

    /// Read up to the end of the outermost group.
    fn read(mut self) -> Result<String, String> {
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            match byte {
                b'{' => {
                    if self.groups.len() >= MAX_DEPTH {
                        return Err("its groups nest too deeply.".into());
                    }
                    let group = self.groups.last().copied().unwrap_or(Group {
                        destination: Destination::Body,
                        codepage: DEFAULT_CODEPAGE,
                        unicode_fallback: 1,
                    });
                    self.groups.push(group);
                    self.fallback_left = 0;
                }
                b'}' => {
                    self.groups.pop();
                    self.fallback_left = 0;
                    if self.groups.is_empty() {
                        return Ok(self.text);
                    }
                }
                b'\\' => self.control(),
                b'\r' | b'\n' => {}
                _ => self.push_byte(byte),
            }
            if self.text.len() > MAX_TEXT_BYTES {
                return Ok(limit_text(self.text));
            }
        }
        Err("it ends inside a group.".into())
    }

    /// The innermost open group (there is one while `read` runs).
    fn group(&mut self) -> &mut Group {
        let last = self.groups.len() - 1;
        &mut self.groups[last]
    }

    /// Read a control word or control symbol after its backslash.
    fn control(&mut self) {
        let Some(&first) = self.bytes.get(self.position) else { return };
        if !first.is_ascii_alphabetic() {
            self.position += 1;
            self.symbol(first);
            return;
        }
        let name_start = self.position;
        while self.bytes.get(self.position).is_some_and(u8::is_ascii_alphabetic) && self.position - name_start < 32 {
            self.position += 1;
        }
        let name = String::from_utf8_lossy(&self.bytes[name_start..self.position]).into_owned();
        let negative = self.bytes.get(self.position) == Some(&b'-');
        let digits_start = self.position + usize::from(negative);
        let mut digits_end = digits_start;
        while self.bytes.get(digits_end).is_some_and(u8::is_ascii_digit) && digits_end - digits_start < 10 {
            digits_end += 1;
        }
        let parameter = if digits_end > digits_start {
            self.position = digits_end;
            let value = std::str::from_utf8(&self.bytes[digits_start..digits_end])
                .ok()
                .and_then(|digits| digits.parse::<i64>().ok())
                .unwrap_or(0);
            let value = if negative { -value } else { value };
            Some(i32::try_from(value).unwrap_or(0))
        } else {
            None
        };
        if self.bytes.get(self.position) == Some(&b' ') {
            self.position += 1;
        }
        self.word(&name, parameter);
    }

    /// Act on a control symbol such as `\'e9`, `\~` or `\{`.
    fn symbol(&mut self, symbol: u8) {
        match symbol {
            b'\'' => {
                let hex = self.bytes.get(self.position..self.position + 2);
                let value =
                    hex.and_then(|hex| std::str::from_utf8(hex).ok()).and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(value) = value {
                    self.position += 2;
                    self.push_byte(value);
                }
            }
            b'*' => self.group().destination = Destination::Skipped,
            b'\\' | b'{' | b'}' => self.push_byte(symbol),
            b'~' => self.push_counted('\u{a0}'),
            b'_' => self.push_counted('\u{2011}'),
            b'\r' | b'\n' => self.push('\n'),
            _ => {}
        }
    }

    /// Act on a control word.
    fn word(&mut self, name: &str, parameter: Option<i32>) {
        if name == "bin" {
            let length = usize::try_from(parameter.unwrap_or(0)).unwrap_or(0);
            self.position = self.position.saturating_add(length).min(self.bytes.len());
            return;
        }
        let destination = self.group().destination;
        if destination == Destination::Skipped {
            return;
        }
        if destination == Destination::FontTable {
            self.font_table_word(name, parameter);
            return;
        }
        match name {
            "par" | "line" | "sect" | "page" | "row" => self.push('\n'),
            "tab" | "cell" => self.push('\t'),
            "emdash" => self.push('\u{2014}'),
            "endash" => self.push('\u{2013}'),
            "emspace" | "enspace" | "qmspace" => self.push(' '),
            "bullet" => self.push('\u{2022}'),
            "lquote" => self.push('\u{2018}'),
            "rquote" => self.push('\u{2019}'),
            "ldblquote" => self.push('\u{201c}'),
            "rdblquote" => self.push('\u{201d}'),
            "u" => {
                if let Some(code) = parameter {
                    self.push_unicode(code);
                }
            }
            "uc" => self.group().unicode_fallback = usize::try_from(parameter.unwrap_or(1)).unwrap_or(1).min(16),
            "ansi" => self.set_document_codepage(1252),
            "mac" => self.set_document_codepage(10000),
            "pc" => self.set_document_codepage(437),
            "pca" => self.set_document_codepage(850),
            "ansicpg" => {
                if let Some(codepage) = parameter.and_then(|value| u16::try_from(value).ok()) {
                    self.set_document_codepage(codepage);
                }
            }
            "fonttbl" => self.group().destination = Destination::FontTable,
            "f" => {
                let font = parameter.unwrap_or(0);
                let codepage =
                    self.fonts.iter().find(|(number, _)| *number == font).and_then(|(_, codepage)| *codepage);
                self.group().codepage = codepage.unwrap_or(self.document_codepage);
            }
            _ if SKIPPED_DESTINATIONS.contains(&name) => self.group().destination = Destination::Skipped,
            _ => {}
        }
    }

    /// Note each font's code page from `\fN` with `\fcharsetN` or `\cpgN`.
    fn font_table_word(&mut self, name: &str, parameter: Option<i32>) {
        let value = parameter.unwrap_or(0);
        match name {
            "f" => self.font_in_table = Some(value),
            "fcharset" | "cpg" => {
                let codepage = if name == "cpg" { u16::try_from(value).ok() } else { charset_codepage(value) };
                if let Some(font) = self.font_in_table {
                    self.fonts.retain(|(number, _)| *number != font);
                    self.fonts.push((font, codepage));
                }
            }
            _ => {}
        }
    }

    fn set_document_codepage(&mut self, codepage: u16) {
        self.document_codepage = codepage;
        self.group().codepage = codepage;
    }

    /// Add the character for `\uN`, joining surrogate pairs, then skip its
    /// fallback characters.
    fn push_unicode(&mut self, code: i32) {
        // RTF writes code points above 32767 as negative numbers.
        let unit = u16::try_from(if code < 0 { code + 65_536 } else { code }).unwrap_or(0xfffd);
        let character = match (self.high_surrogate.take(), unit) {
            (_, 0xd800..=0xdbff) => {
                self.high_surrogate = Some(unit);
                None
            }
            (Some(high), 0xdc00..=0xdfff) => char::decode_utf16([high, unit]).next().and_then(Result::ok),
            (_, unit) => Some(char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}')),
        };
        if let Some(character) = character {
            self.push(character);
        }
        self.fallback_left = self.group().unicode_fallback;
    }

    /// Add one byte of text, decoded in the group's code page.
    fn push_byte(&mut self, byte: u8) {
        let character = if byte.is_ascii() {
            char::from(byte)
        } else {
            let codepage = self.group().codepage;
            codepage::upper_half(codepage).map_or('\u{fffd}', |table| table[usize::from(byte - 0x80)])
        };
        self.push_counted(character);
    }

    /// Add a character that may be the fallback of a `\u` and then skipped.
    fn push_counted(&mut self, character: char) {
        if self.fallback_left > 0 {
            self.fallback_left -= 1;
            return;
        }
        self.push(character);
    }

    /// Add a character when the group holds body text.
    fn push(&mut self, character: char) {
        if self.group().destination == Destination::Body {
            self.text.push(character);
        }
    }
}

/// The code page of an RTF `\fcharset`; `None` means the document's own.
fn charset_codepage(charset: i32) -> Option<u16> {
    match charset {
        77 => Some(10000),
        128 => Some(932),
        129 => Some(949),
        134 => Some(936),
        136 => Some(950),
        161 => Some(1253),
        162 => Some(1254),
        163 => Some(1258),
        177 => Some(1255),
        178 => Some(1256),
        186 => Some(1257),
        204 => Some(1251),
        222 => Some(874),
        238 => Some(1250),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_codepage_characters_unicode_breaks_and_field_labels() {
        let rtf = br#"{\rtf1\ansi\ansicpg1252\uc1{\fonttbl{\f0 Arial;}}
            Caf\'e9 \u9731?\par {\field{\*\fldinst HYPERLINK "https://example.org"}{\fldrslt Link}}\line Next}"#;
        assert_eq!(text(rtf).unwrap().trim(), "Café ☃\nLink\nNext");
    }

    #[test]
    fn reads_non_western_legacy_code_pages() {
        assert_eq!(text(br"{\rtf1\ansi\ansicpg1251 \'cf\'f0\'e8\'e2\'e5\'f2}").unwrap(), "Привет");
    }

    #[test]
    fn uses_the_code_page_of_the_current_font() {
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0\fcharset0 Arial;}{\f1\fcharset161 Greek;}}\f1 \'e1{\f0 \'e9}\'e2}";
        assert_eq!(text(rtf).unwrap(), "αéβ");
    }

    #[test]
    fn joins_surrogate_pairs_and_skips_longer_fallbacks() {
        assert_eq!(text(br"{\rtf1\uc2\u-10179\'3f\'3f\u-8704\'3f\'3f!}").unwrap(), "😀!");
    }

    #[test]
    fn skips_embedded_binary_and_rejects_invalid_input() {
        assert_eq!(text(br"{\rtf1 Before{\pict\bin3 {x}}After}").unwrap(), "BeforeAfter");
        assert!(text(b"<html>Not RTF</html>").is_err());
        assert!(text(br"{\rtf1 Unclosed").is_err());
        assert!(text(&[b"{\\rtf1".as_slice(), &[b'{'; MAX_DEPTH]].concat()).is_err());
    }
}
