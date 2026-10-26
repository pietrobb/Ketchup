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


def test_the_ways_around_the_first_check_are_budgets_too(tmp_path):
    # Review 2026-10-18 P3: a lowercase bound, a reversed comparison, a struct field, a
    # short wait the test needs and a perf_* helper called from an ordinary test all
    # passed.
    write(tmp_path, "crates/a/tests/flow.rs", (
        "fn timed() {\n"
        "    let elapsed = Instant::now().elapsed();\n"
        "    assert!(elapsed < budget);\n"
        "    assert!(Duration::from_millis(50) > started.elapsed());\n"
        "    assert!(report.cancel_ms <= 250.0);\n"
        "    assert!(stats.took.as_secs_f64() * 1000.0 < 250.0, \"slow\");\n"
        "    finished\n"
        "        .recv_timeout(Duration::from_millis(250))\n"
        "        .expect(\"cleanup blocked\");\n"
        "    let reply = queue.recv_timeout(SHORT).unwrap();\n"
        "    perf_within_budget(&frames);\n"
        "}\n"
        "const SHORT: Duration = Duration::from_secs(2);\n"
        "fn perf_within_budget(frames: &[Duration]) {\n"
        "    assert!(frames.iter().all(|frame| *frame < FRAME_BUDGET));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/flow.rs:{line}" for line in (3, 4, 5, 6, 8, 10, 11)
    ]


def test_the_shapes_the_second_check_missed_are_budgets_too(tmp_path):
    # Review 2026-10-24 TST-21: a measured time kept under another name, a percentile
    # field, the comparison on a line of its own, `.lt()` and `assert_lt!` all passed.
    write(tmp_path, "crates/a/tests/flow.rs", (
        "fn timed() {\n"
        "    let started = Instant::now();\n"
        "    let spent = started.elapsed();\n"
        "    assert!(spent < limit);\n"
        "    assert!(report.p95 < 16.0);\n"
        "    assert!(\n"
        "        started.elapsed()\n"
        "            < limit,\n"
        "    );\n"
        "    assert!(started.elapsed().lt(&limit));\n"
        "    assert!(FRAME_BUDGET.gt(&spent));\n"
        "    assert_lt!(started.elapsed(), limit);\n"
        "    debug_assert_ge!(FRAME_BUDGET, spent);\n"
        # Lower bounds, and a deadline of a wait, stay allowed in every shape.
        "    assert_le!(limit, spent);\n"
        "    assert!(spent.ge(&limit));\n"
        "    assert!(\n"
        "        Instant::now()\n"
        "            < deadline\n"
        "    );\n"
        "    let count = parts.len();\n"
        "    assert!(count < 3);\n"
        "    assert!(\n"
        "        (back.total_ms - sent.total_ms).abs()\n"
        "            <= f64::EPSILON * sent.total_ms.abs()\n"
        "    );\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/flow.rs:{line}" for line in (4, 5, 8, 10, 11, 12, 13)
    ]
    # A unit-test module compiled for tests on one platform is a test module.
    write(tmp_path, "crates/a/src/lib.rs", (
        "fn poll() { let _ = started.elapsed(); }\n"
        "#[cfg(all(test, windows))]\n"
        "mod tests {\n"
        "    fn t() { assert!(started.elapsed() < Duration::from_secs(1)); }\n"
        "}\n"
    ))
    assert "crates/a/src/lib.rs:4" in checker.violations(tmp_path)


def test_unit_tests_in_an_indented_module_count(tmp_path):
    # Review 2026-10-18 P3: only a `mod tests` at the start of a line was read.
    write(tmp_path, "crates/a/src/lib.rs", (
        "mod inner {\n"
        "    fn poll() { let _ = started.elapsed(); }\n"
        "    #[cfg(test)]\n"
        "    mod tests {\n"
        "        fn t() { assert!(started.elapsed() < Duration::from_secs(1)); }\n"
        "    }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == ["crates/a/src/lib.rs:5"]


def test_lower_bounds_polls_and_patient_waits_pass(tmp_path):
    write(tmp_path, "crates/a/tests/wait.rs", (
        "const PATIENCE: Duration = Duration::from_secs(5);\n"
        "fn wait() {\n"
        "    let started = Instant::now();\n"
        "    assert!(started.elapsed() >= Duration::from_millis(1800));\n"
        "    assert!(budgets.len() >= 2);\n"
        "    if let Ok(reply) = queue.recv_timeout(Duration::from_millis(5)) { handle(reply); }\n"
        "    match queue.recv_timeout(Duration::from_millis(10)) { _ => {} }\n"
        "    let held = compared.recv_timeout(Duration::from_millis(250)).is_err();\n"
        "    queue.recv_timeout(PATIENCE).unwrap();\n"
        "    let _: Vec<Duration> = Vec::new();\n"
        "}\n"
    ))
    assert checker.check(tmp_path) == 0


def test_a_matched_wait_that_fails_on_timeout_is_a_budget_under_the_hang_floor(tmp_path):
    # Review 2026-10-24 TST-23: the DXF fuzz test failed the PR suite on a loaded machine
    # through `match finished.recv_timeout(PER_INPUT) { … Timeout => panic!(…) }` with
    # 5 s per input: a `match` was taken for polling and 5 s met the patience floor.
    write(tmp_path, "crates/a/tests/fuzz.rs", (
        "const PER_INPUT: Duration = Duration::from_secs(5);\n"
        "const HUNG: Duration = Duration::from_secs(300);\n"
        "fn fuzz() {\n"
        "    match finished.recv_timeout(PER_INPUT) {\n"
        "        Ok(()) => {}\n"
        "        Err(mpsc::RecvTimeoutError::Timeout) => {\n"
        "            panic!(\"ran longer than {PER_INPUT:?}\");\n"
        "        }\n"
        "    }\n"
        "    match finished.recv_timeout(HUNG) {\n"
        "        Err(RecvTimeoutError::Timeout) => panic!(\"hung\"),\n"
        "        _ => {}\n"
        "    }\n"
        "    match finished.recv_timeout(PER_INPUT) {\n"
        "        Err(RecvTimeoutError::Timeout) => {}\n"
        "        _ => {}\n"
        "    }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == ["crates/a/tests/fuzz.rs:4"]


def test_the_viewer_crates_are_checked_too(tmp_path):
    # Review 2026-10-21 P3: only crates/ was scanned.
    write(tmp_path, "viewer/web/tests/smoke.rs", (
        "fn timed() {\n"
        "    let started = Instant::now();\n"
        "    assert!(started.elapsed() < Duration::from_secs(1));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == ["viewer/web/tests/smoke.rs:3"]


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
    assert checker.violations(tmp_path) == ["crates/a/src/lib.rs:4"]


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
