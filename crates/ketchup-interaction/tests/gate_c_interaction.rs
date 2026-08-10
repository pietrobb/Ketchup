use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    InstancePath, OccurrenceId, Transform,
};
use ketchup_interaction::projection::CanonicalInteractionProjection;
use ketchup_interaction::{
    ElementId, InteractionError, InteractionScene, LocaleCatalog, PreviewError, PreviewSession,
    Ray, Side, SmartPushPullOutcome, SnapKind, SnapPolicy, SnapTracker, Vec3, plan_smart_push_pull,
};

#[derive(Clone, Copy)]
struct BoxDefinition {
    id: DefinitionId,
    profile_id: FeatureId,
    extrusion_id: FeatureId,
    size_mm: Vec3,
}

fn projected_scene(
    definitions: &[BoxDefinition],
    occurrences: &[(OccurrenceId, DefinitionId, Vec3)],
) -> InteractionScene {
    let mut commands = Vec::new();
    for definition in definitions {
        commands.extend([
            CanonicalCommand::CreateDefinition {
                id: definition.id,
                name: format!("Definition {}", definition.id.0),
            },
            CanonicalCommand::CreateFeature {
                id: definition.profile_id,
                definition_id: definition.id,
                name: format!("Profile {}", definition.profile_id.0),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [definition.size_mm.x, 0.0],
                        [definition.size_mm.x, definition.size_mm.y],
                        [0.0, definition.size_mm.y],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: definition.extrusion_id,
                definition_id: definition.id,
                name: format!("Extrusion {}", definition.extrusion_id.0),
                kind: FeatureKind::Extrusion {
                    profile: definition.profile_id,
                    height: Dimension::new(definition.size_mm.z.to_string(), definition.size_mm.z)
                        .unwrap(),
                },
            },
        ]);
    }
    commands.extend(occurrences.iter().map(|(id, definition_id, origin_mm)| {
        CanonicalCommand::CreateOccurrence {
            id: *id,
            definition_id: *definition_id,
            name: format!("Occurrence {}", id.0),
            transform: Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        }
    }));
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    CanonicalInteractionProjection::from_snapshot(&store.current())
        .scene()
        .unwrap()
}

fn source_document() -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile-1".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Extrude-1".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
        ]))
        .unwrap();
    store
}

#[test]
fn ten_thousand_occurrences_share_one_authoritative_geometry() {
    let scene = projected_scene(
        &[BoxDefinition {
            id: DefinitionId(1),
            profile_id: FeatureId(1),
            extrusion_id: FeatureId(2),
            size_mm: Vec3::new(100.0, 60.0, 20.0),
        }],
        &(0..10_000_u64)
            .map(|index| {
                (
                    OccurrenceId(index + 1),
                    DefinitionId(1),
                    Vec3::new(
                        (index % 100) as f64 * 125.0,
                        (index / 100) as f64 * 85.0,
                        0.0,
                    ),
                )
            })
            .collect::<Vec<_>>(),
    );
    assert_eq!(scene.occurrence_count(), 10_000);
    assert_eq!(scene.authoritative_geometry_count(), 1);
}

#[test]
fn cpu_query_returns_exact_face_identity_and_endpoint_snap() {
    let scene = projected_scene(
        &[BoxDefinition {
            id: DefinitionId(11),
            profile_id: FeatureId(11),
            extrusion_id: FeatureId(12),
            size_mm: Vec3::new(100.0, 60.0, 20.0),
        }],
        &[(OccurrenceId(7), DefinitionId(11), Vec3::ZERO)],
    );
    let ray = Ray::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let result = scene.exact_pick(ray, 0.01).unwrap();

    assert_eq!(result.primary.reference.definition_id, DefinitionId(11));
    assert_eq!(
        result.primary.reference.instance_path,
        InstancePath::root(OccurrenceId(7))
    );
    assert_eq!(
        result.primary.reference.element,
        ElementId::Face {
            axis: ketchup_interaction::Axis::Z,
            side: Side::Maximum,
        }
    );
    assert_eq!(result.primary.position_mm, Vec3::new(0.0, 0.0, 20.0));
    assert_eq!(result.snap.kind, SnapKind::Endpoint);
    assert_eq!(result.snap.reference.element, ElementId::Endpoint(4));
}

#[test]
fn exact_filters_distinguish_faces_edges_and_points() {
    let scene = projected_scene(
        &[BoxDefinition {
            id: DefinitionId(1),
            profile_id: FeatureId(1),
            extrusion_id: FeatureId(2),
            size_mm: Vec3::new(100.0, 60.0, 20.0),
        }],
        &[(OccurrenceId(1), DefinitionId(1), Vec3::ZERO)],
    );
    let edge_ray = Ray::new(Vec3::new(50.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let edge = scene
        .exact_pick_filtered(edge_ray, 0.01, ketchup_interaction::SelectionFilter::Edge)
        .unwrap();
    assert!(matches!(edge.primary.reference.element, ElementId::Edge(_)));

    let point_ray = Ray::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let point = scene
        .exact_pick_filtered(point_ray, 0.01, ketchup_interaction::SelectionFilter::Point)
        .unwrap();
    assert_eq!(point.primary.reference.element, ElementId::Endpoint(4));
}

#[test]
fn crossing_exact_edges_produce_an_explicit_intersection_snap() {
    let scene = projected_scene(
        &[BoxDefinition {
            id: DefinitionId(1),
            profile_id: FeatureId(1),
            extrusion_id: FeatureId(2),
            size_mm: Vec3::new(10.0, 10.0, 10.0),
        }],
        &[
            (OccurrenceId(1), DefinitionId(1), Vec3::new(0.0, 5.0, 0.0)),
            (OccurrenceId(2), DefinitionId(1), Vec3::new(5.0, 0.0, 0.0)),
        ],
    );
    let ray = Ray::new(Vec3::new(5.0, 5.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let result = scene.exact_pick(ray, 0.01).unwrap();
    assert_eq!(result.snap.kind, SnapKind::Intersection);
    assert!(matches!(
        result.snap.reference.element,
        ElementId::Intersection {
            other_instance_path,
            ..
        } if other_instance_path == InstancePath::root(OccurrenceId(2))
    ));
}

#[test]
fn overlapping_candidates_are_stable_and_nearest_first() {
    let scene = projected_scene(
        &[
            BoxDefinition {
                id: DefinitionId(2),
                profile_id: FeatureId(21),
                extrusion_id: FeatureId(22),
                size_mm: Vec3::new(10.0, 10.0, 10.0),
            },
            BoxDefinition {
                id: DefinitionId(1),
                profile_id: FeatureId(11),
                extrusion_id: FeatureId(12),
                size_mm: Vec3::new(10.0, 10.0, 10.0),
            },
        ],
        &[
            (OccurrenceId(2), DefinitionId(2), Vec3::new(0.0, 0.0, -20.0)),
            (OccurrenceId(1), DefinitionId(1), Vec3::ZERO),
        ],
    );
    let ray = Ray::new(Vec3::new(5.0, 5.0, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let result = scene.exact_pick(ray, 0.1).unwrap();
    assert_eq!(
        result.primary.reference.instance_path,
        InstancePath::root(OccurrenceId(1))
    );
    assert_eq!(
        result
            .overlapping
            .iter()
            .map(|hit| hit.reference.instance_path.clone())
            .collect::<Vec<_>>(),
        vec![
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ]
    );
    assert_eq!(
        result.overlap_choice(3).unwrap().reference.instance_path,
        InstancePath::root(OccurrenceId(2)),
        "overlap choice must cycle deterministically"
    );
}

#[test]
fn snap_scoring_and_hysteresis_are_deterministic() {
    let scene = projected_scene(
        &[BoxDefinition {
            id: DefinitionId(1),
            profile_id: FeatureId(1),
            extrusion_id: FeatureId(2),
            size_mm: Vec3::new(10.0, 10.0, 10.0),
        }],
        &[(OccurrenceId(1), DefinitionId(1), Vec3::ZERO)],
    );
    let pick = |x| {
        scene
            .exact_pick(
                Ray::new(Vec3::new(x, 0.0, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
                2.0,
            )
            .unwrap()
    };
    let policy = SnapPolicy::new(1.0, 2.0).unwrap();
    let mut tracker = SnapTracker::default();

    let acquired = pick(0.5);
    assert_eq!(acquired.snap.kind, SnapKind::Endpoint);
    assert_eq!(acquired.snap.score().kind_rank, 0);
    assert_eq!(
        tracker.update(Some(&acquired), policy).unwrap().kind,
        SnapKind::Endpoint
    );

    let retained = pick(1.5);
    assert_eq!(retained.snap.kind, SnapKind::Endpoint);
    assert_eq!(retained.snap.reference, acquired.snap.reference);
    assert_eq!(
        tracker.update(Some(&retained), policy).unwrap().position_mm,
        acquired.snap.position_mm,
        "release tolerance must retain the original stable snap"
    );

    let released = pick(2.5);
    assert_eq!(
        tracker.update(Some(&released), policy).unwrap().kind,
        SnapKind::Face
    );
    tracker.update(None, policy);
    assert!(tracker.locked().is_none());
}

#[test]
fn invalid_snap_hysteresis_policy_fails_closed() {
    assert!(SnapPolicy::new(f64::NAN, 2.0).is_none());
    assert!(SnapPolicy::new(2.0, 1.0).is_none());
    assert!(SnapPolicy::new(-1.0, 2.0).is_none());
}

#[test]
fn smart_push_pull_preview_and_commit_share_one_canonical_digest() {
    let mut store = source_document();
    let plan = match plan_smart_push_pull(&store, &[FeatureId(2)], "35").unwrap() {
        SmartPushPullOutcome::Ready(plan) => plan,
        SmartPushPullOutcome::NeedsChoice { .. } => panic!("source should be unambiguous"),
    };
    let catalog = LocaleCatalog::english();
    assert_eq!(
        plan.action_digest().render(&catalog),
        "Change Extrude-1 height from 20 mm to 35 mm"
    );
    let preview_digest = plan.action_digest().command_digest.clone();
    let committed = PreviewSession::new(1, plan).confirm(&mut store).unwrap();
    assert_eq!(committed.action_digest.command_digest, preview_digest);
    assert_eq!(committed.revision.batch_digest(), preview_digest);
    assert!(matches!(
        store.current().feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.source_token() == "35"
    ));
}

#[test]
fn cancelled_and_stale_previews_never_mutate_the_document() {
    let mut store = source_document();
    let original = store.current().canonical_digest();
    let plan = match plan_smart_push_pull(&store, &[FeatureId(2)], "35").unwrap() {
        SmartPushPullOutcome::Ready(plan) => plan,
        SmartPushPullOutcome::NeedsChoice { .. } => unreachable!(),
    };
    let mut preview = PreviewSession::new(1, plan);
    preview.cancel();
    assert!(matches!(
        preview.confirm(&mut store),
        Err(PreviewError::Cancelled)
    ));
    assert_eq!(store.current().canonical_digest(), original);

    let stale_plan = match plan_smart_push_pull(&store, &[FeatureId(2)], "35").unwrap() {
        SmartPushPullOutcome::Ready(plan) => plan,
        SmartPushPullOutcome::NeedsChoice { .. } => unreachable!(),
    };
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::from_decimal("25").unwrap(),
            },
        ]))
        .unwrap();
    let after_external_edit = store.current().canonical_digest();
    assert!(matches!(
        PreviewSession::new(2, stale_plan).confirm(&mut store),
        Err(PreviewError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), after_external_edit);
}

#[test]
fn ambiguous_push_pull_requires_a_choice_and_creates_no_command() {
    let store = source_document();
    let revision = store.current().revision_id();
    let outcome = plan_smart_push_pull(&store, &[FeatureId(2), FeatureId(3)], "35").unwrap();
    match outcome {
        SmartPushPullOutcome::NeedsChoice { candidates } => {
            assert_eq!(candidates, vec![FeatureId(2), FeatureId(3)]);
        }
        SmartPushPullOutcome::Ready(_) => panic!("ambiguous sources must not be selected silently"),
    }
    assert_eq!(store.current().revision_id(), revision);
}

#[test]
fn real_and_pseudo_locales_match_the_complete_english_key_set() {
    let english = LocaleCatalog::english();
    let slovak = LocaleCatalog::slovak();
    let pseudo = LocaleCatalog::pseudo();

    assert!(
        english.key_count() > 400,
        "the complete shell resource must be covered"
    );
    assert_eq!(slovak.validate_complete_against(&english), Ok(()));
    assert_eq!(pseudo.validate_complete_against(&english), Ok(()));
    assert_eq!(slovak.key_count(), english.key_count());
    assert_eq!(pseudo.key_count(), english.key_count());
    assert_eq!(slovak.text("menu-file"), "Súbor");
    assert_ne!(slovak.text("menu-file"), english.text("menu-file"));
}

#[test]
fn locale_completeness_rejects_missing_and_extra_keys() {
    let reference = LocaleCatalog::parse("app-title = Ketchup\nmenu-file = File").unwrap();
    let missing = LocaleCatalog::parse("app-title = Kečup").unwrap();
    let extra =
        LocaleCatalog::parse("app-title = Kečup\nmenu-file = Súbor\nmenu-unexpected = Neočakávané")
            .unwrap();

    assert_eq!(
        missing.validate_complete_against(&reference),
        Err(InteractionError::InvalidLocaleResource)
    );
    assert_eq!(
        extra.validate_complete_against(&reference),
        Err(InteractionError::InvalidLocaleResource)
    );
}

#[test]
fn pseudo_locale_expands_visible_text_without_rewriting_arguments() {
    let pseudo = LocaleCatalog::pseudo();
    let rendered = pseudo.format(
        "scene-box-count",
        &std::collections::BTreeMap::from([("count", "7".to_owned())]),
    );

    assert!(rendered.starts_with("[!! "));
    assert!(rendered.ends_with(" !!]"));
    assert!(rendered.len() > "Boxes: 7".len() * 2);
    assert!(rendered.contains('7'));
    assert!(!rendered.contains("{ $count }"));
    assert_eq!(pseudo.text("shortcut-none"), "[!!  !!]");
}

#[test]
fn every_narrow_ui_string_is_resolved_from_the_english_resource() {
    let catalog = LocaleCatalog::english();
    for key in [
        "app-title",
        "viewport-label",
        "tool-select",
        "tool-push-pull",
        "help-push-pull",
        "help-camera",
        "selection-none",
        "selection-top-face",
        "selection-other-face",
        "status-ready",
        "status-preview",
        "status-exact-pending",
        "action-smart-push-pull-height",
        "choice-smart-push-pull-source",
        "error-preview-stale",
    ] {
        assert!(catalog.contains(key), "missing locale key {key}");
    }
}
