mod harness;

use harness::Shell;
use ketchup_app::AppCommand;

#[test]
fn gui_color_apply_reset_undo_and_xray_are_independent() {
    let mut shell = Shell::new();
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-edit", AppCommand::Duplicate);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    let original = shell.app().document_snapshot();
    let count = original.occurrences().count();
    assert!(count > 1);
    assert_eq!(shell.app().selected_occurrence_count(), count);
    let undo = shell.app().undo_step_count();
    shell.click_button_label("Apply color");
    assert_eq!(shell.app().undo_step_count(), undo + 1);
    assert!(
        shell
            .app()
            .document_snapshot()
            .occurrences()
            .all(|o| o.color() == Some([140, 160, 180]))
    );
    let colored_digest = shell.app().canonical_digest();
    let revision = shell.app().document_revision();
    shell.click_menu_command("menu-view", AppCommand::ViewXray);
    assert!(shell.app().xray_visible());
    assert_eq!(shell.app().canonical_digest(), colored_digest);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().undo_step_count(), undo + 1);
    shell.click_button_label("Apply color");
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "same color is a no-op"
    );
    shell.click_button_label("Reset color");
    assert!(
        shell
            .app()
            .document_snapshot()
            .occurrences()
            .all(|o| o.color().is_none())
    );
    assert_eq!(shell.app().undo_step_count(), undo + 2);
    assert!(shell.app().xray_visible());
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(
        shell
            .app()
            .document_snapshot()
            .occurrences()
            .all(|o| o.color() == Some([140, 160, 180]))
    );
    assert!(shell.app().xray_visible());
    shell.click_menu_command("menu-view", AppCommand::ViewXray);
    assert!(!shell.app().xray_visible());
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert!(
        shell
            .app()
            .document_snapshot()
            .occurrences()
            .all(|o| o.color().is_none())
    );
    assert_eq!(
        shell.app().document_snapshot().features().count(),
        original.features().count()
    );
}

#[test]
fn gui_color_survives_cut_paste_copy_paste_and_duplicate() {
    for operation in [AppCommand::Cut, AppCommand::Copy, AppCommand::Duplicate] {
        let mut shell = Shell::new();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_button_label("Apply color");
        let before = shell.app().document_snapshot();
        let source = before.occurrences().next().unwrap();
        let undo = shell.app().undo_step_count();
        shell.click_menu_command("menu-edit", operation);
        let is_cut = operation == AppCommand::Cut;
        if operation != AppCommand::Duplicate {
            assert_eq!(shell.app().undo_step_count(), undo + usize::from(is_cut));
            shell.click_menu_command("menu-edit", AppCommand::Paste);
        }
        assert_eq!(
            shell.app().undo_step_count(),
            undo + 1 + usize::from(is_cut)
        );
        let after = shell.app().document_snapshot();
        assert_eq!(after.occurrences().count(), if is_cut { 1 } else { 2 });
        assert!(
            after
                .occurrences()
                .all(|occurrence| occurrence.color() == Some([140, 160, 180]))
        );
        assert!(
            after
                .occurrences()
                .all(|occurrence| occurrence.definition_id() == source.definition_id())
        );
        assert_eq!(
            format!("{:?}", before.features().collect::<Vec<_>>()),
            format!("{:?}", after.features().collect::<Vec<_>>())
        );
        let digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().document_snapshot().occurrences().count(),
            if is_cut { 0 } else { 1 }
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), digest);
    }
}
