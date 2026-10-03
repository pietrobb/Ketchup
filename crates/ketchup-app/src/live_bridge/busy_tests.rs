use super::*;

#[test]
fn busy_diagnostics_selection_alone_allows_edit() {
    let (mut app, mut bridge) = setup();
    app.selection.select_occurrence(OccurrenceId(1), true);
    let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(status["selection"], json!([1]));
    assert_eq!(status["busy"], false);
    assert_eq!(status["busy_reasons"], json!([]));
    let commit = proposal(&mut app, &mut bridge);
    let result = bridge.execute(&mut app, commit, false).unwrap();
    assert_eq!(result["committed"], true);
    assert_eq!(app.undo_step_count(), 1);
}

#[test]
fn busy_diagnostics_report_all_blockers_and_preserve_human_work() {
    let (mut app, mut bridge) = setup();
    let commit = proposal(&mut app, &mut bridge);
    app.active_tool = ActiveTool::Helix;
    app.parameter.editor_node = Some(NodeId(999));
    app.parameter.expression_input = "unfinished human expression".into();
    app.tool_preview = foreign_push_pull_preview();
    let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    let reasons = status["busy_reasons"].as_array().unwrap();
    for (kind, target) in [
        ("tool", "helix"),
        ("editor", "parameter_editor"),
        ("preview", "tool_preview"),
    ] {
        assert!(
            reasons
                .iter()
                .any(|r| r["kind"] == kind && r["target"] == target)
        );
    }
    assert_eq!(bridge.execute(&mut app, commit, false), Err("busy"));
    let error = Response::error(1, "busy").result.unwrap();
    assert_eq!(error["details"]["busy_reasons"], status["busy_reasons"]);
    assert!(error["reason"].as_str().unwrap().contains("preview"));
    assert!(
        error["fix_hint"]
            .as_str()
            .unwrap()
            .contains("do not cancel")
    );
    assert_eq!(app.active_tool, ActiveTool::Helix);
    assert!(app.tool_preview.is_some());
    assert_eq!(
        app.parameter.expression_input,
        "unfinished human expression"
    );
    assert_eq!(app.undo_step_count(), 0);
}

#[test]
fn busy_diagnostics_transient_pointer_clears_without_clearing_selection() {
    let (mut app, mut bridge) = setup();
    app.selection.select_occurrence(OccurrenceId(1), true);
    app.camera.wheel_active = true;
    let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(status["busy_reasons"][0]["target"], "camera_wheel");
    assert_eq!(status["busy_reasons"][0]["kind"], "pointer");
    assert!(
        !status["busy_reasons"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("editor")
    );
    assert_eq!(
        bridge.execute(&mut app, Request::Undo { expected: None }, false),
        Err("busy")
    );
    let error = Response::error(2, "busy").result.unwrap();
    assert_eq!(error["details"]["busy_reasons"], status["busy_reasons"]);
    assert!(app.camera.wheel_active);
    app.camera.wheel_active = false; // User finishes the interaction, not the bridge.
    let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(status["busy"], false);
    assert_eq!(status["selection"], json!([1]));
}

#[test]
fn busy_diagnostics_headless_keyboard_focus_is_not_pointer_or_editor() {
    let (mut app, mut bridge) = setup();
    let ctx = egui::Context::default();
    let mut text = String::from("human input");
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
            });
        });
    }
    assert!(ui_busy(&ctx));
    let status = bridge
        .execute(&mut app, Request::Status {}, ui_busy(&ctx))
        .unwrap();
    assert_eq!(status["busy_reasons"][0]["kind"], "keyboard");
    assert_eq!(
        bridge.execute(&mut app, Request::Undo { expected: None }, ui_busy(&ctx)),
        Err("busy")
    );
    let error = Response::error(3, "busy").result.unwrap();
    assert_eq!(error["details"]["busy_reasons"], status["busy_reasons"]);
    assert_eq!(text, "human input");
    let _ = ctx.run(
        egui::RawInput {
            focused: false,
            ..Default::default()
        },
        |_| {},
    );
    assert!(
        !ui_busy(&ctx),
        "a field in an unfocused window must not block"
    );
    let status = bridge
        .execute(&mut app, Request::Status {}, ui_busy(&ctx))
        .unwrap();
    assert_eq!(status["busy_reasons"], json!([]));
}
