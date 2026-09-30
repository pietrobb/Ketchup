//! Documents written before extrusions, pockets and through cuts became one `Pad`
//! load as pads that keep their digest, parameter targets and exact geometry.

use ketchup_core::document::{
    DefinitionId, FeatureId, FeatureKind, PersistentDimensionId, PersistentDimensionTarget,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_core::persistence;
use ketchup_geometry::sketch::{CutStart, FeatureExtent, PadOperation, PadProfile, PadSpec};

/// Written by the retired field-by-field codec: a part with a sketch pocket (15),
/// a part with a downward extrusion (31), and a part with an extrusion (41), a
/// through cut (43) and a plain-profile pocket (45); dimensions 1-3 target the
/// old `depth` of 15 and 45 and the old `height` of 31.
const FIXTURE: &[u8] = include_bytes!("fixtures/persistence/legacy/pad-records-schema97.bin");
/// The canonical digest the document had when the retired codec wrote it.
const DIGEST: &str = "04124ea1705657a1";

fn pad(kind: &FeatureKind) -> &PadSpec {
    match kind {
        FeatureKind::Pad(spec) => spec,
        other => panic!("expected a pad, got {other:?}"),
    }
}

fn blind_mm(spec: &PadSpec) -> f64 {
    spec.extent.blind_distance().unwrap().millimetres()
}

#[test]
fn old_extrusion_pocket_and_through_cut_records_load_as_pads() {
    let loaded = persistence::load(FIXTURE).unwrap();
    assert!(loaded.migration_losses().is_empty());
    // The pads hash, under the frozen pre-serde digest, to what the old records did.
    assert_eq!(loaded.audit().source_canonical_digest, DIGEST);
    let snapshot = loaded.snapshot();
    let kind = |id| snapshot.feature(FeatureId(id)).unwrap().kind();

    // An old pocket of a sketch was cut from the sketch plane.
    let sketch_pocket = pad(kind(15));
    assert_eq!(sketch_pocket.profile, PadProfile::Feature(FeatureId(14)));
    assert_eq!(blind_mm(sketch_pocket), 5.0);
    assert_eq!(
        sketch_pocket.operation,
        PadOperation::Cut {
            target: FeatureId(13),
            start: CutStart::ProfilePlane
        }
    );
    // An old pocket of a plain profile was cut down from the target's face.
    let plain_pocket = pad(kind(45));
    assert_eq!(blind_mm(plain_pocket), 4.0);
    assert_eq!(
        plain_pocket.operation,
        PadOperation::Cut {
            target: FeatureId(43),
            start: CutStart::TargetFace
        }
    );
    let through = pad(kind(43));
    assert_eq!(through.extent, FeatureExtent::ThroughAll);
    assert_eq!(through.operation.target(), Some(FeatureId(41)));
    assert_eq!(pad(kind(41)).operation, PadOperation::NewBody);
    // A negative height keeps its sign: the pad sweeps against the normal.
    assert_eq!(blind_mm(pad(kind(31))), -8.0);

    for id in 1..=3 {
        let PersistentDimensionTarget::FeatureParameter(target) = &snapshot
            .persistent_dimension(PersistentDimensionId(id))
            .unwrap()
            .target
        else {
            panic!("dimension {id} lost its feature target");
        };
        assert_eq!(target.path.as_str(), "extent.distance");
    }

    for (definition, feature, expected) in [
        (1, 15, (0.0, 5.0)),
        (3, 43, (-1.0, 21.0)),
        (3, 45, (16.0, 20.0)),
    ] {
        let graph =
            ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(definition), FeatureId(feature))
                .unwrap();
        let ExactBRepOperation::ProfileCut { interval, .. } = graph.nodes.last().unwrap().operation
        else {
            panic!("feature {feature} is not a cut");
        };
        assert_eq!(interval.direction(), [0.0, 0.0, 1.0]);
        assert_eq!(
            (interval.start_mm(), interval.end_mm()),
            expected,
            "{feature}"
        );
    }
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(2), FeatureId(31)).unwrap();
    let ExactBRepOperation::Extrude { interval, .. } = graph.nodes.last().unwrap().operation else {
        panic!("feature 31 is not an extrusion");
    };
    assert_eq!((interval.start_mm(), interval.end_mm()), (-8.0, 0.0));

    let reloaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        reloaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
}
