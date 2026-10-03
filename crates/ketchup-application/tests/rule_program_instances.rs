use super::rule_program_components::program;
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::Snapshot;

fn assert_same_layout(actual: &Snapshot, expected: &Snapshot) {
    let layout = |snapshot: &Snapshot| {
        let mut rows = snapshot
            .scene_query()
            .iter()
            .map(|part| {
                let root = snapshot.occurrence(part.occurrence_id).unwrap();
                (
                    root.name().to_owned(),
                    part.occurrence_name.clone(),
                    *part.transform.matrix(),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        rows
    };
    assert_eq!(layout(actual), layout(expected));
}

#[test]
fn add_remove_regroup_instances_preserves_shared_geometry_and_surviving_identity() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let initial = session
        .apply_rule_program(program(), false)
        .unwrap()
        .snapshot;
    let roots = initial
        .occurrences()
        .map(|item| (item.id(), item.definition_id()))
        .collect::<Vec<_>>();
    let locals = initial
        .local_occurrences()
        .map(|item| (item.key(), item.definition_id()))
        .collect::<Vec<_>>();
    let outer = initial.groups().next().unwrap().id();
    let mut expanded = program();
    expanded.source = expanded.source.replace(
        "group(\"all\",[c,i])",
        "j=instance(\"third\",c,at=(500,600,70))\ngroup(\"all\",[c,i,j])",
    );
    expanded.overrides.insert("width".into(), 35.0);
    let added = session
        .apply_rule_program(expanded.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(added.definitions().count(), 3);
    assert_eq!(
        added
            .local_occurrences()
            .map(|item| (item.key(), item.definition_id()))
            .collect::<Vec<_>>(),
        locals
    );
    assert_eq!(added.occurrences().count(), 3);
    for (id, definition) in roots {
        let surviving = added.occurrence(id).unwrap();
        assert_eq!(surviving.definition_id(), definition);
        assert_eq!(surviving.parent(), Some(outer));
    }
    let third = added
        .occurrences()
        .find(|item| item.name() == "third")
        .unwrap();
    assert_eq!(third.parent(), Some(outer));
    let scene = added.scene_query();
    let third_a = scene
        .iter()
        .find(|item| item.occurrence_id == third.id() && item.occurrence_name == "a")
        .unwrap();
    assert_eq!(
        [
            third_a.transform.matrix()[3],
            third_a.transform.matrix()[7],
            third_a.transform.matrix()[11]
        ],
        [510.0, 620.0, 70.0]
    );
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let rebuilt = fresh
        .apply_rule_program(expanded.clone(), false)
        .unwrap()
        .snapshot;
    assert_same_layout(&added, &rebuilt);
    assert_eq!(
        added.features().collect::<Vec<_>>(),
        rebuilt.features().collect::<Vec<_>>()
    );
    assert_eq!(session.undo().unwrap().scene_query(), initial.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), added.scene_query());

    let mut reduced = expanded;
    reduced.source = reduced
        .source
        .replace("i=instance(\"second\", c, at=(200,300,0), x=(0,1,0))", "")
        .replace("group(\"all\",[c,i,j])", "group(\"new-parent\",[j])");
    let removed = session
        .apply_rule_program(reduced.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(removed.occurrences().count(), 2);
    assert_eq!(removed.definitions().count(), 3);
    assert!(removed.occurrences().all(|item| item.name() != "second"));
    assert!(removed.group(outer).is_none());
    let surviving = removed.occurrence(third.id()).unwrap();
    assert_eq!(surviving.definition_id(), third.definition_id());
    assert_eq!(
        removed.group(surviving.parent().unwrap()).unwrap().name(),
        "new-parent"
    );
    assert_eq!(
        removed
            .occurrences()
            .find(|item| item.name() == "assembly")
            .unwrap()
            .parent(),
        None
    );
    assert_eq!(
        removed.features().collect::<Vec<_>>(),
        added.features().collect::<Vec<_>>()
    );
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_same_layout(
        &removed,
        &fresh.apply_rule_program(reduced, false).unwrap().snapshot,
    );
    assert_eq!(session.undo().unwrap().scene_query(), added.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), removed.scene_query());
}

#[test]
fn reopened_components_can_remove_all_copies_then_add_a_new_one() {
    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instances.ketchup");
    let mut session = DocumentSession::new(SessionSettings::default());
    let initial = session
        .apply_rule_program(program(), false)
        .unwrap()
        .snapshot;
    let root = initial
        .occurrences()
        .find(|item| item.name() == "assembly")
        .unwrap();
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    let mut single = program();
    single.source = single
        .source
        .replace("i=instance(\"second\", c, at=(200,300,0), x=(0,1,0))", "")
        .replace("group(\"all\",[c,i])", "");
    let removed = reopened
        .apply_rule_program(single.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(removed.occurrences().count(), 1);
    assert_eq!(
        removed.occurrence(root.id()).unwrap().definition_id(),
        root.definition_id()
    );
    assert_eq!(removed.groups().count(), 0);
    single
        .source
        .push_str("\ninstance(\"new-copy\",c,at=(300,0,0))\n");
    let added = reopened
        .apply_rule_program(single.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(added.definitions().count(), 3);
    assert_eq!(added.occurrences().count(), 2);
    assert!(
        added
            .occurrences()
            .all(|item| item.definition_id() == root.definition_id())
    );
    reopened
        .save(&path, SaveOptions { overwrite: true })
        .unwrap();
    drop(reopened);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.rule_program(), Some(&single));
    assert_eq!(reopened.snapshot().scene_query(), added.scene_query());
}

#[test]
fn instance_can_switch_shared_definition_without_replacing_its_root() {
    let mut source = program();
    source.source = source.source.replace(
        "i=instance",
        "d=box(\"other-part\",(8,9,10))\nother=component(\"other\",[d])\ni=instance",
    );
    let mut session = DocumentSession::new(SessionSettings::default());
    let initial = session
        .apply_rule_program(source.clone(), false)
        .unwrap()
        .snapshot;
    let second = initial
        .occurrences()
        .find(|item| item.name() == "second")
        .unwrap();
    let other = initial
        .occurrences()
        .find(|item| item.name() == "other")
        .unwrap();
    source.source = source
        .source
        .replace("instance(\"second\", c,", "instance(\"second\", other,");
    let changed = session
        .apply_rule_program(source.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(
        changed.occurrence(second.id()).unwrap().definition_id(),
        other.definition_id()
    );
    assert_eq!(changed.definitions().count(), initial.definitions().count());
    let scene = changed.scene_query();
    let children = scene
        .iter()
        .filter(|item| item.occurrence_id == second.id() && !item.instance_path.is_root())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].occurrence_name, "other-part");
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_same_layout(
        &changed,
        &fresh.apply_rule_program(source, false).unwrap().snapshot,
    );
    assert_eq!(session.undo().unwrap().scene_query(), initial.scene_query());
}
