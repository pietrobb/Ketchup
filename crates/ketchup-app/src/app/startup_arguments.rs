//! The document the window was started with (a double-click, "Open with", a
//! shortcut): one path, made absolute against the folder the window started
//! in, so a later Open of the same file is the same document. What is not a
//! document file is refused with a message in the window, which has no
//! console to print to, and before anything is written next to it (review
//! 2026-10-18 UI-13).

use crate::*;
use std::ffi::OsString;

impl KetchupApp {
    /// Queues the document named by the window's start `arguments`, or says
    /// in the window why there is none to open.
    pub fn queue_startup_arguments(&mut self, arguments: Vec<OsString>) {
        let [argument] = arguments.as_slice() else {
            if arguments.len() > 1 {
                self.digest = self.catalog.format(
                    "error-startup-arguments",
                    &BTreeMap::from([("count", arguments.len().to_string())]),
                );
            }
            return;
        };
        let refused = |key: &str, path: &Path| {
            (
                key.to_owned(),
                BTreeMap::from([("path", path.display().to_string())]),
            )
        };
        let text = argument.to_string_lossy();
        let refusal = if text.starts_with('-') {
            Some(refused("error-startup-switch", Path::new(argument)))
        } else {
            match std::path::absolute(argument) {
                Err(_) => Some(refused("error-startup-missing", Path::new(argument))),
                Ok(path) if path.is_dir() => Some(refused("error-startup-folder", &path)),
                Ok(path)
                    if path.extension().is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("ketchup-view")
                    }) =>
                {
                    Some(refused("error-startup-viewer-package", &path))
                }
                // Opening it as a document would lock it and fail on its content.
                Ok(path) if super::import_file::import_format(&path).is_some() => {
                    Some(refused("error-startup-import", &path))
                }
                Ok(path) if !path.is_file() => Some(refused("error-startup-missing", &path)),
                Ok(path) => {
                    self.queue_open_document_path(&path);
                    None
                }
            }
        };
        if let Some((key, arguments)) = refusal {
            self.digest = self.catalog.format(&key, &arguments);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(app: &KetchupApp) -> Option<PathBuf> {
        app.file.activity.queued_open_path()
    }

    /// Review 2026-10-18 UI-13: a relative path opens as the absolute one, so
    /// a later Open of the same file is not a second owner of its recovery.
    #[test]
    fn a_relative_start_path_is_opened_as_the_absolute_one() {
        let mut app = KetchupApp::new();
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = directory.path().join("house.ketchup");
        assert!(KetchupApp::new().save_document_to(&path));
        let relative = path
            .strip_prefix(std::env::current_dir().unwrap())
            .unwrap()
            .to_owned();
        assert!(relative.is_relative());
        app.queue_startup_arguments(vec![relative.into_os_string()]);
        assert_eq!(queued(&app), Some(path));
    }

    /// A switch, a folder, a missing file and a Viewer package are refused in
    /// the window, nothing is queued, and no lock file is left beside them;
    /// several paths are refused as well, since the window has no console.
    #[test]
    fn what_is_not_one_document_is_refused_in_the_window_and_leaves_nothing_behind() {
        let directory = tempfile::tempdir().unwrap();
        let viewer = directory.path().join("kitchen.ketchup-view");
        std::fs::write(&viewer, b"package").unwrap();
        // Review 2026-10-31 P3: `Ketchup plan.dxf` left a lock beside the
        // drawing and ended in a general error.
        let drawing = directory.path().join("plan.DXF");
        std::fs::write(&drawing, b"0\nEOF\n").unwrap();
        let missing = directory.path().join("missing.ketchup");
        let cases = [
            (OsString::from("--help"), "error-startup-switch"),
            (directory.path().into(), "error-startup-folder"),
            (missing.clone().into(), "error-startup-missing"),
            (viewer.clone().into(), "error-startup-viewer-package"),
            (drawing.clone().into(), "error-startup-import"),
        ];
        for (argument, key) in cases {
            let mut app = KetchupApp::new();
            app.queue_startup_arguments(vec![argument.clone()]);
            let expected = app.catalog.format(
                key,
                &BTreeMap::from([("path", Path::new(&argument).display().to_string())]),
            );
            assert_eq!(app.digest, expected, "{argument:?}");
            assert_eq!(queued(&app), None);
        }
        let mut left = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        left.sort();
        assert_eq!(
            left,
            [
                OsString::from("kitchen.ketchup-view"),
                OsString::from("plan.DXF")
            ]
        );

        let mut app = KetchupApp::new();
        app.queue_startup_arguments(vec![missing.into(), viewer.into()]);
        assert_eq!(
            app.digest,
            app.catalog.format(
                "error-startup-arguments",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert_eq!(queued(&app), None);
    }
}
