//! Scale stretches a part's own definition and leaves its placement only
//! moving and turning it, so whatever reads the definition geometry (its
//! volume, its drawing, the saved file) sees the scaled part. A part Scale
//! cannot stretch that way is refused by name and nothing changes.

use super::*;
use crate::drawn_shape::tests::evaluate_exact;

const PART: OccurrenceId = OccurrenceId(1);

fn volume(app: &mut KetchupApp) -> f64 {
    evaluate_exact(app);
    app.selection.select_occurrence(PART, false);
    let measures = app.selection_measures().unwrap();
    measures.volume_mm3.expect("the part has an exact volume")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6 * expected.abs(),
        "{actual} != {expected}"
    );
}

fn scale_factors(app: &KetchupApp) -> Vec<[f64; 3]> {
    app.document
        .current()
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::Scale { factors, .. } => Some(*factors),
            _ => None,
        })
        .collect()
}

/// Scale was refused with the message `key` and left the document and its
/// Undo history as they were.
fn assert_refused(app: &mut KetchupApp, factor: f64, key: &str) {
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());
    app.selection.select_occurrence(PART, false);
    assert!(!app.scale_selected(factor), "{key}");
    assert_eq!(app.action_digest(), app.catalog.text(key));
    assert_eq!(app.canonical_digest(), digest, "{key}");
    assert_eq!(app.undo_step_count(), undo, "{key}");
}

#[test]
fn scale_stretches_the_part_definition_in_one_undo_step_and_survives_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = KetchupApp::new();
    let solid = volume(&mut app);
    let placement = *app
        .document
        .current()
        .occurrence(PART)
        .unwrap()
        .transform()
        .matrix();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());

    assert!(app.scale_selected(1.5), "{}", app.action_digest());
    assert_eq!(app.undo_step_count(), undo + 1);
    assert_eq!(
        *app.document
            .current()
            .occurrence(PART)
            .unwrap()
            .transform()
            .matrix(),
        placement,
        "the placement must still only move and turn the part"
    );
    assert_eq!(scale_factors(&app), [[1.5; 3]]);
    let grown = volume(&mut app);
    assert_close(grown, solid * 1.5_f64.powi(3));

    let path = directory.path().join("scaled.ketchup");
    assert!(app.save_document_to(&path), "{}", app.action_digest());
    let mut reopened = KetchupApp::new();
    assert!(
        reopened.open_document_from(&path),
        "{}",
        reopened.action_digest()
    );
    assert_eq!(reopened.canonical_digest(), app.canonical_digest());
    assert_close(volume(&mut reopened), grown);
    // A placement-scaled part refused its drawing; a scaled definition has one.
    reopened.selection.select_occurrence(PART, false);
    assert!(
        reopened.preview_selection_drawing(),
        "{}",
        reopened.action_digest()
    );

    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_close(volume(&mut app), solid);
    assert!(app.redo());
    assert_close(volume(&mut app), grown);
}

#[test]
fn an_axis_scale_stretches_the_part_along_its_own_aligned_axis_only() {
    let mut app = KetchupApp::new();
    let solid = volume(&mut app);
    app.gesture.transform.scale_axis_lock = Some(Axis::X);
    assert!(app.scale_selected(2.0), "{}", app.action_digest());
    assert_eq!(scale_factors(&app), [[2.0, 1.0, 1.0]]);
    assert_close(volume(&mut app), solid * 2.0);
    assert!(app.undo());

    // A quarter turn about Z lays the part's own y axis along world x.
    app.selection.select_occurrence(PART, false);
    assert!(app.rotate_selected(90.0), "{}", app.action_digest());
    app.selection.select_occurrence(PART, false);
    app.gesture.transform.scale_axis_lock = Some(Axis::X);
    assert!(app.scale_selected(2.0), "{}", app.action_digest());
    assert_eq!(scale_factors(&app), [[1.0, 2.0, 1.0]]);
    assert_close(volume(&mut app), solid * 2.0);
}

#[test]
fn scale_refuses_by_name_a_shear_and_a_placement_that_already_scales() {
    let mut app = KetchupApp::new();
    let solid = volume(&mut app);

    // Along world x a part turned by 30° would shear; evenly it only grows.
    app.selection.select_occurrence(PART, false);
    assert!(app.rotate_selected(30.0), "{}", app.action_digest());
    app.gesture.transform.scale_axis_lock = Some(Axis::X);
    assert_refused(&mut app, 2.0, "digest-scale-refused-shear");
    app.gesture.transform.scale_axis_lock = None;
    app.selection.select_occurrence(PART, false);
    assert!(app.scale_selected(2.0), "{}", app.action_digest());
    assert_close(volume(&mut app), solid * 8.0);
    assert!(app.undo());
    assert!(app.undo());

    // An older document can hold a placement that scales: Scale leaves it.
    let mut scaled = *app
        .document
        .current()
        .occurrence(PART)
        .unwrap()
        .transform()
        .matrix();
    scaled[0] *= 1.5;
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: PART,
                transform: Transform::from_matrix(scaled).unwrap(),
            },
        ]))
        .unwrap();
    assert_refused(&mut app, 2.0, "digest-scale-refused-placement");
}

#[test]
fn a_scale_feature_with_a_factor_of_zero_or_less_or_not_finite_is_refused() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current();
    let definition_id = snapshot.occurrence(PART).unwrap().definition_id();
    let target =
        *ketchup_model::exact_product::exact_body_terminal_features(&snapshot, definition_id)
            .unwrap()
            .values()
            .next()
            .unwrap();
    let id = FeatureId(
        snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap()
            + 1,
    );
    let digest = app.canonical_digest();
    let scale = |origin_mm: [f64; 3], factors: [f64; 3]| {
        CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name: "Scale".to_owned(),
            kind: FeatureKind::Scale {
                target,
                origin_mm,
                factors,
            },
        }])
    };
    for factors in [
        [0.0, 1.0, 1.0],
        [1.0, -2.0, 1.0],
        [1.0, 1.0, f64::INFINITY],
        [f64::NAN; 3],
    ] {
        let error = app.document.apply_batch(&scale([0.0; 3], factors)).err();
        assert!(
            matches!(error, Some(CanonicalError::InvalidScale)),
            "{factors:?}: {error:?}"
        );
    }
    let error = app
        .document
        .apply_batch(&scale([f64::INFINITY, 0.0, 0.0], [2.0; 3]))
        .err();
    assert!(
        matches!(error, Some(CanonicalError::InvalidScale)),
        "{error:?}"
    );
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.document.apply_batch(&scale([0.0; 3], [2.0; 3])).is_ok());
}
