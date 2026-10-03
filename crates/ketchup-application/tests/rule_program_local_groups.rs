use super::rule_program_components::program;
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::Snapshot;
use std::collections::BTreeMap;

fn placements(snapshot: &Snapshot) -> BTreeMap<(String, String), [f64; 16]> {
    snapshot
        .scene_query()
        .into_iter()
        .map(|part| {
            (
                (
                    snapshot
                        .occurrence(part.occurrence_id)
                        .unwrap()
                        .name()
                        .to_owned(),
                    part.occurrence_name,
                ),
                *part.transform.matrix(),
            )
        })
        .collect()
}

#[test]
fn shared_group_creation_reparenting_and_removal_preserve_parts_in_every_copy() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = program();
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let inner = before.local_groups().next().unwrap().key();
    let members = |snapshot: &Snapshot| {
        snapshot
            .local_occurrences()
            .map(|part| (part.key(), part.definition_id(), part.transform()))
            .collect::<Vec<_>>()
    };
    let roots = |snapshot: &Snapshot| {
        snapshot
            .occurrences()
            .map(|part| (part.id(), part.definition_id()))
            .collect::<Vec<_>>()
    };
    let mut nested = original.clone();
    nested.source = nested.source.replace(
        "c=component(\"assembly\", [g,b])",
        "h=group(\"wrapper\", [g,b])\nc=component(\"assembly\", [h])",
    );
    let after = session
        .apply_rule_program(nested.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(after.local_groups().count(), 2);
    let wrapper = after
        .local_groups()
        .find(|g| g.name() == "wrapper")
        .unwrap()
        .key();
    assert_eq!(
        after.local_group(inner).unwrap().parent(),
        Some(wrapper.local_id)
    );
    assert_eq!(members(&after), members(&before));
    assert_eq!(roots(&after), roots(&before));
    assert_eq!(
        after.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    assert_eq!(placements(&after), placements(&before));
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_eq!(
        placements(&after),
        placements(
            &fresh
                .apply_rule_program(nested.clone(), false)
                .unwrap()
                .snapshot
        )
    );
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let mut inverted = original.clone();
    inverted.source = inverted.source.replace(
        "g=group(\"inner\", [a])\nc=component(\"assembly\", [g,b])",
        "h=group(\"wrapper\", [b])\ng=group(\"inner\", [a,h])\nc=component(\"assembly\", [g])",
    );
    let reversed = session
        .apply_rule_program(inverted.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(
        reversed.local_group(wrapper).unwrap().parent(),
        Some(inner.local_id)
    );
    assert_eq!(reversed.local_group(inner).unwrap().parent(), None);
    assert_eq!(placements(&reversed), placements(&before));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared-groups.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), reversed.scene_query());
    let mut flattened = original.clone();
    flattened.source = flattened.source.replace(
        "g=group(\"inner\", [a])\nc=component(\"assembly\", [g,b])",
        "c=component(\"assembly\", [a,b])",
    );
    let flat = reopened
        .apply_rule_program(flattened.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(flat.local_groups().count(), 0);
    assert!(flat.local_occurrences().all(|part| part.parent().is_none()));
    assert_eq!(members(&flat), members(&before));
    assert_eq!(roots(&flat), roots(&before));
    assert_eq!(placements(&flat), placements(&before));
    assert_eq!(
        flat.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    assert_eq!(
        reopened.undo().unwrap().scene_query(),
        reversed.scene_query()
    );
    assert_eq!(reopened.redo().unwrap().scene_query(), flat.scene_query());
    assert_eq!(reopened.rule_program(), Some(&flattened));
}
