//! Extract the readable body text of an `OpenDocument` word-processing file.

use std::io::Cursor;

use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::NamespaceResolver;

use super::package::{in_namespace, push_reference, read_part, require_root};
use super::{MAX_TEXT_BYTES, limit_text};

const OFFICE: &[&str] = &["urn:oasis:names:tc:opendocument:xmlns:office:1.0"];
const TEXT: &[&str] = &["urn:oasis:names:tc:opendocument:xmlns:text:1.0"];

/// Read the text body from the package's content part.
pub(super) fn text(bytes: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let xml = read_part(&mut archive, "content.xml")?;
    let mut reader = NsReader::from_reader(xml.as_slice());
    reader.config_mut().expand_empty_elements = true;
    require_root(&mut reader, "document-content", OFFICE)?;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) => {
                let (namespace, name) = reader.resolver().resolve_element(tag.name());
                if in_namespace(&namespace, OFFICE) && name.as_ref() == "text" {
                    return body_text(&mut reader);
                }
            }
            Event::Eof => return Err("This file has no OpenDocument text body.".into()),
            _ => {}
        }
    }
}

/// Collect paragraphs and headings, skipping annotations and tracked changes.
fn body_text(reader: &mut NsReader<&[u8]>) -> Result<String, String> {
    let mut text = String::new();
    let mut paragraphs = 0_usize;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| error.to_string())?;
        let is_text = in_namespace(&namespace, TEXT);
        let is_office = in_namespace(&namespace, OFFICE);
        match event {
            Event::Start(tag) if skip_group(is_text, is_office, tag.local_name().as_ref()) => {
                reader.read_to_end(tag.name()).map_err(|error| error.to_string())?;
            }
            Event::Start(tag) if is_text && is_paragraph(tag.local_name().as_ref()) => paragraphs += 1,
            Event::End(tag) if is_text && is_paragraph(tag.local_name().as_ref()) => {
                paragraphs = paragraphs.saturating_sub(1);
                text.push('\n');
            }
            Event::Start(tag) if is_text && paragraphs > 0 => push_spacing(&mut text, &tag, reader.resolver())?,
            Event::Text(value) if paragraphs > 0 => text.push_str(&value.xml10_content()),
            Event::CData(value) if paragraphs > 0 => text.push_str(&value.xml10_content()),
            Event::GeneralRef(reference) if paragraphs > 0 => {
                push_reference(&mut text, &reference).map_err(|error| error.to_string())?;
            }
            Event::End(tag) if is_office && tag.local_name().as_ref() == "text" => return Ok(text),
            Event::Eof => return Err("The OpenDocument text body is incomplete.".into()),
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
fn skip_group(is_text: bool, is_office: bool, name: &str) -> bool {
    (is_office && name == "annotation")
        || (is_text && matches!(name, "tracked-changes" | "deletion" | "dde-connection-decls"))
}

/// Preserve explicit spaces, tabs, and line breaks in an `OpenDocument` paragraph.
fn push_spacing(text: &mut String, tag: &BytesStart<'_>, resolver: &NamespaceResolver) -> Result<(), String> {
    match tag.local_name().as_ref() {
        "s" => {
            let repeats = space_count(tag, resolver)?;
            text.extend(std::iter::repeat_n(' ', repeats.min(MAX_TEXT_BYTES + 1 - text.len())));
        }
        "tab" => text.push('\t'),
        "line-break" => text.push('\n'),
        _ => {}
    }
    Ok(())
}

/// Get the bounded repetition count of a space marker, which defaults to one.
fn space_count(tag: &BytesStart<'_>, resolver: &NamespaceResolver) -> Result<usize, String> {
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        let (namespace, name) = resolver.resolve_attribute(attribute.key);
        if name.as_ref() == "c" && in_namespace(&namespace, TEXT) {
            return attribute.value.parse().map_err(|_| "Invalid space count in OpenDocument text.".into());
        }
    }
    Ok(1)
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
