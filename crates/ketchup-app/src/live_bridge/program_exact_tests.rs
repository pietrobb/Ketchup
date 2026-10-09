use super::*;

#[test]
fn typed_physical_joint_repair_survives_program_detachment_and_undo() {
    let mut wire = Wire::new();
    let source = "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(210,0,0))\njoint(a,b,kind='fixed',name='physical')";
    assert!(
        wire.call_within(apply(source, true), Duration::from_secs(60))
            .ok
    );
    let b = names(&wire.app)["b"];
    for (distance, state, count) in [(-100., "failed", 1), (-100., "passed", 0)] {
        let response = wire.call_within(
            Request::ApplyAndVerify {
                expected: None,
                selection: None,
                program: AssistantCadEditProgram {
                    operations: vec![AssistantCadEditOperation::Transform {
                        selector: AssistantCadEntitySelector::Occurrences {
                            occurrence_ids: vec![b],
                        },
                        translation_mm: [distance, 0., 0.],
                        rotation: None,
                    }],
                },
                validators: vec!["group_connectivity".into()],
                timeout_ms: 60_000,
                strict: false,
                save: None,
            },
            Duration::from_secs(60),
        );
        assert!(response.ok, "{response:?}");
        let result = response.result.unwrap();
        assert_eq!(result["validation"]["state"], state, "{result:#}");
        assert_eq!(result["validation"]["complete"], true);
        assert_eq!(result["validation"]["issue_count"], count);
        if count == 1 {
            assert_eq!(result["program_detached"], true);
            assert_eq!(
                result["validation"]["issues"][0]["kind"],
                "joint_without_contact"
            );
            assert_eq!(result["validation"]["issues"][0]["distance_mm"], 100.);
        }
    }
    let repaired = wire.app.document.current().scene_query();
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(wire.app.document.current().contact_joints().len(), 1);
    assert_ne!(wire.app.document.current().scene_query(), repaired);
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(wire.app.document.current().scene_query(), repaired);
}

#[test]
fn typed_group_connectivity_survives_detachment_and_root_motion() {
    let mut wire = Wire::new();
    let source = "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(30,0,0))\ncomponent('g',[a,b],grounded=True)";
    assert!(
        wire.call_within(apply(source, true), Duration::from_secs(60))
            .ok
    );
    let b = names(&wire.app)["g"];
    let initial = wire.app.document.current().scene_query();
    for (distance, expected) in [(200., "failed"), (-200., "failed")] {
        let response = wire.call_within(
            Request::ApplyAndVerify {
                expected: None,
                selection: None,
                program: AssistantCadEditProgram {
                    operations: vec![AssistantCadEditOperation::Transform {
                        selector: AssistantCadEntitySelector::Occurrences {
                            occurrence_ids: vec![b],
                        },
                        translation_mm: [distance, 0., 0.],
                        rotation: None,
                    }],
                },
                validators: vec!["group_connectivity".into()],
                timeout_ms: 60_000,
                strict: false,
                save: None,
            },
            Duration::from_secs(60),
        );
        assert!(response.ok, "{response:?}");
        let result = response.result.unwrap();
        assert_eq!(result["validation"]["issue_count"], 1, "{result:#}");
        assert_eq!(result["validation"]["complete"], true, "{result:#}");
        assert_eq!(result["validation"]["state"], expected);
        if distance > 0. {
            assert_eq!(result["program_detached"], true);
            assert_eq!(
                result["validation"]["issues"][0]["kind"],
                "disconnected_group"
            );
        }
    }
    assert_eq!(wire.app.document.current().scene_query(), initial);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_ne!(wire.app.document.current().scene_query(), initial);
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(wire.app.document.current().scene_query(), initial);
}

#[test]
fn program_reports_disconnected_grounded_members_and_override_repairs_contact() {
    let source = "d=param('d',200)\na=box('base',(100,100,10))\nb=box('front',(10,100,100),at=(100+d,0,0))\nk=component('k',[a,b],grounded=True)\ninstance('copy',k,at=(0,300,0))";
    let mut wire = Wire::new();
    let first = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    let detached = |value: &serde_json::Value| {
        value["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"] == "disconnected_group")
            .map(|i| i["parts"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(detached(&first).len(), 2, "{first}");
    assert!(detached(&first).contains(&serde_json::json!(["copy/base", "copy/front"])));
    let before = wire.app.document.current().scene_query();
    let mut request = apply(source, false);
    if let Request::ApplyProgram { overrides, .. } = &mut request {
        overrides.get_or_insert_default().insert("d".into(), 0.);
    }
    let fixed = wire
        .call_within(request, Duration::from_secs(60))
        .result
        .unwrap();
    assert!(detached(&fixed).is_empty(), "{fixed}");
    let after = wire.app.document.current().scene_query();
    assert_ne!(after, before);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(wire.app.document.current().scene_query(), before);
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(wire.app.document.current().scene_query(), after);
}

#[test]
fn grounded_member_override_and_typed_edit_do_not_anchor_its_sibling() {
    let source = "floor(0)\nlocked=param('locked',1)\na=box('anchor',(20,20,10),at=(0,0,100),grounded=locked>0)\nb=box('air',(10,10,10),at=(100,0,100))\nc=component('c',[a,b])\ninstance('copy',c,at=(0,200,0))";
    let mut wire = Wire::new();
    let first = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    let floating = |value: &serde_json::Value| {
        value["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"] == "floating_part")
            .map(|i| i["parts"][0].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        floating(&first),
        BTreeSet::from(["air".into(), "copy/air".into()])
    );
    let before = wire.app.document.current().scene_query();
    let anchors = wire.app.document.current().grounded_instances().clone();
    let mut request = apply(source, false);
    if let Request::ApplyProgram { overrides, .. } = &mut request {
        overrides
            .get_or_insert_default()
            .insert("locked".into(), 0.);
    }
    let changed = wire
        .call_within(request, Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(floating(&changed).len(), 4);
    assert_eq!(wire.app.document.current().scene_query(), before);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(wire.app.document.current().grounded_instances(), &anchors);
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert!(wire.app.document.current().grounded_instances().is_empty());
    assert!(wire.call(Request::Undo { expected: None }).ok);
    let copy = names(&wire.app)["copy"];
    let result = wire
        .call_within(
            Request::ApplyAndVerify {
                expected: None,
                selection: None,
                program: AssistantCadEditProgram {
                    operations: vec![AssistantCadEditOperation::Transform {
                        selector: AssistantCadEntitySelector::Occurrences {
                            occurrence_ids: vec![copy],
                        },
                        translation_mm: [0., 20., 0.],
                        rotation: None,
                    }],
                },
                validators: vec!["gravity_support".into()],
                timeout_ms: 60_000,
                strict: false,
                save: None,
            },
            Duration::from_secs(60),
        )
        .result
        .unwrap();
    assert_eq!(result["program_detached"], true, "{result}");
    assert_eq!(result["validation"]["complete"], true, "{result}");
    assert_eq!(result["validation"]["issue_count"], 2, "{result}");
    assert!(
        result["validation"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["name"].as_str().unwrap().ends_with("air")),
        "{result}"
    );
    assert_eq!(wire.app.document.current().grounded_instances(), &anchors);
}

#[test]
fn floor_override_and_typed_edit_use_the_same_persistent_support_plane() {
    let source = "floor(param('floor-z',100))\nbox('base',(20,20,10),at=(0,0,100))\nbox('top',(20,20,10),at=(0,0,110))\nbox('air',(5,5,5),at=(100,0,130))";
    let mut wire = Wire::new();
    let first = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    let floating = |value: &serde_json::Value| {
        value["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"] == "floating_part")
            .map(|i| i["parts"][0].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(floating(&first), ["air"]);
    let before = wire.app.document.current().scene_query();
    let mut request = apply(source, false);
    if let Request::ApplyProgram { overrides, .. } = &mut request {
        overrides
            .get_or_insert_default()
            .insert("floor-z".into(), 0.);
    }
    let changed = wire
        .call_within(request, Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(floating(&changed), ["base", "top", "air"]);
    assert_eq!(wire.app.document.current().scene_query(), before);
    wire.call(Request::Undo { expected: None });
    assert_eq!(wire.app.document.current().floor_z_mm(), Some(100.));
    wire.call(Request::Redo { expected: None });
    assert_eq!(wire.app.document.current().floor_z_mm(), Some(0.));
    wire.call(Request::Undo { expected: None });
    let air = names(&wire.app)["air"];
    let result = wire
        .call_within(
            Request::ApplyAndVerify {
                expected: None,
                selection: None,
                program: AssistantCadEditProgram {
                    operations: vec![AssistantCadEditOperation::Transform {
                        selector: AssistantCadEntitySelector::Occurrences {
                            occurrence_ids: vec![air],
                        },
                        translation_mm: [10., 0., 0.],
                        rotation: None,
                    }],
                },
                validators: vec!["gravity_support".into()],
                timeout_ms: 60_000,
                strict: false,
                save: None,
            },
            Duration::from_secs(60),
        )
        .result
        .unwrap();
    assert_eq!(result["program_detached"], true, "{result}");
    assert_eq!(
        wire.app.document.current().floor_z_mm(),
        Some(100.),
        "{result}"
    );
    assert_eq!(result["validation"]["complete"], true, "{result}");
    assert_eq!(result["validation"]["issue_count"], 1, "{result}");
    assert_eq!(result["validation"]["issues"][0]["name"], "air", "{result}");
}

#[test]
fn internal_motion_override_updates_shared_paths_and_exact_clearance_in_every_copy() {
    let source = "p=param('open',0)\na=box('anchor',(10,10,10),at=(-100,0,0))\nt=extrude('t',profile=[(0,0),(100,0),(0,100)],distance=20)\ng=group('moving',[t])\njoint(a,g,kind='motion',motion=slide((1,1,0),-100,100),position=p)\nc=component('child',[a,g])\ni=instance('copy',c,at=(300,0,0))\nbox('h',(20,20,20),at=(10,10,0))\nbox('h-copy',(20,20,20),at=(310,10,0))";
    let mut wire = Wire::new();
    let first = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(first["exact_collisions"]["collisions"], 2, "{first}");
    let before = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let mut request = apply(source, false);
    if let Request::ApplyProgram { overrides, .. } = &mut request {
        overrides
            .get_or_insert_default()
            .insert("open".into(), -85.);
    }
    let result = wire
        .call_within(request, Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(result["exact_collisions"]["cleared"], 2, "{result}");
    assert!(!result["report"]["issues"].as_array().unwrap().iter().any(|i| i["kind"]=="collision" || i["kind"]==ketchup_program::COLLISION_UNVERIFIED),"{result}");
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(after["source"], source);
    assert_eq!(after["parts"], before["parts"]);
    assert_eq!(after["overrides"]["open"], -85.);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        before
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        after
    );
}

#[test]
fn motion_override_moves_whole_component_and_preserves_program_and_exact_paths() {
    let source = "p=param(\"open\",0)\na=box(\"anchor\",(10,10,10),at=(-100,0,0))\nt=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\nc=component(\"moving\",[t])\ni=instance(\"copy\",c,at=(300,0,0))\nh=box(\"h\",(20,20,20),at=(10,10,0))\njoint(a,c,kind=\"motion\",motion=slide((1,1,0),0,100),position=p)\n";
    let mut wire = Wire::new();
    let first = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(first["exact_collisions"]["collisions"], 1, "{first}");
    let original = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let mut request = apply(source, false);
    if let Request::ApplyProgram { overrides, .. } = &mut request {
        overrides
            .get_or_insert_default()
            .insert("open".into(), -85.0);
    }
    let changed = wire.call_within(request, Duration::from_secs(60));
    assert!(changed.ok, "{changed:?}");
    let result = changed.result.unwrap();
    assert_eq!(result["exact_collisions"]["cleared"], 1, "{result}");
    assert!(!result["report"]["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "collision" || i["kind"] == ketchup_program::COLLISION_UNVERIFIED), "{result}");
    assert!(
        result["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "joint_out_of_range"),
        "{result}"
    );
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(after["source"], source);
    assert_eq!(after["parts"], original["parts"]);
    assert_eq!(after["overrides"]["open"], -85.0);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        original
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        after
    );
}

#[test]
fn shared_continuation_exposes_new_names_on_the_same_mcp_paths() {
    let source = "a=box(\"top\",(100,20,10))\nc=component(\"child\",[group(\"inside\",[a])])\ni=instance(\"inner\",c,at=(200,0,0))\np=component(\"parent\",[c,i])\ninstance(\"outer\",p,at=(0,300,0),x=(0,1,0))\n";
    let changed = source.replace("a=box(\"top\",(100,20,10))", "a=box(\"left\",(40,20,10))\ncontinue_part(a,was=\"top\")\nb=box(\"right\",(60,20,10),at=(40,0,0))").replace("[a]", "[a,b]");
    let mut wire = Wire::new();
    assert!(
        wire.call_within(apply(source, true), Duration::from_secs(60))
            .ok
    );
    let before = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let result = wire.call_within(apply(&changed, false), Duration::from_secs(60));
    assert!(result.ok, "{result:?}");
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(after["source"], changed);
    assert_eq!(after["parts"].as_array().unwrap().len(), 8);
    for (old_name, new_name) in [
        ("top", "left"),
        ("inner/top", "inner/left"),
        ("outer/top", "outer/left"),
        ("outer/inner/top", "outer/inner/left"),
    ] {
        let old = before["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == old_name)
            .unwrap();
        let new = after["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == new_name)
            .unwrap();
        assert_eq!(new["instance_path"], old["instance_path"]);
        let path = serde_json::from_value(new["instance_path"].clone()).unwrap();
        assert_eq!(
            super::super::program_pick::part_name(&wire.app.document.current(), &path).as_deref(),
            Some(new_name)
        );
    }
    let child = after["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "child")
        .unwrap();
    let members = child["members"].as_array().unwrap();
    assert!(members.iter().any(|p| p["name"] == "left"));
    assert!(!members.iter().any(|p| p["name"] == "top"));
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        before
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        after
    );
}

#[test]
fn nested_component_wire_reports_and_reads_distinguish_all_shared_leaf_paths() {
    let source = "t=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\nc=box(\"clear\",(20,20,20),at=(70,70,0))\nh=box(\"hit\",(20,20,20),at=(10,10,0))\na=component(\"child\",[t,c,h])\ni=instance(\"inner\",a,at=(200,0,0))\np=component(\"parent\",[group(\"pair\",[a,i])])\ninstance(\"outer\",p,at=(800,500,0),x=(0,1,0))\n";
    let mut wire = Wire::new();
    let applied = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(
        applied["exact_collisions"]["state"], "verified",
        "{applied}"
    );
    assert_eq!(applied["exact_collisions"]["collisions"], 4, "{applied}");
    assert_eq!(applied["exact_collisions"]["cleared"], 4, "{applied}");
    let read = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let parts = read["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 12);
    let child = read["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["name"] == "child")
        .unwrap();
    let instances = child["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 4);
    assert_eq!(
        instances
            .iter()
            .map(|instance| instance["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["child", "inner", "outer/child", "outer/inner"])
    );
    for instance in instances {
        assert!(instance["occurrence_id"].is_null());
        assert!(!instance["instance_path"].is_null());
    }
    let snapshot = wire.app.document.current();
    let mut paths = BTreeSet::new();
    for part in parts {
        let path: ketchup_model::document::InstancePath =
            serde_json::from_value(part["instance_path"].clone()).unwrap();
        assert!(paths.insert(path.clone()));
        assert_eq!(
            super::super::program_pick::part_name(&snapshot, &path).as_deref(),
            part["name"].as_str()
        );
        assert_eq!(
            snapshot
                .resolve_instance_path(&path)
                .unwrap()
                .definition_id
                .0,
            part["definition_id"].as_u64().unwrap()
        );
        assert!(part["occurrence_id"].is_null());
    }
    let hit_pairs = applied["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|issue| issue["kind"] == "collision")
        .map(|issue| {
            let mut pair = issue["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| part.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            pair.sort();
            pair
        })
        .collect::<BTreeSet<_>>();
    let expected = ["", "inner/", "outer/", "outer/inner/"]
        .map(|prefix| vec![format!("{prefix}hit"), format!("{prefix}t")])
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(hit_pairs, expected);
    let t = parts
        .iter()
        .filter(|part| {
            part["name"].as_str().unwrap().ends_with('t')
                && !part["name"].as_str().unwrap().ends_with("hit")
        })
        .map(|part| part["definition_id"].as_u64().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(t.len(), 1);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert!(wire.app.document.current().scene_query().is_empty());
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        read
    );
}

#[test]
fn nested_profile_edit_moves_instances_and_updates_exact_reports_without_losing_paths() {
    let source = "t=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\nh=box(\"h\",(10,10,10),at=(70,70,0))\nc=component(\"child\",[t,h])\ni=instance(\"inner\",c,at=(200,0,0))\np=component(\"parent\",[group(\"pair\",[c,i])])\ninstance(\"outer\",p,at=(800,500,0),x=(0,1,0))\n";
    let mut wire = Wire::new();
    let initial = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(initial["exact_collisions"]["cleared"], 4, "{initial}");
    let before = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let changed = source
        .replace("(100,0),(0,100)", "(100,0),(100,100),(0,90)")
        .replace("at=(200,0,0)", "at=(250,20,0),x=(-1,0,0)");
    let updated = wire
        .call_within(apply(&changed, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(
        updated["exact_collisions"]["state"], "verified",
        "{updated}"
    );
    assert_eq!(updated["exact_collisions"]["collisions"], 4, "{updated}");
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(after["source"], changed);
    for old in before["parts"].as_array().unwrap() {
        let new = after["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == old["name"])
            .unwrap();
        assert_eq!(new["instance_path"], old["instance_path"]);
        if old["name"].as_str().unwrap().ends_with('t') {
            assert_ne!(new["definition_id"], old["definition_id"]);
        } else {
            assert_eq!(new["definition_id"], old["definition_id"]);
        }
        let path = serde_json::from_value(new["instance_path"].clone()).unwrap();
        assert_eq!(
            super::super::program_pick::part_name(&wire.app.document.current(), &path).as_deref(),
            new["name"].as_str()
        );
    }
    let pairs = updated["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "collision")
        .map(|i| {
            let mut names = i["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            names.sort();
            names
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        pairs,
        ["", "inner/", "outer/", "outer/inner/"]
            .map(|prefix| vec![format!("{prefix}h"), format!("{prefix}t")])
            .into_iter()
            .collect()
    );
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        before
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap(),
        after
    );
}

#[test]
fn shared_profile_rebuild_updates_every_copy_without_replacing_member_paths() {
    let source = "t=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\nh=box(\"h\",(10,10,10),at=(70,70,0))\nc=component(\"assembly\",[t,h])\ninstance(\"copy\",c,at=(400,500,0),x=(0,1,0))\n";
    let mut wire = Wire::new();
    let initial = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(initial["exact_collisions"]["cleared"], 2, "{initial}");
    let before = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let changed = source.replace("(100,0),(0,100)", "(100,0),(100,100),(0,90)");
    let rebuilt = wire
        .call_within(apply(&changed, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(rebuilt["geometry_evaluated"], true, "{rebuilt}");
    assert_eq!(rebuilt["exact_collisions"]["collisions"], 2, "{rebuilt}");
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let mut expected = before["components"].clone();
    expected[0]["members"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "t")
        .unwrap()["definition_id"] = after["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "t")
        .unwrap()["definition_id"]
        .clone();
    assert_eq!(expected, after["components"]);
    let parts = after["parts"].as_array().unwrap();
    for old in before["parts"].as_array().unwrap() {
        let new = parts.iter().find(|p| p["name"] == old["name"]).unwrap();
        assert_eq!(old["instance_path"], new["instance_path"]);
        if old["name"].as_str().unwrap().ends_with('t') {
            assert_ne!(old["definition_id"], new["definition_id"]);
        } else {
            assert_eq!(old["definition_id"], new["definition_id"]);
        }
        let path = serde_json::from_value(new["instance_path"].clone()).unwrap();
        assert_eq!(
            json!(
                wire.app
                    .document
                    .current()
                    .resolve_instance_path(&path)
                    .unwrap()
                    .definition_id
                    .0
            ),
            new["definition_id"]
        );
    }
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap()["parts"],
        before["parts"]
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap()["parts"],
        after["parts"]
    );
}

#[test]
fn moving_a_shared_member_removes_collisions_in_all_instances_in_one_undo_step() {
    let source = "t=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\nh=box(\"h\",(20,20,20),at=(10,10,0))\nc=component(\"assembly\",[t,h])\ninstance(\"copy\",c,at=(400,500,0),x=(0,1,0))\n";
    let mut wire = Wire::new();
    let before = wire
        .call_within(apply(source, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(before["exact_collisions"]["collisions"], 2, "{before}");
    let original = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let changed = source.replace("at=(10,10,0)", "at=(70,70,0)");
    let moved = wire
        .call_within(apply(&changed, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(moved["exact_collisions"]["state"], "verified", "{moved}");
    assert_eq!(moved["exact_collisions"]["cleared"], 2, "{moved}");
    assert!(
        !moved["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["kind"] == "collision"
                || issue["kind"] == ketchup_program::COLLISION_UNVERIFIED),
        "{moved}"
    );
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(after["parts"], original["parts"]);
    let mut expected = original["components"].clone();
    let h = expected[0]["members"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "h")
        .unwrap();
    h["transform"]["matrix"][3] = json!(70.0);
    h["transform"]["matrix"][7] = json!(70.0);
    assert_eq!(after["components"], expected);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap()["source"],
        source
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap()["source"],
        changed
    );
}

#[test]
fn shared_member_addition_and_removal_update_exact_reports_and_mcp_paths() {
    let source = "t=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\nc=component(\"assembly\",[t])\ninstance(\"copy\",c,at=(400,500,0),x=(0,1,0))\n";
    let mut wire = Wire::new();
    assert!(
        wire.call_within(apply(source, true), Duration::from_secs(60))
            .ok
    );
    let before = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let changed = source
        .replace(
            "c=component",
            "h=box(\"h\",(20,20,20),at=(10,10,0))\nc=component",
        )
        .replace("[t]", "[t,h]");
    let added = wire
        .call_within(apply(&changed, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(added["geometry_evaluated"], true, "{added}");
    assert_eq!(added["exact_collisions"]["state"], "verified", "{added}");
    assert_eq!(added["exact_collisions"]["collisions"], 2, "{added}");
    let read = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    let new_parts = read["parts"].as_array().unwrap();
    assert_eq!(new_parts.len(), 4);
    for part in before["parts"].as_array().unwrap() {
        let same = new_parts
            .iter()
            .find(|p| p["name"] == part["name"])
            .unwrap();
        assert_eq!(same["instance_path"], part["instance_path"]);
        assert_eq!(same["definition_id"], part["definition_id"]);
    }
    for part in new_parts {
        let path: ketchup_model::document::InstancePath =
            serde_json::from_value(part["instance_path"].clone()).unwrap();
        let resolved = wire
            .app
            .document
            .current()
            .resolve_instance_path(&path)
            .unwrap();
        assert_eq!(json!(resolved.definition_id.0), part["definition_id"]);
    }
    let removed = wire
        .call_within(apply(source, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert!(
        !removed["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["kind"] == "collision"),
        "{removed}"
    );
    let after = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(after["parts"], before["parts"]);
    assert_eq!(after["components"], before["components"]);
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap()["parts"],
        read["parts"]
    );
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(
        wire.call(Request::Program { expected: None })
            .result
            .unwrap()["source"],
        source
    );
}

#[test]
fn live_component_exact_reports_distinguish_internal_and_external_pairs() {
    let source = r#"
t = extrude("t", profile=[(0,0),(100,0),(0,100)], distance=20)
c = box("clear", (20,20,20), at=(70,70,0))
h = box("hit", (20,20,20), at=(10,10,0))
a = component("assembly", [group("inner", [t,c]), h])
i = instance("copy", a, at=(400,500,0), x=(0,1,0))
p = box("probe", (10,10,20), at=(380,550,0))
group("all", [a,i,p])
"#;
    let mut wire = Wire::new();
    let response = wire.call_within(apply(source, true), Duration::from_secs(60));
    assert!(response.ok, "{response:?}");
    let result = response.result.unwrap();
    assert_eq!(result["geometry_evaluated"], true, "{result}");
    assert_eq!(result["exact_collisions"]["state"], "verified", "{result}");
    assert_eq!(result["exact_collisions"]["cleared"], 2, "{result}");
    let issues = result["report"]["issues"].as_array().unwrap();
    assert!(
        !issues
            .iter()
            .any(|issue| issue["kind"] == ketchup_program::COLLISION_UNVERIFIED),
        "{result}"
    );
    let pairs = issues
        .iter()
        .filter(|issue| issue["kind"] == "collision")
        .map(|issue| {
            let mut names = issue["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap())
                .collect::<Vec<_>>();
            names.sort_unstable();
            names
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        pairs,
        BTreeSet::from([
            vec!["hit", "t"],
            vec!["copy/hit", "copy/t"],
            vec!["copy/t", "probe"],
        ]),
        "{result}"
    );
    let read = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(read["source"], source);
    assert_eq!(read["components"].as_array().unwrap().len(), 1);
    let moved_source = source.replace("at=(400,500,0)", "at=(700,500,0)");
    let moved = wire
        .call_within(apply(&moved_source, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(moved["exact_collisions"]["state"], "verified", "{moved}");
    let collisions = moved["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|issue| issue["kind"] == "collision")
        .collect::<Vec<_>>();
    assert_eq!(collisions.len(), 2, "{moved}");
    assert!(
        collisions
            .iter()
            .all(|issue| !issue["parts"].as_array().unwrap().contains(&json!("probe")))
    );
    assert!(wire.call(Request::Undo { expected: None }).ok);
    let restored = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(restored["source"], source);
    assert_eq!(restored["components"], read["components"]);
    assert!(wire.call(Request::Redo { expected: None }).ok);
    let redone = wire
        .call(Request::Program { expected: None })
        .result
        .unwrap();
    assert_eq!(redone["source"], moved_source);
}
