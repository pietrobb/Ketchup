//! The folders the user let a connected AI write into for the document, its
//! sandbox of write targets (review 2026-11-08 §5.4). Kept with the user's
//! settings as the written drawings are, never in the document, so a file
//! someone sends grants no folder: one `write_target=` line per folder, the
//! document's path, its file's token, the folder's canonical path and
//! [`FOLDER`], a tab between. A copy or Save As to another file starts with
//! none; a document not saved keeps its own in memory, and the file it is
//! first saved as takes them over.

use super::*;

/// The key of a write target's line.
pub(super) const WRITE_TARGET: &str = "write_target";

/// The last field of a write target's line, where a drawing's has its SHA-256.
const FOLDER: &str = "folder";

impl KetchupApp {
    /// The canonical folders the user let the AI write into for this document,
    /// each with its subfolders.
    pub(crate) fn write_targets(&self) -> Vec<PathBuf> {
        let mut folders = self.drawings.write_targets.clone();
        if let (Some(store), Some(owner)) = (&self.window.written_drawings, self.written_owner()) {
            for line in store.values(WRITE_TARGET) {
                if let Some(line) = parsed(&line)
                    && line.sha256 == FOLDER
                    && owner.holds(&line)
                    && !folders.contains(&line.drawing)
                {
                    folders.push(line.drawing);
                }
            }
        }
        folders
    }

    /// Lets the AI write into the canonical `folder` for this document from
    /// now on, after a restart too once the document is saved.
    pub(crate) fn grant_write_target(&mut self, folder: PathBuf) {
        let (Some(store), Some(owner), Some(text)) = (
            &self.window.written_drawings,
            self.written_owner(),
            field(&folder),
        ) else {
            if !self.drawings.write_targets.contains(&folder) {
                self.drawings.write_targets.push(folder);
            }
            return;
        };
        let line = owner.line(text, FOLDER);
        let saved = store.update(WRITE_TARGET, |mut lines| {
            if !lines.contains(&line) {
                lines.push(line.clone());
            }
            lines
        });
        if let Err(error) = saved {
            ketchup_rejection::log!(
                "{} is a write target for this window only: {error}",
                folder.display()
            );
            self.drawings.write_targets.push(folder);
        }
    }

    /// The document was first saved: the folders granted while it was not
    /// become its file's.
    pub(super) fn write_targets_saved(&mut self, first_save: bool) {
        let unsaved = std::mem::take(&mut self.drawings.write_targets);
        if first_save {
            for folder in unsaved {
                self.grant_write_target(folder);
            }
        }
    }
}
