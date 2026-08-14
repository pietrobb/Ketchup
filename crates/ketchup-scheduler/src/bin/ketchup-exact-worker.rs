#![forbid(unsafe_code)]

use ketchup_core::graph::sha256_hex;
use ketchup_exact::{
    BottleEdgeFinish, BoxSpec, CircleExtrudeSpec, CutMode, CylinderToolSpec, ExactBackend,
    HalfLapFaceRole, HalfLapNotchSpec, HalfLapParticipant, PlanarProfileSegment, Point3,
    RectangleExtrudeSpec, RectangleOffsetSpec, RectangleSweepSpec, ReferenceResolution, Size3,
    SplineLoftSection, SplineLoftSpec, StabilityClass, capture_bounded_pocket_references,
    capture_bounded_through_cut_references, capture_box_shell_references,
    capture_circle_extrusion_references, capture_circular_through_cut_references,
    capture_general_revolve_references, capture_guaranteed_references,
    capture_half_lap_notch_references, capture_mixed_profile_extrusion_references,
    capture_planar_offset_reference, capture_rectangular_intersection_references,
    capture_rectangular_split_references, capture_rectangular_sweep_references,
    capture_rectangular_union_references, capture_revolve_references, capture_shell_references,
    capture_spline_loft_references, resolve_subshape_reference,
};
use ketchup_scheduler::{
    StepAssemblyManifest, StepFeatureExportSpec, StepProfileSegment, StepRevolveExportSpec,
};
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

fn main() {
    let backend = ExactBackend::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let response = handle_request(&backend, &line);
        if let Some(response) = response
            && writeln!(stdout, "{response}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
}

fn handle_request(backend: &ExactBackend, request: &str) -> Option<String> {
    let mut fields = request.split_whitespace();
    match (fields.next(), fields.next(), fields.next()) {
        (Some("PING"), None, None) => Some("PONG".to_owned()),
        (Some("CAPS"), Some("M3_V1"), None) => Some("CAPS M3_V1".to_owned()),
        (Some("CAPS"), Some("M3_CUT_V1"), None) => Some("CAPS M3_CUT_V1".to_owned()),
        (Some("CAPS"), Some("M3_POCKET_V1"), None) => Some("CAPS M3_POCKET_V1".to_owned()),
        (Some("CAPS"), Some("M3_UNION_V1"), None) => Some("CAPS M3_UNION_V1".to_owned()),
        (Some("CAPS"), Some("P6_INTERSECT_V1"), None) => Some("CAPS P6_INTERSECT_V1".to_owned()),
        (Some("CAPS"), Some("P6_SPLIT_V1"), None) => Some("CAPS P6_SPLIT_V1".to_owned()),
        (Some("CAPS"), Some("P6_OFFSET_V1"), None) => Some("CAPS P6_OFFSET_V1".to_owned()),
        (Some("CAPS"), Some("P6_SWEEP_V1"), None) => Some("CAPS P6_SWEEP_V1".to_owned()),
        (Some("CAPS"), Some("P6_LOFT_V1"), None) => Some("CAPS P6_LOFT_V1".to_owned()),
        (Some("CAPS"), Some("P3_CIRCLE_V1"), None) => Some("CAPS P3_CIRCLE_V1".to_owned()),
        (Some("CAPS"), Some("P3_ARC_V1"), None) => Some("CAPS P3_ARC_V1".to_owned()),
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
        (Some("EXTRUDE_CIRCLE_P3_V1"), Some(center_x_bits), Some(center_y_bits)) => {
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
                bits,
                document_id,
                producer_feature_id,
                request_digest,
            ))
        }
        (Some("EXTRUDE_CIRCULAR_CUT_P3_V1"), Some(width_bits), Some(depth_bits)) => {
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
            Some(p3_circular_cut_response(
                backend,
                bits,
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
            for (role, source, expected_type) in [
                ("extrusion.top", "profile.face", "planar_face"),
                ("extrusion.bottom", "profile.face", "planar_face"),
                (
                    "extrusion.side(profile_edge=arc.0)",
                    "profile.edge.arc.0",
                    "face",
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

fn p3_circular_cut_response(
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
    let result = backend.extrude_rectangle(base).and_then(|base_output| {
        backend.cut_cylinder(
            &base_output.body,
            CylinderToolSpec {
                center_mm: [f64::from_bits(bits[3]), f64::from_bits(bits[4])],
                origin_z_mm: -1.0,
                radius_mm: f64::from_bits(bits[5]),
                height_mm: base.height_mm + 2.0,
            },
            CutMode::ThroughAll,
        )
    });
    let elapsed = started.elapsed().as_nanos();
    match result {
        Ok(mut output) => {
            let references = match capture_circular_through_cut_references(
                &mut output,
                document_id,
                producer_feature_id,
                base,
            ) {
                Ok(references) => references,
                Err(error) => return format!("ERR {}", error.code.as_str()),
            };
            let mut evidence = Vec::new();
            for role in [
                "extrusion.top",
                "extrusion.bottom",
                "extrusion.side(profile_edge=east)",
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
            backend.finish_shell_box(spec, thickness, BottleEdgeFinish::Fillet, amount)
        }
        (Some("chamfer"), Some(amount)) => {
            backend.finish_shell_box(spec, thickness, BottleEdgeFinish::Chamfer, amount)
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
    let elapsed = started.elapsed().as_nanos();
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
        Some("fillet") => Some(BottleEdgeFinish::Fillet),
        Some("chamfer") => Some(BottleEdgeFinish::Chamfer),
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
                    BottleEdgeFinish::Fillet
                } else {
                    BottleEdgeFinish::Chamfer
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
                    BottleEdgeFinish::Fillet
                } else if kind == "chamfer" {
                    BottleEdgeFinish::Chamfer
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
                return backend.cut_cylinder(
                    &base_output.body,
                    CylinderToolSpec {
                        center_mm: [
                            f64::from_bits(circle.center_x_bits),
                            f64::from_bits(circle.center_y_bits),
                        ],
                        origin_z_mm: -1.0,
                        radius_mm: f64::from_bits(circle.radius_bits),
                        height_mm: base.height_mm + 2.0,
                    },
                    CutMode::ThroughAll,
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
    let source_bytes = match std::fs::read(&source_path) {
        Ok(bytes) => bytes,
        Err(error) => return transport_error_response("read_step_part", &error.to_string()),
    };
    if sha256_hex(&source_bytes) != source_sha256 {
        return transport_error_response(
            "inspect_step_part",
            "STEP part bytes changed before inspection",
        );
    }
    match backend.import_step(&source_path) {
        Ok(output) => format!(
            "OK_M21_STEP_PART_V1 {source_sha256} {}",
            output.body.result_fingerprint
        ),
        Err(error) => geometry_error_response(&error),
    }
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
        let source_bytes = match std::fs::read(&source_path) {
            Ok(bytes) => bytes,
            Err(error) => return transport_error_response("read_step_part", &error.to_string()),
        };
        if sha256_hex(&source_bytes) != manifest_part.source_sha256 {
            return transport_error_response(
                "verify_step_part",
                "STEP part bytes changed after manifest construction",
            );
        }
        let imported = match backend.import_step(&source_path) {
            Ok(output) => output,
            Err(error) => return geometry_error_response(&error),
        };
        if imported.body.result_fingerprint != manifest_part.imported_result_fingerprint {
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
