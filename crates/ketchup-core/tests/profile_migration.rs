//! Documents written before point, segment and spline profiles became one `Profile`
//! load as segment profiles that keep their digest, parameter targets and exact geometry.

use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, FeatureParameterTarget, ParameterPath, ParameterValueType,
    PersistentDimensionId, PersistentDimensionTarget, ProfileSegment,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepPlanarGeometry};
use ketchup_core::persistence;

/// Written by the retired field-by-field codec: a point profile (10) extruded (11), a
/// segment profile with an arc (20) extruded (21), and two spline profiles (30, 31)
/// lofted (32). Dimension 1 targets `points.1.x` of 10, 2 `segments.1.center.y` of 20,
/// 3 `control_points.0.x` of 30 and 4 `control_points.2.y` of 31.
const FIXTURE: &[u8] = include_bytes!("fixtures/persistence/legacy/profile-records-schema97.bin");
/// The canonical digest the document had when the retired codec wrote it.
const DIGEST: &str = "74452e70c78882d7";
const LOWER: [[f64; 2]; 4] = [[-8.0, -4.0], [9.0, -3.0], [7.0, 6.0], [-6.0, 5.0]];
const UPPER: [[f64; 2]; 4] = [[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]];

#[test]
fn old_point_segment_and_spline_records_load_as_one_profile_kind() {
    let loaded = persistence::load(FIXTURE).unwrap();
    assert!(loaded.migration_losses().is_empty());
    // The profiles hash, under the frozen pre-serde digest, to what the old records did.
    assert_eq!(loaded.audit().source_canonical_digest, DIGEST);
    let snapshot = loaded.snapshot();
    let kind = |id| snapshot.feature(FeatureId(id)).unwrap().kind();

    assert_eq!(
        kind(10),
        &FeatureKind::polygon(&[[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]])
    );
    let FeatureKind::Profile {
        segments,
        closed: true,
    } = kind(20)
    else {
        panic!("segment profile was not kept closed");
    };
    assert!(matches!(
        segments.as_slice(),
        [
            ProfileSegment::Line { .. },
            ProfileSegment::CircularArc { .. },
            ProfileSegment::Line { .. },
            ProfileSegment::Line { .. }
        ]
    ));
    // An old spline profile is one spline segment that closes on its first point.
    assert_eq!(kind(30), &FeatureKind::closed_spline(&LOWER));
    assert_eq!(kind(31).closed_spline_points(), Some(&UPPER[..]));

    // Every old target is renamed and still reads the value it read before.
    for (id, feature, path, value) in [
        (1, 10, "segments.1.start.x", 40.0),
        (2, 20, "segments.1.center.y", 10.0),
        (3, 30, "segments.0.start.x", -8.0),
        (4, 31, "segments.0.points.2.y", 3.0),
    ] {
        let PersistentDimensionTarget::FeatureParameter(target) = &snapshot
            .persistent_dimension(PersistentDimensionId(id))
            .unwrap()
            .target
        else {
            panic!("dimension {id} lost its feature target");
        };
        assert_eq!(
            (target.feature_id, target.path.as_str()),
            (FeatureId(feature), path)
        );
        assert_eq!(
            kind(feature).parameter_value(&target.path),
            Some(value),
            "{path}"
        );
    }

    // The loft still sends each section to the exact kernel as one periodic spline.
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(3), FeatureId(32)).unwrap();
    let splines = graph
        .profiles
        .iter()
        .map(|profile| match &profile.geometry {
            ExactBRepPlanarGeometry::Spline { control_point_bits } => control_point_bits.clone(),
            other => panic!("expected a spline section, got {other:?}"),
        })
        .collect::<Vec<_>>();
    let bits = |points: [[f64; 2]; 4]| {
        points
            .iter()
            .map(|point| point.map(f64::to_bits))
            .collect::<Vec<_>>()
    };
    assert_eq!(splines, vec![bits(LOWER), bits(UPPER)]);

    let reloaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        reloaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
}

#[test]
fn moving_the_start_of_a_closed_spline_moves_its_closing_point() {
    let mut document = DocumentStore::new();
    let profile = FeatureId(1);
    let target = |path: &str| {
        FeatureParameterTarget::new(profile, path, ParameterValueType::Length).unwrap()
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Spline".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: DefinitionId(1),
                name: "Spline".into(),
                kind: FeatureKind::closed_spline(&LOWER),
            },
            CanonicalCommand::SetFeatureParameter {
                target: target("segments.0.start.x"),
                dimension: Dimension::from_decimal("-10").unwrap(),
            },
            CanonicalCommand::SetFeatureParameter {
                target: target("segments.0.points.2.y"),
                dimension: Dimension::from_decimal("8").unwrap(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let kind = snapshot.feature(profile).unwrap().kind();
    assert_eq!(
        kind,
        &FeatureKind::closed_spline(&[[-10.0, -4.0], [9.0, -3.0], [7.0, 8.0], [-6.0, 5.0]])
    );
    // The closing point is not a parameter of its own: it is the start.
    let paths = kind
        .parameter_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.path().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "segments.0.start.x",
            "segments.0.start.y",
            "segments.0.points.1.x",
            "segments.0.points.1.y",
            "segments.0.points.2.x",
            "segments.0.points.2.y",
            "segments.0.points.3.x",
            "segments.0.points.3.y",
        ]
    );
    // An end point is edited through `start`/`end`, not as an inner point.
    assert_eq!(
        kind.parameter_value(&ParameterPath::new("segments.0.points.0.x").unwrap()),
        None
    );
}

#[test]
fn a_closed_loop_may_run_either_way_round_but_must_enclose_an_area() {
    let create = |kind: FeatureKind| {
        DocumentStore::new().apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Loop".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind,
            },
        ]))
    };
    // Line loops were drawn and imported clockwise before they became polygons.
    let clockwise = LOWER.iter().rev().copied().collect::<Vec<_>>();
    assert!(create(FeatureKind::polygon(&clockwise)).is_ok());
    assert!(create(FeatureKind::closed_spline(&clockwise)).is_ok());
    for degenerate in [
        vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]],
        vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]],
    ] {
        assert_eq!(
            create(FeatureKind::polygon(&degenerate)).err(),
            Some(CanonicalError::InvalidProfile)
        );
    }
}
