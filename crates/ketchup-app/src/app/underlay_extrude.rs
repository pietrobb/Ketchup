//! Tools > Pull drawing area up and `drawing action=extrude`: a closed area of a drawing
//! (around a point, or one closed polyline or hatch of it) extruded by a height into a
//! solid, with its holes cut through, as one Undo step. Where a program owns the document
//! (or there is nothing in it yet) the solid is written into the program as
//! `program action=from_underlay` writes it, so it follows the drawing when the drawing is
//! edited; otherwise it is added as a part of its own.

use crate::live_bridge::take_error_details;
use crate::live_bridge::underlay_program::{
    Ring, free_name, underlay_area_outline, underlay_extrusion_program,
};
use crate::*;
use ketchup_application::RuleProgramApplyError;
use ketchup_model::underlay::UnderlayId;
use std::fmt;

/// A closed area of a drawing: on which drawing, by a point in it or by its object.
pub(crate) struct ChosenArea {
    underlay: UnderlayId,
    point_mm: Option<[f64; 2]>,
    object_id: Option<u32>,
    outline: Ring,
    holes: Vec<Ring>,
}

/// What pulling an area up made: the name of its data or part, and whether the program
/// keeps it bound to the drawing.
pub(crate) struct PulledArea {
    pub name: String,
    pub bound: bool,
}

/// Why no area of a drawing was taken or written: the code of the refusal and the
/// rejection recorded with it, so the window and an MCP answer give its cause, not "no
/// closed area" (review 2026-10-29 UO-63).
#[derive(Debug)]
pub(crate) struct AreaRefusal {
    pub code: &'static str,
    /// The rejection as `take_error_details` gave it.
    pub recorded: Option<serde_json::Value>,
}

impl AreaRefusal {
    /// The refusal `code` with the rejection recorded for it.
    fn taken(code: &'static str) -> Self {
        Self {
            code,
            recorded: take_error_details(),
        }
    }
}

impl fmt::Display for AreaRefusal {
    /// The reason of the rejection and its causes, else the code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = |key: &str| {
            self.recorded
                .as_ref()
                .and_then(|recorded| recorded.get(key))
                .cloned()
        };
        let reason = text("reason").and_then(|reason| reason.as_str().map(str::to_owned));
        let causes = text("causes")
            .and_then(|causes| serde_json::from_value::<Vec<String>>(causes).ok())
            .unwrap_or_default();
        let reason = reason.filter(|reason| reason != self.code);
        match (reason, causes.is_empty()) {
            (Some(reason), true) => f.write_str(&reason),
            (Some(reason), false) => write!(f, "{reason}: {}", causes.join(": ")),
            (None, false) => f.write_str(&causes.join(": ")),
            (None, true) => f.write_str(self.code),
        }
    }
}

impl std::error::Error for AreaRefusal {}

/// Why an area of a drawing was not pulled up.
#[derive(Debug)]
pub(crate) enum AreaPullError {
    /// The height is not a positive length.
    Height,
    /// The area cannot be written as program data (e.g. it has too many corners).
    NotWritable(AreaRefusal),
    /// The program owning the document refused the extruded area.
    Program(RuleProgramApplyError),
    /// The document refused the new part.
    Part(WorkRecoveryMutationError<CanonicalError>),
}

impl fmt::Display for AreaPullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Height => f.write_str("the height must be a positive length"),
            Self::NotWritable(refusal) => {
                write!(f, "the area cannot be written into the program: {refusal}")
            }
            Self::Program(error) => write!(f, "the program refused the area: {error}"),
            Self::Part(error) => write!(f, "the part was refused: {error}"),
        }
    }
}

impl std::error::Error for AreaPullError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Height => None,
            Self::NotWritable(refusal) => Some(refusal),
            Self::Program(error) => Some(error),
            Self::Part(error) => Some(error),
        }
    }
}

impl KetchupApp {
    /// One click of the Pull area tool.
    pub(crate) fn click_underlay_extrude(&mut self, pointer: Pos2, rect: Rect) {
        let Some(height_mm) =
            parse_distance_mm(self.value_box.input.trim(), self.value_box.decimal)
        else {
            self.digest = self.catalog.text("digest-underlay-extrude-height");
            return;
        };
        let area = match self.underlay_area_at(pointer, rect) {
            Ok(area) => area,
            Err(None) => {
                self.digest = self.catalog.text("digest-underlay-extrude-no-area");
                return;
            }
            Err(Some(refusal)) => {
                self.digest = self.catalog.format(
                    "digest-underlay-extrude-refused",
                    &BTreeMap::from([("reason", refusal.to_string())]),
                );
                return;
            }
        };
        self.digest = match self.pull_underlay_area(&area, height_mm) {
            Ok(pulled) => self.catalog.format(
                if pulled.bound {
                    "digest-underlay-extruded"
                } else {
                    "digest-underlay-extruded-part"
                },
                &BTreeMap::from([("height", format_height(height_mm)), ("name", pulled.name)]),
            ),
            Err(AreaPullError::Height) => self.catalog.text("digest-underlay-extrude-height"),
            Err(AreaPullError::Program(error)) => error.rejection().reason_text().to_owned(),
            Err(error) => self.catalog.format(
                "digest-underlay-extrude-refused",
                &BTreeMap::from([("reason", error.to_string())]),
            ),
        };
    }

    /// The area under `pointer`: the innermost closed region of a drawing around the
    /// point (the selected drawing first, then the one drawn last), else the closed object
    /// whose line or fill is under it. With none, why a drawing's regions were not found,
    /// if one refused: an object under the pointer that is not closed is just no area.
    fn underlay_area_at(
        &self,
        pointer: Pos2,
        rect: Rect,
    ) -> Result<ChosenArea, Option<AreaRefusal>> {
        let point = self.screen_to_plane(pointer, rect, 0.0).ok_or(None)?;
        let snapshot = self.document.current();
        let mut underlays = snapshot
            .underlays()
            .filter(|underlay| underlay.lies_flat())
            .map(|underlay| underlay.id)
            .collect::<Vec<_>>();
        underlays.reverse();
        if let Some(selected) = self.selected_underlay()
            && underlays.contains(&selected)
        {
            underlays.retain(|id| *id != selected);
            underlays.insert(0, selected);
        }
        let mut by_object = self
            .underlay_line_at_screen(pointer, rect)
            .map(|(underlay, object, _)| (underlay, object))
            .into_iter()
            .chain(self.underlay_object_at_screen(pointer, rect));
        let mut refused = None;
        for underlay in underlays {
            match self.underlay_area(underlay, Some([point.x, point.y]), None) {
                Ok(Some(area)) => return Ok(area),
                Ok(None) => {}
                Err(refusal) => {
                    refused.get_or_insert(refusal);
                }
            }
        }
        by_object
            .find_map(|(underlay, object)| {
                self.underlay_area(underlay, None, Some(object))
                    .ok()
                    .flatten()
            })
            .ok_or(refused)
    }

    /// The innermost closed region of the shown layers of `underlay` around the document
    /// point `point_mm`, or the outline of its closed object `object_id`; none when no
    /// region holds the point.
    pub(crate) fn underlay_area(
        &self,
        underlay: UnderlayId,
        point_mm: Option<[f64; 2]>,
        object_id: Option<u32>,
    ) -> Result<Option<ChosenArea>, AreaRefusal> {
        let found = underlay_area_outline(self, underlay, point_mm, object_id)
            .map_err(AreaRefusal::taken)?;
        let Some((outline, holes)) = found else {
            return Ok(None);
        };
        Ok(Some(ChosenArea {
            underlay,
            point_mm,
            object_id,
            outline,
            holes,
        }))
    }

    /// Extrudes `area` up `height_mm` from the ground, as one Undo step.
    pub(crate) fn pull_underlay_area(
        &mut self,
        area: &ChosenArea,
        height_mm: f64,
    ) -> Result<PulledArea, AreaPullError> {
        if !(height_mm.is_finite() && height_mm > limits::MIN_LENGTH_MM) {
            return Err(AreaPullError::Height);
        }
        let name = free_name(self, "AREA");
        let bound = self.document.current_rule_program().is_some()
            || self.document.current().definitions().next().is_none();
        if bound {
            let source = underlay_extrusion_program(
                self,
                area.underlay,
                area.point_mm,
                area.object_id,
                name.clone(),
                height_mm,
            );
            let source =
                source.map_err(|code| AreaPullError::NotWritable(AreaRefusal::taken(code)))?;
            self.apply_program_source_with(source, false, Vec::new())
                .map_err(AreaPullError::Program)?;
        } else {
            let batch = self
                .area_part_batch(area, height_mm, &name)
                .ok_or(AreaPullError::Height)?;
            self.apply_batch_with_work_recovery(&batch)
                .map_err(AreaPullError::Part)?;
        }
        Ok(PulledArea { name, bound })
    }

    /// A part of its own: the outline extruded `height_mm` up from the ground, each hole
    /// cut through it by a prism a millimetre longer at both ends.
    fn area_part_batch(
        &self,
        area: &ChosenArea,
        height_mm: f64,
        name: &str,
    ) -> Option<CommandBatch> {
        let snapshot = self.document.current();
        let definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let mut next_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0);
        let mut feature_id = || {
            next_feature += 1;
            FeatureId(next_feature)
        };
        let height = |mm: f64| Dimension::new(format_height(mm), mm);
        let feature = |id, kind| CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name: name.to_owned(),
            kind,
        };
        let (profile, solid) = (feature_id(), feature_id());
        let mut commands = vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.to_owned(),
            },
            feature(profile, FeatureKind::polygon(&area.outline)),
            feature(
                solid,
                FeatureKind::extrusion(profile, height(height_mm).ok()?),
            ),
        ];
        let mut target = solid;
        // A new definition's own body is its first.
        let own_body = BodyId(1);
        for (hole, tool) in area.holes.iter().zip(own_body.0 + 1..) {
            let tool_height = height(height_mm + 2.0).ok()?;
            let lowered = Transform::from_translation(0.0, 0.0, -1.0).ok()?;
            let tool_body = BodyId(tool);
            let [hole_profile, prism, placed, cut] = [(); 4].map(|()| feature_id());
            commands.extend([
                CanonicalCommand::CreateBody {
                    definition_id,
                    id: tool_body,
                    name: name.to_owned(),
                    visible: false,
                },
                CanonicalCommand::SetActiveBody {
                    definition_id,
                    id: tool_body,
                },
                feature(hole_profile, FeatureKind::polygon(hole)),
                feature(prism, FeatureKind::extrusion(hole_profile, tool_height)),
                feature(
                    placed,
                    FeatureKind::RigidTransform {
                        target: prism,
                        transform: lowered,
                    },
                ),
                CanonicalCommand::SetActiveBody {
                    definition_id,
                    id: own_body,
                },
                feature(
                    cut,
                    FeatureKind::Boolean {
                        operation: BooleanOperation::Cut,
                        target,
                        tool: placed,
                    },
                ),
                CanonicalCommand::ConsumeBody {
                    definition_id,
                    id: tool_body,
                    by_feature_id: cut,
                },
            ]);
            target = cut;
        }
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: name.to_owned(),
            transform: Transform::identity(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
        Some(CommandBatch::new(commands))
    }
}
