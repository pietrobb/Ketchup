//! A blend between two surfaces, end to end: the document keeps which two
//! surfaces a blend joins and how it meets each, saves it as format 118, the
//! graph carries both surfaces and the real worker bridges the gap between
//! their facing edges. A blend of a surface with itself or with a solid is
//! refused before anything changes.

use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind,
    LoftContinuity, SpatialPathSegment, SurfaceBodySpec,
};
use ketchup_model::exact_brep_graph::{
    ExactBRepGraph, ExactBRepLoftContinuity, ExactBRepOperation,
};
use ketchup_model::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;

const DEFINITION: DefinitionId = DefinitionId(61);
const CURVED: FeatureId = FeatureId(610);
const FLAT: FeatureId = FeatureId(611);
const BLEND: FeatureId = FeatureId(612);
const RADIUS: f64 = 50.0;
const WIDTH: f64 = 50.0;

fn line(start_mm: [f64; 3], end_mm: [f64; 3]) -> SpatialPathSegment {
    SpatialPathSegment::Line { start_mm, end_mm }
}

fn arc_end() -> [f64; 3] {
    let angle = -30.0_f64.to_radians();
    [RADIUS * angle.cos(), 0.0, RADIUS + RADIUS * angle.sin()]
}

fn arc(y: f64, reversed: bool) -> SpatialPathSegment {
    let [x, _, z] = arc_end();
    let (start_mm, end_mm) = if reversed {
        ([x, y, z], [0.0, y, 0.0])
    } else {
        ([0.0, y, 0.0], [x, y, z])
    };
    SpatialPathSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm: [0.0, y, RADIUS],
        normal: [0.0, 1.0, 0.0],
        clockwise: !reversed,
    }
}

/// A patch curved with radius 50 whose free straight edge at x = 43.3 faces
/// a flat plate at z = 60 from x = 80 to 130.
fn surfaces() -> Vec<CanonicalCommand> {
    let [x, _, z] = arc_end();
    let corners = [
        [80.0, 0.0, 60.0],
        [130.0, 0.0, 60.0],
        [130.0, WIDTH, 60.0],
        [80.0, WIDTH, 60.0],
    ];
    vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Duct".into(),
        },
        CanonicalCommand::CreateFeature {
            id: CURVED,
            definition_id: DEFINITION,
            name: "Curved patch".into(),
            kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Boundary {
                segments: vec![
                    arc(0.0, false),
                    line([x, 0.0, z], [x, WIDTH, z]),
                    arc(WIDTH, true),
                    line([0.0, WIDTH, 0.0], [0.0, 0.0, 0.0]),
                ],
                inner: vec![arc(WIDTH / 2.0, false)],
            }),
        },
        CanonicalCommand::CreateFeature {
            id: FLAT,
            definition_id: DEFINITION,
            name: "Flat plate".into(),
            kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Boundary {
                segments: (0..4)
                    .map(|index| line(corners[index], corners[(index + 1) % 4]))
                    .collect(),
                inner: Vec::new(),
            }),
        },
    ]
}

fn blend(id: FeatureId, first: FeatureId, second: FeatureId) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id,
        definition_id: DEFINITION,
        name: "Blend".into(),
        kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Blend {
            first,
            first_continuity: LoftContinuity::Curvature,
            second,
            second_continuity: LoftContinuity::Tangent,
        }),
    }
}

fn blended_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = surfaces();
    commands.push(blend(BLEND, CURVED, FLAT));
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn a_blend_is_saved_compiled_and_bridged_by_the_worker() {
    let _turn = crate::integration_support::file_turn();
    let document = blended_document();
    let snapshot = document.current();

    let bytes = persistence::save(&snapshot);
    assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 118);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());

    // The continuity counts: the same surfaces met only in position differ.
    let mut position = DocumentStore::new();
    let mut commands = surfaces();
    commands.push(CanonicalCommand::CreateFeature {
        id: BLEND,
        definition_id: DEFINITION,
        name: "Blend".into(),
        kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Blend {
            first: CURVED,
            first_continuity: LoftContinuity::Position,
            second: FLAT,
            second_continuity: LoftContinuity::Tangent,
        }),
    });
    position.apply_batch(&CommandBatch::new(commands)).unwrap();
    assert_ne!(
        position.current().canonical_digest(),
        snapshot.canonical_digest()
    );

    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, BLEND).unwrap();
    let last = graph.nodes.last().unwrap();
    assert!(matches!(
        &last.operation,
        ExactBRepOperation::SurfaceBlend {
            first_continuity: ExactBRepLoftContinuity::Curvature,
            second_continuity: ExactBRepLoftContinuity::Tangent,
            ..
        }
    ));
    assert_eq!(graph.nodes.len(), 3, "both surfaces and the blend");

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 0, "a surface, not a solid");
    let [min, max] = package.bounds_mm;
    assert!(
        min[0] > 43.0 && max[0] < 80.0 + 1.0e-3,
        "the blend spans only the gap: {:?}",
        package.bounds_mm
    );
    assert!((max[2] - 60.0).abs() < 1.0e-3, "{:?}", package.bounds_mm);
}

#[test]
fn a_blend_of_a_surface_with_itself_or_a_missing_surface_is_refused() {
    let mut document = blended_document();
    let before = document.current().canonical_digest();
    for (first, second) in [(CURVED, CURVED), (CURVED, FeatureId(999))] {
        let refused = document.apply_batch(&CommandBatch::new(vec![blend(
            FeatureId(613),
            first,
            second,
        )]));
        assert!(refused.is_err(), "{first:?} {second:?}");
        assert_eq!(document.current().canonical_digest(), before);
    }
}

#[test]
fn a_blend_of_a_solid_is_refused() {
    let mut document = blended_document();
    let solid = FeatureId(614);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(613),
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: DEFINITION,
                name: "Block".into(),
                kind: FeatureKind::full_revolve(FeatureId(613)),
            },
        ]))
        .unwrap();
    let before = document.current().canonical_digest();
    let refused =
        document.apply_batch(&CommandBatch::new(vec![blend(FeatureId(615), solid, FLAT)]));
    assert!(refused.is_err());
    assert_eq!(document.current().canonical_digest(), before);
}
