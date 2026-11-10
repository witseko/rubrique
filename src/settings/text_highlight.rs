//! The text highlight rules in the settings file: a `highlight_rules` line,
//! then one `highlight_rule` line per rule, in order.

use std::fmt::Write as _;

use crate::highlight::text_rules::{HighlightRule, default_rules};

/// Key of the line written before the rules. A file without it was saved
/// before rules existed, so it gets the default rules; a file with it and no
/// rule lines means the user deleted them all.
const RULES_KEY: &str = "highlight_rules";
/// Key of each rule's line.
const RULE_KEY: &str = "highlight_rule";

/// The user's rules for coloring plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextHighlightSettings {
    /// Tried top to bottom; the first match colors the text.
    pub rules: Vec<HighlightRule>,
}

impl Default for TextHighlightSettings {
    fn default() -> Self {
        TextHighlightSettings { rules: default_rules() }
    }
}

impl TextHighlightSettings {
    /// Apply one `key=value` line. Returns false when the key is not a
    /// highlight key. A broken rule line is skipped.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            // The saved list replaces the defaults.
            RULES_KEY => self.rules.clear(),
            RULE_KEY => self.rules.extend(HighlightRule::from_value(value)),
            _ => return false,
        }
        true
    }

    /// The `key=value` lines that [`TextHighlightSettings::parse_line`] reads.
    pub fn to_lines(&self) -> String {
        let mut lines = format!("{RULES_KEY}={}\n", self.rules.len());
        for rule in &self.rules {
            let _ = writeln!(lines, "{RULE_KEY}={}", rule.to_value());
        }
        lines
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
        let mut settings = TextHighlightSettings::default();
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
        let empty = TextHighlightSettings { rules: Vec::new() };
        assert_eq!(empty.to_lines(), "highlight_rules=0\n");
        assert_eq!(parse(&empty.to_lines()).rules, Vec::new());
    }

    #[test]
    fn broken_rule_lines_are_skipped() {
        let settings = parse("highlight_rules=2\nhighlight_rule=nonsense\nhighlight_rule=on\tkey\t[0-9]+\tDigits\n");
        assert_eq!(settings.rules, [HighlightRule::new("Digits", "[0-9]+", RuleColor::Key)]);
    }
}
