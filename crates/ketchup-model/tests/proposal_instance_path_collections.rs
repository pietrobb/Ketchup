//! A reviewed proposal that removes a part never silently drops what the user
//! attached to that part after the review (MD-2): contact joints and support
//! anchors name parts by instance path and a batch prunes the ones that stop
//! resolving.
use ketchup_model::document::*;

fn occurrence(id: u64) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DefinitionId(1),
        name: format!("Part {id}"),
        transform: Transform::identity(),
        parent: None,
        tags: Default::default(),
        visible: true,
    }
}

fn plates() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Plate".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(10),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind: FeatureKind::polygon(&[[0., 0.], [50., 0.], [50., 30.], [0., 30.]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(1),
                name: "Solid".into(),
                kind: FeatureKind::extrusion(FeatureId(10), Dimension::from_decimal("10").unwrap()),
            },
            occurrence(20),
            occurrence(21),
            occurrence(22),
        ]))
        .unwrap();
    document
}

fn joint(name: &str, a: u64, b: u64) -> ContactJoint {
    ContactJoint {
        name: name.into(),
        parts: [
            InstancePath::root(OccurrenceId(a)),
            InstancePath::root(OccurrenceId(b)),
        ],
        max_gap_mm: 0.5,
    }
}

fn delete_21(document: &DocumentStore) -> Proposal {
    document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(21),
            },
        ]))
        .unwrap()
}

#[test]
fn a_contact_joint_added_to_the_part_after_review_makes_its_deletion_stale() {
    let mut document = plates();
    let proposal = delete_21(&document);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetContactJoints {
                joints: vec![joint("spoj 78", 20, 21)],
            },
        ]))
        .unwrap();
    let before = document.current().canonical_digest();

    assert!(matches!(
        document.commit_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(
        document.current().contact_joints(),
        [joint("spoj 78", 20, 21)]
    );
}

#[test]
fn grounding_the_part_after_review_makes_its_deletion_stale() {
    let mut document = plates();
    let proposal = delete_21(&document);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroundedInstances {
                paths: [InstancePath::root(OccurrenceId(21))].into(),
            },
        ]))
        .unwrap();

    assert!(matches!(
        document.commit_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert!(
        document
            .current()
            .instance_is_grounded(&InstancePath::root(OccurrenceId(21)))
    );
}

#[test]
fn a_joint_between_other_parts_does_not_stale_the_deletion_and_survives_it() {
    let mut document = plates();
    let proposal = delete_21(&document);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetContactJoints {
                joints: vec![joint("spoj 79", 20, 22)],
            },
            CanonicalCommand::SetGroundedInstances {
                paths: [InstancePath::root(OccurrenceId(20))].into(),
            },
        ]))
        .unwrap();

    document.commit_proposal(&proposal).unwrap();
    let snapshot = document.current();
    assert!(snapshot.occurrence(OccurrenceId(21)).is_none());
    assert_eq!(snapshot.contact_joints(), [joint("spoj 79", 20, 22)]);
    assert!(snapshot.instance_is_grounded(&InstancePath::root(OccurrenceId(20))));
}

fn stale_after(
    document: &mut DocumentStore,
    proposed: Vec<CanonicalCommand>,
    change: Vec<CanonicalCommand>,
) {
    let proposal = document
        .prepare_proposal(CommandBatch::new(proposed))
        .unwrap();
    document.apply_batch(&CommandBatch::new(change)).unwrap();
    let before = document.current().canonical_digest();
    assert!(matches!(
        document.commit_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(document.current().canonical_digest(), before);
}

fn delete_21_command() -> Vec<CanonicalCommand> {
    vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(21),
    }]
}

#[test]
fn grounding_the_whole_occurrence_after_review_makes_its_deletion_stale() {
    let mut document = plates();
    stale_after(
        &mut document,
        delete_21_command(),
        vec![CanonicalCommand::SetOccurrenceGrounded {
            id: OccurrenceId(21),
            grounded: true,
        }],
    );
    assert!(document.current().occurrence_is_grounded(OccurrenceId(21)));
}

#[test]
fn classifying_the_part_after_review_makes_its_deletion_stale() {
    let mut document = plates();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: "Fáza".into(),
                categories: vec![(ClassificationCategoryId(1), "Hrubá stavba".into())],
            },
        ]))
        .unwrap();
    stale_after(
        &mut document,
        delete_21_command(),
        vec![CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(21),
            dimension_id: ClassificationDimensionId(1),
            category_id: Some(ClassificationCategoryId(1)),
        }],
    );
}

/// Definition 2 holds one local plate; occurrence 30 places it.
fn assembly() -> DocumentStore {
    let mut document = plates();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Assembly".into(),
            },
            CanonicalCommand::CreateLocalOccurrence {
                key: inner_key(),
                definition_id: DefinitionId(1),
                name: "Inner plate".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(30),
                definition_id: DefinitionId(2),
                name: "Assembly 30".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn inner_key() -> LocalOccurrenceKey {
    LocalOccurrenceKey {
        definition_id: DefinitionId(2),
        local_id: LocalOccurrenceId(1),
    }
}

fn inner_path() -> InstancePath {
    InstancePath::root(OccurrenceId(30))
        .with_step(InstancePathStep::Occurrence(LocalOccurrenceId(1)))
}

#[test]
fn anchoring_a_nested_part_after_review_makes_deleting_its_local_occurrence_stale() {
    let mut document = assembly();
    stale_after(
        &mut document,
        vec![CanonicalCommand::DeleteLocalOccurrence { key: inner_key() }],
        vec![CanonicalCommand::SetGroundedInstances {
            paths: [inner_path()].into(),
        }],
    );
    assert!(document.current().instance_is_grounded(&inner_path()));
}

#[test]
fn a_contact_on_a_nested_part_after_review_makes_repointing_its_owner_stale() {
    let mut document = assembly();
    stale_after(
        &mut document,
        vec![CanonicalCommand::RepointOccurrence {
            id: OccurrenceId(30),
            definition_id: DefinitionId(1),
        }],
        vec![CanonicalCommand::SetContactJoints {
            joints: vec![ContactJoint {
                name: "spoj 80".into(),
                parts: [InstancePath::root(OccurrenceId(20)), inner_path()],
                max_gap_mm: 0.5,
            }],
        }],
    );
    assert_eq!(document.current().contact_joints().len(), 1);
}

#[test]
fn an_anchor_outside_the_reshaped_definition_does_not_stale_a_local_deletion() {
    let mut document = assembly();
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::DeleteLocalOccurrence { key: inner_key() },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroundedInstances {
                paths: [InstancePath::root(OccurrenceId(22))].into(),
            },
        ]))
        .unwrap();

    document.commit_proposal(&proposal).unwrap();
    assert!(
        document
            .current()
            .instance_is_grounded(&InstancePath::root(OccurrenceId(22)))
    );
}

#[test]
fn an_unplaced_joint_part_or_grounded_path_is_rejected_by_name_and_path() {
    let mut document = assembly();
    let missing = InstancePath::root(OccurrenceId(30))
        .with_step(InstancePathStep::Occurrence(LocalOccurrenceId(9)));
    let before = document.current().canonical_digest();

    let joint_error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetContactJoints {
                joints: vec![ContactJoint {
                    name: "spoj 81".into(),
                    parts: [InstancePath::root(OccurrenceId(20)), missing.clone()],
                    max_gap_mm: 0.5,
                }],
            },
        ]))
        .err()
        .unwrap();
    assert_eq!(
        joint_error,
        CanonicalError::ContactJointPartNotFound {
            joint: "spoj 81".into(),
            path: missing.clone(),
        }
    );
    assert!(joint_error.to_string().contains("spoj 81"), "{joint_error}");

    let ground_error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroundedInstances {
                paths: [InstancePath::root(OccurrenceId(20)), missing.clone()].into(),
            },
        ]))
        .err()
        .unwrap();
    assert_eq!(ground_error, CanonicalError::GroundedPathNotFound(missing));
    assert_eq!(document.current().canonical_digest(), before);
}
