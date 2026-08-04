#![forbid(unsafe_code)]

use ketchup_exact::{ExactBackend, RectangleExtrudeSpec};
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
        (Some("EXTRUDE"), Some(height_bits), None) => {
            let Ok(height_bits) = u64::from_str_radix(height_bits, 16) else {
                return Some("ERR invalid_parameter".to_owned());
            };
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
                    Some(format!(
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
                    ))
                }
                Err(error) => Some(format!("ERR {}", error.code.as_str())),
            }
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
