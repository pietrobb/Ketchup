#![forbid(unsafe_code)]

use ketchup_exact::{
    BottleEdgeFinish, BoxSpec, CutMode, ExactBackend, HalfLapFaceRole, HalfLapNotchSpec,
    HalfLapParticipant, Point3, RectangleExtrudeSpec, ReferenceResolution, Size3, StabilityClass,
    capture_bounded_through_cut_references, capture_guaranteed_references,
    capture_half_lap_notch_references, capture_revolve_references, capture_shell_references,
    resolve_subshape_reference,
};
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
        (Some("CAPS"), Some("M5_NOTCH_V1"), None) => Some("CAPS M5_NOTCH_V1".to_owned()),
        (Some("CAPS"), Some("M6_REVOLVE_V1"), None) => Some("CAPS M6_REVOLVE_V1".to_owned()),
        (Some("CAPS"), Some("M6_SHELL_V1"), None) => Some("CAPS M6_SHELL_V1".to_owned()),
        (Some("CAPS"), Some("M6_FINISH_V1"), None) => Some("CAPS M6_FINISH_V1".to_owned()),
        (Some("CAPS"), Some("M14_STEP_V1"), None) => Some("CAPS M14_STEP_V1".to_owned()),
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

fn decode_hex_utf8(value: &str) -> Option<String> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes)
        .ok()
        .filter(|path| !path.is_empty() && !path.contains('\r') && !path.contains('\n'))
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
