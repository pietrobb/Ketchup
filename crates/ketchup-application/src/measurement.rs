//! Read-only selected-face measurements. Call `minimum_distance` off the GUI thread;
//! the host must guard its expected stamp before preparation and again before publishing.
//! No selection, canonical history, source program or exact registry is mutated here.
use crate::model_query::{EntityKind, ModelQuery};
use crate::rejections::rejected;
use ketchup_interaction::spatial::SnapshotBinding;
use ketchup_model::document::Snapshot;
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry, producer_exact_graph};
use ketchup_model::tolerance::ROUNDING;
use ketchup_model::topology::{TopologicalElementKind, TopologicalReferenceResolution};
use ketchup_rejection::Rejection;
use ketchup_scheduler::{ExactPairCandidate, ExactWorkerSupervisor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FaceTarget {
    /// The id of a current model_query face row, not a native face ordinal.
    pub entity_id: u64,
    /// Required even when the document happens to have only one instance.
    pub instance_path: ketchup_assistant::sidecar::AssistantInstancePath,
}

pub struct SelectedFaceMeasurement {
    binding: SnapshotBinding,
    targets: [FaceTarget; 2],
    graphs: [ExactBRepGraph; 2],
    candidate: ExactPairCandidate,
    ordinals: [u32; 2],
    planes: [Option<([f64; 3], [f64; 3])>; 2],
    tolerance: ketchup_model::tolerance::TolerancePolicy,
}

fn invalid(reason: impl Into<String>) -> Rejection {
    rejected("invalid_params").target("faces").reason(reason)
        .fix_hint("Query current exact faces, supply two face entity IDs and their explicit instance paths; use rigid placements.")
}
fn unavailable(reason: impl Into<String>) -> Rejection {
    rejected("entity_not_found").target("faces").reason(reason)
        .fix_hint("Wait for current exact topology, then query and select the faces again. No approximate measurement is available.")
}

type FaceData = (ExactBRepGraph, u32, [f64; 16], Option<([f64; 3], [f64; 3])>);

fn resolve_face(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
    query: &ModelQuery,
    target: &FaceTarget,
) -> Result<FaceData, Rejection> {
    let row = query
        .detail_with_topology(snapshot, registry, EntityKind::Faces, target.entity_id)
        .map_err(|error| {
            unavailable("The selected face row is not current or available")
                .caused_by_messages([format!("{error:?}")])
        })?;
    let instance = snapshot
        .resolve_instance_path(
            &crate::model_query::canonical_instance_path(snapshot, &target.instance_path).map_err(
                |error| invalid("Invalid instance path").caused_by_messages([format!("{error:?}")]),
            )?,
        )
        .map_err(|error| invalid("The instance path does not resolve").caused_by(error))?;
    let matrix = *instance.world_transform.matrix();
    if !rigid(&matrix) {
        return Err(invalid(
            "Face measurement supports rigid placements only; scale and shear are unsupported",
        ));
    }
    let packages = registry
        .body_values(snapshot)
        .map_err(|error| unavailable("Current exact bodies are unavailable").caused_by(error))?;
    for package in packages.into_values() {
        let ExactBodyPackage::Graph(graph_package) = package.as_ref() else {
            continue;
        };
        for evidence in &graph_package.face_evidence {
            let Some(reference) =
                package.topological_reference(TopologicalElementKind::Face, evidence.face_ordinal)
            else {
                continue;
            };
            if row["item"]["reference_id"].as_str() != Some(reference.lineage_digest.as_str()) {
                continue;
            }
            if reference.definition_id != instance.definition_id {
                return Err(invalid("Face and instance path name different definitions"));
            }
            match registry.resolve_topological_reference(snapshot, reference) {
                TopologicalReferenceResolution::Resolved {
                    reference: resolved,
                } if *resolved == *reference => {}
                resolution => {
                    return Err(unavailable(
                        "Face reference cannot be resolved uniquely in the current snapshot",
                    )
                    .caused_by_messages([format!("{resolution:?}")]));
                }
            }
            let graph = producer_exact_graph(
                snapshot,
                reference.definition_id,
                reference.producer_feature_id,
            )
            .map_err(|error| {
                unavailable("Selected face producer cannot be evaluated").caused_by(error)
            })?;
            let plane = (evidence.surface_kind == "plane").then(|| {
                let mut point = rotate(&matrix, evidence.centroid_mm);
                for i in 0..3 {
                    point[i] += matrix[4 * i + 3];
                }
                (point, rotate(&matrix, evidence.unit_normal))
            });
            return Ok((graph, evidence.face_ordinal, matrix, plane));
        }
    }
    Err(unavailable(
        "Selected face has no current graph-backed exact evidence",
    ))
}

impl SelectedFaceMeasurement {
    pub fn prepare(
        snapshot: &Snapshot,
        registry: &ExactResultRegistry,
        query: &ModelQuery,
        targets: [FaceTarget; 2],
    ) -> Result<Self, Rejection> {
        let (left, a, left_transform, p) = resolve_face(snapshot, registry, query, &targets[0])?;
        let (right, b, right_transform, q) = resolve_face(snapshot, registry, query, &targets[1])?;
        Ok(Self {
            binding: SnapshotBinding::from_snapshot(snapshot),
            targets,
            graphs: [left, right],
            ordinals: [a, b],
            planes: [p, q],
            candidate: ExactPairCandidate {
                left_graph: 0,
                right_graph: 1,
                left_transform,
                right_transform,
            },
            tolerance: snapshot.tolerance(),
        })
    }

    /// Recheck against the live snapshot before publishing an asynchronous result.
    pub fn require_current(&self, snapshot: &Snapshot) -> Result<(), Rejection> {
        if self.binding.is_current(snapshot) {
            Ok(())
        } else {
            Err(rejected("stale_document").target("faces"))
        }
    }

    pub fn minimum_distance_with_worker(
        &self,
        snapshot: &Snapshot,
        path: Option<&std::path::Path>,
        sources: &BTreeMap<String, Vec<u8>>,
        cancelled: &AtomicBool,
    ) -> Result<Value, Rejection> {
        let mut worker = crate::worker_pool::checkout(path, cancelled).map_err(|error| {
            rejected("job_worker_unavailable")
                .target("faces")
                .caused_by(error)
        })?;
        let result = self.minimum_distance(snapshot, &mut worker, sources, cancelled)?;
        worker.release();
        Ok(result)
    }
    pub fn minimum_distance(
        &self,
        snapshot: &Snapshot,
        worker: &mut ExactWorkerSupervisor,
        sources: &BTreeMap<String, Vec<u8>>,
        cancelled: &AtomicBool,
    ) -> Result<Value, Rejection> {
        self.require_current(snapshot)?;
        let distance = worker
            .query_exact_brep_faces_with_cancellation(
                &self.graphs,
                &self.candidate,
                self.ordinals,
                sources,
                cancelled,
            )
            .map_err(|error| {
                rejected("job_worker_unavailable")
                    .target("faces")
                    .caused_by(error)
            })?;
        Ok(self.report("trimmed_face_minimum", distance))
    }

    /// Signed clearance along a normalized world-space direction between parallel
    /// supporting planes. This deliberately makes no finite-face overlap claim.
    pub fn supporting_plane_clearance(
        &self,
        snapshot: &Snapshot,
        direction: [f64; 3],
    ) -> Result<Value, Rejection> {
        self.require_current(snapshot)?;
        let [Some((p, n)), Some((q, m))] = self.planes else {
            return Err(invalid(
                "Supporting-plane clearance requires two planar faces; curved faces are unsupported",
            ));
        };
        let distance = plane_clearance(p, n, q, m, direction, self.tolerance.angular_rad())?;
        let mut report = self.report("signed_supporting_plane_clearance", distance);
        report["finite_face_minimum"] = json!(false);
        report["direction_world"] = json!(unit(direction)?);
        Ok(report)
    }

    fn report(&self, mode: &str, distance: f64) -> Value {
        json!({"state":"verified", "mode":mode, "distance_mm":distance, "unit":"mm", "method":if mode == "trimmed_face_minimum" { "occt_trimmed_face_distance" } else { "exact_supporting_planes" },
            "targets":self.targets, "tolerance":self.tolerance,
            "canonical_mutation":false, "placement_support":"rigid_only"})
    }
}

fn rigid(matrix: &[f64; 16]) -> bool {
    matrix.iter().all(|v| v.is_finite())
        && matrix[12..] == [0.0, 0.0, 0.0, 1.0]
        && (0..3).all(|a| {
            (0..3).all(|b| {
                let dot = dot(
                    [matrix[a], matrix[4 + a], matrix[8 + a]],
                    [matrix[b], matrix[4 + b], matrix[8 + b]],
                );
                (dot - if a == b { 1.0 } else { 0.0 }).abs() <= ROUNDING
            })
        })
}
fn rotate(matrix: &[f64; 16], vector: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        dot(
            [matrix[4 * i], matrix[4 * i + 1], matrix[4 * i + 2]],
            vector,
        )
    })
}
use ketchup_geometry::linalg::dot;

fn unit(v: [f64; 3]) -> Result<[f64; 3], Rejection> {
    let length = dot(v, v).sqrt();
    if !length.is_finite() || length <= ROUNDING {
        return Err(invalid(
            "Direction and exact normals must be finite and nonzero",
        ));
    }
    Ok(v.map(|x| x / length))
}
fn plane_clearance(
    p: [f64; 3],
    n: [f64; 3],
    q: [f64; 3],
    m: [f64; 3],
    d: [f64; 3],
    angular: f64,
) -> Result<f64, Rejection> {
    let (n, m, d) = (unit(n)?, unit(m)?, unit(d)?);
    if ketchup_geometry::linalg::length(ketchup_geometry::linalg::cross(n, m))
        > angular.sin().max(ROUNDING)
    {
        return Err(invalid("Supporting planes must be parallel"));
    }
    let denominator = dot(n, d);
    if denominator.abs() <= angular.sin().max(ROUNDING) {
        return Err(invalid(
            "Measurement direction is tangent to the supporting planes",
        ));
    }
    let distance = dot(n, std::array::from_fn(|i| q[i] - p[i])) / denominator;
    if !distance.is_finite() {
        return Err(invalid("Plane clearance is non-finite"));
    }
    Ok(distance)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_support_planes_oblique_direction_and_rejections() {
        let p = [0.0; 3];
        let q = [650.0, 200.0, 0.0];
        let n = [1.0, 0.0, 0.0];
        let m = [-1.0, 0.0, 0.0];
        assert_eq!(plane_clearance(p, n, q, m, n, 1e-7).unwrap(), 650.0);
        assert_eq!(plane_clearance(p, n, q, m, m, 1e-7).unwrap(), -650.0);
        assert!(
            (plane_clearance(p, n, q, m, [1.0, 1.0, 0.0], 1e-7).unwrap() - 650.0 * 2.0_f64.sqrt())
                .abs()
                < 1e-7
        );
        assert!(plane_clearance(p, n, q, m, [0.0, 1.0, 0.0], 1e-7).is_err());
        assert!(plane_clearance(p, n, q, [0.0, 1.0, 0.0], n, 1e-7).is_err());
        assert!(plane_clearance(p, n, q, m, [f64::NAN, 0.0, 0.0], 1e-7).is_err());
    }
}
