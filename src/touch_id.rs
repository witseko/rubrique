//! Touch ID for encrypted scratches (macOS only).
//!
//! The scratch key is kept in the macOS data protection keychain, readable
//! only after the fingerprint matches: the system shows the Touch ID sheet
//! itself while the key is read, and a changed set of fingerprints makes the
//! kept key unreadable. That keychain is open only to apps whose signature
//! carries an application identifier authorized by a provisioning profile,
//! so Touch ID works in the Rubrique app from a release and reports
//! [`TouchIdError::NeedsSignedApp`] in a `cargo run` build.
//!
//! On other systems [`available`] is `false` and nothing else is reached.

use std::fmt;

use zeroize::Zeroizing;

use crate::crypto::KEY_LEN;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
use unsupported as platform;

/// Why Touch ID did not give the key.
///
/// Only the macOS code makes the first three; the stub for other systems
/// has nothing to report but [`TouchIdError::Other`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TouchIdError {
    /// The user closed the Touch ID sheet.
    #[cfg_attr(not(target_os = "macos"), expect(dead_code, reason = "made by the macOS code only"))]
    Cancelled,
    /// No key is kept (never stored, or the fingerprints changed).
    #[cfg_attr(not(target_os = "macos"), expect(dead_code, reason = "made by the macOS code only"))]
    NotStored,
    /// This build may not use the data protection keychain.
    #[cfg_attr(not(target_os = "macos"), expect(dead_code, reason = "made by the macOS code only"))]
    NeedsSignedApp,
    /// Anything else, with the system's text.
    Other(String),
}

impl fmt::Display for TouchIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TouchIdError::Cancelled => formatter.write_str("Touch ID was cancelled"),
            TouchIdError::NotStored => formatter.write_str("No key is kept for Touch ID"),
            TouchIdError::NeedsSignedApp => {
                formatter.write_str("Touch ID needs the Rubrique app from a release (signed by Apple's rules)")
            }
            TouchIdError::Other(text) => write!(formatter, "Touch ID did not work: {text}"),
        }
    }
}

impl std::error::Error for TouchIdError {}

/// A fingerprint is enrolled on this computer. Checked once per run: it asks
/// the system each time.
pub fn available() -> bool {
    platform::available()
}

/// Keep `key` behind Touch ID, replacing any key kept before. No prompt.
///
/// # Errors
///
/// [`TouchIdError::NeedsSignedApp`] in a build without the entitlement, or
/// the system's error.
pub fn store_key(key: &[u8; KEY_LEN]) -> Result<(), TouchIdError> {
    platform::store_key(key)
}

/// Read the kept key. Blocks while the system shows the Touch ID sheet with
/// `reason` under the app name, so call it off the UI thread.
///
/// # Errors
///
/// [`TouchIdError::Cancelled`] when the sheet is closed,
/// [`TouchIdError::NotStored`] when no key is kept, or the system's error.
pub fn read_key(reason: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, TouchIdError> {
    platform::read_key(reason)
}

/// Remove the kept key. No prompt.
///
/// # Errors
///
/// [`TouchIdError::NotStored`] when there was none, or the system's error.
pub fn forget_key() -> Result<(), TouchIdError> {
    platform::forget_key()
}
