//! Read the text of the selected `PowerPoint` slide in presentation order.

use super::package::{DOCUMENT_REL, is_relationship_type, part_path, read_part, relationships, require_root};
use super::xml::{Event, XmlReader};
use super::zip_archive::ZipArchive;
use super::{Content, MAX_TEXT_BYTES, limit_text};

const PRESENTATION: &[&str] =
    &["http://schemas.openxmlformats.org/presentationml/2006/main", "http://purl.oclc.org/ooxml/presentationml/main"];
const DRAWING: &[&str] =
    &["http://schemas.openxmlformats.org/drawingml/2006/main", "http://purl.oclc.org/ooxml/drawingml/main"];

/// Resolve the requested slide through the presentation's internal relationships.
pub(super) fn slide(bytes: &[u8], selected: usize) -> Result<Content, String> {
    let archive = ZipArchive::new(bytes)?;
    let presentation = read_part(&archive, "ppt/presentation.xml")?;
    let (id, slide_count) = slide_id(&presentation, selected)?;
    let relationships = read_part(&archive, "ppt/_rels/presentation.xml.rels")?;
    let target = slide_target(&relationships, &id)?;
    let path = part_path("ppt", &target)?;
    let xml = read_part(&archive, &path)?;
    let text = slide_text(&xml)?;
    Ok(Content::Slide { text, slide_count })
}

/// Count slides and get the selected relationship ID from the ordered slide list.
fn slide_id(xml: &[u8], selected: usize) -> Result<(String, usize), String> {
    let mut reader = XmlReader::new(xml)?;
    require_root(&mut reader, "presentation", PRESENTATION)?;
    let mut in_list = false;
    let mut count = 0;
    let mut id = None;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(PRESENTATION, "sldIdLst") => in_list = true,
            Event::Start(element) if in_list && element.name.is(PRESENTATION, "sldId") => {
                if count == selected {
                    let relationship = element.attribute_in(DOCUMENT_REL, "id");
                    id = Some(relationship.ok_or("This slide has no relationship ID.")?.to_string());
                }
                count += 1;
            }
            Event::End(name) if in_list && name.is(PRESENTATION, "sldIdLst") => break,
            Event::Eof => break,
            _ => {}
        }
    }
    id.map(|id| (id, count)).ok_or_else(|| "This presentation has no slide to display at that position.".into())
}

/// Find a matching slide relationship and accept internal targets only.
fn slide_target(xml: &[u8], id: &str) -> Result<String, String> {
    let (_, kind, target) = relationships(xml)?
        .into_iter()
        .find(|(relationship, _, _)| relationship == id)
        .ok_or("This slide does not reference an internal slide part.")?;
    if !is_relationship_type(&kind, "slide") {
        return Err("This slide does not reference an internal slide part.".into());
    }
    Ok(target)
}

/// Extract `DrawingML` text and keep paragraph and explicit line breaks.
fn slide_text(xml: &[u8]) -> Result<String, String> {
    let mut reader = XmlReader::new(xml)?;
    require_root(&mut reader, "sld", PRESENTATION)?;
    let mut text = String::new();
    let mut in_text = false;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.in_namespace(DRAWING) => match element.name.local.as_str() {
                "t" => in_text = true,
                "br" => text.push('\n'),
                _ => {}
            },
            Event::End(name) if name.in_namespace(DRAWING) => match name.local.as_str() {
                "t" => in_text = false,
                "p" => text.push('\n'),
                _ => {}
            },
            Event::Text(value) if in_text => text.push_str(&value),
            Event::End(name) if name.is(PRESENTATION, "sld") => return Ok(text),
            _ => {}
        }
        if text.len() > MAX_TEXT_BYTES {
            return Ok(limit_text(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_slide_order_instead_of_sorting_part_filenames() {
        let bytes = include_bytes!("fixtures/ordered-slides.pptx");
        for (index, expected) in ["Second & first", "First\nNext"].into_iter().enumerate() {
            let Content::Slide { text, slide_count } = slide(bytes, index).unwrap() else {
                panic!("expected a slide preview");
            };
            assert_eq!(slide_count, 2);
            assert_eq!(text.trim(), expected);
        }
        assert!(slide(bytes, 2).is_err());
    }

    #[test]
    fn rejects_external_slide_relationships() {
        let xml = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"
                TargetMode="External" Target="https://example.org/slide.xml"/>
            </Relationships>"#;
        assert!(slide_target(xml, "s1").is_err());
    }
}
