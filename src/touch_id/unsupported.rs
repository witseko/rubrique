//! Touch ID on systems without it: never available.

use zeroize::Zeroizing;

use super::TouchIdError;
use crate::crypto::KEY_LEN;

const ONLY_MACOS: &str = "only on macOS";

pub fn available() -> bool {
    false
}

pub fn store_key(_key: &[u8; KEY_LEN]) -> Result<(), TouchIdError> {
    Err(TouchIdError::Other(ONLY_MACOS.to_string()))
}

pub fn read_key(_reason: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, TouchIdError> {
    Err(TouchIdError::Other(ONLY_MACOS.to_string()))
}

pub fn forget_key() -> Result<(), TouchIdError> {
    Err(TouchIdError::Other(ONLY_MACOS.to_string()))
}
