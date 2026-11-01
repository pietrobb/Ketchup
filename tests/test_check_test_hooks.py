import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_test_hooks", ROOT / "scripts" / "check_test_hooks.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_gated_hooks_pass_and_ungated_fail(tmp_path):
    write(tmp_path, "crates/a/src/lib.rs", (
        "impl App {\n"
        "    /// Holds a request.\n"
        '    #[cfg(feature = "testing")]\n'
        "    pub fn headless_hold(&mut self) {}\n"
        "    #[doc(hidden)]\n"
        "    pub fn headless_force(&mut self) {}\n"
        "}\n"
    ))
    assert checker.ungated_hooks(tmp_path) == ["crates/a/src/lib.rs: headless_force"]
    assert checker.check(tmp_path) == 1


def test_only_a_cfg_that_requires_testing_gates_a_hook(tmp_path):
    # Review 2026-10-31 TST-80: `not(feature = "testing")` named the feature and passed.
    # A hook is gated when the production build, without `test` and without the
    # feature, leaves it out whatever the other options are.
    write(tmp_path, "viewer/android/src/lib.rs", (
        '#[cfg(not(feature = "testing"))]\n'
        "pub fn headless_inverted() {}\n"
        '#[cfg(any(windows, feature = "testing"))]\n'
        "pub fn headless_any() {}\n"
        '#[cfg(any(test, feature = "testing"))]\n'
        "pub fn headless_test_or_testing() {}\n"
        '#[cfg(all(windows, feature = "testing"))]\n'
        "pub fn headless_all() {}\n"
        '#[cfg(all(not(test), any(feature = "testing", feature = "testing")))]\n'
        "pub fn headless_nested() {}\n"
        '#[cfg_attr(feature = "testing", allow(dead_code))]\n'
        "pub fn headless_attr() {}\n"
    ))
    assert checker.ungated_hooks(tmp_path) == [
        "viewer/android/src/lib.rs: headless_inverted",
        "viewer/android/src/lib.rs: headless_any",
        "viewer/android/src/lib.rs: headless_attr",
    ]


def test_test_code_may_name_functions_headless(tmp_path):
    write(tmp_path, "crates/a/src/lib.rs", (
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn headless_case() {}\n"
        "}\n"
    ))
    write(tmp_path, "crates/a/src/flow_tests.rs", "fn headless_case() {}\n")
    write(tmp_path, "crates/a/tests/flow.rs", "fn headless_case() {}\n")
    assert checker.check(tmp_path) == 0


def test_the_repository_has_no_ungated_hooks():
    assert checker.ungated_hooks(ROOT) == []
