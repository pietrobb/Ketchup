//! The dock's variant buttons: each is the MCP request of the same name on a
//! bridge without a socket, so the window gets the same checks, refusals and
//! Undo steps. Use and Export run on the bridge's worker threads (Use is the
//! `set_params` apply); the UI thread only starts them and polls each frame.
use super::plugin_menu::rejection_text;
use super::window_request::{Polled, WindowRequest};
use super::*;

/// What a dock button asks for.
#[derive(Clone, Debug)]
pub(crate) enum VariantAction {
    Save(String),
    Use(String),
    Delete(String),
    Export(PathBuf),
}

#[derive(Default)]
pub(crate) struct VariantDock {
    run: Option<WindowRequest<VariantAction>>,
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
        let (run, reply) = WindowRequest::held(action);
        self.live.variants.run = Some(run);
        reply
    }
}

impl KetchupApp {
    pub(crate) fn start_variant_action(&mut self, context: &egui::Context, action: VariantAction) {
        let expected = self.live_bridge_stamp();
        let run =
            WindowRequest::start(
                self,
                context,
                action.clone(),
                |runner, app, queued| match action {
                    VariantAction::Use(name) => runner.start_queued_apply_program(
                        app,
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
                        runner.start_queued_variant_export(app, context, queued, request);
                    }
                    VariantAction::Save(name) => {
                        let request = Request::SaveProgramVariant { expected, name };
                        let result =
                            runner.program_variant_request(app, request, false, &queued.cancelled);
                        LiveBridge::reply(app, 0, &queued.reply, result);
                    }
                    VariantAction::Delete(name) => {
                        let request = Request::DeleteProgramVariant { expected, name };
                        let result =
                            runner.program_variant_request(app, request, false, &queued.cancelled);
                        LiveBridge::reply(app, 0, &queued.reply, result);
                    }
                },
            );
        self.live.variants.outcome = None;
        self.live.variants.run = Some(run);
    }

    /// Drives the dock's running action; taken out of the app meanwhile, so it
    /// is not a busy reason for its own publication.
    pub(crate) fn poll_window_variant(&mut self, context: &egui::Context) {
        let Some(run) = self.live.variants.run.take() else {
            return;
        };
        let text = match run.poll(self, context) {
            Polled::Running(run) => {
                self.live.variants.run = Some(run);
                return;
            }
            Polled::Answered(action, Ok(result)) => {
                let (key, name) = match action {
                    VariantAction::Save(name) => ("variants-saved", name),
                    VariantAction::Use(name) => ("variants-used", name),
                    VariantAction::Delete(name) => ("variants-deleted", name),
                    VariantAction::Export(path) => {
                        ("variants-exported", path.display().to_string())
                    }
                };
                let arguments =
                    BTreeMap::from([("name", name), ("count", result["exported"].to_string())]);
                self.catalog.format(key, &arguments)
            }
            Polled::Answered(_, Err(refusal)) => rejection_text(&refusal),
            Polled::Disconnected(_) => {
                rejection_text(&rejection_result("program_worker_disconnected", None))
            }
        };
        self.live.variants.outcome = Some(text);
    }
}
