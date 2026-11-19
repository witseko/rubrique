//! Extract plain text from Word files without running macros or loading linked resources.

use std::io::{self, Cursor, Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use quick_xml::NsReader;
use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

/// Maximum number of bytes of text shown in a Word preview.
const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Maximum size of the uncompressed DOCX document body.
const MAX_XML_BYTES: u64 = 16 * 1024 * 1024;
/// Time limit for the legacy Word converter.
const CONVERSION_TIMEOUT: Duration = Duration::from_secs(10);
/// The file signature of the compound container used by Word 97 through 2003.
const COMPOUND_FILE_SIGNATURE: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";

/// Extract the main document part without opening links or other archive entries.
pub(super) fn docx_text(bytes: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let body = archive.by_name("word/document.xml").map_err(|error| error.to_string())?;
    if body.size() > MAX_XML_BYTES {
        return Err("The Word document body exceeds the preview limit.".into());
    }
    let mut xml = Vec::new();
    body.take(MAX_XML_BYTES + 1).read_to_end(&mut xml).map_err(|error| error.to_string())?;
    if xml.len() as u64 > MAX_XML_BYTES {
        return Err("The Word document body exceeds the preview limit.".into());
    }
    extract_docx_text(&xml).map_err(|error| error.to_string())
}

/// Read text runs and paragraph breaks from the `WordprocessingML` document.
fn extract_docx_text(xml: &[u8]) -> Result<String, quick_xml::Error> {
    let mut reader = NsReader::from_reader(xml);
    let mut text = String::new();
    let mut in_text = false;
    loop {
        let (namespace, event) = reader.read_resolved_event()?;
        let word_element = is_word_element(&namespace);
        match event {
            Event::Start(tag) if word_element && tag.local_name().as_ref() == "t" => in_text = true,
            Event::End(tag) if word_element => match tag.local_name().as_ref() {
                "t" => in_text = false,
                "p" => text.push('\n'),
                _ => {}
            },
            Event::Empty(tag) if word_element => match tag.local_name().as_ref() {
                "tab" => text.push('\t'),
                "br" | "cr" => text.push('\n'),
                _ => {}
            },
            Event::Text(value) if in_text => text.push_str(&value.xml10_content()),
            Event::GeneralRef(reference) if in_text => {
                if let Some(character) = reference.resolve_char_ref()? {
                    text.push(character);
                } else if let Some(entity) = resolve_xml_entity(reference.as_ref()) {
                    text.push_str(entity);
                }
            }
            Event::Eof => return Ok(text),
            _ => {}
        }
        if text.len() > MAX_TEXT_BYTES {
            return Ok(limit_text(text));
        }
    }
}

/// Recognize both transitional and strict `WordprocessingML` namespace identifiers.
fn is_word_element(namespace: &ResolveResult<'_>) -> bool {
    matches!(namespace, ResolveResult::Bound(value) if matches!(
        value.as_ref(),
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            | "http://purl.oclc.org/ooxml/wordprocessingml/main"
    ))
}

/// Extract text from a Word 97 through 2003 document with the platform converter.
pub(super) fn legacy_text(bytes: Arc<Vec<u8>>) -> Result<String, String> {
    if !bytes.starts_with(COMPOUND_FILE_SIGNATURE) {
        return Err("This file is not a Word 97-2003 document.".into());
    }
    let mut command = legacy_command();
    let output = read_converter(&mut command, bytes).map_err(|error| {
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

/// Collect bounded converter output and terminate the converter if it stops responding.
fn read_converter(command: &mut Command, input: Arc<Vec<u8>>) -> io::Result<Vec<u8>> {
    let mut child = command.stdin(Stdio::piped()).stderr(Stdio::null()).stdout(Stdio::piped()).spawn()?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("Cannot read output from the Word converter."));
    };
    let writer = thread::spawn(move || stdin.write_all(&input));
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        Ok::<_, io::Error>(bytes)
    });
    let deadline = Instant::now() + CONVERSION_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| io::Error::other("The Word converter timed out.")));
            }
        }
    };
    let _ = writer.join();
    let output = reader.join().map_err(|_| io::Error::other("Cannot read output from the Word converter."))??;
    if output.len() <= MAX_TEXT_BYTES && (!status.success() || output.is_empty()) {
        return Err(io::Error::other("The Word converter could not read this document."));
    }
    Ok(output)
}

/// Mark truncated output without splitting a Unicode character.
fn limit_text(mut text: String) -> String {
    if text.len() > MAX_TEXT_BYTES {
        text.truncate(text.floor_char_boundary(MAX_TEXT_BYTES));
        text.push_str("\n\nPreview ended at the text size limit.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_docx_paragraphs_entities_and_tabs() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        archive
            .write_all(
                br#"<doc:document xmlns:doc="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <doc:body><doc:p><doc:r><doc:t>One &amp; two</doc:t><doc:tab/><doc:t>&#x33;</doc:t></doc:r></doc:p>
            <doc:p><doc:r><doc:t>Next line</doc:t></doc:r></doc:p></doc:body></doc:document>"#,
            )
            .unwrap();
        let bytes = archive.finish().unwrap().into_inner();

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
