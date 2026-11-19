//! Decode RTF body text while ignoring markup and embedded objects.

use rclip_rtf::{Parser, RunText};

use super::{MAX_TEXT_BYTES, limit_text};

/// Read text runs in document order and stop at the preview output limit.
pub(super) fn text(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let parser = Parser::new(bytes).map_err(|error| format!("Cannot read this RTF document: {error}"))?;
    let mut text = String::new();
    for run in parser {
        let run = run.map_err(|error| format!("Cannot read this RTF document: {error}"))?;
        match run.text {
            RunText::Text(value) => text.push_str(value),
            RunText::Char(value) => text.push(value),
            RunText::ParagraphBreak | RunText::LineBreak => text.push('\n'),
        }
        if text.len() > MAX_TEXT_BYTES {
            return Ok(limit_text(text));
        }
    }
    Ok(text)
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
    fn skips_embedded_binary_and_rejects_invalid_input() {
        assert_eq!(text(br"{\rtf1 Before{\pict\bin3 {x}}After}").unwrap(), "BeforeAfter");
        assert!(text(b"<html>Not RTF</html>").is_err());
        assert!(text(br"{\rtf1 Unclosed").is_err());
    }
}
