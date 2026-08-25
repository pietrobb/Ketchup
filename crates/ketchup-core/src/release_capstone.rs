#![forbid(unsafe_code)]

use crate::assembly::AssemblyMateId;
use crate::document::{BodyId, DefinitionId, DocumentId, FeatureId, OccurrenceId};
use crate::drawing::DrawingSheetId;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const RELEASE_CAPSTONE_CONTRACT_SCHEMA_V1: &str = "ketchup.release-capstone-contract.v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CapstoneCapability {
    PrincipalWorkplane,
    ConstrainedSketch,
    Pad,
    Pocket,
    SharedDefinitionEdit,
    MakeUnique,
    RigidOccurrencePlacement,
    PlanarMate,
    AxialMate,
    FrontTopRightDrawing,
    CompatibleComponentReplacement,
    ExactRenderPick,
    StepExport,
    StlExport,
    UndoRedo,
    SaveOpen,
    ManualAiParity,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CapstoneRefusalPath {
    Missing,
    Hidden,
    Stale,
    Failed,
    Ambiguous,
    Lost,
    Cyclic,
    CrossDocument,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CapstoneObservationalPath {
    Inspect,
    Preview,
    Cancel,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CapstoneStage {
    Blank,
    PartsAuthored,
    AssemblySolved,
    SharedDefinitionEdited,
    OccurrenceMadeUnique,
    ComponentReplaced,
    Reopened,
}

impl CapstoneStage {
    pub const ALL: [Self; 7] = [
        Self::Blank,
        Self::PartsAuthored,
        Self::AssemblySolved,
        Self::SharedDefinitionEdited,
        Self::OccurrenceMadeUnique,
        Self::ComponentReplaced,
        Self::Reopened,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CapstoneOutputKind {
    ExactBrep,
    RenderMesh,
    PickMap,
    FrontDrawing,
    TopDrawing,
    RightDrawing,
    Step,
    Stl,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapstoneLineage {
    pub definition_ids: Vec<DefinitionId>,
    pub body_ids: Vec<BodyId>,
    pub feature_ids: Vec<FeatureId>,
    pub occurrence_ids: Vec<OccurrenceId>,
}

impl CapstoneLineage {
    fn empty() -> Self {
        Self {
            definition_ids: Vec::new(),
            body_ids: Vec::new(),
            feature_ids: Vec::new(),
            occurrence_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapstoneStageRequirement {
    pub stage: CapstoneStage,
    pub lineage: CapstoneLineage,
    pub required_outputs: Vec<CapstoneOutputKind>,
    pub must_advance_revision: bool,
    pub must_match_prior_canonical_state: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapstoneStageEvidence {
    pub stage: CapstoneStage,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub lineage: CapstoneLineage,
    pub output_fingerprints: Vec<(CapstoneOutputKind, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapstoneFixtureDimensions {
    pub plate_length_mm: u32,
    pub plate_width_mm: u32,
    pub plate_height_mm: u32,
    pub plate_pocket_diameter_mm: u32,
    pub fastener_diameter_mm: u32,
    pub fastener_height_mm: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapstonePlateFeatureIds {
    pub principal_workplane: FeatureId,
    pub base_sketch: FeatureId,
    pub pad: FeatureId,
    pub face_workplane: FeatureId,
    pub pocket_sketch: FeatureId,
    pub pocket: FeatureId,
}

impl CapstonePlateFeatureIds {
    #[must_use]
    pub const fn all(self) -> [FeatureId; 6] {
        [
            self.principal_workplane,
            self.base_sketch,
            self.pad,
            self.face_workplane,
            self.pocket_sketch,
            self.pocket,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapstoneFastenerFeatureIds {
    pub principal_workplane: FeatureId,
    pub sketch: FeatureId,
    pub pad: FeatureId,
}

impl CapstoneFastenerFeatureIds {
    #[must_use]
    pub const fn all(self) -> [FeatureId; 3] {
        [self.principal_workplane, self.sketch, self.pad]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseCapstoneContract {
    pub schema: &'static str,
    pub fixture_name: &'static str,
    pub dimensions: CapstoneFixtureDimensions,
    pub plate_definition_id: DefinitionId,
    pub shared_definition_id: DefinitionId,
    pub replacement_definition_id: DefinitionId,
    pub unique_definition_id: DefinitionId,
    pub plate_body_id: BodyId,
    pub shared_body_id: BodyId,
    pub replacement_body_id: BodyId,
    pub unique_body_id: BodyId,
    pub plate_feature_ids: CapstonePlateFeatureIds,
    pub shared_feature_ids: CapstoneFastenerFeatureIds,
    pub replacement_feature_ids: CapstoneFastenerFeatureIds,
    pub unique_feature_ids: CapstoneFastenerFeatureIds,
    pub plate_occurrence_id: OccurrenceId,
    pub first_shared_occurrence_id: OccurrenceId,
    pub second_shared_occurrence_id: OccurrenceId,
    pub drawing_sheet_id: DrawingSheetId,
    pub planar_mate_id: AssemblyMateId,
    pub axial_mate_id: AssemblyMateId,
    pub required_capabilities: Vec<CapstoneCapability>,
    pub observational_paths: Vec<CapstoneObservationalPath>,
    pub refusal_paths: Vec<CapstoneRefusalPath>,
    pub non_commit_paths_preserve_canonical_history: bool,
    pub non_commit_paths_preserve_last_valid_outputs: bool,
    pub stages: Vec<CapstoneStageRequirement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseCapstoneContractError {
    InvalidFixture,
    InvalidStageOrder,
    InvalidStageLineage(CapstoneStage),
    InvalidOutputContract(CapstoneStage),
    IncompleteEvidence,
    CrossDocument,
    NonMonotonicRevision(CapstoneStage),
    CanonicalReplayMismatch,
    OutputReplayMismatch,
    MissingFingerprint(CapstoneStage, CapstoneOutputKind),
}

impl fmt::Display for ReleaseCapstoneContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ReleaseCapstoneContractError {}

impl ReleaseCapstoneContract {
    #[must_use]
    pub fn mechanical_plate_fixture() -> Self {
        let plate_features = CapstonePlateFeatureIds {
            principal_workplane: FeatureId(110),
            base_sketch: FeatureId(111),
            pad: FeatureId(112),
            face_workplane: FeatureId(113),
            pocket_sketch: FeatureId(114),
            pocket: FeatureId(115),
        };
        let shared_features = CapstoneFastenerFeatureIds {
            principal_workplane: FeatureId(210),
            sketch: FeatureId(211),
            pad: FeatureId(212),
        };
        let replacement_features = CapstoneFastenerFeatureIds {
            principal_workplane: FeatureId(310),
            sketch: FeatureId(311),
            pad: FeatureId(312),
        };
        let unique_features = CapstoneFastenerFeatureIds {
            principal_workplane: FeatureId(313),
            sketch: FeatureId(314),
            pad: FeatureId(315),
        };
        let base_lineage = CapstoneLineage {
            definition_ids: vec![DefinitionId(100), DefinitionId(200), DefinitionId(300)],
            body_ids: vec![BodyId(101), BodyId(201), BodyId(301)],
            feature_ids: plate_features
                .all()
                .into_iter()
                .chain(shared_features.all())
                .chain(replacement_features.all())
                .collect(),
            occurrence_ids: vec![OccurrenceId(1000), OccurrenceId(1001), OccurrenceId(1002)],
        };
        let mut unique_lineage = base_lineage.clone();
        unique_lineage.definition_ids.push(DefinitionId(301));
        unique_lineage.feature_ids.extend(unique_features.all());
        let part_outputs = vec![
            CapstoneOutputKind::ExactBrep,
            CapstoneOutputKind::RenderMesh,
            CapstoneOutputKind::PickMap,
        ];
        let complete_outputs = vec![
            CapstoneOutputKind::ExactBrep,
            CapstoneOutputKind::RenderMesh,
            CapstoneOutputKind::PickMap,
            CapstoneOutputKind::FrontDrawing,
            CapstoneOutputKind::TopDrawing,
            CapstoneOutputKind::RightDrawing,
            CapstoneOutputKind::Step,
            CapstoneOutputKind::Stl,
        ];
        Self {
            schema: RELEASE_CAPSTONE_CONTRACT_SCHEMA_V1,
            fixture_name: "bounded-mechanical-plate-and-fasteners",
            dimensions: CapstoneFixtureDimensions {
                plate_length_mm: 120,
                plate_width_mm: 80,
                plate_height_mm: 8,
                plate_pocket_diameter_mm: 20,
                fastener_diameter_mm: 12,
                fastener_height_mm: 30,
            },
            plate_definition_id: DefinitionId(100),
            shared_definition_id: DefinitionId(200),
            replacement_definition_id: DefinitionId(300),
            unique_definition_id: DefinitionId(301),
            plate_body_id: BodyId(101),
            shared_body_id: BodyId(201),
            replacement_body_id: BodyId(301),
            unique_body_id: BodyId(201),
            plate_feature_ids: plate_features,
            shared_feature_ids: shared_features,
            replacement_feature_ids: replacement_features,
            unique_feature_ids: unique_features,
            plate_occurrence_id: OccurrenceId(1000),
            first_shared_occurrence_id: OccurrenceId(1001),
            second_shared_occurrence_id: OccurrenceId(1002),
            drawing_sheet_id: DrawingSheetId(5000),
            planar_mate_id: AssemblyMateId(6000),
            axial_mate_id: AssemblyMateId(6001),
            required_capabilities: vec![
                CapstoneCapability::PrincipalWorkplane,
                CapstoneCapability::ConstrainedSketch,
                CapstoneCapability::Pad,
                CapstoneCapability::Pocket,
                CapstoneCapability::SharedDefinitionEdit,
                CapstoneCapability::MakeUnique,
                CapstoneCapability::RigidOccurrencePlacement,
                CapstoneCapability::PlanarMate,
                CapstoneCapability::AxialMate,
                CapstoneCapability::FrontTopRightDrawing,
                CapstoneCapability::CompatibleComponentReplacement,
                CapstoneCapability::ExactRenderPick,
                CapstoneCapability::StepExport,
                CapstoneCapability::StlExport,
                CapstoneCapability::UndoRedo,
                CapstoneCapability::SaveOpen,
                CapstoneCapability::ManualAiParity,
            ],
            observational_paths: vec![
                CapstoneObservationalPath::Inspect,
                CapstoneObservationalPath::Preview,
                CapstoneObservationalPath::Cancel,
                CapstoneObservationalPath::Escape,
            ],
            refusal_paths: vec![
                CapstoneRefusalPath::Missing,
                CapstoneRefusalPath::Hidden,
                CapstoneRefusalPath::Stale,
                CapstoneRefusalPath::Failed,
                CapstoneRefusalPath::Ambiguous,
                CapstoneRefusalPath::Lost,
                CapstoneRefusalPath::Cyclic,
                CapstoneRefusalPath::CrossDocument,
                CapstoneRefusalPath::Unsupported,
            ],
            non_commit_paths_preserve_canonical_history: true,
            non_commit_paths_preserve_last_valid_outputs: true,
            stages: vec![
                CapstoneStageRequirement {
                    stage: CapstoneStage::Blank,
                    lineage: CapstoneLineage::empty(),
                    required_outputs: Vec::new(),
                    must_advance_revision: false,
                    must_match_prior_canonical_state: false,
                },
                CapstoneStageRequirement {
                    stage: CapstoneStage::PartsAuthored,
                    lineage: base_lineage.clone(),
                    required_outputs: part_outputs,
                    must_advance_revision: true,
                    must_match_prior_canonical_state: false,
                },
                CapstoneStageRequirement {
                    stage: CapstoneStage::AssemblySolved,
                    lineage: base_lineage.clone(),
                    required_outputs: complete_outputs.clone(),
                    must_advance_revision: true,
                    must_match_prior_canonical_state: false,
                },
                CapstoneStageRequirement {
                    stage: CapstoneStage::SharedDefinitionEdited,
                    lineage: base_lineage,
                    required_outputs: complete_outputs.clone(),
                    must_advance_revision: true,
                    must_match_prior_canonical_state: false,
                },
                CapstoneStageRequirement {
                    stage: CapstoneStage::OccurrenceMadeUnique,
                    lineage: unique_lineage.clone(),
                    required_outputs: complete_outputs.clone(),
                    must_advance_revision: true,
                    must_match_prior_canonical_state: false,
                },
                CapstoneStageRequirement {
                    stage: CapstoneStage::ComponentReplaced,
                    lineage: unique_lineage.clone(),
                    required_outputs: complete_outputs.clone(),
                    must_advance_revision: true,
                    must_match_prior_canonical_state: false,
                },
                CapstoneStageRequirement {
                    stage: CapstoneStage::Reopened,
                    lineage: unique_lineage,
                    required_outputs: complete_outputs,
                    must_advance_revision: false,
                    must_match_prior_canonical_state: true,
                },
            ],
        }
    }

    pub fn validate(&self) -> Result<(), ReleaseCapstoneContractError> {
        if self.schema != RELEASE_CAPSTONE_CONTRACT_SCHEMA_V1
            || self.fixture_name.trim().is_empty()
            || self.dimensions.plate_length_mm == 0
            || self.dimensions.plate_width_mm == 0
            || self.dimensions.plate_height_mm == 0
            || self.dimensions.plate_pocket_diameter_mm == 0
            || self.dimensions.fastener_diameter_mm == 0
            || self.dimensions.fastener_height_mm == 0
            || self
                .required_capabilities
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.required_capabilities.len()
            || self
                .observational_paths
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.observational_paths.len()
            || self
                .refusal_paths
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.refusal_paths.len()
            || !self.non_commit_paths_preserve_canonical_history
            || !self.non_commit_paths_preserve_last_valid_outputs
        {
            return Err(ReleaseCapstoneContractError::InvalidFixture);
        }
        let definition_ids = [
            self.plate_definition_id,
            self.shared_definition_id,
            self.replacement_definition_id,
            self.unique_definition_id,
        ];
        let body_ids = [
            self.plate_body_id,
            self.shared_body_id,
            self.replacement_body_id,
            self.unique_body_id,
        ];
        let feature_ids = self
            .plate_feature_ids
            .all()
            .into_iter()
            .chain(self.shared_feature_ids.all())
            .chain(self.replacement_feature_ids.all())
            .chain(self.unique_feature_ids.all())
            .collect::<Vec<_>>();
        let occurrence_ids = [
            self.plate_occurrence_id,
            self.first_shared_occurrence_id,
            self.second_shared_occurrence_id,
        ];
        if definition_ids.iter().any(|id| id.0 == 0)
            || body_ids.iter().any(|id| id.0 == 0)
            || feature_ids.iter().any(|id| id.0 == 0)
            || occurrence_ids.iter().any(|id| id.0 == 0)
            || definition_ids.into_iter().collect::<BTreeSet<_>>().len() != definition_ids.len()
            || feature_ids.iter().copied().collect::<BTreeSet<_>>().len() != feature_ids.len()
            || occurrence_ids.into_iter().collect::<BTreeSet<_>>().len() != occurrence_ids.len()
            || self.drawing_sheet_id.0 == 0
            || self.planar_mate_id.0 == 0
            || self.axial_mate_id.0 == 0
            || self.planar_mate_id == self.axial_mate_id
        {
            return Err(ReleaseCapstoneContractError::InvalidFixture);
        }
        if self.stages.len() != CapstoneStage::ALL.len()
            || self
                .stages
                .iter()
                .map(|requirement| requirement.stage)
                .ne(CapstoneStage::ALL)
        {
            return Err(ReleaseCapstoneContractError::InvalidStageOrder);
        }
        for requirement in &self.stages {
            if !has_unique_items(&requirement.lineage.definition_ids)
                || !has_unique_items(&requirement.lineage.body_ids)
                || !has_unique_items(&requirement.lineage.feature_ids)
                || !has_unique_items(&requirement.lineage.occurrence_ids)
            {
                return Err(ReleaseCapstoneContractError::InvalidStageLineage(
                    requirement.stage,
                ));
            }
            if requirement
                .required_outputs
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != requirement.required_outputs.len()
            {
                return Err(ReleaseCapstoneContractError::InvalidOutputContract(
                    requirement.stage,
                ));
            }
        }
        Ok(())
    }

    pub fn validate_evidence(
        &self,
        evidence: &[CapstoneStageEvidence],
    ) -> Result<(), ReleaseCapstoneContractError> {
        self.validate()?;
        if evidence.len() != self.stages.len() {
            return Err(ReleaseCapstoneContractError::IncompleteEvidence);
        }
        let document_id = evidence[0].document_id;
        if document_id.0 == 0
            || evidence
                .iter()
                .any(|stage| stage.document_id != document_id)
        {
            return Err(ReleaseCapstoneContractError::CrossDocument);
        }
        for (index, (requirement, actual)) in self.stages.iter().zip(evidence).enumerate() {
            if actual.stage != requirement.stage {
                return Err(ReleaseCapstoneContractError::InvalidStageOrder);
            }
            if actual.source_digest.is_empty()
                || !same_unique_items(
                    &actual.lineage.definition_ids,
                    &requirement.lineage.definition_ids,
                )
                || !same_unique_items(&actual.lineage.body_ids, &requirement.lineage.body_ids)
                || !same_unique_items(
                    &actual.lineage.feature_ids,
                    &requirement.lineage.feature_ids,
                )
                || !same_unique_items(
                    &actual.lineage.occurrence_ids,
                    &requirement.lineage.occurrence_ids,
                )
            {
                return Err(ReleaseCapstoneContractError::InvalidStageLineage(
                    actual.stage,
                ));
            }
            if index > 0 {
                let previous = &evidence[index - 1];
                if requirement.must_advance_revision
                    && actual.source_revision <= previous.source_revision
                {
                    return Err(ReleaseCapstoneContractError::NonMonotonicRevision(
                        actual.stage,
                    ));
                }
                if requirement.must_match_prior_canonical_state
                    && (actual.source_revision != previous.source_revision
                        || actual.source_digest != previous.source_digest)
                {
                    return Err(ReleaseCapstoneContractError::CanonicalReplayMismatch);
                }
            }
            let actual_outputs = actual
                .output_fingerprints
                .iter()
                .map(|(kind, _)| *kind)
                .collect::<Vec<_>>();
            if !same_unique_items(&actual_outputs, &requirement.required_outputs) {
                return Err(ReleaseCapstoneContractError::InvalidOutputContract(
                    actual.stage,
                ));
            }
            for (kind, fingerprint) in &actual.output_fingerprints {
                if fingerprint.trim().is_empty() {
                    return Err(ReleaseCapstoneContractError::MissingFingerprint(
                        actual.stage,
                        *kind,
                    ));
                }
            }
            if requirement.must_match_prior_canonical_state
                && !same_unique_items(
                    &actual.output_fingerprints,
                    &evidence[index - 1].output_fingerprints,
                )
            {
                return Err(ReleaseCapstoneContractError::OutputReplayMismatch);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn contract_fingerprint(&self) -> String {
        let mut canonical = self.clone();
        canonical.required_capabilities.sort_unstable();
        canonical.observational_paths.sort_unstable();
        canonical.refusal_paths.sort_unstable();
        for requirement in &mut canonical.stages {
            requirement.lineage.definition_ids.sort_unstable();
            requirement.lineage.body_ids.sort_unstable();
            requirement.lineage.feature_ids.sort_unstable();
            requirement.lineage.occurrence_ids.sort_unstable();
            requirement.required_outputs.sort_unstable();
        }
        let mut hasher = Sha256::new();
        hasher.update(format!("{canonical:?}"));
        format!("{:x}", hasher.finalize())
    }
}

fn has_unique_items<T: Clone + Ord>(items: &[T]) -> bool {
    items.iter().cloned().collect::<BTreeSet<_>>().len() == items.len()
}

fn same_unique_items<T: Clone + Ord>(actual: &[T], expected: &[T]) -> bool {
    actual.len() == expected.len()
        && actual.iter().cloned().collect::<BTreeSet<_>>().len() == actual.len()
        && actual.iter().cloned().collect::<BTreeSet<_>>()
            == expected.iter().cloned().collect::<BTreeSet<_>>()
}
