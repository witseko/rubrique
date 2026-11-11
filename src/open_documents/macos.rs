//! The macOS "open documents" Apple Event handler.
//!
//! `AppKit` installs its own handler for this event while the app finishes
//! launching, so ours goes in afterwards: when the
//! `NSApplicationWillFinishLaunchingNotification` arrives, which is before
//! the event of a launch from Finder is delivered (Apple's documented place
//! for replacing a core Apple Event handler).
//!
//! The Objective-C calls are the `unsafe` code of this module.

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString};

/// Four-character codes from `AppleEvents.h` / `AEDataModel.h`.
/// `kCoreEventClass` ('aevt').
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
/// `kAEOpenDocuments` ('odoc').
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
/// `keyDirectObject` ('----'): the list of files in the event.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

/// The value of `AppKit`'s `NSApplicationWillFinishLaunchingNotification`
/// (a plain string, so `AppKit` bindings are not needed for it).
const WILL_FINISH_LAUNCHING: &str = "NSApplicationWillFinishLaunchingNotification";

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class has no
    // instance variables or Drop.
    #[unsafe(super(NSObject))]
    #[name = "RubriqueOpenDocumentsListener"]
    struct Listener;

    impl Listener {
        /// Called once, as the app finishes launching.
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            install_handler(self);
        }

        /// Called for each "open documents" event.
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            super::push(event_paths(event));
        }
    }
);

#[expect(unsafe_code, reason = "Objective-C calls from the objc2 bindings")]
pub fn listen() {
    // SAFETY: `new` on an NSObject subclass returns an owned, initialized object.
    let listener: Retained<Listener> = unsafe { msg_send![Listener::class(), new] };
    let name = NSString::from_str(WILL_FINISH_LAUNCHING);
    // SAFETY: the selector names a method of `Listener` taking one NSNotification.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &listener,
            sel!(willFinishLaunching:),
            Some(&name),
            None,
        );
    }
    // Neither the notification center nor the Apple Event manager keeps the
    // listener alive, and it must answer for the whole run: leak it.
    std::mem::forget(listener);
}

/// Make `listener` the handler of "open documents" events.
#[expect(unsafe_code, reason = "Objective-C call; the typed binding needs the CoreServices crate")]
fn install_handler(listener: &Listener) {
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let handler: &AnyObject = listener.as_ref();
    // SAFETY: the selector names a method of `Listener` taking the event and
    // the reply, as `setEventHandler:...` requires; the codes are `u32`
    // (`AEEventClass` and `AEEventID`).
    unsafe {
        let _: () = msg_send![
            &*manager,
            setEventHandler: handler,
            andSelector: sel!(handleOpenDocuments:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: OPEN_DOCUMENTS
        ];
    }
}

/// The file paths listed in an "open documents" event.
#[expect(unsafe_code, reason = "Objective-C call; the typed binding needs the CoreServices crate")]
fn event_paths(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    // SAFETY: `paramDescriptorForKeyword:` takes an `AEKeyword` (`u32`) and
    // returns an autoreleased descriptor or nil.
    let list: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
    let Some(list) = list else {
        return Vec::new();
    };
    // Apple Event lists count from 1.
    (1..=list.numberOfItems())
        .filter_map(|index| list.descriptorAtIndex(index))
        .filter_map(|item| item.fileURLValue())
        .filter_map(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
        .collect()
}
