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
