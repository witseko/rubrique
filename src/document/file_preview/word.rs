//! Extract plain text from Word files without running macros or loading linked resources.

use std::io;
use std::process::Command;
use std::sync::Arc;

use super::converter;
use super::package::read_part;
use super::xml::{Event, XmlReader};
use super::zip_archive::ZipArchive;
use super::{MAX_TEXT_BYTES, limit_text};

/// The file signature of the compound container used by Word 97 through 2003.
const COMPOUND_FILE_SIGNATURE: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
/// `WordprocessingML`, transitional and strict.
const WORD: &[&str] = &[
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "http://purl.oclc.org/ooxml/wordprocessingml/main",
];

/// Extract the main document part without opening links or other archive entries.
pub(super) fn docx_text(bytes: &[u8]) -> Result<String, String> {
    let archive = ZipArchive::new(bytes)?;
    let xml = read_part(&archive, "word/document.xml")?;
    extract_docx_text(&xml)
}

/// Read text runs and paragraph breaks from the `WordprocessingML` document.
fn extract_docx_text(xml: &[u8]) -> Result<String, String> {
    let mut reader = XmlReader::new(xml)?;
    let mut text = String::new();
    let mut in_text = false;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.in_namespace(WORD) => match element.name.local.as_str() {
                "t" => in_text = true,
                "tab" => text.push('\t'),
                "br" | "cr" => text.push('\n'),
                _ => {}
            },
            Event::End(name) if name.in_namespace(WORD) => match name.local.as_str() {
                "t" => in_text = false,
                "p" => text.push('\n'),
                _ => {}
            },
            Event::Text(value) if in_text => text.push_str(&value),
            Event::Eof => return Ok(text),
            _ => {}
        }
        if text.len() > MAX_TEXT_BYTES {
            return Ok(limit_text(text));
        }
    }
}

/// Extract text from a Word 97 through 2003 document with the platform converter.
pub(super) fn legacy_text(bytes: Arc<Vec<u8>>) -> Result<String, String> {
    if !bytes.starts_with(COMPOUND_FILE_SIGNATURE) {
        return Err("This file is not a Word 97-2003 document.".into());
    }
    let mut command = legacy_command();
    let output = converter::run(&mut command, bytes, MAX_TEXT_BYTES, "Word converter").map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound && cfg!(target_os = "linux") {
            "Install antiword to preview legacy Word (.doc) files.".into()
        } else {
            error.to_string()
        }
    })?;
    Ok(limit_text(String::from_utf8_lossy(&output).into_owned()))
}

/// Configure the converter to read a document snapshot from standard input.
fn legacy_command() -> Command {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/textutil");
        command.args(["-convert", "txt", "-format", "doc", "-stdout", "-noload", "-stdin"]);
        command
    } else {
        let mut command = Command::new("antiword");
        command.args(["-m", "UTF-8.txt", "-w", "0", "-"]);
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_docx_paragraphs_entities_and_tabs() {
        let xml = br#"<doc:document xmlns:doc="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <doc:body><doc:p><doc:r><doc:t>One &amp; two</doc:t><doc:tab/><doc:t>&#x33;</doc:t></doc:r></doc:p>
            <doc:p><doc:r><doc:t>Next line</doc:t></doc:r></doc:p></doc:body></doc:document>"#;
        let bytes = super::super::zip_archive::tests::stored_zip(&[("word/document.xml", xml)]);

        assert_eq!(docx_text(&bytes).unwrap(), "One & two\t3\nNext line\n");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_text_from_a_legacy_word_document() {
        let bytes = Arc::new(include_bytes!("fixtures/legacy.doc").to_vec());
        assert_eq!(legacy_text(bytes).unwrap().trim(), "Text from a legacy Word document.");
    }

    #[test]
    fn rejects_invalid_document_format() {
        assert!(docx_text(b"plain text").is_err());
        assert!(legacy_text(Arc::new(b"plain text".to_vec())).is_err());
    }
}
