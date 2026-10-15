//! The bytes a plugin runs are the bytes the user allowed (SEC-2). A plugin
//! folder is copied into a private stage before it is hashed and runs from
//! there, so a file changed or added in the folder while the consent prompt is
//! open, or later, never runs. Every hashed file, staged or outside the folder
//! (the interpreter, a script named on the command line), stays open from its
//! hash until the plugin process has ended; on Windows without write or delete
//! sharing, so nobody can change, replace or rename it meanwhile. Elsewhere the
//! files are only staged and hashed.
use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Opens `path` for reading; on Windows nobody may write, rename or delete
/// it while the handle is open.
pub(super) fn open_held(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

/// What a plugin run holds until its process has ended: the hashed files,
/// then the stage they were copied into, removed last.
#[derive(Debug, Default)]
pub(super) struct HeldFiles {
    files: Vec<File>,
    stage: Option<tempfile::TempDir>,
}

impl HeldFiles {
    /// A new private stage for a plugin folder's copy.
    pub(super) fn staged() -> io::Result<Self> {
        Ok(Self {
            files: Vec::new(),
            stage: Some(
                tempfile::Builder::new()
                    .prefix("ketchup-plugin-")
                    .tempdir()?,
            ),
        })
    }

    pub(super) fn stage(&self) -> Option<&Path> {
        self.stage.as_ref().map(tempfile::TempDir::path)
    }

    /// Opens `path` held and returns its SHA-256, read through that handle.
    pub(super) fn hash(&mut self, path: &Path) -> io::Result<String> {
        let file = open_held(path)?;
        let sha256 = ketchup_model::graph::sha256_reader_hex(&file)?;
        self.files.push(file);
        Ok(sha256)
    }

    /// Opens `path` held and returns at most `limit + 1` of its bytes.
    pub(super) fn read(&mut self, path: &Path, limit: u64) -> io::Result<Vec<u8>> {
        let file = open_held(path)?;
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut io::Read::take(&file, limit + 1), &mut bytes)?;
        self.files.push(file);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_held_file_cannot_be_written_renamed_or_deleted_but_runs_and_reads() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("plugin.py");
        fs::write(&script, "x = 1\n").unwrap();
        let mut held = HeldFiles::default();
        let sha256 = held.hash(&script).unwrap();
        assert_eq!(sha256, ketchup_model::graph::sha256_hex(b"x = 1\n"));
        assert!(fs::write(&script, "x = 2\n").is_err());
        assert!(fs::rename(&script, directory.path().join("moved.py")).is_err());
        assert!(fs::remove_file(&script).is_err());
        assert_eq!(fs::read(&script).unwrap(), b"x = 1\n");
        drop(held);
        fs::write(&script, "x = 2\n").unwrap();
    }

    #[test]
    fn a_stage_is_removed_with_its_files_after_the_run() {
        let mut held = HeldFiles::staged().unwrap();
        let stage = held.stage().unwrap().to_owned();
        fs::write(stage.join("plugin.py"), "x = 1\n").unwrap();
        held.hash(&stage.join("plugin.py")).unwrap();
        drop(held);
        assert!(!stage.exists());
    }
}
