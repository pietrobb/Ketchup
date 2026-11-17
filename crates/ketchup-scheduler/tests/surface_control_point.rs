//! Moving a control point of a surface, end to end: the document keeps which
//! surface moves, which control point, by how much, its weight and the seams
//! to keep, saves it as format 119, the graph carries the surface and every
//! kept neighbour, and the real worker moves the point or refuses a move that
//! breaks a kept seam. Invalid requests are refused before anything changes.

use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind,
    LoftContinuity, SpatialPathSegment, SurfaceBodySpec, SurfaceSeam,
};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_model::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;

const DEFINITION: DefinitionId = DefinitionId(62);
const LOW: FeatureId = FeatureId(620);
const HIGH: FeatureId = FeatureId(621);
const BLEND: FeatureId = FeatureId(622);
const MOVED: FeatureId = FeatureId(623);
const WIDTH: f64 = 50.0;

fn plate(id: FeatureId, x: [f64; 2], z: f64) -> CanonicalCommand {
    let corners = [
        [x[0], 0.0, z],
        [x[1], 0.0, z],
        [x[1], WIDTH, z],
        [x[0], WIDTH, z],
    ];
    CanonicalCommand::CreateFeature {
        id,
        definition_id: DEFINITION,
        name: "Plate".into(),
        kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Boundary {
            segments: (0..4)
                .map(|index| SpatialPathSegment::Line {
                    start_mm: corners[index],
                    end_mm: corners[(index + 1) % 4],
                })
                .collect(),
            inner: Vec::new(),
        }),
    }
}

/// A low and a high plate and a tangent S blend between them.
fn s_blend() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Step".into(),
            },
            plate(LOW, [0.0, 50.0], 0.0),
            plate(HIGH, [80.0, 130.0], 20.0),
            CanonicalCommand::CreateFeature {
                id: BLEND,
                definition_id: DEFINITION,
                name: "Blend".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Blend {
                    first: LOW,
                    first_continuity: LoftContinuity::Tangent,
                    second: HIGH,
                    second_continuity: LoftContinuity::Tangent,
                }),
            },
        ]))
        .unwrap();
    document
}

fn kept(surface: FeatureId) -> SurfaceSeam {
    SurfaceSeam {
        surface,
        continuity: LoftContinuity::Tangent,
    }
}

fn move_point(
    id: FeatureId,
    u_index: u32,
    offset_mm: [f64; 3],
    weight: Option<f64>,
    keep: Vec<SurfaceSeam>,
) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id,
        definition_id: DEFINITION,
        name: "Moved blend".into(),
        kind: FeatureKind::SurfaceBody(SurfaceBodySpec::ControlPoint {
            target: BLEND,
            u_index,
            v_index: 16,
            offset_mm,
            weight,
            keep,
        }),
    }
}

#[test]
fn a_moved_control_point_is_saved_compiled_and_moved_by_the_worker() {
    let _turn = crate::integration_support::file_turn();
    let mut document = s_blend();
    document
        .apply_batch(&CommandBatch::new(vec![move_point(
            MOVED,
            2,
            [0.0, 0.0, -5.0],
            None,
            vec![kept(LOW), kept(HIGH)],
        )]))
        .unwrap();
    let snapshot = document.current();

    let bytes = persistence::save(&snapshot);
    assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 119);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());

    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, MOVED).unwrap();
    let last = graph.nodes.last().unwrap();
    let ExactBRepOperation::SurfaceControlPoint { keep, .. } = &last.operation else {
        panic!("{:?}", last.operation);
    };
    assert_eq!(keep.len(), 2);
    assert_eq!(graph.nodes.len(), 4, "both plates, the blend and the move");

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 0, "a surface, not a solid");
    let [min, max] = package.bounds_mm;
    // The unmoved blend never dips below the low plate at z = 0.
    assert!(
        min[2] < -0.01,
        "the lowered point pulls the surface down: {min:?}"
    );
    assert!(
        min[0] > 50.0 - 1.0e-3 && max[0] < 80.0 + 1.0e-3,
        "{:?}",
        package.bounds_mm
    );
}

#[test]
fn a_move_that_breaks_a_kept_seam_is_refused_by_the_worker() {
    let _turn = crate::integration_support::file_turn();
    let mut document = s_blend();
    document
        .apply_batch(&CommandBatch::new(vec![move_point(
            MOVED,
            1,
            [0.0, 0.0, 5.0],
            None,
            vec![kept(LOW)],
        )]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, MOVED).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let error = supervisor.evaluate_exact_brep_graph(&graph).unwrap_err();
    assert!(format!("{error:?}").contains("normals"), "{error:?}");
}

#[test]
fn invalid_control_point_moves_are_refused_before_anything_changes() {
    let mut document = s_blend();
    let before = document.current().canonical_digest();
    for (offset_mm, weight, keep) in [
        ([0.0, 0.0, 0.0], None, Vec::new()),
        ([0.0, 0.0, 1.0], Some(0.0), Vec::new()),
        ([0.0, 0.0, f64::NAN], None, Vec::new()),
        ([0.0, 0.0, 1.0], None, vec![kept(BLEND)]),
        ([0.0, 0.0, 1.0], None, vec![kept(LOW), kept(LOW)]),
        ([0.0, 0.0, 1.0], None, vec![kept(FeatureId(999))]),
    ] {
        let refused = document.apply_batch(&CommandBatch::new(vec![move_point(
            MOVED,
            2,
            offset_mm,
            weight,
            keep.clone(),
        )]));
        assert!(refused.is_err(), "{offset_mm:?} {weight:?} {keep:?}");
        assert_eq!(document.current().canonical_digest(), before);
    }
}
