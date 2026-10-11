//! Files on Android go through `ViewerActivity` (Java), which native code
//! cannot replace: files other apps open with the Viewer, the system file
//! picker and the "save as" dialog all answer to the activity.

use std::collections::HashMap;

use jni::objects::{JByteArray, JObject, JString};
use jni::{Env, jni_sig, jni_str};
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::Package;
use ketchup_viewer::app::{FileRequest, ViewerApp};

/// Request code ViewerActivity gives a file another app opened with the Viewer.
const OPENED_BY_ANOTHER_APP: i32 = 0;

fn request_code(request: FileRequest) -> i32 {
    match request {
        FileRequest::OpenPackage => 1,
        FileRequest::OpenComments => 2,
        FileRequest::SaveComments => 3,
        FileRequest::SaveCommentedCopy => 4,
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
    /// Saved bytes waiting for the customer to choose their place, by request code.
    unsaved: HashMap<i32, Vec<u8>>,
    /// File name of the opened package, for suggested save names.
    model_name: Option<String>,
    /// A picker or save dialog is open and its answer is still to come.
    pub(crate) waiting: bool,
}

impl Files {
    pub(crate) fn new(android_app: android_activity::AndroidApp) -> Self {
        Self {
            android_app,
            unsaved: HashMap::new(),
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
            FileRequest::SaveComments => {
                let mut bytes = Vec::new();
                app.save_comments(&mut bytes)?;
                let name = ketchup_viewer::share::download_name(name, ".ketchup-comments");
                Some((bytes, name))
            }
            FileRequest::SaveCommentedCopy => {
                let mut bytes = Vec::new();
                app.save_commented_copy(&mut bytes)?;
                let name = ketchup_viewer::share::download_name(name, "-comments.ketchup-view");
                Some((bytes, name))
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
            Some((bytes, _)) if bytes.is_empty() => return Ok(()),
            Some((bytes, name)) => {
                self.unsaved.insert(code, bytes);
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
                // Cancelled: nothing opens, the unsaved bytes are dropped.
                self.unsaved.remove(&code);
                continue;
            }
            if let Err(error) = self.handle(app, code, &uri) {
                app.report(error);
            }
        }
    }

    fn handle(&mut self, app: &mut ViewerApp, code: i32, uri: &str) -> Result<(), Rejection> {
        if let Some(bytes) = self.unsaved.remove(&code) {
            return self.write(uri, &bytes);
        }
        let name = self.display_name(uri);
        let bytes = self.read(uri, &name)?;
        // Another app may hand over a file without a name, so its content decides.
        let comments = code == request_code(FileRequest::OpenComments)
            || (code == OPENED_BY_ANOTHER_APP && !Package::has_signature(&bytes));
        if comments {
            app.open_comments(bytes.as_slice())
        } else {
            app.open(bytes.as_slice())?;
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

    fn read(&self, uri: &str, name: &str) -> Result<Vec<u8>, Rejection> {
        let target = if name.is_empty() { uri } else { name };
        with_activity(&self.android_app, |env, activity| {
            let uri = env.new_string(uri)?;
            let bytes = env
                .call_method(
                    activity,
                    jni_str!("read"),
                    jni_sig!("(Ljava/lang/String;)[B"),
                    &[(&uri).into()],
                )?
                .l()?;
            if bytes.is_null() {
                return Ok(Err(last_error(env, activity)?));
            }
            let bytes = env.cast_local::<JByteArray>(bytes)?;
            env.convert_byte_array(&bytes).map(Ok)
        })
        .map_err(|error| error.to_string())
        .and_then(|read| read)
        .map_err(|detail| android_error("viewer.android.read", target, detail))
    }

    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), Rejection> {
        with_activity(&self.android_app, |env, activity| {
            let target = env.new_string(uri)?;
            let data = env.byte_array_from_slice(bytes)?;
            let written = env
                .call_method(
                    activity,
                    jni_str!("write"),
                    jni_sig!("(Ljava/lang/String;[B)Z"),
                    &[(&target).into(), (&data).into()],
                )?
                .z()?;
            if written {
                Ok(Ok(()))
            } else {
                last_error(env, activity).map(Err)
            }
        })
        .map_err(|error| error.to_string())
        .and_then(|written| written)
        .map_err(|detail| android_error("viewer.android.write", uri, detail))
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
