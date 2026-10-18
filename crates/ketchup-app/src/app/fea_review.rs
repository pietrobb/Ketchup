//! File > Review static FEA: the setup of one exact body's static study and its
//! convergence across two mesh levels.

use crate::*;

impl KetchupApp {
    pub(crate) fn show_fea_review_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.fea_review_dialog.take() else {
            return;
        };
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
                ui.horizontal(|ui| {
                    if ui
                        .button(self.catalog.text("fea-review-run-confirmed"))
                        .clicked()
                    {
                        run = true;
                    }
                    if ui.button(self.catalog.text("dialog-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if run {
            let result = (|| {
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
                let worker_path = self.exact_worker_executable()?;
                self.reviews.fea_reviews.set_worker_path(worker_path);
                self.reviews
                    .fea_reviews
                    .review(
                        &self.document.current(),
                        &FeaStudyRequest {
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
                        },
                        true,
                        &AtomicBool::new(false),
                    )
                    .map_err(|error| failed("fea.review", error))
            })();
            match result {
                Ok(review) => {
                    pending.review = Some(review);
                    self.digest = self.catalog.text("fea-review-ready");
                }
                Err(error) => self.digest = error.reason_text().to_owned(),
            }
        }
        if open && !cancel {
            self.reviews.fea_review_dialog = Some(pending);
        }
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
            value
                .trim()
                .parse::<u32>()
                .map_err(|error| catalog.refusal_because(label, error))
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
}
