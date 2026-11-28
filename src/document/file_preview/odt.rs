//! Extract the readable body text of an `OpenDocument` word-processing file.

use super::package::{read_part, require_root};
use super::xml::{Element, Event, XmlReader};
use super::zip_archive::ZipArchive;
use super::{MAX_TEXT_BYTES, limit_text};

/// The `OpenDocument` office namespace.
pub(super) const OFFICE: &[&str] = &["urn:oasis:names:tc:opendocument:xmlns:office:1.0"];
/// The `OpenDocument` text namespace.
pub(super) const TEXT: &[&str] = &["urn:oasis:names:tc:opendocument:xmlns:text:1.0"];

/// Read the text body from the package's content part.
pub(super) fn text(bytes: &[u8]) -> Result<String, String> {
    let archive = ZipArchive::new(bytes)?;
    let xml = read_part(&archive, "content.xml")?;
    let mut reader = XmlReader::new(&xml)?;
    require_root(&mut reader, "document-content", OFFICE)?;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(OFFICE, "text") => return body_text(&mut reader),
            Event::Eof => return Err("This file has no OpenDocument text body.".into()),
            _ => {}
        }
    }
}

/// Collect paragraphs and headings, skipping annotations and tracked changes.
fn body_text(reader: &mut XmlReader<'_>) -> Result<String, String> {
    let mut text = String::new();
    let mut paragraphs = 0_usize;
    loop {
        match reader.next()? {
            Event::Start(element) if skip_group(&element) => reader.skip_element()?,
            Event::Start(element) if element.name.in_namespace(TEXT) && is_paragraph(&element.name.local) => {
                paragraphs += 1;
            }
            Event::End(name) if name.in_namespace(TEXT) && is_paragraph(&name.local) => {
                paragraphs = paragraphs.saturating_sub(1);
                text.push('\n');
            }
            Event::Start(element) if element.name.in_namespace(TEXT) && paragraphs > 0 => {
                push_spacing(&mut text, &element)?;
            }
            Event::Text(value) if paragraphs > 0 => text.push_str(&value),
            Event::End(name) if name.is(OFFICE, "text") => return Ok(text),
            _ => {}
        }
        if text.len() > MAX_TEXT_BYTES {
            return Ok(limit_text(text));
        }
    }
}

/// True for the two elements that delimit a text paragraph.
fn is_paragraph(name: &str) -> bool {
    matches!(name, "p" | "h")
}

/// True for groups that carry comments, revision history, or declarations.
fn skip_group(element: &Element) -> bool {
    element.name.is(OFFICE, "annotation")
        || (element.name.in_namespace(TEXT)
            && matches!(element.name.local.as_str(), "tracked-changes" | "deletion" | "dde-connection-decls"))
}

/// Preserve explicit spaces, tabs, and line breaks in an `OpenDocument` paragraph.
fn push_spacing(text: &mut String, element: &Element) -> Result<(), String> {
    match element.name.local.as_str() {
        "s" => {
            let repeats = space_count(element)?;
            text.extend(std::iter::repeat_n(' ', repeats.min(MAX_TEXT_BYTES + 1 - text.len())));
        }
        "tab" => text.push('\t'),
        "line-break" => text.push('\n'),
        _ => {}
    }
    Ok(())
}

/// Get the bounded repetition count of a space marker, which defaults to one.
fn space_count(element: &Element) -> Result<usize, String> {
    match element.attribute_in(TEXT, "c") {
        Some(count) => count.parse().map_err(|_| "Invalid space count in OpenDocument text.".into()),
        None => Ok(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_open_document_body_and_skips_revision_history_and_comments() {
        let bytes = include_bytes!("fixtures/text.odt");
        assert_eq!(text(bytes).unwrap(), "Heading\nA & B   café\ttab\nnext\n");
    }

    #[test]
    fn rejects_spreadsheets_renamed_as_documents() {
        assert!(text(include_bytes!("fixtures/two-sheets.ods")).is_err());
    }
}
