use ketchup_exact::ExactBackend;

fn main() {
    let path = std::env::args().nth(1).expect("STEP path");
    let backend = ExactBackend::new();
    for ordinal in 0.. {
        let Ok(output) = backend.import_step_solid(&path, ordinal) else {
            break;
        };
        println!(
            "{ordinal}: volume={:.6} bounds={:?} topology=({}, {}, {}, {}, {}) fingerprint={}",
            output.body.topology.volume_mm3,
            output.body.topology.bounds_mm,
            output.body.topology.vertex_count,
            output.body.topology.edge_count,
            output.body.topology.face_count,
            output.body.topology.shell_count,
            output.body.topology.solid_count,
            output.body.result_fingerprint,
        );
    }
}
