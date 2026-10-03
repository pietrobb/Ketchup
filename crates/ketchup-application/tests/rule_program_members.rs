use super::rule_program_components::program;
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::Snapshot;
use std::collections::BTreeMap;

fn placements(snapshot: &Snapshot) -> BTreeMap<(String, String), [f64; 16]> {
    snapshot
        .scene_query()
        .into_iter()
        .filter(|p| !p.instance_path.is_root())
        .map(|p| {
            (
                (
                    snapshot
                        .occurrence(p.occurrence_id)
                        .unwrap()
                        .name()
                        .to_owned(),
                    p.occurrence_name,
                ),
                *p.transform.matrix(),
            )
        })
        .collect()
}

#[test]
fn rebuilding_shared_machining_keeps_member_paths_and_changes_real_voids_in_every_copy() {
    let _turn = crate::integration_support::file_turn();
    let mut original = program();
    original.source = "p=box(\"p\",(20,20,20))\nhole(p,\"z+\",at=(10,10),diameter=4,depth=8,id=\"bore\")\na=box(\"top-probe\",(1,1,1),at=(9.5,9.5,16))\nb=box(\"side-probe\",(1,1,1),at=(16,9.5,9.5))\nc=component(\"assembly\",[p,a,b])\ninstance(\"copy\",c,at=(100,100,0),x=(0,1,0))\n".into();
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let collisions = |snapshot: &Snapshot| {
        let exact = ketchup_application::validation::assistant_validation_context_with_worker(
            snapshot,
            &ketchup_model::exact_product::ExactResultRegistry::default(),
            &ketchup_application::AssistantValidationSelection::only(&["collision"]),
            &ketchup_model::persistence::ContainerData::default(),
            None,
            std::time::Duration::from_secs(60),
        );
        assert_eq!(exact["complete"], true, "{exact}");
        let mut pairs = exact["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| {
                assert_eq!(issue["evidence_class"], "exact", "{issue}");
                let mut names = [
                    issue["left_name"].as_str().unwrap().to_owned(),
                    issue["right_name"].as_str().unwrap().to_owned(),
                ];
                names.sort();
                names
            })
            .collect::<Vec<_>>();
        pairs.sort();
        pairs
    };
    assert_eq!(
        collisions(&before),
        vec![["p".to_owned(), "side-probe".to_owned()]; 2]
    );
    let mut changed = original.clone();
    changed.source = changed.source.replace("\"z+\"", "\"x+\"");
    let after = session
        .apply_rule_program(changed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(
        collisions(&after),
        vec![["p".to_owned(), "top-probe".to_owned()]; 2]
    );
    assert_eq!(placements(&after), placements(&before));
    assert_eq!(after.definitions().count(), before.definitions().count());
    for root in before.occurrences() {
        assert_eq!(after.occurrence(root.id()).unwrap(), root);
    }
    for member in before.local_occurrences() {
        let updated = after.local_occurrence(member.key()).unwrap();
        if member.name() == "p" {
            assert_ne!(updated.definition_id(), member.definition_id());
            assert!(after.definition(member.definition_id()).is_none());
            for id in before
                .definition(member.definition_id())
                .unwrap()
                .feature_ids()
            {
                assert!(after.feature(*id).is_none());
            }
            assert_eq!(
                after
                    .scene_query()
                    .iter()
                    .filter(|p| p.definition_id == updated.definition_id())
                    .count(),
                2
            );
        } else {
            assert_eq!(updated, member);
        }
    }
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&original));
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rebuilt-members.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), after.scene_query());
    assert_eq!(collisions(&reopened.snapshot()), collisions(&after));
    assert_eq!(
        reopened
            .apply_rule_program(changed.clone(), false)
            .unwrap()
            .snapshot
            .scene_query(),
        after.scene_query()
    );
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let built = fresh.apply_rule_program(changed, false).unwrap().snapshot;
    assert_eq!(collisions(&built), collisions(&after));
    assert_eq!(placements(&built), placements(&after));
}

#[test]
fn an_added_shared_member_has_a_real_drilled_void_not_just_a_feature_record() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let mut source = program();
    source.source.push_str(
        "box(\"in-hole\",(1,1,1),at=(64.5,75.5,90))\nbox(\"in-solid\",(1,1,1),at=(60.5,70.5,90))\n",
    );
    session.apply_rule_program(source.clone(), false).unwrap();
    source.source = source.source.replace("g=group(\"inner\", [a])", "d=box(\"d\",(10,12,14),at=(60,70,80))\nhole(d,\"z+\",at=(5,6),diameter=4,depth=8,id=\"bore\")\ng=group(\"inner\", [a,d])");
    let result = session.apply_rule_program(source, false).unwrap();
    let exact = ketchup_application::validation::assistant_validation_context_with_worker(
        &result.snapshot,
        &ketchup_model::exact_product::ExactResultRegistry::default(),
        &ketchup_application::AssistantValidationSelection::only(&["collision"]),
        &ketchup_model::persistence::ContainerData::default(),
        None,
        std::time::Duration::from_secs(60),
    );
    assert_eq!(exact["complete"], true, "{exact}");
    assert_eq!(exact["issue_count"], 1, "{exact}");
    let collision = &exact["issues"][0];
    let names = [
        collision["left_name"].as_str().unwrap(),
        collision["right_name"].as_str().unwrap(),
    ];
    assert!(names.contains(&"d"), "{exact}");
    assert!(names.contains(&"in-solid"), "{exact}");
    assert_eq!(collision["evidence_class"], "exact", "{exact}");
}

#[test]
fn simultaneous_rebuilds_additions_and_regrouping_share_one_definition_per_member() {
    let mut original = program();
    original.source.push_str("x=box(\"x\",(20,20,20),at=(600,0,0))\nk=component(\"other\",[x])\ninstance(\"other-copy\",k,at=(0,200,0))\n");
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let mut changed = original.clone();
    changed.source = changed.source
        .replace("g=group(\"inner\", [a])", "hole(a,\"z+\",at=(5,5),diameter=2,depth=4,id=\"new-bore\")\nd=box(\"d\",(5,6,7),at=(70,0,0))\ng=group(\"inner\", [a,d])")
        .replace("k=component", "hole(x,\"x+\",at=(10,10),diameter=4,depth=8,id=\"other-bore\")\nk=component");
    let after = session
        .apply_rule_program(changed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(
        after.definitions().count(),
        before.definitions().count() + 1
    );
    assert_eq!(
        after.local_occurrences().count(),
        before.local_occurrences().count() + 1
    );
    for member in before.local_occurrences() {
        let updated = after.local_occurrence(member.key()).unwrap();
        if member.name() == "a" || member.name() == "x" {
            assert_ne!(updated.definition_id(), member.definition_id());
            assert!(after.definition(member.definition_id()).is_none());
            assert_eq!(
                after
                    .scene_query()
                    .iter()
                    .filter(|p| p.definition_id == updated.definition_id())
                    .count(),
                2
            );
        } else {
            assert_eq!(updated, member);
        }
    }
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let built = fresh
        .apply_rule_program(changed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(placements(&after), placements(&built));
    let feature_counts = |snapshot: &Snapshot| {
        snapshot
            .local_occurrences()
            .map(|member| {
                (
                    member.name().to_owned(),
                    snapshot
                        .definition(member.definition_id())
                        .unwrap()
                        .feature_ids()
                        .len(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(feature_counts(&after), feature_counts(&built));
    assert_eq!(
        session
            .apply_rule_program(changed, false)
            .unwrap()
            .snapshot
            .scene_query(),
        after.scene_query()
    );
    assert_eq!(
        session
            .apply_rule_program(original, false)
            .unwrap()
            .snapshot
            .definitions()
            .count(),
        before.definitions().count()
    );
}

#[test]
fn simultaneous_membership_changes_in_two_components_keep_unrelated_geometry() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let mut original = program();
    original.source.push_str("u=box(\"standalone\",(5,6,7),at=(900,0,0))\nx=box(\"x\",(8,9,10),at=(600,0,0))\ny=box(\"y\",(11,12,13),at=(650,0,0))\nk=component(\"other\",[x,y])\ninstance(\"other-copy\",k,at=(0,200,0))\n");
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let standalone = before
        .occurrences()
        .find(|p| p.name() == "standalone")
        .unwrap();
    let mut changed = original.clone();
    changed.source = changed
        .source
        .replace("box(\"b\"", "box(\"replacement-b\"")
        .replace("box(\"y\"", "box(\"replacement-y\"")
        .replace("g=group(\"inner\", [a])", "g=group(\"new-group\", [a,b])")
        .replace("[g,b]", "[g]");
    let after = session
        .apply_rule_program(changed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(after.local_occurrences().count(), 4);
    assert_eq!(after.definitions().count(), 7);
    assert_eq!(after.occurrence(standalone.id()).unwrap(), standalone);
    for name in ["a", "x"] {
        let old = before
            .local_occurrences()
            .find(|p| p.name() == name)
            .unwrap();
        let new = after.local_occurrence(old.key()).unwrap();
        assert_eq!(new.definition_id(), old.definition_id());
        assert_eq!(new.transform(), old.transform());
    }
    for name in ["b", "y"] {
        let removed = before
            .local_occurrences()
            .find(|p| p.name() == name)
            .unwrap();
        assert!(after.local_occurrence(removed.key()).is_none());
        assert!(after.definition(removed.definition_id()).is_none());
    }
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_eq!(
        placements(&after),
        placements(
            &fresh
                .apply_rule_program(changed.clone(), false)
                .unwrap()
                .snapshot
        )
    );
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("two-components.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), after.scene_query());
    let again = reopened
        .apply_rule_program(changed, false)
        .unwrap()
        .snapshot;
    assert_eq!(again.scene_query(), after.scene_query());
}

#[test]
fn shared_members_can_be_added_and_removed_in_all_copies_without_replacing_survivors() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = program();
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let a = before
        .local_occurrences()
        .find(|p| p.name() == "a")
        .unwrap()
        .clone();
    let b = before
        .local_occurrences()
        .find(|p| p.name() == "b")
        .unwrap()
        .clone();
    let mut added = original.clone();
    added.source = added.source.replace("g=group(\"inner\", [a])", "d=box(\"d\", (10,12,14), at=(60,70,80))\nhole(d, \"z+\", at=(5,6), diameter=4, depth=8, id=\"bore\")\nh=group(\"new-group\", [d])\ng=group(\"inner\", [a,h])");
    added
        .source
        .push_str("j=instance(\"third\", c, at=(500,0,0))\n");
    let result = session.apply_rule_program(added.clone(), false).unwrap();
    let after = result.snapshot;
    assert_eq!(after.local_occurrences().count(), 3);
    assert_eq!(after.definitions().count(), 4);
    assert_eq!(after.local_occurrence(a.key()).unwrap(), &a);
    assert_eq!(after.local_occurrence(b.key()).unwrap(), &b);
    for root in before.occurrences() {
        assert_eq!(after.occurrence(root.id()).unwrap(), root);
    }
    let d = after.local_occurrences().find(|p| p.name() == "d").unwrap();
    assert!(d.parent().is_some());
    let positions = placements(&after);
    assert_eq!(positions.len(), 9);
    assert_eq!(positions[&("assembly".into(), "d".into())][3], 60.0);
    assert_eq!(positions[&("second".into(), "d".into())][3], 130.0);
    assert_eq!(positions[&("second".into(), "d".into())][7], 360.0);
    assert_eq!(positions[&("third".into(), "d".into())][3], 560.0);
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let rebuilt = fresh.apply_rule_program(added.clone(), false).unwrap();
    assert_eq!(positions, placements(&rebuilt.snapshot));
    let kinds = |snapshot: &Snapshot, name: &str| {
        let member = snapshot
            .local_occurrences()
            .find(|p| p.name() == name)
            .unwrap();
        snapshot
            .definition(member.definition_id())
            .unwrap()
            .feature_ids()
            .len()
    };
    assert_eq!(kinds(&after, "d"), kinds(&rebuilt.snapshot, "d"));
    assert!(kinds(&after, "d") > kinds(&after, "a"));
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared-members.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut session = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(session.snapshot().scene_query(), after.scene_query());
    let mut removed = added.clone();
    removed.source = removed
        .source
        .replace("b=box(\"b\", (w,30,10), at=(10,20,40))", "")
        .replace("[g,b]", "[g]");
    let reduced = session
        .apply_rule_program(removed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(reduced.local_occurrences().count(), 2);
    assert_eq!(reduced.definitions().count(), 3);
    assert!(reduced.definition(b.definition_id()).is_none());
    assert!(
        reduced
            .scene_query()
            .iter()
            .all(|p| p.occurrence_name != "b")
    );
    assert_eq!(reduced.local_occurrence(a.key()).unwrap(), &a);
    assert_eq!(reduced.local_occurrence(d.key()).unwrap(), d);
    assert_eq!(placements(&reduced).len(), 6);
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_eq!(
        placements(&reduced),
        placements(
            &fresh
                .apply_rule_program(removed.clone(), false)
                .unwrap()
                .snapshot
        )
    );
    assert_eq!(session.undo().unwrap().scene_query(), after.scene_query());
    assert_eq!(session.redo().unwrap().scene_query(), reduced.scene_query());
    assert_eq!(session.rule_program(), Some(&removed));
}
