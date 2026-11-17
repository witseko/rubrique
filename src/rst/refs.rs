//! What a reStructuredText document defines once and uses elsewhere:
//! hyperlink targets, substitutions and footnote numbers. Collected in one
//! pass before the blocks are read, because RST lets a reference come
//! before its target.

use std::collections::{HashMap, HashSet};

use super::lines::indent;

/// Most hops followed through indirect targets (`.. _a: b_`), so a loop of
/// targets pointing at each other ends.
const MAX_INDIRECT_HOPS: usize = 8;

/// What a `|name|` substitution stands for.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Substitution {
    /// `.. |name| replace:: text`: inline text, read as RST.
    Text(String),
    /// `.. |name| image:: url`.
    Image { url: String, alt: String },
}

/// Targets, substitutions and footnote numbers of one document, plus the
/// position of the next anonymous reference while the blocks are read.
#[derive(Debug, Default)]
pub(super) struct Refs {
    /// Named targets by normalized name, resolved to a URL.
    targets: HashMap<String, String>,
    /// Anonymous targets (`__ url`) in document order.
    anonymous: Vec<String>,
    next_anonymous: usize,
    substitutions: HashMap<String, Substitution>,
    /// Numbers of auto-numbered footnotes with a name (`[#note]`).
    named_footnotes: HashMap<String, u32>,
    /// Numbers of every auto-numbered footnote definition, in order.
    auto_footnotes: Vec<u32>,
    next_auto_definition: usize,
    /// Numbers of the plain `[#]` footnote definitions, in order; `[#]_`
    /// references take them one by one.
    anonymous_footnotes: Vec<u32>,
    next_anonymous_footnote: usize,
}

/// A target or substitution name as RST compares them: lowercase, with
/// runs of whitespace as one space.
pub(super) fn normalize_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl Refs {
    /// Collect every target, substitution and footnote label in `lines`.
    pub(super) fn collect(lines: &[&str]) -> Refs {
        let mut refs = Refs::default();
        let mut footnote_labels = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix(".. __:") {
                refs.anonymous.push(target_value(rest, lines, index));
            } else if let Some(rest) = trimmed.strip_prefix("__ ") {
                refs.anonymous.push(target_value(rest, lines, index));
            } else if let Some(rest) = trimmed.strip_prefix(".. _") {
                refs.add_named_target(rest, lines, index);
            } else if let Some(rest) = trimmed.strip_prefix(".. |") {
                refs.add_substitution(rest, lines, index);
            } else if let Some(label) = trimmed.strip_prefix(".. [").and_then(|rest| rest.split_once(']')) {
                footnote_labels.push(label.0.to_string());
            }
            refs.add_embedded_targets(line);
        }
        refs.number_footnotes(&footnote_labels);
        refs.resolve_indirect_targets();
        refs
    }

    /// The URL of the target named `name`.
    pub(super) fn target(&self, name: &str) -> Option<&str> {
        self.targets.get(&normalize_name(name)).map(String::as_str)
    }

    /// The URL of the next anonymous target, for a `name__` reference.
    pub(super) fn next_anonymous_target(&mut self) -> Option<String> {
        let url = self.anonymous.get(self.next_anonymous).cloned();
        self.next_anonymous += 1;
        url
    }

    /// What `|name|` stands for.
    pub(super) fn substitution(&self, name: &str) -> Option<&Substitution> {
        self.substitutions.get(&normalize_name(name))
    }

    /// The label a footnote reference shows: its number for `[#]_` and
    /// `[#name]_`, the label as written otherwise.
    pub(super) fn footnote_reference_label(&mut self, label: &str) -> String {
        if label == "#" {
            let number = self.anonymous_footnotes.get(self.next_anonymous_footnote).copied();
            self.next_anonymous_footnote += 1;
            return number.map_or_else(|| label.to_string(), |number| number.to_string());
        }
        self.named_footnote_number(label).unwrap_or_else(|| label.to_string())
    }

    /// The label a footnote definition shows: like a reference, but `[#]`
    /// definitions take the auto numbers in document order.
    pub(super) fn footnote_definition_label(&mut self, label: &str) -> String {
        if !label.starts_with('#') {
            return label.to_string();
        }
        let number = self.auto_footnotes.get(self.next_auto_definition).copied();
        self.next_auto_definition += 1;
        number.map_or_else(|| label.to_string(), |number| number.to_string())
    }

    /// The number of a named auto footnote (`#name`).
    fn named_footnote_number(&self, label: &str) -> Option<String> {
        let name = label.strip_prefix('#')?;
        self.named_footnotes.get(&normalize_name(name)).map(u32::to_string)
    }

    /// `.. _name: url` (or `` .. _`a name`: url ``); a target with no URL
    /// marks a place in the document and links nowhere here.
    fn add_named_target(&mut self, rest: &str, lines: &[&str], index: usize) {
        let Some((name, value)) = split_target(rest) else {
            return;
        };
        let url = target_value(value, lines, index);
        if !url.is_empty() {
            self.targets.insert(normalize_name(name), url);
        }
    }

    /// `` `text <url>`_ `` in running text also defines a target `text`.
    fn add_embedded_targets(&mut self, line: &str) {
        let mut rest = line;
        while let Some(end) = rest.find(">`_") {
            let before = &rest[..end];
            let after = &rest[end + 3..];
            rest = after;
            if after.starts_with('_') {
                // `` `text <url>`__ `` is anonymous and defines nothing.
                continue;
            }
            let Some(open) = before.rfind('`') else { continue };
            let Some((text, url)) = before[open + 1..].rsplit_once('<') else { continue };
            let text = text.trim();
            if !text.is_empty() && !url.ends_with('_') {
                self.targets.entry(normalize_name(text)).or_insert_with(|| url.trim().to_string());
            }
        }
    }

    /// `.. |name| replace:: text` or `.. |name| image:: url` with an
    /// optional `:alt:` on the lines below.
    fn add_substitution(&mut self, rest: &str, lines: &[&str], index: usize) {
        let Some((name, definition)) = rest.split_once('|') else {
            return;
        };
        let definition = definition.trim_start();
        let substitution = if let Some(text) = definition.strip_prefix("replace::") {
            let mut text = text.trim().to_string();
            for line in following_body(lines, index) {
                text.push(' ');
                text.push_str(line.trim());
            }
            Substitution::Text(text.trim().to_string())
        } else if let Some(url) = definition.strip_prefix("image::") {
            let alt = following_body(lines, index)
                .find_map(|line| line.trim().strip_prefix(":alt:").map(|alt| alt.trim().to_string()))
                .unwrap_or_default();
            Substitution::Image { url: url.trim().to_string(), alt }
        } else if let Some(text) = definition.strip_prefix("unicode::") {
            Substitution::Text(text.trim().to_string())
        } else {
            return;
        };
        self.substitutions.insert(normalize_name(name), substitution);
    }

    /// Give auto-numbered footnotes (`[#]`, `[#name]`) the lowest numbers
    /// that manually numbered footnotes (`[3]`) do not take.
    fn number_footnotes(&mut self, labels: &[String]) {
        let taken: HashSet<u32> = labels.iter().filter_map(|label| label.parse().ok()).collect();
        let mut next = 1;
        for label in labels.iter().filter(|label| label.starts_with('#')) {
            while taken.contains(&next) {
                next += 1;
            }
            self.auto_footnotes.push(next);
            match &label[1..] {
                "" => self.anonymous_footnotes.push(next),
                name => {
                    self.named_footnotes.insert(normalize_name(name), next);
                }
            }
            next += 1;
        }
    }

    /// Follow targets whose value is a reference to another target
    /// (`.. _alias: real_`); drop those that lead nowhere.
    fn resolve_indirect_targets(&mut self) {
        let names: Vec<String> = self.targets.keys().cloned().collect();
        for name in names {
            let mut url = self.targets[&name].clone();
            let mut hops = 0;
            while let Some(reference) = indirect_reference(&url) {
                hops += 1;
                match self.targets.get(&normalize_name(&reference)) {
                    Some(next) if hops <= MAX_INDIRECT_HOPS => url.clone_from(next),
                    _ => {
                        url.clear();
                        break;
                    }
                }
            }
            if url.is_empty() {
                self.targets.remove(&name);
            } else {
                self.targets.insert(name, url);
            }
        }
    }
}

/// Split `name: value` (or `` `a name`: value ``) after `.. _`.
fn split_target(rest: &str) -> Option<(&str, &str)> {
    if let Some(quoted) = rest.strip_prefix('`') {
        let (name, after) = quoted.split_once('`')?;
        return Some((name, after.strip_prefix(':')?));
    }
    // The name ends at the first colon followed by a space or the line end.
    let bytes = rest.as_bytes();
    let colon = (0..bytes.len()).find(|&at| bytes[at] == b':' && bytes.get(at + 1).is_none_or(|next| *next == b' '))?;
    Some((&rest[..colon], &rest[colon + 1..]))
}

/// A target's URL: `value` plus the indented lines below it, joined
/// without spaces (a long URL may wrap).
fn target_value(value: &str, lines: &[&str], index: usize) -> String {
    let mut url = value.trim().to_string();
    for line in following_body(lines, index) {
        url.push_str(line.trim());
    }
    url
}

/// The non-blank lines right below `lines[index]` that are indented
/// deeper than it.
fn following_body<'a>(lines: &[&'a str], index: usize) -> impl Iterator<Item = &'a str> {
    let base = indent(lines[index]);
    lines[index + 1..].iter().copied().take_while(move |line| !line.trim().is_empty() && indent(line) > base)
}

/// The target name in an indirect target value (`name_` or `` `a name`_ ``).
fn indirect_reference(value: &str) -> Option<String> {
    if value.contains("://") || value.starts_with("mailto:") {
        return None;
    }
    let name = value.strip_suffix('_')?;
    let name = name.strip_prefix('`').and_then(|name| name.strip_suffix('`')).unwrap_or(name);
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(text: &str) -> Refs {
        let lines: Vec<&str> = text.lines().collect();
        Refs::collect(&lines)
    }

    #[test]
    fn named_targets_are_case_and_space_insensitive() {
        let refs = collect(".. _Python Docs: https://docs.python.org\n.. _`odd: name`: https://x.org/\n");
        assert_eq!(refs.target("python   docs"), Some("https://docs.python.org"));
        assert_eq!(refs.target("odd: name"), Some("https://x.org/"));
        assert_eq!(refs.target("missing"), None);
    }

    #[test]
    fn wrapped_urls_and_indirect_targets_resolve() {
        let refs = collect(".. _long: https://example.com/\n   very/long\n.. _alias: long_\n.. _loop: loop_\n");
        assert_eq!(refs.target("long"), Some("https://example.com/very/long"));
        assert_eq!(refs.target("alias"), Some("https://example.com/very/long"));
        assert_eq!(refs.target("loop"), None);
    }

    #[test]
    fn anonymous_targets_are_taken_in_order() {
        let mut refs = collect("__ https://a.org\n.. __: https://b.org\n");
        assert_eq!(refs.next_anonymous_target().as_deref(), Some("https://a.org"));
        assert_eq!(refs.next_anonymous_target().as_deref(), Some("https://b.org"));
        assert_eq!(refs.next_anonymous_target(), None);
    }

    #[test]
    fn embedded_targets_can_be_reused() {
        let refs = collect("See `Rust <https://rust-lang.org>`_ and `once <https://x.org>`__.");
        assert_eq!(refs.target("rust"), Some("https://rust-lang.org"));
        assert_eq!(refs.target("once"), None);
    }

    #[test]
    fn substitutions_hold_text_or_images() {
        let refs = collect(".. |name| replace:: *Rubrique*\n.. |logo| image:: logo.png\n   :alt: The logo\n");
        assert_eq!(refs.substitution("NAME"), Some(&Substitution::Text("*Rubrique*".to_string())));
        assert_eq!(
            refs.substitution("logo"),
            Some(&Substitution::Image { url: "logo.png".to_string(), alt: "The logo".to_string() })
        );
    }

    #[test]
    fn auto_footnotes_skip_manual_numbers() {
        let mut refs = collect(".. [1] manual\n.. [#] first auto\n.. [#note] named\n");
        assert_eq!(refs.footnote_reference_label("#note"), "3");
        assert_eq!(refs.footnote_reference_label("#"), "2");
        assert_eq!(refs.footnote_reference_label("1"), "1");
        assert_eq!(refs.footnote_definition_label("1"), "1");
        assert_eq!(refs.footnote_definition_label("#"), "2");
        assert_eq!(refs.footnote_definition_label("#note"), "3");
    }
}
