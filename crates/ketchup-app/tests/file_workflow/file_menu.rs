//! The File menu (#2689): Open Recent, and the formats in submenus.

use super::*;
use ketchup_app::KetchupApp;
use ketchup_app::recent_files::MemoryRecentFiles;
use std::collections::BTreeMap;

fn shell_remembering(store: &MemoryRecentFiles, script: ScriptedFileDialogs) -> Shell {
    Shell::from_app(
        KetchupApp::new()
            .with_dialogs(Box::new(script))
            .with_recent_files(Box::new(store.clone())),
    )
}

fn open_recent(shell: &mut Shell) {
    shell.open_menu("menu-file");
    shell.open_submenu("file-open-recent");
}

fn entry_label(path: &Path) -> String {
    path.display().to_string()
}

#[test]
fn open_recent_lists_saved_documents_newest_first_and_survives_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.ketchup");
    let second = directory.path().join("second.ketchup");
    let store = MemoryRecentFiles::default();
    let script = ScriptedFileDialogs::new()
        .queue_save(&first)
        .queue_save(&second)
        .always_discard();
    let mut shell = shell_remembering(&store, script);

    // The first save of a new model, then Save As of another one.
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(first.is_file() && second.is_file());
    assert_eq!(shell.app().recent_files(), [second.clone(), first.clone()]);
    drop(shell);

    // A new window reads the list the last one kept.
    let mut restarted = shell_remembering(&store, ScriptedFileDialogs::new().always_discard());
    assert_eq!(
        restarted.app().recent_files(),
        [second.clone(), first.clone()]
    );
    open_recent(&mut restarted);
    let labels = restarted.visible_button_labels();
    let position = |path: &Path| {
        labels
            .iter()
            .position(|label| *label == entry_label(path))
            .unwrap_or_else(|| panic!("{} is not offered: {labels:?}", path.display()))
    };
    assert!(position(&second) < position(&first), "{labels:?}");
    let clear = restarted.catalog().text("file-recent-clear");
    assert!(position(&first) < labels.iter().position(|l| *l == clear).unwrap());

    restarted.click_button_label(&entry_label(&first));
    assert_eq!(restarted.app().document_path(), Some(first.as_path()));
    assert_eq!(
        restarted.app().recent_files(),
        [first.clone(), second.clone()]
    );
    assert_eq!(store.stored(), vec![first, second]);
}

#[test]
fn a_vanished_recent_document_is_reported_and_dropped_without_a_crash() {
    let directory = tempfile::tempdir().unwrap();
    let kept = directory.path().join("kept.ketchup");
    let gone = directory.path().join("gone.ketchup");
    let store = MemoryRecentFiles::holding(vec![gone.clone(), kept.clone()]);
    let mut shell = shell_remembering(
        &store,
        ScriptedFileDialogs::new()
            .queue_save(&kept)
            .always_discard(),
    );
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    let revision = shell.app().document_revision();

    open_recent(&mut shell);
    let missing = shell.catalog().format(
        "file-recent-missing-name",
        &BTreeMap::from([("path", entry_label(&gone))]),
    );
    let labels = shell.visible_button_labels();
    assert!(labels.contains(&missing), "{labels:?}");
    assert!(labels.contains(&entry_label(&kept)), "{labels:?}");
    shell.click_button_label(&missing);

    assert_eq!(shell.app().document_path(), None);
    assert_eq!(shell.app().document_revision(), revision);
    assert!(
        shell.app().action_digest().contains(&entry_label(&gone)),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().recent_files(), std::slice::from_ref(&kept));
    assert_eq!(store.stored(), vec![kept]);
}

/// Review 2026-11-04 P3 GUI: a document whose folder did not answer, as on a
/// server that is offline, was dropped from Open Recent for good when
/// clicked. It is said to be unavailable and stays for when it is back.
#[test]
fn a_recent_document_whose_folder_does_not_answer_stays_listed_when_clicked() {
    let directory = tempfile::tempdir().unwrap();
    // No folder above it answers: on Windows a drive nothing is mapped to,
    // as a share whose server is off; elsewhere a folder outside the home
    // folder, as an unmounted `/Volumes/NAS`. A folder deleted on this
    // computer's own disk is gone instead (review 2026-11-04 r19 P3).
    let volume = if cfg!(windows) {
        ('D'..='Z')
            .rev()
            .map(|letter| PathBuf::from(format!("{letter}:\\")))
            .find(|root| !root.exists())
            .expect("a free drive letter")
    } else {
        directory.path().to_owned()
    };
    let offline = volume.join("server-share").join("house.ketchup");
    let store = MemoryRecentFiles::holding(vec![offline.clone()]);
    let mut shell = shell_remembering(&store, ScriptedFileDialogs::new().always_discard());
    let revision = shell.app().document_revision();

    open_recent(&mut shell);
    let unavailable = shell.catalog().format(
        "file-recent-unavailable-name",
        &BTreeMap::from([("path", entry_label(&offline))]),
    );
    let labels = shell.visible_button_labels();
    assert!(labels.contains(&unavailable), "{labels:?}");
    shell.click_button_label(&unavailable);

    assert_eq!(shell.app().document_path(), None);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().format(
            "digest-recent-unavailable",
            &BTreeMap::from([("path", entry_label(&offline))]),
        )
    );
    assert_eq!(shell.app().recent_files(), std::slice::from_ref(&offline));
    assert_eq!(store.stored(), vec![offline]);
}

#[test]
fn opening_a_recent_document_over_unsaved_changes_asks_first() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("saved.ketchup");
    let store = MemoryRecentFiles::default();
    // The script saves once and then refuses to discard changes.
    let script = ScriptedFileDialogs::new().queue_save(&saved);
    let mut shell = shell_remembering(&store, script.clone());
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    compose_two_shared_occurrences(&mut shell);
    assert!(shell.app().is_dirty());
    let digest = shell.app().canonical_digest();

    open_recent(&mut shell);
    shell.click_button_label(&entry_label(&saved));

    assert_eq!(script.discard_prompts(), 1);
    assert!(shell.app().is_dirty());
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn clear_list_empties_open_recent_and_the_settings() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("saved.ketchup");
    let store = MemoryRecentFiles::default();
    let mut shell = shell_remembering(&store, ScriptedFileDialogs::new().queue_save(&saved));
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert_eq!(store.stored(), vec![saved]);

    open_recent(&mut shell);
    let clear = shell.catalog().text("file-recent-clear");
    shell.click_button_label(&clear);

    assert!(shell.app().recent_files().is_empty());
    assert!(store.stored().is_empty());
}

#[test]
fn the_file_menu_keeps_formats_in_submenus_and_analysis_moves_to_the_tools_menu() {
    let mut shell = Shell::new();
    let label = |shell: &Shell, command| shell.app().command_label(command);
    let submenu = |shell: &Shell, key: &str| format!("{} ⏵", shell.catalog().text(key));

    shell.open_menu("menu-file");
    let top = shell.visible_button_labels();
    let expected_top = [
        label(&shell, AppCommand::New),
        label(&shell, AppCommand::Open),
        submenu(&shell, "file-open-recent"),
        label(&shell, AppCommand::Save),
        label(&shell, AppCommand::SaveAs),
        submenu(&shell, "menu-file-import"),
        submenu(&shell, "menu-file-export"),
        label(&shell, AppCommand::ExportProjectDrawings),
        submenu(&shell, "menu-file-viewer"),
        submenu(&shell, "menu-file-manufacturing"),
    ];
    let offered: Vec<&String> = expected_top
        .iter()
        .map(|item| {
            top.iter()
                .find(|label| *label == item)
                .unwrap_or_else(|| panic!("File does not offer {item:?}: {top:?}"))
        })
        .collect();
    let order: Vec<usize> = offered
        .iter()
        .map(|item| top.iter().position(|label| label == *item).unwrap())
        .collect();
    assert!(order.is_sorted(), "{top:?}");
    for command in [
        AppCommand::ImportMeshStl,
        AppCommand::ExportExactStep,
        AppCommand::ExportViewer,
        AppCommand::ExportHundeggerBtlx,
        AppCommand::ReviewStaticFea,
        AppCommand::ReviewLocalPdm,
    ] {
        assert!(
            !top.contains(&label(&shell, command)),
            "{command:?} is still on the top level of File: {top:?}"
        );
    }
    shell.press_key(Key::Escape);

    for (key, commands) in [
        (
            "menu-file-import",
            vec![
                AppCommand::ImportMeshStl,
                AppCommand::ImportDrawingDxf,
                AppCommand::ImportDrawingUnderlay,
                AppCommand::ImportExactStep,
                AppCommand::ImportExactIges,
                AppCommand::ImportSketchupScene,
                AppCommand::ImportBlenderGlb,
            ],
        ),
        (
            "menu-file-export",
            vec![
                AppCommand::ExportExactStep,
                AppCommand::ExportExactIges,
                AppCommand::ExportMeshStl,
                AppCommand::ExportPrintThreeMf,
                AppCommand::ExportBlenderGlb,
                AppCommand::ExportSketchupDae,
                AppCommand::ExportDrawingDxf,
            ],
        ),
        (
            "menu-file-viewer",
            vec![AppCommand::ExportViewer, AppCommand::OpenViewerComments],
        ),
        (
            "menu-file-manufacturing",
            vec![
                AppCommand::ExportGeneralFabrication,
                AppCommand::ExportWeldmentCutList,
                AppCommand::ExportSheetMetalManufacturing,
                AppCommand::ExportHomagMpr,
                AppCommand::ReviewCamExport,
                AppCommand::ExportHundeggerBtlx,
                AppCommand::ExportWorkshopPackage,
            ],
        ),
    ] {
        shell.open_menu("menu-file");
        shell.open_submenu(key);
        let offered = shell.visible_button_labels();
        let positions: Vec<usize> = commands
            .iter()
            .map(|&command| {
                offered
                    .iter()
                    .position(|item| *item == label(&shell, command))
                    .unwrap_or_else(|| panic!("{key} does not offer {command:?}: {offered:?}"))
            })
            .collect();
        assert!(positions.is_sorted(), "{key}: {offered:?}");
        shell.press_key(Key::Escape);
    }

    shell.open_menu("menu-file");
    shell.open_submenu("menu-file-manufacturing");
    assert!(
        shell
            .visible_button_labels()
            .contains(&submenu(&shell, "file-short-btlx-options"))
    );
    shell.press_key(Key::Escape);

    shell.open_menu("menu-tools");
    shell.open_submenu("menu-tools-analysis");
    let analysis = shell.visible_button_labels();
    for command in [AppCommand::ReviewStaticFea, AppCommand::ReviewLocalPdm] {
        assert!(analysis.contains(&label(&shell, command)), "{analysis:?}");
    }
    shell.press_key(Key::Escape);
}

#[test]
fn the_formats_in_the_submenus_carry_short_names_in_both_languages() {
    for catalog in [
        ketchup_interaction::LocaleCatalog::english(),
        ketchup_interaction::LocaleCatalog::slovak(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.open_menu("menu-file");
        shell.open_submenu("menu-file-export");
        let step = shell.catalog().text("file-short-step");
        assert!(
            shell.paints_text(&step),
            "the Export submenu does not paint {step:?}"
        );
        // A screen reader and the command search keep the whole name.
        let full = shell.app().command_label(AppCommand::ExportExactStep);
        assert_ne!(full, step);
        assert!(shell.visible_button_labels().contains(&full));
        shell.press_key(Key::Escape);
    }
}
