//! The G6 File workflow replayed through the designed shell, offscreen.
//!
//! Every step is a real click on a real menu item of the real user interface;
//! only the operating system file dialogs are answered from a script. Outcomes
//! are read from document state and from the disk, never from painted text.

mod harness;

use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_interaction::Vec3;

fn compose_two_shared_occurrences(shell: &mut Shell) {
    shell.click_at(shell.viewport_rect().center());
    assert!(
        shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)),
        "composing the model must add a second occurrence"
    );
    shell.settle();
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
}

fn lossy_legacy_document() -> Vec<u8> {
    let mut bytes = b"KETCHUPDOC".to_vec();
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&7_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&42_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(b'x');
    bytes.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}

/// The digest the shell would report for `key`, resolved through its own
/// catalog so a translation change cannot break the assertion.
fn digest_starts_like(shell: &Shell, key: &str) -> bool {
    let template = shell.catalog().text(key);
    let prefix: String = template.chars().take_while(|c| *c != '{').collect();
    !prefix.trim().is_empty() && shell.app().action_digest().starts_with(prefix.trim_end())
}

#[test]
fn save_as_then_new_then_open_restores_the_same_canonical_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("composed.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());

    compose_two_shared_occurrences(&mut shell);
    let composed_digest = shell.app().canonical_digest();
    let composed_revision = shell.app().document_revision();
    assert!(shell.app().is_dirty(), "an edited document must be dirty");

    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    assert!(path.is_file(), "Save As must write the document to disk");
    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(
        inspection.schema_version,
        ketchup_core::persistence::CURRENT_SCHEMA
    );
    assert_eq!(inspection.definitions, 1);
    assert_eq!(inspection.root_occurrences, 2);
    assert_eq!(inspection.profiles, 1);
    assert_eq!(inspection.extrusions, 1);
    assert_eq!(inspection.profile_extrusion_definitions, 1);
    assert_eq!(inspection.visible_profile_extrusion_root_occurrences, 2);
    assert_eq!(inspection.canonical_digest, composed_digest);
    assert_eq!(
        inspection.container_sha256,
        ketchup_core::graph::sha256_hex(&std::fs::read(&path).unwrap())
    );
    assert!(!shell.app().is_dirty(), "a saved document must be clean");
    assert_eq!(shell.app().document_path(), Some(path.as_path()));

    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "New must replace the composed model with an empty document"
    );
    assert_eq!(shell.app().document_path(), None);

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(
        shell.app().canonical_digest(),
        composed_digest,
        "a reopened document must keep its IDs, hierarchy, transforms, \
         parameters, and sharing"
    );
    assert_eq!(shell.app().document_revision(), composed_revision);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
    assert!(
        !shell.app().is_dirty(),
        "a freshly opened document is clean"
    );
    assert_eq!(
        script.suggested_names(),
        vec!["Untitled.ketchup".to_owned()],
        "Save As must propose the current document name"
    );
}

#[test]
fn native_document_inspection_counts_only_visible_modeled_root_occurrences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("hidden-modeled-root.ketchup");
    let script = ScriptedFileDialogs::new().queue_save(&path);
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    assert!(shell.app_mut().set_selection_visibility(false));
    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(inspection.root_occurrences, 2);
    assert_eq!(inspection.profile_extrusion_definitions, 1);
    assert_eq!(inspection.visible_profile_extrusion_root_occurrences, 1);
}

#[test]
fn confirmed_legacy_migration_writes_and_activates_only_a_new_copy() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy.ketchup");
    let destination = directory.path().join("migrated.ketchup");
    let source_bytes = lossy_legacy_document();
    std::fs::write(&source, &source_bytes).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    let active_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&source)
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(directory.path())
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);

    assert!(
        shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!shell.app().has_review_candidate());
    assert_eq!(shell.app().document_path(), Some(destination.as_path()));
    assert_eq!(shell.app().document_revision(), 8);
    assert!(!shell.app().is_dirty());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(
        ketchup_core::persistence::load_file(&source)
            .unwrap()
            .disposition(),
        ketchup_core::persistence::LoadDisposition::ReviewOnly
    );
    assert_eq!(
        ketchup_core::persistence::load_file(&destination)
            .unwrap()
            .disposition(),
        ketchup_core::persistence::LoadDisposition::EditableLossless
    );
}

#[test]
fn optional_unknown_extension_survives_app_open_edit_and_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("extended.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard()
        .always_confirm_high_risk_as(7);
    let mut shell = Shell::with_dialogs(script);
    shell.click_menu_command("menu-file", AppCommand::Save);

    let snapshot = ketchup_core::persistence::load_file(&path)
        .unwrap()
        .snapshot();
    let mut sidecars = ketchup_core::persistence::ContainerData::default();
    sidecars
        .insert_extension(
            ketchup_core::persistence::ExtensionEntry::new(
                "org.example.optional",
                "opaque.bin",
                false,
                vec![7, 8, 9],
            )
            .unwrap(),
        )
        .unwrap();
    std::fs::write(
        &path,
        ketchup_core::persistence::save_container(&snapshot, &sidecars).unwrap(),
    )
    .unwrap();

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    shell.click_menu_command("menu-file", AppCommand::Save);

    let reopened = ketchup_core::persistence::load_file(&path).unwrap();
    let extension = reopened.container_data().extensions().next().unwrap();
    assert_eq!(extension.namespace(), "org.example.optional");
    assert_eq!(extension.path(), "opaque.bin");
    assert_eq!(extension.bytes(), &[7, 8, 9]);
    assert!(!extension.required());
}

#[test]
fn save_writes_to_the_known_path_without_asking_again() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resaved.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .always_confirm_high_risk_as(8);
    let mut shell = Shell::with_dialogs(script.clone());

    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(path.is_file());

    shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0));
    shell.settle();
    let edited_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Save);

    assert_eq!(
        script.suggested_names().len(),
        1,
        "a document with a path must be saved without a second dialog"
    );
    let on_disk = ketchup_core::persistence::load_file(&path)
        .expect("the saved document reloads")
        .snapshot()
        .canonical_digest();
    assert_eq!(
        on_disk, edited_digest,
        "Save must persist the current state of the document"
    );
    assert!(!shell.app().is_dirty());
}

#[test]
fn overwrite_save_requires_payload_bound_human_receipt_before_disk_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.ketchup");
    let original = b"existing file must survive refusal".to_vec();
    std::fs::write(&path, &original).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_save(&path)
        .queue_refused_high_risk()
        .queue_high_risk_approval(42);
    let mut shell = Shell::with_dialogs(script.clone());
    compose_two_shared_occurrences(&mut shell);
    let canonical = shell.app().canonical_digest();
    let revision = shell.app().document_revision();

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(shell.app().is_dirty());
    assert!(shell.app().last_side_effect_receipt().is_none());

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let receipt = shell
        .app()
        .last_side_effect_receipt()
        .expect("approved overwrite returns an authorization receipt");
    assert_eq!(receipt.approving_human(), 42);
    assert_eq!(receipt.revision_id(), revision);
    assert_eq!(receipt.operation(), "overwrite-native-document");
    assert_eq!(
        receipt.scope().path(),
        Some(path.display().to_string().as_str())
    );
    assert_eq!(shell.app().canonical_digest(), canonical);
    assert_eq!(shell.app().document_revision(), revision);
    assert!(!shell.app().is_dirty());
    assert_eq!(script.high_risk_prompts().len(), 2);
    assert!(script.high_risk_prompts()[0].contains("Payload SHA-256:"));
    assert_eq!(
        ketchup_core::persistence::load_file(&path)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        canonical
    );
}

#[test]
fn a_failed_open_leaves_the_active_document_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.ketchup");
    std::fs::write(&malformed, b"not a ketchup document").unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&malformed)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    let before = shell.app().canonical_digest();
    let revision = shell.app().document_revision();

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(
        shell.app().canonical_digest(),
        before,
        "a failed Open must not replace the active document"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().document_path(), None);
    assert!(
        digest_starts_like(&shell, "error-open-document"),
        "a failed Open must report the localized open error, digest was {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn a_failed_save_keeps_the_document_dirty_and_reports_the_reason() {
    let directory = tempfile::tempdir().unwrap();
    let script = ScriptedFileDialogs::new().queue_save(directory.path());
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    let before = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Save);

    assert_eq!(shell.app().canonical_digest(), before);
    assert!(
        shell.app().is_dirty(),
        "a failed Save must leave the document unsaved"
    );
    assert_eq!(shell.app().document_path(), None);
    assert!(
        digest_starts_like(&shell, "error-save-document"),
        "a failed Save must report the localized save error, digest was {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn discarding_unsaved_work_is_confirmed_before_new_replaces_it() {
    let refused = ScriptedFileDialogs::new();
    let mut shell = Shell::with_dialogs(refused.clone());
    compose_two_shared_occurrences(&mut shell);
    let composed = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::New);

    assert_eq!(refused.discard_prompts(), 1, "the shell must ask first");
    assert_eq!(
        shell.app().canonical_digest(),
        composed,
        "a refused prompt must keep the composed model"
    );
    assert_eq!(shell.app().active_box_count(), 2);
}
