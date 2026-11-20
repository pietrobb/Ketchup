#[cfg(unix)]
use super::*;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// Review 2026-11-13 R1113-SEC-600, SEC-601: the Viewer's lock and damaged copy
/// and the log were opened through a link in their place, so root wrote, appended
/// to and closed whatever file of root's it led to.
#[cfg(unix)]
#[test]
fn no_file_beside_the_users_is_opened_through_a_link_in_its_place() {
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("roots-file");
    std::fs::write(&target, "root's").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    let link = folder.path().join("viewer-settings.txt.lock");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    side_file(&link, true).unwrap_err();
    side_file(&link, false).unwrap_err();
    new_side_file(&link).unwrap_err();
    append_private(&link).unwrap_err();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "root's");
    assert_eq!(std::fs::metadata(&target).unwrap().mode() & 0o777, 0o644);
    // A link to nothing makes nothing where it leads.
    let nowhere = folder.path().join("nowhere");
    let dangling = folder.path().join("viewer.log");
    std::os::unix::fs::symlink(&nowhere, &dangling).unwrap();
    append_private(&dangling).unwrap_err();
    side_file(&dangling, true).unwrap_err();
    assert!(!nowhere.exists());
    // Files of the user's own are theirs alone.
    let own = folder.path().join("viewer.log");
    std::fs::remove_file(&own).unwrap();
    std::fs::write(&own, "older\n").unwrap();
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o644)).unwrap();
    append_private(&own).unwrap();
    assert_eq!(std::fs::metadata(&own).unwrap().mode() & 0o777, 0o600);
    let part = folder.path().join("settings.part");
    new_side_file(&part).unwrap();
    assert_eq!(std::fs::metadata(&part).unwrap().mode() & 0o777, 0o600);
    new_side_file(&part).unwrap_err();
}

/// R1113-SEC-601: a log folder that was a link had the folder it led to closed.
#[cfg(unix)]
#[test]
fn a_private_folder_closes_no_folder_a_link_in_its_place_leads_to() {
    let folder = tempfile::tempdir().unwrap();
    let shared = folder.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
    let logs = folder.path().join("Ketchup");
    std::os::unix::fs::symlink(&shared, &logs).unwrap();
    create_private_folder(&logs).unwrap();
    assert_eq!(std::fs::metadata(&shared).unwrap().mode() & 0o777, 0o755);
    let own = folder.path().join("state").join("Ketchup");
    create_private_folder(&own).unwrap();
    assert_eq!(std::fs::metadata(&own).unwrap().mode() & 0o777, 0o700);
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
    create_private_folder(&own).unwrap();
    assert_eq!(std::fs::metadata(&own).unwrap().mode() & 0o777, 0o700);
}

/// Review 2026-11-13 R1113-SEC-602: the user was the owner of `HOME` while the
/// settings were in `XDG_CONFIG_HOME`; with `sudo -E` and `HOME=/root` that was
/// root, who wrote the user's folder as root. A base of another user than the
/// home's is refused.
#[cfg(unix)]
#[test]
fn a_base_folder_of_another_user_than_the_home_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let user = if effective_user() == 0 {
        std::os::unix::fs::chown(base.path(), Some(65534), Some(65534)).unwrap();
        (65534, 65534)
    } else {
        (effective_user(), effective_group())
    };
    // `/` is root's, whom no test runs as here once the base is another's.
    let refused = owner_of_base(base.path(), Some(Path::new("/"))).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    assert!(
        refused
            .to_string()
            .contains(&*base.path().to_string_lossy()),
        "{refused}"
    );
    assert_eq!(owner_of_base(base.path(), Some(base.path())).unwrap(), user);
    assert_eq!(owner_of_base(base.path(), None).unwrap(), user);
}

/// R1113-SEC-600..602 as root: work on a folder of the user's is done as the
/// owner of its base, so a link the user put there to a folder of root's leads
/// nowhere; work on root's own folder is root's.
#[cfg(unix)]
#[test]
fn under_sudo_a_users_folder_is_written_as_the_owner_of_its_base() {
    if effective_user() != 0 {
        return;
    }
    let user = (65534, 65534);
    // Under sudo, macOS gives root a temporary folder only root can enter, so
    // the user could reach neither folder and every refusal would be that one.
    let base = tempfile::tempdir_in("/tmp").unwrap();
    std::os::unix::fs::chown(base.path(), Some(user.0), Some(user.1)).unwrap();
    let root_only = tempfile::tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(root_only.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let linked = base.path().join("Ketchup");
    std::os::unix::fs::symlink(root_only.path(), &linked).unwrap();
    std::os::unix::fs::lchown(&linked, Some(user.0), Some(user.1)).unwrap();
    let folder = UserFolder::at(base.path().to_owned(), linked.clone());
    let owner = owner_of_base(folder.base(), None).unwrap();
    assert_eq!(owner, user);
    let refused = as_user(owner, || {
        side_file(&linked.join("viewer-settings.txt.lock"), false)
    })
    .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    assert_eq!(std::fs::read_dir(root_only.path()).unwrap().count(), 0);
    assert_eq!(effective_user(), 0);
    let own = base.path().join("own");
    as_user(owner, || create_private_folder(&own)).unwrap();
    let made = std::fs::metadata(&own).unwrap();
    assert_eq!((made.uid(), made.gid()), user);
    let roots = UserFolder::at(root_only.path().to_owned(), root_only.path().join("x"));
    assert!(roots.is_running_users());
    assert!(!folder.is_running_users());
}

/// Review 2026-11-13 r95 R1113r95-SEC-700: the base's owner was read by one
/// `stat` through the links. The user's `~/.config`, a link of theirs to a
/// folder of root's, was root's, so root wrote there as root; swapped meanwhile
/// for a folder of links, root followed them. Whoever could change a folder or
/// link on the way holds it: the user, whom the system refuses root's folder.
#[cfg(unix)]
#[test]
fn under_sudo_a_folder_reached_through_the_users_own_is_theirs() {
    if effective_user() != 0 {
        return;
    }
    let (user, other) = ((65534, 65534), (65533, 65533));
    let mode = |path: &Path, mode| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    let home = tempfile::tempdir_in("/tmp").unwrap();
    std::os::unix::fs::chown(home.path(), Some(user.0), Some(user.1)).unwrap();
    let root_only = tempfile::tempdir_in("/tmp").unwrap();
    mode(root_only.path(), 0o700);
    let config = home.path().join(".config");
    std::os::unix::fs::symlink(root_only.path(), &config).unwrap();
    std::os::unix::fs::lchown(&config, Some(user.0), Some(user.1)).unwrap();
    assert_eq!(user_of_home(&config).unwrap(), user);
    assert_eq!(
        holder(&config.join("Ketchup").join("settings.txt"), true).unwrap(),
        user
    );
    // With `HOME=/root` it is refused; as the user, nothing is made in root's.
    let refused = owner_of_base(&config, Some(root_only.path())).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    let folder = UserFolder::at(config.clone(), config.join("Ketchup"));
    folder
        .as_owner(|| create_private_folder(folder.path()))
        .unwrap_err();
    assert!(!folder.is_running_users());
    assert_eq!(std::fs::read_dir(root_only.path()).unwrap().count(), 0);
    assert_eq!(effective_user(), 0);
    // Root's own folder, reached through root's folders and links, is root's.
    let roots = root_only.path().join("roots");
    std::fs::create_dir(&roots).unwrap();
    let link = root_only.path().join("link");
    std::os::unix::fs::symlink(&roots, &link).unwrap();
    assert_eq!(user_of_home(&link).unwrap().0, 0);

    // A way through the folders of two users is refused.
    let others = tempfile::tempdir_in("/tmp").unwrap();
    std::os::unix::fs::chown(others.path(), Some(other.0), Some(other.1)).unwrap();
    std::fs::remove_file(&config).unwrap();
    std::os::unix::fs::symlink(others.path(), &config).unwrap();
    std::os::unix::fs::lchown(&config, Some(user.0), Some(user.1)).unwrap();
    let refused = user_of_home(&config).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    assert!(
        refused.to_string().contains("more than one user"),
        "{refused}"
    );

    // So is one through a folder of root's anyone may write into, not sticky.
    let open = tempfile::tempdir_in("/tmp").unwrap();
    mode(open.path(), 0o777);
    std::fs::create_dir(open.path().join("in")).unwrap();
    let refused = user_of_home(&open.path().join("in")).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    assert!(
        refused
            .to_string()
            .contains(&*open.path().to_string_lossy()),
        "{refused}"
    );
}

/// Review 2026-11-13 r95 R1113r95-SEC-701: the AI clients' configuration was
/// the user of `HOME`; with `sudo -E`, `HOME=/root` and `CLAUDE_CONFIG_DIR` the
/// user's, root wrote there as root. The way to the file names the user, and a
/// `HOME` of another's is refused.
#[cfg(unix)]
#[test]
fn under_sudo_a_file_is_the_users_who_hold_the_way_to_it() {
    if effective_user() != 0 {
        return;
    }
    let user = (65534, 65534);
    let theirs = tempfile::tempdir_in("/tmp").unwrap();
    std::os::unix::fs::chown(theirs.path(), Some(user.0), Some(user.1)).unwrap();
    let roots = tempfile::tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(roots.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = theirs.path().join(".claude").join(".claude.json");
    assert_eq!(user_of_file_beside(&file, None).unwrap(), user);
    assert_eq!(
        user_of_file_beside(&file, Some(theirs.path())).unwrap(),
        user
    );
    let refused = user_of_file_beside(&file, Some(roots.path())).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    // Root's own, in root's home, is root's.
    let own = roots.path().join(".claude.json");
    assert_eq!(user_of_file_beside(&own, Some(roots.path())).unwrap().0, 0);
}

/// R1113r95-SEC-700, the race: the user's `~/.config` swapped by rename between
/// a link to a folder of root's and a link to a folder of theirs holding
/// `Ketchup -> ` another of root's won 5 199 of 20 000 tries. Whichever it is
/// when read, the way runs through the user's home, so the work is the user's
/// and nothing of root's is made in root's folders: no try wins.
#[cfg(unix)]
#[test]
fn under_sudo_a_folder_swapped_meanwhile_gets_nothing_of_roots() {
    if effective_user() != 0 {
        return;
    }
    let user = (65534, 65534);
    let home = tempfile::tempdir_in("/tmp").unwrap();
    std::os::unix::fs::chown(home.path(), Some(user.0), Some(user.1)).unwrap();
    let (root_only, root_also) = (
        tempfile::tempdir_in("/tmp").unwrap(),
        tempfile::tempdir_in("/tmp").unwrap(),
    );
    for folder in [root_only.path(), root_also.path()] {
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::create_dir(root_only.path().join("Ketchup")).unwrap();
    let theirs = home.path().join("theirs");
    std::fs::create_dir(&theirs).unwrap();
    std::os::unix::fs::symlink(root_also.path(), theirs.join("Ketchup")).unwrap();
    for path in [&theirs, &theirs.join("Ketchup")] {
        std::os::unix::fs::lchown(path, Some(user.0), Some(user.1)).unwrap();
    }
    let config = home.path().join(".config");
    let made = |name: &str, to: &Path| {
        let link = home.path().join(name);
        std::os::unix::fs::symlink(to, &link).unwrap();
        std::os::unix::fs::lchown(&link, Some(user.0), Some(user.1)).unwrap();
        link
    };
    std::fs::rename(made("to-root", root_only.path()), &config).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapping = {
        let (stop, home, config) = (stop.clone(), home.path().to_owned(), config.clone());
        let (root_only, theirs) = (root_only.path().to_owned(), theirs.clone());
        std::thread::spawn(move || {
            let mut to_root = false;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let next = home.join("next");
                std::os::unix::fs::symlink(if to_root { &root_only } else { &theirs }, &next)
                    .unwrap();
                std::os::unix::fs::lchown(&next, Some(65534), Some(65534)).unwrap();
                std::fs::rename(&next, &config).unwrap();
                to_root = !to_root;
            }
        })
    };
    let folder = UserFolder::at(config.clone(), config.join("Ketchup"));
    let mut done = 0;
    for _ in 0..20_000 {
        let lock = folder.path().join("settings.txt.lock");
        if folder
            .as_owner_beside(None, || new_side_file(&lock).map(drop))
            .is_ok()
        {
            done += 1;
            std::fs::remove_file(&lock).ok();
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapping.join().unwrap();
    let roots = |folder: &Path| std::fs::read_dir(folder).unwrap().count();
    assert_eq!(
        roots(&root_only.path().join("Ketchup")),
        0,
        "after {done} writes"
    );
    assert_eq!(roots(root_also.path()), 0, "after {done} writes");
    assert_eq!(effective_user(), 0);
}

/// Review 2026-11-09 R1109-SEC-300: root alone can be another user; any other is
/// refused it, says that rather than failing to be itself again (review
/// 2026-11-13 P3), and writes nothing.
#[cfg(unix)]
#[test]
fn only_root_becomes_another_user_and_a_refusal_says_so() {
    if effective_user() == 0 {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("x.toml");
    let refused = as_user((effective_user(), 0), || std::fs::write(&file, "x = 1\n")).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    if cfg!(target_os = "linux") {
        assert!(
            refused.to_string().starts_with("becoming user"),
            "{refused}"
        );
    }
    assert!(!file.exists());
}

/// Review 2026-11-10 R1110-SEC-401: becoming another user is the whole process's,
/// with no lock and no way back from a panic. Two writes at once turned the
/// process back to root in the middle of the other's, or left root with the
/// user's group; a panic left the process that user.
#[cfg(unix)]
#[test]
fn becoming_another_user_is_one_write_at_a_time_and_ends_root_again() {
    if effective_user() != 0 {
        return;
    }
    let before = effective_group();
    let panicked = std::panic::catch_unwind(|| {
        as_user((65534, 65534), || -> io::Result<()> {
            panic!("write broke")
        })
    });
    assert!(panicked.is_err());
    assert_eq!((effective_user(), effective_group()), (0, before));
    let writers: Vec<_> = [(65534, 65534), (65533, 65533)]
        .into_iter()
        .map(|owner| {
            std::thread::spawn(move || {
                for _ in 0..500 {
                    as_user(owner, || -> io::Result<()> {
                        assert_eq!((effective_user(), effective_group()), owner);
                        assert_eq!((effective_user(), effective_group()), owner);
                        Ok(())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!((effective_user(), effective_group()), (0, before));
}

/// Review 2026-11-12 R1112-SEC-501: becoming the user changed every thread, so
/// another thread writing into a folder of root's meanwhile was refused. Only the
/// thread writing as the user is that user.
#[cfg(unix)]
#[test]
fn becoming_another_user_is_the_calling_threads_alone() {
    use std::sync::{Arc, Barrier};
    if effective_user() != 0 {
        return;
    }
    let root_only = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root_only.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (inside, done) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    let writer = {
        let (inside, done) = (Arc::clone(&inside), Arc::clone(&done));
        std::thread::spawn(move || {
            as_user((65534, 65534), || -> io::Result<()> {
                assert_eq!(effective_user(), 65534);
                inside.wait();
                done.wait();
                Ok(())
            })
        })
    };
    inside.wait();
    let roots = (
        effective_user(),
        std::fs::write(root_only.path().join("autosave"), "x"),
    );
    done.wait();
    writer.join().unwrap().unwrap();
    assert_eq!(roots.0, 0);
    roots.1.unwrap();
}

/// Review 2026-11-13 r95 P3: that the work as the user starts no thread, which would
/// stay the user, was only written down. A thread it leaves behind is an error.
#[cfg(target_os = "linux")]
#[test]
fn work_as_the_user_that_leaves_a_thread_behind_is_an_error() {
    use std::sync::mpsc;
    if effective_user() != 0 {
        return;
    }
    let (release, held) = mpsc::channel::<()>();
    let mut left = None;
    // A user no other test is, whose writes this stray would refuse meanwhile.
    let refused = as_user((65531, 65531), || {
        left = Some(std::thread::spawn(move || held.recv().ok()));
        Ok(())
    })
    .unwrap_err();
    assert!(refused.to_string().contains("stays that user"), "{refused}");
    assert_eq!(effective_user(), 0);
    drop(release);
    left.unwrap().join().unwrap();
}

/// Review 2026-11-12 P3: a home of root's that everyone may write into
/// (`HOME=/tmp`) had root write there with the checks that read through links; a
/// home not there answered a bare `NotFound`.
#[cfg(unix)]
#[test]
fn a_home_shared_with_others_or_not_there_holds_no_configuration() {
    let shared = user_of_home(Path::new("/tmp")).unwrap_err();
    assert_eq!(shared.kind(), io::ErrorKind::PermissionDenied, "{shared}");
    assert!(shared.to_string().contains("/tmp"), "{shared}");
    let gone = tempfile::tempdir().unwrap().path().join("gone");
    let missing = user_of_home(&gone).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound, "{missing}");
    assert!(
        missing.to_string().contains(&*gone.to_string_lossy()),
        "{missing}"
    );
}
