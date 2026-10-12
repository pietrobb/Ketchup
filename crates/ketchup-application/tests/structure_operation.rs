//! The `structure` CAD edit operation: every scene-tree change plans without a
//! change, commits as one Undo step and undoes exactly; a change the document
//! does not allow is refused before anything changes.

use super::document_session::part;
use ketchup_application::{DocumentSession, SessionError};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation as Operation, AssistantCadEditProgram,
    AssistantStructureChange as Change,
};
use ketchup_model::document::*;
use std::collections::BTreeSet;

const EMPTY: GroupId = GroupId(31);
const FILLED: GroupId = GroupId(32);
const FREE_TAG: TagId = TagId(41);
const USED_TAG: TagId = TagId(42);

struct Seeded {
    session: DocumentSession,
    /// A root occurrence.
    loose: OccurrenceId,
    /// The occurrence inside `FILLED`, carrying `USED_TAG`.
    grouped: OccurrenceId,
}

fn program(changes: Vec<Change>) -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: changes
            .into_iter()
            .map(|change| Operation::Structure { change })
            .collect(),
    }
}

fn seeded() -> Seeded {
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![part(), part()],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let ids: Vec<OccurrenceId> = session
        .snapshot()
        .occurrences()
        .map(Occurrence::id)
        .collect();
    let [loose, grouped] = ids[..] else {
        panic!("two seeded occurrences, got {ids:?}")
    };
    let group = |id, name: &str| CanonicalCommand::CreateGroup {
        id,
        name: name.to_owned(),
        transform: Transform::identity(),
        parent: None,
    };
    let tag = |id, name: &str| CanonicalCommand::CreateTag {
        id,
        name: name.to_owned(),
        visible: true,
    };
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            group(EMPTY, "Empty"),
            group(FILLED, "Filled"),
            CanonicalCommand::SetOccurrenceParent {
                id: grouped,
                parent: Some(FILLED),
            },
            tag(FREE_TAG, "Unused"),
            tag(USED_TAG, "Used"),
            CanonicalCommand::SetOccurrenceTags {
                id: grouped,
                tags: BTreeSet::from([USED_TAG]),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    Seeded {
        session,
        loose,
        grouped,
    }
}

fn definition_of(snapshot: &Snapshot, id: OccurrenceId) -> DefinitionId {
    snapshot.occurrence(id).unwrap().definition_id()
}

type Check = Box<dyn Fn(&Snapshot)>;

#[test]
fn every_structure_change_plans_without_change_and_commits_one_undo_step() {
    let Seeded {
        session,
        loose,
        grouped,
    } = seeded();
    let loose_definition = definition_of(&session.snapshot(), loose);
    let grouped_definition = definition_of(&session.snapshot(), grouped);
    let cases: Vec<(Vec<Change>, Check)> = vec![
        (
            vec![Change::CreateGroup {
                group_id: 50,
                name: "Legs".to_owned(),
                parent_group_id: Some(EMPTY.0),
            }],
            Box::new(|snapshot| {
                let group = snapshot.group(GroupId(50)).unwrap();
                assert_eq!(group.name(), "Legs");
                assert_eq!(group.parent(), Some(EMPTY));
            }),
        ),
        (
            vec![Change::MoveOccurrence {
                occurrence_id: loose.0,
                group_id: Some(EMPTY.0),
            }],
            Box::new(move |snapshot| {
                assert_eq!(snapshot.occurrence(loose).unwrap().parent(), Some(EMPTY));
            }),
        ),
        (
            vec![Change::MoveOccurrence {
                occurrence_id: grouped.0,
                group_id: None,
            }],
            Box::new(move |snapshot| {
                assert_eq!(snapshot.occurrence(grouped).unwrap().parent(), None);
            }),
        ),
        (
            vec![Change::MoveGroup {
                group_id: EMPTY.0,
                parent_group_id: Some(FILLED.0),
            }],
            Box::new(|snapshot| assert_eq!(snapshot.group(EMPTY).unwrap().parent(), Some(FILLED))),
        ),
        (
            vec![Change::SetGroupTranslation {
                group_id: FILLED.0,
                translation_mm: [10.0, -20.0, 30.5],
            }],
            Box::new(|snapshot| {
                let matrix = *snapshot.group(FILLED).unwrap().transform().matrix();
                assert_eq!([matrix[3], matrix[7], matrix[11]], [10.0, -20.0, 30.5]);
            }),
        ),
        (
            vec![Change::DeleteGroup { group_id: EMPTY.0 }],
            Box::new(|snapshot| assert!(snapshot.group(EMPTY).is_none())),
        ),
        (
            vec![Change::MakeComponent {
                group_id: FILLED.0,
                name: "Leg assembly".to_owned(),
            }],
            Box::new(move |snapshot| {
                assert!(snapshot.group(FILLED).is_none());
                let component = snapshot
                    .definitions()
                    .find(|definition| definition.name() == "Leg assembly")
                    .expect("component definition");
                assert!(
                    snapshot
                        .occurrences()
                        .any(|occurrence| occurrence.definition_id() == component.id())
                );
                assert!(snapshot.occurrence(loose).is_some());
            }),
        ),
        (
            vec![Change::ReplaceDefinition {
                occurrence_id: loose.0,
                definition_id: grouped_definition.0,
            }],
            Box::new(move |snapshot| {
                assert_eq!(definition_of(snapshot, loose), grouped_definition);
            }),
        ),
        (
            vec![Change::RenameDefinition {
                definition_id: loose_definition.0,
                name: "Renamed part".to_owned(),
            }],
            Box::new(move |snapshot| {
                let definition = snapshot.definition(loose_definition).unwrap();
                assert_eq!(definition.name(), "Renamed part");
            }),
        ),
        (
            vec![Change::SetOccurrenceVisibility {
                occurrence_id: loose.0,
                visible: false,
            }],
            Box::new(move |snapshot| assert!(!snapshot.occurrence(loose).unwrap().visible())),
        ),
        (
            vec![Change::DeleteTag { tag_id: FREE_TAG.0 }],
            Box::new(|snapshot| {
                assert!(snapshot.tag(FREE_TAG).is_none());
                assert!(snapshot.tag(USED_TAG).is_some());
            }),
        ),
        (
            // The second change sees the first: the definition is unused only after it.
            vec![
                Change::ReplaceDefinition {
                    occurrence_id: loose.0,
                    definition_id: grouped_definition.0,
                },
                Change::DeleteDefinition {
                    definition_id: loose_definition.0,
                },
            ],
            Box::new(move |snapshot| {
                assert!(snapshot.definition(loose_definition).is_none());
                assert_eq!(definition_of(snapshot, loose), grouped_definition);
            }),
        ),
    ];
    for (changes, check) in cases {
        let Seeded { mut session, .. } = seeded();
        let before = session.snapshot();
        let undo_before = session.visible_undo_steps();
        let proposal = session
            .plan_cad_program(&program(changes.clone()), &BTreeSet::new())
            .unwrap_or_else(|error| panic!("{changes:?}: {error:?}"));
        assert_eq!(
            session.snapshot().canonical_digest(),
            before.canonical_digest(),
            "planning {changes:?} changed the document"
        );
        assert_eq!(session.visible_undo_steps(), undo_before);

        let committed = session.apply_proposal(&proposal).unwrap();
        check(&committed);
        assert_eq!(session.visible_undo_steps(), undo_before + 1, "{changes:?}");
        session.undo().unwrap();
        assert_eq!(
            session.snapshot().canonical_digest(),
            before.canonical_digest(),
            "undo of {changes:?}"
        );
    }
}

#[test]
fn structure_changes_the_document_does_not_allow_are_refused_without_a_change() {
    let Seeded {
        session,
        loose,
        grouped,
    } = seeded();
    let loose_definition = definition_of(&session.snapshot(), loose);
    let digest_before = session.snapshot().canonical_digest();
    let refused = |change: Change| {
        let error = session
            .plan_cad_program(&program(vec![change.clone()]), &BTreeSet::new())
            .expect_err(&format!("{change:?} must be refused"));
        assert!(
            matches!(error, SessionError::Planning(_) | SessionError::Prepare(_)),
            "{change:?}: {error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest_before);
    };
    for (group_id, name) in [(0, "Zero ID"), (50, " "), (EMPTY.0, "Duplicate")] {
        refused(Change::CreateGroup {
            group_id,
            name: name.to_owned(),
            parent_group_id: None,
        });
    }
    refused(Change::MoveOccurrence {
        occurrence_id: 999,
        group_id: Some(EMPTY.0),
    });
    refused(Change::MoveOccurrence {
        occurrence_id: loose.0,
        group_id: Some(999),
    });
    refused(Change::MoveGroup {
        group_id: EMPTY.0,
        parent_group_id: Some(EMPTY.0),
    });
    refused(Change::SetGroupTranslation {
        group_id: 999,
        translation_mm: [1.0, 0.0, 0.0],
    });
    refused(Change::SetGroupTranslation {
        group_id: FILLED.0,
        translation_mm: [f64::NAN, 0.0, 0.0],
    });
    refused(Change::DeleteGroup { group_id: FILLED.0 });
    refused(Change::MakeComponent {
        group_id: 999,
        name: "Missing".to_owned(),
    });
    refused(Change::ReplaceDefinition {
        occurrence_id: grouped.0,
        definition_id: 999,
    });
    refused(Change::RenameDefinition {
        definition_id: loose_definition.0,
        name: String::new(),
    });
    refused(Change::SetOccurrenceVisibility {
        occurrence_id: 999,
        visible: false,
    });
    refused(Change::DeleteTag { tag_id: USED_TAG.0 });
    refused(Change::DeleteDefinition {
        definition_id: loose_definition.0,
    });
    // A cycle through a descendant: the second change sees the first.
    let error = session
        .plan_cad_program(
            &program(vec![
                Change::MoveGroup {
                    group_id: EMPTY.0,
                    parent_group_id: Some(FILLED.0),
                },
                Change::MoveGroup {
                    group_id: FILLED.0,
                    parent_group_id: Some(EMPTY.0),
                },
            ]),
            &BTreeSet::new(),
        )
        .expect_err("a group cycle must be refused");
    assert!(matches!(
        error,
        SessionError::Planning(_) | SessionError::Prepare(_)
    ));
    assert_eq!(session.snapshot().canonical_digest(), digest_before);
}

fn commit_commands(session: &mut DocumentSession, commands: Vec<CanonicalCommand>) {
    let proposal = session.plan_commands(CommandBatch::new(commands)).unwrap();
    session.apply_proposal(&proposal).unwrap();
}

#[test]
fn group_translation_keeps_the_rotation_and_unparenting_a_group_round_trips() {
    let Seeded { mut session, .. } = seeded();
    let rotated = Transform::from_matrix([
        0.0, -1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    commit_commands(
        &mut session,
        vec![
            CanonicalCommand::SetGroupTransform {
                id: FILLED,
                transform: rotated,
            },
            CanonicalCommand::SetGroupParent {
                id: EMPTY,
                parent: Some(FILLED),
            },
        ],
    );
    let before = session.snapshot().canonical_digest();
    let proposal = session
        .plan_cad_program(
            &program(vec![
                Change::SetGroupTranslation {
                    group_id: FILLED.0,
                    translation_mm: [4.5, -2.0, 11.25],
                },
                Change::MoveGroup {
                    group_id: EMPTY.0,
                    parent_group_id: None,
                },
            ]),
            &BTreeSet::new(),
        )
        .unwrap();
    let committed = session.apply_proposal(&proposal).unwrap();
    assert_eq!(
        committed.group(FILLED).unwrap().transform(),
        Transform::from_matrix([
            0.0, -1.0, 0.0, 4.5, 1.0, 0.0, 0.0, -2.0, 0.0, 0.0, 1.0, 11.25, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap()
    );
    assert_eq!(committed.group(EMPTY).unwrap().parent(), None);
    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), before);
    assert_eq!(
        session.snapshot().group(FILLED).unwrap().transform(),
        rotated
    );
}

#[test]
fn stale_structure_proposals_are_refused_and_keep_the_concurrent_edit() {
    let Seeded {
        session,
        loose,
        grouped,
    } = seeded();
    let loose_definition = definition_of(&session.snapshot(), loose);
    let grouped_definition = definition_of(&session.snapshot(), grouped);
    let moved = |id| CanonicalCommand::SetGroupTransform {
        id,
        transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
    };
    let make_component = Change::MakeComponent {
        group_id: EMPTY.0,
        name: "Component".to_owned(),
    };
    // The definition id the component would take, claimed concurrently below.
    let claimed_definition = {
        let Seeded { mut session, .. } = seeded();
        let known: BTreeSet<_> = session.snapshot().definitions().map(|d| d.id()).collect();
        let proposal = session
            .plan_cad_program(&program(vec![make_component.clone()]), &BTreeSet::new())
            .unwrap();
        let committed = session.apply_proposal(&proposal).unwrap();
        committed
            .definitions()
            .map(|d| d.id())
            .find(|id| !known.contains(id))
            .unwrap()
    };
    let replace_then_delete = vec![
        Change::ReplaceDefinition {
            occurrence_id: loose.0,
            definition_id: grouped_definition.0,
        },
        Change::DeleteDefinition {
            definition_id: loose_definition.0,
        },
    ];
    let cases: Vec<(Vec<Change>, CanonicalCommand)> = vec![
        (
            vec![Change::CreateGroup {
                group_id: 50,
                name: "Reviewed".to_owned(),
                parent_group_id: None,
            }],
            CanonicalCommand::CreateGroup {
                id: GroupId(50),
                name: "Concurrent".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
        ),
        (
            vec![Change::MoveOccurrence {
                occurrence_id: loose.0,
                group_id: Some(EMPTY.0),
            }],
            moved(EMPTY),
        ),
        (
            vec![Change::MoveGroup {
                group_id: EMPTY.0,
                parent_group_id: Some(FILLED.0),
            }],
            moved(FILLED),
        ),
        (
            vec![Change::SetGroupTranslation {
                group_id: FILLED.0,
                translation_mm: [1.0, 2.0, 3.0],
            }],
            moved(FILLED),
        ),
        (
            vec![Change::DeleteGroup { group_id: EMPTY.0 }],
            CanonicalCommand::CreateGroup {
                id: GroupId(51),
                name: "Concurrent child".to_owned(),
                transform: Transform::identity(),
                parent: Some(EMPTY),
            },
        ),
        (vec![make_component.clone()], moved(EMPTY)),
        (
            vec![make_component],
            CanonicalCommand::CreateDefinition {
                id: claimed_definition,
                name: "Concurrent claim".to_owned(),
            },
        ),
        (
            vec![replace_then_delete[0].clone()],
            CanonicalCommand::RenameDefinition {
                id: grouped_definition,
                name: "Concurrent target".to_owned(),
            },
        ),
        (
            vec![Change::RenameDefinition {
                definition_id: loose_definition.0,
                name: "Reviewed".to_owned(),
            }],
            CanonicalCommand::RenameDefinition {
                id: loose_definition,
                name: "Concurrent".to_owned(),
            },
        ),
        (
            vec![Change::SetOccurrenceVisibility {
                occurrence_id: loose.0,
                visible: false,
            }],
            CanonicalCommand::SetOccurrenceTransform {
                id: loose,
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ),
        (
            vec![Change::DeleteTag { tag_id: FREE_TAG.0 }],
            CanonicalCommand::SetTagVisibility {
                id: FREE_TAG,
                visible: false,
            },
        ),
        (
            replace_then_delete,
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(99),
                definition_id: loose_definition,
                name: "Concurrent user".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ),
    ];
    for (changes, concurrent) in cases {
        let Seeded { mut session, .. } = seeded();
        let proposal = session
            .plan_cad_program(&program(changes.clone()), &BTreeSet::new())
            .unwrap();
        commit_commands(&mut session, vec![concurrent.clone()]);
        let changed = session.snapshot().canonical_digest();
        let undo_after_concurrent = session.visible_undo_steps();

        let result = session.apply_proposal(&proposal).err();
        assert!(
            matches!(
                result,
                Some(SessionError::Commit(ProposalCommitError::Stale(_)))
            ),
            "{changes:?} after {concurrent:?}: {result:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), changed);
        assert_eq!(session.visible_undo_steps(), undo_after_concurrent);
    }

    // A tag assigned after planning is refused by the commit-time recheck.
    let Seeded { mut session, .. } = seeded();
    let proposal = session
        .plan_cad_program(
            &program(vec![Change::DeleteTag { tag_id: FREE_TAG.0 }]),
            &BTreeSet::new(),
        )
        .unwrap();
    commit_commands(
        &mut session,
        vec![CanonicalCommand::SetOccurrenceTags {
            id: loose,
            tags: BTreeSet::from([FREE_TAG]),
        }],
    );
    let changed = session.snapshot().canonical_digest();
    assert!(matches!(
        session.apply_proposal(&proposal).err(),
        Some(SessionError::Commit(ProposalCommitError::Preparation(
            ProposalPrepareError::Canonical(CanonicalError::TagInUse { tag: FREE_TAG, .. })
        )))
    ));
    assert_eq!(session.snapshot().canonical_digest(), changed);
    assert!(session.snapshot().tag(FREE_TAG).is_some());
}
