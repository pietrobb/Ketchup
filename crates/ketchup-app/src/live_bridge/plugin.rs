//! Runs a local plugin process against the window's document. The process runs
//! off the UI thread on a snapshot; the typed program it sends is published by
//! the `apply_and_verify` job, the window's one path for typed programs, only
//! while the document is still at the revision the plugin read. It starts only
//! after the user allowed it in the window (`plugin_consent`).
use super::plugin_consent::PluginConsent;
use super::plugin_files::HeldFiles;
use super::*;
use ketchup_assistant::extension::{PluginCapability, PluginGrant, PluginLimits};
use ketchup_scheduler::plugin::{
    PluginCommand, PluginHostError, PluginRunResult, run_plugin_command,
};
use std::ffi::OsString;

pub(super) const fn default_plugin_timeout_ms() -> u64 {
    30_000
}

/// Either a plugin folder (`plugin` with `inputs`, its `plugin.json` decides
/// everything else) or a command line with its grant (`executable` …).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunPluginRequest {
    #[serde(default)]
    pub expected: Option<Stamp>,
    /// Absolute path of a plugin folder or of its `plugin.json`.
    #[serde(default)]
    pub plugin: Option<String>,
    /// Values of the inputs the manifest declares, handed over by protocol.
    #[serde(default)]
    pub inputs: Option<serde_json::Map<String, Value>>,
    /// Absolute path of the plugin executable.
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Must match the principal the plugin declares in its HELLO.
    #[serde(default)]
    pub principal_id: Option<u64>,
    /// Granted capabilities by protocol name, e.g. `edit.cad-program.v1`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Granted typed operations by wire name, e.g. `set_dimension`.
    #[serde(default)]
    pub allowed_operations: Vec<String>,
    /// How long the plugin process may run; publishing has its own deadline.
    #[serde(default = "default_plugin_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub validators: Vec<String>,
    #[serde(default)]
    pub strict: bool,
}

pub(super) struct PluginJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    worker_cancelled: Arc<AtomicBool>,
    before: Stamp,
    validators: Vec<String>,
    strict: bool,
    receiver: mpsc::Receiver<Result<PluginRunResult, RunFailure>>,
}

/// Why a plugin run ended without its result.
enum RunFailure {
    /// The allowed files changed between the consent and the start; nothing ran.
    Changed(std::io::Error),
    Host(PluginHostError),
}

/// What preparing a run ends with: the launch, or the refusal with its cause.
type Prepared = Result<PluginLaunch, (&'static str, Option<Value>)>;

/// A plugin folder or command line being read, staged and hashed off the UI
/// thread; the user is asked once it is ready.
pub(super) struct PluginPrepareJob {
    answer: PluginAnswer,
    request: RunPluginRequest,
    receiver: mpsc::Receiver<Prepared>,
}

/// Holds the `launch` of the named plugin folder until the test sends or
/// drops its sender.
#[cfg(test)]
pub(super) static LAUNCH_GATE: std::sync::Mutex<Option<(String, mpsc::Receiver<()>)>> =
    std::sync::Mutex::new(None);

/// Where a plugin run answers: the request id, its reply channel and whether
/// the caller has gone.
pub(super) struct PluginAnswer {
    pub id: u64,
    pub reply: mpsc::SyncSender<Response>,
    pub cancelled: Arc<AtomicBool>,
}

/// What starts, what it may do, and what the user allows.
struct PluginLaunch {
    command: PluginCommand,
    grant: PluginGrant,
    consent: PluginConsent,
    /// The files the consent hashed, held until the process has ended.
    held: HeldFiles,
}

fn invalid_params(reason: &str) -> &'static str {
    failure(
        "invalid_params",
        reason,
        json!({"canonical_mutation": false}),
    )
}

/// Reads the plugin folder or the command line; nothing starts yet.
fn launch(request: &RunPluginRequest) -> Result<PluginLaunch, &'static str> {
    #[cfg(test)]
    if let Some((_, gate)) = LAUNCH_GATE
        .lock()
        .ok()
        .and_then(|mut gate| gate.take_if(|(plugin, _)| request.plugin.as_ref() == Some(plugin)))
    {
        let _ = gate.recv_timeout(Duration::from_secs(5));
    }
    match (&request.plugin, &request.executable) {
        (Some(plugin), None) => {
            if !request.arguments.is_empty()
                || request.principal_id.is_some()
                || !request.capabilities.is_empty()
                || !request.allowed_operations.is_empty()
            {
                return Err(invalid_params(
                    "plugin.json decides the command line, principal and permissions; send only plugin and inputs",
                ));
            }
            let folder = plugin_manifest::load(plugin, request.inputs.as_ref())?;
            Ok(PluginLaunch {
                grant: grant(
                    folder.principal_id,
                    &folder.capabilities,
                    &folder.allowed_operations,
                )?,
                command: folder.command,
                consent: folder.consent,
                held: folder.held,
            })
        }
        (None, Some(executable)) => {
            if request.inputs.is_some() {
                return Err(invalid_params(
                    "inputs need a plugin folder whose plugin.json declares them",
                ));
            }
            let Some(principal_id) = request.principal_id else {
                return Err(invalid_params("a command line needs its principal_id"));
            };
            let grant = grant(
                principal_id,
                &request.capabilities,
                &request.allowed_operations,
            )?;
            let mut held = HeldFiles::default();
            let consent = PluginConsent::of_command(
                executable,
                &request.arguments,
                principal_id,
                &request.capabilities,
                &request.allowed_operations,
                &mut held,
            )?;
            Ok(PluginLaunch {
                command: PluginCommand {
                    executable: PathBuf::from(consent.executable()),
                    arguments: request.arguments.iter().map(OsString::from).collect(),
                    ..PluginCommand::default()
                },
                grant,
                consent,
                held,
            })
        }
        _ => Err(invalid_params(
            "send either plugin (a plugin folder) or executable (a command line)",
        )),
    }
}

fn grant(
    principal_id: u64,
    capabilities: &[String],
    allowed_operations: &[String],
) -> Result<PluginGrant, &'static str> {
    let capabilities = capabilities
        .iter()
        .map(|name| {
            PluginCapability::from_protocol_name(name).ok_or_else(|| {
                failure(
                    "invalid_params",
                    format!("unknown plugin capability {name:?}"),
                    json!({"canonical_mutation": false}),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PluginGrant::new(
        principal_id,
        capabilities,
        allowed_operations.iter().cloned(),
        PluginLimits::HOST_MAX,
    ))
}

/// A failed plugin run with its original cause.
fn host_failure(error: RunFailure) -> &'static str {
    match error {
        RunFailure::Changed(error) => failed_because("plugin_failed", error),
        RunFailure::Host(PluginHostError::Cancelled) => "request_cancelled",
        RunFailure::Host(error @ PluginHostError::Gateway(_)) => {
            failed_because("plugin_refused", error)
        }
        RunFailure::Host(error) => failed_because("plugin_failed", error),
    }
}

impl LiveBridge {
    fn prepare_plugin(
        &self,
        app: &KetchupApp,
        request: &RunPluginRequest,
        ui_busy: bool,
    ) -> Result<(), &'static str> {
        if self.program_check_job.is_some() {
            return Err(failure(
                "busy",
                "Another program, geometry check or plugin is running.",
                json!({"canonical_mutation": false}),
            ));
        }
        if request.timeout_ms == 0 || request.timeout_ms > MAX_APPLY_VERIFY_TIMEOUT_MS {
            return Err("invalid_job_timeout");
        }
        Self::mandatory_validation_selection(&request.validators)?;
        Self::guard(app, &request.expected)?;
        Self::available(app, ui_busy)
    }

    /// Checks the request, then reads, stages and hashes the plugin on a
    /// worker thread: a large or slow folder never blocks the window.
    pub(super) fn start_plugin(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        answer: PluginAnswer,
        request: RunPluginRequest,
        ui_busy: bool,
    ) {
        take_error_details();
        if let Err(error) = Self::require_request_authority(&answer.cancelled)
            .and_then(|()| self.prepare_plugin(app, &request, ui_busy))
        {
            Self::reply(app, answer.id, &answer.reply, Err(error));
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let worker_request = request.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-plugin-read".into())
            .spawn(move || {
                take_error_details();
                let prepared = launch(&worker_request).map_err(|code| (code, take_error_details()));
                let _ = sender.try_send(prepared);
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            let error = failed_because("job_worker_unavailable", error);
            Self::reply(app, answer.id, &answer.reply, Err(error));
            return;
        }
        self.program_check_job = Some(program_check::ProgramCheckJob::PluginPreparing(
            PluginPrepareJob {
                answer,
                request,
                receiver,
            },
        ));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_plugin_preparing(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: PluginPrepareJob,
    ) {
        take_error_details();
        let launch = if let Err(error) = Self::require_request_authority(&job.answer.cancelled) {
            Err(error)
        } else {
            match job.receiver.try_recv() {
                Ok(Ok(launch)) => Self::guard(app, &job.request.expected).map(|()| launch),
                Ok(Err((code, details))) => {
                    restore_error_details(details);
                    Err(code)
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.program_check_job =
                        Some(program_check::ProgramCheckJob::PluginPreparing(job));
                    context.request_repaint_after(Duration::from_millis(10));
                    return;
                }
                Err(error) => Err(failed_because("job_worker_unavailable", error)),
            }
        };
        match launch {
            Ok(launch) => self.run_prepared_plugin(app, context, job.answer, job.request, launch),
            Err(error) => Self::reply(app, job.answer.id, &job.answer.reply, Err(error)),
        }
    }

    /// The user decides only on a run that could otherwise start.
    fn run_prepared_plugin(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        answer: PluginAnswer,
        request: RunPluginRequest,
        launch: PluginLaunch,
    ) {
        let PluginAnswer {
            id,
            reply,
            cancelled,
        } = answer;
        let PluginLaunch {
            command,
            grant,
            consent,
            held,
        } = launch;
        if let Err(error) = app
            .consent_to_plugin(consent)
            .and_then(|()| Self::require_request_authority(&cancelled))
        {
            Self::reply(app, id, &reply, Err(error));
            return;
        }
        let snapshot = app.document.current();
        let timeout = Duration::from_millis(request.timeout_ms);
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&worker_cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-plugin".into())
            .spawn(move || {
                // The consent prompt may have been open for minutes.
                let result = held
                    .unchanged()
                    .map_err(RunFailure::Changed)
                    .and_then(|()| {
                        run_plugin_command(&command, &snapshot, grant, timeout, &thread_cancelled)
                            .map_err(RunFailure::Host)
                    });
                drop(held);
                let _ = sender.try_send(result);
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            Self::reply(
                app,
                id,
                &reply,
                Err(failed_because("job_worker_unavailable", error)),
            );
            return;
        }
        self.program_check_job = Some(program_check::ProgramCheckJob::Plugin(PluginJob {
            id,
            reply,
            cancelled,
            worker_cancelled,
            before: app.live_bridge_stamp(),
            validators: request.validators,
            strict: request.strict,
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_plugin(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: PluginJob,
    ) {
        take_error_details();
        let run = if let Err(error) = Self::require_request_authority(&job.cancelled) {
            job.worker_cancelled.store(true, Ordering::Release);
            Err(error)
        } else if let Err(error) = Self::guard(app, &Some(job.before.clone())) {
            // A human edit wins at once; the plugin's program can never publish.
            job.worker_cancelled.store(true, Ordering::Release);
            Err(error)
        } else {
            match job.receiver.try_recv() {
                Ok(run) => run.map_err(host_failure),
                Err(mpsc::TryRecvError::Empty) => {
                    self.program_check_job = Some(program_check::ProgramCheckJob::Plugin(job));
                    context.request_repaint_after(Duration::from_millis(10));
                    return;
                }
                Err(error) => Err(failed_because("job_worker_unavailable", error)),
            }
        };
        let program = match run {
            Ok(PluginRunResult {
                program: Some(program),
                ..
            }) => program,
            Ok(run) => {
                let result = json!({
                    "published": false,
                    "plugin": {"package": run.manifest.package(), "version": run.manifest.version()},
                    "query_count": run.query_count,
                });
                Self::reply(app, job.id, &job.reply, Ok(result));
                return;
            }
            Err(error) => {
                Self::reply(app, job.id, &job.reply, Err(error));
                return;
            }
        };
        let current_revision = app.document.current().revision_id();
        if program.revision_id != current_revision {
            let error = failure(
                "stale_document",
                format!(
                    "plugin read revision {}, the document is at {current_revision}",
                    program.revision_id
                ),
                json!({"canonical_mutation": false}),
            );
            Self::reply(app, job.id, &job.reply, Err(error));
            return;
        }
        let request = ApplyAndVerifyRequest {
            expected: None,
            selection: None,
            program: program.program,
            validators: job.validators,
            timeout_ms: DEFAULT_APPLY_VERIFY_TIMEOUT_MS,
            save: None,
            strict: job.strict,
        };
        let busy = ui_busy(context);
        self.start_queued_apply_and_verify(
            app,
            context,
            job.id,
            job.reply,
            job.cancelled,
            request,
            busy,
        );
    }
}
