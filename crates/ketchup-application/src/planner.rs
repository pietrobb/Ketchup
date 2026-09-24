use crate::append_feature::plan_feature_kind;
use crate::creation::plan_creation;
use crate::diagnostics::{
    AssistantPlanningResult, assistant_canonical_rejection, assistant_planning_rejection,
    assistant_rejection,
};
use crate::transforms::{
    rotation_in_parent_space, translated_transform, world_axis_rotation_transform,
    world_plane_mirror_transform,
};
use ketchup_core::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    preview_assembly_joint_drag,
};
use ketchup_core::assistant_sidecar::{
    AssistantAssemblyJointAxis, AssistantAssemblyJointKind, AssistantAssemblyJointLimits,
    AssistantAxisSpec, AssistantCadBodyFeature, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadFeatureReference, AssistantCadNamedProgramOutputReference,
    AssistantCadParameterValueType, AssistantCadPartFeature, AssistantCadProgramFeatureOutput,
    AssistantCadProgramFeatureReference, AssistantCadSurfaceBodySource, AssistantCamToolKind,
    AssistantCamWorkOffset, AssistantInstancePath, AssistantInstancePathStep, AssistantPanelHole,
    AssistantPanelPocket, AssistantPrincipalPlane, AssistantProgramDowelJointFace,
    AssistantRejectionDiagnostic, AssistantRejectionPhase, AssistantSketchEntity,
    AssistantStandardDowel, AssistantWorkplaneSpec, validated_spatial_path_segments,
};
use ketchup_core::cam::{
    CamCutParameters, CamPlan, CamPlanId, CamSetup, CamStock, CamTool, CamToolKind, CamWorkOffset,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, ClassificationCategoryId, ClassificationDimensionId,
    CloneDefinitionPlan, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, FeatureParameterTarget, InstancePath, InstancePathStep, LocalGroupId,
    LocalOccurrenceId, NodeId, OccurrenceId, ParameterPath, ParameterValueType, ProfileSegment,
    Snapshot, SpatialPathSegment, TagId, Transform,
};
use ketchup_core::drawing::{
    DrawingBomBalloon, DrawingBomBalloonId, DrawingError, DrawingMargins, DrawingPageOrientation,
    DrawingPageSize, DrawingPageTemplate, DrawingScale, DrawingSheet, DrawingSheetId,
    DrawingSource, DrawingTitleBlock, OrthographicViewKind, project_orthographic_drawing,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactPlanarOffsetRequest, ExactResultRegistry, ExactSnapshotPreparation,
};
use ketchup_core::joinery::{
    DowelHole, DowelJointContract, DowelJointFace, DowelJointId, DowelPhysicalHolePair,
    StandardDowel, project_dowel_joint_contract,
};
use ketchup_core::sketch::{SketchConstraintId, SketchEntity, WorkplaneSupport};
use ketchup_core::topology::TopologicalElementKind;
use ketchup_interaction::Vec3;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const MAX_AXIS_INSTANCE_OCCURRENCES: usize = 10_000;
const MAX_AXIS_INSTANCE_PATH_STEPS: usize = 256;
const MAX_AXIS_INSTANCE_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadResolvedProgramOutput {
    Definition(u64),
    Occurrence(u64),
    SketchFeature(u64),
    ConstructionFeature(u64),
    BodyFeature(u64),
    AssemblyJoint(u64),
    DowelJoint(u64),
}

#[derive(Clone, Debug)]
pub struct AssistantCadProgramPlan {
    pub batch: CommandBatch,
    pub outputs: BTreeMap<String, AssistantCadResolvedProgramOutput>,
}

#[derive(Default)]
struct StagedProgramOutputs {
    definition: Option<DefinitionId>,
    occurrence: Option<OccurrenceId>,
    sketch_feature: Option<FeatureId>,
    body_feature: Option<FeatureId>,
    construction_feature: Option<FeatureId>,
    assembly_joint: Option<AssemblyJointId>,
    dowel_joint: Option<DowelJointId>,
}

#[derive(Clone, Copy)]
enum StagedProgramOutput {
    Definition(DefinitionId),
    Occurrence(OccurrenceId),
    SketchFeature(FeatureId),
    BodyFeature(FeatureId),
    ConstructionFeature(FeatureId),
    AssemblyJoint(AssemblyJointId),
    DowelJoint(DowelJointId),
}

struct StagedPlanningContext {
    base_snapshot: Snapshot,
    staged_snapshot: Snapshot,
    commands: Vec<CanonicalCommand>,
    staged_command_count: usize,
    operation_outputs: BTreeMap<usize, StagedProgramOutputs>,
    named_outputs: BTreeMap<String, StagedProgramOutput>,
}

impl StagedPlanningContext {
    fn new(base_snapshot: &Snapshot) -> Self {
        Self {
            base_snapshot: base_snapshot.clone(),
            staged_snapshot: base_snapshot.clone(),
            commands: Vec::new(),
            staged_command_count: 0,
            operation_outputs: BTreeMap::new(),
            named_outputs: BTreeMap::new(),
        }
    }

    fn refresh(&mut self, operation: &str, target: &str) -> AssistantPlanningResult<()> {
        debug_assert!(self.commands.len() >= self.staged_command_count);
        if self.commands.len() == self.staged_command_count {
            return Ok(());
        }
        self.staged_snapshot = self
            .staged_snapshot
            .preview_batch(&CommandBatch::new(
                self.commands[self.staged_command_count..].to_vec(),
            ))
            .map_err(|error| assistant_canonical_rejection(error, operation, target))?;
        self.staged_command_count = self.commands.len();
        Ok(())
    }

    fn push(&mut self, command: CanonicalCommand) {
        self.commands.push(command);
    }

    fn extend(&mut self, commands: impl IntoIterator<Item = CanonicalCommand>) {
        self.commands.extend(commands);
    }

    fn into_final_batch(self) -> CommandBatch {
        CommandBatch::new(self.commands)
    }

    fn base_snapshot(&self) -> &Snapshot {
        &self.base_snapshot
    }

    fn staged_snapshot(&self) -> &Snapshot {
        &self.staged_snapshot
    }

    fn record_output(&mut self, operation_index: usize, output: StagedProgramOutput) {
        let outputs = self.operation_outputs.entry(operation_index).or_default();
        match output {
            StagedProgramOutput::Definition(id) => outputs.definition = Some(id),
            StagedProgramOutput::Occurrence(id) => outputs.occurrence = Some(id),
            StagedProgramOutput::SketchFeature(id) => outputs.sketch_feature = Some(id),
            StagedProgramOutput::BodyFeature(id) => outputs.body_feature = Some(id),
            StagedProgramOutput::ConstructionFeature(id) => {
                outputs.construction_feature = Some(id);
            }
            StagedProgramOutput::AssemblyJoint(id) => outputs.assembly_joint = Some(id),
            StagedProgramOutput::DowelJoint(id) => outputs.dowel_joint = Some(id),
        }
    }

    fn resolve_program_output(
        &self,
        reference: AssistantCadProgramFeatureReference,
        expected_output: AssistantCadProgramFeatureOutput,
        operation: &str,
    ) -> AssistantPlanningResult<u64> {
        let output = self
            .operation_outputs
            .get(&(reference.operation_index as usize))
            .and_then(|outputs| match expected_output {
                AssistantCadProgramFeatureOutput::Definition => outputs.definition.map(|id| id.0),
                AssistantCadProgramFeatureOutput::Occurrence => outputs.occurrence.map(|id| id.0),
                AssistantCadProgramFeatureOutput::SketchFeature => {
                    outputs.sketch_feature.map(|id| id.0)
                }
                AssistantCadProgramFeatureOutput::BodyFeature => {
                    outputs.body_feature.map(|id| id.0)
                }
                AssistantCadProgramFeatureOutput::ConstructionFeature => {
                    outputs.construction_feature.map(|id| id.0)
                }
                AssistantCadProgramFeatureOutput::AssemblyJoint => {
                    outputs.assembly_joint.map(|id| id.0)
                }
                AssistantCadProgramFeatureOutput::DowelJoint => outputs.dowel_joint.map(|id| id.0),
            })
            .filter(|_| reference.output == expected_output);
        output.ok_or_else(|| {
            assistant_planning_rejection(
                "planning.cad_program_feature_reference_unavailable",
                operation,
                &format!("operation:{}", reference.operation_index),
                "The referenced earlier operation did not produce the required typed output.",
                "Reference a compatible typed output from an earlier operation in this CAD program.",
            )
        })
    }

    fn bind_named_output(
        &mut self,
        name: &str,
        source: AssistantCadProgramFeatureReference,
        operation: &str,
    ) -> AssistantPlanningResult<()> {
        let id = self.resolve_program_output(source, source.output, operation)?;
        let output = match source.output {
            AssistantCadProgramFeatureOutput::Definition => {
                StagedProgramOutput::Definition(DefinitionId(id))
            }
            AssistantCadProgramFeatureOutput::Occurrence => {
                StagedProgramOutput::Occurrence(OccurrenceId(id))
            }
            AssistantCadProgramFeatureOutput::SketchFeature => {
                StagedProgramOutput::SketchFeature(FeatureId(id))
            }
            AssistantCadProgramFeatureOutput::ConstructionFeature => {
                StagedProgramOutput::ConstructionFeature(FeatureId(id))
            }
            AssistantCadProgramFeatureOutput::BodyFeature => {
                StagedProgramOutput::BodyFeature(FeatureId(id))
            }
            AssistantCadProgramFeatureOutput::AssemblyJoint => {
                StagedProgramOutput::AssemblyJoint(AssemblyJointId(id))
            }
            AssistantCadProgramFeatureOutput::DowelJoint => {
                StagedProgramOutput::DowelJoint(DowelJointId(id))
            }
        };
        if self.named_outputs.insert(name.to_owned(), output).is_some() {
            return Err(assistant_planning_rejection(
                "planning.cad_program_output_alias_duplicate",
                operation,
                name,
                "A program output alias was bound more than once.",
                "Use one unique alias for each bound program output.",
            ));
        }
        Ok(())
    }

    fn resolve_named_output(
        &self,
        reference: &AssistantCadNamedProgramOutputReference,
        expected_output: AssistantCadProgramFeatureOutput,
        operation: &str,
    ) -> AssistantPlanningResult<u64> {
        let output = self.named_outputs.get(&reference.name).copied();
        let id = match (expected_output, output) {
            (
                AssistantCadProgramFeatureOutput::Definition,
                Some(StagedProgramOutput::Definition(id)),
            ) => Some(id.0),
            (
                AssistantCadProgramFeatureOutput::Occurrence,
                Some(StagedProgramOutput::Occurrence(id)),
            ) => Some(id.0),
            (
                AssistantCadProgramFeatureOutput::SketchFeature,
                Some(StagedProgramOutput::SketchFeature(id)),
            ) => Some(id.0),
            (
                AssistantCadProgramFeatureOutput::ConstructionFeature,
                Some(StagedProgramOutput::ConstructionFeature(id)),
            ) => Some(id.0),
            (
                AssistantCadProgramFeatureOutput::BodyFeature,
                Some(StagedProgramOutput::BodyFeature(id)),
            ) => Some(id.0),
            (
                AssistantCadProgramFeatureOutput::AssemblyJoint,
                Some(StagedProgramOutput::AssemblyJoint(id)),
            ) => Some(id.0),
            (
                AssistantCadProgramFeatureOutput::DowelJoint,
                Some(StagedProgramOutput::DowelJoint(id)),
            ) => Some(id.0),
            _ => None,
        };
        id.filter(|_| reference.output == expected_output)
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_named_program_output_unavailable",
                    operation,
                    &reference.name,
                    "The named earlier output is missing or has a different type.",
                    "Bind a unique compatible output before referencing it.",
                )
            })
    }

    fn resolved_named_outputs(&self) -> BTreeMap<String, AssistantCadResolvedProgramOutput> {
        self.named_outputs
            .iter()
            .map(|(name, output)| {
                let resolved = match output {
                    StagedProgramOutput::Definition(id) => {
                        AssistantCadResolvedProgramOutput::Definition(id.0)
                    }
                    StagedProgramOutput::Occurrence(id) => {
                        AssistantCadResolvedProgramOutput::Occurrence(id.0)
                    }
                    StagedProgramOutput::SketchFeature(id) => {
                        AssistantCadResolvedProgramOutput::SketchFeature(id.0)
                    }
                    StagedProgramOutput::ConstructionFeature(id) => {
                        AssistantCadResolvedProgramOutput::ConstructionFeature(id.0)
                    }
                    StagedProgramOutput::BodyFeature(id) => {
                        AssistantCadResolvedProgramOutput::BodyFeature(id.0)
                    }
                    StagedProgramOutput::AssemblyJoint(id) => {
                        AssistantCadResolvedProgramOutput::AssemblyJoint(id.0)
                    }
                    StagedProgramOutput::DowelJoint(id) => {
                        AssistantCadResolvedProgramOutput::DowelJoint(id.0)
                    }
                };
                (name.clone(), resolved)
            })
            .collect()
    }

    fn resolve_program_feature(
        &self,
        reference: AssistantCadFeatureReference,
        expected_output: AssistantCadProgramFeatureOutput,
        operation: &str,
    ) -> AssistantPlanningResult<u64> {
        match reference {
            AssistantCadFeatureReference::Existing(id)
                if self.base_snapshot.feature(FeatureId(id)).is_some() =>
            {
                Ok(id)
            }
            AssistantCadFeatureReference::Existing(id) => Err(assistant_canonical_rejection(
                CanonicalError::FeatureNotFound(FeatureId(id)),
                operation,
                &format!("feature:{id}"),
            )),
            AssistantCadFeatureReference::ProgramOutput(reference) => self
                .resolve_program_output(reference, expected_output, operation)
                .map_err(|_| {
                    assistant_planning_rejection(
                        "planning.cad_program_feature_reference_unavailable",
                        operation,
                        &format!("operation:{}", reference.operation_index),
                        "The referenced earlier operation did not produce the required typed feature.",
                        "Reference a compatible typed output from an earlier operation in this CAD program.",
                    )
                }),
        }
    }

    fn resolve_base_selector(
        &self,
        current_selection: &BTreeSet<OccurrenceId>,
        selector: &AssistantCadEntitySelector,
        operation: &str,
    ) -> AssistantPlanningResult<Vec<OccurrenceId>> {
        let ids = match selector {
            AssistantCadEntitySelector::CurrentSelection {} => {
                current_selection.iter().copied().collect::<Vec<_>>()
            }
            AssistantCadEntitySelector::Occurrences { occurrence_ids } => occurrence_ids
                .iter()
                .copied()
                .map(OccurrenceId)
                .collect::<Vec<_>>(),
        };
        selector
            .validate_resolved_target_count(ids.len())
            .map_err(|error| {
                assistant_planning_rejection(
                    "planning.cad_selector_invalid",
                    operation,
                    "occurrence_selection",
                    error,
                    "Select between one and 100 root occurrences that still exist, then retry.",
                )
            })?;
        if let Some(id) = ids
            .iter()
            .find(|id| self.base_snapshot().occurrence(**id).is_none())
        {
            return Err(assistant_canonical_rejection(
                CanonicalError::OccurrenceNotFound(*id),
                operation,
                &format!("occurrence:{}", id.0),
            ));
        }
        Ok(ids)
    }

    fn topology(&self, base_topology: &ExactResultRegistry) -> ExactResultRegistry {
        if self.staged_command_count == 0 {
            base_topology.clone()
        } else if base_topology.is_bound_to(&self.base_snapshot) {
            ExactResultRegistry::carried_forward(&self.staged_snapshot, base_topology)
        } else {
            ExactResultRegistry::default()
        }
    }
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

/// In-plane x axis for a workplane on an axis-aligned panel face.
fn panel_face_x_axis(normal: [f64; 3]) -> [f64; 3] {
    if normal[0].abs() > 0.5 {
        [0.0, 1.0, 0.0]
    } else if normal[1].abs() > 0.5 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    }
}

fn panel_hole_workplane(hole: &AssistantPanelHole) -> AssistantWorkplaneSpec {
    let normal = hole.inward_unit_local;
    let x_axis = panel_face_x_axis(normal);
    AssistantWorkplaneSpec::Frame {
        origin_mm: hole.entry_local_mm,
        x_axis,
        y_axis: cross(normal, x_axis),
    }
}

/// Workplane on the pocket's entry face and the pocket rectangle in it.
fn panel_pocket_sketch(pocket: &AssistantPanelPocket) -> (AssistantWorkplaneSpec, [f64; 4], f64) {
    let normal = pocket.inward_unit_local;
    let axis = pocket.axis().unwrap_or(2);
    let x_axis = panel_face_x_axis(normal);
    let y_axis = cross(normal, x_axis);
    let mut origin_mm = [0.0; 3];
    for index in 0..3 {
        origin_mm[index] = (pocket.min_local_mm[index] + pocket.max_local_mm[index]) * 0.5;
    }
    origin_mm[axis] = if normal[axis] > 0.0 {
        pocket.min_local_mm[axis]
    } else {
        pocket.max_local_mm[axis]
    };
    let project = |point: [f64; 3], direction: [f64; 3]| {
        (0..3)
            .map(|index| (point[index] - origin_mm[index]) * direction[index])
            .sum::<f64>()
    };
    let corners = [pocket.min_local_mm, pocket.max_local_mm];
    let xs = corners.map(|corner| project(corner, x_axis));
    let ys = corners.map(|corner| project(corner, y_axis));
    let rectangle = [
        xs[0].min(xs[1]),
        ys[0].min(ys[1]),
        xs[0].max(xs[1]),
        ys[0].max(ys[1]),
    ];
    let depth = pocket.max_local_mm[axis] - pocket.min_local_mm[axis];
    (
        AssistantWorkplaneSpec::Frame {
            origin_mm,
            x_axis,
            y_axis,
        },
        rectangle,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn plan_panel(
    name: &str,
    dimensions_mm: [f64; 3],
    holes: &[AssistantPanelHole],
    pockets: &[AssistantPanelPocket],
    translation_mm: [f64; 3],
    rotation: Option<ketchup_core::assistant_sidecar::AssistantCadRotation>,
    staged: &mut StagedPlanningContext,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    document_target: &str,
) -> AssistantPlanningResult<(DefinitionId, OccurrenceId, FeatureId, FeatureId)> {
    let [width, depth, thickness] = dimensions_mm;
    let base = AssistantCadEditOperation::CreatePart {
        name: name.to_owned(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![
            AssistantSketchEntity::Line {
                id: 1,
                start_mm: [0.0, 0.0],
                end_mm: [width, 0.0],
            },
            AssistantSketchEntity::Line {
                id: 2,
                start_mm: [width, 0.0],
                end_mm: [width, depth],
            },
            AssistantSketchEntity::Line {
                id: 3,
                start_mm: [width, depth],
                end_mm: [0.0, depth],
            },
            AssistantSketchEntity::Line {
                id: 4,
                start_mm: [0.0, depth],
                end_mm: [0.0, 0.0],
            },
        ],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion {
            distance_mm: thickness,
        },
        translation_mm,
        rotation,
    };
    let base_commands = plan_creation(
        staged.base_snapshot(),
        &base,
        next_definition,
        next_feature,
        next_occurrence,
        document_target,
    )?;
    let definition_id = base_commands
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::CreateDefinition { id, .. } => Some(*id),
            _ => None,
        })
        .expect("CreatePart always creates a definition");
    let occurrence_id = base_commands
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::CreateOccurrence { id, .. } => Some(*id),
            _ => None,
        })
        .expect("CreatePart always creates an occurrence");
    let base_sketch_id = base_commands
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::CreateFeature {
                id,
                kind: FeatureKind::Sketch(_),
                ..
            } => Some(*id),
            _ => None,
        })
        .expect("CreatePart always creates a sketch");
    let mut target_feature_id = base_commands
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::CreateFeature { id, kind, .. }
                if !matches!(kind, FeatureKind::Workplane(_) | FeatureKind::Sketch(_)) =>
            {
                Some(*id)
            }
            _ => None,
        })
        .expect("CreatePart always creates a body");
    staged.extend(base_commands);

    for hole in holes {
        staged.refresh("create_panel", document_target)?;
        let sketch = AssistantCadEditOperation::CreateSketch {
            definition_id: definition_id.0,
            name: format!("{name} hole {}", hole.id),
            workplane: panel_hole_workplane(hole),
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: hole.diameter_mm * 0.5,
            }],
            constraints: Vec::new(),
        };
        let sketch_commands = plan_creation(
            staged.staged_snapshot(),
            &sketch,
            next_definition,
            next_feature,
            next_occurrence,
            document_target,
        )?;
        let sketch_id = sketch_commands
            .iter()
            .find_map(|command| match command {
                CanonicalCommand::CreateFeature {
                    id,
                    kind: FeatureKind::Sketch(_),
                    ..
                } => Some(*id),
                _ => None,
            })
            .expect("CreateSketch always creates a sketch");
        staged.extend(sketch_commands);
        staged.refresh("create_panel", document_target)?;
        let feature = AssistantCadBodyFeature::Pocket {
            target_feature_id: target_feature_id.0,
            profile_feature_id: sketch_id.0,
            depth_mm: hole.depth_mm,
        };
        let kind = plan_feature_kind(
            staged.staged_snapshot(),
            &staged.topology(&ExactResultRegistry::default()),
            definition_id,
            &feature,
            "create_panel",
        )?;
        let id = next_feature.map(FeatureId).ok_or_else(|| {
            assistant_canonical_rejection(
                CanonicalError::IdExhausted,
                "create_panel",
                document_target,
            )
        })?;
        *next_feature = id.0.checked_add(1);
        staged.push(CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name: format!("{name} hole {} pocket", hole.id),
            kind,
        });
        target_feature_id = id;
    }
    for pocket in pockets {
        staged.refresh("create_panel", document_target)?;
        let (workplane, [x_min, y_min, x_max, y_max], depth_mm) = panel_pocket_sketch(pocket);
        let line = |id, start_mm, end_mm| AssistantSketchEntity::Line {
            id,
            start_mm,
            end_mm,
        };
        let sketch = AssistantCadEditOperation::CreateSketch {
            definition_id: definition_id.0,
            name: format!("{name} pocket {}", pocket.id),
            workplane,
            entities: vec![
                line(1, [x_min, y_min], [x_max, y_min]),
                line(2, [x_max, y_min], [x_max, y_max]),
                line(3, [x_max, y_max], [x_min, y_max]),
                line(4, [x_min, y_max], [x_min, y_min]),
            ],
            constraints: Vec::new(),
        };
        let sketch_commands = plan_creation(
            staged.staged_snapshot(),
            &sketch,
            next_definition,
            next_feature,
            next_occurrence,
            document_target,
        )?;
        let sketch_id = sketch_commands
            .iter()
            .find_map(|command| match command {
                CanonicalCommand::CreateFeature {
                    id,
                    kind: FeatureKind::Sketch(_),
                    ..
                } => Some(*id),
                _ => None,
            })
            .expect("CreateSketch always creates a sketch");
        staged.extend(sketch_commands);
        staged.refresh("create_panel", document_target)?;
        let feature = AssistantCadBodyFeature::Pocket {
            target_feature_id: target_feature_id.0,
            profile_feature_id: sketch_id.0,
            depth_mm,
        };
        let kind = plan_feature_kind(
            staged.staged_snapshot(),
            &staged.topology(&ExactResultRegistry::default()),
            definition_id,
            &feature,
            "create_panel",
        )?;
        let id = next_feature.map(FeatureId).ok_or_else(|| {
            assistant_canonical_rejection(
                CanonicalError::IdExhausted,
                "create_panel",
                document_target,
            )
        })?;
        *next_feature = id.0.checked_add(1);
        staged.push(CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name: format!("{name} pocket {} cut", pocket.id),
            kind,
        });
        target_feature_id = id;
    }
    Ok((
        definition_id,
        occurrence_id,
        base_sketch_id,
        target_feature_id,
    ))
}

fn sole_terminal_body_feature(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    operation: &str,
) -> AssistantPlanningResult<FeatureId> {
    let terminals = ExactSnapshotPreparation::new(snapshot)
        .and_then(|preparation| preparation.terminal_features(definition_id))
        .map_err(|error| {
            assistant_planning_rejection(
                "planning.physical_dowel_part_unsupported",
                operation,
                &format!("definition:{}", definition_id.0),
                format!("The participant body graph is unsupported: {error:?}."),
                "Use a participant with one supported terminal solid body.",
            )
        })?;
    if terminals.len() != 1 {
        return Err(assistant_planning_rejection(
            "planning.physical_dowel_part_ambiguous",
            operation,
            &format!("definition:{}", definition_id.0),
            "The participant must have exactly one terminal solid body.",
            "Select a single-body part before creating physical dowel joinery.",
        ));
    }
    Ok(*terminals.values().next().expect("one terminal body"))
}

fn point_on_segment(point: [f64; 3], start: [f64; 3], end: [f64; 3]) -> [f64; 3] {
    let direction = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let length_squared = direction.iter().map(|value| value * value).sum::<f64>();
    let offset = [
        point[0] - start[0],
        point[1] - start[1],
        point[2] - start[2],
    ];
    let parameter = if length_squared > 0.0 {
        offset
            .iter()
            .zip(direction)
            .map(|(left, right)| left * right)
            .sum::<f64>()
            / length_squared
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    [
        start[0] + direction[0] * parameter,
        start[1] + direction[1] * parameter,
        start[2] + direction[2] * parameter,
    ]
}

fn squared_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| (left - right) * (left - right))
        .sum()
}

fn circular_pocket_axis(
    snapshot: &Snapshot,
    feature_id: FeatureId,
) -> Option<([f64; 3], [f64; 3], f64)> {
    let feature = snapshot.feature(feature_id)?;
    let (profile_id, depth_mm) = match feature.kind() {
        FeatureKind::Pocket { profile, depth, .. } => (*profile, depth.millimetres()),
        _ => return None,
    };
    let sketch = match snapshot.feature(profile_id)?.kind() {
        FeatureKind::Sketch(sketch) => sketch,
        _ => return None,
    };
    let (center_mm, radius_mm) = match sketch.entities.as_slice() {
        [
            SketchEntity::Circle {
                center_mm,
                radius_mm,
                ..
            },
        ] => (*center_mm, *radius_mm),
        _ => return None,
    };
    let frame = match snapshot.feature(sketch.workplane)?.kind() {
        FeatureKind::Workplane(spec) => spec.frame,
        _ => return None,
    };
    let start = [
        frame.origin_mm[0] + frame.x_axis[0] * center_mm[0] + frame.y_axis[0] * center_mm[1],
        frame.origin_mm[1] + frame.x_axis[1] * center_mm[0] + frame.y_axis[1] * center_mm[1],
        frame.origin_mm[2] + frame.x_axis[2] * center_mm[0] + frame.y_axis[2] * center_mm[1],
    ];
    let end = [
        start[0] + frame.normal[0] * depth_mm,
        start[1] + frame.normal[1] * depth_mm,
        start[2] + frame.normal[2] * depth_mm,
    ];
    Some((start, end, radius_mm))
}

fn reject_colliding_physical_dowel_hole(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    hole: &DowelHole,
    operation: &str,
) -> AssistantPlanningResult<()> {
    let proposed_end = [
        hole.entry_local_mm[0] + hole.inward_unit_local[0] * hole.depth_mm,
        hole.entry_local_mm[1] + hole.inward_unit_local[1] * hole.depth_mm,
        hole.entry_local_mm[2] + hole.inward_unit_local[2] * hole.depth_mm,
    ];
    let proposed_radius = hole.diameter_mm * 0.5;
    let definition = snapshot.definition(definition_id).ok_or_else(|| {
        assistant_canonical_rejection(
            CanonicalError::DefinitionNotFound(definition_id),
            operation,
            &format!("definition:{}", definition_id.0),
        )
    })?;
    for feature_id in definition.feature_ids() {
        let Some((existing_start, existing_end, existing_radius)) =
            circular_pocket_axis(snapshot, *feature_id)
        else {
            continue;
        };
        let closest_existing = point_on_segment(hole.entry_local_mm, existing_start, existing_end);
        let closest_proposed =
            point_on_segment(closest_existing, hole.entry_local_mm, proposed_end);
        let closest_existing = point_on_segment(closest_proposed, existing_start, existing_end);
        if squared_distance(closest_proposed, closest_existing)
            < (proposed_radius + existing_radius).powi(2) - 1.0e-12
        {
            return Err(assistant_planning_rejection(
                "planning.physical_dowel_hole_collision",
                operation,
                &format!("feature:{}", feature_id.0),
                "A generated dowel hole would intersect an existing circular pocket.",
                "Move the dowel row clear of existing joinery or hardware.",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_physical_dowel_hole(
    hole: &DowelHole,
    definition_id: DefinitionId,
    target_feature_id: &mut FeatureId,
    staged: &mut StagedPlanningContext,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    document_target: &str,
) -> AssistantPlanningResult<FeatureId> {
    reject_colliding_physical_dowel_hole(
        staged.staged_snapshot(),
        definition_id,
        hole,
        "create_physical_dowel_joint",
    )?;
    let panel_hole = AssistantPanelHole {
        id: hole.stable_hole_id.clone(),
        entry_local_mm: hole.entry_local_mm,
        inward_unit_local: hole.inward_unit_local,
        diameter_mm: hole.diameter_mm,
        depth_mm: hole.depth_mm,
    };
    let sketch = AssistantCadEditOperation::CreateSketch {
        definition_id: definition_id.0,
        name: format!("{} sketch", hole.stable_hole_id),
        workplane: panel_hole_workplane(&panel_hole),
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: hole.diameter_mm * 0.5,
        }],
        constraints: Vec::new(),
    };
    let sketch_commands = plan_creation(
        staged.staged_snapshot(),
        &sketch,
        next_definition,
        next_feature,
        next_occurrence,
        document_target,
    )?;
    let sketch_id = sketch_commands
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::CreateFeature {
                id,
                kind: FeatureKind::Sketch(_),
                ..
            } => Some(*id),
            _ => None,
        })
        .expect("CreateSketch always creates a sketch");
    staged.extend(sketch_commands);
    staged.refresh("create_physical_dowel_joint", document_target)?;

    let feature = AssistantCadBodyFeature::Pocket {
        target_feature_id: target_feature_id.0,
        profile_feature_id: sketch_id.0,
        depth_mm: hole.depth_mm,
    };
    let kind = plan_feature_kind(
        staged.staged_snapshot(),
        &staged.topology(&ExactResultRegistry::default()),
        definition_id,
        &feature,
        "create_physical_dowel_joint",
    )?;
    let pocket_id = next_feature.map(FeatureId).ok_or_else(|| {
        assistant_canonical_rejection(
            CanonicalError::IdExhausted,
            "create_physical_dowel_joint",
            document_target,
        )
    })?;
    *next_feature = pocket_id.0.checked_add(1);
    staged.push(CanonicalCommand::CreateFeature {
        id: pocket_id,
        definition_id,
        name: format!("{} pocket", hole.stable_hole_id),
        kind,
    });
    *target_feature_id = pocket_id;
    Ok(pocket_id)
}

fn delete_owned_physical_dowel_joint(
    id: DowelJointId,
    staged: &mut StagedPlanningContext,
    operation: &str,
    document_target: &str,
) -> AssistantPlanningResult<()> {
    let snapshot = staged.staged_snapshot();
    let joint = snapshot.dowel_joint(id).ok_or_else(|| {
        assistant_planning_rejection(
            "planning.physical_dowel_joint_not_found",
            operation,
            &format!("dowel_joint:{}", id.0),
            "The requested physical dowel joint does not exist.",
            "Inspect the current joinery and retry with an existing joint ID.",
        )
    })?;
    let pairs = joint.physical_hole_pairs.as_ref().ok_or_else(|| {
        assistant_planning_rejection(
            "planning.physical_dowel_joint_not_owned",
            operation,
            &format!("dowel_joint:{}", id.0),
            "The joint does not declare physical hole ownership.",
            "Only update or remove holes created and owned by a physical dowel joint.",
        )
    })?;
    let graph = snapshot
        .feature_dependency_graph()
        .map_err(|error| assistant_canonical_rejection(error, operation, document_target))?;
    let pocket_ids = pairs
        .iter()
        .flat_map(|pair| [pair.first_pocket_feature_id, pair.second_pocket_feature_id])
        .collect::<BTreeSet<_>>();
    if pocket_ids.len() != pairs.len() * 2 {
        return Err(assistant_planning_rejection(
            "planning.physical_dowel_joint_ownership_ambiguous",
            operation,
            &format!("dowel_joint:{}", id.0),
            "The joint reuses one physical pocket in more than one owned pair.",
            "Repair the joint ownership before updating or removing it.",
        ));
    }
    if snapshot.dowel_joints().any(|other| {
        other.id != id
            && other
                .physical_hole_pairs
                .as_ref()
                .is_some_and(|other_pairs| {
                    other_pairs.iter().any(|pair| {
                        pocket_ids.contains(&pair.first_pocket_feature_id)
                            || pocket_ids.contains(&pair.second_pocket_feature_id)
                    })
                })
    }) {
        return Err(assistant_planning_rejection(
            "planning.physical_dowel_joint_ownership_shared",
            operation,
            &format!("dowel_joint:{}", id.0),
            "Another joint references at least one owned physical pocket.",
            "Separate shared ownership before updating or removing this joint.",
        ));
    }

    let mut owned = pocket_ids.clone();
    for pocket_id in &pocket_ids {
        let (profile_id, definition_id) = snapshot
            .feature(*pocket_id)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Pocket { profile, .. } => Some((*profile, feature.definition_id())),
                _ => None,
            })
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.physical_dowel_joint_ownership_invalid",
                    operation,
                    &format!("feature:{}", pocket_id.0),
                    "An owned physical hole is not a supported pocket feature.",
                    "Repair the physical joint before updating or removing it.",
                )
            })?;
        let workplane_id = snapshot
            .feature(profile_id)
            .filter(|feature| feature.definition_id() == definition_id)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Sketch(spec) => Some(spec.workplane),
                _ => None,
            })
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.physical_dowel_joint_ownership_invalid",
                    operation,
                    &format!("feature:{}", profile_id.0),
                    "An owned pocket does not use a supported sketch profile.",
                    "Repair the physical joint before updating or removing it.",
                )
            })?;
        let free_workplane = snapshot
            .feature(workplane_id)
            .filter(|feature| feature.definition_id() == definition_id)
            .is_some_and(|feature| {
                matches!(
                    feature.kind(),
                    FeatureKind::Workplane(spec) if spec.support == WorkplaneSupport::Free
                )
            });
        if !free_workplane || !owned.insert(profile_id) || !owned.insert(workplane_id) {
            return Err(assistant_planning_rejection(
                "planning.physical_dowel_joint_ownership_ambiguous",
                operation,
                &format!("dowel_joint:{}", id.0),
                "Owned pockets do not have distinct free-workplane sketch chains.",
                "Repair the joint ownership before updating or removing it.",
            ));
        }
    }
    for feature_id in &owned {
        if graph
            .dependents(*feature_id)
            .is_some_and(|dependents| dependents.iter().any(|id| !owned.contains(id)))
        {
            return Err(assistant_planning_rejection(
                "planning.physical_dowel_joint_has_foreign_dependents",
                operation,
                &format!("feature:{}", feature_id.0),
                "A manual or separately owned feature depends on this joint's hole chain.",
                "Remove or rebase the foreign dependent before changing this joint.",
            ));
        }
    }

    staged.push(CanonicalCommand::DeleteDowelJoint { id });
    for feature_id in graph
        .topological_order()
        .iter()
        .rev()
        .filter(|feature_id| owned.contains(feature_id))
    {
        staged.push(CanonicalCommand::DeleteFeature { id: *feature_id });
    }
    staged.refresh(operation, document_target)
}

fn resolve_assistant_instance_path(
    path: &AssistantInstancePath,
    snapshot: &Snapshot,
) -> Option<InstancePath> {
    let mut resolved = InstancePath::root(OccurrenceId(path.root_occurrence_id));
    let mut owner_definition_id = snapshot
        .occurrence(resolved.root_occurrence())?
        .definition_id();
    for step in &path.steps {
        let (declared_owner, path_step) = match *step {
            AssistantInstancePathStep::Group {
                owner_definition_id,
                local_id,
            } => (
                DefinitionId(owner_definition_id),
                InstancePathStep::Group(LocalGroupId(local_id)),
            ),
            AssistantInstancePathStep::Occurrence {
                owner_definition_id,
                local_id,
            } => (
                DefinitionId(owner_definition_id),
                InstancePathStep::Occurrence(LocalOccurrenceId(local_id)),
            ),
        };
        if declared_owner != owner_definition_id {
            return None;
        }
        resolved = resolved.with_step(path_step);
        owner_definition_id = snapshot
            .resolve_instance_path(&resolved)
            .ok()?
            .definition_id;
    }
    snapshot.resolve_instance_path(&resolved).ok()?;
    Some(resolved)
}

fn assembly_joint_axis(axis: AssistantAssemblyJointAxis) -> AssemblyJointAxis {
    AssemblyJointAxis::new(axis.direction_in_parent, axis.pivot_in_parent_mm)
}

fn assembly_joint_limits(
    limits: Option<AssistantAssemblyJointLimits>,
) -> Option<AssemblyJointLimits> {
    limits.map(|limits| AssemblyJointLimits::new(limits.min, limits.max))
}

fn assembly_joint_kind(kind: AssistantAssemblyJointKind) -> AssemblyJointKind {
    match kind {
        AssistantAssemblyJointKind::Fixed => AssemblyJointKind::Fixed,
        AssistantAssemblyJointKind::Revolute {
            axis,
            limits,
            position_degrees,
        } => AssemblyJointKind::Revolute {
            axis: assembly_joint_axis(axis),
            limits: assembly_joint_limits(limits),
            position_degrees,
        },
        AssistantAssemblyJointKind::Prismatic {
            axis,
            limits,
            position_mm,
        } => AssemblyJointKind::Prismatic {
            axis: assembly_joint_axis(axis),
            limits: assembly_joint_limits(limits),
            position_mm,
        },
        AssistantAssemblyJointKind::Helical {
            axis,
            limits,
            lead_mm_per_revolution,
            position_degrees,
        } => AssemblyJointKind::Helical {
            axis: assembly_joint_axis(axis),
            limits: assembly_joint_limits(limits),
            lead_mm_per_revolution,
            position_degrees,
        },
    }
}

fn transform_axis_to_world(
    transform: Transform,
    origin_mm: [f64; 3],
    direction: [f64; 3],
    requires_similarity: bool,
) -> Option<([f64; 3], [f64; 3])> {
    let matrix = transform.matrix();
    if requires_similarity {
        let columns = [
            [matrix[0], matrix[4], matrix[8]],
            [matrix[1], matrix[5], matrix[9]],
            [matrix[2], matrix[6], matrix[10]],
        ];
        let squared_lengths =
            columns.map(|column| column.iter().map(|value| value * value).sum::<f64>());
        let maximum = squared_lengths.into_iter().fold(0.0_f64, f64::max);
        if !maximum.is_finite()
            || maximum <= f64::EPSILON
            || squared_lengths
                .into_iter()
                .any(|length| (length - maximum).abs() > maximum * 1.0e-9)
            || [(0, 1), (0, 2), (1, 2)].into_iter().any(|(left, right)| {
                let dot = columns[left]
                    .iter()
                    .zip(columns[right])
                    .map(|(left, right)| left * right)
                    .sum::<f64>();
                dot.abs() > maximum * 1.0e-9
            })
        {
            return None;
        }
    }
    let point = |row: usize| {
        matrix[row * 4] * origin_mm[0]
            + matrix[row * 4 + 1] * origin_mm[1]
            + matrix[row * 4 + 2] * origin_mm[2]
            + matrix[row * 4 + 3]
    };
    let vector = |row: usize| {
        matrix[row * 4] * direction[0]
            + matrix[row * 4 + 1] * direction[1]
            + matrix[row * 4 + 2] * direction[2]
    };
    let axis = AssistantAxisSpec::OriginDirection {
        origin_mm: [point(0), point(1), point(2)],
        direction: [vector(0), vector(1), vector(2)],
    };
    axis.origin_and_direction().ok()
}

fn resolve_assistant_axis_spec(
    axis: &AssistantAxisSpec,
    original_snapshot: &Snapshot,
    topology_results: &ExactResultRegistry,
    staged_planning: &StagedPlanningContext,
    operations: &[AssistantCadEditOperation],
    operation: &str,
) -> AssistantPlanningResult<([f64; 3], [f64; 3])> {
    if let AssistantAxisSpec::Edge {
        edge_reference_id,
        instance_path,
    } = axis
    {
        let packages = topology_results
            .body_values(original_snapshot)
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_axis_reference_unavailable",
                    operation,
                    "axis.edge_reference_id",
                    "The current exact topology evidence is unavailable.",
                    "Evaluate the current document and use a listed line or circle edge reference.",
                )
            })?;
        let mut matches = Vec::new();
        for package in packages.into_values() {
            let ExactBodyPackage::Graph(package) = package.as_ref() else {
                continue;
            };
            for evidence in &package.edge_evidence {
                let reference = package
                    .topological_references
                    .iter()
                    .filter(|reference| reference.kind == TopologicalElementKind::Edge)
                    .nth(evidence.edge_ordinal as usize);
                if reference.is_some_and(|reference| {
                    reference.lineage_digest == *edge_reference_id
                        && reference.document_id == original_snapshot.document_id()
                        && reference.source_feature_id == reference.producer_feature_id
                        && reference.has_valid_lineage()
                }) && matches!(evidence.curve_kind.as_str(), "line" | "circle")
                    && let (Some(origin_mm), Some(direction)) =
                        (evidence.axis_origin_mm, evidence.unit_axis_direction)
                {
                    matches.push((
                        DefinitionId(package.graph.definition_id),
                        origin_mm,
                        direction,
                        evidence.curve_kind == "circle",
                    ));
                }
            }
        }
        let [(definition_id, origin_mm, direction, requires_similarity)] = matches.as_slice()
        else {
            return Err(assistant_planning_rejection(
                "planning.cad_axis_reference_unavailable",
                operation,
                "axis.edge_reference_id",
                "The edge reference is not one unique current line or circle edge with exact axis evidence.",
                "Refresh exact edge inspection and copy one current line or circle reference_id.",
            ));
        };
        let occurrences = original_snapshot
            .scene_query_bounded(
                MAX_AXIS_INSTANCE_OCCURRENCES,
                MAX_AXIS_INSTANCE_PATH_STEPS,
                MAX_AXIS_INSTANCE_TEXT_BYTES,
            )
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_axis_instance_path_unavailable",
                    operation,
                    "axis.instance_path",
                    "The bounded visible instance index is unavailable.",
                    "Reduce instance nesting or model size, then retry with one current instance_path.",
                )
            })?;
        let resolved_path = if let Some(path) = instance_path {
            resolve_assistant_instance_path(path, original_snapshot)
        } else {
            let mut candidates = occurrences
                .iter()
                .filter(|occurrence| occurrence.visible && occurrence.definition_id == *definition_id)
                .map(|occurrence| occurrence.instance_path.clone());
            let candidate = candidates.next();
            candidate.filter(|_| candidates.next().is_none())
        }
        .ok_or_else(|| {
            assistant_planning_rejection(
                "planning.cad_axis_instance_path_unavailable",
                operation,
                "axis.instance_path",
                "The edge axis does not identify one current visible instance.",
                "Copy the exact instance_path from current instance inspection; repeated definitions require it.",
            )
        })?;
        let resolved = original_snapshot
            .resolve_instance_path(&resolved_path)
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_axis_instance_path_unavailable",
                    operation,
                    "axis.instance_path",
                    "The edge axis instance path is not current.",
                    "Refresh instance inspection and copy one current instance_path.",
                )
            })?;
        if resolved.definition_id != *definition_id
            || !occurrences
                .iter()
                .any(|occurrence| occurrence.visible && occurrence.instance_path == resolved_path)
        {
            return Err(assistant_planning_rejection(
                "planning.cad_axis_instance_path_unavailable",
                operation,
                "axis.instance_path",
                "The edge reference and visible instance path resolve to different definitions.",
                "Use an instance_path whose definition_id matches the inspected edge.",
            ));
        }
        return transform_axis_to_world(
            resolved.world_transform,
            *origin_mm,
            *direction,
            *requires_similarity,
        )
        .ok_or_else(|| {
            assistant_planning_rejection(
                "planning.cad_axis_instance_transform_invalid",
                operation,
                "axis.instance_path",
                "The visible edge axis could not be transformed into a finite world axis.",
                "Use a current instance with a finite non-degenerate transform.",
            )
        });
    }
    let AssistantAxisSpec::ConstructionAxis { axis } = axis else {
        return axis.origin_and_direction().map_err(|error| {
            assistant_planning_rejection(
                "planning.cad_axis_invalid",
                operation,
                "axis",
                error,
                "Use two distinct bounded points, a finite non-zero origin and direction, or one current exact edge reference.",
            )
        });
    };
    let (feature_id, construction_axis) = match axis {
        AssistantCadFeatureReference::Existing(id) => {
            let feature_id = FeatureId(*id);
            let feature = original_snapshot.feature(feature_id).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(feature_id),
                    operation,
                    &format!("feature:{id}"),
                )
            })?;
            (feature_id, feature.kind())
        }
        AssistantCadFeatureReference::ProgramOutput(reference) => {
            let id = staged_planning.resolve_program_output(
                *reference,
                AssistantCadProgramFeatureOutput::ConstructionFeature,
                operation,
            )?;
            let Some(AssistantCadEditOperation::CreateConstructionAxis {
                origin_mm,
                direction,
                ..
            }) = operations.get(reference.operation_index as usize)
            else {
                return Err(assistant_planning_rejection(
                    "planning.cad_axis_reference_invalid",
                    operation,
                    &format!("feature:{id}"),
                    "The referenced construction output is not an axis.",
                    "Reference a ConstructionFeature output from create_construction_axis.",
                ));
            };
            return Ok((*origin_mm, *direction));
        }
    };
    let FeatureKind::ConstructionAxis {
        origin_mm,
        direction,
    } = construction_axis
    else {
        return Err(assistant_planning_rejection(
            "planning.cad_axis_reference_invalid",
            operation,
            &format!("feature:{}", feature_id.0),
            "The referenced feature is not a construction axis.",
            "Reference an existing ConstructionAxis feature.",
        ));
    };
    Ok((*origin_mm, *direction))
}

fn resolve_staged_program_feature_references(
    feature: &AssistantCadBodyFeature,
    staged_planning: &StagedPlanningContext,
    operation: &str,
) -> AssistantPlanningResult<AssistantCadBodyFeature> {
    let mut feature = feature.clone();
    if let AssistantCadBodyFeature::Boolean {
        target_feature_id,
        tool_feature_id,
        ..
    } = &mut feature
    {
        let target = staged_planning.resolve_program_feature(
            *target_feature_id,
            AssistantCadProgramFeatureOutput::BodyFeature,
            operation,
        )?;
        let tool = staged_planning.resolve_program_feature(
            *tool_feature_id,
            AssistantCadProgramFeatureOutput::BodyFeature,
            operation,
        )?;
        if target == tool {
            return Err(assistant_planning_rejection(
                "planning.cad_feature_inputs_identical",
                operation,
                &format!("feature:{target}"),
                "The resolved Boolean operands refer to the same body feature.",
                "Use two distinct existing or earlier program body features.",
            ));
        }
        *target_feature_id = target.into();
        *tool_feature_id = tool.into();
    }
    if let AssistantCadBodyFeature::WeldmentJoint {
        first_member_id,
        second_member_id,
        ..
    } = &mut feature
    {
        let first = staged_planning.resolve_program_feature(
            *first_member_id,
            AssistantCadProgramFeatureOutput::BodyFeature,
            operation,
        )?;
        let second = staged_planning.resolve_program_feature(
            *second_member_id,
            AssistantCadProgramFeatureOutput::BodyFeature,
            operation,
        )?;
        if first == second {
            return Err(assistant_planning_rejection(
                "planning.cad_feature_inputs_identical",
                operation,
                &format!("feature:{first}"),
                "The resolved weldment joint inputs refer to the same member.",
                "Use two distinct existing or earlier program weldment members.",
            ));
        }
        *first_member_id = first.into();
        *second_member_id = second.into();
    }
    if let AssistantCadBodyFeature::Loft {
        sections,
        guide_feature_id,
        ..
    } = &mut feature
    {
        for section in sections {
            section.profile_feature_id = staged_planning
                .resolve_program_feature(
                    section.profile_feature_id,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                    operation,
                )?
                .into();
        }
        if let Some(guide) = guide_feature_id {
            *guide = staged_planning
                .resolve_program_feature(
                    *guide,
                    AssistantCadProgramFeatureOutput::ConstructionFeature,
                    operation,
                )?
                .into();
        }
    }
    match &mut feature {
        AssistantCadBodyFeature::SurfaceBody { source } => match source {
            AssistantCadSurfaceBodySource::Planar { profile_feature_id } => {
                *profile_feature_id = staged_planning
                    .resolve_program_feature(
                        *profile_feature_id,
                        AssistantCadProgramFeatureOutput::SketchFeature,
                        operation,
                    )?
                    .into();
            }
            AssistantCadSurfaceBodySource::Loft {
                sections,
                guide_feature_id,
                ..
            } => {
                for section in sections {
                    section.profile_feature_id = staged_planning
                        .resolve_program_feature(
                            section.profile_feature_id,
                            AssistantCadProgramFeatureOutput::SketchFeature,
                            operation,
                        )?
                        .into();
                }
                if let Some(guide) = guide_feature_id {
                    *guide = staged_planning
                        .resolve_program_feature(
                            *guide,
                            AssistantCadProgramFeatureOutput::ConstructionFeature,
                            operation,
                        )?
                        .into();
                }
            }
        },
        AssistantCadBodyFeature::SurfaceTrim {
            target_feature_id,
            cutter_feature_id,
        } => {
            *target_feature_id = staged_planning
                .resolve_program_feature(
                    *target_feature_id,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                    operation,
                )?
                .into();
            *cutter_feature_id = staged_planning
                .resolve_program_feature(
                    *cutter_feature_id,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                    operation,
                )?
                .into();
        }
        AssistantCadBodyFeature::SurfaceExtend {
            target_feature_id, ..
        }
        | AssistantCadBodyFeature::SurfaceThicken {
            target_feature_id, ..
        } => {
            *target_feature_id = staged_planning
                .resolve_program_feature(
                    *target_feature_id,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                    operation,
                )?
                .into();
        }
        AssistantCadBodyFeature::SurfaceKnit {
            surface_feature_ids,
            ..
        } => {
            for reference in surface_feature_ids {
                *reference = staged_planning
                    .resolve_program_feature(
                        *reference,
                        AssistantCadProgramFeatureOutput::BodyFeature,
                        operation,
                    )?
                    .into();
            }
        }
        _ => {}
    }
    Ok(feature)
}

fn plan_assistant_construction_creation(
    name: &str,
    feature_suffix: &str,
    feature_kind: FeatureKind,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    rejection_context: (&str, &str),
) -> AssistantPlanningResult<(Vec<CanonicalCommand>, DefinitionId, FeatureId)> {
    let (operation, document_target) = rejection_context;
    let mut exhausted =
        || assistant_canonical_rejection(CanonicalError::IdExhausted, operation, document_target);
    let definition_id = next_definition
        .map(DefinitionId)
        .ok_or_else(&mut exhausted)?;
    *next_definition = definition_id.0.checked_add(1);
    let construction_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = construction_feature_id.0.checked_add(1);
    let occurrence_id = next_occurrence
        .map(OccurrenceId)
        .ok_or_else(&mut exhausted)?;
    *next_occurrence = occurrence_id.0.checked_add(1);
    Ok((
        vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: construction_feature_id,
                definition_id,
                name: format!("{name} {feature_suffix}"),
                kind: feature_kind,
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: name.to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ],
        definition_id,
        construction_feature_id,
    ))
}

fn plan_assistant_helix_thread_creation(
    name: &str,
    profile_segments: Vec<ProfileSegment>,
    path_segments: Vec<SpatialPathSegment>,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    rejection_context: (&str, &str),
) -> AssistantPlanningResult<(Vec<CanonicalCommand>, DefinitionId, FeatureId)> {
    let (operation, document_target) = rejection_context;
    let mut exhausted =
        || assistant_canonical_rejection(CanonicalError::IdExhausted, operation, document_target);
    let definition_id = next_definition
        .map(DefinitionId)
        .ok_or_else(&mut exhausted)?;
    *next_definition = definition_id.0.checked_add(1);
    let profile_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = profile_feature_id.0.checked_add(1);
    let path_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = path_feature_id.0.checked_add(1);
    let body_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = body_feature_id.0.checked_add(1);
    let occurrence_id = next_occurrence
        .map(OccurrenceId)
        .ok_or_else(&mut exhausted)?;
    *next_occurrence = occurrence_id.0.checked_add(1);
    Ok((
        vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile_feature_id,
                definition_id,
                name: format!("{name} profile"),
                kind: FeatureKind::SegmentProfile {
                    segments: profile_segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: path_feature_id,
                definition_id,
                name: format!("{name} path"),
                kind: FeatureKind::SpatialPath {
                    segments: path_segments,
                },
            },
            CanonicalCommand::CreateFeature {
                id: body_feature_id,
                definition_id,
                name: format!("{name} body"),
                kind: FeatureKind::Sweep {
                    profile: profile_feature_id,
                    path: path_feature_id,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: name.to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ],
        definition_id,
        body_feature_id,
    ))
}

/// Plans against the current document and explicit host-provided context without mutation.
///
/// An empty selection is valid unless an operation uses `CurrentSelection`.
/// Host selection and explicit occurrence selectors are resolved only against the
/// base snapshot; references to outputs of earlier program operations resolve only
/// against the staged context. Topology references must be current in the supplied
/// registry. The returned batch still requires canonical
/// preview/proposal validation before commit; this function retains the existing
/// appended-feature exact graph and planar-offset preflight gates, not worker execution.
pub fn plan_assistant_cad_edit_program_with_outputs(
    document: &DocumentStore,
    current_selection: &BTreeSet<OccurrenceId>,
    topology_results: &ExactResultRegistry,
    program: &AssistantCadEditProgram,
) -> Result<AssistantCadProgramPlan, Box<AssistantRejectionDiagnostic>> {
    let snapshot = document.current();
    let document_target = format!("document:{}", snapshot.document_id().0);
    program.validate().map_err(|error| {
        assistant_rejection(
            AssistantRejectionPhase::IntentValidation,
            "intent.cad_edit_program_invalid",
            "cad_edit_program",
            &document_target,
            error,
            "Return a bounded CAD edit program that satisfies the Assistant schema invariants.",
            true,
        )
    })?;

    let mut staged_planning = StagedPlanningContext::new(&snapshot);
    let mut appended_exact_features = Vec::new();
    let mut appended_planar_offsets = Vec::new();
    let mut next_definition = snapshot
        .definitions()
        .map(|definition| definition.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_occurrence = snapshot
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_feature = snapshot
        .features()
        .map(|feature| feature.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_assembly_joint = snapshot
        .assembly_joints()
        .map(|joint| joint.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_dowel_joint = snapshot
        .dowel_joints()
        .map(|joint| joint.id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_drawing_sheet = snapshot
        .drawing_sheets()
        .map(|sheet| sheet.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);

    for (operation_index, operation) in program.operations.iter().enumerate() {
        let operation_name = match operation {
            AssistantCadEditOperation::CreateSketch { .. } => "create_sketch",
            AssistantCadEditOperation::CreateProgramSketch { .. } => "create_program_sketch",
            AssistantCadEditOperation::CreatePart { .. } => "create_part",
            AssistantCadEditOperation::CreatePanel { .. } => "create_panel",
            AssistantCadEditOperation::CreateSpatialPath { .. } => "create_spatial_path",
            AssistantCadEditOperation::CreateHelixPath { .. } => "create_helix_path",
            AssistantCadEditOperation::CreateConstructionPoint { .. } => {
                "create_construction_point"
            }
            AssistantCadEditOperation::CreateConstructionAxis { .. } => "create_construction_axis",
            AssistantCadEditOperation::CreateConstructionPlane { .. } => {
                "create_construction_plane"
            }
            AssistantCadEditOperation::CreateHelix { .. } => "create_helix",
            AssistantCadEditOperation::CreateThread { .. } => "create_thread",
            AssistantCadEditOperation::FilletEdges { .. } => "fillet_edges",
            AssistantCadEditOperation::ChamferEdges { .. } => "chamfer_edges",
            AssistantCadEditOperation::AppendFeature { .. } => "append_feature",
            AssistantCadEditOperation::AppendProgramPocket { .. } => "append_program_pocket",
            AssistantCadEditOperation::BindProgramOutput { .. } => "bind_program_output",
            AssistantCadEditOperation::SetDimension { .. } => "set_dimension",
            AssistantCadEditOperation::SetFeatureParameter { .. } => "set_feature_parameter",
            AssistantCadEditOperation::MakeOccurrenceUnique { .. } => "make_occurrence_unique",
            AssistantCadEditOperation::CreateAssemblyJoint { .. } => "create_assembly_joint",
            AssistantCadEditOperation::CreateDowelJoint { .. } => "create_dowel_joint",
            AssistantCadEditOperation::CreateProgramDowelJoint { .. } => {
                "create_program_dowel_joint"
            }
            AssistantCadEditOperation::CreatePhysicalDowelJoint { .. } => {
                "create_physical_dowel_joint"
            }
            AssistantCadEditOperation::DeletePhysicalDowelJoint { .. } => {
                "delete_physical_dowel_joint"
            }
            AssistantCadEditOperation::SetAssemblyJointPosition { .. } => {
                "set_assembly_joint_position"
            }
            AssistantCadEditOperation::CreateDrawing { .. } => "create_drawing",
            AssistantCadEditOperation::UpsertCamPlan { .. } => "upsert_cam_plan",
            AssistantCadEditOperation::Delete { .. } => "delete_occurrence",
            AssistantCadEditOperation::Transform { .. } => "transform_occurrence",
            AssistantCadEditOperation::SetColor { .. } => "set_color",
            AssistantCadEditOperation::SetGrounded { .. } => "set_grounded",
            AssistantCadEditOperation::CreateTag { .. } => "create_tag",
            AssistantCadEditOperation::SetOccurrenceTag { .. } => "set_occurrence_tag",
            AssistantCadEditOperation::SetTagVisibility { .. } => "set_tag_visibility",
            AssistantCadEditOperation::UpsertClassificationDimension { .. } => {
                "upsert_classification_dimension"
            }
            AssistantCadEditOperation::SetOccurrenceClassification { .. } => {
                "set_occurrence_classification"
            }
            AssistantCadEditOperation::CreateEvaluatorInput { .. } => "create_evaluator_input",
            AssistantCadEditOperation::Copy { .. } => "copy_occurrence",
            AssistantCadEditOperation::LinearPattern { .. } => "linear_pattern_occurrence",
            AssistantCadEditOperation::CircularPattern { .. } => "circular_pattern_occurrence",
            AssistantCadEditOperation::Mirror { .. } => "mirror_occurrence",
        };
        staged_planning.refresh(operation_name, &document_target)?;
        let normalized_operation = match operation {
            AssistantCadEditOperation::CreatePart {
                feature: AssistantCadPartFeature::Revolve { axis, .. },
                ..
            } => {
                let (origin_mm, direction) = resolve_assistant_axis_spec(
                    axis,
                    &snapshot,
                    topology_results,
                    &staged_planning,
                    &program.operations,
                    operation_name,
                )?;
                let mut resolved = operation.clone();
                let AssistantCadEditOperation::CreatePart {
                    feature: AssistantCadPartFeature::Revolve { axis, .. },
                    ..
                } = &mut resolved
                else {
                    unreachable!("matched Revolve part")
                };
                *axis = AssistantAxisSpec::OriginDirection {
                    origin_mm,
                    direction,
                };
                Some(resolved)
            }
            AssistantCadEditOperation::CircularPattern { axis, .. } => {
                let (origin_mm, direction) = resolve_assistant_axis_spec(
                    axis,
                    &snapshot,
                    topology_results,
                    &staged_planning,
                    &program.operations,
                    operation_name,
                )?;
                let mut resolved = operation.clone();
                let AssistantCadEditOperation::CircularPattern { axis, .. } = &mut resolved else {
                    unreachable!("matched circular pattern")
                };
                *axis = AssistantAxisSpec::OriginDirection {
                    origin_mm,
                    direction,
                };
                Some(resolved)
            }
            AssistantCadEditOperation::FilletEdges {
                definition_id,
                name,
                target_feature_id,
                edge_reference_ids,
                radius_mm,
            } => Some(AssistantCadEditOperation::AppendFeature {
                definition_id: *definition_id,
                name: name.clone(),
                feature: AssistantCadBodyFeature::TopologyFillet {
                    target_feature_id: *target_feature_id,
                    edge_reference_ids: edge_reference_ids.clone(),
                    radius_mm: *radius_mm,
                    radius_stations: Vec::new(),
                },
            }),
            AssistantCadEditOperation::ChamferEdges {
                definition_id,
                name,
                target_feature_id,
                edge_reference_ids,
                distance_mm,
            } => Some(AssistantCadEditOperation::AppendFeature {
                definition_id: *definition_id,
                name: name.clone(),
                feature: AssistantCadBodyFeature::TopologyChamfer {
                    target_feature_id: *target_feature_id,
                    edge_reference_ids: edge_reference_ids.clone(),
                    distance_mm: *distance_mm,
                    mode: Default::default(),
                    side_face_reference_ids: Vec::new(),
                },
            }),
            _ => None,
        };
        let operation = normalized_operation.as_ref().unwrap_or(operation);
        let selector = match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreateProgramSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. }
            | AssistantCadEditOperation::CreatePanel { .. }
            | AssistantCadEditOperation::CreateSpatialPath { .. }
            | AssistantCadEditOperation::CreateHelixPath { .. }
            | AssistantCadEditOperation::CreateConstructionPoint { .. }
            | AssistantCadEditOperation::CreateConstructionAxis { .. }
            | AssistantCadEditOperation::CreateConstructionPlane { .. }
            | AssistantCadEditOperation::CreateHelix { .. }
            | AssistantCadEditOperation::CreateThread { .. }
            | AssistantCadEditOperation::FilletEdges { .. }
            | AssistantCadEditOperation::ChamferEdges { .. }
            | AssistantCadEditOperation::AppendFeature { .. }
            | AssistantCadEditOperation::AppendProgramPocket { .. }
            | AssistantCadEditOperation::BindProgramOutput { .. }
            | AssistantCadEditOperation::SetDimension { .. }
            | AssistantCadEditOperation::SetFeatureParameter { .. }
            | AssistantCadEditOperation::MakeOccurrenceUnique { .. }
            | AssistantCadEditOperation::CreateAssemblyJoint { .. }
            | AssistantCadEditOperation::CreateDowelJoint { .. }
            | AssistantCadEditOperation::CreateProgramDowelJoint { .. }
            | AssistantCadEditOperation::CreatePhysicalDowelJoint { .. }
            | AssistantCadEditOperation::DeletePhysicalDowelJoint { .. }
            | AssistantCadEditOperation::SetAssemblyJointPosition { .. }
            | AssistantCadEditOperation::CreateDrawing { .. }
            | AssistantCadEditOperation::UpsertCamPlan { .. }
            | AssistantCadEditOperation::CreateTag { .. }
            | AssistantCadEditOperation::SetTagVisibility { .. }
            | AssistantCadEditOperation::UpsertClassificationDimension { .. }
            | AssistantCadEditOperation::CreateEvaluatorInput { .. } => None,
            AssistantCadEditOperation::Delete { selector, .. }
            | AssistantCadEditOperation::Transform { selector, .. }
            | AssistantCadEditOperation::SetColor { selector, .. }
            | AssistantCadEditOperation::SetGrounded { selector, .. }
            | AssistantCadEditOperation::SetOccurrenceTag { selector, .. }
            | AssistantCadEditOperation::SetOccurrenceClassification { selector, .. }
            | AssistantCadEditOperation::Copy { selector, .. }
            | AssistantCadEditOperation::LinearPattern { selector, .. }
            | AssistantCadEditOperation::CircularPattern { selector, .. }
            | AssistantCadEditOperation::Mirror { selector, .. } => Some(selector),
        };
        let targets = selector.map_or_else(
            || Ok(Vec::new()),
            |selector| {
                staged_planning.resolve_base_selector(current_selection, selector, operation_name)
            },
        )?;
        if let Some(id) = targets
            .iter()
            .find(|id| staged_planning.staged_snapshot().occurrence(**id).is_none())
        {
            return Err(assistant_planning_rejection(
                "planning.cad_target_deleted",
                operation_name,
                &format!("occurrence:{}", id.0),
                "An earlier operation in this CAD edit program already deleted the target.",
                "Remove the later operation or target an occurrence that remains in the program.",
            ));
        }

        match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. } => {
                let creation_commands = plan_creation(
                    &snapshot,
                    operation,
                    &mut next_definition,
                    &mut next_feature,
                    &mut next_occurrence,
                    &document_target,
                )?;
                for command in &creation_commands {
                    match command {
                        CanonicalCommand::CreateDefinition { id, .. } => {
                            staged_planning.record_output(
                                operation_index,
                                StagedProgramOutput::Definition(*id),
                            );
                        }
                        CanonicalCommand::CreateFeature {
                            id,
                            kind: ketchup_core::document::FeatureKind::Sketch(_),
                            ..
                        } => {
                            staged_planning.record_output(
                                operation_index,
                                StagedProgramOutput::SketchFeature(*id),
                            );
                        }
                        CanonicalCommand::CreateFeature { id, kind, .. }
                            if matches!(
                                operation,
                                AssistantCadEditOperation::CreatePart { .. }
                            ) && !matches!(
                                kind,
                                ketchup_core::document::FeatureKind::Workplane(_)
                                    | ketchup_core::document::FeatureKind::Sketch(_)
                            ) =>
                        {
                            staged_planning.record_output(
                                operation_index,
                                StagedProgramOutput::BodyFeature(*id),
                            );
                        }
                        CanonicalCommand::CreateOccurrence { id, .. } => {
                            staged_planning.record_output(
                                operation_index,
                                StagedProgramOutput::Occurrence(*id),
                            );
                        }
                        _ => {}
                    }
                }
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreatePanel {
                name,
                dimensions_mm,
                holes,
                pockets,
                translation_mm,
                rotation,
            } => {
                let (definition_id, occurrence_id, sketch_id, body_feature_id) = plan_panel(
                    name,
                    *dimensions_mm,
                    holes,
                    pockets,
                    *translation_mm,
                    rotation.clone(),
                    &mut staged_planning,
                    &mut next_definition,
                    &mut next_feature,
                    &mut next_occurrence,
                    &document_target,
                )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Occurrence(occurrence_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::SketchFeature(sketch_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::BodyFeature(body_feature_id),
                );
                appended_exact_features.push((definition_id, body_feature_id));
            }
            AssistantCadEditOperation::CreateSpatialPath { name, segments } => {
                let path_segments = validated_spatial_path_segments(segments).map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_spatial_path_invalid",
                        operation_name,
                        &document_target,
                        error,
                        "Use one to 64 finite, connected, non-degenerate 3D line, circular-arc, or cubic-Bezier segments.",
                    )
                })?;
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "path",
                        FeatureKind::SpatialPath {
                            segments: path_segments,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::ConstructionFeature(feature_id),
                );
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateHelixPath { name, parameters } => {
                let axis = resolve_assistant_axis_spec(
                    &parameters.axis,
                    &snapshot,
                    topology_results,
                    &staged_planning,
                    &program.operations,
                    operation_name,
                )?;
                let path_segments =
                    parameters
                        .spatial_path_segments_for_axis(axis)
                        .map_err(|error| {
                            assistant_planning_rejection(
                                "planning.cad_helix_path_invalid",
                                operation_name,
                                &document_target,
                                error,
                                "Use finite bounded Helix parameters and a non-zero 3D axis.",
                            )
                        })?;
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "path",
                        FeatureKind::SpatialPath {
                            segments: path_segments,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::ConstructionFeature(feature_id),
                );
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateConstructionPoint { name, position_mm } => {
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "point",
                        FeatureKind::ConstructionPoint {
                            position_mm: *position_mm,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::ConstructionFeature(feature_id),
                );
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateConstructionAxis {
                name,
                origin_mm,
                direction,
            } => {
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "axis",
                        FeatureKind::ConstructionAxis {
                            origin_mm: *origin_mm,
                            direction: *direction,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::ConstructionFeature(feature_id),
                );
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateConstructionPlane {
                name,
                origin_mm,
                normal,
                x_direction,
            } => {
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "plane",
                        FeatureKind::ConstructionPlane {
                            origin_mm: *origin_mm,
                            normal: *normal,
                            x_direction: *x_direction,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::ConstructionFeature(feature_id),
                );
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateHelix { name, parameters } => {
                let axis = resolve_assistant_axis_spec(
                    &parameters.axis,
                    &snapshot,
                    topology_results,
                    &staged_planning,
                    &program.operations,
                    operation_name,
                )?;
                let path_segments =
                    parameters
                        .spatial_path_segments_for_axis(axis)
                        .map_err(|error| {
                            assistant_planning_rejection(
                                "planning.cad_helix_invalid",
                                operation_name,
                                &document_target,
                                error,
                                "Use finite bounded Helix parameters and a non-zero 3D axis.",
                            )
                        })?;
                let wire_radius = (parameters.radius_mm * 0.01)
                    .min(parameters.pitch_mm * 0.2)
                    .clamp(0.002, 0.25);
                let profile_segments = vec![
                    ProfileSegment::CircularArc {
                        start_mm: [-wire_radius, 0.0],
                        end_mm: [wire_radius, 0.0],
                        center_mm: [0.0, 0.0],
                        clockwise: false,
                    },
                    ProfileSegment::CircularArc {
                        start_mm: [wire_radius, 0.0],
                        end_mm: [-wire_radius, 0.0],
                        center_mm: [0.0, 0.0],
                        clockwise: false,
                    },
                ];
                let (creation_commands, definition_id, body_feature_id) =
                    plan_assistant_helix_thread_creation(
                        name,
                        profile_segments,
                        path_segments,
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::BodyFeature(body_feature_id),
                );
                appended_exact_features.push((definition_id, body_feature_id));
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateThread { name, parameters } => {
                let axis = resolve_assistant_axis_spec(
                    &parameters.helix.axis,
                    &snapshot,
                    topology_results,
                    &staged_planning,
                    &program.operations,
                    operation_name,
                )?;
                let path_segments = parameters
                    .helix
                    .spatial_path_segments_for_axis(axis)
                    .map_err(|error| {
                        assistant_planning_rejection(
                            "planning.cad_thread_invalid",
                            operation_name,
                            &document_target,
                            error,
                            "Use finite bounded Thread parameters and a non-zero 3D axis.",
                        )
                    })?;
                let profile_segments = parameters.profile_segments().map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_thread_invalid",
                        operation_name,
                        &document_target,
                        error,
                        "Use a positive profile radius smaller than half the pitch.",
                    )
                })?;
                let (creation_commands, definition_id, body_feature_id) =
                    plan_assistant_helix_thread_creation(
                        name,
                        profile_segments,
                        path_segments,
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::Definition(definition_id),
                );
                staged_planning.record_output(
                    operation_index,
                    StagedProgramOutput::BodyFeature(body_feature_id),
                );
                appended_exact_features.push((definition_id, body_feature_id));
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateProgramSketch {
                definition,
                name,
                workplane,
                entities,
                constraints,
            } => {
                let definition_id = staged_planning.resolve_program_output(
                    *definition,
                    AssistantCadProgramFeatureOutput::Definition,
                    operation_name,
                )?;
                staged_planning.refresh(operation_name, &document_target)?;
                let planning_snapshot = staged_planning.staged_snapshot();
                let workplane = match workplane {
                    ketchup_core::assistant_sidecar::AssistantWorkplaneSpec::ConstructionPlane {
                        plane: AssistantCadFeatureReference::ProgramOutput(reference),
                    } => ketchup_core::assistant_sidecar::AssistantWorkplaneSpec::ConstructionPlane {
                        plane: AssistantCadFeatureReference::Existing(
                            staged_planning.resolve_program_output(
                                *reference,
                                AssistantCadProgramFeatureOutput::ConstructionFeature,
                                operation_name,
                            )?,
                        ),
                    },
                    _ => workplane.clone(),
                };
                let resolved = AssistantCadEditOperation::CreateSketch {
                    definition_id,
                    name: name.clone(),
                    workplane,
                    entities: entities.clone(),
                    constraints: constraints.clone(),
                };
                let creation_commands = plan_creation(
                    planning_snapshot,
                    &resolved,
                    &mut next_definition,
                    &mut next_feature,
                    &mut next_occurrence,
                    &document_target,
                )?;
                let sketch_id = creation_commands.iter().find_map(|command| match command {
                    CanonicalCommand::CreateFeature {
                        id,
                        kind: ketchup_core::document::FeatureKind::Sketch(_),
                        ..
                    } => Some(*id),
                    _ => None,
                });
                if let Some(sketch_id) = sketch_id {
                    staged_planning.record_output(
                        operation_index,
                        StagedProgramOutput::SketchFeature(sketch_id),
                    );
                }
                staged_planning.extend(creation_commands);
            }
            AssistantCadEditOperation::AppendFeature {
                definition_id,
                name,
                feature,
            } => {
                staged_planning.refresh(operation_name, &document_target)?;
                let planning_snapshot = staged_planning.staged_snapshot();
                let planning_topology = staged_planning.topology(topology_results);
                let feature = resolve_staged_program_feature_references(
                    feature,
                    &staged_planning,
                    operation_name,
                )?;
                let definition_id = DefinitionId(*definition_id);
                if planning_snapshot.definition(definition_id).is_none() {
                    return Err(assistant_canonical_rejection(
                        CanonicalError::DefinitionNotFound(definition_id),
                        operation_name,
                        &format!("definition:{}", definition_id.0),
                    ));
                }
                let kind = plan_feature_kind(
                    planning_snapshot,
                    &planning_topology,
                    definition_id,
                    &feature,
                    operation_name,
                )?;
                let id = next_feature.map(FeatureId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_feature = id.0.checked_add(1);
                staged_planning.push(CanonicalCommand::CreateFeature {
                    id,
                    definition_id,
                    name: name.clone(),
                    kind,
                });
                if feature.produces_body_feature_output() {
                    staged_planning
                        .record_output(operation_index, StagedProgramOutput::BodyFeature(id));
                }
                if matches!(feature, AssistantCadBodyFeature::PlanarOffset { .. }) {
                    appended_planar_offsets.push((definition_id, id));
                } else {
                    appended_exact_features.push((definition_id, id));
                }
            }
            AssistantCadEditOperation::AppendProgramPocket {
                definition,
                name,
                target_feature,
                profile_feature,
                depth_mm,
            } => {
                let definition_id = DefinitionId(staged_planning.resolve_program_output(
                    *definition,
                    AssistantCadProgramFeatureOutput::Definition,
                    operation_name,
                )?);
                let target_feature_id = staged_planning.resolve_program_output(
                    *target_feature,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                    operation_name,
                )?;
                let profile_feature_id = staged_planning.resolve_program_output(
                    *profile_feature,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                    operation_name,
                )?;
                staged_planning.refresh(operation_name, &document_target)?;
                let planning_snapshot = staged_planning.staged_snapshot();
                let planning_topology = staged_planning.topology(topology_results);
                let feature = AssistantCadBodyFeature::Pocket {
                    target_feature_id,
                    profile_feature_id,
                    depth_mm: *depth_mm,
                };
                let kind = plan_feature_kind(
                    planning_snapshot,
                    &planning_topology,
                    definition_id,
                    &feature,
                    operation_name,
                )?;
                let id = next_feature.map(FeatureId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_feature = id.0.checked_add(1);
                staged_planning.push(CanonicalCommand::CreateFeature {
                    id,
                    definition_id,
                    name: name.clone(),
                    kind,
                });
                staged_planning
                    .record_output(operation_index, StagedProgramOutput::BodyFeature(id));
                appended_exact_features.push((definition_id, id));
            }
            AssistantCadEditOperation::BindProgramOutput { name, source } => {
                staged_planning.bind_named_output(name, *source, operation_name)?;
            }
            AssistantCadEditOperation::SetDimension {
                feature_id,
                constraint_id,
                value_mm,
            } => {
                let feature_id = FeatureId(*feature_id);
                let dimension =
                    Dimension::new(value_mm.to_string(), *value_mm).map_err(|error| {
                        assistant_canonical_rejection(
                            error,
                            operation_name,
                            &format!("feature:{}", feature_id.0),
                        )
                    })?;
                staged_planning.push(if let Some(constraint_id) = constraint_id {
                    CanonicalCommand::SetSketchConstraintDimension {
                        id: feature_id,
                        constraint_id: SketchConstraintId(*constraint_id),
                        dimension,
                    }
                } else {
                    CanonicalCommand::SetFeatureDimension {
                        id: feature_id,
                        dimension,
                    }
                });
            }
            AssistantCadEditOperation::SetFeatureParameter {
                feature_id,
                parameter_path,
                value_type,
                value,
            } => {
                let feature_id = FeatureId(*feature_id);
                let path = ParameterPath::new(parameter_path.clone()).map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_parameter_path_invalid",
                        operation_name,
                        &format!("feature:{}", feature_id.0),
                        error.to_string(),
                        "Copy one current parameter path and value type exactly from feature inspection.",
                    )
                })?;
                let dimension = Dimension::new(value.to_string(), *value).map_err(|error| {
                    assistant_canonical_rejection(
                        error,
                        operation_name,
                        &format!("feature:{}", feature_id.0),
                    )
                })?;
                staged_planning.push(CanonicalCommand::SetFeatureParameter {
                    target: FeatureParameterTarget {
                        feature_id,
                        path,
                        value_type: match value_type {
                            AssistantCadParameterValueType::Length => ParameterValueType::Length,
                            AssistantCadParameterValueType::Angle => ParameterValueType::Angle,
                            AssistantCadParameterValueType::Scalar => ParameterValueType::Scalar,
                        },
                    },
                    dimension,
                });
            }
            AssistantCadEditOperation::MakeOccurrenceUnique { occurrence_id } => {
                let occurrence_id = OccurrenceId(*occurrence_id);
                let current = staged_planning.staged_snapshot();
                let occurrence = current.occurrence(occurrence_id).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::OccurrenceNotFound(occurrence_id),
                        operation_name,
                        &format!("occurrence:{}", occurrence_id.0),
                    )
                })?;
                let source_definition_id = occurrence.definition_id();
                let instances = current.scene_query_bounded(
                    MAX_AXIS_INSTANCE_OCCURRENCES,
                    MAX_AXIS_INSTANCE_PATH_STEPS,
                    MAX_AXIS_INSTANCE_TEXT_BYTES,
                ).map_err(|_| assistant_planning_rejection(
                    "planning.make_unique_scope_incomplete", operation_name, &document_target,
                    "The bounded instance query could not establish whether the part is shared.",
                    "Reduce the assembly scope before making this occurrence unique.",
                ))?;
                let was_shared_at_start = staged_planning
                    .base_snapshot()
                    .occurrence(occurrence_id)
                    .filter(|base| base.definition_id() == source_definition_id)
                    .map(|_| {
                        staged_planning.base_snapshot().scene_query_bounded(
                            MAX_AXIS_INSTANCE_OCCURRENCES,
                            MAX_AXIS_INSTANCE_PATH_STEPS,
                            MAX_AXIS_INSTANCE_TEXT_BYTES,
                        )
                    })
                    .transpose()
                    .map_err(|_| assistant_planning_rejection(
                        "planning.make_unique_scope_incomplete", operation_name, &document_target,
                        "The bounded original instance query could not establish whether the part was shared.",
                        "Reduce the assembly scope before making this occurrence unique.",
                    ))?
                    .is_some_and(|original| original.iter().any(|instance| {
                        instance.definition_id == source_definition_id
                            && instance.shared_occurrence_count > 1
                    }));
                if was_shared_at_start
                    || instances.iter().any(|instance| {
                        instance.definition_id == source_definition_id
                            && instance.shared_occurrence_count > 1
                    })
                {
                    let definition = current.definition(source_definition_id).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::DefinitionNotFound(source_definition_id),
                            operation_name,
                            &document_target,
                        )
                    })?;
                    let new_definition_id = next_definition.map(DefinitionId).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::IdExhausted,
                            operation_name,
                            &document_target,
                        )
                    })?;
                    next_definition = new_definition_id.0.checked_add(1);
                    let mut feature_id_map = Vec::new();
                    for source_id in definition.feature_ids() {
                        let new_id = next_feature.map(FeatureId).ok_or_else(|| {
                            assistant_canonical_rejection(
                                CanonicalError::IdExhausted,
                                operation_name,
                                &document_target,
                            )
                        })?;
                        next_feature = new_id.0.checked_add(1);
                        feature_id_map.push((*source_id, new_id));
                    }
                    staged_planning.push(CanonicalCommand::CloneDefinitionAndRepoint(
                        CloneDefinitionPlan::new(
                            occurrence_id,
                            source_definition_id,
                            new_definition_id,
                            format!("{} (unique {})", definition.name(), occurrence_id.0),
                            feature_id_map,
                        ),
                    ));
                }
            }
            AssistantCadEditOperation::CreateAssemblyJoint {
                parent_instance_path,
                child_instance_path,
                kind,
            } => {
                let parent_instance_path =
                    resolve_assistant_instance_path(parent_instance_path, &snapshot).ok_or_else(
                        || {
                            assistant_canonical_rejection(
                                CanonicalError::InvalidInstancePath,
                                operation_name,
                                "parent_instance_path",
                            )
                        },
                    )?;
                let child_instance_path =
                    resolve_assistant_instance_path(child_instance_path, &snapshot).ok_or_else(
                        || {
                            assistant_canonical_rejection(
                                CanonicalError::InvalidInstancePath,
                                operation_name,
                                "child_instance_path",
                            )
                        },
                    )?;
                let id = next_assembly_joint.map(AssemblyJointId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_assembly_joint = id.0.checked_add(1);
                staged_planning.push(CanonicalCommand::CreateAssemblyJoint(
                    AssemblyJoint::new_at_paths(
                        id,
                        parent_instance_path,
                        child_instance_path,
                        assembly_joint_kind(*kind),
                    ),
                ));
                staged_planning
                    .record_output(operation_index, StagedProgramOutput::AssemblyJoint(id));
            }
            AssistantCadEditOperation::CreateDowelJoint {
                name,
                first,
                second,
                first_center_local_mm,
                row_unit_first_local,
                count,
                spacing_mm,
                dowel,
                physical_hole_pairs,
            } => {
                let id = next_dowel_joint.map(DowelJointId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_dowel_joint = id.0.checked_add(1);
                let face = |input: &ketchup_core::assistant_sidecar::AssistantDowelJointFace| {
                    resolve_assistant_instance_path(
                        &input.instance_path,
                        staged_planning.staged_snapshot(),
                    )
                    .map(|instance_path| DowelJointFace {
                        instance_path,
                        face_origin_local_mm: input.face_origin_local_mm,
                        inward_unit_local: input.inward_unit_local,
                        bounds_min_local_mm: input.bounds_min_local_mm,
                        bounds_max_local_mm: input.bounds_max_local_mm,
                    })
                };
                let contract = DowelJointContract {
                    id,
                    name: name.clone(),
                    first: face(first).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::InvalidInstancePath,
                            operation_name,
                            "first.instance_path",
                        )
                    })?,
                    second: face(second).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::InvalidInstancePath,
                            operation_name,
                            "second.instance_path",
                        )
                    })?,
                    first_center_local_mm: *first_center_local_mm,
                    row_unit_first_local: *row_unit_first_local,
                    count: *count,
                    spacing_mm: *spacing_mm,
                    dowel: match dowel {
                        AssistantStandardDowel::D6x30 => StandardDowel::D6x30,
                        AssistantStandardDowel::D8x30 => StandardDowel::D8x30,
                        AssistantStandardDowel::D8x40 => StandardDowel::D8x40,
                        AssistantStandardDowel::D10x40 => StandardDowel::D10x40,
                    }
                    .symmetric_spec(),
                    physical_hole_pairs: (!physical_hole_pairs.is_empty()).then(|| {
                        physical_hole_pairs
                            .iter()
                            .map(|pair| DowelPhysicalHolePair {
                                first_pocket_feature_id: FeatureId(pair.first_pocket_feature_id),
                                second_pocket_feature_id: FeatureId(pair.second_pocket_feature_id),
                            })
                            .collect()
                    }),
                };
                project_dowel_joint_contract(staged_planning.staged_snapshot(), &contract).map_err(
                    |error| {
                        assistant_planning_rejection(
                            "planning.dowel_joint_invalid",
                            operation_name,
                            &format!("dowel_joint:{}", id.0),
                            error.to_string(),
                            "Use two coincident opposed part faces and a row that remains inside both parts.",
                        )
                    },
                )?;
                staged_planning.push(CanonicalCommand::UpsertDowelJoint(contract));
                staged_planning.record_output(operation_index, StagedProgramOutput::DowelJoint(id));
            }
            AssistantCadEditOperation::CreateProgramDowelJoint {
                name,
                first,
                second,
                first_center_local_mm,
                row_unit_first_local,
                count,
                spacing_mm,
                dowel,
                physical_hole_pairs,
            } => {
                staged_planning.refresh(operation_name, &document_target)?;
                let id = next_dowel_joint.map(DowelJointId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_dowel_joint = id.0.checked_add(1);
                let face = |input: &AssistantProgramDowelJointFace| -> AssistantPlanningResult<_> {
                    let occurrence_id = OccurrenceId(staged_planning.resolve_named_output(
                        &input.occurrence,
                        AssistantCadProgramFeatureOutput::Occurrence,
                        operation_name,
                    )?);
                    let instance_path = InstancePath::root(occurrence_id);
                    staged_planning
                        .staged_snapshot()
                        .resolve_instance_path(&instance_path)
                        .map_err(|_| {
                            assistant_canonical_rejection(
                                CanonicalError::InvalidInstancePath,
                                operation_name,
                                &input.occurrence.name,
                            )
                        })?;
                    Ok(DowelJointFace {
                        instance_path,
                        face_origin_local_mm: input.face_origin_local_mm,
                        inward_unit_local: input.inward_unit_local,
                        bounds_min_local_mm: input.bounds_min_local_mm,
                        bounds_max_local_mm: input.bounds_max_local_mm,
                    })
                };
                let pairs = physical_hole_pairs
                    .iter()
                    .map(|pair| {
                        Ok(DowelPhysicalHolePair {
                            first_pocket_feature_id: FeatureId(
                                staged_planning.resolve_named_output(
                                    &pair.first_pocket_feature,
                                    AssistantCadProgramFeatureOutput::BodyFeature,
                                    operation_name,
                                )?,
                            ),
                            second_pocket_feature_id: FeatureId(
                                staged_planning.resolve_named_output(
                                    &pair.second_pocket_feature,
                                    AssistantCadProgramFeatureOutput::BodyFeature,
                                    operation_name,
                                )?,
                            ),
                        })
                    })
                    .collect::<AssistantPlanningResult<Vec<_>>>()?;
                let contract = DowelJointContract {
                    id,
                    name: name.clone(),
                    first: face(first)?,
                    second: face(second)?,
                    first_center_local_mm: *first_center_local_mm,
                    row_unit_first_local: *row_unit_first_local,
                    count: *count,
                    spacing_mm: *spacing_mm,
                    dowel: match dowel {
                        AssistantStandardDowel::D6x30 => StandardDowel::D6x30,
                        AssistantStandardDowel::D8x30 => StandardDowel::D8x30,
                        AssistantStandardDowel::D8x40 => StandardDowel::D8x40,
                        AssistantStandardDowel::D10x40 => StandardDowel::D10x40,
                    }
                    .symmetric_spec(),
                    physical_hole_pairs: Some(pairs),
                };
                project_dowel_joint_contract(staged_planning.staged_snapshot(), &contract).map_err(
                    |error| {
                        assistant_planning_rejection(
                            "planning.dowel_joint_invalid",
                            operation_name,
                            &format!("dowel_joint:{}", id.0),
                            error.to_string(),
                            "Use two coincident opposed part faces and paired physical pocket outputs.",
                        )
                    },
                )?;
                staged_planning.push(CanonicalCommand::UpsertDowelJoint(contract));
                staged_planning.record_output(operation_index, StagedProgramOutput::DowelJoint(id));
            }
            AssistantCadEditOperation::CreatePhysicalDowelJoint {
                joint_id,
                name,
                first,
                second,
                first_center_local_mm,
                row_unit_first_local,
                count,
                spacing_mm,
                dowel,
            } => {
                staged_planning.refresh(operation_name, &document_target)?;
                let id = if let Some(id) = joint_id {
                    DowelJointId(*id)
                } else {
                    let id = next_dowel_joint.map(DowelJointId).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::IdExhausted,
                            operation_name,
                            &document_target,
                        )
                    })?;
                    next_dowel_joint = id.0.checked_add(1);
                    id
                };
                let face = |input: &ketchup_core::assistant_sidecar::AssistantDowelJointFace| {
                    resolve_assistant_instance_path(
                        &input.instance_path,
                        staged_planning.staged_snapshot(),
                    )
                    .map(|instance_path| DowelJointFace {
                        instance_path,
                        face_origin_local_mm: input.face_origin_local_mm,
                        inward_unit_local: input.inward_unit_local,
                        bounds_min_local_mm: input.bounds_min_local_mm,
                        bounds_max_local_mm: input.bounds_max_local_mm,
                    })
                };
                let mut contract = DowelJointContract {
                    id,
                    name: name.clone(),
                    first: face(first).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::InvalidInstancePath,
                            operation_name,
                            "first.instance_path",
                        )
                    })?,
                    second: face(second).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::InvalidInstancePath,
                            operation_name,
                            "second.instance_path",
                        )
                    })?,
                    first_center_local_mm: *first_center_local_mm,
                    row_unit_first_local: *row_unit_first_local,
                    count: *count,
                    spacing_mm: *spacing_mm,
                    dowel: match dowel {
                        AssistantStandardDowel::D6x30 => StandardDowel::D6x30,
                        AssistantStandardDowel::D8x30 => StandardDowel::D8x30,
                        AssistantStandardDowel::D8x40 => StandardDowel::D8x40,
                        AssistantStandardDowel::D10x40 => StandardDowel::D10x40,
                    }
                    .symmetric_spec(),
                    physical_hole_pairs: None,
                };
                if joint_id.is_some() {
                    if let Some(existing) = staged_planning.staged_snapshot().dowel_joint(id) {
                        let mut unchanged = contract.clone();
                        unchanged.physical_hole_pairs = existing.physical_hole_pairs.clone();
                        if existing == &unchanged {
                            project_dowel_joint_contract(
                                staged_planning.staged_snapshot(),
                                existing,
                            )
                            .map_err(|error| {
                                assistant_planning_rejection(
                                    "planning.physical_dowel_joint_invalid",
                                    operation_name,
                                    &format!("dowel_joint:{}", id.0),
                                    error.to_string(),
                                    "Repair the existing physical holes before repeating this joint.",
                                )
                            })?;
                            staged_planning.record_output(
                                operation_index,
                                StagedProgramOutput::DowelJoint(id),
                            );
                            continue;
                        }
                    }
                    delete_owned_physical_dowel_joint(
                        id,
                        &mut staged_planning,
                        operation_name,
                        &document_target,
                    )?;
                }
                let projection = project_dowel_joint_contract(
                    staged_planning.staged_snapshot(),
                    &contract,
                )
                .map_err(|error| {
                    assistant_planning_rejection(
                        "planning.dowel_joint_invalid",
                        operation_name,
                        &format!("dowel_joint:{}", id.0),
                        error.to_string(),
                        "Use two coincident opposed part faces and a row that remains inside both parts.",
                    )
                })?;
                let first_definition = staged_planning
                    .staged_snapshot()
                    .resolve_instance_path(&contract.first.instance_path)
                    .map_err(|_| {
                        assistant_canonical_rejection(
                            CanonicalError::InvalidInstancePath,
                            operation_name,
                            "first.instance_path",
                        )
                    })?
                    .definition_id;
                let second_definition = staged_planning
                    .staged_snapshot()
                    .resolve_instance_path(&contract.second.instance_path)
                    .map_err(|_| {
                        assistant_canonical_rejection(
                            CanonicalError::InvalidInstancePath,
                            operation_name,
                            "second.instance_path",
                        )
                    })?
                    .definition_id;
                let instances = staged_planning.staged_snapshot().scene_query_bounded(
                    MAX_AXIS_INSTANCE_OCCURRENCES,
                    MAX_AXIS_INSTANCE_PATH_STEPS,
                    MAX_AXIS_INSTANCE_TEXT_BYTES,
                ).map_err(|_| assistant_planning_rejection(
                    "planning.physical_dowel_scope_incomplete", operation_name, &document_target,
                    "The bounded instance query could not establish exclusive ownership of the drilled parts.",
                    "Reduce the assembly scope before creating physical holes.",
                ))?;
                if instances.iter().any(|instance| {
                    [first_definition, second_definition].contains(&instance.definition_id)
                        && instance.shared_occurrence_count > 1
                }) {
                    return Err(assistant_planning_rejection(
                        "planning.physical_dowel_shared_definition",
                        operation_name,
                        &document_target,
                        "Creating these holes would also modify other instances of a shared definition.",
                        "Make the target parts unique while preserving existing joinery before drilling.",
                    ));
                }
                let mut targets = BTreeMap::new();
                for definition_id in [first_definition, second_definition] {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        targets.entry(definition_id)
                    {
                        entry.insert(sole_terminal_body_feature(
                            staged_planning.staged_snapshot(),
                            definition_id,
                            operation_name,
                        )?);
                    }
                }
                let mut physical_hole_pairs = Vec::with_capacity(projection.pairs.len());
                for pair in &projection.pairs {
                    let first_pocket_feature_id = append_physical_dowel_hole(
                        &pair.first,
                        first_definition,
                        targets
                            .get_mut(&first_definition)
                            .expect("first target initialized"),
                        &mut staged_planning,
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        &document_target,
                    )?;
                    appended_exact_features.push((first_definition, first_pocket_feature_id));
                    let second_pocket_feature_id = append_physical_dowel_hole(
                        &pair.second,
                        second_definition,
                        targets
                            .get_mut(&second_definition)
                            .expect("second target initialized"),
                        &mut staged_planning,
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        &document_target,
                    )?;
                    appended_exact_features.push((second_definition, second_pocket_feature_id));
                    physical_hole_pairs.push(DowelPhysicalHolePair {
                        first_pocket_feature_id,
                        second_pocket_feature_id,
                    });
                }
                staged_planning.refresh(operation_name, &document_target)?;
                contract.physical_hole_pairs = Some(physical_hole_pairs);
                project_dowel_joint_contract(staged_planning.staged_snapshot(), &contract).map_err(
                    |error| {
                        assistant_planning_rejection(
                            "planning.physical_dowel_joint_invalid",
                            operation_name,
                            &format!("dowel_joint:{}", id.0),
                            error.to_string(),
                            "Use a supported single-body part pair with unobstructed physical holes.",
                        )
                    },
                )?;
                staged_planning.push(CanonicalCommand::UpsertDowelJoint(contract));
                staged_planning.record_output(operation_index, StagedProgramOutput::DowelJoint(id));
            }
            AssistantCadEditOperation::DeletePhysicalDowelJoint { joint_id } => {
                delete_owned_physical_dowel_joint(
                    DowelJointId(*joint_id),
                    &mut staged_planning,
                    operation_name,
                    &document_target,
                )?;
            }
            AssistantCadEditOperation::SetAssemblyJointPosition { joint_id, position } => {
                let joint_id = AssemblyJointId(*joint_id);
                if snapshot.assembly_joint(joint_id).is_none() {
                    return Err(assistant_planning_rejection(
                        "planning.assembly_joint_not_found",
                        operation_name,
                        &format!("assembly_joint:{}", joint_id.0),
                        "The requested assembly joint does not exist in the current document.",
                        "Inspect the current assembly joints and retry with an existing ID.",
                    ));
                }
                let preview = preview_assembly_joint_drag(&snapshot, joint_id, *position, false)
                    .map_err(|error| {
                        assistant_planning_rejection(
                            "planning.assembly_motion_unsolved",
                            operation_name,
                            &format!("assembly_joint:{}", joint_id.0),
                            error.to_string(),
                            "Use a position within the joint limits and a solvable assembly state.",
                        )
                    })?;
                let publication =
                    preview
                        .solution()
                        .publication_batch(&snapshot)
                        .map_err(|error| {
                            assistant_planning_rejection(
                                "planning.assembly_motion_unpublishable",
                                operation_name,
                                &format!("assembly_joint:{}", joint_id.0),
                                error.to_string(),
                                "Refresh the document state and retry the motion edit.",
                            )
                        })?;
                staged_planning.extend(publication.commands().iter().cloned());
            }
            AssistantCadEditOperation::CreateDrawing {
                name,
                instance_paths,
            } => {
                let mut resolved_paths = instance_paths
                    .iter()
                    .map(|path| {
                        resolve_assistant_instance_path(path, &snapshot).ok_or_else(|| {
                            assistant_canonical_rejection(
                                CanonicalError::InvalidInstancePath,
                                operation_name,
                                "instance_paths",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                resolved_paths.sort();
                let id = next_drawing_sheet.map(DrawingSheetId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_drawing_sheet = id.0.checked_add(1);
                let mut positions = BTreeMap::<DefinitionId, u32>::new();
                let mut next_position = 1_u32;
                let balloons = resolved_paths
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        let resolved = snapshot.resolve_instance_path(path).map_err(|_| {
                            assistant_canonical_rejection(
                                CanonicalError::InvalidInstancePath,
                                operation_name,
                                "instance_paths",
                            )
                        })?;
                        let position =
                            *positions.entry(resolved.definition_id).or_insert_with(|| {
                                let assigned = next_position;
                                next_position += 1;
                                assigned
                            });
                        DrawingBomBalloon::new(
                            DrawingBomBalloonId(index as u64 + 1),
                            OrthographicViewKind::Front,
                            path.clone(),
                            position,
                            [8.0, 8.0],
                        )
                        .map_err(|error| {
                            assistant_planning_rejection(
                                "planning.drawing_invalid",
                                operation_name,
                                &format!("drawing_sheet:{}", id.0),
                                error.to_string(),
                                "Use current unique assembly instance paths.",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let source = DrawingSource::RigidAssemblyInstances {
                    instance_paths: resolved_paths,
                };
                let title_block =
                    DrawingTitleBlock::new(name.clone(), "", "", "").map_err(|error| {
                        assistant_planning_rejection(
                            "planning.drawing_invalid",
                            operation_name,
                            &format!("drawing_sheet:{}", id.0),
                            error.to_string(),
                            "Use a non-empty name and current unique assembly instance paths.",
                        )
                    })?;
                let mut fitted_sheet = None;
                let mut layout_overflow = None;
                for denominator in [
                    1, 2, 5, 10, 20, 50, 100, 200, 500, 1_000, 10_000, 100_000, 1_000_000,
                ] {
                    let page = DrawingPageTemplate::new(
                        DrawingPageSize::A3,
                        DrawingPageOrientation::Landscape,
                        DrawingScale::new(1, denominator).expect("bounded drawing scale is valid"),
                        DrawingMargins::default(),
                    )
                    .expect("standard A3 drawing page is valid");
                    let candidate = DrawingSheet::with_contract(
                        id,
                        name.clone(),
                        source.clone(),
                        page,
                        title_block.clone(),
                    )
                    .and_then(|sheet| sheet.with_bom_balloons(balloons.clone()))
                    .map_err(|error| {
                        assistant_planning_rejection(
                            "planning.drawing_invalid",
                            operation_name,
                            &format!("drawing_sheet:{}", id.0),
                            error.to_string(),
                            "Use a non-empty name and current unique assembly instance paths.",
                        )
                    })?;
                    match project_orthographic_drawing(&snapshot, topology_results, &candidate) {
                        Ok(_) => {
                            fitted_sheet = Some(candidate);
                            break;
                        }
                        Err(error @ DrawingError::LayoutOverflow) => layout_overflow = Some(error),
                        Err(error) => {
                            return Err(assistant_planning_rejection(
                                "planning.drawing_projection_failed",
                                operation_name,
                                &format!("drawing_sheet:{}", id.0),
                                error.to_string(),
                                "Use current exact assembly instances with a fully constrained rigid mate graph.",
                            ));
                        }
                    }
                }
                let sheet = fitted_sheet.ok_or_else(|| {
                    assistant_planning_rejection(
                        "planning.drawing_projection_failed",
                        operation_name,
                        &format!("drawing_sheet:{}", id.0),
                        layout_overflow
                            .unwrap_or(DrawingError::LayoutOverflow)
                            .to_string(),
                        "Use a larger sheet or reduce the assembly extent.",
                    )
                })?;
                staged_planning.push(CanonicalCommand::CreateDrawingSheet(sheet));
            }
            AssistantCadEditOperation::UpsertCamPlan {
                plan_id,
                name,
                target_definition_id,
                target_feature_id,
                stock_minimum_mm,
                stock_maximum_mm,
                tool_number,
                tool_kind,
                tool_diameter_mm,
                flute_length_mm,
                overall_length_mm,
                holder_diameter_mm,
                holder_length_mm,
                spindle_rpm,
                feed_mm_per_min,
                plunge_mm_per_min,
                work_offset,
                origin_mm,
                x_axis,
                y_axis,
                safe_height_mm,
                maximum_stepdown_mm,
                stepover_ratio,
                radial_allowance_mm,
                axial_allowance_mm,
            } => {
                let plan = CamPlan::new(
                    &snapshot,
                    CamPlanId(*plan_id),
                    name.clone(),
                    CamStock {
                        minimum_mm: *stock_minimum_mm,
                        maximum_mm: *stock_maximum_mm,
                    },
                    CamTool {
                        number: *tool_number,
                        kind: match tool_kind {
                            AssistantCamToolKind::FlatEndMill => CamToolKind::FlatEndMill,
                            AssistantCamToolKind::BallEndMill => CamToolKind::BallEndMill,
                            AssistantCamToolKind::Drill => CamToolKind::Drill,
                        },
                        diameter_mm: *tool_diameter_mm,
                        flute_length_mm: *flute_length_mm,
                        overall_length_mm: *overall_length_mm,
                        holder_diameter_mm: *holder_diameter_mm,
                        holder_length_mm: *holder_length_mm,
                        spindle_rpm: *spindle_rpm,
                        feed_mm_per_min: *feed_mm_per_min,
                        plunge_mm_per_min: *plunge_mm_per_min,
                    },
                    CamSetup {
                        work_offset: match work_offset {
                            AssistantCamWorkOffset::G54 => CamWorkOffset::G54,
                            AssistantCamWorkOffset::G55 => CamWorkOffset::G55,
                            AssistantCamWorkOffset::G56 => CamWorkOffset::G56,
                            AssistantCamWorkOffset::G57 => CamWorkOffset::G57,
                            AssistantCamWorkOffset::G58 => CamWorkOffset::G58,
                            AssistantCamWorkOffset::G59 => CamWorkOffset::G59,
                        },
                        origin_mm: *origin_mm,
                        x_axis: *x_axis,
                        y_axis: *y_axis,
                        safe_height_mm: *safe_height_mm,
                    },
                    CamCutParameters {
                        maximum_stepdown_mm: *maximum_stepdown_mm,
                        stepover_ratio: *stepover_ratio,
                        radial_allowance_mm: *radial_allowance_mm,
                        axial_allowance_mm: *axial_allowance_mm,
                    },
                    DefinitionId(*target_definition_id),
                    FeatureId(*target_feature_id),
                )
                .map_err(|error| {
                    assistant_canonical_rejection(
                        CanonicalError::Cam(error),
                        operation_name,
                        &format!("cam_plan:{plan_id}"),
                    )
                })?;
                staged_planning.push(CanonicalCommand::UpsertCamPlan(plan));
            }
            AssistantCadEditOperation::Delete {
                dependency_policy, ..
            } => {
                let target_set = targets.iter().copied().collect::<BTreeSet<_>>();
                let referenced_collections = staged_planning
                    .staged_snapshot()
                    .collections()
                    .filter(|collection| {
                        collection
                            .occurrence_ids()
                            .any(|id| target_set.contains(&id))
                    })
                    .map(|collection| {
                        (
                            collection.id(),
                            collection
                                .occurrence_ids()
                                .filter(|id| !target_set.contains(id))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>();
                let incident_mates = staged_planning
                    .staged_snapshot()
                    .assembly_mates()
                    .filter(|mate| {
                        target_set.contains(&mate.endpoint_a().occurrence_id())
                            || target_set.contains(&mate.endpoint_b().occurrence_id())
                    })
                    .map(|mate| mate.id())
                    .collect::<Vec<_>>();
                if *dependency_policy == AssistantCadDeletePolicy::RejectIfReferenced
                    && (!referenced_collections.is_empty() || !incident_mates.is_empty())
                {
                    let id = targets[0];
                    return Err(assistant_planning_rejection(
                        "planning.cad_delete_referenced",
                        operation_name,
                        &format!("occurrence:{}", id.0),
                        "The occurrence is referenced by a collection or assembly mate.",
                        "Use remove_references only when removing those dependencies is intended.",
                    ));
                }
                if *dependency_policy == AssistantCadDeletePolicy::RemoveReferences {
                    for (collection_id, members) in referenced_collections {
                        staged_planning.push(CanonicalCommand::SetCollectionOccurrences {
                            id: collection_id,
                            occurrence_ids: members,
                        });
                    }
                    for mate_id in incident_mates {
                        staged_planning.push(CanonicalCommand::DeleteAssemblyMate { id: mate_id });
                    }
                }
                for id in targets {
                    staged_planning.push(CanonicalCommand::DeleteOccurrence { id });
                }
            }
            AssistantCadEditOperation::Transform {
                translation_mm,
                rotation,
                ..
            } => {
                let delta = Vec3::new(translation_mm[0], translation_mm[1], translation_mm[2]);
                let world_rotation = rotation
                    .as_ref()
                    .map(|rotation| {
                        world_axis_rotation_transform(
                            Vec3::new(
                                rotation.pivot_mm[0],
                                rotation.pivot_mm[1],
                                rotation.pivot_mm[2],
                            ),
                            Vec3::new(rotation.axis[0], rotation.axis[1], rotation.axis[2]),
                            rotation.angle_degrees,
                        )
                    })
                    .transpose()
                    .map_err(|_| {
                        assistant_planning_rejection(
                            "planning.cad_transform_invalid",
                            operation_name,
                            "occurrence_selection",
                            "The requested rigid transform could not be represented.",
                            "Use a finite translation and a non-zero finite rotation axis.",
                        )
                    })?;
                for id in targets {
                    let occurrence = staged_planning
                        .staged_snapshot()
                        .occurrence(id)
                        .expect("resolved CAD selector targets a staged occurrence");
                    let translated =
                        translated_transform(occurrence.transform(), delta).map_err(|_| {
                            assistant_planning_rejection(
                                "planning.cad_transform_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The requested translation could not be represented.",
                                "Use a finite bounded translation.",
                            )
                        })?;
                    let transform = if let Some(world_rotation) = world_rotation {
                        let parent_transform = occurrence
                            .parent()
                            .map_or(Some(Transform::identity()), |parent| {
                                staged_planning
                                    .staged_snapshot()
                                    .world_transform_for_group(parent)
                            })
                            .ok_or_else(|| {
                                assistant_planning_rejection(
                                    "planning.cad_parent_transform_unavailable",
                                    operation_name,
                                    &format!("occurrence:{}", id.0),
                                    "The occurrence parent transform could not be resolved.",
                                    "Refresh the document context and retry the transform.",
                                )
                            })?;
                        rotation_in_parent_space(
                            world_rotation,
                            parent_transform,
                            translated,
                        )
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.cad_transform_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The requested world-space rotation could not be represented in the occurrence parent.",
                                "Use a finite rigid parent and rotation.",
                            )
                        })?
                    } else {
                        translated
                    };
                    staged_planning
                        .push(CanonicalCommand::SetOccurrenceTransform { id, transform });
                }
            }
            AssistantCadEditOperation::SetColor { color, .. } => {
                for id in targets {
                    staged_planning
                        .push(CanonicalCommand::SetOccurrenceColor { id, color: *color });
                }
            }
            AssistantCadEditOperation::SetGrounded { grounded, .. } => {
                for id in targets {
                    staged_planning.push(CanonicalCommand::SetOccurrenceGrounded {
                        id,
                        grounded: *grounded,
                    });
                }
            }
            AssistantCadEditOperation::CreateTag {
                tag_id,
                name,
                visible,
            } => staged_planning.push(CanonicalCommand::CreateTag {
                id: TagId(*tag_id),
                name: name.clone(),
                visible: *visible,
            }),
            AssistantCadEditOperation::SetOccurrenceTag { tag_id, .. } => {
                for id in targets {
                    staged_planning.push(CanonicalCommand::SetOccurrenceTag {
                        id,
                        tag: tag_id.map(TagId),
                    });
                }
            }
            AssistantCadEditOperation::SetTagVisibility { tag_id, visible } => {
                staged_planning.push(CanonicalCommand::SetTagVisibility {
                    id: TagId(*tag_id),
                    visible: *visible,
                });
            }
            AssistantCadEditOperation::UpsertClassificationDimension {
                dimension_id,
                name,
                categories,
            } => staged_planning.push(CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(*dimension_id),
                name: name.clone(),
                categories: categories
                    .iter()
                    .map(|category| (ClassificationCategoryId(category.id), category.name.clone()))
                    .collect(),
            }),
            AssistantCadEditOperation::SetOccurrenceClassification {
                dimension_id,
                category_id,
                ..
            } => {
                for occurrence_id in targets {
                    staged_planning.push(CanonicalCommand::SetOccurrenceClassification {
                        occurrence_id,
                        dimension_id: ClassificationDimensionId(*dimension_id),
                        category_id: category_id.map(ClassificationCategoryId),
                    });
                }
            }
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id,
                name,
                value,
            } => staged_planning.push(CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(*node_id),
                name: name.clone(),
                dimension: Dimension::new(value.to_string(), *value).map_err(|error| {
                    assistant_canonical_rejection(error, operation_name, &format!("node:{node_id}"))
                })?,
                dependencies: Vec::new(),
            }),
            AssistantCadEditOperation::Copy { translation_mm, .. } => {
                let delta = Vec3::new(translation_mm[0], translation_mm[1], translation_mm[2]);
                for id in targets {
                    let source = staged_planning
                        .staged_snapshot()
                        .occurrence(id)
                        .cloned()
                        .expect("resolved CAD selector targets a staged occurrence");
                    let transform =
                        translated_transform(source.transform(), delta).map_err(|_| {
                            assistant_planning_rejection(
                                "planning.cad_copy_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The requested copy transform could not be represented.",
                                "Use a finite bounded translation.",
                            )
                        })?;
                    let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::IdExhausted,
                            operation_name,
                            &document_target,
                        )
                    })?;
                    next_occurrence = occurrence_id.0.checked_add(1);
                    staged_planning.push(CanonicalCommand::CreateOccurrence {
                        id: occurrence_id,
                        definition_id: source.definition_id(),
                        name: source.name().to_owned(),
                        transform,
                        parent: source.parent(),
                        tag: source.tag(),
                        visible: source.visible(),
                    });
                    if source.color().is_some() {
                        staged_planning.push(CanonicalCommand::SetOccurrenceColor {
                            id: occurrence_id,
                            color: source.color(),
                        });
                    }
                }
            }
            AssistantCadEditOperation::LinearPattern {
                instances, step_mm, ..
            } => {
                let step = Vec3::new(step_mm[0], step_mm[1], step_mm[2]);
                for instance in 1..*instances {
                    let delta = step * f64::from(instance);
                    for id in &targets {
                        let source = staged_planning
                            .staged_snapshot()
                            .occurrence(*id)
                            .cloned()
                            .expect("resolved CAD selector targets a staged occurrence");
                        let transform =
                            translated_transform(source.transform(), delta).map_err(|_| {
                                assistant_planning_rejection(
                                    "planning.cad_pattern_invalid",
                                    operation_name,
                                    &format!("occurrence:{}", id.0),
                                    "The requested pattern transform could not be represented.",
                                    "Use a finite bounded pattern step and instance count.",
                                )
                            })?;
                        let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                            assistant_canonical_rejection(
                                CanonicalError::IdExhausted,
                                operation_name,
                                &document_target,
                            )
                        })?;
                        next_occurrence = occurrence_id.0.checked_add(1);
                        staged_planning.push(CanonicalCommand::CreateOccurrence {
                            id: occurrence_id,
                            definition_id: source.definition_id(),
                            name: source.name().to_owned(),
                            transform,
                            parent: source.parent(),
                            tag: source.tag(),
                            visible: source.visible(),
                        });
                        if source.color().is_some() {
                            staged_planning.push(CanonicalCommand::SetOccurrenceColor {
                                id: occurrence_id,
                                color: source.color(),
                            });
                        }
                    }
                }
            }
            AssistantCadEditOperation::CircularPattern {
                instances,
                axis,
                angle_step_degrees,
                ..
            } => {
                let (origin_mm, direction) = axis
                    .origin_and_direction()
                    .expect("referenced circular pattern axis was normalized before planning");
                let centre = Vec3::new(origin_mm[0], origin_mm[1], origin_mm[2]);
                let direction = Vec3::new(direction[0], direction[1], direction[2]);
                for instance in 1..*instances {
                    let world_rotation = world_axis_rotation_transform(
                        centre,
                        direction,
                        angle_step_degrees * f64::from(instance),
                    )
                    .map_err(|_| {
                        assistant_planning_rejection(
                            "planning.cad_pattern_invalid",
                            operation_name,
                            "occurrence_selection",
                            "The requested circular pattern transform could not be represented.",
                            "Use a valid shared axis, finite angle step, and instance count.",
                        )
                    })?;
                    for id in &targets {
                        let source = staged_planning
                            .staged_snapshot()
                            .occurrence(*id)
                            .cloned()
                            .expect("resolved CAD selector targets a staged occurrence");
                        let parent_transform = source
                            .parent()
                            .map_or(Some(Transform::identity()), |parent| {
                                staged_planning
                                    .staged_snapshot()
                                    .world_transform_for_group(parent)
                            })
                            .ok_or_else(|| {
                                assistant_planning_rejection(
                                    "planning.cad_parent_transform_unavailable",
                                    operation_name,
                                    &format!("occurrence:{}", id.0),
                                    "The occurrence parent transform could not be resolved.",
                                    "Refresh the document context and retry the circular pattern.",
                                )
                            })?;
                        let transform = rotation_in_parent_space(
                            world_rotation,
                            parent_transform,
                            source.transform(),
                        )
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.cad_pattern_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The circular pattern could not be represented in the occurrence parent.",
                                "Use a finite invertible parent transform and shared axis.",
                            )
                        })?;
                        let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                            assistant_canonical_rejection(
                                CanonicalError::IdExhausted,
                                operation_name,
                                &document_target,
                            )
                        })?;
                        next_occurrence = occurrence_id.0.checked_add(1);
                        staged_planning.push(CanonicalCommand::CreateOccurrence {
                            id: occurrence_id,
                            definition_id: source.definition_id(),
                            name: source.name().to_owned(),
                            transform,
                            parent: source.parent(),
                            tag: source.tag(),
                            visible: source.visible(),
                        });
                        if source.color().is_some() {
                            staged_planning.push(CanonicalCommand::SetOccurrenceColor {
                                id: occurrence_id,
                                color: source.color(),
                            });
                        }
                    }
                }
            }
            AssistantCadEditOperation::Mirror {
                plane_origin_mm,
                plane_normal,
                ..
            } => {
                let world_mirror = world_plane_mirror_transform(
                    Vec3::new(plane_origin_mm[0], plane_origin_mm[1], plane_origin_mm[2]),
                    Vec3::new(plane_normal[0], plane_normal[1], plane_normal[2]),
                )
                .map_err(|_| {
                    assistant_planning_rejection(
                        "planning.cad_mirror_invalid",
                        operation_name,
                        "occurrence_selection",
                        "The requested mirror plane could not be represented.",
                        "Use a finite plane origin and non-zero finite normal.",
                    )
                })?;
                for id in targets {
                    let source = staged_planning
                        .staged_snapshot()
                        .occurrence(id)
                        .cloned()
                        .expect("resolved CAD selector targets a staged occurrence");
                    let parent_transform = source
                        .parent()
                        .map_or(Some(Transform::identity()), |parent| {
                            staged_planning
                                .staged_snapshot()
                                .world_transform_for_group(parent)
                        })
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.cad_parent_transform_unavailable",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The occurrence parent transform could not be resolved.",
                                "Refresh the document context and retry the mirror.",
                            )
                        })?;
                    let transform = rotation_in_parent_space(
                        world_mirror,
                        parent_transform,
                        source.transform(),
                    )
                    .ok_or_else(|| {
                        assistant_planning_rejection(
                            "planning.cad_mirror_invalid",
                            operation_name,
                            &format!("occurrence:{}", id.0),
                            "The requested mirror could not be represented in the occurrence parent.",
                            "Use a finite invertible parent transform and mirror plane.",
                        )
                    })?;
                    let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::IdExhausted,
                            operation_name,
                            &document_target,
                        )
                    })?;
                    next_occurrence = occurrence_id.0.checked_add(1);
                    staged_planning.push(CanonicalCommand::CreateOccurrence {
                        id: occurrence_id,
                        definition_id: source.definition_id(),
                        name: source.name().to_owned(),
                        transform,
                        parent: source.parent(),
                        tag: source.tag(),
                        visible: source.visible(),
                    });
                    if source.color().is_some() {
                        staged_planning.push(CanonicalCommand::SetOccurrenceColor {
                            id: occurrence_id,
                            color: source.color(),
                        });
                    }
                }
            }
            AssistantCadEditOperation::FilletEdges { .. }
            | AssistantCadEditOperation::ChamferEdges { .. } => {
                unreachable!("direct edge finishes are normalized before planning")
            }
        }
    }
    let outputs = staged_planning.resolved_named_outputs();
    let batch = staged_planning.into_final_batch();
    if !appended_exact_features.is_empty() || !appended_planar_offsets.is_empty() {
        let candidate = document.preview_batch(&batch).map_err(|error| {
            assistant_canonical_rejection(error, "append_feature", &document_target)
        })?;
        for (definition_id, feature_id) in appended_exact_features {
            ExactBRepGraph::from_snapshot(&candidate, definition_id, feature_id).map_err(
                |error| {
                    assistant_planning_rejection(
                        "planning.cad_feature_result_unsupported",
                        "append_feature",
                        &format!("feature:{}", feature_id.0),
                        error.to_string(),
                        "Use operands and an operation that produce a supported exact body.",
                    )
                },
            )?;
        }
        for (definition_id, feature_id) in appended_planar_offsets {
            let request = ExactPlanarOffsetRequest::from_snapshot(&candidate, definition_id)
                .map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_feature_result_unsupported",
                        "append_feature",
                        &format!("feature:{}", feature_id.0),
                        error.to_string(),
                        "Use one rectangular profile and a signed distance that leaves a non-collapsing planar result.",
                    )
                })?;
            if request.offset_feature_id != feature_id {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_result_unsupported",
                    "append_feature",
                    &format!("feature:{}", feature_id.0),
                    "The Planar Offset result does not match the host-assigned output feature.",
                    "Use the sole rectangular profile in the requested definition.",
                ));
            }
        }
    }
    Ok(AssistantCadProgramPlan { batch, outputs })
}

/// Plans many `create_panel` operations as one command batch. Used by rule
/// programs, which may generate far more parts than one Assistant program is
/// allowed to create.
pub fn plan_panel_batch(
    document: &DocumentStore,
    panels: &[AssistantCadEditOperation],
) -> Result<CommandBatch, Box<AssistantRejectionDiagnostic>> {
    let snapshot = document.current();
    let document_target = format!("document:{}", snapshot.document_id().0);
    let next_id = |ids: &mut dyn Iterator<Item = u64>| ids.max().unwrap_or(0).checked_add(1);
    let mut next_definition = next_id(&mut snapshot.definitions().map(|item| item.id().0));
    let mut next_occurrence = next_id(&mut snapshot.occurrences().map(|item| item.id().0));
    let mut next_feature = next_id(&mut snapshot.features().map(|item| item.id().0));
    let mut staged = StagedPlanningContext::new(&snapshot);
    for operation in panels {
        let AssistantCadEditOperation::CreatePanel {
            name,
            dimensions_mm,
            holes,
            pockets,
            translation_mm,
            rotation,
        } = operation
        else {
            return Err(assistant_planning_rejection(
                "planning.panel_expected",
                "create_panel",
                &document_target,
                "plan_panel_batch accepts only create_panel operations.",
                "Pass create_panel operations only.",
            ));
        };
        AssistantCadEditProgram {
            operations: vec![operation.clone()],
        }
        .validate()
        .map_err(|error| {
            assistant_rejection(
                AssistantRejectionPhase::IntentValidation,
                "intent.panel_invalid",
                "create_panel",
                &format!("panel:{name}"),
                error,
                "Fix the panel dimensions, holes or pockets named in the message.",
                true,
            )
        })?;
        plan_panel(
            name,
            *dimensions_mm,
            holes,
            pockets,
            *translation_mm,
            rotation.clone(),
            &mut staged,
            &mut next_definition,
            &mut next_feature,
            &mut next_occurrence,
            &document_target,
        )?;
    }
    Ok(staged.into_final_batch())
}

pub fn plan_assistant_cad_edit_program(
    document: &DocumentStore,
    current_selection: &BTreeSet<OccurrenceId>,
    topology_results: &ExactResultRegistry,
    program: &AssistantCadEditProgram,
) -> Result<CommandBatch, Box<AssistantRejectionDiagnostic>> {
    plan_assistant_cad_edit_program_with_outputs(
        document,
        current_selection,
        topology_results,
        program,
    )
    .map(|plan| plan.batch)
}
