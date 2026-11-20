//! The control net of a surface feature, read in the exact worker: where each
//! control point of its B-spline form lies, which a control-point move names
//! by row, column and `from_mm` (R1120ac-EXA-900). Before this the only way to
//! learn a point's position was a move refused for expecting it elsewhere.
//! Read-only: nothing in the document changes.
use crate::rejections::rejected;
use ketchup_model::document::{DefinitionId, FeatureId, Snapshot};
use ketchup_model::exact_product::producer_exact_graph;
use ketchup_rejection::Rejection;
use ketchup_scheduler::ExactWorkerSupervisor;
use ketchup_scheduler::protocol::SurfaceControlNetWire as SurfaceControlNet;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

/// The most control points one answer lists; `offset` pages through the rest.
pub const MAX_CONTROL_POINTS_SHOWN: usize = 256;

/// The control net of surface `feature_id` of `definition_id`, its points from
/// `offset` on.
pub fn surface_control_net_with_worker(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    feature_id: FeatureId,
    offset: usize,
    path: Option<&std::path::Path>,
    sources: &ketchup_model::persistence::Blobs,
    cancelled: &AtomicBool,
) -> Result<Value, Rejection> {
    let graph = producer_exact_graph(snapshot, definition_id, feature_id).map_err(|error| {
        rejected("entity_not_found")
            .target("feature_id")
            .reason(format!(
                "Feature {} of definition {} makes no exact surface",
                feature_id.0, definition_id.0
            ))
            .fix_hint("Give a surface feature and its definition from inspect query kind=features.")
            .caused_by(error)
    })?;
    let mut worker = crate::worker_pool::checkout(path, cancelled).map_err(|error| {
        rejected("job_worker_unavailable")
            .target("feature_id")
            .caused_by(error)
    })?;
    let net = worker_net(&mut worker, &graph, sources, cancelled)?;
    worker.release();
    Ok(report(definition_id, feature_id, &net, offset))
}

fn worker_net(
    worker: &mut ExactWorkerSupervisor,
    graph: &ketchup_model::exact_brep_graph::ExactBRepGraph,
    sources: &ketchup_model::persistence::Blobs,
    cancelled: &AtomicBool,
) -> Result<SurfaceControlNet, Rejection> {
    worker
        .surface_control_net(graph, sources, cancelled)
        .map_err(|error| {
            rejected("invalid_params")
                .target("feature_id")
                .reason("The feature's shape has no single-face B-spline control net")
                .fix_hint("Give a one-face surface feature (loft, boundary, blend or an earlier control-point move).")
                .caused_by(error)
        })
}

/// The answer: the net's size, the tolerance a move's `from_mm` is matched
/// within, and up to [`MAX_CONTROL_POINTS_SHOWN`] points from `offset`.
pub fn report(
    definition_id: DefinitionId,
    feature_id: FeatureId,
    net: &SurfaceControlNet,
    offset: usize,
) -> Value {
    let columns = usize::try_from(net.v_count.max(1)).unwrap_or(usize::MAX);
    let end = net
        .points_mm
        .len()
        .min(offset.saturating_add(MAX_CONTROL_POINTS_SHOWN));
    let points: Vec<Value> = (offset.min(end)..end)
        .map(|index| {
            json!({
                "u_index": index / columns,
                "v_index": index % columns,
                "from_mm": net.points_mm[index],
                "weight": net.weights[index],
            })
        })
        .collect();
    json!({
        "definition_id": definition_id.0,
        "feature_id": feature_id.0,
        "result_fingerprint": net.result_fingerprint,
        "u_count": net.u_count,
        "v_count": net.v_count,
        "point_count": net.points_mm.len(),
        "match_tolerance_mm": net.match_tolerance_mm,
        "offset": offset,
        "next_offset": (end < net.points_mm.len()).then_some(end),
        "points": points,
        "hint": "A ControlPoint surface feature names a point by u_index, v_index and from_mm; \
                 from_mm matches within match_tolerance_mm.",
        "canonical_mutation": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_names_each_point_by_row_and_column_and_pages_the_rest() {
        let net = SurfaceControlNet {
            result_fingerprint: "surface".into(),
            u_count: 30,
            v_count: 10,
            points_mm: (0..300).map(|index| [f64::from(index), 0.0, 1.0]).collect(),
            weights: vec![1.0; 300],
            match_tolerance_mm: 1.0e-3,
        };
        let first = report(DefinitionId(1), FeatureId(2), &net, 0);
        assert_eq!(
            first["points"].as_array().unwrap().len(),
            MAX_CONTROL_POINTS_SHOWN
        );
        assert_eq!(first["points"][13]["u_index"], 1);
        assert_eq!(first["points"][13]["v_index"], 3);
        assert_eq!(first["points"][13]["from_mm"], json!([13.0, 0.0, 1.0]));
        assert_eq!(first["next_offset"], MAX_CONTROL_POINTS_SHOWN);
        assert_eq!(first["match_tolerance_mm"], 1.0e-3);
        let rest = report(
            DefinitionId(1),
            FeatureId(2),
            &net,
            MAX_CONTROL_POINTS_SHOWN,
        );
        assert_eq!(
            rest["points"].as_array().unwrap().len(),
            300 - MAX_CONTROL_POINTS_SHOWN
        );
        assert_eq!(rest["points"][0]["u_index"], 25);
        assert_eq!(rest["next_offset"], Value::Null);
        let past = report(DefinitionId(1), FeatureId(2), &net, 1_000);
        assert!(past["points"].as_array().unwrap().is_empty());
    }
}
