use ketchup_application::{DocumentSession, SaveOptions, SessionSettings, rule_program_part_name};
use ketchup_model::document::{InstancePathStep, RuleProgramSource};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn program() -> RuleProgramSource {
    RuleProgramSource {
        file_name: "nested.star".into(),
        source: r#"
a=box("a",(10,20,30),at=(1,2,3))
hole(a,"z+",at=(4,5),diameter=2,depth=6)
c=component("child",[a])
i=instance("inner",c,at=(100,200,0),x=(0,1,0))
p=component("parent",[group("pair",[c,i])])
j=instance("outer",p,at=(400,500,0),x=(0,1,0))
g=component("grand",[p,j])
instance("last",g,at=(1000,0,0),x=(-1,0,0))
"#
        .into(),
        overrides: BTreeMap::new(),
    }
}

#[test]
fn nested_updates_keep_paths_and_match_fresh_geometry_after_save_and_undo() {
    let _turn = crate::integration_support::file_turn();
    let mut original = program();
    original
        .source
        .push_str("\ngroup(\"world\",[g,group(\"copies\",[\"last\"])])\n");
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let mut changed = original.clone();
    changed.source = changed
        .source
        .replace("(10,20,30)", "(15,25,35)")
        .replace("(100,200,0),x=(0,1,0)", "(120,210,10),x=(-1,0,0)")
        .replace("at=(4,5)", "at=(6,7)");
    let applied = session.apply_rule_program(changed.clone(), false).unwrap();
    let after = &applied.snapshot;
    for member in before.local_occurrences() {
        let updated = after.local_occurrence(member.key()).unwrap();
        assert_eq!(updated.name(), member.name());
        assert_eq!(updated.definition_id(), member.definition_id());
    }
    for root in before.occurrences() {
        assert_eq!(after.occurrence(root.id()).unwrap(), root);
    }
    let named = |snapshot: &ketchup_model::document::Snapshot| {
        snapshot
            .scene_query()
            .into_iter()
            .map(|p| {
                (
                    rule_program_part_name(snapshot, &p.instance_path).unwrap(),
                    (p.instance_path, *p.transform.matrix()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let old_paths = named(&before);
    let new_paths = named(after);
    for (name, (path, _)) in &old_paths {
        assert_eq!(&new_paths[name].0, path, "{name}");
    }
    assert_eq!(new_paths["inner/a"].1[3], 119.0);
    assert_eq!(new_paths["inner/a"].1[7], 208.0);
    assert_eq!(new_paths["inner/a"].1[11], 13.0);
    assert_eq!(new_paths["last/outer/inner/a"].1[3], 808.0);
    assert_eq!(new_paths["last/outer/inner/a"].1[7], -619.0);
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let rebuilt = fresh.apply_rule_program(changed.clone(), false).unwrap();
    assert_eq!(new_paths, named(&rebuilt.snapshot));
    assert_eq!(
        after.features().collect::<Vec<_>>(),
        rebuilt.snapshot.features().collect::<Vec<_>>()
    );
    assert_eq!(applied.model, rebuilt.model);
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("updated.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), after.scene_query());
    assert_eq!(
        reopened
            .apply_rule_program(changed, false)
            .unwrap()
            .snapshot
            .scene_query(),
        after.scene_query()
    );
    assert_eq!(
        reopened
            .apply_rule_program(original, false)
            .unwrap()
            .snapshot
            .scene_query(),
        before.scene_query()
    );
}

#[test]
fn nested_member_rebuild_and_instance_add_remove_repoint_regroup_match_fresh_build() {
    let _turn = crate::integration_support::file_turn();
    let mut original = program();
    original.source = original
        .source
        .replace(
            "i=instance",
            "b=box(\"b\",(5,6,7),at=(20,0,0))\nd=component(\"other\",[b])\ni=instance",
        )
        .replace("[group(\"pair\",[c,i])]", "[group(\"pair\",[c,i]),d]");
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let mut changed = original.clone();
    changed.source = changed
        .source
        .replace(
            "a=box(\"a\",(10,20,30),at=(1,2,3))",
            "a=extrude(\"a\",profile=[(0,0),(10,0),(0,20)],distance=30,at=(1,2,3))",
        )
        .replace(
            "c=component(\"child\",[a])",
            "q=box(\"q\",(3,4,5),at=(50,0,0))\nc=component(\"child\",[group(\"inside\",[a,q])])",
        )
        .replace("hole(a,\"z+\"", "hole(a,\"end\"")
        .replace("b=box(\"b\"", "b=box(\"replacement\"")
        .replace("i=instance(\"inner\",c,", "i=instance(\"inner\",d,")
        .replace(
            "p=component",
            "k=instance(\"bonus\",c,at=(300,0,0))\np=component",
        )
        .replace(
            "[group(\"pair\",[c,i]),d]",
            "[group(\"wrapper\",[group(\"pair\",[c,i]),k]),d]",
        )
        .replace("j=instance(\"outer\",p,at=(400,500,0),x=(0,1,0))", "")
        .replace("g=component(\"grand\",[p,j])", "g=component(\"grand\",[p])");
    let applied = session.apply_rule_program(changed.clone(), false).unwrap();
    let after = &applied.snapshot;
    for member in before
        .local_occurrences()
        .filter(|m| m.name() != "outer" && m.name() != "b")
    {
        let updated = after.local_occurrence(member.key()).unwrap();
        assert_eq!(updated.name(), member.name());
        if member.name() == "a" || member.name() == "inner" {
            assert_ne!(updated.definition_id(), member.definition_id());
        } else {
            assert_eq!(updated.definition_id(), member.definition_id());
        }
    }
    let pair = before.local_groups().find(|g| g.name() == "pair").unwrap();
    assert_eq!(after.local_group(pair.key()).unwrap().name(), "pair");
    assert!(after.local_group(pair.key()).unwrap().parent().is_some());
    assert!(
        !after
            .local_occurrences()
            .any(|m| m.name() == "outer" || m.name() == "b")
    );
    assert_eq!(after.definitions().count(), 7);
    let named = |snapshot: &ketchup_model::document::Snapshot| {
        snapshot
            .scene_query()
            .into_iter()
            .filter(|p| {
                !snapshot
                    .definition(p.definition_id)
                    .unwrap()
                    .feature_ids()
                    .is_empty()
            })
            .map(|p| {
                (
                    rule_program_part_name(snapshot, &p.instance_path).unwrap(),
                    *p.transform.matrix(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let actual = named(after);
    assert_eq!(actual.len(), 12);
    assert_eq!(actual["inner/replacement"][3], 100.0);
    assert_eq!(actual["inner/replacement"][7], 220.0);
    assert_eq!(actual["last/bonus/q"][3], 650.0);
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let rebuilt = fresh.apply_rule_program(changed.clone(), false).unwrap();
    assert_eq!(actual, named(&rebuilt.snapshot));
    assert_eq!(applied.model, rebuilt.model);
    assert_eq!(
        after.definitions().count(),
        rebuilt.snapshot.definitions().count()
    );
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("restructured.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened
            .apply_rule_program(changed, false)
            .unwrap()
            .snapshot
            .scene_query(),
        after.scene_query()
    );
    let reversed = reopened
        .apply_rule_program(original.clone(), false)
        .unwrap();
    assert_eq!(named(&reversed.snapshot), named(&before));
    assert_eq!(reopened.rule_program(), Some(&original));
}

#[test]
fn nested_components_build_shared_definitions_and_compose_every_leaf_frame() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session.apply_rule_program(program(), false).unwrap();
    let snapshot = &applied.snapshot;
    assert_eq!(snapshot.definitions().count(), 4);
    assert_eq!(snapshot.occurrences().count(), 2);
    assert_eq!(snapshot.local_occurrences().count(), 5);
    assert_eq!(snapshot.local_groups().count(), 1);
    assert_eq!(snapshot.groups().count(), 0);
    let scene = snapshot.scene_query();
    assert_eq!(scene.len(), 22);
    let leaves = scene
        .iter()
        .filter(|part| {
            !snapshot
                .definition(part.definition_id)
                .unwrap()
                .feature_ids()
                .is_empty()
        })
        .collect::<Vec<_>>();
    assert_eq!(leaves.len(), 8);
    assert_eq!(
        leaves
            .iter()
            .map(|p| p.definition_id)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    let named = leaves
        .iter()
        .map(|leaf| {
            (
                rule_program_part_name(snapshot, &leaf.instance_path).unwrap(),
                leaf,
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        named.keys().cloned().collect::<BTreeSet<_>>(),
        applied.model.parts.iter().map(|p| p.name.clone()).collect()
    );
    for part in &applied.model.parts {
        let leaf = named[&part.name];
        assert_eq!(
            leaf.transform.matrix(),
            &part.transform_matrix(),
            "{}",
            part.name
        );
        assert_eq!(
            leaf.instance_path
                .steps()
                .iter()
                .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
                .count(),
            3
        );
    }
    assert_eq!(named["outer/inner/a"].transform.matrix()[3], 199.0);
    assert_eq!(named["last/outer/inner/a"].transform.matrix()[7], -598.0);
    let child = snapshot
        .definitions()
        .find(|def| def.name() == "child")
        .unwrap();
    assert_eq!(
        snapshot
            .local_occurrences()
            .filter(|member| member.definition_id() == child.id())
            .count(),
        2
    );
    let parent = snapshot
        .definitions()
        .find(|def| def.name() == "parent")
        .unwrap();
    assert_eq!(
        snapshot
            .local_occurrences()
            .filter(|member| member.definition_id() == parent.id())
            .count(),
        2
    );
    assert_eq!(session.undo().unwrap().scene_query().len(), 0);
    assert_eq!(session.redo().unwrap().scene_query(), scene);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), scene);
    assert_eq!(reopened.rule_program(), Some(&program()));
}
