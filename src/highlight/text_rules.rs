//! Text highlight rules: regular expressions from Settings that color
//! matches (IP addresses, numbers, UUIDs, hashes) in plain text files.
//!
//! Rules are tried top to bottom and the first rule that matches a piece of
//! text colors it, so a specific rule (a UUID) wins over a broad one (a
//! number) placed below it. A rule whose pattern does not compile is
//! skipped; [`pattern_error`] says why, for Settings to show.

use std::collections::BTreeMap;
use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::theme::palette::Rgb;

/// Largest compiled program one pattern may grow to. Patterns such as
/// `\w{1000}{1000}` would otherwise take a lot of memory and time to build.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;
/// Longer texts are left plain: every rule scans the whole text again after
/// each edit, and a huge log should stay quick to type in.
pub const MAX_HIGHLIGHT_BYTES: usize = 2_000_000;

/// The color of a rule: one of the code colors (so it follows the theme and
/// the Code colors setting) or a color the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleColor {
    Keyword,
    String,
    Number,
    Name,
    Key,
    Comment,
    Custom(Rgb),
}

impl RuleColor {
    /// The code colors a rule can borrow, in the order Settings offers them.
    pub const PALETTE: [RuleColor; 6] =
        [RuleColor::Keyword, RuleColor::String, RuleColor::Number, RuleColor::Name, RuleColor::Key, RuleColor::Comment];

    /// The word stored in the settings file: a palette name or `#rrggbb`.
    pub fn id(self) -> String {
        match self {
            RuleColor::Keyword => "keyword".to_string(),
            RuleColor::String => "string".to_string(),
            RuleColor::Number => "number".to_string(),
            RuleColor::Name => "name".to_string(),
            RuleColor::Key => "key".to_string(),
            RuleColor::Comment => "comment".to_string(),
            RuleColor::Custom(Rgb(red, green, blue)) => format!("#{red:02x}{green:02x}{blue:02x}"),
        }
    }

    /// Read a stored word; `None` when it is neither a palette name nor a color.
    pub fn parse(value: &str) -> Option<Self> {
        RuleColor::PALETTE.into_iter().find(|color| color.id() == value).or_else(|| {
            // Only `#rrggbb` is a custom color; a bare word is a palette name.
            value.starts_with('#').then(|| Rgb::parse(value)).flatten().map(RuleColor::Custom)
        })
    }

    /// Shown in the color picker.
    pub fn name(self) -> &'static str {
        match self {
            RuleColor::Keyword => "Keyword",
            RuleColor::String => "String",
            RuleColor::Number => "Number",
            RuleColor::Name => "Name",
            RuleColor::Key => "Key",
            RuleColor::Comment => "Comment",
            RuleColor::Custom(_) => "Custom",
        }
    }
}

/// One rule as the user sees and edits it in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HighlightRule {
    pub name: String,
    /// A regular expression in the `regex` crate's syntax.
    pub pattern: String,
    pub color: RuleColor,
    pub enabled: bool,
}

impl HighlightRule {
    /// A switched-on rule.
    pub fn new(name: &str, pattern: &str, color: RuleColor) -> Self {
        HighlightRule { name: name.to_string(), pattern: pattern.to_string(), color, enabled: true }
    }

    /// The settings value: `on`/`off`, color, pattern and name, joined by
    /// tabs. The name goes last so the settings reader, which trims the
    /// value, can only cut spaces from the end of the name, never from the
    /// pattern. Tabs and line breaks inside the pattern are written as
    /// `\t` and `\n`, which mean the same in a regular expression.
    pub fn to_value(&self) -> String {
        let pattern = self.pattern.replace('\t', "\\t").replace('\n', "\\n");
        let name = self.name.replace(['\t', '\n'], " ");
        let state = if self.enabled { "on" } else { "off" };
        format!("{state}\t{}\t{pattern}\t{name}", self.color.id())
    }

    /// Read a value written by [`HighlightRule::to_value`]; `None` when it is broken.
    pub fn from_value(value: &str) -> Option<Self> {
        let mut parts = value.splitn(4, '\t');
        let enabled = match parts.next()? {
            "on" => true,
            "off" => false,
            _ => return None,
        };
        let color = RuleColor::parse(parts.next()?)?;
        let pattern = parts.next()?.to_string();
        let name = parts.next().unwrap_or_default().trim().to_string();
        Some(HighlightRule { name, pattern, color, enabled })
    }
}

/// Octet of an IPv4 address: 0 to 255, without leading zeros.
const OCTET: &str = "(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])";

/// The rules a new install starts with. They are ordinary rules: the user can
/// change, reorder, switch off or delete each one.
pub fn default_rules() -> Vec<HighlightRule> {
    let ipv4 = format!(r"\b(?:{OCTET}\.){{3}}{OCTET}\b");
    vec![
        HighlightRule::new(
            "UUID",
            r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            RuleColor::Name,
        ),
        HighlightRule::new("IPv4 address", &ipv4, RuleColor::Keyword),
        // Full form, then the shortened forms with `::` (fe80::1, ::1,
        // 2001:db8::). Clock times like 12:30:45 have neither.
        HighlightRule::new(
            "IPv6 address",
            r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|\b(?:[0-9a-f]{1,4}:){1,6}(?::[0-9a-f]{1,4}){1,6}\b|\b(?:[0-9a-f]{1,4}:){1,7}:|::(?:[0-9a-f]{1,4}:){0,5}[0-9a-f]{1,4}\b",
            RuleColor::Keyword,
        ),
        HighlightRule::new("Email", r"\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b", RuleColor::String),
        HighlightRule::new("Date", r"\b[0-9]{4}-[0-9]{2}-[0-9]{2}\b", RuleColor::Key),
        // Whole words only, so the digits inside a hash or an id stay theirs.
        HighlightRule::new("Number", r"\b[0-9]+(?:\.[0-9]+)?\b", RuleColor::Number),
        // Git commit ids (short and full) and SHA-256 sums. Below Number, so
        // a long plain number stays a number.
        HighlightRule::new("Hash", r"\b[0-9a-f]{7,64}\b", RuleColor::Comment),
    ]
}

/// Compile one pattern with Rubrique's limits. Matching is linear in the
/// text's length for every pattern (the `regex` crate has no backtracking).
fn compile(pattern: &str) -> Result<Regex, String> {
    if pattern.is_empty() {
        return Err("Write a pattern".to_string());
    }
    RegexBuilder::new(pattern).size_limit(PATTERN_SIZE_LIMIT).build().map_err(|error| short_error(&error))
}

/// The last line of the crate's message ("unclosed group"), without the
/// drawing of the pattern above it.
fn short_error(error: &regex::Error) -> String {
    let text = error.to_string();
    let last = text.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or("Invalid pattern");
    last.strip_prefix("error: ").unwrap_or(last).to_string()
}

/// Why `pattern` cannot be used, or `None` when it compiles.
pub fn pattern_error(pattern: &str) -> Option<String> {
    compile(pattern).err()
}

/// The switched-on rules that compile, built once when the rules change and
/// shared by every layout until they change again. Two are equal (and hash
/// the same) when they come from the same rules, so layouts cached for one
/// stay valid for the other.
#[derive(Debug, Default)]
pub struct CompiledRules {
    /// The rules as written, to tell when they changed.
    source: Vec<HighlightRule>,
    /// The usable rules, in order, each with its place in `source`.
    compiled: Vec<(usize, Regex)>,
}

impl CompiledRules {
    /// Compile the switched-on rules; broken patterns are left out.
    pub fn new(rules: &[HighlightRule]) -> Self {
        let compiled = rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.enabled)
            .filter_map(|(index, rule)| Some((index, compile(&rule.pattern).ok()?)))
            .collect();
        CompiledRules { source: rules.to_vec(), compiled }
    }

    /// The rules these were compiled from.
    pub fn source(&self) -> &[HighlightRule] {
        &self.source
    }

    /// The colored pieces of `text`, in order and never overlapping. Each
    /// piece belongs to the first rule that matched it; a match that would
    /// overlap a piece already taken is dropped.
    pub fn spans(&self, text: &str) -> Vec<(Range<usize>, RuleColor)> {
        self.pieces(text).into_iter().map(|(range, index)| (range, self.source[index].color)).collect()
    }

    /// How many pieces of `text` each rule colors, by its place in the rules
    /// (zero for rules that are off or broken), for Settings' sample.
    pub fn match_counts(&self, text: &str) -> Vec<usize> {
        let mut counts = vec![0; self.source.len()];
        for (_, index) in self.pieces(text) {
            counts[index] += 1;
        }
        counts
    }

    /// The pieces [`CompiledRules::spans`] colors, each with the place of the
    /// rule that took it.
    fn pieces(&self, text: &str) -> Vec<(Range<usize>, usize)> {
        if text.len() > MAX_HIGHLIGHT_BYTES {
            return Vec::new();
        }
        // Start of each piece taken so far, with its end and rule.
        let mut taken: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
        for (index, regex) in &self.compiled {
            for found in regex.find_iter(text) {
                let range = found.range();
                if !range.is_empty() && !overlaps_taken(&taken, &range) {
                    taken.insert(range.start, (range.end, *index));
                }
            }
        }
        taken.into_iter().map(|(start, (end, index))| (start..end, index)).collect()
    }
}

impl PartialEq for CompiledRules {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for CompiledRules {}

impl std::hash::Hash for CompiledRules {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
    }
}

/// Whether `range` overlaps a taken piece. Pieces never overlap each other,
/// so only the last one starting before `range` ends can reach into it.
fn overlaps_taken(taken: &BTreeMap<usize, (usize, usize)>, range: &Range<usize>) -> bool {
    taken.range(..range.end).next_back().is_some_and(|(_, (end, _))| *end > range.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each colored piece, with its color.
    fn pieces<'a>(rules: &CompiledRules, text: &'a str) -> Vec<(&'a str, RuleColor)> {
        rules.spans(text).into_iter().map(|(range, color)| (&text[range], color)).collect()
    }

    /// The matched text of the default rule called `name` in `text`.
    fn default_matches<'a>(name: &str, text: &'a str) -> Vec<&'a str> {
        let rule = default_rules().into_iter().find(|rule| rule.name == name).expect("a default rule");
        pieces(&CompiledRules::new(&[rule]), text).into_iter().map(|(piece, _)| piece).collect()
    }

    #[test]
    fn every_default_rule_compiles() {
        for rule in default_rules() {
            assert_eq!(pattern_error(&rule.pattern), None, "{}", rule.name);
        }
    }

    #[test]
    fn default_rules_find_what_they_say() {
        assert_eq!(default_matches("IPv4 address", "from 10.0.0.255 to 256.1.1.1"), ["10.0.0.255"]);
        assert_eq!(
            default_matches("IPv6 address", "fe80::1 and 2001:db8::8a2e:370:7334"),
            ["fe80::1", "2001:db8::8a2e:370:7334"]
        );
        assert_eq!(
            default_matches("IPv6 address", "::1 2001:0db8:0000:0000:0000:ff00:0042:8329"),
            ["::1", "2001:0db8:0000:0000:0000:ff00:0042:8329"]
        );
        assert_eq!(default_matches("IPv6 address", "at 12:30:45 sharp"), Vec::<&str>::new());
        assert_eq!(
            default_matches("UUID", "id 550E8400-e29b-41d4-a716-446655440000."),
            ["550E8400-e29b-41d4-a716-446655440000"]
        );
        assert_eq!(default_matches("Hash", "commit 3f9a2c1 and deadbeefcafe0"), ["3f9a2c1", "deadbeefcafe0"]);
        assert_eq!(default_matches("Hash", "abc12 is too short"), Vec::<&str>::new());
        assert_eq!(default_matches("Number", "v2 has 42 and 3.14"), ["42", "3.14"]);
        assert_eq!(default_matches("Date", "due 2026-11-10."), ["2026-11-10"]);
        assert_eq!(default_matches("Email", "write to a.b+c@mail.example.org."), ["a.b+c@mail.example.org"]);
    }

    #[test]
    fn the_first_rule_wins_an_overlap() {
        let rules = CompiledRules::new(&default_rules());
        let text = "host 192.168.1.10 id 550e8400-e29b-41d4-a716-446655440000 count 7";
        assert_eq!(
            pieces(&rules, text),
            [
                ("192.168.1.10", RuleColor::Keyword),
                ("550e8400-e29b-41d4-a716-446655440000", RuleColor::Name),
                ("7", RuleColor::Number),
            ]
        );
    }

    #[test]
    fn rule_order_decides() {
        let number = HighlightRule::new("Number", "[0-9]+", RuleColor::Number);
        let year = HighlightRule::new("Year", "20[0-9]{2}", RuleColor::Key);
        let number_first = CompiledRules::new(&[number.clone(), year.clone()]);
        assert_eq!(pieces(&number_first, "in 2026"), [("2026", RuleColor::Number)]);
        let year_first = CompiledRules::new(&[year, number]);
        assert_eq!(pieces(&year_first, "in 2026"), [("2026", RuleColor::Key)]);
    }

    #[test]
    fn broken_switched_off_and_empty_rules_are_skipped() {
        let rules = CompiledRules::new(&[
            HighlightRule::new("Broken", "(unclosed", RuleColor::Keyword),
            HighlightRule { enabled: false, ..HighlightRule::new("Off", "[a-z]+", RuleColor::Name) },
            HighlightRule::new("Empty matches", "x*", RuleColor::Key),
            HighlightRule::new("Digits", "[0-9]+", RuleColor::Number),
        ]);
        assert_eq!(pieces(&rules, "abc 12"), [("12", RuleColor::Number)]);
    }

    #[test]
    fn match_counts_follow_the_rule_that_wins() {
        let rules = CompiledRules::new(&[
            HighlightRule::new("Year", "20[0-9]{2}", RuleColor::Key),
            HighlightRule { enabled: false, ..HighlightRule::new("Off", "[a-z]+", RuleColor::Name) },
            HighlightRule::new("Number", "[0-9]+", RuleColor::Number),
        ]);
        // 2026 goes to Year, so Number only counts 7 and 12.
        assert_eq!(rules.match_counts("in 2026, 7 of 12"), [1, 0, 2]);
    }

    #[test]
    fn pattern_errors_are_short() {
        assert_eq!(pattern_error("[0-9]+"), None);
        assert_eq!(pattern_error("").as_deref(), Some("Write a pattern"));
        let error = pattern_error("(unclosed").expect("an error");
        assert!(!error.contains('\n'), "{error}");
        assert!(error.contains("unclosed"), "{error}");
        // Look-around is not supported by the crate and is reported, not a crash.
        assert!(pattern_error("(?=a)b").is_some());
        // A pattern that would compile to a huge program is refused.
        assert!(pattern_error(r"\w{1000}{1000}").is_some());
    }

    #[test]
    fn huge_texts_stay_plain() {
        let rules = CompiledRules::new(&default_rules());
        let text = "1 ".repeat(MAX_HIGHLIGHT_BYTES / 2 + 1);
        assert_eq!(rules.spans(&text), Vec::new());
    }

    #[test]
    fn rules_round_trip_through_values() {
        let rules = [
            HighlightRule::new("Ticket", r"\bPROJ-[0-9]+\b", RuleColor::Custom(Rgb(0xff, 0x80, 0x00))),
            HighlightRule { enabled: false, ..HighlightRule::new("Spaces ", "a b  ", RuleColor::Comment) },
        ];
        for rule in rules {
            let parsed = HighlightRule::from_value(rule.to_value().trim()).expect("parses");
            assert_eq!(parsed, HighlightRule { name: rule.name.trim().to_string(), ..rule });
        }
    }

    #[test]
    fn tabs_in_patterns_are_written_as_escapes() {
        let rule = HighlightRule::new("Tab\tname", "a\tb", RuleColor::Key);
        let parsed = HighlightRule::from_value(&rule.to_value()).expect("parses");
        assert_eq!(parsed.pattern, r"a\tb");
        assert_eq!(parsed.name, "Tab name");
    }

    #[test]
    fn broken_values_are_ignored() {
        assert_eq!(HighlightRule::from_value("maybe\tkeyword\t[0-9]\tX"), None);
        assert_eq!(HighlightRule::from_value("on\tpurple\t[0-9]\tX"), None);
        assert_eq!(HighlightRule::from_value("on"), None);
        assert_eq!(RuleColor::parse("#12345"), None);
        assert_eq!(RuleColor::parse("#a0b1c2"), Some(RuleColor::Custom(Rgb(0xa0, 0xb1, 0xc2))));
    }
}
