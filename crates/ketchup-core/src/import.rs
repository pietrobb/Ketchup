use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind,
    IMPORTED_EXACT_BODY_SCHEMA_V1, ImportedExactBodySpec, MESH_BODY_SCHEMA_V1, MeshAuthority,
    MeshBodySpec, OccurrenceId, Snapshot, Transform,
};
use crate::graph::sha256_bytes;

pub const IMPORT_RECEIPT_SCHEMA_V1: &str = "ketchup.import-receipt.v1";
pub const MAX_IMPORT_SOURCE_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
pub const MAX_IMPORT_DIAGNOSTICS: usize = 1_024;
pub const MAX_IMPORT_OUTPUTS: usize = 1_024;
const MAX_IMPORT_TEXT_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ImportId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportFormat {
    Stl,
    Dxf,
    Step,
    SketchupScene,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportLengthUnit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
}

impl ImportLengthUnit {
    #[must_use]
    pub const fn millimetres_per_unit(self) -> f64 {
        match self {
            Self::Millimetre => 1.0,
            Self::Centimetre => 10.0,
            Self::Metre => 1_000.0,
            Self::Inch => 25.4,
            Self::Foot => 304.8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportUnitAuthority {
    FileDeclared,
    UserDeclared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportUnitDecision {
    source_unit: ImportLengthUnit,
    authority: ImportUnitAuthority,
}

impl ImportUnitDecision {
    #[must_use]
    pub const fn new(source_unit: ImportLengthUnit, authority: ImportUnitAuthority) -> Self {
        Self {
            source_unit,
            authority,
        }
    }

    #[must_use]
    pub const fn source_unit(self) -> ImportLengthUnit {
        self.source_unit
    }

    #[must_use]
    pub const fn authority(self) -> ImportUnitAuthority {
        self.authority
    }

    #[must_use]
    pub fn millimetres_per_unit(self) -> f64 {
        self.source_unit.millimetres_per_unit()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ImportDiagnosticSeverity {
    Info,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ImportDiagnostic {
    severity: ImportDiagnosticSeverity,
    code: String,
    subject: Option<String>,
    count: u32,
}

impl ImportDiagnostic {
    pub fn new(
        severity: ImportDiagnosticSeverity,
        code: impl Into<String>,
        subject: Option<String>,
        count: u32,
    ) -> Result<Self, ImportContractError> {
        let code = code.into();
        validate_text(&code)?;
        if subject
            .as_deref()
            .is_some_and(|value| validate_text(value).is_err())
            || count == 0
        {
            return Err(ImportContractError::InvalidDiagnostic);
        }
        Ok(Self {
            severity,
            code,
            subject,
            count,
        })
    }

    #[must_use]
    pub const fn severity(&self) -> ImportDiagnosticSeverity {
        self.severity
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ImportOutputRef {
    Definition(DefinitionId),
    Feature(FeatureId),
    Occurrence(OccurrenceId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReceipt {
    schema: String,
    id: ImportId,
    format: ImportFormat,
    source_sha256: [u8; 32],
    source_byte_len: u64,
    source_name: String,
    units: ImportUnitDecision,
    parser_id: String,
    parser_version: String,
    diagnostics: Vec<ImportDiagnostic>,
    outputs: Vec<ImportOutputRef>,
}

impl ImportReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ImportId,
        format: ImportFormat,
        source_sha256: [u8; 32],
        source_byte_len: u64,
        source_name: impl Into<String>,
        units: ImportUnitDecision,
        parser_id: impl Into<String>,
        parser_version: impl Into<String>,
        diagnostics: Vec<ImportDiagnostic>,
        outputs: Vec<ImportOutputRef>,
    ) -> Result<Self, ImportContractError> {
        let receipt = Self {
            schema: IMPORT_RECEIPT_SCHEMA_V1.to_owned(),
            id,
            format,
            source_sha256,
            source_byte_len,
            source_name: source_name.into(),
            units,
            parser_id: parser_id.into(),
            parser_version: parser_version.into(),
            diagnostics,
            outputs,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_source_bytes(
        id: ImportId,
        format: ImportFormat,
        source_bytes: &[u8],
        source_name: impl Into<String>,
        units: ImportUnitDecision,
        parser_id: impl Into<String>,
        parser_version: impl Into<String>,
        diagnostics: Vec<ImportDiagnostic>,
        outputs: Vec<ImportOutputRef>,
    ) -> Result<Self, ImportContractError> {
        Self::new(
            id,
            format,
            sha256_bytes(source_bytes),
            source_bytes.len() as u64,
            source_name,
            units,
            parser_id,
            parser_version,
            diagnostics,
            outputs,
        )
    }

    pub fn validate(&self) -> Result<(), ImportContractError> {
        if self.schema != IMPORT_RECEIPT_SCHEMA_V1 || self.id.0 == 0 {
            return Err(ImportContractError::InvalidIdentity);
        }
        if self.source_byte_len == 0 || self.source_byte_len > MAX_IMPORT_SOURCE_BYTES {
            return Err(ImportContractError::InvalidSource);
        }
        validate_text(&self.source_name)?;
        if self.source_name.contains(['/', '\\']) {
            return Err(ImportContractError::InvalidSource);
        }
        validate_text(&self.parser_id)?;
        validate_text(&self.parser_version)?;
        if self.diagnostics.len() > MAX_IMPORT_DIAGNOSTICS
            || self.diagnostics.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ImportContractError::DiagnosticsNotCanonical);
        }
        if self.outputs.is_empty()
            || self.outputs.len() > MAX_IMPORT_OUTPUTS
            || self.outputs.windows(2).any(|pair| pair[0] >= pair[1])
            || self.outputs.iter().any(|output| match output {
                ImportOutputRef::Definition(id) => id.0 == 0,
                ImportOutputRef::Feature(id) => id.0 == 0,
                ImportOutputRef::Occurrence(id) => id.0 == 0,
            })
        {
            return Err(ImportContractError::OutputsNotCanonical);
        }
        Ok(())
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> ImportId {
        self.id
    }

    #[must_use]
    pub const fn format(&self) -> ImportFormat {
        self.format
    }

    #[must_use]
    pub const fn source_sha256(&self) -> &[u8; 32] {
        &self.source_sha256
    }

    #[must_use]
    pub const fn source_byte_len(&self) -> u64 {
        self.source_byte_len
    }

    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    #[must_use]
    pub const fn units(&self) -> ImportUnitDecision {
        self.units
    }

    #[must_use]
    pub fn parser_id(&self) -> &str {
        &self.parser_id
    }

    #[must_use]
    pub fn parser_version(&self) -> &str {
        &self.parser_version
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn outputs(&self) -> &[ImportOutputRef] {
        &self.outputs
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportContractError {
    InvalidIdentity,
    InvalidSource,
    InvalidText,
    InvalidDiagnostic,
    DiagnosticsNotCanonical,
    OutputsNotCanonical,
}

impl fmt::Display for ImportContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "import identity is invalid",
            Self::InvalidSource => "import source identity is invalid",
            Self::InvalidText => "import metadata text is invalid",
            Self::InvalidDiagnostic => "import diagnostic is invalid",
            Self::DiagnosticsNotCanonical => "import diagnostics are not canonical",
            Self::OutputsNotCanonical => "import outputs are not canonical",
        })
    }
}

impl std::error::Error for ImportContractError {}

mod dxf;
pub use dxf::*;
mod sketchup_scene;
pub use sketchup_scene::*;

pub const STEP_PARSER_ID: &str = "ketchup-occt-step";
pub const STEP_PARSER_VERSION: &str = "2";
pub const MAX_STEP_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct StepImportEvidence {
    pub source_unit: ImportLengthUnit,
    pub result_fingerprint: String,
    pub solid_count: u32,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
}

/// Upper bound on a derived STEP display mesh.
///
/// The mesh is a discardable render product, not canonical state, so its
/// envelope is wider than the canonical mesh-body limits — but it is still
/// bounded so a hostile or pathological part cannot exhaust memory.
pub const MAX_STEP_MESH_TRIANGLES: u32 = 2_000_000;
pub const MAX_STEP_MESH_VERTICES: u32 = 6_000_000;
pub const STEP_MESH_MAGIC: &[u8; 12] = b"KETCHUPMESH1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepMeshTriangle {
    pub vertex_indices: [u32; 3],
    pub face_ordinal: u32,
}

/// A derived display mesh of an imported exact STEP body.
#[derive(Clone, Debug, PartialEq)]
pub struct StepImportMesh {
    pub vertices_mm: Vec<[f64; 3]>,
    pub triangles: Vec<StepMeshTriangle>,
}

impl StepImportMesh {
    /// Encode the mesh in the fixed little-endian transport layout.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = STEP_MESH_MAGIC.to_vec();
        bytes.extend_from_slice(&(self.vertices_mm.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.triangles.len() as u32).to_le_bytes());
        for coordinate in self.vertices_mm.iter().flatten() {
            bytes.extend_from_slice(&coordinate.to_le_bytes());
        }
        for triangle in &self.triangles {
            for index in triangle.vertex_indices {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
            bytes.extend_from_slice(&triangle.face_ordinal.to_le_bytes());
        }
        bytes
    }

    /// Decode a transported mesh, refusing anything that is not a complete,
    /// in-range, finite indexed mesh of exactly the declared size.
    pub fn decode(bytes: &[u8]) -> Result<Self, StepMeshError> {
        if bytes.len() < STEP_MESH_MAGIC.len() + 8 || !bytes.starts_with(STEP_MESH_MAGIC) {
            return Err(StepMeshError::Malformed);
        }
        let counts = &bytes[STEP_MESH_MAGIC.len()..STEP_MESH_MAGIC.len() + 8];
        let vertex_count = u32::from_le_bytes(counts[0..4].try_into().expect("four bytes"));
        let triangle_count = u32::from_le_bytes(counts[4..8].try_into().expect("four bytes"));
        if vertex_count == 0
            || triangle_count == 0
            || vertex_count > MAX_STEP_MESH_VERTICES
            || triangle_count > MAX_STEP_MESH_TRIANGLES
        {
            return Err(StepMeshError::OutOfEnvelope);
        }
        let vertex_bytes = vertex_count as usize * 24;
        let triangle_bytes = triangle_count as usize * 16;
        let payload = &bytes[STEP_MESH_MAGIC.len() + 8..];
        if payload.len() != vertex_bytes + triangle_bytes {
            return Err(StepMeshError::Malformed);
        }
        let mut vertices_mm = Vec::with_capacity(vertex_count as usize);
        for vertex in payload[..vertex_bytes].chunks_exact(24) {
            let mut position = [0.0_f64; 3];
            for (axis, value) in vertex.chunks_exact(8).enumerate() {
                position[axis] = f64::from_le_bytes(value.try_into().expect("eight bytes"));
            }
            if position.iter().any(|value| !value.is_finite()) {
                return Err(StepMeshError::Malformed);
            }
            vertices_mm.push(position);
        }
        let mut triangles = Vec::with_capacity(triangle_count as usize);
        for triangle in payload[vertex_bytes..].chunks_exact(16) {
            let mut fields = [0_u32; 4];
            for (slot, value) in triangle.chunks_exact(4).enumerate() {
                fields[slot] = u32::from_le_bytes(value.try_into().expect("four bytes"));
            }
            if fields[..3].iter().any(|index| *index >= vertex_count) {
                return Err(StepMeshError::Malformed);
            }
            triangles.push(StepMeshTriangle {
                vertex_indices: [fields[0], fields[1], fields[2]],
                face_ordinal: fields[3],
            });
        }
        Ok(Self {
            vertices_mm,
            triangles,
        })
    }

    /// Whether every vertex sits inside the canonical bounds the import
    /// receipt already committed to, within `tolerance_mm`.
    #[must_use]
    pub fn is_within_bounds(&self, bounds_mm: [[f64; 3]; 2], tolerance_mm: f64) -> bool {
        self.vertices_mm.iter().all(|vertex| {
            (0..3).all(|axis| {
                vertex[axis] >= bounds_mm[0][axis] - tolerance_mm
                    && vertex[axis] <= bounds_mm[1][axis] + tolerance_mm
            })
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepMeshError {
    Malformed,
    OutOfEnvelope,
}

impl fmt::Display for StepMeshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "imported STEP display mesh is malformed",
            Self::OutOfEnvelope => "imported STEP display mesh exceeds the bounded envelope",
        })
    }
}

impl std::error::Error for StepMeshError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepImportPlanError {
    Empty,
    SourceTooLarge,
    InvalidSourceIdentity,
    MissingOrAmbiguousUnits,
    InvalidWorkerEvidence,
    IdSpaceExhausted,
}

impl fmt::Display for StepImportPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "STEP source is empty",
            Self::SourceTooLarge => "STEP source exceeds the bounded 32 MiB envelope",
            Self::InvalidSourceIdentity => "STEP source name or provenance is invalid",
            Self::MissingOrAmbiguousUnits => {
                "STEP source has missing or ambiguous declared length units"
            }
            Self::InvalidWorkerEvidence => "STEP worker evidence is incomplete or invalid",
            Self::IdSpaceExhausted => "canonical import ID space is exhausted",
        })
    }
}

impl std::error::Error for StepImportPlanError {}

/// Build one detached canonical transaction for a reviewed, worker-validated exact STEP body.
pub fn plan_step_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    evidence: &StepImportEvidence,
) -> Result<CommandBatch, StepImportPlanError> {
    if source.is_empty() {
        return Err(StepImportPlanError::Empty);
    }
    if source.len() as u64 > MAX_STEP_SOURCE_BYTES {
        return Err(StepImportPlanError::SourceTooLarge);
    }
    let bounds_valid = evidence
        .bounds_mm
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        && (0..3).all(|axis| evidence.bounds_mm[0][axis] <= evidence.bounds_mm[1][axis]);
    if evidence.result_fingerprint.is_empty()
        || evidence.solid_count == 0
        || evidence.solid_count > 1_024
        || !evidence.volume_mm3.is_finite()
        || evidence.volume_mm3 <= 0.0
        || !bounds_valid
        || evidence.backend.is_empty()
        || evidence.tolerance.is_empty()
    {
        return Err(StepImportPlanError::InvalidWorkerEvidence);
    }
    let next_id = |ids: Vec<u64>| {
        ids.into_iter()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|id| *id != 0)
            .ok_or(StepImportPlanError::IdSpaceExhausted)
    };
    let import_id = snapshot
        .next_import_id()
        .map_err(|_| StepImportPlanError::IdSpaceExhausted)?;
    let definition_id = DefinitionId(next_id(
        snapshot.definitions().map(|item| item.id().0).collect(),
    )?);
    let feature_id = FeatureId(next_id(
        snapshot.features().map(|item| item.id().0).collect(),
    )?);
    let occurrence_id = OccurrenceId(next_id(
        snapshot.occurrences().map(|item| item.id().0).collect(),
    )?);
    let outputs = vec![
        ImportOutputRef::Definition(definition_id),
        ImportOutputRef::Feature(feature_id),
        ImportOutputRef::Occurrence(occurrence_id),
    ];
    let diagnostics = vec![
        ImportDiagnostic::new(
            ImportDiagnosticSeverity::Info,
            "step_exact_brep_preserved",
            None,
            1,
        ),
        ImportDiagnostic::new(
            ImportDiagnosticSeverity::Warning,
            "step_color_metadata_unavailable",
            None,
            1,
        ),
        ImportDiagnostic::new(
            ImportDiagnosticSeverity::Warning,
            "step_hierarchy_flattened",
            None,
            1,
        ),
        ImportDiagnostic::new(
            ImportDiagnosticSeverity::Warning,
            "step_name_metadata_unavailable",
            None,
            1,
        ),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| StepImportPlanError::InvalidWorkerEvidence)?;
    let units = ImportUnitDecision::new(evidence.source_unit, ImportUnitAuthority::FileDeclared);
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::Step,
        source,
        source_name,
        units,
        STEP_PARSER_ID,
        STEP_PARSER_VERSION,
        diagnostics,
        outputs,
    )
    .map_err(|_| StepImportPlanError::InvalidSourceIdentity)?;
    let display_name = source_name
        .strip_suffix(".step")
        .or_else(|| source_name.strip_suffix(".stp"))
        .or_else(|| source_name.strip_suffix(".STEP"))
        .or_else(|| source_name.strip_suffix(".STP"))
        .filter(|name| !name.is_empty())
        .unwrap_or(source_name)
        .to_owned();
    Ok(CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: display_name.clone(),
        },
        CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: "Imported STEP exact body".to_owned(),
            kind: FeatureKind::ImportedExactBody(ImportedExactBodySpec {
                schema: IMPORTED_EXACT_BODY_SCHEMA_V1.to_owned(),
                import_id,
                source_sha256: sha256_bytes(source),
                source_byte_len: source.len() as u64,
                result_fingerprint: evidence.result_fingerprint.clone(),
                solid_count: evidence.solid_count,
                volume_mm3: evidence.volume_mm3,
                bounds_mm: evidence.bounds_mm,
                backend: evidence.backend.clone(),
                tolerance: evidence.tolerance.clone(),
            }),
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: display_name,
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::RecordImport(receipt),
    ]))
}

fn validate_text(value: &str) -> Result<(), ImportContractError> {
    if value.is_empty()
        || value.len() > MAX_IMPORT_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        Err(ImportContractError::InvalidText)
    } else {
        Ok(())
    }
}

pub const STL_PARSER_ID: &str = "ketchup-stl";
pub const STL_PARSER_VERSION: &str = "1";
const MAX_STL_TRIANGLES: usize = 200_000;
const MAX_STL_ASCII_LINE_BYTES: usize = 128;
const MAX_STL_ASCII_LINES_PER_TRIANGLE: usize = 8;
const MAX_STL_ASCII_LINES: usize = (MAX_STL_TRIANGLES * MAX_STL_ASCII_LINES_PER_TRIANGLE) + 2;
// Each facet requires seven structural lines. The bounded ASCII subset permits
// one additional blank line per facet and CRLF endings without unbounded input.
pub const MAX_STL_SOURCE_BYTES: u64 =
    MAX_STL_ASCII_LINES as u64 * (MAX_STL_ASCII_LINE_BYTES as u64 + 2);
const MAX_STL_VERTICES: usize = 100_000;
const MAX_STL_ABS_MM: f64 = 1_000_000.0;
const STL_AREA_EPSILON: f64 = 1.0e-18;
const STL_VOLUME_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedStlMesh {
    vertices_mm: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    diagnostics: Vec<ImportDiagnostic>,
}

impl ParsedStlMesh {
    #[must_use]
    pub fn vertices_mm(&self) -> &[[f64; 3]] {
        &self.vertices_mm
    }

    #[must_use]
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StlImportError {
    Empty,
    NoTriangles,
    SourceTooLarge,
    UnrecognizedEncoding,
    InvalidBinaryLength,
    InvalidAscii,
    InvalidNumber,
    UnitsMustBeUserDeclared,
    TooManyTriangles,
    TooManyVertices,
    CoordinateOutOfRange,
    DegenerateTriangle,
    DuplicateTriangle,
    NonManifoldEdge,
    InconsistentOrientation,
    DisconnectedShells,
    NonPositiveVolume,
}

impl fmt::Display for StlImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "STL source is empty",
            Self::NoTriangles => "STL source contains no triangles",
            Self::SourceTooLarge => "STL source exceeds the bounded 200,000-facet text envelope",
            Self::UnrecognizedEncoding => "source is neither a bounded binary nor ASCII STL",
            Self::InvalidBinaryLength => "binary STL triangle count does not match its byte length",
            Self::InvalidAscii => "ASCII STL structure is malformed or contains unsupported text",
            Self::InvalidNumber => "STL contains an invalid numeric coordinate or normal",
            Self::UnitsMustBeUserDeclared => {
                "STL has no authoritative units; choose the source length unit explicitly"
            }
            Self::TooManyTriangles => "STL exceeds the 200,000 triangle limit",
            Self::TooManyVertices => "STL exceeds the 100,000 unique vertex limit",
            Self::CoordinateOutOfRange => {
                "scaled STL coordinates exceed the canonical ±1,000,000 mm envelope"
            }
            Self::DegenerateTriangle => {
                "STL contains a zero-area or repeated-vertex triangle; geometry was not repaired"
            }
            Self::DuplicateTriangle => "STL contains duplicate facets; geometry was not repaired",
            Self::NonManifoldEdge => {
                "STL is not a closed two-manifold; every edge must have exactly two facets"
            }
            Self::InconsistentOrientation => {
                "STL facet winding is inconsistent; orientation was not silently changed"
            }
            Self::DisconnectedShells => {
                "STL contains disconnected shells; import one solid per file"
            }
            Self::NonPositiveVolume => {
                "STL winding does not define a strictly positive enclosed volume"
            }
        })
    }
}

impl std::error::Error for StlImportError {}

/// Parse an STL into detached canonical mesh data. The function never welds,
/// removes, fills, flips, or otherwise repairs source geometry.
pub fn parse_stl(
    source: &[u8],
    units: ImportUnitDecision,
) -> Result<ParsedStlMesh, StlImportError> {
    if source.is_empty() {
        return Err(StlImportError::Empty);
    }
    if source.len() as u64 > MAX_STL_SOURCE_BYTES {
        return Err(StlImportError::SourceTooLarge);
    }
    if units.authority() != ImportUnitAuthority::UserDeclared {
        return Err(StlImportError::UnitsMustBeUserDeclared);
    }

    let expected_binary_len = source.get(80..84).map(|count| {
        let count = u32::from_le_bytes(count.try_into().expect("four-byte STL count")) as usize;
        (
            count,
            count
                .checked_mul(50)
                .and_then(|size| 84_usize.checked_add(size)),
        )
    });
    let (facets, encoding) = match expected_binary_len {
        Some((count, Some(expected))) if expected == source.len() => {
            (parse_binary_stl(source, count)?, "binary")
        }
        _ if source.starts_with(b"solid") => (parse_ascii_stl(source)?, "ascii"),
        Some((_, Some(_))) => return Err(StlImportError::InvalidBinaryLength),
        _ => return Err(StlImportError::UnrecognizedEncoding),
    };
    if facets.is_empty() {
        return Err(StlImportError::NoTriangles);
    }
    if facets.len() > MAX_STL_TRIANGLES {
        return Err(StlImportError::TooManyTriangles);
    }
    normalize_and_validate_stl(facets, units.millimetres_per_unit(), encoding)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StlImportPlanError {
    Parse(StlImportError),
    InvalidSourceIdentity,
    IdSpaceExhausted,
}

impl fmt::Display for StlImportPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(formatter),
            Self::InvalidSourceIdentity => {
                formatter.write_str("STL source name or provenance is invalid")
            }
            Self::IdSpaceExhausted => formatter.write_str("canonical import ID space is exhausted"),
        }
    }
}

impl std::error::Error for StlImportPlanError {}

impl From<StlImportError> for StlImportPlanError {
    fn from(error: StlImportError) -> Self {
        Self::Parse(error)
    }
}

/// Build one detached canonical transaction for a reviewed STL import.
pub fn plan_stl_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    units: ImportUnitDecision,
) -> Result<CommandBatch, StlImportPlanError> {
    let mesh = parse_stl(source, units)?;
    let import_id = snapshot
        .next_import_id()
        .map_err(|_| StlImportPlanError::IdSpaceExhausted)?;
    let definition_id = DefinitionId(next_product_id(
        snapshot.definitions().map(|item| item.id().0),
    )?);
    let feature_id = FeatureId(next_product_id(
        snapshot.features().map(|item| item.id().0),
    )?);
    let occurrence_id = OccurrenceId(next_product_id(
        snapshot.occurrences().map(|item| item.id().0),
    )?);
    let outputs = vec![
        ImportOutputRef::Definition(definition_id),
        ImportOutputRef::Feature(feature_id),
        ImportOutputRef::Occurrence(occurrence_id),
    ];
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::Stl,
        source,
        source_name,
        units,
        STL_PARSER_ID,
        STL_PARSER_VERSION,
        mesh.diagnostics,
        outputs,
    )
    .map_err(|_| StlImportPlanError::InvalidSourceIdentity)?;
    let display_name = source_name
        .strip_suffix(".stl")
        .or_else(|| source_name.strip_suffix(".STL"))
        .filter(|name| !name.is_empty())
        .unwrap_or(source_name)
        .to_owned();
    Ok(CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: display_name.clone(),
        },
        CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: "STL mesh".to_owned(),
            kind: FeatureKind::MeshBody(MeshBodySpec {
                schema: MESH_BODY_SCHEMA_V1.to_owned(),
                vertices_mm: mesh.vertices_mm,
                triangles: mesh.triangles,
                authority: MeshAuthority::ImportedStl { import_id },
            }),
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: display_name,
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::RecordImport(receipt),
    ]))
}

fn next_product_id(ids: impl Iterator<Item = u64>) -> Result<u64, StlImportPlanError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id != 0)
        .ok_or(StlImportPlanError::IdSpaceExhausted)
}

type StlFacet = ([[f64; 3]; 3], [f64; 3]);

fn parse_binary_stl(source: &[u8], count: usize) -> Result<Vec<StlFacet>, StlImportError> {
    if count > MAX_STL_TRIANGLES {
        return Err(StlImportError::TooManyTriangles);
    }
    let mut facets = Vec::with_capacity(count);
    for record in source[84..].chunks_exact(50) {
        let mut values = [0.0; 12];
        for (index, value) in values.iter_mut().enumerate() {
            let start = index * 4;
            *value = f64::from(f32::from_le_bytes(
                record[start..start + 4]
                    .try_into()
                    .expect("bounded STL float"),
            ));
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(StlImportError::InvalidNumber);
        }
        facets.push((
            [
                [values[3], values[4], values[5]],
                [values[6], values[7], values[8]],
                [values[9], values[10], values[11]],
            ],
            [values[0], values[1], values[2]],
        ));
    }
    Ok(facets)
}

fn parse_ascii_stl(source: &[u8]) -> Result<Vec<StlFacet>, StlImportError> {
    let text = std::str::from_utf8(source).map_err(|_| StlImportError::InvalidAscii)?;
    if !text.is_ascii() {
        return Err(StlImportError::InvalidAscii);
    }
    let mut source_line_count = 0_usize;
    for line in text.lines() {
        source_line_count += 1;
        if source_line_count > MAX_STL_ASCII_LINES {
            return Err(StlImportError::SourceTooLarge);
        }
        if line.len() > MAX_STL_ASCII_LINE_BYTES {
            return Err(StlImportError::InvalidAscii);
        }
    }
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let header = lines.next().ok_or(StlImportError::InvalidAscii)?;
    if header != "solid" && !header.starts_with("solid ") {
        return Err(StlImportError::InvalidAscii);
    }
    let mut facets = Vec::new();
    loop {
        let line = lines.next().ok_or(StlImportError::InvalidAscii)?;
        if line == "endsolid" || line.starts_with("endsolid ") {
            if facets.is_empty() || lines.next().is_some() {
                return Err(StlImportError::InvalidAscii);
            }
            return Ok(facets);
        }
        if facets.len() == MAX_STL_TRIANGLES {
            return Err(StlImportError::TooManyTriangles);
        }
        let normal = parse_ascii_vector(line, "facet normal")?;
        if lines.next() != Some("outer loop") {
            return Err(StlImportError::InvalidAscii);
        }
        let vertices = [
            parse_ascii_vector(lines.next().ok_or(StlImportError::InvalidAscii)?, "vertex")?,
            parse_ascii_vector(lines.next().ok_or(StlImportError::InvalidAscii)?, "vertex")?,
            parse_ascii_vector(lines.next().ok_or(StlImportError::InvalidAscii)?, "vertex")?,
        ];
        if lines.next() != Some("endloop") || lines.next() != Some("endfacet") {
            return Err(StlImportError::InvalidAscii);
        }
        facets.push((vertices, normal));
    }
}

fn parse_ascii_vector(line: &str, prefix: &str) -> Result<[f64; 3], StlImportError> {
    let mut values = line
        .strip_prefix(prefix)
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .ok_or(StlImportError::InvalidAscii)?
        .split_whitespace()
        .map(|token| {
            token
                .parse::<f64>()
                .map_err(|_| StlImportError::InvalidNumber)
        });
    let coordinates = [
        values.next().ok_or(StlImportError::InvalidAscii)??,
        values.next().ok_or(StlImportError::InvalidAscii)??,
        values.next().ok_or(StlImportError::InvalidAscii)??,
    ];
    if values.next().is_some() {
        return Err(StlImportError::InvalidAscii);
    }
    if coordinates.into_iter().any(|value| !value.is_finite()) {
        return Err(StlImportError::InvalidNumber);
    }
    Ok(coordinates)
}

fn normalize_and_validate_stl(
    facets: Vec<StlFacet>,
    scale: f64,
    encoding: &str,
) -> Result<ParsedStlMesh, StlImportError> {
    let mut unique = BTreeMap::<[u64; 3], [f64; 3]>::new();
    let mut scaled_facets = Vec::with_capacity(facets.len());
    let mut normal_mismatches = 0_u32;
    for (vertices, normal) in facets {
        let mut scaled = [[0.0; 3]; 3];
        for (destination, source_vertex) in scaled.iter_mut().zip(vertices) {
            for axis in 0..3 {
                let value = source_vertex[axis] * scale;
                if !value.is_finite() || value.abs() > MAX_STL_ABS_MM {
                    return Err(StlImportError::CoordinateOutOfRange);
                }
                destination[axis] = if value == 0.0 { 0.0 } else { value };
            }
            unique.insert(destination.map(f64::to_bits), *destination);
            if unique.len() > MAX_STL_VERTICES {
                return Err(StlImportError::TooManyVertices);
            }
        }
        if facet_normal_disagrees(scaled, normal) {
            normal_mismatches = normal_mismatches.saturating_add(1);
        }
        scaled_facets.push(scaled);
    }
    if unique.len() > MAX_STL_VERTICES {
        return Err(StlImportError::TooManyVertices);
    }
    let vertices_mm = unique.values().copied().collect::<Vec<_>>();
    let indices = unique
        .keys()
        .enumerate()
        .map(|(index, key)| (*key, index as u32))
        .collect::<BTreeMap<_, _>>();
    let mut triangles = Vec::with_capacity(scaled_facets.len());
    let mut seen = BTreeSet::new();
    for vertices in scaled_facets {
        let mut triangle = vertices.map(|vertex| indices[&vertex.map(f64::to_bits)]);
        if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
            return Err(StlImportError::DegenerateTriangle);
        }
        let first = vertices_mm[triangle[0] as usize];
        let second = vertices_mm[triangle[1] as usize];
        let third = vertices_mm[triangle[2] as usize];
        if squared_cross(first, second, third) <= STL_AREA_EPSILON {
            return Err(StlImportError::DegenerateTriangle);
        }
        let mut unordered = triangle;
        unordered.sort_unstable();
        if !seen.insert(unordered) {
            return Err(StlImportError::DuplicateTriangle);
        }
        let rotations = [
            triangle,
            [triangle[1], triangle[2], triangle[0]],
            [triangle[2], triangle[0], triangle[1]],
        ];
        triangle = *rotations.iter().min().expect("three rotations");
        triangles.push(triangle);
    }
    triangles.sort_unstable();
    validate_stl_topology(&vertices_mm, &triangles)?;

    let mut diagnostics = vec![
        ImportDiagnostic::new(
            ImportDiagnosticSeverity::Info,
            format!("stl.{encoding}"),
            None,
            1,
        )
        .expect("built-in STL diagnostic is valid"),
    ];
    if normal_mismatches > 0 {
        diagnostics.push(
            ImportDiagnostic::new(
                ImportDiagnosticSeverity::Warning,
                "stl.facet-normal-mismatch",
                None,
                normal_mismatches,
            )
            .expect("built-in STL diagnostic is valid"),
        );
    }
    diagnostics.sort();
    Ok(ParsedStlMesh {
        vertices_mm,
        triangles,
        diagnostics,
    })
}

fn validate_stl_topology(
    vertices: &[[f64; 3]],
    triangles: &[[u32; 3]],
) -> Result<(), StlImportError> {
    let mut edges = BTreeMap::<(u32, u32), Vec<(usize, bool)>>::new();
    let mut incident = vec![BTreeSet::new(); vertices.len()];
    let origin = vertices[0];
    let mut volume = 0.0;
    let mut compensation = 0.0;
    for (triangle_index, [a, b, c]) in triangles.iter().copied().enumerate() {
        for vertex in [a, b, c] {
            incident[vertex as usize].insert(triangle_index);
        }
        let shifted = [a, b, c].map(|index| {
            let point = vertices[index as usize];
            [
                point[0] - origin[0],
                point[1] - origin[1],
                point[2] - origin[2],
            ]
        });
        let term = shifted[0][0] * (shifted[1][1] * shifted[2][2] - shifted[1][2] * shifted[2][1])
            + shifted[0][1] * (shifted[1][2] * shifted[2][0] - shifted[1][0] * shifted[2][2])
            + shifted[0][2] * (shifted[1][0] * shifted[2][1] - shifted[1][1] * shifted[2][0]);
        let corrected = term - compensation;
        let next = volume + corrected;
        compensation = (next - volume) - corrected;
        volume = next;
        for (from, to) in [(a, b), (b, c), (c, a)] {
            edges
                .entry((from.min(to), from.max(to)))
                .or_default()
                .push((triangle_index, from < to));
        }
    }
    if edges.values().any(|uses| uses.len() != 2) {
        return Err(StlImportError::NonManifoldEdge);
    }
    if edges.values().any(|uses| uses[0].1 == uses[1].1) {
        return Err(StlImportError::InconsistentOrientation);
    }
    let mut adjacency = vec![Vec::new(); triangles.len()];
    let mut vertex_adjacency = vec![BTreeMap::<usize, BTreeSet<usize>>::new(); vertices.len()];
    for ((first, second), uses) in &edges {
        adjacency[uses[0].0].push(uses[1].0);
        adjacency[uses[1].0].push(uses[0].0);
        for vertex in [*first, *second] {
            vertex_adjacency[vertex as usize]
                .entry(uses[0].0)
                .or_default()
                .insert(uses[1].0);
            vertex_adjacency[vertex as usize]
                .entry(uses[1].0)
                .or_default()
                .insert(uses[0].0);
        }
    }
    for (vertex, faces) in incident.iter().enumerate() {
        let Some(start) = faces.first().copied() else {
            return Err(StlImportError::NonManifoldEdge);
        };
        if faces.iter().any(|face| {
            vertex_adjacency[vertex]
                .get(face)
                .is_none_or(|neighbours| neighbours.len() != 2)
        }) {
            return Err(StlImportError::NonManifoldEdge);
        }
        let mut fan = BTreeSet::new();
        let mut pending = vec![start];
        while let Some(face) = pending.pop() {
            if fan.insert(face) {
                pending.extend(vertex_adjacency[vertex][&face].iter().copied());
            }
        }
        if &fan != faces {
            return Err(StlImportError::NonManifoldEdge);
        }
    }
    let mut visited = BTreeSet::new();
    let mut pending = VecDeque::from([0_usize]);
    while let Some(index) = pending.pop_front() {
        if visited.insert(index) {
            pending.extend(adjacency[index].iter().copied());
        }
    }
    if visited.len() != triangles.len() {
        return Err(StlImportError::DisconnectedShells);
    }
    if volume <= STL_VOLUME_EPSILON {
        return Err(StlImportError::NonPositiveVolume);
    }
    Ok(())
}

fn squared_cross(first: [f64; 3], second: [f64; 3], third: [f64; 3]) -> f64 {
    let a = [
        second[0] - first[0],
        second[1] - first[1],
        second[2] - first[2],
    ];
    let b = [
        third[0] - first[0],
        third[1] - first[1],
        third[2] - first[2],
    ];
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    cross.into_iter().map(|value| value * value).sum()
}

fn facet_normal_disagrees(vertices: [[f64; 3]; 3], normal: [f64; 3]) -> bool {
    let a = [
        vertices[1][0] - vertices[0][0],
        vertices[1][1] - vertices[0][1],
        vertices[1][2] - vertices[0][2],
    ];
    let b = [
        vertices[2][0] - vertices[0][0],
        vertices[2][1] - vertices[0][1],
        vertices[2][2] - vertices[0][2],
    ];
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let normal_length = normal.into_iter().map(|value| value * value).sum::<f64>();
    normal_length > 0.0
        && cross
            .into_iter()
            .zip(normal)
            .map(|(left, right)| left * right)
            .sum::<f64>()
            <= 0.0
}
