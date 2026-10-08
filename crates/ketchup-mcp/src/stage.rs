//! Windows started by `open_window` can run from a private copy of the build,
//! so a running window never locks the executables a rebuild must replace.
//!
//! Opt-in: [`STAGE_DIR_ENV`] names the directory that holds the copies.
//! Without it the window runs the source executable itself.
use std::{
    fs,
    io::{self, Read},
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
    // The key is predictable and the root writable, so a copy found there is
    // started only when it is exactly the build: same files, same bytes.
    let mut verified = false;
    if target.exists() {
        if is_copy_of(&files, &target)? {
            verified = true;
        } else {
            fs::remove_dir_all(&target).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "the staged copy {} differs from the build {} and cannot be replaced ({error}); close the window running from it or remove the directory",
                        target.display(),
                        source_dir.display()
                    ),
                )
            })?;
        }
    }
    if !verified {
        let partial = root.join(format!("{key}.partial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(&partial)?;
        fs::write(partial.join(STAGE_MARKER), "")?;
        for file in &files {
            let file_name = file.file_name().expect("staged files have names");
            fs::copy(file, partial.join(file_name))?;
        }
        if let Err(error) = fs::rename(&partial, &target) {
            let _ = fs::remove_dir_all(&partial);
            // Another server staged the same build concurrently.
            if !is_copy_of(&files, &target)? {
                return Err(error);
            }
        }
    }
    remove_stale(root, &key, name);
    Ok(staged)
}

/// Marks a directory [`stage`] created; cleanup removes nothing else.
const STAGE_MARKER: &str = ".ketchup-staged-build";

/// `target` holds exactly `files` (and the marker) with identical contents.
fn is_copy_of(files: &[PathBuf], target: &Path) -> io::Result<bool> {
    let mut expected: Vec<_> = files
        .iter()
        .filter_map(|file| file.file_name().map(ToOwned::to_owned))
        .collect();
    expected.push(STAGE_MARKER.into());
    expected.sort();
    let mut present = fs::read_dir(target)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    present.sort();
    if present != expected {
        return Ok(false);
    }
    for file in files {
        let copy = target.join(file.file_name().expect("staged files have names"));
        if !same_contents(file, &copy)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn same_contents(left: &Path, right: &Path) -> io::Result<bool> {
    if fs::metadata(left)?.len() != fs::metadata(right)?.len() {
        return Ok(false);
    }
    let (mut left, mut right) = (fs::File::open(left)?, fs::File::open(right)?);
    let (mut left_chunk, mut right_chunk) = (vec![0; 1 << 20], vec![0; 1 << 20]);
    loop {
        let read = left.read(&mut left_chunk)?;
        if read == 0 {
            return Ok(true);
        }
        right.read_exact(&mut right_chunk[..read])?;
        if left_chunk[..read] != right_chunk[..read] {
            return Ok(false);
        }
    }
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

/// Removes older staged builds. Only directories carrying [`STAGE_MARKER`] are
/// touched: the root may be `%TEMP%` or a projects folder. A running window
/// locks its executable; its copy, worker included, stays.
fn remove_stale(root: &Path, current: &str, name: &std::ffi::OsStr) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let other = entry.file_name();
        if other == current
            || other.to_string_lossy().contains(".partial-")
            || !dir.join(STAGE_MARKER).is_file()
        {
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

    #[test]
    fn cleanup_removes_only_staged_builds_and_keeps_everything_else_in_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let release = temp.path().join("release");
        fs::create_dir(&release).unwrap();
        // The root is a folder the user also keeps other things in.
        let root = temp.path().join("projects");
        let project = root.join("house");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("house.ketchup"), "work").unwrap();
        let lookalike = root.join("0123456789abcdef");
        fs::create_dir_all(&lookalike).unwrap();
        fs::write(lookalike.join("ketchup-app.exe"), "someone else's").unwrap();
        fs::write(root.join("notes.txt"), "notes").unwrap();

        let source = build(&release, "app v1", "worker v1");
        let first = stage(&source, &root).unwrap();
        fs::write(&source, "app v2, rebuilt").unwrap();
        let second = stage(&source, &root).unwrap();

        assert_eq!(fs::read_to_string(&second).unwrap(), "app v2, rebuilt");
        assert!(!first.exists(), "the older staged build is removed");
        assert_eq!(
            fs::read_to_string(project.join("house.ketchup")).unwrap(),
            "work"
        );
        assert_eq!(
            fs::read_to_string(lookalike.join("ketchup-app.exe")).unwrap(),
            "someone else's"
        );
        assert_eq!(fs::read_to_string(root.join("notes.txt")).unwrap(), "notes");
    }

    #[test]
    fn a_staged_copy_that_differs_from_the_build_is_replaced_before_it_runs() {
        let temp = tempfile::tempdir().unwrap();
        let release = temp.path().join("release");
        fs::create_dir(&release).unwrap();
        let root = temp.path().join("stage");
        let source = build(&release, "app v1", "worker v1");
        let staged = stage(&source, &root).unwrap();

        // Same size, other bytes: the build stamp cannot tell.
        fs::write(&staged, "app X1").unwrap();
        assert_eq!(stage(&source, &root).unwrap(), staged);
        assert_eq!(fs::read_to_string(&staged).unwrap(), "app v1");

        // A library planted beside the copy would load before the build's own.
        let planted = staged.with_file_name(format!("version.{LIBRARY_EXTENSION}"));
        fs::write(&planted, "planted").unwrap();
        assert_eq!(stage(&source, &root).unwrap(), staged);
        assert!(!planted.exists());
        assert_eq!(fs::read_to_string(&staged).unwrap(), "app v1");
    }

    /// `scripts\ketchup-mcp-server.cmd` in a scratch checkout whose "builds"
    /// are `where.exe` with a marker appended: the script really starts it,
    /// it rejects `--mcp` and exits, and what the script staged stays. Plain
    /// bytes as the executable would make Windows show a modal "16-bit
    /// application" dialog instead.
    #[cfg(windows)]
    #[test]
    fn the_server_script_stages_a_rebuild_of_the_same_size_and_minute_and_repairs_a_tampered_copy()
    {
        let checkout = tempfile::tempdir().unwrap();
        let scripts = checkout.path().join("scripts");
        let release = checkout.path().join("target").join("release");
        let servers = checkout.path().join("target").join("mcp-server");
        fs::create_dir_all(&scripts).unwrap();
        fs::create_dir_all(&release).unwrap();
        let script = scripts.join("ketchup-mcp-server.cmd");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/ketchup-mcp-server.cmd"),
            &script,
        )
        .unwrap();
        let run = || {
            std::process::Command::new("cmd")
                .arg("/C")
                .arg(&script)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
        };
        let staged = || -> Vec<(Vec<u8>, Vec<u8>)> {
            fs::read_dir(&servers)
                .unwrap()
                .map(|entry| {
                    let dir = entry.unwrap().path();
                    (
                        fs::read(dir.join("ketchup-app.exe")).unwrap(),
                        fs::read(dir.join("occt.dll")).unwrap(),
                    )
                })
                .collect()
        };
        let located = std::process::Command::new("where")
            .arg("where.exe")
            .output()
            .unwrap();
        let located = String::from_utf8(located.stdout).unwrap();
        let program = fs::read(located.lines().next().unwrap().trim()).unwrap();
        let build = |marker: &[u8]| [program.as_slice(), marker].concat();
        let exe = release.join("ketchup-app.exe");
        fs::write(&exe, build(b"one")).unwrap();
        fs::write(release.join("occt.dll"), "library one").unwrap();
        run();
        assert_eq!(staged(), [(build(b"one"), b"library one".to_vec())]);

        // A rebuild of the same size within the same minute is a new build.
        let modified = fs::metadata(&exe).unwrap().modified().unwrap();
        fs::write(&exe, build(b"two")).unwrap();
        fs::File::options()
            .write(true)
            .open(&exe)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        run();
        assert_eq!(staged(), [(build(b"two"), b"library one".to_vec())]);

        // A staged copy that no longer matches the build is never started as is.
        let copy = fs::read_dir(&servers)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::write(copy.join("ketchup-app.exe"), build(b"XXX")).unwrap();
        fs::write(copy.join("occt.dll"), "library XXX").unwrap();
        run();
        assert_eq!(staged(), [(build(b"two"), b"library one".to_vec())]);
    }
}
