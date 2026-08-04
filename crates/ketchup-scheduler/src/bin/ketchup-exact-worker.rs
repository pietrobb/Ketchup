#![forbid(unsafe_code)]

use ketchup_exact::{
    ExactBackend, RectangleExtrudeSpec, ReferenceResolution, StabilityClass,
    capture_guaranteed_references, resolve_subshape_reference,
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

fn is_canonical_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
