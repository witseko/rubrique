//! Encrypted scratches and the app PIN.
//!
//! An encrypted file is `MAGIC`, the vault salt, a fresh nonce, then the
//! text sealed with XChaCha20-Poly1305. The key comes from the user's
//! password through Argon2id and lives only in memory, wiped on lock.
//! The PIN is stored as an Argon2id hash and only guards the window.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

/// First bytes of every encrypted file. Text files never start with them
/// by accident, and other programs show a clear hint instead of noise.
pub const MAGIC: &[u8] = b"RUBRIQUE-ENC1\n";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// Length of a vault key: what Touch ID keeps in the keychain.
pub const KEY_LEN: usize = 32;
/// Sealed with the key at setup, so a wrong password is caught at once.
const CHECK_TEXT: &[u8] = b"rubrique vault";

/// The file starts with [`MAGIC`].
pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// The file on disk is encrypted (reads only its first bytes).
pub fn file_is_encrypted(path: &Path) -> bool {
    let mut head = [0; MAGIC.len()];
    File::open(path).and_then(|mut file| file.read_exact(&mut head)).is_ok() && head == MAGIC
}

/// Why encrypted bytes could not be opened.
#[derive(Debug, PartialEq, Eq)]
pub enum DecryptError {
    /// Sealed with another password (or changed since).
    WrongKey,
    /// Not an encrypted file, or cut short.
    Corrupt,
}

/// What settings keep about the vault: the salt and the sealed check text.
/// Neither tells anything about the password on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultInfo {
    salt: [u8; SALT_LEN],
    check: Vec<u8>,
}

impl VaultInfo {
    /// `salt-hex:check-hex`, the form in the settings file.
    pub fn to_text(&self) -> String {
        format!("{}:{}", to_hex(&self.salt), to_hex(&self.check))
    }

    /// Read [`VaultInfo::to_text`]; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let (salt, check) = text.split_once(':')?;
        let salt = from_hex(salt)?.try_into().ok()?;
        let check = from_hex(check)?;
        (!check.is_empty()).then_some(VaultInfo { salt, check })
    }
}

/// The unlocked key for encrypted scratches.
pub struct Vault {
    key: Zeroizing<[u8; KEY_LEN]>,
    salt: [u8; SALT_LEN],
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Vault(unlocked)")
    }
}

impl Vault {
    /// A new vault for `password` with a random salt, and what to store.
    pub fn create(password: &str) -> (Vault, VaultInfo) {
        let salt = random_bytes::<SALT_LEN>();
        let vault = Vault { key: derive_key(password, &salt), salt };
        let check = vault.encrypt(CHECK_TEXT);
        (vault, VaultInfo { salt, check })
    }

    /// Open the vault with `password`; `None` when the password is wrong.
    pub fn unlock(password: &str, info: &VaultInfo) -> Option<Vault> {
        let vault = Vault { key: derive_key(password, &info.salt), salt: info.salt };
        (vault.decrypt(&info.check).ok()?.as_slice() == CHECK_TEXT).then_some(vault)
    }

    /// Open the vault from an encrypted file when the settings lost what
    /// [`Vault::create`] stored: the salt is in the file too.
    pub fn recover(password: &str, file: &[u8]) -> Option<(Vault, VaultInfo)> {
        let salt: [u8; SALT_LEN] = file.strip_prefix(MAGIC)?.get(..SALT_LEN)?.try_into().ok()?;
        let vault = Vault { key: derive_key(password, &salt), salt };
        vault.decrypt(file).ok()?;
        let check = vault.encrypt(CHECK_TEXT);
        Some((vault, VaultInfo { salt, check }))
    }

    /// A copy of the key, for a store that gives it back later (Touch ID).
    pub fn key_bytes(&self) -> Zeroizing<[u8; KEY_LEN]> {
        self.key.clone()
    }

    /// The vault from a key kept by [`Vault::key_bytes`]; `None` when it is
    /// not the key of `info` (the password changed since it was kept).
    pub fn from_key(key: Zeroizing<[u8; KEY_LEN]>, info: &VaultInfo) -> Option<Vault> {
        let vault = Vault { key, salt: info.salt };
        (vault.decrypt(&info.check).ok()?.as_slice() == CHECK_TEXT).then_some(vault)
    }

    /// Seal `plain` with a fresh nonce.
    pub fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        let nonce = random_bytes::<NONCE_LEN>();
        let sealed = self
            .cipher()
            .encrypt(&XNonce::from(nonce), plain)
            // Only fails for inputs far larger than any file we open.
            .unwrap_or_default();
        let mut bytes = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + sealed.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.salt);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&sealed);
        bytes
    }

    /// Open bytes made by [`Vault::encrypt`].
    ///
    /// # Errors
    ///
    /// [`DecryptError::WrongKey`] for another key or changed bytes,
    /// [`DecryptError::Corrupt`] for bytes that are not an encrypted file.
    pub fn decrypt(&self, bytes: &[u8]) -> Result<Vec<u8>, DecryptError> {
        let rest = bytes.strip_prefix(MAGIC).ok_or(DecryptError::Corrupt)?;
        if rest.len() < SALT_LEN + NONCE_LEN {
            return Err(DecryptError::Corrupt);
        }
        let (salt, rest) = rest.split_at(SALT_LEN);
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        if salt != self.salt {
            return Err(DecryptError::WrongKey);
        }
        let nonce = XNonce::try_from(nonce).map_err(|_| DecryptError::Corrupt)?;
        self.cipher().decrypt(&nonce, sealed).map_err(|_| DecryptError::WrongKey)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        // The key is always KEY_LEN bytes, the length the cipher takes.
        XChaCha20Poly1305::new_from_slice(self.key.as_ref()).expect("vault keys are 32 bytes")
    }
}

/// Argon2id settings: the library defaults (19 MiB, 2 passes) in the app,
/// tiny ones in tests so they stay fast.
fn argon2() -> Argon2<'static> {
    argon2_with(if cfg!(test) { Params::new(8, 1, 1, Some(KEY_LEN)) } else { production_params() })
}

/// The Argon2id settings every vault key and PIN hash in the app is made with.
/// Changing them would lock people out of their encrypted scratches.
fn production_params() -> Result<Params, argon2::Error> {
    Params::new(19 * 1024, 2, 1, Some(KEY_LEN))
}

fn argon2_with(params: Result<Params, argon2::Error>) -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params.unwrap_or_default())
}

fn derive_key(password: &str, salt: &[u8]) -> Zeroizing<[u8; KEY_LEN]> {
    derive_key_with(&argon2(), password, salt)
}

fn derive_key_with(argon2: &Argon2, password: &str, salt: &[u8]) -> Zeroizing<[u8; KEY_LEN]> {
    let mut key = Zeroizing::new([0; KEY_LEN]);
    // Fails only for salts or outputs of the wrong length, which are fixed here.
    let _ = argon2.hash_password_into(password.as_bytes(), salt, key.as_mut());
    key
}

/// The PIN as an Argon2id hash string for the settings file (with a fresh
/// random salt).
pub fn pin_hash(pin: &str) -> String {
    argon2().hash_password(pin.as_bytes()).map(|hash| hash.to_string()).unwrap_or_default()
}

/// `pin` matches the stored [`pin_hash`]. The hash string carries its own
/// Argon2 settings, so hashes made with other settings verify too.
pub fn verify_pin(pin: &str, hash: &str) -> bool {
    argon2().verify_password(pin.as_bytes(), hash).is_ok()
}

/// `N` bytes from the operating system's secure random source.
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    // The OS source only fails on a broken system, where nothing here can
    // be done safely; the previous `OsRng` stopped the same way.
    getrandom::fill(&mut bytes).expect("the system random source works");
    bytes
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|start| u8::from_str_radix(text.get(start..start + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn round_trip_and_fresh_nonces() {
        let (vault, _) = Vault::create("correct horse");
        let first = vault.encrypt("secret note".as_bytes());
        let second = vault.encrypt("secret note".as_bytes());
        assert!(is_encrypted(&first));
        assert_ne!(first, second);
        assert!(!first.windows(6).any(|window| window == b"secret"));
        assert_eq!(vault.decrypt(&first).unwrap(), b"secret note");
        assert_eq!(vault.decrypt(&vault.encrypt(b"")).unwrap(), b"");
    }

    #[test]
    fn unlock_checks_the_password() {
        let (_, info) = Vault::create("correct horse");
        let info = VaultInfo::parse(&info.to_text()).unwrap();
        assert!(Vault::unlock("wrong", &info).is_none());
        assert!(Vault::unlock("correct horse", &info).is_some());
    }

    #[test]
    fn other_keys_and_changed_bytes_fail() {
        let (vault, _) = Vault::create("a");
        let (other, _) = Vault::create("a");
        let mut bytes = vault.encrypt(b"text");
        assert_eq!(other.decrypt(&bytes), Err(DecryptError::WrongKey));
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(vault.decrypt(&bytes), Err(DecryptError::WrongKey));
        assert_eq!(vault.decrypt(b"plain text"), Err(DecryptError::Corrupt));
        assert_eq!(vault.decrypt(MAGIC), Err(DecryptError::Corrupt));
    }

    #[test]
    fn a_kept_key_opens_its_own_vault_only() {
        let (vault, info) = Vault::create("pw");
        let file = vault.encrypt(b"kept");
        let again = Vault::from_key(vault.key_bytes(), &info).unwrap();
        assert_eq!(again.decrypt(&file).unwrap(), b"kept");
        let (other, _) = Vault::create("pw");
        assert!(Vault::from_key(other.key_bytes(), &info).is_none());
    }

    #[test]
    fn recovers_from_a_file() {
        let (vault, info) = Vault::create("pw");
        let file = vault.encrypt(b"x");
        assert!(Vault::recover("wrong", &file).is_none());
        let (_, found) = Vault::recover("pw", &file).unwrap();
        assert!(Vault::unlock("pw", &found).is_some());
        assert_eq!(found.salt, info.salt);
        assert!(Vault::recover("pw", b"plain").is_none());
    }

    #[test]
    fn pin_hash_verifies() {
        let hash = pin_hash("1234");
        assert!(verify_pin("1234", &hash));
        assert!(!verify_pin("1235", &hash));
        assert!(!verify_pin("1234", "garbage"));
    }

    /// Made with argon2 0.5 and chacha20poly1305 0.10 (before the 0.7.0
    /// upgrade) at the production settings: the vaults, encrypted files and
    /// PINs people already have must keep opening after any upgrade.
    const OLD_KEY: &str = "59261a0a7884df92e0302f3ad5eb49933ade68b0077d1c82bc34c7bb9aa76036";
    const OLD_VAULT_INFO: &str = "0102030405060708090a0b0c0d0e0f10:52554252495155452d454e43310a0102030405060708090a0b0c0d0e0f109a3ee93b55e5e1b007bebcfd1211f8807f419bcefdccb55d1679107d5e36d73b68d42c8c566ff2c2e075c9148b1c4b89a9beee8ce680";
    const OLD_FILE: &str = "52554252495155452d454e43310a0102030405060708090a0b0c0d0e0f105d20bffbb3057d81679188ffab85f33879068ee70db813b18a180a02b82eca416775ebdbfd719226764ffba76f2f2fc6c2b5b2";
    const OLD_PIN: &str =
        "$argon2id$v=19$m=19456,t=2,p=1$LWykIH9glqRAvTBlztVZdw$bi+JTWS4uzByQC2sgqNSOqSfJrmQDVAOBUohmg2FI2U";

    #[test]
    fn keys_files_and_pins_from_older_versions_still_open() {
        let salt: [u8; SALT_LEN] = std::array::from_fn(|index| u8::try_from(index + 1).unwrap());
        let key = derive_key_with(&argon2_with(production_params()), "correct horse", &salt);
        assert_eq!(to_hex(key.as_ref()), OLD_KEY);
        let vault = Vault { key, salt };
        let info = VaultInfo::parse(OLD_VAULT_INFO).unwrap();
        assert_eq!(info.salt, salt);
        assert_eq!(vault.decrypt(&info.check).unwrap(), CHECK_TEXT);
        assert_eq!(vault.decrypt(&from_hex(OLD_FILE).unwrap()).unwrap(), b"secret note");
        assert!(verify_pin("1234", OLD_PIN));
        assert!(!verify_pin("1235", OLD_PIN));
    }

    #[test]
    fn vault_info_rejects_garbage() {
        assert_eq!(VaultInfo::parse(""), None);
        assert_eq!(VaultInfo::parse("zz:00"), None);
        assert_eq!(VaultInfo::parse("00:00"), None);
    }

    #[test]
    fn detects_encrypted_files() {
        let temp = TempFolder::new();
        let (vault, _) = Vault::create("a");
        let sealed = temp.path().join("s.md");
        std::fs::write(&sealed, vault.encrypt(b"x")).unwrap();
        let plain = temp.write("p.md", "# hi");
        assert!(file_is_encrypted(&sealed));
        assert!(!file_is_encrypted(&plain));
        assert!(!file_is_encrypted(&temp.path().join("missing.md")));
    }
}
