use super::{ActiveTool, KetchupApp};
use eframe::egui;
use ketchup_assistant::request_invalid::AssistantRequestInvalid;
pub use ketchup_assistant::sidecar::{
    AssistantAxisSpec as AxisSpec, AssistantHelixHandedness as HelixHandedness,
    AssistantHelixParameters as HelixToolParameters,
};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantSketchEntity,
};
use ketchup_geometry::linalg::CubicBezier;
#[cfg(test)]
use ketchup_geometry::linalg::{dot, sub};
use ketchup_interaction::{ElementId, Vec3};
use ketchup_model::document::SpatialPathSegment;
use ketchup_model::tolerance::ROUNDING;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
enum ConstructionKind {
    Point,
    Axis,
    Plane,
}

/// What the helix tool sweeps along the helix: nothing (a path), a tooth or a circle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelixProfile {
    None,
    Tooth,
    Circle,
}

impl HelixProfile {
    const ALL: [Self; 3] = [Self::None, Self::Tooth, Self::Circle];

    const fn label_key(self) -> &'static str {
        match self {
            Self::None => "helix-profile-none",
            Self::Tooth => "helix-profile-tooth",
            Self::Circle => "helix-profile-circle",
        }
    }
}

/// A closed polygon through `points` as sketch lines.
fn line_loop(points: &[[f64; 2]]) -> Vec<AssistantSketchEntity> {
    (0..points.len())
        .map(|index| AssistantSketchEntity::Line {
            id: index as u64 + 1,
            start_mm: points[index],
            end_mm: points[(index + 1) % points.len()],
        })
        .collect()
}

/// A tooth in the helix's axial section (u away from the axis, v along it):
/// base `width` wide on the path, `crest` wide at `depth` along u; a zero
/// crest closes it to a point. `None` unless `depth > 0` and `0 <= crest < width`.
fn tooth(depth: f64, width: f64, crest: f64) -> Option<Vec<AssistantSketchEntity>> {
    (depth.is_finite() && depth > 0.0 && crest.is_finite() && crest >= 0.0 && crest < width).then(
        || {
            if crest == 0.0 {
                line_loop(&[[0.0, -width / 2.0], [depth, 0.0], [0.0, width / 2.0]])
            } else {
                line_loop(&[
                    [0.0, -width / 2.0],
                    [depth, -crest / 2.0],
                    [depth, crest / 2.0],
                    [0.0, width / 2.0],
                ])
            }
        },
    )
}

#[derive(Clone, Debug)]
pub(super) struct HelixUiState {
    origin: [String; 3],
    axis: [String; 3],
    radius: String,
    pitch: String,
    turns: String,
    start_angle: String,
    handedness: HelixHandedness,
    profile: HelixProfile,
    tooth_depth: String,
    tooth_width: String,
    tooth_crest: String,
    circle_radius: String,
    preview_segments: Vec<SpatialPathSegment>,
    valid: bool,
}

impl Default for HelixUiState {
    fn default() -> Self {
        Self {
            origin: ["0".into(), "0".into(), "0".into()],
            axis: ["0".into(), "0".into(), "1".into()],
            radius: "10".into(),
            pitch: "5".into(),
            turns: "3".into(),
            start_angle: "0".into(),
            handedness: HelixHandedness::Right,
            profile: HelixProfile::None,
            tooth_depth: "2".into(),
            tooth_width: "4".into(),
            tooth_crest: "0".into(),
            circle_radius: "1".into(),
            preview_segments: Vec::new(),
            valid: false,
        }
    }
}

impl HelixUiState {
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

    /// The profile swept along the helix, empty for a bare path; `None` when its
    /// numbers do not describe one that fits between two turns (narrower than the pitch).
    fn profile_entities(&self) -> Option<Vec<AssistantSketchEntity>> {
        let number = |text: &String| text.trim().parse::<f64>().ok();
        let pitch = number(&self.pitch)?;
        let (entities, width) = match self.profile {
            HelixProfile::None => return Some(Vec::new()),
            HelixProfile::Tooth => {
                let width = number(&self.tooth_width)?;
                (
                    tooth(
                        number(&self.tooth_depth)?,
                        width,
                        number(&self.tooth_crest)?,
                    )?,
                    width,
                )
            }
            HelixProfile::Circle => {
                let radius = number(&self.circle_radius)?;
                (radius.is_finite() && radius > 0.0).then_some(())?;
                (
                    vec![AssistantSketchEntity::Circle {
                        id: 1,
                        center_mm: [0.0, 0.0],
                        radius_mm: radius,
                    }],
                    radius * 2.0,
                )
            }
        };
        (width < pitch).then_some(entities)
    }

    fn refresh(&mut self) {
        let parameters = self.helix_parameters();
        self.preview_segments = parameters
            .and_then(|parameters| helix_segments(&parameters).ok())
            .unwrap_or_default();
        self.valid = !self.preview_segments.is_empty() && self.profile_entities().is_some();
    }
}

pub fn helix_segments(
    parameters: &HelixToolParameters,
) -> Result<Vec<SpatialPathSegment>, AssistantRequestInvalid> {
    parameters.spatial_path_segments()
}

impl KetchupApp {
    fn selected_edge_axis(&self) -> Option<([f64; 3], [f64; 3])> {
        let selection = self.selection.primary.as_ref()?;
        let ElementId::TopologicalEdge {
            feature_id,
            ordinal,
        } = selection.element
        else {
            return None;
        };
        let snapshot = self.document.current();
        let results = self.topology_results_for_snapshot(&snapshot)?;
        let package = results.render_values(&snapshot).find(|package| {
            package.definition_id() == selection.definition_id
                && package.producer_feature_id() == feature_id
        })?;
        let edge = package
            .edge_evidence()
            .iter()
            .find(|edge| edge.edge_ordinal == ordinal && edge.curve_kind == "line")?;
        let origin = edge.axis_origin_mm?;
        let axis = edge.unit_axis_direction?;
        let start = Vec3::new(origin[0], origin[1], origin[2]);
        let end = start + Vec3::new(axis[0], axis[1], axis[2]) * edge.length_mm;
        let transform = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == selection.instance_path)?
            .transform;
        let start = super::transform_model_point(transform, start);
        let end = super::transform_model_point(transform, end);
        let direction = end - start;
        (direction.distance(Vec3::ZERO) > ROUNDING).then_some((
            [start.x, start.y, start.z],
            [direction.x, direction.y, direction.z],
        ))
    }

    fn use_selected_edge_axis(&mut self) -> bool {
        let Some((origin, direction)) = self.selected_edge_axis() else {
            self.digest = self.catalog.text("digest-helix-axis-selection-required");
            return false;
        };
        self.helix_tool.origin = origin.map(format_coordinate);
        self.helix_tool.axis = direction.map(format_coordinate);
        self.helix_tool.refresh();
        self.digest = self.catalog.text("digest-helix-axis-selected");
        true
    }

    fn construction_input(&self) -> Option<([f64; 3], [f64; 3])> {
        let origin = HelixUiState::parse_vector(&self.helix_tool.origin)?;
        let direction = HelixUiState::parse_vector(&self.helix_tool.axis)?;
        let length_squared = direction.iter().map(|value| value * value).sum::<f64>();
        (origin
            .iter()
            .chain(&direction)
            .all(|value| value.is_finite())
            && length_squared > ROUNDING * ROUNDING)
            .then_some((origin, direction))
    }

    fn create_construction_geometry(&mut self, kind: ConstructionKind) -> bool {
        let origin = HelixUiState::parse_vector(&self.helix_tool.origin)
            .filter(|origin| origin.iter().all(|value| value.is_finite()));
        let direction = self.construction_input().map(|(_, direction)| direction);
        let Some(origin) = origin else {
            self.digest = self.catalog.text("digest-construction-invalid");
            return false;
        };
        let number = self
            .document
            .current()
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let name = self.catalog.format(
            match kind {
                ConstructionKind::Point => "model-construction-point-definition",
                ConstructionKind::Axis => "model-construction-axis-definition",
                ConstructionKind::Plane => "model-construction-plane-definition",
            },
            &BTreeMap::from([("number", number.to_string())]),
        );
        let operation = match kind {
            ConstructionKind::Point => AssistantCadEditOperation::CreateConstructionPoint {
                name,
                position_mm: origin,
            },
            ConstructionKind::Axis => {
                let Some(direction) = direction else {
                    self.digest = self.catalog.text("digest-construction-invalid");
                    return false;
                };
                AssistantCadEditOperation::CreateConstructionAxis {
                    name,
                    origin_mm: origin,
                    direction,
                }
            }
            ConstructionKind::Plane => {
                let Some(direction) = direction else {
                    self.digest = self.catalog.text("digest-construction-invalid");
                    return false;
                };
                let reference = if direction[0].abs() < direction[2].abs() {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0]
                };
                let x_direction = ketchup_geometry::linalg::cross(reference, direction);
                AssistantCadEditOperation::CreateConstructionPlane {
                    name,
                    origin_mm: origin,
                    normal: direction,
                    x_direction,
                }
            }
        };
        let program = AssistantCadEditProgram {
            operations: vec![operation],
        };
        let batch = match self.plan_assistant_cad_edit_program(&program) {
            Ok(batch) => batch,
            Err(error) => {
                self.digest = self.catalog.format(
                    "digest-construction-refused",
                    &BTreeMap::from([("reason", error.failed_invariant)]),
                );
                return false;
            }
        };
        if let Err(error) = self.apply_batch_with_work_recovery(&batch) {
            self.digest = self.catalog.format(
                "digest-construction-refused",
                &BTreeMap::from([("reason", error.to_string())]),
            );
            return false;
        }
        self.digest = self.catalog.text(match kind {
            ConstructionKind::Point => "digest-construction-point-committed",
            ConstructionKind::Axis => "digest-construction-axis-committed",
            ConstructionKind::Plane => "digest-construction-plane-committed",
        });
        true
    }

    pub(super) fn begin_helix_tool(&mut self) {
        self.helix_tool = HelixUiState::default();
        self.helix_tool.refresh();
        self.status_key = "status-helix-preview";
    }

    pub(super) fn clear_helix_preview(&mut self) {
        self.helix_tool.preview_segments.clear();
        self.helix_tool.valid = false;
    }

    pub(super) fn helix_preview_points(&self) -> Vec<Vec3> {
        self.helix_tool
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
                        let curve =
                            CubicBezier::new([*start_mm, *control_1_mm, *control_2_mm, *end_mm]);
                        Vec3::from(curve.eval(f64::from(step) / 8.0))
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// Adds a helix: a path without a profile, a swept body with one.
    pub fn create_helix(
        &mut self,
        parameters: HelixToolParameters,
        profile: Vec<AssistantSketchEntity>,
    ) -> bool {
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
        let swept = !profile.is_empty();
        let name = self.catalog.format(
            "model-helix-definition",
            &BTreeMap::from([("number", number.to_string())]),
        );
        let program = AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateHelix {
                name,
                parameters,
                profile,
            }],
        };
        let batch = match self.plan_assistant_cad_edit_program(&program) {
            Ok(batch) => batch,
            Err(error) => {
                self.digest = format!("Helix refused: {}", error.failed_invariant);
                return false;
            }
        };
        if let Err(error) = self.apply_batch_with_work_recovery(&batch) {
            self.digest = format!("Helix refused: {error}");
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text(if swept {
            "digest-helix-body-committed"
        } else {
            "digest-helix-committed"
        });
        true
    }

    pub(super) fn show_helix_tool(&mut self, ui: &mut egui::Ui) {
        if self.active_tool != ActiveTool::Helix {
            return;
        }
        ui.heading(self.catalog.text("helix-panel-title"));
        ui.small(self.catalog.text("helix-panel-help"));
        let selected_edge_available = self.selected_edge_axis().is_some();
        if ui
            .add_enabled(
                selected_edge_available,
                egui::Button::new(self.catalog.text("helix-use-selected-edge")),
            )
            .clicked()
        {
            self.use_selected_edge_axis();
        }
        ui.small(self.catalog.text(if selected_edge_available {
            "helix-selected-edge-ready"
        } else {
            "helix-selected-edge-missing"
        }));
        ui.separator();

        let mut changed = false;
        changed |= vector_row(
            ui,
            self.catalog.text("helix-origin"),
            &mut self.helix_tool.origin,
        );
        changed |= vector_row(
            ui,
            self.catalog.text("helix-axis"),
            &mut self.helix_tool.axis,
        );
        ui.label(self.catalog.text("construction-tools"));
        let point_valid = HelixUiState::parse_vector(&self.helix_tool.origin)
            .is_some_and(|origin| origin.iter().all(|value| value.is_finite()));
        let axis_valid = self.construction_input().is_some();
        let mut construction = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    point_valid,
                    egui::Button::new(self.catalog.text("action-create-construction-point")),
                )
                .clicked()
            {
                construction = Some(ConstructionKind::Point);
            }
            if ui
                .add_enabled(
                    axis_valid,
                    egui::Button::new(self.catalog.text("action-create-construction-axis")),
                )
                .clicked()
            {
                construction = Some(ConstructionKind::Axis);
            }
            if ui
                .add_enabled(
                    axis_valid,
                    egui::Button::new(self.catalog.text("action-create-construction-plane")),
                )
                .clicked()
            {
                construction = Some(ConstructionKind::Plane);
            }
        });
        if let Some(kind) = construction {
            self.create_construction_geometry(kind);
        }
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-radius"),
            &mut self.helix_tool.radius,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-pitch"),
            &mut self.helix_tool.pitch,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-turns"),
            &mut self.helix_tool.turns,
        );
        changed |= scalar_row(
            ui,
            self.catalog.text("helix-start-angle"),
            &mut self.helix_tool.start_angle,
        );
        let handedness_label = self.catalog.text("helix-handedness");
        ui.label(&handedness_label);
        ui.horizontal(|ui| {
            let right_label = self.catalog.text("helix-right-handed");
            let right = ui.selectable_value(
                &mut self.helix_tool.handedness,
                HelixHandedness::Right,
                &right_label,
            );
            super::name_widget(&right, true, &format!("{handedness_label}: {right_label}"));
            changed |= right.changed();
            let left_label = self.catalog.text("helix-left-handed");
            let left = ui.selectable_value(
                &mut self.helix_tool.handedness,
                HelixHandedness::Left,
                &left_label,
            );
            super::name_widget(&left, true, &format!("{handedness_label}: {left_label}"));
            changed |= left.changed();
        });
        changed |= self.show_helix_profile(ui);
        if changed {
            self.helix_tool.refresh();
            self.digest = self.catalog.text(if self.helix_tool.valid {
                "digest-helix-live"
            } else {
                "digest-helix-invalid"
            });
        }
        ui.separator();
        let create = ui
            .add_enabled(
                self.helix_tool.valid,
                egui::Button::new(self.catalog.text("action-create-helix")),
            )
            .clicked();
        if ui.button(self.catalog.text("action-cancel")).clicked() {
            self.clear_ephemeral_edit_state();
            self.active_tool = ActiveTool::Select;
            self.status_key = "status-ready";
        } else if create
            && let (Some(parameters), Some(profile)) = (
                self.helix_tool.helix_parameters(),
                self.helix_tool.profile_entities(),
            )
        {
            self.create_helix(parameters, profile);
        }
    }

    /// The profile chooser and its sizes; true when any of them changed.
    fn show_helix_profile(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let profile_label = self.catalog.text("helix-profile");
        ui.label(&profile_label);
        ui.horizontal_wrapped(|ui| {
            for profile in HelixProfile::ALL {
                let label = self.catalog.text(profile.label_key());
                let response = ui.selectable_value(&mut self.helix_tool.profile, profile, &label);
                super::name_widget(&response, true, &format!("{profile_label}: {label}"));
                changed |= response.changed();
            }
        });
        let state = &mut self.helix_tool;
        let rows: Vec<(&str, &mut String)> = match state.profile {
            HelixProfile::None => Vec::new(),
            HelixProfile::Tooth => vec![
                ("helix-tooth-depth", &mut state.tooth_depth),
                ("helix-tooth-width", &mut state.tooth_width),
                ("helix-tooth-crest", &mut state.tooth_crest),
            ],
            HelixProfile::Circle => vec![("helix-circle-radius", &mut state.circle_radius)],
        };
        for (key, value) in rows {
            changed |= scalar_row(ui, self.catalog.text(key), value);
        }
        changed
    }
}

fn format_coordinate(value: f64) -> String {
    let formatted = format!("{value:.6}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn vector_row(ui: &mut egui::Ui, label: String, values: &mut [String; 3]) -> bool {
    ui.label(&label);
    let mut changed = false;
    ui.horizontal(|ui| {
        for (axis, value) in ["X", "Y", "Z"].into_iter().zip(values.iter_mut()) {
            ui.label(axis);
            let accessible_name = format!("{label} {axis}");
            let response = ui.add(egui::TextEdit::singleline(value).desired_width(52.0));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, accessible_name.clone())
            });
            changed |= response.changed();
        }
    });
    changed
}

fn scalar_row(ui: &mut egui::Ui, label: String, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        ui.label(&label);
        let response = ui.add(egui::TextEdit::singleline(value).desired_width(84.0));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label.clone())
        });
        response.changed()
    })
    .inner
}

#[cfg(test)]
fn unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    ketchup_geometry::linalg::normalize_within(vector, ROUNDING)
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
    fn tooth_is_a_closed_triangle_or_trapezoid_and_refuses_impossible_sizes() {
        let corners = |entities: Vec<AssistantSketchEntity>| {
            entities
                .into_iter()
                .map(|entity| match entity {
                    AssistantSketchEntity::Line { start_mm, .. } => start_mm,
                    other => panic!("a tooth is made of lines, got {other:?}"),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            corners(tooth(0.8, 1.3, 0.0).unwrap()),
            [[0.0, -0.65], [0.8, 0.0], [0.0, 0.65]]
        );
        assert_eq!(
            corners(tooth(2.0, 4.0, 1.0).unwrap()),
            [[0.0, -2.0], [2.0, -0.5], [2.0, 0.5], [0.0, 2.0]]
        );
        assert!(tooth(0.0, 4.0, 1.0).is_none());
        assert!(tooth(2.0, 4.0, 4.0).is_none());
        assert!(tooth(2.0, 4.0, -1.0).is_none());
    }

    #[test]
    fn profile_must_fit_between_two_turns() {
        let mut state = HelixUiState {
            profile: HelixProfile::Circle,
            circle_radius: "2.5".into(),
            ..HelixUiState::default()
        };
        assert!(state.profile_entities().is_none(), "diameter 5 == pitch 5");
        state.circle_radius = "2.4".into();
        assert_eq!(state.profile_entities().unwrap().len(), 1);
        state.profile = HelixProfile::None;
        assert!(state.profile_entities().unwrap().is_empty());
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
