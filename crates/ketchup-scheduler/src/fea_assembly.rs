//! Binds the exact volume meshes of several occurrences into one FEA model:
//! each part placed where the assembly puts it, and faces of different parts
//! joined rigidly or by springs.
//!
//! A joint ties every node of each joined face rigidly to a reference node at
//! the first face's area centroid, so a joined face stays plane and moves as
//! one, the way a bolted or bonded joint is often modelled. A rigid joint
//! shares one reference node; a spring joint gives each face its own and puts
//! the springs between them. A node a constraint holds stays held and is left
//! out of the joint.

use crate::{ExactBRepVolumeMeshPackage, ExactFeaFaceTraction, ExactFeaSetupError};
use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaCoupling, FeaCouplingKind, FeaElement, FeaElementKind,
    FeaLoad, FeaMaterial, FeaModel, FeaNode,
};
use ketchup_geometry::linalg::{add, cross, length, scale, sub};
use ketchup_model::document::{InstancePath, Snapshot};
use ketchup_model::tolerance::NEGLIGIBLE;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaAssemblyPart {
    pub instance_path: InstancePath,
    pub material: FeaMaterial,
    /// Faces held in place; a part held only by its joints names none.
    pub constrained_face_ordinals: Vec<u32>,
    /// Face loads in the part's own axes.
    pub face_tractions: Vec<ExactFeaFaceTraction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactFeaJointFace {
    /// The part's index in the setup's parts.
    pub part: usize,
    pub face_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaJoint {
    pub faces: [ExactFeaJointFace; 2],
    pub kind: FeaCouplingKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaAssemblySetup {
    pub case_id: String,
    pub parts: Vec<ExactFeaAssemblyPart>,
    pub joints: Vec<ExactFeaJoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyBoundFeaPart {
    pub instance_path: InstancePath,
    pub graph_digest: String,
    pub mesh_fingerprint: String,
    /// The part's mesh nodes are the model's nodes `first_node..first_node + node_count`.
    pub first_node: usize,
    pub node_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyBoundFeaModel {
    pub source_revision: u64,
    pub source_digest: String,
    pub parts: Vec<AssemblyBoundFeaPart>,
    /// The reference node of each joint's first face, after every part's nodes.
    pub joint_nodes: Vec<usize>,
    pub model: FeaModel,
}

/// One placed part: the boundary triangles of each face, in model node numbers.
struct PlacedPart {
    faces: BTreeMap<u32, Vec<[usize; 3]>>,
}

impl PlacedPart {
    fn face(&self, ordinal: u32) -> Option<&[[usize; 3]]> {
        self.faces.get(&ordinal).map(Vec::as_slice)
    }

    fn face_nodes(&self, ordinal: u32) -> BTreeSet<usize> {
        self.faces
            .get(&ordinal)
            .into_iter()
            .flatten()
            .flatten()
            .copied()
            .collect()
    }
}

/// Builds one model from `packages[i]`, the exact volume mesh of
/// `setup.parts[i]`'s definition.
pub fn assembly_bound_model(
    snapshot: &Snapshot,
    packages: &[&ExactBRepVolumeMeshPackage],
    setup: &ExactFeaAssemblySetup,
) -> Result<AssemblyBoundFeaModel, ExactFeaSetupError> {
    if setup.case_id.trim().is_empty()
        || setup.parts.is_empty()
        || packages.len() != setup.parts.len()
    {
        return Err(ExactFeaSetupError::IncompleteSetup);
    }
    let mut nodes = Vec::new();
    let mut elements = Vec::new();
    let mut materials = Vec::<FeaMaterial>::new();
    let mut held = BTreeSet::new();
    let mut loads = Vec::new();
    let mut placed = Vec::with_capacity(packages.len());
    let mut bound = Vec::with_capacity(packages.len());
    for (package, part) in packages.iter().zip(&setup.parts) {
        if !package.is_current(snapshot) {
            return Err(ExactFeaSetupError::StaleGeometry);
        }
        let resolved = snapshot
            .resolve_instance_path(&part.instance_path)
            .map_err(ExactFeaSetupError::InvalidInstancePath)?;
        if resolved.definition_id != package.source.identity.definition_id {
            return Err(ExactFeaSetupError::OccurrenceMismatch);
        }
        let placement = resolved.world_transform;
        if !placement.is_rigid() {
            return Err(ExactFeaSetupError::NonRigidPlacement);
        }
        match materials
            .iter()
            .find(|material| material.id == part.material.id)
        {
            Some(material) if *material != part.material => {
                return Err(ExactFeaSetupError::ConflictingMaterial);
            }
            Some(_) => {}
            None => materials.push(part.material.clone()),
        }

        let first_node = nodes.len();
        let mesh = &package.mesh;
        let node = |index: u32| {
            usize::try_from(index)
                .ok()
                .filter(|index| *index < mesh.vertices_mm.len())
                .map(|index| first_node + index)
                .ok_or(ExactFeaSetupError::InvalidBoundarySelection)
        };
        let mut faces = BTreeMap::<u32, Vec<[usize; 3]>>::new();
        for triangle in &mesh.boundary_triangles {
            let [a, b, c] = triangle.vertex_indices;
            let corners = [node(a)?, node(b)?, node(c)?];
            match faces.get_mut(&triangle.face_ordinal) {
                Some(triangles) => triangles.push(corners),
                None => {
                    faces.insert(triangle.face_ordinal, vec![corners]);
                }
            }
        }
        for tetrahedron in &mesh.tetrahedra {
            let [a, b, c, d] = tetrahedron;
            elements.push(FeaElement {
                id: u64::try_from(elements.len()).map_or(u64::MAX, |id| id + 1),
                kind: FeaElementKind::LinearTetrahedron4 {
                    nodes: [node(*a)?, node(*b)?, node(*c)?, node(*d)?],
                    material_id: part.material.id,
                },
            });
        }
        nodes.extend(mesh.vertices_mm.iter().map(|position| FeaNode {
            position_mm: placement.transform_point(*position),
        }));
        let part_placed = PlacedPart { faces };

        let mut seen = BTreeSet::new();
        for ordinal in &part.constrained_face_ordinals {
            if !seen.insert(*ordinal) || part_placed.face(*ordinal).is_none() {
                return Err(ExactFeaSetupError::InvalidBoundarySelection);
            }
            held.extend(part_placed.face_nodes(*ordinal));
        }
        let mut seen = BTreeSet::new();
        for traction in &part.face_tractions {
            let triangles = part_placed
                .face(traction.face_ordinal)
                .filter(|_| seen.insert(traction.face_ordinal))
                .ok_or(ExactFeaSetupError::InvalidBoundarySelection)?;
            if traction
                .traction_local_n_per_mm2
                .iter()
                .any(|value| !value.is_finite())
            {
                return Err(ExactFeaSetupError::InvalidBoundarySelection);
            }
            let traction_n_per_mm2 = placement.transform_vector(traction.traction_local_n_per_mm2);
            loads.extend(triangles.iter().map(|nodes| FeaLoad::SurfaceTraction {
                nodes: *nodes,
                traction_n_per_mm2,
            }));
        }
        bound.push(AssemblyBoundFeaPart {
            instance_path: part.instance_path.clone(),
            graph_digest: mesh.graph_digest.clone(),
            mesh_fingerprint: mesh.mesh_fingerprint.clone(),
            first_node,
            node_count: mesh.vertices_mm.len(),
        });
        placed.push(part_placed);
    }
    if held.is_empty() || loads.is_empty() {
        return Err(ExactFeaSetupError::IncompleteSetup);
    }

    let (couplings, joint_nodes) = joints(&mut nodes, &placed, &held, &setup.joints)?;
    Ok(AssemblyBoundFeaModel {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        parts: bound,
        joint_nodes,
        model: FeaModel {
            schema: FEA_MODEL_SCHEMA_V1.to_owned(),
            case_id: setup.case_id.clone(),
            nodes,
            materials,
            elements,
            constraints: held.into_iter().map(FeaConstraint::held).collect(),
            loads,
            couplings,
        },
    })
}

/// The area centroid of a face's triangles.
fn centroid(nodes: &[FeaNode], triangles: &[[usize; 3]]) -> Option<[f64; 3]> {
    let mut area = 0.0;
    let mut moment = [0.0; 3];
    for triangle in triangles {
        let [a, b, c] = triangle.map(|node| nodes[node].position_mm);
        let weight = length(cross(sub(b, a), sub(c, a))) / 2.0;
        area += weight;
        moment = add(moment, scale(add(add(a, b), c), weight / 3.0));
    }
    (area > NEGLIGIBLE).then(|| scale(moment, 1.0 / area))
}

/// Adds each joint's reference nodes and returns the couplings with the
/// reference node of each joint's first face.
fn joints(
    nodes: &mut Vec<FeaNode>,
    placed: &[PlacedPart],
    held: &BTreeSet<usize>,
    joints: &[ExactFeaJoint],
) -> Result<(Vec<FeaCoupling>, Vec<usize>), ExactFeaSetupError> {
    let mut couplings = Vec::new();
    let mut references = Vec::with_capacity(joints.len());
    let mut joined = BTreeSet::new();
    for (index, joint) in joints.iter().enumerate() {
        let refused = ExactFeaSetupError::InvalidJoint { joint: index };
        let [first, second] = joint.faces;
        if first.part == second.part {
            return Err(refused);
        }
        let mut sides = Vec::with_capacity(2);
        for face in joint.faces {
            let part = placed.get(face.part).ok_or(refused.clone())?;
            if part.face(face.face_ordinal).is_none() {
                return Err(refused);
            }
            let side = part
                .face_nodes(face.face_ordinal)
                .into_iter()
                .filter(|node| !held.contains(node))
                .collect::<Vec<_>>();
            // A node can follow one joint only.
            if side.is_empty() || side.iter().any(|node| !joined.insert(*node)) {
                return Err(refused);
            }
            sides.push(side);
        }
        let first_face = placed[first.part]
            .face(first.face_ordinal)
            .ok_or(refused.clone())?;
        let place = centroid(nodes, first_face).ok_or(refused.clone())?;
        let reference = nodes.len();
        nodes.push(FeaNode { position_mm: place });
        references.push(reference);
        let second_reference = match joint.kind {
            FeaCouplingKind::Rigid => reference,
            FeaCouplingKind::Spring { .. } => {
                nodes.push(FeaNode { position_mm: place });
                couplings.push(FeaCoupling {
                    nodes: [reference, reference + 1],
                    kind: joint.kind,
                });
                reference + 1
            }
        };
        for (leader, side) in [reference, second_reference].into_iter().zip(sides) {
            couplings.extend(side.into_iter().map(|node| FeaCoupling {
                nodes: [leader, node],
                kind: FeaCouplingKind::Rigid,
            }));
        }
    }
    Ok((couplings, references))
}
