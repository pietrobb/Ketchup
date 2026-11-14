//! Review 2026-11-13 R1113-SEC-601: the app and the Viewer opened their log in
//! the user's home as root under `sudo`, through the user's links: the folder a
//! link in place of the log folder led to was closed to others, and a file of
//! root's a link in place of the log led to was appended to and closed. A binary
//! of its own: the log is the process's own.

#[cfg(unix)]
#[test]
fn a_log_is_opened_through_no_link_and_under_sudo_as_the_user() {
    use ketchup_rejection::dropped::{log_file, log_to_file};
    use std::os::unix::fs::PermissionsExt;
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let roots = tempfile::tempdir().unwrap();
    std::fs::set_permissions(roots.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let secret = roots.path().join("shadow");
    std::fs::write(&secret, "root's\n").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();

    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    std::fs::create_dir(&state).unwrap();
    let log = state.join("ketchup.log");
    std::os::unix::fs::symlink(&secret, &log).unwrap();
    log_to_file(&log).unwrap_err();
    assert_eq!(std::fs::read_to_string(&secret).unwrap(), "root's\n");
    assert_eq!(mode(&secret), 0o644);
    assert_eq!(log_file(), None);

    // A link in place of the log folder: the folder it leads to stays as it was.
    let shared = roots.path().join("logs");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
    let folder = home.path().join("logs");
    std::os::unix::fs::symlink(&shared, &folder).unwrap();
    log_to_file(&folder.join("ketchup.log")).unwrap();
    assert_eq!(mode(&shared), 0o755);
    assert_eq!(mode(&shared.join("ketchup.log")), 0o600);
}

/// The home of the user [`under_sudo_the_log_is_the_users_and_a_link_leads_it_nowhere`]
/// runs in, set by the run of it that started this one.
#[cfg(unix)]
const USERS_HOME: &str = "KETCHUP_TEST_USERS_HOME";

/// Review 2026-11-13 r95 P3: under `sudo` `HOME` is `/root`, so the log folder of
/// the user's home was refused as another user's than `HOME`'s before anything was
/// written as the user, and the test passed for that. Root runs this test again
/// with `HOME` the user's, as `sudo -E` keeps it: their own log folder is theirs,
/// and their link in place of it to a folder of root's leads the log nowhere.
#[cfg(unix)]
#[test]
fn under_sudo_the_log_is_the_users_and_a_link_leads_it_nowhere() {
    use ketchup_rejection::dropped::{log_file, log_to_user_file};
    use ketchup_user_files::{UserFolder, effective_user};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if effective_user() != 0 {
        return;
    }
    let user = (65534, 65534);
    let Some(home) = std::env::var_os(USERS_HOME).map(std::path::PathBuf::from) else {
        let home = tempfile::tempdir().unwrap();
        std::os::unix::fs::chown(home.path(), Some(user.0), Some(user.1)).unwrap();
        let name = "under_sudo_the_log_is_the_users_and_a_link_leads_it_nowhere";
        let run = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--test-threads", "1"])
            .env("HOME", home.path())
            .env(USERS_HOME, home.path())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&run.stdout);
        assert!(
            run.status.success(),
            "{said}{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(said.contains("test result: ok. 1 passed"), "{said}");
        return;
    };
    // The user's link in place of their log folder, to a folder of root's.
    let root_only = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root_only.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let linked = home.join("Ketchup");
    std::os::unix::fs::symlink(root_only.path(), &linked).unwrap();
    std::os::unix::fs::lchown(&linked, Some(user.0), Some(user.1)).unwrap();
    let refused =
        log_to_user_file(&UserFolder::at(home.clone(), linked), "ketchup.log").unwrap_err();
    assert_eq!(
        refused.kind(),
        std::io::ErrorKind::PermissionDenied,
        "{refused}"
    );
    assert_eq!(std::fs::read_dir(root_only.path()).unwrap().count(), 0);
    assert_eq!(log_file(), None);
    // Their own log folder: made and written as them.
    let logs = home.join("logs");
    log_to_user_file(&UserFolder::at(home.clone(), logs.clone()), "ketchup.log").unwrap();
    for written in [logs.clone(), logs.join("ketchup.log")] {
        let found = std::fs::metadata(&written).unwrap();
        assert_eq!((found.uid(), found.gid()), user, "{}", written.display());
    }
    assert_eq!(log_file(), Some(logs.join("ketchup.log")));
}
