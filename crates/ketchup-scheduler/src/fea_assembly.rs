//! Binds the exact volume meshes of several occurrences into one FEA model:
//! each part placed where the assembly puts it, and faces of different parts
//! bonded where they touch, or joined by a rigid or a spring connector.
//!
//! A bonded joint ties the two faces only where they lie on each other: each
//! node of one face that lies on the other moves with the point of the other
//! face's mesh it lies at, so two parts glued over a face bend as one piece,
//! and the parts of the faces off each other stay free (R1119og-FEA-800).
//! Faces that do not touch anywhere are refused. The finer face follows the
//! coarser, so none of its nodes stays free between the other's
//! (R1121ac-FEA-1000); a node of an edge two bonded faces share follows one
//! joint only (R1120ac-FEA-901), and when both faces tie as many nodes the
//! part bonded in more joints leads.
//!
//! A connector, the way a bolt is often modelled, ties every node of each
//! joined face rigidly to a reference node at the first face's area centroid,
//! so the face stays plane and moves as one: a rigid connector shares one
//! reference node; a spring connector gives each face its own and puts the
//! springs between them. It stiffens both faces, so it is only for a joint the
//! user names as one. A node a constraint holds stays held and is left out of
//! the joint.

use crate::{ExactBRepVolumeMeshPackage, ExactFeaFaceTraction, ExactFeaSetupError};
use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaCoupling, FeaCouplingKind, FeaElement, FeaElementKind,
    FeaLoad, FeaMaterial, FeaModel, FeaNode,
};
use ketchup_geometry::linalg::{add, cross, dot, length, scale, sub};
use ketchup_model::assembly::{
    AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, AssemblyReferenceHealth,
};
use ketchup_model::document::{InstancePath, Snapshot};
use ketchup_model::tolerance::{ACCUMULATED_ROUNDING, APPROXIMATION, NEGLIGIBLE};
use std::cmp::Ordering;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExactFeaJointKind {
    /// Glued where the faces lie on each other, nowhere else.
    Bonded,
    /// Both faces held plane and moving as one rigid body, as a bolt.
    RigidConnector,
    /// Both faces held plane, with springs between them, as a bolt.
    SpringConnector {
        translation_n_per_mm: [f64; 3],
        rotation_n_mm_per_rad: [f64; 3],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaJoint {
    pub faces: [ExactFeaJointFace; 2],
    pub kind: ExactFeaJointKind,
}

/// A face-to-face (coincident planar) mate of the document, joined at the two
/// faces it names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactFeaMateJoint {
    pub mate_id: AssemblyMateId,
    pub kind: ExactFeaJointKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaAssemblySetup {
    pub case_id: String,
    pub parts: Vec<ExactFeaAssemblyPart>,
    pub joints: Vec<ExactFeaJoint>,
    /// Joined after `joints`, so they number on from them.
    pub mate_joints: Vec<ExactFeaMateJoint>,
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
    /// The reference node of each connector's first face, after every part's
    /// nodes; none for a bonded joint.
    pub joint_nodes: Vec<Option<usize>>,
    /// Nodes of a bonded face lying on the other face that no tie holds to it:
    /// they lead a tie of another joint, or the other face's triangle under
    /// them has a node that follows one.
    pub untied_bonded_nodes: usize,
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

    let mut all_joints = setup.joints.clone();
    for mate in &setup.mate_joints {
        all_joints.push(ExactFeaJoint {
            faces: mate_faces(snapshot, packages, setup, mate.mate_id)?,
            kind: mate.kind,
        });
    }
    let Joined {
        couplings,
        references: joint_nodes,
        untied,
    } = joints(&mut nodes, &placed, &held, &all_joints)?;
    Ok(AssemblyBoundFeaModel {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        parts: bound,
        joint_nodes,
        untied_bonded_nodes: untied,
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

/// The faces a coincident planar mate names. Each end's part is the setup part
/// at the end's instance path, and its face the one face of that part's exact
/// evidence with the end's reference role and source element, facing the way
/// the end does.
fn mate_faces(
    snapshot: &Snapshot,
    packages: &[&ExactBRepVolumeMeshPackage],
    setup: &ExactFeaAssemblySetup,
    mate_id: AssemblyMateId,
) -> Result<[ExactFeaJointFace; 2], ExactFeaSetupError> {
    let refused = ExactFeaSetupError::InvalidMate { mate_id };
    let mate = snapshot
        .assembly_mate(mate_id)
        .filter(|mate| matches!(mate.kind(), AssemblyMateKind::CoincidentPlanar { .. }))
        .ok_or(refused.clone())?;
    let face = |end: &AssemblyMateEndpoint| {
        let attachment = end
            .planar_face_attachment()
            .filter(|_| end.health() == AssemblyReferenceHealth::Resolved)?;
        let part = setup
            .parts
            .iter()
            .position(|part| part.instance_path == *end.instance_path())?;
        let source = &packages.get(part)?.source;
        let reference = attachment.reference();
        if reference.definition_id != source.identity.definition_id
            || reference.producer_feature_id != source.identity.producer_feature_id
        {
            return None;
        }
        let mut named = source.face_evidence.iter().filter(|face| {
            face.semantic_role == reference.semantic_role
                && face.source_element_id == reference.source_element_id
        });
        let face = named.next().filter(|_| named.next().is_none())?;
        (1.0 - dot(face.unit_normal, attachment.local_unit_normal()) <= ACCUMULATED_ROUNDING)
            .then_some(ExactFeaJointFace {
                part,
                face_ordinal: face.face_ordinal,
            })
    };
    face(mate.endpoint_a())
        .zip(face(mate.endpoint_b()))
        .map(|(first, second)| [first, second])
        .ok_or(refused)
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

/// Where a node lies on a face's triangles: the triangle and the node's
/// barycentric weights in it, if the node lies on the face.
fn lies_on(
    nodes: &[FeaNode],
    triangles: &[[usize; 3]],
    point: [f64; 3],
) -> Option<([usize; 3], [f64; 3])> {
    let mut best: Option<(f64, [usize; 3], [f64; 3])> = None;
    for triangle in triangles {
        let [a, b, c] = triangle.map(|node| nodes[node].position_mm);
        let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(point, a));
        let normal = cross(ab, ac);
        let area2 = dot(normal, normal);
        let size = length(ab).max(length(ac)).max(1.0);
        if area2 <= NEGLIGIBLE * size.powi(4) {
            continue;
        }
        let gap = dot(ap, normal).abs() / area2.sqrt();
        if gap > APPROXIMATION * size {
            continue;
        }
        let v = dot(cross(ap, ac), normal) / area2;
        let w = dot(cross(ab, ap), normal) / area2;
        let weights = [1.0 - v - w, v, w];
        if weights.iter().any(|weight| *weight < -ACCUMULATED_ROUNDING) {
            continue;
        }
        if best.is_none_or(|(closest, ..)| gap < closest) {
            // Rounding outside an edge is put back on it.
            let clamped = weights.map(|weight| weight.max(0.0));
            let sum: f64 = clamped.iter().sum();
            best = Some((gap, *triangle, clamped.map(|weight| weight / sum)));
        }
    }
    best.map(|(_, triangle, weights)| (triangle, weights))
}

/// The nodes of `follower` that lie on `leader`, each with the triangle and
/// the weights it is tied by; none when no node does.
fn bond(
    nodes: &[FeaNode],
    leader: &[[usize; 3]],
    follower: &BTreeSet<usize>,
) -> Vec<(usize, [usize; 3], [f64; 3])> {
    follower
        .iter()
        .filter_map(|node| {
            let (triangle, weights) = lies_on(nodes, leader, nodes[*node].position_mm)?;
            Some((*node, triangle, weights))
        })
        .collect()
}

/// The couplings of all joints, the reference node of each connector's first
/// face, and the bonded nodes no tie holds.
struct Joined {
    couplings: Vec<FeaCoupling>,
    references: Vec<Option<usize>>,
    untied: usize,
}

/// Adds each connector's reference nodes and joins the faces.
fn joints(
    nodes: &mut Vec<FeaNode>,
    placed: &[PlacedPart],
    held: &BTreeSet<usize>,
    joints: &[ExactFeaJoint],
) -> Result<Joined, ExactFeaSetupError> {
    let mut couplings = Vec::new();
    let mut references = Vec::with_capacity(joints.len());
    let mut untied = 0;
    // Nodes that follow a joint, and nodes a tie follows: a node follows one
    // joint only, and a tie leads from nodes that follow nothing.
    let mut joined = BTreeSet::new();
    let mut tie_leaders = BTreeSet::new();
    for (index, joint) in joints.iter().enumerate() {
        let refused = ExactFeaSetupError::InvalidJoint { joint: index };
        let [first, second] = joint.faces;
        if first.part == second.part {
            return Err(refused);
        }
        // A face's triangles and its nodes no constraint holds.
        let side = |face: ExactFeaJointFace| {
            let part = placed.get(face.part)?;
            let free = part
                .face_nodes(face.face_ordinal)
                .into_iter()
                .filter(|node| !held.contains(node))
                .collect::<BTreeSet<_>>();
            Some((part.face(face.face_ordinal)?, free))
        };
        let (first_face, first_free) = side(first).ok_or(refused.clone())?;
        let (second_face, second_free) = side(second).ok_or(refused.clone())?;
        let (leaders, followers): (BTreeSet<usize>, Vec<usize>) = match joint.kind {
            ExactFeaJointKind::Bonded => {
                // A part bonded on two adjacent faces has the nodes of their
                // common edge in both joints (review 2026-11-20 ac FEA-901).
                // A node that already follows a joint or leads a tie does not
                // follow this one, and a triangle with a node that follows
                // does not lead; the joint is refused only when no node is
                // left to tie. Of what is left, the finer face, with more
                // nodes on the other, follows, so no node of it is left free
                // between the coarse face's nodes (R1121ac-FEA-1000); only
                // when both tie as many does the part bonded in more joints
                // lead.
                let may_follow = |free: &BTreeSet<usize>| -> BTreeSet<usize> {
                    free.iter()
                        .copied()
                        .filter(|node| !joined.contains(node) && !tie_leaders.contains(node))
                        .collect()
                };
                let may_lead = |face: &[[usize; 3]]| -> Vec<[usize; 3]> {
                    face.iter()
                        .copied()
                        .filter(|triangle| triangle.iter().all(|node| !joined.contains(node)))
                        .collect()
                };
                let bonds_of = |part: usize| {
                    joints
                        .iter()
                        .filter(|joint| matches!(joint.kind, ExactFeaJointKind::Bonded))
                        .filter(|joint| joint.faces.iter().any(|face| face.part == part))
                        .count()
                };
                let onto_first = bond(nodes, &may_lead(first_face), &may_follow(&second_free));
                let onto_second = bond(nodes, &may_lead(second_face), &may_follow(&first_free));
                let onto_first = (onto_first, first_face, second_face, &second_free);
                let onto_second = (onto_second, second_face, first_face, &first_free);
                let (ties, leader, follower, follower_free) =
                    match onto_first.0.len().cmp(&onto_second.0.len()) {
                        Ordering::Greater => onto_first,
                        Ordering::Less => onto_second,
                        Ordering::Equal if bonds_of(second.part) > bonds_of(first.part) => {
                            onto_second
                        }
                        Ordering::Equal => onto_first,
                    };
                // Faces that touch only along an edge, side by side or at a
                // right angle, would tie into a line that turns: some triangle of the
                // follower must lie on the leader whole (review 2026-11-20 ac P3).
                let tied = ties.iter().map(|(node, ..)| *node).collect::<BTreeSet<_>>();
                let touches = |node: &usize| {
                    tied.contains(node)
                        || lies_on(nodes, leader, nodes[*node].position_mm).is_some()
                };
                if ties.is_empty() || !follower.iter().any(|triangle| triangle.iter().all(touches))
                {
                    return Err(refused);
                }
                // A node on the other face that follows no joint, as it leads
                // a tie of another or the triangle under it has a node that
                // follows, moves apart from it (R1121ac P3: 3-5 such nodes,
                // unsaid); the study reports how many.
                untied += follower_free
                    .iter()
                    .filter(|node| !tied.contains(*node) && !joined.contains(*node))
                    .filter(|node| lies_on(nodes, leader, nodes[**node].position_mm).is_some())
                    .count();
                let leaders = ties.iter().flat_map(|(_, triangle, _)| *triangle).collect();
                let followers = ties.iter().map(|(node, ..)| *node).collect();
                couplings.extend(ties.into_iter().map(
                    |(node, [first, second, third], weights)| FeaCoupling {
                        nodes: [first, node],
                        kind: FeaCouplingKind::Tie {
                            others: [second, third],
                            weights,
                        },
                    },
                ));
                references.push(None);
                (leaders, followers)
            }
            ExactFeaJointKind::RigidConnector | ExactFeaJointKind::SpringConnector { .. } => {
                if first_free.is_empty() || second_free.is_empty() {
                    return Err(refused);
                }
                let place = centroid(nodes, first_face).ok_or(refused.clone())?;
                let reference = nodes.len();
                nodes.push(FeaNode { position_mm: place });
                references.push(Some(reference));
                let second_reference = match joint.kind {
                    ExactFeaJointKind::SpringConnector {
                        translation_n_per_mm,
                        rotation_n_mm_per_rad,
                    } => {
                        nodes.push(FeaNode { position_mm: place });
                        couplings.push(FeaCoupling {
                            nodes: [reference, reference + 1],
                            kind: FeaCouplingKind::Spring {
                                translation_n_per_mm,
                                rotation_n_mm_per_rad,
                            },
                        });
                        reference + 1
                    }
                    _ => reference,
                };
                for (leader, side) in [(reference, &first_free), (second_reference, &second_free)] {
                    couplings.extend(side.iter().map(|node| FeaCoupling {
                        nodes: [leader, *node],
                        kind: FeaCouplingKind::Rigid,
                    }));
                }
                (
                    BTreeSet::new(),
                    first_free.into_iter().chain(second_free).collect(),
                )
            }
        };
        if followers
            .iter()
            .any(|node| tie_leaders.contains(node) || !joined.insert(*node))
            || leaders.iter().any(|node| joined.contains(node))
        {
            return Err(refused);
        }
        tie_leaders.extend(leaders);
    }
    Ok(Joined {
        couplings,
        references,
        untied,
    })
}

#[cfg(test)]
mod tests;
