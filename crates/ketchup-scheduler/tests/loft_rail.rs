//! A Loft with a rail, end to end: the document keeps it, saves it as format
//! 116, the graph carries the rail and the real worker stretches the sections
//! to it. A rail without a guide is refused before anything changes, and a
//! rail that does not start on the first section is refused by the worker.

use ketchup_geometry::sketch::{
    PrincipalPlane, SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
};
use ketchup_model::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, FeatureId,
    FeatureKind, LoftContinuity, LoftSection, SpatialPathSegment, SurfaceBodySpec,
};
use ketchup_model::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V23, ExactBRepGraph, ExactBRepOperation,
};
use ketchup_model::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::f64::consts::PI;

const DEFINITION: DefinitionId = DefinitionId(41);
const PLANE: FeatureId = FeatureId(410);
const LOWER: FeatureId = FeatureId(411);
const UPPER: FeatureId = FeatureId(412);
const AXIS: FeatureId = FeatureId(413);
const RAIL: FeatureId = FeatureId(414);
const LOFT: FeatureId = FeatureId(415);
const HEIGHT: f64 = 20.0;
const END_RADIUS: f64 = 5.0;
const MIDDLE_RADIUS: f64 = 8.0;

fn circle() -> FeatureKind {
    FeatureKind::Sketch(SketchSpec {
        workplane: PLANE,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm: END_RADIUS,
        }],
        constraints: Vec::new(),
    })
}

/// From the first circle to the last in the XZ plane, out to
/// `MIDDLE_RADIUS` half way: r(t) = 5 + 12 t (1 - t), z = 20 t.
fn bulging_rail(start_x: f64) -> FeatureKind {
    let control_x = (8.0 * MIDDLE_RADIUS - 2.0 * END_RADIUS) / 6.0;
    FeatureKind::SpatialPath {
        segments: vec![SpatialPathSegment::CubicBezier {
            start_mm: [start_x, 0.0, 0.0],
            control_1_mm: [control_x, 0.0, HEIGHT / 3.0],
            control_2_mm: [control_x, 0.0, 2.0 * HEIGHT / 3.0],
            end_mm: [END_RADIUS, 0.0, HEIGHT],
        }],
    }
}

fn sections() -> Vec<LoftSection> {
    vec![
        LoftSection {
            profile: LOWER,
            elevation_mm: 0.0,
        },
        LoftSection {
            profile: UPPER,
            elevation_mm: HEIGHT,
        },
    ]
}

fn create(id: FeatureId, name: &str, kind: FeatureKind) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id,
        definition_id: DEFINITION,
        name: name.into(),
        kind,
    }
}

fn railed_document(rail_start_x: f64) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Barrel".into(),
            },
            create(
                PLANE,
                "XY",
                FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            ),
            create(LOWER, "Lower circle", circle()),
            create(UPPER, "Upper circle", circle()),
            create(
                AXIS,
                "Axis",
                FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, HEIGHT],
                    }],
                },
            ),
            create(RAIL, "Rail", bulging_rail(rail_start_x)),
            create(
                LOFT,
                "Barrel",
                FeatureKind::Loft {
                    sections: sections(),
                    guide: Some(AXIS),
                    continuity: LoftContinuity::Position,
                    rail: Some(RAIL),
                },
            ),
        ]))
        .unwrap();
    document
}

#[test]
fn a_loft_with_a_rail_is_saved_compiled_and_stretched_by_the_worker() {
    let _turn = crate::integration_support::file_turn();
    let document = railed_document(END_RADIUS);
    let snapshot = document.current();

    let bytes = persistence::save(&snapshot);
    assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 116);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());

    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, LOFT).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V23);
    assert!(matches!(
        &graph.nodes[0].operation,
        ExactBRepOperation::Loft { rail: Some(rail), .. } if rail.source_feature_id == RAIL.0
    ));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 1);
    // The barrel turned about the axis: V = 20 pi * 49.8 (see the native test).
    let expected = 20.0 * PI * 49.8;
    assert!(
        (package.volume_mm3 - expected).abs() <= 0.02 * expected,
        "barrel volume {} != {expected}",
        package.volume_mm3
    );

    let surface = FeatureId(416);
    let mut with_surface = document;
    with_surface
        .apply_batch(&CommandBatch::new(vec![create(
            surface,
            "Barrel surface",
            FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
                sections: sections(),
                guide: Some(AXIS),
                continuity: LoftContinuity::Position,
                rail: Some(RAIL),
            }),
        )]))
        .unwrap();
    let surface_graph =
        ExactBRepGraph::from_snapshot(&with_surface.current(), DEFINITION, surface).unwrap();
    let surface_package = supervisor
        .evaluate_exact_brep_graph(&surface_graph)
        .unwrap();
    assert_eq!(surface_package.topology_counts[4], 0);
}

#[test]
fn a_rail_without_a_guide_is_refused_before_anything_changes() {
    let mut document = railed_document(END_RADIUS);
    let before = document.current().canonical_digest();
    for (guide, rail) in [(None, Some(RAIL)), (Some(RAIL), Some(RAIL))] {
        let refused = document.apply_batch(&CommandBatch::new(vec![create(
            FeatureId(417),
            "Bad rail",
            FeatureKind::Loft {
                sections: sections(),
                guide,
                continuity: LoftContinuity::Position,
                rail,
            },
        )]));
        assert_eq!(refused.err(), Some(CanonicalError::InvalidLoft));
        assert_eq!(document.current().canonical_digest(), before);
    }
}

#[test]
fn the_worker_refuses_a_rail_off_the_sections() {
    let _turn = crate::integration_support::file_turn();
    let document = railed_document(END_RADIUS + 3.0);
    let graph = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, LOFT).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    assert!(supervisor.evaluate_exact_brep_graph(&graph).is_err());
}
