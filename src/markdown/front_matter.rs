//! YAML front matter at the top of a Markdown file, and what an Open
//! Knowledge Format (OKF) concept says in it.
//!
//! OKF (<https://github.com/GoogleCloudPlatform/open-knowledge-format>) is a
//! folder of Markdown files with front matter. The preview reads the
//! top-level keys as text and the simple YAML shapes OKF writes in them
//! (`[a, b]` lists, `- ` lists, `{ by: x, at: y }` mappings), with no YAML
//! library: enough to show each field readably. The file itself is never
//! rewritten.

/// The line that opens and closes a front matter block.
const FENCE: &str = "---";

/// Top-level keys of a front matter block, in file order, each with its
/// value as written: the text after `key:`, plus any indented or list lines
/// under it (joined with line breaks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontMatter {
    pub fields: Vec<(String, String)>,
}

/// How far an OKF concept has been checked, from its `verified` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustTier {
    /// No `verified` field.
    Unverified,
    /// Verified by agents or processes only.
    MachineConfirmed,
    /// Verified by a `human:<id>` actor.
    HumanReviewed,
}

impl TrustTier {
    /// The name OKF gives the tier.
    pub fn label(self) -> &'static str {
        match self {
            TrustTier::Unverified => "Unverified",
            TrustTier::MachineConfirmed => "Machine-confirmed",
            TrustTier::HumanReviewed => "Human-reviewed",
        }
    }
}

/// Who did something to a concept and when: one `generated` or `verified` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// An actor: `human:<id>`, `process:<id>` or `<producer>/<version>`.
    pub actor: String,
    pub at: Option<String>,
}

/// One entry of an OKF concept's `sources`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub title: Option<String>,
    /// A URL, a path in the bundle, or a description of a scope.
    pub resource: Option<String>,
    pub author: Option<String>,
    pub usage_count: Option<String>,
    pub last_modified: Option<String>,
    /// The entry as written, for one that names none of the fields above.
    pub text: String,
}

impl FrontMatter {
    /// The front matter that opens `lines`, and how many lines it takes
    /// (both fences included). `None` when the first line is not `---`,
    /// there is no closing `---`, or a line between them is not YAML-like,
    /// so a document that starts with a horizontal rule stays one.
    pub fn parse(lines: &[&str]) -> Option<(FrontMatter, usize)> {
        if lines.first()?.trim_end() != FENCE {
            return None;
        }
        let close = lines.iter().skip(1).position(|line| line.trim_end() == FENCE)? + 1;
        let mut fields: Vec<(String, String)> = Vec::new();
        for line in &lines[1..close] {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let continues = line.starts_with([' ', '\t']) || line.starts_with("- ");
            if continues {
                let (_, value) = fields.last_mut()?;
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(line.trim_end());
                continue;
            }
            let (key, value) = line.split_once(':')?;
            if key.is_empty() || key.contains(char::is_whitespace) {
                return None;
            }
            fields.push((key.to_string(), value.trim().to_string()));
        }
        if fields.is_empty() {
            return None;
        }
        for (_, value) in &mut fields {
            *value = dedented(value);
        }
        Some((FrontMatter { fields }, close + 1))
    }

    /// The value of `key`, without quotes around it; `None` when missing or empty.
    pub fn get(&self, key: &str) -> Option<&str> {
        let (_, value) = self.fields.iter().find(|(name, _)| name == key)?;
        let value = unquote(value.trim());
        (!value.is_empty()).then_some(value)
    }

    /// The OKF concept type (`type`), which makes the file an OKF concept.
    pub fn concept_type(&self) -> Option<&str> {
        self.get("type")
    }

    /// The OKF trust tier, from who is listed under `verified`.
    pub fn trust_tier(&self) -> TrustTier {
        if self.get("verified").is_none() {
            return TrustTier::Unverified;
        }
        let human = self.events("verified").iter().any(|event| event.actor.starts_with("human:"));
        if human { TrustTier::HumanReviewed } else { TrustTier::MachineConfirmed }
    }

    /// The `generated` or `verified` entries under `key` that name who (`by`).
    pub fn events(&self, key: &str) -> Vec<Event> {
        let items = self.get(key).map(list_items).unwrap_or_default();
        items
            .iter()
            .filter_map(|item| {
                let fields = mapping(item);
                let actor = field(&fields, "by")?.to_string();
                Some(Event { actor, at: field(&fields, "at").map(str::to_string) })
            })
            .collect()
    }

    /// The entries of `sources`.
    pub fn sources(&self) -> Vec<Source> {
        let items = self.get("sources").map(list_items).unwrap_or_default();
        items
            .into_iter()
            .map(|item| {
                let fields = mapping(&item);
                let take = |name: &str| field(&fields, name).map(str::to_string);
                Source {
                    title: take("title"),
                    resource: take("resource"),
                    author: take("author"),
                    usage_count: take("usage_count"),
                    last_modified: take("last_modified"),
                    text: item,
                }
            })
            .collect()
    }

    /// The `stale_after` date (`YYYY-MM-DD`) when it is `today` or earlier.
    /// Only the day is compared, not the time or the zone.
    pub fn stale_since(&self, today: &str) -> Option<&str> {
        let date = self.get("stale_after")?.get(..10)?;
        (date <= today).then_some(date)
    }
}

/// The items of a list value: `[a, b]`, or `- ` lines (an item's indented
/// lines stay with it), or the value itself as the only item.
pub fn list_items(value: &str) -> Vec<String> {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        return inner.split(',').map(|item| unquote(item.trim()).to_string()).filter(|item| !item.is_empty()).collect();
    }
    if !value.starts_with("- ") {
        return if value.is_empty() { Vec::new() } else { vec![unquote(value).to_string()] };
    }
    let mut items: Vec<String> = Vec::new();
    for line in value.lines() {
        match (line.strip_prefix("- "), items.last_mut()) {
            (Some(item), _) => items.push(item.trim().to_string()),
            (None, Some(item)) => {
                item.push('\n');
                item.push_str(line.trim());
            }
            (None, None) => {}
        }
    }
    items
}

/// The `key: value` pairs of a mapping: `{ by: x, at: y }` on one line, or
/// one pair per line. Values lose their quotes.
pub fn mapping(item: &str) -> Vec<(String, String)> {
    let item = item.trim();
    let pairs: Vec<&str> = match item.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')) {
        Some(inner) => inner.split(',').collect(),
        None => item.lines().collect(),
    };
    pairs
        .iter()
        .filter_map(|pair| {
            let (key, value) = pair.split_once(':')?;
            Some((key.trim().to_string(), unquote(value.trim()).to_string()))
        })
        .collect()
}

/// The value of `key` in pairs from [`mapping`], when not empty.
fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str()).filter(|value| !value.is_empty())
}

/// An actor's name and what it is: `human:kliu` is `kliu`, a person;
/// `process:nightly` a process; `agent/1.0` an agent. Other names have no kind.
pub fn actor_parts(actor: &str) -> (&str, Option<&'static str>) {
    if let Some(name) = actor.strip_prefix("human:") {
        (name, Some("person"))
    } else if let Some(name) = actor.strip_prefix("process:") {
        (name, Some("process"))
    } else if actor.contains('/') {
        (actor, Some("agent"))
    } else {
        (actor, None)
    }
}

/// Short English month names, January first.
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// An ISO 8601 date or timestamp as `Jun 30, 2026`; `None` when `value`
/// does not start with a `YYYY-MM-DD` date.
pub fn readable_date(value: &str) -> Option<String> {
    let date = value.get(..10)?;
    let rest = &value[10..];
    if !(rest.is_empty() || rest.starts_with(['T', ' '])) {
        return None;
    }
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if year.len() != 4 || !year.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let month = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    let day: u8 = day.parse().ok().filter(|day| (1..=31).contains(day))?;
    Some(format!("{month} {day}, {year}"))
}

/// `value` with the indentation its lines share removed, so a nested list
/// keeps its shape when shown on its own.
fn dedented(value: &str) -> String {
    let indent = value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    value.lines().map(|line| line.get(indent..).unwrap_or(line.trim_start())).collect::<Vec<_>>().join("\n")
}

/// `value` without one pair of matching quotes around it.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote)) {
            return inner;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<(FrontMatter, usize)> {
        let lines: Vec<&str> = text.lines().collect();
        FrontMatter::parse(&lines)
    }

    const CONCEPT: &str = "---
type: BigQuery Table
title: \"Customer Orders\"
tags: [sales, orders]
sources:
  - id: ga4
    resource: https://example.com
verified:
  - { by: process:nightly, at: 2026-06-26T02:00:00Z }
stale_after: 2026-09-23T00:00:00Z
---

# Schema";

    #[test]
    fn reads_top_level_keys_and_their_nested_lines() {
        let (front, lines) = parse(CONCEPT).unwrap();
        assert_eq!(lines, 11);
        assert_eq!(front.concept_type(), Some("BigQuery Table"));
        assert_eq!(front.get("title"), Some("Customer Orders"));
        assert_eq!(front.get("tags"), Some("[sales, orders]"));
        assert_eq!(front.get("sources"), Some("- id: ga4\n  resource: https://example.com"));
        assert_eq!(front.get("missing"), None);
    }

    #[test]
    fn a_rule_or_prose_between_rules_is_not_front_matter() {
        assert_eq!(parse("---\n\nSome text.\n\n---"), None);
        assert_eq!(parse("---\ntype: x"), None);
        assert_eq!(parse("text\n---\ntype: x\n---"), None);
        assert_eq!(parse("---\n---"), None);
    }

    #[test]
    fn trust_tier_follows_who_verified() {
        let tier = |verified: &str| parse(&format!("---\ntype: x\n{verified}---")).unwrap().0.trust_tier();
        assert_eq!(tier(""), TrustTier::Unverified);
        assert_eq!(tier("verified: { by: process:nightly, at: 2026-06-26T02:00:00Z }\n"), TrustTier::MachineConfirmed);
        assert_eq!(
            tier("verified:\n  - { by: process:nightly }\n  - { by: \"human:ahormati\" }\n"),
            TrustTier::HumanReviewed
        );
    }

    #[test]
    fn lists_in_both_forms() {
        assert_eq!(list_items("[sales, \"orders\", ]"), ["sales", "orders"]);
        assert_eq!(list_items("- a\n- id: b\n  title: B"), ["a", "id: b\ntitle: B"]);
        assert_eq!(list_items("single"), ["single"]);
        assert_eq!(list_items(""), Vec::<String>::new());
    }

    #[test]
    fn events_and_sources() {
        let (front, _) = parse(CONCEPT).unwrap();
        assert_eq!(
            front.events("verified"),
            [Event { actor: "process:nightly".into(), at: Some("2026-06-26T02:00:00Z".into()) }]
        );
        let sources = front.sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].resource.as_deref(), Some("https://example.com"));
        assert_eq!(sources[0].title, None);
        let (bare, _) = parse("---\ngenerated: { by: human:ann, at: \"2026-01-02T00:00:00Z\" }\n---").unwrap();
        assert_eq!(bare.events("generated")[0].at.as_deref(), Some("2026-01-02T00:00:00Z"));
    }

    #[test]
    fn actors_and_dates_read_plainly() {
        assert_eq!(actor_parts("human:kliu@acme"), ("kliu@acme", Some("person")));
        assert_eq!(actor_parts("process:nightly"), ("nightly", Some("process")));
        assert_eq!(actor_parts("reference_agent/gemini-2.5-pro"), ("reference_agent/gemini-2.5-pro", Some("agent")));
        assert_eq!(actor_parts("team:data"), ("team:data", None));
        assert_eq!(readable_date("2026-06-30T14:00:00Z").as_deref(), Some("Jun 30, 2026"));
        assert_eq!(readable_date("2026-01-05").as_deref(), Some("Jan 5, 2026"));
        assert_eq!(readable_date("2026-13-05"), None);
        assert_eq!(readable_date("2026-06-30-extra"), None);
        assert_eq!(readable_date("soon"), None);
    }

    #[test]
    fn stale_on_and_after_the_date() {
        let (front, _) = parse(CONCEPT).unwrap();
        assert_eq!(front.stale_since("2026-09-22"), None);
        assert_eq!(front.stale_since("2026-09-23"), Some("2026-09-23"));
        assert_eq!(front.stale_since("2027-01-01"), Some("2026-09-23"));
    }
}
