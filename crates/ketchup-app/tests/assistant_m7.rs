mod harness;

use harness::Shell;
use ketchup_core::document::{FeatureId, ProposalGoal};
use ketchup_core::intent::WorkflowIntent;

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
