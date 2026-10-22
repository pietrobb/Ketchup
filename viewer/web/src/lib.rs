//! Web entry point of the Kečup Viewer: the same Viewer compiled to
//! WebAssembly. It opens the model a shared link names (`?model=`), lets the
//! customer open local files and downloads their comments; nothing is uploaded.
#![cfg(target_arch = "wasm32")]
// wasm-bindgen exports expand to unsafe ABI shims; this file has no unsafe blocks.
#![allow(unsafe_code)]

use eframe::egui;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_viewer::{
    app::{FileRequest, PendingOpen, ViewerApp},
    review::Written,
    share,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast as _, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};

/// Files the browser delivered since the last frame.
enum Incoming {
    Package {
        name: String,
        bytes: Vec<u8>,
    },
    /// The shared link's download, which a package opened since supersedes.
    Shared {
        pending: PendingOpen,
        address: String,
        result: Result<Vec<u8>, Rejection>,
    },
    Comments(Vec<u8>),
    /// The browser's save dialog wrote these comments to a file.
    Saved(Option<Written>),
    Failed(Rejection),
}

type Inbox = Rc<RefCell<Vec<Incoming>>>;

/// Start the Viewer on `canvas`. `allowed_origins` lists, separated by
/// spaces, the other origins whose models a shared link may open.
#[wasm_bindgen]
pub async fn start(
    canvas: web_sys::HtmlCanvasElement,
    allowed_origins: String,
) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("The Viewer needs a browser window.")?;
    // Rust has no randomness of its own here; comment IDs must differ
    // between two customers' browsers.
    let random = || (js_sys::Math::random() * 2f64.powi(32)) as u64;
    ketchup_viewer::review::seed_comment_ids(
        (random() << 32 | random()) ^ js_sys::Date::now().to_bits(),
    );
    let query = window.location().search()?;
    let origins: Vec<&str> = allowed_origins.split_whitespace().collect();
    let shared = share::shared_model(&query, &origins).and_then(|address| {
        address
            .map(|address| browser_origin_allowed(&window, &address, &origins).map(|()| address))
            .transpose()
    });
    let unsaved = Rc::new(Cell::new(false));
    guard_unsaved_comments(&window, unsaved.clone());
    let status_canvas = canvas.clone();
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                let inbox = Inbox::default();
                let mut app = ViewerApp::new(cc.wgpu_render_state.clone()).with_host_files();
                let mut model_name = None;
                match shared {
                    Ok(Some(address)) => {
                        let pending = app.begin_open();
                        fetch_model(pending, address.clone(), inbox.clone(), cc.egui_ctx.clone());
                        model_name = Some(address);
                    }
                    Ok(None) => {}
                    Err(error) => app.report(error),
                }
                let backend = cc
                    .wgpu_render_state
                    .as_ref()
                    .map_or("none".to_owned(), |state| {
                        format!("{:?}", state.adapter.get_info().backend).to_lowercase()
                    });
                Ok(Box::new(WebViewer {
                    app,
                    inbox,
                    model_name,
                    unsaved,
                    canvas: status_canvas,
                    backend,
                    status: String::new(),
                }))
            }),
        )
        .await
}

struct WebViewer {
    app: ViewerApp,
    inbox: Inbox,
    /// The shared address or local file name, for download names.
    model_name: Option<String>,
    /// Read by the page's leave warning.
    unsaved: Rc<Cell<bool>>,
    /// Carries the `data-ketchup-viewer` status for the hosting page and tests.
    canvas: web_sys::HtmlCanvasElement,
    /// The GPU path the browser gave: `browserwebgpu` or `gl` (WebGL2).
    backend: String,
    status: String,
}

impl eframe::App for WebViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let incoming = std::mem::take(&mut *self.inbox.borrow_mut());
        for item in incoming {
            let result = match item {
                Incoming::Package { name, bytes } => {
                    self.app.set_loading(false);
                    self.app
                        .open(bytes.as_slice())
                        .map(|()| self.model_name = Some(name))
                }
                Incoming::Shared {
                    pending,
                    address,
                    result: Ok(bytes),
                } => self
                    .app
                    .finish_open(pending, bytes.as_slice())
                    .map(|shown| {
                        if shown {
                            self.model_name = Some(address);
                        }
                    }),
                Incoming::Shared {
                    pending,
                    result: Err(error),
                    ..
                } => {
                    self.app.fail_open(pending, error);
                    Ok(())
                }
                Incoming::Comments(bytes) => self.app.open_comments(bytes.as_slice()),
                Incoming::Saved(written) => {
                    self.app.comments_written(written);
                    Ok(())
                }
                Incoming::Failed(error) => {
                    self.app.set_loading(false);
                    Err(error)
                }
            };
            if let Err(error) = result {
                self.app.report(error);
            }
        }
        self.app.show(ctx);
        for request in self.app.take_file_requests() {
            if let Err(error) = self.perform(request, ctx) {
                self.app.report(error);
            }
        }
        self.unsaved.set(self.app.has_unsaved_comments());
        self.publish_status();
    }
}

impl WebViewer {
    /// `ready components=N scenes=N comments=N backend=B`, `error <code>`
    /// or `empty backend=B` on the canvas, only when it changes.
    fn publish_status(&mut self) {
        let backend = &self.backend;
        let status = match (self.app.error(), self.app.opened_counts()) {
            (Some(error), _) => format!("error {}", error.code()),
            (None, Some([components, scenes, comments])) => format!(
                "ready components={components} scenes={scenes} comments={comments} backend={backend}"
            ),
            (None, None) => format!("empty backend={backend}"),
        };
        if status != self.status {
            let _ = self.canvas.set_attribute("data-ketchup-viewer", &status);
            self.status = status;
        }
    }

    fn perform(&mut self, request: FileRequest, ctx: &egui::Context) -> Result<(), Rejection> {
        let name = self.model_name.as_deref();
        let (bytes, written, file_name) = match request {
            FileRequest::OpenPackage => {
                return pick_file(".ketchup-view", false, self.inbox.clone(), ctx.clone());
            }
            FileRequest::OpenComments => {
                return pick_file(".ketchup-comments", true, self.inbox.clone(), ctx.clone());
            }
            // The browser keeps no earlier content, so it never offers this.
            FileRequest::SaveKept => return Ok(()),
            // The web Viewer offers no share button; a download is its way out.
            FileRequest::SaveComments | FileRequest::ShareComments => {
                let mut bytes = Vec::new();
                let written = self.app.save_comments(&mut bytes)?;
                (
                    bytes,
                    written,
                    share::download_name(name, ".ketchup-comments"),
                )
            }
            FileRequest::SaveCommentedCopy => {
                let mut bytes = Vec::new();
                let written = self.app.save_commented_copy(&mut bytes)?;
                (
                    bytes,
                    written,
                    share::download_name(name, "-comments.ketchup-view"),
                )
            }
        };
        if bytes.is_empty() {
            return Ok(());
        }
        // Saved only once the browser says the file was written; a cancelled
        // save dialog keeps the comments unsaved and the leave warning on.
        if let Some(picker) = save_file_picker() {
            save_through_picker(
                picker,
                file_name,
                bytes,
                written,
                self.inbox.clone(),
                ctx.clone(),
            );
            return Ok(());
        }
        // A plain download (Firefox, Safari) never says whether the customer
        // kept the file or cancelled its dialog, so the comments stay unsaved.
        download(&file_name, &bytes)
    }
}

/// The origin the browser itself resolves `address` to must be this page's or
/// one of `allowed`: the link check reads the text, this reads the URL the
/// browser will fetch, so no quirk of URL parsing can leave the host.
fn browser_origin_allowed(
    window: &web_sys::Window,
    address: &str,
    allowed: &[&str],
) -> Result<(), Rejection> {
    // The browser's own answer, when it has one, says why the address failed.
    let refused = |detail: JsValue| {
        let refusal = Rejection::new("viewer.link.origin", RejectionPhase::Request);
        browser_cause(refusal.target(address.to_owned()), detail)
            .reason("This Viewer does not open models from that address.")
            .fix_hint("Open the link published by the manufacturer, or ask them to allow the model's origin.")
    };
    let location = window.location();
    let page = location.origin().map_err(refused)?;
    let base = location.href().map_err(refused)?;
    let origin = web_sys::Url::new_with_base(address, &base)
        .map_err(refused)?
        .origin();
    let known = origin == page
        || allowed
            .iter()
            .any(|allowed| allowed.trim_end_matches('/').eq_ignore_ascii_case(&origin));
    if known {
        Ok(())
    } else {
        Err(refused(JsValue::NULL))
    }
}

/// Download the shared model; same-origin or CORS-enabled hosts only.
fn fetch_model(pending: PendingOpen, address: String, inbox: Inbox, ctx: egui::Context) {
    spawn_local(async move {
        let result = fetch_bytes(&address).await;
        inbox.borrow_mut().push(Incoming::Shared {
            pending,
            address,
            result,
        });
        ctx.request_repaint();
    });
}

async fn fetch_bytes(address: &str) -> Result<Vec<u8>, Rejection> {
    let failed = |detail: JsValue| {
        browser_error("viewer.link.fetch", address, detail)
            .reason("The shared model could not be downloaded.")
            .fix_hint(
                "Check the connection; the model must be on this site or on an origin that allows it (CORS).",
            )
    };
    let window = web_sys::window().ok_or_else(|| failed(JsValue::NULL))?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(address))
        .await
        .map_err(failed)?
        .dyn_into()
        .map_err(failed)?;
    if !response.ok() {
        return Err(failed(JsValue::NULL).reason(format!(
            "The server answered {} for the shared model.",
            response.status()
        )));
    }
    // Read as it arrives, so an answer past what a package can be stops the
    // download instead of filling the page's memory (VW-11).
    let length = response.headers().get("content-length").map_err(failed)?;
    let mut download = share::Download::new(address, length.as_deref(), share::file_limit(false))?;
    let Some(body) = response.body() else {
        return Ok(download.finish());
    };
    let reader: web_sys::ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|reader: js_sys::Object| failed(reader.into()))?;
    loop {
        let next = JsFuture::from(reader.read()).await.map_err(failed)?;
        let done = js_sys::Reflect::get(&next, &JsValue::from_str("done")).map_err(failed)?;
        if done.as_bool() != Some(false) {
            return Ok(download.finish());
        }
        let chunk: js_sys::Uint8Array = js_sys::Reflect::get(&next, &JsValue::from_str("value"))
            .map_err(failed)?
            .dyn_into()
            .map_err(failed)?;
        if let Err(too_large) = download.push(&chunk.to_vec()) {
            // Stops the transfer; the promise only reports that it stopped.
            drop(reader.cancel());
            return Err(too_large);
        }
    }
}

/// Let the customer choose a local file; it arrives in the inbox.
fn pick_file(
    accept: &str,
    comments: bool,
    inbox: Inbox,
    ctx: egui::Context,
) -> Result<(), Rejection> {
    let failed = |detail| {
        browser_error("viewer.web.pick", accept, detail)
            .reason("The browser did not offer a file to open.")
            .fix_hint("Allow file selection in the browser, or open the shared link again.")
    };
    let input: web_sys::HtmlInputElement = document()?
        .create_element("input")
        .map_err(failed)?
        .dyn_into()
        .map_err(|element: web_sys::Element| failed(element.into()))?;
    input.set_type("file");
    input.set_accept(accept);
    let picked = input.clone();
    let on_change = Closure::once_into_js(move || {
        let Some(file) = picked.files().and_then(|files| files.get(0)) else {
            return;
        };
        let name = file.name();
        if let Err(too_large) =
            share::check_file_size(&name, file.size(), share::file_limit(comments))
        {
            inbox.borrow_mut().push(Incoming::Failed(too_large));
            ctx.request_repaint();
            return;
        }
        spawn_local(async move {
            let item = match JsFuture::from(file.array_buffer()).await {
                Ok(buffer) => {
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    if comments {
                        Incoming::Comments(bytes)
                    } else {
                        Incoming::Package { name, bytes }
                    }
                }
                Err(detail) => Incoming::Failed(
                    browser_error("viewer.web.read", &name, detail)
                        .reason("The chosen file could not be read.")
                        .fix_hint("Choose the file again."),
                ),
            };
            inbox.borrow_mut().push(item);
            ctx.request_repaint();
        });
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
    Ok(())
}

/// The browser's save dialog that reports where the file went
/// (`showSaveFilePicker`: Chrome, Edge), if it has one.
fn save_file_picker() -> Option<js_sys::Function> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &"showSaveFilePicker".into())
        .ok()?
        .dyn_into()
        .ok()
}

/// Let the customer save `bytes` through the save dialog; once written, the
/// comments `written` describes are saved. Cancelling the dialog keeps them
/// unsaved without an error.
fn save_through_picker(
    picker: js_sys::Function,
    file_name: String,
    bytes: Vec<u8>,
    written: Option<Written>,
    inbox: Inbox,
    ctx: egui::Context,
) {
    spawn_local(async move {
        let item = match write_through_picker(&picker, &file_name, &bytes).await {
            Ok(()) => Incoming::Saved(written),
            Err(detail) if browser_error_name(&detail).as_deref() == Some("AbortError") => return,
            Err(detail) => Incoming::Failed(
                browser_error("viewer.web.save", &file_name, detail)
                    .reason("The browser did not save the file.")
                    .fix_hint("Save again and choose a folder the browser may write to."),
            ),
        };
        inbox.borrow_mut().push(item);
        ctx.request_repaint();
    });
}

async fn write_through_picker(
    picker: &js_sys::Function,
    file_name: &str,
    bytes: &[u8],
) -> Result<(), JsValue> {
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"suggestedName".into(), &file_name.into())?;
    let window: JsValue = web_sys::window().map_or(JsValue::UNDEFINED, Into::into);
    let handle = JsFuture::from(
        picker
            .call1(&window, &options)?
            .dyn_into::<js_sys::Promise>()?,
    )
    .await?;
    let writable = call_async(&handle, "createWritable", &js_sys::Array::new()).await?;
    let data = js_sys::Uint8Array::from(bytes);
    call_async(&writable, "write", &js_sys::Array::of1(&data)).await?;
    call_async(&writable, "close", &js_sys::Array::new())
        .await
        .map(drop)
}

/// Await `target.method(...arguments)`, a method returning a promise.
async fn call_async(
    target: &JsValue,
    method: &str,
    arguments: &js_sys::Array,
) -> Result<JsValue, JsValue> {
    let function: js_sys::Function = js_sys::Reflect::get(target, &method.into())?.dyn_into()?;
    JsFuture::from(
        function
            .apply(target, arguments)?
            .dyn_into::<js_sys::Promise>()?,
    )
    .await
}

/// The `name` of a browser error (`AbortError`, `NotAllowedError`, …).
fn browser_error_name(detail: &JsValue) -> Option<String> {
    js_sys::Reflect::get(detail, &"name".into())
        .ok()?
        .as_string()
}

/// Save `bytes` through the browser's download, under `file_name`.
fn download(file_name: &str, bytes: &[u8]) -> Result<(), Rejection> {
    let failed = |detail| {
        browser_error("viewer.web.download", file_name, detail)
            .reason("The browser did not start the download.")
            .fix_hint("Allow downloads for this site and try again.")
    };
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(failed)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(failed)?;
    let anchor: web_sys::HtmlAnchorElement = document()?
        .create_element("a")
        .map_err(failed)?
        .dyn_into()
        .map_err(|element: web_sys::Element| failed(element.into()))?;
    anchor.set_href(&url);
    anchor.set_download(file_name);
    anchor.click();
    // Release the file after the browser has taken it over.
    let release = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    if let Some(window) = web_sys::window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(release.unchecked_ref(), 60_000);
    }
    Ok(())
}

/// Warn before the page closes while comments are not saved.
fn guard_unsaved_comments(window: &web_sys::Window, unsaved: Rc<Cell<bool>>) {
    let on_leave = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        move |event: web_sys::BeforeUnloadEvent| {
            if unsaved.get() {
                event.prevent_default();
                event.set_return_value("");
            }
        },
    );
    window.set_onbeforeunload(Some(on_leave.as_ref().unchecked_ref()));
    // The page owns the handler for its whole life.
    on_leave.forget();
}

fn document() -> Result<web_sys::Document, Rejection> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| {
            Rejection::new("viewer.web.document", RejectionPhase::Io)
                .reason("The Viewer is not running in a web page.")
                .fix_hint("Open the Viewer page in a browser.")
        })
}

fn browser_error(code: &'static str, target: &str, detail: JsValue) -> Rejection {
    browser_cause(
        Rejection::new(code, RejectionPhase::Io).target(target.to_owned()),
        detail,
    )
}

/// `rejection` caused by the browser's error `detail`, when it has a message.
fn browser_cause(rejection: Rejection, detail: JsValue) -> Rejection {
    match detail.as_string().or_else(|| {
        detail
            .dyn_ref::<js_sys::Error>()
            .map(|error| String::from(error.message()))
    }) {
        Some(message) => rejection.caused_by_messages([message]),
        None => rejection,
    }
}
