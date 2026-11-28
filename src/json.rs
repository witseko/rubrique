//! JSON values: parse the replies of `OpenRig`, Chromium and Sublime Text,
//! and write small messages and task files (RFC 8259).
//!
//! Reading is forgiving in the way the callers need: indexing a missing key,
//! or a value that is not an object, gives `null` instead of failing.

use std::fmt::{self, Write as _};
use std::ops::Index;

/// Deepest nesting of arrays and objects read.
const MAX_DEPTH: usize = 128;
/// Largest integer a JSON number holds exactly as `f64` (2^53).
const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;

/// A JSON value. Object keys keep their order; when a key repeats, the
/// last value counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// The value indexing gives for a missing key.
static NULL: Value = Value::Null;

impl Value {
    /// An object with these keys and values, in this order.
    pub fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
        Value::Object(entries.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
    }

    /// The value at `key` when this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().rev().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    /// The value at a JSON Pointer (RFC 6901) such as `/settings/name`.
    pub fn pointer(&self, pointer: &str) -> Option<&Value> {
        if pointer.is_empty() {
            return Some(self);
        }
        let mut value = self;
        for token in pointer.strip_prefix('/')?.split('/') {
            let token = token.replace("~1", "/").replace("~0", "~");
            value = match value {
                Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
                _ => value.get(&token)?,
            };
        }
        Some(value)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(number) => Some(*number),
            _ => None,
        }
    }

    /// The number when it is a whole number from 0 to 2^53.
    pub fn as_u64(&self) -> Option<u64> {
        let number = self.as_f64()?;
        if number.fract() == 0.0 && (0.0..=MAX_EXACT_INTEGER).contains(&number) {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a whole number from 0 to 2^53 converts exactly"
            )]
            Some(number as u64)
        } else {
            None
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
}

impl Index<&str> for Value {
    type Output = Value;

    /// The value at `key`, or `null` when it is missing.
    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }
}

impl PartialEq<&str> for Value {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == Some(*other)
    }
}

impl PartialEq<bool> for Value {
    fn eq(&self, other: &bool) -> bool {
        self.as_bool() == Some(*other)
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::String(text.to_string())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::String(text)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<f64> for Value {
    fn from(number: f64) -> Self {
        Value::Number(number)
    }
}

impl From<u32> for Value {
    fn from(number: u32) -> Self {
        Value::Number(f64::from(number))
    }
}

impl From<u64> for Value {
    fn from(number: u64) -> Self {
        #[expect(clippy::cast_precision_loss, reason = "message IDs and sizes stay far below 2^53")]
        Value::Number(number as f64)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Value::Null, Into::into)
    }
}

impl fmt::Display for Value {
    /// Compact JSON text.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => formatter.write_str("null"),
            Value::Bool(value) => write!(formatter, "{value}"),
            Value::Number(number) => write_number(formatter, *number),
            Value::String(text) => write_string(formatter, text),
            Value::Array(items) => {
                formatter.write_char('[')?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        formatter.write_char(',')?;
                    }
                    write!(formatter, "{item}")?;
                }
                formatter.write_char(']')
            }
            Value::Object(entries) => {
                formatter.write_char('{')?;
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        formatter.write_char(',')?;
                    }
                    write_string(formatter, key)?;
                    write!(formatter, ":{value}")?;
                }
                formatter.write_char('}')
            }
        }
    }
}

/// Whole numbers without a fraction (`3`, not `3.0`); JSON has no NaN or
/// infinity, so those become `null`.
fn write_number(formatter: &mut fmt::Formatter<'_>, number: f64) -> fmt::Result {
    if !number.is_finite() {
        formatter.write_str("null")
    } else if number.fract() == 0.0 && number.abs() <= MAX_EXACT_INTEGER {
        #[expect(clippy::cast_possible_truncation, reason = "a whole number within 2^53 converts exactly")]
        let whole = number as i64;
        write!(formatter, "{whole}")
    } else {
        write!(formatter, "{number}")
    }
}

/// A quoted string with `"`, `\` and control characters escaped.
fn write_string(formatter: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    formatter.write_char('"')?;
    for character in text.chars() {
        match character {
            '"' => formatter.write_str("\\\"")?,
            '\\' => formatter.write_str("\\\\")?,
            '\n' => formatter.write_str("\\n")?,
            '\r' => formatter.write_str("\\r")?,
            '\t' => formatter.write_str("\\t")?,
            control if u32::from(control) < 0x20 => write!(formatter, "\\u{:04x}", u32::from(control))?,
            other => formatter.write_char(other)?,
        }
    }
    formatter.write_char('"')
}

/// Parse JSON text.
///
/// # Errors
///
/// The text is not JSON, nests deeper than 128 levels, or has text after
/// the value. The message names the byte offset.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser { bytes: text.as_bytes(), position: 0 };
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err(parser.error("unexpected text after the value"));
    }
    Ok(value)
}

/// Parse JSON bytes, which must be UTF-8.
///
/// # Errors
///
/// As [`parse`], or the bytes are not UTF-8.
pub fn parse_bytes(bytes: &[u8]) -> Result<Value, String> {
    parse(std::str::from_utf8(bytes).map_err(|_| "JSON text is not UTF-8".to_string())?)
}

/// A pass over JSON text.
struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.position)
    }

    fn skip_whitespace(&mut self) {
        while self.bytes.get(self.position).is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    /// Move past `expected` when it comes next.
    fn eat(&mut self, expected: u8) -> bool {
        self.skip_whitespace();
        let found = self.bytes.get(self.position) == Some(&expected);
        if found {
            self.position += 1;
        }
        found
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("JSON nests too deeply"));
        }
        self.skip_whitespace();
        match self.bytes.get(self.position) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of JSON")),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected word"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.position += 1;
        let mut entries = Vec::new();
        if self.eat(b'}') {
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_whitespace();
            if self.bytes.get(self.position) != Some(&b'"') {
                return Err(self.error("expected a key"));
            }
            let key = self.string()?;
            if !self.eat(b':') {
                return Err(self.error("expected `:`"));
            }
            entries.push((key, self.value(depth + 1)?));
            if self.eat(b'}') {
                return Ok(Value::Object(entries));
            }
            if !self.eat(b',') {
                return Err(self.error("expected `,` or `}`"));
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.position += 1;
        let mut items = Vec::new();
        if self.eat(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            if self.eat(b']') {
                return Ok(Value::Array(items));
            }
            if !self.eat(b',') {
                return Err(self.error("expected `,` or `]`"));
            }
        }
    }

    /// Read a string at its opening quote.
    fn string(&mut self) -> Result<String, String> {
        self.position += 1;
        let mut text = String::new();
        loop {
            let start = self.position;
            while self.bytes.get(self.position).is_some_and(|byte| !matches!(byte, b'"' | b'\\') && *byte >= 0x20) {
                self.position += 1;
            }
            // The input is a `str` and the run stops at ASCII, so it is whole characters.
            text.push_str(
                std::str::from_utf8(&self.bytes[start..self.position]).map_err(|_| self.error("broken UTF-8"))?,
            );
            match self.bytes.get(self.position) {
                Some(b'"') => {
                    self.position += 1;
                    return Ok(text);
                }
                Some(b'\\') => {
                    self.position += 1;
                    self.escape(&mut text)?;
                }
                Some(_) => return Err(self.error("control character in a string")),
                None => return Err(self.error("unclosed string")),
            }
        }
    }

    /// Read the escape after a backslash.
    fn escape(&mut self, text: &mut String) -> Result<(), String> {
        let Some(&byte) = self.bytes.get(self.position) else { return Err(self.error("unclosed string")) };
        self.position += 1;
        let character = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let first = self.hex4()?;
                if (0xd800..0xdc00).contains(&first) {
                    if !self.bytes[self.position..].starts_with(b"\\u") {
                        return Err(self.error("unpaired surrogate"));
                    }
                    self.position += 2;
                    let second = self.hex4()?;
                    char::decode_utf16([first, second])
                        .next()
                        .and_then(Result::ok)
                        .ok_or_else(|| self.error("unpaired surrogate"))?
                } else {
                    char::from_u32(u32::from(first)).ok_or_else(|| self.error("unpaired surrogate"))?
                }
            }
            _ => return Err(self.error("unknown escape")),
        };
        text.push(character);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let digits = self.bytes.get(self.position..self.position + 4).ok_or_else(|| self.error("short \\u escape"))?;
        let value = std::str::from_utf8(digits)
            .ok()
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|digits| u16::from_str_radix(digits, 16).ok())
            .ok_or_else(|| self.error("bad \\u escape"))?;
        self.position += 4;
        Ok(value)
    }

    /// Read a number by the JSON grammar.
    fn number(&mut self) -> Result<Value, String> {
        let start = self.position;
        let digits = |parser: &mut Self| {
            let begin = parser.position;
            while parser.bytes.get(parser.position).is_some_and(u8::is_ascii_digit) {
                parser.position += 1;
            }
            parser.position > begin
        };
        if self.bytes.get(self.position) == Some(&b'-') {
            self.position += 1;
        }
        if self.bytes.get(self.position) == Some(&b'0') {
            self.position += 1;
        } else if !digits(self) {
            return Err(self.error("bad number"));
        }
        if self.bytes.get(self.position) == Some(&b'.') {
            self.position += 1;
            if !digits(self) {
                return Err(self.error("bad number"));
            }
        }
        if matches!(self.bytes.get(self.position), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.bytes.get(self.position), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !digits(self) {
                return Err(self.error("bad number"));
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position]).map_err(|_| self.error("bad number"))?;
        text.parse().map(Value::Number).map_err(|_| self.error("bad number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_reads_nested_values() {
        let value =
            parse(r#" {"a": [1, -2.5e2, true, null], "b": {"c": "x\"\u00e9\ud83d\ude00"}, "a": [3]} "#).unwrap();
        assert_eq!(value["a"], Value::Array(vec![Value::Number(3.0)]));
        assert_eq!(value["b"]["c"], "x\"é😀");
        assert_eq!(value.pointer("/b/c").and_then(Value::as_str), Some("x\"é😀"));
        assert_eq!(value["missing"]["deeper"], Value::Null);
        assert!(value.is_object());
        assert_eq!(parse("[1, 2]").unwrap().pointer("/1").and_then(Value::as_u64), Some(2));
        assert_eq!(parse("1.5").unwrap().as_u64(), None);
        assert_eq!(parse("-1").unwrap().as_u64(), None);
    }

    #[test]
    fn rejects_text_that_is_not_json() {
        for broken in
            ["", "{", "[1,]", "{\"a\" 1}", "01", "1.", "-", "\"\\x\"", "\"\\ud800\"", "\"a\nb\"", "nul", "1 2", "{1:2}"]
        {
            assert!(parse(broken).is_err(), "{broken:?}");
        }
        assert!(parse(&"[".repeat(MAX_DEPTH + 2)).is_err());
        assert!(parse_bytes(b"\"\xff\"").is_err());
    }

    #[test]
    fn writes_compact_json_that_reads_back() {
        let value = Value::object([
            ("id", Value::from(7_u64)),
            ("half", Value::from(0.5)),
            ("text", Value::from("a\"b\\c\nd\u{1}")),
            ("none", Value::from(None::<&str>)),
            ("list", Value::Array(vec![Value::from(true), Value::object([])])),
        ]);
        let text = value.to_string();
        assert_eq!(text, r#"{"id":7,"half":0.5,"text":"a\"b\\c\nd\u0001","none":null,"list":[true,{}]}"#);
        assert_eq!(parse(&text).unwrap(), value);
        assert_eq!(Value::from(f64::NAN).to_string(), "null");
    }
}
