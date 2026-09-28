//! Canonical commands for rule-program booleans. Each tool is rebuilt as a
//! hidden body inside the target part's definition, moved into the target's
//! frame and consumed by one Boolean feature.

use crate::diagnostics::{AssistantRejection, assistant_canonical_rejection};
use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, DefinitionId, Dimension, FeatureId,
    FeatureKind, ProfileSegment, Transform,
};
use ketchup_program::model::{Part, ProgramBoolean, ProgramBooleanKind, ProgramPartBody};

/// A new definition starts with this body active; the part's own features live in it.
const PART_BODY: BodyId = BodyId(1);

pub(crate) struct BooleanPlanner<'a> {
    pub commands: &'a mut Vec<CanonicalCommand>,
    pub definition_id: DefinitionId,
    pub next_feature: &'a mut u64,
    next_body: u64,
    part_name: String,
}

impl<'a> BooleanPlanner<'a> {
    pub fn new(
        commands: &'a mut Vec<CanonicalCommand>,
        definition_id: DefinitionId,
        next_feature: &'a mut u64,
        part_name: &str,
    ) -> Self {
        Self {
            commands,
            definition_id,
            next_feature,
            next_body: PART_BODY.0,
            part_name: part_name.to_owned(),
        }
    }

    /// Applies `part`'s booleans to its final solid `target`; returns the new final solid.
    pub fn apply(
        &mut self,
        part: &Part,
        target: FeatureId,
    ) -> Result<FeatureId, AssistantRejection> {
        self.apply_in_body(part, target, PART_BODY)
    }

    fn apply_in_body(
        &mut self,
        part: &Part,
        mut target: FeatureId,
        body: BodyId,
    ) -> Result<FeatureId, AssistantRejection> {
        for boolean in &part.booleans {
            target = self.boolean(part, boolean, target, body)?;
        }
        Ok(target)
    }

    fn boolean(
        &mut self,
        owner: &Part,
        boolean: &ProgramBoolean,
        target: FeatureId,
        body: BodyId,
    ) -> Result<FeatureId, AssistantRejection> {
        let prefix = format!("{} {}", owner.name, boolean.name);
        let (tool_solid, tool_body) = self.tool_solid(&boolean.tool, &prefix)?;
        let transform = Transform::from_matrix(owner.frame_of(&boolean.tool))
            .map_err(|error| self.rejection(error))?;
        let placed = self.feature(
            format!("{prefix} placement"),
            FeatureKind::RigidTransform {
                target: tool_solid,
                transform,
            },
        )?;
        self.commands.push(CanonicalCommand::SetActiveBody {
            definition_id: self.definition_id,
            id: body,
        });
        let result = self.feature(
            boolean.name.clone(),
            FeatureKind::Boolean {
                operation: match boolean.kind {
                    ProgramBooleanKind::Subtract => BooleanOperation::Cut,
                    ProgramBooleanKind::Intersect => BooleanOperation::Intersect,
                },
                target,
                tool: placed,
            },
        )?;
        self.commands.push(CanonicalCommand::ConsumeBody {
            definition_id: self.definition_id,
            id: tool_body,
            by_feature_id: result,
        });
        Ok(result)
    }

    /// Builds `tool` (body shape plus its own booleans) in a new hidden body,
    /// in the tool's own frame.
    fn tool_solid(
        &mut self,
        tool: &Part,
        prefix: &str,
    ) -> Result<(FeatureId, BodyId), AssistantRejection> {
        self.next_body += 1;
        let body = BodyId(self.next_body);
        self.commands.push(CanonicalCommand::CreateBody {
            definition_id: self.definition_id,
            id: body,
            name: format!("{prefix} tool"),
            visible: false,
        });
        self.commands.push(CanonicalCommand::SetActiveBody {
            definition_id: self.definition_id,
            id: body,
        });
        let line = |start_mm: [f64; 2], end_mm: [f64; 2]| ProfileSegment::Line { start_mm, end_mm };
        let segments = match &tool.body {
            ProgramPartBody::Panel => {
                let [x, y, _] = tool.size_mm;
                vec![
                    line([0.0, 0.0], [x, 0.0]),
                    line([x, 0.0], [x, y]),
                    line([x, y], [0.0, y]),
                    line([0.0, y], [0.0, 0.0]),
                ]
            }
            ProgramPartBody::Extrusion { segments, .. }
            | ProgramPartBody::Revolve { segments, .. } => segments
                .iter()
                .map(|segment| line(segment.start_mm, segment.end_mm))
                .collect(),
        };
        let profile = self.feature(
            format!("{prefix} tool profile"),
            FeatureKind::SegmentProfile {
                segments,
                closed: true,
            },
        )?;
        let solid_kind = match &tool.body {
            ProgramPartBody::Panel => self.extrusion(profile, tool.size_mm[2])?,
            ProgramPartBody::Extrusion { distance_mm, .. } => {
                self.extrusion(profile, *distance_mm)?
            }
            ProgramPartBody::Revolve {
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
                ..
            } => FeatureKind::Revolve {
                profile,
                axis_start_mm: *axis_start_mm,
                axis_end_mm: *axis_end_mm,
                angle_degrees: *angle_degrees,
            },
        };
        let solid = self.feature(format!("{prefix} tool"), solid_kind)?;
        let solid = self.apply_in_body(tool, solid, body)?;
        Ok((solid, body))
    }

    fn extrusion(
        &self,
        profile: FeatureId,
        height_mm: f64,
    ) -> Result<FeatureKind, AssistantRejection> {
        Ok(FeatureKind::Extrusion {
            profile,
            height: Dimension::new(height_mm.to_string(), height_mm)
                .map_err(|error| self.rejection(error))?,
        })
    }

    fn feature(
        &mut self,
        name: String,
        kind: FeatureKind,
    ) -> Result<FeatureId, AssistantRejection> {
        *self.next_feature = self
            .next_feature
            .checked_add(1)
            .ok_or_else(|| self.rejection(CanonicalError::IdExhausted))?;
        let id = FeatureId(*self.next_feature);
        self.commands.push(CanonicalCommand::CreateFeature {
            id,
            definition_id: self.definition_id,
            name,
            kind,
        });
        Ok(id)
    }

    fn rejection(&self, error: CanonicalError) -> AssistantRejection {
        assistant_canonical_rejection(error, "rule_part", &self.part_name)
    }
}
