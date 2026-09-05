#[path = "../../ketchup-core/tests/support/generic_sketch_pocket.rs"]
mod fixture;
use fixture::*;
use ketchup_core::document::{CanonicalCommand, CommandBatch};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{ExactProductError, ExactResultRegistry};
use ketchup_core::sketch::PrincipalPlane;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::sync::Arc;

fn assert_bounds(actual: [[f64; 3]; 2], expected: [[f64; 3]; 2]) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        assert!(
            (actual - expected).abs() <= 2.0e-7,
            "{actual} != {expected}"
        );
    }
}

#[test]
fn worker_cuts_real_generic_sketch_through_hole_in_principal_and_offset_frames() {
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for (plane, offset, bounds) in [
        (
            PrincipalPlane::Xy,
            0.0,
            [[0.0, 0.0, 0.0], [400.0, 200.0, 20.0]],
        ),
        (
            PrincipalPlane::Xy,
            35.0,
            [[0.0, 0.0, 35.0], [400.0, 200.0, 55.0]],
        ),
        (
            PrincipalPlane::Yz,
            35.0,
            [[35.0, 0.0, 0.0], [55.0, 400.0, 200.0]],
        ),
        (
            PrincipalPlane::Xz,
            35.0,
            [[0.0, -55.0, 0.0], [400.0, -35.0, 200.0]],
        ),
    ] {
        let mut document = document(plane, offset);
        let snapshot = document.current();
        let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, POCKET).unwrap();
        let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
        assert!((package.volume_mm3 - 1_568_000.0).abs() < 1.0e-6);
        assert_bounds(package.bounds_mm, bounds);
        assert_eq!(package.topology_counts[4], 1);
        assert_eq!(package.topology_counts[2], 10); // Six outer faces and four through-hole walls, no floor.
        assert!(!package.triangles.is_empty());
        ExactResultRegistry::accept(&snapshot, [Arc::new(package.clone().into())]).unwrap();

        // Move the live sketch so only half its area intersects the pad. The changed
        // volume proves recompute uses the edited dependency, not a static baked profile.
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::TranslateProfile {
                    id: CUT_SKETCH,
                    delta_mm: [-60.0, 0.0],
                },
            ]))
            .unwrap();
        let after = document.current();
        assert!(matches!(
            ExactResultRegistry::accept(&after, [Arc::new(package.into())]),
            Err(ExactProductError::StaleResult)
        ));
        let updated_graph = ExactBRepGraph::from_snapshot(&after, DEFINITION, POCKET).unwrap();
        assert_ne!(graph.graph_digest, updated_graph.graph_digest);
        let updated = worker.evaluate_exact_brep_graph(&updated_graph).unwrap();
        assert!((updated.volume_mm3 - 1_584_000.0).abs() < 1.0e-6);
        assert_bounds(updated.bounds_mm, bounds);
        assert_eq!(updated.topology_counts[4], 1);
        ExactResultRegistry::accept(&after, [Arc::new(updated.into())]).unwrap();
    }
}
