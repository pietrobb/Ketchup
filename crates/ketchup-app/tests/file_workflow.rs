//! The G6 File workflow replayed through the designed shell, offscreen.
//!
//! Every step is a real click on a real menu item of the real user interface;
//! only the operating system file dialogs are answered from a script. Outcomes
//! are read from document state and from the disk, never from painted text.

mod harness;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, atomic::AtomicBool};
use std::time::Duration;

use eframe::egui::{Key, accesskit::Role};
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::document::FeatureKind;
use ketchup_core::graph::sha256_bytes;
use ketchup_core::import::{
    ImportFormat, ImportLengthUnit, ImportUnitAuthority, MAX_DXF_SOURCE_BYTES,
    MAX_STEP_SOURCE_BYTES, MAX_STL_SOURCE_BYTES,
};
use ketchup_interaction::Vec3;
use ketchup_scheduler::ExactWorkerSupervisor;

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

static EXACT_FILE_EXPORT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalState {
    revision: u64,
    digest: String,
    can_undo: bool,
    can_redo: bool,
    undo_steps: usize,
    redo_steps: usize,
    dirty: bool,
    definitions: usize,
    features: usize,
    occurrences: usize,
    mesh_bodies: usize,
    import_receipts: usize,
}

fn canonical_state(shell: &Shell) -> CanonicalState {
    CanonicalState {
        revision: shell.app().document_revision(),
        digest: shell.app().canonical_digest(),
        can_undo: shell.app().can_undo(),
        can_redo: shell.app().can_redo(),
        undo_steps: shell.app().undo_step_count(),
        redo_steps: shell.app().redo_step_count(),
        dirty: shell.app().is_dirty(),
        definitions: shell.app().definition_count(),
        features: shell.app().feature_count(),
        occurrences: shell.app().occurrence_count(),
        mesh_bodies: shell.app().mesh_body_count(),
        import_receipts: shell.app().import_receipt_count(),
    }
}

fn reachable_history_digests(shell: &mut Shell) -> (Vec<String>, Vec<String>) {
    let initial = canonical_state(shell);
    let mut undo = Vec::with_capacity(initial.undo_steps);
    for _ in 0..initial.undo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        undo.push(shell.app().canonical_digest());
    }
    for _ in 0..initial.undo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Redo);
    }

    let mut redo = Vec::with_capacity(initial.redo_steps);
    for _ in 0..initial.redo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        redo.push(shell.app().canonical_digest());
    }
    for _ in 0..initial.redo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Undo);
    }
    assert_eq!(canonical_state(shell), initial);
    (undo, redo)
}

fn assert_state_and_history_unchanged(
    shell: &mut Shell,
    expected_state: &CanonicalState,
    expected_history: &(Vec<String>, Vec<String>),
) {
    assert_eq!(&canonical_state(shell), expected_state);
    assert_eq!(&reachable_history_digests(shell), expected_history);
}

fn exact_worker_path() -> PathBuf {
    let name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(name);
    if colocated.is_file() {
        colocated
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(name)
    }
}

fn wait_for_current_exact_body(shell: &mut Shell) {
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "the real worker must publish current exact evidence within two seconds"
    );
}

fn arrange_one_visible_and_one_hidden_occurrence_with_redo(shell: &mut Shell) {
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-edit", AppCommand::Copy);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    shell.click_menu_command("menu-view", AppCommand::Hide);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    shell.click_menu_command("menu-edit", AppCommand::Undo);

    assert_eq!(shell.app().active_box_count(), 2);
    assert!(shell.app().can_undo());
    assert!(shell.app().can_redo());
    assert!(shell.app().is_dirty());
}

fn ascii_stl_facet_count(bytes: &[u8]) -> usize {
    assert!(bytes.is_ascii(), "File export must be ASCII STL");
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(
        text.lines()
            .next()
            .is_some_and(|line| line.starts_with("solid ")),
        "ASCII STL must start with a solid declaration"
    );
    assert!(
        text.lines()
            .next_back()
            .is_some_and(|line| line.starts_with("endsolid ")),
        "ASCII STL must end with a matching solid declaration"
    );
    let facets = text
        .lines()
        .filter(|line| line.trim_start().starts_with("facet normal "))
        .count();
    assert_eq!(
        text.lines()
            .filter(|line| line.trim_start().starts_with("vertex "))
            .count(),
        facets * 3
    );
    facets
}

fn valid_binary_tetrahedron() -> Vec<u8> {
    let facets: [([[f32; 3]; 3], [f32; 3]); 4] = [
        (
            [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            [0.0, 0.0, -1.0],
        ),
        (
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [0.0, -1.0, 0.0],
        ),
        (
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            [-1.0, 0.0, 0.0],
        ),
        (
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [1.0, 1.0, 1.0],
        ),
    ];
    let mut bytes = vec![0_u8; 80];
    bytes[..21].copy_from_slice(b"deterministic binary ");
    bytes.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for (vertices, normal) in facets {
        for value in normal.into_iter().chain(vertices.into_iter().flatten()) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u16.to_le_bytes());
    }
    bytes
}

fn valid_ascii_tetrahedron() -> &'static [u8] {
    b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 1\n   vertex 0 1 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 1 0 0\n   vertex 0 1 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
endsolid tetrahedron\n"
}

fn valid_dxf_subset() -> &'static [u8] {
    b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n\
0\nLWPOLYLINE\n8\ncut\n90\n3\n70\n1\n10\n0\n20\n0\n42\n0\n10\n10\n20\n0\n42\n1\n10\n0\n20\n10\n42\n0\n\
0\nTEXT\n8\nnotes\n10\n0\n20\n0\n1\nunsupported\n\
0\nENDSEC\n0\nEOF\n"
}

fn assert_persisted_stl(
    document_path: &Path,
    source: &[u8],
    unit: ImportLengthUnit,
    encoding_diagnostic: &str,
) {
    let loaded = ketchup_core::persistence::load_file(document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(source));
    assert_eq!(receipt.source_byte_len(), source.len() as u64);
    assert_eq!(receipt.units().source_unit(), unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::UserDeclared
    );
    assert!(
        receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == encoding_diagnostic)
    );
    let mesh = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh) => Some(mesh),
            _ => None,
        })
        .unwrap();
    let max_coordinate = mesh
        .vertices_mm
        .iter()
        .flatten()
        .copied()
        .fold(0.0_f64, f64::max);
    assert_eq!(max_coordinate, unit.millimetres_per_unit());
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

#[test]
fn file_menu_exports_fail_closed_when_current_exact_evidence_is_unavailable() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let stl = directory.path().join("preserved.stl");
    let stl_loss = stl.with_extension("stl.loss.txt");
    let step = directory.path().join("preserved.step");
    let step_loss = step.with_extension("step.loss.txt");
    let unavailable_worker = directory.path().join("not-an-exact-worker");
    let original_stl = b"preserve STL when exact evidence is unavailable";
    let original_stl_loss = b"preserve STL loss report when exact evidence is unavailable";
    let original_step = b"preserve STEP when exact evidence is unavailable";
    let original_step_loss = b"preserve STEP loss report when exact evidence is unavailable";
    std::fs::write(&stl, original_stl).unwrap();
    std::fs::write(&stl_loss, original_stl_loss).unwrap();
    std::fs::write(&step, original_step).unwrap();
    std::fs::write(&step_loss, original_step_loss).unwrap();
    std::fs::write(&unavailable_worker, b"not an executable worker").unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_export(&stl)
        .queue_export(&step)
        .always_confirm_high_risk_as(91);
    let mut shell = Shell::with_dialogs(script.clone());
    shell
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    shell.settle();
    assert_eq!(shell.app().exact_render_body_count(), 0);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);

    assert_eq!(std::fs::read(&stl).unwrap(), original_stl);
    assert_eq!(std::fs::read(&stl_loss).unwrap(), original_stl_loss);
    assert_eq!(std::fs::read(&step).unwrap(), original_step);
    assert_eq!(std::fs::read(&step_loss).unwrap(), original_step_loss);
    assert!(
        script.high_risk_prompts().is_empty(),
        "missing current evidence must be rejected before lossy confirmation"
    );
    assert!(shell.app().last_side_effect_receipt().is_none());
}

#[test]
fn file_menu_exports_current_visible_exact_model_without_mutating_canonical_state() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let protected_stl = directory.path().join("protected.stl");
    let protected_loss = protected_stl.with_extension("stl.loss.txt");
    let first_stl = directory.path().join("visible-a.stl");
    let second_stl = directory.path().join("visible-b.stl");
    let step = directory.path().join("visible.step");
    let script = ScriptedFileDialogs::new()
        .queue_cancelled_export()
        .queue_cancelled_export()
        .queue_export(&protected_stl)
        .queue_export(&protected_stl)
        .queue_export(&first_stl)
        .queue_export(&second_stl)
        .queue_export(&step)
        .queue_refused_high_risk()
        .queue_high_risk_approval(40)
        .queue_high_risk_approval(41)
        .queue_high_risk_approval(42)
        .queue_high_risk_approval(43)
        .queue_high_risk_approval(44)
        .queue_high_risk_approval(45);
    let mut shell = Shell::with_dialogs(script.clone());
    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        0,
        "cancelling either export dialog must not create an artifact"
    );
    assert!(script.high_risk_prompts().is_empty());

    let original_stl = b"existing STL must survive refusal";
    let original_loss = b"existing STL loss report must survive refusal";
    std::fs::write(&protected_stl, original_stl).unwrap();
    std::fs::write(&protected_loss, original_loss).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(std::fs::read(&protected_stl).unwrap(), original_stl);
    assert_eq!(std::fs::read(&protected_loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_eq!(canonical_state(&shell), before);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_ne!(std::fs::read(&protected_stl).unwrap(), original_stl);
    assert_ne!(std::fs::read(&protected_loss).unwrap(), original_loss);
    assert_eq!(canonical_state(&shell), before);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    let first_stl_bytes = std::fs::read(&first_stl).unwrap();
    let second_stl_bytes = std::fs::read(&second_stl).unwrap();
    assert_eq!(
        first_stl_bytes, second_stl_bytes,
        "the same visible exact model must produce byte-identical STL"
    );
    assert_eq!(
        ascii_stl_facet_count(&first_stl_bytes),
        12,
        "the hidden shared occurrence must not contribute facets"
    );
    let stl_loss = std::fs::read_to_string(first_stl.with_extension("stl.loss.txt")).unwrap();
    assert!(stl_loss.contains("format=ASCII STL"));
    assert!(stl_loss.contains("editability_loss="));
    assert!(stl_loss.contains("topology_loss="));
    assert!(stl_loss.contains(&format!("source_digest={}", before.digest)));

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(
        step.is_file(),
        "STEP export failed with digest {:?}",
        shell.app().action_digest()
    );
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    let step_loss = std::fs::read_to_string(step.with_extension("step.loss.txt")).unwrap();
    assert!(step_loss.contains("format=ISO 10303 STEP"));
    assert!(step_loss.contains("editability_loss="));
    assert!(step_loss.contains(&format!("source_digest={}", before.digest)));

    let requests = script.export_requests();
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.extension.as_str())
            .collect::<Vec<_>>(),
        vec!["stl", "step", "stl", "stl", "stl", "stl", "step"]
    );
    for request in &requests {
        assert!(
            request
                .filter_label
                .to_ascii_lowercase()
                .contains(request.extension.as_str()),
            "the recorded filter must identify its format: {request:?}"
        );
        assert_eq!(
            request.suggested_name,
            format!("Untitled.{}", request.extension)
        );
    }
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 7);
    assert!(prompts[2].contains(&protected_stl.display().to_string()));
    assert!(prompts[3].contains(&protected_loss.display().to_string()));
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
}

#[test]
fn file_menu_step_export_requires_bound_overwrite_approval_without_canonical_mutation() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("protected.step");
    let loss = step.with_extension("step.loss.txt");
    let original_step = b"preserve STEP until overwrite approval";
    let original_loss = b"preserve STEP loss report until overwrite approval";
    std::fs::write(&step, original_step).unwrap();
    std::fs::write(&loss, original_loss).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_export(&step)
        .queue_export(&step)
        .queue_refused_high_risk()
        .queue_high_risk_approval(111)
        .queue_high_risk_approval(112)
        .queue_high_risk_approval(113);
    let mut shell = Shell::with_dialogs(script.clone());
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(std::fs::read(&step).unwrap(), original_step);
    assert_eq!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_ne!(std::fs::read(&step).unwrap(), original_step);
    assert_ne!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(&step.display().to_string()))
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(&loss.display().to_string()))
    );
}

#[test]
fn file_import_stl_commits_one_canonical_mesh_transaction_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron.stl");
    let document_path = directory.path().join("imported.ketchup");
    std::fs::write(&path, valid_ascii_tetrahedron()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);
    let before_definitions = shell.app().definition_count();

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));

    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert_eq!(shell.app().definition_count(), before_definitions + 1);
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert!(shell.app().can_undo());
    assert!(digest_starts_like(&shell, "digest-imported-stl"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Stl,
            filter_label: shell.catalog().text("file-filter-stl"),
            extensions: vec!["stl".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().mesh_body_count(), 0);
    assert_eq!(shell.app().import_receipt_count(), 0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);

    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!shell.app().is_dirty());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert_persisted_stl(
        &document_path,
        valid_ascii_tetrahedron(),
        ImportLengthUnit::Millimetre,
        "stl.ascii",
    );
}

#[test]
fn file_import_dxf_reviews_and_commits_one_canonical_profile_transaction_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    let document_path = directory.path().join("imported-dxf.ketchup");
    std::fs::write(&path, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);
    let before_box_count = shell.app().active_box_count();

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 2);
    assert_eq!(shell.app().feature_count(), before.features + 2);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 2);
    assert_eq!(shell.app().active_box_count(), before_box_count + 2);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&shell, "digest-imported-dxf"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Dxf,
            filter_label: shell.catalog().text("file-filter-dxf"),
            extensions: vec!["dxf".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().definition_count(), before.definitions);
    assert_eq!(shell.app().feature_count(), before.features);
    assert_eq!(shell.app().occurrence_count(), before.occurrences);
    assert_eq!(shell.app().import_receipt_count(), before.import_receipts);
    assert!(shell.app().can_redo());
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::Dxf);
    assert_eq!(receipt.source_sha256(), &sha256_bytes(valid_dxf_subset()));
    assert_eq!(receipt.source_byte_len(), valid_dxf_subset().len() as u64);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("TEXT")
    }));
    assert_eq!(
        loaded_snapshot
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::SegmentProfile { .. }))
            .count(),
        2
    );
}

#[test]
fn exact_worker_derives_step_units_from_representation_context() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let original = std::fs::read_to_string(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    for (name, source, expected) in [
        (
            "millimetre",
            original.replace(
                "END-ISO-10303-21;",
                "/* misleading SI_UNIT(.CENTI.,.METRE.) comment */\nEND-ISO-10303-21;",
            ),
            ImportLengthUnit::Millimetre,
        ),
        (
            "centimetre",
            original.replace(".MILLI.", ".CENTI."),
            ImportLengthUnit::Centimetre,
        ),
        (
            "metre",
            original.replace(".MILLI.", "$"),
            ImportLengthUnit::Metre,
        ),
    ] {
        let path = directory.path().join(format!("{name}.step"));
        std::fs::write(&path, source.as_bytes()).unwrap();
        let evidence = inspector
            .inspect_step_import_with_cancellation(
                &path,
                &ketchup_core::graph::sha256_hex(source.as_bytes()),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(evidence.source_unit, expected);
    }
}

#[test]
fn file_import_exact_step_commits_undoes_and_persists_source_blob_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let source = std::fs::read(&source_path).unwrap();
    let source_hash = ketchup_core::graph::sha256_hex(&source);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source_evidence = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    let repeated_evidence = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(repeated_evidence, source_evidence);
    assert!(!source_evidence.backend.is_empty());
    assert!(!source_evidence.tolerance.is_empty());
    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-step.ketchup");
    let exported_path = directory.path().join("reexported-step.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(91)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the focused STEP workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 1);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-step"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Step,
            filter_label: shell.catalog().text("file-filter-step"),
            extensions: vec!["step".to_owned(), "stp".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().import_receipt_count(), before.import_receipts);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );

    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(shell.app().exact_render_body_count(), 1);
    assert!(
        shell.app().exact_render_triangle_count() > 0,
        "an imported STEP body without triangles is invisible and unpickable"
    );
    shell.settle();
    assert!(
        shell.app().instanced_scene_triangle_count() > 0,
        "an exact product that arrives after its revision must still reach the painted scene"
    );
    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    let screen = shell.app().project_to_screen(centre, viewport);
    shell.click_at(screen);
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "an imported STEP body must be pickable where it is painted"
    );
    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "STEP re-export failed with digest {:?}",
        shell.app().action_digest()
    );
    assert!(
        std::fs::read_to_string(&exported_path)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_evidence = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &ketchup_core::graph::sha256_hex(&exported),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, source_evidence.source_unit);
    assert_eq!(exported_evidence.solid_count, source_evidence.solid_count);
    assert!((exported_evidence.volume_mm3 - source_evidence.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (exported_evidence.bounds_mm[0][axis] - source_evidence.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (exported_evidence.bounds_mm[1][axis] - source_evidence.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.source_byte_len(), source.len() as u64);
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert_eq!(receipt.parser_id(), ketchup_core::import::STEP_PARSER_ID);
    assert_eq!(
        receipt.parser_version(),
        ketchup_core::import::STEP_PARSER_VERSION
    );
    let spec = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(spec.source_sha256, sha256_bytes(&source));
    assert_eq!(spec.source_byte_len, source.len() as u64);
    let hash = ketchup_core::graph::sha256_hex(&source);
    assert_eq!(loaded.container_data().blobs().get(&hash), Some(&source));
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn moving_an_imported_step_body_keeps_it_painted_and_drops_it_when_the_import_is_undone() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .always_confirm_high_risk_as(103);
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the imported STEP move workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    shell.settle();
    let painted = shell.app().instanced_scene_triangle_count();
    assert!(painted > 0);

    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    shell.click_at(shell.app().project_to_screen(centre, viewport));
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    // A move publishes a new revision without touching the import, and the
    // isolated worker needs seconds to re-derive the body. The body must stay
    // painted in the very next frame instead of blinking out until it catches up.
    // The move goes through a real viewport drag: a drag ends its preview in the
    // same frame that commits the revision, so the scene plan is rebuilt at the
    // moment the carried-forward products are still bound to the old revision.
    let offset = Vec3::new(40.0, 0.0, 0.0);
    shell.click_command(AppCommand::Move);
    let from = shell.app().project_to_screen(centre, viewport);
    let to = shell.app().project_to_screen(centre + offset, viewport);
    // Every single frame of the gesture counts, not just the settled result: the
    // frame that commits the revision paints before the next one can repair it,
    // and a shell only repaints on demand, so one blank frame stays on screen.
    let mut blank_frames = 0;
    shell.drag_observing(from, to, |app| {
        if app.instanced_scene_triangle_count() == 0 {
            blank_frames += 1;
        }
    });
    assert_eq!(
        blank_frames, 0,
        "no frame of a move may paint an empty scene"
    );
    shell.settle();
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "moving an imported STEP body must not invalidate it"
    );
    assert_eq!(
        shell.app().instanced_scene_triangle_count(),
        painted,
        "an imported STEP body must stay painted across a move"
    );
    let viewport = shell.viewport_rect();
    let moved = shell.app().project_to_screen(centre + offset, viewport);
    shell.click_at(moved);
    assert!(
        shell.app().hovered_selection().is_some(),
        "a moved imported STEP body must be hoverable where it is painted"
    );
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "a moved imported STEP body must be pickable where it is painted"
    );

    // Move only starts on a hovered occurrence, so a pick projection that stops
    // following the carried-forward products leaves the body painted but dead:
    // the first move works and no later one does.
    let before_second_move = shell.app().document_revision();
    shell.click_command(AppCommand::Move);
    let further = shell
        .app()
        .project_to_screen(centre + offset + offset, viewport);
    shell.drag(moved, further);
    shell.settle();
    assert_eq!(
        shell.app().document_revision(),
        before_second_move + 1,
        "a second move of an imported STEP body must still commit: {:?}",
        shell.app().action_digest()
    );

    // Carrying the product forward must stay fail-closed: undoing the import
    // removes the feature it was derived from, so it must disappear at once.
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.settle();
    assert_eq!(
        shell.app().exact_render_body_count(),
        0,
        "an imported body whose feature is gone must not be carried forward"
    );
    assert_eq!(shell.app().instanced_scene_triangle_count(), 0);
}

#[test]
fn file_import_transformed_multi_solid_step_round_trips_through_save_open_and_occt() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("transformed-multi-solid.step");
    let document_path = directory.path().join("transformed-multi-solid.ketchup");
    let exported_path = directory
        .path()
        .join("transformed-multi-solid-reexport.step");

    let source_script = ScriptedFileDialogs::new()
        .queue_export(&source_path)
        .always_confirm_high_risk_as(101);
    let mut source_shell = Shell::with_dialogs(source_script);
    source_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    compose_two_shared_occurrences(&mut source_shell);
    for _ in 0..100 {
        source_shell.settle();
        if source_shell.app().exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(source_shell.app().exact_render_body_count(), 1);
    source_shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert!(
        source_path.is_file(),
        "multi-solid STEP source export failed with digest {:?}",
        source_shell.app().action_digest()
    );

    let source = std::fs::read(&source_path).unwrap();
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source_evidence = inspector
        .inspect_step_import_with_cancellation(
            &source_path,
            &ketchup_core::graph::sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(source_evidence.solid_count, 2);
    assert!(source_evidence.bounds_mm[1][0] > 150.0);
    assert!(source_evidence.bounds_mm[1][1] > 25.0);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(102)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(shell.app().exact_render_body_count(), 1);
    shell.settle();
    assert!(
        shell.app().instanced_scene_triangle_count() > 0,
        "a reopened multi-solid STEP body must reach the painted scene"
    );
    // The two source solids are disjoint and offset by (150, 25, 0), so the
    // centre of their combined bounding box is empty space. Aim at the centre
    // of the first solid instead, which is the only point guaranteed to be on
    // material.
    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0] - 150.0) * 0.5,
        (bounds[0][1] + bounds[1][1] - 25.0) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    let screen = shell.app().project_to_screen(centre, viewport);
    shell.click_at(screen);
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "a reopened multi-solid STEP body must be pickable where it is painted"
    );

    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "multi-solid STEP re-export failed with digest {:?}",
        shell.app().action_digest()
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_evidence = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &ketchup_core::graph::sha256_hex(&exported),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, source_evidence.source_unit);
    assert_eq!(exported_evidence.solid_count, source_evidence.solid_count);
    assert!((exported_evidence.volume_mm3 - source_evidence.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (exported_evidence.bounds_mm[0][axis] - source_evidence.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (exported_evidence.bounds_mm[1][axis] - source_evidence.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn file_import_step_cancel_worker_source_stale_and_oversize_refuse_without_mutation() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let original = std::fs::read(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.step");
    let corrupt = directory.path().join("corrupt.step");
    let unsupported_unit = directory.path().join("unsupported-unit.step");
    let oversized = directory.path().join("oversized.step");
    let replacement = directory.path().join("replacement.ketchup");
    let unavailable_worker = directory.path().join("unavailable-worker");
    std::fs::write(&source, &original).unwrap();
    std::fs::write(&corrupt, b"not an ISO-10303-21 exchange file").unwrap();
    let unsupported_source = String::from_utf8(original.clone())
        .unwrap()
        .replace(".MILLI.", ".MICRO.")
        .into_bytes();
    assert_ne!(unsupported_source, original);
    std::fs::write(&unsupported_unit, unsupported_source).unwrap();
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_STEP_SOURCE_BYTES + 1)
        .unwrap();
    std::fs::write(&unavailable_worker, b"not an exact worker").unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Step)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &corrupt)
        .queue_import(ImportFormat::Step, &unsupported_unit)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &oversized)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    let mut changed = original.clone();
    changed[0] ^= 1;
    std::fs::write(&source, &changed).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
    std::fs::write(&source, &original).unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    for _ in 0..2 {
        let before_refusal = canonical_state(&shell);
        let before_refusal_history = reachable_history_digests(&mut shell);
        shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
        assert!(digest_starts_like(&shell, "error-import-step"));
        assert_state_and_history_unchanged(&mut shell, &before_refusal, &before_refusal_history);
    }

    shell.click_at(shell.viewport_rect().center());
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_change_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(
        &mut shell,
        &after_document_change,
        &after_document_change_history,
    );

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);

    let before_oversize = canonical_state(&shell);
    let before_oversize_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before_oversize, &before_oversize_history);
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn file_import_unitless_dxf_requires_and_persists_explicit_user_units() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unitless.dxf");
    let document_path = directory.path().join("unitless.ketchup");
    let source = std::str::from_utf8(valid_dxf_subset())
        .unwrap()
        .replace("9\n$INSUNITS\n70\n4\n", "")
        .into_bytes();
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &path)
        .queue_save(&document_path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert_eq!(canonical_state(&shell), before);

    shell.click_role_and_label(Role::RadioButton, &shell.catalog().text("unit-centimetre"));
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    shell.click_menu_command("menu-file", AppCommand::Save);

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Centimetre);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::UserDeclared
    );
}

#[test]
fn file_import_dxf_dialog_and_review_cancel_leave_canonical_state_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    std::fs::write(&path, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Dxf)
        .queue_import(ImportFormat::Dxf, &path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert_state_and_history_unchanged(&mut shell, &before, &history);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_dxf_refusals_preserve_exact_state_and_reachable_history() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.dxf");
    let ambiguous = directory.path().join("ambiguous.dxf");
    let oversized = directory.path().join("oversized.dxf");
    std::fs::write(
        &malformed,
        b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n1\n",
    )
    .unwrap();
    std::fs::write(
        &ambiguous,
        b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n\
0\nLINE\n10\n0\n20\n0\n11\n0\n21\n1\n\
0\nLINE\n10\n0\n20\n0\n11\n-1\n21\n0\n\
0\nENDSEC\n0\nEOF\n",
    )
    .unwrap();
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_DXF_SOURCE_BYTES + 1)
        .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &malformed)
        .queue_import(ImportFormat::Dxf, &ambiguous)
        .queue_import(ImportFormat::Dxf, &oversized);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);
    assert!(before.redo_steps > 0);

    for _ in 0..3 {
        shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
        assert!(digest_starts_like(&shell, "error-import-dxf"));
        assert_state_and_history_unchanged(&mut shell, &before, &history);
    }
}

#[test]
fn file_import_dxf_rejects_source_document_history_and_open_staleness() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    let replacement = directory.path().join("replacement.ketchup");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    let before_source_change = canonical_state(&shell);
    let before_source_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    let mut same_length_change = valid_dxf_subset().to_vec();
    same_length_change[0] = b'1';
    std::fs::write(&source, same_length_change).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before_source_change, &before_source_history);

    std::fs::write(&source, valid_dxf_subset()).unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_document_change, &after_document_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
}

#[test]
fn file_import_dxf_undo_redo_round_trip_cannot_restore_confirmability() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &source);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(canonical_state(&shell), before);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_dxf_edit_context_navigation_invalidates_review() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &source);
    let mut shell = Shell::with_dialogs(script);
    let solid = shell.top_face_centre(1);
    shell.click_at(solid);
    shell.double_click_at(solid);
    assert_eq!(shell.app().edit_context_depth(), 1);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 0);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_binary_stl_commits_through_the_same_headless_workflow() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron-binary.stl");
    let document_path = directory.path().join("binary-import.ketchup");
    let source = valid_binary_tetrahedron();
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&shell, "digest-imported-stl"));

    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_persisted_stl(
        &document_path,
        &source,
        ImportLengthUnit::Millimetre,
        "stl.binary",
    );
}

#[test]
fn file_import_stl_review_applies_and_persists_every_declared_unit() {
    let directory = tempfile::tempdir().unwrap();
    for (index, unit, label_key) in [
        (0, ImportLengthUnit::Millimetre, "unit-millimetre"),
        (1, ImportLengthUnit::Centimetre, "unit-centimetre"),
        (2, ImportLengthUnit::Metre, "unit-metre"),
        (3, ImportLengthUnit::Inch, "unit-inch"),
        (4, ImportLengthUnit::Foot, "unit-foot"),
    ] {
        let source_path = directory.path().join(format!("unit-{index}.stl"));
        let document_path = directory.path().join(format!("unit-{index}.ketchup"));
        std::fs::write(&source_path, valid_ascii_tetrahedron()).unwrap();
        let script = ScriptedFileDialogs::new()
            .queue_import(ImportFormat::Stl, &source_path)
            .queue_save(&document_path);
        let mut shell = Shell::with_dialogs(script);

        shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
        shell.click_role_and_label(Role::RadioButton, &shell.catalog().text(label_key));
        shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
        shell.click_menu_command("menu-file", AppCommand::Save);

        assert_persisted_stl(&document_path, valid_ascii_tetrahedron(), unit, "stl.ascii");
    }
}

#[test]
fn file_import_stl_cancel_and_every_refusal_leave_canonical_state_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let valid = directory.path().join("valid.stl");
    let zero_facets = directory.path().join("zero-facets.stl");
    let malformed = directory.path().join("truncated-binary.stl");
    let non_manifold = directory.path().join("open-shell.stl");
    let oversized = directory.path().join("oversized.stl");
    std::fs::write(&valid, valid_ascii_tetrahedron()).unwrap();
    let mut zero_facet_source = vec![0_u8; 80];
    zero_facet_source.extend_from_slice(&0_u32.to_le_bytes());
    std::fs::write(&zero_facets, zero_facet_source).unwrap();
    let mut truncated_binary = valid_binary_tetrahedron();
    truncated_binary.pop();
    std::fs::write(&malformed, truncated_binary).unwrap();
    let last_facet = std::str::from_utf8(valid_ascii_tetrahedron())
        .unwrap()
        .find("facet normal 1 1 1")
        .unwrap();
    let mut open_shell = valid_ascii_tetrahedron()[..last_facet].to_vec();
    open_shell.extend_from_slice(b"endsolid tetrahedron\n");
    std::fs::write(&non_manifold, open_shell).unwrap();
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_STL_SOURCE_BYTES + 1)
        .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Stl)
        .queue_import(ImportFormat::Stl, &valid)
        .queue_import(ImportFormat::Stl, &zero_facets)
        .queue_import(ImportFormat::Stl, &malformed)
        .queue_import(ImportFormat::Stl, &non_manifold)
        .queue_import(ImportFormat::Stl, &oversized);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);
    assert!(before.redo_steps > 0);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
}

#[test]
fn file_import_stl_rejects_source_document_history_and_open_staleness() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.stl");
    let replacement = directory.path().join("replacement.ketchup");
    std::fs::write(&source, valid_ascii_tetrahedron()).unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    let before_source_change = canonical_state(&shell);
    let before_source_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    let mut same_length_change = valid_ascii_tetrahedron().to_vec();
    same_length_change[0] = b'S';
    std::fs::write(&source, same_length_change).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before_source_change, &before_source_history);

    std::fs::write(&source, valid_ascii_tetrahedron()).unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_document_change, &after_document_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
}
