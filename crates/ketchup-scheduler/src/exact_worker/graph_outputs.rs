//! What a graph's result is asked for besides its evidence: its STEP file and
//! the control net of the surface it makes.

use super::{
    WorkerResult, evaluate_exact_brep_graph, geometry_failure, utf8_path, validated_graph,
    verified_exact_brep_graph_sources,
};
use crate::protocol::{
    ExportReceipt, GraphInput, SurfaceControlNetWire, WorkerFailure, WorkerReply,
};
use ketchup_exact::ExactBackend;
use std::path::Path;

pub(super) fn step_response(
    backend: &ExactBackend,
    input: &GraphInput,
    result_fingerprint: &str,
    output_path: &Path,
) -> WorkerResult {
    validated_graph(&input.graph)?;
    let sources = verified_exact_brep_graph_sources(&input.graph, &input.sources)?;
    let output = evaluate_exact_brep_graph(backend, &input.graph, &sources)
        .map_err(|error| geometry_failure(&error))?;
    if output.body.result_fingerprint != result_fingerprint {
        return Err(WorkerFailure::code("invalid_result"));
    }
    backend
        .export_step(&output.body, utf8_path(output_path)?)
        .map_err(|error| geometry_failure(&error))?;
    Ok(WorkerReply::Exported(ExportReceipt {
        result_fingerprint: output.body.result_fingerprint.clone(),
        sha256: None,
    }))
}

/// The control net of the surface the graph makes, which a control-point move
/// names its point by (R1120ac-EXA-900).
pub(super) fn control_net_response(backend: &ExactBackend, input: &GraphInput) -> WorkerResult {
    validated_graph(&input.graph)?;
    let sources = verified_exact_brep_graph_sources(&input.graph, &input.sources)?;
    let output = evaluate_exact_brep_graph(backend, &input.graph, &sources)
        .map_err(|error| geometry_failure(&error))?;
    let net = backend
        .surface_control_net(&output.body)
        .map_err(|error| geometry_failure(&error))?;
    Ok(WorkerReply::ControlNet(SurfaceControlNetWire {
        result_fingerprint: output.body.result_fingerprint.clone(),
        u_count: net.u_count,
        v_count: net.v_count,
        match_tolerance_mm: net.match_tolerance_mm(),
        points_mm: net.points_mm,
        weights: net.weights,
    }))
}
