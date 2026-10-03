use ketchup_application::{DocumentSession, SaveOptions, SessionSettings, rule_program_part_name};
use ketchup_model::document::RuleProgramSource;
use std::collections::BTreeMap;

fn source(target: &str, position: f64) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "motion.star".into(),
        source: format!(
            r#"
p=param("position",0)
a=box("anchor",(10,10,10),at=(-100,0,0))
b=box("b",(10,20,30),at=(110,20,30))
hole(b,"z+",at=(4,5),diameter=2,depth=6)
c=component("child",[b])
i=instance("inner",c,at=(100,0,0),x=(0,1,0))
g=component("parent",[c,i])
j=instance("copy",g,at=(400,0,0))
k=group("moving",[g])
joint(a,{target},kind="motion",motion=slide((0,2,0),0,100),position=p)
"#
        ),
        overrides: BTreeMap::from([("position".into(), position)]),
    }
}

const INTERNAL: &str = r#"
p=param("position",10)
angle=param("angle",90)
lift=param("lift",30)
a=box("a",(5,5,5),at=(-30,0,0))
b=box("b",(20,20,20),at=(20,0,0))
hole(b,"z+",at=(10,10),diameter=4,depth=8)
g=group("local-group",[b])
joint(a,g,kind="motion",motion=slide((1,0,0),0,100),position=p)
c=component("child",[a,g])
i=instance("inner",c,at=(100,0,0),x=(0,1,0))
d=box("d",(5,5,5),at=(200,0,0))
joint(d,i,kind="motion",motion=rotate_motion((0,0,1),0,90,pivot=(100,0,0)),position=angle)
parent=component("parent",[c,i,d])
instance("copy",parent,at=(0,300,0),x=(-1,0,0))
anchor=box("anchor",(5,5,5),at=(-100,-100,0))
joint(anchor,parent,kind="motion",motion=slide((0,0,1),0,100),position=lift)
"#;

#[test]
fn internal_shared_group_part_and_nested_instance_poses_keep_geometry_and_identity() {
    let _turn = crate::integration_support::file_turn();
    for target in ["g", "b"] {
        let mut program = RuleProgramSource {
            file_name: "internal.star".into(),
            source: INTERNAL.replace("joint(a,g,", &format!("joint(a,{target},")),
            overrides: BTreeMap::new(),
        };
        let mut session = DocumentSession::new(SessionSettings::default());
        let first = session
            .apply_rule_program(program.clone(), false)
            .unwrap()
            .snapshot;
        let paths = first
            .scene_query()
            .into_iter()
            .map(|p| p.instance_path)
            .collect::<Vec<_>>();
        for position in [40., 0., 15.] {
            program.overrides.insert("position".into(), position);
            let applied = session.apply_rule_program(program.clone(), false).unwrap();
            assert_eq!(
                applied.snapshot.definitions().cloned().collect::<Vec<_>>(),
                first.definitions().cloned().collect::<Vec<_>>()
            );
            assert_eq!(
                applied
                    .snapshot
                    .scene_query()
                    .into_iter()
                    .map(|p| p.instance_path)
                    .collect::<Vec<_>>(),
                paths
            );
            for leaf in applied.snapshot.scene_query() {
                let Some(name) = rule_program_part_name(&applied.snapshot, &leaf.instance_path)
                else {
                    continue;
                };
                let Some(part) = applied.model.part(&name) else {
                    continue;
                };
                for (actual, expected) in leaf
                    .transform
                    .transform_point([0.; 3])
                    .into_iter()
                    .zip(part.at_mm)
                {
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{name}: {actual} != {expected}"
                    );
                }
                let expected = match name.as_str() {
                    "b" => Some([20. + position, 0., 30.]),
                    "inner/b" => Some([80. - position, 0., 30.]),
                    "copy/b" => Some([-20. - position, 300., 0.]),
                    "copy/inner/b" => Some([position - 80., 300., 0.]),
                    _ => None,
                };
                if let Some(expected) = expected {
                    for (actual, expected) in leaf
                        .transform
                        .transform_point([0.; 3])
                        .into_iter()
                        .zip(expected)
                    {
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{name}: {actual} != {expected}"
                        );
                    }
                }
            }
        }
        let before = session.snapshot();
        program.source = program.source.replace("(20,20,20)", "(25,20,20)");
        let changed = session.apply_rule_program(program.clone(), false).unwrap();
        assert_eq!(
            changed.model.part("copy/inner/b").unwrap().size_mm,
            [25., 20., 20.]
        );
        session.undo().unwrap();
        assert_eq!(session.snapshot().scene_query(), before.scene_query());
        session.redo().unwrap();
        assert_eq!(
            session.snapshot().scene_query(),
            changed.snapshot.scene_query()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("internal.ketchup");
        session
            .save(&path, SaveOptions { overwrite: false })
            .unwrap();
        drop(session);
        let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
        assert_eq!(
            reopened.snapshot().scene_query(),
            changed.snapshot.scene_query()
        );
        program.overrides.insert("angle".into(), 0.);
        program.overrides.insert("lift".into(), 0.);
        let updated = reopened.apply_rule_program(program.clone(), false).unwrap();
        let mut fresh = DocumentSession::new(SessionSettings::default());
        let built = fresh.apply_rule_program(program.clone(), false).unwrap();
        assert_eq!(updated.snapshot.scene_query(), built.snapshot.scene_query());
        program.source = program
            .source
            .lines()
            .filter(|line| !line.starts_with("joint("))
            .collect::<Vec<_>>()
            .join("\n");
        let removed = reopened.apply_rule_program(program.clone(), false).unwrap();
        let mut fresh = DocumentSession::new(SessionSettings::default());
        assert_eq!(
            removed.snapshot.scene_query(),
            fresh
                .apply_rule_program(program, false)
                .unwrap()
                .snapshot
                .scene_query()
        );
    }
}

#[test]
fn invalid_shared_motion_or_driver_cycle_leaves_document_and_source_untouched() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let original = RuleProgramSource {
        file_name: "safe.star".into(),
        source: INTERNAL.into(),
        overrides: BTreeMap::new(),
    };
    let before = session
        .apply_rule_program(original.clone(), false)
        .unwrap()
        .snapshot
        .scene_query();
    for (source,message) in [
        (format!("{INTERNAL}\njoint(anchor,'copy/b',kind='motion',motion=slide((1,0,0),0,100))"),"copied member"),
        (format!("{INTERNAL}\njoint(anchor,b,kind='motion',motion=slide((1,0,0),0,100))"),"before component"),
        ("a=box('a',(5,5,5))\nb=box('b',(5,5,5),at=(20,0,0))\njoint(a,b,kind='motion',motion=slide((1,0,0),0,100))\njoint(b,a,kind='motion',motion=slide((1,0,0),0,100))".into(),"anchor is driven"),
        (format!("{INTERNAL}\njoint(anchor,'missing',kind='motion',motion=slide((1,0,0),0,100))"),"unknown endpoint"),
    ] {
        let invalid=RuleProgramSource { source,..original.clone() };
        let error=session.apply_rule_program(invalid,false).err().expect("invalid motion accepted");
        assert!(error.to_string().contains(message),"{error}");
        assert_eq!(session.snapshot().scene_query(),before);
        assert_eq!(session.rule_program(),Some(&original));
    }
}

#[test]
fn nested_internal_motion_rotates_real_drilled_void_in_each_shared_copy() {
    let _turn = crate::integration_support::file_turn();
    let program = RuleProgramSource {
        file_name: "internal-exact.star".into(),
        source: format!(
            "{INTERNAL}\nbox('void-one',(1,1,1),at=(59.5,-10.5,46))\nbox('material-one',(1,1,1),at=(53.5,-10.5,46))\nbox('void-two',(1,1,1),at=(-60.5,309.5,16))\nbox('material-two',(1,1,1),at=(-54.5,309.5,16))"
        ),
        overrides: BTreeMap::new(),
    };
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session.apply_rule_program(program, false).unwrap();
    let exact = ketchup_application::validation::assistant_validation_context_with_worker(
        &applied.snapshot,
        &ketchup_model::exact_product::ExactResultRegistry::default(),
        &ketchup_application::AssistantValidationSelection::only(&["collision"]),
        &ketchup_model::persistence::ContainerData::default(),
        None,
        std::time::Duration::from_secs(60),
    );
    assert_eq!(exact["complete"], true, "{exact}");
    let issues = exact["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2, "{exact}");
    for probe in ["material-one", "material-two"] {
        assert!(
            issues.iter().any(|i| i["evidence_class"] == "exact"
                && (i["left_name"] == probe || i["right_name"] == probe)),
            "{exact}"
        );
    }
}

#[test]
fn ordinary_nested_group_and_root_part_use_absolute_poses() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let mut program = RuleProgramSource { file_name: "groups.star".into(), source: "p=param('p',10)\na=box('a',(5,5,5))\nb=box('b',(5,5,5),at=(20,0,0))\ng=group('g',[b])\nf=box('f',(5,5,5),at=(100,0,0))\nouter=group('outer',[a,g])\njoint(a,g,kind='motion',motion=slide((1,0,0),0,100),position=p)\njoint(f,outer,kind='motion',motion=rotate_motion((0,0,1),0,90),position=90)\nr=box('root',(5,5,5),at=(200,0,0))\njoint(f,r,kind='motion',motion=slide((0,0,1),0,100),position=p)".into(), overrides: BTreeMap::new() };
    for position in [10., 30., 0.] {
        program.overrides.insert("p".into(), position);
        let applied = session.apply_rule_program(program.clone(), false).unwrap();
        for (name, expected) in [
            ("b", [0., 20. + position, 0.]),
            ("root", [200., 0., position]),
        ] {
            let leaf = applied
                .snapshot
                .scene_query()
                .into_iter()
                .find(|p| p.occurrence_name == name)
                .unwrap();
            for (actual, expected) in leaf
                .transform
                .transform_point([0.; 3])
                .into_iter()
                .zip(expected)
            {
                assert!((actual - expected).abs() < 1e-8);
            }
        }
    }
}

#[test]
fn whole_nested_group_pose_preserves_definitions_paths_and_reference_edits() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let initial = session
        .apply_rule_program(source("k", 40.0), false)
        .unwrap();
    let definitions = initial.snapshot.definitions().cloned().collect::<Vec<_>>();
    let paths = initial
        .snapshot
        .scene_query()
        .into_iter()
        .map(|p| p.instance_path)
        .collect::<Vec<_>>();
    for position in [10.0, 0.0, 120.0, 25.0] {
        let program = source("k", position);
        let applied = session.apply_rule_program(program.clone(), false).unwrap();
        assert_eq!(session.rule_program(), Some(&program));
        assert_eq!(
            applied.snapshot.definitions().cloned().collect::<Vec<_>>(),
            definitions
        );
        assert_eq!(
            applied
                .snapshot
                .scene_query()
                .into_iter()
                .map(|p| p.instance_path)
                .collect::<Vec<_>>(),
            paths
        );
        for leaf in applied.snapshot.scene_query().into_iter().filter(|p| {
            applied
                .snapshot
                .definition(p.definition_id)
                .unwrap()
                .local_occurrence_ids()
                .is_empty()
        }) {
            let name = rule_program_part_name(&applied.snapshot, &leaf.instance_path).unwrap();
            let part = applied.model.part(&name).unwrap();
            assert_eq!(
                leaf.transform.transform_point([0.0; 3]),
                part.at_mm,
                "{name}"
            );
            let expected = match name.as_str() {
                "b" => 20.0 + position,
                "inner/b" => 110.0 + position,
                "copy/b" => 20.0,
                "copy/inner/b" => 110.0,
                _ => 0.0,
            };
            assert_eq!(part.at_mm[1], expected, "{name}");
        }
        assert_eq!(
            applied
                .report
                .issues
                .iter()
                .filter(|i| i.kind == "joint_out_of_range")
                .count(),
            usize::from(position > 100.0)
        );
    }
    let before = session.snapshot();
    let mut edited = source("k", 55.0);
    edited.source = edited.source.replace("(10,20,30)", "(15,20,30)");
    let applied = session.apply_rule_program(edited.clone(), false).unwrap();
    for name in ["b", "inner/b", "copy/b", "copy/inner/b"] {
        assert_eq!(
            applied.model.part(name).unwrap().size_mm,
            [15.0, 20.0, 30.0]
        );
    }
    session.undo().unwrap();
    assert_eq!(session.snapshot().scene_query(), before.scene_query());
    session.redo().unwrap();
    assert_eq!(
        session.snapshot().scene_query(),
        applied.snapshot.scene_query()
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("motion.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened.snapshot().scene_query(),
        applied.snapshot.scene_query()
    );
    edited.overrides.insert("position".into(), 5.0);
    let changed = reopened.apply_rule_program(edited.clone(), false).unwrap();
    let mut fresh = DocumentSession::new(SessionSettings::default());
    let built = fresh.apply_rule_program(edited, false).unwrap();
    assert_eq!(changed.snapshot.scene_query(), built.snapshot.scene_query());
}

#[test]
fn rotating_shared_assembly_moves_real_drilled_void_and_not_its_copy() {
    let _turn = crate::integration_support::file_turn();
    let mut program = RuleProgramSource {
        file_name: "rotation.star".into(),
        source: "angle=param(\"angle\",0)\na=box(\"anchor\",(5,5,5))\nb=box(\"drilled\",(20,20,20),at=(100,20,30))\nhole(b,\"z+\",at=(10,10),diameter=4,depth=8)\nc=component(\"moving\",[b])\ninstance(\"copy\",c,at=(400,0,0))\nbox(\"void-probe\",(1,1,1),at=(89.5,29.5,46))\nbox(\"material-probe\",(1,1,1),at=(83.5,29.5,46))\njoint(a,c,kind=\"motion\",motion=rotate_motion((0,0,1),0,90,pivot=(100,20,0)),position=angle)".into(),
        overrides: BTreeMap::new(),
    };
    let mut session = DocumentSession::new(SessionSettings::default());
    let initial = session.apply_rule_program(program.clone(), false).unwrap();
    program.overrides.insert("angle".into(), 90.0);
    let applied = session.apply_rule_program(program, false).unwrap();
    assert_eq!(
        applied.snapshot.definitions().cloned().collect::<Vec<_>>(),
        initial.snapshot.definitions().cloned().collect::<Vec<_>>()
    );
    let exact = ketchup_application::validation::assistant_validation_context_with_worker(
        &applied.snapshot,
        &ketchup_model::exact_product::ExactResultRegistry::default(),
        &ketchup_application::AssistantValidationSelection::only(&["collision"]),
        &ketchup_model::persistence::ContainerData::default(),
        None,
        std::time::Duration::from_secs(60),
    );
    assert_eq!(exact["complete"], true, "{exact}");
    let issues = exact["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 1, "{exact}");
    assert_eq!(issues[0]["evidence_class"], "exact");
    let names = [
        issues[0]["left_name"].as_str().unwrap(),
        issues[0]["right_name"].as_str().unwrap(),
    ];
    assert!(
        names.contains(&"drilled") && names.contains(&"material-probe"),
        "{exact}"
    );
    assert_eq!(
        applied.model.part("copy/drilled").unwrap().at_mm,
        [500.0, 20.0, 30.0]
    );
}

#[test]
fn component_copy_pose_does_not_change_other_instances_and_can_be_removed() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let mut program = source("j", 40.0);
    let before = session.apply_rule_program(program.clone(), false).unwrap();
    assert_eq!(
        before.model.part("copy/b").unwrap().at_mm,
        [510.0, 60.0, 30.0]
    );
    assert_eq!(before.model.part("b").unwrap().at_mm, [110.0, 20.0, 30.0]);
    program.source = program.source.replace(
        "joint(a,j,kind=\"motion\",motion=slide((0,2,0),0,100),position=p)",
        "",
    );
    let removed = session.apply_rule_program(program, false).unwrap();
    for leaf in removed.snapshot.scene_query().into_iter().filter(|p| {
        removed
            .snapshot
            .definition(p.definition_id)
            .unwrap()
            .local_occurrence_ids()
            .is_empty()
    }) {
        let name = rule_program_part_name(&removed.snapshot, &leaf.instance_path).unwrap();
        assert_eq!(
            leaf.transform.transform_point([0.0; 3]),
            removed.model.part(&name).unwrap().at_mm
        );
    }
}
