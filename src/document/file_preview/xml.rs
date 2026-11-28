//! A small pull reader for the XML parts of Office and `OpenDocument` files.
//!
//! It reads UTF-8 XML into start tags, end tags and text, with each element
//! and attribute name resolved to its namespace URI. An empty element
//! (`<a/>`) comes out as a start tag followed by an end tag. Document type
//! declarations are refused, so no entity can expand.

/// Deepest element nesting read.
const MAX_DEPTH: usize = 512;
/// The namespace the `xml` prefix always stands for.
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// An element or attribute name with its namespace URI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Name {
    pub namespace: Option<String>,
    pub local: String,
}

impl Name {
    /// Whether this is `local` in one of the namespaces `uris`.
    pub(super) fn is(&self, uris: &[&str], local: &str) -> bool {
        self.local == local && self.namespace.as_deref().is_some_and(|namespace| uris.contains(&namespace))
    }

    /// Whether the name is in one of the namespaces `uris`.
    pub(super) fn in_namespace(&self, uris: &[&str]) -> bool {
        self.namespace.as_deref().is_some_and(|namespace| uris.contains(&namespace))
    }
}

/// A start tag: its name and attributes.
#[derive(Clone, Debug, Default)]
pub(super) struct Element {
    pub name: Name,
    attributes: Vec<(Name, String)>,
}

impl Element {
    /// The value of the attribute `local` without a namespace prefix.
    pub(super) fn attribute(&self, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| name.namespace.is_none() && name.local == local)
            .map(|(_, value)| value.as_str())
    }

    /// The value of the attribute `local` in one of the namespaces `uris`.
    pub(super) fn attribute_in(&self, uris: &[&str], local: &str) -> Option<&str> {
        self.attributes.iter().find(|(name, _)| name.is(uris, local)).map(|(_, value)| value.as_str())
    }
}

/// One step through the document.
#[derive(Debug)]
pub(super) enum Event {
    Start(Element),
    End(Name),
    /// Character data with references decoded, CDATA sections included.
    Text(String),
    Eof,
}

/// An open element: its raw name (to match the end tag), resolved name and
/// the namespace prefixes it declared.
#[derive(Debug)]
struct Open {
    raw: String,
    name: Name,
    prefixes: Vec<(String, String)>,
}

/// A pass over one XML document.
#[derive(Debug)]
pub(super) struct XmlReader<'a> {
    text: &'a str,
    position: usize,
    open: Vec<Open>,
    /// Set after an empty element's start tag: its end comes next.
    pending_end: bool,
}

impl<'a> XmlReader<'a> {
    /// Read `bytes`, which must be UTF-8 (with or without a byte order mark).
    pub(super) fn new(bytes: &'a [u8]) -> Result<Self, String> {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let text = std::str::from_utf8(bytes).map_err(|_| "This XML part is not UTF-8 text.".to_string())?;
        Ok(Self { text, position: 0, open: Vec::new(), pending_end: false })
    }

    /// The next start tag, end tag or text; `Eof` at the end, repeatedly.
    pub(super) fn next(&mut self) -> Result<Event, String> {
        if self.pending_end {
            self.pending_end = false;
            return Ok(Event::End(self.close()));
        }
        loop {
            let rest = &self.text[self.position..];
            if rest.is_empty() {
                return Ok(Event::Eof);
            }
            if !rest.starts_with('<') {
                let length = rest.find('<').unwrap_or(rest.len());
                self.position += length;
                return Ok(Event::Text(decode(&rest[..length])));
            }
            if let Some(body) = rest.strip_prefix("<!--") {
                self.position += 4 + body.find("-->").ok_or_else(|| unclosed("comment"))? + 3;
            } else if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let length = body.find("]]>").ok_or_else(|| unclosed("CDATA section"))?;
                self.position += 9 + length + 3;
                return Ok(Event::Text(body[..length].to_string()));
            } else if rest.starts_with("<!") {
                return Err("XML document type declarations are not supported.".into());
            } else if let Some(body) = rest.strip_prefix("<?") {
                self.position += 2 + body.find("?>").ok_or_else(|| unclosed("processing instruction"))? + 2;
            } else if let Some(body) = rest.strip_prefix("</") {
                let length = body.find('>').ok_or_else(|| unclosed("end tag"))?;
                let raw = body[..length].trim_end();
                self.position += 2 + length + 1;
                if self.open.last().is_none_or(|open| open.raw != raw) {
                    return Err(format!("The XML end tag </{raw}> does not match its start tag."));
                }
                return Ok(Event::End(self.close()));
            } else {
                return self.start_tag();
            }
        }
    }

    /// Skip the rest of the element whose start tag was just read.
    pub(super) fn skip_element(&mut self) -> Result<(), String> {
        let depth = self.open.len();
        while self.open.len() >= depth {
            if let Event::Eof = self.next()? {
                return Err("The XML part ends inside an element.".into());
            }
        }
        Ok(())
    }

    /// Read a start tag at the current `<`.
    fn start_tag(&mut self) -> Result<Event, String> {
        if self.open.len() >= MAX_DEPTH {
            return Err("The XML elements nest too deeply.".into());
        }
        let body = &self.text[self.position + 1..];
        let mut cursor = 0;
        let raw = take_name(body, &mut cursor).ok_or_else(|| "The XML part has a tag without a name.".to_string())?;
        let mut raw_attributes = Vec::new();
        let empty = loop {
            cursor += body[cursor..].len() - body[cursor..].trim_start().len();
            let rest = &body[cursor..];
            if let Some(after) = rest.strip_prefix("/>") {
                cursor = body.len() - after.len();
                break true;
            }
            if let Some(after) = rest.strip_prefix('>') {
                cursor = body.len() - after.len();
                break false;
            }
            let name = take_name(body, &mut cursor).ok_or_else(|| unclosed("start tag"))?;
            cursor += body[cursor..].len() - body[cursor..].trim_start().len();
            let rest = body[cursor..].strip_prefix('=').ok_or_else(|| unclosed("attribute"))?.trim_start();
            let quote =
                rest.chars().next().filter(|quote| matches!(quote, '"' | '\'')).ok_or_else(|| unclosed("attribute"))?;
            let value_length = rest[1..].find(quote).ok_or_else(|| unclosed("attribute"))?;
            let value = &rest[1..=value_length];
            cursor = body.len() - rest.len() + 1 + value_length + 1;
            raw_attributes.push((name, decode(value).replace(['\t', '\n', '\r'], " ")));
        };
        self.position += 1 + cursor;

        let prefixes = raw_attributes
            .iter()
            .filter_map(|(name, value)| match name.split_once(':') {
                Some(("xmlns", prefix)) => Some((prefix.to_string(), value.clone())),
                None if *name == "xmlns" => Some((String::new(), value.clone())),
                _ => None,
            })
            .collect();
        self.open.push(Open { raw: raw.to_string(), name: Name::default(), prefixes });
        let name = self.resolve(raw, true);
        let attributes = raw_attributes
            .into_iter()
            .filter(|(name, _)| *name != "xmlns" && !name.starts_with("xmlns:"))
            .map(|(name, value)| (self.resolve(name, false), value))
            .collect();
        if let Some(open) = self.open.last_mut() {
            open.name = name.clone();
        }
        self.pending_end = empty;
        Ok(Event::Start(Element { name, attributes }))
    }

    /// Close the innermost element and return its name.
    fn close(&mut self) -> Name {
        self.open.pop().map(|open| open.name).unwrap_or_default()
    }

    /// Resolve a `prefix:local` name. Unprefixed elements take the default
    /// namespace; unprefixed attributes have none.
    fn resolve(&self, raw: &str, is_element: bool) -> Name {
        let (prefix, local) = match raw.split_once(':') {
            Some((prefix, local)) => (prefix, local),
            None if is_element => ("", raw),
            None => return Name { namespace: None, local: raw.to_string() },
        };
        let namespace = if prefix == "xml" {
            Some(XML_NAMESPACE.to_string())
        } else {
            self.open
                .iter()
                .rev()
                .flat_map(|open| open.prefixes.iter())
                .find(|(declared, _)| declared == prefix)
                .map(|(_, uri)| uri.clone())
                .filter(|uri| !uri.is_empty())
        };
        Name { namespace, local: local.to_string() }
    }
}

/// Read a name at `cursor` in `text` and move past it.
fn take_name<'t>(text: &'t str, cursor: &mut usize) -> Option<&'t str> {
    let rest = &text[*cursor..];
    let length = rest
        .find(|character: char| character.is_whitespace() || matches!(character, '/' | '>' | '='))
        .unwrap_or(rest.len());
    if length == 0 {
        return None;
    }
    *cursor += length;
    Some(&rest[..length])
}

/// The message for a construct that never ends.
fn unclosed(what: &str) -> String {
    format!("The XML part has an unclosed {what}.")
}

/// Decode the predefined entities and character references in `text`.
/// Unknown entities stay as written.
fn decode(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 12) else {
            decoded.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let character = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "apos" => Some('\''),
            "quot" => Some('"'),
            _ => entity
                .strip_prefix("#x")
                .map(|hex| u32::from_str_radix(hex, 16))
                .or_else(|| entity.strip_prefix('#').map(str::parse))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        if let Some(character) = character {
            decoded.push(character);
            rest = &rest[end + 1..];
        } else {
            decoded.push('&');
            rest = &rest[1..];
        }
    }
    decoded.push_str(rest);
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(xml: &str) -> Result<Vec<String>, String> {
        let mut reader = XmlReader::new(xml.as_bytes())?;
        let mut events = Vec::new();
        loop {
            let event = reader.next()?;
            let text = match &event {
                Event::Start(element) => {
                    let attributes: Vec<String> = element
                        .attributes
                        .iter()
                        .map(|(name, value)| {
                            format!(" {}{}={value}", name.namespace.as_deref().unwrap_or(""), name.local)
                        })
                        .collect();
                    format!(
                        "<{}{}{}>",
                        element.name.namespace.as_deref().unwrap_or(""),
                        element.name.local,
                        attributes.concat()
                    )
                }
                Event::End(name) => format!("</{}>", name.local),
                Event::Text(text) => text.clone(),
                Event::Eof => break,
            };
            events.push(text);
        }
        Ok(events)
    }

    #[test]
    fn resolves_namespaces_and_expands_empty_elements() {
        let xml = r#"<?xml version="1.0"?><!-- note --><a xmlns="urn:a" xmlns:b="urn:b" b:x="1" y='2'><b:c/><d xmlns="">t</d></a>"#;
        assert_eq!(events(xml).unwrap(), ["<urn:aa urn:bx=1 y=2>", "<urn:bc>", "</c>", "<d>", "t", "</d>", "</a>"]);
    }

    #[test]
    fn decodes_references_and_cdata() {
        let xml = "<a t=\"x&amp;y\tz\">&lt;&#65;&#x42;&unknown;&<![CDATA[<raw>]]></a>";
        assert_eq!(events(xml).unwrap(), ["<a t=x&y z>", "<AB&unknown;&", "<raw>", "</a>"]);
    }

    #[test]
    fn refuses_doctypes_mismatched_tags_and_deep_nesting() {
        assert!(events("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>").is_err());
        assert!(events("<a><b></a>").is_err());
        assert!(events("<a").is_err());
        assert!(events(&"<a>".repeat(MAX_DEPTH + 1)).is_err());
    }

    #[test]
    fn skips_the_rest_of_an_element() {
        let mut reader = XmlReader::new(b"<a><skip><x>1</x></skip><keep/></a>").unwrap();
        reader.next().unwrap();
        reader.next().unwrap();
        reader.skip_element().unwrap();
        let Event::Start(element) = reader.next().unwrap() else { panic!("expected keep") };
        assert_eq!(element.name.local, "keep");
    }
}
