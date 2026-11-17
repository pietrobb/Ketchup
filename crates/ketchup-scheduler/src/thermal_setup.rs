//! A steady heat-conduction model of one occurrence's exact volume mesh:
//! temperatures, heat fluxes and convection are set on exact faces by their
//! ordinals and land on every boundary triangle the mesher made of them.

use crate::{ExactBRepVolumeMeshPackage, ExactFeaSetupError};
use ketchup_analysis::fea::FeaNode;
use ketchup_analysis::fea_thermal::{
    THERMAL_MODEL_SCHEMA_V1, ThermalBoundary, ThermalElement, ThermalMaterial, ThermalModel,
    ThermalTemperature,
};
use ketchup_model::document::{InstancePath, Snapshot};
use std::collections::BTreeSet;

/// A face held at a temperature.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactThermalFaceTemperature {
    pub face_ordinal: u32,
    pub temperature_c: f64,
}

/// Heat entering through a face, W/m² (negative leaves it).
#[derive(Clone, Debug, PartialEq)]
pub struct ExactThermalFaceFlux {
    pub face_ordinal: u32,
    pub flux_w_per_m2: f64,
}

/// A face exchanging heat with a surrounding fluid.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactThermalFaceConvection {
    pub face_ordinal: u32,
    pub film_coefficient_w_per_m2_k: f64,
    pub ambient_c: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactThermalSetup {
    pub case_id: String,
    pub instance_path: InstancePath,
    pub material: ThermalMaterial,
    pub held_faces: Vec<ExactThermalFaceTemperature>,
    pub flux_faces: Vec<ExactThermalFaceFlux>,
    pub convection_faces: Vec<ExactThermalFaceConvection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceBoundThermalModel {
    pub instance_path: InstancePath,
    pub source_revision: u64,
    pub source_digest: String,
    pub graph_digest: String,
    pub mesh_fingerprint: String,
    pub model: ThermalModel,
}

impl ExactBRepVolumeMeshPackage {
    /// The conduction model of the occurrence with the setup's faces.
    ///
    /// # Errors
    /// A stale mesh, an occurrence of another definition, a setup with
    /// nothing fixing the temperature level, or a face that is not on the
    /// mesh or is named twice.
    pub fn occurrence_bound_thermal_model(
        &self,
        snapshot: &Snapshot,
        setup: &ExactThermalSetup,
    ) -> Result<OccurrenceBoundThermalModel, ExactFeaSetupError> {
        self.check_occurrence(snapshot, &setup.instance_path)?;
        if setup.case_id.trim().is_empty()
            || (setup.held_faces.is_empty() && setup.convection_faces.is_empty())
        {
            return Err(ExactFeaSetupError::IncompleteSetup);
        }
        let mesh_faces = self
            .mesh
            .boundary_triangles
            .iter()
            .map(|triangle| triangle.face_ordinal)
            .collect::<BTreeSet<_>>();
        let named = setup
            .held_faces
            .iter()
            .map(|face| face.face_ordinal)
            .chain(setup.flux_faces.iter().map(|face| face.face_ordinal))
            .chain(setup.convection_faces.iter().map(|face| face.face_ordinal))
            .collect::<Vec<_>>();
        let distinct = named.iter().copied().collect::<BTreeSet<_>>();
        if distinct.len() != named.len() || !distinct.is_subset(&mesh_faces) {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }

        let temperatures = setup
            .held_faces
            .iter()
            .flat_map(|face| {
                self.triangles_of(face.face_ordinal)
                    .flatten()
                    .map(|node| ThermalTemperature {
                        node,
                        temperature_c: face.temperature_c,
                    })
            })
            .collect();
        let fluxes = setup.flux_faces.iter().flat_map(|face| {
            self.triangles_of(face.face_ordinal)
                .map(|nodes| ThermalBoundary::HeatFlux {
                    nodes,
                    flux_w_per_m2: face.flux_w_per_m2,
                })
        });
        let convections = setup.convection_faces.iter().flat_map(|face| {
            self.triangles_of(face.face_ordinal)
                .map(|nodes| ThermalBoundary::Convection {
                    nodes,
                    film_coefficient_w_per_m2_k: face.film_coefficient_w_per_m2_k,
                    ambient_c: face.ambient_c,
                })
        });
        let model = ThermalModel {
            schema: THERMAL_MODEL_SCHEMA_V1.to_owned(),
            case_id: setup.case_id.clone(),
            nodes: self
                .mesh
                .vertices_mm
                .iter()
                .map(|&position_mm| FeaNode { position_mm })
                .collect(),
            materials: vec![setup.material.clone()],
            elements: (1_u64..)
                .zip(&self.mesh.tetrahedra)
                .map(|(id, nodes)| ThermalElement {
                    id,
                    nodes: nodes.map(node_index),
                    material_id: setup.material.id,
                })
                .collect(),
            temperatures,
            boundaries: fluxes.chain(convections).collect(),
        };
        Ok(OccurrenceBoundThermalModel {
            instance_path: setup.instance_path.clone(),
            source_revision: self.source.identity.source_revision,
            source_digest: self.source.identity.source_digest.clone(),
            graph_digest: self.mesh.graph_digest.clone(),
            mesh_fingerprint: self.mesh.mesh_fingerprint.clone(),
            model,
        })
    }

    /// The mesh is of the current snapshot and of the occurrence's definition.
    pub(crate) fn check_occurrence(
        &self,
        snapshot: &Snapshot,
        instance_path: &InstancePath,
    ) -> Result<(), ExactFeaSetupError> {
        if !self.is_current(snapshot) {
            return Err(ExactFeaSetupError::StaleGeometry);
        }
        if snapshot
            .resolve_instance_path(instance_path)
            .map_err(ExactFeaSetupError::InvalidInstancePath)?
            .definition_id
            != self.source.identity.definition_id
        {
            return Err(ExactFeaSetupError::OccurrenceMismatch);
        }
        Ok(())
    }

    fn triangles_of(&self, face_ordinal: u32) -> impl Iterator<Item = [usize; 3]> + '_ {
        self.mesh
            .boundary_triangles
            .iter()
            .filter(move |triangle| triangle.face_ordinal == face_ordinal)
            .map(|triangle| triangle.vertex_indices.map(node_index))
    }
}

/// A mesh index; one past the address space names no node and the solver
/// refuses it.
fn node_index(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}
