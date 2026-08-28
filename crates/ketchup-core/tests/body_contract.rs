use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId,
    Dimension, DocumentStore, FeatureBodyOwnership, FeatureId, FeatureKind, MultiBodyBooleanPlan,
    NewBodyFeaturePlan, OccurrenceId, ProposalCommitError, ProposalContext, ProposalPrepareError,
    ToolBodyPolicy, Transform,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactResultRegistry,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PrincipalPlane, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind, SketchPointRef,
    SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
};
use ketchup_core::{persistence, state_view::encode_semantic_state};
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const OCCURRENCE: OccurrenceId = OccurrenceId(20);

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

fn rewrite_payload(bytes: &mut [u8], rewrite: impl FnOnce(&mut [u8])) {
    let manifest_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    rewrite(&mut bytes[payload_offset..]);
    let checksum = ketchup_core::graph::sha256_bytes(&bytes[payload_offset..]);
    bytes[24..56].copy_from_slice(&checksum);
}

fn seed() -> DocumentStore {
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
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
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Part occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document
}

#[test]
fn body_contract_is_reviewed_atomic_persistent_and_clone_stable() {
    let mut document = seed();
    let initial = document.current();
    let definition = initial.definition(DEFINITION).unwrap();
    assert_eq!(definition.active_body_id(), BodyId(1));
    assert_eq!(
        definition
            .bodies()
            .map(|body| body.id())
            .collect::<Vec<_>>(),
        vec![BodyId(1)]
    );
    assert_eq!(
        definition
            .feature_body_ownership(PROFILE)
            .unwrap()
            .output_body_id(),
        None
    );
    assert_eq!(
        definition
            .feature_body_ownership(EXTRUSION)
            .unwrap()
            .output_body_id(),
        Some(BodyId(1))
    );

    let create = document
        .plan_body_command(CanonicalCommand::CreateBody {
            definition_id: DEFINITION,
            id: BodyId(2),
            name: "Tool body".to_owned(),
            visible: true,
        })
        .unwrap();
    let before_create = document.current().canonical_digest();
    assert_eq!(document.current().canonical_digest(), before_create);
    let undo_before = document.visible_undo_steps();
    document.commit_proposal(&create).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    let created = document.current().canonical_digest();
    assert_ne!(created, before_create);
    assert_eq!(document.undo().unwrap().canonical_digest(), before_create);
    assert_eq!(document.redo().unwrap().canonical_digest(), created);

    let activate = document
        .plan_body_command(CanonicalCommand::SetActiveBody {
            definition_id: DEFINITION,
            id: BodyId(2),
        })
        .unwrap();
    document.commit_proposal(&activate).unwrap();
    let second_features = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(12),
                definition_id: DEFINITION,
                name: "Second profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(13),
                definition_id: DEFINITION,
                name: "Second extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(12),
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
        ]))
        .unwrap();
    document.commit_proposal(&second_features).unwrap();
    assert_eq!(
        document
            .current()
            .definition(DEFINITION)
            .unwrap()
            .feature_body_ownership(FeatureId(13))
            .unwrap()
            .output_body_id(),
        Some(BodyId(2))
    );
    let body_requests =
        ExactFeatureChainRequest::terminal_body_requests(&document.current(), DEFINITION).unwrap();
    assert_eq!(
        body_requests
            .iter()
            .map(|(body_id, request)| (*body_id, request.producer_feature_id()))
            .collect::<Vec<_>>(),
        vec![(BodyId(1), EXTRUSION), (BodyId(2), FeatureId(13))]
    );

    let stale = document
        .plan_body_command(CanonicalCommand::RenameBody {
            definition_id: DEFINITION,
            id: BodyId(2),
            name: "Stale rename".to_owned(),
        })
        .unwrap();
    let visibility = document
        .plan_body_command(CanonicalCommand::SetBodyVisibility {
            definition_id: DEFINITION,
            id: BodyId(2),
            visible: false,
        })
        .unwrap();
    document.commit_proposal(&visibility).unwrap();
    let stale_stamp = (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
    );
    assert!(matches!(
        document.commit_proposal(&stale),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
        ),
        stale_stamp
    );

    assert!(matches!(
        document.plan_body_command(CanonicalCommand::CreateBody {
            definition_id: DEFINITION,
            id: BodyId(2),
            name: "Duplicate".to_owned(),
            visible: true,
        }),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyAlreadyExists(DEFINITION, BodyId(2))
        ))
    ));
    let atomic_stamp = (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
    );
    assert!(matches!(
        document.prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(3),
                name: "Rolled back".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetFeatureBodyOwnership {
                id: EXTRUSION,
                ownership: FeatureBodyOwnership::new(vec![BodyId(3)], Some(BodyId(1))).unwrap(),
            },
        ])),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::InvalidBodyOwnership(EXTRUSION)
        ))
    ));
    assert_eq!(
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
        ),
        atomic_stamp
    );

    let saved = document.current();
    let saved_bytes = persistence::save(&saved);
    let reopened = persistence::load(&saved_bytes).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        saved.canonical_digest()
    );
    assert_eq!(persistence::save(&reopened.snapshot()), saved_bytes);
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        encode_semantic_state(&saved).complete_v1()
    );
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).agent_v1(),
        encode_semantic_state(&saved).agent_v1()
    );
    assert_eq!(
        reopened
            .snapshot()
            .definition(DEFINITION)
            .unwrap()
            .active_body_id(),
        BodyId(2)
    );
    assert!(
        !reopened
            .snapshot()
            .definition(DEFINITION)
            .unwrap()
            .body(BodyId(2))
            .unwrap()
            .visible()
    );

    let before_unique = stamp(&document);
    document.make_unique(OCCURRENCE, "Unique part").unwrap();
    assert_eq!(document.visible_undo_steps(), before_unique.2 + 1);
    assert_eq!(document.visible_redo_steps(), 0);
    let unique = document.current();
    let unique_digest = unique.canonical_digest();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let cloned = unique.definition(unique_definition).unwrap();
    assert_eq!(cloned.active_body_id(), BodyId(2));
    assert_eq!(
        cloned.bodies().map(|body| body.id()).collect::<Vec<_>>(),
        vec![BodyId(1), BodyId(2)]
    );
    assert!(!cloned.body(BodyId(2)).unwrap().visible());
    assert_eq!(
        cloned
            .feature_body_ownership(cloned.feature_ids()[3])
            .unwrap()
            .output_body_id(),
        Some(BodyId(2))
    );

    let views = encode_semantic_state(&unique);
    assert!(views.complete_v1().contains("active_body=2"));
    assert!(views.complete_v1().contains("output_body=2"));
    assert!(
        views
            .agent_v1()
            .contains("body_ownership=inputs:[],output:2")
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), before_unique.1);
    assert_eq!(document.visible_redo_steps(), 1);
    assert_eq!(document.redo().unwrap().canonical_digest(), unique_digest);
}

#[test]
fn invalid_body_mutations_preserve_revision_digest_and_history() {
    let mut document = seed();

    let before_duplicate = stamp(&document);
    assert!(matches!(
        document.plan_body_command(CanonicalCommand::CreateBody {
            definition_id: DEFINITION,
            id: BodyId(1),
            name: "Duplicate".to_owned(),
            visible: true,
        }),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyAlreadyExists(DEFINITION, BodyId(1))
        ))
    ));
    assert_eq!(stamp(&document), before_duplicate);

    let before_active_delete = stamp(&document);
    assert!(matches!(
        document.plan_body_command(CanonicalCommand::DeleteBody {
            definition_id: DEFINITION,
            id: BodyId(1),
        }),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyIsActive(DEFINITION, BodyId(1))
        ))
    ));
    assert_eq!(stamp(&document), before_active_delete);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateBody {
                definition_id: DefinitionId(2),
                id: BodyId(2),
                name: "Foreign body".to_owned(),
                visible: true,
            },
        ]))
        .unwrap();
    let before_cross_definition = stamp(&document);
    assert!(matches!(
        document.plan_body_command(CanonicalCommand::SetFeatureBodyOwnership {
            id: EXTRUSION,
            ownership: FeatureBodyOwnership::new(Vec::new(), Some(BodyId(2))).unwrap(),
        }),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyNotFound(DEFINITION, BodyId(2))
        ))
    ));
    assert_eq!(stamp(&document), before_cross_definition);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Second body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(12),
                definition_id: DEFINITION,
                name: "Second profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(13),
                definition_id: DEFINITION,
                name: "Second extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(12),
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(1),
            },
        ]))
        .unwrap();

    let before_in_use_delete = stamp(&document);
    assert!(matches!(
        document.plan_body_command(CanonicalCommand::DeleteBody {
            definition_id: DEFINITION,
            id: BodyId(2),
        }),
        Err(ProposalPrepareError::Canonical(CanonicalError::BodyInUse(
            DEFINITION,
            BodyId(2)
        )))
    ));
    assert_eq!(stamp(&document), before_in_use_delete);

    let before_cycle = stamp(&document);
    assert!(matches!(
        document.prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(14),
                definition_id: DEFINITION,
                name: "Second into first".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: EXTRUSION,
                    tool: FeatureId(13),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(15),
                definition_id: DEFINITION,
                name: "First into second".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: FeatureId(13),
                    tool: FeatureId(14),
                },
            },
        ])),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyDependencyCycle(DEFINITION)
        ))
    ));
    assert_eq!(stamp(&document), before_cycle);
}

#[test]
fn malformed_persisted_ownership_and_duplicate_ids_fail_closed() {
    let document = seed();
    let before = stamp(&document);

    let mut missing_body = persistence::save(&document.current());
    rewrite_payload(&mut missing_body, |payload| {
        let output_body = payload.len() - 20;
        payload[output_body..output_body + 8].copy_from_slice(&BodyId(99).0.to_le_bytes());
    });
    assert!(matches!(
        persistence::load(&missing_body),
        Err(persistence::PersistenceError::InvalidCanonicalData(
            CanonicalError::InvalidBodyOwnership(EXTRUSION)
        ))
    ));
    assert_eq!(stamp(&document), before);

    let mut duplicate_ownership = persistence::save(&document.current());
    rewrite_payload(&mut duplicate_ownership, |payload| {
        let second_feature_id = payload.len() - 33;
        payload[second_feature_id..second_feature_id + 8].copy_from_slice(&PROFILE.0.to_le_bytes());
    });
    assert!(matches!(
        persistence::load(&duplicate_ownership),
        Err(persistence::PersistenceError::InvalidCanonicalData(
            CanonicalError::InvalidBodyOwnership(PROFILE)
        ))
    ));
    assert_eq!(stamp(&document), before);
}

#[test]
fn body_order_is_deterministic_across_equivalent_batches() {
    let mut first = seed();
    let mut second = persistence::load(&persistence::save(&first.current()))
        .unwrap()
        .into_editable()
        .unwrap_or_else(|_| panic!("schema-34 round-trip must stay editable"));
    let body_two = CanonicalCommand::CreateBody {
        definition_id: DEFINITION,
        id: BodyId(2),
        name: "Two".to_owned(),
        visible: false,
    };
    let body_three = CanonicalCommand::CreateBody {
        definition_id: DEFINITION,
        id: BodyId(3),
        name: "Three".to_owned(),
        visible: true,
    };
    first
        .apply_batch(&CommandBatch::new(vec![
            body_three.clone(),
            body_two.clone(),
        ]))
        .unwrap();
    second
        .apply_batch(&CommandBatch::new(vec![body_two, body_three]))
        .unwrap();

    assert_eq!(
        first.current().canonical_digest(),
        second.current().canonical_digest()
    );
    assert_eq!(
        persistence::save(&first.current()),
        persistence::save(&second.current())
    );
    assert_eq!(
        encode_semantic_state(&first.current()).complete_v1(),
        encode_semantic_state(&second.current()).complete_v1()
    );
    assert_eq!(
        encode_semantic_state(&first.current()).agent_v1(),
        encode_semantic_state(&second.current()).agent_v1()
    );
}

#[test]
fn ambiguous_and_lost_references_reject_ownership_without_history_changes() {
    let mut document = seed();
    let document_id = document.current().document_id();
    let request = ExactFeatureChainRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
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
        [[0.0, 0.0, 0.0], [20.0, 10.0, 5.0]],
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    document
        .register_exact_reference_evidence(top.clone())
        .unwrap();
    let face_plane = FeatureId(12);
    let tool_sketch_id = FeatureId(13);
    let tool_pad = FeatureId(14);
    let tool_sketch = SketchSpec {
        workplane: face_plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [5.0, 5.0],
            radius_mm: 2.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("2").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [5.0, 5.0],
                },
            },
        ],
    };
    let tool_region = tool_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: face_plane,
                definition_id: DEFINITION,
                name: "Top face".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(5.0),
                }),
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Referenced tool body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: tool_sketch_id,
                definition_id: DEFINITION,
                name: "Referenced tool sketch".to_owned(),
                kind: FeatureKind::Sketch(tool_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: tool_pad,
                definition_id: DEFINITION,
                name: "Referenced tool pad".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: tool_sketch_id,
                    region: tool_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("2").unwrap()),
                }),
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(1),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("6").unwrap(),
            },
        ]))
        .unwrap();

    let changed = document.current();
    let changed_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&changed, DEFINITION, EXTRUSION)
            .unwrap();
    let changed_evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                document_id,
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:6"),
        )
    });
    let changed_package = build_box_render_package(
        &changed_request,
        "exact-input-6".to_owned(),
        "result-6".to_owned(),
        "occt".to_owned(),
        "r0".to_owned(),
        [[0.0, 0.0, 0.0], [20.0, 10.0, 6.0]],
        changed_evidence,
    )
    .unwrap();
    let mut incompatible = changed_package.clone();
    incompatible.identity.backend.push_str("-alternate");
    for reference in &mut incompatible.references {
        reference.backend = incompatible.identity.backend.clone();
    }
    let ambiguous = ExactResultRegistry::accept(
        &changed,
        [
            Arc::new(ExactBodyPackage::from(changed_package)),
            Arc::new(ExactBodyPackage::from(incompatible)),
        ],
    )
    .unwrap();
    document
        .register_exact_reference_evidence(&ambiguous)
        .unwrap();
    let ownership = document
        .current()
        .definition(DEFINITION)
        .unwrap()
        .feature_body_ownership(face_plane)
        .unwrap()
        .clone();
    let before_ambiguous = stamp(&document);
    assert!(matches!(
        document.plan_body_command(CanonicalCommand::SetFeatureBodyOwnership {
            id: face_plane,
            ownership: ownership.clone(),
        }),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::UnresolvedBodyOwnershipReference(id)
        )) if id == face_plane
    ));
    assert!(matches!(
        document.plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Union,
                target_body_id: BodyId(1),
                target_feature_id: EXTRUSION,
                tool_body_id: BodyId(2),
                tool_feature_id: tool_pad,
                result_feature_id: FeatureId(15),
                result_feature_name: "Unresolved union".to_owned(),
                tool_policy: ToolBodyPolicy::Preserve,
            },
            ProposalContext::canonical_preview(),
        ),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::InvalidBodyAuthoringPlan
        ))
    ));
    assert_eq!(stamp(&document), before_ambiguous);

    document
        .register_exact_reference_evidence(&ExactResultRegistry::default())
        .unwrap();
    let before_lost = stamp(&document);
    assert!(matches!(
        document.plan_body_command(CanonicalCommand::SetFeatureBodyOwnership {
            id: face_plane,
            ownership,
        }),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::UnresolvedBodyOwnershipReference(id)
        )) if id == face_plane
    ));
    assert!(matches!(
        document.plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Intersect,
                target_body_id: BodyId(1),
                target_feature_id: EXTRUSION,
                tool_body_id: BodyId(2),
                tool_feature_id: tool_pad,
                result_feature_id: FeatureId(15),
                result_feature_name: "Lost intersect".to_owned(),
                tool_policy: ToolBodyPolicy::Consume,
            },
            ProposalContext::canonical_preview(),
        ),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::InvalidBodyAuthoringPlan
        ))
    ));
    assert_eq!(stamp(&document), before_lost);
}

#[test]
fn reviewed_multibody_authoring_is_one_undo_and_preserves_stable_lineage() {
    let mut document = seed();
    let second_profile = document
        .prepare_proposal(CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(12),
            definition_id: DEFINITION,
            name: "Second profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 10.0], [0.0, 10.0]],
            },
        }]))
        .unwrap();
    document.commit_proposal(&second_profile).unwrap();
    document.discard_history_before_current();

    let before_body = stamp(&document);
    let new_body = document
        .plan_new_body_feature(
            NewBodyFeaturePlan {
                definition_id: DEFINITION,
                body_id: BodyId(2),
                body_name: "Tool body".to_owned(),
                feature_id: FeatureId(13),
                feature_name: "Tool extrusion".to_owned(),
                feature_kind: FeatureKind::Extrusion {
                    profile: FeatureId(12),
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert_eq!(new_body.batch().commands().len(), 3);
    assert_eq!(stamp(&document), before_body);
    document.commit_proposal(&new_body).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    let body_snapshot = document.current();
    let body_digest = body_snapshot.canonical_digest();
    let definition = body_snapshot.definition(DEFINITION).unwrap();
    assert_eq!(definition.active_body_id(), BodyId(2));
    assert_eq!(
        definition
            .feature_body_ownership(FeatureId(13))
            .unwrap()
            .output_body_id(),
        Some(BodyId(2))
    );
    assert_eq!(
        ExactFeatureChainRequest::terminal_body_requests(&document.current(), DEFINITION)
            .unwrap()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![BodyId(1), BodyId(2)]
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), before_body.1);
    assert_eq!(document.redo().unwrap().canonical_digest(), body_digest);

    let before_combine = stamp(&document);
    assert!(matches!(
        document.plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Union,
                target_body_id: BodyId(1),
                target_feature_id: EXTRUSION,
                tool_body_id: BodyId(1),
                tool_feature_id: EXTRUSION,
                result_feature_id: FeatureId(14),
                result_feature_name: "Self union".to_owned(),
                tool_policy: ToolBodyPolicy::Consume,
            },
            ProposalContext::canonical_preview(),
        ),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::InvalidBodyAuthoringPlan
        ))
    ));
    assert!(matches!(
        document.plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Cut,
                target_body_id: BodyId(99),
                target_feature_id: EXTRUSION,
                tool_body_id: BodyId(2),
                tool_feature_id: FeatureId(13),
                result_feature_id: FeatureId(14),
                result_feature_name: "Missing target".to_owned(),
                tool_policy: ToolBodyPolicy::Preserve,
            },
            ProposalContext::canonical_preview(),
        ),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyNotFound(DEFINITION, BodyId(99))
        ))
    ));
    assert_eq!(stamp(&document), before_combine);
    let consume = document
        .plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Union,
                target_body_id: BodyId(1),
                target_feature_id: EXTRUSION,
                tool_body_id: BodyId(2),
                tool_feature_id: FeatureId(13),
                result_feature_id: FeatureId(14),
                result_feature_name: "Body union".to_owned(),
                tool_policy: ToolBodyPolicy::Consume,
            },
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert_eq!(consume.batch().commands().len(), 3);
    assert_eq!(stamp(&document), before_combine);
    let consume_revision = document.commit_proposal(&consume).unwrap();
    assert_eq!(
        consume_revision
            .dirty_features()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![FeatureId(14)]
    );
    assert_eq!(document.visible_undo_steps(), before_combine.2 + 1);
    let consumed_digest = document.current().canonical_digest();
    let consumed = document.current();
    let definition = consumed.definition(DEFINITION).unwrap();
    assert_eq!(definition.active_body_id(), BodyId(1));
    assert!(definition.body(BodyId(2)).unwrap().visible());
    assert_eq!(
        definition.body(BodyId(2)).unwrap().consumed_by(),
        Some(FeatureId(14))
    );
    let ownership = definition.feature_body_ownership(FeatureId(14)).unwrap();
    assert_eq!(ownership.input_body_ids(), &[BodyId(1), BodyId(2)]);
    assert_eq!(ownership.output_body_id(), Some(BodyId(1)));
    assert!(matches!(
        consumed.feature(FeatureId(14)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Union,
            target: EXTRUSION,
            tool: FeatureId(13),
        }
    ));
    let requests = ExactFeatureChainRequest::terminal_body_requests(&consumed, DEFINITION).unwrap();
    assert_eq!(requests[&BodyId(1)].producer_feature_id(), FeatureId(14));
    assert!(!requests.contains_key(&BodyId(2)));
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        before_combine.1
    );
    assert_eq!(document.redo().unwrap().canonical_digest(), consumed_digest);
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), consumed_digest);
    assert_eq!(
        reopened
            .snapshot()
            .definition(DEFINITION)
            .unwrap()
            .body(BodyId(2))
            .unwrap()
            .consumed_by(),
        Some(FeatureId(14))
    );

    document.undo().unwrap();
    let preserve = document
        .plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Cut,
                target_body_id: BodyId(1),
                target_feature_id: EXTRUSION,
                tool_body_id: BodyId(2),
                tool_feature_id: FeatureId(13),
                result_feature_id: FeatureId(15),
                result_feature_name: "Body cut".to_owned(),
                tool_policy: ToolBodyPolicy::Preserve,
            },
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert_eq!(preserve.batch().commands().len(), 2);
    document.commit_proposal(&preserve).unwrap();
    assert!(
        document
            .current()
            .definition(DEFINITION)
            .unwrap()
            .body(BodyId(2))
            .unwrap()
            .visible()
    );
    assert_eq!(document.visible_redo_steps(), 0);
}
