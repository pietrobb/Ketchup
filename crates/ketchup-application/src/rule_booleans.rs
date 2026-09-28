//! Canonical commands for rule-program booleans. Each tool is rebuilt as a
//! hidden body inside the target part's definition, moved into the target's
//! frame and consumed by one Boolean feature.

use crate::diagnostics::{AssistantRejection, assistant_canonical_rejection};
use crate::sketch::assistant_sketch_entities;
use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, DefinitionId, Dimension, FeatureId,
    FeatureKind, LoftContinuity, LoftSection, ProfileSegment, SpatialPathSegment, Transform,
};
use ketchup_core::sketch::{PrincipalPlane, SketchSpec, WorkplaneSpec};
use ketchup_program::model::{
    Part, ProgramBoolean, ProgramBooleanKind, ProgramLoftSection, ProgramPartBody,
    ProgramPathSegment, ProgramProfileSegment,
};

/// The canonical segment of one program profile segment.
pub(crate) fn profile_segment(segment: &ProgramProfileSegment) -> ProfileSegment {
    match segment.arc {
        None => ProfileSegment::Line {
            start_mm: segment.start_mm,
            end_mm: segment.end_mm,
        },
        Some(arc) => ProfileSegment::CircularArc {
            start_mm: segment.start_mm,
            end_mm: segment.end_mm,
            center_mm: arc.center_mm,
            clockwise: arc.clockwise,
        },
    }
}

/// The canonical sweep path of a program path.
pub(crate) fn spatial_path(path: &[ProgramPathSegment]) -> FeatureKind {
    FeatureKind::SpatialPath {
        segments: path
            .iter()
            .map(|segment| match segment.arc {
                None => SpatialPathSegment::Line {
                    start_mm: segment.start_mm,
                    end_mm: segment.end_mm,
                },
                Some(arc) => SpatialPathSegment::CircularArc {
                    start_mm: segment.start_mm,
                    end_mm: segment.end_mm,
                    center_mm: arc.center_mm,
                    normal: arc.normal,
                    clockwise: false,
                },
            })
            .collect(),
    }
}

/// One loft section drawn on `workplane` (the part's XY plane); the loft
/// lifts it to its height along the plane normal.
fn section_sketch(section: &ProgramLoftSection, workplane: FeatureId) -> FeatureKind {
    FeatureKind::Sketch(SketchSpec {
        workplane,
        entities: ketchup_program::cad::profile_entities(&section.segments)
            .iter()
            .flat_map(assistant_sketch_entities)
            .collect(),
        constraints: Vec::new(),
    })
}

/// A loft through `profiles`, one per section, at the sections' heights.
pub(crate) fn loft(sections: &[ProgramLoftSection], profiles: &[FeatureId]) -> FeatureKind {
    FeatureKind::Loft {
        sections: sections
            .iter()
            .zip(profiles)
            .map(|(section, profile)| LoftSection {
                profile: *profile,
                elevation_mm: section.elevation_mm,
            })
            .collect(),
        guide: None,
        continuity: LoftContinuity::Position,
    }
}

/// Turns the profile sketch and one-millimetre base that `cad::part` plans
/// for a swept or lofted part into the real body: the sketch becomes the
/// sweep profile or stays the first loft section, the path or further
/// sections are added before `body`, and `body` becomes the sweep or loft.
pub(crate) fn replace_base_body(
    commands: &mut Vec<CanonicalCommand>,
    part: &Part,
    definition_id: DefinitionId,
    body: FeatureId,
    next_feature: &mut u64,
) -> Result<(), AssistantRejection> {
    let position = |commands: &[CanonicalCommand],
                    wanted: &dyn Fn(FeatureId, &FeatureKind) -> bool| {
        commands.iter().position(|command| {
            matches!(command, CanonicalCommand::CreateFeature { id, definition_id: owner, kind, .. }
                if *owner == definition_id && wanted(*id, kind))
        })
    };
    let (Some(sketch), Some(mut base)) = (
        position(commands, &|_, kind| matches!(kind, FeatureKind::Sketch(_))),
        position(commands, &|id, _| id == body),
    ) else {
        unreachable!("cad::part plans a sketch and a base body for every profile part");
    };
    let CanonicalCommand::CreateFeature {
        id: profile,
        kind: FeatureKind::Sketch(SketchSpec { workplane, .. }),
        ..
    } = commands[sketch]
    else {
        unreachable!("located as a sketch");
    };
    let mut added = Vec::new();
    let mut create = |name: String, kind: FeatureKind| -> Result<FeatureId, AssistantRejection> {
        *next_feature = next_feature.checked_add(1).ok_or_else(|| {
            assistant_canonical_rejection(CanonicalError::IdExhausted, "rule_part", &part.name)
        })?;
        let id = FeatureId(*next_feature);
        added.push(CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name,
            kind,
        });
        Ok(id)
    };
    let solid = match &part.body {
        ProgramPartBody::Sweep { segments, path } => {
            // Only a plain profile is read in the path's own section frame.
            if let CanonicalCommand::CreateFeature { kind, .. } = &mut commands[sketch] {
                *kind = FeatureKind::SegmentProfile {
                    segments: segments.iter().map(profile_segment).collect(),
                    closed: true,
                };
            }
            FeatureKind::Sweep {
                profile,
                path: create(format!("{} path", part.name), spatial_path(path))?,
            }
        }
        ProgramPartBody::Loft { sections } => {
            let mut profiles = vec![profile];
            for (index, section) in sections.iter().enumerate().skip(1) {
                profiles.push(create(
                    format!("{} section {}", part.name, index + 1),
                    section_sketch(section, workplane),
                )?);
            }
            loft(sections, &profiles)
        }
        _ => unreachable!("only swept and lofted parts replace their base"),
    };
    for command in added {
        commands.insert(base, command);
        base += 1;
    }
    if let CanonicalCommand::CreateFeature { kind, .. } = &mut commands[base] {
        *kind = solid;
    }
    Ok(())
}

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
            | ProgramPartBody::Revolve { segments, .. }
            | ProgramPartBody::Sweep { segments, .. } => {
                segments.iter().map(profile_segment).collect()
            }
            ProgramPartBody::Loft { sections } => {
                let workplane = self.feature(
                    format!("{prefix} tool workplane"),
                    FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                )?;
                let mut profiles = Vec::new();
                for (index, section) in sections.iter().enumerate() {
                    profiles.push(self.feature(
                        format!("{prefix} tool section {}", index + 1),
                        section_sketch(section, workplane),
                    )?);
                }
                let solid = self.feature(format!("{prefix} tool"), loft(sections, &profiles))?;
                let solid = self.apply_in_body(tool, solid, body)?;
                return Ok((solid, body));
            }
        };
        let profile = self.feature(
            format!("{prefix} tool profile"),
            FeatureKind::SegmentProfile {
                segments,
                closed: true,
            },
        )?;
        let solid_kind = match &tool.body {
            ProgramPartBody::Sweep { path, .. } => FeatureKind::Sweep {
                profile,
                path: self.feature(format!("{prefix} tool path"), spatial_path(path))?,
            },
            ProgramPartBody::Loft { .. } => unreachable!("built above"),
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
