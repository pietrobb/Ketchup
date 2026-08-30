use super::*;
use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
    AssemblyRecomputePublishError, AssemblyRecomputeStatus, AssemblySolveResult,
    AssemblySolveStatus, AssemblySolverPolicy, recompute_rigid_assembly, solve_rigid_assembly,
};
use ketchup_core::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    AssemblyMotionDriver, AssemblyMotionStudy, AssemblyMotionStudyId,
    solve_assembly_joint_kinematics_with_drivers,
};
#[cfg(debug_assertions)]
use ketchup_core::drawing::project_orthographic_drawing;
use ketchup_core::drawing::{
    DrawingSheet, DrawingSheetId, DrawingSource, prepare_create_drawing_sheet,
};
use ketchup_core::exact_product::BodySubshapeRef;
#[cfg(debug_assertions)]
use ketchup_core::release_capstone::ReleaseCapstoneContract;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MateKindChoice {
    #[default]
    CoincidentPlanar,
    ConcentricAxial,
    Distance,
    Angle,
}

impl MateKindChoice {
    const ALL: [Self; 4] = [
        Self::CoincidentPlanar,
        Self::ConcentricAxial,
        Self::Distance,
        Self::Angle,
    ];

    const fn label_key(self) -> &'static str {
        match self {
            Self::CoincidentPlanar => "assembly-mate-coincident",
            Self::ConcentricAxial => "assembly-mate-concentric",
            Self::Distance => "assembly-mate-distance",
            Self::Angle => "assembly-mate-angle",
        }
    }

    const fn needs_value(self) -> bool {
        !matches!(self, Self::ConcentricAxial)
    }

    const fn requires_planar_faces(self) -> bool {
        !matches!(self, Self::ConcentricAxial)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum AssemblyPreviewSource {
    InsertOccurrence {
        id: OccurrenceId,
        definition_id: DefinitionId,
        name: String,
        transform: Transform,
    },
    GroundOccurrence {
        id: OccurrenceId,
        grounded: bool,
    },
    SelectionDrawing {
        occurrence_ids: Vec<OccurrenceId>,
        sheet_id: DrawingSheetId,
        name: String,
    },
    Mate {
        mate: AssemblyMate,
        editing: bool,
    },
    RemoveMate(AssemblyMateId),
    Joint {
        joint: AssemblyJoint,
        selected_occurrences: [OccurrenceId; 2],
        editing: bool,
    },
    MotionStudy {
        study: AssemblyMotionStudy,
        selected_occurrences: [OccurrenceId; 2],
        editing: bool,
    },
    Solve,
}

impl AssemblyPreviewSource {
    const fn action_key(&self) -> &'static str {
        match self {
            Self::InsertOccurrence { .. } => "assembly-action-insert",
            Self::GroundOccurrence { grounded: true, .. } => "assembly-action-ground",
            Self::GroundOccurrence {
                grounded: false, ..
            } => "assembly-action-unground",
            Self::SelectionDrawing { .. } => "assembly-action-selection-drawing",
            Self::Mate { editing: true, .. } => "assembly-action-edit-mate",
            Self::Mate { editing: false, .. } => "assembly-action-create-mate",
            Self::RemoveMate(_) => "assembly-action-remove-mate",
            Self::Joint { editing: true, .. } => "assembly-action-edit-joint",
            Self::Joint { editing: false, .. } => "assembly-action-create-joint",
            Self::MotionStudy { editing: true, .. } => "assembly-action-edit-motion-study",
            Self::MotionStudy { editing: false, .. } => "assembly-action-create-motion-study",
            Self::Solve => "assembly-action-solve",
        }
    }

    const fn clear_occurrence_name(&self) -> bool {
        matches!(self, Self::InsertOccurrence { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct AssemblyPreviewPlan {
    source: AssemblyPreviewSource,
    proposal: Proposal,
    solve_required: bool,
    solve_result: Option<AssemblySolveResult>,
}

#[derive(Clone, Debug, PartialEq)]
struct AssemblyProposalPreview {
    plan: AssemblyPreviewPlan,
    action: String,
    clear_occurrence_name: bool,
}

#[derive(Default)]
pub(super) struct AssemblyEditorState {
    definition: Option<DefinitionId>,
    occurrence_name: String,
    endpoint_a: Option<OccurrenceId>,
    endpoint_b: Option<OccurrenceId>,
    reference_a: Option<String>,
    reference_b: Option<String>,
    kind: MateKindChoice,
    value_input: String,
    reversed: bool,
    selected_mate: Option<AssemblyMateId>,
    joint_position_input: String,
    motion_position_input: String,
    motion_name_input: String,
    preview: Option<AssemblyProposalPreview>,
    solve_result: Option<AssemblySolveResult>,
}

enum AssemblyUiAction {
    Insert,
    CreateSelectionDrawing,
    Ground(OccurrenceId, bool),
    SelectEndpointA(OccurrenceId),
    SelectEndpointB(OccurrenceId),
    EditMate(AssemblyMateId),
    RemoveMate(AssemblyMateId),
    PreviewMate,
    PreviewJoint,
    PreviewMotionStudy,
    Solve,
}

impl KetchupApp {
    fn assembly_references(
        &self,
        snapshot: &Snapshot,
        occurrence_id: OccurrenceId,
        kind: MateKindChoice,
    ) -> Vec<BodySubshapeRef> {
        let Some(occurrence) = snapshot.occurrence(occurrence_id) else {
            return Vec::new();
        };
        let Some(package) = self
            .exact_results
            .get_render(snapshot, occurrence.definition_id())
        else {
            return Vec::new();
        };
        let mut references = package
            .references()
            .iter()
            .filter(|reference| {
                !kind.requires_planar_faces() || reference.expected_type == "planar_face"
            })
            .cloned()
            .collect::<Vec<_>>();
        references.sort_by(|left, right| {
            left.semantic_role
                .cmp(&right.semantic_role)
                .then(left.source_element_id.cmp(&right.source_element_id))
                .then(left.lineage_digest.cmp(&right.lineage_digest))
        });
        references
    }

    fn reconcile_assembly_editor(&mut self, snapshot: &Snapshot) {
        let definitions = snapshot
            .definitions()
            .map(|definition| definition.id())
            .collect::<Vec<_>>();
        if self
            .assembly_editor
            .definition
            .is_none_or(|id| !definitions.contains(&id))
        {
            self.assembly_editor.definition = definitions.first().copied();
            self.assembly_editor.occurrence_name.clear();
        }
        let occurrences = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id())
            .collect::<Vec<_>>();
        if self
            .assembly_editor
            .endpoint_a
            .is_none_or(|id| !occurrences.contains(&id))
        {
            self.assembly_editor.endpoint_a = occurrences.first().copied();
            self.assembly_editor.reference_a = None;
        }
        if self
            .assembly_editor
            .endpoint_b
            .is_none_or(|id| !occurrences.contains(&id))
        {
            self.assembly_editor.endpoint_b = occurrences.get(1).copied();
            self.assembly_editor.reference_b = None;
        }
        if self
            .assembly_editor
            .selected_mate
            .is_some_and(|id| snapshot.assembly_mate(id).is_none())
        {
            self.assembly_editor.selected_mate = None;
        }
        if self.assembly_editor.value_input.is_empty() {
            self.assembly_editor.value_input = "0".to_owned();
        }
        if self.assembly_editor.joint_position_input.is_empty() {
            self.assembly_editor.joint_position_input = "0".to_owned();
        }
        if self.assembly_editor.motion_position_input.is_empty() {
            self.assembly_editor.motion_position_input = "25".to_owned();
        }
        if self.assembly_editor.motion_name_input.is_empty() {
            self.assembly_editor.motion_name_input = "Motion study".to_owned();
        }

        let kind = self.assembly_editor.kind;
        let references_a = self
            .assembly_editor
            .endpoint_a
            .map_or_else(Vec::new, |id| self.assembly_references(snapshot, id, kind));
        if self
            .assembly_editor
            .reference_a
            .as_ref()
            .is_none_or(|digest| {
                !references_a
                    .iter()
                    .any(|reference| &reference.lineage_digest == digest)
            })
        {
            self.assembly_editor.reference_a = references_a
                .first()
                .map(|reference| reference.lineage_digest.clone());
        }
        let references_b = self
            .assembly_editor
            .endpoint_b
            .map_or_else(Vec::new, |id| self.assembly_references(snapshot, id, kind));
        if self
            .assembly_editor
            .reference_b
            .as_ref()
            .is_none_or(|digest| {
                !references_b
                    .iter()
                    .any(|reference| &reference.lineage_digest == digest)
            })
        {
            self.assembly_editor.reference_b = references_b
                .first()
                .map(|reference| reference.lineage_digest.clone());
        }
    }

    fn assembly_error(&mut self, reason: impl ToString) {
        self.digest = self.catalog.format(
            "assembly-error",
            &BTreeMap::from([("reason", reason.to_string())]),
        );
    }

    fn assembly_preview_solve_is_acceptable(result: Option<&AssemblySolveResult>) -> bool {
        result.is_none_or(|result| {
            matches!(
                result.status(),
                AssemblySolveStatus::UnderConstrained | AssemblySolveStatus::FullyConstrained
            )
        })
    }

    fn derive_assembly_preview_solve(
        &self,
        proposal: &Proposal,
    ) -> Result<Option<AssemblySolveResult>, String> {
        let current = self.document.current();
        if proposal.document_id() != current.document_id()
            || proposal.provenance_revision() != current.revision_id()
            || proposal.provenance_digest() != current.canonical_digest()
        {
            return Err(self.catalog.text("error-preview-stale"));
        }
        let candidate = self
            .document
            .preview_batch(proposal.batch())
            .map_err(|error| error.to_string())?;
        if candidate.assembly_mates().next().is_none() {
            return Ok(None);
        }
        solve_rigid_assembly(&candidate, AssemblySolverPolicy::default())
            .map(Some)
            .map_err(|error| error.to_string())
    }

    pub(super) fn derive_assembly_preview_proposal(
        &self,
        source: &AssemblyPreviewSource,
    ) -> Result<Proposal, String> {
        let snapshot = self.document.current();
        let batch = match source {
            AssemblyPreviewSource::InsertOccurrence {
                id,
                definition_id,
                name,
                transform,
            } => {
                if name.trim().is_empty() || snapshot.definition(*definition_id).is_none() {
                    return Err(self.catalog.text("assembly-error-name"));
                }
                CommandBatch::new(vec![CanonicalCommand::CreateOccurrence {
                    id: *id,
                    definition_id: *definition_id,
                    name: name.clone(),
                    transform: *transform,
                    parent: None,
                    tag: None,
                    visible: true,
                }])
            }
            AssemblyPreviewSource::GroundOccurrence { id, grounded } => {
                CommandBatch::new(vec![CanonicalCommand::SetOccurrenceGrounded {
                    id: *id,
                    grounded: *grounded,
                }])
            }
            AssemblyPreviewSource::SelectionDrawing {
                occurrence_ids,
                sheet_id,
                name,
            } => {
                return self.plan_selection_drawing(occurrence_ids, *sheet_id, name);
            }
            AssemblyPreviewSource::Mate { mate, editing } => {
                let existing = snapshot.assembly_mate(mate.id());
                if *editing != existing.is_some() {
                    return Err(self.catalog.text("error-preview-stale"));
                }
                let commands = if let Some(existing) = existing {
                    if existing.endpoint_a() == mate.endpoint_a()
                        && existing.endpoint_b() == mate.endpoint_b()
                    {
                        vec![CanonicalCommand::SetAssemblyMateKind {
                            id: mate.id(),
                            kind: mate.kind(),
                        }]
                    } else {
                        vec![
                            CanonicalCommand::DeleteAssemblyMate { id: mate.id() },
                            CanonicalCommand::CreateAssemblyMate(mate.clone()),
                        ]
                    }
                } else {
                    vec![CanonicalCommand::CreateAssemblyMate(mate.clone())]
                };
                CommandBatch::new(commands)
            }
            AssemblyPreviewSource::RemoveMate(id) => {
                CommandBatch::new(vec![CanonicalCommand::DeleteAssemblyMate { id: *id }])
            }
            AssemblyPreviewSource::Joint {
                joint,
                selected_occurrences,
                editing,
            } => {
                if self
                    .selected_occurrence_ids()
                    .into_iter()
                    .collect::<Vec<_>>()
                    != selected_occurrences.to_vec()
                    || snapshot.occurrence(selected_occurrences[0]).is_none()
                    || snapshot.occurrence(selected_occurrences[1]).is_none()
                    || *editing != snapshot.assembly_joint(joint.id()).is_some()
                {
                    return Err(self.catalog.text("error-preview-stale"));
                }
                if *editing {
                    let position = joint
                        .kind()
                        .position()
                        .ok_or_else(|| self.catalog.text("assembly-error-joint-position"))?;
                    let solution = solve_assembly_joint_kinematics_with_drivers(
                        &snapshot,
                        &[AssemblyMotionDriver::new(joint.id(), position)],
                    )
                    .map_err(|error| error.to_string())?;
                    let transforms = solution
                        .poses()
                        .iter()
                        .filter_map(|pose| {
                            snapshot
                                .occurrence(pose.occurrence_id())
                                .filter(|occurrence| {
                                    occurrence.transform() != pose.local_transform()
                                })
                                .map(|_| (pose.occurrence_id(), pose.local_transform()))
                        })
                        .collect::<Vec<_>>();
                    CommandBatch::new(vec![
                        CanonicalCommand::SetAssemblyJointPosition {
                            id: joint.id(),
                            position,
                        },
                        CanonicalCommand::ApplyAssemblySolve {
                            source_revision: snapshot.revision_id(),
                            source_digest: snapshot.canonical_digest(),
                            transforms,
                        },
                    ])
                } else {
                    CommandBatch::new(vec![CanonicalCommand::CreateAssemblyJoint(joint.clone())])
                }
            }
            AssemblyPreviewSource::MotionStudy {
                study,
                selected_occurrences,
                editing,
            } => {
                if self
                    .selected_occurrence_ids()
                    .into_iter()
                    .collect::<Vec<_>>()
                    != selected_occurrences.to_vec()
                    || *editing != snapshot.assembly_motion_study(study.id()).is_some()
                    || study
                        .drivers()
                        .iter()
                        .any(|driver| snapshot.assembly_joint(driver.joint_id()).is_none())
                {
                    return Err(self.catalog.text("error-preview-stale"));
                }
                let command = if *editing {
                    CanonicalCommand::UpdateAssemblyMotionStudy(study.clone())
                } else {
                    CanonicalCommand::CreateAssemblyMotionStudy(study.clone())
                };
                CommandBatch::new(vec![command])
            }
            AssemblyPreviewSource::Solve => return self.plan_assembly_solve(),
        };
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| error.to_string())
    }

    fn derive_assembly_preview_plan(
        &self,
        source: &AssemblyPreviewSource,
    ) -> Result<AssemblyPreviewPlan, String> {
        let proposal = self.derive_assembly_preview_proposal(source)?;
        let solve_result = self.derive_assembly_preview_solve(&proposal)?;
        if !Self::assembly_preview_solve_is_acceptable(solve_result.as_ref()) {
            return Err(self.catalog.text("assembly-error-solve-refused"));
        }
        Ok(AssemblyPreviewPlan {
            source: source.clone(),
            proposal,
            solve_required: solve_result.is_some(),
            solve_result,
        })
    }

    fn prepare_assembly_preview(&mut self, source: AssemblyPreviewSource) -> bool {
        let proposal = match self.derive_assembly_preview_proposal(&source) {
            Ok(proposal) => proposal,
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        let solve_result = match self.derive_assembly_preview_solve(&proposal) {
            Ok(result) => result,
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        self.assembly_editor.solve_result = solve_result.clone();
        if !Self::assembly_preview_solve_is_acceptable(solve_result.as_ref()) {
            self.assembly_error(self.catalog.text("assembly-error-solve-refused"));
            return false;
        }
        let action = self.catalog.text(source.action_key());
        self.assembly_editor.preview = Some(AssemblyProposalPreview {
            plan: AssemblyPreviewPlan {
                source: source.clone(),
                proposal,
                solve_required: solve_result.is_some(),
                solve_result,
            },
            action: action.clone(),
            clear_occurrence_name: source.clear_occurrence_name(),
        });
        self.digest = self.catalog.format(
            "assembly-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    fn preview_insert_occurrence(&mut self) -> bool {
        let snapshot = self.document.current();
        let Some(definition_id) = self.assembly_editor.definition else {
            self.assembly_error(self.catalog.text("assembly-error-definition"));
            return false;
        };
        if snapshot.definition(definition_id).is_none() {
            self.assembly_error(self.catalog.text("assembly-error-definition"));
            return false;
        }
        let name = self.assembly_editor.occurrence_name.trim().to_owned();
        if name.is_empty() {
            self.assembly_error(self.catalog.text("assembly-error-name"));
            return false;
        }
        let id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let x = snapshot.occurrences().count() as f64 * 25.0;
        let transform = Transform::from_translation(x, 0.0, 0.0)
            .expect("bounded insertion translation is finite");
        self.prepare_assembly_preview(AssemblyPreviewSource::InsertOccurrence {
            id,
            definition_id,
            name,
            transform,
        })
    }

    fn preview_ground_occurrence(&mut self, id: OccurrenceId, grounded: bool) -> bool {
        self.prepare_assembly_preview(AssemblyPreviewSource::GroundOccurrence { id, grounded })
    }

    pub(super) fn assembly_kinematic_selection(&self) -> Option<[OccurrenceId; 2]> {
        self.selected_occurrence_ids()
            .into_iter()
            .collect::<Vec<_>>()
            .try_into()
            .ok()
    }

    fn assembly_joint_preview_source(&self) -> Result<AssemblyPreviewSource, String> {
        let selected_occurrences = self
            .assembly_kinematic_selection()
            .ok_or_else(|| self.catalog.text("assembly-error-joint-selection"))?;
        let snapshot = self.document.current();
        let existing = snapshot.assembly_joints().find(|joint| {
            joint.parent_occurrence_id() == selected_occurrences[0]
                && joint.child_occurrence_id() == selected_occurrences[1]
        });
        let position_mm = self
            .assembly_editor
            .joint_position_input
            .trim()
            .parse::<f64>()
            .map_err(|_| self.catalog.text("assembly-error-joint-position"))?;
        let id = existing.map_or_else(
            || {
                AssemblyJointId(
                    snapshot
                        .assembly_joints()
                        .map(|joint| joint.id().0)
                        .max()
                        .unwrap_or(0)
                        + 1,
                )
            },
            AssemblyJoint::id,
        );
        Ok(AssemblyPreviewSource::Joint {
            joint: AssemblyJoint::new(
                id,
                selected_occurrences[0],
                selected_occurrences[1],
                AssemblyJointKind::Prismatic {
                    axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                    limits: Some(AssemblyJointLimits::new(-100.0, 100.0)),
                    position_mm,
                },
            ),
            selected_occurrences,
            editing: existing.is_some(),
        })
    }

    fn assembly_motion_study_preview_source(&self) -> Result<AssemblyPreviewSource, String> {
        let selected_occurrences = self
            .assembly_kinematic_selection()
            .ok_or_else(|| self.catalog.text("assembly-error-joint-selection"))?;
        let snapshot = self.document.current();
        let joint = snapshot
            .assembly_joints()
            .find(|joint| {
                joint.parent_occurrence_id() == selected_occurrences[0]
                    && joint.child_occurrence_id() == selected_occurrences[1]
            })
            .ok_or_else(|| self.catalog.text("assembly-error-motion-joint"))?;
        let position = self
            .assembly_editor
            .motion_position_input
            .trim()
            .parse::<f64>()
            .map_err(|_| self.catalog.text("assembly-error-motion-position"))?;
        let name = self.assembly_editor.motion_name_input.trim();
        if name.is_empty() {
            return Err(self.catalog.text("assembly-error-motion-name"));
        }
        let existing = snapshot.assembly_motion_studies().find(|study| {
            study
                .drivers()
                .iter()
                .any(|driver| driver.joint_id() == joint.id())
        });
        let id = existing.map_or_else(
            || {
                AssemblyMotionStudyId(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| study.id().0)
                        .max()
                        .unwrap_or(0)
                        + 1,
                )
            },
            AssemblyMotionStudy::id,
        );
        let reference_position = joint.kind().position().unwrap_or(0.0);
        let drivers = existing.map_or_else(
            || vec![AssemblyMotionDriver::new(joint.id(), position)],
            |study| {
                study
                    .drivers()
                    .iter()
                    .map(|driver| {
                        let coupled_position = if driver.joint_id() == joint.id() {
                            position
                        } else if reference_position.abs() > f64::EPSILON {
                            snapshot
                                .assembly_joint(driver.joint_id())
                                .and_then(|coupled| coupled.kind().position())
                                .map_or(driver.position(), |current| {
                                    current / reference_position * position
                                })
                        } else {
                            driver.position()
                        };
                        AssemblyMotionDriver::new(driver.joint_id(), coupled_position)
                    })
                    .collect()
            },
        );
        Ok(AssemblyPreviewSource::MotionStudy {
            study: AssemblyMotionStudy::new(id, name, drivers),
            selected_occurrences,
            editing: existing.is_some(),
        })
    }

    fn preview_assembly_joint_from_selection(&mut self) -> bool {
        match self.assembly_joint_preview_source() {
            Ok(source) => self.prepare_assembly_preview(source),
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    fn preview_assembly_motion_study_from_selection(&mut self) -> bool {
        match self.assembly_motion_study_preview_source() {
            Ok(source) => self.prepare_assembly_preview(source),
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    fn prepare_assistant_assembly_source(&mut self, source: AssemblyPreviewSource) -> bool {
        match self.derive_assembly_preview_proposal(&source) {
            Ok(proposal) => {
                self.digest = self.catalog.format(
                    "assistant-digest-preview",
                    &BTreeMap::from([
                        (
                            "reads",
                            proposal.authoritative_dependencies().len().to_string(),
                        ),
                        ("writes", proposal.authoritative_writes().len().to_string()),
                    ]),
                );
                self.status_key = "status-preview";
                self.assistant_verification = None;
                self.assistant_proposal = Some(AssistantPreviewPlan {
                    source: AssistantPreviewSource::Assembly(source),
                    proposal,
                    repair: None,
                });
                true
            }
            Err(error) => {
                self.assistant_proposal = None;
                self.assembly_error(error);
                false
            }
        }
    }

    pub fn prepare_assistant_assembly_joint_from_selection(&mut self) -> bool {
        match self.assembly_joint_preview_source() {
            Ok(source) => self.prepare_assistant_assembly_source(source),
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    pub fn prepare_assistant_motion_study_from_selection(&mut self) -> bool {
        match self.assembly_motion_study_preview_source() {
            Ok(source) => self.prepare_assistant_assembly_source(source),
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    fn plan_selection_drawing(
        &self,
        occurrence_ids: &[OccurrenceId],
        sheet_id: DrawingSheetId,
        name: &str,
    ) -> Result<Proposal, String> {
        let snapshot = self.document.current();
        let selected = self.selected_occurrence_ids();
        if occurrence_ids.is_empty()
            || occurrence_ids.iter().copied().collect::<BTreeSet<_>>() != selected
            || snapshot.drawing_sheet(sheet_id).is_some()
        {
            return Err(self.catalog.text("error-preview-stale"));
        }
        let source = if let [occurrence_id] = occurrence_ids {
            let definition_id = snapshot
                .occurrence(*occurrence_id)
                .ok_or_else(|| self.catalog.text("error-preview-stale"))?
                .definition_id();
            DrawingSource::Definition(definition_id)
        } else {
            if occurrence_ids
                .iter()
                .any(|occurrence_id| snapshot.occurrence(*occurrence_id).is_none())
            {
                return Err(self.catalog.text("error-preview-stale"));
            }
            DrawingSource::RigidAssembly {
                occurrence_ids: occurrence_ids.to_vec(),
            }
        };
        let sheet = DrawingSheet::new(sheet_id, name, source).map_err(|error| error.to_string())?;
        let (proposal, drawing) =
            prepare_create_drawing_sheet(&self.document, &self.exact_results, sheet)
                .map_err(|error| error.to_string())?;
        if drawing.views.len() != 3
            || drawing
                .views
                .iter()
                .any(|view| view.visible_lines.is_empty())
        {
            return Err(self.catalog.text("assembly-error-solve-refused"));
        }
        Ok(proposal)
    }

    fn preview_selection_drawing(&mut self) -> bool {
        let snapshot = self.document.current();
        let occurrence_ids = self
            .selected_occurrence_ids()
            .into_iter()
            .collect::<Vec<_>>();
        if occurrence_ids.is_empty() {
            self.assembly_error(self.catalog.text("assembly-error-drawing-selection"));
            return false;
        }
        let sheet_id = DrawingSheetId(
            snapshot
                .drawing_sheets()
                .map(|sheet| sheet.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let name = if let [occurrence_id] = occurrence_ids.as_slice() {
            let Some(occurrence) = snapshot.occurrence(*occurrence_id) else {
                self.assembly_error(self.catalog.text("assembly-error-drawing-selection"));
                return false;
            };
            self.catalog.format(
                "assembly-selection-drawing-part-name",
                &BTreeMap::from([("name", occurrence.name().to_owned())]),
            )
        } else {
            self.catalog.format(
                "assembly-selection-drawing-name",
                &BTreeMap::from([("count", occurrence_ids.len().to_string())]),
            )
        };
        self.prepare_assembly_preview(AssemblyPreviewSource::SelectionDrawing {
            occurrence_ids,
            sheet_id,
            name,
        })
    }

    fn selected_assembly_reference(
        &self,
        snapshot: &Snapshot,
        occurrence_id: OccurrenceId,
        digest: &str,
    ) -> Option<BodySubshapeRef> {
        self.assembly_references(snapshot, occurrence_id, self.assembly_editor.kind)
            .into_iter()
            .find(|reference| reference.lineage_digest == digest)
    }

    fn editor_mate_kind(&self) -> Option<AssemblyMateKind> {
        let value = self
            .assembly_editor
            .value_input
            .trim()
            .parse::<f64>()
            .ok()?;
        let kind = match self.assembly_editor.kind {
            MateKindChoice::CoincidentPlanar => AssemblyMateKind::CoincidentPlanar {
                offset_mm: value,
                reversed: self.assembly_editor.reversed,
            },
            MateKindChoice::ConcentricAxial => AssemblyMateKind::ConcentricAxial {
                reversed: self.assembly_editor.reversed,
            },
            MateKindChoice::Distance => AssemblyMateKind::Distance { distance_mm: value },
            MateKindChoice::Angle => AssemblyMateKind::Angle {
                angle_degrees: value,
            },
        };
        kind.is_valid().then_some(kind)
    }

    fn preview_editor_mate(&mut self) -> bool {
        let snapshot = self.document.current();
        let (Some(occurrence_a), Some(occurrence_b)) = (
            self.assembly_editor.endpoint_a,
            self.assembly_editor.endpoint_b,
        ) else {
            self.assembly_error(self.catalog.text("assembly-error-endpoints"));
            return false;
        };
        if occurrence_a == occurrence_b {
            self.assembly_error(self.catalog.text("assembly-error-endpoints"));
            return false;
        }
        let (Some(reference_a_digest), Some(reference_b_digest)) = (
            self.assembly_editor.reference_a.as_deref(),
            self.assembly_editor.reference_b.as_deref(),
        ) else {
            self.assembly_error(self.catalog.text("assembly-error-references"));
            return false;
        };
        let Some(reference_a) =
            self.selected_assembly_reference(&snapshot, occurrence_a, reference_a_digest)
        else {
            self.assembly_error(self.catalog.text("assembly-error-references"));
            return false;
        };
        let Some(reference_b) =
            self.selected_assembly_reference(&snapshot, occurrence_b, reference_b_digest)
        else {
            self.assembly_error(self.catalog.text("assembly-error-references"));
            return false;
        };
        let Some(kind) = self.editor_mate_kind() else {
            self.assembly_error(self.catalog.text("assembly-error-value"));
            return false;
        };
        let id = self.assembly_editor.selected_mate.unwrap_or_else(|| {
            AssemblyMateId(
                snapshot
                    .assembly_mates()
                    .map(|mate| mate.id().0)
                    .max()
                    .unwrap_or(0)
                    + 1,
            )
        });
        let mate = AssemblyMate::new(
            id,
            AssemblyMateEndpoint::resolved(occurrence_a, reference_a),
            AssemblyMateEndpoint::resolved(occurrence_b, reference_b),
            kind,
        );
        self.prepare_assembly_preview(AssemblyPreviewSource::Mate {
            mate,
            editing: self.assembly_editor.selected_mate.is_some(),
        })
    }

    fn edit_assembly_mate(&mut self, id: AssemblyMateId) {
        let snapshot = self.document.current();
        let Some(mate) = snapshot.assembly_mate(id) else {
            return;
        };
        self.assembly_editor.selected_mate = Some(id);
        self.assembly_editor.endpoint_a = Some(mate.endpoint_a().occurrence_id());
        self.assembly_editor.endpoint_b = Some(mate.endpoint_b().occurrence_id());
        self.assembly_editor.reference_a =
            Some(mate.endpoint_a().reference().lineage_digest.clone());
        self.assembly_editor.reference_b =
            Some(mate.endpoint_b().reference().lineage_digest.clone());
        match mate.kind() {
            AssemblyMateKind::CoincidentPlanar {
                offset_mm,
                reversed,
            } => {
                self.assembly_editor.kind = MateKindChoice::CoincidentPlanar;
                self.assembly_editor.value_input = format_height(offset_mm);
                self.assembly_editor.reversed = reversed;
            }
            AssemblyMateKind::ConcentricAxial { reversed } => {
                self.assembly_editor.kind = MateKindChoice::ConcentricAxial;
                self.assembly_editor.value_input = "0".to_owned();
                self.assembly_editor.reversed = reversed;
            }
            AssemblyMateKind::Distance { distance_mm } => {
                self.assembly_editor.kind = MateKindChoice::Distance;
                self.assembly_editor.value_input = format_height(distance_mm);
                self.assembly_editor.reversed = false;
            }
            AssemblyMateKind::Angle { angle_degrees } => {
                self.assembly_editor.kind = MateKindChoice::Angle;
                self.assembly_editor.value_input = format_height(angle_degrees);
                self.assembly_editor.reversed = false;
            }
        }
    }

    fn preview_remove_assembly_mate(&mut self, id: AssemblyMateId) -> bool {
        self.prepare_assembly_preview(AssemblyPreviewSource::RemoveMate(id))
    }

    fn plan_assembly_solve(&self) -> Result<Proposal, String> {
        let recomputed = recompute_rigid_assembly(
            &self.document,
            &self.exact_results,
            AssemblySolverPolicy::default(),
        )
        .map_err(|error| error.to_string())?;
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            return Err(self.catalog.text("assembly-error-solve-refused"));
        }
        recomputed
            .prepare_publication(&self.document)
            .map_err(|error| error.to_string())
    }

    fn preview_assembly_solve(&mut self) -> bool {
        let recomputed = match recompute_rigid_assembly(
            &self.document,
            &self.exact_results,
            AssemblySolverPolicy::default(),
        ) {
            Ok(result) => result,
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        self.assembly_editor.solve_result = recomputed.solve().cloned();
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            self.assembly_error(self.catalog.text("assembly-error-solve-refused"));
            return false;
        }
        match recomputed.prepare_publication(&self.document) {
            Ok(_) => self.prepare_assembly_preview(AssemblyPreviewSource::Solve),
            Err(AssemblyRecomputePublishError::NoCanonicalChanges) => {
                self.digest = self.catalog.text("assembly-solve-current");
                true
            }
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    fn confirm_assembly_preview(&mut self) -> bool {
        let Some(preview) = self.assembly_editor.preview.take() else {
            return false;
        };
        let rederived = self.derive_assembly_preview_plan(&preview.plan.source);
        if rederived.as_ref() != Ok(&preview.plan)
            || preview.action != self.catalog.text(preview.plan.source.action_key())
            || preview.clear_occurrence_name != preview.plan.source.clear_occurrence_name()
        {
            self.assembly_error(self.catalog.text("error-preview-stale"));
            return false;
        }
        match self
            .document
            .commit_verified_proposal(&preview.plan.proposal)
        {
            Ok(_) => {
                if preview.clear_occurrence_name {
                    self.assembly_editor.occurrence_name.clear();
                }
                self.assembly_editor.selected_mate = None;
                self.assembly_editor.solve_result = None;
                self.reconcile_selection();
                self.digest = self.catalog.format(
                    "assembly-committed",
                    &BTreeMap::from([("action", preview.action)]),
                );
                true
            }
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    fn cancel_assembly_preview(&mut self) {
        self.assembly_editor.preview = None;
        self.assembly_editor.solve_result = None;
        self.digest = self.catalog.text("assembly-cancelled");
    }

    fn show_assembly_solve_diagnostic(
        &self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        result: Option<&AssemblySolveResult>,
    ) {
        let Some(result) = result else {
            return;
        };
        let source_is_current = result.document_id() == snapshot.document_id()
            && result.source_revision() == snapshot.revision_id()
            && result.source_digest() == snapshot.canonical_digest();
        let status = self.catalog.text(match result.status() {
            AssemblySolveStatus::UnderConstrained => "assembly-solve-under",
            AssemblySolveStatus::FullyConstrained => "assembly-solve-fully",
            AssemblySolveStatus::OverConstrained => "assembly-solve-over",
            AssemblySolveStatus::Failed => "assembly-solve-failed",
        });
        ui.label(self.catalog.format(
            "assembly-solve-summary",
            &BTreeMap::from([
                ("status", status),
                ("dof", result.remaining_dof().to_string()),
                ("current", source_is_current.to_string()),
            ]),
        ));
    }

    pub(super) fn show_assembly_editor(&mut self, ui: &mut egui::Ui) {
        let title = self.catalog.text("assembly-title");
        egui::CollapsingHeader::new(title)
            .id_salt("assembly-editor")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("assembly-editor-scroll")
                    .max_height(700.0)
                    .show(ui, |ui| self.show_assembly_editor_content(ui));
            });
        ui.separator();
    }

    fn show_assembly_editor_content(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        self.reconcile_assembly_editor(&snapshot);

        if let Some(preview) = self.assembly_editor.preview.as_ref() {
            ui.label(self.catalog.format(
                "assembly-preview-observational",
                &BTreeMap::from([("action", preview.action.clone())]),
            ));
            self.show_assembly_solve_diagnostic(ui, &snapshot, preview.plan.solve_result.as_ref());
            let confirm = ui
                .button(self.catalog.text("assembly-confirm-preview"))
                .clicked();
            let cancel = ui
                .button(self.catalog.text("assembly-cancel-preview"))
                .clicked();
            if confirm {
                self.confirm_assembly_preview();
            } else if cancel {
                self.cancel_assembly_preview();
            }
            ui.separator();
            return;
        }

        let definitions = snapshot
            .definitions()
            .map(|definition| (definition.id(), definition.name().to_owned()))
            .collect::<Vec<_>>();
        if let Some(mut selected) = self.assembly_editor.definition {
            let label = self.catalog.text("assembly-definition");
            let selected_name = definitions
                .iter()
                .find(|(id, _)| *id == selected)
                .map_or_else(|| selected.0.to_string(), |(_, name)| name.clone());
            let response = egui::ComboBox::from_id_salt("assembly-definition")
                .width(ui.available_width())
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (id, name) in &definitions {
                        ui.selectable_value(&mut selected, *id, name);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
            if self.assembly_editor.definition != Some(selected) {
                self.assembly_editor.definition = Some(selected);
                self.assembly_editor.occurrence_name.clear();
            }
        }
        if self.assembly_editor.occurrence_name.is_empty()
            && let Some(definition_id) = self.assembly_editor.definition
            && let Some(definition) = snapshot.definition(definition_id)
        {
            let count = snapshot
                .occurrences()
                .filter(|occurrence| occurrence.definition_id() == definition_id)
                .count()
                + 1;
            self.assembly_editor.occurrence_name = self.catalog.format(
                "assembly-default-occurrence-name",
                &BTreeMap::from([
                    ("definition", definition.name().to_owned()),
                    ("number", count.to_string()),
                ]),
            );
        }
        let name_label = self.catalog.text("assembly-occurrence-name");
        let name = ui.add(
            egui::TextEdit::singleline(&mut self.assembly_editor.occurrence_name)
                .hint_text(&name_label),
        );
        name.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &name_label)
        });
        let mut action = ui
            .button(self.catalog.text("assembly-preview-insert"))
            .clicked()
            .then_some(AssemblyUiAction::Insert);
        if ui
            .add_enabled(
                !self.selected_occurrence_ids().is_empty(),
                egui::Button::new(self.catalog.text("assembly-preview-selection-drawing")),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::CreateSelectionDrawing);
        }
        ui.label(self.catalog.text("assembly-occurrences"));
        for occurrence in snapshot.occurrences() {
            let id = occurrence.id();
            let grounded = snapshot.occurrence_is_grounded(id);
            let remaining = self
                .assembly_editor
                .solve_result
                .as_ref()
                .and_then(|result| result.occurrence(id))
                .map_or_else(
                    || {
                        if grounded {
                            "0".to_owned()
                        } else {
                            self.catalog.text("assembly-dof-pending")
                        }
                    },
                    |solved| solved.remaining_dof().to_string(),
                );
            ui.label(self.catalog.format(
                "assembly-occurrence-row",
                &BTreeMap::from([
                    ("name", occurrence.name().to_owned()),
                    ("id", id.0.to_string()),
                    ("dof", remaining),
                ]),
            ));
            ui.horizontal_wrapped(|ui| {
                let ground_key = if grounded {
                    "assembly-preview-unground"
                } else {
                    "assembly-preview-ground"
                };
                let ground_label = self.catalog.format(
                    ground_key,
                    &BTreeMap::from([("name", occurrence.name().to_owned())]),
                );
                if ui.button(ground_label).clicked() {
                    action = Some(AssemblyUiAction::Ground(id, !grounded));
                }
                let endpoint_a = self.catalog.format(
                    "assembly-use-endpoint-a",
                    &BTreeMap::from([("name", occurrence.name().to_owned())]),
                );
                if ui.button(endpoint_a).clicked() {
                    action = Some(AssemblyUiAction::SelectEndpointA(id));
                }
                let endpoint_b = self.catalog.format(
                    "assembly-use-endpoint-b",
                    &BTreeMap::from([("name", occurrence.name().to_owned())]),
                );
                if ui.button(endpoint_b).clicked() {
                    action = Some(AssemblyUiAction::SelectEndpointB(id));
                }
            });
        }

        ui.separator();
        ui.label(self.catalog.text("assembly-kinematics-title"));
        let joint_position_label = self.catalog.text("assembly-joint-position");
        let joint_position = ui.add(
            egui::TextEdit::singleline(&mut self.assembly_editor.joint_position_input)
                .hint_text(&joint_position_label),
        );
        joint_position.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &joint_position_label)
        });
        let has_kinematic_selection = self.assembly_kinematic_selection().is_some();
        if ui
            .add_enabled(
                has_kinematic_selection,
                egui::Button::new(self.catalog.text("assembly-preview-joint")),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::PreviewJoint);
        }
        let motion_name_label = self.catalog.text("assembly-motion-name");
        let motion_name = ui.add(
            egui::TextEdit::singleline(&mut self.assembly_editor.motion_name_input)
                .hint_text(&motion_name_label),
        );
        motion_name.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &motion_name_label)
        });
        let motion_position_label = self.catalog.text("assembly-motion-position");
        let motion_position = ui.add(
            egui::TextEdit::singleline(&mut self.assembly_editor.motion_position_input)
                .hint_text(&motion_position_label),
        );
        motion_position.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &motion_position_label)
        });
        if ui
            .add_enabled(
                has_kinematic_selection,
                egui::Button::new(self.catalog.text("assembly-preview-motion-study")),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::PreviewMotionStudy);
        }
        ui.separator();

        let previous_kind = self.assembly_editor.kind;
        let kind_label = self.catalog.text("assembly-mate-kind");
        let kind_response = egui::ComboBox::from_id_salt("assembly-mate-kind")
            .width(ui.available_width())
            .selected_text(self.catalog.text(self.assembly_editor.kind.label_key()))
            .show_ui(ui, |ui| {
                for kind in MateKindChoice::ALL {
                    ui.selectable_value(
                        &mut self.assembly_editor.kind,
                        kind,
                        self.catalog.text(kind.label_key()),
                    );
                }
            });
        kind_response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &kind_label)
        });
        if previous_kind != self.assembly_editor.kind {
            self.assembly_editor.reference_a = None;
            self.assembly_editor.reference_b = None;
        }
        if self.assembly_editor.kind.needs_value() {
            let value_label = self.catalog.text(match self.assembly_editor.kind {
                MateKindChoice::Angle => "assembly-angle-value",
                _ => "assembly-distance-value",
            });
            let value = ui.add(
                egui::TextEdit::singleline(&mut self.assembly_editor.value_input)
                    .hint_text(&value_label),
            );
            value.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &value_label)
            });
        }
        if matches!(
            self.assembly_editor.kind,
            MateKindChoice::CoincidentPlanar | MateKindChoice::ConcentricAxial
        ) {
            ui.checkbox(
                &mut self.assembly_editor.reversed,
                self.catalog.text("assembly-reversed"),
            );
        }

        let references_a = self.assembly_editor.endpoint_a.map_or_else(Vec::new, |id| {
            self.assembly_references(&snapshot, id, self.assembly_editor.kind)
        });
        let references_b = self.assembly_editor.endpoint_b.map_or_else(Vec::new, |id| {
            self.assembly_references(&snapshot, id, self.assembly_editor.kind)
        });
        for (side, references, selected) in [
            ("a", &references_a, &mut self.assembly_editor.reference_a),
            ("b", &references_b, &mut self.assembly_editor.reference_b),
        ] {
            let label_key = if side == "a" {
                "assembly-reference-a"
            } else {
                "assembly-reference-b"
            };
            let label = self.catalog.text(label_key);
            let selected_text = selected
                .as_ref()
                .and_then(|digest| {
                    references
                        .iter()
                        .find(|reference| &reference.lineage_digest == digest)
                })
                .map_or_else(
                    || self.catalog.text("assembly-reference-unavailable"),
                    |reference| reference.semantic_role.clone(),
                );
            let response = egui::ComboBox::from_id_salt(("assembly-reference", side))
                .width(ui.available_width())
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for reference in references {
                        ui.selectable_value(
                            selected,
                            Some(reference.lineage_digest.clone()),
                            &reference.semantic_role,
                        );
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
        }

        let can_preview_mate = self.assembly_editor.endpoint_a.is_some()
            && self.assembly_editor.endpoint_b.is_some()
            && self.assembly_editor.reference_a.is_some()
            && self.assembly_editor.reference_b.is_some();
        if ui
            .add_enabled(
                can_preview_mate,
                egui::Button::new(self.catalog.text(
                    if self.assembly_editor.selected_mate.is_some() {
                        "assembly-preview-update-mate"
                    } else {
                        "assembly-preview-create-mate"
                    },
                )),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::PreviewMate);
        }

        for mate in snapshot.assembly_mates() {
            ui.horizontal_wrapped(|ui| {
                ui.label(self.catalog.format(
                    "assembly-mate-row",
                    &BTreeMap::from([("id", mate.id().0.to_string())]),
                ));
                if ui
                    .button(self.catalog.format(
                        "assembly-edit-mate",
                        &BTreeMap::from([("id", mate.id().0.to_string())]),
                    ))
                    .clicked()
                {
                    action = Some(AssemblyUiAction::EditMate(mate.id()));
                }
                if ui
                    .button(self.catalog.format(
                        "assembly-remove-mate",
                        &BTreeMap::from([("id", mate.id().0.to_string())]),
                    ))
                    .clicked()
                {
                    action = Some(AssemblyUiAction::RemoveMate(mate.id()));
                }
            });
        }
        if ui
            .add_enabled(
                snapshot.assembly_mates().next().is_some(),
                egui::Button::new(self.catalog.text("assembly-preview-solve")),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::Solve);
        }
        self.show_assembly_solve_diagnostic(
            ui,
            &snapshot,
            self.assembly_editor.solve_result.as_ref(),
        );
        ui.separator();

        match action {
            Some(AssemblyUiAction::Insert) => {
                self.preview_insert_occurrence();
            }
            Some(AssemblyUiAction::CreateSelectionDrawing) => {
                self.preview_selection_drawing();
            }
            Some(AssemblyUiAction::Ground(id, grounded)) => {
                self.preview_ground_occurrence(id, grounded);
            }
            Some(AssemblyUiAction::SelectEndpointA(id)) => {
                self.assembly_editor.endpoint_a = Some(id);
                self.assembly_editor.reference_a = None;
            }
            Some(AssemblyUiAction::SelectEndpointB(id)) => {
                self.assembly_editor.endpoint_b = Some(id);
                self.assembly_editor.reference_b = None;
            }
            Some(AssemblyUiAction::EditMate(id)) => self.edit_assembly_mate(id),
            Some(AssemblyUiAction::RemoveMate(id)) => {
                self.preview_remove_assembly_mate(id);
            }
            Some(AssemblyUiAction::PreviewMate) => {
                self.preview_editor_mate();
            }
            Some(AssemblyUiAction::PreviewJoint) => {
                self.preview_assembly_joint_from_selection();
            }
            Some(AssemblyUiAction::PreviewMotionStudy) => {
                self.preview_assembly_motion_study_from_selection();
            }
            Some(AssemblyUiAction::Solve) => {
                self.preview_assembly_solve();
            }
            None => {}
        }
    }

    #[must_use]
    pub fn assembly_preview_pending(&self) -> bool {
        self.assembly_editor.preview.is_some()
    }

    #[must_use]
    pub fn assembly_mate_count(&self) -> usize {
        self.document.current().assembly_mates().count()
    }

    #[must_use]
    pub fn assembly_joint_count(&self) -> usize {
        self.document.current().assembly_joints().count()
    }

    #[must_use]
    pub fn assembly_motion_study_count(&self) -> usize {
        self.document.current().assembly_motion_studies().count()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn headless_set_assembly_motion_position(&mut self, position: f64) {
        self.assembly_editor.motion_position_input = position.to_string();
    }

    #[must_use]
    pub fn grounded_occurrence_count(&self) -> usize {
        self.document.current().grounded_occurrences().count()
    }

    #[must_use]
    pub fn assembly_endpoint_references_ready(&self) -> (bool, bool) {
        let snapshot = self.document.current();
        (
            self.assembly_editor.endpoint_a.is_some()
                && self.assembly_editor.reference_a.is_some()
                && self.assembly_editor.endpoint_a.is_some_and(|id| {
                    !self
                        .assembly_references(&snapshot, id, self.assembly_editor.kind)
                        .is_empty()
                }),
            self.assembly_editor.endpoint_b.is_some()
                && self.assembly_editor.reference_b.is_some()
                && self.assembly_editor.endpoint_b.is_some_and(|id| {
                    !self
                        .assembly_references(&snapshot, id, self.assembly_editor.kind)
                        .is_empty()
                }),
        )
    }

    #[must_use]
    pub fn assembly_solve_status(&self) -> Option<AssemblySolveStatus> {
        self.assembly_editor
            .preview
            .as_ref()
            .and_then(|preview| preview.plan.solve_result.as_ref())
            .or(self.assembly_editor.solve_result.as_ref())
            .map(AssemblySolveResult::status)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn headless_capstone_assembly_summary(
        &self,
    ) -> Option<(AssemblySolveStatus, usize, usize, usize)> {
        let solved =
            solve_rigid_assembly(&self.document.current(), AssemblySolverPolicy::default()).ok()?;
        Some((
            solved.status(),
            solved.remaining_dof(),
            solved.redundant_mate_ids().len(),
            solved.conflicting_mate_ids().len(),
        ))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn headless_drawing_fingerprint(
        &self,
        sheet_id: DrawingSheetId,
    ) -> Option<(String, Vec<&'static str>)> {
        let snapshot = self.document.current();
        let sheet = snapshot.drawing_sheet(sheet_id)?;
        let drawing = project_orthographic_drawing(&snapshot, &self.exact_results, sheet).ok()?;
        Some((
            drawing.result_digest,
            drawing
                .views
                .iter()
                .map(|view| view.kind.stable_name())
                .collect(),
        ))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn headless_capstone_drawing_fingerprint(&self) -> Option<(String, Vec<&'static str>)> {
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let snapshot = self.document.current();
        let sheet = snapshot.drawing_sheet(contract.drawing_sheet_id)?;
        let drawing = project_orthographic_drawing(&snapshot, &self.exact_results, sheet).ok()?;
        Some((
            drawing.result_digest,
            drawing
                .views
                .iter()
                .map(|view| view.kind.stable_name())
                .collect(),
        ))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn headless_capstone_assembly_refusal_paths(&self) -> Vec<&'static str> {
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let snapshot = self.document.current();
        let Some(planar) = snapshot.assembly_mate(contract.planar_mate_id).cloned() else {
            return Vec::new();
        };
        let Some(axial) = snapshot.assembly_mate(contract.axial_mate_id).cloned() else {
            return Vec::new();
        };
        let encoded = ketchup_core::persistence::save(&snapshot);
        let clone_document = || {
            let mut document = ketchup_core::persistence::load(&encoded)
                .ok()?
                .into_editable()
                .ok()?;
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::DeleteDrawingSheet {
                        id: contract.drawing_sheet_id,
                    },
                ]))
                .ok()?;
            Some(document)
        };
        let mut paths = Vec::new();

        if let Some(mut document) = clone_document()
            && document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::DeleteAssemblyMate {
                        id: contract.axial_mate_id,
                    },
                    CanonicalCommand::SetOccurrenceGrounded {
                        id: contract.second_shared_occurrence_id,
                        grounded: false,
                    },
                ]))
                .is_ok()
            && solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default())
                .is_ok_and(|solve| solve.status() == AssemblySolveStatus::UnderConstrained)
        {
            paths.push("under-constrained");
        }

        if let Some(mut document) = clone_document() {
            let duplicate = AssemblyMate::new(
                AssemblyMateId(90_001),
                axial.endpoint_a().clone(),
                axial.endpoint_b().clone(),
                axial.kind(),
            );
            if document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateAssemblyMate(duplicate),
                ]))
                .is_ok()
                && solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default())
                    .is_ok_and(|solve| !solve.redundant_mate_ids().is_empty())
            {
                paths.push("redundant");
            }
        }

        if let Some(mut document) = clone_document()
            && let AssemblyMateKind::CoincidentPlanar { offset_mm, .. } = planar.kind()
        {
            let conflicting_a = AssemblyMate::new(
                AssemblyMateId(90_002),
                planar.endpoint_a().clone(),
                planar.endpoint_b().clone(),
                AssemblyMateKind::Distance {
                    distance_mm: offset_mm,
                },
            );
            let conflicting_b = AssemblyMate::new(
                AssemblyMateId(90_003),
                planar.endpoint_a().clone(),
                planar.endpoint_b().clone(),
                AssemblyMateKind::Distance {
                    distance_mm: offset_mm + 5.0,
                },
            );
            if document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::DeleteAssemblyMate {
                        id: contract.planar_mate_id,
                    },
                    CanonicalCommand::DeleteAssemblyMate {
                        id: contract.axial_mate_id,
                    },
                    CanonicalCommand::CreateAssemblyMate(conflicting_a),
                    CanonicalCommand::CreateAssemblyMate(conflicting_b),
                ]))
                .is_ok()
                && solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default())
                    .is_ok_and(|solve| {
                        solve.status() == AssemblySolveStatus::OverConstrained
                            && !solve.conflicting_mate_ids().is_empty()
                            && solve.publication_batch(&document.current()).is_err()
                    })
            {
                paths.push("conflicting-over-constrained");
            }
        }

        if let Some(mut document) = clone_document()
            && let Ok(proposal) = document.prepare_proposal(CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceGrounded {
                    id: contract.second_shared_occurrence_id,
                    grounded: false,
                },
            ]))
            && document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetOccurrenceGrounded {
                        id: contract.second_shared_occurrence_id,
                        grounded: false,
                    },
                ]))
                .is_ok()
        {
            let before = (
                document.current().revision_id(),
                document.current().canonical_digest(),
                document.visible_undo_steps(),
                document.visible_redo_steps(),
            );
            if document.commit_verified_proposal(&proposal).is_err()
                && before
                    == (
                        document.current().revision_id(),
                        document.current().canonical_digest(),
                        document.visible_undo_steps(),
                        document.visible_redo_steps(),
                    )
            {
                paths.push("stale-confirmation");
            }
        }

        for (name, endpoint) in [
            (
                "ambiguous",
                AssemblyMateEndpoint::ambiguous(
                    planar.endpoint_a().occurrence_id(),
                    planar.endpoint_a().reference().clone(),
                    2,
                ),
            ),
            (
                "lost",
                AssemblyMateEndpoint::lost(
                    planar.endpoint_a().occurrence_id(),
                    planar.endpoint_a().reference().clone(),
                ),
            ),
        ] {
            if let Some(mut document) = clone_document() {
                let unresolved = AssemblyMate::new(
                    planar.id(),
                    endpoint,
                    planar.endpoint_b().clone(),
                    planar.kind(),
                );
                if let Ok(proposal) = document.prepare_proposal(CommandBatch::new(vec![
                    CanonicalCommand::RebindAssemblyMate(unresolved),
                ])) && document.commit_verified_proposal(&proposal).is_ok()
                    && solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default())
                        .is_err()
                {
                    paths.push(name);
                }
            }
        }

        if let Some(document) = clone_document() {
            let mut unsupported_reference = planar.endpoint_a().reference().clone();
            unsupported_reference.expected_type = "unsupported".to_owned();
            let unsupported = AssemblyMate::new(
                planar.id(),
                AssemblyMateEndpoint::resolved(
                    planar.endpoint_a().occurrence_id(),
                    unsupported_reference,
                ),
                planar.endpoint_b().clone(),
                planar.kind(),
            );
            if document
                .prepare_proposal(CommandBatch::new(vec![
                    CanonicalCommand::RebindAssemblyMate(unsupported),
                ]))
                .is_err()
            {
                paths.push("unsupported");
            }
        }

        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground_preview(app: &mut KetchupApp) -> AssemblyProposalPreview {
        assert!(
            app.prepare_assembly_preview(AssemblyPreviewSource::GroundOccurrence {
                id: OccurrenceId(1),
                grounded: true,
            },)
        );
        app.assembly_editor
            .preview
            .take()
            .expect("ground preview is prepared")
    }

    fn document_state(app: &KetchupApp) -> (u64, String, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
        )
    }

    #[test]
    fn exact_preview_plan_rejects_source_proposal_solve_metadata_tamper_stale_and_replay_atomically()
     {
        let mut valid = KetchupApp::new();
        let initial = document_state(&valid);
        let preview = ground_preview(&mut valid);
        valid.assembly_editor.preview = Some(preview.clone());
        assert!(valid.confirm_assembly_preview());
        assert_eq!(valid.document_revision(), initial.0 + 1);
        assert_eq!(valid.undo_step_count(), initial.2 + 1);
        let committed = document_state(&valid);

        valid.assembly_editor.preview = Some(preview);
        assert!(!valid.confirm_assembly_preview());
        assert_eq!(document_state(&valid), committed);
        assert!(valid.undo());
        assert_eq!(valid.canonical_digest(), initial.1);
        assert!(valid.redo());
        assert_eq!(document_state(&valid), committed);

        let mut tampered = KetchupApp::new();
        let mut tampered_preview = ground_preview(&mut tampered);
        tampered_preview.plan.solve_required = true;
        let before_tamper = document_state(&tampered);
        tampered.assembly_editor.preview = Some(tampered_preview);
        assert!(!tampered.confirm_assembly_preview());
        assert_eq!(document_state(&tampered), before_tamper);

        let mut proposal_tampered = KetchupApp::new();
        let mut proposal_tampered_preview = ground_preview(&mut proposal_tampered);
        proposal_tampered_preview.plan.proposal = proposal_tampered
            .document
            .prepare_proposal_with_context(
                CommandBatch::new(vec![CanonicalCommand::SetOccurrenceVisibility {
                    id: OccurrenceId(1),
                    visible: false,
                }]),
                ProposalContext::canonical_preview(),
            )
            .expect("same-revision malicious proposal is independently valid");
        let before_proposal_tamper = document_state(&proposal_tampered);
        proposal_tampered.assembly_editor.preview = Some(proposal_tampered_preview);
        assert!(!proposal_tampered.confirm_assembly_preview());
        assert_eq!(document_state(&proposal_tampered), before_proposal_tamper);

        let mut source_tampered = KetchupApp::new();
        let mut source_tampered_preview = ground_preview(&mut source_tampered);
        source_tampered_preview.plan.source = AssemblyPreviewSource::GroundOccurrence {
            id: OccurrenceId(1),
            grounded: false,
        };
        let before_source_tamper = document_state(&source_tampered);
        source_tampered.assembly_editor.preview = Some(source_tampered_preview);
        assert!(!source_tampered.confirm_assembly_preview());
        assert_eq!(document_state(&source_tampered), before_source_tamper);

        let mut metadata_tampered = KetchupApp::new();
        let mut metadata_tampered_preview = ground_preview(&mut metadata_tampered);
        metadata_tampered_preview.action =
            metadata_tampered.catalog.text("assembly-action-unground");
        let before_metadata_tamper = document_state(&metadata_tampered);
        metadata_tampered.assembly_editor.preview = Some(metadata_tampered_preview);
        assert!(!metadata_tampered.confirm_assembly_preview());
        assert_eq!(document_state(&metadata_tampered), before_metadata_tamper);

        let mut stale = KetchupApp::new();
        let stale_preview = ground_preview(&mut stale);
        stale
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceVisibility {
                    id: OccurrenceId(1),
                    visible: false,
                },
            ]))
            .expect("intervening visibility edit is valid");
        let after_drift = document_state(&stale);
        stale.assembly_editor.preview = Some(stale_preview);
        assert!(!stale.confirm_assembly_preview());
        assert_eq!(document_state(&stale), after_drift);
    }
}
