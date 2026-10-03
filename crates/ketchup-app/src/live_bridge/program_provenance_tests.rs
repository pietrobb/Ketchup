use super::*;

const PARTS: &str = "p=box('shared',(18,120,120))\na=box('left',(80,120,18),at=(-80,0,40))\nb=box('right',(80,120,18),at=(18,0,40))\nhole(p,'z+',at=(9,60),diameter=4,depth=5,id='service')\n";
const LEFT: &str = "dowels(p,a,count=2,margin=20)\n";
const RIGHT: &str = "dowels(p,b,count=2,margin=35)\n";

/// Inspect actual evaluated walls, not merely the re-evaluated source's BOM.
fn holes(app: &mut KetchupApp, bridge: &mut LiveBridge, part: &str) -> BTreeMap<String, Value> {
    let faces = picks(app, bridge, part, TopologicalElementKind::Face);
    let mut result = BTreeMap::new();
    for face in faces {
        assert_eq!(face["representation"], "solid_part");
        for hole in face["holes"].as_array().unwrap() {
            if hole["surface"] != "wall" {
                continue;
            }
            let id = hole["id"].as_str().unwrap().to_owned();
            assert!(
                result.insert(id.clone(), hole.clone()).is_none(),
                "duplicate bore {id}"
            );
        }
    }
    result
}

fn state(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
) -> BTreeMap<String, BTreeMap<String, Value>> {
    evaluate_exact(app);
    ["shared", "left", "right"]
        .into_iter()
        .map(|part| (part.to_owned(), holes(app, bridge, part)))
        .collect()
}

#[test]
fn neighboring_join_change_delete_undo_redo_and_save_open_preserve_only_owned_holes() {
    adjacent_joins(false);
}

#[test]
fn reverse_order_adjacent_join_patch_preserves_neighbor_and_reopens() {
    adjacent_joins(true);
}

fn adjacent_joins(reverse: bool) {
    let (mut app, mut bridge) = setup();
    let source = if reverse {
        format!("{PARTS}{RIGHT}{LEFT}")
    } else {
        format!("{PARTS}{LEFT}{RIGHT}")
    };
    bridge
        .execute(&mut app, apply(&source, true), false)
        .unwrap();
    let identities = names(&app);
    let before = state(&mut app, &mut bridge);
    assert_eq!(before["shared"].len(), 5);
    assert_eq!(before["left"].len(), 2);
    assert_eq!(before["right"].len(), 2);
    let picked = &before["shared"]["dowel:left:1"];
    assert_eq!(picked["representation"], "machining");
    assert_eq!(picked["entry_face"], "x-");
    assert_eq!(picked["inward_local"], json!([1., 0., 0.]));
    assert_eq!(picked["depth_mm"], 12.);
    let joint = &picked["joints"][0];
    assert_eq!(joint["name"], "dowel:shared+left");
    assert_eq!(joint["association"], "explicit_operation_link");
    assert_eq!(
        joint["operations"],
        json!([{ "part": "shared", "id": "dowel:left:1" }, { "part": "left", "id": "dowel:shared:1" }])
    );
    assert_eq!(joint["counterpart_part"], "left");
    assert_eq!(joint["counterpart_holes"][0]["id"], "dowel:shared:1");
    assert_eq!(
        joint["counterpart_holes"][0]["entry_world_mm"],
        picked["entry_world_mm"]
    );
    assert_eq!(joint["hardware"]["representation"], "metadata_only");
    assert!(joint["hardware"]["solid_part"].is_null());
    assert!(
        before["shared"]["service"]["joints"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let replacement = "dowels(p,a,count=1,margin=20)\n";
    let changed = source.replace(LEFT, replacement);
    let undo_steps = app.undo_step_count();
    let request = Request::PatchProgram {
        expected: app.live_bridge_stamp(),
        edits: vec![SourceEdit {
            old: LEFT.into(),
            new: replacement.into(),
        }],
    };
    bridge.execute(&mut app, request, false).unwrap();
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.document.current_rule_program().unwrap().source, changed);
    let after_change = state(&mut app, &mut bridge);
    assert_eq!(after_change["shared"].len(), 4);
    assert_eq!(after_change["left"].len(), 1);
    assert_ne!(
        after_change["shared"]["dowel:left:1"]["entry_world_mm"],
        picked["entry_world_mm"]
    );
    assert_eq!(after_change["right"], before["right"]);
    for id in ["dowel:right:1", "dowel:right:2", "service"] {
        assert_eq!(after_change["shared"][id], before["shared"][id]);
    }
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(state(&mut app, &mut bridge), before);
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(state(&mut app, &mut bridge), after_change);

    let deleted = format!("{PARTS}{RIGHT}");
    let request = Request::PatchProgram {
        expected: app.live_bridge_stamp(),
        edits: vec![SourceEdit {
            old: replacement.into(),
            new: String::new(),
        }],
    };
    bridge.execute(&mut app, request, false).unwrap();
    let after_delete = state(&mut app, &mut bridge);
    assert_eq!(after_delete["shared"].len(), 3);
    assert!(after_delete["left"].is_empty());
    assert_eq!(after_delete["right"], before["right"]);
    for id in ["dowel:right:1", "dowel:right:2", "service"] {
        assert_eq!(after_delete["shared"][id], before["shared"][id]);
    }
    assert_eq!(names(&app), identities);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(state(&mut app, &mut bridge), after_change);
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(state(&mut app, &mut bridge), after_delete);
    // Reapplying must not append another set of operations.
    bridge
        .execute(&mut app, apply(&deleted, false), false)
        .unwrap();
    assert_eq!(state(&mut app, &mut bridge), after_delete);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("joins.ketchup");
    bridge
        .execute(
            &mut app,
            Request::SaveAs {
                expected: None,
                path: path.to_string_lossy().into_owned(),
            },
            false,
        )
        .unwrap();
    drop(bridge);
    drop(app);
    let (mut opened, mut bridge) = setup();
    bridge
        .execute(
            &mut opened,
            Request::Open {
                expected: None,
                path: path.to_string_lossy().into_owned(),
            },
            false,
        )
        .unwrap();
    assert_eq!(
        opened.document.current_rule_program().unwrap().source,
        deleted
    );
    assert_eq!(state(&mut opened, &mut bridge), after_delete);
    assert_eq!(names(&opened), identities);
    let undo_steps = opened.undo_step_count();
    let request = Request::PatchProgram {
        expected: opened.live_bridge_stamp(),
        edits: vec![SourceEdit {
            old: RIGHT.into(),
            new: String::new(),
        }],
    };
    bridge.execute(&mut opened, request, false).unwrap();
    assert_eq!(opened.undo_step_count(), undo_steps + 1);
    let bare = state(&mut opened, &mut bridge);
    assert_eq!(bare["shared"].len(), 1);
    assert_eq!(bare["shared"]["service"], before["shared"]["service"]);
    assert!(bare["left"].is_empty() && bare["right"].is_empty());
    bridge
        .execute(&mut opened, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(state(&mut opened, &mut bridge), after_delete);
    bridge
        .execute(&mut opened, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(state(&mut opened, &mut bridge), bare);
}

#[test]
fn generic_rotated_joint_picks_do_not_invent_solid_hardware_links() {
    let (mut app, mut bridge) = setup();
    let source = "a=box('a',(100,100,20))\nb=box('b',(100,100,20),at=(0,0,20))\nha=hole(a,'z+',at=(40,50),diameter=8,depth=8,id='mount')\nhb=hole(b,'z-',at=(40,50),diameter=8,depth=8,id='mate')\nhole(a,'z+',at=(70,50),diameter=4,depth=5,id='unowned')\njoint(a,b,kind='metadata',fastener='pin',fasteners=[(70,50,20)])\npin=box('pin solid',(2,2,2),at=(400,0,0))\njoint(a,b,kind='mount',name='named mount',fastener='pin',fasteners=[(40,50,20)],links=[joint_link([ha,hb],hardware=[pin])])\nc=component('pair',[a,b,pin])\ninstance('turned',c,at=(200,0,0),x=(0,0,-1),z=(1,0,0))";
    bridge
        .execute(&mut app, apply(source, true), false)
        .unwrap();
    evaluate_exact(&mut app);
    let original = holes(&mut app, &mut bridge, "a");
    assert_eq!(original["mount"]["joints"][0]["name"], "named mount");
    let copied = holes(&mut app, &mut bridge, "turned/a");
    assert_eq!(copied["mount"]["entry_world_mm"], json!([220., 50., -40.]));
    assert_eq!(copied["mount"]["inward_world"], json!([-1., 0., 0.]));
    assert_eq!(copied["mount"]["joints"][0]["counterpart_part"], "turned/b");
    assert_eq!(
        copied["mount"]["joints"][0]["hardware"]["solid_part"],
        "turned/pin solid"
    );
    assert_eq!(
        copied["mount"]["joints"][0]["hardware"]["representation"],
        "linked_solid_parts"
    );
    assert_eq!(copied["mount"]["ownership"], "explicit");
    assert_eq!(copied["unowned"]["ownership"], "unowned");
    assert!(copied["unowned"]["joints"].as_array().unwrap().is_empty());
    assert!(holes(&mut app, &mut bridge, "pin solid").is_empty());
    let edges = picks(&mut app, &mut bridge, "a", TopologicalElementKind::Edge);
    assert!(
        edges
            .iter()
            .any(|edge| edge["faces"].as_array().unwrap().iter().any(|face| {
                face["holes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|hole| hole["id"] == "mount")
            }))
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("linked-hardware.ketchup");
    bridge
        .execute(
            &mut app,
            Request::SaveAs {
                expected: None,
                path: path.to_string_lossy().into_owned(),
            },
            false,
        )
        .unwrap();
    drop(bridge);
    drop(app);
    let (mut reopened, mut bridge) = setup();
    bridge
        .execute(
            &mut reopened,
            Request::Open {
                expected: None,
                path: path.to_string_lossy().into_owned(),
            },
            false,
        )
        .unwrap();
    evaluate_exact(&mut reopened);
    assert_eq!(holes(&mut reopened, &mut bridge, "a"), original);
    assert_eq!(holes(&mut reopened, &mut bridge, "turned/a"), copied);
}

#[test]
fn generated_hinge_cups_and_plate_bores_explain_their_shared_connector() {
    let (mut app, mut bridge) = setup();
    let source = "s=board('side',(18,350,600))\nd=board('door',(560,18,560),at=(20,0,20))\nhinge(d,s,count=2,margin=100)\n";
    bridge
        .execute(&mut app, apply(source, true), false)
        .unwrap();
    evaluate_exact(&mut app);
    let cups = holes(&mut app, &mut bridge, "door");
    let plates = holes(&mut app, &mut bridge, "side");
    assert_eq!((cups.len(), plates.len()), (2, 4));
    for cup in cups.values() {
        assert_eq!(cup["diameter_mm"], 35.0);
        assert_eq!(cup["depth_mm"], 13.0);
        assert_eq!(cup["ownership"], "explicit");
        let joint = &cup["joints"][0];
        assert_eq!(joint["kind"], "hinge");
        assert_eq!(joint["counterpart_part"], "side");
        assert_eq!(joint["hardware"]["representation"], "metadata_only");
        let counterparts = joint["counterpart_holes"].as_array().unwrap();
        assert_eq!(counterparts.len(), 2);
        for counterpart in counterparts {
            assert_eq!(counterpart["part"], "side");
            assert_eq!(counterpart["diameter_mm"], 5.0);
            assert_eq!(counterpart["depth_mm"], 12.0);
            assert_eq!(
                plates[counterpart["id"].as_str().unwrap()]["joints"][0]["link_index"],
                joint["link_index"]
            );
        }
    }
}

#[test]
fn a_boolean_cut_surface_reports_unidentified_machining_not_an_invented_owner() {
    let (mut app, mut bridge) = setup();
    let source = "p=box('part',(100,100,20))\nt=box('tool',(20,20,12),at=(40,40,8),tool=True)\nsubtract(p,t)\n";
    bridge
        .execute(&mut app, apply(source, true), false)
        .unwrap();
    evaluate_exact(&mut app);
    let faces = picks(&mut app, &mut bridge, "part", TopologicalElementKind::Face);
    let floor = faces
        .iter()
        .find(|face| {
            face["point_local_mm"][2]
                .as_f64()
                .is_some_and(|z| (z - 8.0).abs() < 0.001)
                && face["normal_local"][2] == 1.0
        })
        .expect("the cut has a real floor at z=8");
    assert!(floor["holes"].as_array().unwrap().is_empty());
    assert_eq!(floor["machining_provenance"]["state"], "not_identified");
    assert!(
        floor["machining_provenance"]["reason"]
            .as_str()
            .unwrap()
            .contains("No ownership is inferred")
    );
}
