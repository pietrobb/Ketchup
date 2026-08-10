mod harness;

use harness::Shell;
use ketchup_core::document::{FeatureId, OccurrenceId, ProposalGoal, ProposalValue, Transform};
use ketchup_core::intent::WorkflowIntent;
use ketchup_interaction::Vec3;

#[test]
fn assistant_review_is_ephemeral_and_cancel_leaves_the_model_unchanged() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let preview = shell.catalog().text("assistant-preview");

    shell.click_row(&preview);

    let proposal = shell
        .app()
        .assistant_proposal()
        .expect("the visible Assistant action creates a review proposal");
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetFeatureDimension(FeatureId(2))
    );
    assert_eq!(proposal.authoritative_diff().len(), 1);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);

    let cancel = shell.catalog().text("assistant-cancel");
    shell.click_row(&cancel);
    assert!(shell.app().assistant_proposal().is_none());
    assert!(shell.app().assistant_verification().is_none());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn assistant_confirm_commits_one_verified_undoable_batch() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let original_height = shell.app().document_height_mm();

    let preview = shell.catalog().text("assistant-preview");
    shell.click_row(&preview);
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);

    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().document_height_mm(), 35.0);
    let verification = shell
        .app()
        .assistant_verification()
        .expect("confirmation returns a verification receipt");
    assert_eq!(verification.revision_id, revision + 1);
    assert_eq!(verification.verified_write_count, 1);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_height_mm(), original_height);
}

#[test]
fn assistant_occurrence_visibility_review_is_explicit_and_undoable() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    assert!(shell.app().occurrence_box_geometry(1).is_some());

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(1),
                visible: false,
            })
    );
    let proposal = shell.app().assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceVisibility(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Boolean(true)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Boolean(false)
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert!(shell.app().occurrence_box_geometry(1).is_some());

    assert!(shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert!(shell.app().occurrence_box_geometry(1).is_none());
    assert!(shell.app_mut().undo());
    assert!(shell.app().occurrence_box_geometry(1).is_some());
}

#[test]
fn assistant_occurrence_translation_review_is_exact_observational_and_undoable() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let geometry_before = shell.app().occurrence_box_geometry(1).unwrap();

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTranslation {
                target: OccurrenceId(1),
                x_mm_text: "12.5".to_owned(),
                y_mm_text: "-4".to_owned(),
                z_mm_text: "8.25".to_owned(),
            })
    );
    let proposal = shell.app().assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceTranslation(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Transform(Transform::identity())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Transform(Transform::from_translation(12.5, -4.0, 8.25).unwrap())
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap(),
        geometry_before
    );

    assert!(shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        Vec3::new(12.5, -4.0, 8.25)
    );
    assert!(shell.app_mut().undo());
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap(),
        geometry_before
    );
}

#[test]
fn assistant_definition_rename_review_is_textual_observational_and_undoable() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: ketchup_core::document::DefinitionId(1),
                name: "Housing".to_owned(),
            })
    );
    let proposal = shell.app().assistant_proposal().unwrap();
    let original_name = proposal.authoritative_diff()[0].before.clone();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::RenameDefinition(ketchup_core::document::DefinitionId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Housing".to_owned())
    );
    assert_eq!(shell.app().document_revision(), revision);

    assert!(shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert!(shell.app_mut().undo());
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: ketchup_core::document::DefinitionId(1),
                name: "Housing".to_owned(),
            })
    );
    assert_eq!(
        shell
            .app()
            .assistant_proposal()
            .unwrap()
            .authoritative_diff()[0]
            .before,
        original_name
    );
}

#[test]
fn assistant_rejects_invalid_and_stale_intents_without_an_extra_mutation() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    assert!(
        !shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(999),
                value_text: "30".to_owned(),
            })
    );
    assert_eq!(shell.app().document_revision(), initial_revision);

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(2),
                value_text: "40".to_owned(),
            })
    );
    shell.app_mut().set_push_pull_distance_input("5");
    assert!(shell.app_mut().start_preview());
    assert!(shell.app_mut().confirm_preview());
    let changed_revision = shell.app().document_revision();
    let changed_digest = shell.app().canonical_digest();

    assert!(!shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
}
