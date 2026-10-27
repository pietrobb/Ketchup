use super::*;

pub(crate) fn apply_source(source: &str) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.into(),
        overrides: Some(BTreeMap::from([("width".into(), 25.)])),
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
    // Review 2026-10-09 (P3): missing and repeated text got the same message.
    for (old, new, reason) in [
        ("box", "other", Some("more than once")),
        ("absent", "value", Some("does not occur")),
        ("", "value", Some("empty")),
        ("aa", "b", Some("more than once")),
        ("a=box", "a=invalid(", None),
    ] {
        assert!(
            bridge
                .execute(&mut app, patch(stamp.clone(), old, new), false)
                .is_err()
        );
        let details = take_error_details();
        if let Some(reason) = reason {
            let details = details.unwrap();
            assert!(
                details["reason"].as_str().unwrap().contains(reason),
                "{old:?}: {details}"
            );
        }
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

/// Review 2026-10-09 §5.1 (4): a patch that breaks the program answers with
/// the line and column and the patched lines around them, which the agent
/// never saw as a whole.
#[test]
fn a_rejected_patch_names_the_line_column_and_the_patched_text_around_it() {
    let (mut app, mut bridge) = setup();
    let source = "a=box(\"a\", (10,20,30))\nb=box(\"b\", (10,20,30), at=(100,0,0))\nc=box(\"c\", (10,20,30), at=(200,0,0))\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let stamp = app.live_bridge_stamp();
    assert!(
        bridge
            .execute(
                &mut app,
                patch(stamp, "(10,20,30), at=(100", "(10,20,missing), at=(100"),
                false
            )
            .is_err()
    );
    let details = take_error_details().unwrap();
    let location = &details["details"]["location"];
    assert_eq!(location["file"], "local.star", "{details}");
    assert_eq!(location["line"], 2, "{details}");
    assert!(location["column"].as_u64().unwrap() > 1, "{details}");
    let excerpt = location["excerpt"].as_array().unwrap();
    assert_eq!(
        excerpt
            .iter()
            .map(|line| (
                line["line"].as_u64().unwrap(),
                line["text"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            (1, "a=box(\"a\", (10,20,30))"),
            (2, "b=box(\"b\", (10,20,missing), at=(100,0,0))"),
            (3, "c=box(\"c\", (10,20,30), at=(200,0,0))"),
        ]
    );
    assert_eq!(app.document.current_rule_program().unwrap().source, source);
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
    // The rated Arunda joint compares its load with the allowable rating; the
    // unrated hanger cannot be verified.
    assert_eq!(rows[0]["status"], "pass", "{:?}", rows[0]);
    assert_eq!(rows[1]["status"], "not_verified");
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
fn a_box_screwed_together_lists_its_screws_as_hardware_not_in_the_cut_list() {
    let (mut app, mut bridge) = setup();
    let source = "base = box(\"base\", (200, 200, 40))\n\
         lid = box(\"lid\", (200, 200, 18), at = (0, 0, 40))\n\
         for i, (x, y) in enumerate([(20, 20), (180, 20), (20, 180), (180, 180)]):\n\
         \x20   hole(lid, \"z+\", world = (x, y, 58), diameter = 9, depth = 18, through = True)\n\
         \x20   hole(base, \"z+\", world = (x, y, 40), depth = 32, thread = (\"M8\", 25))\n\
         \x20   bolt(\"screw %d\" % (i + 1), \"M8\", 40, at = (x, y, 58))\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let screws = ["screw 1", "screw 2", "screw 3", "screw 4"];
    let (evaluated, report) = ketchup_program::run("box.star", source, &BTreeMap::new()).unwrap();
    for screw in screws {
        assert_eq!(
            evaluated.model.part(screw).unwrap().designation(),
            Some("ISO 4017 M8x40")
        );
    }
    // The screws engage the tapped base; the clearance holes in the lid are
    // settled on the exact solids, which a box-only report leaves unverified.
    assert!(
        report
            .issues
            .iter()
            .all(|issue| issue.kind != "collision" && !issue.parts.contains(&"base".to_owned())),
        "{:?}",
        report.issues
    );
    let hardware = all_rows(&mut app, &mut bridge, ReportSection::Hardware);
    assert_eq!(hardware.len(), 1, "{hardware:?}");
    assert_eq!(hardware[0]["item"], "ISO 4017 M8x40");
    assert_eq!(hardware[0]["count"], 4);
    assert_eq!(hardware[0]["parts"], json!(screws));
    assert_eq!(hardware[0]["attributes"]["thread"], "M8");
    let cut = all_rows(&mut app, &mut bridge, ReportSection::CutList);
    let mut made: Vec<_> = cut
        .iter()
        .map(|row| row["part"].as_str().unwrap())
        .collect();
    made.sort_unstable();
    assert_eq!(made, ["base", "lid"]);
    let machining = all_rows(&mut app, &mut bridge, ReportSection::Machining);
    let tapped = machining
        .iter()
        .filter(|row| row["part"] == "base" && row["operation"]["thread"].is_string())
        .count();
    assert_eq!(tapped, 4, "{machining:?}");
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

/// #2476: steel from the rules, C24 from the program's material_weight, a
/// conflict between both and a part without material. Only the two densities
/// with one value are weighed; the sum is partial and names the others.
#[test]
fn mass_weighs_parts_with_a_cited_density_and_marks_the_sum_partial() {
    let (mut app, mut bridge) = setup();
    let source = "material_weight(\"C24\", kg_m3 = 420, source = \"EN 338: C24\")\n\
        material_weight(\"aluminium\", kg_m3 = 2800, source = \"guess\")\n\
        box(\"oceľ\", (100, 50, 20), material = \"steel\")\n\
        box(\"trám\", (100, 50, 20), at = (200, 0, 0), material = \"C24\")\n\
        box(\"hliník\", (10, 10, 10), at = (400, 0, 0), material = \"aluminium\")\n\
        box(\"bez\", (10, 10, 10), at = (600, 0, 0))\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let undo = app.undo_step_count();
    let mass = |app: &mut KetchupApp, bridge: &mut LiveBridge, ids: Option<Vec<u64>>| {
        let expected = Some(app.live_bridge_stamp());
        bridge.execute(
            app,
            Request::Mass {
                expected,
                occurrence_ids: ids,
            },
            false,
        )
    };
    // Before the exact solids exist nothing is weighed, nothing is zero.
    let pending = mass(&mut app, &mut bridge, None).unwrap();
    assert_eq!(pending["assembly"]["status"], "not_verified", "{pending}");
    assert!(pending["assembly"].get("mass_kg").is_none());
    assert_eq!(pending["parts"][0]["status"], "not_computed");

    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let report = mass(&mut app, &mut bridge, None).unwrap();
    let part = |name: &str| {
        report["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["name"] == name)
            .unwrap_or_else(|| panic!("{name} in {report}"))
            .clone()
    };
    let close = |value: &Value, expected: f64| {
        (value.as_f64().unwrap() - expected).abs() <= 1.0e-9 * expected.abs().max(1.0)
    };
    let steel = part("oceľ");
    assert_eq!(steel["status"], "ok", "{steel}");
    assert!(close(&steel["mass_kg"], 0.785), "{steel}");
    assert!(close(&steel["inertia_kg_mm2"]["xx"], 0.785 * 2900.0 / 12.0));
    assert_eq!(
        steel["density"]["sources"][0]["from"],
        "validation_rules.json"
    );
    let beam = part("trám");
    assert!(close(&beam["mass_kg"], 0.042), "{beam}");
    assert_eq!(
        beam["density"]["sources"][0]["from"],
        "program material_weight"
    );
    assert_eq!(beam["density"]["sources"][0]["citation"], "EN 338: C24");
    let conflict = part("hliník");
    assert_eq!(conflict["status"], "not_verified");
    assert_eq!(conflict["density"]["sources"].as_array().unwrap().len(), 2);
    assert!(conflict.get("mass_kg").is_none());
    assert_eq!(part("bez")["reason"], "the part has no material");

    let assembly = &report["assembly"];
    assert_eq!(assembly["status"], "partial", "{assembly}");
    assert_eq!(assembly["parts_counted"], 2);
    assert_eq!(assembly["missing"].as_array().unwrap().len(), 2);
    assert!(close(&assembly["mass_kg"], 0.827));
    assert!(close(
        &assembly["centroid_mm"][0],
        (0.785 * 50.0 + 0.042 * 250.0) / 0.827
    ));
    assert!(
        report["convention"]
            .as_str()
            .unwrap()
            .contains("xy = −∫x·y dm")
    );

    // One named part alone is complete; an unknown occurrence is refused.
    let id = steel["occurrence_id"].as_u64().unwrap();
    let alone = mass(&mut app, &mut bridge, Some(vec![id])).unwrap();
    assert_eq!(alone["assembly"]["status"], "complete", "{alone}");
    assert_eq!(
        mass(&mut app, &mut bridge, Some(vec![999_999])),
        Err("invalid_params")
    );
    assert_eq!(app.undo_step_count(), undo, "weighing changes nothing");
}

/// MASS-1: a hidden part is not weighed, but it is named as missing and the sum
/// is partial, both for the whole model and when the hidden part is asked for.
#[test]
fn mass_names_hidden_parts_as_missing_instead_of_reporting_complete() {
    let (mut app, mut bridge) = setup();
    let source = "box(\"stĺp\", (100, 50, 20), material = \"steel\", tags = [\"nosné\"])\n\
        box(\"skica\", (100, 50, 20), at = (200, 0, 0), material = \"steel\", tags = [\"koncept\"])\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let mass = |app: &mut KetchupApp, bridge: &mut LiveBridge, ids: Option<Vec<u64>>| {
        let expected = Some(app.live_bridge_stamp());
        bridge
            .execute(
                app,
                Request::Mass {
                    expected,
                    occurrence_ids: ids,
                },
                false,
            )
            .unwrap()
    };
    let shown = mass(&mut app, &mut bridge, None);
    assert_eq!(shown["assembly"]["status"], "complete", "{shown}");
    let sketch = shown["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "skica")
        .unwrap()["occurrence_id"]
        .as_u64()
        .unwrap();
    let concept = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "koncept")
        .unwrap()
        .id();
    assert!(app.set_tag_visibility(concept, false));

    for ids in [None, Some(vec![sketch])] {
        let report = mass(&mut app, &mut bridge, ids.clone());
        let assembly = &report["assembly"];
        let missing = assembly["missing"].as_array().unwrap();
        assert_eq!(missing.len(), 1, "{ids:?}: {report}");
        assert_eq!(missing[0]["name"], "skica");
        assert_eq!(missing[0]["status"], "hidden");
        assert!(
            assembly["status"] != "complete",
            "{ids:?}: a hidden part left out is not complete: {report}"
        );
    }
    let whole = mass(&mut app, &mut bridge, None);
    assert_eq!(whole["assembly"]["status"], "partial");
    assert_eq!(whole["assembly"]["parts_counted"], 1);
    assert!((whole["assembly"]["mass_kg"].as_f64().unwrap() - 0.785).abs() < 1.0e-9);
}

/// #2476 step 4: without material_weight in the program the density comes
/// from the prelude's MATERIAL_WEIGHTS with its citation verbatim; an
/// `estimate:` entry, a weight per area and a material the table lacks stay
/// not verified.
#[test]
fn mass_takes_a_density_from_the_prelude_table_but_not_its_estimates() {
    let (mut app, mut bridge) = setup();
    let source = "box(\"trám\", (100, 50, 20), material = \"C24\")\n\
        box(\"doska\", (100, 50, 20), at = (200, 0, 0), material = \"gypsum board\")\n\
        box(\"krytina\", (10, 10, 10), at = (400, 0, 0), material = \"sheet metal roofing\")\n\
        box(\"skriňa\", (10, 10, 10), at = (600, 0, 0), material = \"DTD\")\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let expected = Some(app.live_bridge_stamp());
    let report = bridge
        .execute(
            &mut app,
            Request::Mass {
                expected,
                occurrence_ids: None,
            },
            false,
        )
        .unwrap();
    let part = |name: &str| {
        report["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["name"] == name)
            .unwrap_or_else(|| panic!("{name} in {report}"))
            .clone()
    };
    let beam = part("trám");
    assert_eq!(beam["status"], "ok", "{beam}");
    assert!(
        (beam["mass_kg"].as_f64().unwrap() - 0.042).abs() <= 1.0e-12,
        "{beam}"
    );
    assert_eq!(beam["density"]["kg_m3"], 420.0);
    let cited = &beam["density"]["sources"][0];
    assert_eq!(cited["from"], "library prelude MATERIAL_WEIGHTS");
    assert_eq!(cited["citation"], "EN 338: C24, mean density 420 kg/m3");

    let board = part("doska");
    assert_eq!(board["status"], "not_verified", "{board}");
    assert!(board["reason"].as_str().unwrap().starts_with("estimate:"));
    assert!(board.get("mass_kg").is_none());
    assert!(
        board["density"]["sources"][0]["citation"]
            .as_str()
            .unwrap()
            .starts_with("estimate: 12.5 mm gypsum board")
    );
    assert!(
        part("krytina")["reason"]
            .as_str()
            .unwrap()
            .contains("kg/m²")
    );
    let cabinet = part("skriňa");
    assert_eq!(cabinet["status"], "not_verified");
    assert!(
        cabinet["reason"]
            .as_str()
            .unwrap()
            .contains("material_weight(\"DTD\", kg_m3=…")
    );
    assert_eq!(report["assembly"]["status"], "partial");
    assert_eq!(report["assembly"]["parts_counted"], 1);
}

/// #2482: the library's standard parts are exact solids the window weighs:
/// the simplified body (head and shank joined, socket and bore cut) x 7850.
#[test]
fn standard_fasteners_weigh_their_simplified_steel_body() {
    let (mut app, mut bridge) = setup();
    let source = "bolt(\"hex\", \"M8\", 40)\n\
        bolt(\"cap\", \"M8\", 40, norm = \"ISO 4762\", at = (50, 0, 0))\n\
        nut(\"nut\", \"M8\", at = (100, 0, 0))\n\
        threaded_rod(\"rod\", \"M12\", 1000, at = (150, 0, 0))\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let expected = Some(app.live_bridge_stamp());
    let report = bridge
        .execute(
            &mut app,
            Request::Mass {
                expected,
                occurrence_ids: None,
            },
            false,
        )
        .unwrap();
    let hexagon = |s: f64| 3f64.sqrt() / 2.0 * s * s;
    let circle = |d: f64| std::f64::consts::PI * d * d / 4.0;
    let kg = |volume_mm3: f64| volume_mm3 * 1.0e-9 * 7850.0;
    let expected = [
        ("hex", kg(hexagon(13.0) * 5.3 + circle(8.0) * 40.0)),
        (
            "cap",
            kg(circle(13.0) * 8.0 - hexagon(6.0) * 4.0 + circle(8.0) * 40.0),
        ),
        ("nut", kg((hexagon(13.0) - circle(8.0)) * 6.8)),
        ("rod", kg(circle(12.0) * 1000.0)),
    ];
    for (name, mass) in expected {
        let part = report["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["name"] == name)
            .unwrap_or_else(|| panic!("{name} in {report}"));
        assert_eq!(part["status"], "ok", "{part}");
        let weighed = part["mass_kg"].as_f64().unwrap();
        assert!(
            (weighed - mass).abs() <= 1.0e-6 * mass,
            "{name}: {weighed} kg, expected {mass} kg"
        );
    }
    // Ø12 x 1000 mm of full-diameter steel is 0.888 kg.
    assert!((kg(circle(12.0) * 1000.0) - 0.888).abs() < 0.0005);
    assert_eq!(report["assembly"]["status"], "complete", "{report}");
}

/// The report's cutting section pages one row per piece on its bar, then the
/// pieces no stock holds: 2500 and 3000 share one 6000 bar (2500 + 4 + 3000).
#[test]
fn the_cutting_section_lists_every_piece_on_its_bar_and_what_fits_no_stock() {
    let (mut app, mut bridge) = setup();
    let source = "bar_stock('KVH', [6000], kerf = 4)\n\
        member('a', (0, 0, 0), (2500, 0, 0), (60, 120), material = 'KVH')\n\
        member('b', (0, 200, 0), (3000, 200, 0), (60, 120), material = 'KVH')\n\
        member('c', (0, 400, 0), (6500, 400, 0), (60, 120), material = 'KVH')\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let rows = all_rows(&mut app, &mut bridge, ReportSection::Cutting);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0]["piece"]["part"], "b");
    assert_eq!(rows[1]["piece"]["part"], "a");
    assert_eq!(rows[1]["piece"]["at_mm"][0], 3004.0);
    assert_eq!(rows[0]["stock_no"], 1);
    assert_eq!(rows[0]["stock_mm"][0], 6000.0);
    assert_eq!(rows[0]["offcut_mm"], 492.0);
    assert_eq!(rows[0]["kind"], "lengths");
    assert_eq!(rows[2]["unplaced"]["part"], "c");
}

/// The report's quote section pages the priced lines, the total, then what
/// has no price: 3 m of 120 x 60 is 0.0216 m3, at 480 a m3 10.37.
#[test]
fn the_quote_section_lists_priced_lines_the_total_and_unpriced_items() {
    let (mut app, mut bridge) = setup();
    let source = "member('a', (0, 0, 0), (3000, 0, 0), (60, 120), material = 'KVH')\n\
        board('p', (500, 500, 18), at = (0, 500, 0), material = 'Board')\n\
        price('KVH', 480, 'm3', currency = 'EUR')\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    let rows = all_rows(&mut app, &mut bridge, ReportSection::Quote);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0]["item"], "KVH");
    assert_eq!(rows[0]["quantity"], 0.0216);
    assert_eq!(rows[0]["unit"], "m3");
    assert_eq!(rows[0]["amount"], 10.37);
    assert_eq!(rows[0]["currency"], "EUR");
    assert_eq!(rows[1]["total"], 10.37);
    assert_eq!(rows[1]["unpriced_count"], 1);
    assert_eq!(rows[2]["unpriced"]["item"], "Board");
    assert_eq!(rows[2]["unpriced"]["reason"], "no price()");
}

/// #2482: a B500B bar Ø12 x 1000 is 0.888 kg ±1 % both in the bar schedule
/// (the library's nominal EN 10080 mass, derived; bars are ordered as pieces)
/// and in inspect mass (the exact solid x steel density). Each is checked on its
/// own: should they ever part, this fails instead of trusting one of them.
#[test]
fn a_b500b_bar_weighs_its_nominal_mass_in_the_bar_schedule_and_in_inspect_mass() {
    let (mut app, mut bridge) = setup();
    let source = "rebar(\"bar\", 12, 1000, axis = (1, 0, 0))\n";
    bridge
        .execute(&mut app, apply_source(source), false)
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let expected = Some(app.live_bridge_stamp());
    let report = bridge
        .execute(
            &mut app,
            Request::Mass {
                expected,
                occurrence_ids: None,
            },
            false,
        )
        .unwrap();
    let part = &report["parts"][0];
    assert_eq!(part["name"], "bar", "{report}");
    assert_eq!(part["status"], "ok", "{part}");
    let weighed = part["mass_kg"].as_f64().unwrap();
    assert!(
        (weighed - 0.888).abs() <= 0.00888,
        "inspect mass {weighed} kg"
    );

    assert!(all_rows(&mut app, &mut bridge, ReportSection::Hardware).is_empty());
    let bars = all_rows(&mut app, &mut bridge, ReportSection::BarSchedule);
    assert_eq!(bars.len(), 1, "{bars:?}");
    assert_eq!(bars[0]["diameter_mm"], 12.0);
    assert_eq!(bars[0]["cut_length_mm"], 1000.0);
    assert_eq!(bars[0]["count"], 1);
    let listed = bars[0]["mass_kg"].as_f64().unwrap();
    assert!(
        (listed - 0.888).abs() <= 0.00888,
        "bar schedule {listed} kg"
    );
}
