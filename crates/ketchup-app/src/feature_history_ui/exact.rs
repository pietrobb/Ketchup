//! Exact bodies the feature history needs before it can preview a replacement
//! or commit a propagated change. A worker evaluates them on a thread, so a slow
//! body never freezes the window, and Cancel stops that worker (review
//! 2026-10-17 EX-6).

use super::*;
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_scheduler::WorkerError;
use ketchup_scheduler::background::{BackgroundTask, TaskEvent, TaskStop};
use std::fmt;
use std::sync::mpsc::TryRecvError;

/// One body's evaluation budget plus one restart of a lost worker.
const EXACT_BODY_DEADLINE: Duration = Duration::from_secs(120);

/// Each evaluated body with the input digest of the graph it came from.
pub(crate) type EvaluatedBodies = Vec<(String, Arc<ExactBodyPackage>)>;

/// What the window does once the bodies arrive.
pub(crate) enum FeatureHistoryExactPurpose {
    /// Publish them and derive the replacement preview again.
    ReplacementPreview,
    /// Commit the reviewed propagated change with them.
    Confirm,
}

pub(crate) struct FeatureHistoryExactTask {
    task: BackgroundTask<(), Result<EvaluatedBodies, WorkerError>>,
    revision: u64,
    purpose: FeatureHistoryExactPurpose,
}

/// A commit asked for a body the background evaluation did not produce.
#[derive(Debug)]
pub(crate) struct UnevaluatedBody;

impl fmt::Display for UnevaluatedBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the change asks for an exact body that was not evaluated")
    }
}

/// The evaluation hook for a commit: it hands back the body evaluated for the
/// same graph and refuses a graph nobody evaluated.
pub(crate) fn evaluated_body(
    bodies: &EvaluatedBodies,
) -> impl FnMut(&ExactBRepGraph) -> Result<Arc<ExactBodyPackage>, UnevaluatedBody> + '_ {
    |graph| {
        bodies
            .iter()
            .find(|(digest, _)| *digest == graph.canonical_input_digest)
            .map(|(_, package)| Arc::clone(package))
            .ok_or(UnevaluatedBody)
    }
}

impl KetchupApp {
    /// Starts the worker for `graphs`; the previous background evaluation it
    /// replaces is cancelled. Answers false after reporting why it could not start.
    pub(crate) fn start_feature_history_exact(
        &mut self,
        graphs: Vec<ExactBRepGraph>,
        purpose: FeatureHistoryExactPurpose,
    ) -> bool {
        let executable = match self.exact_worker_executable() {
            Ok(executable) => executable,
            Err(error) => {
                self.feature_history_error(error.reason_text());
                return false;
            }
        };
        if let Some(task) = self.exact.task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        let bodies = u32::try_from(graphs.len().max(1)).unwrap_or(u32::MAX);
        let deadline = EXACT_BODY_DEADLINE.saturating_mul(bodies);
        let spawned = BackgroundTask::spawn(
            "feature-history-exact",
            deadline,
            move |context| {
                let cancelled = context.cancellation();
                let mut worker =
                    ExactWorkerSupervisor::spawn_with_cancellation(executable, cancelled)?;
                graphs
                    .iter()
                    .map(|graph| {
                        worker
                            .evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
                                graph,
                                &[],
                                cancelled,
                            )
                            .map(|package| {
                                (
                                    graph.canonical_input_digest.clone(),
                                    Arc::new(ExactBodyPackage::from(package)),
                                )
                            })
                    })
                    .collect()
            },
            // The window polls every frame while the task runs.
            || {},
        );
        match spawned {
            Ok(task) => {
                self.feature_history.exact_task = Some(FeatureHistoryExactTask {
                    task,
                    revision: self.document_revision(),
                    purpose,
                });
                self.digest = self.catalog.text("feature-history-exact-running");
                true
            }
            Err(error) => {
                self.feature_history_error(error);
                false
            }
        }
    }

    #[must_use]
    pub fn feature_history_exact_running(&self) -> bool {
        self.feature_history.exact_task.is_some()
    }

    /// A spinner while the bodies are being evaluated; answers whether they are.
    pub(super) fn show_feature_history_exact_running(&self, ui: &mut egui::Ui) -> bool {
        let running = self.feature_history_exact_running();
        if running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.catalog.text("feature-history-exact-running"));
            });
        }
        running
    }

    /// Stops the worker; dropping the task cancels it.
    pub(crate) fn cancel_feature_history_exact(&mut self) {
        if self.feature_history.exact_task.take().is_some() {
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    pub(crate) fn poll_feature_history_exact(&mut self, context: &egui::Context) {
        let Some(pending) = &self.feature_history.exact_task else {
            return;
        };
        let result = match pending.task.poll() {
            Ok(TaskEvent::Finished(result)) => result,
            Ok(TaskEvent::Progress(())) | Err(TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(25));
                return;
            }
            Err(TryRecvError::Disconnected) => Err(TaskStop::Cancelled),
        };
        let pending = self
            .feature_history
            .exact_task
            .take()
            .expect("polled feature history evaluation");
        context.request_repaint();
        let bodies = match result {
            Ok(Ok(bodies)) if pending.revision == self.document_revision() => bodies,
            stopped => {
                let reason = match stopped {
                    Ok(Ok(_)) => FeatureHistoryPreviewError::PlanChanged.to_string(),
                    Ok(Err(error)) => error.to_string(),
                    Err(TaskStop::Cancelled) => self.catalog.text("digest-cancelled"),
                    Err(TaskStop::TimedOut) => self.catalog.text("feature-history-exact-timed-out"),
                };
                self.feature_history.preview = None;
                self.feature_history_error(reason);
                return;
            }
        };
        match pending.purpose {
            FeatureHistoryExactPurpose::ReplacementPreview => {
                let snapshot = self.document.current();
                let packages = bodies
                    .into_iter()
                    .map(|(_, package)| package)
                    .collect::<Vec<_>>();
                match ExactResultRegistry::publish_body_results(
                    &snapshot,
                    &self.exact.results,
                    packages,
                ) {
                    Ok(results) => {
                        self.exact.results = results;
                        self.prepare_component_replacement();
                    }
                    Err(error) => self.feature_history_error(error),
                }
            }
            FeatureHistoryExactPurpose::Confirm => {
                self.confirm_feature_history_preview_with(Some(&bodies));
            }
        }
    }
}
