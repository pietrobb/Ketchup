use super::super::tests::setup;
use super::*;

const SOURCE: &str = "a=box(\"a\", (10,20,30))\nb=box(\"b\", (10,20,30), at=(100,0,0))";

fn check(expected: Option<Stamp>, source: Option<&str>, edits: Option<Vec<SourceEdit>>) -> Request {
    Request::CheckProgram {
        expected,
        source: source.map(str::to_owned),
        edits,
        overrides: None,
        file_name: None,
    }
}

fn apply(source: &str) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.into(),
        overrides: None,
        file_name: Some("check.star".into()),
        replace_document: true,
    }
}

/// Review 2026-10-09 §5.1 (3): an agent can try a program or a patch, with the
/// same answer and exact check as apply, before anything reaches the window.
#[test]
fn check_answers_like_apply_and_publishes_nothing() {
    let (mut app, mut bridge) = setup();
    app.new_document();

    let checked = bridge
        .execute(&mut app, check(None, Some(SOURCE), None), false)
        .unwrap();
    assert_eq!(checked["change"], "created");
    assert_eq!(checked["parts"], 2);
    assert_eq!(checked["requires_replace_document"], false);
    assert!(app.document.current_rule_program().is_none());
    assert_eq!(app.document.current().occurrences().count(), 0);

    bridge.execute(&mut app, apply(SOURCE), false).unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    // A third box that overlaps the second, so the exact check has work to do.
    let patch = vec![SourceEdit {
        old: "at=(100,0,0))".into(),
        new: "at=(100,0,0))\nc=box(\"c\", (10,20,30), at=(105,0,0))".into(),
    }];
    let checked = bridge
        .execute(
            &mut app,
            check(Some(stamp.clone()), None, Some(patch.clone())),
            false,
        )
        .unwrap();
    assert_eq!(checked["check_only"], true);
    assert_eq!(checked["canonical_mutation"], false);
    assert_eq!(checked["change"], "incremental");
    assert_eq!(checked["added_total"], 1);
    assert_eq!(checked["parts"], 3);
    assert_eq!(checked["source_diff"]["added_lines"], 1);

    assert_eq!(app.document.current().scene_query(), before);
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(app.document.current_rule_program().unwrap().source, SOURCE);

    let applied = bridge
        .execute(
            &mut app,
            Request::PatchProgram {
                expected: stamp,
                edits: patch,
            },
            false,
        )
        .unwrap();
    for key in [
        "change",
        "parts",
        "added_total",
        "removed_total",
        "report",
        "geometry_evaluated",
        "validation",
    ] {
        assert_eq!(checked[key], applied[key], "{key}");
    }
}

#[test]
fn a_rejected_or_ambiguous_check_publishes_nothing() {
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(SOURCE), false).unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    let edit = vec![SourceEdit {
        old: "(10,20,30))".into(),
        new: "(10,20))".into(),
    }];
    for request in [
        check(None, Some("a=box(\"a\", (10,20))"), None),
        check(Some(stamp.clone()), Some(SOURCE), Some(edit.clone())),
        check(None, None, Some(edit)),
        check(Some(stamp), None, None),
    ] {
        assert!(bridge.execute(&mut app, request, false).is_err());
        assert_eq!(app.document.current().scene_query(), before);
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(app.document.current_rule_program().unwrap().source, SOURCE);
    }
}
