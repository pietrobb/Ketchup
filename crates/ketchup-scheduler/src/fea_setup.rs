//! Binds an exact volume mesh to an FEA model: one occurrence with its held
//! faces and face loads.

use crate::ExactBRepVolumeMeshPackage;
use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaElement, FeaElementKind, FeaLoad, FeaMaterial, FeaModel,
    FeaNode,
};
use ketchup_model::document::{InstancePath, Snapshot};
use std::collections::BTreeSet;

impl ExactBRepVolumeMeshPackage {
    pub fn occurrence_bound_model(
        &self,
        snapshot: &Snapshot,
        setup: &ExactFeaSetup,
    ) -> Result<OccurrenceBoundFeaModel, ExactFeaSetupError> {
        self.check_occurrence(snapshot, &setup.instance_path)?;
        if setup.case_id.trim().is_empty()
            || setup.constrained_face_ordinals.is_empty()
            || setup.face_tractions.is_empty()
        {
            return Err(ExactFeaSetupError::IncompleteSetup);
        }
        let boundary_face_ordinals = self
            .mesh
            .boundary_triangles
            .iter()
            .map(|triangle| triangle.face_ordinal)
            .collect::<BTreeSet<_>>();
        if setup
            .constrained_face_ordinals
            .iter()
            .any(|ordinal| !boundary_face_ordinals.contains(ordinal))
            || setup
                .face_tractions
                .iter()
                .any(|traction| !boundary_face_ordinals.contains(&traction.face_ordinal))
            || setup
                .face_tractions
                .iter()
                .flat_map(|traction| traction.traction_local_n_per_mm2)
                .any(|value| !value.is_finite())
        {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let constrained_faces = setup
            .constrained_face_ordinals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if constrained_faces.len() != setup.constrained_face_ordinals.len() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let traction_faces = setup
            .face_tractions
            .iter()
            .map(|traction| traction.face_ordinal)
            .collect::<BTreeSet<_>>();
        if traction_faces.len() != setup.face_tractions.len() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }

        let constrained_nodes = self
            .mesh
            .boundary_triangles
            .iter()
            .filter(|triangle| constrained_faces.contains(&triangle.face_ordinal))
            .flat_map(|triangle| triangle.vertex_indices)
            .collect::<BTreeSet<_>>();
        if constrained_nodes.is_empty() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let loads = setup
            .face_tractions
            .iter()
            .flat_map(|traction| {
                self.mesh
                    .boundary_triangles
                    .iter()
                    .filter(move |triangle| triangle.face_ordinal == traction.face_ordinal)
                    .map(move |triangle| FeaLoad::SurfaceTraction {
                        nodes: triangle.vertex_indices.map(|index| index as usize),
                        traction_n_per_mm2: traction.traction_local_n_per_mm2,
                    })
            })
            .collect::<Vec<_>>();
        if loads.is_empty() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let model = FeaModel {
            schema: FEA_MODEL_SCHEMA_V1.to_owned(),
            case_id: setup.case_id.clone(),
            nodes: self
                .mesh
                .vertices_mm
                .iter()
                .copied()
                .map(|position_mm| FeaNode { position_mm })
                .collect(),
            materials: vec![setup.material.clone()],
            elements: self
                .mesh
                .tetrahedra
                .iter()
                .enumerate()
                .map(|(index, nodes)| FeaElement {
                    id: index as u64 + 1,
                    kind: FeaElementKind::LinearTetrahedron4 {
                        nodes: nodes.map(|node| node as usize),
                        material_id: setup.material.id,
                    },
                })
                .collect(),
            constraints: constrained_nodes
                .into_iter()
                .map(|node| FeaConstraint::held(node as usize))
                .collect(),
            loads,
            couplings: Vec::new(),
        };
        Ok(OccurrenceBoundFeaModel {
            instance_path: setup.instance_path.clone(),
            source_revision: self.source.identity.source_revision,
            source_digest: self.source.identity.source_digest.clone(),
            graph_digest: self.mesh.graph_digest.clone(),
            mesh_fingerprint: self.mesh.mesh_fingerprint.clone(),
            model,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaFaceTraction {
    pub face_ordinal: u32,
    pub traction_local_n_per_mm2: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaSetup {
    pub case_id: String,
    pub instance_path: InstancePath,
    pub material: FeaMaterial,
    pub constrained_face_ordinals: Vec<u32>,
    pub face_tractions: Vec<ExactFeaFaceTraction>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceBoundFeaModel {
    pub instance_path: InstancePath,
    pub source_revision: u64,
    pub source_digest: String,
    pub graph_digest: String,
    pub mesh_fingerprint: String,
    pub model: FeaModel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactFeaSetupError {
    StaleGeometry,
    InvalidInstancePath(ketchup_model::document::CanonicalError),
    OccurrenceMismatch,
    IncompleteSetup,
    InvalidBoundarySelection,
    /// A part is scaled or sheared where the assembly places it.
    NonRigidPlacement,
    /// Two parts name one material id with different values.
    ConflictingMaterial,
    /// The joint at index `joint` joins faces of one part, names a face the
    /// mesh does not have, has no free node on a face, or shares a node with
    /// an earlier joint.
    InvalidJoint {
        joint: usize,
    },
    /// The mate is not a face-to-face mate of the document whose two
    /// resolved faces belong to studied parts.
    InvalidMate {
        mate_id: ketchup_model::assembly::AssemblyMateId,
    },
}

/// What the user changes, not the Rust name (review 2026-11-20 ac P3).
impl std::fmt::Display for ExactFeaSetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleGeometry => {
                formatter.write_str("the part changed since it was meshed; run the study again")
            }
            Self::InvalidInstancePath(cause) => write!(formatter, "the studied part: {cause}"),
            Self::OccurrenceMismatch => {
                formatter.write_str("the mesh belongs to another part than the one studied")
            }
            Self::IncompleteSetup => formatter.write_str(
                "the study needs a name, at least one fixed face and at least one loaded face",
            ),
            Self::InvalidBoundarySelection => formatter.write_str(
                "a fixed or loaded face is not a face of the meshed part; pick the faces again",
            ),
            Self::NonRigidPlacement => formatter.write_str(
                "a part is scaled or sheared where the assembly places it; study it unscaled",
            ),
            Self::ConflictingMaterial => {
                formatter.write_str("two parts name one material with different values")
            }
            Self::InvalidJoint { joint } => write!(
                formatter,
                "joint {} cannot be built: its faces belong to one part, do not touch over an \
                 area, or share every free node with an earlier joint",
                joint + 1
            ),
            Self::InvalidMate { mate_id } => write!(
                formatter,
                "mate {} is not a face-to-face mate between two studied parts",
                mate_id.0
            ),
        }
    }
}
