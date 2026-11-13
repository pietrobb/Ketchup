//! Review 2026-11-13 R1113-SEC-601: the app and the Viewer opened their log in
//! the user's home as root under `sudo`, through the user's links: the folder a
//! link in place of the log folder led to was closed to others, and a file of
//! root's a link in place of the log led to was appended to and closed. One test:
//! the log is the process's own.

#[cfg(unix)]
#[test]
fn a_log_is_opened_through_no_link_and_under_sudo_as_the_user() {
    use ketchup_rejection::dropped::{log_file, log_to_file, log_to_user_file};
    use ketchup_user_files::{UserFolder, effective_user};
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

    if effective_user() == 0 {
        // The user's link in place of their log folder, to a folder of root's.
        let user = (65534, 65534);
        std::os::unix::fs::chown(home.path(), Some(user.0), Some(user.1)).unwrap();
        let root_only = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root_only.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let linked = home.path().join("Ketchup");
        std::os::unix::fs::symlink(root_only.path(), &linked).unwrap();
        std::os::unix::fs::lchown(&linked, Some(user.0), Some(user.1)).unwrap();
        let folder = UserFolder::at(home.path().to_owned(), linked);
        let refused = log_to_user_file(&folder, "ketchup.log").unwrap_err();
        assert_eq!(
            refused.kind(),
            std::io::ErrorKind::PermissionDenied,
            "{refused}"
        );
        assert_eq!(std::fs::read_dir(root_only.path()).unwrap().count(), 0);
        assert_eq!(log_file(), None);
    }

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
