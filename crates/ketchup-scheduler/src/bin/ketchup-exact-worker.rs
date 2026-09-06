#![forbid(unsafe_code)]

use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V6, EXACT_BREP_GRAPH_SCHEMA_V7, EXACT_BREP_GRAPH_SCHEMA_V8,
    EXACT_BREP_GRAPH_SCHEMA_V9, EXACT_BREP_GRAPH_SCHEMA_V10, EXACT_BREP_GRAPH_SCHEMA_V11,
    EXACT_BREP_GRAPH_SCHEMA_V12, EXACT_BREP_GRAPH_SCHEMA_V13, ExactBRepBooleanOperation,
    ExactBRepEdgeFinishKind, ExactBRepGraph, ExactBRepLinearInterval, ExactBRepOperation,
    ExactBRepPlanarGeometry, ExactBRepPlanarLoop, ExactBRepPlanarSegment, ExactBRepProfile,
    ExactBRepSpatialPathSegment, ExactBRepTopologyKind, ExactBRepTopologySelector,
    MAX_EXACT_BREP_GRAPH_BYTES, MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS, MAX_EXACT_BREP_REGION_HOLES,
    MAX_EXACT_BREP_REGION_SEGMENTS, SKETCH_SWEEP_FRAME_EPSILON_MM,
    exact_brep_planar_rectangle_bounds,
};
use ketchup_core::exact_product::{EXACT_BREP_GRAPH_EVALUATOR_V1, ExactCircleProfile};
use ketchup_core::graph::sha256_hex;
use ketchup_core::import::{
    MAX_STEP_MESH_TRIANGLES, MAX_STEP_SOURCE_BYTES, StepImportMesh, StepMeshTriangle,
};
use ketchup_exact::{
    BoxSpec, CircleExtrudeSpec, CutMode, CylinderToolSpec, EdgeFinish, ExactBackend,
    ExactBodyBooleanOperation, ExactKernel, ExactOpOutput, PlanarProfileLoop, PlanarProfileSegment,
    Point3, RectangleExtrudeSpec, RectangleOffsetSpec, RectangleSweepSpec, ReferenceResolution,
    Size3, SpatialProfileSegment, SplineLoftSection, SplineLoftSpec, StabilityClass,
    capture_bounded_pocket_references, capture_bounded_through_cut_references,
    capture_box_shell_references, capture_circle_extrusion_references,
    capture_circular_pocket_references, capture_circular_split_references,
    capture_circular_through_cut_references, capture_contained_polygon_intersection_references,
    capture_contained_polygon_union_references, capture_general_revolve_references,
    capture_guaranteed_references, capture_mixed_profile_extrusion_references,
    capture_planar_offset_reference, capture_polygon_through_cut_references,
    capture_profile_split_references, capture_rectangular_intersection_references,
    capture_rectangular_split_references, capture_rectangular_sweep_references,
    capture_rectangular_union_references, capture_revolve_references, capture_shell_references,
    capture_spline_loft_references, resolve_subshape_reference,
};
#[cfg(feature = "named-product-fixtures")]
use ketchup_exact::{
    HalfLapFaceRole, HalfLapNotchSpec, HalfLapParticipant, capture_half_lap_notch_references,
};
use ketchup_scheduler::{
    MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES, MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES,
    StepAssemblyManifest, StepFeatureExportSpec, StepProfileSegment, StepRevolveExportSpec,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, Read, Write};
use std::time::{Duration, Instant};

const MAX_WORKER_REQUEST_LINE_BYTES: usize = MAX_EXACT_BREP_GRAPH_BYTES * 2 + 64 * 1024;

mod pair_query;

fn main() {
    let backend = ExactBackend::new();
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout().lock();
    let mut pairs = pair_query::PairQuerySession::default();
    loop {
        let Ok(Some(line)) = read_bounded_request_line(&mut stdin) else {
            break;
        };
        let response = pairs
            .handle(&backend, &line)
            .or_else(|| handle_request(&backend, &line));
        if let Some(response) = response
            && writeln!(stdout, "{response}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
}

fn read_bounded_request_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    let bytes_read = reader
        .take((MAX_WORKER_REQUEST_LINE_BYTES + 1) as u64)
        .read_line(&mut line)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    if bytes_read > MAX_WORKER_REQUEST_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "worker request line exceeds the bounded protocol envelope",
        ));
    }
    Ok(Some(line))
}

fn handle_request(backend: &ExactBackend, request: &str) -> Option<String> {
    let mut fields = request.split_whitespace();
    match (fields.next(), fields.next(), fields.next()) {
        (Some("PING"), None, None) => Some("PONG".to_owned()),
        (Some("CAPS"), Some("M3_V1"), None) => Some("CAPS M3_V1".to_owned()),
        (Some("CAPS"), Some("M3_V2"), None) => Some("CAPS M3_V2".to_owned()),
        (Some("CAPS"), Some("M3_CUT_V1"), None) => Some("CAPS M3_CUT_V1".to_owned()),
        (Some("CAPS"), Some("M3_POCKET_V1"), None) => Some("CAPS M3_POCKET_V1".to_owned()),
        (Some("CAPS"), Some("M3_UNION_V1"), None) => Some("CAPS M3_UNION_V1".to_owned()),
        (Some("CAPS"), Some("P6_INTERSECT_V1"), None) => Some("CAPS P6_INTERSECT_V1".to_owned()),
        (Some("CAPS"), Some("P6_SPLIT_V1"), None) => Some("CAPS P6_SPLIT_V1".to_owned()),
        (Some("CAPS"), Some("P6_OFFSET_V1"), None) => Some("CAPS P6_OFFSET_V1".to_owned()),
        (Some("CAPS"), Some("P6_OFFSET_V2"), None) => Some("CAPS P6_OFFSET_V2".to_owned()),
        (Some("CAPS"), Some("P6_OFFSET_V3"), None) => Some("CAPS P6_OFFSET_V3".to_owned()),
        (Some("CAPS"), Some("P6_SWEEP_V1"), None) => Some("CAPS P6_SWEEP_V1".to_owned()),
        (Some("CAPS"), Some("P6_LOFT_V1"), None) => Some("CAPS P6_LOFT_V1".to_owned()),
        (Some("CAPS"), Some("P3_CIRCLE_V1"), None) => Some("CAPS P3_CIRCLE_V1".to_owned()),
        (Some("CAPS"), Some("P3_CIRCLE_V2"), None) => Some("CAPS P3_CIRCLE_V2".to_owned()),
        (Some("CAPS"), Some("P3_ARC_V1"), None) => Some("CAPS P3_ARC_V1".to_owned()),
        (Some("CAPS"), Some("P3_POLYGON_CUT_V1"), None) => {
            Some("CAPS P3_POLYGON_CUT_V1".to_owned())
        }
        #[cfg(feature = "named-product-fixtures")]
        (Some("CAPS"), Some("M5_NOTCH_V1"), None) => Some("CAPS M5_NOTCH_V1".to_owned()),
        (Some("CAPS"), Some("M6_REVOLVE_V1"), None) => Some("CAPS M6_REVOLVE_V1".to_owned()),
        (Some("CAPS"), Some("P4_REVOLVE_V1"), None) => Some("CAPS P4_REVOLVE_V1".to_owned()),
        (Some("CAPS"), Some("M6_SHELL_V1"), None) => Some("CAPS M6_SHELL_V1".to_owned()),
        (Some("CAPS"), Some("M6_FINISH_V1"), None) => Some("CAPS M6_FINISH_V1".to_owned()),
        (Some("CAPS"), Some("P5_SHELL_V1"), None) => Some("CAPS P5_SHELL_V1".to_owned()),
        (Some("CAPS"), Some("P5_FINISH_V1"), None) => Some("CAPS P5_FINISH_V1".to_owned()),
        (Some("CAPS"), Some("M14_STEP_V1"), None) => Some("CAPS M14_STEP_V1".to_owned()),
        (Some("CAPS"), Some("M21_STEP_MODEL_V1"), None) => {
            Some("CAPS M21_STEP_MODEL_V1".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V6"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V6".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V7"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V7".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V8"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V8".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V9"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V9".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V10"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V10".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V11"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V11".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V12"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V12".to_owned())
        }
        (Some("CAPS"), Some("EXACT_BREP_GRAPH_V13"), None) => {
            Some("CAPS EXACT_BREP_GRAPH_V13".to_owned())
        }
        (
            Some(
                operation @ ("TESSELLATE_BREP_GRAPH_V6"
                | "TESSELLATE_BREP_GRAPH_V7"
                | "TESSELLATE_BREP_GRAPH_V8"
                | "TESSELLATE_BREP_GRAPH_V9"
                | "TESSELLATE_BREP_GRAPH_V10"
                | "TESSELLATE_BREP_GRAPH_V11"
                | "TESSELLATE_BREP_GRAPH_V12"
                | "TESSELLATE_BREP_GRAPH_V13"),
            ),
            Some(graph_digest),
            Some(encoded_graph),
        ) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(exact_brep_graph_mesh_response(
                backend,
                operation,
                graph_digest,
                encoded_graph,
                &remaining,
            ))
        }
        (
            Some(
                operation @ ("EXPORT_BREP_GRAPH_STEP_V2"
                | "EXPORT_BREP_GRAPH_STEP_V3"
                | "EXPORT_BREP_GRAPH_STEP_V4"),
            ),
            Some(graph_digest),
            Some(encoded_graph),
        ) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(exact_brep_graph_step_response(
                backend,
                operation,
                graph_digest,
                encoded_graph,
                &remaining,
            ))
        }
        (
            Some(
                operation @ ("EVAL_BREP_GRAPH_V6"
                | "EVAL_BREP_GRAPH_V7"
                | "EVAL_BREP_GRAPH_V8"
                | "EVAL_BREP_GRAPH_V9"
                | "EVAL_BREP_GRAPH_V10"
                | "EVAL_BREP_GRAPH_V11"
                | "EVAL_BREP_GRAPH_V12"
                | "EVAL_BREP_GRAPH_V13"),
            ),
            Some(graph_digest),
            Some(encoded_graph),
        ) => {
            let remaining = fields.collect::<Vec<_>>();
            let Some(graph) = decode_exact_brep_graph(operation, graph_digest, encoded_graph)
            else {
                return Some("ERR invalid_request".to_owned());
            };
            Some(exact_brep_graph_response(backend, &graph, &remaining))
        }
        (Some("EXPORT_REVOLVE_STEP_M21_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m21_revolve_step_export_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
            ))
        }
        (Some("EXPORT_FEATURE_STEP_M21_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m21_box_step_export_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
            ))
        }
        (Some("INSPECT_STEP_PART_M21_V1"), Some(source_sha256), Some(source_path)) => Some(
            m21_step_part_inspection_response(backend, source_sha256, source_path),
        ),
        (Some("TESSELLATE_STEP_PART_M21_V1"), Some(source_sha256), Some(source_path)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m21_step_part_mesh_response(
                backend,
                source_sha256,
                source_path,
                &remaining,
            ))
        }
        (Some("ASSEMBLE_STEP_M21_V1"), Some(assembly_digest), Some(output_path)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m21_step_assembly_response(
                backend,
                assembly_digest,
                output_path,
                &remaining,
            ))
        }
        (Some("EXPORT_STEP_M14_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m14_step_export_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
            ))
        }
        #[cfg(feature = "named-product-fixtures")]
        (Some("EVAL_NOTCHED_M5_V1"), Some(document_id), Some(piece_key)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m5_notched_response(
                backend,
                document_id,
                piece_key,
                &remaining,
            ))
        }
        (Some("REVOLVE_M6_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m6_revolve_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
                None,
            ))
        }
        (Some("REVOLVE_P4_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(p4_revolve_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
            ))
        }
        (Some("SHELL_M6_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(m6_revolve_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
                Some("shell"),
            ))
        }
        (Some("FINISH_M6_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            let finish = remaining.get(2).copied();
            Some(m6_revolve_response(
                backend,
                document_id,
                producer_feature_id,
                &remaining,
                finish,
            ))
        }
        (Some("SHELL_BOX_P5_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            Some(p5_box_shell_response(
                backend, width_bits, depth_bits, &remaining, None,
            ))
        }
        (Some("FINISH_BOX_P5_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let finish = remaining.get(2).copied();
            Some(p5_box_shell_response(
                backend, width_bits, depth_bits, &remaining, finish,
            ))
        }
        (Some("EXTRUDE_MIXED_P3_V1"), Some(segment_count), Some(height_bits)) => {
            let Ok(segment_count) = segment_count.parse::<usize>() else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(height_bits) = u64::from_str_radix(height_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let remaining = fields.collect::<Vec<_>>();
            if !(2..=64).contains(&segment_count) || remaining.len() != segment_count + 3 {
                return Some("ERR invalid_request".to_owned());
            }
            let document_id = remaining[0];
            let producer_feature_id = remaining[1];
            let request_digest = remaining[2];
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            let Some(segments) = remaining[3..]
                .iter()
                .map(|token| parse_profile_segment(token))
                .collect::<Option<Vec<_>>>()
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            Some(p3_mixed_extrude_response(
                backend,
                &segments,
                f64::from_bits(height_bits),
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (
            operation @ (Some("EXTRUDE_POLYGON_UNION_P3_V1")
            | Some("EXTRUDE_POLYGON_INTERSECT_P3_V1")
            | Some("EXTRUDE_POLYGON_SPLIT_P3_V1")),
            Some(segment_count),
            Some(width_bits),
        ) => {
            let Ok(segment_count) = segment_count.parse::<usize>() else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let remaining = fields.collect::<Vec<_>>();
            let segment_count_supported = (3..=64).contains(&segment_count)
                || (matches!(
                    operation,
                    Some("EXTRUDE_POLYGON_UNION_P3_V1")
                        | Some("EXTRUDE_POLYGON_INTERSECT_P3_V1")
                        | Some("EXTRUDE_POLYGON_SPLIT_P3_V1")
                ) && segment_count == 2);
            if !segment_count_supported || remaining.len() != segment_count + 5 {
                return Some("ERR invalid_request".to_owned());
            }
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [width_bits, remaining[0], remaining[1]]
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 3]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let document_id = remaining[2];
            let producer_feature_id = remaining[3];
            let request_digest = remaining[4];
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            let Some(segments) = remaining[5..]
                .iter()
                .map(|token| parse_profile_segment(token))
                .collect::<Option<Vec<_>>>()
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            Some(if operation == Some("EXTRUDE_POLYGON_UNION_P3_V1") {
                p3_polygon_union_response(
                    backend,
                    bits,
                    &segments,
                    document_id,
                    producer_feature_id,
                    request_digest,
                )
            } else if operation == Some("EXTRUDE_POLYGON_INTERSECT_P3_V1") {
                p3_polygon_intersection_response(
                    backend,
                    bits,
                    &segments,
                    document_id,
                    producer_feature_id,
                    request_digest,
                )
            } else {
                p3_polygon_split_response(
                    backend,
                    bits,
                    &segments,
                    document_id,
                    producer_feature_id,
                    request_digest,
                )
            })
        }
        (Some("EXTRUDE_POLYGON_CUT_P3_V1"), Some(segment_count), Some(width_bits)) => {
            let Ok(segment_count) = segment_count.parse::<usize>() else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let remaining = fields.collect::<Vec<_>>();
            if !(2..=64).contains(&segment_count) || remaining.len() != segment_count + 5 {
                return Some("ERR invalid_request".to_owned());
            }
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [width_bits, remaining[0], remaining[1]]
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 3]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let document_id = remaining[2];
            let producer_feature_id = remaining[3];
            let request_digest = remaining[4];
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            let Some(segments) = remaining[5..]
                .iter()
                .map(|token| parse_profile_segment(token))
                .collect::<Option<Vec<_>>>()
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            Some(p3_polygon_cut_response(
                backend,
                bits,
                None,
                &segments,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_POLYGON_POCKET_P3_V1"), Some(segment_count), Some(width_bits)) => {
            let Ok(segment_count) = segment_count.parse::<usize>() else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let remaining = fields.collect::<Vec<_>>();
            if !(2..=64).contains(&segment_count) || remaining.len() != segment_count + 6 {
                return Some("ERR invalid_request".to_owned());
            }
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [width_bits, remaining[0], remaining[1]]
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 3]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(pocket_depth_bits) = parse_bits(remaining[2]) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let document_id = remaining[3];
            let producer_feature_id = remaining[4];
            let request_digest = remaining[5];
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            let Some(segments) = remaining[6..]
                .iter()
                .map(|token| parse_profile_segment(token))
                .collect::<Option<Vec<_>>>()
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            Some(p3_polygon_cut_response(
                backend,
                bits,
                Some(pocket_depth_bits),
                &segments,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (
            Some(protocol @ ("EXTRUDE_CIRCLE_P3_V1" | "EXTRUDE_CIRCLE_P3_V2")),
            Some(center_x_bits),
            Some(center_y_bits),
        ) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                radius_bits,
                height_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [center_x_bits, center_y_bits, radius_bits, height_bits]
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 4]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p3_circle_extrude_response(
                backend,
                protocol,
                bits,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_CIRCULAR_POCKET_P3_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                center_x_bits,
                center_y_bits,
                radius_bits,
                pocket_depth_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [
                width_bits,
                depth_bits,
                height_bits,
                center_x_bits,
                center_y_bits,
                radius_bits,
                pocket_depth_bits,
            ]
            .into_iter()
            .map(parse_bits)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| <[u64; 7]>::try_from(values).ok()) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p3_circular_cut_response(
                backend,
                bits[..6]
                    .try_into()
                    .expect("bounded circular pocket request"),
                Some(f64::from_bits(bits[6])),
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (
            operation @ (Some("EXTRUDE_CIRCULAR_CUT_P3_V1")
            | Some("EXTRUDE_CIRCULAR_UNION_P3_V1")
            | Some("EXTRUDE_CIRCULAR_INTERSECT_P3_V1")
            | Some("EXTRUDE_CIRCULAR_SPLIT_P3_V1")),
            Some(width_bits),
            Some(depth_bits),
        ) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                center_x_bits,
                center_y_bits,
                radius_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [
                width_bits,
                depth_bits,
                height_bits,
                center_x_bits,
                center_y_bits,
                radius_bits,
            ]
            .into_iter()
            .map(parse_bits)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| <[u64; 6]>::try_from(values).ok()) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(if operation == Some("EXTRUDE_CIRCULAR_UNION_P3_V1") {
                p3_circular_boolean_response(
                    backend,
                    bits,
                    document_id,
                    producer_feature_id,
                    request_digest,
                    true,
                )
            } else if operation == Some("EXTRUDE_CIRCULAR_INTERSECT_P3_V1") {
                p3_circular_boolean_response(
                    backend,
                    bits,
                    document_id,
                    producer_feature_id,
                    request_digest,
                    false,
                )
            } else if operation == Some("EXTRUDE_CIRCULAR_SPLIT_P3_V1") {
                p3_circular_split_response(
                    backend,
                    bits,
                    document_id,
                    producer_feature_id,
                    request_digest,
                )
            } else {
                p3_circular_cut_response(
                    backend,
                    bits,
                    None,
                    document_id,
                    producer_feature_id,
                    request_digest,
                )
            })
        }
        (Some("OFFSET_CIRCLE_P6_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                request_digest,
                center_x_bits,
                center_y_bits,
                radius_bits,
                distance_bits,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let canonical_decimal = |value: &str| {
                value
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|parsed| parsed.to_string() == value)
            };
            let parse_bits = |value: &str| {
                (value.len() == 16
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
                .then(|| u64::from_str_radix(value, 16).ok())
                .flatten()
            };
            let Some([center_x_bits, center_y_bits, radius_bits, distance_bits]) =
                [center_x_bits, center_y_bits, radius_bits, distance_bits]
                    .into_iter()
                    .map(|value| parse_bits(value))
                    .collect::<Option<Vec<_>>>()
                    .and_then(|values| <[u64; 4]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if !canonical_decimal(document_id)
                || !canonical_decimal(producer_feature_id)
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p6_profile_offset_response(
                backend,
                PlanarProfileLoop::Circle {
                    center_mm: [f64::from_bits(center_x_bits), f64::from_bits(center_y_bits)],
                    radius_mm: f64::from_bits(radius_bits),
                },
                f64::from_bits(distance_bits),
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("OFFSET_REGION_P6_V3"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [request_digest, distance_bits, loop_count, payload @ ..] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let canonical_decimal = |value: &str| {
                value
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|parsed| parsed.to_string() == value)
            };
            let parse_bits = |value: &str| {
                (value.len() == 16
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
                .then(|| u64::from_str_radix(value, 16).ok())
                .flatten()
            };
            let Some(loop_count) = loop_count.parse::<usize>().ok().filter(|count| {
                count.to_string() == *loop_count
                    && (2..=MAX_EXACT_BREP_REGION_HOLES + 1).contains(count)
            }) else {
                return Some("ERR invalid_request".to_owned());
            };
            let Some(distance_bits) = parse_bits(distance_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if !canonical_decimal(document_id)
                || !canonical_decimal(producer_feature_id)
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            let mut cursor = 0_usize;
            let mut total_segments = 0_usize;
            let mut loops = Vec::with_capacity(loop_count);
            for _ in 0..loop_count {
                let Some(segment_count) = payload.get(cursor).and_then(|value| {
                    value.parse::<usize>().ok().filter(|count| {
                        count.to_string() == **value
                            && (1..=MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS).contains(count)
                    })
                }) else {
                    return Some("ERR invalid_request".to_owned());
                };
                cursor += 1;
                total_segments = match total_segments.checked_add(segment_count) {
                    Some(total) if total <= MAX_EXACT_BREP_REGION_SEGMENTS => total,
                    _ => return Some("ERR invalid_request".to_owned()),
                };
                let Some(record_end) = cursor.checked_add(segment_count * 10) else {
                    return Some("ERR invalid_request".to_owned());
                };
                let Some(records) = payload.get(cursor..record_end) else {
                    return Some("ERR invalid_request".to_owned());
                };
                cursor = record_end;
                let mut circle = None;
                let mut segments = Vec::with_capacity(segment_count);
                for record in records.chunks_exact(10) {
                    let Some(bits) = record[1..9]
                        .iter()
                        .map(|value| parse_bits(value))
                        .collect::<Option<Vec<_>>>()
                        .and_then(|values| <[u64; 8]>::try_from(values).ok())
                    else {
                        return Some("ERR invalid_parameter".to_owned());
                    };
                    match (record[0], record[9]) {
                        ("R", "0")
                            if segment_count == 1
                                && bits[3..] == [0, 0, 0, 0, 0]
                                && circle.is_none() =>
                        {
                            circle = Some(PlanarProfileLoop::Circle {
                                center_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                                radius_mm: f64::from_bits(bits[2]),
                            });
                        }
                        ("L", "0") if bits[4..] == [0, 0, 0, 0] && circle.is_none() => {
                            segments.push(PlanarProfileSegment::Line {
                                start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                                end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                            });
                        }
                        ("A", clockwise @ ("0" | "1"))
                            if bits[6..] == [0, 0] && circle.is_none() =>
                        {
                            segments.push(PlanarProfileSegment::CircularArc {
                                start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                                end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                                center_mm: [f64::from_bits(bits[4]), f64::from_bits(bits[5])],
                                clockwise: clockwise == "1",
                            });
                        }
                        ("C", "0") if circle.is_none() => {
                            segments.push(PlanarProfileSegment::CubicBezier {
                                start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                                end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                                control_1_mm: [f64::from_bits(bits[4]), f64::from_bits(bits[5])],
                                control_2_mm: [f64::from_bits(bits[6]), f64::from_bits(bits[7])],
                            });
                        }
                        _ => return Some("ERR invalid_request".to_owned()),
                    }
                }
                if let Some(circle) = circle {
                    loops.push(circle);
                } else if segments.len() >= 2 {
                    loops.push(PlanarProfileLoop::Segments(segments));
                } else {
                    return Some("ERR invalid_request".to_owned());
                }
            }
            if cursor != payload.len() {
                return Some("ERR invalid_request".to_owned());
            }
            let outer = loops.remove(0);
            Some(p6_region_offset_response(
                backend,
                outer,
                loops,
                f64::from_bits(distance_bits),
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (
            Some(protocol @ ("OFFSET_PROFILE_P6_V1" | "OFFSET_PROFILE_P6_V2")),
            Some(document_id),
            Some(producer_feature_id),
        ) => {
            let remaining = fields.collect::<Vec<_>>();
            let Some((request_digest, remaining)) = remaining.split_first() else {
                return Some("ERR invalid_request".to_owned());
            };
            let [distance_bits, segment_count, payload @ ..] = remaining else {
                return Some("ERR invalid_request".to_owned());
            };
            let canonical_decimal = |value: &str| {
                value
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|parsed| parsed.to_string() == value)
            };
            let parse_bits = |value: &str| {
                (value.len() == 16
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
                .then(|| u64::from_str_radix(value, 16).ok())
                .flatten()
            };
            let Some(count) = segment_count
                .parse::<usize>()
                .ok()
                .filter(|count| count.to_string() == *segment_count && (2..=64).contains(count))
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let record_width = if protocol == "OFFSET_PROFILE_P6_V2" {
                10
            } else {
                8
            };
            if !canonical_decimal(document_id)
                || !canonical_decimal(producer_feature_id)
                || !is_canonical_digest(request_digest)
                || payload.len() != count * record_width
            {
                return Some("ERR invalid_request".to_owned());
            }
            let Some(distance_bits) = parse_bits(distance_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let mut segments = Vec::with_capacity(count);
            for record in payload.chunks_exact(record_width) {
                let segment = if record_width == 8 {
                    let Some(bits) = record[1..7]
                        .iter()
                        .map(|value| parse_bits(value))
                        .collect::<Option<Vec<_>>>()
                        .and_then(|values| <[u64; 6]>::try_from(values).ok())
                    else {
                        return Some("ERR invalid_parameter".to_owned());
                    };
                    match (record[0], record[7]) {
                        ("L", "0") if bits[4..] == [0, 0] => PlanarProfileSegment::Line {
                            start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                            end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                        },
                        ("A", clockwise @ ("0" | "1")) => PlanarProfileSegment::CircularArc {
                            start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                            end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                            center_mm: [f64::from_bits(bits[4]), f64::from_bits(bits[5])],
                            clockwise: clockwise == "1",
                        },
                        _ => return Some("ERR invalid_request".to_owned()),
                    }
                } else {
                    let Some(bits) = record[1..9]
                        .iter()
                        .map(|value| parse_bits(value))
                        .collect::<Option<Vec<_>>>()
                        .and_then(|values| <[u64; 8]>::try_from(values).ok())
                    else {
                        return Some("ERR invalid_parameter".to_owned());
                    };
                    match (record[0], record[9]) {
                        ("L", "0") if bits[4..] == [0, 0, 0, 0] => PlanarProfileSegment::Line {
                            start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                            end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                        },
                        ("A", clockwise @ ("0" | "1")) if bits[6..] == [0, 0] => {
                            PlanarProfileSegment::CircularArc {
                                start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                                end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                                center_mm: [f64::from_bits(bits[4]), f64::from_bits(bits[5])],
                                clockwise: clockwise == "1",
                            }
                        }
                        ("C", "0") => PlanarProfileSegment::CubicBezier {
                            start_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
                            end_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
                            control_1_mm: [f64::from_bits(bits[4]), f64::from_bits(bits[5])],
                            control_2_mm: [f64::from_bits(bits[6]), f64::from_bits(bits[7])],
                        },
                        _ => return Some("ERR invalid_request".to_owned()),
                    }
                };
                segments.push(segment);
            }
            Some(p6_profile_offset_response(
                backend,
                PlanarProfileLoop::Segments(segments),
                f64::from_bits(distance_bits),
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("OFFSET_RECTANGLE_P6_V1"), Some(min_x_bits), Some(min_y_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                max_x_bits,
                max_y_bits,
                distance_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [
                min_x_bits,
                min_y_bits,
                max_x_bits,
                max_y_bits,
                distance_bits,
            ]
            .into_iter()
            .map(parse_bits)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| <[u64; 5]>::try_from(values).ok()) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p6_offset_response(
                backend,
                bits,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("LOFT_SPLINE_P6_V1"), Some(document_id), Some(producer_feature_id)) => {
            let remaining = fields.collect::<Vec<_>>();
            let Some((request_digest, payload)) = remaining.split_first() else {
                return Some("ERR invalid_request".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            let Some(values) = payload
                .iter()
                .map(|value| u64::from_str_radix(value, 16).map(f64::from_bits))
                .collect::<Result<Vec<_>, _>>()
                .ok()
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            Some(p6_loft_response(
                backend,
                &values,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("SWEEP_RECTANGLE_P6_V1"), Some(min_u_bits), Some(min_v_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                max_u_bits,
                max_v_bits,
                path_start_x_bits,
                path_start_y_bits,
                path_end_x_bits,
                path_end_y_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Some(bits) = [
                min_u_bits,
                min_v_bits,
                max_u_bits,
                max_v_bits,
                path_start_x_bits,
                path_start_y_bits,
                path_end_x_bits,
                path_end_y_bits,
            ]
            .into_iter()
            .map(parse_bits)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| <[u64; 8]>::try_from(values).ok()) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p6_sweep_response(
                backend,
                bits,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE"), Some(height_bits), None) => {
            let Ok(height_bits) = u64::from_str_radix(height_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            Some(legacy_extrude_response(backend, height_bits))
        }
        (Some("EXTRUDE_M3_V1"), Some(width_bits), Some(depth_bits)) => {
            let Some((height_bits, document_id, producer_feature_id, request_digest)) = fields
                .next()
                .zip(fields.next())
                .zip(fields.next())
                .zip(fields.next())
                .map(|(((height, document), producer), digest)| {
                    (height, document, producer, digest)
                })
            else {
                return Some("ERR invalid_request".to_owned());
            };
            if fields.next().is_some() {
                return Some("ERR invalid_request".to_owned());
            }
            let Ok(width_bits) = u64::from_str_radix(width_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(depth_bits) = u64::from_str_radix(depth_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(height_bits) = u64::from_str_radix(height_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(m3_extrude_response(
                backend,
                width_bits,
                depth_bits,
                height_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_M3_V2"), Some(width_bits), Some(depth_bits)) => {
            let Some((height_bits, document_id, producer_feature_id, request_digest)) = fields
                .next()
                .zip(fields.next())
                .zip(fields.next())
                .zip(fields.next())
                .map(|(((height, document), producer), digest)| {
                    (height, document, producer, digest)
                })
            else {
                return Some("ERR invalid_request".to_owned());
            };
            if fields.next().is_some() {
                return Some("ERR invalid_request".to_owned());
            }
            let Ok(width_bits) = u64::from_str_radix(width_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(depth_bits) = u64::from_str_radix(depth_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(height_bits) = u64::from_str_radix(height_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(m3_extrude_planar_face_response(
                backend,
                width_bits,
                depth_bits,
                height_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_CUT_M3_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                cut_x_bits,
                cut_y_bits,
                cut_width_bits,
                cut_depth_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let Ok(width_bits) = parse_bits(width_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(depth_bits) = parse_bits(depth_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(height_bits) = parse_bits(height_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(cut_x_bits) = parse_bits(cut_x_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(cut_y_bits) = parse_bits(cut_y_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(cut_width_bits) = parse_bits(cut_width_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            let Ok(cut_depth_bits) = parse_bits(cut_depth_bits) else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(m3_through_cut_response(
                backend,
                [width_bits, depth_bits, height_bits],
                [cut_x_bits, cut_y_bits, cut_width_bits, cut_depth_bits],
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_POCKET_M3_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                pocket_x_bits,
                pocket_y_bits,
                pocket_width_bits,
                pocket_plan_depth_bits,
                pocket_depth_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let values = [
                width_bits,
                depth_bits,
                height_bits,
                pocket_x_bits,
                pocket_y_bits,
                pocket_width_bits,
                pocket_plan_depth_bits,
                pocket_depth_bits,
            ];
            let Some(bits) = values
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 8]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(m3_pocket_response(
                backend,
                [bits[0], bits[1], bits[2]],
                [bits[3], bits[4], bits[5], bits[6], bits[7]],
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_INTERSECT_P6_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                tool_x_bits,
                tool_y_bits,
                tool_width_bits,
                tool_depth_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let values = [
                width_bits,
                depth_bits,
                height_bits,
                tool_x_bits,
                tool_y_bits,
                tool_width_bits,
                tool_depth_bits,
            ];
            let Some(bits) = values
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 7]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p6_intersect_response(
                backend,
                [bits[0], bits[1], bits[2]],
                [bits[3], bits[4], bits[5], bits[6]],
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_SPLIT_P6_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                tool_x_bits,
                tool_y_bits,
                tool_width_bits,
                tool_depth_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let values = [
                width_bits,
                depth_bits,
                height_bits,
                tool_x_bits,
                tool_y_bits,
                tool_width_bits,
                tool_depth_bits,
            ];
            let Some(bits) = values
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 7]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(p6_split_response(
                backend,
                [bits[0], bits[1], bits[2]],
                [bits[3], bits[4], bits[5], bits[6]],
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_UNION_M3_V1"), Some(width_bits), Some(depth_bits)) => {
            let remaining = fields.collect::<Vec<_>>();
            let [
                height_bits,
                tool_x_bits,
                tool_y_bits,
                tool_width_bits,
                tool_depth_bits,
                document_id,
                producer_feature_id,
                request_digest,
            ] = remaining.as_slice()
            else {
                return Some("ERR invalid_request".to_owned());
            };
            let parse_bits = |value: &str| u64::from_str_radix(value, 16);
            let values = [
                width_bits,
                depth_bits,
                height_bits,
                tool_x_bits,
                tool_y_bits,
                tool_width_bits,
                tool_depth_bits,
            ];
            let Some(bits) = values
                .into_iter()
                .map(parse_bits)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .and_then(|values| <[u64; 7]>::try_from(values).ok())
            else {
                return Some("ERR invalid_parameter".to_owned());
            };
            if document_id.parse::<u64>().is_err()
                || producer_feature_id.parse::<u64>().is_err()
                || !is_canonical_digest(request_digest)
            {
                return Some("ERR invalid_request".to_owned());
            }
            Some(m3_union_response(
                backend,
                [bits[0], bits[1], bits[2]],
                [bits[3], bits[4], bits[5], bits[6]],
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXCEPTION"), None, None) => match backend.exception_probe() {
            Ok(_) => Some("ERR unexpected_success".to_owned()),
            Err(error) => Some(format!("ERR {}", error.code.as_str())),
        },
        (Some("SLEEP"), Some(milliseconds), None) => {
            let Ok(milliseconds) = milliseconds.parse::<u64>() else {
                return Some("ERR invalid_parameter".to_owned());
            };
            std::thread::sleep(Duration::from_millis(milliseconds));
            Some("DONE".to_owned())
        }
        (Some("CRASH"), None, None) => std::process::abort(),
        _ => Some("ERR invalid_request".to_owned()),
    }
}

const MAX_EXACT_BREP_GRAPH_MESH_TRIANGLES: u32 = 200_000;

fn exact_brep_graph_schema_matches_operation(operation: &str, schema: &str) -> bool {
    match operation {
        "EVAL_BREP_GRAPH_V6" | "TESSELLATE_BREP_GRAPH_V6" => schema == EXACT_BREP_GRAPH_SCHEMA_V6,
        "EVAL_BREP_GRAPH_V7" | "TESSELLATE_BREP_GRAPH_V7" => schema == EXACT_BREP_GRAPH_SCHEMA_V7,
        "EVAL_BREP_GRAPH_V8" | "TESSELLATE_BREP_GRAPH_V8" => schema == EXACT_BREP_GRAPH_SCHEMA_V8,
        "EVAL_BREP_GRAPH_V9" | "TESSELLATE_BREP_GRAPH_V9" => schema == EXACT_BREP_GRAPH_SCHEMA_V9,
        "EVAL_BREP_GRAPH_V10" | "TESSELLATE_BREP_GRAPH_V10" => {
            schema == EXACT_BREP_GRAPH_SCHEMA_V10
        }
        "EVAL_BREP_GRAPH_V11" | "TESSELLATE_BREP_GRAPH_V11" => {
            schema == EXACT_BREP_GRAPH_SCHEMA_V11
        }
        "EVAL_BREP_GRAPH_V12" | "TESSELLATE_BREP_GRAPH_V12" => {
            schema == EXACT_BREP_GRAPH_SCHEMA_V12
        }
        "EVAL_BREP_GRAPH_V13" | "TESSELLATE_BREP_GRAPH_V13" => {
            schema == EXACT_BREP_GRAPH_SCHEMA_V13
        }
        "EXPORT_BREP_GRAPH_STEP_V2" => matches!(
            schema,
            EXACT_BREP_GRAPH_SCHEMA_V6
                | EXACT_BREP_GRAPH_SCHEMA_V7
                | EXACT_BREP_GRAPH_SCHEMA_V8
                | EXACT_BREP_GRAPH_SCHEMA_V9
                | EXACT_BREP_GRAPH_SCHEMA_V10
                | EXACT_BREP_GRAPH_SCHEMA_V11
        ),
        "EXPORT_BREP_GRAPH_STEP_V3" => schema == EXACT_BREP_GRAPH_SCHEMA_V12,
        "EXPORT_BREP_GRAPH_STEP_V4" => schema == EXACT_BREP_GRAPH_SCHEMA_V13,
        _ => false,
    }
}

fn decode_exact_brep_graph(
    operation: &str,
    graph_digest: &str,
    encoded_graph: &str,
) -> Option<ExactBRepGraph> {
    if !is_canonical_digest(graph_digest) {
        return None;
    }
    let bytes = decode_hex_bytes(encoded_graph)?;
    let graph = ExactBRepGraph::from_bytes(&bytes).ok()?;
    (exact_brep_graph_schema_matches_operation(operation, &graph.schema)
        && graph.graph_digest == graph_digest)
        .then_some(graph)
}

fn verified_exact_brep_graph_sources(
    graph: &ExactBRepGraph,
    fields: &[&str],
) -> Result<Vec<(String, tempfile::NamedTempFile)>, String> {
    let mut expected = BTreeMap::<String, u64>::new();
    let mut source_order = Vec::new();
    for node in &graph.nodes {
        let ExactBRepOperation::ImportedExact {
            source_sha256,
            source_byte_len,
            ..
        } = &node.operation
        else {
            continue;
        };
        let source_sha256 = source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if let Some(previous_len) = expected.get(&source_sha256) {
            if previous_len != source_byte_len {
                return Err("ERR invalid_request".to_owned());
            }
        } else {
            expected.insert(source_sha256.clone(), *source_byte_len);
            source_order.push(source_sha256);
        }
    }
    if source_order.len() > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES {
        return Err("ERR invalid_request".to_owned());
    }
    let source_fields = match fields {
        [] => &[][..],
        [source_sha256, source_path] if is_canonical_digest(source_sha256) => &fields[..2],
        [count, remaining @ ..] => {
            let Ok(count) = count.parse::<usize>() else {
                return Err("ERR invalid_request".to_owned());
            };
            if count > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES
                || remaining.len() != count.saturating_mul(2)
            {
                return Err("ERR invalid_request".to_owned());
            }
            remaining
        }
    };
    if source_fields.len() / 2 != source_order.len() {
        return Err("ERR invalid_request".to_owned());
    }
    let mut total_bytes = 0_u64;
    let mut sources = Vec::with_capacity(source_order.len());
    for (expected_sha256, fields) in source_order.iter().zip(source_fields.chunks_exact(2)) {
        if fields[0] != expected_sha256 || !is_canonical_digest(fields[0]) {
            return Err("ERR invalid_request".to_owned());
        }
        let Some(source_path) = decode_hex_utf8(fields[1]) else {
            return Err("ERR invalid_request".to_owned());
        };
        let expected_byte_len = expected[expected_sha256];
        total_bytes = total_bytes
            .checked_add(expected_byte_len)
            .ok_or_else(|| "ERR invalid_request".to_owned())?;
        if expected_byte_len > MAX_STEP_SOURCE_BYTES
            || total_bytes > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES
        {
            return Err("ERR invalid_request".to_owned());
        }
        let source =
            verified_step_copy(&source_path, expected_sha256, "evaluate_exact_brep_graph")?;
        if source
            .as_file()
            .metadata()
            .map_err(|error| {
                transport_error_response("evaluate_exact_brep_graph", &error.to_string())
            })?
            .len()
            != expected_byte_len
        {
            return Err(transport_error_response(
                "evaluate_exact_brep_graph",
                "STEP source byte length does not match the exact graph identity",
            ));
        }
        sources.push((expected_sha256.clone(), source));
    }
    Ok(sources)
}

fn exact_brep_graph_mesh_response(
    backend: &ExactBackend,
    operation: &str,
    graph_digest: &str,
    encoded_graph: &str,
    fields: &[&str],
) -> String {
    if fields.len() < 2 || !is_result_fingerprint(fields[0]) {
        return "ERR invalid_request".to_owned();
    }
    let Some(graph) = decode_exact_brep_graph(operation, graph_digest, encoded_graph) else {
        return "ERR invalid_request".to_owned();
    };
    let Some(output_path) = decode_hex_utf8(fields[1]) else {
        return "ERR invalid_request".to_owned();
    };
    let sources = match verified_exact_brep_graph_sources(&graph, &fields[2..]) {
        Ok(sources) => sources,
        Err(response) => return response,
    };
    let output = match evaluate_exact_brep_graph(backend, &graph, &sources) {
        Ok(output) if output.body.result_fingerprint == fields[0] => output,
        Ok(_) => return "ERR invalid_result".to_owned(),
        Err(error) => return geometry_error_response(&error),
    };
    let bounds = output.body.topology.bounds_mm;
    let diagonal = ((bounds.max.x - bounds.min.x).powi(2)
        + (bounds.max.y - bounds.min.y).powi(2)
        + (bounds.max.z - bounds.min.z).powi(2))
    .sqrt();
    if !diagonal.is_finite() || diagonal <= 0.0 {
        return "ERR invalid_shape".to_owned();
    }
    let deflection = (diagonal * 1.0e-3).max(1.0e-3);
    let mesh = match backend.tessellate_body(
        &output.body,
        deflection,
        STEP_MESH_ANGULAR_DEFLECTION,
        MAX_EXACT_BREP_GRAPH_MESH_TRIANGLES,
    ) {
        Ok(mesh) => StepImportMesh {
            vertices_mm: mesh.vertices_mm,
            triangles: mesh
                .triangles
                .into_iter()
                .map(|triangle| StepMeshTriangle {
                    vertex_indices: triangle.vertex_indices,
                    face_ordinal: triangle.face_ordinal,
                })
                .collect(),
        },
        Err(error) => return geometry_error_response(&error),
    };
    let encoded = mesh.encode();
    if let Err(error) = std::fs::write(&output_path, &encoded) {
        return transport_error_response("tessellate_exact_brep_graph", &error.to_string());
    }
    format!(
        "OK_BREP_GRAPH_MESH_V1 {graph_digest} {} {} {} {} {:016x}",
        output.body.result_fingerprint,
        mesh.vertices_mm.len(),
        mesh.triangles.len(),
        sha256_hex(&encoded),
        deflection.to_bits(),
    )
}

fn exact_brep_graph_step_response(
    backend: &ExactBackend,
    operation: &str,
    graph_digest: &str,
    encoded_graph: &str,
    fields: &[&str],
) -> String {
    if fields.len() < 2 || !is_result_fingerprint(fields[0]) {
        return "ERR invalid_request".to_owned();
    }
    let Some(graph) = decode_exact_brep_graph(operation, graph_digest, encoded_graph) else {
        return "ERR invalid_request".to_owned();
    };
    let Some(output_path) = decode_hex_utf8(fields[1]) else {
        return "ERR invalid_request".to_owned();
    };
    let sources = match verified_exact_brep_graph_sources(&graph, &fields[2..]) {
        Ok(sources) => sources,
        Err(response) => return response,
    };
    let output = match evaluate_exact_brep_graph(backend, &graph, &sources) {
        Ok(output) if output.body.result_fingerprint == fields[0] => output,
        Ok(_) => return "ERR invalid_result".to_owned(),
        Err(error) => return geometry_error_response(&error),
    };
    let response_protocol = match operation {
        "EXPORT_BREP_GRAPH_STEP_V2" => "OK_BREP_GRAPH_STEP_V2",
        "EXPORT_BREP_GRAPH_STEP_V3" => "OK_BREP_GRAPH_STEP_V3",
        "EXPORT_BREP_GRAPH_STEP_V4" => "OK_BREP_GRAPH_STEP_V4",
        _ => return "ERR invalid_request".to_owned(),
    };
    match backend.export_step(&output.body, &output_path) {
        Ok(()) => format!(
            "{response_protocol} {graph_digest} {}",
            output.body.result_fingerprint
        ),
        Err(error) => geometry_error_response(&error),
    }
}

fn exact_brep_graph_response(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    source_fields: &[&str],
) -> String {
    let sources = match verified_exact_brep_graph_sources(graph, source_fields) {
        Ok(sources) => sources,
        Err(response) => return response,
    };
    let output = match evaluate_exact_brep_graph(backend, graph, &sources) {
        Ok(output) => output,
        Err(error) => return geometry_error_response(&error),
    };
    let topology = &output.body.topology;
    let area_mm2 = if graph.terminal_is_planar_offset() {
        if topology.face_count != 1 || topology.faces.len() != 1 {
            return "ERR invalid_shape".to_owned();
        }
        topology.faces[0].area_mm2
    } else {
        0.0
    };
    let (protocol, topology_evidence) = if matches!(
        graph.schema.as_str(),
        EXACT_BREP_GRAPH_SCHEMA_V8
            | EXACT_BREP_GRAPH_SCHEMA_V9
            | EXACT_BREP_GRAPH_SCHEMA_V10
            | EXACT_BREP_GRAPH_SCHEMA_V11
            | EXACT_BREP_GRAPH_SCHEMA_V12
            | EXACT_BREP_GRAPH_SCHEMA_V13
    ) {
        (
            match graph.schema.as_str() {
                EXACT_BREP_GRAPH_SCHEMA_V13 => "OK_BREP_GRAPH_V13",
                EXACT_BREP_GRAPH_SCHEMA_V12 => "OK_BREP_GRAPH_V12",
                EXACT_BREP_GRAPH_SCHEMA_V11 => "OK_BREP_GRAPH_V11",
                EXACT_BREP_GRAPH_SCHEMA_V10 => "OK_BREP_GRAPH_V10",
                EXACT_BREP_GRAPH_SCHEMA_V9 => "OK_BREP_GRAPH_V9",
                _ => "OK_BREP_GRAPH_V8",
            },
            format!(
                "{} {} {} {} {} {}",
                topology.vertex_count,
                topology.edge_count,
                topology.wire_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
            ),
        )
    } else {
        (
            "OK_BREP_GRAPH_V6",
            format!(
                "{} {} {} {} {}",
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
            ),
        )
    };
    format!(
        "{protocol} {} {} {} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {topology_evidence} {} {}",
        graph.canonical_input_digest,
        graph.graph_digest,
        graph.producer_feature_id,
        output.body.result_fingerprint,
        output.input_digest,
        topology.volume_mm3.to_bits(),
        area_mm2.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
        encode_hex(output.backend_fingerprint.as_bytes()),
        encode_hex(output.tolerance_report.profile.as_bytes()),
    )
}

fn evaluate_exact_brep_graph(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    imported_sources: &[(String, tempfile::NamedTempFile)],
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    graph
        .validate()
        .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
    let mut outputs = Vec::<ExactOpOutput>::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let output = match &node.operation {
            ExactBRepOperation::Extrude {
                profile, interval, ..
            } => exact_brep_profile_body(backend, &graph.profiles[profile.0 as usize], *interval)?,
            ExactBRepOperation::ProfileCut {
                target,
                profile,
                interval,
                ..
            } => {
                let target = &outputs[target.0 as usize].body;
                let tool = exact_brep_profile_body(
                    backend,
                    &graph.profiles[profile.0 as usize],
                    *interval,
                )?;
                backend.boolean_bodies(target, &tool.body, ExactBodyBooleanOperation::Cut)?
            }
            ExactBRepOperation::RigidTransform {
                target,
                matrix_bits,
            } => backend.transform_body(
                &outputs[target.0 as usize].body,
                &matrix_bits.map(f64::from_bits),
            )?,
            ExactBRepOperation::Boolean {
                operation,
                target,
                tool,
            } => {
                let target = &outputs[target.0 as usize].body;
                let tool = &outputs[tool.0 as usize].body;
                backend.boolean_bodies(
                    target,
                    tool,
                    match operation {
                        ExactBRepBooleanOperation::Cut => ExactBodyBooleanOperation::Cut,
                        ExactBRepBooleanOperation::Union => ExactBodyBooleanOperation::Union,
                        ExactBRepBooleanOperation::Intersect => {
                            ExactBodyBooleanOperation::Intersect
                        }
                        ExactBRepBooleanOperation::Split => ExactBodyBooleanOperation::Split,
                    },
                )?
            }
            ExactBRepOperation::Revolve {
                profile,
                axis_start_bits,
                axis_end_bits,
                angle_degrees_bits,
            } => exact_brep_revolve(
                backend,
                &graph.profiles[profile.0 as usize],
                axis_start_bits.map(f64::from_bits),
                axis_end_bits.map(f64::from_bits),
                f64::from_bits(*angle_degrees_bits),
            )?,
            ExactBRepOperation::PlanarOffset {
                profile,
                distance_bits,
            } => exact_brep_planar_offset(
                backend,
                &graph.profiles[profile.0 as usize],
                f64::from_bits(*distance_bits),
            )?,
            ExactBRepOperation::Sweep { profile, path } => exact_brep_sweep(
                backend,
                &graph.profiles[profile.0 as usize],
                &graph.profiles[path.0 as usize],
                false,
            )?,
            ExactBRepOperation::SketchSweep { profile, path } => exact_brep_sweep(
                backend,
                &graph.profiles[profile.0 as usize],
                &graph.profiles[path.0 as usize],
                true,
            )?,
            ExactBRepOperation::SpatialSweep { profile, path } => exact_brep_spatial_sweep(
                backend,
                &graph.profiles[profile.0 as usize],
                &path.segments,
            )?,
            ExactBRepOperation::Loft { sections } => {
                let sections = sections
                    .iter()
                    .map(|section| {
                        let profile = &graph.profiles[section.profile.0 as usize];
                        let ExactBRepPlanarGeometry::Spline { control_point_bits } =
                            &profile.geometry
                        else {
                            return Err(exact_brep_profile_error(
                                profile,
                                "exact loft requires bounded spline sections",
                            ));
                        };
                        Ok(SplineLoftSection {
                            elevation_mm: f64::from_bits(section.elevation_bits),
                            control_points_mm: control_point_bits
                                .iter()
                                .map(|point| point.map(f64::from_bits))
                                .collect(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                backend.loft_spline(&SplineLoftSpec { sections })?
            }
            ExactBRepOperation::Shell {
                target,
                removed_faces,
                thickness_bits,
            } => {
                let target_output = &outputs[target.0 as usize];
                let ordinals = exact_brep_topology_ordinals(
                    graph,
                    *target,
                    target_output,
                    removed_faces,
                    ExactBRepTopologyKind::Face,
                )?;
                backend.shell_body(
                    &target_output.body,
                    &ordinals,
                    f64::from_bits(*thickness_bits),
                )?
            }
            ExactBRepOperation::FaceOffset {
                target,
                face,
                distance_bits,
            } => {
                let target_output = &outputs[target.0 as usize];
                let [ordinal] = exact_brep_topology_ordinals(
                    graph,
                    *target,
                    target_output,
                    std::slice::from_ref(face),
                    ExactBRepTopologyKind::Face,
                )?
                .try_into()
                .map_err(|_| exact_brep_graph_error(graph, "face offset selector is invalid"))?;
                backend.offset_body_face(
                    &target_output.body,
                    ordinal,
                    f64::from_bits(*distance_bits),
                )?
            }
            ExactBRepOperation::EdgeFinish {
                target,
                edges,
                kind,
                amount_bits,
            } => {
                let target_output = &outputs[target.0 as usize];
                let ordinals = exact_brep_topology_ordinals(
                    graph,
                    *target,
                    target_output,
                    edges,
                    ExactBRepTopologyKind::Edge,
                )?;
                backend.finish_body(
                    &target_output.body,
                    &ordinals,
                    match kind {
                        ExactBRepEdgeFinishKind::Fillet => EdgeFinish::Fillet,
                        ExactBRepEdgeFinishKind::Chamfer => EdgeFinish::Chamfer,
                    },
                    f64::from_bits(*amount_bits),
                )?
            }
            ExactBRepOperation::ImportedExact {
                source_sha256,
                result_fingerprint,
                ..
            } => {
                let source_sha256 = source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let source = imported_sources
                    .iter()
                    .find_map(|(digest, source)| {
                        (digest == &source_sha256).then_some(source.path())
                    })
                    .ok_or_else(|| {
                        exact_brep_graph_error(graph, "imported exact graph source is unavailable")
                    })?;
                let mut output = backend.import_step(&source.to_string_lossy())?;
                if step_import_result_fingerprint(&source_sha256, &output) != *result_fingerprint {
                    return Err(exact_brep_graph_error(
                        graph,
                        "imported exact graph result fingerprint does not match its source",
                    ));
                }
                output.body.result_fingerprint = result_fingerprint.clone();
                output.input_digest = source_sha256;
                output
            }
        };
        outputs.push(output);
    }
    outputs
        .pop()
        .ok_or_else(|| exact_brep_graph_error(graph, "graph produced no exact body"))
}

fn exact_brep_topology_ordinals(
    graph: &ExactBRepGraph,
    target: ketchup_core::exact_brep_graph::ExactBRepNodeId,
    target_output: &ExactOpOutput,
    selectors: &[ExactBRepTopologySelector],
    expected_kind: ExactBRepTopologyKind,
) -> Result<Vec<u32>, ketchup_exact::GeometryError> {
    let target_node = &graph.nodes[target.0 as usize];
    let kind_token = match expected_kind {
        ExactBRepTopologyKind::Face => "face",
        ExactBRepTopologyKind::Edge => "edge",
    };
    let (producer_prefix, source_prefix, expected_evaluator, expected_result_fingerprint) =
        match &target_node.operation {
            ExactBRepOperation::ImportedExact {
                source_sha256,
                result_fingerprint,
                ..
            } => {
                let source_sha256 = source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                (
                    format!("imported-result/{kind_token}/"),
                    format!("imported-source/{source_sha256}/{kind_token}/"),
                    "ketchup.imported-step-evaluator.v1",
                    result_fingerprint.as_str(),
                )
            }
            _ => (
                format!("generated-result/{kind_token}/"),
                format!("generated-source/{kind_token}/"),
                EXACT_BREP_GRAPH_EVALUATOR_V1,
                target_output.body.result_fingerprint.as_str(),
            ),
        };
    let mut ordinals = selectors
        .iter()
        .map(|selector| {
            if selector.kind != expected_kind {
                return Err(exact_brep_graph_error(
                    graph,
                    "topology selector has the wrong element kind",
                ));
            }
            let reference = selector
                .reference()
                .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
            if reference.producer_feature_id.0 != target_node.source_feature_id
                || reference.source_feature_id.0 != target_node.source_feature_id
                || reference.result_fingerprint != expected_result_fingerprint
                || reference.evaluator != expected_evaluator
                || reference.backend != target_output.backend_fingerprint
                || reference.tolerance != target_output.tolerance_report.profile
            {
                return Err(exact_brep_graph_error(
                    graph,
                    "topology selector is stale or belongs to a different exact target",
                ));
            }
            let ordinal_token = reference
                .producer_element_id
                .strip_prefix(&producer_prefix)
                .ok_or_else(|| {
                    exact_brep_graph_error(graph, "topology selector producer token is invalid")
                })?;
            let ordinal = ordinal_token.parse::<u32>().map_err(|_| {
                exact_brep_graph_error(graph, "topology selector ordinal is invalid")
            })?;
            if ordinal_token != ordinal.to_string()
                || reference.source_element_id != format!("{source_prefix}{ordinal}")
            {
                return Err(exact_brep_graph_error(
                    graph,
                    "topology selector ordinal encoding is non-canonical",
                ));
            }
            Ok(ordinal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    ordinals.sort_unstable();
    if ordinals.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(exact_brep_graph_error(
            graph,
            "topology selector ordinals are duplicated",
        ));
    }
    Ok(ordinals)
}

fn exact_brep_revolve(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let segments = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { .. } => exact_brep_boundary_segments(profile, true)?,
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            let positive = [center[0] + radius, center[1]];
            let negative = [center[0] - radius, center[1]];
            vec![
                PlanarProfileSegment::CircularArc {
                    start_mm: positive,
                    end_mm: negative,
                    center_mm: center,
                    clockwise: false,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm: negative,
                    end_mm: positive,
                    center_mm: center,
                    clockwise: false,
                },
            ]
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            let outer = exact_brep_planar_loop(outer);
            let holes = holes.iter().map(exact_brep_planar_loop).collect::<Vec<_>>();
            let local = backend.revolve_planar_region(
                &outer,
                &holes,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            )?;
            return transform_revolve_to_profile_frame(backend, profile, &local);
        }
        ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact revolve requires a closed line/arc/circle/cubic profile",
            ));
        }
    };
    let local =
        backend.revolve_general_profile(&segments, axis_start_mm, axis_end_mm, angle_degrees)?;
    transform_revolve_to_profile_frame(backend, profile, &local)
}

fn transform_revolve_to_profile_frame(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    local: &ExactOpOutput,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let frame = profile.frame_bits.map(f64::from_bits);
    backend.transform_body(
        &local.body,
        &[
            frame[3], frame[6], frame[9], frame[0], frame[4], frame[7], frame[10], frame[1],
            frame[5], frame[8], frame[11], frame[2], 0.0, 0.0, 0.0, 1.0,
        ],
    )
}

fn exact_brep_sweep(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    path: &ExactBRepProfile,
    sketch_sweep: bool,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let ExactBRepPlanarGeometry::Boundary {
        closed: false,
        segments,
    } = &path.geometry
    else {
        return Err(exact_brep_profile_error(
            path,
            "exact sweep requires an open planar path",
        ));
    };
    if segments.len() > 1 {
        let profile_segments = exact_brep_boundary_segments(profile, true)?;
        let path_segments = exact_brep_boundary_segments(path, false)?;
        return backend.sweep_planar_profile(&profile_segments, &path_segments);
    }
    let [
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        },
    ] = segments.as_slice()
    else {
        return Err(exact_brep_profile_error(
            path,
            "exact sweep currently requires one straight path segment",
        ));
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let direction = [end[0] - start[0], end[1] - start[1]];
    let length = direction[0].hypot(direction[1]);
    if sketch_sweep {
        let path_frame = path.frame_bits.map(f64::from_bits);
        let to_world = |point: [f64; 2]| {
            [0, 1, 2].map(|axis| {
                path_frame[axis] + path_frame[3 + axis] * point[0] + path_frame[6 + axis] * point[1]
            })
        };
        let world_start = to_world(start);
        let world_end = to_world(end);
        let world_delta = [0, 1, 2].map(|axis| world_end[axis] - world_start[axis]);
        let world_length = world_delta[0].hypot(world_delta[1]).hypot(world_delta[2]);
        let world_direction = world_delta.map(|component| component / world_length);
        let profile_frame = profile.frame_bits.map(f64::from_bits);
        let starts_at_profile = [0, 1, 2].into_iter().all(|axis| {
            (world_start[axis] - profile_frame[axis]).abs() <= SKETCH_SWEEP_FRAME_EPSILON_MM
        });
        let aligned = [0, 1, 2]
            .into_iter()
            .map(|axis| world_direction[axis] * profile_frame[9 + axis])
            .sum::<f64>()
            >= 1.0 - SKETCH_SWEEP_FRAME_EPSILON_MM;
        if !world_length.is_finite() || world_length <= 1.0e-9 || !starts_at_profile || !aligned {
            return Err(exact_brep_profile_error(
                profile,
                "Sketch Sweep requires the path to start at the profile frame origin and follow its normal",
            ));
        }
        return exact_brep_profile_body(
            backend,
            profile,
            ExactBRepLinearInterval {
                direction_bits: world_direction.map(f64::to_bits),
                start_bits: 0.0_f64.to_bits(),
                end_bits: world_length.to_bits(),
            },
        );
    }
    let tangent = [direction[0] / length, direction[1] / length];
    let section = [tangent[1], -tangent[0]];
    let local = exact_brep_profile_body(
        backend,
        profile,
        ExactBRepLinearInterval {
            direction_bits: [
                profile.frame_bits[9],
                profile.frame_bits[10],
                profile.frame_bits[11],
            ],
            start_bits: 0.0_f64.to_bits(),
            end_bits: length.to_bits(),
        },
    )?;
    backend.transform_body(
        &local.body,
        &[
            section[0], 0.0, tangent[0], start[0], section[1], 0.0, tangent[1], start[1], 0.0, 1.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    )
}

fn exact_brep_spatial_sweep(
    backend: &ExactKernel,
    profile: &ExactBRepProfile,
    path: &[ExactBRepSpatialPathSegment],
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let (outer, holes) = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { closed: true, .. }
        | ExactBRepPlanarGeometry::Circle { .. } => {
            (exact_brep_planar_offset_loop(profile)?, Vec::new())
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => (
            exact_brep_planar_loop(outer),
            holes.iter().map(exact_brep_planar_loop).collect(),
        ),
        ExactBRepPlanarGeometry::Boundary { closed: false, .. }
        | ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact spatial sweep requires a closed line/arc/circle/cubic profile or region",
            ));
        }
    };
    let path = path
        .iter()
        .map(exact_brep_spatial_path_segment)
        .collect::<Vec<_>>();
    let mut output = exact_brep_spatial_sweep_loop(backend, outer, &path)?;
    for hole in holes {
        let tool = exact_brep_spatial_sweep_loop(backend, hole, &path)?;
        output =
            backend.boolean_bodies(&output.body, &tool.body, ExactBodyBooleanOperation::Cut)?;
    }
    Ok(output)
}

fn exact_brep_spatial_sweep_loop(
    backend: &ExactKernel,
    profile: PlanarProfileLoop,
    path: &[SpatialProfileSegment],
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let segments = match profile {
        PlanarProfileLoop::Segments(segments) => segments,
        PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } => {
            let positive = [center_mm[0] + radius_mm, center_mm[1]];
            let negative = [center_mm[0] - radius_mm, center_mm[1]];
            vec![
                PlanarProfileSegment::CircularArc {
                    start_mm: positive,
                    end_mm: negative,
                    center_mm,
                    clockwise: false,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm: negative,
                    end_mm: positive,
                    center_mm,
                    clockwise: false,
                },
            ]
        }
    };
    backend.sweep_spatial_profile(&segments, path)
}

fn exact_brep_spatial_path_segment(segment: &ExactBRepSpatialPathSegment) -> SpatialProfileSegment {
    match segment {
        ExactBRepSpatialPathSegment::Line {
            start_bits,
            end_bits,
        } => SpatialProfileSegment::Line {
            start_mm: start_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
        },
        ExactBRepSpatialPathSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            normal_bits,
            clockwise,
        } => SpatialProfileSegment::CircularArc {
            start_mm: start_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
            center_mm: center_bits.map(f64::from_bits),
            normal: normal_bits.map(f64::from_bits),
            clockwise: *clockwise,
        },
        ExactBRepSpatialPathSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => SpatialProfileSegment::CubicBezier {
            start_mm: start_bits.map(f64::from_bits),
            control_1_mm: control_1_bits.map(f64::from_bits),
            control_2_mm: control_2_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
        },
    }
}

fn exact_brep_planar_offset(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    distance_mm: f64,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    if let ExactBRepPlanarGeometry::Region { outer, holes } = &profile.geometry {
        return backend.offset_planar_region(
            &exact_brep_planar_loop(outer),
            &holes.iter().map(exact_brep_planar_loop).collect::<Vec<_>>(),
            distance_mm,
        );
    }
    if let Some([min_x, min_y, max_x, max_y]) = exact_brep_planar_rectangle_bounds(profile) {
        backend.offset_rectangle(RectangleOffsetSpec {
            min_mm: [min_x, min_y],
            max_mm: [max_x, max_y],
            distance_mm,
        })
    } else {
        backend.offset_planar_profile(&exact_brep_planar_offset_loop(profile)?, distance_mm)
    }
}

fn exact_brep_planar_offset_loop(
    profile: &ExactBRepProfile,
) -> Result<PlanarProfileLoop, ketchup_exact::GeometryError> {
    match &profile.geometry {
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => Ok(PlanarProfileLoop::Circle {
            center_mm: center_bits.map(f64::from_bits),
            radius_mm: f64::from_bits(*radius_bits),
        }),
        ExactBRepPlanarGeometry::Boundary { .. } => Ok(PlanarProfileLoop::Segments(
            exact_brep_boundary_segments(profile, true)?,
        )),
        _ => Err(exact_brep_profile_error(
            profile,
            "exact planar offset requires a line/arc boundary or circle",
        )),
    }
}

fn exact_brep_boundary_segments(
    profile: &ExactBRepProfile,
    require_closed: bool,
) -> Result<Vec<PlanarProfileSegment>, ketchup_exact::GeometryError> {
    let ExactBRepPlanarGeometry::Boundary { closed, segments } = &profile.geometry else {
        return Err(exact_brep_profile_error(
            profile,
            "exact operation requires a line/arc boundary",
        ));
    };
    if *closed != require_closed {
        return Err(exact_brep_profile_error(
            profile,
            "exact boundary closure does not match the operation",
        ));
    }
    Ok(segments
        .iter()
        .map(|segment| match segment {
            ExactBRepPlanarSegment::Line {
                start_bits,
                end_bits,
            } => PlanarProfileSegment::Line {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
            ExactBRepPlanarSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
                center_mm: center_bits.map(f64::from_bits),
                clockwise: *clockwise,
            },
            ExactBRepPlanarSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => PlanarProfileSegment::CubicBezier {
                start_mm: start_bits.map(f64::from_bits),
                control_1_mm: control_1_bits.map(f64::from_bits),
                control_2_mm: control_2_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
        })
        .collect())
}

fn exact_brep_planar_loop(planar_loop: &ExactBRepPlanarLoop) -> PlanarProfileLoop {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => PlanarProfileLoop::Segments(
            segments
                .iter()
                .map(|segment| match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => PlanarProfileSegment::Line {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => PlanarProfileSegment::CircularArc {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                        center_mm: center_bits.map(f64::from_bits),
                        clockwise: *clockwise,
                    },
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => PlanarProfileSegment::CubicBezier {
                        start_mm: start_bits.map(f64::from_bits),
                        control_1_mm: control_1_bits.map(f64::from_bits),
                        control_2_mm: control_2_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                })
                .collect(),
        ),
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => PlanarProfileLoop::Circle {
            center_mm: center_bits.map(f64::from_bits),
            radius_mm: f64::from_bits(*radius_bits),
        },
    }
}

fn exact_brep_profile_body(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let distance_mm = interval.length_mm();
    let local = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => {
            let segments = segments
                .iter()
                .map(|segment| match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => PlanarProfileSegment::Line {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => PlanarProfileSegment::CircularArc {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                        center_mm: center_bits.map(f64::from_bits),
                        clockwise: *clockwise,
                    },
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => PlanarProfileSegment::CubicBezier {
                        start_mm: start_bits.map(f64::from_bits),
                        control_1_mm: control_1_bits.map(f64::from_bits),
                        control_2_mm: control_2_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                })
                .collect::<Vec<_>>();
            backend.extrude_mixed_profile(&segments, distance_mm)?
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => backend.extrude_circle(CircleExtrudeSpec {
            center_mm: center_bits.map(f64::from_bits),
            radius_mm: f64::from_bits(*radius_bits),
            height_mm: distance_mm,
        })?,
        ExactBRepPlanarGeometry::Region { outer, holes } => backend.extrude_planar_region(
            &exact_brep_planar_loop(outer),
            &holes.iter().map(exact_brep_planar_loop).collect::<Vec<_>>(),
            distance_mm,
        )?,
        ExactBRepPlanarGeometry::Boundary { closed: false, .. }
        | ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact extrusion requires a closed line/arc/circle profile",
            ));
        }
    };
    let frame = profile.frame_bits.map(f64::from_bits);
    let direction = interval.direction();
    let start_mm = interval.start_mm();
    let origin = [
        frame[0] + direction[0] * start_mm,
        frame[1] + direction[1] * start_mm,
        frame[2] + direction[2] * start_mm,
    ];
    let matrix = [
        frame[3],
        frame[6],
        direction[0],
        origin[0],
        frame[4],
        frame[7],
        direction[1],
        origin[1],
        frame[5],
        frame[8],
        direction[2],
        origin[2],
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    if matrix
        == [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    {
        Ok(local)
    } else {
        backend.transform_body(&local.body, &matrix)
    }
}

fn exact_brep_graph_error(
    graph: &ExactBRepGraph,
    diagnostic: &str,
) -> ketchup_exact::GeometryError {
    ketchup_exact::GeometryError {
        code: ketchup_exact::GeometryErrorCode::InvalidParameter,
        diagnostic: diagnostic.to_owned(),
        operation: "evaluate_exact_brep_graph",
        input_digest: graph.graph_digest.clone(),
        backend_fingerprint: ketchup_exact::backend_fingerprint(),
    }
}

fn exact_brep_profile_error(
    profile: &ExactBRepProfile,
    diagnostic: &str,
) -> ketchup_exact::GeometryError {
    ketchup_exact::GeometryError {
        code: ketchup_exact::GeometryErrorCode::InvalidProfile,
        diagnostic: diagnostic.to_owned(),
        operation: "evaluate_exact_brep_profile",
        input_digest: sha256_hex(format!("profile:{}", profile.id.0).as_bytes()),
        backend_fingerprint: ketchup_exact::backend_fingerprint(),
    }
}

fn parse_profile_segment(token: &str) -> Option<PlanarProfileSegment> {
    let fields = token.split(',').collect::<Vec<_>>();
    let bits = |index: usize| {
        u64::from_str_radix(fields.get(index)?, 16)
            .ok()
            .map(f64::from_bits)
    };
    match fields.as_slice() {
        ["L", _, _, _, _] => Some(PlanarProfileSegment::Line {
            start_mm: [bits(1)?, bits(2)?],
            end_mm: [bits(3)?, bits(4)?],
        }),
        ["A", _, _, _, _, _, _, clockwise] => Some(PlanarProfileSegment::CircularArc {
            start_mm: [bits(1)?, bits(2)?],
            end_mm: [bits(3)?, bits(4)?],
            center_mm: [bits(5)?, bits(6)?],
            clockwise: match *clockwise {
                "0" => false,
                "1" => true,
                _ => return None,
            },
        }),
        _ => None,
    }
}

fn p3_mixed_extrude_response(
    backend: &ExactBackend,
    segments: &[PlanarProfileSegment],
    height_mm: f64,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.extrude_mixed_profile(segments, height_mm);
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(output) => {
            let references = match capture_mixed_profile_extrusion_references(
                &output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let mut evidence = Vec::new();
            let side = if segments
                .iter()
                .all(|segment| matches!(segment, PlanarProfileSegment::Line { .. }))
            {
                (
                    "extrusion.side(profile_edge=line.0)",
                    "profile.edge.line.0",
                    "planar_face",
                )
            } else {
                (
                    "extrusion.side(profile_edge=arc.0)",
                    "profile.edge.arc.0",
                    "face",
                )
            };
            for (role, source, expected_type) in [
                ("extrusion.top", "profile.face", "planar_face"),
                ("extrusion.bottom", "profile.face", "planar_face"),
                side,
            ] {
                let Some(reference) = references.iter().find(|reference| {
                    reference.semantic_role == role && reference.source_element_id == source
                }) else {
                    return "ERR incomplete_history".to_owned();
                };
                if reference.expected_type != expected_type
                    || reference.stability_class != StabilityClass::Guaranteed
                {
                    return "ERR incomplete_history".to_owned();
                }
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return "ERR incomplete_history".to_owned();
                };
                evidence.push((face_ordinal, reference));
            }
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                evidence[0].0,
                evidence[0].1.corroborating_geometry_fingerprint,
                evidence[0].1.lineage_digest,
                evidence[1].0,
                evidence[1].1.corroborating_geometry_fingerprint,
                evidence[1].1.lineage_digest,
                evidence[2].0,
                evidence[2].1.corroborating_geometry_fingerprint,
                evidence[2].1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_circle_extrude_response(
    backend: &ExactBackend,
    protocol: &str,
    bits: [u64; 4],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.extrude_circle(CircleExtrudeSpec {
        center_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
        radius_mm: f64::from_bits(bits[2]),
        height_mm: f64::from_bits(bits[3]),
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(output) => {
            let references = match capture_circle_extrusion_references(
                &output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let mut evidence = Vec::new();
            for (role, source, expected_type) in [
                ("extrusion.top", "profile.face", "planar_face"),
                ("extrusion.bottom", "profile.face", "planar_face"),
                (
                    "extrusion.side(profile_edge=circle)",
                    "profile.edge.circle",
                    "cylindrical_face",
                ),
            ] {
                let Some(reference) = references.iter().find(|reference| {
                    reference.semantic_role == role && reference.source_element_id == source
                }) else {
                    return "ERR incomplete_history".to_owned();
                };
                if reference.expected_type != expected_type
                    || reference.stability_class != StabilityClass::Guaranteed
                {
                    return "ERR incomplete_history".to_owned();
                }
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return "ERR incomplete_history".to_owned();
                };
                evidence.push((face_ordinal, reference));
            }
            let topology = &output.body.topology;
            if protocol == "EXTRUDE_CIRCLE_P3_V2" {
                let resolved_face = |index: usize| {
                    let (ordinal, reference) = evidence[index];
                    let mut matches = topology.faces.iter().filter(|face| {
                        face.ordinal == ordinal
                            && face.geometric_fingerprint
                                == reference.corroborating_geometry_fingerprint
                    });
                    matches.next().filter(|_| matches.next().is_none())
                };
                let (Some(top), Some(bottom), Some(side)) =
                    (resolved_face(0), resolved_face(1), resolved_face(2))
                else {
                    return "ERR incomplete_history".to_owned();
                };
                let (Some(axis_origin), Some(axis_direction)) =
                    (side.axis_origin_mm, side.axis_direction)
                else {
                    return "ERR incomplete_history".to_owned();
                };
                return format!(
                    "OK_P3_CIRCLE_V2 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
                    output.body.result_fingerprint,
                    topology.volume_mm3.to_bits(),
                    topology.bounds_mm.min.x.to_bits(),
                    topology.bounds_mm.min.y.to_bits(),
                    topology.bounds_mm.min.z.to_bits(),
                    topology.bounds_mm.max.x.to_bits(),
                    topology.bounds_mm.max.y.to_bits(),
                    topology.bounds_mm.max.z.to_bits(),
                    topology.vertex_count,
                    topology.edge_count,
                    topology.face_count,
                    topology.shell_count,
                    topology.solid_count,
                    output.input_digest,
                    output.backend_fingerprint,
                    output.tolerance_report.profile,
                    evidence[0].0,
                    evidence[0].1.corroborating_geometry_fingerprint,
                    evidence[0].1.lineage_digest,
                    top.centroid_mm.x.to_bits(),
                    top.centroid_mm.y.to_bits(),
                    top.centroid_mm.z.to_bits(),
                    top.normal.x.to_bits(),
                    top.normal.y.to_bits(),
                    top.normal.z.to_bits(),
                    evidence[1].0,
                    evidence[1].1.corroborating_geometry_fingerprint,
                    evidence[1].1.lineage_digest,
                    bottom.centroid_mm.x.to_bits(),
                    bottom.centroid_mm.y.to_bits(),
                    bottom.centroid_mm.z.to_bits(),
                    bottom.normal.x.to_bits(),
                    bottom.normal.y.to_bits(),
                    bottom.normal.z.to_bits(),
                    evidence[2].0,
                    evidence[2].1.corroborating_geometry_fingerprint,
                    evidence[2].1.lineage_digest,
                    axis_origin.x.to_bits(),
                    axis_origin.y.to_bits(),
                    axis_origin.z.to_bits(),
                    axis_direction.x.to_bits(),
                    axis_direction.y.to_bits(),
                    axis_direction.z.to_bits(),
                );
            }
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                evidence[0].0,
                evidence[0].1.corroborating_geometry_fingerprint,
                evidence[0].1.lineage_digest,
                evidence[1].0,
                evidence[1].1.corroborating_geometry_fingerprint,
                evidence[1].1.lineage_digest,
                evidence[2].0,
                evidence[2].1.corroborating_geometry_fingerprint,
                evidence[2].1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_circular_boolean_response(
    backend: &ExactBackend,
    bits: [u64; 6],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
    union: bool,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(bits[0]),
        depth_mm: f64::from_bits(bits[1]),
        height_mm: f64::from_bits(bits[2]),
    };
    let tool = CylinderToolSpec {
        center_mm: [f64::from_bits(bits[3]), f64::from_bits(bits[4])],
        origin_z_mm: 0.0,
        radius_mm: f64::from_bits(bits[5]),
        height_mm: base.height_mm,
    };
    let result = backend.extrude_rectangle(base).and_then(|base_output| {
        if union {
            backend.fuse_cylinder(&base_output.body, tool)
        } else {
            backend.common_cylinder(&base_output.body, tool)
        }
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(output) => {
            let references = match capture_circle_extrusion_references(
                &output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let roles = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=circle)", "profile.edge.circle"),
            ];
            let evidence = roles.map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let face_candidates = output
                    .body
                    .topology
                    .faces
                    .iter()
                    .filter(|face| {
                        face.geometric_fingerprint == reference.corroborating_geometry_fingerprint
                    })
                    .collect::<Vec<_>>();
                let [face] = face_candidates.as_slice() else {
                    return Err(());
                };
                Ok((face.ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(side)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                side.0,
                side.1.corroborating_geometry_fingerprint,
                side.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_circular_split_response(
    backend: &ExactBackend,
    bits: [u64; 6],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(bits[0]),
        depth_mm: f64::from_bits(bits[1]),
        height_mm: f64::from_bits(bits[2]),
    };
    let tool = CylinderToolSpec {
        center_mm: [f64::from_bits(bits[3]), f64::from_bits(bits[4])],
        origin_z_mm: 0.0,
        radius_mm: f64::from_bits(bits[5]),
        height_mm: base.height_mm,
    };
    let result = backend
        .extrude_rectangle(base)
        .and_then(|base_output| backend.split_cylinder(&base_output.body, tool));
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let side_overlap = ExactCircleProfile {
                center_x_bits: bits[3],
                center_y_bits: bits[4],
                radius_bits: bits[5],
                clockwise: false,
            };
            let side_overlap = side_overlap
                .side_overlap(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
                .or_else(|| {
                    side_overlap.corner_overlap(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
                })
                .or_else(|| {
                    side_overlap
                        .outside_side_overlap(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
                })
                .or_else(|| {
                    side_overlap
                        .center_on_side_overlap(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
                })
                .or_else(|| {
                    side_overlap
                        .center_on_corner_overlap(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
                })
                .or_else(|| {
                    side_overlap
                        .outside_corner_overlap(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
                })
                .is_some();
            let references = match if side_overlap {
                capture_circular_split_references(&mut output, document_id, producer_feature_id)
            } else {
                capture_rectangular_split_references(&mut output, document_id, producer_feature_id)
            } {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let roles = if side_overlap {
                [
                    ("extrusion.top", "profile.face"),
                    ("extrusion.bottom", "profile.face"),
                    ("extrusion.side(profile_edge=circle)", "profile.edge.circle"),
                ]
            } else {
                [
                    ("extrusion.top", "profile.face"),
                    ("extrusion.bottom", "profile.face"),
                    ("extrusion.side(profile_edge=east)", "profile.edge.east"),
                ]
            };
            let evidence = roles.map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_circular_cut_response(
    backend: &ExactBackend,
    bits: [u64; 6],
    pocket_depth_mm: Option<f64>,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(bits[0]),
        depth_mm: f64::from_bits(bits[1]),
        height_mm: f64::from_bits(bits[2]),
    };
    let circle = ExactCircleProfile {
        center_x_bits: bits[3],
        center_y_bits: bits[4],
        radius_bits: bits[5],
        clockwise: true,
    };
    let circular_boundary_overlap = circle.side_overlap(base.width_mm, base.depth_mm).is_some()
        || circle
            .corner_overlap(base.width_mm, base.depth_mm)
            .is_some()
        || circle
            .outside_side_overlap(base.width_mm, base.depth_mm)
            .is_some()
        || circle
            .center_on_side_overlap(base.width_mm, base.depth_mm)
            .is_some()
        || circle
            .center_on_corner_overlap(base.width_mm, base.depth_mm)
            .is_some()
        || circle
            .outside_corner_overlap(base.width_mm, base.depth_mm)
            .is_some();
    if pocket_depth_mm.is_some_and(|depth_mm| {
        !depth_mm.is_finite() || depth_mm <= 0.0 || depth_mm >= base.height_mm
    }) {
        return "ERR invalid_parameter".to_owned();
    }
    let (origin_z_mm, height_mm, mode) = pocket_depth_mm.map_or(
        (-1.0, base.height_mm + 2.0, CutMode::ThroughAll),
        |depth_mm| {
            (
                base.height_mm - depth_mm,
                depth_mm + 1.0,
                CutMode::BlindPlanar,
            )
        },
    );
    let result = backend.extrude_rectangle(base).and_then(|base_output| {
        backend.cut_cylinder(
            &base_output.body,
            CylinderToolSpec {
                center_mm: [f64::from_bits(bits[3]), f64::from_bits(bits[4])],
                origin_z_mm,
                radius_mm: f64::from_bits(bits[5]),
                height_mm,
            },
            mode,
        )
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let captured =
                if let Some(depth_mm) = pocket_depth_mm.filter(|_| circular_boundary_overlap) {
                    capture_circular_pocket_references(
                        &mut output,
                        document_id,
                        producer_feature_id,
                        base,
                        base.height_mm - depth_mm,
                    )
                } else {
                    capture_circular_through_cut_references(
                        &mut output,
                        document_id,
                        producer_feature_id,
                        base,
                    )
                };
            let references = match captured {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let Some(host_side_role) = references
                .iter()
                .map(|reference| reference.semantic_role.as_str())
                .find(|role| {
                    matches!(
                        *role,
                        "extrusion.side(profile_edge=east)" | "extrusion.side(profile_edge=west)"
                    )
                })
            else {
                return "ERR incomplete_history".to_owned();
            };
            let mut evidence = Vec::new();
            let bottom_or_floor_role = if pocket_depth_mm.is_some() && circular_boundary_overlap {
                "pocket.floor"
            } else {
                "extrusion.bottom"
            };
            for role in [
                "extrusion.top",
                bottom_or_floor_role,
                host_side_role,
                "through_cut.wall.circle",
            ] {
                let Some(reference) = references
                    .iter()
                    .find(|reference| reference.semantic_role == role)
                else {
                    return "ERR incomplete_history".to_owned();
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return "ERR incomplete_history".to_owned();
                };
                evidence.push((face_ordinal, reference));
            }
            let topology = &output.body.topology;
            format!(
                "OK_P3_CIRCULAR_CUT_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                evidence[0].0,
                evidence[0].1.corroborating_geometry_fingerprint,
                evidence[0].1.lineage_digest,
                evidence[1].0,
                evidence[1].1.corroborating_geometry_fingerprint,
                evidence[1].1.lineage_digest,
                evidence[2].0,
                evidence[2].1.corroborating_geometry_fingerprint,
                evidence[2].1.lineage_digest,
                evidence[3].0,
                evidence[3].1.corroborating_geometry_fingerprint,
                evidence[3].1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn legacy_extrude_response(backend: &ExactBackend, height_bits: u64) -> String {
    let started = Instant::now();
    let result = backend.extrude_rectangle(RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: f64::from_bits(height_bits),
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(output) => {
            let topology = &output.body.topology;
            format!(
                "OK {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p5_box_shell_response(
    backend: &ExactBackend,
    width_bits: &str,
    depth_bits: &str,
    fields: &[&str],
    finish: Option<&str>,
) -> String {
    let expected_fields = if finish.is_some() { 7 } else { 5 };
    if fields.len() != expected_fields {
        return "ERR invalid_request".to_owned();
    }
    let parse_bits = |value: &str| u64::from_str_radix(value, 16).map(f64::from_bits);
    let (Ok(width), Ok(depth), Ok(height), Ok(thickness)) = (
        parse_bits(width_bits),
        parse_bits(depth_bits),
        parse_bits(fields[0]),
        parse_bits(fields[1]),
    ) else {
        return "ERR invalid_parameter".to_owned();
    };
    let (amount, document_offset) = if finish.is_some() {
        let Ok(amount) = parse_bits(fields[3]) else {
            return "ERR invalid_parameter".to_owned();
        };
        (Some(amount), 4)
    } else {
        (None, 2)
    };
    let document_id = fields[document_offset];
    let producer_feature_id = fields[document_offset + 1];
    let request_digest = fields[document_offset + 2];
    if document_id.parse::<u64>().is_err()
        || producer_feature_id.parse::<u64>().is_err()
        || !is_canonical_digest(request_digest)
    {
        return "ERR invalid_request".to_owned();
    }
    let spec = RectangleExtrudeSpec {
        width_mm: width,
        depth_mm: depth,
        height_mm: height,
    };
    let output = match (finish, amount) {
        (Some("fillet"), Some(amount)) => {
            backend.finish_shell_box(spec, thickness, EdgeFinish::Fillet, amount)
        }
        (Some("chamfer"), Some(amount)) => {
            backend.finish_shell_box(spec, thickness, EdgeFinish::Chamfer, amount)
        }
        (None, None) => backend.shell_box(spec, thickness),
        _ => return "ERR invalid_request".to_owned(),
    };
    let output = match output {
        Ok(output) => output,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    let references = match capture_box_shell_references(&output, document_id, producer_feature_id) {
        Ok(references) => references,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    let roles = [
        ("shell.box.rim", "extrusion.top"),
        ("shell.box.outer.bottom", "extrusion.bottom"),
        ("shell.box.outer.east", "extrusion.side(profile_edge=east)"),
    ];
    let mut evidence = Vec::with_capacity(roles.len());
    for (role, source) in roles {
        let matching = references
            .iter()
            .filter(|reference| {
                reference.semantic_role == role && reference.source_element_id == source
            })
            .collect::<Vec<_>>();
        let [reference] = matching.as_slice() else {
            return "ERR incomplete_history".to_owned();
        };
        if reference.expected_type != "planar_face"
            || reference.stability_class != StabilityClass::Guaranteed
            || reference.backend_fingerprint != output.backend_fingerprint
        {
            return "ERR incomplete_history".to_owned();
        }
        let ReferenceResolution::Resolved {
            face_ordinal,
            migrated_backend: false,
        } = resolve_subshape_reference(reference, &output)
        else {
            return "ERR incomplete_history".to_owned();
        };
        evidence.push((face_ordinal, *reference));
    }
    let topology = &output.body.topology;
    format!(
        "OK_M3_V1 0 {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
        output.body.result_fingerprint,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
        output.input_digest,
        output.backend_fingerprint,
        output.tolerance_report.profile,
        evidence[0].0,
        evidence[0].1.corroborating_geometry_fingerprint,
        evidence[0].1.lineage_digest,
        evidence[1].0,
        evidence[1].1.corroborating_geometry_fingerprint,
        evidence[1].1.lineage_digest,
        evidence[2].0,
        evidence[2].1.corroborating_geometry_fingerprint,
        evidence[2].1.lineage_digest,
    )
}

fn m3_extrude_response(
    backend: &ExactBackend,
    width_bits: u64,
    depth_bits: u64,
    height_bits: u64,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.extrude_rectangle(RectangleExtrudeSpec {
        width_mm: f64::from_bits(width_bits),
        depth_mm: f64::from_bits(depth_bits),
        height_mm: f64::from_bits(height_bits),
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(output) => {
            let references =
                match capture_guaranteed_references(&output, document_id, producer_feature_id) {
                    Ok(references) => references,
                    Err(error) => return format!("ERR {}", error.code.as_str()),
                };
            let evidence = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
            ]
            .map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                if reference.document_id != document_id
                    || reference.producer_feature_id != producer_feature_id
                    || reference.expected_type != "planar_face"
                    || reference.stability_class != StabilityClass::Guaranteed
                    || reference.backend_fingerprint != output.backend_fingerprint
                    || reference.lineage_digest.is_empty()
                    || reference.corroborating_geometry_fingerprint.is_empty()
                {
                    return Err(());
                }
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn m3_extrude_planar_face_response(
    backend: &ExactBackend,
    width_bits: u64,
    depth_bits: u64,
    height_bits: u64,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.extrude_rectangle(RectangleExtrudeSpec {
        width_mm: f64::from_bits(width_bits),
        depth_mm: f64::from_bits(depth_bits),
        height_mm: f64::from_bits(height_bits),
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(output) => {
            let references =
                match capture_guaranteed_references(&output, document_id, producer_feature_id) {
                    Ok(references) => references,
                    Err(error) => return format!("ERR {}", error.code.as_str()),
                };
            let evidence = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
            ]
            .map(|(role, source)| -> Result<_, ()> {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                if reference.document_id != document_id
                    || reference.producer_feature_id != producer_feature_id
                    || reference.expected_type != "planar_face"
                    || reference.stability_class != StabilityClass::Guaranteed
                    || reference.backend_fingerprint != output.backend_fingerprint
                    || reference.lineage_digest.is_empty()
                    || reference.corroborating_geometry_fingerprint.is_empty()
                {
                    return Err(());
                }
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                let face = output
                    .body
                    .topology
                    .faces
                    .iter()
                    .find(|face| face.ordinal == face_ordinal)
                    .ok_or(())?;
                let centroid = [face.centroid_mm.x, face.centroid_mm.y, face.centroid_mm.z];
                let normal = [face.normal.x, face.normal.y, face.normal.z];
                let normal_length_squared = normal.iter().map(|value| value * value).sum::<f64>();
                if face.surface_kind != "plane"
                    || face.geometric_fingerprint != reference.corroborating_geometry_fingerprint
                    || !centroid.into_iter().all(f64::is_finite)
                    || !normal.into_iter().all(f64::is_finite)
                    || (normal_length_squared - 1.0).abs() > 1.0e-12
                {
                    return Err(());
                }
                Ok((face_ordinal, *reference, centroid, normal))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            let mut response = format!(
                "OK_M3_V2 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
            );
            for (face_ordinal, reference, centroid, normal) in [top, bottom, east] {
                write!(
                    response,
                    " {face_ordinal} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
                    reference.corroborating_geometry_fingerprint,
                    reference.lineage_digest,
                    centroid[0].to_bits(),
                    centroid[1].to_bits(),
                    centroid[2].to_bits(),
                    normal[0].to_bits(),
                    normal[1].to_bits(),
                    normal[2].to_bits(),
                )
                .expect("writing to String cannot fail");
            }
            response
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_polygon_union_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    segments: &[PlanarProfileSegment],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    p3_polygon_boolean_response(
        backend,
        base_bits,
        segments,
        document_id,
        producer_feature_id,
        request_digest,
        false,
    )
}

fn p3_polygon_intersection_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    segments: &[PlanarProfileSegment],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    p3_polygon_boolean_response(
        backend,
        base_bits,
        segments,
        document_id,
        producer_feature_id,
        request_digest,
        true,
    )
}

fn p3_polygon_split_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    segments: &[PlanarProfileSegment],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let result = backend.extrude_rectangle(base).and_then(|base_output| {
        backend.split_mixed_profile(&base_output.body, segments, 0.0, base.height_mm)
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let split_references = match if segments
                .iter()
                .any(|segment| matches!(segment, PlanarProfileSegment::CircularArc { .. }))
            {
                capture_profile_split_references(
                    &mut output,
                    document_id,
                    producer_feature_id,
                    segments,
                )
            } else {
                capture_rectangular_split_references(&mut output, document_id, producer_feature_id)
            } {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let side =
                if split_references.iter().any(|reference| {
                    reference.semantic_role == "extrusion.side(profile_edge=arc.0)"
                }) {
                    ("extrusion.side(profile_edge=arc.0)", "profile.edge.arc.0")
                } else if split_references.iter().any(|reference| {
                    reference.semantic_role == "extrusion.side(profile_edge=line.0)"
                }) {
                    ("extrusion.side(profile_edge=line.0)", "profile.edge.line.0")
                } else {
                    ("extrusion.side(profile_edge=east)", "profile.edge.east")
                };
            let roles = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                side,
            ];
            let evidence = roles.map(|(role, source)| {
                let candidates = split_references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_polygon_boolean_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    segments: &[PlanarProfileSegment],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
    intersection: bool,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let result = backend.extrude_rectangle(base).and_then(|base_output| {
        if intersection {
            backend.common_mixed_profile(&base_output.body, segments, 0.0, base.height_mm)
        } else {
            backend.fuse_mixed_profile(&base_output.body, segments, 0.0, base.height_mm)
        }
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match if intersection {
                capture_contained_polygon_intersection_references(
                    &mut output,
                    document_id,
                    producer_feature_id,
                    segments,
                )
            } else {
                capture_contained_polygon_union_references(
                    &mut output,
                    document_id,
                    producer_feature_id,
                    segments,
                )
            } {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let side = if references
                .iter()
                .any(|reference| reference.semantic_role == "extrusion.side(profile_edge=arc.0)")
            {
                ("extrusion.side(profile_edge=arc.0)", "profile.edge.arc.0")
            } else {
                ("extrusion.side(profile_edge=line.0)", "profile.edge.line.0")
            };
            let roles = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                side,
            ];
            let evidence = roles.map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(side)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                side.0,
                side.1.corroborating_geometry_fingerprint,
                side.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p3_polygon_cut_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    pocket_depth_bits: Option<u64>,
    segments: &[PlanarProfileSegment],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let pocket_depth_mm = pocket_depth_bits.map(f64::from_bits);
    if pocket_depth_mm
        .is_some_and(|depth| !depth.is_finite() || depth <= 0.0 || depth >= base.height_mm)
    {
        return "ERR invalid_parameter".to_owned();
    }
    let (origin_z_mm, cut_height_mm) = pocket_depth_mm
        .map_or((-1.0, base.height_mm + 2.0), |depth| {
            (base.height_mm - depth, depth)
        });
    let result = backend.extrude_rectangle(base).and_then(|base_output| {
        backend.cut_mixed_profile(&base_output.body, segments, origin_z_mm, cut_height_mm)
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_polygon_through_cut_references(
                &mut output,
                document_id,
                producer_feature_id,
                pocket_depth_mm.map(|depth| base.height_mm - depth),
                Some(segments),
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let retained_side_roles = references
                .iter()
                .filter_map(|reference| {
                    match (
                        reference.semantic_role.as_str(),
                        reference.source_element_id.as_str(),
                    ) {
                        ("extrusion.side(profile_edge=east)", "profile.edge.east") => {
                            Some(("extrusion.side(profile_edge=east)", "profile.edge.east"))
                        }
                        ("extrusion.side(profile_edge=west)", "profile.edge.west") => {
                            Some(("extrusion.side(profile_edge=west)", "profile.edge.west"))
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            let [retained_side_role] = retained_side_roles.as_slice() else {
                return "ERR incomplete_history".to_owned();
            };
            let cut_wall_roles = references
                .iter()
                .filter_map(|reference| {
                    match (
                        reference.semantic_role.as_str(),
                        reference.source_element_id.as_str(),
                    ) {
                        ("through_cut.wall.line.0", "cut_profile.edge.line.0") => {
                            Some(("through_cut.wall.line.0", "cut_profile.edge.line.0"))
                        }
                        ("through_cut.wall.arc.0", "cut_profile.edge.arc.0") => {
                            Some(("through_cut.wall.arc.0", "cut_profile.edge.arc.0"))
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            let [cut_wall_role] = cut_wall_roles.as_slice() else {
                return "ERR incomplete_history".to_owned();
            };
            let roles = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                *retained_side_role,
                *cut_wall_role,
            ];
            let evidence = roles.map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east), Ok(wall)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let floor = if pocket_depth_bits.is_some() {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == "pocket.floor"
                            && reference.source_element_id == "pocket_profile.face"
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return "ERR incomplete_history".to_owned();
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return "ERR incomplete_history".to_owned();
                };
                Some((face_ordinal, *reference))
            } else {
                None
            };
            let topology = &output.body.topology;
            let mut response = format!(
                "{} {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                if floor.is_some() {
                    "OK_P3_POLYGON_POCKET_V1"
                } else {
                    "OK_P3_POLYGON_CUT_V1"
                },
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
                wall.0,
                wall.1.corroborating_geometry_fingerprint,
                wall.1.lineage_digest,
            );
            if let Some(floor) = floor {
                response.push_str(&format!(
                    " {} {} {}",
                    floor.0, floor.1.corroborating_geometry_fingerprint, floor.1.lineage_digest,
                ));
            }
            response
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn m3_through_cut_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    cut_bits: [u64; 4],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let tool = BoxSpec {
        origin_mm: Point3 {
            x: f64::from_bits(cut_bits[0]),
            y: f64::from_bits(cut_bits[1]),
            z: -1.0,
        },
        size_mm: Size3 {
            x: f64::from_bits(cut_bits[2]),
            y: f64::from_bits(cut_bits[3]),
            z: base.height_mm + 2.0,
        },
    };
    let result = backend
        .extrude_rectangle(base)
        .and_then(|base_output| backend.cut_box(&base_output.body, tool, CutMode::ThroughAll));
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_bounded_through_cut_references(
                &mut output,
                document_id,
                producer_feature_id,
                base,
                tool,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let roles = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
                ("through_cut.wall.west", "cut_profile.edge.west"),
                ("through_cut.wall.east", "cut_profile.edge.east"),
                ("through_cut.wall.south", "cut_profile.edge.south"),
                ("through_cut.wall.north", "cut_profile.edge.north"),
            ];
            let evidence = roles.map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                if reference.document_id != document_id
                    || reference.producer_feature_id != producer_feature_id
                    || reference.expected_type != "planar_face"
                    || reference.stability_class != StabilityClass::Guaranteed
                    || reference.backend_fingerprint != output.backend_fingerprint
                    || reference.lineage_digest.is_empty()
                    || reference.corroborating_geometry_fingerprint.is_empty()
                {
                    return Err(());
                }
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [
                Ok(top),
                Ok(bottom),
                Ok(east),
                Ok(cut_west),
                Ok(cut_east),
                Ok(cut_south),
                Ok(cut_north),
            ] = evidence
            else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_CUT_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
                cut_west.0,
                cut_west.1.corroborating_geometry_fingerprint,
                cut_west.1.lineage_digest,
                cut_east.0,
                cut_east.1.corroborating_geometry_fingerprint,
                cut_east.1.lineage_digest,
                cut_south.0,
                cut_south.1.corroborating_geometry_fingerprint,
                cut_south.1.lineage_digest,
                cut_north.0,
                cut_north.1.corroborating_geometry_fingerprint,
                cut_north.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p6_intersect_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    tool_bits: [u64; 4],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let tool = BoxSpec {
        origin_mm: Point3 {
            x: f64::from_bits(tool_bits[0]),
            y: f64::from_bits(tool_bits[1]),
            z: 0.0,
        },
        size_mm: Size3 {
            x: f64::from_bits(tool_bits[2]),
            y: f64::from_bits(tool_bits[3]),
            z: base.height_mm,
        },
    };
    let result = backend
        .extrude_rectangle(base)
        .and_then(|base_output| backend.common_box(&base_output.body, tool));
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_rectangular_intersection_references(
                &mut output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let evidence = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
            ]
            .map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p6_loft_response(
    backend: &ExactBackend,
    values: &[f64],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let Some(section_count) = values.first().copied().map(|value| value as usize) else {
        return "ERR invalid_parameter".to_owned();
    };
    let mut cursor = 1;
    let mut sections = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        if cursor + 2 > values.len() {
            return "ERR invalid_parameter".to_owned();
        }
        let point_count = values[cursor] as usize;
        let elevation_mm = values[cursor + 1];
        cursor += 2;
        if cursor + point_count * 2 > values.len() {
            return "ERR invalid_parameter".to_owned();
        }
        let control_points_mm = values[cursor..cursor + point_count * 2]
            .chunks_exact(2)
            .map(|point| [point[0], point[1]])
            .collect();
        cursor += point_count * 2;
        sections.push(SplineLoftSection {
            elevation_mm,
            control_points_mm,
        });
    }
    if cursor != values.len() {
        return "ERR invalid_parameter".to_owned();
    }
    match backend.loft_spline(&SplineLoftSpec { sections }) {
        Ok(output) => {
            let references =
                match capture_spline_loft_references(&output, document_id, producer_feature_id) {
                    Ok(references) => references,
                    Err(error) => return format!("ERR {}", error.code.as_str()),
                };
            let resolved = references
                .iter()
                .map(
                    |reference| match resolve_subshape_reference(reference, &output) {
                        ReferenceResolution::Resolved {
                            face_ordinal,
                            migrated_backend: false,
                        } => Some((face_ordinal, reference)),
                        _ => None,
                    },
                )
                .collect::<Option<Vec<_>>>();
            let Some(resolved) = resolved else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            let mut response = format!(
                "OK_P6_LOFT_V1 {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
            );
            for (face_ordinal, reference) in resolved {
                response.push_str(&format!(
                    " {face_ordinal} {} {}",
                    reference.corroborating_geometry_fingerprint, reference.lineage_digest,
                ));
            }
            response
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p6_sweep_response(
    backend: &ExactBackend,
    bits: [u64; 8],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let result = backend.sweep_rectangle(RectangleSweepSpec {
        profile_min_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
        profile_max_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
        path_start_mm: [f64::from_bits(bits[4]), f64::from_bits(bits[5])],
        path_end_mm: [f64::from_bits(bits[6]), f64::from_bits(bits[7])],
    });
    match result {
        Ok(output) => {
            let references = match capture_rectangular_sweep_references(
                &output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let resolved = references
                .iter()
                .map(
                    |reference| match resolve_subshape_reference(reference, &output) {
                        ReferenceResolution::Resolved {
                            face_ordinal,
                            migrated_backend: false,
                        } => Some((face_ordinal, reference)),
                        _ => None,
                    },
                )
                .collect::<Option<Vec<_>>>();
            let Some(resolved) = resolved else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            let mut response = format!(
                "OK_P6_SWEEP_V1 {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
            );
            for (face_ordinal, reference) in resolved {
                response.push_str(&format!(
                    " {face_ordinal} {} {}",
                    reference.corroborating_geometry_fingerprint, reference.lineage_digest,
                ));
            }
            response
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p6_offset_response(
    backend: &ExactBackend,
    bits: [u64; 5],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.offset_rectangle(RectangleOffsetSpec {
        min_mm: [f64::from_bits(bits[0]), f64::from_bits(bits[1])],
        max_mm: [f64::from_bits(bits[2]), f64::from_bits(bits[3])],
        distance_mm: f64::from_bits(bits[4]),
    });
    format_p6_offset_response(
        result,
        started.elapsed().as_nanos(),
        document_id,
        producer_feature_id,
        request_digest,
        false,
    )
}

fn p6_profile_offset_response(
    backend: &ExactBackend,
    profile: PlanarProfileLoop,
    distance_mm: f64,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.offset_planar_profile(&profile, distance_mm);
    format_p6_offset_response(
        result,
        started.elapsed().as_nanos(),
        document_id,
        producer_feature_id,
        request_digest,
        false,
    )
}

fn p6_region_offset_response(
    backend: &ExactBackend,
    outer: PlanarProfileLoop,
    holes: Vec<PlanarProfileLoop>,
    distance_mm: f64,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let result = backend.offset_planar_region(&outer, &holes, distance_mm);
    format_p6_offset_response(
        result,
        started.elapsed().as_nanos(),
        document_id,
        producer_feature_id,
        request_digest,
        true,
    )
}

fn format_p6_offset_response(
    result: Result<ExactOpOutput, ketchup_exact::GeometryError>,
    elapsed: u128,
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
    include_wire_count: bool,
) -> String {
    match result {
        Ok(mut output) => {
            let reference = match capture_planar_offset_reference(
                &mut output,
                document_id,
                producer_feature_id,
            ) {
                Ok(reference) => reference,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let ReferenceResolution::Resolved {
                face_ordinal,
                migrated_backend: false,
            } = resolve_subshape_reference(&reference, &output)
            else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            let face = &topology.faces[face_ordinal as usize];
            if include_wire_count {
                format!(
                    "OK_P6_OFFSET_V3 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {} {request_digest} {} {} {} {} {} {}",
                    output.body.result_fingerprint,
                    face.area_mm2.to_bits(),
                    topology.bounds_mm.min.x.to_bits(),
                    topology.bounds_mm.min.y.to_bits(),
                    topology.bounds_mm.min.z.to_bits(),
                    topology.bounds_mm.max.x.to_bits(),
                    topology.bounds_mm.max.y.to_bits(),
                    topology.bounds_mm.max.z.to_bits(),
                    topology.vertex_count,
                    topology.edge_count,
                    topology.wire_count,
                    topology.face_count,
                    topology.shell_count,
                    topology.solid_count,
                    output.input_digest,
                    output.backend_fingerprint,
                    output.tolerance_report.profile,
                    face_ordinal,
                    reference.corroborating_geometry_fingerprint,
                    reference.lineage_digest,
                )
            } else {
                format!(
                    "OK_P6_OFFSET_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {}",
                    output.body.result_fingerprint,
                    face.area_mm2.to_bits(),
                    topology.bounds_mm.min.x.to_bits(),
                    topology.bounds_mm.min.y.to_bits(),
                    topology.bounds_mm.min.z.to_bits(),
                    topology.bounds_mm.max.x.to_bits(),
                    topology.bounds_mm.max.y.to_bits(),
                    topology.bounds_mm.max.z.to_bits(),
                    topology.vertex_count,
                    topology.edge_count,
                    topology.face_count,
                    topology.shell_count,
                    topology.solid_count,
                    output.input_digest,
                    output.backend_fingerprint,
                    output.tolerance_report.profile,
                    face_ordinal,
                    reference.corroborating_geometry_fingerprint,
                    reference.lineage_digest,
                )
            }
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p6_split_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    tool_bits: [u64; 4],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let tool = BoxSpec {
        origin_mm: Point3 {
            x: f64::from_bits(tool_bits[0]),
            y: f64::from_bits(tool_bits[1]),
            z: 0.0,
        },
        size_mm: Size3 {
            x: f64::from_bits(tool_bits[2]),
            y: f64::from_bits(tool_bits[3]),
            z: base.height_mm,
        },
    };
    let result = backend
        .extrude_rectangle(base)
        .and_then(|base_output| backend.split_box(&base_output.body, tool));
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_rectangular_split_references(
                &mut output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let evidence = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
            ]
            .map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let mut ordinals = output
                    .topology_history
                    .iter()
                    .filter(|entry| {
                        entry.relation == "rectangular_split_classification"
                            && entry.semantic_role.as_deref() == Some(role)
                            && entry.source_element_id == source
                    })
                    .filter_map(|entry| entry.output_face_ordinal)
                    .collect::<Vec<_>>();
                ordinals.sort_unstable();
                ordinals.dedup();
                let [face_ordinal] = ordinals.as_slice() else {
                    return Err(());
                };
                let face_matches = output.body.topology.faces.iter().any(|face| {
                    face.ordinal == *face_ordinal
                        && face.geometric_fingerprint
                            == reference.corroborating_geometry_fingerprint
                });
                if !face_matches {
                    return Err(());
                }
                Ok((*face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn m3_union_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    tool_bits: [u64; 4],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let tool = BoxSpec {
        origin_mm: Point3 {
            x: f64::from_bits(tool_bits[0]),
            y: f64::from_bits(tool_bits[1]),
            z: 0.0,
        },
        size_mm: Size3 {
            x: f64::from_bits(tool_bits[2]),
            y: f64::from_bits(tool_bits[3]),
            z: base.height_mm,
        },
    };
    let result = backend
        .extrude_rectangle(base)
        .and_then(|base_output| backend.fuse_box(&base_output.body, tool));
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_rectangular_union_references(
                &mut output,
                document_id,
                producer_feature_id,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let evidence = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
            ]
            .map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [Ok(top), Ok(bottom), Ok(east)] = evidence else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn m3_pocket_response(
    backend: &ExactBackend,
    base_bits: [u64; 3],
    pocket_bits: [u64; 5],
    document_id: &str,
    producer_feature_id: &str,
    request_digest: &str,
) -> String {
    let started = Instant::now();
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(base_bits[0]),
        depth_mm: f64::from_bits(base_bits[1]),
        height_mm: f64::from_bits(base_bits[2]),
    };
    let depth_mm = f64::from_bits(pocket_bits[4]);
    let tool = BoxSpec {
        origin_mm: Point3 {
            x: f64::from_bits(pocket_bits[0]),
            y: f64::from_bits(pocket_bits[1]),
            z: base.height_mm - depth_mm,
        },
        size_mm: Size3 {
            x: f64::from_bits(pocket_bits[2]),
            y: f64::from_bits(pocket_bits[3]),
            z: depth_mm + 1.0,
        },
    };
    let result = backend
        .extrude_rectangle(base)
        .and_then(|base_output| backend.cut_box(&base_output.body, tool, CutMode::BlindPlanar));
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_bounded_pocket_references(
                &mut output,
                document_id,
                producer_feature_id,
                base,
                tool,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let roles = [
                ("extrusion.top", "profile.face"),
                ("extrusion.bottom", "profile.face"),
                ("extrusion.side(profile_edge=east)", "profile.edge.east"),
                ("pocket.floor", "pocket_profile.face"),
                ("pocket.wall.west", "pocket_profile.edge.west"),
                ("pocket.wall.east", "pocket_profile.edge.east"),
                ("pocket.wall.south", "pocket_profile.edge.south"),
                ("pocket.wall.north", "pocket_profile.edge.north"),
            ];
            let evidence = roles.map(|(role, source)| {
                let candidates = references
                    .iter()
                    .filter(|reference| {
                        reference.semantic_role == role && reference.source_element_id == source
                    })
                    .collect::<Vec<_>>();
                let [reference] = candidates.as_slice() else {
                    return Err(());
                };
                if reference.document_id != document_id
                    || reference.producer_feature_id != producer_feature_id
                    || reference.expected_type != "planar_face"
                    || reference.stability_class != StabilityClass::Guaranteed
                    || reference.backend_fingerprint != output.backend_fingerprint
                    || reference.lineage_digest.is_empty()
                    || reference.corroborating_geometry_fingerprint.is_empty()
                {
                    return Err(());
                }
                let ReferenceResolution::Resolved {
                    face_ordinal,
                    migrated_backend: false,
                } = resolve_subshape_reference(reference, &output)
                else {
                    return Err(());
                };
                Ok((face_ordinal, *reference))
            });
            let [
                Ok(top),
                Ok(bottom),
                Ok(east),
                Ok(floor),
                Ok(west),
                Ok(pocket_east),
                Ok(south),
                Ok(north),
            ] = evidence
            else {
                return "ERR incomplete_history".to_owned();
            };
            let topology = &output.body.topology;
            format!(
                "OK_M3_POCKET_V1 {elapsed} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {request_digest} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                output.body.result_fingerprint,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                topology.solid_count,
                output.input_digest,
                output.backend_fingerprint,
                output.tolerance_report.profile,
                top.0,
                top.1.corroborating_geometry_fingerprint,
                top.1.lineage_digest,
                bottom.0,
                bottom.1.corroborating_geometry_fingerprint,
                bottom.1.lineage_digest,
                east.0,
                east.1.corroborating_geometry_fingerprint,
                east.1.lineage_digest,
                floor.0,
                floor.1.corroborating_geometry_fingerprint,
                floor.1.lineage_digest,
                west.0,
                west.1.corroborating_geometry_fingerprint,
                west.1.lineage_digest,
                pocket_east.0,
                pocket_east.1.corroborating_geometry_fingerprint,
                pocket_east.1.lineage_digest,
                south.0,
                south.1.corroborating_geometry_fingerprint,
                south.1.lineage_digest,
                north.0,
                north.1.corroborating_geometry_fingerprint,
                north.1.lineage_digest,
            )
        }
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn p4_revolve_response(
    backend: &ExactBackend,
    document_id: &str,
    producer_feature_id: &str,
    fields: &[&str],
) -> String {
    if fields.len() < 9
        || document_id.parse::<u64>().is_err()
        || producer_feature_id.parse::<u64>().is_err()
        || !is_canonical_digest(fields[0])
    {
        return "ERR invalid_request".to_owned();
    }
    let parse_bits = |value: &str| u64::from_str_radix(value, 16).map(f64::from_bits);
    let Some(axis_angle) = fields[1..=5]
        .iter()
        .map(|value| parse_bits(value))
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .and_then(|values| <[f64; 5]>::try_from(values).ok())
    else {
        return "ERR invalid_parameter".to_owned();
    };
    let Ok(segment_count) = fields[6].parse::<usize>() else {
        return "ERR invalid_parameter".to_owned();
    };
    if !(2..=64).contains(&segment_count) || fields.len() != segment_count + 7 {
        return "ERR invalid_request".to_owned();
    }
    let Some(segments) = fields[7..]
        .iter()
        .map(|token| parse_profile_segment(token))
        .collect::<Option<Vec<_>>>()
    else {
        return "ERR invalid_parameter".to_owned();
    };
    let angle_degrees = axis_angle[4];
    let output = match backend.revolve_general_profile(
        &segments,
        [axis_angle[0], axis_angle[1]],
        [axis_angle[2], axis_angle[3]],
        angle_degrees,
    ) {
        Ok(output) => output,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    let references = match capture_general_revolve_references(
        &output,
        document_id,
        producer_feature_id,
        angle_degrees < 360.0,
    ) {
        Ok(references) => references,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    let mut evidence = Vec::with_capacity(references.len());
    for reference in references {
        if reference.document_id != document_id
            || reference.producer_feature_id != producer_feature_id
            || reference.expected_type != "face"
            || reference.stability_class != StabilityClass::Guaranteed
            || reference.backend_fingerprint != output.backend_fingerprint
            || reference.lineage_digest.is_empty()
            || reference.corroborating_geometry_fingerprint.is_empty()
        {
            return "ERR incomplete_history".to_owned();
        }
        let ReferenceResolution::Resolved {
            face_ordinal,
            migrated_backend: false,
        } = resolve_subshape_reference(&reference, &output)
        else {
            return "ERR incomplete_history".to_owned();
        };
        evidence.push((face_ordinal, reference));
    }
    let topology = &output.body.topology;
    let mut response = format!(
        "OK_P4_REVOLVE_V1 0 {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {} {} {} {} {}",
        output.body.result_fingerprint,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
        fields[0],
        output.input_digest,
        output.backend_fingerprint,
        output.tolerance_report.profile,
        evidence.len(),
    );
    for (ordinal, reference) in evidence {
        response.push_str(&format!(
            " {ordinal} {} {}",
            reference.corroborating_geometry_fingerprint, reference.lineage_digest,
        ));
    }
    response
}

fn m6_revolve_response(
    backend: &ExactBackend,
    document_id: &str,
    producer_feature_id: &str,
    fields: &[&str],
    operation: Option<&str>,
) -> String {
    let finish = match operation {
        Some("fillet") => Some(EdgeFinish::Fillet),
        Some("chamfer") => Some(EdgeFinish::Chamfer),
        _ => None,
    };
    let shell = operation.is_some();
    let expected_fields = if finish.is_some() {
        16
    } else if shell {
        14
    } else {
        13
    };
    if document_id.parse::<u64>().is_err()
        || producer_feature_id.parse::<u64>().is_err()
        || fields.len() != expected_fields
        || !is_canonical_digest(fields[0])
    {
        return "ERR invalid_request".to_owned();
    }
    let parse_bits = |value: &str| u64::from_str_radix(value, 16).map(f64::from_bits);
    let thickness_mm = shell
        .then(|| parse_bits(fields[1]))
        .transpose()
        .ok()
        .flatten();
    let amount_mm = finish
        .map(|_| parse_bits(fields[3]))
        .transpose()
        .ok()
        .flatten();
    if shell && thickness_mm.is_none() || finish.is_some() && amount_mm.is_none() {
        return "ERR invalid_parameter".to_owned();
    }
    let point_offset = if finish.is_some() {
        4
    } else if shell {
        2
    } else {
        1
    };
    let mut points = Vec::with_capacity(6);
    for pair in fields[point_offset..].chunks_exact(2) {
        let (Ok(radius), Ok(z)) = (parse_bits(pair[0]), parse_bits(pair[1])) else {
            return "ERR invalid_parameter".to_owned();
        };
        points.push([radius, z]);
    }
    let output = match (thickness_mm, finish, amount_mm) {
        (Some(thickness), Some(finish), Some(amount)) => {
            backend.finish_shell_revolve_profile(&points, thickness, finish, amount)
        }
        (Some(thickness), None, None) => backend.shell_revolve_profile(&points, thickness),
        (None, None, None) => backend.revolve_profile(&points),
        _ => return "ERR invalid_request".to_owned(),
    };
    let output = match output {
        Ok(output) => output,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    let references = if shell {
        capture_shell_references(&output, document_id, producer_feature_id)
    } else {
        capture_revolve_references(&output, document_id, producer_feature_id)
    };
    let references = match references {
        Ok(references) => references,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    let roles = if shell {
        vec![
            ("shell.outer.bottom", "revolve.face.bottom"),
            ("shell.outer.body", "revolve.face.body"),
            ("shell.outer.shoulder", "revolve.face.shoulder"),
            ("shell.outer.neck", "revolve.face.neck"),
            ("shell.rim", "revolve.face.mouth"),
            ("shell.inner.bottom", "shell.offset.bottom"),
            ("shell.inner.body", "shell.offset.body"),
            ("shell.inner.shoulder", "shell.offset.shoulder"),
            ("shell.inner.neck", "shell.offset.neck"),
        ]
    } else {
        vec![
            ("revolve.bottom", "profile.edge.0"),
            ("revolve.body", "profile.edge.1"),
            ("revolve.shoulder", "profile.edge.2"),
            ("revolve.neck", "profile.edge.3"),
            ("revolve.mouth", "profile.edge.4"),
        ]
    };
    let mut evidence = Vec::with_capacity(roles.len());
    for (role, source) in roles {
        let matching = references
            .iter()
            .filter(|reference| {
                reference.semantic_role == role && reference.source_element_id == source
            })
            .collect::<Vec<_>>();
        let [reference] = matching.as_slice() else {
            return "ERR incomplete_history".to_owned();
        };
        if reference.document_id != document_id
            || reference.producer_feature_id != producer_feature_id
            || reference.expected_type != "face"
            || reference.stability_class != StabilityClass::Guaranteed
            || reference.backend_fingerprint != output.backend_fingerprint
            || reference.lineage_digest.is_empty()
            || reference.corroborating_geometry_fingerprint.is_empty()
        {
            return "ERR incomplete_history".to_owned();
        }
        let ReferenceResolution::Resolved {
            face_ordinal,
            migrated_backend: false,
        } = resolve_subshape_reference(reference, &output)
        else {
            return "ERR incomplete_history".to_owned();
        };
        evidence.push((face_ordinal, *reference));
    }
    let topology = &output.body.topology;
    let response_kind = if finish.is_some() {
        "OK_M6_FINISH_V1"
    } else if shell {
        "OK_M6_SHELL_V1"
    } else {
        "OK_M6_REVOLVE_V1"
    };
    let mut response = format!(
        "{response_kind} 0 {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {} {} {} {}",
        output.body.result_fingerprint,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
        fields[0],
        output.input_digest,
        output.backend_fingerprint,
        output.tolerance_report.profile,
    );
    for (ordinal, reference) in evidence {
        response.push_str(&format!(
            " {ordinal} {} {}",
            reference.corroborating_geometry_fingerprint, reference.lineage_digest,
        ));
    }
    response
}

fn m14_step_export_response(
    backend: &ExactBackend,
    document_id: &str,
    producer_feature_id: &str,
    fields: &[&str],
) -> String {
    if document_id.parse::<u64>().is_err()
        || producer_feature_id.parse::<u64>().is_err()
        || fields.len() != 18
        || !is_canonical_digest(fields[0])
        || fields[1].len() != 24
        || !fields[1].starts_with("fnv1a64:")
        || !fields[1][8..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return "ERR invalid_request".to_owned();
    }
    let parse_bits = |value: &str| u64::from_str_radix(value, 16).map(f64::from_bits);
    let mut points = Vec::with_capacity(6);
    for pair in fields[6..].chunks_exact(2) {
        let (Ok(radius), Ok(z)) = (parse_bits(pair[0]), parse_bits(pair[1])) else {
            return "ERR invalid_parameter".to_owned();
        };
        points.push([radius, z]);
    }
    let output = match fields[2] {
        "revolve" if fields[3] == "-" && fields[4] == "-" => backend.revolve_profile(&points),
        "shell" if fields[4] == "-" => {
            let Ok(thickness) = parse_bits(fields[3]) else {
                return "ERR invalid_parameter".to_owned();
            };
            backend.shell_revolve_profile(&points, thickness)
        }
        "fillet" | "chamfer" => {
            let (Ok(thickness), Ok(amount)) = (parse_bits(fields[3]), parse_bits(fields[4])) else {
                return "ERR invalid_parameter".to_owned();
            };
            backend.finish_shell_revolve_profile(
                &points,
                thickness,
                if fields[2] == "fillet" {
                    EdgeFinish::Fillet
                } else {
                    EdgeFinish::Chamfer
                },
                amount,
            )
        }
        _ => return "ERR invalid_request".to_owned(),
    };
    let output = match output {
        Ok(output) => output,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    if output.body.result_fingerprint != fields[1] {
        return "ERR invalid_shape".to_owned();
    }
    let Some(path) = decode_hex_utf8(fields[5]) else {
        return "ERR invalid_request".to_owned();
    };
    match backend.export_step(&output.body, &path) {
        Ok(()) => format!("OK_M14_STEP_V1 {} {}", fields[0], fields[1]),
        Err(error) => format!("ERR {}", error.code.as_str()),
    }
}

fn m21_revolve_step_export_response(
    backend: &ExactBackend,
    document_id: &str,
    producer_feature_id: &str,
    fields: &[&str],
) -> String {
    if document_id.parse::<u64>().is_err()
        || producer_feature_id.parse::<u64>().is_err()
        || fields.len() != 4
        || !is_canonical_digest(fields[0])
        || !is_result_fingerprint(fields[1])
    {
        return "ERR invalid_request".to_owned();
    }
    let Some(path) = decode_hex_utf8(fields[2]) else {
        return "ERR invalid_request".to_owned();
    };
    let Some(encoded) = decode_hex_utf8(fields[3]) else {
        return "ERR invalid_request".to_owned();
    };
    let Ok(specification) = serde_json::from_str::<StepRevolveExportSpec>(&encoded) else {
        return "ERR invalid_request".to_owned();
    };
    let segments = specification
        .segments
        .iter()
        .map(step_profile_segment)
        .collect::<Vec<_>>();
    let output = match backend.revolve_general_profile(
        &segments,
        specification.axis_start_bits.map(f64::from_bits),
        specification.axis_end_bits.map(f64::from_bits),
        f64::from_bits(specification.angle_degrees_bits),
    ) {
        Ok(output) => output,
        Err(error) => return geometry_error_response(&error),
    };
    if output.body.result_fingerprint != fields[1] {
        return "ERR invalid_shape".to_owned();
    }
    match backend.export_step(&output.body, &path) {
        Ok(()) => format!("OK_M21_REVOLVE_STEP_V1 {} {}", fields[0], fields[1]),
        Err(error) => geometry_error_response(&error),
    }
}

fn step_profile_segment(segment: &StepProfileSegment) -> PlanarProfileSegment {
    match segment {
        StepProfileSegment::Line {
            start_bits,
            end_bits,
        } => PlanarProfileSegment::Line {
            start_mm: start_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
        },
        StepProfileSegment::Arc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => PlanarProfileSegment::CircularArc {
            start_mm: start_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
            center_mm: center_bits.map(f64::from_bits),
            clockwise: *clockwise,
        },
    }
}

fn m21_box_step_export_response(
    backend: &ExactBackend,
    document_id: &str,
    producer_feature_id: &str,
    fields: &[&str],
) -> String {
    if document_id.parse::<u64>().is_err()
        || producer_feature_id.parse::<u64>().is_err()
        || fields.len() != 4
        || !is_canonical_digest(fields[0])
        || fields[1].len() != 24
        || !fields[1].starts_with("fnv1a64:")
    {
        return "ERR invalid_request".to_owned();
    }
    let Some(path) = decode_hex_utf8(fields[2]) else {
        return "ERR invalid_request".to_owned();
    };
    let Some(encoded) = decode_hex_utf8(fields[3]) else {
        return "ERR invalid_request".to_owned();
    };
    let Ok(specification) = serde_json::from_str::<StepFeatureExportSpec>(&encoded) else {
        return "ERR invalid_request".to_owned();
    };
    let base = RectangleExtrudeSpec {
        width_mm: f64::from_bits(specification.width_bits),
        depth_mm: f64::from_bits(specification.depth_bits),
        height_mm: f64::from_bits(specification.height_bits),
    };
    let output = if let Some(shell) = specification.shell {
        let thickness = f64::from_bits(shell.thickness_bits);
        match (shell.finish.as_deref(), shell.amount_bits) {
            (None, None) => backend.shell_box(base, thickness),
            (Some(kind), Some(amount)) => backend.finish_shell_box(
                base,
                thickness,
                if kind == "fillet" {
                    EdgeFinish::Fillet
                } else if kind == "chamfer" {
                    EdgeFinish::Chamfer
                } else {
                    return "ERR invalid_request".to_owned();
                },
                f64::from_bits(amount),
            ),
            _ => return "ERR invalid_request".to_owned(),
        }
    } else {
        let initial = if let Some(circle) = specification.circle {
            backend.extrude_circle(CircleExtrudeSpec {
                center_mm: [
                    f64::from_bits(circle.center_x_bits),
                    f64::from_bits(circle.center_y_bits),
                ],
                radius_mm: f64::from_bits(circle.radius_bits),
                height_mm: base.height_mm,
            })
        } else if !specification.mixed_segments.is_empty() {
            let segments = specification
                .mixed_segments
                .iter()
                .map(|segment| match segment {
                    StepProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => PlanarProfileSegment::Line {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                    StepProfileSegment::Arc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => PlanarProfileSegment::CircularArc {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                        center_mm: center_bits.map(f64::from_bits),
                        clockwise: *clockwise,
                    },
                })
                .collect::<Vec<_>>();
            backend.extrude_mixed_profile(&segments, base.height_mm)
        } else {
            backend.extrude_rectangle(base)
        };
        initial.and_then(|base_output| {
            let Some(boolean) = specification.boolean else {
                return Ok(base_output);
            };
            if let Some(circle) = boolean.circle {
                let center_mm = [
                    f64::from_bits(circle.center_x_bits),
                    f64::from_bits(circle.center_y_bits),
                ];
                let radius_mm = f64::from_bits(circle.radius_bits);
                if let Some(depth_bits) = specification.pocket_depth_bits {
                    let depth_mm = f64::from_bits(depth_bits);
                    if boolean.operation != "cut"
                        || !depth_mm.is_finite()
                        || depth_mm <= 0.0
                        || depth_mm >= base.height_mm
                    {
                        return Err(ketchup_exact::GeometryError {
                            code: ketchup_exact::GeometryErrorCode::InvalidParameter,
                            diagnostic: "invalid circular STEP pocket depth".to_owned(),
                            operation: "export_feature_step",
                            input_digest: fields[0].to_owned(),
                            backend_fingerprint: ketchup_exact::backend_fingerprint(),
                        });
                    }
                    return backend.cut_cylinder(
                        &base_output.body,
                        CylinderToolSpec {
                            center_mm,
                            origin_z_mm: base.height_mm - depth_mm,
                            radius_mm,
                            height_mm: depth_mm + 1.0,
                        },
                        CutMode::BlindPlanar,
                    );
                }
                let tool = CylinderToolSpec {
                    center_mm,
                    origin_z_mm: 0.0,
                    radius_mm,
                    height_mm: base.height_mm,
                };
                return match boolean.operation.as_str() {
                    "union" => backend.fuse_cylinder(&base_output.body, tool),
                    "intersect" => backend.common_cylinder(&base_output.body, tool),
                    "split" => backend.split_cylinder(&base_output.body, tool),
                    "cut" => backend.cut_cylinder(
                        &base_output.body,
                        CylinderToolSpec {
                            origin_z_mm: -1.0,
                            height_mm: base.height_mm + 2.0,
                            ..tool
                        },
                        CutMode::ThroughAll,
                    ),
                    _ => Err(ketchup_exact::GeometryError {
                        code: ketchup_exact::GeometryErrorCode::InvalidParameter,
                        diagnostic: "unsupported circular STEP boolean".to_owned(),
                        operation: "export_feature_step",
                        input_digest: fields[0].to_owned(),
                        backend_fingerprint: ketchup_exact::backend_fingerprint(),
                    }),
                };
            }
            if !boolean.mixed_segments.is_empty() {
                let segments = boolean
                    .mixed_segments
                    .iter()
                    .map(step_profile_segment)
                    .collect::<Vec<_>>();
                if specification.pocket_depth_bits.is_none() {
                    if boolean.operation == "union" {
                        return backend.fuse_mixed_profile(
                            &base_output.body,
                            &segments,
                            0.0,
                            base.height_mm,
                        );
                    }
                    if boolean.operation == "intersect" {
                        return backend.common_mixed_profile(
                            &base_output.body,
                            &segments,
                            0.0,
                            base.height_mm,
                        );
                    }
                    if boolean.operation == "split" {
                        return backend.split_mixed_profile(
                            &base_output.body,
                            &segments,
                            0.0,
                            base.height_mm,
                        );
                    }
                }
                if boolean.operation != "cut" {
                    return Err(ketchup_exact::GeometryError {
                        code: ketchup_exact::GeometryErrorCode::InvalidParameter,
                        diagnostic: "unsupported mixed-profile STEP boolean".to_owned(),
                        operation: "export_feature_step",
                        input_digest: fields[0].to_owned(),
                        backend_fingerprint: ketchup_exact::backend_fingerprint(),
                    });
                }
                let (origin_z_mm, cut_height_mm) = specification
                    .pocket_depth_bits
                    .map(f64::from_bits)
                    .map_or(Ok((-1.0, base.height_mm + 2.0)), |depth| {
                        if depth.is_finite() && depth > 0.0 && depth < base.height_mm {
                            Ok((base.height_mm - depth, depth))
                        } else {
                            Err(ketchup_exact::GeometryError {
                                code: ketchup_exact::GeometryErrorCode::InvalidParameter,
                                diagnostic: "invalid mixed-profile STEP pocket depth".to_owned(),
                                operation: "export_feature_step",
                                input_digest: fields[0].to_owned(),
                                backend_fingerprint: ketchup_exact::backend_fingerprint(),
                            })
                        }
                    })?;
                return backend.cut_mixed_profile(
                    &base_output.body,
                    &segments,
                    origin_z_mm,
                    cut_height_mm,
                );
            }
            let depth = specification.pocket_depth_bits.map(f64::from_bits);
            let (tool_z, tool_height) = if boolean.operation == "cut" {
                depth.map_or((-1.0, base.height_mm + 2.0), |depth| {
                    (base.height_mm - depth, depth + 1.0)
                })
            } else {
                (0.0, base.height_mm)
            };
            let tool = BoxSpec {
                origin_mm: Point3 {
                    x: f64::from_bits(boolean.min_x_bits),
                    y: f64::from_bits(boolean.min_y_bits),
                    z: tool_z,
                },
                size_mm: Size3 {
                    x: f64::from_bits(boolean.width_bits),
                    y: f64::from_bits(boolean.depth_bits),
                    z: tool_height,
                },
            };
            match boolean.operation.as_str() {
                "cut" => backend.cut_box(
                    &base_output.body,
                    tool,
                    if depth.is_some() {
                        CutMode::BlindPlanar
                    } else {
                        CutMode::ThroughAll
                    },
                ),
                "union" => backend.fuse_box(&base_output.body, tool),
                "intersect" => backend.common_box(&base_output.body, tool),
                "split" => backend.split_box(&base_output.body, tool),
                _ => Err(ketchup_exact::GeometryError {
                    code: ketchup_exact::GeometryErrorCode::InvalidParameter,
                    diagnostic: "unsupported STEP boolean".to_owned(),
                    operation: "export_feature_step",
                    input_digest: fields[0].to_owned(),
                    backend_fingerprint: ketchup_exact::backend_fingerprint(),
                }),
            }
        })
    };
    let output = match output {
        Ok(output) => output,
        Err(error) => return geometry_error_response(&error),
    };
    if output.body.result_fingerprint != fields[1] {
        return "ERR invalid_shape".to_owned();
    }
    match backend.export_step(&output.body, &path) {
        Ok(()) => format!("OK_M21_BOX_STEP_V1 {} {}", fields[0], fields[1]),
        Err(error) => geometry_error_response(&error),
    }
}

/// Fixed angular deflection so the same part always meshes identically.
const STEP_MESH_ANGULAR_DEFLECTION: f64 = 0.35;

fn step_import_result_fingerprint(
    source_sha256: &str,
    output: &ketchup_exact::ExactOpOutput,
) -> String {
    let topology = &output.body.topology;
    let signature = format!(
        "{source_sha256}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        output.backend_fingerprint,
        output.tolerance_report.profile,
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
    );
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in signature.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn verified_step_copy(
    source_path: &str,
    source_sha256: &str,
    operation: &'static str,
) -> Result<tempfile::NamedTempFile, String> {
    let mut source = std::fs::File::open(source_path)
        .map_err(|error| transport_error_response(operation, &error.to_string()))?;
    let mut source_bytes = Vec::new();
    std::io::Read::by_ref(&mut source)
        .take(MAX_STEP_SOURCE_BYTES + 1)
        .read_to_end(&mut source_bytes)
        .map_err(|error| transport_error_response(operation, &error.to_string()))?;
    if source_bytes.len() as u64 > MAX_STEP_SOURCE_BYTES {
        return Err(transport_error_response(
            operation,
            "STEP source exceeds the bounded 32 MiB envelope",
        ));
    }
    if sha256_hex(&source_bytes) != source_sha256 {
        return Err(transport_error_response(
            operation,
            "STEP part bytes do not match the declared SHA-256",
        ));
    }
    let mut copy = tempfile::Builder::new()
        .prefix("ketchup-verified-step-")
        .suffix(".step")
        .tempfile()
        .map_err(|error| transport_error_response(operation, &error.to_string()))?;
    copy.write_all(&source_bytes)
        .and_then(|()| copy.flush())
        .map_err(|error| transport_error_response(operation, &error.to_string()))?;
    Ok(copy)
}

fn m21_step_part_inspection_response(
    backend: &ExactBackend,
    source_sha256: &str,
    source_path: &str,
) -> String {
    if !is_canonical_digest(source_sha256) {
        return "ERR invalid_request".to_owned();
    }
    let Some(source_path) = decode_hex_utf8(source_path) else {
        return "ERR invalid_request".to_owned();
    };
    let source = match verified_step_copy(&source_path, source_sha256, "inspect_step_part") {
        Ok(source) => source,
        Err(response) => return response,
    };
    let source_path = source.path().to_string_lossy();
    let Some(source_unit) = backend.step_length_unit_name(&source_path) else {
        return transport_error_response(
            "inspect_step_part",
            "STEP source has missing or ambiguous representation length units",
        );
    };
    match backend.import_step(&source_path) {
        Ok(output) => {
            let topology = &output.body.topology;
            format!(
                "OK_M21_STEP_PART_V3 {source_sha256} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {} {}",
                step_import_result_fingerprint(source_sha256, &output),
                topology.solid_count,
                topology.volume_mm3.to_bits(),
                topology.bounds_mm.min.x.to_bits(),
                topology.bounds_mm.min.y.to_bits(),
                topology.bounds_mm.min.z.to_bits(),
                topology.bounds_mm.max.x.to_bits(),
                topology.bounds_mm.max.y.to_bits(),
                topology.bounds_mm.max.z.to_bits(),
                topology.vertex_count,
                topology.edge_count,
                topology.face_count,
                topology.shell_count,
                encode_hex(source_unit.as_bytes()),
                encode_hex(output.backend_fingerprint.as_bytes()),
                encode_hex(output.tolerance_report.profile.as_bytes()),
            )
        }
        Err(error) => geometry_error_response(&error),
    }
}

/// Tessellate an already-inspected STEP part into a bounded display mesh.
///
/// The mesh is written to the caller's path and identified by its own digest;
/// the response repeats the result fingerprint so the caller can only bind a
/// mesh to the exact body it already committed to.
fn m21_step_part_mesh_response(
    backend: &ExactBackend,
    source_sha256: &str,
    source_path: &str,
    fields: &[&str],
) -> String {
    if !is_canonical_digest(source_sha256) || fields.len() != 1 {
        return "ERR invalid_request".to_owned();
    }
    let (Some(source_path), Some(output_path)) =
        (decode_hex_utf8(source_path), decode_hex_utf8(fields[0]))
    else {
        return "ERR invalid_request".to_owned();
    };
    let source = match verified_step_copy(&source_path, source_sha256, "tessellate_step_part") {
        Ok(source) => source,
        Err(response) => return response,
    };
    let output = match backend.import_step(&source.path().to_string_lossy()) {
        Ok(output) => output,
        Err(error) => return geometry_error_response(&error),
    };
    let bounds = output.body.topology.bounds_mm;
    let diagonal = ((bounds.max.x - bounds.min.x).powi(2)
        + (bounds.max.y - bounds.min.y).powi(2)
        + (bounds.max.z - bounds.min.z).powi(2))
    .sqrt();
    if !diagonal.is_finite() || diagonal <= 0.0 {
        return transport_error_response(
            "tessellate_step_part",
            "STEP part has no measurable extent to tessellate",
        );
    }
    let deflection = (diagonal * 1.0e-3).max(1.0e-3);
    let mesh = match backend.tessellate_body(
        &output.body,
        deflection,
        STEP_MESH_ANGULAR_DEFLECTION,
        MAX_STEP_MESH_TRIANGLES,
    ) {
        Ok(mesh) => mesh,
        Err(error) => return geometry_error_response(&error),
    };
    let mesh = StepImportMesh {
        vertices_mm: mesh.vertices_mm,
        triangles: mesh
            .triangles
            .into_iter()
            .map(|triangle| StepMeshTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: triangle.face_ordinal,
            })
            .collect(),
    };
    let encoded = mesh.encode();
    if let Err(error) = std::fs::write(&output_path, &encoded) {
        return transport_error_response("tessellate_step_part", &error.to_string());
    }
    format!(
        "OK_M21_STEP_MESH_V1 {source_sha256} {} {} {} {} {:016x}",
        step_import_result_fingerprint(source_sha256, &output),
        mesh.vertices_mm.len(),
        mesh.triangles.len(),
        sha256_hex(&encoded),
        deflection.to_bits(),
    )
}

fn m21_step_assembly_response(
    backend: &ExactBackend,
    assembly_digest: &str,
    output_path: &str,
    fields: &[&str],
) -> String {
    if !is_canonical_digest(assembly_digest) || fields.len() < 2 {
        return "ERR invalid_request".to_owned();
    }
    let Some(output_path) = decode_hex_utf8(output_path) else {
        return "ERR invalid_request".to_owned();
    };
    let Some(encoded_manifest) = decode_hex_bytes(fields[0]) else {
        return "ERR invalid_request".to_owned();
    };
    if sha256_hex(&encoded_manifest) != assembly_digest {
        return transport_error_response(
            "verify_step_manifest",
            "STEP assembly manifest digest mismatch",
        );
    }
    let Ok(manifest) = serde_json::from_slice::<StepAssemblyManifest>(&encoded_manifest) else {
        return "ERR invalid_request".to_owned();
    };
    if !is_snapshot_digest(&manifest.source_digest) || manifest.parts.is_empty() {
        return transport_error_response(
            "verify_step_manifest",
            "STEP assembly manifest has no source digest or parts",
        );
    }
    let Some(count) = fields.get(1).and_then(|value| value.parse::<usize>().ok()) else {
        return "ERR invalid_request".to_owned();
    };
    if count != manifest.parts.len() || fields.len() != 2 + count {
        return "ERR invalid_request".to_owned();
    }

    let mut transformed = Vec::with_capacity(count);
    for (manifest_part, source_field) in manifest.parts.iter().zip(&fields[2..]) {
        if manifest_part.document_id != manifest.document_id
            || manifest_part.source_revision != manifest.source_revision
            || manifest_part.source_digest != manifest.source_digest
            || !is_result_fingerprint(&manifest_part.expected_result_fingerprint)
            || !is_result_fingerprint(&manifest_part.imported_result_fingerprint)
            || !is_canonical_digest(&manifest_part.source_sha256)
        {
            return transport_error_response(
                "verify_step_part",
                "STEP part manifest fingerprint or SHA-256 is malformed",
            );
        }
        let Some(source_path) = decode_hex_utf8(source_field) else {
            return "ERR invalid_request".to_owned();
        };
        let source = match verified_step_copy(
            &source_path,
            &manifest_part.source_sha256,
            "verify_step_part",
        ) {
            Ok(source) => source,
            Err(response) => return response,
        };
        let imported = match backend.import_step(&source.path().to_string_lossy()) {
            Ok(output) => output,
            Err(error) => return geometry_error_response(&error),
        };
        if step_import_result_fingerprint(&manifest_part.source_sha256, &imported)
            != manifest_part.imported_result_fingerprint
        {
            return transport_error_response(
                "verify_step_part",
                "STEP part reimport identity differs from the inspected manifest identity",
            );
        }
        let matrix = manifest_part.transform_bits.map(f64::from_bits);
        let body = match backend.transform_body(&imported.body, &matrix) {
            Ok(output) => output.body,
            Err(error) => return geometry_error_response(&error),
        };
        transformed.push(body);
    }

    let mut bodies = transformed.into_iter();
    let Some(mut assembly) = bodies.next() else {
        return "ERR invalid_request".to_owned();
    };
    for body in bodies {
        assembly = match backend.combine_bodies(&assembly, &body) {
            Ok(output) => output.body,
            Err(error) => return geometry_error_response(&error),
        };
    }
    if let Err(error) = backend.export_step(&assembly, &output_path) {
        return geometry_error_response(&error);
    }
    let step_bytes = match std::fs::read(&output_path) {
        Ok(bytes) => bytes,
        Err(error) => return transport_error_response("read_step_output", &error.to_string()),
    };
    let step_sha256 = sha256_hex(&step_bytes);
    let reread = match backend.import_step(&output_path) {
        Ok(output) => output,
        Err(error) => return geometry_error_response(&error),
    };
    format!(
        "OK_M21_STEP_MODEL_V1 {assembly_digest} {} {step_sha256}",
        reread.body.result_fingerprint
    )
}

fn geometry_error_response(error: &ketchup_exact::GeometryError) -> String {
    format!(
        "ERR_DETAIL {} {} {} {} {}",
        error.code.as_str(),
        encode_hex(error.diagnostic.as_bytes()),
        encode_hex(error.operation.as_bytes()),
        error.input_digest,
        encode_hex(error.backend_fingerprint.as_bytes()),
    )
}

fn transport_error_response(operation: &str, diagnostic: &str) -> String {
    format!(
        "ERR_DETAIL backend_exception {} {} {} {}",
        encode_hex(diagnostic.as_bytes()),
        encode_hex(operation.as_bytes()),
        sha256_hex(diagnostic.as_bytes()),
        encode_hex(ketchup_exact::backend_fingerprint().as_bytes()),
    )
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn decode_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        })
        .collect()
}

fn decode_hex_utf8(value: &str) -> Option<String> {
    let bytes = decode_hex_bytes(value)?;
    String::from_utf8(bytes)
        .ok()
        .filter(|path| !path.is_empty() && !path.contains('\r') && !path.contains('\n'))
}

fn is_snapshot_digest(value: &str) -> bool {
    value.len() == 16 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_result_fingerprint(value: &str) -> bool {
    value.len() == 24
        && value.starts_with("fnv1a64:")
        && value[8..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(feature = "named-product-fixtures")]
fn m5_notched_response(
    backend: &ExactBackend,
    document_id: &str,
    piece_key: &str,
    fields: &[&str],
) -> String {
    if fields.len() < 8
        || document_id.parse::<u64>().is_err()
        || !is_canonical_digest(piece_key)
        || !is_canonical_digest(fields[0])
    {
        return "ERR invalid_request".to_owned();
    }
    let Ok(document_id) = document_id.parse::<u64>() else {
        return "ERR invalid_request".to_owned();
    };
    let request_digest = fields[0];
    let parse_number = |value: &str| {
        u64::from_str_radix(value, 16)
            .map(f64::from_bits)
            .map_err(|_| ())
    };
    let parse_bounds = |values: &[&str]| -> Result<BoxSpec, ()> {
        let [min_x, min_y, min_z, max_x, max_y, max_z] = values else {
            return Err(());
        };
        let min = [
            parse_number(min_x)?,
            parse_number(min_y)?,
            parse_number(min_z)?,
        ];
        let max = [
            parse_number(max_x)?,
            parse_number(max_y)?,
            parse_number(max_z)?,
        ];
        Ok(BoxSpec {
            origin_mm: Point3 {
                x: min[0],
                y: min[1],
                z: min[2],
            },
            size_mm: Size3 {
                x: max[0] - min[0],
                y: max[1] - min[1],
                z: max[2] - min[2],
            },
        })
    };
    let Ok(stock) = parse_bounds(&fields[1..7]) else {
        return "ERR invalid_parameter".to_owned();
    };
    let Ok(notch_count) = fields[7].parse::<usize>() else {
        return "ERR invalid_request".to_owned();
    };
    let Some(expected_len) = notch_count
        .checked_mul(9)
        .and_then(|count| count.checked_add(8))
    else {
        return "ERR invalid_request".to_owned();
    };
    if notch_count == 0 || fields.len() != expected_len {
        return "ERR invalid_request".to_owned();
    }
    let mut notches = Vec::with_capacity(notch_count);
    let mut previous_feature_ordinal = 0_u32;
    for index in 0..notch_count {
        let offset = 8 + index * 9;
        let Ok(joint_id) = fields[offset].parse::<u64>() else {
            return "ERR invalid_request".to_owned();
        };
        let participant = match fields[offset + 1] {
            "a" => HalfLapParticipant::A,
            "b" => HalfLapParticipant::B,
            _ => return "ERR invalid_request".to_owned(),
        };
        let Ok(feature_ordinal) = fields[offset + 2].parse::<u32>() else {
            return "ERR invalid_request".to_owned();
        };
        if feature_ordinal <= previous_feature_ordinal
            || notches
                .iter()
                .any(|notch: &HalfLapNotchSpec| notch.joint_id == joint_id)
        {
            return "ERR invalid_request".to_owned();
        }
        previous_feature_ordinal = feature_ordinal;
        let Ok(removed) = parse_bounds(&fields[(offset + 3)..(offset + 9)]) else {
            return "ERR invalid_parameter".to_owned();
        };
        notches.push(HalfLapNotchSpec {
            joint_id,
            participant,
            removed,
        });
    }
    let mut output = match backend.make_box(stock) {
        Ok(output) => output,
        Err(error) => return format!("ERR {}", error.code.as_str()),
    };
    for notch in &notches {
        output = match backend.cut_box(&output.body, notch.removed, CutMode::BlindPlanar) {
            Ok(output) => output,
            Err(error) => return format!("ERR {}", error.code.as_str()),
        };
    }
    let references =
        match capture_half_lap_notch_references(&output, document_id, piece_key, &notches) {
            Ok(references) => references,
            Err(error) => return format!("ERR {}", error.code.as_str()),
        };
    let topology = &output.body.topology;
    let mut response = format!(
        "OK_M5_V1 {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {} {} {} {} {} {} {} {}",
        output.body.result_fingerprint,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
        request_digest,
        output.input_digest,
        output.backend_fingerprint,
        output.tolerance_report.profile,
        references.len()
    );
    for reference in references {
        let role = match reference.role {
            HalfLapFaceRole::Contact => "contact",
            HalfLapFaceRole::WestWall => "wall.west",
            HalfLapFaceRole::EastWall => "wall.east",
        };
        response.push_str(&format!(
            " {} {} {} {} {} {}",
            reference.joint_id,
            reference.participant.token(),
            role,
            reference.face_ordinal,
            reference.geometric_fingerprint,
            reference.lineage_digest
        ));
    }
    response
}

fn is_canonical_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_request_line_reader_rejects_oversized_input() {
        let input = vec![b'x'; MAX_WORKER_REQUEST_LINE_BYTES + 1];
        let error = read_bounded_request_line(&mut io::Cursor::new(input)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn exact_brep_graph_commands_require_matching_schema() {
        let backend = ExactBackend::new();
        assert_eq!(
            handle_request(&backend, "CAPS EXACT_BREP_GRAPH_V9").as_deref(),
            Some("CAPS EXACT_BREP_GRAPH_V9")
        );
        assert_eq!(
            handle_request(&backend, "CAPS EXACT_BREP_GRAPH_V10").as_deref(),
            Some("CAPS EXACT_BREP_GRAPH_V10")
        );
        assert_eq!(
            handle_request(&backend, "CAPS EXACT_BREP_GRAPH_V11").as_deref(),
            Some("CAPS EXACT_BREP_GRAPH_V11")
        );
        assert_eq!(
            handle_request(&backend, "CAPS EXACT_BREP_GRAPH_V12").as_deref(),
            Some("CAPS EXACT_BREP_GRAPH_V12")
        );
        assert_eq!(
            handle_request(&backend, "CAPS EXACT_BREP_GRAPH_V13").as_deref(),
            Some("CAPS EXACT_BREP_GRAPH_V13")
        );
        for (operation, schema) in [
            ("EVAL_BREP_GRAPH_V6", EXACT_BREP_GRAPH_SCHEMA_V6),
            ("TESSELLATE_BREP_GRAPH_V6", EXACT_BREP_GRAPH_SCHEMA_V6),
            ("EVAL_BREP_GRAPH_V7", EXACT_BREP_GRAPH_SCHEMA_V7),
            ("TESSELLATE_BREP_GRAPH_V7", EXACT_BREP_GRAPH_SCHEMA_V7),
            ("EVAL_BREP_GRAPH_V8", EXACT_BREP_GRAPH_SCHEMA_V8),
            ("TESSELLATE_BREP_GRAPH_V8", EXACT_BREP_GRAPH_SCHEMA_V8),
            ("EVAL_BREP_GRAPH_V9", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("TESSELLATE_BREP_GRAPH_V9", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("EVAL_BREP_GRAPH_V10", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("TESSELLATE_BREP_GRAPH_V10", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("EVAL_BREP_GRAPH_V11", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("TESSELLATE_BREP_GRAPH_V11", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("EVAL_BREP_GRAPH_V12", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("TESSELLATE_BREP_GRAPH_V12", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("EVAL_BREP_GRAPH_V13", EXACT_BREP_GRAPH_SCHEMA_V13),
            ("TESSELLATE_BREP_GRAPH_V13", EXACT_BREP_GRAPH_SCHEMA_V13),
            ("EXPORT_BREP_GRAPH_STEP_V3", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("EXPORT_BREP_GRAPH_STEP_V4", EXACT_BREP_GRAPH_SCHEMA_V13),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V6),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V7),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V8),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V11),
        ] {
            assert!(exact_brep_graph_schema_matches_operation(operation, schema));
        }
        for (operation, schema) in [
            ("EVAL_BREP_GRAPH_V6", EXACT_BREP_GRAPH_SCHEMA_V7),
            ("TESSELLATE_BREP_GRAPH_V6", EXACT_BREP_GRAPH_SCHEMA_V7),
            ("EVAL_BREP_GRAPH_V7", EXACT_BREP_GRAPH_SCHEMA_V6),
            ("TESSELLATE_BREP_GRAPH_V7", EXACT_BREP_GRAPH_SCHEMA_V6),
            ("EVAL_BREP_GRAPH_V8", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("TESSELLATE_BREP_GRAPH_V8", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("EVAL_BREP_GRAPH_V9", EXACT_BREP_GRAPH_SCHEMA_V8),
            ("TESSELLATE_BREP_GRAPH_V9", EXACT_BREP_GRAPH_SCHEMA_V8),
            ("EVAL_BREP_GRAPH_V9", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("TESSELLATE_BREP_GRAPH_V9", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("EVAL_BREP_GRAPH_V10", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("TESSELLATE_BREP_GRAPH_V10", EXACT_BREP_GRAPH_SCHEMA_V9),
            ("EVAL_BREP_GRAPH_V10", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("TESSELLATE_BREP_GRAPH_V10", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("EVAL_BREP_GRAPH_V11", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("TESSELLATE_BREP_GRAPH_V11", EXACT_BREP_GRAPH_SCHEMA_V10),
            ("EVAL_BREP_GRAPH_V11", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("TESSELLATE_BREP_GRAPH_V11", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("EVAL_BREP_GRAPH_V12", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("TESSELLATE_BREP_GRAPH_V12", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("EVAL_BREP_GRAPH_V12", EXACT_BREP_GRAPH_SCHEMA_V13),
            ("TESSELLATE_BREP_GRAPH_V12", EXACT_BREP_GRAPH_SCHEMA_V13),
            ("EVAL_BREP_GRAPH_V13", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("TESSELLATE_BREP_GRAPH_V13", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("EXPORT_BREP_GRAPH_STEP_V2", EXACT_BREP_GRAPH_SCHEMA_V12),
            ("EXPORT_BREP_GRAPH_STEP_V3", EXACT_BREP_GRAPH_SCHEMA_V11),
            ("EXPORT_BREP_GRAPH_STEP_V3", EXACT_BREP_GRAPH_SCHEMA_V13),
            ("EXPORT_BREP_GRAPH_STEP_V4", EXACT_BREP_GRAPH_SCHEMA_V12),
        ] {
            assert!(!exact_brep_graph_schema_matches_operation(
                operation, schema
            ));
        }
    }

    #[test]
    fn legacy_sweep_does_not_infer_sketch_semantics_from_region_metadata() {
        let line = |start: [f64; 2], end: [f64; 2]| ExactBRepPlanarSegment::Line {
            start_bits: start.map(f64::to_bits),
            end_bits: end.map(f64::to_bits),
        };
        let profile = ExactBRepProfile {
            id: ketchup_core::exact_brep_graph::ExactBRepProfileId(0),
            source_feature_id: 1,
            region_id: Some(1),
            frame_bits: [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
                .map(f64::to_bits),
            geometry: ExactBRepPlanarGeometry::Boundary {
                closed: true,
                segments: vec![
                    line([-2.0, -1.0], [2.0, -1.0]),
                    line([2.0, -1.0], [2.0, 1.0]),
                    line([2.0, 1.0], [-2.0, 1.0]),
                    line([-2.0, 1.0], [-2.0, -1.0]),
                ],
            },
        };
        let path = ExactBRepProfile {
            id: ketchup_core::exact_brep_graph::ExactBRepProfileId(1),
            source_feature_id: 2,
            region_id: None,
            frame_bits: profile.frame_bits,
            geometry: ExactBRepPlanarGeometry::Boundary {
                closed: false,
                segments: vec![line([0.0, 0.0], [100.0, 0.0])],
            },
        };
        let backend = ExactBackend::new();
        assert!(exact_brep_sweep(&backend, &profile, &path, false).is_ok());
        assert!(exact_brep_sweep(&backend, &profile, &path, true).is_err());
    }

    #[test]
    fn spatial_path_conversion_preserves_all_coordinate_bits_handles_and_orientation() {
        let bits = |values: [f64; 3]| values.map(f64::to_bits);

        let line = ExactBRepSpatialPathSegment::Line {
            start_bits: bits([-0.0, 2.5, -7.25]),
            end_bits: bits([11.0, -13.5, 17.75]),
        };
        let SpatialProfileSegment::Line { start_mm, end_mm } =
            exact_brep_spatial_path_segment(&line)
        else {
            panic!("line conversion changed the segment kind");
        };
        assert_eq!(start_mm.map(f64::to_bits), bits([-0.0, 2.5, -7.25]));
        assert_eq!(end_mm.map(f64::to_bits), bits([11.0, -13.5, 17.75]));

        let arc = ExactBRepSpatialPathSegment::CircularArc {
            start_bits: bits([-19.25, 23.5, -29.75]),
            end_bits: bits([31.0, -37.5, 41.75]),
            center_bits: bits([43.25, -47.5, 53.75]),
            normal_bits: bits([-0.0, -1.0, 0.0]),
            clockwise: true,
        };
        let SpatialProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            normal,
            clockwise,
        } = exact_brep_spatial_path_segment(&arc)
        else {
            panic!("circular arc conversion changed the segment kind");
        };
        assert_eq!(start_mm.map(f64::to_bits), bits([-19.25, 23.5, -29.75]));
        assert_eq!(end_mm.map(f64::to_bits), bits([31.0, -37.5, 41.75]));
        assert_eq!(center_mm.map(f64::to_bits), bits([43.25, -47.5, 53.75]));
        assert_eq!(normal.map(f64::to_bits), bits([-0.0, -1.0, 0.0]));
        assert!(clockwise);

        let cubic = ExactBRepSpatialPathSegment::CubicBezier {
            start_bits: bits([-59.25, 61.5, -67.75]),
            control_1_bits: bits([71.25, -73.5, 79.75]),
            control_2_bits: bits([-83.25, 89.5, -97.75]),
            end_bits: bits([101.25, -103.5, 107.75]),
        };
        let SpatialProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } = exact_brep_spatial_path_segment(&cubic)
        else {
            panic!("cubic Bezier conversion changed the segment kind");
        };
        assert_eq!(start_mm.map(f64::to_bits), bits([-59.25, 61.5, -67.75]));
        assert_eq!(control_1_mm.map(f64::to_bits), bits([71.25, -73.5, 79.75]));
        assert_eq!(control_2_mm.map(f64::to_bits), bits([-83.25, 89.5, -97.75]));
        assert_eq!(end_mm.map(f64::to_bits), bits([101.25, -103.5, 107.75]));
    }

    #[test]
    fn planar_offset_v2_rejects_malformed_fixed_width_records() {
        let backend = ExactBackend::new();
        assert_eq!(
            handle_request(&backend, "CAPS P6_OFFSET_V2").as_deref(),
            Some("CAPS P6_OFFSET_V2")
        );
        let digest = "0".repeat(64);
        let zero = "0000000000000000";
        let one = "3ff0000000000000";
        let line = format!("L {zero} {zero} {one} {zero} {zero} {zero} {zero} {zero} 0");
        let header = format!("OFFSET_PROFILE_P6_V2 1 2 {digest} {one} 2");

        for request in [
            format!("{header} {line}"),
            format!("{header} {line} {line} extra"),
            format!("{header} L {zero} {zero} {one} {zero} {one} {zero} {zero} {zero} 0 {line}"),
            format!("{header} C {zero} {zero} {one} {zero} {zero} {one} {one} {one} 1 {line}"),
        ] {
            assert_eq!(
                handle_request(&backend, &request).as_deref(),
                Some("ERR invalid_request")
            );
        }

        let malformed =
            format!("{header} C {zero} {zero} {one} {zero} not_hex {one} {one} {one} 0 {line}");
        assert_eq!(
            handle_request(&backend, &malformed).as_deref(),
            Some("ERR invalid_parameter")
        );
    }

    #[test]
    fn planar_offset_v3_is_bounded_and_rejects_malformed_loops() {
        let backend = ExactBackend::new();
        assert_eq!(
            handle_request(&backend, "CAPS P6_OFFSET_V3").as_deref(),
            Some("CAPS P6_OFFSET_V3")
        );
        let digest = "0".repeat(64);
        let bits = |value: f64| format!("{:016x}", value.to_bits());
        let zero = bits(0.0);
        let line = |start: [f64; 2], end: [f64; 2]| {
            format!(
                "L {} {} {} {} {zero} {zero} {zero} {zero} 0",
                bits(start[0]),
                bits(start[1]),
                bits(end[0]),
                bits(end[1]),
            )
        };
        let outer = [
            line([-10.0, -10.0], [10.0, -10.0]),
            line([10.0, -10.0], [10.0, 10.0]),
            line([10.0, 10.0], [-10.0, 10.0]),
            line([-10.0, 10.0], [-10.0, -10.0]),
        ]
        .join(" ");
        let circle = format!(
            "R {zero} {zero} {} {zero} {zero} {zero} {zero} {zero} 0",
            bits(2.0)
        );
        let header = format!("OFFSET_REGION_P6_V3 1 2 {digest} {} 2", bits(1.0));
        let valid = format!("{header} 4 {outer} 1 {circle}");
        assert!(
            handle_request(&backend, &valid)
                .is_some_and(|response| response.starts_with("OK_P6_OFFSET_V3 "))
        );

        for request in [
            format!("{header} 4 {outer}"),
            format!("{valid} extra"),
            format!(
                "{header} 4 {outer} 1 L {zero} {zero} {zero} {zero} {zero} {zero} {zero} {zero} 0"
            ),
            format!("OFFSET_REGION_P6_V3 1 2 {digest} {} 66", bits(1.0)),
        ] {
            assert_eq!(
                handle_request(&backend, &request).as_deref(),
                Some("ERR invalid_request")
            );
        }

        let repeated = format!(" 64 {}", vec![line([0.0, 0.0], [1.0, 0.0]); 64].join(" "));
        let oversized = format!(
            "OFFSET_REGION_P6_V3 1 2 {digest} {} 65{}",
            bits(1.0),
            repeated.repeat(65)
        );
        assert_eq!(
            handle_request(&backend, &oversized).as_deref(),
            Some("ERR invalid_request")
        );
    }
}
