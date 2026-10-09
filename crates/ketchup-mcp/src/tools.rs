//! How each MCP tool call becomes a live bridge request.
//!
//! The window validates every request itself and explains rejections
//! (`reason`, `fix_hint`); this layer only routes, renames a few arguments and
//! keeps one connection to the attached window.
use crate::{
    bridge::{BridgeError, Connection, MAX_REQUEST_BYTES, Reply},
    discovery::{self, Window},
    docs, stage,
};
use serde_json::{Map, Value, json};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// The window owns every response deadline; the client waits this much longer
/// so a timeout answer from the window still arrives.
const DELIVERY_MARGIN: Duration = Duration::from_secs(5);
const DEFAULT_WAIT: Duration = crate::DEFAULT_RESPONSE_WAIT.saturating_add(DELIVERY_MARGIN);
const APPLY_PROGRAM_WAIT: Duration = crate::PROGRAM_RESPONSE_WAIT.saturating_add(DELIVERY_MARGIN);
const OPEN_WAIT: Duration = crate::OPEN_RESPONSE_WAIT.saturating_add(DELIVERY_MARGIN);
const DEFAULT_APPLY_AND_VERIFY_MS: u64 = 60_000;

/// How long the client waits for a verified edit of `timeout_ms`.
fn apply_and_verify_wait(timeout_ms: u64) -> Duration {
    crate::apply_and_verify_response_wait(timeout_ms) + DELIVERY_MARGIN
}
const IMAGE_PROTOCOL_VERSION: u32 = 4;
const WINDOW_START_TIMEOUT: Duration = Duration::from_secs(30);

/// A window started with a document opens it before it reports ready, so it
/// gets as long as `open` gives the same document in a running window.
fn window_start_wait(document: Option<&str>) -> Duration {
    if document.is_some() {
        OPEN_WAIT
    } else {
        WINDOW_START_TIMEOUT
    }
}

/// Fields of a model query that the bridge nests under `query`.
const QUERY_FIELDS: &[&str] = &[
    "kind",
    "limit",
    "search",
    "definition_id",
    "tag_id",
    "classification_dimension_id",
    "classification_category_id",
    "world_bounds_mm",
    "cursor",
    "compact",
];
/// Rejections after which this connection is no longer usable.
const CONNECTION_ENDING_CODES: &[&str] = &[
    "unauthorized",
    "invalid_request",
    "unsupported_version",
    "queue_unavailable",
];

#[derive(Debug)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn output(self) -> ToolOutput {
        let mut value = json!({"error": self.code, "message": self.message});
        if let Some(details) = self.details {
            value["details"] = details;
        }
        ToolOutput::error(value)
    }
}

pub struct ToolOutput {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolOutput {
    fn text(value: &Value) -> Self {
        Self {
            content: vec![json!({"type": "text", "text": value.to_string()})],
            is_error: false,
        }
    }

    fn error(value: Value) -> Self {
        Self {
            is_error: true,
            ..Self::text(&value)
        }
    }
}

struct Attached {
    connection: Connection,
    window: Window,
}

pub struct Tools {
    app_executable: Option<PathBuf>,
    discovery_root: Option<PathBuf>,
    attached: Option<Attached>,
}

impl Tools {
    pub fn new(app_executable: Option<PathBuf>, discovery_root: Option<PathBuf>) -> Self {
        Self {
            app_executable,
            discovery_root,
            attached: None,
        }
    }

    pub fn call(&mut self, name: &str, mut arguments: Map<String, Value>) -> ToolOutput {
        // Clients send absent optional arguments as null; the window expects them omitted.
        arguments.retain(|_, value| !value.is_null());
        self.dispatch(name, arguments)
            .unwrap_or_else(ToolError::output)
    }

    fn dispatch(
        &mut self,
        name: &str,
        mut args: Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        match name {
            "windows" => {
                no_more(&args)?;
                Ok(self.windows())
            }
            "connect" => {
                let instance_id = take_text(&mut args, "instance_id")?;
                no_more(&args)?;
                self.connect(instance_id.as_deref())
            }
            "open_window" => {
                let document = take_text(&mut args, "document_path")?;
                no_more(&args)?;
                self.open_window(document.as_deref())
            }
            "list_validators" => {
                no_more(&args)?;
                self.send("list_validators", args, DEFAULT_WAIT)
            }
            "program" => self.program(args),
            "inspect" => self.inspect(args),
            "model" => self.model(args),
            "edit" => {
                let action = take_action(&mut args, &["propose", "commit", "undo", "redo"])?;
                self.send(&action, args, DEFAULT_WAIT)
            }
            "file" => {
                let action =
                    take_action(&mut args, &["save", "save_as", "open", "export_drawings"])?;
                let wait = if action == "open" {
                    OPEN_WAIT
                } else {
                    DEFAULT_WAIT
                };
                self.send(&action, args, wait)
            }
            "view" => self.view(args),
            "batch" => self.batch(args),
            _ => Err(ToolError::new(
                "unknown_tool",
                format!("There is no tool {name}; list the tools again."),
            )),
        }
    }

    fn program(&mut self, mut args: Map<String, Value>) -> Result<ToolOutput, ToolError> {
        match take_action(
            &mut args,
            &[
                "read",
                "apply",
                "patch",
                "check",
                "set_params",
                "report",
                "docs",
                "validate",
            ],
        )?
        .as_str()
        {
            "read" => {
                let piece = ["lines", "search", "part"]
                    .iter()
                    .any(|key| args.contains_key(*key));
                let mode = take_text(&mut args, "mode")?.unwrap_or_else(|| "source".into());
                match mode.as_str() {
                    "source" if piece => self.send("program_piece", args, DEFAULT_WAIT),
                    "outline" if !piece => {
                        args.insert("outline".into(), json!(true));
                        self.send("program_piece", args, DEFAULT_WAIT)
                    }
                    _ if piece => Err(ToolError::new(
                        "invalid_arguments",
                        "lines, search and part read a piece of the source; give one of them without mode.",
                    )),
                    "full" => self.send("program", args, DEFAULT_WAIT),
                    "source" | "selection" => {
                        args.insert("selection_context".into(), json!(mode == "selection"));
                        self.send("program_context", args, DEFAULT_WAIT)
                    }
                    _ => Err(ToolError::new(
                        "invalid_arguments",
                        "Read mode must be source, selection, outline or full.",
                    )),
                }
            }
            "patch" => self.send("patch_program", args, APPLY_PROGRAM_WAIT),
            "set_params" => self.send("set_program_params", args, APPLY_PROGRAM_WAIT),
            "report" => {
                args.entry("limit").or_insert(json!(50));
                self.send("program_report", args, DEFAULT_WAIT)
            }
            "validate" => self.send("validate_program", args, APPLY_PROGRAM_WAIT),
            action @ ("apply" | "check") => {
                if let Some(path) = take_text(&mut args, "source_path")? {
                    if args.contains_key("source") {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            "Give either source or source_path, not both.",
                        ));
                    }
                    let source = read_program_source(Path::new(&path))?;
                    args.insert("source".into(), source.into());
                }
                let method = if action == "check" {
                    "check_program"
                } else {
                    "apply_program"
                };
                self.send(method, args, APPLY_PROGRAM_WAIT)
            }
            _ => {
                let name = take_text(&mut args, "name")?;
                let detail = take_text(&mut args, "detail")?.unwrap_or_else(|| "concise".into());
                no_more(&args)?;
                Ok(ToolOutput::text(&docs::library_detail(
                    name.as_deref(),
                    &detail,
                )?))
            }
        }
    }

    fn inspect(&mut self, mut args: Map<String, Value>) -> Result<ToolOutput, ToolError> {
        let action = take_action(
            &mut args,
            &[
                "status",
                "summary",
                "operations",
                "query",
                "detail",
                "workset_create",
                "workset_status",
                "measure",
            ],
        )?;
        match action.as_str() {
            "measure" => return self.send("measure_faces", args, APPLY_PROGRAM_WAIT),
            "operations" => rename(&mut args, "operation", "name"),
            "workset_status" => rename(&mut args, "workset_handle", "handle"),
            "query" | "workset_create" => {
                let query: Map<String, Value> = QUERY_FIELDS
                    .iter()
                    .filter_map(|field| args.remove_entry(*field))
                    .collect();
                args.insert("query".into(), Value::Object(query));
            }
            _ => {}
        }
        self.send(&action, args, DEFAULT_WAIT)
    }

    fn model(&mut self, mut args: Map<String, Value>) -> Result<ToolOutput, ToolError> {
        let action = take_action(&mut args, &["edit_context", "apply_and_verify"])?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_APPLY_AND_VERIFY_MS);
        let wait = if action == "apply_and_verify" {
            apply_and_verify_wait(timeout_ms)
        } else {
            DEFAULT_WAIT
        };
        self.send(&action, args, wait)
    }

    fn batch(&mut self, mut args: Map<String, Value>) -> Result<ToolOutput, ToolError> {
        let action = take_action(&mut args, &["start", "status", "step", "cancel"])?;
        if action != "start" {
            rename(&mut args, "job_handle", "handle");
        }
        self.send(&format!("batch_job_{action}"), args, DEFAULT_WAIT)
    }

    fn view(&mut self, mut args: Map<String, Value>) -> Result<ToolOutput, ToolError> {
        let action = take_action(
            &mut args,
            &[
                "selection",
                "view",
                "image",
                "saved_views",
                "save_view",
                "show_view",
                "tag_visibility",
                "section",
                "close_section",
            ],
        )?;
        if action == "tag_visibility" {
            args.retain(|key, value| {
                (key == "expected" || key == "name" || key == "visible") && is_set(value)
            });
            return self.send(&action, args, DEFAULT_WAIT);
        }
        if matches!(action.as_str(), "section" | "close_section") {
            // A cut changes only the viewport; closing it ignores any defaulted plane fields.
            // An offset of 0 is a real plane through the origin, so it stays with a normal.
            let has_normal = action == "section" && args.get("normal").is_some_and(is_set);
            args.retain(|key, value| match key.as_str() {
                "expected" => is_set(value),
                "normal" => has_normal,
                "offset_mm" => has_normal && value.is_number(),
                _ => false,
            });
            return self.send("section", args, DEFAULT_WAIT);
        }
        if matches!(action.as_str(), "saved_views" | "save_view" | "show_view") {
            // Clients often fill every schema field with its default; a saved-view
            // action reads only the view name and the stamp guard.
            args.retain(|key, value| {
                (key == "expected" || (key == "name" && action != "saved_views")) && is_set(value)
            });
            return self.send(&action, args, DEFAULT_WAIT);
        }
        if action != "image" {
            return self.send(&action, args, DEFAULT_WAIT);
        }
        args.retain(|_, value| is_set(value));
        let detail: Map<String, Value> = [
            ("detail_occurrence_id", "occurrence_id"),
            ("detail_kind", "kind"),
            ("detail_entity_id", "entity_id"),
        ]
        .into_iter()
        .filter_map(|(from, to)| args.remove(from).map(|value| (to.to_owned(), value)))
        .collect();
        // Clients often send every schema field with its default (0, ""); a
        // detail only means something for detail framing, so it is dropped
        // for the others instead of turning them into an invalid request.
        let framing = args
            .get("framing")
            .and_then(Value::as_str)
            .unwrap_or(if detail.is_empty() {
                "viewport"
            } else {
                "detail_selection"
            })
            .to_owned();
        if framing == "detail_selection" {
            args.insert("detail_target".into(), Value::Object(detail));
        }
        args.insert(
            "image_protocol_version".into(),
            IMAGE_PROTOCOL_VERSION.into(),
        );
        args.entry("capture_mode").or_insert("offscreen".into());
        args.entry("max_side_px").or_insert(512.into());
        args.insert("framing".into(), framing.into());
        match self.request("image", args, DEFAULT_WAIT)? {
            Reply::Done { stamp, mut result } => {
                let data = result
                    .as_object_mut()
                    .and_then(|result| result.remove("data"))
                    .unwrap_or_default();
                let summary = json!({
                    "stamp": stamp,
                    "width": result["width"],
                    "height": result["height"],
                    "framing": result["framing"],
                    "view": result["view"],
                    "selection": result["selection"],
                });
                Ok(ToolOutput {
                    content: vec![
                        json!({"type": "image", "data": data, "mimeType": "image/png"}),
                        json!({"type": "text", "text": summary.to_string()}),
                    ],
                    is_error: false,
                })
            }
            rejected => Ok(self.output(rejected)),
        }
    }

    fn windows(&self) -> ToolOutput {
        let connected = self
            .attached
            .as_ref()
            .map(|a| a.window.instance_id.as_str());
        let windows: Vec<Value> = self
            .list_windows()
            .iter()
            .map(|window| {
                let ours = connected == Some(window.instance_id.as_str());
                json!({
                    "instance_id": window.instance_id,
                    "document": window.document,
                    "connected": ours,
                    "used_by_another_client": window.in_use && !ours,
                })
            })
            .collect();
        ToolOutput::text(&json!({"windows": windows}))
    }

    fn connect(&mut self, instance_id: Option<&str>) -> Result<ToolOutput, ToolError> {
        self.attached = None;
        let window = match instance_id {
            Some(id) => self
                .list_windows()
                .into_iter()
                .find(|window| window.instance_id == id)
                .ok_or_else(|| {
                    ToolError::new(
                        "window_not_found",
                        format!("No open Kečup window has instance_id {id}; call windows."),
                    )
                })?,
            None => self.sole_window()?,
        };
        self.attach(window)?;
        self.send("status", Map::new(), DEFAULT_WAIT)
    }

    fn open_window(&mut self, document: Option<&str>) -> Result<ToolOutput, ToolError> {
        let executable = self.app_executable.clone().ok_or_else(|| {
            ToolError::new(
                "open_unavailable",
                "This server cannot start Kečup windows.",
            )
        })?;
        if let Some(path) = document
            && !(Path::new(path).is_absolute() && Path::new(path).is_file())
        {
            return Err(ToolError::new(
                "invalid_path",
                format!("document_path must be an absolute path of an existing file: {path}"),
            ));
        }
        let failed = |error| {
            ToolError::new(
                "open_failed",
                format!("Cannot start {}: {error}", executable.display()),
            )
        };
        let token = discovery::nonce().map_err(failed)?;
        let staged = stage::window_executable(&executable).map_err(failed)?;
        let mut child = spawn_window(&staged, document).map_err(failed)?;
        let launched = launch_connection(&mut child, token, document);
        let (window, connection) = match launched {
            Ok(launched) => launched,
            Err(error) => {
                // Only our failed child is stopped; never select or touch another window.
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed(error));
            }
        };
        self.attached = Some(Attached { window, connection });
        self.send("status", Map::new(), DEFAULT_WAIT)
    }

    fn list_windows(&self) -> Vec<Window> {
        self.discovery_root
            .as_deref()
            .map(discovery::list_windows)
            .unwrap_or_default()
    }

    fn sole_window(&self) -> Result<Window, ToolError> {
        let mut windows = self.list_windows();
        match windows.len() {
            0 => Err(ToolError::new(
                "no_window",
                "No Kečup window is open. Ask the user to open Kečup, or call open_window.",
            )),
            1 => Ok(windows.remove(0)),
            _ => {
                let listed: Vec<Value> = windows
                    .iter()
                    .map(|w| json!({"instance_id": w.instance_id, "document": w.document}))
                    .collect();
                Err(ToolError::new(
                    "choose_window",
                    "Several Kečup windows are open; call connect with one instance_id.",
                )
                .with_details(json!({"windows": listed})))
            }
        }
    }

    fn attach(&mut self, window: Window) -> Result<(), ToolError> {
        let failed = |error: std::io::Error| {
            let hint = if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
            ) {
                ""
            } else {
                "; it may have closed. Call windows."
            };
            ToolError::new(
                "attach_failed",
                format!("Cannot attach to the Kečup window ({error}){hint}"),
            )
        };
        let grant = discovery::attach(&window).map_err(failed)?;
        let connection = Connection::open(grant.address, grant.token).map_err(failed)?;
        self.attached = Some(Attached { connection, window });
        Ok(())
    }

    fn send(
        &mut self,
        method: &str,
        args: Map<String, Value>,
        wait: Duration,
    ) -> Result<ToolOutput, ToolError> {
        let reply = self.request(method, args, wait)?;
        Ok(self.output(reply))
    }

    fn output(&mut self, reply: Reply) -> ToolOutput {
        match reply {
            Reply::Done { stamp, result } => {
                ToolOutput::text(&json!({"stamp": stamp, "result": result}))
            }
            Reply::Rejected { code, result } => {
                if CONNECTION_ENDING_CODES.contains(&code.as_str()) {
                    self.attached = None;
                }
                let mut value = match result {
                    Value::Object(fields) => fields,
                    Value::Null => Map::new(),
                    other => Map::from_iter([("result".to_owned(), other)]),
                };
                value.insert("error".into(), code.into());
                ToolOutput::error(Value::Object(value))
            }
        }
    }

    fn request(
        &mut self,
        method: &str,
        mut args: Map<String, Value>,
        wait: Duration,
    ) -> Result<Reply, ToolError> {
        if self.attached.is_none() {
            let window = self.sole_window()?;
            self.attach(window)?;
        }
        let Some(attached) = self.attached.as_mut() else {
            return Err(ToolError::new(
                "no_window",
                "Not connected to a Kečup window.",
            ));
        };
        args.insert("method".into(), method.into());
        match attached.connection.request(Value::Object(args), wait) {
            Ok(reply) => Ok(reply),
            Err(BridgeError::TooLarge { bytes }) => Err(ToolError::new(
                "request_too_large",
                format!(
                    "The request has {bytes} bytes; the window accepts at most {MAX_REQUEST_BYTES}. \
                     Shorten the program or split the edit."
                ),
            )),
            Err(BridgeError::Transport(error)) => {
                self.attached = None;
                Err(ToolError::new(
                    "connection_lost",
                    format!(
                        "The connection to the Kečup window failed ({error}). A change may or may \
                         not have been applied: read inspect action=status before changing \
                         anything again. The next call reconnects."
                    ),
                ))
            }
        }
    }
}

fn launch_connection(
    child: &mut std::process::Child,
    token: String,
    document: Option<&str>,
) -> io::Result<(Window, Connection)> {
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing child stdin"))?;
    writeln!(input, "{}", json!({"version": 1, "token": token}))?;
    drop(input);
    let mut output = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing child stdout"))?;
    let wait = window_start_wait(document);
    let document = document.unwrap_or("Untitled").to_owned();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ketchup-child-readiness".into())
        .spawn(move || {
            let ready = discovery::launched_window(&mut output, document);
            let _ = send.send(ready);
            // The detached window may outlive this MCP connection. Drain, never retain logs.
            let _ = io::copy(&mut output, &mut io::sink());
        })?;
    let (window, address) = receive
        .recv_timeout(wait)
        .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))??;
    Ok((window, Connection::open(address, token)?))
}

/// Starts a new window detached from this server, so it outlives the AI client.
fn spawn_window(executable: &Path, document: Option<&str>) -> std::io::Result<std::process::Child> {
    let command = || {
        let mut command = Command::new(executable);
        command.arg("--supervisor-live-stdin").args(document);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let flags = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
        // A client's job object may forbid breaking away; then stay inside it.
        command()
            .creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .or_else(|_: std::io::Error| command().creation_flags(flags).spawn())
    }
    #[cfg(not(windows))]
    command().spawn()
}

pub(crate) fn read_program_source(path: &Path) -> Result<String, ToolError> {
    let failed = |error: io::Error| {
        ToolError::new(
            "invalid_path",
            format!(
                "Cannot read program file {}: {error}. Choose a regular UTF-8 file.",
                path.display()
            ),
        )
    };
    if !path.is_absolute() {
        return Err(ToolError::new(
            "invalid_path",
            "source_path must be an absolute regular-file path.",
        ));
    }
    let metadata = std::fs::symlink_metadata(path).map_err(failed)?;
    if !metadata.is_file() {
        return Err(ToolError::new(
            "invalid_path",
            "source_path must name a regular file, not a link, directory or device.",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A raced-in FIFO must not block open; a final symlink must not be followed.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the final reparse point itself and refuse it below, rather than following it.
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(failed)?;
    let metadata = file.metadata().map_err(failed)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ToolError::new(
            "invalid_path",
            "The opened source is not a regular file.",
        ));
    }
    if metadata.len() > MAX_REQUEST_BYTES as u64 {
        return Err(source_too_large());
    }
    read_bounded_source(file)
}

fn source_too_large() -> ToolError {
    ToolError::new(
        "request_too_large",
        format!(
            "Program source exceeds {MAX_REQUEST_BYTES} bytes; shorten the program. The escaped request envelope must also fit this limit."
        ),
    )
}

pub(crate) fn read_bounded_source(reader: impl Read) -> Result<String, ToolError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ToolError::new(
                "invalid_path",
                format!("Cannot read program source: {error}"),
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(source_too_large());
    }
    String::from_utf8(bytes).map_err(|error| {
        ToolError::new(
            "invalid_path",
            format!("Program source must be UTF-8: {error}"),
        )
    })
}

fn take_action(args: &mut Map<String, Value>, choices: &[&str]) -> Result<String, ToolError> {
    match args.remove("action") {
        Some(Value::String(action)) if choices.contains(&action.as_str()) => Ok(action),
        _ => Err(ToolError::new(
            "invalid_action",
            format!("action must be one of: {}", choices.join(", ")),
        )),
    }
}

fn take_text(args: &mut Map<String, Value>, key: &str) -> Result<Option<String>, ToolError> {
    match args.remove(key) {
        None => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(ToolError::new(
            "invalid_arguments",
            format!("{key} must be a string"),
        )),
    }
}

fn rename(args: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(value) = args.remove(from) {
        args.insert(to.into(), value);
    }
}

/// Whether a client gave a real value rather than a schema default (0, "").
fn is_set(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

fn no_more(args: &Map<String, Value>) -> Result<(), ToolError> {
    if args.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = args.keys().map(String::as_str).collect();
    Err(ToolError::new(
        "invalid_arguments",
        format!("Unexpected arguments: {}", names.join(", ")),
    ))
}

#[cfg(test)]
mod wait_tests {
    use super::*;

    /// The window answers every request by its own deadline, so the client must
    /// still be listening then; otherwise a slow but answered edit looks lost.
    #[test]
    fn the_client_waits_longer_than_the_window_for_every_request() {
        for timeout_ms in [1, 1_000, 9_999, 15_000, 60_000, 120_000] {
            assert!(
                apply_and_verify_wait(timeout_ms)
                    > crate::apply_and_verify_response_wait(timeout_ms),
                "apply_and_verify with timeout_ms {timeout_ms}"
            );
        }
        assert!(DEFAULT_WAIT > crate::DEFAULT_RESPONSE_WAIT);
        assert!(APPLY_PROGRAM_WAIT > crate::PROGRAM_RESPONSE_WAIT);
        assert!(OPEN_WAIT > crate::OPEN_RESPONSE_WAIT);
    }

    /// A window started with a document loads it before it reports ready.
    #[test]
    fn a_window_started_with_a_document_gets_as_long_as_opening_it() {
        assert!(window_start_wait(Some("C:/house.ketchup")) >= OPEN_WAIT);
        assert!(window_start_wait(None) >= WINDOW_START_TIMEOUT);
    }
}
