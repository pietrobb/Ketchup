use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use std::collections::BTreeMap;

fn document_floating(snapshot: &ketchup_model::document::Snapshot) -> Vec<String> {
    let report = ketchup_application::validation::assistant_validation_context_with_worker(
        snapshot,
        &ketchup_model::exact_product::ExactResultRegistry::default(),
        &ketchup_application::AssistantValidationSelection::only(&["gravity_support"]),
        &ketchup_model::persistence::ContainerData::default(),
        None,
        std::time::Duration::from_secs(60),
    );
    assert_eq!(report["gravity_support"]["complete"], true, "{report:#}");
    let mut names = report["gravity_support"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn floor_is_persistent_after_detachment_and_removal_resets_to_default() {
    use ketchup_model::document::{CanonicalCommand, CommandBatch};
    let _turn = crate::integration_support::file_turn();
    let mut source = RuleProgramSource {
        file_name: "floor.star".into(),
        source:
            "floor(-35)\nbox('base',(20,20,10),at=(0,0,-35))\nbox('air',(10,10,10),at=(50,0,0))"
                .into(),
        overrides: BTreeMap::new(),
    };
    let mut session = DocumentSession::default();
    let initial = session.apply_rule_program(source.clone(), false).unwrap();
    assert_eq!(document_floating(&initial.snapshot), ["air"]);
    source.source = source.source.replace("floor(-35)\n", "");
    let reset = session.apply_rule_program(source, false).unwrap();
    assert_eq!(reset.snapshot.floor_z_mm(), None);
    assert_eq!(document_floating(&reset.snapshot), ["base"]);
    assert_eq!(reset.snapshot.scene_query(), initial.snapshot.scene_query());
    session.undo().unwrap();
    let id = initial
        .snapshot
        .occurrences()
        .find(|o| o.name() == "air")
        .unwrap()
        .id();
    let batch = CommandBatch::new(vec![CanonicalCommand::RenameEntity {
        id,
        name: "renamed".into(),
    }]);
    let proposal = session.plan_commands(batch).unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert!(session.rule_program().is_none());
    assert_eq!(session.snapshot().floor_z_mm(), Some(-35.));
    assert_eq!(document_floating(&session.snapshot()), ["renamed"]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("detached.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert!(reopened.rule_program().is_none());
    assert_eq!(reopened.snapshot().floor_z_mm(), Some(-35.));
    assert_eq!(document_floating(&reopened.snapshot()), ["renamed"]);
}

#[test]
fn nested_grounding_changes_support_without_geometry_and_survives_reopen() {
    use ketchup_application::rule_program_part_name;
    let _turn = crate::integration_support::file_turn();
    let mut source = RuleProgramSource {
        file_name: "ground.star".into(),
        source: "floor(0)\nlocked=param('locked',1)\na=box('anchor',(20,20,10),at=(0,0,100),grounded=locked>0)\nb=box('touching',(20,20,10),at=(0,0,110))\nc=box('air',(5,5,5),at=(100,0,100))\ng=group('g',[a,b,c])\nk=component('k',[g])\ni=instance('inner',k,at=(200,0,0))\no=component('outer',[k,i])\ninstance('copy',o,at=(0,300,0))".into(),
        overrides: BTreeMap::new(),
    };
    let mut session = DocumentSession::default();
    let original = session.apply_rule_program(source.clone(), false).unwrap();
    let anchors = original.snapshot.grounded_instances();
    assert_eq!(anchors.len(), 4);
    for leaf in original.snapshot.scene_query() {
        let name = rule_program_part_name(&original.snapshot, &leaf.instance_path).unwrap();
        assert_eq!(
            original.snapshot.instance_is_grounded(&leaf.instance_path),
            name.ends_with("anchor"),
            "{name}"
        );
    }
    let floating = document_floating(&original.snapshot);
    assert_eq!(floating.len(), 4, "{floating:?}");
    assert!(
        floating.iter().all(|name| name.ends_with("air")),
        "{floating:?}"
    );
    source.overrides.insert("locked".into(), 0.);
    let released = session.apply_rule_program(source.clone(), false).unwrap();
    assert!(released.snapshot.grounded_instances().is_empty());
    assert_eq!(
        released.snapshot.scene_query(),
        original.snapshot.scene_query()
    );
    let released_names = document_floating(&released.snapshot);
    assert_eq!(released_names.len(), 12);
    for name in ["anchor", "touching", "air"] {
        assert_eq!(
            released_names
                .iter()
                .filter(|item| item.as_str() == name)
                .count(),
            4,
            "{released_names:?}"
        );
    }
    session.undo().unwrap();
    assert_eq!(session.snapshot().grounded_instances(), anchors);
    assert_eq!(document_floating(&session.snapshot()), floating);
    session.redo().unwrap();
    assert!(session.snapshot().grounded_instances().is_empty());
    source.overrides.insert("locked".into(), 1.);
    session.apply_rule_program(source.clone(), false).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ground.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().grounded_instances(), anchors);
    assert_eq!(document_floating(&reopened.snapshot()), floating);
    assert_eq!(
        reopened.snapshot().scene_query(),
        original.snapshot.scene_query()
    );
}

#[test]
fn group_anchors_survive_detachment_and_deleted_leaves_leave_no_stale_anchor() {
    use ketchup_model::document::{CanonicalCommand, CommandBatch, InstancePath, OccurrenceId};
    let _turn = crate::integration_support::file_turn();
    let mut source = RuleProgramSource {
        file_name:"ground.star".into(), overrides:BTreeMap::new(),
        source:"floor(0)\na=box('a',(10,10,10),at=(0,0,100))\nb=box('b',(10,10,10),at=(100,0,100))\ngroup('g',[a,b],grounded=True)".into(),
    };
    let mut session = DocumentSession::default();
    let first = session.apply_rule_program(source.clone(), false).unwrap();
    assert!(document_floating(&first.snapshot).is_empty());
    assert_eq!(first.snapshot.grounded_instances().len(), 2);
    source.source = source.source.replace("grounded=True", "grounded=False");
    let released = session.apply_rule_program(source, false).unwrap();
    assert_eq!(document_floating(&released.snapshot), ["a", "b"]);
    assert_eq!(
        first.snapshot.scene_query(),
        released.snapshot.scene_query()
    );
    session.undo().unwrap();
    let a = first
        .snapshot
        .occurrences()
        .find(|item| item.name() == "a")
        .unwrap()
        .id();
    let proposal = session
        .plan_commands(CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: a,
            name: "renamed".into(),
        }]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert!(session.rule_program().is_none());
    assert!(document_floating(&session.snapshot()).is_empty());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("detached-ground.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let mut session = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert!(session.rule_program().is_none());
    assert!(document_floating(&session.snapshot()).is_empty());
    assert!(
        session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::SetGroundedInstances {
                    paths: [InstancePath::root(OccurrenceId(u64::MAX))].into()
                }
            ]))
            .is_err()
    );
    assert_eq!(session.snapshot().grounded_instances().len(), 2);
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence { id: a },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert_eq!(session.snapshot().grounded_instances().len(), 1);
    assert!(document_floating(&session.snapshot()).is_empty());
    session.undo().unwrap();
    assert_eq!(session.snapshot().grounded_instances().len(), 2);
}

#[test]
fn invalid_floor_commands_leave_the_document_unchanged() {
    use ketchup_model::document::{CanonicalCommand, CommandBatch, DocumentStore};
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetFloorHeight {
            z_mm: Some(-35.),
        }]))
        .unwrap();
    for z in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e100] {
        assert!(
            document
                .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetFloorHeight {
                    z_mm: Some(z)
                }]))
                .is_err()
        );
        assert_eq!(document.current().floor_z_mm(), Some(-35.));
    }
}

#[test]
fn floor_override_changes_support_not_geometry_and_survives_undo_and_reopen() {
    let _turn = crate::integration_support::file_turn();
    let mut program = RuleProgramSource {
        file_name: "support.star".into(),
        source: "floor(param('floor-z',100))\nbox('base',(20,20,10),at=(0,0,100))\nbox('top',(20,20,10),at=(0,0,110))\nbox('detached',(5,5,5),at=(100,0,130))".into(),
        overrides: BTreeMap::new(),
    };
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = session.apply_rule_program(program.clone(), false).unwrap();
    let floating = |report: &ketchup_program::Report| {
        report
            .issues
            .iter()
            .filter(|i| i.kind == "floating_part")
            .flat_map(|i| i.parts.iter().cloned())
            .collect::<Vec<_>>()
    };
    assert_eq!(floating(&original.report), ["detached"]);
    assert_eq!(original.snapshot.floor_z_mm(), Some(100.));
    assert_eq!(document_floating(&original.snapshot), ["detached"]);
    let old_source = program.clone();
    program.overrides.insert("floor-z".into(), 0.);
    let changed = session.apply_rule_program(program.clone(), false).unwrap();
    assert_eq!(floating(&changed.report), ["base", "top", "detached"]);
    assert_eq!(changed.snapshot.floor_z_mm(), Some(0.));
    assert_eq!(
        document_floating(&changed.snapshot),
        ["base", "detached", "top"]
    );
    assert_eq!(
        changed.snapshot.scene_query(),
        original.snapshot.scene_query()
    );
    session.undo().unwrap();
    assert_eq!(session.rule_program(), Some(&old_source));
    assert_eq!(session.snapshot().floor_z_mm(), Some(100.));
    assert_eq!(document_floating(&session.snapshot()), ["detached"]);
    session.redo().unwrap();
    assert_eq!(session.rule_program(), Some(&program));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("support.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.rule_program(), Some(&program));
    assert_eq!(reopened.snapshot().floor_z_mm(), Some(0.));
    assert_eq!(
        document_floating(&reopened.snapshot()),
        ["base", "detached", "top"]
    );
    assert_eq!(
        reopened.snapshot().scene_query(),
        original.snapshot.scene_query()
    );
    program.overrides.insert("floor-z".into(), 100.);
    let restored = reopened.apply_rule_program(program, false).unwrap();
    assert_eq!(floating(&restored.report), ["detached"]);
    assert_eq!(
        restored.snapshot.scene_query(),
        original.snapshot.scene_query()
    );
}
