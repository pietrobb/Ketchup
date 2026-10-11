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
                let parse_f64 = |value: &str, name: &str| {
                    value
                        .parse::<f64>()
                        .map_err(|error| invalid_field(name, "a finite number", error))
                };
                let youngs_modulus_mpa = parse_f64(&pending.youngs_modulus_mpa, "Young's modulus")?;
                let poisson_ratio = parse_f64(&pending.poisson_ratio, "Poisson ratio")?;
                let yield_strength_mpa = parse_f64(&pending.yield_strength_mpa, "Yield strength")?;
                let constrained_face_ordinals = pending
                    .constrained_face_ordinals
                    .split(',')
                    .map(str::trim)
                    .map(|value| {
                        value.parse::<u32>().map_err(|error| {
                            invalid_field(
                                "Fixed face ordinals",
                                "comma-separated u32 values",
                                error,
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let loaded_face_ordinal = pending
                    .loaded_face_ordinal
                    .parse::<u32>()
                    .map_err(|error| invalid_field("Loaded face ordinal", "u32", error))?;
                let traction = [
                    parse_f64(&pending.traction_x_n_per_mm2, "Traction X")?,
                    parse_f64(&pending.traction_y_n_per_mm2, "Traction Y")?,
                    parse_f64(&pending.traction_z_n_per_mm2, "Traction Z")?,
                ];
                let coarse = parse_f64(&pending.coarse_deflection_mm, "Coarse deflection")?;
                let fine = parse_f64(&pending.fine_deflection_mm, "Fine deflection")?;
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
