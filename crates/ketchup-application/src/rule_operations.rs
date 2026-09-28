//! Canonical commands for the operations of a rule-program part, applied in
//! program order, each to the solid the one before produced. A boolean's
//! tool is rebuilt as a hidden body inside the part's definition, moved into
//! the part's frame and consumed by one Boolean feature.

use crate::diagnostics::{
    AssistantRejection, assistant_canonical_rejection, assistant_planning_rejection,
};
use crate::sketch::assistant_sketch_entities;
use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, ChamferMode, DefinitionId,
    Dimension, EdgeFinishKind, FeatureId, FeatureKind, LoftContinuity, LoftSection,
    ProfileEdgeReference, ProfileFaceReference, ProfileSegment, SpatialPathSegment, Transform,
};
use ketchup_core::sketch::{PrincipalPlane, SketchSpec, WorkplaneSpec};
use ketchup_program::model::{
    Part, ProgramBoolean, ProgramBooleanKind, ProgramCut, ProgramEdgeFillet, ProgramEdgeFinishKind,
    ProgramFaceOffset, ProgramLoftSection, ProgramOperation, ProgramPartBody, ProgramPathSegment,
    ProgramProfileSegment,
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

pub(crate) struct OperationPlanner<'a> {
    pub commands: &'a mut Vec<CanonicalCommand>,
    pub definition_id: DefinitionId,
    pub next_feature: &'a mut u64,
    next_body: u64,
    part_name: String,
}

impl<'a> OperationPlanner<'a> {
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

    /// Applies `part`'s operations in program order to its body solid
    /// `target`; returns the final solid.
    pub fn apply(
        &mut self,
        part: &Part,
        mut target: FeatureId,
    ) -> Result<FeatureId, AssistantRejection> {
        for operation in &part.operations {
            target = match operation {
                ProgramOperation::Cut(cut) => self.cut(cut, target)?,
                ProgramOperation::Finish(finish) => self.finish(part, finish, target)?,
                ProgramOperation::FaceOffset(offset) => self.face_offset(part, offset, target)?,
                ProgramOperation::Boolean(boolean) => {
                    self.boolean(part, boolean, target, PART_BODY)?
                }
            };
        }
        Ok(target)
    }

    /// A tool's volume is its body and its own booleans; its cuts, finishes
    /// and moved faces are not part of what it removes or keeps.
    fn apply_tool_booleans(
        &mut self,
        tool: &Part,
        mut target: FeatureId,
        body: BodyId,
    ) -> Result<FeatureId, AssistantRejection> {
        for boolean in tool.booleans() {
            target = self.boolean(tool, boolean, target, body)?;
        }
        Ok(target)
    }

    fn cut(&mut self, cut: &ProgramCut, target: FeatureId) -> Result<FeatureId, AssistantRejection> {
        let profile = self.feature(
            format!("{} sketch", cut.name),
            FeatureKind::SegmentProfile {
                segments: cut.segments.iter().map(profile_segment).collect(),
                closed: true,
            },
        )?;
        let depth = self.dimension(cut.depth_mm)?;
        self.feature(
            cut.name.clone(),
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            },
        )
    }

    fn finish(
        &mut self,
        part: &Part,
        finish: &ProgramEdgeFillet,
        target: FeatureId,
    ) -> Result<FeatureId, AssistantRejection> {
        let profile_edges = finish
            .edges
            .iter()
            .map(|[first, second]| {
                Ok(ProfileEdgeReference {
                    first: self.face_reference(part, first)?,
                    second: self.face_reference(part, second)?,
                })
            })
            .collect::<Result<Vec<_>, AssistantRejection>>()?;
        let amount = self.dimension(finish.radius_mm)?;
        self.feature(
            finish.name.clone(),
            FeatureKind::TopologyEdgeFinish {
                target,
                edges: Vec::new(),
                profile_edges,
                kind: match finish.kind {
                    ProgramEdgeFinishKind::Fillet => EdgeFinishKind::Fillet,
                    ProgramEdgeFinishKind::Chamfer => EdgeFinishKind::Chamfer,
                },
                amount,
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ChamferMode::Symmetric,
                chamfer_edge_sides: Vec::new(),
            },
        )
    }

    fn face_offset(
        &mut self,
        part: &Part,
        offset: &ProgramFaceOffset,
        target: FeatureId,
    ) -> Result<FeatureId, AssistantRejection> {
        let profile_face = self.face_reference(part, &offset.face)?;
        let distance = self.dimension(offset.distance_mm)?;
        self.feature(
            offset.name.clone(),
            FeatureKind::TopologyFaceOffset {
                target,
                face: None,
                profile_face: Some(profile_face),
                distance,
            },
        )
    }

    /// The exact kernel's reference to a program face name.
    fn face_reference(
        &self,
        part: &Part,
        name: &str,
    ) -> Result<ProfileFaceReference, AssistantRejection> {
        let label = part.exact_face_label(name).map_err(|error| {
            assistant_planning_rejection(
                "planning.rule_part_face_missing",
                "rule_part",
                &part.name,
                error,
                "Use a face the part has: the listed names, <operation>.<tool face> or <face>#<n>.",
            )
        })?;
        Ok(match label.as_str() {
            "start" => ProfileFaceReference::Start,
            "end" => ProfileFaceReference::End,
            _ => match label
                .strip_prefix("segment_")
                .and_then(|id| id.parse::<u64>().ok())
            {
                Some(entity_id) => ProfileFaceReference::Segment {
                    entity_id,
                    source_name: name.to_owned(),
                },
                None => ProfileFaceReference::NamedResult(label),
            },
        })
    }

    fn dimension(&self, millimetres: f64) -> Result<Dimension, AssistantRejection> {
        Dimension::new(millimetres.to_string(), millimetres).map_err(|error| self.rejection(error))
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
                let solid = self.apply_tool_booleans(tool, solid, body)?;
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
        let solid = self.apply_tool_booleans(tool, solid, body)?;
        Ok((solid, body))
    }

    fn extrusion(
        &self,
        profile: FeatureId,
        height_mm: f64,
    ) -> Result<FeatureKind, AssistantRejection> {
        Ok(FeatureKind::Extrusion {
            profile,
            height: self.dimension(height_mm)?,
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
