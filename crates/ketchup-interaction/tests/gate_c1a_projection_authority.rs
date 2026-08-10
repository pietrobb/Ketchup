use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    GroupId, OccurrenceId, Transform,
};
use ketchup_core::state_view::encode_semantic_state;
use ketchup_interaction::projection::{
    CanonicalInteractionProjection, INTERACTION_PROJECTION_V1, PROXY_BACKEND_V1,
    PROXY_EVALUATOR_V1, ProjectionStatus,
};
use ketchup_interaction::{Axis, ElementId, Ray, Vec3};

const DEFINITION: DefinitionId = DefinitionId(10);
const UNRELATED_PROFILE: FeatureId = FeatureId(9);
const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);
const SECOND_EXTRUSION: FeatureId = FeatureId(13);
const CUT_PROFILE: FeatureId = FeatureId(14);
const CUT: FeatureId = FeatureId(15);
const GROUP: GroupId = GroupId(20);
const FIRST: OccurrenceId = OccurrenceId(30);
const SECOND: OccurrenceId = OccurrenceId(31);

fn source_document() -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Cabinet".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: UNRELATED_PROFILE,
                definition_id: DEFINITION,
                name: "Unrelated profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [-50.0, -40.0],
                        [-10.0, -40.0],
                        [-10.0, -20.0],
                        [-50.0, -20.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[25.0, 40.0], [625.0, 40.0], [625.0, 620.0], [25.0, 620.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("720").unwrap(),
                },
            },
            CanonicalCommand::CreateGroup {
                id: GROUP,
                name: "Kitchen run".to_owned(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Cabinet #1".to_owned(),
                transform: Transform::identity(),
                parent: Some(GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Cabinet #2".to_owned(),
                transform: Transform::from_translation(700.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: false,
            },
        ]))
        .unwrap();
    store
}

#[test]
fn canonical_projection_carries_every_c1a_authority_field() {
    let store = source_document();
    let snapshot = store.current();
    let digest = snapshot.canonical_digest();
    let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
    let state = encode_semantic_state(&snapshot);
    let complete = state.complete_v1();
    let agent = state.agent_v1();

    for state_view in [&complete, &agent] {
        assert!(state_view.contains(&format!(
            "source.document_id={}",
            projection.document_id().0
        )));
        assert!(state_view.contains(&format!("source.revision={}", projection.source_revision())));
        assert!(state_view.contains(&format!(
            "source.canonical_digest={}",
            projection.source_digest()
        )));
    }
    assert!(complete.contains("definition.10.features=[9,11,12]"));
    assert!(agent.contains("definition.10=name:\"Cabinet\",features:[9,11,12]"));
    for feature_id in [UNRELATED_PROFILE, PROFILE, EXTRUSION] {
        assert!(complete.contains(&format!("feature.{}.definition=10", feature_id.0)));
        assert!(agent.contains(&format!("feature.{}=", feature_id.0)));
    }
    for occurrence_id in [FIRST, SECOND] {
        assert!(complete.contains(&format!("occurrence.{}.definition=10", occurrence_id.0)));
        assert!(agent.contains(&format!("occurrence.{}=", occurrence_id.0)));
    }
    assert!(agent.contains(
        "summary.counts=evaluator_nodes:0,overrides:0,parameter_bindings:0,spaces:0,clearance_volumes:0,persistent_dimensions:0,tags:0,collections:0,definitions:1,features:3,occurrences:2,groups:1,local_groups:0,local_occurrences:0"
    ));

    assert_eq!(projection.schema(), INTERACTION_PROJECTION_V1);
    assert_eq!(projection.evaluator(), PROXY_EVALUATOR_V1);
    assert_eq!(projection.backend(), PROXY_BACKEND_V1);
    assert_eq!(projection.document_id(), snapshot.document_id());
    assert_eq!(projection.source_revision(), snapshot.revision_id());
    assert_eq!(projection.source_digest(), digest);
    assert_eq!(projection.status(), ProjectionStatus::ProxyIncomplete);
    assert!(projection.is_current(&snapshot));
    assert_eq!(projection.occurrences().len(), 2);

    let first = &projection.occurrences()[0];
    assert_eq!(first.occurrence_id, FIRST);
    assert_eq!(first.body.definition_id, DEFINITION);
    assert_eq!(first.body.profile_feature_id, Some(PROFILE));
    assert_eq!(first.body.extrusion_feature_id, Some(EXTRUSION));
    assert_eq!(first.parent, Some(GROUP));
    assert!(first.visible);
    assert_eq!(first.shared_occurrence_count, 2);
    assert_eq!(first.canonical_world_transform.matrix()[3], 100.0);
    let local_box = first.local_box.unwrap();
    assert_eq!(local_box.origin_mm, Vec3::new(25.0, 40.0, 0.0));
    assert_eq!(local_box.size_mm, Vec3::new(600.0, 580.0, 720.0));
    let first_box = first.box_proxy.unwrap();
    assert_eq!(first_box.origin_mm, Vec3::new(125.0, 40.0, 0.0));
    assert_eq!(first_box.size_mm.x, 600.0);
    assert_eq!(first_box.size_mm.y, 580.0);
    assert_eq!(first_box.size_mm.z, 720.0);

    let hidden = &projection.occurrences()[1];
    assert_eq!(hidden.occurrence_id, SECOND);
    assert!(!hidden.visible);
    let scene = projection.scene().unwrap();
    assert_eq!(scene.occurrence_count(), 1);
    assert_eq!(scene.authoritative_geometry_count(), 1);
    assert_eq!(snapshot.canonical_digest(), digest);
}

#[test]
fn profile_only_definition_projects_as_a_flat_selectable_plane() {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Profile only".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Closed polyline".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, 3.0], [12.0, 3.0], [12.0, 9.0], [2.0, 9.0]],
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Profile occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();

    let projection = CanonicalInteractionProjection::from_snapshot(&store.current());
    let occurrence = &projection.occurrences()[0];
    assert_eq!(occurrence.body.profile_feature_id, Some(PROFILE));
    assert_eq!(occurrence.body.extrusion_feature_id, None);
    assert_eq!(
        occurrence.local_box.unwrap(),
        ketchup_interaction::projection::ProjectedBox {
            origin_mm: Vec3::new(2.0, 3.0, 0.0),
            size_mm: Vec3::new(10.0, 6.0, 0.0),
        }
    );
    let hit = projection
        .scene()
        .unwrap()
        .exact_pick(
            Ray::new(Vec3::new(7.0, 6.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .unwrap();
    assert_eq!(
        hit.primary.reference.element,
        ElementId::Face {
            axis: Axis::Z,
            side: ketchup_interaction::Side::Maximum,
        }
    );
}

#[test]
fn old_projection_becomes_stale_and_edits_appear_only_after_successful_batch() {
    let mut store = source_document();
    let before = store.current();
    let old_projection = CanonicalInteractionProjection::from_snapshot(&before);
    assert_eq!(old_projection.scene().unwrap().occurrence_count(), 1);

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SECOND,
                visible: true,
            },
        ]))
        .unwrap();
    let after = store.current();
    assert!(!old_projection.is_current(&after));
    assert_eq!(old_projection.scene().unwrap().occurrence_count(), 1);

    let current_projection = CanonicalInteractionProjection::from_snapshot(&after);
    assert!(current_projection.is_current(&after));
    let current_scene = current_projection.scene().unwrap();
    assert_eq!(current_scene.occurrence_count(), 2);
    assert_eq!(current_scene.authoritative_geometry_count(), 1);
}

#[test]
fn exact_only_cut_definition_never_falls_back_to_a_filled_box_proxy() {
    let mut store = source_document();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Cut profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [100.0, 100.0],
                        [200.0, 100.0],
                        [200.0, 200.0],
                        [100.0, 200.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT,
                definition_id: DEFINITION,
                name: "Through cut".to_owned(),
                kind: FeatureKind::ThroughCut {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                },
            },
        ]))
        .unwrap();

    let projection = CanonicalInteractionProjection::from_snapshot(&store.current());
    assert_eq!(projection.occurrences().len(), 2);
    for occurrence in projection.occurrences() {
        assert_eq!(occurrence.body.definition_id, DEFINITION);
        assert_eq!(occurrence.body.profile_feature_id, None);
        assert_eq!(occurrence.body.extrusion_feature_id, None);
        assert_eq!(occurrence.local_box, None);
        assert_eq!(occurrence.box_proxy, None);
    }
    assert_eq!(projection.scene().unwrap().occurrence_count(), 0);
}

#[test]
fn multiple_extrusions_fail_closed() {
    let mut store = source_document();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: SECOND_EXTRUSION,
            definition_id: DEFINITION,
            name: "Ambiguous extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::from_decimal("100").unwrap(),
            },
        }]))
        .unwrap();

    let projection = CanonicalInteractionProjection::from_snapshot(&store.current());
    assert_eq!(projection.occurrences().len(), 2);
    for occurrence in projection.occurrences() {
        assert_eq!(occurrence.body.definition_id, DEFINITION);
        assert_eq!(occurrence.body.profile_feature_id, None);
        assert_eq!(occurrence.body.extrusion_feature_id, None);
        assert_eq!(occurrence.local_box, None);
        assert_eq!(occurrence.box_proxy, None);
    }
    assert_eq!(projection.scene().unwrap().occurrence_count(), 0);
}
