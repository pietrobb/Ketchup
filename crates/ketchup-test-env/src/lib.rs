//! The environment variables that steer Ketchup tests, one name per meaning.
//!
//! Tests of every crate read their settings here instead of naming a variable
//! of their own, so one thing (the Python interpreter, refreshing golden files,
//! where measurements go) is set the same way for the whole workspace.
//! `scripts/check_test_env.py` keeps other variable names out of test code.

use std::ffi::OsString;
use std::path::PathBuf;

/// The Python interpreter tests run scripts with.
pub const PYTHON: &str = "KETCHUP_PYTHON";
/// When set, tests rewrite their golden files from the actual output.
pub const UPDATE_GOLDEN: &str = "KETCHUP_UPDATE_GOLDEN";
/// The directory tests write their measurements and inspectable artifacts to.
pub const REPORT_DIR: &str = "KETCHUP_TEST_REPORT_DIR";

/// The configured Python interpreter, for tests that must not run without one.
pub fn configured_python() -> Option<OsString> {
    std::env::var_os(PYTHON)
}

/// The configured Python interpreter, or the platform's default one on `PATH`.
pub fn python() -> OsString {
    configured_python().unwrap_or_else(|| {
        OsString::from(if cfg!(windows) {
            "python.exe"
        } else {
            "python3"
        })
    })
}

/// Whether golden files are to be rewritten instead of compared.
pub fn update_golden() -> bool {
    std::env::var_os(UPDATE_GOLDEN).is_some()
}

/// Where a test writes the report `name` when a report directory is set; the
/// directory is created. `None` means the test keeps the report to itself.
pub fn report_path(name: &str) -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os(REPORT_DIR)?);
    std::fs::create_dir_all(&directory).expect("create the test report directory");
    Some(directory.join(name))
}
