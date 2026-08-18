use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind,
};
use ketchup_core::exact_product::{
    BodySubshapeRef, ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactRenderPackage,
    ExactResultRegistry, build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::feature_history::{
    FeatureHistoryError, FeatureHistoryQuery, FeatureHistoryState, RollbackPreviewRequest,
    project_feature_history,
};
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const OTHER_DEFINITION: DefinitionId = DefinitionId(2);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

fn profile(size: f64) -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]],
    }
}

fn seed_single_body() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".to_owned(),
                kind: profile(20.0),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
        ]))
        .unwrap();
    document
}

fn exact_selection(
    snapshot: &ketchup_core::document::Snapshot,
) -> (ExactRenderPackage, BodySubshapeRef) {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:5"),
        )
    });
    let package = build_box_render_package(
        &request,
        "exact-input-5".to_owned(),
        "result-5".to_owned(),
        "occt".to_owned(),
        "r0".to_owned(),
        [[0.0, 0.0, 0.0], [20.0, 20.0, 5.0]],
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    (package, top)
}

fn seed_permuted(reverse: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Part".to_owned(),
            },
        ]))
        .unwrap();
    let profile_a = CanonicalCommand::CreateFeature {
        id: FeatureId(10),
        definition_id: DEFINITION,
        name: "Profile A".to_owned(),
        kind: profile(10.0),
    };
    let profile_b = CanonicalCommand::CreateFeature {
        id: FeatureId(20),
        definition_id: DEFINITION,
        name: "Profile B".to_owned(),
        kind: profile(20.0),
    };
    let extrusion_a = CanonicalCommand::CreateFeature {
        id: FeatureId(11),
        definition_id: DEFINITION,
        name: "Extrusion A".to_owned(),
        kind: FeatureKind::Extrusion {
            profile: FeatureId(10),
            height: Dimension::from_decimal("1").unwrap(),
        },
    };
    let extrusion_b = CanonicalCommand::CreateFeature {
        id: FeatureId(21),
        definition_id: DEFINITION,
        name: "Extrusion B".to_owned(),
        kind: FeatureKind::Extrusion {
            profile: FeatureId(20),
            height: Dimension::from_decimal("2").unwrap(),
        },
    };
    let commands = if reverse {
        vec![profile_b, profile_a, extrusion_b, extrusion_a]
    } else {
        vec![profile_a, profile_b, extrusion_a, extrusion_b]
    };
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn independent_feature_order_is_permutation_stable() {
    let first = seed_permuted(false);
    let second = seed_permuted(true);
    let first_before = stamp(&first);
    let second_before = stamp(&second);
    let query = FeatureHistoryQuery::default();
    let first_projection = project_feature_history(
        &first.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &query,
    )
    .unwrap();
    let second_projection = project_feature_history(
        &second.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &query,
    )
    .unwrap();

    assert_eq!(
        first_projection.bodies[0]
            .features
            .iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>(),
        vec![FeatureId(10), FeatureId(11), FeatureId(20), FeatureId(21)]
    );
    assert_eq!(first_projection.bodies, second_projection.bodies);
    assert_eq!(stamp(&first), first_before);
    assert_eq!(stamp(&second), second_before);
}

#[test]
fn hidden_body_selection_and_cancel_remain_observational_and_isolated() {
    let mut document = seed_single_body();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Hidden".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(20),
                definition_id: DEFINITION,
                name: "Hidden profile".to_owned(),
                kind: profile(4.0),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(21),
                definition_id: DEFINITION,
                name: "Hidden extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(20),
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(1),
            },
        ]))
        .unwrap();
    let before = stamp(&document);
    let preview = project_feature_history(
        &document.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &FeatureHistoryQuery {
            selected_feature_id: Some(FeatureId(21)),
            rollback_preview: Some(RollbackPreviewRequest {
                body_id: BodyId(2),
                first_suppressed_feature_id: FeatureId(21),
            }),
            ..FeatureHistoryQuery::default()
        },
    )
    .unwrap();
    assert!(!preview.bodies[1].visible);
    assert!(!preview.bodies[1].active);
    assert_eq!(preview.selected_feature.unwrap().body_ids, vec![BodyId(2)]);
    assert_eq!(
        preview.bodies[1].features[1].state,
        FeatureHistoryState::RollbackSuppressed
    );

    let cancelled = project_feature_history(
        &document.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &FeatureHistoryQuery::default(),
    )
    .unwrap();
    assert!(
        cancelled
            .bodies
            .iter()
            .flat_map(|body| &body.features)
            .all(|feature| { feature.state == FeatureHistoryState::Active })
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn stale_ambiguous_lost_and_cross_definition_inputs_fail_without_mutation() {
    let mut document = seed_single_body();
    let snapshot = document.current();
    let (package, top) = exact_selection(&snapshot);
    let current_registry = ExactResultRegistry::accept(
        &snapshot,
        [Arc::new(ExactBodyPackage::from(package.clone()))],
    )
    .unwrap();
    let query = FeatureHistoryQuery {
        selected_feature_id: Some(EXTRUSION),
        selected_subshape: Some(top.clone()),
        rollback_preview: None,
    };
    let before_current = stamp(&document);
    assert!(project_feature_history(&snapshot, &current_registry, DEFINITION, &query).is_ok());
    assert_eq!(stamp(&document), before_current);

    let mut alternate = package.clone();
    alternate.identity.backend.push_str("-alternate");
    for reference in &mut alternate.references {
        reference.backend = alternate.identity.backend.clone();
    }
    let ambiguous = ExactResultRegistry::accept(
        &snapshot,
        [
            Arc::new(ExactBodyPackage::from(package)),
            Arc::new(ExactBodyPackage::from(alternate)),
        ],
    )
    .unwrap();
    assert_eq!(
        project_feature_history(&snapshot, &ambiguous, DEFINITION, &query),
        Err(FeatureHistoryError::SubshapeAmbiguous(2))
    );
    assert_eq!(
        project_feature_history(
            &snapshot,
            &ExactResultRegistry::default(),
            DEFINITION,
            &query,
        ),
        Err(FeatureHistoryError::SubshapeLost)
    );
    assert_eq!(stamp(&document), before_current);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("6").unwrap(),
            },
        ]))
        .unwrap();
    let stale_before = stamp(&document);
    assert_eq!(
        project_feature_history(&document.current(), &current_registry, DEFINITION, &query),
        Err(FeatureHistoryError::SubshapeLost)
    );
    assert_eq!(stamp(&document), stale_before);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: OTHER_DEFINITION,
                name: "Other".to_owned(),
            },
        ]))
        .unwrap();
    let cross_before = stamp(&document);
    assert_eq!(
        project_feature_history(
            &document.current(),
            &ExactResultRegistry::default(),
            OTHER_DEFINITION,
            &FeatureHistoryQuery {
                selected_feature_id: Some(EXTRUSION),
                ..FeatureHistoryQuery::default()
            },
        ),
        Err(FeatureHistoryError::FeatureOutsideDefinition(
            EXTRUSION,
            OTHER_DEFINITION
        ))
    );
    assert_eq!(
        project_feature_history(
            &document.current(),
            &ExactResultRegistry::default(),
            OTHER_DEFINITION,
            &FeatureHistoryQuery {
                selected_subshape: Some(top),
                ..FeatureHistoryQuery::default()
            },
        ),
        Err(FeatureHistoryError::SubshapeWrongDefinition(
            OTHER_DEFINITION,
            DEFINITION
        ))
    );
    assert_eq!(stamp(&document), cross_before);
}
