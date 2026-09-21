//! Background mesh-to-exact conversion orchestration and review UI.

use super::*;
use ketchup_application::mesh_conversion::{
    MeshConversionPlan, MeshConversionProgress, MeshConversionStage, MeshConversionTask,
    MeshConversionTaskEvent, MeshConversionVerification, commit_mesh_conversion,
    start_mesh_conversion,
};

const MESH_CONVERSION_TIMEOUT: Duration = Duration::from_secs(60);

struct PendingMeshConversion {
    plan: MeshConversionPlan,
    verification: MeshConversionVerification,
}

#[derive(Default)]
pub(super) struct MeshConversionUiState {
    task: Option<MeshConversionTask>,
    pending: Option<PendingMeshConversion>,
    progress: Option<MeshConversionProgress>,
}

impl KetchupApp {
    pub(super) fn selected_mesh_feature_id(&self) -> Option<FeatureId> {
        let selected = self.selected_occurrence_ids();
        if selected.len() != 1 {
            return None;
        }
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(*selected.iter().next()?)?;
        let definition = snapshot.definition(occurrence.definition_id())?;
        let [feature_id] = definition.feature_ids() else {
            return None;
        };
        snapshot
            .feature(*feature_id)
            .is_some_and(|feature| matches!(feature.kind(), FeatureKind::MeshBody(_)))
            .then_some(*feature_id)
    }

    pub(super) fn mesh_conversion_active(&self) -> bool {
        self.mesh_conversion_state.task.is_some() || self.mesh_conversion_state.pending.is_some()
    }

    pub(super) fn begin_mesh_conversion_review(&mut self) {
        self.cancel_mesh_conversion();
        let result = (|| {
            let feature_id = self
                .selected_mesh_feature_id()
                .ok_or_else(|| "select exactly one mesh body occurrence".to_owned())?;
            let executable = self.exact_worker_executable()?;
            start_mesh_conversion(
                &self.document,
                feature_id,
                MESH_CONVERSION_TOLERANCE_MM,
                executable,
                MESH_CONVERSION_TIMEOUT,
                || {},
            )
        })();
        match result {
            Ok(task) => {
                self.mesh_conversion_state.task = Some(task);
                self.mesh_conversion_state.progress = Some(MeshConversionProgress {
                    stage: MeshConversionStage::Recognizing,
                    completed: 0,
                    total: 3,
                });
            }
            Err(reason) => self.set_mesh_conversion_error(reason),
        }
    }

    pub(super) fn cancel_mesh_conversion(&mut self) {
        if let Some(task) = self.mesh_conversion_state.task.take() {
            task.cancel();
        }
        self.mesh_conversion_state.pending = None;
        self.mesh_conversion_state.progress = None;
    }

    fn set_mesh_conversion_error(&mut self, reason: String) {
        self.digest = self.catalog.format(
            "error-mesh-conversion",
            &BTreeMap::from([("reason", reason)]),
        );
    }

    pub(super) fn poll_mesh_conversion(&mut self, context: &egui::Context) {
        let snapshot = self.document.current();
        let mutation_epoch = self.document.mutation_epoch();
        if self
            .mesh_conversion_state
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.plan.matches_source(&snapshot, mutation_epoch))
        {
            self.mesh_conversion_state.pending = None;
            self.mesh_conversion_state.progress = None;
            self.set_mesh_conversion_error("mesh conversion plan is stale".to_owned());
        }
        if self
            .mesh_conversion_state
            .task
            .as_ref()
            .is_some_and(|task| !task.source.matches(&snapshot, mutation_epoch))
        {
            self.cancel_mesh_conversion();
            self.set_mesh_conversion_error("mesh conversion plan is stale".to_owned());
            return;
        }

        loop {
            let received = match self.mesh_conversion_state.task.as_ref() {
                Some(task) => task.poll(),
                None => return,
            };
            match received {
                Ok(MeshConversionTaskEvent::Progress(progress)) => {
                    let previous = self
                        .mesh_conversion_state
                        .progress
                        .map_or(0, |value| value.completed);
                    if progress.completed >= previous {
                        self.mesh_conversion_state.progress = Some(progress);
                    }
                }
                Ok(MeshConversionTaskEvent::Finished(result)) => {
                    let task = self
                        .mesh_conversion_state
                        .task
                        .take()
                        .expect("finished mesh conversion task");
                    if !task.source.matches(&snapshot, mutation_epoch) {
                        self.mesh_conversion_state.progress = None;
                        self.set_mesh_conversion_error("mesh conversion plan is stale".to_owned());
                    } else {
                        match *result {
                            Ok(prepared) => {
                                self.mesh_conversion_state.pending = Some(PendingMeshConversion {
                                    plan: prepared.plan,
                                    verification: prepared.verification,
                                });
                                self.mesh_conversion_state.progress = None;
                                self.digest = self.catalog.text("digest-mesh-conversion-verified");
                            }
                            Err(error) => {
                                self.mesh_conversion_state.progress = None;
                                self.set_mesh_conversion_error(error.to_string());
                            }
                        }
                    }
                    return;
                }
                Err(TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(25));
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.mesh_conversion_state.task = None;
                    self.mesh_conversion_state.progress = None;
                    self.set_mesh_conversion_error(
                        "mesh conversion worker disconnected".to_owned(),
                    );
                    return;
                }
            }
        }
    }

    fn confirm_mesh_conversion(&mut self) {
        let Some(pending) = self.mesh_conversion_state.pending.take() else {
            return;
        };
        match self.complete_mutation_with_work_recovery(|document| {
            commit_mesh_conversion(document, &pending.plan, pending.verification)
        }) {
            Ok(package) => {
                if let Some(task) = self.exact_task.take() {
                    task.cancelled.store(true, Ordering::Release);
                }
                let snapshot = self.document.current();
                let package = Arc::new(ExactBodyPackage::from(package));
                self.exact_results
                    .insert_current(&snapshot, Arc::clone(&package))
                    .expect("verified conversion package matches committed snapshot");
                if !package.topological_references().is_empty() {
                    self.topology_results
                        .insert_current(&snapshot, package)
                        .expect("verified topology package matches committed snapshot");
                }
                self.exact_source = None;
                self.exact_retry_at = None;
                self.render_plan = Some(Arc::new(InstancedRenderPlan::from_snapshot(
                    &snapshot,
                    &self.exact_results,
                    &mut self.render_cache,
                )));
                self.interaction_projection_cache.get_mut().take();
                self.digest = self.catalog.text("digest-mesh-conversion-committed");
            }
            Err(error) => self.set_mesh_conversion_error(error.to_string()),
        }
    }

    pub(super) fn show_mesh_conversion_window(&mut self, context: &egui::Context) {
        if self.mesh_conversion_state.task.is_some() {
            let progress = self
                .mesh_conversion_state
                .progress
                .unwrap_or(MeshConversionProgress {
                    stage: MeshConversionStage::Recognizing,
                    completed: 0,
                    total: 3,
                });
            let stage = match progress.stage {
                MeshConversionStage::Recognizing => self
                    .catalog
                    .text("dialog-mesh-conversion-stage-recognizing"),
                MeshConversionStage::EvaluatingExact => {
                    self.catalog.text("dialog-mesh-conversion-stage-evaluating")
                }
                MeshConversionStage::Verifying => {
                    self.catalog.text("dialog-mesh-conversion-stage-verifying")
                }
            };
            let mut cancel = false;
            egui::Window::new(self.catalog.text("dialog-mesh-conversion-title"))
                .id(egui::Id::new("mesh-conversion-progress"))
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label(self.catalog.format(
                        "dialog-mesh-conversion-progress",
                        &BTreeMap::from([
                            ("stage", stage),
                            ("completed", progress.completed.to_string()),
                            ("total", progress.total.to_string()),
                        ]),
                    ));
                    cancel = ui
                        .button(self.catalog.text("dialog-mesh-conversion-cancel"))
                        .clicked();
                });
            if cancel {
                self.cancel_mesh_conversion();
                self.digest = self.catalog.text("digest-cancelled");
            }
            return;
        }

        let Some(pending) = self.mesh_conversion_state.pending.as_ref() else {
            return;
        };
        let kind_key = match pending.plan.candidate().kind() {
            ketchup_core::mesh_recognition::RecognizedMeshKind::Box => {
                "dialog-mesh-conversion-kind-box"
            }
            ketchup_core::mesh_recognition::RecognizedMeshKind::Cylinder => {
                "dialog-mesh-conversion-kind-cylinder"
            }
            ketchup_core::mesh_recognition::RecognizedMeshKind::LinearExtrusion => {
                "dialog-mesh-conversion-kind-extrusion"
            }
        };
        let values = BTreeMap::from([
            ("kind", self.catalog.text(kind_key)),
            ("tolerance", format!("{:.6}", pending.plan.tolerance_mm())),
            (
                "recognition",
                format!("{:.6}", pending.plan.residuals().maximum_mm()),
            ),
            (
                "source_to_exact",
                format!("{:.6}", pending.verification.max_source_to_exact_mm()),
            ),
            (
                "exact_to_source",
                format!("{:.6}", pending.verification.max_exact_to_source_mm()),
            ),
        ]);
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-mesh-conversion-title"))
            .id(egui::Id::new("mesh-conversion-review"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format("dialog-mesh-conversion-kind", &values));
                ui.label(
                    self.catalog
                        .format("dialog-mesh-conversion-tolerance", &values),
                );
                ui.label(
                    self.catalog
                        .format("dialog-mesh-conversion-recognition", &values),
                );
                ui.label(self.catalog.format("dialog-mesh-conversion-exact", &values));
                ui.colored_label(
                    Color32::YELLOW,
                    self.catalog.text("dialog-mesh-conversion-warning"),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    confirm = ui
                        .button(self.catalog.text("dialog-mesh-conversion-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-mesh-conversion-cancel"))
                        .clicked();
                });
            });
        if cancel {
            self.cancel_mesh_conversion();
            self.digest = self.catalog.text("digest-cancelled");
        } else if confirm {
            self.confirm_mesh_conversion();
        }
    }
}
