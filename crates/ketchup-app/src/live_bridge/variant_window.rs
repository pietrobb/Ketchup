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
    /// One parameter typed in the dock: the `set_params` apply.
    SetParam(String, f64),
}

#[derive(Default)]
pub(crate) struct VariantDock {
    run: Option<WindowRequest<VariantAction>>,
    /// The name typed for the next saved variant.
    pub(crate) name: String,
    /// How the last action ended.
    pub(crate) outcome: Option<String>,
    /// The parameter being typed and its text.
    pub(crate) draft: Option<(String, String)>,
    /// The parameters of the last evaluation, shown while the next one runs.
    pub(crate) params: Vec<ketchup_program::model::Param>,
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
                        super::program_check::program_request_deadline(),
                    ),
                    VariantAction::SetParam(name, value) => runner.start_queued_apply_program(
                        app,
                        context,
                        0,
                        queued.reply,
                        queued.cancelled,
                        Request::SetProgramParams {
                            expected,
                            params: BTreeMap::from([(name, value)]),
                        },
                        false,
                        super::program_check::program_request_deadline(),
                    ),
                    VariantAction::Export(path) => {
                        let path = path.display().to_string();
                        let request = Request::ExportProgramVariants {
                            expected,
                            path,
                            decimal: app.value_box.decimal.into(),
                        };
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
                let (key, arguments) = match action {
                    VariantAction::Save(name) => ("variants-saved", vec![("name", name)]),
                    VariantAction::Use(name) => ("variants-used", vec![("name", name)]),
                    VariantAction::Delete(name) => ("variants-deleted", vec![("name", name)]),
                    VariantAction::Export(path) => (
                        "variants-exported",
                        vec![
                            ("name", path.display().to_string()),
                            ("count", result["exported"].to_string()),
                        ],
                    ),
                    VariantAction::SetParam(name, value) => {
                        let decimal = self.value_box.decimal;
                        let value = crate::program_variants_ui::number_text(value, decimal);
                        ("params-set", vec![("name", name), ("value", value)])
                    }
                };
                self.catalog.format(key, &arguments.into_iter().collect())
            }
            Polled::Answered(_, Err(refusal)) => rejection_text(&refusal),
            Polled::Disconnected(_) => {
                rejection_text(&rejection_result("program_worker_disconnected", None))
            }
        };
        self.live.variants.outcome = Some(text);
    }
}
