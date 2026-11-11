//! The macOS side of Touch ID: a generic password item in the data
//! protection keychain whose access control is "the fingerprints enrolled
//! now", and `LocalAuthentication` to know whether any are.
//!
//! The Objective-C calls are the `unsafe` code of this module, each behind
//! a safe function here.

use core_foundation::base::{CFType, TCFType};
use objc2::rc::Retained;
use objc2_foundation::NSString;
use objc2_local_authentication::{LAContext, LAPolicy};
use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
use security_framework::passwords::{AccessControlOptions, PasswordOptions, set_generic_password_options};
use zeroize::Zeroizing;

use super::TouchIdError;
use crate::crypto::KEY_LEN;

/// Keychain item identity. The service is the bundle identifier.
const SERVICE: &str = "io.github.witseko.rubrique";
const ACCOUNT: &str = "scratch-key";
/// What Keychain Access shows for the item.
const LABEL: &str = "Rubrique encrypted scratches";

// Result codes from `SecBase.h`.
/// `errSecUserCanceled`: the Touch ID sheet was closed.
const USER_CANCELED: i32 = -128;
/// `errSecItemNotFound`.
const ITEM_NOT_FOUND: i32 = -25300;
/// `errSecMissingEntitlement`: no authorized application identifier.
const MISSING_ENTITLEMENT: i32 = -34018;

pub fn available() -> bool {
    let context = new_context();
    can_use_biometrics(&context)
}

pub fn store_key(key: &[u8; KEY_LEN]) -> Result<(), TouchIdError> {
    // Updating an item behind Touch ID would ask for the fingerprint, so
    // the old item goes first and a fresh one is added.
    match forget_key() {
        Ok(()) | Err(TouchIdError::NotStored) => {}
        Err(error) => return Err(error),
    }
    let access = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
        AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
    )
    .map_err(classify)?;
    let mut options = PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
    options.use_protected_keychain();
    options.set_label(LABEL);
    options.set_access_control(access);
    set_generic_password_options(key, options).map_err(classify)
}

pub fn read_key(reason: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, TouchIdError> {
    let context = new_context();
    set_reason(&context, reason);
    let results = key_search().local_authentication_context(Some(as_cf_type(&context))).search().map_err(classify)?;
    // `context` stays alive until here: the search dictionary only borrowed it.
    drop(context);
    let bytes = results
        .into_iter()
        .find_map(|result| match result {
            SearchResult::Data(bytes) => Some(Zeroizing::new(bytes)),
            _ => None,
        })
        .ok_or(TouchIdError::NotStored)?;
    let key: [u8; KEY_LEN] =
        bytes.as_slice().try_into().map_err(|_| TouchIdError::Other("the kept key has the wrong size".to_string()))?;
    Ok(Zeroizing::new(key))
}

pub fn forget_key() -> Result<(), TouchIdError> {
    key_search().delete().map_err(classify)
}

/// The search that finds the kept key, in the data protection keychain only.
fn key_search() -> ItemSearchOptions {
    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .account(ACCOUNT)
        .load_data(true)
        .ignore_legacy_keychains();
    search
}

/// A Security framework error as what the user should hear.
fn classify(error: security_framework::base::Error) -> TouchIdError {
    classify_code(error.code(), || error.to_string())
}

fn classify_code(code: i32, message: impl FnOnce() -> String) -> TouchIdError {
    match code {
        USER_CANCELED => TouchIdError::Cancelled,
        ITEM_NOT_FOUND => TouchIdError::NotStored,
        MISSING_ENTITLEMENT => TouchIdError::NeedsSignedApp,
        _ => TouchIdError::Other(message()),
    }
}

#[expect(unsafe_code, reason = "Objective-C constructor from the objc2 bindings")]
fn new_context() -> Retained<LAContext> {
    // SAFETY: `LAContext` has a plain `init`; no arguments to get wrong.
    unsafe { LAContext::new() }
}

#[expect(unsafe_code, reason = "Objective-C call from the objc2 bindings")]
fn can_use_biometrics(context: &LAContext) -> bool {
    // SAFETY: `context` is a live LAContext and the policy a valid constant.
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) }.is_ok()
}

#[expect(unsafe_code, reason = "Objective-C call from the objc2 bindings")]
fn set_reason(context: &LAContext, reason: &str) {
    let reason = NSString::from_str(reason);
    // SAFETY: both objects are live; the context copies the string.
    unsafe { context.setLocalizedReason(&reason) }
}

/// The context as the Core Foundation object the keychain query takes
/// (toll-free bridging: every Objective-C object is a `CFTypeRef`).
#[expect(unsafe_code, reason = "toll-free bridge of an NSObject pointer to CFType")]
fn as_cf_type(context: &Retained<LAContext>) -> CFType {
    // SAFETY: the pointer comes from a live retained object, and the get
    // rule retains it once more, so the CFType owns its own reference and
    // releases it on drop.
    unsafe { CFType::wrap_under_get_rule(Retained::as_ptr(context).cast()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_become_the_right_errors() {
        let text = || "boom".to_string();
        assert_eq!(classify_code(USER_CANCELED, text), TouchIdError::Cancelled);
        assert_eq!(classify_code(ITEM_NOT_FOUND, text), TouchIdError::NotStored);
        assert_eq!(classify_code(MISSING_ENTITLEMENT, text), TouchIdError::NeedsSignedApp);
        assert_eq!(classify_code(-25293, text), TouchIdError::Other("boom".to_string()));
    }
}
