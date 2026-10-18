import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_test_timing", ROOT / "scripts" / "check_test_timing.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_budgets_fail_outside_perf_functions(tmp_path):
    write(tmp_path, "crates/a/tests/flow.rs", (
        "fn timed() {\n"
        "    let started = Instant::now();\n"
        "    assert!(started.elapsed() < Duration::from_secs(1));\n"
        "    assert!(took < FRAME_BUDGET);\n"
        "    assert!(started.elapsed().as_millis() < 50);\n"
        "}\n"
        "fn perf_timed() {\n"
        "    assert!(started.elapsed() < Duration::from_secs(1));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/flow.rs:3",
        "crates/a/tests/flow.rs:4",
        "crates/a/tests/flow.rs:5",
    ]
    assert checker.check(tmp_path) == 1


def test_deadlines_of_waits_and_computed_durations_pass(tmp_path):
    write(tmp_path, "crates/a/tests/wait.rs", (
        "fn wait() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(30);\n"
        "    while !done() { assert!(Instant::now() < deadline); }\n"
        "    receiver.recv_timeout(Duration::from_secs(5)).unwrap();\n"
        "}\n"
    ))
    write(tmp_path, "crates/a/tests/delay.rs", "fn t() { assert!(delay <= Duration::from_secs(1)); }\n")
    assert checker.check(tmp_path) == 0


def test_only_unit_tests_of_production_files_count(tmp_path):
    write(tmp_path, "crates/a/src/lib.rs", (
        "fn poll() { if started.elapsed() < Duration::from_secs(1) { retry(); } }\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn t() { assert!(started.elapsed() < Duration::from_secs(1)); }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == ["crates/a/src/lib.rs (unit tests)"]


def test_a_perf_test_must_be_ignored(tmp_path):
    write(tmp_path, "crates/a/tests/perf.rs", (
        "#[test]\n"
        "#[ignore = \"perf: release timing budget, runs nightly\"]\n"
        "fn perf_ignored() {}\n"
        "/// Timed in every pull request.\n"
        "#[test]\n"
        "fn perf_in_the_pr_suite() {}\n"
        "fn perf_helper() {}\n"
    ))
    assert checker.violations(tmp_path) == ["crates/a/tests/perf.rs:6 perf test not ignored"]


def test_the_repository_has_no_budget_outside_perf_tests():
    assert checker.check(ROOT) == 0
