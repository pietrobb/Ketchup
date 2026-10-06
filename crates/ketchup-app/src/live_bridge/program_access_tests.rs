use super::*;

fn apply_source(source: &str) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.into(),
        overrides: BTreeMap::from([("width".into(), 25.)]),
        file_name: Some("local.star".into()),
        replace_document: true,
    }
}

fn patch(stamp: Stamp, old: &str, new: &str) -> Request {
    Request::PatchProgram {
        expected: stamp,
        edits: vec![SourceEdit {
            old: old.into(),
            new: new.into(),
        }],
    }
}

#[test]
fn local_patch_retains_source_overrides_geometry_and_one_undo() {
    let (mut app, mut bridge) = setup();
    let source =
        "W=param(\"width\", 10)\na=box(\"a\", (W,20,30))\nb=box(\"b\", (10,20,30), at=(100,0,0))";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    let before_geometry = app.document.current();
    let read = bridge
        .execute(
            &mut app,
            Request::ProgramContext {
                expected: None,
                selection_context: false,
            },
            false,
        )
        .unwrap();
    assert_eq!(read["source"], source);
    assert!(read.get("parts").is_none());
    let result = bridge
        .execute(
            &mut app,
            patch(stamp.clone(), "(W,20,30)", "(W,20,40)"),
            false,
        )
        .unwrap();
    assert_eq!(result["source_diff"]["removed_lines"], 1);
    assert_eq!(result["source_diff"]["added_lines"], 1);
    assert_eq!(app.undo_step_count(), undo + 1);
    assert_ne!(
        app.document.current().features().collect::<Vec<_>>(),
        before_geometry.features().collect::<Vec<_>>()
    );
    let current = app.document.current_rule_program().unwrap();
    assert_eq!(current.file_name, "local.star");
    assert_eq!(current.overrides["width"], 25.);
    assert_eq!(current.source, source.replace("(W,20,30)", "(W,20,40)"));
    let changed = app.document.current().scene_query();
    assert!(
        bridge
            .execute(&mut app, patch(stamp, "(W,20,40)", "(W,20,50)"), false)
            .is_err()
    );
    assert_eq!(app.document.current().scene_query(), changed);
    assert_eq!(app.undo_step_count(), undo + 1);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current().scene_query(), before);
    assert_eq!(app.document.current_rule_program().unwrap().source, source);
}

#[test]
fn grouped_root_parts_can_be_selected_read_and_cleared_without_an_edit() {
    let (mut app, mut bridge) = setup();
    let source = "parts=[]\nfor i in range(4):\n    p=box('cap '+str(i),(20,30,18),at=(i*100,0,0))\n    parts.append(group('module '+str(i),[p]))\ngroup('assembly',parts)\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let ids: Vec<_> = before
        .iter()
        .map(|part| part.instance_path.root_occurrence().0)
        .collect();
    assert_eq!(ids.len(), 4);
    bridge
        .execute(
            &mut app,
            Request::Selection {
                expected: None,
                occurrence_ids: ids,
            },
            false,
        )
        .unwrap();
    let context = bridge
        .execute(
            &mut app,
            Request::ProgramContext {
                expected: None,
                selection_context: true,
            },
            false,
        )
        .unwrap();
    assert_eq!(context["parts"].as_array().unwrap().len(), 4);
    bridge
        .execute(
            &mut app,
            Request::Selection {
                expected: None,
                occurrence_ids: Vec::new(),
            },
            false,
        )
        .unwrap();
    assert!(app.selected_occurrence_ids().is_empty());
    assert_eq!(app.document.current().scene_query(), before);
    assert_eq!(app.undo_step_count(), undo);
}

#[test]
fn selection_context_returns_selected_source_and_no_whole_program() {
    let (mut app, mut bridge) = setup();
    let source = "a=box(\"a\", (10,20,30))\n# separation\n# separation\n# separation\n# separation\nb=box(\"b\", (10,20,30), at=(100,0,0))";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let snapshot = app.document.current();
    let id = snapshot
        .occurrences()
        .find(|part| part.name() == "b")
        .unwrap()
        .id()
        .0;
    bridge
        .execute(
            &mut app,
            Request::Selection {
                expected: None,
                occurrence_ids: vec![id],
            },
            false,
        )
        .unwrap();
    let undo = app.undo_step_count();
    let context = bridge
        .execute(
            &mut app,
            Request::ProgramContext {
                expected: None,
                selection_context: true,
            },
            false,
        )
        .unwrap();
    assert!(context.get("source").is_none());
    assert_eq!(context["parts"].as_array().unwrap().len(), 1);
    assert_eq!(context["parts"][0]["name"], "b");
    let lines = context["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line["text"].as_str().unwrap().starts_with("b=box"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line["text"].as_str().unwrap().starts_with("a=box"))
    );
    assert_eq!(app.undo_step_count(), undo);
}

#[test]
fn ambiguous_missing_overlapping_or_invalid_patch_publishes_nothing() {
    let (mut app, mut bridge) = setup();
    let source = "a=box(\"a\", (10,20,30))\nb=box(\"b\", (10,20,30), at=(100,0,0))\n# aaa";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    for (old, new) in [
        ("box", "other"),
        ("absent", "value"),
        ("", "value"),
        ("aa", "b"),
        ("a=box", "a=invalid("),
    ] {
        assert!(
            bridge
                .execute(&mut app, patch(stamp.clone(), old, new), false)
                .is_err()
        );
        assert_eq!(app.document.current().scene_query(), before);
        assert_eq!(app.document.current_rule_program().unwrap().source, source);
        assert_eq!(app.undo_step_count(), undo);
    }
    let partial = Request::PatchProgram {
        expected: stamp.clone(),
        edits: vec![
            SourceEdit {
                old: "a=box".into(),
                new: "c=box".into(),
            },
            SourceEdit {
                old: "missing".into(),
                new: "value".into(),
            },
        ],
    };
    assert!(bridge.execute(&mut app, partial, false).is_err());
    assert_eq!(app.document.current_rule_program().unwrap().source, source);
    let overlapping = Request::PatchProgram {
        expected: stamp,
        edits: vec![
            SourceEdit {
                old: "a=box".into(),
                new: "c=box".into(),
            },
            SourceEdit {
                old: "a=box(\"a\"".into(),
                new: "d=box(\"d\"".into(),
            },
        ],
    };
    assert!(bridge.execute(&mut app, overlapping, false).is_err());
    assert_eq!(app.document.current().scene_query(), before);
    assert_eq!(app.document.current_rule_program().unwrap().source, source);
    assert_eq!(app.undo_step_count(), undo);
}

fn all_rows(app: &mut KetchupApp, bridge: &mut LiveBridge, section: ReportSection) -> Vec<Value> {
    let expected = app.live_bridge_stamp();
    let mut offset = 0;
    let mut rows = Vec::new();
    loop {
        let page = bridge
            .execute(
                app,
                Request::ProgramReport {
                    expected: expected.clone(),
                    section,
                    offset,
                    limit: 3,
                },
                false,
            )
            .unwrap();
        rows.extend(page["rows"].as_array().unwrap().iter().cloned());
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next as usize > offset);
                offset = next as usize;
            }
            None => {
                assert_eq!(rows.len(), page["total"].as_u64().unwrap() as usize);
                break;
            }
        }
    }
    rows
}

#[test]
fn the_joints_report_lists_every_bearing_joint_with_its_rating_or_what_it_lacks() {
    let (mut app, mut bridge) = setup();
    let source = "load_path(only=[\"f\"])\nself_weight([\"f\"])\nh=box(\"h\", (120,2000,240), grounded=True, material=\"C24\", tags=[\"f\"])\nj=box(\"j\", (3000,60,200), at=(120,500,40), material=\"C24\", tags=[\"f\"])\nk=box(\"k\", (3000,60,200), at=(120,1000,40), material=\"C24\", tags=[\"f\"])\nbox(\"w\", (100,2000,240), at=(3020,0,-200), grounded=True, tags=[\"f\"], material=\"C24\")\narunda(j, h, \"50 B\")\njoint(k, h, kind=\"hanger\", fastener=\"strmeň\", bearing=True)";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let rows = all_rows(&mut app, &mut bridge, ReportSection::Joints);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["rating"]["basis"], "allowable");
    assert!(rows.iter().all(|row| row["status"] == "not_verified"));
    assert_eq!(rows[1]["rating"], Value::Null);
    assert_eq!(
        rows[1]["missing"],
        json!(["no published rating with a source"])
    );
    assert!(rows[0]["load_n"]["permanent"].as_f64().unwrap() > 0.0);
    let members = all_rows(&mut app, &mut bridge, ReportSection::Loads);
    assert_eq!(members.len(), 4, "{members:?}");
    let joist = members.iter().find(|row| row["part"] == "j").unwrap();
    assert_eq!(joist["reactions"].as_array().unwrap().len(), 2);
}

#[test]
fn the_members_report_lists_the_timber_check_of_every_load_path_member() {
    let (mut app, mut bridge) = setup();
    let source = "load_path(only=[\"f\"])\nself_weight([\"f\"])\ntimber_design({\"C24\": \"C24\"})\nbox(\"a\", (100,100,1000), grounded=True, material=\"C24\")\nbox(\"b\", (100,100,1000), at=(2900,0,0), grounded=True, material=\"C24\")\nbox(\"beam\", (3000,100,200), at=(0,0,1000), material=\"C24\", tags=[\"f\"])";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let applied = bridge
        .execute(
            &mut app,
            apply_source(&source.replace("200)", "201)")),
            false,
        )
        .unwrap();
    let statics = &applied["report"]["statics"]["members"];
    assert_eq!(statics["members"], 1, "{applied}");
    assert_eq!(statics["pass"], 1);
    assert_eq!(statics["highest"]["part"], "beam");
    assert_eq!(applied["validation"]["load_capacity"]["state"], "passed");
    let weak_source = format!(
        "{}\narea_load(\"live\", kind=\"imposed\", kn_m2=2, on=[\"d\"], source=\"test\")\nbox(\"d\", (3000,600,20), at=(0,-250,1060), material=\"C24\", tags=[\"d\"])",
        source.replace("100,200)", "40,60)")
    );
    let weak = bridge
        .execute(&mut app, apply_source(&weak_source), false)
        .unwrap();
    assert_eq!(
        weak["report"]["statics"]["members"]["failing"],
        json!(["beam"])
    );
    assert_eq!(weak["validation"]["load_capacity"]["state"], "failed");
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let rows = all_rows(&mut app, &mut bridge, ReportSection::Members);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["part"], "beam");
    assert_eq!(rows[0]["status"], "pass");
    assert!(rows[0]["utilization"].as_f64().unwrap() > 0.0);
    assert_eq!(rows[0]["strength_class"], "C24");
}

#[test]
fn report_pages_reconstruct_complete_model_bom_and_relations() {
    let (mut app, mut bridge) = setup();
    let source = "parts=[]\nfor i in range(18):\n    p=board(\"p%d\" % i, (40,100,100), at=(i*40,0,0))\n    parts.append(p)\n    hole(p, \"z+\", at=(9,50), diameter=4, depth=8)\nfor i in range(17):\n    dowels(parts[i], parts[i+1], dowel=\"8x30\", margin=25)";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let (_, report) = ketchup_program::run(
        "local.star",
        source,
        &BTreeMap::from([("width".into(), 25.)]),
    )
    .unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let cut = all_rows(&mut app, &mut bridge, ReportSection::CutList);
    assert_eq!(cut.len(), report.bom.total_parts);
    for (group, row) in report.bom.cut_list.iter().enumerate() {
        let names: Vec<_> = cut
            .iter()
            .filter(|part| part["group"] == group)
            .map(|part| part["part"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            row.parts.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(
            cut.iter()
                .filter(|part| part["group"] == group)
                .all(|part| part["group_count"] == row.count
                    && part["dimensions_mm"] == json!(row.dimensions_mm))
        );
    }
    assert!(!report.bom.hardware.is_empty());
    assert_eq!(
        all_rows(&mut app, &mut bridge, ReportSection::Hardware),
        report
            .bom
            .hardware
            .iter()
            .map(|row| json!(row))
            .collect::<Vec<_>>()
    );
    let machining = all_rows(&mut app, &mut bridge, ReportSection::Machining);
    for part in &report.bom.machining {
        let operations: Vec<_> = machining
            .iter()
            .filter(|row| row["part"] == part.part)
            .map(|row| row["operation"].clone())
            .collect();
        assert_eq!(
            operations,
            part.operations
                .iter()
                .map(|operation| json!(operation))
                .collect::<Vec<_>>()
        );
    }
    assert!(report.relations.len() > MAX_REPORTED_RELATIONS);
    assert_eq!(
        all_rows(&mut app, &mut bridge, ReportSection::Relations),
        report
            .relations
            .iter()
            .map(|row| json!(row))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        all_rows(&mut app, &mut bridge, ReportSection::Issues),
        report
            .issues
            .iter()
            .map(|row| json!(row))
            .collect::<Vec<_>>()
    );
    assert_eq!(app.document.current().scene_query(), before);
    assert_eq!(app.undo_step_count(), undo);
}

#[test]
fn material_takeoff_report_counts_only_visible_layers_and_pages_completely() {
    let (mut app, mut bridge) = setup();
    let source = "for i in range(5):\n    box(\"stena/stĺpik %d\" % i, (60, 140, 2500), at = (i * 600, 0, 0), material = \"drevo\", tags = [\"konštrukcia\"])\n\
        box(\"stena/doska\", (2500, 15, 2500), at = (0, 200, 0), material = \"OSB\", tags = [\"konštrukcia\"])\n\
        box(\"koncept/stena\", (3000, 160, 2500), at = (0, 400, 0), material = \"drevostavba\", tags = [\"koncept\"])\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let rows = all_rows(&mut app, &mut bridge, ReportSection::MaterialTakeoff);
    let studs = rows
        .iter()
        .find(|row| row["kind"] == "row" && row["material"] == "drevo")
        .unwrap();
    assert_eq!(studs["category"], "stena");
    assert_eq!(studs["count"], 5);
    assert_eq!(studs["section_mm"], json!([140.0, 60.0]));
    assert_eq!(studs["length_m"], 12.5);
    assert_eq!(studs["volume_m3"], 0.105);
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "material_total" && row["material"] == "drevostavba")
    );

    let concept = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "koncept")
        .unwrap()
        .id();
    let undo = app.undo_step_count();
    assert!(app.set_tag_visibility(concept, false));
    let rows = all_rows(&mut app, &mut bridge, ReportSection::MaterialTakeoff);
    assert!(
        rows.iter().all(|row| row["material"] != "drevostavba"),
        "{rows:?}"
    );
    let expected = app.live_bridge_stamp();
    let page = bridge
        .execute(
            &mut app,
            Request::ProgramReport {
                expected,
                section: ReportSection::MaterialTakeoff,
                offset: 0,
                limit: 50,
            },
            false,
        )
        .unwrap();
    assert_eq!(page["counted_parts"], 6);
    assert_eq!(page["excluded_parts"], 1);

    let selection_page = |app: &mut KetchupApp, bridge: &mut LiveBridge| {
        let expected = app.live_bridge_stamp();
        bridge.execute(
            app,
            Request::ProgramReport {
                expected,
                section: ReportSection::SelectionTakeoff,
                offset: 0,
                limit: 50,
            },
            false,
        )
    };
    assert_eq!(
        selection_page(&mut app, &mut bridge),
        Err("invalid_params"),
        "nothing selected"
    );
    let studs: Vec<u64> = app
        .document
        .current()
        .scene_query()
        .iter()
        .filter(|part| {
            ketchup_application::rule_program_part_name(
                &app.document.current(),
                &part.instance_path,
            )
            .is_some_and(|name| name.starts_with("stena/stĺpik"))
        })
        .map(|part| part.instance_path.root_occurrence().0)
        .take(2)
        .collect();
    bridge
        .execute(
            &mut app,
            Request::Selection {
                expected: None,
                occurrence_ids: studs,
            },
            false,
        )
        .unwrap();
    let page = selection_page(&mut app, &mut bridge).unwrap();
    assert_eq!(page["basis"], "selected_visible_program_parts");
    assert_eq!(page["counted_parts"], 2, "{page}");
    assert_eq!(page["rows"][0]["material"], "drevo");
    assert_eq!(page["rows"][0]["count"], 2);
    assert_eq!(
        app.undo_step_count(),
        undo + 1,
        "reading the takeoff changes nothing"
    );
}
