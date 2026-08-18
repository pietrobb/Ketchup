use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactResultRegistry,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::feature_history::{
    FeatureHistoryError, FeatureHistoryQuery, FeatureHistoryState, RollbackPreviewRequest,
    project_feature_history,
};
use ketchup_core::persistence;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const BASE_PROFILE: FeatureId = FeatureId(10);
const BASE_EXTRUSION: FeatureId = FeatureId(11);
const TOOL_PROFILE: FeatureId = FeatureId(20);
const TOOL_EXTRUSION: FeatureId = FeatureId(21);
const UNION: FeatureId = FeatureId(30);

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

fn profile() -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
    }
}

fn seed_two_body_history() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base profile".to_owned(),
                kind: profile(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Tool body".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_PROFILE,
                definition_id: DEFINITION,
                name: "Tool profile".to_owned(),
                kind: profile(),
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: TOOL_PROFILE,
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(1),
            },
            CanonicalCommand::CreateFeature {
                id: UNION,
                definition_id: DEFINITION,
                name: "Union".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: BASE_EXTRUSION,
                    tool: TOOL_EXTRUSION,
                },
            },
        ]))
        .unwrap();
    document
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
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base profile".to_owned(),
                kind: profile(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
        ]))
        .unwrap();
    document
}

#[test]
fn body_history_projection_is_deterministic_body_scoped_and_observational() {
    let document = seed_two_body_history();
    let before = stamp(&document);
    let query = FeatureHistoryQuery {
        selected_feature_id: Some(UNION),
        rollback_preview: Some(RollbackPreviewRequest {
            body_id: BodyId(1),
            first_suppressed_feature_id: BASE_EXTRUSION,
        }),
        ..FeatureHistoryQuery::default()
    };

    let first = project_feature_history(
        &document.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &query,
    )
    .unwrap();
    let second = project_feature_history(
        &document.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &query,
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(stamp(&document), before);
    assert_eq!(
        first
            .bodies
            .iter()
            .map(|body| body.body_id)
            .collect::<Vec<_>>(),
        vec![BodyId(1), BodyId(2)]
    );
    assert_eq!(
        first.bodies[0]
            .features
            .iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>(),
        vec![BASE_PROFILE, BASE_EXTRUSION, UNION]
    );
    assert_eq!(
        first.bodies[1]
            .features
            .iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>(),
        vec![TOOL_PROFILE, TOOL_EXTRUSION]
    );
    assert!(first.bodies[0].active);
    assert!(!first.bodies[1].active);
    assert!(!first.bodies[1].visible);
    assert_eq!(
        first.bodies[0]
            .features
            .iter()
            .map(|feature| feature.state)
            .collect::<Vec<_>>(),
        vec![
            FeatureHistoryState::Active,
            FeatureHistoryState::RollbackSuppressed,
            FeatureHistoryState::RollbackSuppressed,
        ]
    );
    assert!(
        first.bodies[1]
            .features
            .iter()
            .all(|feature| feature.state == FeatureHistoryState::Active)
    );
    assert_eq!(
        first
            .rollback_preview
            .as_ref()
            .unwrap()
            .suppressed_feature_ids,
        vec![BASE_EXTRUSION, UNION]
    );
    let selected = first.selected_feature.as_ref().unwrap();
    assert_eq!(selected.body_ids, vec![BodyId(1)]);
    assert_eq!(selected.dependencies, vec![BASE_EXTRUSION, TOOL_EXTRUSION]);
    assert_eq!(selected.input_body_ids, vec![BodyId(1), BodyId(2)]);
    assert_eq!(selected.output_body_id, Some(BodyId(1)));

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    let reopened_projection = project_feature_history(
        &reopened.snapshot(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &query,
    )
    .unwrap();
    assert_eq!(reopened_projection, first);
}

#[test]
fn rollback_preview_rejects_a_suffix_with_cross_body_dependents() {
    let document = seed_two_body_history();
    let before = stamp(&document);
    let result = project_feature_history(
        &document.current(),
        &ExactResultRegistry::default(),
        DEFINITION,
        &FeatureHistoryQuery {
            rollback_preview: Some(RollbackPreviewRequest {
                body_id: BodyId(2),
                first_suppressed_feature_id: TOOL_EXTRUSION,
            }),
            ..FeatureHistoryQuery::default()
        },
    );
    assert_eq!(
        result,
        Err(FeatureHistoryError::RollbackNotDependencyClosed(
            TOOL_EXTRUSION
        ))
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn resolved_subshape_selection_projects_stable_provenance_without_mutation() {
    let document = seed_single_body();
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let document_id = snapshot.document_id();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                document_id,
                BASE_EXTRUSION,
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
        [[0.0, 0.0, 0.0], [20.0, 10.0, 5.0]],
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    let registry =
        ExactResultRegistry::accept(&snapshot, [Arc::new(ExactBodyPackage::from(package))])
            .unwrap();
    let before = stamp(&document);
    let query = FeatureHistoryQuery {
        selected_feature_id: Some(BASE_EXTRUSION),
        selected_subshape: Some(top.clone()),
        rollback_preview: None,
    };

    let projection = project_feature_history(&snapshot, &registry, DEFINITION, &query).unwrap();
    let provenance = projection.selected_subshape.unwrap();
    assert_eq!(provenance.body_id, BodyId(1));
    assert_eq!(provenance.profile_feature_id, BASE_PROFILE);
    assert_eq!(provenance.producer_feature_id, BASE_EXTRUSION);
    assert_eq!(provenance.semantic_role, top.semantic_role);
    assert_eq!(provenance.source_element_id, top.source_element_id);
    assert_eq!(provenance.lineage_digest, top.lineage_digest);
    assert_eq!(provenance.result_fingerprint, top.result_fingerprint);
    assert_eq!(stamp(&document), before);

    assert_eq!(
        project_feature_history(
            &snapshot,
            &ExactResultRegistry::default(),
            DEFINITION,
            &query,
        ),
        Err(FeatureHistoryError::SubshapeLost)
    );
    assert_eq!(stamp(&document), before);
}
