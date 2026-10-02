//! Typed, observational and undoable Assistant review operations.

use super::*;

#[test]
fn assistant_evaluator_rename_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let node = NodeId(20);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: node,
                name: "width".to_owned(),
                dimension: Dimension::from_decimal("600").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::EvaluatorName;
    app.assistant.target_input = node.0.to_string();
    app.assistant.value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = "cabinet width".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::RenameEvaluatorNode(node));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("width".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("cabinet width".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document.current().evaluator_node(node).unwrap().name(),
        "cabinet width"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().evaluator_node(node).unwrap().name(),
        "width"
    );
}

#[test]
fn assistant_evaluator_expression_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let input = NodeId(20);
    let expression = NodeId(21);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: input,
                name: "width".to_owned(),
                dimension: Dimension::from_decimal("600").unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateExpressionNode {
                id: expression,
                name: "double width".to_owned(),
                expression: "$20 * 2".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::EvaluatorExpression;
    app.assistant.target_input = expression.0.to_string();
    app.assistant.value_input = "(".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = "$20 * 3".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetEvaluatorExpression(expression)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("$20 * 2".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("$20 * 3".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .evaluator_node(expression)
            .unwrap()
            .kind()
            .source(),
        "$20 * 3"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .evaluator_node(expression)
            .unwrap()
            .kind()
            .source(),
        "$20 * 2"
    );
}

#[test]
fn assistant_tag_visibility_review_is_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(7);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: tag,
            name: "Hardware".to_owned(),
            visible: true,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::TagVisibility;
    app.assistant.target_input = tag.0.to_string();
    app.assistant.value_input = "yes".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    assert!(
        app.prepare_assistant_intent(WorkflowIntent::SetTagVisibility {
            target: tag,
            visible: false,
        })
    );
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetTagVisibility(tag));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Boolean(true)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Boolean(false)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(!app.document.current().tag(tag).unwrap().visible());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().tag(tag).unwrap().visible());
}

#[test]
fn assistant_occurrence_tag_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(8);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Fixtures".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(tag),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::OccurrenceTag;
    app.assistant.target_input = "1".to_owned();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "none".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceTag(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Tag(Some(tag))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Tag(None)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        None
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        Some(tag)
    );
}

#[test]
fn assistant_occurrence_repoint_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(9);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Alternate".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::OccurrenceDefinition;
    app.assistant.target_input = "1".to_owned();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = definition.0.to_string();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::RepointOccurrence(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Definition(INITIAL_BOX_DEFINITION)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Definition(definition)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .definition_id(),
        definition
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .definition_id(),
        INITIAL_BOX_DEFINITION
    );
}

#[test]
fn assistant_occurrence_parent_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(10);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(group),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::OccurrenceParent;
    app.assistant.target_input = "1".to_owned();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "none".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceParent(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Group(Some(group))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Group(None)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .parent(),
        None
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .parent(),
        Some(group)
    );
}

#[test]
fn assistant_group_parent_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(10);
    let parent = GroupId(11);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: parent,
                name: "Parent".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::GroupParent;
    app.assistant.target_input = group.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = parent.0.to_string();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetGroupParent(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Group(None)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Group(Some(parent))
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document.current().group(group).unwrap().parent(),
        Some(parent)
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().group(group).unwrap().parent(), None);
}

#[test]
fn assistant_group_translation_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(10);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: group,
            name: "Assembly".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::GroupTranslation;
    app.assistant.target_input = group.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "4.5, -2, 11.25".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    let expected = Transform::from_translation(4.5, -2.0, 11.25).unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetGroupTranslation(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Transform(Transform::identity())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Transform(expected)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document.current().group(group).unwrap().transform(),
        expected
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().group(group).unwrap().transform(),
        Transform::identity()
    );
}

#[test]
fn assistant_profile_points_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(50);
    let profile = FeatureId(51);
    let original = vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Assistant profile".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Profile".to_owned(),
                kind: FeatureKind::polygon(&original),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::ProfilePoints;
    app.assistant.target_input = profile.0.to_string();
    app.assistant.value_input = "0,0; invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    let requested = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
    app.assistant.value_input = "0,0; 12,0; 12,8; 0,8".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetProfilePoints(profile));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ProfilePoints(original.clone())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ProfilePoints(requested.clone())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().feature(profile).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &requested
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(profile).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &original
    ));
}

#[test]
fn assistant_rule_outputs_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(30);
    let output = |key: &str| {
        RuleOutput::new(SlotSegment::new(rule, "result", key).unwrap(), Vec::new()).unwrap()
    };
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "layout".to_owned(),
            expression: "1".to_owned(),
            input_ports: vec![PortSpec::number("source").unwrap()],
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: vec![output("left")],
            override_parameters: Vec::new(),
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::RuleOutputs;
    app.assistant.target_input = rule.0.to_string();
    app.assistant.value_input = "result".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    let requested = vec![output("center"), output("right")];
    app.assistant.value_input = "result:center; result:right".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetRuleOutputs(rule));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::RuleOutputs(vec![output("left")])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::RuleOutputs(requested.clone())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().evaluator_node(rule).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &requested
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().evaluator_node(rule).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &vec![output("left")]
    ));
}

#[test]
fn assistant_create_tag_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateTag;
    app.assistant.target_input = tag.0.to_string();
    app.assistant.value_input = "visible:Reviewed".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "true:Reviewed tag".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateTag(tag));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::TagState {
            name: "Reviewed tag".to_owned(),
            visible: true,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.tag(tag).unwrap();
    assert_eq!(created.name(), "Reviewed tag");
    assert!(created.visible());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().tag(tag).is_none());
}

#[test]
fn assistant_create_collection_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateCollection;
    app.assistant.target_input = collection.0.to_string();
    app.assistant.value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "Reviewed selection".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateCollection(collection));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Reviewed selection".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .name(),
        "Reviewed selection"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().collection(collection).is_none());
}

#[test]
fn assistant_delete_collection_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(24);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: collection,
                name: "Reviewed selection".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: collection,
                occurrence_ids: vec![OccurrenceId(1)],
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteCollection;
    app.assistant.target_input = collection.0.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteCollection(collection));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::CollectionState {
            name: "Reviewed selection".to_owned(),
            occurrence_ids: vec![OccurrenceId(1)],
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().collection(collection).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.collection(collection).unwrap();
    assert_eq!(restored.name(), "Reviewed selection");
    assert_eq!(
        restored.occurrence_ids().collect::<Vec<_>>(),
        vec![OccurrenceId(1)]
    );
}

#[test]
fn assistant_delete_tag_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(24);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: tag,
            name: "Reviewed tag".to_owned(),
            visible: false,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteTag;
    app.assistant.target_input = tag.0.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteTag(tag));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::TagState {
            name: "Reviewed tag".to_owned(),
            visible: false,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().tag(tag).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.tag(tag).unwrap();
    assert_eq!(restored.name(), "Reviewed tag");
    assert!(!restored.visible());
}

#[test]
fn assistant_delete_group_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(24);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: group,
            name: "Reviewed group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteGroup;
    app.assistant.target_input = group.0.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteGroup(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::GroupState {
            name: "Reviewed group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().group(group).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.group(group).unwrap();
    assert_eq!(restored.name(), "Reviewed group");
    assert_eq!(restored.transform(), Transform::identity());
    assert_eq!(restored.parent(), None);
}

#[test]
fn assistant_delete_occurrence_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let occurrence = OccurrenceId(1);
    let snapshot = app.document.current();
    let existing = snapshot.occurrence(occurrence).unwrap();
    let expected_definition = existing.definition_id();
    let expected_name = existing.name().to_owned();
    let expected_transform = existing.transform();
    let expected_parent = existing.parent();
    let expected_tag = existing.tag();
    let expected_visible = existing.visible();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteOccurrence;
    app.assistant.target_input = occurrence.0.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteOccurrence(occurrence));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::OccurrenceState {
            definition: expected_definition,
            name: expected_name.clone(),
            transform: expected_transform,
            parent: expected_parent,
            tag: expected_tag,
            visible: expected_visible,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().occurrence(occurrence).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.occurrence(occurrence).unwrap();
    assert_eq!(restored.definition_id(), expected_definition);
    assert_eq!(restored.name(), expected_name);
    assert_eq!(restored.transform(), expected_transform);
    assert_eq!(restored.parent(), expected_parent);
    assert_eq!(restored.tag(), expected_tag);
    assert_eq!(restored.visible(), expected_visible);
}

#[test]
fn assistant_create_definition_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateDefinition;
    app.assistant.target_input = definition.0.to_string();
    app.assistant.value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "Reviewed component".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateDefinition(definition));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Reviewed component".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .definition(definition)
            .unwrap()
            .name(),
        "Reviewed component"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().definition(definition).is_none());
}

#[test]
fn assistant_create_group_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateGroup;
    app.assistant.target_input = group.0.to_string();
    app.assistant.value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "Reviewed root group".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateGroup(group));
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
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.group(group).unwrap();
    assert_eq!(created.name(), "Reviewed root group");
    assert_eq!(created.transform(), Transform::identity());
    assert_eq!(created.parent(), None);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().group(group).is_none());
}

#[test]
fn assistant_create_occurrence_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let occurrence = OccurrenceId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateOccurrence;
    app.assistant.target_input = occurrence.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "1:Reviewed occurrence".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateOccurrence(occurrence));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::OccurrenceState {
            definition: INITIAL_BOX_DEFINITION,
            name: "Reviewed occurrence".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.occurrence(occurrence).unwrap();
    assert_eq!(created.definition_id(), INITIAL_BOX_DEFINITION);
    assert_eq!(created.name(), "Reviewed occurrence");
    assert_eq!(created.transform(), Transform::identity());
    assert_eq!(created.parent(), None);
    assert_eq!(created.tag(), None);
    assert!(created.visible());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().occurrence(occurrence).is_none());
}

#[test]
fn assistant_create_profile_feature_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let feature = FeatureId(24);
    let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    let feature_ids_before = app
        .document
        .current()
        .definition(INITIAL_BOX_DEFINITION)
        .unwrap()
        .feature_ids()
        .to_vec();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateProfileFeature;
    app.assistant.target_input = feature.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "1:Reviewed profile:0,0;20,0;20,10;0,10".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateProfileFeature(feature));
    assert_eq!(proposal.authoritative_writes().len(), 2);
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_model::document::AuthoritativeDependency::Feature(feature)
        })
        .unwrap();
    assert_eq!(feature_diff.before, ProposalValue::Missing);
    assert_eq!(
        feature_diff.after,
        ProposalValue::ProfileFeatureState {
            definition: INITIAL_BOX_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    let definition_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target
                == ketchup_model::document::AuthoritativeDependency::Definition(
                    INITIAL_BOX_DEFINITION,
                )
        })
        .unwrap();
    let mut feature_ids_after = feature_ids_before.clone();
    feature_ids_after.push(feature);
    assert_eq!(
        definition_diff.before,
        ProposalValue::DefinitionFeatures(feature_ids_before.clone())
    );
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionFeatures(feature_ids_after)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.feature(feature).unwrap();
    assert_eq!(created.definition_id(), INITIAL_BOX_DEFINITION);
    assert_eq!(created.name(), "Reviewed profile");
    assert!(matches!(
        created.kind().polygon_points(),
        Some(ref created_points) if created_points == &points_mm
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().feature(feature).is_none());
    assert_eq!(
        app.document
            .current()
            .definition(INITIAL_BOX_DEFINITION)
            .unwrap()
            .feature_ids(),
        feature_ids_before
    );
}

#[test]
fn assistant_delete_profile_feature_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let feature = FeatureId(24);
    let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature,
            definition_id: INITIAL_BOX_DEFINITION,
            name: "Reviewed profile".to_owned(),
            kind: FeatureKind::polygon(&points_mm),
        }]))
        .unwrap();
    let feature_ids_before = app
        .document
        .current()
        .definition(INITIAL_BOX_DEFINITION)
        .unwrap()
        .feature_ids()
        .to_vec();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteProfileFeature;
    app.assistant.target_input = feature.0.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteProfileFeature(feature));
    assert_eq!(proposal.authoritative_writes().len(), 2);
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_model::document::AuthoritativeDependency::Feature(feature)
        })
        .unwrap();
    assert_eq!(
        feature_diff.before,
        ProposalValue::ProfileFeatureState {
            definition: INITIAL_BOX_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(feature_diff.after, ProposalValue::Missing);
    let definition_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target
                == ketchup_model::document::AuthoritativeDependency::Definition(
                    INITIAL_BOX_DEFINITION,
                )
        })
        .unwrap();
    let mut feature_ids_after = feature_ids_before.clone();
    feature_ids_after.retain(|candidate| *candidate != feature);
    assert_eq!(
        definition_diff.before,
        ProposalValue::DefinitionFeatures(feature_ids_before.clone())
    );
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionFeatures(feature_ids_after)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().feature(feature).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.feature(feature).unwrap();
    assert_eq!(restored.definition_id(), INITIAL_BOX_DEFINITION);
    assert_eq!(restored.name(), "Reviewed profile");
    assert_eq!(restored.kind(), &FeatureKind::polygon(&points_mm));
    assert_eq!(
        snapshot
            .definition(INITIAL_BOX_DEFINITION)
            .unwrap()
            .feature_ids(),
        feature_ids_before
    );
}

#[test]
fn assistant_create_evaluator_input_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = NodeId(99);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateEvaluatorInput;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "missing delimiter".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "Reviewed depth:42.5".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorInput(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
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
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed depth");
    assert_eq!(
        created.dimension(),
        Some(&Dimension::from_decimal("42.5").unwrap())
    );
    assert!(created.dependencies().is_empty());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().evaluator_node(target).is_none());
}

#[test]
fn assistant_create_evaluator_expression_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "Reviewed source".to_owned(),
                dimension: Dimension::from_decimal("21").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let target = NodeId(100);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateEvaluatorExpression;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "missing delimiter".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "Reviewed double:$1 * 2".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreateEvaluatorExpression(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorExpressionState {
            name: "Reviewed double".to_owned(),
            expression: "$1 * 2".to_owned(),
            dependencies: vec![NodeId(1)],
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed double");
    assert_eq!(created.kind().source(), "$1 * 2");
    assert_eq!(created.dependencies(), &[NodeId(1)]);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().evaluator_node(target).is_none());
}

#[test]
fn assistant_create_evaluator_rule_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "Reviewed source".to_owned(),
                dimension: Dimension::from_decimal("21").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let target = NodeId(100);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateEvaluatorRule;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "missing delimiter".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "Reviewed rule:$1 * 2".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorRule(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorRuleState {
            name: "Reviewed rule".to_owned(),
            expression: "$1 * 2".to_owned(),
            dependencies: vec![NodeId(1)],
            input_ports: Vec::new(),
            output_ports: vec![ketchup_model::document::PortSpec::number("result").unwrap()],
            outputs: Vec::new(),
            override_parameters: Vec::new(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed rule");
    assert_eq!(created.kind().source(), "$1 * 2");
    assert_eq!(created.dependencies(), &[NodeId(1)]);
    assert!(created.input_ports().is_empty());
    assert_eq!(
        created.output_ports(),
        &[ketchup_model::document::PortSpec::number("result").unwrap()]
    );
    assert!(created.allowed_parameters().is_empty());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().evaluator_node(target).is_none());
}

#[test]
fn assistant_create_rule_override_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(101);
    let target = 102;
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "Reviewed override source".to_owned(),
            expression: "1".to_owned(),
            input_ports: Vec::new(),
            output_ports: vec![ketchup_model::document::PortSpec::number("result").unwrap()],
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
    let identity = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateRuleOverride;
    app.assistant.target_input = target.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "101:result:left:offset:2.5".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateRuleOverride(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
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
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.override_by_id(target).unwrap();
    assert_eq!(created.target, identity);
    assert_eq!(created.parameter, "offset");
    assert_eq!(created.value(), 2.5);
    assert_eq!(created.health, SlotResolution::Resolved);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().override_by_id(target).is_none());
}

#[test]
fn assistant_create_feature_parameter_binding_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(200);
    let profile = FeatureId(201);
    let feature = FeatureId(202);
    let rule = NodeId(203);
    let target =
        FeatureParameterTarget::new(feature, "extent.distance", ParameterValueType::Length)
            .unwrap();
    let derived_from = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Bound box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Bound profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: definition,
                name: "Bound extrusion".to_owned(),
                kind: FeatureKind::extrusion(profile, Dimension::from_decimal("20").unwrap()),
            },
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Binding source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_model::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateFeatureParameterBinding;
    app.assistant.target_input = feature.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "extent.distance:203:result:left".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreateFeatureParameterBinding(target.clone())
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::FeatureParameterBindingState {
            target: target.clone(),
            derived_from: derived_from.clone(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .unwrap()
            .derived_from,
        derived_from
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .is_none()
    );
}

#[test]
fn assistant_delete_feature_parameter_binding_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(204);
    let profile = FeatureId(205);
    let feature = FeatureId(206);
    let rule = NodeId(207);
    let target =
        FeatureParameterTarget::new(feature, "extent.distance", ParameterValueType::Length)
            .unwrap();
    let derived_from = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Bound box deletion".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Bound profile deletion".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: definition,
                name: "Bound extrusion deletion".to_owned(),
                kind: FeatureKind::extrusion(profile, Dimension::from_decimal("20").unwrap()),
            },
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Binding deletion source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_model::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_model::document::FeatureParameterBinding {
                    target: target.clone(),
                    derived_from: derived_from.clone(),
                },
            ),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteFeatureParameterBinding;
    app.assistant.target_input = feature.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "extent.distance".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeleteFeatureParameterBinding(target.clone())
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::FeatureParameterBindingState {
            target: target.clone(),
            derived_from: derived_from.clone(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .is_none()
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .unwrap()
            .derived_from,
        derived_from
    );
}

#[test]
fn assistant_recompute_feature_parameter_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(208);
    let profile = FeatureId(209);
    let feature = FeatureId(210);
    let rule = NodeId(211);
    let target =
        FeatureParameterTarget::new(feature, "extent.distance", ParameterValueType::Length)
            .unwrap();
    let derived_from = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "height").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Recomputed box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Recomputed profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: definition,
                name: "Recomputed extrusion".to_owned(),
                kind: FeatureKind::extrusion(profile, Dimension::from_decimal("20").unwrap()),
            },
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Recompute source".to_owned(),
                expression: "42".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_model::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "height").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_model::document::FeatureParameterBinding {
                    target: target.clone(),
                    derived_from,
                },
            ),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::RecomputeFeatureParameter;
    app.assistant.target_input = feature.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "extent.distance".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::RecomputeFeatureParameter(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(Dimension::from_decimal("20").unwrap())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(Dimension::from_decimal("42").unwrap())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().feature(feature).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "42" && height.millimetres() == 42.0
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(feature).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "20" && height.millimetres() == 20.0
    ));
}

#[test]
fn assistant_clone_profile_definition_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let source_definition = DefinitionId(300);
    let source_feature = FeatureId(301);
    let occurrence = OccurrenceId(302);
    let new_definition = DefinitionId(303);
    let new_feature = FeatureId(304);
    let points_mm = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: source_definition,
                name: "Clone source".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: source_feature,
                definition_id: source_definition,
                name: "Source profile".to_owned(),
                kind: FeatureKind::polygon(&points_mm),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: source_definition,
                name: "Clone occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CloneProfileDefinitionAndRepoint;
    app.assistant.target_input = occurrence.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = format!(
        "{}:{}:{}:{}:Independent profile",
        source_definition.0, source_feature.0, new_definition.0, new_feature.0
    );
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CloneProfileDefinitionAndRepoint(occurrence)
    );
    assert_eq!(proposal.authoritative_writes().len(), 3);
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_model::document::AuthoritativeDependency::Feature(new_feature)
        })
        .unwrap();
    assert_eq!(feature_diff.before, ProposalValue::Missing);
    assert_eq!(
        feature_diff.after,
        ProposalValue::ProfileFeatureState {
            definition: new_definition,
            name: "Source profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(occurrence)
            .unwrap()
            .definition_id(),
        new_definition
    );
    assert!(matches!(
        app.document.current().feature(new_feature).unwrap().kind().polygon_points(),
        Some(ref cloned) if cloned == &points_mm
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(occurrence)
            .unwrap()
            .definition_id(),
        source_definition
    );
    assert!(app.document.current().definition(new_definition).is_none());
    assert!(app.document.current().feature(new_feature).is_none());
}

#[test]
fn assistant_convert_empty_group_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(300);
    let new_definition = DefinitionId(301);
    let new_occurrence = OccurrenceId(302);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: group,
            name: "Reviewed empty group".to_owned(),
            transform: Transform::from_translation(1.0, 2.0, 3.0).unwrap(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::ConvertEmptyGroupToComponent;
    app.assistant.target_input = group.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = format!(
        "{}:{}:Reviewed component",
        new_definition.0, new_occurrence.0
    );
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::ConvertEmptyGroupToComponent(group)
    );
    assert_eq!(proposal.authoritative_writes().len(), 3);
    let group_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_model::document::AuthoritativeDependency::GroupSubtree(group)
        })
        .unwrap();
    assert!(matches!(
        group_diff.before,
        ProposalValue::GroupState { ref name, .. } if name == "Reviewed empty group"
    ));
    assert_eq!(group_diff.after, ProposalValue::Missing);
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().group(group).is_none());
    assert!(app.document.current().definition(new_definition).is_some());
    assert_eq!(
        app.document
            .current()
            .occurrence(new_occurrence)
            .unwrap()
            .transform(),
        Transform::from_translation(1.0, 2.0, 3.0).unwrap()
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().group(group).is_some());
    assert!(app.document.current().definition(new_definition).is_none());
    assert!(app.document.current().occurrence(new_occurrence).is_none());
}

#[test]
fn assistant_create_joint_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(500);
    let target = JointId(222);
    let output =
        |key| RuleOutput::new(SlotSegment::new(rule, "result", key).unwrap(), Vec::new()).unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "joint participants".to_owned(),
            expression: "1".to_owned(),
            input_ports: Vec::new(),
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: vec![output("left"), output("right")],
            override_parameters: Vec::new(),
        }]))
        .unwrap();
    let participant = |key| {
        DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateJoint;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = "500,result,left:500,result,right:1,2,3:4,5,6".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateJoint(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::JointState {
            participant_a: participant("left"),
            participant_b: participant("right"),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let joint = ketchup_geometry::prismatic::CanonicalJoint::new(
        target,
        participant("left"),
        participant("right"),
        ketchup_geometry::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
            .unwrap(),
    )
    .unwrap();
    assert_eq!(app.document.current().joint(target), Some(&joint));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().joint(target).is_none());
}

#[test]
fn assistant_delete_joint_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = JointId(212);
    let participant = |key| {
        DerivedIdentity::new(
            NodeId(213),
            SlotPath::new(vec![SlotSegment::new(NodeId(213), "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let joint = ketchup_geometry::prismatic::CanonicalJoint::new(
        target,
        participant("left"),
        participant("right"),
        ketchup_geometry::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])
            .unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            joint.clone(),
        )]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteJoint;
    app.assistant.target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteJoint(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
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
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().joint(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().joint(target), Some(&joint));
}

#[test]
fn assistant_create_space_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = SpaceId(219);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateSpace;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = "maintenance access:1,2,3:4,5,6".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateSpace(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::SpaceState {
            purpose: "maintenance access".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            adjacent_to: Vec::new(),
            accessible_to: Vec::new(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let space = ketchup_model::space::CanonicalSpace::new(
        target,
        "maintenance access",
        ketchup_geometry::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
            .unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(app.document.current().space(target), Some(&space));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().space(target).is_none());
}

#[test]
fn assistant_create_clearance_volume_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let owner = SpaceId(220);
    let target = ClearanceVolumeId(221);
    let space = ketchup_model::space::CanonicalSpace::new(
        owner,
        "equipment",
        ketchup_geometry::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [5.0, 5.0, 5.0])
            .unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            space,
        )]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreateClearanceVolume;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = "220:maintenance envelope:1,2,3:4,5,6:0.01:required".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateClearanceVolume(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ClearanceVolumeState {
            owner: ClearanceOwner::Space(owner),
            reason: "maintenance envelope".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            coordinate_frame: ketchup_model::space::ClearanceCoordinateFrame::World,
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Required,
            derived_from: None,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let clearance = ketchup_model::space::CanonicalClearanceVolume::new(
        target,
        ClearanceOwner::Space(owner),
        "maintenance envelope",
        ketchup_geometry::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
            .unwrap(),
        TolerancePolicy::new(0.01).unwrap(),
        ClearanceSeverity::Required,
        None,
    )
    .unwrap();
    assert_eq!(
        app.document.current().clearance_volume(target),
        Some(&clearance)
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().clearance_volume(target).is_none());
}

#[test]
fn assistant_delete_space_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = SpaceId(214);
    let space = ketchup_model::space::CanonicalSpace::new(
        target,
        "maintenance access",
        ketchup_geometry::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])
            .unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            space.clone(),
        )]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteSpace;
    app.assistant.target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteSpace(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::SpaceState {
            purpose: "maintenance access".to_owned(),
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
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().space(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().space(target), Some(&space));
}

#[test]
fn assistant_delete_clearance_volume_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let owner = SpaceId(215);
    let target = ClearanceVolumeId(216);
    let space = ketchup_model::space::CanonicalSpace::new(
        owner,
        "equipment",
        ketchup_geometry::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [5.0, 5.0, 5.0])
            .unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let clearance = ketchup_model::space::CanonicalClearanceVolume::new(
        target,
        ketchup_model::space::ClearanceOwner::Space(owner),
        "maintenance envelope",
        ketchup_geometry::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])
            .unwrap(),
        ketchup_model::tolerance::TolerancePolicy::new(0.01).unwrap(),
        ketchup_model::space::ClearanceSeverity::Required,
        None,
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertSpace(space),
            CanonicalCommand::UpsertClearanceVolume(clearance.clone()),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteClearanceVolume;
    app.assistant.target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteClearanceVolume(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ClearanceVolumeState {
            owner: ketchup_model::space::ClearanceOwner::Space(owner),
            reason: "maintenance envelope".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            coordinate_frame: ketchup_model::space::ClearanceCoordinateFrame::World,
            tolerance_mm: 0.01,
            severity: ketchup_model::space::ClearanceSeverity::Required,
            derived_from: None,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().clearance_volume(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().clearance_volume(target),
        Some(&clearance)
    );
}

#[test]
fn assistant_delete_persistent_dimension_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = PersistentDimensionId(217);
    let dimension_target = PersistentDimensionTarget::FeatureParameter(
        FeatureParameterTarget::new(FeatureId(2), "bounds.width", ParameterValueType::Length)
            .unwrap(),
    );
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
    let dimension = PersistentDimension::new(
        target,
        "Cabinet width",
        dimension_target.clone(),
        presentation,
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(dimension.clone()),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeletePersistentDimension;
    app.assistant.target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeletePersistentDimension(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::PersistentDimensionState {
            name: "Cabinet width".to_owned(),
            target: dimension_target,
            presentation,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(
        app.document
            .current()
            .persistent_dimension(target)
            .is_none()
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().persistent_dimension(target),
        Some(&dimension)
    );
}

#[test]
fn assistant_create_persistent_dimension_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = PersistentDimensionId(218);
    let dimension_target =
        FeatureParameterTarget::new(FeatureId(2), "extent.distance", ParameterValueType::Length)
            .unwrap();
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CreatePersistentDimension;
    app.assistant.target_input = target.0.to_string();
    app.assistant.value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant.value_input = "Reviewed height:2:extent.distance:cm:2".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreatePersistentDimension(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::PersistentDimensionState {
            name: "Reviewed height".to_owned(),
            target: PersistentDimensionTarget::FeatureParameter(dimension_target.clone()),
            presentation,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let dimension = PersistentDimension::new(
        target,
        "Reviewed height",
        PersistentDimensionTarget::FeatureParameter(dimension_target),
        presentation,
    )
    .unwrap();
    assert_eq!(
        app.document.current().persistent_dimension(target),
        Some(&dimension)
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(
        app.document
            .current()
            .persistent_dimension(target)
            .is_none()
    );
}

#[test]
fn assistant_delete_rule_override_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(101);
    let target = 102;
    let identity = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Reviewed override source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_model::document::PortSpec::number("result").unwrap()],
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
                ketchup_model::document::CanonicalOverride::new(
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
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteRuleOverride;
    app.assistant.target_input = target.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteRuleOverride(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
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
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().override_by_id(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let restored = app
        .document
        .current()
        .override_by_id(target)
        .unwrap()
        .clone();
    assert_eq!(restored.target, identity);
    assert_eq!(restored.parameter, "offset");
    assert_eq!(restored.value(), 2.5);
    assert_eq!(restored.health, SlotResolution::Resolved);
}

#[test]
fn assistant_delete_definition_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(99);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Reviewed empty definition".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::DeleteDefinition;
    app.assistant.target_input = definition.0.to_string();
    app.assistant.value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteDefinition(definition));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::DefinitionState {
            name: "Reviewed empty definition".to_owned(),
            feature_ids: Vec::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().definition(definition).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.definition(definition).unwrap();
    assert_eq!(restored.name(), "Reviewed empty definition");
    assert!(restored.feature_ids().is_empty());
}

#[test]
fn assistant_collection_membership_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(12);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: collection,
                name: "Selection set".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.intent_kind = AssistantIntentKind::CollectionOccurrences;
    app.assistant.target_input = collection.0.to_string();
    app.assistant.value_input = "1, invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "1, 1".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant.value_input = "1".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetCollectionOccurrences(collection)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Occurrences(Vec::new())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Occurrences(vec![OccurrenceId(1)])
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OccurrenceId(1)]
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
}
