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
use ketchup_scheduler::protocol::SurfaceControlNetWire;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

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
    from_mm: [f64; 3],
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
            from_mm,
            offset_mm,
            weight,
            keep,
        }),
    }
}

/// The blend's control net, read from the worker as a caller reads it before
/// a move (R1120ac-EXA-900; before, only a refused move told where a point lay).
fn blend_net(supervisor: &mut ExactWorkerSupervisor) -> SurfaceControlNetWire {
    let graph = ExactBRepGraph::from_snapshot(&s_blend().current(), DEFINITION, BLEND).unwrap();
    supervisor
        .surface_control_net(&graph, &BTreeMap::new(), &AtomicBool::new(false))
        .unwrap()
}

/// Where control point (`u_index`, 16) of the blend lies.
fn pole(supervisor: &mut ExactWorkerSupervisor, u_index: u32) -> [f64; 3] {
    let net = blend_net(supervisor);
    net.points_mm[usize::try_from(u_index * net.v_count + 16).unwrap()]
}

#[test]
fn the_control_net_is_read_and_a_rounded_from_mm_still_names_its_point() {
    let _turn = crate::integration_support::file_turn();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let net = blend_net(&mut supervisor);
    let points = usize::try_from(net.u_count * net.v_count).unwrap();
    assert!(
        net.u_count > 2 && net.v_count > 16,
        "{} x {}",
        net.u_count,
        net.v_count
    );
    assert_eq!(net.points_mm.len(), points);
    assert_eq!(net.weights.len(), points);
    assert!(
        net.match_tolerance_mm > 1.0e-7 && net.match_tolerance_mm <= 1.0e-3,
        "{}",
        net.match_tolerance_mm
    );
    // Copied with four decimals, as an AI reading the net would write it.
    let exact = net.points_mm[usize::try_from(2 * net.v_count + 16).unwrap()];
    let rounded = exact.map(|value| (value * 1.0e4).round() / 1.0e4);
    let mut moved = |from_mm| {
        let mut document = s_blend();
        document
            .apply_batch(&CommandBatch::new(vec![move_point(
                MOVED,
                2,
                from_mm,
                [0.0, 0.0, -5.0],
                None,
                Vec::new(),
            )]))
            .unwrap();
        let graph = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, MOVED).unwrap();
        supervisor.evaluate_exact_brep_graph(&graph)
    };
    moved(rounded).unwrap();
    let far = [exact[0] + 0.01, exact[1], exact[2]];
    let refused = format!("{:?}", moved(far).unwrap_err());
    assert!(refused.contains("inspect surface_control_net"), "{refused}");
}

#[test]
fn a_moved_control_point_is_saved_compiled_and_moved_by_the_worker() {
    let _turn = crate::integration_support::file_turn();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let from_mm = pole(&mut supervisor, 2);
    let mut document = s_blend();
    document
        .apply_batch(&CommandBatch::new(vec![move_point(
            MOVED,
            2,
            from_mm,
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
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let from_mm = pole(&mut supervisor, 1);
    let mut document = s_blend();
    document
        .apply_batch(&CommandBatch::new(vec![move_point(
            MOVED,
            1,
            from_mm,
            [0.0, 0.0, 5.0],
            None,
            vec![kept(LOW)],
        )]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, MOVED).unwrap();
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
            [60.0, 25.0, 1.0],
            offset_mm,
            weight,
            keep.clone(),
        )]));
        assert!(refused.is_err(), "{offset_mm:?} {weight:?} {keep:?}");
        assert_eq!(document.current().canonical_digest(), before);
    }
}
