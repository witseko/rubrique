//! Read bounded XML parts from ZIP-based documents without extracting them.

use std::io::{Cursor, Read};

use quick_xml::NsReader;
use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesRef, Event};
use quick_xml::name::ResolveResult;
use zip::ZipArchive;

/// Maximum size of one uncompressed XML part.
const MAX_PART_BYTES: u64 = 16 * 1024 * 1024;

/// Read a named part from the archive and enforce its declared and actual size.
pub(super) fn read_part(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    let part = archive.by_name(name).map_err(|error| format!("Cannot read {name}: {error}"))?;
    if part.size() > MAX_PART_BYTES {
        return Err("This document part exceeds the preview limit.".into());
    }
    let mut xml = Vec::new();
    part.take(MAX_PART_BYTES + 1).read_to_end(&mut xml).map_err(|error| error.to_string())?;
    if xml.len() as u64 > MAX_PART_BYTES {
        return Err("This document part exceeds the preview limit.".into());
    }
    Ok(xml)
}

/// Compare the resolved namespace URI independently of its prefix.
pub(super) fn in_namespace(namespace: &ResolveResult<'_>, uris: &[&str]) -> bool {
    matches!(namespace, ResolveResult::Bound(value) if uris.contains(&value.as_ref()))
}

/// Verify the XML part's root element before reading its content.
pub(super) fn require_root(reader: &mut NsReader<&[u8]>, local_name: &str, uris: &[&str]) -> Result<(), String> {
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| error.to_string())?;
        match event {
            Event::Start(tag) if tag.local_name().as_ref() == local_name && in_namespace(&namespace, uris) => {
                return Ok(());
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            Event::Text(text) if text.as_ref().trim().is_empty() => {}
            _ => return Err(format!("The document part is missing its {local_name} root element.")),
        }
    }
}

/// Append a numeric character or a predefined XML entity to the preview.
pub(super) fn push_reference(text: &mut String, reference: &BytesRef<'_>) -> Result<(), quick_xml::Error> {
    if let Some(character) = reference.resolve_char_ref()? {
        text.push(character);
    } else if let Some(entity) = resolve_xml_entity(reference.as_ref()) {
        text.push_str(entity);
    }
    Ok(())
}
