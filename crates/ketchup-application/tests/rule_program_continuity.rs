use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use std::collections::BTreeMap;

fn source(text: &str) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "continuity.star".into(),
        source: text.into(),
        overrides: BTreeMap::new(),
    }
}

#[test]
fn splitting_a_named_part_keeps_the_chosen_identity_and_one_undo_step() {
    let _turn = crate::integration_support::file_turn();
    let original = source(
        "a=box(\"top\",(100,20,10))\nb=box(\"side\",(10,20,30),at=(150,0,0))\ngroup(\"body\",[a,b])\n",
    );
    let changed = source(
        "a=box(\"left\",(40,20,10))\ncontinue_part(a,was=\"top\")\nc=box(\"right\",(60,20,10),at=(40,0,0))\nb=box(\"side\",(10,20,30),at=(150,0,0))\ngroup(\"body\",[a,b,c])\n",
    );
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let top = before.occurrences().find(|p| p.name() == "top").unwrap();
    let applied = session.apply_rule_program(changed.clone(), false).unwrap();
    let after = &applied.snapshot;
    let left = after.occurrences().find(|p| p.name() == "left").unwrap();
    let right = after.occurrences().find(|p| p.name() == "right").unwrap();
    assert_eq!(left.id(), top.id());
    assert_eq!(after.occurrences().count(), 3);
    assert_ne!(right.id(), top.id());
    assert_eq!(left.parent(), top.parent());
    assert_eq!(
        after.definition(left.definition_id()).unwrap().name(),
        "left"
    );
    let side = before.occurrences().find(|p| p.name() == "side").unwrap();
    assert_eq!(after.occurrence(side.id()).unwrap(), side);
    assert_eq!(
        applied.model.part("left").unwrap().size_mm,
        [40.0, 20.0, 10.0]
    );
    assert_eq!(applied.model.part("right").unwrap().at_mm, [40.0, 0.0, 0.0]);
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let rebuilt = fresh.apply_rule_program(changed.clone(), false).unwrap();
    assert_eq!(applied.model, rebuilt.model);
    let mut worker = crate::operations_support::worker();
    for (name, expected, bounds) in [
        ("left", 8000.0, [[0.0, 0.0, 0.0], [40.0, 20.0, 10.0]]),
        ("right", 12000.0, [[40.0, 0.0, 0.0], [100.0, 20.0, 10.0]]),
    ] {
        let occurrence = after.occurrences().find(|p| p.name() == name).unwrap();
        let definition = after.definition(occurrence.definition_id()).unwrap();
        let graph = ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(
            after,
            definition.id(),
            *definition.feature_ids().last().unwrap(),
        )
        .unwrap();
        let solid = worker.evaluate_exact_brep_graph(&graph).unwrap();
        crate::operations_support::assert_volume(solid.volume_mm3, expected);
        for (index, (local, expected)) in solid
            .bounds_mm
            .into_iter()
            .flatten()
            .zip(bounds.into_iter().flatten())
            .enumerate()
        {
            let measured = local + occurrence.transform().matrix()[(index % 3) * 4 + 3];
            assert!(
                (measured - expected).abs() < 1e-5,
                "{name}: {measured} != {expected}"
            );
        }
    }
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&original));
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("split.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    let mut next = changed.clone();
    next.source = next
        .source
        .replace("(40,20,10)", "(35,20,10)")
        .replace("(60,20,10)", "(65,20,10)")
        .replace("at=(40,0,0)", "at=(35,0,0)");
    let updated = reopened.apply_rule_program(next, false).unwrap();
    assert_eq!(
        updated.snapshot.occurrence(top.id()).unwrap().name(),
        "left"
    );
    assert_eq!(
        updated.model.part("left").unwrap().size_mm,
        [35.0, 20.0, 10.0]
    );
    assert_eq!(updated.model.part("right").unwrap().at_mm, [35.0, 0.0, 0.0]);
}

#[test]
fn nested_shared_continuity_keeps_paths_and_real_split_geometry() {
    let _turn = crate::integration_support::file_turn();
    let original = source(
        "a=box(\"top\",(100,20,10))\nc=component(\"child\",[a])\ni=instance(\"inner\",c,at=(200,0,0))\np=component(\"parent\",[c,i])\ninstance(\"outer\",p,at=(0,300,0),x=(0,1,0))\n",
    );
    let changed = source(&original.source.replace(
        "a=box(\"top\",(100,20,10))",
        "a=box(\"left\",(40,20,10))\ncontinue_part(a,was=\"top\")\nb=box(\"right\",(60,20,10),at=(40,0,0))",
    ).replace("[a]", "[a,b]"));
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let applied = session.apply_rule_program(changed.clone(), false).unwrap();
    let after = &applied.snapshot;
    let top = before
        .local_occurrences()
        .find(|p| p.name() == "top")
        .unwrap();
    let left = after.local_occurrence(top.key()).unwrap();
    assert_eq!(left.name(), "left");
    assert_eq!(left.parent(), top.parent());
    let right = after
        .local_occurrences()
        .find(|p| p.name() == "right")
        .unwrap();
    assert_ne!(right.key(), top.key());
    let scene = after.scene_query();
    assert_eq!(
        scene
            .iter()
            .filter(|p| p.definition_id == left.definition_id()
                || p.definition_id == right.definition_id())
            .count(),
        8
    );
    for old in before
        .scene_query()
        .into_iter()
        .filter(|p| p.definition_id == top.definition_id())
    {
        let leaf = scene
            .iter()
            .find(|p| p.instance_path == old.instance_path)
            .unwrap();
        assert_eq!(leaf.occurrence_name, "left");
        assert_eq!(leaf.transform, old.transform);
        let name = ketchup_application::rule_program_part_name(after, &leaf.instance_path).unwrap();
        assert!(applied.model.part(&name).is_some(), "{name}");
    }
    for member in before.local_occurrences().filter(|p| p.key() != top.key()) {
        assert_eq!(after.local_occurrence(member.key()).unwrap(), member);
    }
    let mut worker = crate::operations_support::worker();
    for (member, volume) in [(left, 8000.0), (right, 12000.0)] {
        let definition = after.definition(member.definition_id()).unwrap();
        let graph = ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(
            after,
            definition.id(),
            *definition.feature_ids().last().unwrap(),
        )
        .unwrap();
        let solid = worker.evaluate_exact_brep_graph(&graph).unwrap();
        crate::operations_support::assert_volume(solid.volume_mm3, volume);
    }
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&original));
    assert_eq!(session.redo().unwrap().scene_query(), scene);
    assert_eq!(session.rule_program(), Some(&changed));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared-split.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), scene);
    assert_eq!(
        reopened
            .apply_rule_program(changed.clone(), false)
            .unwrap()
            .snapshot
            .scene_query(),
        scene
    );
    let next = source(&changed.source.replace("(40,20,10)", "(35,20,10)"));
    let updated = reopened.apply_rule_program(next, false).unwrap();
    assert_eq!(
        updated.snapshot.local_occurrence(top.key()).unwrap().name(),
        "left"
    );
    assert_eq!(
        updated.model.part("outer/inner/left").unwrap().size_mm,
        [35.0, 20.0, 10.0]
    );
}

#[test]
fn root_and_shared_parts_can_split_together_while_adding_an_instance() {
    let _turn = crate::integration_support::file_turn();
    let original = source(
        "a=box(\"root\",(100,20,10))\nb=box(\"shared\",(100,20,10),at=(0,100,0))\nc=component(\"child\",[b])\n",
    );
    let changed = source(
        "a=box(\"root-left\",(40,20,10))\ncontinue_part(a,was=\"root\")\nbox(\"root-right\",(60,20,10),at=(40,0,0))\nb=box(\"shared-left\",(30,20,10),at=(0,100,0))\ncontinue_part(b,was=\"shared\")\nd=box(\"shared-right\",(70,20,10),at=(30,100,0))\nc=component(\"child\",[b,d])\ninstance(\"copy\",c,at=(200,0,0))\n",
    );
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let root = before.occurrences().find(|p| p.name() == "root").unwrap();
    let member = before
        .local_occurrences()
        .find(|p| p.name() == "shared")
        .unwrap();
    let after = session
        .apply_rule_program(changed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(after.occurrence(root.id()).unwrap().name(), "root-left");
    assert_eq!(
        after.local_occurrence(member.key()).unwrap().name(),
        "shared-left"
    );
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let expected = fresh.apply_rule_program(changed, false).unwrap().snapshot;
    let geometry = |snapshot: &ketchup_model::document::Snapshot| {
        snapshot
            .scene_query()
            .into_iter()
            .map(|p| {
                (
                    ketchup_application::rule_program_part_name(snapshot, &p.instance_path)
                        .unwrap(),
                    (p.definition_name, *p.transform.matrix()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(geometry(&after), geometry(&expected));
    assert_eq!(after.occurrences().count(), 4);
    assert_eq!(after.local_occurrences().count(), 2);
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&original));
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
}

#[test]
fn shared_identity_rejects_owner_changes_and_independent_instance_claims() {
    let _turn = crate::integration_support::file_turn();
    let original = source(
        "a=box(\"top\",(100,20,10))\nc=component(\"child\",[a])\ninstance(\"copy\",c,at=(200,0,0))\n",
    );
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    for text in [
        "a=box(\"left\",(100,20,10))\ncontinue_part(a,was=\"top\")\nc=component(\"child\",[])\ninstance(\"copy\",c,at=(200,0,0))",
        "a=box(\"left\",(100,20,10))\nc=component(\"child\",[a])\ninstance(\"copy\",c,at=(200,0,0))\ncontinue_part(\"copy/left\",was=\"copy/top\")",
        "a=box(\"left\",(100,20,10))\ncontinue_part(a,was=\"missing\")\nc=component(\"child\",[a])\ninstance(\"copy\",c,at=(200,0,0))",
    ] {
        let error = session
            .apply_rule_program(source(text), false)
            .err()
            .expect("invalid continuity")
            .to_string();
        assert!(error.contains("continue_part"), "{error}");
        assert_eq!(session.snapshot().scene_query(), before.scene_query());
        assert_eq!(session.rule_program(), Some(&original));
    }
}

#[test]
fn ambiguous_or_missing_identity_does_not_change_geometry_or_program() {
    let _turn = crate::integration_support::file_turn();
    let original = source("box(\"top\",(100,20,10))");
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    for invalid in [
        "a=box(\"left\",(40,20,10))\ncontinue_part(a,was=\"missing\")",
        "a=box(\"top\",(100,20,10))\ncontinue_part(a,was=\"missing\")",
        "a=box(\"left\",(40,20,10))\ncontinue_part(a,was=\"top\")\nb=box(\"right\",(60,20,10))\ncontinue_part(b,was=\"top\")",
        "a=box(\"left\",(40,20,10))\ncontinue_part(a,was=\"top\")\nbox(\"top\",(60,20,10))",
    ] {
        let error = session
            .apply_rule_program(source(invalid), false)
            .err()
            .expect("invalid identity must be rejected")
            .to_string();
        assert!(
            error.contains("continue_part")
                && (error.contains("identity") || error.contains("previous")),
            "{error}"
        );
        assert_eq!(session.snapshot().scene_query(), before.scene_query());
        assert_eq!(session.rule_program(), Some(&original));
    }
}
