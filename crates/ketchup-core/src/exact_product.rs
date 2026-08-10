#![forbid(unsafe_code)]

use crate::beam_m5::{BeamExactPiecePackage, BeamExactResultKey};
use crate::document::{
    BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, DocumentId,
    ExactReferenceConversionConsequence, ExactToMeshConversion, FeatureId, FeatureKind,
    InstancePath, MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, Snapshot, Transform,
};
use crate::graph::DerivedIdentity;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

pub const EXACT_PRODUCT_SCHEMA_V1: &str = "ketchup.exact-product.v1";
pub const EXACT_RECTANGLE_EVALUATOR_V1: &str = "ketchup.exact-rectangle-evaluator.v1";
pub const EXACT_THROUGH_CUT_EVALUATOR_V1: &str = "ketchup.exact-through-cut-evaluator.v1";
pub const EXACT_BOOLEAN_UNION_EVALUATOR_V1: &str = "ketchup.exact-boolean-union-evaluator.v1";
pub const BODY_SUBSHAPE_REF_SCHEMA_V1: &str = "ketchup.body-subshape-ref.v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactFaceRole {
    Top,
    Bottom,
    East,
    CutWest,
    CutEast,
    CutSouth,
    CutNorth,
    RevolveBottom,
    RevolveBody,
    RevolveShoulder,
    RevolveNeck,
    RevolveMouth,
    ShellOuterBottom,
    ShellOuterBody,
    ShellOuterShoulder,
    ShellOuterNeck,
    ShellRim,
    ShellInnerBottom,
    ShellInnerBody,
    ShellInnerShoulder,
    ShellInnerNeck,
}

const EXTRUSION_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
];
const THROUGH_CUT_FACE_ROLES: [ExactFaceRole; 7] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutWest,
    ExactFaceRole::CutEast,
    ExactFaceRole::CutSouth,
    ExactFaceRole::CutNorth,
];

impl ExactFaceRole {
    #[must_use]
    pub const fn semantic_role(self) -> &'static str {
        match self {
            Self::Top => "extrusion.top",
            Self::Bottom => "extrusion.bottom",
            Self::East => "extrusion.side(profile_edge=east)",
            Self::CutWest => "through_cut.wall.west",
            Self::CutEast => "through_cut.wall.east",
            Self::CutSouth => "through_cut.wall.south",
            Self::CutNorth => "through_cut.wall.north",
            Self::RevolveBottom => "revolve.bottom",
            Self::RevolveBody => "revolve.body",
            Self::RevolveShoulder => "revolve.shoulder",
            Self::RevolveNeck => "revolve.neck",
            Self::RevolveMouth => "revolve.mouth",
            Self::ShellOuterBottom => "shell.outer.bottom",
            Self::ShellOuterBody => "shell.outer.body",
            Self::ShellOuterShoulder => "shell.outer.shoulder",
            Self::ShellOuterNeck => "shell.outer.neck",
            Self::ShellRim => "shell.rim",
            Self::ShellInnerBottom => "shell.inner.bottom",
            Self::ShellInnerBody => "shell.inner.body",
            Self::ShellInnerShoulder => "shell.inner.shoulder",
            Self::ShellInnerNeck => "shell.inner.neck",
        }
    }

    #[must_use]
    pub const fn source_element_id(self) -> &'static str {
        match self {
            Self::Top | Self::Bottom => "profile.face",
            Self::East => "profile.edge.east",
            Self::CutWest => "cut_profile.edge.west",
            Self::CutEast => "cut_profile.edge.east",
            Self::CutSouth => "cut_profile.edge.south",
            Self::CutNorth => "cut_profile.edge.north",
            Self::RevolveBottom => "profile.edge.0",
            Self::RevolveBody => "profile.edge.1",
            Self::RevolveShoulder => "profile.edge.2",
            Self::RevolveNeck => "profile.edge.3",
            Self::RevolveMouth => "profile.edge.4",
            Self::ShellOuterBottom => "revolve.face.bottom",
            Self::ShellOuterBody => "revolve.face.body",
            Self::ShellOuterShoulder => "revolve.face.shoulder",
            Self::ShellOuterNeck => "revolve.face.neck",
            Self::ShellRim => "revolve.face.mouth",
            Self::ShellInnerBottom => "shell.offset.bottom",
            Self::ShellInnerBody => "shell.offset.body",
            Self::ShellInnerShoulder => "shell.offset.shoulder",
            Self::ShellInnerNeck => "shell.offset.neck",
        }
    }

    #[must_use]
    pub const fn expected_type(self) -> &'static str {
        match self {
            Self::Top
            | Self::Bottom
            | Self::East
            | Self::CutWest
            | Self::CutEast
            | Self::CutSouth
            | Self::CutNorth => "planar_face",
            Self::RevolveBottom
            | Self::RevolveBody
            | Self::RevolveShoulder
            | Self::RevolveNeck
            | Self::RevolveMouth
            | Self::ShellOuterBottom
            | Self::ShellOuterBody
            | Self::ShellOuterShoulder
            | Self::ShellOuterNeck
            | Self::ShellRim
            | Self::ShellInnerBottom
            | Self::ShellInnerBody
            | Self::ShellInnerShoulder
            | Self::ShellInnerNeck => "face",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceStability {
    Guaranteed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodySubshapeRef {
    pub schema: String,
    pub document_id: DocumentId,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub semantic_role: String,
    pub source_element_id: String,
    pub expected_type: String,
    pub expected_cardinality: u32,
    pub stability: ReferenceStability,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

impl BodySubshapeRef {
    #[must_use]
    pub fn role(&self) -> Option<ExactFaceRole> {
        [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
            ExactFaceRole::RevolveBottom,
            ExactFaceRole::RevolveBody,
            ExactFaceRole::RevolveShoulder,
            ExactFaceRole::RevolveNeck,
            ExactFaceRole::RevolveMouth,
            ExactFaceRole::ShellOuterBottom,
            ExactFaceRole::ShellOuterBody,
            ExactFaceRole::ShellOuterShoulder,
            ExactFaceRole::ShellOuterNeck,
            ExactFaceRole::ShellRim,
            ExactFaceRole::ShellInnerBottom,
            ExactFaceRole::ShellInnerBody,
            ExactFaceRole::ShellInnerShoulder,
            ExactFaceRole::ShellInnerNeck,
        ]
        .into_iter()
        .find(|role| {
            self.semantic_role == role.semantic_role()
                && self.source_element_id == role.source_element_id()
        })
    }

    #[must_use]
    pub fn has_valid_lineage(&self) -> bool {
        self.schema == BODY_SUBSHAPE_REF_SCHEMA_V1
            && self.expected_cardinality == 1
            && self
                .role()
                .is_some_and(|role| self.expected_type == role.expected_type())
            && self.lineage_digest == reference_lineage_digest(self)
    }

    #[must_use]
    pub fn matches_request(&self, request: &ExactFeatureChainRequest) -> bool {
        let Some(role) = self.role() else {
            return false;
        };
        self.has_valid_lineage()
            && self.document_id == request.document_id
            && self.definition_id == request.definition_id
            && request.profile_feature_id_for_role(role) == Some(self.profile_feature_id)
            && self.producer_feature_id == request.producer_feature_id()
            && self.canonical_input_digest == request.canonical_input_digest
            && self.evaluator == request.evaluator()
            && !self.exact_input_digest.is_empty()
            && !self.result_fingerprint.is_empty()
            && !self.backend.is_empty()
            && !self.tolerance.is_empty()
            && !self.corroborating_geometry_fingerprint.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactReferenceQuarantineReason {
    InvalidLineage,
    WrongDocument,
    IncompatibleEvaluationEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactReferenceResolution {
    Resolved {
        reference: Box<BodySubshapeRef>,
    },
    Ambiguous {
        candidate_count: usize,
    },
    Lost,
    Quarantined {
        reason: ExactReferenceQuarantineReason,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BodyResultIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub extrusion_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactVertex {
    pub position_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactTriangle {
    pub vertex_indices: [u32; 3],
    pub face_role: Option<ExactFaceRole>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactRenderPackage {
    pub identity: BodyResultIdentity,
    pub bounds_mm: [[f64; 3]; 2],
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub references: Vec<BodySubshapeRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExactBodyPackage {
    Rectangle(ExactRenderPackage),
    Revolve(crate::bottle_m6::ExactRevolvePackage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactMeshExport {
    pub mesh_obj: String,
    pub loss_report: String,
}

pub trait ExactBodyView {
    fn bounds_mm(&self) -> [[f64; 3]; 2];
    fn vertex_count(&self) -> usize;
    fn vertex_position_mm(&self, index: usize) -> [f64; 3];
    fn triangle_count(&self) -> usize;
    fn triangle_indices(&self, index: usize) -> [u32; 3];
    fn triangle_group(&self, index: usize) -> &'static str;
    fn tolerance(&self) -> &str;
    fn source_digest(&self) -> &str;
    fn producer_identity(&self) -> String;
    fn result_fingerprint(&self) -> &str;

    #[must_use]
    fn mesh_export(&self, transform: Transform) -> ExactMeshExport {
        mesh_export_from_view(self, transform)
    }
}

impl ExactBodyPackage {
    #[must_use]
    pub fn definition_id(&self) -> DefinitionId {
        match self {
            Self::Rectangle(package) => package.identity.definition_id,
            Self::Revolve(package) => package.identity.definition_id,
        }
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        match self {
            Self::Rectangle(package) => package.identity.producer_feature_id,
            Self::Revolve(package) => package.identity.producer_feature_id,
        }
    }

    #[must_use]
    pub fn result_key(&self) -> ExactResultKey {
        match self {
            Self::Rectangle(package) => ExactResultKey {
                document_id: package.identity.document_id,
                source_revision: package.identity.source_revision,
                source_digest: package.identity.source_digest.clone(),
                definition_id: package.identity.definition_id,
                producer_feature_id: package.identity.producer_feature_id,
                canonical_input_digest: package.identity.canonical_input_digest.clone(),
                exact_input_digest: package.identity.exact_input_digest.clone(),
                evaluator: package.identity.evaluator.clone(),
                backend: package.identity.backend.clone(),
                tolerance: package.identity.tolerance.clone(),
                schema: package.identity.schema.clone(),
                result_fingerprint: package.identity.result_fingerprint.clone(),
            },
            Self::Revolve(package) => ExactResultKey {
                document_id: package.identity.document_id,
                source_revision: package.identity.source_revision,
                source_digest: package.identity.source_digest.clone(),
                definition_id: package.identity.definition_id,
                producer_feature_id: package.identity.producer_feature_id,
                canonical_input_digest: package.identity.canonical_input_digest.clone(),
                exact_input_digest: package.identity.exact_input_digest.clone(),
                evaluator: package.identity.evaluator.clone(),
                backend: package.identity.backend.clone(),
                tolerance: package.identity.tolerance.clone(),
                schema: package.identity.schema.clone(),
                result_fingerprint: package.identity.result_fingerprint.clone(),
            },
        }
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        match self {
            Self::Rectangle(package) => package.is_current(snapshot),
            Self::Revolve(package) => package.is_current(snapshot),
        }
    }

    #[must_use]
    pub fn bounds_mm(&self) -> [[f64; 3]; 2] {
        match self {
            Self::Rectangle(package) => package.bounds_mm,
            Self::Revolve(package) => package.bounds_mm,
        }
    }

    #[must_use]
    pub fn vertices(&self) -> &[ExactVertex] {
        match self {
            Self::Rectangle(package) => &package.vertices,
            Self::Revolve(package) => &package.vertices,
        }
    }

    #[must_use]
    pub fn triangles(&self) -> &[ExactTriangle] {
        match self {
            Self::Rectangle(package) => &package.triangles,
            Self::Revolve(package) => &package.triangles,
        }
    }

    #[must_use]
    pub fn references(&self) -> &[BodySubshapeRef] {
        match self {
            Self::Rectangle(package) => &package.references,
            Self::Revolve(package) => &package.references,
        }
    }

    #[must_use]
    pub fn reference(&self, role: ExactFaceRole) -> Option<&BodySubshapeRef> {
        self.references()
            .iter()
            .find(|reference| reference.role() == Some(role))
    }

    #[must_use]
    pub fn mesh_export(&self, transform: Transform) -> ExactMeshExport {
        mesh_export_from_view(self, transform)
    }

    #[must_use]
    pub fn revolve(&self) -> Option<&crate::bottle_m6::ExactRevolvePackage> {
        match self {
            Self::Revolve(package) => Some(package),
            Self::Rectangle(_) => None,
        }
    }

    pub fn detached_mesh_conversion_batch(
        &self,
        snapshot: &Snapshot,
        destination_definition_id: DefinitionId,
        destination_definition_name: impl Into<String>,
        destination_feature_id: FeatureId,
        destination_feature_name: impl Into<String>,
    ) -> Result<CommandBatch, ExactProductError> {
        if !self.is_current(snapshot) {
            return Err(ExactProductError::StaleResult);
        }
        let key = self.result_key();
        let spec = MeshBodySpec {
            schema: MESH_BODY_SCHEMA_V1.to_owned(),
            vertices_mm: self
                .vertices()
                .iter()
                .map(|vertex| vertex.position_mm)
                .collect(),
            triangles: self
                .triangles()
                .iter()
                .map(|triangle| triangle.vertex_indices)
                .collect(),
            authority: MeshAuthority::ExactConversion(ExactToMeshConversion {
                source_document_id: key.document_id,
                source_revision: key.source_revision,
                source_digest: key.source_digest,
                source_definition_id: key.definition_id,
                source_feature_id: key.producer_feature_id,
                source_result_fingerprint: key.result_fingerprint,
                source_evaluator: key.evaluator,
                source_backend: key.backend,
                source_tolerance: key.tolerance.clone(),
                tessellation_tolerance: key.tolerance,
                destination_definition_id,
                destination_feature_id,
                unsupported_semantics: vec![
                    "analytic_surfaces".to_owned(),
                    "canonical_exact_features_rules_dimensions".to_owned(),
                    "durable_exact_subshape_references".to_owned(),
                    "exact_topology".to_owned(),
                ],
                exact_reference_consequence: ExactReferenceConversionConsequence::Lost,
            }),
        };
        Ok(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: destination_definition_id,
                name: destination_definition_name.into(),
            },
            CanonicalCommand::CreateFeature {
                id: destination_feature_id,
                definition_id: destination_definition_id,
                name: destination_feature_name.into(),
                kind: FeatureKind::MeshBody(spec),
            },
        ]))
    }
}

impl ExactBodyView for ExactBodyPackage {
    fn bounds_mm(&self) -> [[f64; 3]; 2] {
        self.bounds_mm()
    }

    fn vertex_count(&self) -> usize {
        self.vertices().len()
    }

    fn vertex_position_mm(&self, index: usize) -> [f64; 3] {
        self.vertices()[index].position_mm
    }

    fn triangle_count(&self) -> usize {
        self.triangles().len()
    }

    fn triangle_indices(&self, index: usize) -> [u32; 3] {
        self.triangles()[index].vertex_indices
    }

    fn triangle_group(&self, index: usize) -> &'static str {
        self.triangles()[index]
            .face_role
            .map_or("unreferenced", ExactFaceRole::semantic_role)
    }

    fn tolerance(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.tolerance,
            Self::Revolve(package) => &package.identity.tolerance,
        }
    }

    fn source_digest(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.source_digest,
            Self::Revolve(package) => &package.identity.source_digest,
        }
    }

    fn producer_identity(&self) -> String {
        format!("producer_feature_id={}", self.producer_feature_id().0)
    }

    fn result_fingerprint(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.result_fingerprint,
            Self::Revolve(package) => &package.identity.result_fingerprint,
        }
    }
}

impl ExactBodyView for BeamExactPiecePackage {
    fn bounds_mm(&self) -> [[f64; 3]; 2] {
        [self.bounds_mm.min(), self.bounds_mm.max()]
    }

    fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    fn vertex_position_mm(&self, index: usize) -> [f64; 3] {
        self.vertices[index].position_mm
    }

    fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    fn triangle_indices(&self, index: usize) -> [u32; 3] {
        self.triangles[index].vertex_indices
    }

    fn triangle_group(&self, _index: usize) -> &'static str {
        "unreferenced"
    }

    fn tolerance(&self) -> &str {
        &self.identity.tolerance
    }

    fn source_digest(&self) -> &str {
        &self.identity.source_digest
    }

    fn producer_identity(&self) -> String {
        format!("producer_piece_key={}", self.identity.piece_key)
    }

    fn result_fingerprint(&self) -> &str {
        &self.identity.result_fingerprint
    }
}

fn mesh_export_from_view(
    view: &(impl ExactBodyView + ?Sized),
    transform: Transform,
) -> ExactMeshExport {
    let matrix = transform.matrix();
    let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
        - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
        + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
    let loss_report = format!(
        "authority=accepted exact OCCT B-Rep\nformat=Wavefront OBJ\nconversion=exact-body-to-world-space-mesh\neditability_loss=canonical features, rules, and dimensions are not preserved\ntopology_loss=exact topology, analytic surfaces, and durable face identity are not preserved\ntolerance_loss=geometry is approximated by the accepted tessellation under the source tolerance profile\nsource_tolerance={}\nsource_digest={}\n{}\nresult_fingerprint={}\n",
        view.tolerance(),
        view.source_digest(),
        view.producer_identity(),
        view.result_fingerprint()
    );
    let mut mesh_obj = format!(
        "# Ketchup exact body OBJ\n# {}# canonical_authority=canonical source identity and transform\n",
        loss_report.replace('\n', "\n# ")
    );
    for index in 0..view.vertex_count() {
        let [x, y, z] = transform_exact_point(matrix, view.vertex_position_mm(index));
        writeln!(mesh_obj, "v {x:.17} {y:.17} {z:.17}").expect("writing to a String cannot fail");
    }
    let mut current_group = None;
    for triangle_index in 0..view.triangle_count() {
        let group = view.triangle_group(triangle_index);
        if current_group != Some(group) {
            writeln!(mesh_obj, "g {group}").expect("writing to a String cannot fail");
            current_group = Some(group);
        }
        let mut indices = view.triangle_indices(triangle_index);
        if determinant < 0.0 {
            indices.swap(1, 2);
        }
        writeln!(
            mesh_obj,
            "f {} {} {}",
            indices[0] + 1,
            indices[1] + 1,
            indices[2] + 1
        )
        .expect("writing to a String cannot fail");
    }
    ExactMeshExport {
        mesh_obj,
        loss_report,
    }
}

fn transform_exact_point(matrix: &[f64; 16], point: [f64; 3]) -> [f64; 3] {
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

impl From<ExactRenderPackage> for ExactBodyPackage {
    fn from(package: ExactRenderPackage) -> Self {
        Self::Rectangle(package)
    }
}

impl From<crate::bottle_m6::ExactRevolvePackage> for ExactBodyPackage {
    fn from(package: crate::bottle_m6::ExactRevolvePackage) -> Self {
        Self::Revolve(package)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactResultKey {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub producer_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
    pub schema: String,
    pub result_fingerprint: String,
}

#[derive(Clone, Debug, Default)]
pub struct ExactResultRegistry {
    packages: BTreeMap<ExactResultKey, Arc<ExactBodyPackage>>,
    beam_packages: BTreeMap<BeamExactResultKey, Arc<BeamExactPiecePackage>>,
}

impl ExactResultRegistry {
    pub fn accept(
        snapshot: &Snapshot,
        packages: impl IntoIterator<Item = Arc<ExactBodyPackage>>,
    ) -> Result<Self, ExactProductError> {
        let mut registry = Self::default();
        for package in packages {
            registry.insert_current(snapshot, package)?;
        }
        Ok(registry)
    }

    pub fn accept_beam(
        snapshot: &Snapshot,
        packages: impl IntoIterator<Item = Arc<BeamExactPiecePackage>>,
    ) -> Result<Self, ExactProductError> {
        let mut registry = Self::default();
        for package in packages {
            registry.insert_current_beam(snapshot, package)?;
        }
        Ok(registry)
    }

    pub fn insert_current(
        &mut self,
        snapshot: &Snapshot,
        package: Arc<ExactBodyPackage>,
    ) -> Result<(), ExactProductError> {
        if !package.is_current(snapshot) {
            return Err(ExactProductError::StaleResult);
        }
        let key = package.result_key();
        if self.packages.contains_key(&key) {
            return Err(ExactProductError::DuplicateResult {
                definition_id: key.definition_id,
                producer_feature_id: key.producer_feature_id,
            });
        }
        self.packages.insert(key, package);
        Ok(())
    }

    pub fn insert_current_beam(
        &mut self,
        snapshot: &Snapshot,
        package: Arc<BeamExactPiecePackage>,
    ) -> Result<(), ExactProductError> {
        if package.identity.document_id != snapshot.document_id()
            || package.identity.source_revision != snapshot.revision_id()
            || package.identity.source_digest != snapshot.canonical_digest()
        {
            return Err(ExactProductError::StaleResult);
        }
        if !package.has_valid_registry_evidence() {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let key = package.result_key();
        if self
            .beam_packages
            .keys()
            .any(|existing| existing.piece == key.piece)
        {
            return Err(ExactProductError::DuplicateDerivedResult {
                piece: key.piece.clone(),
            });
        }
        self.beam_packages.insert(key, package);
        Ok(())
    }

    #[must_use]
    pub fn get_result(&self, key: &ExactResultKey) -> Option<&Arc<ExactBodyPackage>> {
        self.packages.get(key)
    }

    #[must_use]
    pub fn get_beam_result(&self, key: &BeamExactResultKey) -> Option<&Arc<BeamExactPiecePackage>> {
        self.beam_packages.get(key)
    }

    #[must_use]
    pub fn get_beam(&self, piece: &DerivedIdentity) -> Option<&Arc<BeamExactPiecePackage>> {
        let mut matches = self
            .beam_packages
            .iter()
            .filter(|(key, _)| key.piece == *piece)
            .map(|(_, package)| package);
        let package = matches.next()?;
        matches.next().is_none().then_some(package)
    }

    #[must_use]
    pub fn get(&self, definition_id: &DefinitionId) -> Option<&Arc<ExactBodyPackage>> {
        let mut matches = self
            .packages
            .iter()
            .filter(|(key, _)| key.definition_id == *definition_id)
            .map(|(_, package)| package);
        let package = matches.next()?;
        matches.next().is_none().then_some(package)
    }

    #[must_use]
    pub fn resolve_reference(
        &self,
        snapshot: &Snapshot,
        reference: &BodySubshapeRef,
    ) -> ExactReferenceResolution {
        if !reference.has_valid_lineage() {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::InvalidLineage,
            };
        }
        if reference.document_id != snapshot.document_id() {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::WrongDocument,
            };
        }
        if snapshot
            .feature(reference.producer_feature_id)
            .is_none_or(|producer| producer.definition_id() != reference.definition_id)
        {
            return ExactReferenceResolution::Lost;
        }

        let candidates = self
            .packages
            .values()
            .filter(|package| package.is_current(snapshot))
            .flat_map(|package| package.references())
            .filter(|candidate| {
                candidate.has_valid_lineage()
                    && candidate.document_id == reference.document_id
                    && candidate.definition_id == reference.definition_id
                    && candidate.profile_feature_id == reference.profile_feature_id
                    && candidate.producer_feature_id == reference.producer_feature_id
                    && candidate.semantic_role == reference.semantic_role
                    && candidate.source_element_id == reference.source_element_id
                    && candidate.expected_type == reference.expected_type
                    && candidate.expected_cardinality == reference.expected_cardinality
                    && candidate.stability == reference.stability
            })
            .collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else {
            return if candidates.is_empty() {
                ExactReferenceResolution::Lost
            } else {
                ExactReferenceResolution::Ambiguous {
                    candidate_count: candidates.len(),
                }
            };
        };
        if candidate.lineage_digest != reference.lineage_digest {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::InvalidLineage,
            };
        }
        if candidate.evaluator != reference.evaluator
            || candidate.backend != reference.backend
            || candidate.tolerance != reference.tolerance
        {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::IncompatibleEvaluationEnvelope,
            };
        }
        ExactReferenceResolution::Resolved {
            reference: Box::new((*candidate).clone()),
        }
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<ExactBodyPackage>> {
        self.packages.values()
    }

    pub fn beam_values(&self) -> impl Iterator<Item = &Arc<BeamExactPiecePackage>> {
        self.beam_packages.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    #[must_use]
    pub fn beam_len(&self) -> usize {
        self.beam_packages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn clear(&mut self) {
        self.packages.clear();
        self.beam_packages.clear();
    }
}

impl ExactRenderPackage {
    #[must_use]
    pub fn reference(&self, role: ExactFaceRole) -> Option<&BodySubshapeRef> {
        self.references
            .iter()
            .find(|reference| reference.role() == Some(role))
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactFeatureChainRequest::from_snapshot(snapshot, self.identity.definition_id)
                .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactFeatureChainRequest,
    ) -> Result<(), ExactProductError> {
        let expected_roles = request.expected_face_roles();
        let mut actual_roles = self
            .references
            .iter()
            .map(BodySubshapeRef::role)
            .collect::<Option<Vec<_>>>()
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        actual_roles.sort_unstable();
        let mut sorted_expected = expected_roles.to_vec();
        sorted_expected.sort_unstable();
        let is_cut = request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut);
        let expected_counts = if is_cut { (16, 32) } else { (8, 12) };
        let expected_triangle_roles = expected_roles.iter().all(|role| {
            let expected = match role {
                ExactFaceRole::Top | ExactFaceRole::Bottom if is_cut => 8,
                _ => 2,
            };
            self.triangles
                .iter()
                .filter(|triangle| triangle.face_role == Some(*role))
                .count()
                == expected
        });
        if self.identity.schema != EXACT_PRODUCT_SCHEMA_V1
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.definition_id != request.definition_id
            || self.identity.profile_feature_id != request.profile_feature_id
            || self.identity.extrusion_feature_id != request.extrusion_feature_id
            || self.identity.producer_feature_id != request.producer_feature_id()
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != request.evaluator()
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || self.vertices.len() != expected_counts.0
            || self.triangles.len() != expected_counts.1
            || self.triangles.iter().any(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= self.vertices.len())
            })
            || self.references.len() != expected_roles.len()
            || actual_roles != sorted_expected
            || !expected_triangle_roles
            || self.references.iter().any(|reference| {
                !reference.matches_request(request)
                    || reference.exact_input_digest != self.identity.exact_input_digest
                    || reference.result_fingerprint != self.identity.result_fingerprint
                    || reference.backend != self.identity.backend
                    || reference.tolerance != self.identity.tolerance
            })
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssemblySelectionTarget {
    pub instance_path: InstancePath,
    pub body: BodySubshapeRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactBooleanRequest {
    pub feature_id: FeatureId,
    pub operation: BooleanOperation,
    pub target_feature_id: FeatureId,
    pub tool_feature_id: FeatureId,
    pub profile_feature_id: FeatureId,
    pub min_x_bits: u64,
    pub min_y_bits: u64,
    pub width_bits: u64,
    pub depth_bits: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactFeatureChainRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub extrusion_feature_id: FeatureId,
    pub width_bits: u64,
    pub depth_bits: u64,
    pub height_bits: u64,
    pub boolean: Option<ExactBooleanRequest>,
    pub canonical_input_digest: String,
}

impl ExactFeatureChainRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        if definition.feature_ids().iter().any(|feature_id| {
            snapshot.feature(*feature_id).is_some_and(|feature| {
                matches!(
                    feature.kind(),
                    FeatureKind::BottleProfileControl { .. }
                        | FeatureKind::Revolve { .. }
                        | FeatureKind::Shell { .. }
                        | FeatureKind::BottleEdgeFinish { .. }
                )
            })
        }) {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let extrusions = definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Extrusion { profile, height } = feature.kind() else {
                    return None;
                };
                Some((*id, *profile, height.millimetres()))
            })
            .collect::<Vec<_>>();
        let legacy_cuts = definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::ThroughCut { target, profile } = feature.kind() else {
                    return None;
                };
                Some((*id, *target, *profile))
            })
            .collect::<Vec<_>>();
        let legacy_through_cut = !legacy_cuts.is_empty();
        let booleans = definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Boolean {
                    operation,
                    target,
                    tool,
                } = feature.kind()
                else {
                    return None;
                };
                Some((*id, *operation, *target, *tool))
            })
            .collect::<Vec<_>>();
        let (extrusion_feature_id, profile_feature_id, height_mm, boolean_source) =
            match (legacy_cuts.as_slice(), booleans.as_slice()) {
                ([], []) => {
                    let [(extrusion, profile, height)] = extrusions.as_slice() else {
                        return Err(ExactProductError::UnsupportedDefinition);
                    };
                    (*extrusion, *profile, *height, None)
                }
                ([(feature_id, target, profile)], []) => {
                    let [(extrusion, base_profile, height)] = extrusions.as_slice() else {
                        return Err(ExactProductError::UnsupportedDefinition);
                    };
                    if target != extrusion {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                    (
                        *extrusion,
                        *base_profile,
                        *height,
                        Some((
                            *feature_id,
                            BooleanOperation::Cut,
                            *target,
                            *profile,
                            *profile,
                            *height,
                        )),
                    )
                }
                ([], [(feature_id, operation, target, tool)]) => {
                    if extrusions.len() != 2 {
                        return Err(ExactProductError::UnsupportedBoolean(*operation));
                    }
                    let (target_id, target_profile, target_height) = extrusions
                        .iter()
                        .find(|(id, _, _)| id == target)
                        .copied()
                        .ok_or(ExactProductError::UnsupportedDefinition)?;
                    let (tool_id, tool_profile, tool_height) = extrusions
                        .iter()
                        .find(|(id, _, _)| id == tool)
                        .copied()
                        .ok_or(ExactProductError::UnsupportedDefinition)?;
                    if target_height.to_bits() != tool_height.to_bits() {
                        return Err(ExactProductError::UnsupportedBoolean(*operation));
                    }
                    (
                        target_id,
                        target_profile,
                        target_height,
                        Some((
                            *feature_id,
                            *operation,
                            target_id,
                            tool_id,
                            tool_profile,
                            tool_height,
                        )),
                    )
                }
                _ => return Err(ExactProductError::UnsupportedDefinition),
            };
        let profile = snapshot
            .feature(profile_feature_id)
            .ok_or(ExactProductError::ProfileNotFound(profile_feature_id))?;
        let FeatureKind::Profile { points_mm } = profile.kind() else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let (width_mm, depth_mm) =
            origin_rectangle_size(points_mm).ok_or(ExactProductError::UnsupportedProfile)?;
        if !height_mm.is_finite() || height_mm <= 0.0 {
            return Err(ExactProductError::UnsupportedExtrusion);
        }
        let boolean = boolean_source
            .map(
                |(feature_id, operation, target, tool, tool_profile_id, tool_height)| {
                    let tool_profile = snapshot
                        .feature(tool_profile_id)
                        .ok_or(ExactProductError::ProfileNotFound(tool_profile_id))?;
                    let FeatureKind::Profile {
                        points_mm: tool_points,
                    } = tool_profile.kind()
                    else {
                        return Err(ExactProductError::UnsupportedProfile);
                    };
                    let [min_x, min_y, max_x, max_y] = rectangle_bounds(tool_points)
                        .ok_or(ExactProductError::UnsupportedBoolean(operation))?;
                    if tool_height.to_bits() != height_mm.to_bits()
                        || min_x <= 0.0
                        || min_y <= 0.0
                        || max_x >= width_mm
                        || max_y >= depth_mm
                    {
                        return Err(ExactProductError::UnsupportedBoolean(operation));
                    }
                    Ok(ExactBooleanRequest {
                        feature_id,
                        operation,
                        target_feature_id: target,
                        tool_feature_id: tool,
                        profile_feature_id: tool_profile_id,
                        min_x_bits: min_x.to_bits(),
                        min_y_bits: min_y.to_bits(),
                        width_bits: (max_x - min_x).to_bits(),
                        depth_bits: (max_y - min_y).to_bits(),
                    })
                },
            )
            .transpose()?;
        let source_digest = snapshot.canonical_digest();
        let canonical_input = format!(
            "{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{}",
            EXACT_PRODUCT_SCHEMA_V1,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            extrusion_feature_id.0,
            width_mm.to_bits(),
            depth_mm.to_bits(),
            height_mm.to_bits(),
            source_digest
        );
        let canonical_input_digest = boolean.as_ref().map_or_else(
            || digest(&canonical_input),
            |cut| {
                if legacy_through_cut {
                    digest(&format!(
                        "{canonical_input}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
                        cut.feature_id.0,
                        cut.profile_feature_id.0,
                        cut.min_x_bits,
                        cut.min_y_bits,
                        cut.width_bits,
                        cut.depth_bits
                    ))
                } else {
                    digest(&format!(
                        "{canonical_input}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
                        cut.feature_id.0,
                        match cut.operation {
                            BooleanOperation::Cut => "cut",
                            BooleanOperation::Union => "union",
                        },
                        cut.target_feature_id.0,
                        cut.tool_feature_id.0,
                        cut.profile_feature_id.0,
                        cut.min_x_bits,
                        cut.min_y_bits,
                        cut.width_bits,
                        cut.depth_bits
                    ))
                }
            },
        );
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id,
            extrusion_feature_id,
            width_bits: width_mm.to_bits(),
            depth_bits: depth_mm.to_bits(),
            height_bits: height_mm.to_bits(),
            boolean,
            canonical_input_digest,
        })
    }

    #[must_use]
    pub fn dimensions_mm(&self) -> [f64; 3] {
        [
            f64::from_bits(self.width_bits),
            f64::from_bits(self.depth_bits),
            f64::from_bits(self.height_bits),
        ]
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        self.boolean
            .as_ref()
            .map_or(self.extrusion_feature_id, |cut| cut.feature_id)
    }

    #[must_use]
    pub fn evaluator(&self) -> &'static str {
        match self.boolean.as_ref().map(|boolean| boolean.operation) {
            Some(BooleanOperation::Cut) => EXACT_THROUGH_CUT_EVALUATOR_V1,
            Some(BooleanOperation::Union) => EXACT_BOOLEAN_UNION_EVALUATOR_V1,
            None => EXACT_RECTANGLE_EVALUATOR_V1,
        }
    }

    #[must_use]
    pub fn profile_feature_id_for_role(&self, role: ExactFaceRole) -> Option<FeatureId> {
        match role {
            ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::East => {
                Some(self.profile_feature_id)
            }
            ExactFaceRole::CutWest
            | ExactFaceRole::CutEast
            | ExactFaceRole::CutSouth
            | ExactFaceRole::CutNorth => self.boolean.as_ref().map(|cut| cut.profile_feature_id),
            ExactFaceRole::RevolveBottom
            | ExactFaceRole::RevolveBody
            | ExactFaceRole::RevolveShoulder
            | ExactFaceRole::RevolveNeck
            | ExactFaceRole::RevolveMouth
            | ExactFaceRole::ShellOuterBottom
            | ExactFaceRole::ShellOuterBody
            | ExactFaceRole::ShellOuterShoulder
            | ExactFaceRole::ShellOuterNeck
            | ExactFaceRole::ShellRim
            | ExactFaceRole::ShellInnerBottom
            | ExactFaceRole::ShellInnerBody
            | ExactFaceRole::ShellInnerShoulder
            | ExactFaceRole::ShellInnerNeck => None,
        }
    }

    fn expected_face_roles(&self) -> &'static [ExactFaceRole] {
        if self
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut)
        {
            &THROUGH_CUT_FACE_ROLES
        } else {
            &EXTRUSION_FACE_ROLES
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactProductError {
    DefinitionNotFound(DefinitionId),
    ProfileNotFound(FeatureId),
    UnsupportedDefinition,
    UnsupportedProfile,
    UnsupportedExtrusion,
    UnsupportedThroughCut,
    UnsupportedBoolean(BooleanOperation),
    UnsupportedShell,
    InvalidWorkerEvidence,
    StaleResult,
    DuplicateResult {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    },
    DuplicateDerivedResult {
        piece: DerivedIdentity,
    },
}

impl fmt::Display for ExactProductError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} was not found", id.0),
            Self::ProfileNotFound(id) => write!(formatter, "profile {} was not found", id.0),
            Self::UnsupportedDefinition => {
                formatter.write_str("exact M3 supports exactly one rectangle extrusion")
            }
            Self::UnsupportedProfile => {
                formatter.write_str("exact M3 supports an origin-based axis-aligned rectangle")
            }
            Self::UnsupportedExtrusion => {
                formatter.write_str("exact M3 supports a finite positive extrusion")
            }
            Self::UnsupportedThroughCut => formatter.write_str(
                "exact M3 supports one strictly bounded axis-aligned rectangle through-cut",
            ),
            Self::UnsupportedBoolean(operation) => write!(
                formatter,
                "exact feature-chain evaluator does not support {operation:?} in this envelope"
            ),
            Self::UnsupportedShell => formatter
                .write_str("exact M6 shell thickness is outside the conservative bottle envelope"),
            Self::InvalidWorkerEvidence => {
                formatter.write_str("exact worker evidence does not match the canonical request")
            }
            Self::StaleResult => formatter.write_str("exact result is stale for the snapshot"),
            Self::DuplicateResult {
                definition_id,
                producer_feature_id,
            } => write!(
                formatter,
                "duplicate exact result for definition {} producer {}",
                definition_id.0, producer_feature_id.0
            ),
            Self::DuplicateDerivedResult { piece } => write!(
                formatter,
                "duplicate exact result for derived rule {} slot path",
                piece.root_rule_node_id.0
            ),
        }
    }
}

impl std::error::Error for ExactProductError {}

pub fn build_box_render_package<const N: usize>(
    request: &ExactFeatureChainRequest,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    worker_bounds_mm: [[f64; 3]; 2],
    face_evidence: [(ExactFaceRole, String, String); N],
) -> Result<ExactRenderPackage, ExactProductError> {
    let [worker_min, worker_max] = worker_bounds_mm;
    let dimensions = request.dimensions_mm();
    if worker_min
        .into_iter()
        .chain(worker_max)
        .any(|value| !value.is_finite())
        || (0..3).any(|axis| {
            worker_min[axis].abs() > 1.0e-6 || (worker_max[axis] - dimensions[axis]).abs() > 1.0e-6
        })
        || exact_input_digest.is_empty()
        || result_fingerprint.is_empty()
        || backend.is_empty()
        || tolerance.is_empty()
        || !valid_evidence_roles(request, &face_evidence)
        || face_evidence
            .iter()
            .any(|(_, lineage_digest, fingerprint)| {
                lineage_digest.is_empty() || fingerprint.is_empty()
            })
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let min = [0.0; 3];
    let max = dimensions;
    let (vertices, triangles) = render_mesh(request)?;
    let references = face_evidence
        .into_iter()
        .map(
            |(role, lineage_digest, corroborating_geometry_fingerprint)| {
                Ok(BodySubshapeRef {
                    schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
                    document_id: request.document_id,
                    definition_id: request.definition_id,
                    profile_feature_id: request
                        .profile_feature_id_for_role(role)
                        .ok_or(ExactProductError::InvalidWorkerEvidence)?,
                    producer_feature_id: request.producer_feature_id(),
                    semantic_role: role.semantic_role().to_owned(),
                    source_element_id: role.source_element_id().to_owned(),
                    expected_type: "planar_face".to_owned(),
                    expected_cardinality: 1,
                    stability: ReferenceStability::Guaranteed,
                    canonical_input_digest: request.canonical_input_digest.clone(),
                    exact_input_digest: exact_input_digest.clone(),
                    result_fingerprint: result_fingerprint.clone(),
                    evaluator: request.evaluator().to_owned(),
                    backend: backend.clone(),
                    tolerance: tolerance.clone(),
                    lineage_digest,
                    corroborating_geometry_fingerprint,
                })
            },
        )
        .collect::<Result<Vec<_>, ExactProductError>>()?;
    if references
        .iter()
        .any(|reference| !reference.matches_request(request))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(ExactRenderPackage {
        identity: BodyResultIdentity {
            schema: EXACT_PRODUCT_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            extrusion_feature_id: request.extrusion_feature_id,
            producer_feature_id: request.producer_feature_id(),
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest,
            result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend,
            tolerance,
        },
        bounds_mm: [min, max],
        vertices,
        triangles,
        references,
    })
}

fn valid_evidence_roles<const N: usize>(
    request: &ExactFeatureChainRequest,
    evidence: &[(ExactFaceRole, String, String); N],
) -> bool {
    let mut roles = evidence
        .iter()
        .map(|(role, _, _)| *role)
        .collect::<Vec<_>>();
    roles.sort_unstable();
    if roles.windows(2).any(|pair| pair[0] == pair[1])
        || roles
            .iter()
            .any(|role| !request.expected_face_roles().contains(role))
        || evidence.iter().any(|(role, lineage, _)| {
            *lineage
                != canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    "planar_face",
                )
        })
    {
        return false;
    }
    let expected_roles = request.expected_face_roles();
    roles.len() == expected_roles.len() && roles == expected_roles.to_vec()
}

fn render_mesh(
    request: &ExactFeatureChainRequest,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let [width, depth, height] = request.dimensions_mm();
    if [width, depth, height]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let outer = [
        [0.0, 0.0, 0.0],
        [width, 0.0, 0.0],
        [width, depth, 0.0],
        [0.0, depth, 0.0],
        [0.0, 0.0, height],
        [width, 0.0, height],
        [width, depth, height],
        [0.0, depth, height],
    ];
    let Some(cut) = request
        .boolean
        .as_ref()
        .filter(|boolean| boolean.operation == BooleanOperation::Cut)
    else {
        let vertices = outer
            .map(|position_mm| ExactVertex { position_mm })
            .to_vec();
        let triangles = [
            ([0, 2, 1], Some(ExactFaceRole::Bottom)),
            ([0, 3, 2], Some(ExactFaceRole::Bottom)),
            ([4, 5, 6], Some(ExactFaceRole::Top)),
            ([4, 6, 7], Some(ExactFaceRole::Top)),
            ([1, 2, 6], Some(ExactFaceRole::East)),
            ([1, 6, 5], Some(ExactFaceRole::East)),
            ([0, 4, 7], None),
            ([0, 7, 3], None),
            ([3, 7, 6], None),
            ([3, 6, 2], None),
            ([0, 1, 5], None),
            ([0, 5, 4], None),
        ]
        .map(|(vertex_indices, face_role)| ExactTriangle {
            vertex_indices,
            face_role,
        })
        .to_vec();
        return Ok((vertices, triangles));
    };
    let x0 = f64::from_bits(cut.min_x_bits);
    let y0 = f64::from_bits(cut.min_y_bits);
    let x1 = x0 + f64::from_bits(cut.width_bits);
    let y1 = y0 + f64::from_bits(cut.depth_bits);
    if [x0, y0, x1, y1].into_iter().any(|value| !value.is_finite())
        || x0 <= 0.0
        || y0 <= 0.0
        || x1 >= width
        || y1 >= depth
        || x0 >= x1
        || y0 >= y1
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut positions = outer.to_vec();
    positions.extend([
        [x0, y0, 0.0],
        [x1, y0, 0.0],
        [x1, y1, 0.0],
        [x0, y1, 0.0],
        [x0, y0, height],
        [x1, y0, height],
        [x1, y1, height],
        [x0, y1, height],
    ]);
    let vertices = positions
        .into_iter()
        .map(|position_mm| ExactVertex { position_mm })
        .collect();
    let mut triangles = Vec::with_capacity(32);
    let mut quad = |indices: [u32; 4], role| {
        triangles.push(ExactTriangle {
            vertex_indices: [indices[0], indices[1], indices[2]],
            face_role: role,
        });
        triangles.push(ExactTriangle {
            vertex_indices: [indices[0], indices[2], indices[3]],
            face_role: role,
        });
    };
    for indices in [[0, 8, 9, 1], [1, 9, 10, 2], [2, 10, 11, 3], [3, 11, 8, 0]] {
        quad(indices, Some(ExactFaceRole::Bottom));
    }
    for indices in [
        [4, 5, 13, 12],
        [5, 6, 14, 13],
        [6, 7, 15, 14],
        [7, 4, 12, 15],
    ] {
        quad(indices, Some(ExactFaceRole::Top));
    }
    quad([0, 1, 5, 4], None);
    quad([1, 2, 6, 5], Some(ExactFaceRole::East));
    quad([2, 3, 7, 6], None);
    quad([3, 0, 4, 7], None);
    quad([11, 15, 12, 8], Some(ExactFaceRole::CutWest));
    quad([9, 13, 14, 10], Some(ExactFaceRole::CutEast));
    quad([8, 12, 13, 9], Some(ExactFaceRole::CutSouth));
    quad([10, 14, 15, 11], Some(ExactFaceRole::CutNorth));
    Ok((vertices, triangles))
}

fn rectangle_bounds(points: &[[f64; 2]]) -> Option<[f64; 4]> {
    if points.len() != 4 || points.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let min_x = points.iter().map(|point| point[0]).reduce(f64::min)?;
    let min_y = points.iter().map(|point| point[1]).reduce(f64::min)?;
    let max_x = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let max_y = points.iter().map(|point| point[1]).reduce(f64::max)?;
    if min_x >= max_x || min_y >= max_y {
        return None;
    }
    let mut corners = points.to_vec();
    corners.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    (corners
        == vec![
            [min_x, min_y],
            [min_x, max_y],
            [max_x, min_y],
            [max_x, max_y],
        ])
    .then_some([min_x, min_y, max_x, max_y])
}

fn origin_rectangle_size(points: &[[f64; 2]]) -> Option<(f64, f64)> {
    if points.len() != 4 || points.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let width = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let depth = points.iter().map(|point| point[1]).reduce(f64::max)?;
    if width <= 0.0
        || depth <= 0.0
        || points.iter().any(|point| {
            !matches!(point[0], 0.0) && point[0] != width
                || !matches!(point[1], 0.0) && point[1] != depth
        })
    {
        return None;
    }
    let mut corners = points.to_vec();
    corners.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    (corners == vec![[0.0, 0.0], [0.0, depth], [width, 0.0], [width, depth]])
        .then_some((width, depth))
}

fn reference_lineage_digest(reference: &BodySubshapeRef) -> String {
    canonical_reference_lineage_digest(
        reference.document_id,
        reference.producer_feature_id,
        &reference.semantic_role,
        &reference.source_element_id,
        &reference.expected_type,
    )
}

#[must_use]
pub fn canonical_reference_lineage_digest(
    document_id: DocumentId,
    producer_feature_id: FeatureId,
    semantic_role: &str,
    source_element_id: &str,
    expected_type: &str,
) -> String {
    let identity = format!(
        "{}:{}:{}:{}:{}",
        document_id.0, producer_feature_id.0, semantic_role, source_element_id, expected_type
    );
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in identity.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
