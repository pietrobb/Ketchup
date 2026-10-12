//! Runs a local plugin process against the window's document. The process runs
//! off the UI thread on a snapshot; the typed program it sends is published by
//! the `apply_and_verify` job, the window's one path for typed programs, only
//! while the document is still at the revision the plugin read. It starts only
//! after the user allowed it in the window (`plugin_consent`).
use super::*;
use ketchup_assistant::extension::{PluginCapability, PluginGrant, PluginLimits};
use ketchup_scheduler::plugin::{PluginHostError, PluginRunResult, run_plugin_process};
use std::ffi::OsString;

const fn default_plugin_timeout_ms() -> u64 {
    30_000
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunPluginRequest {
    #[serde(default)]
    pub expected: Option<Stamp>,
    /// Absolute path of the plugin executable.
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Must match the principal the plugin declares in its HELLO.
    pub principal_id: u64,
    /// Granted capabilities by protocol name, e.g. `edit.cad-program.v1`.
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
    receiver: mpsc::Receiver<Result<PluginRunResult, PluginHostError>>,
}

fn grant(request: &RunPluginRequest) -> Result<PluginGrant, &'static str> {
    let capabilities = request
        .capabilities
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
        request.principal_id,
        capabilities,
        request.allowed_operations.iter().cloned(),
        PluginLimits::HOST_MAX,
    ))
}

/// A failed plugin run with its original cause.
fn host_failure(error: PluginHostError) -> &'static str {
    match error {
        PluginHostError::Cancelled => "request_cancelled",
        PluginHostError::Gateway(_) => failed_because("plugin_refused", error),
        error => failed_because("plugin_failed", error),
    }
}

impl LiveBridge {
    fn prepare_plugin(
        &self,
        app: &KetchupApp,
        request: &RunPluginRequest,
        ui_busy: bool,
    ) -> Result<PluginGrant, &'static str> {
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
        if !Path::new(&request.executable).is_absolute() {
            return Err("invalid_path");
        }
        Self::mandatory_validation_selection(&request.validators)?;
        Self::guard(app, &request.expected)?;
        Self::available(app, ui_busy)?;
        grant(request)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_plugin(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        request: RunPluginRequest,
        ui_busy: bool,
    ) {
        take_error_details();
        // The user decides only on a run that could otherwise start.
        let grant = match Self::require_request_authority(&cancelled)
            .and_then(|()| self.prepare_plugin(app, &request, ui_busy))
            .and_then(|grant| app.consent_to_plugin(&request).map(|()| grant))
            .and_then(|grant| Self::require_request_authority(&cancelled).map(|()| grant))
        {
            Ok(grant) => grant,
            Err(error) => {
                Self::reply(app, id, &reply, Err(error));
                return;
            }
        };
        let snapshot = app.document.current();
        let executable = PathBuf::from(&request.executable);
        let arguments = request
            .arguments
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        let timeout = Duration::from_millis(request.timeout_ms);
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&worker_cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-plugin".into())
            .spawn(move || {
                let result = run_plugin_process(
                    executable,
                    &arguments,
                    &snapshot,
                    grant,
                    timeout,
                    &thread_cancelled,
                );
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
