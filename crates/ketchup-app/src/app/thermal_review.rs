//! Tools > Analysis > Steady heat conduction: one exact body's faces held at
//! temperatures, heated by a flux or cooled by convection, solved on two mesh
//! levels. The conductivity is the body's material's in the library rules,
//! shown with its source. The review runs on a thread, so the window keeps
//! painting and Cancel stops the exact worker.

use crate::*;
use ketchup_analysis::fea_thermal::ThermalSolveSettings;
use ketchup_application::thermal_workflow::{
    ExactThermalFaceConvection, ExactThermalFaceFlux, ExactThermalFaceTemperature,
    ExactVolumeMeshWireOptions, ThermalReviewError, ThermalReviewSummary, ThermalReviewWorkflow,
    ThermalStudyRequest,
};
use ketchup_application::validation_rules::ValidationRules;

pub(crate) struct ThermalReviewDialog {
    /// The document the dialog's ids are of; another closes it.
    pub(crate) document_id: DocumentId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) feature_id: FeatureId,
    pub(crate) occurrence_id: OccurrenceId,
    pub(crate) case_id: String,
    pub(crate) hot_faces: String,
    pub(crate) hot_temperature_c: String,
    pub(crate) cold_faces: String,
    pub(crate) cold_temperature_c: String,
    pub(crate) flux_faces: String,
    pub(crate) flux_w_per_m2: String,
    pub(crate) convection_faces: String,
    pub(crate) film_coefficient_w_per_m2_k: String,
    pub(crate) ambient_c: String,
    pub(crate) coarse_deflection_mm: String,
    pub(crate) fine_deflection_mm: String,
    pub(crate) review: Option<ThermalReviewSummary>,
    /// The review running now, if any.
    pub(crate) running: Option<ThermalRun>,
}

impl ThermalReviewDialog {
    /// The dialog for the one selected root occurrence's one exact body.
    pub(crate) fn for_selection(app: &KetchupApp) -> Result<Self, Rejection> {
        let selected = app
            .selected_root_occurrence_ids()
            .map_err(|error| app.root_occurrence_selection_error(&error))?;
        let [occurrence_id] = selected.iter().copied().collect::<Vec<_>>()[..] else {
            return Err(app.catalog.refusal("fea-review-select-one"));
        };
        let snapshot = app.document.current();
        let occurrence = snapshot
            .occurrence(occurrence_id)
            .ok_or_else(|| app.catalog.refusal("fea-review-select-one"))?;
        let terminals = exact_body_terminal_features(&snapshot, occurrence.definition_id())
            .map_err(|error| failed("thermal_review.terminal_features", error))?;
        let [feature_id] = terminals.values().copied().collect::<Vec<_>>()[..] else {
            return Err(app.catalog.refusal("fea-review-one-body"));
        };
        Ok(Self {
            document_id: snapshot.document_id(),
            definition_id: occurrence.definition_id(),
            feature_id,
            occurrence_id,
            case_id: "steady-conduction-case".to_owned(),
            hot_faces: "1".to_owned(),
            hot_temperature_c: "100".to_owned(),
            cold_faces: "0".to_owned(),
            cold_temperature_c: "20".to_owned(),
            flux_faces: String::new(),
            flux_w_per_m2: "0".to_owned(),
            convection_faces: String::new(),
            film_coefficient_w_per_m2_k: "10".to_owned(),
            ambient_c: "20".to_owned(),
            coarse_deflection_mm: "0.3".to_owned(),
            fine_deflection_mm: "0.12".to_owned(),
            review: None,
            running: None,
        })
    }
}

impl KetchupApp {
    pub(crate) fn show_thermal_review_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.thermal_review_dialog.take() else {
            return;
        };
        if !self.poll_thermal_run(&mut pending) {
            return;
        }
        let mut open = true;
        let mut run = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("thermal-review-title"))
            .id(egui::Id::new("steady-thermal-review"))
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
                    ("thermal-review-hot-faces", &mut pending.hot_faces),
                    ("thermal-review-hot-temperature", &mut pending.hot_temperature_c),
                    ("thermal-review-cold-faces", &mut pending.cold_faces),
                    (
                        "thermal-review-cold-temperature",
                        &mut pending.cold_temperature_c,
                    ),
                    ("thermal-review-flux-faces", &mut pending.flux_faces),
                    ("thermal-review-flux", &mut pending.flux_w_per_m2),
                    (
                        "thermal-review-convection-faces",
                        &mut pending.convection_faces,
                    ),
                    (
                        "thermal-review-film",
                        &mut pending.film_coefficient_w_per_m2_k,
                    ),
                    ("thermal-review-ambient", &mut pending.ambient_c),
                    ("fea-review-coarse", &mut pending.coarse_deflection_mm),
                    ("fea-review-fine", &mut pending.fine_deflection_mm),
                ] {
                    ui.horizontal(|ui| {
                        let name = ui.label(self.catalog.text(label));
                        ui.text_edit_singleline(value).labelled_by(name.id);
                    });
                    show_value_refusal(ui, &self.catalog, value, self.window.decimal);
                }
                ui.label(self.catalog.text("thermal-review-limits"));
                if let Some(review) = &pending.review {
                    ui.separator();
                    ui.label(format!(
                        "{}: {} · {} W/(m·K) · {}",
                        self.catalog.text("thermal-review-material"),
                        review.material.material,
                        review.material.conductivity_w_per_m_k,
                        review.material.citation
                    ));
                    ui.label(format!(
                        "{}: {}",
                        self.catalog.text("fea-review-converged"),
                        review.converged
                    ));
                    for (index, level) in review.levels.iter().enumerate() {
                        ui.label(format!(
                            "L{}: {} nodes · {} tetra · T {:.3}…{:.3} °C · {:.6} W in · balance {:.3e} W",
                            index + 1,
                            level.node_count,
                            level.element_count,
                            level.minimum_temperature_c,
                            level.maximum_temperature_c,
                            level.heat_into_held_faces_w
                                + level.flux_heat_in_w.max(0.0)
                                + level.convection_heat_in_w.max(0.0),
                            level.heat_balance_w
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
                        egui::Button::new(self.catalog.text("thermal-review-run-confirmed"));
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
            match self.thermal_study(&pending) {
                Ok(study) => pending.running = Some(ThermalRun::start(study, context)),
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
        self.reviews.thermal_review_dialog = Some(pending);
    }

    /// The study as typed in the dialog, on the current document.
    fn thermal_study(&mut self, pending: &ThermalReviewDialog) -> Result<ThermalStudy, Rejection> {
        let fields = pending.read_fields(&self.catalog, self.window.decimal)?;
        let level = |deflection: f64, max_tetrahedra: u32, volume_error: f64| {
            ExactVolumeMeshWireOptions {
                surface_deflection_mm: deflection,
                angular_deflection_rad: deflection,
                max_tetrahedra,
                max_relative_volume_error: volume_error,
                min_tetrahedron_quality: 1.0e-6, // not a tolerance: mesh quality
            }
        };
        let held_faces = [
            (&fields.hot_faces, fields.hot_temperature_c),
            (&fields.cold_faces, fields.cold_temperature_c),
        ]
        .into_iter()
        .flat_map(|(faces, temperature_c)| {
            faces
                .iter()
                .map(move |&face_ordinal| ExactThermalFaceTemperature {
                    face_ordinal,
                    temperature_c,
                })
        })
        .collect();
        let request = ThermalStudyRequest {
            definition_id: pending.definition_id,
            feature_id: pending.feature_id,
            case_id: pending.case_id.clone(),
            instance_path: InstancePath::root(pending.occurrence_id),
            held_faces,
            flux_faces: fields
                .flux_faces
                .iter()
                .map(|&face_ordinal| ExactThermalFaceFlux {
                    face_ordinal,
                    flux_w_per_m2: fields.flux_w_per_m2,
                })
                .collect(),
            convection_faces: fields
                .convection_faces
                .iter()
                .map(|&face_ordinal| ExactThermalFaceConvection {
                    face_ordinal,
                    film_coefficient_w_per_m2_k: fields.film_coefficient_w_per_m2_k,
                    ambient_c: fields.ambient_c,
                })
                .collect(),
            mesh_levels: vec![
                level(fields.coarse, 512, 0.05),
                level(fields.fine, 1_024, 0.02),
            ],
            solve_settings: ThermalSolveSettings::default(),
            convergence_relative_tolerance: 0.1,
        };
        Ok(ThermalStudy {
            workflow: ThermalReviewWorkflow::new(Some(self.exact_worker_executable()?)),
            snapshot: self.document.current(),
            request,
        })
    }

    /// Takes the run's result when it has come. A run or a result of a
    /// revision the document has left is dropped, and the dialog closes when
    /// the document is another. Whether the dialog stays open.
    fn poll_thermal_run(&mut self, pending: &mut ThermalReviewDialog) -> bool {
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
            pending.running = None;
            self.digest = self.catalog.text("fea-review-stale");
            return true;
        }
        let result = match run.receiver.try_recv() {
            Ok(result) => result.map_err(|error| failed("thermal.review", error)),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => Err(failed(
                "thermal.review",
                std::io::Error::other("the thermal review ended unexpectedly"),
            )),
        };
        pending.running = None;
        match result {
            Ok(review) if stale((review.document_id, review.revision)) => {
                self.digest = self.catalog.text("fea-review-stale");
            }
            Ok(review) => {
                pending.review = Some(review);
                self.digest = self.catalog.text("thermal-review-ready");
            }
            Err(error) => self.digest = error.reason_text().to_owned(),
        }
        true
    }
}

/// What one thermal review needs, owned so it runs on a thread.
struct ThermalStudy {
    workflow: ThermalReviewWorkflow,
    snapshot: Snapshot,
    request: ThermalStudyRequest,
}

/// A thermal review meshing and solving on a thread; Cancel, closing the
/// dialog or leaving the document drops it, which stops its exact worker.
pub(crate) struct ThermalRun {
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<Result<ThermalReviewSummary, ThermalReviewError>>,
    /// The document id and revision it reviews.
    basis: (u64, u64),
}

impl ThermalRun {
    fn start(study: ThermalStudy, context: &egui::Context) -> Self {
        let basis = (study.snapshot.document_id().0, study.snapshot.revision_id());
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (thread_cancelled, repaint) = (Arc::clone(&cancelled), context.clone());
        std::thread::spawn(move || {
            let ThermalStudy {
                workflow,
                snapshot,
                request,
            } = study;
            let review = workflow.review(
                &snapshot,
                &request,
                ValidationRules::library(),
                true,
                &thread_cancelled,
            );
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

impl Drop for ThermalRun {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// The study's numbers as typed in the dialog; an empty face list is none.
struct ThermalFields {
    hot_faces: Vec<u32>,
    hot_temperature_c: f64,
    cold_faces: Vec<u32>,
    cold_temperature_c: f64,
    flux_faces: Vec<u32>,
    flux_w_per_m2: f64,
    convection_faces: Vec<u32>,
    film_coefficient_w_per_m2_k: f64,
    ambient_c: f64,
    coarse: f64,
    fine: f64,
}

impl ThermalReviewDialog {
    /// Each field read as the window reads values; a refusal names the
    /// field by its label, both in the user's language.
    fn read_fields(
        &self,
        catalog: &LocaleCatalog,
        decimal: crate::length_input::DecimalSeparator,
    ) -> Result<ThermalFields, Rejection> {
        let number = |value: &str, label: &'static str| {
            crate::length_input::read_number(value, decimal)
                .map_err(|refusal| catalog.field_refusal(label, refusal))
        };
        let faces = |value: &str, label: &'static str| {
            value
                .split(crate::length_input::VALUE_SEPARATORS)
                .filter(|item| !item.trim().is_empty())
                .map(|item| {
                    crate::length_input::read_whole::<u32>(item, decimal)
                        .map_err(|refusal| catalog.field_refusal(label, refusal))
                })
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(ThermalFields {
            hot_faces: faces(&self.hot_faces, "thermal-review-hot-faces")?,
            hot_temperature_c: number(&self.hot_temperature_c, "thermal-review-hot-temperature")?,
            cold_faces: faces(&self.cold_faces, "thermal-review-cold-faces")?,
            cold_temperature_c: number(
                &self.cold_temperature_c,
                "thermal-review-cold-temperature",
            )?,
            flux_faces: faces(&self.flux_faces, "thermal-review-flux-faces")?,
            flux_w_per_m2: number(&self.flux_w_per_m2, "thermal-review-flux")?,
            convection_faces: faces(&self.convection_faces, "thermal-review-convection-faces")?,
            film_coefficient_w_per_m2_k: number(
                &self.film_coefficient_w_per_m2_k,
                "thermal-review-film",
            )?,
            ambient_c: number(&self.ambient_c, "thermal-review-ambient")?,
            coarse: number(&self.coarse_deflection_mm, "fea-review-coarse")?,
            fine: number(&self.fine_deflection_mm, "fea-review-fine")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::length_input::DecimalSeparator::{Comma, Point};

    fn dialog(hot_temperature_c: &str) -> ThermalReviewDialog {
        ThermalReviewDialog {
            document_id: DocumentId(1),
            definition_id: DefinitionId(1),
            feature_id: FeatureId(1),
            occurrence_id: OccurrenceId(1),
            case_id: "case".to_owned(),
            hot_faces: "1; 3".to_owned(),
            hot_temperature_c: hot_temperature_c.to_owned(),
            cold_faces: "0".to_owned(),
            cold_temperature_c: "20".to_owned(),
            flux_faces: " ".to_owned(),
            flux_w_per_m2: "0".to_owned(),
            convection_faces: String::new(),
            film_coefficient_w_per_m2_k: "10".to_owned(),
            ambient_c: "20".to_owned(),
            coarse_deflection_mm: "0.3".to_owned(),
            fine_deflection_mm: "0.12".to_owned(),
            review: None,
            running: None,
        }
    }

    /// A refused field says which one and why in the user's language; an
    /// empty face list is no faces, not a refusal.
    #[test]
    fn a_refused_thermal_field_is_named_and_an_empty_face_list_is_none() {
        let slovak = LocaleCatalog::slovak();
        let refusal = dialog("120,5")
            .read_fields(&slovak, Point)
            .err()
            .expect("a comma where the decimal separator is a point");
        let reason = refusal.reason_text();
        assert!(
            reason.starts_with(&slovak.text("thermal-review-hot-temperature")),
            "{reason}"
        );
        assert!(
            reason.contains(&slovak.text("value-comma-not-decimal")),
            "{reason}"
        );

        let fields = dialog("120,5").read_fields(&slovak, Comma).unwrap();
        assert_eq!(fields.hot_temperature_c, 120.5);
        assert_eq!(fields.hot_faces, [1, 3]);
        assert!(fields.flux_faces.is_empty() && fields.convection_faces.is_empty());
    }
}
