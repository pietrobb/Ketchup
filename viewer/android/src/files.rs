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

use crate::replace::{ReplaceFailure, replace_keeping_original};

/// Request code ViewerActivity gives a file another app opened with the Viewer.
const OPENED_BY_ANOTHER_APP: i32 = 0;

/// Bytes read first to tell a package from a comments file.
const SIGNATURE_BYTES: u64 = 64;

fn request_code(request: FileRequest) -> i32 {
    match request {
        FileRequest::OpenPackage => 1,
        FileRequest::OpenComments => 2,
        FileRequest::SaveComments => 3,
        FileRequest::SaveCommentedCopy => 4,
        FileRequest::ShareComments => 5,
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
    /// A picker or save dialog is open and its answer is still to come.
    pub(crate) waiting: bool,
}

impl Files {
    pub(crate) fn new(android_app: android_activity::AndroidApp) -> Self {
        Self {
            android_app,
            pending: HashMap::new(),
            model_name: None,
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
        started.map_err(|error| android_error("viewer.android.dialog", "", error.to_string()))?;
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
                    app.report(android_error(
                        "viewer.android.dialog",
                        "",
                        error.to_string(),
                    ));
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
            return Ok(());
        }
        let name = self.display_name(uri);
        let target = if name.is_empty() { uri } else { &name };
        let read_error =
            |error: std::io::Error| android_error("viewer.android.read", target, error.to_string());
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
        let code = if mode == "r" {
            "viewer.android.read"
        } else {
            "viewer.android.write"
        };
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
                return last_error(env, activity).map(Err);
            }
            Ok(Ok(fd))
        })
        .map_err(|error| error.to_string())
        .and_then(|fd| fd)
        .map_err(|detail| android_error(code, target, detail))?;
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
        with_activity(&self.android_app, |env, activity| {
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
        })
        .map_err(|error| error.to_string())
        .and_then(|shared| shared)
        .map_err(|detail| android_error("viewer.android.share", name, detail))
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
        let open_write = || {
            self.open_file(uri, "wt", uri)
                .map_err(std::io::Error::other)
        };
        replace_keeping_original(original, open_write, &backup, bytes).map_err(|failure| {
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
                        "Saving failed and the file could not be put back ({restore}); its earlier content is kept in {}.",
                        backup.display()
                    ))
                    .fix_hint("Save again, or choose another place."),
            }
        })
    }
}

fn last_error(env: &mut Env, activity: &JObject) -> jni::errors::Result<String> {
    let error = env
        .call_method(
            activity,
            jni_str!("lastError"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        )?
        .l()?;
    env.cast_local::<JString>(error)?.try_to_string(env)
}

fn android_error(code: &'static str, target: &str, detail: String) -> Rejection {
    Rejection::new(code, RejectionPhase::Io)
        .target(target.to_owned())
        .caused_by_messages([detail])
}
