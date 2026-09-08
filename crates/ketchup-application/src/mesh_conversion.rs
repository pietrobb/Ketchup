use std::fmt;

use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentId, DocumentStore, FeatureId,
    FeatureKind, MeshBodySpec, Snapshot, Transform,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError};
use ketchup_core::exact_product::ExactBRepGraphPackage;
use ketchup_core::mesh_recognition::{
    CylinderRecognition, MeshRecognition, MeshRecognitionCandidate, MeshRecognitionResiduals,
    recognize_mesh_body,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, SketchEntity, SketchEntityId, SketchSpec,
    WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};

const MAX_DISTANCE_COMPARISONS: usize = 4_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum MeshConversionError {
    SourceNotFound,
    SourceIsNotSoleMeshBody,
    NoMatch(String),
    Ambiguous(String),
    IdentifierOverflow,
    InvalidCandidate,
    InvalidBatch(String),
    ExactGraph(String),
    ExactVerificationRequired,
    ExactVerificationMismatch,
    VerificationResourceLimit,
    Stale,
}

impl fmt::Display for MeshConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotFound => formatter.write_str("source mesh feature was not found"),
            Self::SourceIsNotSoleMeshBody => {
                formatter.write_str("source definition is not a sole mesh body")
            }
            Self::NoMatch(reason) => write!(formatter, "mesh was not recognized: {reason}"),
            Self::Ambiguous(reason) => write!(formatter, "mesh recognition is ambiguous: {reason}"),
            Self::IdentifierOverflow => {
                formatter.write_str("feature identifier space is exhausted")
            }
            Self::InvalidCandidate => formatter.write_str("recognized exact candidate is invalid"),
            Self::InvalidBatch(reason) => {
                write!(formatter, "conversion batch is invalid: {reason}")
            }
            Self::ExactGraph(reason) => {
                write!(formatter, "exact conversion graph is invalid: {reason}")
            }
            Self::ExactVerificationRequired => {
                formatter.write_str("exact worker verification is required before conversion")
            }
            Self::ExactVerificationMismatch => {
                formatter.write_str("exact result does not match the recognized source mesh")
            }
            Self::VerificationResourceLimit => {
                formatter.write_str("mesh is too large for bounded exact conversion verification")
            }
            Self::Stale => formatter.write_str("mesh conversion plan is stale"),
        }
    }
}

impl std::error::Error for MeshConversionError {}

#[derive(Clone)]
pub struct MeshConversionPlan {
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    mutation_epoch: u64,
    source_definition_id: DefinitionId,
    source_feature_id: FeatureId,
    source_mesh: MeshBodySpec,
    tolerance_mm: f64,
    candidate: MeshRecognitionCandidate,
    residuals: MeshRecognitionResiduals,
    batch: CommandBatch,
    preview: Snapshot,
    graph: ExactBRepGraph,
}

impl MeshConversionPlan {
    #[must_use]
    pub const fn source_definition_id(&self) -> DefinitionId {
        self.source_definition_id
    }

    #[must_use]
    pub const fn source_feature_id(&self) -> FeatureId {
        self.source_feature_id
    }

    #[must_use]
    pub const fn candidate(&self) -> &MeshRecognitionCandidate {
        &self.candidate
    }

    #[must_use]
    pub const fn residuals(&self) -> MeshRecognitionResiduals {
        self.residuals
    }

    #[must_use]
    pub const fn tolerance_mm(&self) -> f64 {
        self.tolerance_mm
    }

    #[must_use]
    pub const fn graph(&self) -> &ExactBRepGraph {
        &self.graph
    }

    #[must_use]
    pub fn preview(&self) -> &Snapshot {
        &self.preview
    }

    #[must_use]
    pub fn batch_digest(&self) -> String {
        self.batch.digest()
    }
}

#[derive(Clone, Debug)]
pub struct MeshConversionVerification {
    graph_digest: String,
    result_fingerprint: String,
    max_source_to_exact_mm: f64,
    max_exact_to_source_mm: f64,
    package: ExactBRepGraphPackage,
}

impl MeshConversionVerification {
    #[must_use]
    pub const fn max_source_to_exact_mm(&self) -> f64 {
        self.max_source_to_exact_mm
    }

    #[must_use]
    pub const fn max_exact_to_source_mm(&self) -> f64 {
        self.max_exact_to_source_mm
    }

    #[must_use]
    pub const fn package(&self) -> &ExactBRepGraphPackage {
        &self.package
    }
}

pub fn prepare_mesh_conversion(
    document: &DocumentStore,
    source_feature_id: FeatureId,
    tolerance_mm: f64,
) -> Result<MeshConversionPlan, MeshConversionError> {
    let snapshot = document.current();
    let source = snapshot
        .feature(source_feature_id)
        .ok_or(MeshConversionError::SourceNotFound)?;
    let definition = snapshot
        .definition(source.definition_id())
        .ok_or(MeshConversionError::SourceNotFound)?;
    let FeatureKind::MeshBody(source_mesh) = source.kind() else {
        return Err(MeshConversionError::SourceIsNotSoleMeshBody);
    };
    if definition.feature_ids() != [source_feature_id] {
        return Err(MeshConversionError::SourceIsNotSoleMeshBody);
    }
    let (candidate, residuals) = match recognize_mesh_body(source_mesh, tolerance_mm) {
        MeshRecognition::Candidate {
            candidate,
            residuals,
        } => (candidate, residuals),
        MeshRecognition::NoMatch { reason } => return Err(MeshConversionError::NoMatch(reason)),
        MeshRecognition::Ambiguous { reason, .. } => {
            return Err(MeshConversionError::Ambiguous(reason));
        }
    };
    let next_feature = snapshot
        .features()
        .map(|feature| feature.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(MeshConversionError::IdentifierOverflow)?;
    let extrusion_id = FeatureId(next_feature);
    let transform_id = FeatureId(
        next_feature
            .checked_add(1)
            .ok_or(MeshConversionError::IdentifierOverflow)?,
    );
    let chain = candidate_feature_chain(
        source.definition_id(),
        source_feature_id,
        extrusion_id,
        transform_id,
        &candidate,
    )?;
    let producer_feature_id = chain.producer_feature_id;
    let batch = CommandBatch::new(chain.commands);
    let preview = document
        .preview_batch(&batch)
        .map_err(|error| MeshConversionError::InvalidBatch(error.to_string()))?;
    let graph =
        ExactBRepGraph::from_snapshot(&preview, source.definition_id(), producer_feature_id)
            .map_err(exact_graph_error)?;
    Ok(MeshConversionPlan {
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        mutation_epoch: document.mutation_epoch(),
        source_definition_id: source.definition_id(),
        source_feature_id,
        source_mesh: source_mesh.clone(),
        tolerance_mm,
        candidate,
        residuals,
        batch,
        preview,
        graph,
    })
}

pub fn verify_mesh_conversion(
    plan: &MeshConversionPlan,
    package: ExactBRepGraphPackage,
) -> Result<MeshConversionVerification, MeshConversionError> {
    if package.graph != plan.graph
        || package.identity.document_id != plan.preview.document_id()
        || package.identity.source_revision != plan.preview.revision_id()
        || package.identity.source_digest != plan.preview.canonical_digest()
        || package.identity.definition_id != plan.source_definition_id
        || package.identity.producer_feature_id != FeatureId(plan.graph.producer_feature_id)
    {
        return Err(MeshConversionError::ExactVerificationMismatch);
    }
    let source_triangles = plan.source_mesh.triangles.len();
    let exact_triangles = package.triangles.len();
    if source_triangles == 0
        || exact_triangles == 0
        || source_triangles
            .checked_mul(exact_triangles)
            .is_none_or(|count| count > MAX_DISTANCE_COMPARISONS)
    {
        return Err(MeshConversionError::VerificationResourceLimit);
    }
    let source_vertices = plan.source_mesh.vertices_mm.as_slice();
    let source_faces = plan.source_mesh.triangles.as_slice();
    let exact_vertices = package
        .vertices
        .iter()
        .map(|vertex| vertex.position_mm)
        .collect::<Vec<_>>();
    let exact_faces = package
        .triangles
        .iter()
        .map(|triangle| triangle.vertex_indices)
        .collect::<Vec<_>>();
    let source_to_exact = directed_mesh_sample_distance(
        source_vertices,
        source_faces,
        &exact_vertices,
        &exact_faces,
    )?;
    let exact_to_source = directed_mesh_sample_distance(
        &exact_vertices,
        &exact_faces,
        source_vertices,
        source_faces,
    )?;
    if source_to_exact > plan.tolerance_mm || exact_to_source > plan.tolerance_mm {
        return Err(MeshConversionError::ExactVerificationMismatch);
    }
    Ok(MeshConversionVerification {
        graph_digest: package.graph.graph_digest.clone(),
        result_fingerprint: package.identity.result_fingerprint.clone(),
        max_source_to_exact_mm: source_to_exact,
        max_exact_to_source_mm: exact_to_source,
        package,
    })
}

pub fn commit_mesh_conversion(
    document: &mut DocumentStore,
    plan: &MeshConversionPlan,
    verification: MeshConversionVerification,
) -> Result<ExactBRepGraphPackage, MeshConversionError> {
    let current = document.current();
    if document.mutation_epoch() != plan.mutation_epoch
        || current.document_id() != plan.document_id
        || current.revision_id() != plan.source_revision
        || current.canonical_digest() != plan.source_digest
        || verification.graph_digest != plan.graph.graph_digest
        || verification.result_fingerprint != verification.package.identity.result_fingerprint
    {
        return Err(MeshConversionError::Stale);
    }
    let current_mesh = current
        .feature(plan.source_feature_id)
        .and_then(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh) => Some(mesh),
            _ => None,
        })
        .ok_or(MeshConversionError::Stale)?;
    if current_mesh != &plan.source_mesh {
        return Err(MeshConversionError::Stale);
    }
    let repeated = prepare_mesh_conversion(document, plan.source_feature_id, plan.tolerance_mm)?;
    if repeated.batch.digest() != plan.batch.digest()
        || repeated.graph.graph_digest != plan.graph.graph_digest
        || repeated.candidate != plan.candidate
        || repeated.residuals != plan.residuals
    {
        return Err(MeshConversionError::Stale);
    }
    document
        .apply_batch(&plan.batch)
        .map_err(|error| MeshConversionError::InvalidBatch(error.to_string()))?;
    let committed = document.current();
    if committed.canonical_digest() != plan.preview.canonical_digest()
        || committed.revision_id() != plan.preview.revision_id()
    {
        return Err(MeshConversionError::Stale);
    }
    Ok(verification.package)
}

struct CandidateFeatureChain {
    commands: Vec<CanonicalCommand>,
    producer_feature_id: FeatureId,
}

fn candidate_feature_chain(
    definition_id: DefinitionId,
    profile_id: FeatureId,
    extrusion_id: FeatureId,
    transform_id: FeatureId,
    candidate: &MeshRecognitionCandidate,
) -> Result<CandidateFeatureChain, MeshConversionError> {
    if let MeshRecognitionCandidate::Cylinder(cylinder) = candidate {
        return cylinder_feature_chain(
            definition_id,
            profile_id,
            extrusion_id,
            transform_id,
            cylinder,
        );
    }
    let (profile, height_mm, origin, basis_u, mut basis_v, axis) = match candidate {
        MeshRecognitionCandidate::Box(value) => {
            let [width, depth, height] = value.dimensions_mm;
            (
                FeatureKind::Profile {
                    points_mm: vec![
                        [-width * 0.5, -depth * 0.5],
                        [width * 0.5, -depth * 0.5],
                        [width * 0.5, depth * 0.5],
                        [-width * 0.5, depth * 0.5],
                    ],
                },
                height,
                subtract_3d(value.center_mm, scale_3d(value.axes[2], height * 0.5)),
                value.axes[0],
                value.axes[1],
                value.axes[2],
            )
        }
        MeshRecognitionCandidate::LinearExtrusion(value) => (
            FeatureKind::Profile {
                points_mm: value.profile_mm.clone(),
            },
            value.height_mm,
            value.base_origin_mm,
            value.profile_basis[0],
            value.profile_basis[1],
            value.axis,
        ),
        MeshRecognitionCandidate::Cylinder(_) => {
            unreachable!("cylinder candidates return before generic extrusion planning")
        }
    };
    let mut profile = profile;
    if determinant(basis_u, basis_v, axis) < 0.0 {
        basis_v = scale_3d(basis_v, -1.0);
        reflect_profile_y(&mut profile);
    }
    ensure_counter_clockwise_profile(&mut profile);
    if !height_mm.is_finite() || height_mm <= 0.0 {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let transform = Transform::from_matrix([
        basis_u[0], basis_v[0], axis[0], origin[0], basis_u[1], basis_v[1], axis[1], origin[1],
        basis_u[2], basis_v[2], axis[2], origin[2], 0.0, 0.0, 0.0, 1.0,
    ])
    .map_err(|_| MeshConversionError::InvalidCandidate)?;
    if transform.rigid_inverse().is_none() {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let mut commands = vec![
        CanonicalCommand::DeleteFeature { id: profile_id },
        CanonicalCommand::CreateFeature {
            id: profile_id,
            definition_id,
            name: "Recognized profile".to_owned(),
            kind: profile,
        },
        CanonicalCommand::CreateFeature {
            id: extrusion_id,
            definition_id,
            name: "Recognized extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: profile_id,
                height: Dimension::new(format!("{height_mm:.17}"), height_mm)
                    .map_err(|_| MeshConversionError::InvalidCandidate)?,
            },
        },
    ];
    let producer_feature_id = if transform == Transform::identity() {
        extrusion_id
    } else {
        commands.push(CanonicalCommand::CreateFeature {
            id: transform_id,
            definition_id,
            name: "Recognized placement".to_owned(),
            kind: FeatureKind::RigidTransform {
                target: extrusion_id,
                transform,
            },
        });
        transform_id
    };
    Ok(CandidateFeatureChain {
        commands,
        producer_feature_id,
    })
}

fn cylinder_feature_chain(
    definition_id: DefinitionId,
    workplane_id: FeatureId,
    sketch_id: FeatureId,
    pad_id: FeatureId,
    cylinder: &CylinderRecognition,
) -> Result<CandidateFeatureChain, MeshConversionError> {
    if !cylinder.radius_mm.is_finite()
        || cylinder.radius_mm <= 0.0
        || !cylinder.height_mm.is_finite()
        || cylinder.height_mm <= 0.0
    {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let [basis_u, basis_v] = perpendicular_basis(cylinder.axis);
    let origin = subtract_3d(
        cylinder.center_mm,
        scale_3d(cylinder.axis, cylinder.height_mm * 0.5),
    );
    let frame = WorkplaneFrame::from_axes(origin, basis_u, basis_v)
        .map_err(|_| MeshConversionError::InvalidCandidate)?;
    let sketch = SketchSpec {
        workplane: workplane_id,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm: cylinder.radius_mm,
        }],
        constraints: Vec::new(),
    };
    let region = sketch
        .solved_regions()
        .map_err(|_| MeshConversionError::InvalidCandidate)?
        .into_iter()
        .next()
        .ok_or(MeshConversionError::InvalidCandidate)?
        .id;
    Ok(CandidateFeatureChain {
        commands: vec![
            CanonicalCommand::DeleteFeature { id: workplane_id },
            CanonicalCommand::CreateFeature {
                id: workplane_id,
                definition_id,
                name: "Recognized cylinder plane".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id,
                name: "Recognized circle".to_owned(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad_id,
                definition_id,
                name: "Recognized cylinder".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(
                        Dimension::new(format!("{:.17}", cylinder.height_mm), cylinder.height_mm)
                            .map_err(|_| MeshConversionError::InvalidCandidate)?,
                    ),
                }),
            },
        ],
        producer_feature_id: pad_id,
    })
}

fn reflect_profile_y(profile: &mut FeatureKind) {
    if let FeatureKind::Profile { points_mm } = profile {
        for point in points_mm.iter_mut() {
            point[1] = -point[1];
        }
        points_mm.reverse();
    }
}

fn ensure_counter_clockwise_profile(profile: &mut FeatureKind) {
    let FeatureKind::Profile { points_mm } = profile else {
        return;
    };
    let twice_area = points_mm
        .iter()
        .zip(points_mm.iter().cycle().skip(1))
        .take(points_mm.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>();
    if twice_area < 0.0 {
        points_mm.reverse();
    }
}

fn perpendicular_basis(axis: [f64; 3]) -> [[f64; 3]; 2] {
    if axis[0].abs() > 0.5 {
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    } else if axis[1].abs() > 0.5 {
        [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0]]
    } else {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    }
}

fn directed_mesh_sample_distance(
    sample_vertices: &[[f64; 3]],
    sample_faces: &[[u32; 3]],
    target_vertices: &[[f64; 3]],
    target_faces: &[[u32; 3]],
) -> Result<f64, MeshConversionError> {
    let mut maximum: f64 = 0.0;
    for face in sample_faces {
        let [a, b, c] = face.map(|index| sample_vertices[index as usize]);
        let samples = [
            a,
            b,
            c,
            midpoint(a, b),
            midpoint(b, c),
            midpoint(c, a),
            scale_3d(add_3d(add_3d(a, b), c), 1.0 / 3.0),
        ];
        for point in samples {
            let distance = target_faces
                .iter()
                .map(|triangle| {
                    let [x, y, z] = triangle.map(|index| target_vertices[index as usize]);
                    point_triangle_distance(point, x, y, z)
                })
                .min_by(f64::total_cmp)
                .ok_or(MeshConversionError::ExactVerificationMismatch)?;
            maximum = maximum.max(distance);
        }
    }
    Ok(maximum)
}

fn point_triangle_distance(point: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let ab = subtract_3d(b, a);
    let ac = subtract_3d(c, a);
    let ap = subtract_3d(point, a);
    let d1 = dot_3d(ab, ap);
    let d2 = dot_3d(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return length_3d(ap);
    }
    let bp = subtract_3d(point, b);
    let d3 = dot_3d(ab, bp);
    let d4 = dot_3d(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return length_3d(bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return length_3d(subtract_3d(point, add_3d(a, scale_3d(ab, v))));
    }
    let cp = subtract_3d(point, c);
    let d5 = dot_3d(ab, cp);
    let d6 = dot_3d(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return length_3d(cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return length_3d(subtract_3d(point, add_3d(a, scale_3d(ac, w))));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let edge = subtract_3d(c, b);
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return length_3d(subtract_3d(point, add_3d(b, scale_3d(edge, w))));
    }
    let denominator = 1.0 / (va + vb + vc);
    let v = vb * denominator;
    let w = vc * denominator;
    length_3d(subtract_3d(
        point,
        add_3d(a, add_3d(scale_3d(ab, v), scale_3d(ac, w))),
    ))
}

fn exact_graph_error(error: ExactBRepGraphError) -> MeshConversionError {
    MeshConversionError::ExactGraph(error.to_string())
}

fn midpoint(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    scale_3d(add_3d(left, right), 0.5)
}

fn add_3d(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract_3d(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale_3d(value: [f64; 3], scale: f64) -> [f64; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

fn dot_3d(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn length_3d(value: [f64; 3]) -> f64 {
    dot_3d(value, value).sqrt()
}

fn determinant(u: [f64; 3], v: [f64; 3], w: [f64; 3]) -> f64 {
    u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
        + u[2] * (v[0] * w[1] - v[1] * w[0])
}
