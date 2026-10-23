//! Files on Android go through `ViewerActivity` (Java), which native code
//! cannot replace: files other apps open with the Viewer, the system file
//! picker and the "save as" dialog all answer to the activity.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::fd::{FromRawFd, OwnedFd};

use jni::objects::{JObject, JString};
use jni::{Env, jni_sig, jni_str};
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::Package;
use ketchup_viewer::app::{FileRequest, ViewerApp};
use ketchup_viewer::review::Written;

use crate::replace::{
    Interrupted, ReplaceFailure, drop_kept, kept_files, pending_targets, recover_interrupted,
    replace_keeping_original,
};

/// Request code ViewerActivity gives a file another app opened with the Viewer.
const OPENED_BY_ANOTHER_APP: i32 = 0;

/// What ViewerActivity.openFile answers for a document that is gone
/// (FileNotFoundException) and for one no longer granted (SecurityException);
/// -1 is any other failure.
const OPEN_NOT_FOUND: i32 = -2;
const OPEN_NOT_GRANTED: i32 = -3;

/// Bytes read first to tell a package from a comments file.
const SIGNATURE_BYTES: u64 = 64;

fn request_code(request: FileRequest) -> i32 {
    match request {
        FileRequest::OpenPackage => 1,
        FileRequest::OpenComments => 2,
        FileRequest::SaveComments => 3,
        FileRequest::SaveCommentedCopy => 4,
        FileRequest::ShareComments => 5,
        FileRequest::SaveKept => 6,
    }
}

/// Runs `call` with the JNI environment and the ViewerActivity object.
#[allow(unsafe_code)] // The VM and activity pointers come from android-activity and outlive this call.
pub(crate) fn with_activity<T>(
    android_app: &android_activity::AndroidApp,
    call: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>,
) -> jni::errors::Result<T> {
    let vm = unsafe { jni::JavaVM::from_raw(android_app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| {
        let activity = unsafe { JObject::from_raw(env, android_app.activity_as_ptr().cast()) };
        call(env, &activity)
    })
}

pub(crate) struct Files {
    android_app: android_activity::AndroidApp,
    /// Saved bytes waiting for the customer to choose their place, by request
    /// code; the comments count as saved only once they are written there.
    pending: HashMap<i32, (Vec<u8>, Option<Written>)>,
    /// File name of the opened package, for suggested save names.
    model_name: Option<String>,
    /// The earlier content set aside in private storage whose save dialog
    /// is open; removed once it is saved elsewhere.
    saving_kept: Option<std::path::PathBuf>,
    /// A picker or save dialog is open and its answer is still to come.
    pub(crate) waiting: bool,
}

impl Files {
    pub(crate) fn new(android_app: android_activity::AndroidApp) -> Self {
        Self {
            android_app,
            pending: HashMap::new(),
            model_name: None,
            saving_kept: None,
            waiting: false,
        }
    }

    /// Starts the picker or save dialog the customer asked for.
    pub(crate) fn request(
        &mut self,
        app: &mut ViewerApp,
        request: FileRequest,
    ) -> Result<(), Rejection> {
        let code = request_code(request);
        let name = self.model_name.as_deref();
        let save = match request {
            FileRequest::OpenPackage | FileRequest::OpenComments => None,
            // The share sheet never says whether anything was sent, so the
            // comments stay unsaved.
            FileRequest::ShareComments => {
                let mut bytes = Vec::new();
                let _unconfirmed = app.save_comments(&mut bytes)?;
                let name = ketchup_viewer::share::download_name(name, ".ketchup-comments");
                return self.share(&name, &bytes);
            }
            FileRequest::SaveComments => {
                let mut bytes = Vec::new();
                let written = app.save_comments(&mut bytes)?;
                let name = ketchup_viewer::share::download_name(name, ".ketchup-comments");
                Some((bytes, written, name))
            }
            FileRequest::SaveCommentedCopy => {
                let mut bytes = Vec::new();
                let written = app.save_commented_copy(&mut bytes)?;
                let name = ketchup_viewer::share::download_name(name, "-comments.ketchup-view");
                Some((bytes, written, name))
            }
            FileRequest::SaveKept => {
                let kept = self
                    .android_app
                    .internal_data_path()
                    .map(|private| kept_files(&private));
                let Some(path) = kept.and_then(|kept| kept.into_iter().next()) else {
                    app.offer_kept(false);
                    return Ok(());
                };
                let bytes = std::fs::read(&path).map_err(|error| {
                    Rejection::new("viewer.android.read", RejectionPhase::Io)
                        .target(path.display().to_string())
                        .caused_by(error)
                })?;
                let name = if Package::has_signature(&bytes) {
                    "earlier-content.ketchup-view"
                } else {
                    "earlier-content.ketchup-comments"
                };
                self.saving_kept = Some(path);
                Some((bytes, None, name.to_owned()))
            }
        };
        let started = match save {
            None => with_activity(&self.android_app, |env, activity| {
                env.call_method(
                    activity,
                    jni_str!("pickFile"),
                    jni_sig!("(I)V"),
                    &[code.into()],
                )
                .map(drop)
            }),
            Some((bytes, _, _)) if bytes.is_empty() => return Ok(()),
            Some((bytes, written, name)) => {
                self.pending.insert(code, (bytes, written));
                with_activity(&self.android_app, |env, activity| {
                    let name = env.new_string(name)?;
                    env.call_method(
                        activity,
                        jni_str!("createFile"),
                        jni_sig!("(ILjava/lang/String;)V"),
                        &[code.into(), (&name).into()],
                    )
                    .map(drop)
                })
            }
        };
        started.map_err(|error| jni_error("viewer.android.dialog", "", error))?;
        self.waiting = true;
        Ok(())
    }

    /// Opens or saves every file the activity has received since the last call.
    pub(crate) fn receive(&mut self, app: &mut ViewerApp) {
        loop {
            let next = with_activity(&self.android_app, |env, activity| {
                let result = env
                    .call_method(
                        activity,
                        jni_str!("takeResult"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                if result.is_null() {
                    return Ok(None);
                }
                env.cast_local::<JString>(result)?
                    .try_to_string(env)
                    .map(Some)
            });
            let (code, uri) = match next {
                Ok(Some(next)) => match next.split_once('\n') {
                    Some((code, uri)) => (code.parse().unwrap_or(-1), uri.to_owned()),
                    None => continue,
                },
                Ok(None) => return,
                Err(error) => {
                    app.report(jni_error("viewer.android.dialog", "", error));
                    return;
                }
            };
            self.waiting = false;
            if uri.is_empty() {
                // Cancelled: nothing opens, the pending bytes are dropped and
                // the comments stay unsaved.
                self.pending.remove(&code);
                continue;
            }
            if let Err(error) = self.handle(app, code, &uri) {
                app.report(error);
            }
        }
    }

    fn handle(&mut self, app: &mut ViewerApp, code: i32, uri: &str) -> Result<(), Rejection> {
        if let Some((bytes, written)) = self.pending.remove(&code) {
            self.write(uri, &bytes)?;
            app.comments_written(written);
            if code == request_code(FileRequest::SaveKept)
                && let Some(kept) = self.saving_kept.take()
            {
                // Saved elsewhere: the next one, if any, is offered.
                if let Err(error) = drop_kept(&kept) {
                    log::error!("Removing {} failed: {error}", kept.display());
                }
                self.offer_kept(app);
            }
            return Ok(());
        }
        // As a desktop double-click: a file another app hands over only warns
        // over unsaved comments, and handing it over again replaces them; the
        // picker's Open buttons ask before they start (review 2026-10-19 VW-16).
        if code == OPENED_BY_ANOTHER_APP && !app.may_discard_comments() {
            return Ok(());
        }
        let name = self.display_name(uri);
        let target = if name.is_empty() { uri } else { &name };
        let read_error = |error: std::io::Error| {
            Rejection::new("viewer.android.read", RejectionPhase::Io)
                .target(target.to_owned())
                .caused_by(error)
        };
        // The readers take only what the format allows, from the file itself:
        // a large package never has to fit the Java heap (VW-6).
        let mut file = self.open_file(uri, "r", target)?;
        let mut start = Vec::new();
        (&mut file)
            .take(SIGNATURE_BYTES)
            .read_to_end(&mut start)
            .map_err(read_error)?;
        let reader = BufReader::new(std::io::Cursor::new(start.as_slice()).chain(file));
        // Another app may hand over a file without a name, so its content decides.
        let comments = code == request_code(FileRequest::OpenComments)
            || (code == OPENED_BY_ANOTHER_APP && !Package::has_signature(&start));
        if comments {
            app.open_comments(reader)
        } else {
            app.open(reader)?;
            self.model_name = (!name.is_empty()).then_some(name);
            Ok(())
        }
    }

    fn display_name(&self, uri: &str) -> String {
        with_activity(&self.android_app, |env, activity| {
            let uri = env.new_string(uri)?;
            let name = env
                .call_method(
                    activity,
                    jni_str!("displayName"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[(&uri).into()],
                )?
                .l()?;
            env.cast_local::<JString>(name)?.try_to_string(env)
        })
        .unwrap_or_default()
    }

    /// The file behind `uri` opened in `mode` ("r" or "wt"), read and written
    /// here so its bytes never pass through the Java heap.
    fn open_file(&self, uri: &str, mode: &str, target: &str) -> Result<File, Rejection> {
        self.opened(uri, mode, target)
            .map_err(|(_, rejection)| *rejection)
    }

    /// [`Self::open_file`] as I/O, its error of the kind the activity named:
    /// a document that is gone or no longer granted is told from one out of
    /// reach for a while (review 2026-10-18 VW-14).
    fn open_io(&self, uri: &str, mode: &str) -> std::io::Result<File> {
        self.opened(uri, mode, uri)
            .map_err(|(kind, rejection)| std::io::Error::new(kind, *rejection))
    }

    fn opened(
        &self,
        uri: &str,
        mode: &str,
        target: &str,
    ) -> Result<File, (std::io::ErrorKind, Box<Rejection>)> {
        let code = if mode == "r" {
            "viewer.android.read"
        } else {
            "viewer.android.write"
        };
        let mut kind = std::io::ErrorKind::Other;
        let fd = with_activity(&self.android_app, |env, activity| {
            let uri = env.new_string(uri)?;
            let mode = env.new_string(mode)?;
            let fd = env
                .call_method(
                    activity,
                    jni_str!("openFile"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)I"),
                    &[(&uri).into(), (&mode).into()],
                )?
                .i()?;
            if fd < 0 {
                kind = match fd {
                    OPEN_NOT_FOUND => std::io::ErrorKind::NotFound,
                    OPEN_NOT_GRANTED => std::io::ErrorKind::PermissionDenied,
                    _ => std::io::ErrorKind::Other,
                };
                return last_error(env, activity).map(Err);
            }
            Ok(Ok(fd))
        });
        let fd = answered(code, target, fd).map_err(|rejection| (kind, Box::new(rejection)))?;
        // SAFETY: `openFile` detached `fd` from its ParcelFileDescriptor, so
        // nothing else owns or closes it.
        #[allow(unsafe_code)]
        let file = unsafe { File::from(OwnedFd::from_raw_fd(fd)) };
        Ok(file)
    }

    /// Opens the share sheet with `bytes` as file `name`; the sheet needs no answer.
    fn share(&self, name: &str, bytes: &[u8]) -> Result<(), Rejection> {
        if bytes.is_empty() {
            return Ok(());
        }
        let shared = with_activity(&self.android_app, |env, activity| {
            let file_name = env.new_string(name)?;
            let data = env.byte_array_from_slice(bytes)?;
            let shared = env
                .call_method(
                    activity,
                    jni_str!("share"),
                    jni_sig!("(Ljava/lang/String;[B)Z"),
                    &[(&file_name).into(), (&data).into()],
                )?
                .z()?;
            if shared {
                Ok(Ok(()))
            } else {
                last_error(env, activity).map(Err)
            }
        });
        answered("viewer.android.share", name, shared)
    }

    /// Replaces the file with `bytes`; a commented copy of a large package
    /// never has to fit the Java heap. A write that fails halfway puts the
    /// file's earlier bytes back (VW-11).
    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), Rejection> {
        let failed =
            || Rejection::new("viewer.android.write", RejectionPhase::Io).target(uri.to_owned());
        let private = self.android_app.internal_data_path().ok_or_else(|| {
            failed().reason("The app has no private storage to keep the old file aside.")
        })?;
        // A kept original that could not be put back is never overwritten
        // by the next save.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let backup = private.join(format!("replacing-{stamp}.bak"));
        let original = BufReader::new(self.open_file(uri, "r", uri)?);
        // A provider may hand out a pipe, which cannot be synced; the file
        // is unbuffered and closing it hands every byte over.
        let open_write = || self.open_io(uri, "wt");
        // Without the grant a save cut off now is still put back in this
        // process, just not at the next start.
        if let Err(error) = self.hold_grant(uri) {
            log::error!("{error}");
        }
        let replaced = replace_keeping_original(original, open_write, uri, &backup, bytes);
        self.release_grants_except(&pending_targets(&private));
        replaced.map_err(|failure| {
            match failure {
                ReplaceFailure::Unchanged(error) => failed()
                    .caused_by(error)
                    .reason("The old file could not be kept aside, so nothing was written.")
                    .fix_hint("Free some space on the phone and save again."),
                ReplaceFailure::Restored(error) => failed()
                    .caused_by(error)
                    .reason("Saving failed; the file holds what it held before.")
                    .fix_hint("Save again, or choose another place."),
                ReplaceFailure::Damaged {
                    write,
                    restore,
                    backup,
                } => failed()
                    .caused_by(write)
                    .reason(format!(
                        "Saving failed and the file could not be put back ({restore}); its earlier content is kept in {} and the next start tries again.",
                        backup.display()
                    ))
                    .fix_hint("Save again, or choose another place."),
            }
        })
    }

    /// Starts undoing the saves Android cut off by ending the process
    /// (VW-11) on a thread of its own: putting a document back copies all of
    /// its earlier bytes, which on the UI thread could hold the screen long
    /// enough for Android to close the app as not responding. Nothing may
    /// save until [`Self::finish_recovery`], or recovery could take that
    /// save's record for a cut-off one.
    pub(crate) fn start_recovery(&self) -> Recovery {
        let files = Files::new(self.android_app.clone());
        Recovery(std::thread::spawn(move || {
            let Some(private) = files.android_app.internal_data_path() else {
                return Vec::new();
            };
            let open = |mode: &'static str| {
                let files = &files;
                move |uri: &str| files.open_io(uri, mode)
            };
            let outcomes = recover_interrupted(&private, open("r"), open("wt"));
            // Also gives back the grants earlier versions kept for every file.
            files.release_grants_except(&pending_targets(&private));
            outcomes
        }))
    }

    /// Tells the customer what the recovery put back or could not.
    pub(crate) fn finish_recovery(&self, app: &mut ViewerApp, recovery: Recovery) {
        let outcomes = recovery.0.join().unwrap_or_else(|_| {
            app.report(
                Rejection::new("viewer.android.write", RejectionPhase::Io)
                    .reason("Putting back the saves Android cut off ended unexpectedly; the next start tries again.")
                    .fix_hint("Check the files you saved last."),
            );
            Vec::new()
        });
        for outcome in outcomes {
            let rejection = || Rejection::new("viewer.android.write", RejectionPhase::Io);
            let failed = |target: String| rejection().target(target);
            app.report(match outcome {
                Interrupted::Restored { target } => failed(target)
                    .reason("A save was cut off when Android ended the app; its earlier content was put back.")
                    .fix_hint("Save again."),
                Interrupted::Unchanged { target } => failed(target)
                    .reason("A save was cut off when Android ended the app before it changed the file, which holds what it held before.")
                    .fix_hint("Save again."),
                Interrupted::GivenUp {
                    target,
                    kept,
                    error,
                } => failed(target)
                    .caused_by(error)
                    .reason(format!(
                        "A save cut off when Android ended the app could not be finished at several starts; it is no longer tried, and the file's earlier content is kept in {}.",
                        kept.display()
                    ))
                    .fix_hint("Check the file, and save again if it should hold your comments; Save earlier content as… puts the earlier content where you choose."),
                Interrupted::Kept {
                    target,
                    backup,
                    error,
                } => failed(target)
                    .caused_by(error)
                    .reason(format!(
                        "A save was cut off when Android ended the app and the file could not be put back; its earlier content is kept in {} and the next start tries again.",
                        backup.display()
                    ))
                    .fix_hint("Save again, or choose another place."),
                Interrupted::Left { target, kept } => target
                    .map_or_else(rejection, failed)
                    .reason(format!(
                        "A save was cut off when Android ended the app, and the file now holds something that save did not write, or the save's record could not be read; the file was left as it is and its earlier content is kept in {}.",
                        kept.display()
                    ))
                    .fix_hint("Check the file, and save again if it should hold your comments; Save earlier content as… puts the earlier content where you choose."),
            });
        }
        self.offer_kept(app);
    }

    /// Offers saving the earlier contents set aside in private storage,
    /// which the customer cannot reach there (review 2026-10-17 VW-13).
    fn offer_kept(&self, app: &mut ViewerApp) {
        let kept = self
            .android_app
            .internal_data_path()
            .is_some_and(|private| !kept_files(&private).is_empty());
        app.offer_kept(kept);
    }

    /// Keeps the grant to write `uri` across starts while a save to it runs.
    fn hold_grant(&self, uri: &str) -> Result<(), Rejection> {
        let held = with_activity(&self.android_app, |env, activity| {
            let uri = env.new_string(uri)?;
            let held = env
                .call_method(
                    activity,
                    jni_str!("holdGrant"),
                    jni_sig!("(Ljava/lang/String;)Z"),
                    &[(&uri).into()],
                )?
                .z()?;
            if held {
                Ok(Ok(()))
            } else {
                last_error(env, activity).map(Err)
            }
        });
        answered("viewer.android.grant", uri, held)
    }

    /// Gives back every write grant kept across starts but those of the
    /// documents in `pending`: Android allows an app only so many.
    fn release_grants_except(&self, pending: &[String]) {
        let released = with_activity(&self.android_app, |env, activity| {
            let keep = env.new_string(pending.join("\n"))?;
            env.call_method(
                activity,
                jni_str!("releaseGrantsExcept"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[(&keep).into()],
            )
            .map(drop)
        });
        if let Err(error) = released {
            log::error!("Giving back file grants failed: {error}");
        }
    }
}

/// The cut-off saves being put back at the start.
pub(crate) struct Recovery(std::thread::JoinHandle<Vec<Interrupted>>);

impl Recovery {
    /// Whether [`Files::finish_recovery`] would not wait.
    pub(crate) fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

/// What the activity's `lastError` says went wrong in its previous call.
struct ActivityFailure(String);

fn last_error(env: &mut Env, activity: &JObject) -> jni::errors::Result<ActivityFailure> {
    let error = env
        .call_method(
            activity,
            jni_str!("lastError"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        )?
        .l()?;
    env.cast_local::<JString>(error)?
        .try_to_string(env)
        .map(ActivityFailure)
}

fn android_error(code: &'static str, target: &str, detail: String) -> Rejection {
    Rejection::new(code, RejectionPhase::Io)
        .target(target.to_owned())
        .caused_by_messages([detail])
}

/// A failed JNI call, keeping the JNI error itself.
fn jni_error(code: &'static str, target: &str, error: jni::errors::Error) -> Rejection {
    Rejection::new(code, RejectionPhase::Io)
        .target(target.to_owned())
        .caused_by(error)
}

/// The answer of an activity method that reports its own failure as text
/// (`lastError`), or the JNI error of calling it.
fn answered<T>(
    code: &'static str,
    target: &str,
    call: jni::errors::Result<Result<T, ActivityFailure>>,
) -> Result<T, Rejection> {
    match call {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(ActivityFailure(detail))) => Err(android_error(code, target, detail)),
        Err(error) => Err(jni_error(code, target, error)),
    }
}
