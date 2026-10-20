use super::*;

/// A fresh program of many independent drilled parts, applied into the open
/// window, gets the window's normal answer with every part in one Undo step,
/// not `response_timeout` while the parts are published anyway.
#[test]
fn many_part_program_applied_into_a_new_window_gets_its_answer() {
    const PARTS: usize = 1000;
    let source = format!(
        "for i in range({PARTS}):\n    \
         p = box(\"b%d\" % i, (40, 40, 18), at=((i % 40) * 100, (i // 40) * 100, 0))\n    \
         hole(p, \"z+\", at=(20, 20), diameter=5, depth=8)\n"
    );
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.new_document();
    let undo_steps = app.undo_step_count();
    let mut wire = Wire::connect(app);
    wire.call(Request::Status {});
    let response = wire.answer_within(
        Request::ApplyProgram {
            expected: None,
            source,
            overrides: None,
            file_name: Some("parts.star".into()),
            replace_document: false,
        },
        ketchup_mcp::PROGRAM_RESPONSE_WAIT * 4,
    );
    assert!(response.ok, "{:?} {:?}", response.error, response.result);
    let result = response.result.unwrap();
    assert_eq!(result["change"], "created", "{result}");
    assert_eq!(result["added_total"], PARTS);
    assert_eq!(wire.app.document.current().occurrences().count(), PARTS);
    assert_eq!(wire.app.undo_step_count(), undo_steps + 1);
}
