//! Finder, "Open with" and drops on the Dock icon send the files as an
//! "open documents" Apple Event, at launch and to a running Viewer. winit
//! keeps the application delegate to itself, so the Viewer registers its own
//! Apple Event handler once AppKit has installed its defaults.

// The whole module is the Objective-C bridge: defining the handler class and
// registering its selectors with AppKit is unsafe by nature.
#![allow(unsafe_code)]

use ketchup_viewer::desktop::OpenRequests;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter,
    NSObjectProtocol, NSString,
};
use std::path::PathBuf;

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KetchupViewerOpenEvents"]
    #[ivars = OpenRequests]
    pub(crate) struct OpenEvents;

    unsafe impl NSObjectProtocol for OpenEvents {}

    impl OpenEvents {
        /// AppKit installs its default handlers before this notification and
        /// delivers the launch's open event after it, so registering here
        /// replaces the default and still catches the first file.
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            let handler: &AnyObject = self.as_ref();
            // SAFETY: the selector below is defined on this class with the
            // (event, reply) signature NSAppleEventManager calls.
            unsafe {
                NSAppleEventManager::sharedAppleEventManager()
                    .setEventHandler_andSelector_forEventClass_andEventID(
                        handler,
                        sel!(openDocuments:withReplyEvent:),
                        CORE_EVENT_CLASS,
                        OPEN_DOCUMENTS,
                    );
            }
        }

        #[unsafe(method(openDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            for path in document_paths(event) {
                self.ivars().push(path);
            }
        }
    }
);

/// The file paths in an open-documents event: a list of files, or one file.
fn document_paths(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let Some(direct) = event.paramDescriptorForKeyword(DIRECT_OBJECT) else {
        return vec![];
    };
    let items = direct.numberOfItems();
    let descriptors: Vec<_> = if items > 0 {
        (1..=items)
            .filter_map(|i| direct.descriptorAtIndex(i))
            .collect()
    } else {
        vec![direct]
    };
    descriptors
        .iter()
        .filter_map(|descriptor| descriptor.fileURLValue()?.to_file_path())
        .collect()
}

/// Start listening for open events; keep the returned handler alive for the
/// life of the app. `None` off the main thread, where AppKit cannot run.
pub(crate) fn install(requests: OpenRequests) -> Option<Retained<OpenEvents>> {
    let mtm = MainThreadMarker::new()?;
    let this = mtm.alloc::<OpenEvents>().set_ivars(requests);
    // SAFETY: NSObject's designated initializer on a freshly allocated instance.
    let handler: Retained<OpenEvents> = unsafe { msg_send![super(this), init] };
    let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
    // SAFETY: the selector is defined on the handler with an NSNotification
    // argument; the handler outlives the run loop because `main` keeps it.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            handler.as_ref(),
            sel!(willFinishLaunching:),
            Some(&name),
            None,
        );
    }
    Some(handler)
}
