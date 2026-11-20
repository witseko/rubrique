//! Asks GitHub whether a newer Rubrique release exists.
//!
//! The check asks `curl` where GitHub's "latest release" page redirects to.
//! The target names the newest tag (`.../releases/tag/rubrique-v1.6.1`), so
//! the check needs no JSON, no HTTP library and no API rate limit. Nothing
//! is downloaded or installed: the user gets a link to the release page.

use std::fmt;
use std::process::{Command, Stdio};

use crate::ai::find_on_path;

/// GitHub's "latest release" page; it redirects to the newest tag.
const LATEST_RELEASE_URL: &str = "https://github.com/witseko/rubrique/releases/latest";
/// The start of a release tag's page; the version follows it.
const TAG_PAGE_PREFIX: &str = "https://github.com/witseko/rubrique/releases/tag/rubrique-v";
/// Give up on GitHub after this many seconds.
const TIMEOUT_SECONDS: &str = "15";
/// Automatic checks run at most once in this many seconds (a day).
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

/// A release version, `major.minor.patch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// Read `1.6.1`. Anything else (a pre-release suffix, a missing part,
    /// other text) gives `None`.
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.').map(|part| part.parse::<u32>().ok());
        let version = Version { major: parts.next()??, minor: parts.next()??, patch: parts.next()?? };
        parts.next().is_none().then_some(version)
    }

    /// The version of this build, from `Cargo.toml`.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("the Cargo.toml version is major.minor.patch")
    }

    /// The GitHub page of this release, with its notes and downloads.
    pub fn release_page(self) -> String {
        format!("{TAG_PAGE_PREFIX}{self}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckResult {
    /// This build is the newest release (or newer).
    UpToDate,
    /// A newer release exists.
    NewVersion(Version),
    /// The check did not get an answer; the text says why.
    Failed(String),
}

/// Compare the newest release on GitHub with `current`. Blocks for up to
/// [`TIMEOUT_SECONDS`], so run it on a background thread.
pub fn check_latest(current: Version) -> CheckResult {
    match fetch_latest_version() {
        Ok(latest) if latest > current => CheckResult::NewVersion(latest),
        Ok(_) => CheckResult::UpToDate,
        Err(reason) => CheckResult::Failed(reason),
    }
}

/// Whether an automatic check is due: never checked, a day has passed, or
/// the clock went back.
pub fn check_due(last_check: Option<u64>, now: u64) -> bool {
    match last_check {
        None => true,
        Some(last) => now < last || now - last >= CHECK_INTERVAL_SECONDS,
    }
}

/// The newest release's version, read from where GitHub redirects
/// [`LATEST_RELEASE_URL`].
fn fetch_latest_version() -> Result<Version, String> {
    let curl = find_on_path("curl").ok_or_else(|| "curl was not found".to_string())?;
    // A HEAD request without following the redirect: only the redirect
    // target is needed. `--proto =https` refuses anything but HTTPS.
    let output = Command::new(curl)
        .args(["--silent", "--show-error", "--head", "--proto", "=https", "--max-time", TIMEOUT_SECONDS])
        .args(["--output", "/dev/null", "--write-out", "%{redirect_url}", LATEST_RELEASE_URL])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run curl: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if message.is_empty() { "no answer from GitHub".to_string() } else { message });
    }
    let target = String::from_utf8_lossy(&output.stdout);
    version_from_tag_page(target.trim()).ok_or_else(|| "GitHub did not name a release".to_string())
}

/// The version in a release tag's page address, `None` for any other address.
fn version_from_tag_page(url: &str) -> Option<Version> {
    Version::parse(url.strip_prefix(TAG_PAGE_PREFIX)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn parses_release_versions_only() {
        assert_eq!(version("1.6.1").to_string(), "1.6.1");
        for bad in ["", "1.6", "1.6.1.2", "1.6.x", "1.7.0-beta", "v1.6.1", " 1.6.1"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn compares_by_number_not_text() {
        assert!(version("1.10.0") > version("1.9.9"));
        assert!(version("2.0.0") > version("1.99.99"));
        assert!(version("1.6.2") > version("1.6.1"));
        assert_eq!(version("1.6.1"), version("1.6.1"));
    }

    #[test]
    fn this_build_has_a_release_version() {
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn reads_the_version_from_the_tag_page() {
        let page = "https://github.com/witseko/rubrique/releases/tag/rubrique-v1.7.0";
        assert_eq!(version_from_tag_page(page), Some(version("1.7.0")));
        assert_eq!(version("1.7.0").release_page(), page);
        // No redirect (empty), another repository, or a tag that is no release.
        for other in [
            "",
            "https://github.com/someone/rubrique/releases/tag/rubrique-v1.7.0",
            "https://github.com/witseko/rubrique/releases/tag/rubrique-v1.7.0/../../evil",
            "https://github.com/witseko/rubrique/releases",
        ] {
            assert_eq!(version_from_tag_page(other), None, "{other}");
        }
    }

    #[test]
    fn checks_once_a_day() {
        let now = 1_800_000_000;
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - 60), now));
        assert!(check_due(Some(now - CHECK_INTERVAL_SECONDS), now));
        // A clock set back must not stop checks for good.
        assert!(check_due(Some(now + 60), now));
    }
}
