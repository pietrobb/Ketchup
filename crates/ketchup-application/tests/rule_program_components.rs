use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use std::collections::BTreeMap;

pub(super) fn program() -> RuleProgramSource {
    RuleProgramSource {
        file_name: "components.star".into(),
        source: r#"
w=param("width",20)
a=box("a", (w,30,40), at=(10,20,0))
b=box("b", (w,30,10), at=(10,20,40))
g=group("inner", [a])
c=component("assembly", [g,b])
i=instance("second", c, at=(200,300,0), x=(0,1,0))
group("all",[c,i])
"#
        .into(),
        overrides: BTreeMap::new(),
    }
}

fn identities(snapshot: &Snapshot) -> Vec<(String, u64, u64)> {
    snapshot
        .occurrences()
        .map(|part| (part.name().to_owned(), part.id().0, part.definition_id().0))
        .collect()
}

fn widths(snapshot: &Snapshot) -> Vec<f64> {
    snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            ketchup_model::document::FeatureKind::Sketch(sketch) => {
                let regions = sketch.solved_regions().unwrap();
                match &regions[0].outer {
                    ketchup_geometry::sketch::SolvedSketchRegionProfile::Polyline(points) => {
                        Some(points.clone())
                    }
                    other => panic!("expected rectangle, got {other:?}"),
                }
            }
            _ => None,
        })
        .map(|points| {
            let min = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
            let max = points
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max);
            max - min
        })
        .collect()
}

#[test]
fn components_share_one_definition_and_preserve_ids_during_geometry_and_placement_edits() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let first = session
        .apply_rule_program(program(), false)
        .unwrap()
        .snapshot;
    let roots = first.occurrences().collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].definition_id(), roots[1].definition_id());
    assert_eq!(first.definitions().count(), 3);
    assert_eq!(first.local_occurrences().count(), 2);
    assert_eq!(
        first
            .definitions()
            .map(|definition| definition.local_group_ids().len())
            .sum::<usize>(),
        1
    );
    assert_eq!(first.groups().count(), 1);
    assert_eq!(
        first
            .scene_query()
            .iter()
            .filter(|part| !part.instance_path.is_root())
            .count(),
        4
    );
    let ids = identities(&first);
    let local = first
        .local_occurrences()
        .map(|item| (item.key(), item.definition_id()))
        .collect::<Vec<_>>();
    let initial_scene = first.scene_query();
    let second = roots.iter().find(|root| root.name() == "second").unwrap();
    let copied_a = initial_scene
        .iter()
        .find(|part| part.occurrence_id == second.id() && part.occurrence_name == "a")
        .unwrap();
    assert_eq!(
        copied_a.transform.matrix(),
        &[
            0.0, -1.0, 0.0, 180.0, 1.0, 0.0, 0.0, 310.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0
        ]
    );
    let mut wider = program();
    wider.overrides.insert("width".into(), 35.0);
    let updated = session
        .apply_rule_program(wider.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(identities(&updated), ids);
    assert_eq!(
        updated
            .local_occurrences()
            .map(|item| (item.key(), item.definition_id()))
            .collect::<Vec<_>>(),
        local
    );
    assert_eq!(widths(&first), [20.0, 20.0]);
    assert_eq!(widths(&updated), [35.0, 35.0]);
    assert_eq!(updated.definitions().count(), 3);
    let wider_scene = updated.scene_query();
    wider.source = wider.source.replace("(200,300,0)", "(400,500,0)");
    let moved = session.apply_rule_program(wider, false).unwrap().snapshot;
    assert_eq!(identities(&moved), ids);
    assert_ne!(moved.scene_query(), wider_scene);
    assert_eq!(session.undo().unwrap().scene_query(), wider_scene);
    let restored = session.undo().unwrap();
    assert_eq!(restored.scene_query(), initial_scene);
    assert_eq!(widths(&restored), [20.0, 20.0]);
    let redone = session.redo().unwrap();
    assert_eq!(redone.scene_query(), wider_scene);
    assert_eq!(widths(&redone), [35.0, 35.0]);
}

#[test]
fn shared_components_reopen_and_remain_parametric() {
    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("components.ketchup");
    let mut session = DocumentSession::new(SessionSettings::default());
    let before = session
        .apply_rule_program(program(), false)
        .unwrap()
        .snapshot;
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), before.scene_query());
    let mut changed = program();
    changed.overrides.insert("width".into(), 25.0);
    let after = reopened
        .apply_rule_program(changed, false)
        .unwrap()
        .snapshot;
    assert_eq!(identities(&after), identities(&before));
    assert_eq!(widths(&before), [20.0, 20.0]);
    assert_eq!(widths(&after), [25.0, 25.0]);
}

#[test]
fn shared_member_placement_updates_every_instance_without_rebuilding_geometry() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = program();
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let mut changed = original.clone();
    changed.source = changed.source.replace(
        "g=group",
        "place(a, origin=(70,80,90), x=(0,1,0), z=(0,0,1))\ng=group",
    );
    let after = session
        .apply_rule_program(changed.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(identities(&after), identities(&before));
    assert_eq!(
        after.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    assert_eq!(
        after.definitions().collect::<Vec<_>>(),
        before.definitions().collect::<Vec<_>>()
    );
    assert_eq!(
        after
            .local_occurrences()
            .map(|item| (item.key(), item.definition_id()))
            .collect::<Vec<_>>(),
        before
            .local_occurrences()
            .map(|item| (item.key(), item.definition_id()))
            .collect::<Vec<_>>()
    );
    let scene = after.scene_query();
    let original_a = scene
        .iter()
        .find(|part| {
            part.occurrence_name == "a"
                && after.occurrence(part.occurrence_id).unwrap().name() == "assembly"
        })
        .unwrap();
    let copied_a = scene
        .iter()
        .find(|part| {
            part.occurrence_name == "a"
                && after.occurrence(part.occurrence_id).unwrap().name() == "second"
        })
        .unwrap();
    assert_eq!(
        original_a.transform.matrix(),
        &[
            0.0, -1.0, 0.0, 70.0, 1.0, 0.0, 0.0, 80.0, 0.0, 0.0, 1.0, 90.0, 0.0, 0.0, 0.0, 1.0
        ]
    );
    assert_eq!(
        copied_a.transform.matrix(),
        &[
            -1.0, 0.0, 0.0, 120.0, 0.0, -1.0, 0.0, 370.0, 0.0, 0.0, 1.0, 90.0, 0.0, 0.0, 0.0, 1.0
        ]
    );
    for untouched in before
        .scene_query()
        .iter()
        .filter(|part| part.occurrence_name == "b")
    {
        assert_eq!(
            scene
                .iter()
                .find(|part| part.instance_path == untouched.instance_path),
            Some(untouched)
        );
    }
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_eq!(
        scene,
        fresh
            .apply_rule_program(changed.clone(), false)
            .unwrap()
            .snapshot
            .scene_query()
    );
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&original));
    assert_eq!(session.redo().unwrap().scene_query(), scene);
    assert_eq!(session.rule_program(), Some(&changed));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("member-placement.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), scene);
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
fn regrouping_shared_members_preserves_geometry_and_identity_in_all_instances() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = program();
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot;
    let mut regrouped = original.clone();
    regrouped.source = regrouped
        .source
        .replace("group(\"inner\", [a])", "group(\"inner\", [b])")
        .replace(
            "component(\"assembly\", [g,b])",
            "component(\"assembly\", [g,a])",
        );
    let after = session
        .apply_rule_program(regrouped.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(identities(&after), identities(&before));
    assert_eq!(
        after.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    assert_eq!(
        after.local_groups().collect::<Vec<_>>(),
        before.local_groups().collect::<Vec<_>>()
    );
    let group = after.local_groups().next().unwrap().key().local_id;
    for member in after.local_occurrences() {
        let old = before.local_occurrence(member.key()).unwrap();
        assert_eq!(member.definition_id(), old.definition_id());
        assert_eq!(member.transform(), old.transform());
        assert_eq!(
            member.parent(),
            if member.name() == "b" {
                Some(group)
            } else {
                None
            }
        );
    }
    let positions = |snapshot: &Snapshot| {
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
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(positions(&after), positions(&before));
    assert_eq!(
        after
            .scene_query()
            .iter()
            .filter(|part| !part.instance_path.is_root())
            .count(),
        4
    );
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let rebuilt = fresh
        .apply_rule_program(regrouped.clone(), false)
        .unwrap()
        .snapshot;
    assert_eq!(positions(&after), positions(&rebuilt));
    assert_eq!(session.undo().unwrap().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&original));
    assert_eq!(session.redo().unwrap().scene_query(), after.scene_query());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("regrouped.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().scene_query(), after.scene_query());
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
fn unsupported_structure_changes_do_not_flatten_components_or_replace_user_work() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let source = program();
    let before = session
        .apply_rule_program(source.clone(), false)
        .unwrap()
        .snapshot;
    let mut changed = source.clone();
    changed.source = changed.source.replace(
        "component(\"assembly\", [g,b])",
        "component(\"assembly\", [g])",
    );
    assert!(session.apply_rule_program(changed, false).is_err());
    assert_eq!(session.snapshot().scene_query(), before.scene_query());
    assert_eq!(session.rule_program(), Some(&source));
    assert_eq!(identities(&session.snapshot()), identities(&before));
}
