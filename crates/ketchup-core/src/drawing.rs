#![forbid(unsafe_code)]

use crate::assembly::{
    AssemblyReferenceHealth, AssemblySolveStatus, AssemblySolverPolicy, solve_rigid_assembly,
};
use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentId, DocumentStore, OccurrenceId,
    Proposal, ProposalPrepareError, Snapshot, Transform,
};
use crate::exact_product::{ExactBodyPackage, ExactResultRegistry};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub const ORTHOGRAPHIC_DRAWING_SCHEMA_V1: &str = "ketchup.orthographic-drawing.v1";
const VISIBILITY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingSheetId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrthographicViewKind {
    Front,
    Top,
    Right,
}

impl OrthographicViewKind {
    const ALL: [Self; 3] = [Self::Front, Self::Top, Self::Right];

    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Top => "top",
            Self::Right => "right",
        }
    }

    const fn axes(self) -> (usize, usize, usize) {
        match self {
            Self::Front => (0, 2, 1),
            Self::Top => (0, 1, 2),
            Self::Right => (1, 2, 0),
        }
    }

    fn face_is_visible(self, normal: [f64; 3], depth: usize) -> bool {
        match self {
            Self::Front => normal[depth] < -VISIBILITY_EPSILON,
            Self::Top | Self::Right => normal[depth] > VISIBILITY_EPSILON,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingSource {
    Definition(DefinitionId),
    RigidAssembly { occurrence_ids: Vec<OccurrenceId> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingSheet {
    schema: &'static str,
    id: DrawingSheetId,
    name: String,
    source: DrawingSource,
}

impl DrawingSheet {
    pub fn new(
        id: DrawingSheetId,
        name: impl Into<String>,
        source: DrawingSource,
    ) -> Result<Self, DrawingError> {
        let name = name.into();
        if id.0 == 0 || name.trim().is_empty() {
            return Err(DrawingError::InvalidSheet);
        }
        if let DrawingSource::RigidAssembly { occurrence_ids } = &source
            && (occurrence_ids.is_empty()
                || occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1]))
        {
            return Err(DrawingError::InvalidSheet);
        }
        Ok(Self {
            schema: ORTHOGRAPHIC_DRAWING_SCHEMA_V1,
            id,
            name,
            source,
        })
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    #[must_use]
    pub const fn id(&self) -> DrawingSheetId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn source(&self) -> &DrawingSource {
        &self.source
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedVisibleLine {
    pub stable_line_id: String,
    pub start_mm: [f64; 2],
    pub end_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrthographicView {
    pub kind: OrthographicViewKind,
    pub stable_view_id: String,
    pub bounds_mm: [[f64; 2]; 2],
    pub visible_lines: Vec<ProjectedVisibleLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrthographicDrawing {
    pub schema: &'static str,
    pub sheet_id: DrawingSheetId,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub stable_source_identity: String,
    pub result_digest: String,
    pub views: Vec<OrthographicView>,
}

impl OrthographicDrawing {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingError {
    InvalidSheet,
    SourceLost,
    SourceStale,
    SourceFailed,
    SourceAmbiguous,
    SourceNotRigid,
}

impl fmt::Display for DrawingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSheet => "drawing sheet identity or source is invalid",
            Self::SourceLost => "drawing source identity is lost",
            Self::SourceStale => "drawing source geometry is stale",
            Self::SourceFailed => "drawing source geometry failed or is unavailable",
            Self::SourceAmbiguous => "drawing source geometry is ambiguous",
            Self::SourceNotRigid => "drawing assembly source is not a resolved rigid component",
        })
    }
}

impl std::error::Error for DrawingError {}

#[derive(Debug)]
pub enum DrawingAuthoringError {
    Drawing(DrawingError),
    Proposal(ProposalPrepareError),
}

impl fmt::Display for DrawingAuthoringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drawing(error) => error.fmt(formatter),
            Self::Proposal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DrawingAuthoringError {}

impl From<DrawingError> for DrawingAuthoringError {
    fn from(error: DrawingError) -> Self {
        Self::Drawing(error)
    }
}

impl From<ProposalPrepareError> for DrawingAuthoringError {
    fn from(error: ProposalPrepareError) -> Self {
        Self::Proposal(error)
    }
}

pub fn prepare_create_drawing_sheet(
    document: &DocumentStore,
    results: &ExactResultRegistry,
    sheet: DrawingSheet,
) -> Result<(Proposal, OrthographicDrawing), DrawingAuthoringError> {
    project_orthographic_drawing(&document.current(), results, &sheet)?;
    let batch = CommandBatch::new(vec![CanonicalCommand::CreateDrawingSheet(sheet.clone())]);
    let candidate = document
        .preview_batch(&batch)
        .map_err(ProposalPrepareError::Canonical)?;
    let candidate_results = ExactResultRegistry::carried_forward(&candidate, results);
    let drawing = project_orthographic_drawing(&candidate, &candidate_results, &sheet)?;
    let proposal = document.prepare_proposal(batch)?;
    Ok((proposal, drawing))
}

pub fn prepare_edit_drawing_sheet(
    document: &DocumentStore,
    results: &ExactResultRegistry,
    sheet: DrawingSheet,
) -> Result<(Proposal, OrthographicDrawing), DrawingAuthoringError> {
    project_orthographic_drawing(&document.current(), results, &sheet)?;
    let batch = CommandBatch::new(vec![CanonicalCommand::UpdateDrawingSheet(sheet.clone())]);
    let candidate = document
        .preview_batch(&batch)
        .map_err(ProposalPrepareError::Canonical)?;
    let candidate_results = ExactResultRegistry::carried_forward(&candidate, results);
    let drawing = project_orthographic_drawing(&candidate, &candidate_results, &sheet)?;
    let proposal = document.prepare_proposal(batch)?;
    Ok((proposal, drawing))
}

pub fn prepare_delete_drawing_sheet(
    document: &DocumentStore,
    id: DrawingSheetId,
) -> Result<Proposal, ProposalPrepareError> {
    document.prepare_proposal(CommandBatch::new(vec![
        CanonicalCommand::DeleteDrawingSheet { id },
    ]))
}

pub fn project_orthographic_drawing(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    sheet: &DrawingSheet,
) -> Result<OrthographicDrawing, DrawingError> {
    validate_source(snapshot, sheet.source())?;
    let instances = source_instances(snapshot, results, sheet.source())?;
    let stable_source_identity = stable_source_identity(sheet.source(), &instances);
    let mut views = Vec::with_capacity(OrthographicViewKind::ALL.len());
    for kind in OrthographicViewKind::ALL {
        views.push(project_view(sheet.id(), kind, &instances)?);
    }
    let result_digest = drawing_result_digest(&stable_source_identity, &views);
    Ok(OrthographicDrawing {
        schema: ORTHOGRAPHIC_DRAWING_SCHEMA_V1,
        sheet_id: sheet.id(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        stable_source_identity,
        result_digest,
        views,
    })
}

pub(crate) fn validate_source(
    snapshot: &Snapshot,
    source: &DrawingSource,
) -> Result<(), DrawingError> {
    match source {
        DrawingSource::Definition(id) => {
            if snapshot.definition(*id).is_none() {
                return Err(DrawingError::SourceLost);
            }
        }
        DrawingSource::RigidAssembly { occurrence_ids } => {
            if occurrence_ids.is_empty()
                || occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1])
                || occurrence_ids
                    .iter()
                    .any(|id| snapshot.occurrence(*id).is_none())
            {
                return Err(DrawingError::SourceLost);
            }
            if !occurrence_ids
                .iter()
                .any(|id| snapshot.occurrence_is_grounded(*id))
            {
                return Err(DrawingError::SourceNotRigid);
            }
            let mut has_internal_mate = false;
            for mate in snapshot.assembly_mates() {
                let a_in = occurrence_ids.contains(&mate.endpoint_a().occurrence_id());
                let b_in = occurrence_ids.contains(&mate.endpoint_b().occurrence_id());
                if a_in != b_in
                    || ((a_in || b_in)
                        && (mate.endpoint_a().health() != AssemblyReferenceHealth::Resolved
                            || mate.endpoint_b().health() != AssemblyReferenceHealth::Resolved))
                {
                    return Err(DrawingError::SourceNotRigid);
                }
                has_internal_mate |= a_in && b_in;
            }
            if occurrence_ids
                .iter()
                .all(|id| snapshot.occurrence_is_grounded(*id))
            {
                return Ok(());
            }
            if !has_internal_mate {
                return Err(DrawingError::SourceNotRigid);
            }
            let solved = solve_rigid_assembly(snapshot, AssemblySolverPolicy::default())
                .map_err(|_| DrawingError::SourceNotRigid)?;
            if solved.status() != AssemblySolveStatus::FullyConstrained
                || !solved.conflicting_mate_ids().is_empty()
                || !solved.maximum_residual().is_finite()
                || occurrence_ids.iter().any(|id| {
                    solved
                        .occurrence(*id)
                        .is_none_or(|occurrence| occurrence.remaining_dof() != 0)
                })
            {
                return Err(DrawingError::SourceNotRigid);
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
struct DrawingInstance {
    token: String,
    transform: Transform,
    package: Arc<ExactBodyPackage>,
}

fn source_instances(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    source: &DrawingSource,
) -> Result<Vec<DrawingInstance>, DrawingError> {
    match source {
        DrawingSource::Definition(id) => Ok(vec![DrawingInstance {
            token: format!("definition-{}", id.0),
            transform: Transform::identity(),
            package: unique_current_package(snapshot, results, *id)?,
        }]),
        DrawingSource::RigidAssembly { occurrence_ids } => occurrence_ids
            .iter()
            .map(|id| {
                let occurrence = snapshot.occurrence(*id).ok_or(DrawingError::SourceLost)?;
                let transform = snapshot
                    .scene_query()
                    .into_iter()
                    .find(|candidate| {
                        candidate.instance_path.is_root() && candidate.occurrence_id == *id
                    })
                    .map(|candidate| candidate.transform)
                    .ok_or(DrawingError::SourceLost)?;
                Ok(DrawingInstance {
                    token: format!("occurrence-{}", id.0),
                    transform,
                    package: unique_current_package(snapshot, results, occurrence.definition_id())?,
                })
            })
            .collect(),
    }
}

fn unique_current_package(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    definition_id: DefinitionId,
) -> Result<Arc<ExactBodyPackage>, DrawingError> {
    let mut current = results
        .render_values(snapshot)
        .filter(|package| package.definition_id() == definition_id);
    let package = current.next();
    if current.next().is_some() {
        return Err(DrawingError::SourceAmbiguous);
    }
    if let Some(package) = package {
        return Ok(Arc::clone(package));
    }
    if results
        .values()
        .any(|package| package.definition_id() == definition_id)
    {
        Err(DrawingError::SourceStale)
    } else {
        Err(DrawingError::SourceFailed)
    }
}

fn stable_source_identity(source: &DrawingSource, instances: &[DrawingInstance]) -> String {
    let source_token = match source {
        DrawingSource::Definition(id) => format!("definition:{}", id.0),
        DrawingSource::RigidAssembly { occurrence_ids } => format!(
            "assembly:{}",
            occurrence_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    };
    let geometry = instances
        .iter()
        .map(|instance| {
            let key = instance.package.result_key();
            format!(
                "{}:{}:{}",
                instance.token, key.definition_id.0, key.producer_feature_id.0
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    format!("{source_token}/{geometry}")
}

#[derive(Clone)]
struct EdgeEvidence {
    start: [f64; 2],
    end: [f64; 2],
    normals: Vec<[f64; 3]>,
}

fn project_view(
    sheet_id: DrawingSheetId,
    kind: OrthographicViewKind,
    instances: &[DrawingInstance],
) -> Result<OrthographicView, DrawingError> {
    let (horizontal, vertical, depth) = kind.axes();
    let mut edges = BTreeMap::<String, EdgeEvidence>::new();
    for instance in instances {
        for triangle in instance.package.triangles() {
            let indices = triangle.vertex_indices;
            let points = indices.map(|index| {
                transform_point(
                    instance.transform,
                    instance.package.vertices()[index as usize].position_mm,
                )
            });
            let normal = cross(
                subtract(points[1], points[0]),
                subtract(points[2], points[0]),
            );
            if !kind.face_is_visible(normal, depth) {
                continue;
            }
            for (left, right) in [(0, 1), (1, 2), (2, 0)] {
                let (first, second) = if indices[left] <= indices[right] {
                    (indices[left], indices[right])
                } else {
                    (indices[right], indices[left])
                };
                let key = format!("{}:{first}:{second}", instance.token);
                let start = [points[left][horizontal], points[left][vertical]];
                let end = [points[right][horizontal], points[right][vertical]];
                edges
                    .entry(key)
                    .and_modify(|evidence| evidence.normals.push(normal))
                    .or_insert(EdgeEvidence {
                        start,
                        end,
                        normals: vec![normal],
                    });
            }
        }
    }
    let mut visible_lines = edges
        .into_iter()
        .filter(|(_, evidence)| {
            evidence.normals.len() == 1
                || evidence
                    .normals
                    .windows(2)
                    .any(|pair| length_squared(cross(pair[0], pair[1])) > VISIBILITY_EPSILON)
        })
        .map(|(edge, evidence)| ProjectedVisibleLine {
            stable_line_id: format!("sheet-{}/view-{}/{}", sheet_id.0, kind.stable_name(), edge),
            start_mm: evidence.start,
            end_mm: evidence.end,
        })
        .collect::<Vec<_>>();
    visible_lines.sort_by(|left, right| left.stable_line_id.cmp(&right.stable_line_id));
    if visible_lines.is_empty() {
        return Err(DrawingError::SourceFailed);
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for line in &visible_lines {
        for point in [line.start_mm, line.end_mm] {
            min[0] = min[0].min(point[0]);
            min[1] = min[1].min(point[1]);
            max[0] = max[0].max(point[0]);
            max[1] = max[1].max(point[1]);
        }
    }
    Ok(OrthographicView {
        kind,
        stable_view_id: format!("sheet-{}/view-{}", sheet_id.0, kind.stable_name()),
        bounds_mm: [min, max],
        visible_lines,
    })
}

fn drawing_result_digest(stable_source_identity: &str, views: &[OrthographicView]) -> String {
    let mut digest = Sha256::new();
    push_digest(&mut digest, ORTHOGRAPHIC_DRAWING_SCHEMA_V1.as_bytes());
    push_digest(&mut digest, stable_source_identity.as_bytes());
    for view in views {
        push_digest(&mut digest, view.kind.stable_name().as_bytes());
        for bound in view.bounds_mm.iter().flatten() {
            digest.update(bound.to_bits().to_le_bytes());
        }
        for line in &view.visible_lines {
            push_digest(&mut digest, line.stable_line_id.as_bytes());
            for coordinate in line.start_mm.into_iter().chain(line.end_mm) {
                digest.update(coordinate.to_bits().to_le_bytes());
            }
        }
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn push_digest(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

fn transform_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn length_squared(value: [f64; 3]) -> f64 {
    value[0] * value[0] + value[1] * value[1] + value[2] * value[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthographic_camera_facing_signs_are_consistent_with_view_axes() {
        assert!(OrthographicViewKind::Front.face_is_visible([0.0, -1.0, 0.0], 1));
        assert!(!OrthographicViewKind::Front.face_is_visible([0.0, 1.0, 0.0], 1));
        assert!(OrthographicViewKind::Top.face_is_visible([0.0, 0.0, 1.0], 2));
        assert!(OrthographicViewKind::Right.face_is_visible([1.0, 0.0, 0.0], 0));
    }
}
