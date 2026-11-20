//! Whether Rubrique checks for new versions, and what it remembers about it.

use crate::update::Version;

/// The user's answer to "Check for new versions once a day?".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoCheck {
    /// Not answered yet: Rubrique asks, and checks nothing until then.
    #[default]
    NotAsked,
    On,
    Off,
}

impl AutoCheck {
    fn id(self) -> &'static str {
        match self {
            AutoCheck::NotAsked => "ask",
            AutoCheck::On => "on",
            AutoCheck::Off => "off",
        }
    }

    fn parse(value: &str) -> AutoCheck {
        match value {
            "on" => AutoCheck::On,
            "off" => AutoCheck::Off,
            _ => AutoCheck::NotAsked,
        }
    }
}

/// Update check settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateSettings {
    pub auto_check: AutoCheck,
    /// When the last automatic check started, in seconds since 1970.
    pub last_check: Option<u64>,
    /// The version the user chose to skip: no notice for it.
    pub skipped_version: Option<Version>,
}

impl UpdateSettings {
    /// Read one `key=value` line. Returns `false` for keys it does not know.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "update_check" => self.auto_check = AutoCheck::parse(value),
            "update_last_check" => self.last_check = value.parse().ok(),
            "update_skipped_version" => self.skipped_version = Version::parse(value),
            _ => return false,
        }
        true
    }

    /// Settings lines for [`UpdateSettings::parse_line`].
    pub fn to_lines(&self) -> String {
        format!(
            "update_check={}\nupdate_last_check={}\nupdate_skipped_version={}\n",
            self.auto_check.id(),
            self.last_check.map(|seconds| seconds.to_string()).unwrap_or_default(),
            self.skipped_version.map(|version| version.to_string()).unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> UpdateSettings {
        let mut settings = UpdateSettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').unwrap();
            settings.parse_line(key, value);
        }
        settings
    }

    #[test]
    fn round_trip() {
        let settings = UpdateSettings {
            auto_check: AutoCheck::On,
            last_check: Some(1_800_000_000),
            skipped_version: Version::parse("1.7.0"),
        };
        assert_eq!(parse(&settings.to_lines()), settings);
        let off = UpdateSettings { auto_check: AutoCheck::Off, ..UpdateSettings::default() };
        assert_eq!(parse(&off.to_lines()), off);
        assert_eq!(parse(&UpdateSettings::default().to_lines()), UpdateSettings::default());
        assert!(!UpdateSettings::default().parse_line("theme", "dark"));
    }

    #[test]
    fn bad_values_mean_not_asked_and_not_set() {
        let settings = parse("update_check=yes\nupdate_last_check=soon\nupdate_skipped_version=next\n");
        assert_eq!(settings, UpdateSettings::default());
    }
}
