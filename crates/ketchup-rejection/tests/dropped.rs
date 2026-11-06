//! Review 2026-11-04 r19 TST-160: an error thrown away is written with its
//! reason and cause to the log file a window gives, not to a standard error
//! nobody reads. One test: the log is the process's own.

use ketchup_rejection::dropped::{log_to_file, lost_lines};
use ketchup_rejection::{Held, Panic, Reason, dropped};

#[test]
fn a_dropped_error_is_logged_with_its_reason_and_cause() {
    let folder = std::env::temp_dir().join(format!("ketchup-dropped-{}", std::process::id()));
    let file = folder.join("logs").join("ketchup.log");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, vec![b'x'; (1 << 20) + 1]).unwrap();
    log_to_file(&file).unwrap();

    dropped(Ok::<(), std::io::Error>(()), Reason::AlreadyEnded);
    dropped(
        Err::<(), _>(std::io::Error::other("the process has exited")),
        Reason::AlreadyEnded,
    );
    let panic = std::thread::spawn(|| std::panic::panic_any("worker gave up")).join();
    dropped(panic.map_err(Panic), Reason::PanicReported);
    let cell = std::sync::OnceLock::new();
    cell.set(1).unwrap();
    dropped(cell.set(2).map_err(Held), Reason::AlreadyHeld);
    ketchup_rejection::log!("STL import refused: {}", "no facets");

    let log = std::fs::read_to_string(&file).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 4, "{log}");
    assert!(
        lines[0].ends_with("] dropped (AlreadyEnded): the process has exited"),
        "{log}"
    );
    assert!(lines[1].ends_with("dropped (PanicReported): the thread panicked: worker gave up"));
    assert!(lines[2].ends_with("dropped (AlreadyHeld): a value is held already"));
    assert!(lines[3].ends_with("] STL import refused: no facets"));
    let mut old = file.as_os_str().to_owned();
    old.push(".old");
    assert_eq!(
        std::fs::metadata(old).unwrap().len(),
        (1 << 20) + 1,
        "a full log is kept aside once"
    );
    assert_eq!(lost_lines(), 0);
    std::fs::remove_dir_all(folder).unwrap();
}
