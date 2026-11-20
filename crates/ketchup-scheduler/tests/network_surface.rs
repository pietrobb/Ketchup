//! A surface through a network of curves, end to end: the document keeps the
//! inner curves of a boundary surface, saves it as format 117, the graph
//! carries them and the real worker fits the surface through every one. More
//! inner curves than a network takes are refused before anything changes, and
//! a network no surface can pass through is refused by the worker.

use ketchup_model::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, FeatureId,
    FeatureKind, SpatialPathSegment, SurfaceBodySpec,
};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_model::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;

const DEFINITION: DefinitionId = DefinitionId(51);
const SURFACE: FeatureId = FeatureId(510);
const SIDE: f64 = 100.0;
const PEAK: f64 = 20.0;

fn line(start_mm: [f64; 3], end_mm: [f64; 3]) -> SpatialPathSegment {
    SpatialPathSegment::Line { start_mm, end_mm }
}

fn square() -> Vec<SpatialPathSegment> {
    let corners = [
        [0.0, 0.0, 0.0],
        [SIDE, 0.0, 0.0],
        [SIDE, SIDE, 0.0],
        [0.0, SIDE, 0.0],
    ];
    (0..4)
        .map(|index| line(corners[index], corners[(index + 1) % 4]))
        .collect()
}

/// A cubic Bezier across the square that rises to `peak` at its middle.
fn arch(start_mm: [f64; 3], end_mm: [f64; 3], peak: f64) -> SpatialPathSegment {
    let lift = peak * 4.0 / 3.0;
    let at = |t: f64| {
        [
            start_mm[0] + (end_mm[0] - start_mm[0]) * t,
            start_mm[1] + (end_mm[1] - start_mm[1]) * t,
            lift,
        ]
    };
    SpatialPathSegment::CubicBezier {
        start_mm,
        control_1_mm: at(0.25),
        control_2_mm: at(0.75),
        end_mm,
    }
}

fn arches(second_peak: f64) -> Vec<SpatialPathSegment> {
    vec![
        arch([0.0, 50.0, 0.0], [SIDE, 50.0, 0.0], PEAK),
        arch([50.0, 0.0, 0.0], [50.0, SIDE, 0.0], second_peak),
    ]
}

fn create_surface(id: FeatureId, inner: Vec<SpatialPathSegment>) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id,
        definition_id: DEFINITION,
        name: "Dome".into(),
        kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Boundary {
            segments: square(),
            inner,
        }),
    }
}

fn network_document(inner: Vec<SpatialPathSegment>) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Canopy".into(),
            },
            create_surface(SURFACE, inner),
        ]))
        .unwrap();
    document
}

#[test]
fn a_network_surface_is_saved_compiled_and_fitted_by_the_worker() {
    let _turn = crate::integration_support::file_turn();
    let document = network_document(arches(PEAK));
    let snapshot = document.current();

    let bytes = persistence::save(&snapshot);
    assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 117);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    // The inner curves count: the bare boundary is another surface.
    assert_ne!(
        network_document(Vec::new()).current().canonical_digest(),
        snapshot.canonical_digest()
    );
    let bare = persistence::save(&network_document(Vec::new()).current());
    assert_eq!(u16::from_le_bytes([bare[10], bare[11]]), 115);

    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, SURFACE).unwrap();
    assert!(matches!(
        &graph.nodes[0].operation,
        ExactBRepOperation::BoundarySurface { inner: Some(inner), .. }
            if inner.segments.len() == 2 && inner.source_feature_id == SURFACE.0
    ));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 0, "a surface, not a solid");
    assert!(
        (package.bounds_mm[1][2] - PEAK).abs() < 0.05,
        "the surface rises to where the arches cross: {:?}",
        package.bounds_mm
    );
}

#[test]
fn more_inner_curves_than_a_network_takes_are_refused_before_anything_changes() {
    let mut document = network_document(arches(PEAK));
    let before = document.current().canonical_digest();
    let many = (0..65)
        .map(|index| {
            let x = 1.0 + f64::from(index) * 1.5;
            line([x, 0.0, 0.0], [x, SIDE, 0.0])
        })
        .collect();
    let degenerate = vec![line([10.0, 10.0, 0.0], [10.0, 10.0, 0.0])];
    for inner in [many, degenerate] {
        let refused = document.apply_batch(&CommandBatch::new(vec![create_surface(
            FeatureId(511),
            inner,
        )]));
        assert!(
            matches!(
                refused,
                Err(CanonicalError::InvalidFeatureInput(
                    ketchup_model::document::FeatureInputError {
                        field: ketchup_model::document::FeatureInputField::Inner,
                        ..
                    }
                ))
            ),
            "{:?}",
            refused.as_ref().err()
        );
        assert_eq!(document.current().canonical_digest(), before);
    }
}

#[test]
fn the_worker_refuses_a_network_no_surface_passes_through() {
    let _turn = crate::integration_support::file_turn();
    let document = network_document(arches(-PEAK));
    let graph = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, SURFACE).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    assert!(supervisor.evaluate_exact_brep_graph(&graph).is_err());
}
