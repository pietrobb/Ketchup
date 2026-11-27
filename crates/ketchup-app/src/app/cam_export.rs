//! The CAM program of the document's plan as a manufacturing export: simulated exactly,
//! written for a controller of the dialect library, after the user confirms its release.

use crate::*;
use ketchup_application::cam_workflow::{CamComputedReview, CamReviewError};
use ketchup_model::cam::{
    CamContourSide, CamGCodeDialect, CamGCodeDialectError, CamOperation, CamPlan,
};

#[cfg(test)]
#[path = "cam_export_tests.rs"]
mod tests;

/// The File menu's CAM program export: the plan, what it machines, the controller it is
/// written for, and the simulated review once the user asked for it.
pub(crate) struct CamExportDialog {
    pub(crate) plan_id: CamPlanId,
    pub(crate) operation: CamWindowOperation,
    /// The selected face of the plan's part, as the review was last made for.
    pub(crate) face: Option<u32>,
    /// The library dialect the program is written for.
    pub(crate) dialect_id: String,
    /// The library's controller dialects as (id, name).
    pub(crate) dialect_choices: Vec<(String, String)>,
    pub(crate) review: Option<CamReviewSummary>,
    /// The preview or export planning and simulating now, if any.
    pub(crate) running: Option<CamRun>,
}

/// A CAM review planning and simulating on a thread, so the window keeps painting; Cancel,
/// closing the window or an edit of the document drops it, which stops its exact worker
/// (review 2026-11-27 R1127-CAM-2).
pub(crate) struct CamRun {
    pub(crate) cancelled: Arc<AtomicBool>,
    receiver: Receiver<Result<CamComputedReview, CamReviewError>>,
    /// The document state it is computed on.
    snapshot: Snapshot,
    mutation_epoch: u64,
    /// The review token and the new file a confirmed export writes; none for a preview.
    export: Option<(String, PathBuf)>,
}

impl CamRun {
    fn start(
        worker_path: PathBuf,
        snapshot: Snapshot,
        mutation_epoch: u64,
        request: CamReviewRequest,
        export: Option<(String, PathBuf)>,
        context: &egui::Context,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (thread_cancelled, repaint) = (Arc::clone(&cancelled), context.clone());
        let thread_snapshot = snapshot.clone();
        std::thread::spawn(move || {
            let computed = CamComputedReview::compute(
                Some(&worker_path),
                &thread_snapshot,
                request,
                &thread_cancelled,
            );
            if sender.send(computed).is_ok() {
                repaint.request_repaint();
            }
        });
        Self {
            cancelled,
            receiver,
            snapshot,
            mutation_epoch,
            export,
        }
    }
}

impl Drop for CamRun {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// What the window's program machines: the stock top, or the face the user selected,
/// read from the exact part as a client's `face_*` operation is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CamWindowOperation {
    #[default]
    Facing,
    ContourOutside,
    ContourInside,
    Pocket,
    Drill,
}

impl CamWindowOperation {
    const ALL: [Self; 5] = [
        Self::Facing,
        Self::ContourOutside,
        Self::ContourInside,
        Self::Pocket,
        Self::Drill,
    ];

    const fn label_key(self) -> &'static str {
        match self {
            Self::Facing => "cam-operation-facing",
            Self::ContourOutside => "cam-operation-contour-outside",
            Self::ContourInside => "cam-operation-contour-inside",
            Self::Pocket => "cam-operation-pocket",
            Self::Drill => "cam-operation-drill",
        }
    }

    /// The plan operation on the face `face_ordinal`; facing names none.
    const fn on_face(self, face_ordinal: u32) -> Option<CamOperation> {
        let side = match self {
            Self::Facing => return None,
            Self::Pocket => {
                return Some(CamOperation::FacePocket {
                    id: 1,
                    face_ordinal,
                });
            }
            Self::Drill => {
                return Some(CamOperation::FaceDrill {
                    id: 1,
                    face_ordinal,
                });
            }
            Self::ContourOutside => CamContourSide::Outside,
            Self::ContourInside => CamContourSide::Inside,
        };
        Some(CamOperation::FaceContour {
            id: 1,
            face_ordinal,
            side,
        })
    }
}

/// What the exact simulation of a written program found, told to the client that asked for
/// it in place of the window's release prompt, with what the simulation does not check
/// (owner decisions R1 and R3).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Machining {
    pub(crate) dialect: String,
    pub(crate) removed_stock_mm3: f64,
    pub(crate) residual_stock_mm3: f64,
    pub(crate) gouge_mm3: f64,
}

impl Machining {
    /// What the simulation of the stock, the plan's fixtures, the cutter and its holder
    /// leaves to the user.
    pub(crate) const NOT_CHECKED: [&'static str; 8] = [
        "the machine itself: travel limits, spindle head, table and clamps not declared as fixtures of the plan",
        // R1124og-CAM-901: the planner checks only the flute length of a through hole.
        "the room under a through hole: the drill goes past the stock's bottom by its tip and 1 mm, and only fixtures declared in the plan are checked there, not the table, spoilboard or clamps under it",
        "cutting forces, tool deflection or breakage, and whether the feeds and speeds suit the tool and material",
        "the tool length and work offset values set on the controller; the program only calls them",
        "how the controller runs the program beyond the dialect's codes; run it in the controller's simulator or dry first",
        // Review 2026-11-21 ac P3: an arc's rounded centre and ends move it further than a point.
        "the rounding of the written coordinates to the dialect's step: the simulation runs the planned path; a written point may differ from it by up to half a step, and a written arc by up to four steps",
        // Review 2026-11-20 ac P3: neither was said.
        "coolant: the program switches none on (no M7 or M8); turn it on yourself where the tool or material needs it",
        "how the cutter enters: every plunge goes straight down, without a ramp or helix, so the cutter must cut at its centre",
    ];
}

/// The library's controller dialects as (id, name), in library order.
pub(crate) fn cam_dialect_choices() -> Result<Vec<(String, String)>, CamGCodeDialectError> {
    Ok(CamGCodeDialect::library()?
        .iter()
        .map(|dialect| (dialect.id().to_owned(), dialect.name().to_owned()))
        .collect())
}

/// What a user is told the program is for: the dialect's name, or its id when it has none.
pub(crate) fn cam_dialect_name(dialect: &CamPostprocessorDialect) -> String {
    match dialect {
        CamPostprocessorDialect::GCode(dialect) => dialect.name().to_owned(),
        CamPostprocessorDialect::ControllerNeutralJson => dialect.id().to_owned(),
    }
}

impl KetchupApp {
    /// Writes the program of the document's first CAM plan running `operations`, or facing
    /// its stock when there are none, for `dialect` to the new file `path`. The program is
    /// simulated exactly first, stopped when `cancelled` is set. The user confirms its release
    /// knowing the controller and the result; a client is told them in the reply instead.
    pub(crate) fn export_current_cam_program_to(
        &mut self,
        path: &Path,
        dialect: CamPostprocessorDialect,
        operations: Vec<CamOperation>,
        cancelled: &AtomicBool,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let result = self.write_cam_program(path, dialect, operations, cancelled);
        self.report_export_outcome("cam-nc", path, result)
    }

    fn write_cam_program(
        &mut self,
        path: &Path,
        dialect: CamPostprocessorDialect,
        mut operations: Vec<CamOperation>,
        cancelled: &AtomicBool,
    ) -> Result<Written, ExportError> {
        let new_file = || ExportError::Destination {
            requirement: "a new file in an existing folder",
        };
        // Refused before the simulation and the prompt: a program replaces a file only for a
        // client that asked to with overwrite, and only once the user agrees in the window.
        let replaced = if path.exists() {
            if self.live.client_export.is_none() || !self.live.client_overwrite {
                return Err(ExportError::Destination {
                    requirement: "a new file in an existing folder; a client's program replaces an existing file only with overwrite and once the user agrees",
                });
            }
            Some(export_target_sha256(path)?.ok_or_else(new_file)?)
        } else {
            None
        };
        let snapshot = self.document.current();
        let mutation_epoch = self.document.mutation_epoch();
        let plan = snapshot
            .cam_plans()
            .next()
            .ok_or(ExportError::NothingToExport {
                subject: "CAM plan",
            })?;
        let plan_id = plan.id();
        // No operation cuts a thread: the program names the ones it leaves (review 2026-11-19 MF-807).
        let part = snapshot
            .definition(plan.target().definition_id)
            .map_or_else(String::new, |part| part.name().to_owned());
        let lost: Vec<String> = plan
            .uncut_threads(&snapshot)
            .into_iter()
            .map(|thread| format!("{part}: thread {thread}"))
            .collect();
        if operations.is_empty() {
            operations.push(
                plan.default_facing_operation(&snapshot, 1)
                    .map_err(ExportError::failed)?,
            );
        }
        let worker_path = self.exact_worker_executable()?;
        self.reviews.cam_reviews.set_worker_path(worker_path);
        let review = self
            .reviews
            .cam_reviews
            .preview(
                &snapshot,
                mutation_epoch,
                CamReviewRequest {
                    plan_id,
                    operations,
                    fixtures: Vec::new(),
                    dialect: dialect.clone(),
                },
                cancelled,
            )
            .map_err(ExportError::failed)?;
        let title = self.catalog.text("dialog-export-cam-title");
        let mut risk = self.catalog.format(
            "dialog-export-cam-risk",
            &BTreeMap::from([
                ("dialect", cam_dialect_name(&dialect)),
                ("removed", format!("{:.3}", review.removed_stock_mm3)),
                ("gouge", format!("{:.3}", review.gouge_mm3)),
            ]),
        );
        if !lost.is_empty() {
            risk.push_str(&self.catalog.format(
                "dialog-export-cam-threads",
                &BTreeMap::from([("threads", lost.join("; "))]),
            ));
        }
        self.authorize_path_side_effect(
            HighRiskClass::ReleaseManufacturingExportWithWarnings,
            "release-cam-nc-program",
            &title,
            &risk,
            path,
            review.content_digest.as_bytes(),
        )?;
        if replaced.is_some() {
            let title = self.catalog.text("dialog-export-overwrite-title");
            let risk = self.catalog.text("dialog-export-overwrite-risk");
            self.authorize_path_side_effect(
                HighRiskClass::Overwrite,
                "overwrite-cam-nc-program",
                &title,
                &risk,
                path,
                review.content_digest.as_bytes(),
            )?;
        }
        let still_same = |target: &Path| {
            export_target_sha256(target).ok().flatten().as_deref() == replaced.as_deref()
        };
        self.reviews
            .cam_reviews
            .export(
                &snapshot,
                mutation_epoch,
                &review.token,
                path,
                true,
                replaced
                    .is_some()
                    .then_some(&still_same as &dyn Fn(&Path) -> bool),
                cancelled,
            )
            .map_err(|error| match error {
                CamReviewError::FileExists | CamReviewError::InvalidPath => new_file(),
                CamReviewError::ReplacedFileChanged => ExportError::ChangedDuringConsent,
                other => ExportError::failed(other),
            })?;
        Ok(Written {
            lost,
            machining: Some(Machining {
                dialect: cam_dialect_name(&dialect),
                removed_stock_mm3: review.removed_stock_mm3,
                residual_stock_mm3: review.residual_stock_mm3,
                gouge_mm3: review.gouge_mm3,
            }),
            ..Written::alone(path)
        })
    }

    /// The one selected face of the part the plan `plan_id` machines, numbered as a client's
    /// `query kind=faces` numbers it; `None` when anything else or nothing is selected.
    fn selected_cam_face(&self, plan_id: CamPlanId) -> Option<u32> {
        let snapshot = self.document.current();
        let target = snapshot.cam_plan(plan_id)?.target().clone();
        let faces = self.selected_face_references()?;
        let [face] = faces.as_slice() else {
            return None;
        };
        let package = self
            .exact
            .topology_results
            .get_render(&snapshot, target.definition_id)?;
        if face.definition_id != target.definition_id
            || package.producer_feature_id() != target.feature_id
        {
            return None;
        }
        crate::planar_push_pull::face_ordinal(package.as_ref(), face)
    }

    /// The operation of the program the dialog `pending` reviews.
    fn cam_window_operation(
        &self,
        pending: &CamExportDialog,
        plan: &CamPlan,
        snapshot: &Snapshot,
    ) -> Result<CamOperation, Rejection> {
        if pending.operation == CamWindowOperation::Facing {
            return plan
                .default_facing_operation(snapshot, 1)
                .map_err(|error| failed("cam.operation", error));
        }
        pending
            .face
            .and_then(|face| pending.operation.on_face(face))
            .ok_or_else(|| self.catalog.refusal("cam-review-face-required"))
    }

    pub(crate) fn show_cam_export_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.cam_export_dialog.take() else {
            return;
        };
        if !self.poll_cam_run(&mut pending) {
            return;
        }
        // A review is of one operation on one face for one controller: another needs its own.
        let face = self.selected_cam_face(pending.plan_id);
        if face != pending.face {
            pending.face = face;
            if pending.operation != CamWindowOperation::Facing {
                pending.review = None;
                pending.running = None;
            }
        }
        let idle = pending.running.is_none();
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("cam-review-title"))
            .id(egui::Id::new("cam-review-export"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(format!(
                    "{} {}",
                    self.catalog.text("cam-review-plan"),
                    pending.plan_id.0
                ));
                let operation = pending.operation;
                let label = self.catalog.text("cam-review-operation");
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&label);
                        let response = egui::ComboBox::from_id_salt("cam-review-operation")
                            .selected_text(self.catalog.text(operation.label_key()))
                            .show_ui(ui, |ui| {
                                for choice in CamWindowOperation::ALL {
                                    ui.selectable_value(
                                        &mut pending.operation,
                                        choice,
                                        self.catalog.text(choice.label_key()),
                                    );
                                }
                            });
                        response.response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
                        });
                    })
                });
                if pending.operation != operation {
                    pending.review = None;
                }
                if pending.operation != CamWindowOperation::Facing {
                    ui.label(match pending.face {
                        Some(face) => self.catalog.format(
                            "cam-review-face",
                            &BTreeMap::from([("face", face.to_string())]),
                        ),
                        None => self.catalog.text("cam-review-face-required"),
                    });
                }
                let choices = std::mem::take(&mut pending.dialect_choices);
                let chosen = pending.dialect_id.clone();
                let label = self.catalog.text("cam-review-dialect");
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&label);
                        let response = egui::ComboBox::from_id_salt("cam-review-dialect")
                            .selected_text(
                                choices
                                    .iter()
                                    .find(|(id, _)| *id == chosen)
                                    .map_or(chosen.as_str(), |(_, name)| name.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for (id, name) in &choices {
                                    ui.selectable_value(&mut pending.dialect_id, id.clone(), name);
                                }
                            });
                        response.response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
                        });
                    })
                });
                pending.dialect_choices = choices;
                if pending.dialect_id != chosen {
                    pending.review = None;
                }
                if let Some(review) = &pending.review {
                    ui.label(cam_dialect_name(&review.dialect));
                    ui.label(format!("{}: {}", review.units, review.work_offset));
                    ui.label(format!(
                        "T{} · S{} · F{} mm/min",
                        review.tool_number, review.spindle_rpm, review.cutting_feed_mm_per_min
                    ));
                    ui.label(format!(
                        "{}: {:.6} mm³ · {}: {:.6} mm³",
                        self.catalog.text("cam-review-removed"),
                        review.removed_stock_mm3,
                        self.catalog.text("cam-review-gouge"),
                        review.gouge_mm3
                    ));
                    ui.monospace(format!("SHA-256 {}", review.content_digest));
                } else {
                    ui.label(self.catalog.text("cam-review-not-previewed"));
                }
                if !idle {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(self.catalog.text("cam-review-running"));
                    });
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            idle,
                            egui::Button::new(self.catalog.text("cam-review-preview")),
                        )
                        .clicked()
                    {
                        preview = true;
                    }
                    if ui
                        .add_enabled(
                            idle && pending.review.is_some(),
                            egui::Button::new(self.catalog.text("cam-review-confirm-export")),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button(self.catalog.text("dialog-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if preview && idle {
            let started = (|| {
                let snapshot = self.document.current();
                let mutation_epoch = self.document.mutation_epoch();
                let plan = snapshot
                    .cam_plan(pending.plan_id)
                    .ok_or_else(|| self.catalog.refusal("cam-review-no-plan"))?;
                let operation = self.cam_window_operation(&pending, plan, &snapshot)?;
                let dialect = CamPostprocessorDialect::library(&pending.dialect_id)
                    .map_err(|error| failed("cam.dialect", error))?;
                let worker_path = self.exact_worker_executable()?;
                self.reviews
                    .cam_reviews
                    .set_worker_path(worker_path.clone());
                let request = CamReviewRequest {
                    plan_id: pending.plan_id,
                    operations: vec![operation],
                    fixtures: Vec::new(),
                    dialect,
                };
                Ok::<_, Rejection>(CamRun::start(
                    worker_path,
                    snapshot,
                    mutation_epoch,
                    request,
                    None,
                    context,
                ))
            })();
            pending.review = None;
            match started {
                Ok(run) => pending.running = Some(run),
                Err(error) => self.digest = error.reason_text().to_owned(),
            }
        }
        // The program is planned and simulated again, and must come out as reviewed.
        if confirm
            && idle
            && let Some(review) = &pending.review
            && let Some(path) = self.choose_export_path("nc")
        {
            let snapshot = self.document.current();
            let mutation_epoch = self.document.mutation_epoch();
            let token = review.token.clone();
            let request =
                self.reviews
                    .cam_reviews
                    .reviewed_request(&snapshot, mutation_epoch, &token, true);
            match (request, self.reviews.cam_reviews.worker_path()) {
                (Ok(request), Some(worker_path)) => {
                    pending.running = Some(CamRun::start(
                        worker_path.to_path_buf(),
                        snapshot,
                        mutation_epoch,
                        request,
                        Some((token, path)),
                        context,
                    ));
                }
                (Ok(_), None) => self.digest = CamReviewError::WorkerUnavailable.to_string(),
                (Err(error), _) => self.digest = error.to_string(),
            }
        }
        if !open || cancel {
            // Dropping the run stops its exact worker.
            if pending.running.take().is_some() {
                self.digest = self.catalog.text("digest-cancelled");
            }
            return;
        }
        self.reviews.cam_export_dialog = Some(pending);
    }

    /// Takes the running review's result when it has come: a preview is shown, an export
    /// written. A run of a document state the user has left by an edit is dropped, so no
    /// numbers of the model before it are shown. Whether the dialog stays open.
    fn poll_cam_run(&mut self, pending: &mut CamExportDialog) -> bool {
        let Some(run) = &pending.running else {
            return true;
        };
        let current = self.document.current();
        if run.snapshot.document_id() != current.document_id()
            || run.snapshot.revision_id() != current.revision_id()
            || run.mutation_epoch != self.document.mutation_epoch()
        {
            // Dropping the run stops its exact worker.
            pending.running = None;
            pending.review = None;
            self.digest = self.catalog.text("cam-review-stale");
            return true;
        }
        let computed = match run.receiver.try_recv() {
            Ok(computed) => computed,
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => Err(CamReviewError::Simulation(
                "the CAM review ended unexpectedly".to_owned(),
            )),
        };
        let Some(run) = pending.running.take() else {
            return true;
        };
        match (computed, &run.export) {
            (Ok(computed), None) => {
                match self
                    .reviews
                    .cam_reviews
                    .admit(&run.snapshot, run.mutation_epoch, computed)
                {
                    Ok(review) => {
                        pending.review = Some(review);
                        self.digest = self.catalog.text("cam-review-ready");
                    }
                    Err(error) => {
                        self.digest = failed("cam.review", error).reason_text().to_owned()
                    }
                }
                true
            }
            (Ok(computed), Some((token, path))) => {
                match self.reviews.cam_reviews.export_computed(
                    &run.snapshot,
                    run.mutation_epoch,
                    token,
                    path,
                    None,
                    &computed,
                ) {
                    Ok(_) => {
                        self.digest = self.catalog.text("cam-review-exported");
                        false
                    }
                    Err(error) => {
                        self.digest = error.to_string();
                        true
                    }
                }
            }
            (Err(error), None) => {
                self.digest = failed("cam.review", error).reason_text().to_owned();
                true
            }
            (Err(error), Some(_)) => {
                self.digest = error.to_string();
                true
            }
        }
    }
}
