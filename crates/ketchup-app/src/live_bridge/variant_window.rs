//! The dock's variant buttons: each is the MCP request of the same name on a
//! bridge without a socket, so the window gets the same checks, refusals and
//! Undo steps. Use and Export run on the bridge's worker threads (Use is the
//! `set_params` apply); the UI thread only starts them and polls each frame.
use super::plugin_menu::rejection_text;
use super::program_dry_run::QueuedReply;
use super::*;

/// What a dock button asks for.
#[derive(Clone, Debug)]
pub(crate) enum VariantAction {
    Save(String),
    Use(String),
    Delete(String),
    Export(PathBuf),
}

struct WindowRun {
    action: VariantAction,
    runner: LiveBridge,
    receiver: mpsc::Receiver<Response>,
}

#[derive(Default)]
pub(crate) struct VariantDock {
    run: Option<WindowRun>,
    /// The name typed for the next saved variant.
    pub(crate) name: String,
    /// How the last action ended.
    pub(crate) outcome: Option<String>,
}

impl VariantDock {
    pub(crate) const fn is_running(&self) -> bool {
        self.run.is_some()
    }
}

#[cfg(test)]
impl KetchupApp {
    /// Whether the dock's running action is the queued program apply that
    /// `set_params` starts, and whether the window reports it as busy.
    pub(crate) fn variant_run_is_queued_apply_and_busy(&self) -> (bool, bool) {
        let queued = self.live.variants.run.as_ref().is_some_and(|run| {
            matches!(
                run.runner.program_check_job,
                Some(program_check::ProgramCheckJob::Planning(_))
            )
        });
        let busy = busy::blockers(self, false).iter().any(|blocker| {
            serde_json::to_value(blocker).is_ok_and(|value| value["target"] == "window_variant")
        });
        (queued, busy)
    }

    /// Keeps a dock action running until the returned sender answers or drops.
    pub(crate) fn hold_variant_run(&mut self, action: VariantAction) -> mpsc::SyncSender<Response> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.live.variants.run = Some(WindowRun {
            action,
            runner: transport::local(),
            receiver,
        });
        reply
    }
}

impl KetchupApp {
    pub(crate) fn start_variant_action(&mut self, context: &egui::Context, action: VariantAction) {
        take_error_details();
        let (reply, receiver) = mpsc::sync_channel(1);
        let mut runner = transport::local();
        let expected = self.live_bridge_stamp();
        let queued = QueuedReply {
            id: 0,
            reply,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        // A job an MCP client started keeps the window as busy as one of its own.
        let client_jobs = self
            .live
            .bridge
            .as_ref()
            .map(LiveBridge::busy_jobs)
            .unwrap_or_default();
        if let Err(code) = busy::reject_if_busy(&client_jobs) {
            LiveBridge::reply(self, 0, &queued.reply, Err(code));
        } else {
            match action.clone() {
                VariantAction::Use(name) => runner.start_queued_apply_program(
                    self,
                    context,
                    0,
                    queued.reply,
                    queued.cancelled,
                    Request::UseProgramVariant { expected, name },
                    false,
                ),
                VariantAction::Export(path) => {
                    let path = path.display().to_string();
                    let request = Request::ExportProgramVariants { expected, path };
                    runner.start_queued_variant_export(self, context, queued, request);
                }
                VariantAction::Save(name) | VariantAction::Delete(name) => {
                    let request = if matches!(action, VariantAction::Save(_)) {
                        Request::SaveProgramVariant { expected, name }
                    } else {
                        Request::DeleteProgramVariant { expected, name }
                    };
                    let result =
                        runner.program_variant_request(self, request, false, &queued.cancelled);
                    LiveBridge::reply(self, 0, &queued.reply, result);
                }
            }
        }
        self.live.variants.outcome = None;
        self.live.variants.run = Some(WindowRun {
            action,
            runner,
            receiver,
        });
        context.request_repaint();
    }

    /// Drives the dock's running action; taken out of the app meanwhile, so it
    /// is not a busy reason for its own publication.
    pub(crate) fn poll_window_variant(&mut self, context: &egui::Context) {
        let Some(mut run) = self.live.variants.run.take() else {
            return;
        };
        run.runner.poll_program_check_job(self, context);
        let text = match run.receiver.try_recv() {
            Ok(response) if response.ok => {
                let result = response.result.unwrap_or_default();
                let (key, name) = match &run.action {
                    VariantAction::Save(name) => ("variants-saved", name.clone()),
                    VariantAction::Use(name) => ("variants-used", name.clone()),
                    VariantAction::Delete(name) => ("variants-deleted", name.clone()),
                    VariantAction::Export(path) => {
                        ("variants-exported", path.display().to_string())
                    }
                };
                let arguments =
                    BTreeMap::from([("name", name), ("count", result["exported"].to_string())]);
                self.catalog.format(key, &arguments)
            }
            Ok(response) => rejection_text(response.result.as_ref().unwrap_or(&Value::Null)),
            Err(mpsc::TryRecvError::Empty) => {
                self.live.variants.run = Some(run);
                context.request_repaint_after(Duration::from_millis(20));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                rejection_text(&rejection_result("program_worker_disconnected", None))
            }
        };
        self.live.variants.outcome = Some(text);
    }
}
