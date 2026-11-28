//! Read bounded XML parts from ZIP-based documents without extracting them.

use super::xml::{Event, XmlReader};
use super::zip_archive::ZipArchive;

/// Maximum size of one uncompressed XML part.
const MAX_PART_BYTES: u64 = 16 * 1024 * 1024;

/// Relationship attributes in Office Open XML parts (transitional and strict).
pub(super) const DOCUMENT_REL: &[&str] = &[
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];
/// The `.rels` parts that list a part's relationships.
pub(super) const PACKAGE_REL: &[&str] = &[
    "http://schemas.openxmlformats.org/package/2006/relationships",
    "http://purl.oclc.org/ooxml/package/relationships",
];

/// Read a named part from the archive and enforce its declared and actual size.
pub(super) fn read_part(archive: &ZipArchive<'_>, name: &str) -> Result<Vec<u8>, String> {
    archive.read(name, MAX_PART_BYTES)
}

/// Verify the XML part's root element before reading its content.
pub(super) fn require_root(reader: &mut XmlReader<'_>, local_name: &str, uris: &[&str]) -> Result<(), String> {
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(uris, local_name) => return Ok(()),
            Event::Text(text) if text.trim().is_empty() => {}
            _ => return Err(format!("The document part is missing its {local_name} root element.")),
        }
    }
}

/// The relationship targets in a `.rels` part, as (Id, Type, Target) for
/// internal targets only.
pub(super) fn relationships(xml: &[u8]) -> Result<Vec<(String, String, String)>, String> {
    let mut reader = XmlReader::new(xml)?;
    require_root(&mut reader, "Relationships", PACKAGE_REL)?;
    let mut found = Vec::new();
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(PACKAGE_REL, "Relationship") => {
                if element.attribute("TargetMode").is_some_and(|mode| mode != "Internal") {
                    continue;
                }
                if let (Some(id), Some(kind), Some(target)) =
                    (element.attribute("Id"), element.attribute("Type"), element.attribute("Target"))
                {
                    found.push((id.to_string(), kind.to_string(), target.to_string()));
                }
            }
            Event::Eof => return Ok(found),
            _ => {}
        }
    }
}

/// Whether a relationship type is the Office document relationship `kind`
/// (such as `slide` or `worksheet`).
pub(super) fn is_relationship_type(relationship_type: &str, kind: &str) -> bool {
    relationship_type
        .strip_suffix(kind)
        .and_then(|prefix| prefix.strip_suffix('/'))
        .is_some_and(|prefix| DOCUMENT_REL.contains(&prefix))
}

/// Resolve a target relative to the folder `base`, entirely inside the
/// package.
pub(super) fn part_path(base: &str, target: &str) -> Result<String, String> {
    let invalid = || "This document has an invalid package path.".to_string();
    if target.is_empty() || target.starts_with("//") || target.contains([':', '?', '#', '\\']) {
        return Err(invalid());
    }
    let (target, mut path) =
        target.strip_prefix('/').map_or((target, base.split('/').collect::<Vec<_>>()), |part| (part, Vec::new()));
    for part in target.split('/') {
        match part {
            "" => return Err(invalid()),
            "." => {}
            ".." => {
                path.pop().ok_or_else(invalid)?;
            }
            name => path.push(name),
        }
    }
    if path.is_empty() {
        return Err(invalid());
    }
    Ok(path.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_package_paths_inside_the_package() {
        assert!(part_path("ppt", "../../escape.xml").is_err());
        assert!(part_path("ppt", "https://example.org/slide.xml").is_err());
        assert_eq!(part_path("ppt", "/ppt/slides/slide3.xml").unwrap(), "ppt/slides/slide3.xml");
        assert_eq!(part_path("xl", "worksheets/sheet1.xml").unwrap(), "xl/worksheets/sheet1.xml");
    }

    #[test]
    fn lists_internal_relationships_only() {
        let xml = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"
                TargetMode="External" Target="https://example.org/slide.xml"/>
            <Relationship Id="s2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"
                Target="slides/slide2.xml"/>
            </Relationships>"#;
        let found = relationships(xml).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "s2");
        assert!(is_relationship_type(&found[0].1, "slide"));
        assert!(!is_relationship_type(&found[0].1, "worksheet"));
    }
}
