//! File ▸ Import ▸ Revision of the underlay (review 2026-11-08 §5.3): the architect's
//! revision B of the plan traced as an underlay is compared with it entity by entity, and
//! only what changed is added, as an underlay of its own lying on the plan.

use super::*;

fn line(handle: &str, [x0, y0, x1, y1]: [i32; 4]) -> String {
    format!("0\nLINE\n5\n{handle}\n8\nwalls\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n")
}

fn drawing(entities: &[String]) -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{}0\nENDSEC\n0\nEOF\n",
        entities.concat()
    )
}

#[test]
fn a_revision_chosen_from_the_menu_lies_on_the_plan_as_its_changes() {
    let directory = tempfile::tempdir().unwrap();
    let (a, b) = (
        directory.path().join("plan.dxf"),
        directory.path().join("plan-b.dxf"),
    );
    std::fs::write(
        &a,
        drawing(&[
            line("A0", [0, 0, 4_000, 0]),
            line("A1", [4_000, 0, 4_000, 3_000]),
        ]),
    )
    .unwrap();
    std::fs::write(
        &b,
        drawing(&[
            line("A0", [0, 0, 4_000, 0]),
            line("A1", [4_500, 0, 4_500, 3_000]),
            line("B0", [2_000, 0, 2_000, 3_000]),
        ]),
    )
    .unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &a)
        .queue_import(ImportFormat::Dxf, &b);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.document = DocumentStore::new();
    app.dispatch_file_command(AppCommand::CompareUnderlayRevision);
    assert_eq!(
        app.action_digest(),
        "Select the underlay the drawing is a revision of, then compare."
    );
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let steps = app.undo_step_count();
    // The only underlay is the one compared with; nothing need be selected.
    app.dispatch_file_command(AppCommand::CompareUnderlayRevision);

    assert_eq!(app.undo_step_count(), steps + 1, "{}", app.action_digest());
    let names = app
        .document
        .current()
        .underlays()
        .map(|underlay| underlay.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, ["plan", "plan · changes in plan-b"]);
    assert!(
        app.action_digest().starts_with(
            "Compared with the underlay entity by entity: 1 changed, 1 added, 0 removed, 1 alike."
        ),
        "{}",
        app.action_digest()
    );
}
