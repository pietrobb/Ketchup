//! The sandbox of a connected AI's write targets (review 2026-11-08 §5.4). A
//! client writes a file, a document saved or anything exported, only into a
//! folder of this project: the document's folder and those under it, a folder
//! of a document in Open Recent, the system's temporary folder, or a folder the
//! user let the AI write into for this document. A file anywhere else, such as
//! a startup folder or `~/.ssh`, the window asks the user about first, and the
//! folder they agree to is the project's from then on
//! ([`KetchupApp::grant_write_target`]); the AI cannot grant it itself.
//! Replacing a file inside the sandbox follows the rules of each export (only
//! its own file unasked, `replace`/`take_over` asking in the window).

use super::*;

/// The path `request` writes a file at as the client named it: a document
/// saved or a file exported, none for a request that writes no file.
fn written_path(request: &Request) -> Option<&str> {
    match request {
        Request::SaveAs { path, .. }
        | Request::Export { path, .. }
        | Request::DrawingExport { path, .. }
        | Request::ExportProgramVariants { path, .. } => Some(path),
        Request::ExportDrawings(export) => Some(&export.path),
        Request::ApplyAndVerify {
            save: Some(ApplyAndVerifySave::Path { path }),
            ..
        } => Some(path),
        _ => None,
    }
}

/// Whether `request` saves the document rather than exporting a file.
fn saves_document(request: &Request) -> bool {
    matches!(
        request,
        Request::SaveAs { .. }
            | Request::ApplyAndVerify {
                save: Some(ApplyAndVerifySave::Path { .. }),
                ..
            }
    )
}

/// The code refusing the file `request` writes, its reason recorded, or none:
/// a document saved as anything but a `.ketchup` file, or as a hidden one, is
/// `invalid_path` (review 2026-11-08 SEC-240, now for `apply_and_verify` too),
/// and a folder outside the sandbox the user did not agree to is
/// `save_declined` or `write_target_declined`. A path that is not local and
/// absolute is left to the request, which refuses it as it did.
pub(super) fn refusal(app: &mut KetchupApp, request: &Request) -> Option<&'static str> {
    let path = written_path(request)?;
    if !drawings::local_absolute_path(path) {
        return None;
    }
    take_error_details();
    let path = Path::new(path);
    if saves_document(request) {
        if let Some(refused) = document_name_refusal(path) {
            return Some(refused);
        }
        // A file already there is asked about as replacing it.
        if path.exists() {
            return None;
        }
    }
    let folder = match folder_of(path) {
        Some(folder) => folder,
        None => {
            return Some(failure(
                "invalid_path",
                "The path's folder does not resolve: name a folder that exists, without `..` past it.",
                json!({"path": path}),
            ));
        }
    };
    if in_sandbox(app, path, &folder) {
        return None;
    }
    let (title, risk, operation) = if saves_document(request) {
        (
            "dialog-save-as-new-folder-title",
            "dialog-save-as-new-folder-risk",
            "save-document-in-new-folder",
        )
    } else {
        (
            "dialog-write-target-title",
            "dialog-write-target-risk",
            "write-into-new-folder",
        )
    };
    let title = app.catalog.text(title);
    let risk = app.catalog.text(risk);
    app.side_effect_receipts.clear();
    match app.authorize_path_side_effect(
        crate::HighRiskClass::CapabilityExpansion,
        operation,
        &title,
        &risk,
        &folder,
        folder.to_string_lossy().as_bytes(),
    ) {
        Ok(()) => {
            app.grant_write_target(folder);
            None
        }
        Err(rejection) => Some(failure(
            if saves_document(request) {
                "save_declined"
            } else {
                "write_target_declined"
            },
            format!(
                "{} The folder is not one this project writes into (the document's, one in Open Recent, the temporary folder, or one the user let the AI write into); ask the user to write there, or write beside the document.",
                rejection.reason_text()
            ),
            json!({"path": path, "folder": folder, "write_targets": app.write_targets()}),
        )),
    }
}

/// The code refusing `path` as the name of a document saved, or none: a
/// `.ketchup` file, never a hidden one.
fn document_name_refusal(path: &Path) -> Option<&'static str> {
    if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
    {
        return Some(failure(
            "invalid_path",
            "A document is saved as no hidden file: the name begins with a dot. Name the document like plan.ketchup.",
            json!({"path": path}),
        ));
    }
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ketchup"))
    {
        return Some(failure(
            "invalid_path",
            "A document is saved as a Ketchup document, whose name ends in .ketchup; export other formats with file action=export.",
            json!({"path": path}),
        ));
    }
    None
}

/// The folder `path` is written into as the disk names it: its nearest folder
/// that exists made canonical, the folders still to be made under it added.
/// None when a part past it is `..` or the root does not resolve.
pub(super) fn folder_of(path: &Path) -> Option<PathBuf> {
    let mut missing = Vec::new();
    let mut existing = path.parent()?;
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut folder) => {
                folder.extend(missing.iter().rev());
                return Some(folder);
            }
            Err(_) => {
                missing.push(existing.file_name()?);
                existing = existing.parent()?;
            }
        }
    }
}

/// Whether `folder`, the canonical folder of `path`, is one the client writes
/// into unasked.
fn in_sandbox(app: &KetchupApp, path: &Path, folder: &Path) -> bool {
    let canonical = |known: &Path| std::fs::canonicalize(known).ok();
    let mut roots: Vec<PathBuf> = app.write_targets();
    roots.extend(
        app.file
            .path
            .as_deref()
            .and_then(Path::parent)
            .and_then(canonical),
    );
    roots.extend(app.window.temporary_folder.iter().cloned());
    if roots.iter().any(|root| folder.starts_with(root)) {
        return true;
    }
    // A folder of Open Recent as written there, the disk not asked for each
    // (one may be on a share that does not answer); that folder only.
    let named = path.parent();
    app.recent_files()
        .iter()
        .filter_map(|file| file.parent())
        .any(|known| {
            Some(known) == named
                || cfg!(windows)
                    && named.is_some_and(|named| {
                        named.to_string_lossy().to_lowercase()
                            == known.to_string_lossy().to_lowercase()
                    })
        })
}
