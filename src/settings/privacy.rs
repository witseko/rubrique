//! Encrypted scratches and the app PIN.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use crate::crypto::{VaultInfo, from_hex, to_hex};

/// Choices for "Lock after", in minutes; 0 means never.
pub const LOCK_AFTER_CHOICES: [u32; 5] = [0, 1, 5, 15, 30];

/// What Rubrique keeps about scratch encryption and the PIN. No password,
/// PIN or key is stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivacySettings {
    /// New scratches are saved encrypted.
    pub encrypt_scratches: bool,
    /// Set once a scratch password was chosen.
    pub vault: Option<VaultInfo>,
    /// Argon2id hash of the app PIN; `None` means no PIN.
    pub pin_hash: Option<String>,
    /// Lock the window after this many idle minutes; 0 means never.
    pub lock_after_minutes: u32,
    /// Touch ID opens encrypted scratches (macOS): the key is in the keychain.
    pub touch_id: bool,
    /// Encrypted files moved in the app, so password changes include them.
    pub moved_scratches: Vec<PathBuf>,
}

impl PrivacySettings {
    /// Read one `key=value` line. Returns `false` for keys it does not know.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "encrypt_scratches" => self.encrypt_scratches = value == "true",
            "scratch_vault" => self.vault = VaultInfo::parse(value),
            "pin_hash" => self.pin_hash = (!value.is_empty()).then(|| value.to_string()),
            "lock_after_minutes" => self.lock_after_minutes = value.parse().unwrap_or(0),
            "touch_id" => self.touch_id = value == "true",
            "moved_scratch" => {
                self.moved_scratches.extend(from_hex(value).and_then(moved_scratch_path));
            }
            _ => return false,
        }
        true
    }

    /// Settings lines for [`PrivacySettings::parse_line`].
    pub fn to_lines(&self) -> String {
        let mut lines = format!(
            "encrypt_scratches={}\nscratch_vault={}\npin_hash={}\nlock_after_minutes={}\ntouch_id={}\n",
            self.encrypt_scratches,
            self.vault.as_ref().map(VaultInfo::to_text).unwrap_or_default(),
            self.pin_hash.as_deref().unwrap_or_default(),
            self.lock_after_minutes,
            self.touch_id,
        );
        for path in &self.moved_scratches {
            let _ = writeln!(lines, "moved_scratch={}", to_hex(path.as_os_str().as_bytes()));
        }
        lines
    }
}

/// Decode a local path without losing non-UTF-8 file names or line breaks.
fn moved_scratch_path(bytes: Vec<u8>) -> Option<PathBuf> {
    if bytes.contains(&0) {
        return None;
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Vault, pin_hash};

    fn parse(text: &str) -> PrivacySettings {
        let mut settings = PrivacySettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').unwrap();
            settings.parse_line(key, value);
        }
        settings
    }

    #[test]
    fn round_trip() {
        let settings = PrivacySettings {
            encrypt_scratches: true,
            vault: Some(Vault::create("pw").1),
            pin_hash: Some(pin_hash("1234")),
            lock_after_minutes: 5,
            touch_id: true,
            moved_scratches: vec![PathBuf::from(OsString::from_vec(b"/dev/secret\n\xff.md".to_vec()))],
        };
        assert_eq!(parse(&settings.to_lines()), settings);
        assert_eq!(parse(&PrivacySettings::default().to_lines()), PrivacySettings::default());
        assert!(!PrivacySettings::default().parse_line("theme", "dark"));
    }

    #[test]
    fn bad_values_are_ignored() {
        assert_eq!(parse("scratch_vault=nope\nlock_after_minutes=soon\n"), PrivacySettings::default());
    }
}
