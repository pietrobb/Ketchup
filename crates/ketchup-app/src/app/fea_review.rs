//! File > Review static FEA: the setup of one exact body's static study and its
//! convergence across two mesh levels. The review runs on a thread, so the
//! window keeps painting and Cancel stops the exact worker (review 2026-11-04
//! MCP-101).

use crate::*;
use ketchup_application::fea_workflow::FeaReviewError;

pub(crate) struct FeaReviewDialog {
    /// The document the dialog's ids are of; another closes it (review
    /// 2026-11-04 r19 MCP-160).
    pub(crate) document_id: DocumentId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) feature_id: FeatureId,
    pub(crate) occurrence_id: OccurrenceId,
    pub(crate) case_id: String,
    pub(crate) youngs_modulus_mpa: String,
    pub(crate) poisson_ratio: String,
    pub(crate) yield_strength_mpa: String,
    pub(crate) constrained_face_ordinals: String,
    pub(crate) loaded_face_ordinal: String,
    pub(crate) traction_x_n_per_mm2: String,
    pub(crate) traction_y_n_per_mm2: String,
    pub(crate) traction_z_n_per_mm2: String,
    pub(crate) coarse_deflection_mm: String,
    pub(crate) fine_deflection_mm: String,
    pub(crate) review: Option<FeaReviewSummary>,
    /// The review running now, if any.
    pub(crate) running: Option<FeaRun>,
}

impl KetchupApp {
    pub(crate) fn show_fea_review_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.fea_review_dialog.take() else {
            return;
        };
        if !self.poll_fea_run(&mut pending) {
            return;
        }
        let mut open = true;
        let mut run = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("fea-review-title"))
            .id(egui::Id::new("static-fea-review"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(format!(
                    "Definition {} · Feature {} · Occurrence {}",
                    pending.definition_id.0, pending.feature_id.0, pending.occurrence_id.0
                ));
                ui.horizontal(|ui| {
                    ui.label(self.catalog.text("fea-review-case"));
                    ui.text_edit_singleline(&mut pending.case_id);
                });
                for (label, value) in [
                    ("fea-review-young", &mut pending.youngs_modulus_mpa),
                    ("fea-review-poisson", &mut pending.poisson_ratio),
                    ("fea-review-yield", &mut pending.yield_strength_mpa),
                    (
                        "fea-review-fixed-face",
                        &mut pending.constrained_face_ordinals,
                    ),
                    ("fea-review-loaded-face", &mut pending.loaded_face_ordinal),
                    ("fea-review-traction-x", &mut pending.traction_x_n_per_mm2),
                    ("fea-review-traction-y", &mut pending.traction_y_n_per_mm2),
                    ("fea-review-traction-z", &mut pending.traction_z_n_per_mm2),
                    ("fea-review-coarse", &mut pending.coarse_deflection_mm),
                    ("fea-review-fine", &mut pending.fine_deflection_mm),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(self.catalog.text(label));
                        ui.text_edit_singleline(value);
                    });
                    show_value_refusal(ui, &self.catalog, value, self.value_box.decimal);
                }
                ui.label(self.catalog.text("fea-review-limits"));
                if let Some(review) = &pending.review {
                    ui.separator();
                    ui.label(format!(
                        "{}: {} · {}: {}",
                        self.catalog.text("fea-review-converged"),
                        review.converged,
                        self.catalog.text("fea-review-within-limits"),
                        review.all_levels_within_declared_limits
                    ));
                    for (index, level) in review.levels.iter().enumerate() {
                        ui.label(format!(
                            "L{}: {} nodes · {} tetra · u={:.6} mm · σvm={:.6} MPa · qmin={:.6}",
                            index + 1,
                            level.node_count,
                            level.element_count,
                            level.maximum_displacement_mm,
                            level.maximum_von_mises_stress_mpa,
                            level.minimum_quality
                        ));
                    }
                    ui.monospace(format!("SHA-256 {}", review.review_digest));
                }
                if pending.running.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(self.catalog.text("fea-review-running"));
                    });
                }
                ui.horizontal(|ui| {
                    let run_button =
                        egui::Button::new(self.catalog.text("fea-review-run-confirmed"));
                    if ui
                        .add_enabled(pending.running.is_none(), run_button)
                        .clicked()
                    {
                        run = true;
                    }
                    if ui.button(self.catalog.text("dialog-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if run && pending.running.is_none() {
            match self.fea_study(&pending) {
                Ok(study) => pending.running = Some(FeaRun::start(study, context)),
                Err(error) => self.digest = error.reason_text().to_owned(),
            }
        }
        if !open || cancel {
            // Dropping the run stops its exact worker.
            if pending.running.take().is_some() {
                self.digest = self.catalog.text("digest-cancelled");
            }
            return;
        }
        self.reviews.fea_review_dialog = Some(pending);
    }

    /// The study as typed in the dialog, on the current document.
    fn fea_study(&mut self, pending: &FeaReviewDialog) -> Result<FeaStudy, Rejection> {
        let FeaFields {
            youngs_modulus_mpa,
            poisson_ratio,
            yield_strength_mpa,
            constrained_face_ordinals,
            loaded_face_ordinal,
            traction,
            coarse,
            fine,
        } = pending.read_fields(&self.catalog, self.value_box.decimal)?;
        let request = FeaStudyRequest {
            definition_id: pending.definition_id,
            feature_id: pending.feature_id,
            setup: ExactFeaSetup {
                case_id: pending.case_id.clone(),
                instance_path: InstancePath::root(pending.occurrence_id),
                material: FeaMaterial {
                    id: 1,
                    youngs_modulus_mpa,
                    poisson_ratio,
                    yield_strength_mpa: Some(yield_strength_mpa),
                },
                constrained_face_ordinals,
                face_tractions: vec![ExactFeaFaceTraction {
                    face_ordinal: loaded_face_ordinal,
                    traction_local_n_per_mm2: traction,
                }],
            },
            mesh_levels: vec![
                ExactVolumeMeshWireOptions {
                    surface_deflection_mm: coarse,
                    angular_deflection_rad: coarse,
                    max_tetrahedra: 512,
                    max_relative_volume_error: 0.05,
                    min_tetrahedron_quality: 1.0e-6, // not a tolerance: mesh quality
                },
                ExactVolumeMeshWireOptions {
                    surface_deflection_mm: fine,
                    angular_deflection_rad: fine,
                    max_tetrahedra: 1_024,
                    max_relative_volume_error: 0.02,
                    min_tetrahedron_quality: 1.0e-6, // not a tolerance: mesh quality
                },
            ],
            solve_settings: FeaSolveSettings::default(),
        };
        Ok(FeaStudy {
            workflow: FeaReviewWorkflow::new(Some(self.exact_worker_executable()?)),
            snapshot: self.document.current(),
            request,
        })
    }

    /// Takes the run's result when it has come; the dialog shows it. A run or
    /// a result of a revision the document has left is dropped, so no numbers
    /// of the geometry before an edit are shown, and the dialog closes when
    /// the document is another (review 2026-11-04 r19 MCP-160). Whether the
    /// dialog stays open.
    fn poll_fea_run(&mut self, pending: &mut FeaReviewDialog) -> bool {
        let snapshot = self.document.current();
        if pending.document_id != snapshot.document_id() {
            return false;
        }
        let current = (snapshot.document_id().0, snapshot.revision_id());
        let stale = |basis: (u64, u64)| basis != current;
        if pending
            .review
            .as_ref()
            .is_some_and(|review| stale((review.document_id, review.revision)))
        {
            pending.review = None;
            self.digest = self.catalog.text("fea-review-stale");
        }
        let Some(run) = &pending.running else {
            return true;
        };
        if stale(run.basis) {
            // Dropping the run stops its exact worker.
            pending.running = None;
            self.digest = self.catalog.text("fea-review-stale");
            return true;
        }
        let result = match run.receiver.try_recv() {
            Ok(result) => result.map_err(|error| failed("fea.review", error)),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => Err(failed(
                "fea.review",
                std::io::Error::other("the FEA review ended unexpectedly"),
            )),
        };
        pending.running = None;
        match result {
            Ok(review) if stale((review.document_id, review.revision)) => {
                self.digest = self.catalog.text("fea-review-stale");
            }
            Ok(review) => {
                pending.review = Some(review);
                self.digest = self.catalog.text("fea-review-ready");
            }
            Err(error) => self.digest = error.reason_text().to_owned(),
        }
        true
    }
}

/// What one FEA review needs, owned so it runs on a thread.
struct FeaStudy {
    workflow: FeaReviewWorkflow,
    snapshot: Snapshot,
    request: FeaStudyRequest,
}

/// An FEA review meshing and solving on a thread, so the window keeps
/// painting; Cancel, closing the dialog or leaving the document drops it,
/// which stops its exact worker (review 2026-11-04 MCP-101).
pub(crate) struct FeaRun {
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<Result<FeaReviewSummary, FeaReviewError>>,
    /// The document id and revision it reviews.
    basis: (u64, u64),
}

impl FeaRun {
    fn start(study: FeaStudy, context: &egui::Context) -> Self {
        let basis = (study.snapshot.document_id().0, study.snapshot.revision_id());
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (thread_cancelled, repaint) = (Arc::clone(&cancelled), context.clone());
        std::thread::spawn(move || {
            let FeaStudy {
                workflow,
                snapshot,
                request,
            } = study;
            let review = workflow.review(&snapshot, &request, true, &thread_cancelled);
            if sender.send(review).is_ok() {
                repaint.request_repaint();
            }
        });
        Self {
            cancelled,
            receiver,
            basis,
        }
    }
}

impl Drop for FeaRun {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// The study's numbers as typed in the dialog.
struct FeaFields {
    youngs_modulus_mpa: f64,
    poisson_ratio: f64,
    yield_strength_mpa: f64,
    constrained_face_ordinals: Vec<u32>,
    loaded_face_ordinal: u32,
    traction: [f64; 3],
    coarse: f64,
    fine: f64,
}

impl FeaReviewDialog {
    /// Each field read as the window reads values; a refusal names the
    /// field by its label, both in the user's language.
    fn read_fields(
        &self,
        catalog: &LocaleCatalog,
        decimal: crate::length_input::DecimalSeparator,
    ) -> Result<FeaFields, Rejection> {
        let number = |value: &str, label: &'static str| {
            crate::length_input::read_number(value, decimal)
                .map_err(|refusal| catalog.field_refusal(label, refusal))
        };
        let ordinal = |value: &str, label: &'static str| {
            crate::length_input::read_whole::<u32>(value, decimal)
                .map_err(|refusal| catalog.field_refusal(label, refusal))
        };
        Ok(FeaFields {
            youngs_modulus_mpa: number(&self.youngs_modulus_mpa, "fea-review-young")?,
            poisson_ratio: number(&self.poisson_ratio, "fea-review-poisson")?,
            yield_strength_mpa: number(&self.yield_strength_mpa, "fea-review-yield")?,
            constrained_face_ordinals: self
                .constrained_face_ordinals
                .split(crate::length_input::VALUE_SEPARATORS)
                .map(|value| ordinal(value, "fea-review-fixed-face"))
                .collect::<Result<_, _>>()?,
            loaded_face_ordinal: ordinal(&self.loaded_face_ordinal, "fea-review-loaded-face")?,
            traction: [
                number(&self.traction_x_n_per_mm2, "fea-review-traction-x")?,
                number(&self.traction_y_n_per_mm2, "fea-review-traction-y")?,
                number(&self.traction_z_n_per_mm2, "fea-review-traction-z")?,
            ],
            coarse: number(&self.coarse_deflection_mm, "fea-review-coarse")?,
            fine: number(&self.fine_deflection_mm, "fea-review-fine")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::length_input::DecimalSeparator::{Comma, Point};

    fn dialog(youngs_modulus_mpa: &str) -> FeaReviewDialog {
        FeaReviewDialog {
            document_id: DocumentId(1),
            definition_id: DefinitionId(1),
            feature_id: FeatureId(1),
            occurrence_id: OccurrenceId(1),
            case_id: "case".to_owned(),
            youngs_modulus_mpa: youngs_modulus_mpa.to_owned(),
            poisson_ratio: "0.3".to_owned(),
            yield_strength_mpa: "250".to_owned(),
            constrained_face_ordinals: "0; 2".to_owned(),
            loaded_face_ordinal: "1".to_owned(),
            traction_x_n_per_mm2: "0".to_owned(),
            traction_y_n_per_mm2: "0".to_owned(),
            traction_z_n_per_mm2: "-1".to_owned(),
            coarse_deflection_mm: "0.3".to_owned(),
            fine_deflection_mm: "0.12".to_owned(),
            review: None,
            running: None,
        }
    }

    /// Review 2026-10-17 P3-G: a refused FEA field says which one and why in
    /// the user's language, not in English inside the Slovak window.
    #[test]
    fn a_refused_fea_field_is_named_and_explained_in_the_users_language() {
        let slovak = LocaleCatalog::slovak();
        let refusal = dialog("2,1e5")
            .read_fields(&slovak, Point)
            .err()
            .expect("a comma where the decimal separator is a point");
        let reason = refusal.reason_text();
        assert!(
            reason.starts_with(&slovak.text("fea-review-young")),
            "{reason}"
        );
        assert!(
            reason.contains(&slovak.text("value-comma-not-decimal")),
            "{reason}"
        );
        let refusal = dialog("tvrdé").read_fields(&slovak, Comma).err().unwrap();
        assert!(
            refusal
                .reason_text()
                .ends_with(&slovak.text("value-unreadable"))
        );

        let fields = dialog("2,1e5").read_fields(&slovak, Comma).unwrap();
        assert_eq!(fields.youngs_modulus_mpa, 210_000.0);
        assert_eq!(fields.constrained_face_ordinals, [0, 2]);
    }

    /// A box program applied as source `size`, a new revision each time.
    fn apply_box(app: &mut KetchupApp, size: u32) {
        app.apply_program_source(
            ketchup_model::document::RuleProgramSource {
                variants: Default::default(),
                file_name: "fea.star".into(),
                source: format!("box(\"b\", ({size}, 20, 30))\n"),
                overrides: BTreeMap::new(),
            },
            true,
        )
        .unwrap();
    }

    /// A run of the document's current revision whose result `send` delivers.
    fn run_now(
        app: &KetchupApp,
    ) -> (
        FeaRun,
        mpsc::Sender<Result<FeaReviewSummary, FeaReviewError>>,
    ) {
        let snapshot = app.document.current();
        let (sender, receiver) = mpsc::channel();
        let run = FeaRun {
            cancelled: Arc::new(AtomicBool::new(false)),
            receiver,
            basis: (snapshot.document_id().0, snapshot.revision_id()),
        };
        (run, sender)
    }

    fn summary(app: &KetchupApp, revision: u64) -> FeaReviewSummary {
        FeaReviewSummary {
            review_digest: "digest".to_owned(),
            document_id: app.document.current().document_id().0,
            revision,
            canonical_digest: String::new(),
            graph_digest: String::new(),
            case_id: "case".to_owned(),
            units: "mm",
            solver_version: "test",
            levels: Vec::new(),
            final_displacement_relative_change: 0.0,
            final_energy_relative_change: 0.0,
            required_relative_tolerance: 0.01,
            converged: true,
            all_levels_within_declared_limits: true,
            excluded_physics: ["", "", "", ""],
        }
    }

    /// Review 2026-11-04 r19 MCP-160: the window paints while FEA runs, so the
    /// model may be edited meanwhile; the result was shown without comparing
    /// the revision, numbers for the geometry before the edit. Such a run,
    /// and a result shown before an edit, is dropped; another document closes
    /// the dialog, whose ids are of the one that went.
    #[test]
    fn an_fea_review_of_a_revision_the_document_left_is_dropped() {
        let mut app = KetchupApp::new();
        apply_box(&mut app, 10);
        let mut pending = dialog("200000");
        pending.document_id = app.document.current().document_id();

        // Unchanged meanwhile: the result is shown.
        let (run, sender) = run_now(&app);
        let revision = app.document.current().revision_id();
        pending.running = Some(run);
        sender.send(Ok(summary(&app, revision))).unwrap();
        assert!(app.poll_fea_run(&mut pending));
        assert!(pending.review.is_some(), "{}", app.digest);

        // Edited: the shown result goes.
        apply_box(&mut app, 11);
        assert!(app.poll_fea_run(&mut pending));
        assert!(
            pending.review.is_none(),
            "numbers of the box before the edit"
        );
        assert_eq!(app.digest, app.catalog.text("fea-review-stale"));

        // Edited while it ran: the run stops and its result is not shown.
        let (run, sender) = run_now(&app);
        let revision = app.document.current().revision_id();
        let cancelled = Arc::clone(&run.cancelled);
        pending.running = Some(run);
        apply_box(&mut app, 12);
        sender.send(Ok(summary(&app, revision))).unwrap();
        app.digest.clear();
        assert!(app.poll_fea_run(&mut pending));
        assert!(pending.running.is_none() && pending.review.is_none());
        assert!(cancelled.load(Ordering::Acquire), "its worker is stopped");
        assert_eq!(app.digest, app.catalog.text("fea-review-stale"));

        // Another document: the dialog closes.
        app.reviews.fea_review_dialog = Some(pending);
        app.reset_document_presentation();
        assert!(app.reviews.fea_review_dialog.is_none());
        let mut pending = dialog("200000");
        pending.document_id = DocumentId(app.document.current().document_id().0 + 1);
        assert!(!app.poll_fea_run(&mut pending), "ids of another document");
    }
}
