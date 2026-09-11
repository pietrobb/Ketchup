use super::{ActiveTool, KetchupApp};
use eframe::egui;
pub use ketchup_core::assistant_sidecar::{
    AssistantAxisSpec as AxisSpec, AssistantHelixHandedness as HelixHandedness,
    AssistantHelixParameters as HelixToolParameters,
    AssistantThreadParameters as ThreadToolParameters, AssistantThreadProfile as ThreadProfile,
};
use ketchup_core::assistant_sidecar::{AssistantCadEditOperation, AssistantCadEditProgram};
use ketchup_core::document::SpatialPathSegment;
use ketchup_interaction::Vec3;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) struct HelixThreadUiState {
    origin: [String; 3],
    axis: [String; 3],
    radius: String,
    pitch: String,
    turns: String,
    start_angle: String,
    handedness: HelixHandedness,
    profile_radius: String,
    profile: ThreadProfile,
    preview_segments: Vec<SpatialPathSegment>,
    valid: bool,
}

impl Default for HelixThreadUiState {
    fn default() -> Self {
        let mut state = Self {
            origin: ["0".into(), "0".into(), "0".into()],
            axis: ["0".into(), "0".into(), "1".into()],
            radius: "10".into(),
            pitch: "5".into(),
            turns: "3".into(),
            start_angle: "0".into(),
            handedness: HelixHandedness::Right,
            profile_radius: "0.8".into(),
            profile: ThreadProfile::Round,
            preview_segments: Vec::new(),
            valid: false,
        };
        state.refresh(false);
        state
    }
}

impl HelixThreadUiState {
    fn parse_vector(values: &[String; 3]) -> Option<[f64; 3]> {
        Some([
            values[0].trim().parse().ok()?,
            values[1].trim().parse().ok()?,
            values[2].trim().parse().ok()?,
        ])
    }

    fn helix_parameters(&self) -> Option<HelixToolParameters> {
        Some(HelixToolParameters {
            axis: AxisSpec::OriginDirection {
                origin_mm: Self::parse_vector(&self.origin)?,
                direction: Self::parse_vector(&self.axis)?,
            },
            radius_mm: self.radius.trim().parse().ok()?,
            pitch_mm: self.pitch.trim().parse().ok()?,
            turns: self.turns.trim().parse().ok()?,
            start_angle_degrees: self.start_angle.trim().parse().ok()?,
            handedness: self.handedness,
        })
    }

    fn thread_parameters(&self) -> Option<ThreadToolParameters> {
        Some(ThreadToolParameters {
            helix: self.helix_parameters()?,
            profile_radius_mm: self.profile_radius.trim().parse().ok()?,
            profile: self.profile,
        })
    }

    fn refresh(&mut self, thread: bool) {
        let parameters = self.helix_parameters();
        self.preview_segments = parameters
            .and_then(|parameters| helix_segments(&parameters).ok())
            .unwrap_or_default();
        self.valid = !self.preview_segments.is_empty()
            && (!thread
                || self
                    .thread_parameters()
                    .is_some_and(|parameters| parameters.profile_segments().is_ok()));
    }
}

pub fn helix_segments(parameters: &HelixToolParameters) -> Result<Vec<SpatialPathSegment>, String> {
    parameters.spatial_path_segments()
}

impl KetchupApp {
    pub(super) fn begin_helix_thread_tool(&mut self, tool: ActiveTool) {
        self.helix_thread = HelixThreadUiState::default();
        self.helix_thread.refresh(tool == ActiveTool::Thread);
        self.status_key = if tool == ActiveTool::Helix {
            "status-helix-preview"
        } else {
            "status-thread-preview"
        };
    }

    pub(super) fn clear_helix_thread_preview(&mut self) {
        self.helix_thread.preview_segments.clear();
        self.helix_thread.valid = false;
    }

    pub(super) fn helix_thread_preview_points(&self) -> Vec<Vec3> {
        self.helix_thread
            .preview_segments
            .iter()
            .flat_map(|segment| match segment {
                SpatialPathSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => (0..=8)
                    .map(|step| {
                        let t = step as f64 / 8.0;
                        let one = 1.0 - t;
                        Vec3::new(
                            one.powi(3) * start_mm[0]
                                + 3.0 * one.powi(2) * t * control_1_mm[0]
                                + 3.0 * one * t.powi(2) * control_2_mm[0]
                                + t.powi(3) * end_mm[0],
                            one.powi(3) * start_mm[1]
                                + 3.0 * one.powi(2) * t * control_1_mm[1]
                                + 3.0 * one * t.powi(2) * control_2_mm[1]
                                + t.powi(3) * end_mm[1],
                            one.powi(3) * start_mm[2]
                                + 3.0 * one.powi(2) * t * control_1_mm[2]
                                + 3.0 * one * t.powi(2) * control_2_mm[2]
                                + t.powi(3) * end_mm[2],
                        )
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    pub fn create_helix(&mut self, parameters: HelixToolParameters) -> bool {
        let Some(number) = self
            .document
            .current()
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let name = self.catalog.format(
            "model-helix-definition",
            &BTreeMap::from([("number", number.to_string())]),
        );
        let program = AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateHelix { name, parameters }],
        };
        let batch = match self.plan_assistant_cad_edit_program(&program) {
            Ok(batch) => batch,
            Err(error) => {
                self.digest = format!("Helix refused: {}", error.failed_invariant);
                return false;
            }
        };
        if let Err(error) = self.document.apply_batch(&batch) {
            self.digest = format!("Helix refused: {error}");
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text("digest-helix-committed");
        true
    }

    pub fn create_thread(&mut self, parameters: ThreadToolParameters) -> bool {
        let Some(number) = self
            .document
            .current()
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let name = self.catalog.format(
            "model-thread-definition",
            &BTreeMap::from([("number", number.to_string())]),
        );
        let program = AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateThread { name, parameters }],
        };
        let batch = match self.plan_assistant_cad_edit_program(&program) {
            Ok(batch) => batch,
            Err(error) => {
                self.digest = format!("Thread refused: {}", error.failed_invariant);
                return false;
            }
        };
        if let Err(error) = self.document.apply_batch(&batch) {
            self.digest = format!("Thread refused: {error}");
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text("digest-thread-committed");
        true
    }

    pub(super) fn show_helix_thread_tool(&mut self, ui: &mut egui::Ui) {
        if !matches!(self.active_tool, ActiveTool::Helix | ActiveTool::Thread) {
            return;
        }
        let thread = self.active_tool == ActiveTool::Thread;
        ui.heading(self.catalog.text(if thread {
            "thread-panel-title"
        } else {
            "helix-panel-title"
        }));
        ui.small(self.catalog.text("helix-panel-help"));
        ui.separator();

        let mut changed = false;
        changed |= vector_row(
            ui,
            self.catalog.text("helix-origin"),
            &mut self.helix_thread.origin,
        );
        changed |= vector_row(
            ui,
            self.catalog.text("helix-axis"),
            &mut self.helix_thread.axis,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-radius"),
            &mut self.helix_thread.radius,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-pitch"),
            &mut self.helix_thread.pitch,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-turns"),
            &mut self.helix_thread.turns,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-start-angle"),
            &mut self.helix_thread.start_angle,
        );
        ui.label(self.catalog.text("helix-handedness"));
        ui.horizontal(|ui| {
            changed |= ui
                .selectable_value(
                    &mut self.helix_thread.handedness,
                    HelixHandedness::Right,
                    self.catalog.text("helix-right-handed"),
                )
                .changed();
            changed |= ui
                .selectable_value(
                    &mut self.helix_thread.handedness,
                    HelixHandedness::Left,
                    self.catalog.text("helix-left-handed"),
                )
                .changed();
        });
        if thread {
            changed |= scalar_row(
                ui,
                self.catalog.text("thread-profile-radius"),
                &mut self.helix_thread.profile_radius,
            );
            ui.label(self.catalog.text("thread-profile"));
            egui::ComboBox::from_id_salt("thread-profile")
                .selected_text(self.catalog.text(match self.helix_thread.profile {
                    ThreadProfile::Round => "thread-profile-round",
                    ThreadProfile::V => "thread-profile-v",
                    ThreadProfile::Trapezoid => "thread-profile-trapezoid",
                }))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.helix_thread.profile,
                            ThreadProfile::Round,
                            self.catalog.text("thread-profile-round"),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.helix_thread.profile,
                            ThreadProfile::V,
                            self.catalog.text("thread-profile-v"),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.helix_thread.profile,
                            ThreadProfile::Trapezoid,
                            self.catalog.text("thread-profile-trapezoid"),
                        )
                        .changed();
                });
        }
        if changed {
            self.helix_thread.refresh(thread);
            self.digest = self.catalog.text(if self.helix_thread.valid {
                if thread {
                    "digest-thread-live"
                } else {
                    "digest-helix-live"
                }
            } else if thread {
                "digest-thread-invalid"
            } else {
                "digest-helix-invalid"
            });
        }
        ui.separator();
        let create = ui
            .add_enabled(
                self.helix_thread.valid,
                egui::Button::new(self.catalog.text(if thread {
                    "action-create-thread"
                } else {
                    "action-create-helix"
                })),
            )
            .clicked();
        if ui.button(self.catalog.text("action-cancel")).clicked() {
            self.clear_ephemeral_edit_state();
            self.active_tool = ActiveTool::Select;
            self.status_key = "status-ready";
        } else if create {
            if thread {
                if let Some(parameters) = self.helix_thread.thread_parameters() {
                    self.create_thread(parameters);
                }
            } else if let Some(parameters) = self.helix_thread.helix_parameters() {
                self.create_helix(parameters);
            }
        }
    }
}

fn vector_row(ui: &mut egui::Ui, label: String, values: &mut [String; 3]) -> bool {
    ui.label(label);
    let mut changed = false;
    ui.horizontal(|ui| {
        for (axis, value) in ["X", "Y", "Z"].into_iter().zip(values.iter_mut()) {
            ui.label(axis);
            changed |= ui
                .add(egui::TextEdit::singleline(value).desired_width(52.0))
                .changed();
        }
    });
    changed
}

fn scalar_row(ui: &mut egui::Ui, label: String, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(84.0))
            .changed()
    })
    .inner
}

#[cfg(test)]
fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter().zip(right).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
fn unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    if vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let length = dot(vector, vector).sqrt();
    (length > 1.0e-9).then(|| scale(vector, length.recip()))
}

#[cfg(test)]
fn sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] - right[axis])
}

#[cfg(test)]
fn scale(vector: [f64; 3], factor: f64) -> [f64; 3] {
    vector.map(|value| value * factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_axis_helix_is_connected_and_reaches_requested_rise() {
        let parameters = HelixToolParameters {
            axis: AxisSpec::OriginDirection {
                origin_mm: [4.0, -3.0, 2.0],
                direction: [1.0, 2.0, 3.0],
            },
            radius_mm: 7.0,
            pitch_mm: 4.5,
            turns: 2.25,
            start_angle_degrees: 37.0,
            handedness: HelixHandedness::Left,
        };
        let segments = helix_segments(&parameters).unwrap();
        assert_eq!(segments.len(), 9);
        for pair in segments.windows(2) {
            assert_eq!(pair[0].end_mm(), pair[1].start_mm());
        }
        let (_, direction) = parameters.axis.origin_and_direction().unwrap();
        let axis = unit(direction).unwrap();
        let start = segments.first().unwrap().start_mm();
        let end = segments.last().unwrap().end_mm();
        let rise = dot(sub(end, start), axis);
        assert!((rise - parameters.pitch_mm * parameters.turns).abs() < 1.0e-8);
    }

    #[test]
    fn invalid_or_unbounded_helix_is_refused() {
        assert!(
            helix_segments(&HelixToolParameters {
                axis: AxisSpec::OriginDirection {
                    origin_mm: [0.0; 3],
                    direction: [0.0; 3],
                },
                ..HelixToolParameters::default()
            })
            .is_err()
        );
        assert!(
            helix_segments(&HelixToolParameters {
                turns: 17.0,
                ..HelixToolParameters::default()
            })
            .is_err()
        );
    }
}
