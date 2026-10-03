use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use std::collections::BTreeMap;

const PARTS: &str =
    "a = box(\"a\", (10,20,30), at=(100,200,300))\nb = box(\"b\", (20,30,40), at=(400,500,600))\n";

fn source(groups: &str) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "groups.star".to_owned(),
        source: format!("{PARTS}{groups}"),
        overrides: BTreeMap::new(),
    }
}

fn layout(snapshot: &Snapshot) -> BTreeMap<String, Option<String>> {
    snapshot
        .occurrences()
        .map(|part| {
            (
                part.name().to_owned(),
                part.parent()
                    .map(|id| snapshot.group(id).unwrap().name().to_owned()),
            )
        })
        .chain(snapshot.groups().map(|group| {
            (
                group.name().to_owned(),
                group
                    .parent()
                    .map(|id| snapshot.group(id).unwrap().name().to_owned()),
            )
        }))
        .collect()
}

#[test]
fn nested_groups_update_and_ungroup_without_moving_or_recreating_parts() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let first = source("g = group(\"inner\", [a])\ngroup(\"outer\", [g,b])\n");
    let initial = session
        .apply_rule_program(first.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(
        layout(&initial),
        BTreeMap::from([
            ("a".to_owned(), Some("inner".to_owned())),
            ("b".to_owned(), Some("outer".to_owned())),
            ("inner".to_owned(), Some("outer".to_owned())),
            ("outer".to_owned(), None),
        ])
    );
    let parts = initial
        .occurrences()
        .map(|part| (part.id(), part.definition_id(), part.transform()))
        .collect::<Vec<_>>();
    let group_ids = initial
        .groups()
        .map(|group| (group.name().to_owned(), group.id()))
        .collect::<BTreeMap<_, _>>();
    let second = source("g = group(\"inner\", [b])\ngroup(\"outer\", [g,a])\n");
    let updated = session.apply_rule_program(second, false).unwrap().snapshot;
    assert_eq!(layout(&updated)["a"].as_deref(), Some("outer"));
    assert_eq!(layout(&updated)["b"].as_deref(), Some("inner"));
    for group in updated.groups() {
        assert_eq!(group.id(), group_ids[group.name()]);
    }
    assert_eq!(layout(&session.undo().unwrap()), layout(&initial));
    assert_eq!(layout(&session.redo().unwrap()), layout(&updated));
    let plain = session
        .apply_rule_program(source(""), false)
        .unwrap()
        .snapshot;
    assert_eq!(plain.groups().count(), 0);
    for snapshot in [&initial, &updated, &plain] {
        assert_eq!(
            snapshot
                .occurrences()
                .map(|part| (part.id(), part.definition_id(), part.transform()))
                .collect::<Vec<_>>(),
            parts
        );
    }
    let restored = session.undo().unwrap();
    assert_eq!(layout(&restored), layout(&updated));
    assert!(session.rule_program().is_some());
}

#[test]
fn grouped_part_add_remove_and_rebuild_preserve_survivors() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let first = source("group(\"assembly\", [a,b])");
    let before = session
        .apply_rule_program(first.clone(), false)
        .unwrap()
        .snapshot;
    let a_id = before
        .occurrences()
        .find(|part| part.name() == "a")
        .unwrap()
        .id();
    let group_id = before.groups().next().unwrap().id();
    let mut changed = first;
    changed.source = changed
        .source
        .replace(
            "box(\"a\", (10,20,30), at=(100,200,300))",
            "extrude(\"a\", profile=[(0,0),(10,0),(0,20)], distance=30, at=(100,200,300))",
        )
        .replace(
            "b = box(\"b\", (20,30,40), at=(400,500,600))",
            "c = box(\"c\", (40,50,60), at=(700,800,900))",
        )
        .replace("[a,b]", "[a,c]");
    let after = session.apply_rule_program(changed, false).unwrap().snapshot;
    assert_eq!(after.occurrences().count(), 2);
    assert_eq!(after.occurrence(a_id).unwrap().name(), "a");
    assert_eq!(after.groups().next().unwrap().id(), group_id);
    assert_eq!(layout(&after)["c"].as_deref(), Some("assembly"));
    assert!(
        after
            .occurrences()
            .all(|part| part.parent() == Some(group_id))
    );
    assert_eq!(layout(&session.undo().unwrap()), layout(&before));
}

#[test]
fn groups_can_reverse_nesting_or_be_empty_without_losing_parts() {
    let mut session = DocumentSession::new(SessionSettings::default());
    session
        .apply_rule_program(
            source("g=group(\"inner\", [a])\ngroup(\"outer\", [g,b])"),
            false,
        )
        .unwrap();
    let changed = session
        .apply_rule_program(
            source("g=group(\"outer\", [b])\ngroup(\"inner\", [g,a])"),
            false,
        )
        .unwrap()
        .snapshot;
    assert_eq!(layout(&changed)["outer"].as_deref(), Some("inner"));
    assert_eq!(layout(&changed)["inner"], None);
    let empty = session
        .apply_rule_program(source("group(\"inner\", [])"), false)
        .unwrap()
        .snapshot;
    assert_eq!(empty.groups().count(), 1);
    assert!(empty.occurrences().all(|part| part.parent().is_none()));
    let mut only_groups = DocumentSession::new(SessionSettings::default());
    only_groups
        .apply_rule_program(
            RuleProgramSource {
                source: "group(\"empty\", [])".to_owned(),
                ..source("")
            },
            false,
        )
        .unwrap();
    assert_eq!(only_groups.snapshot().groups().count(), 1);
    assert_eq!(only_groups.snapshot().occurrences().count(), 0);
}

#[test]
fn invalid_group_edit_leaves_geometry_membership_and_source_unchanged() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = source("group(\"assembly\", [a,b])");
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    for invalid in ["group(\"bad\", [a,a])", "group(\"bad\", [\"missing\"])"] {
        assert!(session.apply_rule_program(source(invalid), false).is_err());
        assert_eq!(layout(&session.snapshot()), layout(&before));
        assert_eq!(session.rule_program(), Some(&original));
        assert_eq!(session.snapshot().scene_query(), before.scene_query());
    }
}

#[test]
fn saved_groups_reopen_with_source_and_editable_membership() {
    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("groups.ketchup");
    let mut session = DocumentSession::new(SessionSettings::default());
    let program = source("group(\"assembly\", [a,b])");
    let expected = session
        .apply_rule_program(program.clone(), false)
        .unwrap()
        .snapshot;
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(layout(&reopened.snapshot()), layout(&expected));
    assert_eq!(reopened.rule_program(), Some(&program));
    let edited = reopened
        .apply_rule_program(source("group(\"assembly\", [b])"), false)
        .unwrap()
        .snapshot;
    assert_eq!(layout(&edited)["a"], None);
    assert_eq!(layout(&edited)["b"].as_deref(), Some("assembly"));
}
