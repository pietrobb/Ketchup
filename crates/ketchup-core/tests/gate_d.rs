use ketchup_core::document::{
    AuthenticatedApprover, AuthoritativeDependency, BOTTLE_SHELL_OPENING_FACE_ROLE,
    BOTTLE_SHOULDER_EDGE_ROLE, BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand,
    CollectionId, CommandBatch, DefinitionId, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DocumentStore, EvaluatorNodeKind, FeatureId, FeatureKind,
    FeatureParameterSlot, FeatureParameterTarget, GroupId, HighRiskClass, HighRiskScope,
    HumanConfirmationError, MAX_HUMAN_CONFIRMATION_LIFETIME_MS, NodeId, OccurrenceId,
    OverrideParameterSpec, PersistentDimension, PersistentDimensionId, PersistentDimensionTarget,
    PortSpec, Proposal, ProposalBudget, ProposalCommitError, ProposalConfirmation, ProposalContext,
    ProposalGoal, ProposalPrepareError, ProposalPrincipal, ProposalRisk, ProposalValue, RuleOutput,
    SlotPath, SlotResolution, SlotSegment, StableEdgeRole, StableFaceRole, TagId, Transform,
    TrustedConfirmationSurface,
};
use ketchup_core::intent::{
    IntentCapability, IntentError, IntentGrant, IntentRequest, RequestingPrincipal, WorkflowIntent,
    propose_intent,
};
use ketchup_core::prismatic::{Aabb, CanonicalJoint, JointId, TolerancePolicy};
use ketchup_core::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceCoordinateFrame, ClearanceOwner,
    ClearanceSeverity, ClearanceVolumeId, SpaceId,
};

const RULE: NodeId = NodeId(1);
const EXPRESSION_INPUT: NodeId = NodeId(2);
const EXPRESSION: NodeId = NodeId(3);
const RULE_OUTPUTS: NodeId = NodeId(4);
const DEFINITION: DefinitionId = DefinitionId(10);
const SECOND_DEFINITION: DefinitionId = DefinitionId(15);
const BOTTLE_DEFINITION: DefinitionId = DefinitionId(18);
const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);
const BOTTLE_PROFILE: FeatureId = FeatureId(19);
const BOTTLE_CONTROL: FeatureId = FeatureId(20);
const BOTTLE_REVOLVE: FeatureId = FeatureId(21);
const BOTTLE_SHELL: FeatureId = FeatureId(22);
const BOTTLE_FINISH: FeatureId = FeatureId(23);
const OCCURRENCE: OccurrenceId = OccurrenceId(13);
const TAG: TagId = TagId(14);
const GROUP: GroupId = GroupId(16);
const COLLECTION: CollectionId = CollectionId(17);

fn dimension(token: &str, value: f64) -> Dimension {
    Dimension::new(token, value).unwrap()
}

fn rule_output(key: &str) -> RuleOutput {
    RuleOutput::new(
        SlotSegment::new(RULE_OUTPUTS, "result", key).unwrap(),
        Vec::new(),
    )
    .unwrap()
}

fn seed() -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: RULE,
                name: "width".to_owned(),
                dimension: dimension("600", 600.0),
                dependencies: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: EXPRESSION_INPUT,
                name: "allowance".to_owned(),
                dimension: dimension("25", 25.0),
                dependencies: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: EXPRESSION,
                name: "derived width".to_owned(),
                expression: "$1 * 2".to_owned(),
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE_OUTPUTS,
                name: "layout".to_owned(),
                expression: "$3".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateDefinition {
                id: SECOND_DEFINITION,
                name: "Housing".to_owned(),
            },
            CanonicalCommand::CreateDefinition {
                id: BOTTLE_DEFINITION,
                name: "Bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: dimension("20", 20.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_PROFILE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [30.0, 0.0],
                        [30.0, 110.0],
                        [12.0, 130.0],
                        [12.0, 155.0],
                        [0.0, 155.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_CONTROL,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle controls".to_owned(),
                kind: FeatureKind::BottleProfileControl {
                    profile: BOTTLE_PROFILE,
                    body_radius: dimension("30", 30.0),
                    body_height: dimension("110", 110.0),
                    shoulder_rise: dimension("20", 20.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_REVOLVE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::full_revolve(BOTTLE_CONTROL),
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_SHELL,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: BOTTLE_REVOLVE,
                    removed_faces: vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE).unwrap(),
                    ],
                    thickness: dimension("2", 2.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_FINISH,
                definition_id: BOTTLE_DEFINITION,
                name: "Edge finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: BOTTLE_SHELL,
                    edges: vec![StableEdgeRole::new(BOTTLE_SHOULDER_EDGE_ROLE).unwrap()],
                    kind: BottleEdgeFinishKind::Fillet,
                    amount: dimension("2", 2.0),
                },
            },
            CanonicalCommand::CreateTag {
                id: TAG,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateGroup {
                id: GROUP,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Selection set".to_owned(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Box occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    store
}

fn high_risk_proposal(
    store: &DocumentStore,
    principal: ProposalPrincipal,
    value_text: &str,
    scope: HighRiskScope,
) -> Proposal {
    store
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal(value_text).unwrap(),
            }]),
            ProposalContext {
                principal,
                goal: ProposalGoal::SetFeatureDimension(EXTRUSION),
                assumptions: vec![],
                risk: ProposalRisk::High(scope.class()),
                confirmation: ProposalConfirmation::HumanOnly(scope),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        )
        .unwrap()
}

#[test]
fn gate_d_rule_intent_exposes_authoritative_review_and_commits_one_verified_batch() {
    let mut store = seed();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetRuleDimension {
            target: RULE,
            value_text: "720.125".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.principal(), ProposalPrincipal::LocalAssistant);
    assert_eq!(proposal.goal(), ProposalGoal::SetRuleDimension(RULE));
    assert_eq!(proposal.risk(), ProposalRisk::Standard);
    assert_eq!(
        proposal.confirmation(),
        &ProposalConfirmation::ReviewRequired
    );
    assert_eq!(proposal.cost().commands, 1);
    assert_eq!(proposal.cost().write_targets, 1);
    assert!(proposal.cost().read_dependencies <= 64);
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::EvaluatorNode(RULE)])
    );
    assert_eq!(proposal.authoritative_diff().len(), 1);
    assert!(matches!(
        &proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(value) if value.millimetres() == 600.0
    ));
    assert!(matches!(
        &proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(value) if value.millimetres() == 720.125
    ));

    let committed = store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(committed.command_digest(), proposal.command_digest());
    assert_eq!(committed.result_digest(), proposal.intended_result_digest());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        store
            .current()
            .evaluator_node(RULE)
            .unwrap()
            .dimension()
            .unwrap()
            .millimetres(),
        720.125
    );
}

#[test]
fn gate_d_evaluator_rename_is_observational_and_commits_exact_text_once() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RenameEvaluatorNode {
            target: RULE,
            name: "cabinet width".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::RenameEvaluatorNode(RULE));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::EvaluatorNode(RULE)
        )]
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("width".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("cabinet width".to_owned())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().evaluator_node(RULE).unwrap().name(),
        "cabinet width"
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store.current().evaluator_node(RULE).unwrap().name(),
        "width"
    );
}

#[test]
fn gate_d_evaluator_rename_rejects_denied_empty_missing_and_stale_targets() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetRuleDimension],
                ),
                intent: WorkflowIntent::RenameEvaluatorNode {
                    target: RULE,
                    name: "Denied".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::RenameEvaluatorNode
        ))
    );
    for intent in [
        WorkflowIntent::RenameEvaluatorNode {
            target: RULE,
            name: "".to_owned(),
        },
        WorkflowIntent::RenameEvaluatorNode {
            target: NodeId(999),
            name: "Missing".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RenameEvaluatorNode {
            target: RULE,
            name: "Proposed".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameEvaluatorNode {
                id: RULE,
                name: "Concurrent".to_owned(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().evaluator_node(RULE).unwrap().name(),
        "Concurrent"
    );
}

#[test]
fn gate_d_evaluator_expression_is_observational_and_commits_exact_text_once() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetEvaluatorExpression {
            target: EXPRESSION,
            expression: "$2 + 5".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetEvaluatorExpression(EXPRESSION)
    );
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(EXPRESSION),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(EXPRESSION_INPUT),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_dependencies(),
        &std::collections::BTreeSet::from([
            AuthoritativeDependency::EvaluatorNode(RULE),
            AuthoritativeDependency::EvaluatorNode(EXPRESSION_INPUT),
            AuthoritativeDependency::EvaluatorNode(EXPRESSION),
        ])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("$1 * 2".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("$2 + 5".to_owned())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store
            .current()
            .evaluator_node(EXPRESSION)
            .unwrap()
            .kind()
            .source(),
        "$2 + 5"
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store
            .current()
            .evaluator_node(EXPRESSION)
            .unwrap()
            .kind()
            .source(),
        "$1 * 2"
    );
}

#[test]
fn gate_d_evaluator_expression_rejects_denied_invalid_wrong_kind_missing_and_stale() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::RenameEvaluatorNode],
                ),
                intent: WorkflowIntent::SetEvaluatorExpression {
                    target: EXPRESSION,
                    expression: "$2 + 5".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetEvaluatorExpression
        ))
    );
    for intent in [
        WorkflowIntent::SetEvaluatorExpression {
            target: EXPRESSION,
            expression: "(".to_owned(),
        },
        WorkflowIntent::SetEvaluatorExpression {
            target: RULE,
            expression: "1".to_owned(),
        },
        WorkflowIntent::SetEvaluatorExpression {
            target: NodeId(999),
            expression: "1".to_owned(),
        },
        WorkflowIntent::SetEvaluatorExpression {
            target: EXPRESSION,
            expression: "$999".to_owned(),
        },
        WorkflowIntent::SetEvaluatorExpression {
            target: EXPRESSION,
            expression: "$3".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetEvaluatorExpression {
            target: EXPRESSION,
            expression: "$2 + 5".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: EXPRESSION_INPUT,
                dimension: dimension("30", 30.0),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store
            .current()
            .evaluator_node(EXPRESSION)
            .unwrap()
            .kind()
            .source(),
        "$1 * 2"
    );
}

#[test]
fn gate_d_bottle_control_dimension_is_observational_typed_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetBottleControlDimension {
            target: BOTTLE_CONTROL,
            control: BottleControlDimension::BodyRadius,
            value_text: "32".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetBottleControlDimension(BOTTLE_CONTROL, BottleControlDimension::BodyRadius)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(BOTTLE_CONTROL)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Feature(BOTTLE_CONTROL)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(dimension("30", 30.0))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(dimension("32", 32.0))
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.current().feature(BOTTLE_CONTROL).unwrap().kind(),
        FeatureKind::BottleProfileControl { body_radius, .. }
            if body_radius.millimetres() == 32.0
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(matches!(
        store.current().feature(BOTTLE_CONTROL).unwrap().kind(),
        FeatureKind::BottleProfileControl { body_radius, .. }
            if body_radius.millimetres() == 30.0
    ));
}

#[test]
fn gate_d_bottle_control_dimension_rejects_denied_invalid_wrong_kind_missing_and_stale() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetFeatureDimension],
                ),
                intent: WorkflowIntent::SetBottleControlDimension {
                    target: BOTTLE_CONTROL,
                    control: BottleControlDimension::BodyHeight,
                    value_text: "120".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetBottleControlDimension
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetBottleControlDimension {
                target: BOTTLE_CONTROL,
                control: BottleControlDimension::BodyHeight,
                value_text: "not-a-number".to_owned(),
            })
        ),
        Err(IntentError::Canonical(_))
    ));
    for target in [EXTRUSION, FeatureId(999)] {
        assert!(matches!(
            propose_intent(
                &store,
                IntentRequest::m7a(WorkflowIntent::SetBottleControlDimension {
                    target,
                    control: BottleControlDimension::BodyHeight,
                    value_text: "120".to_owned(),
                })
            ),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetBottleControlDimension {
            target: BOTTLE_CONTROL,
            control: BottleControlDimension::BodyHeight,
            value_text: "120".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBottleControlDimension {
                id: BOTTLE_CONTROL,
                control: BottleControlDimension::ShoulderRise,
                dimension: dimension("21", 21.0),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(matches!(
        store.current().feature(BOTTLE_CONTROL).unwrap().kind(),
        FeatureKind::BottleProfileControl {
            body_height,
            shoulder_rise,
            ..
        } if body_height.millimetres() == 110.0 && shoulder_rise.millimetres() == 21.0
    ));
}

#[test]
fn gate_d_bottle_finish_kind_is_observational_typed_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetBottleEdgeFinishKind {
            target: BOTTLE_FINISH,
            kind: BottleEdgeFinishKind::Chamfer,
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetBottleEdgeFinishKind(BOTTLE_FINISH)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(BOTTLE_FINISH)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Feature(BOTTLE_FINISH)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Fillet)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Chamfer)
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.current().feature(BOTTLE_FINISH).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Chamfer,
            ..
        }
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(matches!(
        store.current().feature(BOTTLE_FINISH).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Fillet,
            ..
        }
    ));
}

#[test]
fn gate_d_bottle_finish_kind_rejects_denied_wrong_kind_missing_and_stale() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetFeatureDimension],
                ),
                intent: WorkflowIntent::SetBottleEdgeFinishKind {
                    target: BOTTLE_FINISH,
                    kind: BottleEdgeFinishKind::Chamfer,
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetBottleEdgeFinishKind
        ))
    );
    for target in [EXTRUSION, FeatureId(999)] {
        assert!(matches!(
            propose_intent(
                &store,
                IntentRequest::m7a(WorkflowIntent::SetBottleEdgeFinishKind {
                    target,
                    kind: BottleEdgeFinishKind::Chamfer,
                })
            ),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetBottleEdgeFinishKind {
            target: BOTTLE_FINISH,
            kind: BottleEdgeFinishKind::Chamfer,
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: BOTTLE_FINISH,
                dimension: dimension("1.5", 1.5),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(matches!(
        store.current().feature(BOTTLE_FINISH).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Fillet,
            amount,
            ..
        } if amount.millimetres() == 1.5
    ));
}

#[test]
fn gate_d_profile_points_are_observational_typed_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let requested = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetProfilePoints {
            target: PROFILE,
            points_mm: requested.clone(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetProfilePoints(PROFILE));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(PROFILE)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Feature(PROFILE)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ProfilePoints(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ProfilePoints(requested.clone())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.current().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &requested
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(matches!(
        store.current().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm }
            if points_mm == &vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]
    ));
}

#[test]
fn gate_d_profile_points_reject_denied_invalid_wrong_kind_missing_and_stale() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    let requested = vec![[0.0, 0.0], [12.0, 0.0], [0.0, 8.0]];
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetFeatureDimension],
                ),
                intent: WorkflowIntent::SetProfilePoints {
                    target: PROFILE,
                    points_mm: requested.clone(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetProfilePoints
        ))
    );
    for (target, points_mm) in [
        (PROFILE, vec![[0.0, 0.0], [1.0, 0.0]]),
        (EXTRUSION, requested.clone()),
        (FeatureId(999), requested.clone()),
    ] {
        assert!(matches!(
            propose_intent(
                &store,
                IntentRequest::m7a(WorkflowIntent::SetProfilePoints { target, points_mm })
            ),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetProfilePoints {
            target: PROFILE,
            points_mm: requested,
        }),
    )
    .unwrap();
    let concurrent = vec![[0.0, 0.0], [11.0, 0.0], [0.0, 9.0]];
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: concurrent.clone(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(matches!(
        store.current().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &concurrent
    ));
}

#[test]
fn gate_d_feature_intent_uses_the_same_safe_path_and_is_not_replayable() {
    let mut store = seed();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "35".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetFeatureDimension(EXTRUSION)
    );
    assert_eq!(proposal.authoritative_diff().len(), 1);
    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    let snapshot = store.current();
    let FeatureKind::Extrusion { height, .. } = snapshot.feature(EXTRUSION).unwrap().kind() else {
        panic!("fixture extrusion changed kind");
    };
    assert_eq!(height.millimetres(), 35.0);
}

#[test]
fn gate_d_definition_rename_is_observational_and_commits_exact_text_once() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RenameDefinition {
            target: DEFINITION,
            name: "Housing".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::RenameDefinition(DEFINITION));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Definition(DEFINITION)
        )]
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("Box".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Housing".to_owned())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().definition(DEFINITION).unwrap().name(),
        "Housing"
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store.current().definition(DEFINITION).unwrap().name(),
        "Box"
    );
}

#[test]
fn gate_d_occurrence_visibility_intent_is_observational_then_commits_one_verified_batch() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceVisibility {
            target: OCCURRENCE,
            visible: false,
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceVisibility(OCCURRENCE)
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Occurrence(OCCURRENCE)])
    );
    assert_eq!(proposal.authoritative_diff().len(), 1);
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Occurrence(OCCURRENCE)
        )]
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Boolean(true)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Boolean(false)
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(!store.current().occurrence(OCCURRENCE).unwrap().visible());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
}

#[test]
fn gate_d_tag_visibility_is_observational_and_commits_one_verified_batch() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetTagVisibility {
            target: TAG,
            visible: false,
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetTagVisibility(TAG));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Tag(TAG)
        )]
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Boolean(true)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Boolean(false)
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(!store.current().tag(TAG).unwrap().visible());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().tag(TAG).unwrap().visible());
}

#[test]
fn gate_d_occurrence_tag_is_observational_and_commits_one_verified_batch() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceTag {
            target: OCCURRENCE,
            tag: Some(TAG),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetOccurrenceTag(OCCURRENCE));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Occurrence(OCCURRENCE),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(AuthoritativeDependency::Tag(
                TAG
            ),),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Occurrence(OCCURRENCE)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Tag(TAG))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Tag(None)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Tag(Some(TAG))
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().tag(),
        Some(TAG)
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(store.current().occurrence(OCCURRENCE).unwrap().tag(), None);
}

#[test]
fn gate_d_occurrence_repoint_is_observational_and_commits_one_verified_batch() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RepointOccurrence {
            target: OCCURRENCE,
            definition: SECOND_DEFINITION,
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::RepointOccurrence(OCCURRENCE));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Occurrence(OCCURRENCE),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Definition(SECOND_DEFINITION),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Occurrence(OCCURRENCE)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Definition(SECOND_DEFINITION))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Definition(DEFINITION)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Definition(SECOND_DEFINITION)
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store
            .current()
            .occurrence(OCCURRENCE)
            .unwrap()
            .definition_id(),
        SECOND_DEFINITION
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store
            .current()
            .occurrence(OCCURRENCE)
            .unwrap()
            .definition_id(),
        DEFINITION
    );
}

#[test]
fn gate_d_occurrence_repoint_rejects_denied_missing_and_stale_dependencies() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetOccurrenceTag],
                ),
                intent: WorkflowIntent::RepointOccurrence {
                    target: OCCURRENCE,
                    definition: SECOND_DEFINITION,
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::RepointOccurrence
        ))
    );
    for intent in [
        WorkflowIntent::RepointOccurrence {
            target: OccurrenceId(999),
            definition: SECOND_DEFINITION,
        },
        WorkflowIntent::RepointOccurrence {
            target: OCCURRENCE,
            definition: DefinitionId(999),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RepointOccurrence {
            target: OCCURRENCE,
            definition: SECOND_DEFINITION,
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: SECOND_DEFINITION,
                name: "Concurrent housing".to_owned(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store
            .current()
            .occurrence(OCCURRENCE)
            .unwrap()
            .definition_id(),
        DEFINITION
    );
}

#[test]
fn gate_d_definition_rename_rejects_empty_missing_and_stale_targets() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetFeatureDimension],
                ),
                intent: WorkflowIntent::RenameDefinition {
                    target: DEFINITION,
                    name: "Denied".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::RenameDefinition
        ))
    );
    for intent in [
        WorkflowIntent::RenameDefinition {
            target: DEFINITION,
            name: "".to_owned(),
        },
        WorkflowIntent::RenameDefinition {
            target: DefinitionId(999),
            name: "Missing".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RenameDefinition {
            target: DEFINITION,
            name: "Proposed".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Concurrent".to_owned(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().definition(DEFINITION).unwrap().name(),
        "Concurrent"
    );
}

#[test]
fn gate_d_tag_visibility_rejects_denied_missing_and_stale_targets() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetOccurrenceVisibility],
                ),
                intent: WorkflowIntent::SetTagVisibility {
                    target: TAG,
                    visible: false,
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetTagVisibility
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetTagVisibility {
                target: TagId(999),
                visible: false,
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetTagVisibility {
            target: TAG,
            visible: false,
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: TAG,
                visible: false,
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    let changed_revision = store.current().revision_id();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().revision_id(), changed_revision);
    assert!(!store.current().tag(TAG).unwrap().visible());
}

#[test]
fn gate_d_occurrence_tag_rejects_denied_missing_and_stale_targets() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetTagVisibility],
                ),
                intent: WorkflowIntent::SetOccurrenceTag {
                    target: OCCURRENCE,
                    tag: Some(TAG),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetOccurrenceTag
        ))
    );
    for intent in [
        WorkflowIntent::SetOccurrenceTag {
            target: OccurrenceId(999),
            tag: Some(TAG),
        },
        WorkflowIntent::SetOccurrenceTag {
            target: OCCURRENCE,
            tag: Some(TagId(999)),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceTag {
            target: OCCURRENCE,
            tag: Some(TAG),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: TAG,
                visible: false,
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().occurrence(OCCURRENCE).unwrap().tag(), None);
}

#[test]
fn gate_d_occurrence_visibility_intent_rejects_missing_or_stale_targets() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(999),
                visible: false,
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceVisibility {
            target: OCCURRENCE,
            visible: false,
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OCCURRENCE,
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    let changed_revision = store.current().revision_id();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().revision_id(), changed_revision);
    assert!(store.current().occurrence(OCCURRENCE).unwrap().visible());
}

#[test]
fn gate_d_occurrence_parent_is_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceParent {
            target: OCCURRENCE,
            parent: Some(GROUP),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceParent(OCCURRENCE)
    );
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Occurrence(OCCURRENCE),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Group(GROUP),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Occurrence(OCCURRENCE)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Group(GROUP))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Group(None)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Group(Some(GROUP))
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().parent(),
        Some(GROUP)
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().parent(),
        None
    );

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceParent {
                id: OCCURRENCE,
                parent: Some(GROUP),
            },
        ]))
        .unwrap();
    let removal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceParent {
            target: OCCURRENCE,
            parent: None,
        }),
    )
    .unwrap();
    assert_eq!(
        removal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Occurrence(OCCURRENCE),
        )]
    );
    assert!(
        !removal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Group(GROUP))
    );
    assert_eq!(
        removal.authoritative_diff()[0].before,
        ProposalValue::Group(Some(GROUP))
    );
    assert_eq!(
        removal.authoritative_diff()[0].after,
        ProposalValue::Group(None)
    );
    store.commit_verified_proposal(&removal).unwrap();
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().parent(),
        None
    );
}

#[test]
fn gate_d_occurrence_parent_rejects_denied_missing_and_stale_dependencies() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::RepointOccurrence],
                ),
                intent: WorkflowIntent::SetOccurrenceParent {
                    target: OCCURRENCE,
                    parent: Some(GROUP),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetOccurrenceParent
        ))
    );
    for intent in [
        WorkflowIntent::SetOccurrenceParent {
            target: OccurrenceId(999),
            parent: Some(GROUP),
        },
        WorkflowIntent::SetOccurrenceParent {
            target: OCCURRENCE,
            parent: Some(GroupId(999)),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceParent {
            target: OCCURRENCE,
            parent: Some(GROUP),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: GROUP,
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().parent(),
        None
    );
}

#[test]
fn gate_d_group_parent_is_typed_observational_and_undoable() {
    let mut store = seed();
    let parent = GroupId(17);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: parent,
            name: "Parent".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetGroupParent {
            target: GROUP,
            parent: Some(parent),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetGroupParent(GROUP));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Group(GROUP),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Group(parent),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Group(GROUP)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Group(parent))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Group(None)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Group(Some(parent))
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(store.current().group(GROUP).unwrap().parent(), Some(parent));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(store.current().group(GROUP).unwrap().parent(), None);

    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetGroupParent {
            id: GROUP,
            parent: Some(parent),
        }]))
        .unwrap();
    let removal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetGroupParent {
            target: GROUP,
            parent: None,
        }),
    )
    .unwrap();
    assert_eq!(
        removal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Group(GROUP),
        )]
    );
    assert!(
        !removal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Group(parent))
    );
    assert_eq!(
        removal.authoritative_diff()[0].before,
        ProposalValue::Group(Some(parent))
    );
    assert_eq!(
        removal.authoritative_diff()[0].after,
        ProposalValue::Group(None)
    );
    store.commit_verified_proposal(&removal).unwrap();
    assert_eq!(store.current().group(GROUP).unwrap().parent(), None);
}

#[test]
fn gate_d_group_parent_rejects_denied_missing_cycle_and_stale_ancestry() {
    let mut store = seed();
    let parent = GroupId(17);
    let ancestor = GroupId(18);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: ancestor,
                name: "Ancestor".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: parent,
                name: "Parent".to_owned(),
                transform: Transform::identity(),
                parent: Some(ancestor),
            },
        ]))
        .unwrap();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetGroupTranslation],
                ),
                intent: WorkflowIntent::SetGroupParent {
                    target: GROUP,
                    parent: Some(parent),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetGroupParent
        ))
    );
    for intent in [
        WorkflowIntent::SetGroupParent {
            target: GroupId(999),
            parent: Some(parent),
        },
        WorkflowIntent::SetGroupParent {
            target: GROUP,
            parent: Some(GroupId(999)),
        },
        WorkflowIntent::SetGroupParent {
            target: ancestor,
            parent: Some(parent),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetGroupParent {
            target: GROUP,
            parent: Some(parent),
        }),
    )
    .unwrap();
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Group(ancestor))
    );
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: ancestor,
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().group(GROUP).unwrap().parent(), None);
}

#[test]
fn gate_d_group_translation_is_observational_and_commits_exact_transform_once() {
    let mut store = seed();
    let initial = Transform::from_matrix([
        0.0, -1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: GROUP,
                transform: initial,
            },
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let expected = Transform::from_matrix([
        0.0, -1.0, 0.0, 4.5, 1.0, 0.0, 0.0, -2.0, 0.0, 0.0, 1.0, 11.25, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetGroupTranslation {
            target: GROUP,
            x_mm_text: "4.5".to_owned(),
            y_mm_text: "-2".to_owned(),
            z_mm_text: "11.25".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetGroupTranslation(GROUP));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Group(GROUP)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Group(GROUP)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Transform(initial)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Transform(expected)
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(store.current().group(GROUP).unwrap().transform(), expected);
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(store.current().group(GROUP).unwrap().transform(), initial);
}

#[test]
fn gate_d_group_translation_rejects_denied_invalid_missing_and_stale_targets() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetOccurrenceParent],
                ),
                intent: WorkflowIntent::SetGroupTranslation {
                    target: GROUP,
                    x_mm_text: "1".to_owned(),
                    y_mm_text: "2".to_owned(),
                    z_mm_text: "3".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetGroupTranslation
        ))
    );
    for intent in [
        WorkflowIntent::SetGroupTranslation {
            target: GROUP,
            x_mm_text: "invalid".to_owned(),
            y_mm_text: "2".to_owned(),
            z_mm_text: "3".to_owned(),
        },
        WorkflowIntent::SetGroupTranslation {
            target: GROUP,
            x_mm_text: "1e999".to_owned(),
            y_mm_text: "2".to_owned(),
            z_mm_text: "3".to_owned(),
        },
        WorkflowIntent::SetGroupTranslation {
            target: GroupId(999),
            x_mm_text: "1".to_owned(),
            y_mm_text: "2".to_owned(),
            z_mm_text: "3".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Canonical(_))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetGroupTranslation {
            target: GROUP,
            x_mm_text: "1".to_owned(),
            y_mm_text: "2".to_owned(),
            z_mm_text: "3".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: GROUP,
                transform: Transform::from_translation(9.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().group(GROUP).unwrap().transform(),
        Transform::from_translation(9.0, 0.0, 0.0).unwrap()
    );
}

#[test]
fn gate_d_collection_membership_is_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetCollectionOccurrences {
            target: COLLECTION,
            occurrence_ids: vec![OCCURRENCE],
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetCollectionOccurrences(COLLECTION)
    );
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Collection(COLLECTION),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Occurrence(OCCURRENCE),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Collection(COLLECTION)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Occurrence(OCCURRENCE))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Occurrences(Vec::new())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Occurrences(vec![OCCURRENCE])
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store
            .current()
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OCCURRENCE]
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store
            .current()
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
}

#[test]
fn gate_d_occurrence_translation_is_observational_and_commits_exact_transform_once() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceTranslation {
            target: OCCURRENCE,
            x_mm_text: "12.5".to_owned(),
            y_mm_text: "-4".to_owned(),
            z_mm_text: "8.25".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceTranslation(OCCURRENCE)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Occurrence(OCCURRENCE)
        )]
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Transform(Transform::identity())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Transform(Transform::from_translation(12.5, -4.0, 8.25).unwrap())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().transform(),
        Transform::from_translation(12.5, -4.0, 8.25).unwrap()
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
}

#[test]
fn gate_d_collection_membership_rejects_denied_noncanonical_missing_and_stale_inputs() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetGroupParent],
                ),
                intent: WorkflowIntent::SetCollectionOccurrences {
                    target: COLLECTION,
                    occurrence_ids: vec![OCCURRENCE],
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetCollectionOccurrences
        ))
    );
    for intent in [
        WorkflowIntent::SetCollectionOccurrences {
            target: COLLECTION,
            occurrence_ids: vec![OCCURRENCE, OCCURRENCE],
        },
        WorkflowIntent::SetCollectionOccurrences {
            target: CollectionId(999),
            occurrence_ids: vec![OCCURRENCE],
        },
        WorkflowIntent::SetCollectionOccurrences {
            target: COLLECTION,
            occurrence_ids: vec![OccurrenceId(999)],
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetCollectionOccurrences {
            target: COLLECTION,
            occurrence_ids: vec![OCCURRENCE],
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OCCURRENCE,
                visible: false,
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store
            .current()
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
}

#[test]
fn gate_d_occurrence_translation_rejects_invalid_missing_and_stale_inputs() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetOccurrenceTranslation {
                target: OCCURRENCE,
                x_mm_text: "NaN".to_owned(),
                y_mm_text: "0".to_owned(),
                z_mm_text: "0".to_owned(),
            })
        ),
        Err(IntentError::Canonical(_))
    ));
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetOccurrenceTranslation {
                target: OccurrenceId(999),
                x_mm_text: "1".to_owned(),
                y_mm_text: "2".to_owned(),
                z_mm_text: "3".to_owned(),
            })
        ),
        Err(IntentError::Canonical(_))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetOccurrenceTranslation {
            target: OCCURRENCE,
            x_mm_text: "1".to_owned(),
            y_mm_text: "2".to_owned(),
            z_mm_text: "3".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OCCURRENCE,
                visible: false,
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().occurrence(OCCURRENCE).unwrap().transform(),
        Transform::identity()
    );
}

#[test]
fn gate_d_revalidates_unrelated_edits_but_rejects_relevant_changes() {
    let mut store = seed();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "30".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(99),
                name: "Unrelated".to_owned(),
            },
        ]))
        .unwrap();
    store.commit_verified_proposal(&proposal).unwrap();

    let stale = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "40".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0]],
            },
        ]))
        .unwrap();
    assert!(matches!(
        store.commit_verified_proposal(&stale),
        Err(ProposalCommitError::Stale(_))
    ));
}

#[test]
fn gate_d_capability_value_and_budget_failures_leave_the_document_unchanged() {
    let store = seed();
    let digest = store.current().canonical_digest();
    let revisions = store.revision_count();

    let denied = IntentRequest {
        grant: IntentGrant::new(RequestingPrincipal::LocalAssistant, []),
        intent: WorkflowIntent::SetRuleDimension {
            target: RULE,
            value_text: "700".to_owned(),
        },
        requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
    };
    assert_eq!(
        propose_intent(&store, denied),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetRuleDimension
        ))
    );

    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
                target: EXTRUSION,
                value_text: "not-a-number".to_owned(),
            })
        ),
        Err(IntentError::Canonical(_))
    ));

    let oversized = IntentRequest {
        grant: IntentGrant::m7a_local_assistant(),
        intent: WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "30".to_owned(),
        },
        requested_budget: ProposalBudget {
            max_commands: ProposalBudget::HOST_MAX.max_commands + 1,
            max_read_dependencies: 64,
            max_write_targets: 1,
        },
    };
    assert_eq!(
        propose_intent(&store, oversized),
        Err(IntentError::Proposal(
            ProposalPrepareError::HostBudgetExceeded
        ))
    );

    let insufficient = IntentRequest {
        grant: IntentGrant::m7a_local_assistant(),
        intent: WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "30".to_owned(),
        },
        requested_budget: ProposalBudget {
            max_commands: 0,
            max_read_dependencies: 64,
            max_write_targets: 1,
        },
    };
    assert_eq!(
        propose_intent(&store, insufficient),
        Err(IntentError::Proposal(
            ProposalPrepareError::RequestedBudgetExceeded
        ))
    );

    assert_eq!(store.current().canonical_digest(), digest);
    assert_eq!(store.revision_count(), revisions);
}

#[test]
fn gate_d_invalid_target_and_cross_document_proposal_fail_closed() {
    let store = seed();
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetRuleDimension {
                target: NodeId(999),
                value_text: "10".to_owned(),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetRuleDimension {
            target: RULE,
            value_text: "700".to_owned(),
        }),
    )
    .unwrap();
    let mut other = seed();
    let other_digest = other.current().canonical_digest();
    assert!(matches!(
        other.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(other.current().canonical_digest(), other_digest);
}

#[test]
fn gate_d_high_risk_commit_requires_distinct_authenticated_human_and_consumes_token_once() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([7; 32], 1).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 1)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some("api.example.test:443".to_owned()),
        Some("example-cloud".to_owned()),
        None,
    )
    .unwrap();
    let proposal = high_risk_proposal(&store, ProposalPrincipal::LocalAssistant, "31", scope);
    let digest_before = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::HumanApprovalRequired)
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(
        authority.issue(
            &proposal,
            AuthenticatedApprover::Machine(ProposalPrincipal::Plugin(9)),
            1_000,
            2_000,
        ),
        Err(HumanConfirmationError::MachineCannotApprove)
    );

    let approval = authority
        .issue(&proposal, AuthenticatedApprover::Human(77), 1_000, 2_000)
        .unwrap();
    let undo_before = store.visible_undo_steps();
    let committed = store
        .commit_high_risk_proposal(&proposal, &approval, 1_500)
        .unwrap();
    assert_eq!(committed.command_digest(), proposal.command_digest());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &approval, 1_500),
        Err(ProposalCommitError::HumanApprovalReplayed)
    ));
}

#[test]
fn gate_d_high_risk_token_fails_closed_on_scope_signature_expiry_and_policy_change() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([11; 32], 4).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 4)
        .unwrap();
    let first_scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some("api.first.test:443".to_owned()),
        Some("first-provider".to_owned()),
        None,
    )
    .unwrap();
    let proposal = high_risk_proposal(&store, ProposalPrincipal::Plugin(41), "32", first_scope);
    let approval = authority
        .issue(&proposal, AuthenticatedApprover::Human(88), 10_000, 11_000)
        .unwrap();
    let digest_before = store.current().canonical_digest();

    let other_scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some("api.second.test:443".to_owned()),
        Some("second-provider".to_owned()),
        None,
    )
    .unwrap();
    let other_proposal =
        high_risk_proposal(&store, ProposalPrincipal::Plugin(41), "32", other_scope);
    assert!(matches!(
        store.commit_high_risk_proposal(&other_proposal, &approval, 10_500),
        Err(ProposalCommitError::HumanApprovalInvalid)
    ));

    let untrusted = TrustedConfirmationSurface::new([12; 32], 4).unwrap();
    let wrong_signature = untrusted
        .issue(&proposal, AuthenticatedApprover::Human(88), 10_000, 11_000)
        .unwrap();
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &wrong_signature, 10_500),
        Err(ProposalCommitError::HumanApprovalInvalid)
    ));
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &approval, 11_001),
        Err(ProposalCommitError::HumanApprovalInvalid)
    ));

    store
        .configure_human_confirmation_policy(authority.verifying_key(), 5)
        .unwrap();
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &approval, 10_500),
        Err(ProposalCommitError::HumanApprovalPolicyStale)
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);
}

#[test]
fn gate_d_high_risk_preparation_and_issuance_reject_ambiguous_identity_or_scope() {
    assert_eq!(
        HighRiskScope::new(
            HighRiskClass::ExternalDisclosure,
            Some("api.example.test:443".to_owned()),
            None,
            None,
        ),
        Err(HumanConfirmationError::InvalidScope)
    );

    let store = seed();
    let overwrite = HighRiskScope::new(
        HighRiskClass::Overwrite,
        None,
        None,
        Some("C:/models/part.ketchup".to_owned()),
    )
    .unwrap();
    let unidentified = store.prepare_proposal_with_context(
        CommandBatch::new(vec![CanonicalCommand::SetFeatureDimension {
            id: EXTRUSION,
            dimension: Dimension::from_decimal("33").unwrap(),
        }]),
        ProposalContext {
            principal: ProposalPrincipal::ManualClient,
            goal: ProposalGoal::SetFeatureDimension(EXTRUSION),
            assumptions: vec![],
            risk: ProposalRisk::High(HighRiskClass::Overwrite),
            confirmation: ProposalConfirmation::HumanOnly(overwrite),
            requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
        },
    );
    assert!(matches!(
        unidentified,
        Err(ProposalPrepareError::Confirmation(
            HumanConfirmationError::UnidentifiedRequester
        ))
    ));

    let scope = HighRiskScope::new(HighRiskClass::CapabilityExpansion, None, None, None).unwrap();
    let proposal = high_risk_proposal(&store, ProposalPrincipal::Human(55), "33", scope);
    let authority = TrustedConfirmationSurface::new([13; 32], 1).unwrap();
    assert_eq!(
        authority.issue(&proposal, AuthenticatedApprover::Human(55), 1_000, 2_000,),
        Err(HumanConfirmationError::RequesterCannotApprove)
    );
    assert_eq!(
        authority.issue(
            &proposal,
            AuthenticatedApprover::Human(56),
            1_000,
            1_000 + MAX_HUMAN_CONFIRMATION_LIFETIME_MS + 1,
        ),
        Err(HumanConfirmationError::InvalidLifetime)
    );
}

#[test]
fn gate_d_side_effect_receipt_is_payload_bound_one_use_and_non_canonical() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([17; 32], 3).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 3)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::Overwrite,
        None,
        None,
        Some("C:/models/part.ketchup".to_owned()),
    )
    .unwrap();
    let proposal = store
        .prepare_high_risk_side_effect(
            "overwrite-native-document",
            ProposalPrincipal::LocalAssistant,
            scope,
            b"exact bytes displayed for overwrite",
        )
        .unwrap();
    let approval = authority
        .issue_side_effect(&proposal, AuthenticatedApprover::Human(91), 20_000, 21_000)
        .unwrap();
    let digest_before = store.current().canonical_digest();
    let revision_count_before = store.revision_count();
    let undo_before = store.visible_undo_steps();

    let receipt = store
        .authorize_high_risk_side_effect(&proposal, &approval, 20_500)
        .unwrap();
    assert_eq!(receipt.approving_human(), 91);
    assert_eq!(receipt.document_id(), proposal.document_id());
    assert_eq!(receipt.revision_id(), proposal.provenance_revision());
    assert_eq!(receipt.operation(), "overwrite-native-document");
    assert_eq!(receipt.operation_digest(), proposal.operation_digest());
    assert_eq!(receipt.payload_digest(), proposal.payload_digest());
    assert_eq!(receipt.scope(), proposal.scope());
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.revision_count(), revision_count_before);
    assert_eq!(store.visible_undo_steps(), undo_before);
    assert_eq!(
        store.authorize_high_risk_side_effect(&proposal, &approval, 20_500),
        Err(ketchup_core::document::SideEffectAuthorizationError::Replayed)
    );
}

#[test]
fn gate_d_side_effect_authorization_rejects_payload_substitution_and_stale_snapshot() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([19; 32], 2).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 2)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::Overwrite,
        None,
        None,
        Some("C:/models/part.ketchup".to_owned()),
    )
    .unwrap();
    let original = store
        .prepare_high_risk_side_effect(
            "overwrite-native-document",
            ProposalPrincipal::LocalAssistant,
            scope.clone(),
            b"approved bytes",
        )
        .unwrap();
    let approval = authority
        .issue_side_effect(&original, AuthenticatedApprover::Human(92), 30_000, 31_000)
        .unwrap();
    let substituted = store
        .prepare_high_risk_side_effect(
            "overwrite-native-document",
            ProposalPrincipal::LocalAssistant,
            scope,
            b"different bytes",
        )
        .unwrap();
    assert_eq!(
        store.authorize_high_risk_side_effect(&substituted, &approval, 30_500),
        Err(ketchup_core::document::SideEffectAuthorizationError::Invalid)
    );

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(99),
                name: "Changed after review".to_owned(),
            },
        ]))
        .unwrap();
    assert_eq!(
        store.authorize_high_risk_side_effect(&original, &approval, 30_500),
        Err(ketchup_core::document::SideEffectAuthorizationError::Invalid)
    );
}

#[test]
fn gate_d_rule_outputs_are_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let requested = vec![rule_output("center"), rule_output("right")];
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetRuleOutputs {
            target: RULE_OUTPUTS,
            outputs: requested.clone(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetRuleOutputs(RULE_OUTPUTS));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::EvaluatorNode(RULE_OUTPUTS)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::EvaluatorNode(RULE_OUTPUTS)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::RuleOutputs(vec![rule_output("left")])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::RuleOutputs(requested.clone())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.current().evaluator_node(RULE_OUTPUTS).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &requested
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(matches!(
        store.current().evaluator_node(RULE_OUTPUTS).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &vec![rule_output("left")]
    ));
}

#[test]
fn gate_d_rule_outputs_reject_denied_invalid_wrong_kind_missing_and_stale_dependencies() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetEvaluatorExpression],
                ),
                intent: WorkflowIntent::SetRuleOutputs {
                    target: RULE_OUTPUTS,
                    outputs: vec![rule_output("right")],
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetRuleOutputs
        ))
    );
    let wrong_port = RuleOutput::new(
        SlotSegment::new(RULE_OUTPUTS, "missing_port", "right").unwrap(),
        Vec::new(),
    )
    .unwrap();
    for intent in [
        WorkflowIntent::SetRuleOutputs {
            target: RULE_OUTPUTS,
            outputs: vec![wrong_port],
        },
        WorkflowIntent::SetRuleOutputs {
            target: RULE,
            outputs: vec![rule_output("right")],
        },
        WorkflowIntent::SetRuleOutputs {
            target: NodeId(999),
            outputs: vec![rule_output("right")],
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetRuleOutputs {
            target: RULE_OUTPUTS,
            outputs: vec![rule_output("right")],
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: RULE,
                dimension: dimension("601", 601.0),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(matches!(
        store.current().evaluator_node(RULE_OUTPUTS).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &vec![rule_output("left")]
    ));
}

#[test]
fn gate_d_create_tag_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = TagId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateTag {
            target,
            name: "Reviewed tag".to_owned(),
            visible: false,
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateTag(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Tag(target)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Tag(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::TagState {
            name: "Reviewed tag".to_owned(),
            visible: false,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.tag(target).unwrap();
    assert_eq!(created.name(), "Reviewed tag");
    assert!(!created.visible());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().tag(target).is_none());
}

#[test]
fn gate_d_create_tag_rejects_denied_invalid_existing_and_stale_id() {
    let mut store = seed();
    let target = TagId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetTagVisibility],
                ),
                intent: WorkflowIntent::CreateTag {
                    target,
                    name: "Denied".to_owned(),
                    visible: true,
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::CreateTag))
    );
    for intent in [
        WorkflowIntent::CreateTag {
            target: TagId(0),
            name: "Invalid ID".to_owned(),
            visible: true,
        },
        WorkflowIntent::CreateTag {
            target,
            name: String::new(),
            visible: true,
        },
        WorkflowIntent::CreateTag {
            target: TAG,
            name: "Duplicate".to_owned(),
            visible: false,
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateTag {
            target,
            name: "Reviewed tag".to_owned(),
            visible: true,
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: target,
            name: "Concurrent tag".to_owned(),
            visible: false,
        }]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    let snapshot = store.current();
    let concurrent = snapshot.tag(target).unwrap();
    assert_eq!(concurrent.name(), "Concurrent tag");
    assert!(!concurrent.visible());
}

#[test]
fn gate_d_delete_tag_is_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteTag { target: TAG }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteTag(TAG));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Tag(TAG)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Tag(TAG)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::TagState {
            name: "Hardware".to_owned(),
            visible: true,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().tag(TAG).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let snapshot = store.current();
    let restored = snapshot.tag(TAG).unwrap();
    assert_eq!(restored.name(), "Hardware");
    assert!(restored.visible());
}

#[test]
fn gate_d_delete_tag_rejects_denied_missing_assigned_and_stale_assignment() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateTag],
                ),
                intent: WorkflowIntent::DeleteTag { target: TAG },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::DeleteTag))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteTag { target: TagId(999) })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTag {
                id: OCCURRENCE,
                tag: Some(TAG),
            },
        ]))
        .unwrap();
    let assigned_digest = store.current().canonical_digest();
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteTag { target: TAG })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), assigned_digest);
    assert_ne!(assigned_digest, digest_before);

    let mut stale_store = seed();
    let proposal = propose_intent(
        &stale_store,
        IntentRequest::m7a(WorkflowIntent::DeleteTag { target: TAG }),
    )
    .unwrap();
    stale_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: TAG,
                visible: false,
            },
        ]))
        .unwrap();
    let stale_digest = stale_store.current().canonical_digest();
    assert!(matches!(
        stale_store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale_store.current().canonical_digest(), stale_digest);
    assert!(!stale_store.current().tag(TAG).unwrap().visible());

    let mut concurrent_store = seed();
    let proposal = propose_intent(
        &concurrent_store,
        IntentRequest::m7a(WorkflowIntent::DeleteTag { target: TAG }),
    )
    .unwrap();
    concurrent_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTag {
                id: OCCURRENCE,
                tag: Some(TAG),
            },
        ]))
        .unwrap();
    let changed_digest = concurrent_store.current().canonical_digest();
    assert!(matches!(
        concurrent_store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Preparation(
            ProposalPrepareError::Canonical(ketchup_core::document::CanonicalError::TagInUse(TAG))
        ))
    ));
    assert_eq!(
        concurrent_store.current().canonical_digest(),
        changed_digest
    );
    assert!(concurrent_store.current().tag(TAG).is_some());
    assert_eq!(
        concurrent_store
            .current()
            .occurrence(OCCURRENCE)
            .unwrap()
            .tag(),
        Some(TAG)
    );
}

#[test]
fn gate_d_create_collection_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = CollectionId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateCollection {
            target,
            name: "Reviewed selection".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateCollection(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Collection(target)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Collection(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Reviewed selection".to_owned())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().collection(target).unwrap().name(),
        "Reviewed selection"
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().collection(target).is_none());
}

#[test]
fn gate_d_create_collection_rejects_denied_invalid_existing_and_stale_id() {
    let mut store = seed();
    let target = CollectionId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetCollectionOccurrences],
                ),
                intent: WorkflowIntent::CreateCollection {
                    target,
                    name: "Denied".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateCollection
        ))
    );
    for intent in [
        WorkflowIntent::CreateCollection {
            target: CollectionId(0),
            name: "Invalid ID".to_owned(),
        },
        WorkflowIntent::CreateCollection {
            target,
            name: String::new(),
        },
        WorkflowIntent::CreateCollection {
            target: COLLECTION,
            name: "Duplicate".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateCollection {
            target,
            name: "Reviewed selection".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: target,
                name: "Concurrent selection".to_owned(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().collection(target).unwrap().name(),
        "Concurrent selection"
    );
}

#[test]
fn gate_d_delete_collection_is_typed_observational_and_undoable() {
    let mut store = seed();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![OCCURRENCE],
            },
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteCollection { target: COLLECTION }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteCollection(COLLECTION));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Collection(COLLECTION)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Collection(COLLECTION)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::CollectionState {
            name: "Selection set".to_owned(),
            occurrence_ids: vec![OCCURRENCE],
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().collection(COLLECTION).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let snapshot = store.current();
    let restored = snapshot.collection(COLLECTION).unwrap();
    assert_eq!(restored.name(), "Selection set");
    assert_eq!(
        restored.occurrence_ids().collect::<Vec<_>>(),
        vec![OCCURRENCE]
    );
}

#[test]
fn gate_d_delete_collection_rejects_denied_missing_and_stale_membership() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateCollection],
                ),
                intent: WorkflowIntent::DeleteCollection { target: COLLECTION },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteCollection
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteCollection {
                target: CollectionId(999),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteCollection { target: COLLECTION }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![OCCURRENCE],
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store
            .current()
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OCCURRENCE]
    );
}

#[test]
fn gate_d_delete_group_is_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteGroup { target: GROUP }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteGroup(GROUP));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Group(GROUP)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Group(GROUP)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::GroupState {
            name: "Assembly".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().group(GROUP).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let snapshot = store.current();
    let restored = snapshot.group(GROUP).unwrap();
    assert_eq!(restored.name(), "Assembly");
    assert_eq!(restored.transform(), Transform::identity());
    assert_eq!(restored.parent(), None);
}

#[test]
fn gate_d_delete_group_rejects_denied_missing_nonempty_and_stale_children() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateGroup],
                ),
                intent: WorkflowIntent::DeleteGroup { target: GROUP },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::DeleteGroup))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteGroup {
                target: GroupId(999),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceParent {
                id: OCCURRENCE,
                parent: Some(GROUP),
            },
        ]))
        .unwrap();
    let nonempty_digest = store.current().canonical_digest();
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteGroup { target: GROUP })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), nonempty_digest);
    assert_ne!(nonempty_digest, digest_before);

    let mut stale_store = seed();
    let proposal = propose_intent(
        &stale_store,
        IntentRequest::m7a(WorkflowIntent::DeleteGroup { target: GROUP }),
    )
    .unwrap();
    let child = GroupId(24);
    stale_store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: child,
            name: "Concurrent child".to_owned(),
            transform: Transform::identity(),
            parent: Some(GROUP),
        }]))
        .unwrap();
    let changed_digest = stale_store.current().canonical_digest();

    assert!(matches!(
        stale_store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale_store.current().canonical_digest(), changed_digest);
    assert!(stale_store.current().group(GROUP).is_some());
    assert_eq!(
        stale_store.current().group(child).unwrap().parent(),
        Some(GROUP)
    );
}

#[test]
fn gate_d_create_definition_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = DefinitionId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateDefinition {
            target,
            name: "Reviewed component".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateDefinition(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Definition(target)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Definition(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Reviewed component".to_owned())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store.current().definition(target).unwrap().name(),
        "Reviewed component"
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().definition(target).is_none());
}

#[test]
fn gate_d_create_definition_rejects_denied_invalid_existing_and_stale_id() {
    let mut store = seed();
    let target = DefinitionId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::RenameDefinition],
                ),
                intent: WorkflowIntent::CreateDefinition {
                    target,
                    name: "Denied".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateDefinition
        ))
    );
    for intent in [
        WorkflowIntent::CreateDefinition {
            target: DefinitionId(0),
            name: "Invalid ID".to_owned(),
        },
        WorkflowIntent::CreateDefinition {
            target,
            name: String::new(),
        },
        WorkflowIntent::CreateDefinition {
            target: DEFINITION,
            name: "Duplicate".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateDefinition {
            target,
            name: "Reviewed component".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: target,
                name: "Concurrent component".to_owned(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().definition(target).unwrap().name(),
        "Concurrent component"
    );
}

#[test]
fn gate_d_create_group_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = GroupId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateGroup {
            target,
            name: "Reviewed root group".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateGroup(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Group(target)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Group(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::GroupState {
            name: "Reviewed root group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.group(target).unwrap();
    assert_eq!(created.name(), "Reviewed root group");
    assert_eq!(created.transform(), Transform::identity());
    assert_eq!(created.parent(), None);
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().group(target).is_none());
}

#[test]
fn gate_d_create_group_rejects_denied_invalid_existing_and_stale_id() {
    let mut store = seed();
    let target = GroupId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetGroupParent],
                ),
                intent: WorkflowIntent::CreateGroup {
                    target,
                    name: "Denied".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::CreateGroup))
    );
    for intent in [
        WorkflowIntent::CreateGroup {
            target: GroupId(0),
            name: "Invalid ID".to_owned(),
        },
        WorkflowIntent::CreateGroup {
            target,
            name: String::new(),
        },
        WorkflowIntent::CreateGroup {
            target: GROUP,
            name: "Duplicate".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateGroup {
            target,
            name: "Reviewed root group".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: target,
            name: "Concurrent root group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().group(target).unwrap().name(),
        "Concurrent root group"
    );
}

#[test]
fn gate_d_create_occurrence_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = OccurrenceId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateOccurrence {
            target,
            definition: SECOND_DEFINITION,
            name: "Reviewed occurrence".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateOccurrence(target));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::Occurrence(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Definition(SECOND_DEFINITION),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Occurrence(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::OccurrenceState {
            definition: SECOND_DEFINITION,
            name: "Reviewed occurrence".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.occurrence(target).unwrap();
    assert_eq!(created.definition_id(), SECOND_DEFINITION);
    assert_eq!(created.name(), "Reviewed occurrence");
    assert_eq!(created.transform(), Transform::identity());
    assert_eq!(created.parent(), None);
    assert_eq!(created.tag(), None);
    assert!(created.visible());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().occurrence(target).is_none());
}

#[test]
fn gate_d_create_occurrence_rejects_denied_invalid_existing_dependency_and_stale_id() {
    let mut store = seed();
    let target = OccurrenceId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetOccurrenceVisibility],
                ),
                intent: WorkflowIntent::CreateOccurrence {
                    target,
                    definition: SECOND_DEFINITION,
                    name: "Denied".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateOccurrence
        ))
    );
    for intent in [
        WorkflowIntent::CreateOccurrence {
            target: OccurrenceId(0),
            definition: SECOND_DEFINITION,
            name: "Invalid ID".to_owned(),
        },
        WorkflowIntent::CreateOccurrence {
            target,
            definition: SECOND_DEFINITION,
            name: String::new(),
        },
        WorkflowIntent::CreateOccurrence {
            target: OCCURRENCE,
            definition: SECOND_DEFINITION,
            name: "Duplicate".to_owned(),
        },
        WorkflowIntent::CreateOccurrence {
            target,
            definition: DefinitionId(999),
            name: "Missing definition".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateOccurrence {
            target,
            definition: SECOND_DEFINITION,
            name: "Reviewed occurrence".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: target,
                definition_id: SECOND_DEFINITION,
                name: "Concurrent occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().occurrence(target).unwrap().name(),
        "Concurrent occurrence"
    );
}

#[test]
fn gate_d_create_profile_feature_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = FeatureId(24);
    let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateProfileFeature {
            target,
            definition: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateProfileFeature(target));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::Feature(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Definition(SECOND_DEFINITION),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([
            AuthoritativeDependency::Definition(SECOND_DEFINITION),
            AuthoritativeDependency::Feature(target),
        ])
    );
    let definition_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| entry.target == AuthoritativeDependency::Definition(SECOND_DEFINITION))
        .unwrap();
    assert_eq!(
        definition_diff.before,
        ProposalValue::DefinitionFeatures(Vec::new())
    );
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionFeatures(vec![target])
    );
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| entry.target == AuthoritativeDependency::Feature(target))
        .unwrap();
    assert_eq!(feature_diff.before, ProposalValue::Missing);
    assert_eq!(
        feature_diff.after,
        ProposalValue::ProfileFeatureState {
            definition: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.feature(target).unwrap();
    assert_eq!(created.definition_id(), SECOND_DEFINITION);
    assert_eq!(created.name(), "Reviewed profile");
    assert!(matches!(
        created.kind(),
        FeatureKind::Profile { points_mm: created_points } if created_points == &points_mm
    ));
    assert_eq!(
        snapshot
            .definition(SECOND_DEFINITION)
            .unwrap()
            .feature_ids(),
        &[target]
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let snapshot = store.current();
    assert!(snapshot.feature(target).is_none());
    assert!(
        snapshot
            .definition(SECOND_DEFINITION)
            .unwrap()
            .feature_ids()
            .is_empty()
    );
}

#[test]
fn gate_d_create_profile_feature_rejects_denied_invalid_existing_dependency_and_stale_id() {
    let mut store = seed();
    let target = FeatureId(24);
    let valid_points = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetProfilePoints],
                ),
                intent: WorkflowIntent::CreateProfileFeature {
                    target,
                    definition: SECOND_DEFINITION,
                    name: "Denied".to_owned(),
                    points_mm: valid_points.clone(),
                },
                requested_budget: ProposalBudget::M18C_CREATE_FEATURE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateProfileFeature
        ))
    );
    for intent in [
        WorkflowIntent::CreateProfileFeature {
            target: FeatureId(0),
            definition: SECOND_DEFINITION,
            name: "Invalid ID".to_owned(),
            points_mm: valid_points.clone(),
        },
        WorkflowIntent::CreateProfileFeature {
            target,
            definition: SECOND_DEFINITION,
            name: String::new(),
            points_mm: valid_points.clone(),
        },
        WorkflowIntent::CreateProfileFeature {
            target,
            definition: DefinitionId(999),
            name: "Missing definition".to_owned(),
            points_mm: valid_points.clone(),
        },
        WorkflowIntent::CreateProfileFeature {
            target: PROFILE,
            definition: SECOND_DEFINITION,
            name: "Duplicate".to_owned(),
            points_mm: valid_points.clone(),
        },
        WorkflowIntent::CreateProfileFeature {
            target,
            definition: SECOND_DEFINITION,
            name: "Invalid profile".to_owned(),
            points_mm: vec![[0.0, 0.0], [20.0, 10.0], [20.0, 0.0], [0.0, 10.0]],
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let mut dependency_store = seed();
    let dependency_proposal = propose_intent(
        &dependency_store,
        IntentRequest::m7a(WorkflowIntent::CreateProfileFeature {
            target,
            definition: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: valid_points.clone(),
        }),
    )
    .unwrap();
    dependency_store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(25),
            definition_id: SECOND_DEFINITION,
            name: "Concurrent sibling".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: valid_points.clone(),
            },
        }]))
        .unwrap();
    let dependency_digest = dependency_store.current().canonical_digest();
    assert!(matches!(
        dependency_store.commit_verified_proposal(&dependency_proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(
        dependency_store.current().canonical_digest(),
        dependency_digest
    );
    assert!(dependency_store.current().feature(target).is_none());

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateProfileFeature {
            target,
            definition: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: valid_points.clone(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: target,
            definition_id: SECOND_DEFINITION,
            name: "Concurrent profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: valid_points,
            },
        }]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().feature(target).unwrap().name(),
        "Concurrent profile"
    );
}

#[test]
fn gate_d_delete_occurrence_is_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteOccurrence { target: OCCURRENCE }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteOccurrence(OCCURRENCE));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Occurrence(OCCURRENCE)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Occurrence(OCCURRENCE)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::OccurrenceState {
            definition: DEFINITION,
            name: "Box occurrence".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().occurrence(OCCURRENCE).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let snapshot = store.current();
    let restored = snapshot.occurrence(OCCURRENCE).unwrap();
    assert_eq!(restored.definition_id(), DEFINITION);
    assert_eq!(restored.name(), "Box occurrence");
    assert_eq!(restored.transform(), Transform::identity());
    assert_eq!(restored.parent(), None);
    assert_eq!(restored.tag(), None);
    assert!(restored.visible());
}

#[test]
fn gate_d_delete_occurrence_rejects_denied_missing_collected_and_stale_collection() {
    let store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateOccurrence],
                ),
                intent: WorkflowIntent::DeleteOccurrence { target: OCCURRENCE },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteOccurrence
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteOccurrence {
                target: OccurrenceId(999),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let mut collected_store = seed();
    collected_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![OCCURRENCE],
            },
        ]))
        .unwrap();
    let collected_digest = collected_store.current().canonical_digest();
    assert!(matches!(
        propose_intent(
            &collected_store,
            IntentRequest::m7a(WorkflowIntent::DeleteOccurrence { target: OCCURRENCE })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(
        collected_store.current().canonical_digest(),
        collected_digest
    );

    let mut stale_store = seed();
    let proposal = propose_intent(
        &stale_store,
        IntentRequest::m7a(WorkflowIntent::DeleteOccurrence { target: OCCURRENCE }),
    )
    .unwrap();
    stale_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![OCCURRENCE],
            },
        ]))
        .unwrap();
    let changed_digest = stale_store.current().canonical_digest();

    assert!(matches!(
        stale_store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale_store.current().canonical_digest(), changed_digest);
    assert!(stale_store.current().occurrence(OCCURRENCE).is_some());
    assert_eq!(
        stale_store
            .current()
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OCCURRENCE]
    );
}

#[test]
fn gate_d_delete_profile_feature_is_typed_observational_and_undoable() {
    let target = FeatureId(24);
    let points_mm = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0]];
    let mut store = seed();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: target,
            definition_id: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: points_mm.clone(),
            },
        }]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteProfileFeature { target }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteProfileFeature(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(target)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([
            AuthoritativeDependency::Definition(SECOND_DEFINITION),
            AuthoritativeDependency::Feature(target),
        ])
    );
    let definition_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| entry.target == AuthoritativeDependency::Definition(SECOND_DEFINITION))
        .unwrap();
    assert_eq!(
        definition_diff.before,
        ProposalValue::DefinitionFeatures(vec![target])
    );
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionFeatures(Vec::new())
    );
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| entry.target == AuthoritativeDependency::Feature(target))
        .unwrap();
    assert_eq!(
        feature_diff.before,
        ProposalValue::ProfileFeatureState {
            definition: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(feature_diff.after, ProposalValue::Missing);
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().feature(target).is_none());
    assert!(
        store
            .current()
            .definition(SECOND_DEFINITION)
            .unwrap()
            .feature_ids()
            .is_empty()
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let snapshot = store.current();
    let restored = snapshot.feature(target).unwrap();
    assert_eq!(restored.definition_id(), SECOND_DEFINITION);
    assert_eq!(restored.name(), "Reviewed profile");
    assert_eq!(
        restored.kind(),
        &FeatureKind::Profile {
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(
        snapshot
            .definition(SECOND_DEFINITION)
            .unwrap()
            .feature_ids(),
        &[target]
    );
}

#[test]
fn gate_d_delete_profile_feature_rejects_denied_missing_used_and_stale_definition() {
    let store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateProfileFeature],
                ),
                intent: WorkflowIntent::DeleteProfileFeature { target: PROFILE },
                requested_budget: ProposalBudget::M18C_CREATE_FEATURE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteProfileFeature
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteProfileFeature {
                target: FeatureId(999),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteProfileFeature { target: EXTRUSION })
        ),
        Err(IntentError::Canonical(
            ketchup_core::document::CanonicalError::FeatureIsNotProfile(EXTRUSION)
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteProfileFeature { target: PROFILE })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let target = FeatureId(24);
    let mut stale_store = seed();
    stale_store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: target,
            definition_id: SECOND_DEFINITION,
            name: "Reviewed profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0]],
            },
        }]))
        .unwrap();
    let proposal = propose_intent(
        &stale_store,
        IntentRequest::m7a(WorkflowIntent::DeleteProfileFeature { target }),
    )
    .unwrap();
    stale_store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(25),
            definition_id: SECOND_DEFINITION,
            name: "Concurrent sibling".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [6.0, 0.0], [6.0, 4.0]],
            },
        }]))
        .unwrap();
    let changed_digest = stale_store.current().canonical_digest();

    assert!(matches!(
        stale_store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale_store.current().canonical_digest(), changed_digest);
    assert!(stale_store.current().feature(target).is_some());
    assert_eq!(
        stale_store
            .current()
            .definition(SECOND_DEFINITION)
            .unwrap()
            .feature_ids(),
        &[target, FeatureId(25)]
    );
}

#[test]
fn gate_d_create_evaluator_input_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = NodeId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateEvaluatorInput {
            target,
            name: "Reviewed depth".to_owned(),
            value_text: "42.5".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorInput(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::EvaluatorNode(target)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::EvaluatorNode(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorInputState {
            name: "Reviewed depth".to_owned(),
            dimension: Dimension::from_decimal("42.5").unwrap(),
            dependencies: Vec::new(),
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed depth");
    assert_eq!(
        created.dimension(),
        Some(&Dimension::from_decimal("42.5").unwrap())
    );
    assert!(created.dependencies().is_empty());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().evaluator_node(target).is_none());
}

#[test]
fn gate_d_create_evaluator_input_rejects_denied_invalid_existing_and_stale_id() {
    let mut store = seed();
    let target = NodeId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetRuleDimension],
                ),
                intent: WorkflowIntent::CreateEvaluatorInput {
                    target,
                    name: "Denied".to_owned(),
                    value_text: "1".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateEvaluatorInput
        ))
    );
    for intent in [
        WorkflowIntent::CreateEvaluatorInput {
            target: NodeId(0),
            name: "Invalid ID".to_owned(),
            value_text: "1".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorInput {
            target,
            name: String::new(),
            value_text: "1".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorInput {
            target: RULE,
            name: "Duplicate".to_owned(),
            value_text: "1".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CreateEvaluatorInput {
                target,
                name: "Bad value".to_owned(),
                value_text: "not-a-number".to_owned(),
            })
        ),
        Err(IntentError::Canonical(_))
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateEvaluatorInput {
            target,
            name: "Reviewed depth".to_owned(),
            value_text: "42.5".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: target,
                name: "Concurrent input".to_owned(),
                dimension: Dimension::from_decimal("7").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().evaluator_node(target).unwrap().name(),
        "Concurrent input"
    );
}

#[test]
fn gate_d_create_evaluator_expression_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = NodeId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateEvaluatorExpression {
            target,
            name: "Reviewed span".to_owned(),
            expression: "$1 + $2".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreateEvaluatorExpression(target)
    );
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::EvaluatorNode(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(RULE),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(EXPRESSION_INPUT),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::EvaluatorNode(target)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::EvaluatorNode(RULE))
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::EvaluatorNode(EXPRESSION_INPUT))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorExpressionState {
            name: "Reviewed span".to_owned(),
            expression: "$1 + $2".to_owned(),
            dependencies: vec![RULE, EXPRESSION_INPUT],
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed span");
    assert_eq!(created.kind().source(), "$1 + $2");
    assert_eq!(created.dependencies(), &[RULE, EXPRESSION_INPUT]);
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().evaluator_node(target).is_none());
}

#[test]
fn gate_d_create_evaluator_expression_rejects_denied_invalid_and_stale_dependency() {
    let mut store = seed();
    let target = NodeId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateEvaluatorInput],
                ),
                intent: WorkflowIntent::CreateEvaluatorExpression {
                    target,
                    name: "Denied".to_owned(),
                    expression: "$1".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateEvaluatorExpression
        ))
    );
    for intent in [
        WorkflowIntent::CreateEvaluatorExpression {
            target: NodeId(0),
            name: "Invalid ID".to_owned(),
            expression: "$1".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorExpression {
            target,
            name: String::new(),
            expression: "$1".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorExpression {
            target: RULE,
            name: "Duplicate".to_owned(),
            expression: "$2".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorExpression {
            target,
            name: "Malformed".to_owned(),
            expression: "$1 +".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorExpression {
            target,
            name: "Missing dependency".to_owned(),
            expression: "$999".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateEvaluatorExpression {
            target,
            name: "Reviewed span".to_owned(),
            expression: "$1 + $2".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: EXPRESSION_INPUT,
                dimension: Dimension::from_decimal("9").unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(store.current().evaluator_node(target).is_none());
}

#[test]
fn gate_d_create_rule_override_is_typed_observational_and_undoable() {
    let mut store = seed();
    let rule = NodeId(30);
    let target = 31;
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "override source".to_owned(),
            expression: "1".to_owned(),
            input_ports: Vec::new(),
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: vec![
                RuleOutput::new(
                    SlotSegment::new(rule, "result", "left").unwrap(),
                    Vec::new(),
                )
                .unwrap(),
            ],
            override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
        }]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateRuleOverride {
            target,
            rule,
            output_port: "result".to_owned(),
            semantic_key: "left".to_owned(),
            parameter: "offset".to_owned(),
            value_text: "2.5".to_owned(),
        }),
    )
    .unwrap();

    let identity = ketchup_core::document::DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateRuleOverride(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert!(
        proposal
            .authoritative_writes()
            .contains(&AuthoritativeDependency::Override(target))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::RuleOverrideState {
            target: identity.clone(),
            parameter: "offset".to_owned(),
            value: 2.5,
            health: SlotResolution::Resolved,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let created = store.current().override_by_id(target).unwrap().clone();
    assert_eq!(created.target, identity);
    assert_eq!(created.parameter, "offset");
    assert_eq!(created.value(), 2.5);
    assert_eq!(created.health, SlotResolution::Resolved);
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().override_by_id(target).is_none());
}

#[test]
fn gate_d_create_rule_override_rejects_denied_invalid_existing_and_stale_rule() {
    let mut store = seed();
    let rule = NodeId(30);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "override source".to_owned(),
            expression: "1".to_owned(),
            input_ports: Vec::new(),
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: vec![
                RuleOutput::new(
                    SlotSegment::new(rule, "result", "left").unwrap(),
                    Vec::new(),
                )
                .unwrap(),
            ],
            override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
        }]))
        .unwrap();
    let request = || WorkflowIntent::CreateRuleOverride {
        target: 31,
        rule,
        output_port: "result".to_owned(),
        semantic_key: "left".to_owned(),
        parameter: "offset".to_owned(),
        value_text: "2.5".to_owned(),
    };
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateEvaluatorRule],
                ),
                intent: request(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateRuleOverride
        ))
    ));
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CreateRuleOverride {
                target: 31,
                rule,
                output_port: "result".to_owned(),
                semantic_key: "missing".to_owned(),
                parameter: "offset".to_owned(),
                value_text: "2.5".to_owned(),
            })
        )
        .is_err()
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CreateRuleOverride {
                target: 31,
                rule,
                output_port: "result".to_owned(),
                semantic_key: "left".to_owned(),
                parameter: "undeclared".to_owned(),
                value_text: "2.5".to_owned(),
            })
        )
        .is_err()
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CreateRuleOverride {
                target: 0,
                rule,
                output_port: "result".to_owned(),
                semantic_key: "left".to_owned(),
                parameter: "offset".to_owned(),
                value_text: "2.5".to_owned(),
            })
        )
        .is_err()
    );
    let identity = ketchup_core::document::DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertOverride(
            ketchup_core::document::CanonicalOverride::new(
                31,
                identity,
                "offset",
                1.0,
                SlotResolution::Resolved,
            )
            .unwrap(),
        )]))
        .unwrap();
    assert!(propose_intent(&store, IntentRequest::m7a(request())).is_err());
    store.undo().unwrap();

    let proposal = propose_intent(&store, IntentRequest::m7a(request())).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: rule,
            outputs: vec![
                RuleOutput::new(
                    SlotSegment::new(rule, "result", "right").unwrap(),
                    Vec::new(),
                )
                .unwrap(),
            ],
        }]))
        .unwrap();
    assert!(store.commit_verified_proposal(&proposal).is_err());
    assert!(store.current().override_by_id(31).is_none());
}

#[test]
fn gate_d_delete_rule_override_is_typed_observational_and_undoable() {
    let mut store = seed();
    let rule = NodeId(30);
    let target = 31;
    let identity = ketchup_core::document::DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "override source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            },
            CanonicalCommand::UpsertOverride(
                ketchup_core::document::CanonicalOverride::new(
                    target,
                    identity.clone(),
                    "offset",
                    2.5,
                    SlotResolution::Resolved,
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteRuleOverride { target }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteRuleOverride(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Override(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Override(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::RuleOverrideState {
            target: identity.clone(),
            parameter: "offset".to_owned(),
            value: 2.5,
            health: SlotResolution::Resolved,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().override_by_id(target).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let restored = store.current().override_by_id(target).unwrap().clone();
    assert_eq!(restored.target, identity);
    assert_eq!(restored.parameter, "offset");
    assert_eq!(restored.value(), 2.5);
    assert_eq!(restored.health, SlotResolution::Resolved);
}

#[test]
fn gate_d_delete_rule_override_rejects_denied_missing_and_stale_override() {
    let mut store = seed();
    let rule = NodeId(30);
    let target = 31;
    let identity = ketchup_core::document::DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "override source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            },
            CanonicalCommand::UpsertOverride(
                ketchup_core::document::CanonicalOverride::new(
                    target,
                    identity.clone(),
                    "offset",
                    2.5,
                    SlotResolution::Resolved,
                )
                .unwrap(),
            ),
        ]))
        .unwrap();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateRuleOverride],
                ),
                intent: WorkflowIntent::DeleteRuleOverride { target },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteRuleOverride
        ))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteRuleOverride { target: 999 }),
        )
        .is_err()
    );

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteRuleOverride { target }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertOverride(
            ketchup_core::document::CanonicalOverride::new(
                target,
                identity,
                "offset",
                3.0,
                SlotResolution::Resolved,
            )
            .unwrap(),
        )]))
        .unwrap();
    assert!(store.commit_verified_proposal(&proposal).is_err());
    assert_eq!(store.current().override_by_id(target).unwrap().value(), 3.0);
}

#[test]
fn gate_d_create_feature_parameter_binding_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let derived_from = ketchup_core::document::DerivedIdentity::new(
        RULE_OUTPUTS,
        SlotPath::new(vec![
            SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateFeatureParameterBinding {
            target,
            rule: RULE_OUTPUTS,
            output_port: "result".to_owned(),
            semantic_key: "left".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreateFeatureParameterBinding(target)
    );
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::FeatureParameterBinding(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Feature(EXTRUSION),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(RULE_OUTPUTS),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::FeatureParameterBinding(
            target
        )])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::FeatureParameterBindingState {
            target,
            derived_from: derived_from.clone(),
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let created = store
        .current()
        .feature_parameter_binding(target)
        .unwrap()
        .clone();
    assert_eq!(created.target, target);
    assert_eq!(created.derived_from, derived_from);
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().feature_parameter_binding(target).is_none());
}

#[test]
fn gate_d_create_feature_parameter_binding_rejects_denied_invalid_occupied_and_stale() {
    let mut store = seed();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let intent = || WorkflowIntent::CreateFeatureParameterBinding {
        target,
        rule: RULE_OUTPUTS,
        output_port: "result".to_owned(),
        semantic_key: "left".to_owned(),
    };
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateRuleOverride],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateFeatureParameterBinding,
        ))
    );
    for invalid in [
        WorkflowIntent::CreateFeatureParameterBinding {
            target: FeatureParameterTarget {
                feature_id: EXTRUSION,
                slot: FeatureParameterSlot::ProfileWidth,
            },
            rule: RULE_OUTPUTS,
            output_port: "result".to_owned(),
            semantic_key: "left".to_owned(),
        },
        WorkflowIntent::CreateFeatureParameterBinding {
            target: FeatureParameterTarget {
                feature_id: FeatureId(999),
                slot: FeatureParameterSlot::Height,
            },
            rule: RULE_OUTPUTS,
            output_port: "result".to_owned(),
            semantic_key: "left".to_owned(),
        },
        WorkflowIntent::CreateFeatureParameterBinding {
            target,
            rule: NodeId(999),
            output_port: "result".to_owned(),
            semantic_key: "left".to_owned(),
        },
        WorkflowIntent::CreateFeatureParameterBinding {
            target,
            rule: RULE_OUTPUTS,
            output_port: "result".to_owned(),
            semantic_key: "missing".to_owned(),
        },
    ] {
        assert!(propose_intent(&store, IntentRequest::m7a(invalid)).is_err());
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("21").unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(store.current().feature_parameter_binding(target).is_none());

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target,
                    derived_from: ketchup_core::document::DerivedIdentity::new(
                        RULE_OUTPUTS,
                        SlotPath::new(vec![
                            SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
                },
            ),
        ]))
        .unwrap();
    let occupied_digest = store.current().canonical_digest();
    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), occupied_digest);
}

#[test]
fn gate_d_delete_feature_parameter_binding_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let derived_from = ketchup_core::document::DerivedIdentity::new(
        RULE_OUTPUTS,
        SlotPath::new(vec![
            SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target,
                    derived_from: derived_from.clone(),
                },
            ),
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteFeatureParameterBinding { target }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeleteFeatureParameterBinding(target)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::FeatureParameterBinding(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::FeatureParameterBinding(
            target
        )])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::FeatureParameterBindingState {
            target,
            derived_from: derived_from.clone(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().feature_parameter_binding(target).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store
            .current()
            .feature_parameter_binding(target)
            .unwrap()
            .derived_from,
        derived_from
    );
}

#[test]
fn gate_d_delete_feature_parameter_binding_rejects_denied_missing_and_stale() {
    let mut store = seed();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let derived_from = ketchup_core::document::DerivedIdentity::new(
        RULE_OUTPUTS,
        SlotPath::new(vec![
            SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target,
                    derived_from,
                },
            ),
        ]))
        .unwrap();
    let intent = || WorkflowIntent::DeleteFeatureParameterBinding { target };
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateFeatureParameterBinding],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteFeatureParameterBinding,
        ))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteFeatureParameterBinding {
                target: FeatureParameterTarget {
                    feature_id: EXTRUSION,
                    slot: FeatureParameterSlot::ProfileWidth,
                },
            }),
        )
        .is_err()
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    let replacement_rule = NodeId(999);
    let replacement_identity = ketchup_core::document::DerivedIdentity::new(
        replacement_rule,
        SlotPath::new(vec![
            SlotSegment::new(replacement_rule, "result", "replacement").unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateRuleNode {
                id: replacement_rule,
                name: "Replacement binding source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(replacement_rule, "result", "replacement").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target,
                    derived_from: replacement_identity.clone(),
                },
            ),
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store
            .current()
            .feature_parameter_binding(target)
            .unwrap()
            .derived_from,
        replacement_identity
    );

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeatureParameterBinding { target },
        ]))
        .unwrap();
    let removed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), removed_digest);
    assert!(store.current().feature_parameter_binding(target).is_none());
}

#[test]
fn gate_d_recompute_feature_parameter_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target,
                    derived_from: ketchup_core::document::DerivedIdentity::new(
                        RULE_OUTPUTS,
                        SlotPath::new(vec![
                            SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
                },
            ),
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::RecomputeFeatureParameter { target }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::RecomputeFeatureParameter(target)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(EXTRUSION),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Feature(EXTRUSION)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(Dimension::from_decimal("20").unwrap())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(Dimension::from_decimal("1200").unwrap())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "1200" && height.millimetres() == 1200.0
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(matches!(
        store.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "20" && height.millimetres() == 20.0
    ));
}

#[test]
fn gate_d_recompute_feature_parameter_rejects_denied_missing_multiple_and_stale() {
    let mut store = seed();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let intent = || WorkflowIntent::RecomputeFeatureParameter { target };
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteFeatureParameterBinding],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::RecomputeFeatureParameter,
        ))
    );
    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), digest_before);

    let binding = |target| ketchup_core::document::FeatureParameterBinding {
        target,
        derived_from: ketchup_core::document::DerivedIdentity::new(
            RULE_OUTPUTS,
            SlotPath::new(vec![
                SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
            ])
            .unwrap(),
        )
        .unwrap(),
    };
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(binding(target)),
        ]))
        .unwrap();
    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetNodeExpression {
                id: RULE_OUTPUTS,
                expression: "$2".to_owned(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(matches!(
        store.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));

    let second_target = FeatureParameterTarget {
        feature_id: BOTTLE_SHELL,
        slot: FeatureParameterSlot::Thickness,
    };
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(binding(second_target)),
        ]))
        .unwrap();
    let multiple_digest = store.current().canonical_digest();
    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), multiple_digest);
}

#[test]
fn gate_d_create_evaluator_rule_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = NodeId(24);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateEvaluatorRule {
            target,
            name: "Reviewed rule".to_owned(),
            expression: "$1 + $2".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorRule(target));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::EvaluatorNode(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(RULE),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(EXPRESSION_INPUT),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::EvaluatorNode(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorRuleState {
            name: "Reviewed rule".to_owned(),
            expression: "$1 + $2".to_owned(),
            dependencies: vec![RULE, EXPRESSION_INPUT],
            input_ports: Vec::new(),
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: Vec::new(),
            override_parameters: Vec::new(),
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let snapshot = store.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed rule");
    assert_eq!(created.kind().source(), "$1 + $2");
    assert_eq!(created.dependencies(), &[RULE, EXPRESSION_INPUT]);
    assert!(created.input_ports().is_empty());
    assert_eq!(
        created.output_ports(),
        &[PortSpec::number("result").unwrap()]
    );
    assert!(created.allowed_parameters().is_empty());
    let EvaluatorNodeKind::Rule { outputs, .. } = created.kind() else {
        panic!("created evaluator must be a rule");
    };
    assert!(outputs.is_empty());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().evaluator_node(target).is_none());
}

#[test]
fn gate_d_create_evaluator_rule_rejects_denied_invalid_and_stale_dependency() {
    let mut store = seed();
    let target = NodeId(24);
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateEvaluatorExpression],
                ),
                intent: WorkflowIntent::CreateEvaluatorRule {
                    target,
                    name: "Denied".to_owned(),
                    expression: "$1".to_owned(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateEvaluatorRule
        ))
    );
    for intent in [
        WorkflowIntent::CreateEvaluatorRule {
            target: NodeId(0),
            name: "Invalid ID".to_owned(),
            expression: "$1".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorRule {
            target,
            name: String::new(),
            expression: "$1".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorRule {
            target: RULE,
            name: "Duplicate".to_owned(),
            expression: "$2".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorRule {
            target,
            name: "Malformed".to_owned(),
            expression: "$1 +".to_owned(),
        },
        WorkflowIntent::CreateEvaluatorRule {
            target,
            name: "Missing dependency".to_owned(),
            expression: "$999".to_owned(),
        },
    ] {
        assert!(matches!(
            propose_intent(&store, IntentRequest::m7a(intent)),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateEvaluatorRule {
            target,
            name: "Reviewed rule".to_owned(),
            expression: "$1 + $2".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: EXPRESSION_INPUT,
                dimension: Dimension::from_decimal("9").unwrap(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(store.current().evaluator_node(target).is_none());
}

#[test]
fn gate_d_delete_definition_is_typed_observational_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteDefinition {
            target: SECOND_DEFINITION,
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeleteDefinition(SECOND_DEFINITION)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Definition(SECOND_DEFINITION)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([
            AuthoritativeDependency::Definition(SECOND_DEFINITION,)
        ])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::DefinitionUsers(SECOND_DEFINITION))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::DefinitionState {
            name: "Housing".to_owned(),
            feature_ids: Vec::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().definition(SECOND_DEFINITION).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    let restored = store.current();
    let definition = restored.definition(SECOND_DEFINITION).unwrap();
    assert_eq!(definition.name(), "Housing");
    assert!(definition.feature_ids().is_empty());
    assert!(definition.local_occurrence_ids().is_empty());
    assert!(definition.local_group_ids().is_empty());
}

#[test]
fn gate_d_delete_definition_rejects_denied_missing_nonempty_used_and_stale_users() {
    let store = seed();
    let digest_before = store.current().canonical_digest();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateDefinition],
                ),
                intent: WorkflowIntent::DeleteDefinition {
                    target: SECOND_DEFINITION,
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteDefinition
        ))
    );
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteDefinition {
                target: DefinitionId(999),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteDefinition { target: DEFINITION })
        ),
        Err(IntentError::Canonical(
            ketchup_core::document::CanonicalError::DefinitionNotEmpty(DEFINITION)
        ))
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let mut used_store = seed();
    used_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(99),
                definition_id: SECOND_DEFINITION,
                name: "Housing occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let used_digest = used_store.current().canonical_digest();
    assert!(matches!(
        propose_intent(
            &used_store,
            IntentRequest::m7a(WorkflowIntent::DeleteDefinition {
                target: SECOND_DEFINITION,
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));
    assert_eq!(used_store.current().canonical_digest(), used_digest);

    let mut stale_store = seed();
    let proposal = propose_intent(
        &stale_store,
        IntentRequest::m7a(WorkflowIntent::DeleteDefinition {
            target: SECOND_DEFINITION,
        }),
    )
    .unwrap();
    stale_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(99),
                definition_id: SECOND_DEFINITION,
                name: "Concurrent housing occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let changed_digest = stale_store.current().canonical_digest();
    assert!(matches!(
        stale_store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale_store.current().canonical_digest(), changed_digest);
    assert!(
        stale_store
            .current()
            .definition(SECOND_DEFINITION)
            .is_some()
    );
}

#[test]
fn gate_d_clone_profile_definition_is_typed_observational_and_undoable() {
    let mut store = seed();
    let source_feature = FeatureId(90);
    let occurrence = OccurrenceId(91);
    let new_definition = DefinitionId(92);
    let new_feature = FeatureId(93);
    let points_mm = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0]];
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: source_feature,
                definition_id: SECOND_DEFINITION,
                name: "Source profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: points_mm.clone(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: SECOND_DEFINITION,
                name: "Source occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CloneProfileDefinitionAndRepoint {
            target: occurrence,
            source_definition: SECOND_DEFINITION,
            source_feature,
            new_definition,
            new_feature,
            name: "Independent profile".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::CloneProfileDefinitionAndRepoint(occurrence)
    );
    assert_eq!(proposal.cost().write_targets, 3);
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Occurrence(occurrence),
        )
    ));
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Definition(SECOND_DEFINITION),
        )
    ));
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(source_feature),
        )
    ));
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Definition(new_definition),
        )
    ));
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Feature(new_feature),
        )
    ));
    let diff = |target| {
        proposal
            .authoritative_diff()
            .iter()
            .find(|entry| entry.target == target)
            .unwrap()
    };
    let definition_diff = diff(AuthoritativeDependency::Definition(new_definition));
    assert_eq!(definition_diff.before, ProposalValue::Missing);
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionState {
            name: "Independent profile".to_owned(),
            feature_ids: vec![new_feature],
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }
    );
    let feature_diff = diff(AuthoritativeDependency::Feature(new_feature));
    assert_eq!(feature_diff.before, ProposalValue::Missing);
    assert_eq!(
        feature_diff.after,
        ProposalValue::ProfileFeatureState {
            definition: new_definition,
            name: "Source profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    let occurrence_diff = diff(AuthoritativeDependency::Occurrence(occurrence));
    assert_eq!(
        occurrence_diff.before,
        ProposalValue::OccurrenceState {
            definition: SECOND_DEFINITION,
            name: "Source occurrence".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(
        occurrence_diff.after,
        ProposalValue::OccurrenceState {
            definition: new_definition,
            name: "Source occurrence".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        store
            .current()
            .occurrence(occurrence)
            .unwrap()
            .definition_id(),
        new_definition
    );
    assert_eq!(
        store
            .current()
            .definition(new_definition)
            .unwrap()
            .feature_ids(),
        &[new_feature]
    );
    assert!(matches!(
        store.current().feature(new_feature).unwrap().kind(),
        FeatureKind::Profile { points_mm: cloned } if cloned == &points_mm
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store
            .current()
            .occurrence(occurrence)
            .unwrap()
            .definition_id(),
        SECOND_DEFINITION
    );
    assert!(store.current().definition(new_definition).is_none());
    assert!(store.current().feature(new_feature).is_none());
}

#[test]
fn gate_d_clone_profile_definition_rejects_denied_unsupported_stale_and_claimed() {
    let seed_clone = || {
        let mut store = seed();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateFeature {
                    id: FeatureId(90),
                    definition_id: SECOND_DEFINITION,
                    name: "Source profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]],
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(91),
                    definition_id: SECOND_DEFINITION,
                    name: "Source occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        store
    };
    let intent = || WorkflowIntent::CloneProfileDefinitionAndRepoint {
        target: OccurrenceId(91),
        source_definition: SECOND_DEFINITION,
        source_feature: FeatureId(90),
        new_definition: DefinitionId(92),
        new_feature: FeatureId(93),
        name: "Independent profile".to_owned(),
    };

    let store = seed_clone();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateOccurrence],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M18C_CLONE_PROFILE_DEFINITION,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CloneProfileDefinitionAndRepoint
        ))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CloneProfileDefinitionAndRepoint {
                target: OCCURRENCE,
                source_definition: DEFINITION,
                source_feature: PROFILE,
                new_definition: DefinitionId(92),
                new_feature: FeatureId(93),
                name: "Unsupported multi-feature source".to_owned(),
            }),
        )
        .is_err()
    );

    let mut stale_store = seed_clone();
    let stale = propose_intent(&stale_store, IntentRequest::m7a(intent())).unwrap();
    stale_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: FeatureId(90),
                points_mm: vec![[0.0, 0.0], [15.0, 0.0], [15.0, 8.0], [0.0, 8.0]],
            },
        ]))
        .unwrap();
    let stale_digest = stale_store.current().canonical_digest();
    assert!(matches!(
        stale_store.commit_verified_proposal(&stale),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale_store.current().canonical_digest(), stale_digest);
    assert!(stale_store.current().definition(DefinitionId(92)).is_none());

    let mut binding_store = seed_clone();
    let binding_proposal = propose_intent(&binding_store, IntentRequest::m7a(intent())).unwrap();
    let derived_from = ketchup_core::document::DerivedIdentity::new(
        RULE_OUTPUTS,
        SlotPath::new(vec![
            SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    binding_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target: FeatureParameterTarget {
                        feature_id: FeatureId(90),
                        slot: FeatureParameterSlot::ProfileWidth,
                    },
                    derived_from,
                },
            ),
        ]))
        .unwrap();
    let binding_digest = binding_store.current().canonical_digest();
    assert!(matches!(
        binding_store.commit_verified_proposal(&binding_proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(binding_store.current().canonical_digest(), binding_digest);
    assert!(
        binding_store
            .current()
            .definition(DefinitionId(92))
            .is_none()
    );

    let mut claimed_store = seed_clone();
    let claimed = propose_intent(&claimed_store, IntentRequest::m7a(intent())).unwrap();
    claimed_store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(92),
                name: "Concurrent claim".to_owned(),
            },
        ]))
        .unwrap();
    let claimed_digest = claimed_store.current().canonical_digest();
    assert!(matches!(
        claimed_store.commit_verified_proposal(&claimed),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(claimed_store.current().canonical_digest(), claimed_digest);
    assert_eq!(
        claimed_store
            .current()
            .definition(DefinitionId(92))
            .unwrap()
            .name(),
        "Concurrent claim"
    );
    assert!(propose_intent(&claimed_store, IntentRequest::m7a(intent())).is_err());
}

#[test]
fn gate_d_convert_empty_group_is_typed_observational_and_undoable() {
    let mut store = seed();
    let new_definition = DefinitionId(90);
    let new_occurrence = OccurrenceId(91);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::ConvertEmptyGroupToComponent {
            target: GROUP,
            new_definition,
            new_occurrence,
            name: "Assembly component".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::ConvertEmptyGroupToComponent(GROUP)
    );
    assert_eq!(proposal.cost().write_targets, 3);
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::GroupSubtree(GROUP),
        )
    ));
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Definition(new_definition),
        )
    ));
    assert!(proposal.assumptions().contains(
        &ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Occurrence(new_occurrence),
        )
    ));
    let diff = |target| {
        proposal
            .authoritative_diff()
            .iter()
            .find(|entry| entry.target == target)
            .unwrap()
    };
    assert_eq!(
        diff(AuthoritativeDependency::GroupSubtree(GROUP)).before,
        ProposalValue::GroupState {
            name: "Assembly".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }
    );
    assert_eq!(
        diff(AuthoritativeDependency::GroupSubtree(GROUP)).after,
        ProposalValue::Missing
    );
    assert_eq!(
        diff(AuthoritativeDependency::Definition(new_definition)).after,
        ProposalValue::DefinitionState {
            name: "Assembly component".to_owned(),
            feature_ids: Vec::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }
    );
    assert_eq!(
        diff(AuthoritativeDependency::Occurrence(new_occurrence)).after,
        ProposalValue::OccurrenceState {
            definition: new_definition,
            name: "Assembly component".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().group(GROUP).is_none());
    assert!(store.current().definition(new_definition).is_some());
    assert_eq!(
        store
            .current()
            .occurrence(new_occurrence)
            .unwrap()
            .definition_id(),
        new_definition
    );
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().group(GROUP).is_some());
    assert!(store.current().definition(new_definition).is_none());
    assert!(store.current().occurrence(new_occurrence).is_none());
}

#[test]
fn gate_d_convert_empty_group_rejects_denied_nonempty_stale_and_claimed() {
    let intent = || WorkflowIntent::ConvertEmptyGroupToComponent {
        target: GROUP,
        new_definition: DefinitionId(90),
        new_occurrence: OccurrenceId(91),
        name: "Assembly component".to_owned(),
    };
    let store = seed();
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::CreateOccurrence],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M18C_CLONE_PROFILE_DEFINITION,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::ConvertEmptyGroupToComponent
        ))
    );

    let mut nonempty = seed();
    nonempty
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceParent {
                id: OCCURRENCE,
                parent: Some(GROUP),
            },
        ]))
        .unwrap();
    let nonempty_digest = nonempty.current().canonical_digest();
    assert!(propose_intent(&nonempty, IntentRequest::m7a(intent())).is_err());
    assert_eq!(nonempty.current().canonical_digest(), nonempty_digest);

    let mut stale = seed();
    let proposal = propose_intent(&stale, IntentRequest::m7a(intent())).unwrap();
    stale
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: GROUP,
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let stale_digest = stale.current().canonical_digest();
    assert!(matches!(
        stale.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stale.current().canonical_digest(), stale_digest);
    assert!(stale.current().group(GROUP).is_some());

    let mut claimed = seed();
    let proposal = propose_intent(&claimed, IntentRequest::m7a(intent())).unwrap();
    claimed
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(90),
                name: "Concurrent claim".to_owned(),
            },
        ]))
        .unwrap();
    let claimed_digest = claimed.current().canonical_digest();
    assert!(matches!(
        claimed.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(claimed.current().canonical_digest(), claimed_digest);
    assert!(claimed.current().group(GROUP).is_some());
}

fn reviewed_joint(id: JointId, max_x: f64) -> CanonicalJoint {
    let participant = |key| {
        ketchup_core::document::DerivedIdentity::new(
            RULE_OUTPUTS,
            SlotPath::new(vec![SlotSegment::new(RULE_OUTPUTS, "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    CanonicalJoint::new(
        id,
        participant("left"),
        participant("right"),
        Aabb::bounded_volume([0.0, 0.0, 0.0], [max_x, 2.0, 3.0]).unwrap(),
    )
    .unwrap()
}

#[test]
fn gate_d_create_joint_is_typed_observational_and_undoable() {
    let mut store = seed();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: RULE_OUTPUTS,
            outputs: vec![rule_output("left"), rule_output("right")],
        }]))
        .unwrap();
    let target = JointId(98);
    let participant = |key| {
        ketchup_core::document::DerivedIdentity::new(
            RULE_OUTPUTS,
            SlotPath::new(vec![SlotSegment::new(RULE_OUTPUTS, "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let participant_a = participant("left");
    let participant_b = participant("right");
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateJoint {
            target,
            participant_a: participant_a.clone(),
            participant_b: participant_b.clone(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateJoint(target));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::Joint(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::EvaluatorNode(RULE_OUTPUTS),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Joint(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::JointState {
            participant_a: participant_a.clone(),
            participant_b: participant_b.clone(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let created = CanonicalJoint::new(
        target,
        participant_a,
        participant_b,
        Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
    )
    .unwrap();
    assert_eq!(store.current().joint(target), Some(&created));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().joint(target).is_none());
}

#[test]
fn gate_d_create_joint_rejects_denied_invalid_unresolved_reuse_and_stale_claim() {
    let mut store = seed();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: RULE_OUTPUTS,
            outputs: vec![rule_output("left"), rule_output("right")],
        }]))
        .unwrap();
    let target = JointId(99);
    let participant = |key| {
        ketchup_core::document::DerivedIdentity::new(
            RULE_OUTPUTS,
            SlotPath::new(vec![SlotSegment::new(RULE_OUTPUTS, "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let intent = || WorkflowIntent::CreateJoint {
        target,
        participant_a: participant("left"),
        participant_b: participant("right"),
        volume_min: [0.0, 0.0, 0.0],
        volume_max: [1.0, 2.0, 3.0],
    };

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteJoint],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::CreateJoint))
    );
    let digest_before = store.current().canonical_digest();
    for invalid in [
        WorkflowIntent::CreateJoint {
            target: JointId(0),
            participant_a: participant("left"),
            participant_b: participant("right"),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
        },
        WorkflowIntent::CreateJoint {
            target,
            participant_a: participant("left"),
            participant_b: participant("left"),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
        },
        WorkflowIntent::CreateJoint {
            target,
            participant_a: participant("left"),
            participant_b: participant("missing"),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
        },
        WorkflowIntent::CreateJoint {
            target,
            participant_a: participant("left"),
            participant_b: participant("right"),
            volume_min: [2.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
        },
    ] {
        assert!(propose_intent(&store, IntentRequest::m7a(invalid)).is_err());
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let participant_proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: RULE_OUTPUTS,
            outputs: vec![
                rule_output("center"),
                rule_output("left"),
                rule_output("right"),
            ],
        }]))
        .unwrap();
    let participant_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&participant_proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), participant_digest);
    assert!(store.current().joint(target).is_none());

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    let replacement = reviewed_joint(target, 4.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            replacement.clone(),
        )]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().joint(target), Some(&replacement));
    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), changed_digest);
}

#[test]
fn gate_d_delete_joint_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = JointId(80);
    let joint = reviewed_joint(target, 1.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            joint.clone(),
        )]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteJoint { target }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteJoint(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Joint(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Joint(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::JointState {
            participant_a: joint.participant_a().clone(),
            participant_b: joint.participant_b().clone(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().joint(target).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(store.current().joint(target), Some(&joint));
}

#[test]
fn gate_d_delete_joint_rejects_denied_missing_and_stale_replacement() {
    let mut store = seed();
    let target = JointId(81);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            reviewed_joint(target, 1.0),
        )]))
        .unwrap();
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::RecomputeFeatureParameter],
                ),
                intent: WorkflowIntent::DeleteJoint { target },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::DeleteJoint))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteJoint {
                target: JointId(999),
            }),
        )
        .is_err()
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteJoint { target }),
    )
    .unwrap();
    let replacement = reviewed_joint(target, 4.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            replacement.clone(),
        )]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().joint(target), Some(&replacement));
}

fn reviewed_space(id: SpaceId, purpose: &str, max_x: f64) -> CanonicalSpace {
    CanonicalSpace::new(
        id,
        purpose,
        Aabb::bounded_volume([0.0, 0.0, 0.0], [max_x, 2.0, 3.0]).unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn gate_d_create_space_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = SpaceId(92);
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateSpace {
            target,
            purpose: "service access".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateSpace(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetMissing(
            AuthoritativeDependency::Space(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Space(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::SpaceState {
            purpose: "service access".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            adjacent_to: Vec::new(),
            accessible_to: Vec::new(),
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let created = CanonicalSpace::new(
        target,
        "service access",
        Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(store.current().space(target), Some(&created));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().space(target).is_none());
}

#[test]
fn gate_d_create_space_rejects_denied_invalid_reuse_and_stale_claim() {
    let mut store = seed();
    let target = SpaceId(93);
    let intent = || WorkflowIntent::CreateSpace {
        target,
        purpose: "equipment".to_owned(),
        volume_min: [0.0, 0.0, 0.0],
        volume_max: [2.0, 3.0, 4.0],
    };

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteSpace],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::CreateSpace))
    );
    let digest_before = store.current().canonical_digest();
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CreateSpace {
                target,
                purpose: "".to_owned(),
                volume_min: [0.0, 0.0, 0.0],
                volume_max: [1.0, 1.0, 1.0],
            }),
        )
        .is_err()
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::CreateSpace {
                target,
                purpose: "invalid volume".to_owned(),
                volume_min: [2.0, 0.0, 0.0],
                volume_max: [1.0, 1.0, 1.0],
            }),
        )
        .is_err()
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    let replacement = reviewed_space(target, "concurrent", 5.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            replacement.clone(),
        )]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().space(target), Some(&replacement));
    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().space(target), Some(&replacement));
}

#[test]
fn gate_d_delete_space_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = SpaceId(82);
    let space = reviewed_space(target, "service access", 1.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            space.clone(),
        )]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteSpace { target }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteSpace(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Space(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Space(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::SpaceState {
            purpose: "service access".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            adjacent_to: Vec::new(),
            accessible_to: Vec::new(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().space(target).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(store.current().space(target), Some(&space));
}

#[test]
fn gate_d_delete_space_rejects_denied_missing_and_stale_replacement() {
    let mut store = seed();
    let target = SpaceId(83);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            reviewed_space(target, "clearance", 1.0),
        )]))
        .unwrap();
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteJoint],
                ),
                intent: WorkflowIntent::DeleteSpace { target },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(IntentCapability::DeleteSpace))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteSpace {
                target: SpaceId(999),
            }),
        )
        .is_err()
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteSpace { target }),
    )
    .unwrap();
    let replacement = reviewed_space(target, "replaced", 4.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            replacement.clone(),
        )]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().space(target), Some(&replacement));
}

fn reviewed_clearance(
    id: ClearanceVolumeId,
    owner: SpaceId,
    reason: &str,
    max_x: f64,
) -> CanonicalClearanceVolume {
    CanonicalClearanceVolume::new(
        id,
        ClearanceOwner::Space(owner),
        reason,
        Aabb::bounded_volume([0.0, 0.0, 0.0], [max_x, 2.0, 3.0]).unwrap(),
        TolerancePolicy::new(0.01).unwrap(),
        ClearanceSeverity::Required,
        None,
    )
    .unwrap()
}

#[test]
fn gate_d_create_clearance_volume_is_typed_observational_and_undoable() {
    let mut store = seed();
    let owner = SpaceId(94);
    let target = ClearanceVolumeId(95);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            reviewed_space(owner, "equipment", 5.0),
        )]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreateClearanceVolume {
            target,
            owner,
            reason: "service envelope".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Required,
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::CreateClearanceVolume(target));
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::ClearanceVolume(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Space(owner),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::ClearanceVolume(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ClearanceVolumeState {
            owner: ClearanceOwner::Space(owner),
            reason: "service envelope".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            coordinate_frame: ClearanceCoordinateFrame::World,
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Required,
            derived_from: None,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let created = CanonicalClearanceVolume::new(
        target,
        ClearanceOwner::Space(owner),
        "service envelope",
        Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
        TolerancePolicy::new(0.01).unwrap(),
        ClearanceSeverity::Required,
        None,
    )
    .unwrap();
    assert_eq!(store.current().clearance_volume(target), Some(&created));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().clearance_volume(target).is_none());
}

#[test]
fn gate_d_create_clearance_volume_rejects_denied_invalid_missing_owner_reuse_and_stale_claim() {
    let mut store = seed();
    let owner = SpaceId(96);
    let target = ClearanceVolumeId(97);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            reviewed_space(owner, "equipment", 5.0),
        )]))
        .unwrap();
    let intent = || WorkflowIntent::CreateClearanceVolume {
        target,
        owner,
        reason: "access".to_owned(),
        volume_min: [0.0, 0.0, 0.0],
        volume_max: [1.0, 2.0, 3.0],
        tolerance_mm: 0.01,
        severity: ClearanceSeverity::Advisory,
    };

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteClearanceVolume],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreateClearanceVolume
        ))
    );
    let digest_before = store.current().canonical_digest();
    for invalid in [
        WorkflowIntent::CreateClearanceVolume {
            target: ClearanceVolumeId(0),
            owner,
            reason: "reserved target".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Advisory,
        },
        WorkflowIntent::CreateClearanceVolume {
            target,
            owner,
            reason: String::new(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Advisory,
        },
        WorkflowIntent::CreateClearanceVolume {
            target,
            owner,
            reason: "invalid bounds".to_owned(),
            volume_min: [2.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Advisory,
        },
        WorkflowIntent::CreateClearanceVolume {
            target,
            owner,
            reason: "invalid tolerance".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            tolerance_mm: f64::NAN,
            severity: ClearanceSeverity::Advisory,
        },
        WorkflowIntent::CreateClearanceVolume {
            target,
            owner: SpaceId(999),
            reason: "missing owner".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Advisory,
        },
    ] {
        assert!(propose_intent(&store, IntentRequest::m7a(invalid)).is_err());
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let owner_proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    let owner_replacement = reviewed_space(owner, "concurrent owner", 6.0);
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            owner_replacement.clone(),
        )]))
        .unwrap();
    let owner_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&owner_proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), owner_digest);
    assert_eq!(store.current().space(owner), Some(&owner_replacement));
    assert!(store.current().clearance_volume(target).is_none());

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    let replacement = reviewed_clearance(target, owner, "concurrent", 4.0);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClearanceVolume(replacement.clone()),
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().clearance_volume(target), Some(&replacement));
    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), changed_digest);
}

#[test]
fn gate_d_delete_clearance_volume_is_typed_observational_and_undoable() {
    let mut store = seed();
    let owner = SpaceId(84);
    let target = ClearanceVolumeId(85);
    let clearance = reviewed_clearance(target, owner, "service envelope", 1.0);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertSpace(reviewed_space(owner, "equipment", 5.0)),
            CanonicalCommand::UpsertClearanceVolume(clearance.clone()),
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteClearanceVolume { target }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::DeleteClearanceVolume(target));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::ClearanceVolume(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::ClearanceVolume(target)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ClearanceVolumeState {
            owner: ClearanceOwner::Space(owner),
            reason: "service envelope".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            coordinate_frame: ClearanceCoordinateFrame::World,
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Required,
            derived_from: None,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().clearance_volume(target).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(store.current().clearance_volume(target), Some(&clearance));
}

#[test]
fn gate_d_delete_clearance_volume_rejects_denied_missing_and_stale_replacement() {
    let mut store = seed();
    let owner = SpaceId(86);
    let target = ClearanceVolumeId(87);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertSpace(reviewed_space(owner, "equipment", 5.0)),
            CanonicalCommand::UpsertClearanceVolume(reviewed_clearance(
                target, owner, "access", 1.0,
            )),
        ]))
        .unwrap();
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteSpace],
                ),
                intent: WorkflowIntent::DeleteClearanceVolume { target },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeleteClearanceVolume
        ))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeleteClearanceVolume {
                target: ClearanceVolumeId(999),
            }),
        )
        .is_err()
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeleteClearanceVolume { target }),
    )
    .unwrap();
    let replacement = reviewed_clearance(target, owner, "replaced", 4.0);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClearanceVolume(replacement.clone()),
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(store.current().clearance_volume(target), Some(&replacement));
}

#[test]
fn gate_d_delete_persistent_dimension_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = PersistentDimensionId(88);
    let dimension_target = PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
        feature_id: PROFILE,
        slot: FeatureParameterSlot::ProfileWidth,
    });
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
    let dimension = PersistentDimension::new(
        target,
        "Reviewed width",
        dimension_target.clone(),
        presentation,
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(dimension.clone()),
        ]))
        .unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeletePersistentDimension { target }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeletePersistentDimension(target)
    );
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_core::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::PersistentDimension(target),
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::PersistentDimension(target),])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::PersistentDimensionState {
            name: "Reviewed width".to_owned(),
            target: dimension_target,
            presentation,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(store.current().persistent_dimension(target).is_none());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert_eq!(
        store.current().persistent_dimension(target),
        Some(&dimension)
    );
}

#[test]
fn gate_d_delete_persistent_dimension_rejects_denied_missing_and_stale_replacement() {
    let mut store = seed();
    let target = PersistentDimensionId(89);
    let dimension_target = PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
        feature_id: PROFILE,
        slot: FeatureParameterSlot::ProfileHeight,
    });
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    target,
                    "Reviewed height",
                    dimension_target.clone(),
                    DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 1).unwrap(),
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    let digest_before = store.current().canonical_digest();

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeleteClearanceVolume],
                ),
                intent: WorkflowIntent::DeletePersistentDimension { target },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::DeletePersistentDimension
        ))
    );
    assert!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::DeletePersistentDimension {
                target: PersistentDimensionId(999),
            }),
        )
        .is_err()
    );
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::DeletePersistentDimension { target }),
    )
    .unwrap();
    let replacement = PersistentDimension::new(
        target,
        "Replacement height",
        dimension_target,
        DimensionPresentation::new(DimensionDisplayUnit::Inches, 3).unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(replacement.clone()),
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().persistent_dimension(target),
        Some(&replacement)
    );
}

#[test]
fn gate_d_create_persistent_dimension_is_typed_observational_and_undoable() {
    let mut store = seed();
    let target = PersistentDimensionId(90);
    let dimension_target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::CreatePersistentDimension {
            target,
            name: "Reviewed height".to_owned(),
            dimension_target,
            presentation,
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreatePersistentDimension(target)
    );
    assert_eq!(
        proposal.assumptions(),
        &[
            ketchup_core::document::ProposalAssumption::TargetMissing(
                AuthoritativeDependency::PersistentDimension(target),
            ),
            ketchup_core::document::ProposalAssumption::TargetExists(
                AuthoritativeDependency::Feature(EXTRUSION),
            ),
        ]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::PersistentDimension(target)])
    );
    assert!(
        proposal
            .authoritative_dependencies()
            .contains(&AuthoritativeDependency::Feature(EXTRUSION))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::PersistentDimensionState {
            name: "Reviewed height".to_owned(),
            target: PersistentDimensionTarget::FeatureParameter(dimension_target),
            presentation,
        }
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    let created = PersistentDimension::new(
        target,
        "Reviewed height",
        PersistentDimensionTarget::FeatureParameter(dimension_target),
        presentation,
    )
    .unwrap();
    assert_eq!(store.current().persistent_dimension(target), Some(&created));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(store.current().persistent_dimension(target).is_none());
}

#[test]
fn gate_d_create_persistent_dimension_rejects_denied_reuse_and_stale_claim() {
    let mut store = seed();
    let target = PersistentDimensionId(91);
    let dimension_target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 1).unwrap();
    let intent = || WorkflowIntent::CreatePersistentDimension {
        target,
        name: "Reviewed height".to_owned(),
        dimension_target,
        presentation,
    };

    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::DeletePersistentDimension],
                ),
                intent: intent(),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::CreatePersistentDimension
        ))
    );
    let digest_before = store.current().canonical_digest();
    for invalid_target in [
        FeatureParameterTarget {
            feature_id: FeatureId(999),
            slot: FeatureParameterSlot::Height,
        },
        FeatureParameterTarget {
            feature_id: EXTRUSION,
            slot: FeatureParameterSlot::Thickness,
        },
    ] {
        assert!(
            propose_intent(
                &store,
                IntentRequest::m7a(WorkflowIntent::CreatePersistentDimension {
                    target,
                    name: "Invalid target".to_owned(),
                    dimension_target: invalid_target,
                    presentation,
                }),
            )
            .is_err()
        );
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(&store, IntentRequest::m7a(intent())).unwrap();
    let replacement = PersistentDimension::new(
        target,
        "Concurrent height",
        PersistentDimensionTarget::FeatureParameter(dimension_target),
        DimensionPresentation::new(DimensionDisplayUnit::Inches, 3).unwrap(),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(replacement.clone()),
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().persistent_dimension(target),
        Some(&replacement)
    );

    assert!(propose_intent(&store, IntentRequest::m7a(intent())).is_err());
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert_eq!(
        store.current().persistent_dimension(target),
        Some(&replacement)
    );
}
