//! The text highlight rules in the settings file: a `text_highlights` line
//! (the on and off switch), a `highlight_rules` line, then one
//! `highlight_rule` line per rule, in order.

use std::fmt::Write as _;

use crate::highlight::text_rules::{HighlightRule, default_rules};

/// Key of the line that says whether highlights are on (`true` or `false`).
const ENABLED_KEY: &str = "text_highlights";
/// Key of the line written before the rules. A file without it was saved
/// before rules existed, so it gets the default rules; a file with it and no
/// rule lines means the user deleted them all.
const RULES_KEY: &str = "highlight_rules";
/// Key of each rule's line.
const RULE_KEY: &str = "highlight_rule";

/// The user's rules for coloring plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextHighlightSettings {
    /// Highlights are on. Off leaves plain text uncolored but keeps the rules.
    pub enabled: bool,
    /// Tried top to bottom; the first match colors the text.
    pub rules: Vec<HighlightRule>,
}

impl Default for TextHighlightSettings {
    fn default() -> Self {
        TextHighlightSettings { enabled: true, rules: default_rules() }
    }
}

impl TextHighlightSettings {
    /// Apply one `key=value` line. Returns false when the key is not a
    /// highlight key. A broken rule line is skipped.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            ENABLED_KEY => self.enabled = value == "true",
            // The saved list replaces the defaults.
            RULES_KEY => self.rules.clear(),
            RULE_KEY => self.rules.extend(HighlightRule::from_value(value)),
            _ => return false,
        }
        true
    }

    /// The `key=value` lines that [`TextHighlightSettings::parse_line`] reads.
    pub fn to_lines(&self) -> String {
        let mut lines = format!("{ENABLED_KEY}={}\n{RULES_KEY}={}\n", self.enabled, self.rules.len());
        for rule in &self.rules {
            let _ = writeln!(lines, "{RULE_KEY}={}", rule.to_value());
        }
        lines
    }

    /// The rules that color text now: all of them, or none while highlights are off.
    pub fn active_rules(&self) -> &[HighlightRule] {
        if self.enabled { &self.rules } else { &[] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::text_rules::RuleColor;

    /// Read `text` the way the settings file is read: trimmed values.
    fn parse(text: &str) -> TextHighlightSettings {
        let mut settings = TextHighlightSettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').expect("every line is key=value");
            assert!(settings.parse_line(key.trim(), value.trim()));
        }
        settings
    }

    #[test]
    fn round_trips_through_lines() {
        let mut settings = TextHighlightSettings { enabled: false, ..TextHighlightSettings::default() };
        settings.rules[0].enabled = false;
        settings.rules.push(HighlightRule::new("Ticket", "PROJ-[0-9]+ = done", RuleColor::Key));
        assert_eq!(parse(&settings.to_lines()), settings);
    }

    #[test]
    fn files_without_rules_get_the_defaults() {
        let mut settings = TextHighlightSettings::default();
        assert!(!settings.parse_line("theme", "dark"));
        assert_eq!(settings.rules, default_rules());
    }

    #[test]
    fn deleting_every_rule_is_remembered() {
        let empty = TextHighlightSettings { enabled: true, rules: Vec::new() };
        assert_eq!(empty.to_lines(), "text_highlights=true\nhighlight_rules=0\n");
        assert_eq!(parse(&empty.to_lines()).rules, Vec::new());
    }

    #[test]
    fn broken_rule_lines_are_skipped() {
        let settings = parse("highlight_rules=2\nhighlight_rule=nonsense\nhighlight_rule=on\tkey\t[0-9]+\tDigits\n");
        assert_eq!(settings.rules, [HighlightRule::new("Digits", "[0-9]+", RuleColor::Key)]);
    }

    #[test]
    fn highlights_are_on_unless_the_file_says_off() {
        assert!(TextHighlightSettings::default().enabled);
        let off = parse("text_highlights=false\n");
        assert!(!off.enabled);
        // Switching off keeps the rules for when they come back on.
        assert_eq!(off.rules, default_rules());
    }

    #[test]
    fn no_rules_are_active_while_highlights_are_off() {
        let mut settings = TextHighlightSettings::default();
        assert_eq!(settings.active_rules(), default_rules());
        settings.enabled = false;
        assert_eq!(settings.active_rules(), []);
    }
}
