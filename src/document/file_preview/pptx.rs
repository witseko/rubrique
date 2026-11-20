//! Read the text of the selected `PowerPoint` slide in presentation order.

use std::borrow::Cow;
use std::io::Cursor;

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::NamespaceResolver;

use super::package::{in_namespace, push_reference, read_part, require_root};
use super::{Content, MAX_TEXT_BYTES, limit_text};

const PRESENTATION: &[&str] =
    &["http://schemas.openxmlformats.org/presentationml/2006/main", "http://purl.oclc.org/ooxml/presentationml/main"];
const DRAWING: &[&str] =
    &["http://schemas.openxmlformats.org/drawingml/2006/main", "http://purl.oclc.org/ooxml/drawingml/main"];
const DOCUMENT_REL: &[&str] = &[
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];
const PACKAGE_REL: &[&str] = &[
    "http://schemas.openxmlformats.org/package/2006/relationships",
    "http://purl.oclc.org/ooxml/package/relationships",
];

/// Resolve the requested slide through the presentation's internal relationships.
pub(super) fn slide(bytes: &[u8], selected: usize) -> Result<Content, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let presentation = read_part(&mut archive, "ppt/presentation.xml")?;
    let (id, slide_count) = slide_id(&presentation, selected)?;
    let relationships = read_part(&mut archive, "ppt/_rels/presentation.xml.rels")?;
    let target = slide_target(&relationships, &id)?;
    let path = slide_path(&target)?;
    let xml = read_part(&mut archive, &path)?;
    let text = slide_text(&xml)?;
    Ok(Content::Slide { text, slide_count })
}

/// Count slides and get the selected relationship ID from the ordered slide list.
fn slide_id(xml: &[u8], selected: usize) -> Result<(String, usize), String> {
    let mut reader = NsReader::from_reader(xml);
    require_root(&mut reader, "presentation", PRESENTATION)?;
    let mut in_list = false;
    let mut count = 0;
    let mut id = None;
    loop {
        let event = reader.read_event().map_err(|error| error.to_string())?;
        match event {
            Event::Start(tag) if is_presentation_tag(reader.resolver(), &tag, "sldIdLst") => in_list = true,
            Event::Start(tag) | Event::Empty(tag)
                if in_list && is_presentation_tag(reader.resolver(), &tag, "sldId") =>
            {
                if count == selected {
                    id = Some(relationship_id(reader.resolver(), &tag)?);
                }
                count += 1;
            }
            Event::End(tag) if in_list => {
                let (namespace, name) = reader.resolver().resolve_element(tag.name());
                if name.as_ref() == "sldIdLst" && in_namespace(&namespace, PRESENTATION) {
                    break;
                }
            }
            Event::Eof if in_list => return Err("The presentation slide list is incomplete.".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    id.map(|id| (id, count)).ok_or_else(|| "This presentation has no slide to display at that position.".into())
}

/// Compare a presentation element name independently of its XML prefix.
fn is_presentation_tag(resolver: &NamespaceResolver, tag: &BytesStart<'_>, name: &str) -> bool {
    let (namespace, local_name) = resolver.resolve_element(tag.name());
    local_name.as_ref() == name && in_namespace(&namespace, PRESENTATION)
}

/// Distinguish the relationship ID from the slide's own numeric ID.
fn relationship_id(resolver: &NamespaceResolver, tag: &BytesStart<'_>) -> Result<String, String> {
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        let (namespace, name) = resolver.resolve_attribute(attribute.key);
        if name.as_ref() == "id" && in_namespace(&namespace, DOCUMENT_REL) {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map(Cow::into_owned)
                .map_err(|error| error.to_string());
        }
    }
    Err("This slide has no relationship ID.".into())
}

/// Find a matching slide relationship and accept internal targets only.
fn slide_target(xml: &[u8], id: &str) -> Result<String, String> {
    let mut reader = NsReader::from_reader(xml);
    require_root(&mut reader, "Relationships", PACKAGE_REL)?;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| error.to_string())?;
        match event {
            Event::Start(tag) | Event::Empty(tag)
                if tag.local_name().as_ref() == "Relationship" && in_namespace(&namespace, PACKAGE_REL) =>
            {
                if let Some(target) = matching_target(&tag, id)? {
                    return Ok(target);
                }
            }
            Event::Eof => return Err("The presentation is missing this slide's relationship.".into()),
            _ => {}
        }
    }
}

/// Read the target only for the requested slide relationship.
fn matching_target(tag: &BytesStart<'_>, id: &str) -> Result<Option<String>, String> {
    let attribute = |name| -> Result<_, String> {
        tag.try_get_attribute(name)
            .map_err(|error| error.to_string())?
            .map(|attribute| attribute.normalized_value(XmlVersion::Implicit1_0))
            .transpose()
            .map_err(|error| error.to_string())
    };
    if attribute("Id")?.as_deref() != Some(id) {
        return Ok(None);
    }
    let kind = attribute("Type")?.unwrap_or_default();
    let is_slide = kind.strip_suffix("/slide").is_some_and(|prefix| DOCUMENT_REL.contains(&prefix));
    if !is_slide || attribute("TargetMode")?.as_deref().is_some_and(|mode| mode != "Internal") {
        return Err("This slide does not reference an internal slide part.".into());
    }
    attribute("Target")?.map(|target| Some(target.into_owned())).ok_or_else(|| "This slide has no target part.".into())
}

/// Resolve a target relative to the presentation part, entirely inside the package.
fn slide_path(target: &str) -> Result<String, String> {
    let invalid = || "This slide has an invalid package path.".to_string();
    if target.is_empty() || target.starts_with("//") || target.contains([':', '?', '#', '\\']) {
        return Err(invalid());
    }
    let (target, mut path) = target.strip_prefix('/').map_or((target, vec!["ppt"]), |part| (part, Vec::new()));
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

/// Extract `DrawingML` text and keep paragraph and explicit line breaks.
fn slide_text(xml: &[u8]) -> Result<String, String> {
    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().expand_empty_elements = true;
    require_root(&mut reader, "sld", PRESENTATION)?;
    let mut text = String::new();
    let mut in_text = false;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| error.to_string())?;
        let drawing = in_namespace(&namespace, DRAWING);
        match event {
            Event::Start(tag) if drawing => match tag.local_name().as_ref() {
                "t" => in_text = true,
                "br" => text.push('\n'),
                _ => {}
            },
            Event::End(tag) if drawing => match tag.local_name().as_ref() {
                "t" => in_text = false,
                "p" => text.push('\n'),
                _ => {}
            },
            Event::Text(value) if in_text => text.push_str(&value.xml10_content()),
            Event::CData(value) if in_text => text.push_str(&value.xml10_content()),
            Event::GeneralRef(reference) if in_text => {
                push_reference(&mut text, &reference).map_err(|error| error.to_string())?;
            }
            Event::End(tag) if tag.local_name().as_ref() == "sld" && in_namespace(&namespace, PRESENTATION) => {
                return Ok(text);
            }
            Event::Eof => return Err("The slide part is incomplete.".into()),
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
    fn rejects_external_slide_relationships_and_paths_outside_the_package() {
        let xml = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"
                TargetMode="External" Target="https://example.org/slide.xml"/>
            </Relationships>"#;
        assert!(slide_target(xml, "s1").is_err());
        assert!(slide_path("../../escape.xml").is_err());
        assert!(slide_path("https://example.org/slide.xml").is_err());
        assert_eq!(slide_path("/ppt/slides/slide3.xml").unwrap(), "ppt/slides/slide3.xml");
    }
}
