//! Windows started by `open_window` can run from a private copy of the build,
//! so a running window never locks the executables a rebuild must replace.
//!
//! Opt-in: [`STAGE_DIR_ENV`] names the directory that holds the copies.
//! Without it the window runs the source executable itself.
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Directory for staged copies of the window executables.
pub const STAGE_DIR_ENV: &str = "KETCHUP_MCP_STAGE_DIR";

/// Build whose windows `open_window` starts, when the server itself runs from
/// a staged copy; defaults to the server's own executable.
pub const WINDOW_SOURCE_ENV: &str = "KETCHUP_WINDOW_SOURCE";

/// The executable `open_window` starts windows from.
pub fn window_source() -> Option<PathBuf> {
    std::env::var_os(WINDOW_SOURCE_ENV)
        .filter(|source| !source.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

/// Executables a window needs beside itself; the exact worker is optional.
const COMPANIONS: &[&str] = if cfg!(windows) {
    &["ketchup-exact-worker.exe"]
} else {
    &["ketchup-exact-worker"]
};

const LIBRARY_EXTENSION: &str = if cfg!(windows) {
    "dll"
} else if cfg!(target_os = "macos") {
    "dylib"
} else {
    "so"
};

/// Returns the executable a window should run: `source`, or its staged copy.
pub fn window_executable(source: &Path) -> io::Result<PathBuf> {
    match std::env::var_os(STAGE_DIR_ENV) {
        Some(root) if !root.is_empty() => stage(source, Path::new(&root)),
        _ => Ok(source.to_path_buf()),
    }
}

/// Copies `source` and its companions into `root/<build stamp>/` once per build
/// and removes copies of older builds that no window uses any more.
pub fn stage(source: &Path, root: &Path) -> io::Result<PathBuf> {
    let source_dir = source
        .parent()
        .ok_or_else(|| io::Error::other("the window executable has no directory"))?;
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::other("the window executable has no file name"))?;
    let mut files = vec![source.to_path_buf()];
    files.extend(
        COMPANIONS
            .iter()
            .map(|companion| source_dir.join(companion))
            .filter(|path| path.is_file()),
    );
    // The exact worker starts with an empty PATH and loads its libraries from its own directory.
    for entry in fs::read_dir(source_dir)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(LIBRARY_EXTENSION))
            && path.is_file()
        {
            files.push(path);
        }
    }
    let mut key = String::new();
    for file in &files {
        key.push_str(&build_stamp(file)?);
    }
    let key = format!("{:016x}", fnv1a(key.as_bytes()));
    let target = root.join(&key);
    let staged = target.join(name);
    if !staged.is_file() {
        let partial = root.join(format!("{key}.partial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(&partial)?;
        for file in &files {
            let file_name = file.file_name().expect("staged files have names");
            fs::copy(file, partial.join(file_name))?;
        }
        if let Err(error) = fs::rename(&partial, &target) {
            let _ = fs::remove_dir_all(&partial);
            // Another server staged the same build concurrently.
            if !staged.is_file() {
                return Err(error);
            }
        }
    }
    remove_stale(root, &key, name);
    Ok(staged)
}

fn build_stamp(file: &Path) -> io::Result<String> {
    let metadata = fs::metadata(file)?;
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    Ok(format!("{}:{modified}:{};", file.display(), metadata.len()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A running window locks its executable; its copy, worker included, stays.
fn remove_stale(root: &Path, current: &str, name: &std::ffi::OsStr) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let other = entry.file_name();
        if other == current || other.to_string_lossy().contains(".partial-") || !dir.is_dir() {
            continue;
        }
        let window = dir.join(name);
        if window.exists() && fs::remove_file(&window).is_err() {
            continue;
        }
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(dir: &Path, app: &str, worker: &str) -> PathBuf {
        let source = dir.join("ketchup-app.exe");
        fs::write(&source, app).unwrap();
        fs::write(dir.join(COMPANIONS[0]), worker).unwrap();
        fs::write(dir.join(format!("TKernel.{LIBRARY_EXTENSION}")), "library").unwrap();
        source
    }

    fn staged_dirs(root: &Path) -> Vec<PathBuf> {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    #[test]
    fn a_build_is_copied_once_with_its_worker_and_a_rebuild_replaces_the_copy() {
        let temp = tempfile::tempdir().unwrap();
        let release = temp.path().join("release");
        fs::create_dir(&release).unwrap();
        let root = temp.path().join("stage");
        let source = build(&release, "app v1", "worker v1");

        let first = stage(&source, &root).unwrap();
        assert_ne!(first, source);
        assert_eq!(fs::read_to_string(&first).unwrap(), "app v1");
        let worker = first.with_file_name(COMPANIONS[0]);
        assert_eq!(fs::read_to_string(worker).unwrap(), "worker v1");
        let library = first.with_file_name(format!("TKernel.{LIBRARY_EXTENSION}"));
        assert_eq!(fs::read_to_string(library).unwrap(), "library");
        assert_eq!(stage(&source, &root).unwrap(), first);

        // A rebuild of only the worker is a new build too.
        fs::write(release.join(COMPANIONS[0]), "worker v2, longer").unwrap();
        let second = stage(&source, &root).unwrap();
        assert_ne!(second, first);
        let worker = second.with_file_name(COMPANIONS[0]);
        assert_eq!(fs::read_to_string(worker).unwrap(), "worker v2, longer");
        assert_eq!(
            staged_dirs(&root),
            vec![second.parent().unwrap().to_path_buf()]
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "app v1");
    }
}
