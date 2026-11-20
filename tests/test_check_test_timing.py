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


def test_a_time_measured_by_subtraction_or_duration_since_is_a_budget(tmp_path):
    # Review 2026-10-31 TST-80: `Instant::now() - start` and `.duration_since(start)`
    # measure as `.elapsed()` does and passed.
    write(tmp_path, "viewer/android/src/lib.rs", (
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn timed() {\n"
        "        let start = Instant::now();\n"
        "        let spent = Instant::now() - start;\n"
        "        assert!(spent < limit);\n"
        "        assert!(Instant::now().duration_since(start) < limit);\n"
        "        assert!(later.duration_since(start) <= limit);\n"
        "        assert!(std::time::Instant::now() - start < limit);\n"
        "        // A deadline that only stops a wait is still not a budget.\n"
        "        while Instant::now() < deadline {}\n"
        "    }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"viewer/android/src/lib.rs:{line}" for line in (6, 7, 8, 9)
    ]


def test_a_time_read_from_the_system_clock_or_by_checked_sub_is_a_budget(tmp_path):
    # Review 2026-11-01 TST-100: `SystemTime` as the clock, and the checked or
    # saturating subtraction of instants in place of `-`, measured unseen.
    write(tmp_path, "crates/a/src/lib.rs", (
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn timed() {\n"
        "        let start = SystemTime::now();\n"
        "        assert!(SystemTime::now().duration_since(start).unwrap() < limit);\n"
        "        assert!(later.checked_duration_since(start).unwrap() < limit);\n"
        "        assert!(Instant::now().checked_duration_since(start).unwrap() < limit);\n"
        "        assert!(end.saturating_duration_since(start) <= limit);\n"
        "    }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/src/lib.rs:{line}" for line in (5, 6, 7, 8)
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


def test_a_helper_that_fails_when_its_time_runs_out_is_a_budget_under_the_hang_floor(tmp_path):
    # Review 2026-11-04 P3: `connect_ai_e2e.rs` held a connect to 10 s through
    # `answer_within(…, CONNECT_WITHIN)`, which asserts that its poll finished in time.
    write(tmp_path, "crates/a/tests/e2e.rs", (
        "const CONNECT_WITHIN: Duration = Duration::from_secs(10);\n"
        "const HUNG: Duration = Duration::from_secs(300);\n"
        "impl Client {\n"
        "    fn answer_within(\n"
        "        &mut self,\n"
        "        id: u64,\n"
        "        within: Duration,\n"
        "    ) -> Value {\n"
        "        let answered = poll_until(within, || self.done(id));\n"
        "        assert!(answered, \"no answer within {within:?}\");\n"
        "    }\n"
        "}\n"
        "fn waits_for(done: &Receiver<()>, within: std::time::Duration) -> bool {\n"
        "    done.recv_timeout(within).is_ok()\n"
        "}\n"
        "fn connect() {\n"
        "    let taken = second.answer_within(connect, CONNECT_WITHIN);\n"
        "    let late = second.answer_within(\n"
        "        connect,\n"
        "        Duration::from_secs(2),\n"
        "    );\n"
        "    let fine = second.answer_within(connect, HUNG);\n"
        "    let polled = waits_for(&done, CONNECT_WITHIN);\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/e2e.rs:17",
        "crates/a/tests/e2e.rs:18",
    ]


def test_a_failing_helper_of_another_file_or_with_a_number_of_milliseconds_is_found_too(tmp_path):
    # Review 2026-11-04 r19 P3: 7b1788e3 found only helpers of the same file whose
    # parameter is a `Duration`; a harness helper used from another test file, a time
    # given as a number of milliseconds, and a helper passing its time on to such a
    # helper held the work to a budget unseen.
    write(tmp_path, "crates/a/tests/harness/mod.rs", (
        "pub const QUICK: Duration = Duration::from_secs(3);\n"
        "impl Window {\n"
        "    pub fn shows_within(&mut self, within: Duration) {\n"
        "        let shown = poll_until(within, || self.shown());\n"
        "        assert!(shown);\n"
        "    }\n"
        "    pub fn reply_within_ms(&mut self, id: u64, timeout_ms: u64) -> Reply {\n"
        "        self.replies.recv_timeout(Duration::from_millis(timeout_ms)).expect(\"reply\")\n"
        "    }\n"
        "    pub fn opens_within(&mut self, path: &Path, within: Duration) {\n"
        "        self.open(path);\n"
        "        self.shows_within(within);\n"
        "    }\n"
        "}\n"
    ))
    write(tmp_path, "crates/a/tests/open.rs", (
        "fn opens() {\n"
        "    window.shows_within(QUICK);\n"
        "    window.reply_within_ms(7, 2_000);\n"
        "    window.opens_within(&path, Duration::from_secs(5));\n"
        "    window.shows_within(Duration::from_secs(300));\n"
        "    window.reply_within_ms(7, 120_000);\n"
        "}\n"
        "fn perf_opens_quickly() {\n"
        "    window.shows_within(QUICK);\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/open.rs:2",
        "crates/a/tests/open.rs:3",
        "crates/a/tests/open.rs:4",
    ]


def test_the_viewer_crates_are_checked_too(tmp_path):
    # Review 2026-10-21 P3: only crates/ was scanned.
    write(tmp_path, "viewer/web/tests/smoke.rs", (
        "fn timed() {\n"
        "    let started = Instant::now();\n"
        "    assert!(started.elapsed() < Duration::from_secs(1));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == ["viewer/web/tests/smoke.rs:3"]


def test_a_deadline_that_fails_the_test_is_a_budget_under_the_hang_floor(tmp_path):
    # Review 2026-11-08 TST-252: `live_bridge/tests.rs` waited for a prompt or a job's
    # start with `assert!(Instant::now() < deadline)` 2 s or 4 s away, which fails on a
    # loaded machine as a matched wait does; a deadline that only ends a poll passes.
    write(tmp_path, "crates/a/tests/poll.rs", (
        "const QUICK: Duration = Duration::from_secs(4);\n"
        "fn poll() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(2);\n"
        "    while !prompted() { assert!(Instant::now() < deadline); }\n"
        "    let start_deadline = std::time::Instant::now() + QUICK;\n"
        "    loop {\n"
        "        if started() { break; }\n"
        "        assert!(\n"
        "            Instant::now() < start_deadline,\n"
        "            \"job did not start\"\n"
        "        );\n"
        "    }\n"
        "    let late = Instant::now() + Duration::from_millis(2500);\n"
        "    while !done() {\n"
        "        if Instant::now() > late {\n"
        "            panic!(\"not done\");\n"
        "        }\n"
        "    }\n"
        "    let polled = Instant::now() + Duration::from_secs(2);\n"
        "    while Instant::now() < polled && !done() {}\n"
        "    let hung = Instant::now() + Duration::from_secs(120);\n"
        "    while !done() { assert!(Instant::now() < hung); }\n"
        "    let deadline = Instant::now() + Duration::from_secs(2);\n"
        "    while Instant::now() < deadline && !done() {}\n"
        "}\n"
        "fn other() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(90);\n"
        "    while !done() { assert!(Instant::now() < deadline); }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/poll.rs:3",
        "crates/a/tests/poll.rs:5",
        "crates/a/tests/poll.rs:13",
    ]


def test_a_deadline_that_fails_after_other_statements_or_either_way_round_counts(tmp_path):
    # Review 2026-11-09 R1109-TST-304: `live_bridge_bootstrap.rs` killed its child before
    # `panic!` 8 s on, and `deadline <= Instant::now()` was not read at all.
    write(tmp_path, "crates/a/tests/child.rs", (
        "fn hung() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(8);\n"
        "    while child.try_wait().unwrap().is_none() {\n"
        "        if Instant::now() >= deadline {\n"
        "            child.kill().unwrap();\n"
        "            panic!(\"hung\");\n"
        "        }\n"
        "    }\n"
        "    let late = Instant::now() + Duration::from_secs(3);\n"
        "    while !done() {\n"
        "        if late <= Instant::now() { unreachable!(\"late\") }\n"
        "    }\n"
        "    let polled = Instant::now() + Duration::from_secs(3);\n"
        "    while !done() {\n"
        "        if Instant::now() > polled {\n"
        "            break;\n"
        "        }\n"
        "    }\n"
        "    let patient = Instant::now() + Duration::from_secs(60);\n"
        "    while !done() {\n"
        "        if Instant::now() > patient { child.kill().unwrap(); panic!(\"hung\"); }\n"
        "    }\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/child.rs:2",
        "crates/a/tests/child.rs:9",
    ]


def test_a_short_poll_the_test_relies_on_counts_however_it_is_written(tmp_path):
    # Review 2026-11-10 R1110-TST-401: `return Err` past the deadline, `break false` and
    # `assert!(ok)`, `else { panic! }`, `checked_add`, `start + d`, `.lt(&deadline)` and a
    # short poll followed by an assert all passed (`consent.rs` and `transport.rs` 2 s,
    # `exact_modeling.rs` 10 s on an OCCT worker).
    write(tmp_path, "crates/a/tests/shapes.rs", (
        "fn waited() -> Result<(), String> {\n"
        "    let deadline = Instant::now() + Duration::from_secs(2);\n"
        "    while !done() { if Instant::now() > deadline { return Err(\"late\".into()); } }\n"
        "    Ok(())\n"
        "}\n"
        "fn shapes() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(2);\n"
        "    let ok = loop { if done() { break true; } if Instant::now() > deadline { break false; } };\n"
        "    assert!(ok);\n"
        "    let late = Instant::now().checked_add(Duration::from_secs(3)).unwrap();\n"
        "    if Instant::now().lt(&late) { wait(); } else { panic!(\"late\") }\n"
        "    let start = Instant::now();\n"
        "    let until = start + Duration::from_secs(10);\n"
        "    while results.len() != 1 && Instant::now() < until { refresh(); }\n"
        "    let package = results.get().expect(\"one exact body\");\n"
        "    // A deadline that the test expects to run out is the point of the test.\n"
        "    let deadline = Instant::now() + Duration::from_millis(50);\n"
        "    assert!(matches!(await_response(deadline), Awaited::TimedOut));\n"
        "    let deadline = Instant::now() + Duration::from_millis(200);\n"
        "    while Instant::now() < deadline {}\n"
        "    assert!(receiver.try_recv().is_err());\n"
        "    let patient = start + Duration::from_secs(60);\n"
        "    while !done() && Instant::now() < patient {}\n"
        "    assert!(done());\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/shapes.rs:{line}" for line in (2, 7, 10, 13)
    ]


def test_a_helper_whose_time_sets_a_deadline_it_asserts_is_a_budget_under_the_hang_floor(tmp_path):
    # Review 2026-11-12 R1112-TST-500: `live_bridge/tests.rs` held some 90 calls to 4 s
    # through `call_within(…, timeout)`, which sets `Instant::now() + timeout` and
    # asserts it has not passed; `step_until` held 23 calls to 10-30 s so.
    write(tmp_path, "crates/a/tests/harness/mod.rs", (
        "impl Wire {\n"
        "    fn call(&mut self, request: Request) -> Response {\n"
        "        self.call_within(request, Duration::from_secs(4))\n"
        "    }\n"
        "    fn call_within(&mut self, request: Request, timeout: Duration) -> Response {\n"
        "        let deadline = Instant::now() + timeout;\n"
        "        loop {\n"
        "            if let Ok(response) = self.rx.try_recv() { break response; }\n"
        "            assert!(Instant::now() < deadline);\n"
        "        }\n"
        "    }\n"
        "}\n"
        "pub fn step_until(harness: &mut Harness, within: Duration, what: &str) {\n"
        "    let deadline = Instant::now() + within;\n"
        "    while !harness.done() {\n"
        "        assert!(Instant::now() < deadline, \"{what}\");\n"
        "    }\n"
        "}\n"
        "pub fn settle(harness: &mut Harness, within: Duration) {\n"
        "    let deadline = Instant::now() + within;\n"
        "    while Instant::now() < deadline && !harness.done() { harness.step(); }\n"
        "}\n"
    ))
    write(tmp_path, "crates/a/tests/flow.rs", (
        "fn flow() {\n"
        "    step_until(&mut harness, Duration::from_secs(10), \"saved\");\n"
        "    step_until(&mut harness, Duration::from_secs(60), \"saved\");\n"
        "    wire.call_within(request, Duration::from_secs(120));\n"
        "    settle(&mut harness, Duration::from_secs(1));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/flow.rs:2",
        "crates/a/tests/harness/mod.rs:3",
    ]


def test_a_timeout_named_in_a_message_or_a_quiet_wait_is_read_for_what_it_checks(tmp_path):
    # Review 2026-11-12 P3: `TIMED_OUT` let pass any statement with "Timeout" in it,
    # also `assert!(…, "Timeout waiting…")` and `panic!("Timeout…")`, while a wait for
    # nothing to come, `assert_eq!(count, 0)` after it, counted as a budget.
    write(tmp_path, "crates/a/tests/quiet.rs", (
        "fn quiet() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(2);\n"
        "    while !done() { assert!(Instant::now() < deadline, \"Timeout waiting for done\"); }\n"
        "    let late = Instant::now() + Duration::from_secs(2);\n"
        "    while !done() { if Instant::now() > late { panic!(\"Timeout: not done\"); } }\n"
        "    let mut count = 0; let quiet = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < quiet { count += sent.try_iter().count(); }\n"
        "    assert_eq!(count, 0, \"nothing is sent\");\n"
        "    let mut events = Vec::new(); let empty = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < empty { events.extend(sent.try_iter()); }\n"
        "    assert!(events.is_empty());\n"
        "    let counted = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < counted { pump(); }\n"
        "    assert_eq!(count, 3);\n"
        "    let drained = Instant::now() + Duration::from_secs(2);\n"
        "    while Instant::now() < drained && pending() > 0 { pump(); }\n"
        "    assert_eq!(pending(), 0);\n"
        "    let timed = Instant::now() + Duration::from_millis(50);\n"
        "    assert!(matches!(wait(timed), Err(Timeout)));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/quiet.rs:{line}" for line in (2, 4, 12, 15)
    ]


def test_a_quiet_wait_checks_only_what_it_gathered_from_nothing(tmp_path):
    # Review 2026-11-13 P3: after a quiet wait any "nothing" passed, also a real budget:
    # the work left, or a queue the wait never fills, is empty within 300 ms.
    write(tmp_path, "crates/a/tests/settled.rs", (
        "fn settled() {\n"
        "    let mut count = 0; let quiet = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < quiet { count += sent.try_iter().count(); }\n"
        "    assert_eq!(count, 0);\n"
        "    let worked = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < worked { pump(); }\n"
        "    assert_eq!(pending(), 0, \"the work finished within 300 ms\");\n"
        "    let emptied = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < emptied { pump(); }\n"
        "    assert!(queue.is_empty());\n"
        "    let mut left = 3; let counted = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < counted { left -= done.try_iter().count(); }\n"
        "    assert_eq!(left, 0);\n"
        "    let mut seen = Vec::new(); let heard = Instant::now() + Duration::from_millis(300);\n"
        "    while Instant::now() < heard { seen.extend(events.try_iter()); }\n"
        "    assert!(seen.is_empty());\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/settled.rs:{line}" for line in (5, 8, 11)
    ]


def test_generic_helpers_closures_and_scaled_or_renamed_times_are_read_too(tmp_path):
    # Review 2026-11-13 P3: a helper with `->` in its generics, a closure, and a time
    # scaled (`STEP * 4`) or renamed (`let patience = …`, `const B: Duration = A`) were
    # not seen.
    write(tmp_path, "crates/a/tests/shapes.rs", (
        "const STEP: Duration = Duration::from_millis(250);\n"
        "const SAME_STEP: Duration = STEP;\n"
        "fn answer_within<F: Fn() -> bool>(ready: F, within: Duration) {\n"
        "    assert!(poll_until(within, ready));\n"
        "}\n"
        "fn shapes() {\n"
        "    answer_within(|| done(), Duration::from_secs(2));\n"
        "    let reply = |time: Duration| replies.recv_timeout(time).unwrap();\n"
        "    reply(Duration::from_millis(500));\n"
        "    let bare = |time| replies.recv_timeout(time).expect(\"reply\");\n"
        "    bare(STEP);\n"
        "    replies.recv_timeout(STEP * 4).unwrap();\n"
        "    replies.recv_timeout(Duration::from_secs(1) / 2).unwrap();\n"
        "    let patience = Duration::from_millis(800);\n"
        "    replies.recv_timeout(patience).unwrap();\n"
        "    replies.recv_timeout(SAME_STEP).unwrap();\n"
        "    let ends = Instant::now() + STEP.mul_f64(2.0);\n"
        "    assert!(Instant::now() < ends);\n"
        "    replies.recv_timeout(STEP * 400).unwrap();\n"
        "    let generous = Duration::from_secs(90);\n"
        "    reply(generous);\n"
        "    answer_within(|| done(), Duration::from_secs(120));\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/shapes.rs:{line}" for line in (7, 9, 11, 12, 13, 15, 16, 17)
    ]


def test_a_deadline_is_read_by_which_way_its_test_fails(tmp_path):
    # Review 2026-11-13 P3: `PANICS` counted a test that checks the wait timed out and
    # panics on anything else, and `TIMED_OUT` let pass `assert!(!timed_out)`, which
    # fails exactly when a slow machine runs out of time.
    write(tmp_path, "crates/a/tests/timeout.rs", (
        "fn timeout() {\n"
        "    let expected = Instant::now() + Duration::from_millis(100);\n"
        "    match wait(expected) { Err(Timeout) => {} other => panic!(\"{other:?}\") }\n"
        "    let finished = Instant::now() + Duration::from_millis(100);\n"
        "    let timed_out = poll_until(finished);\n"
        "    assert!(!timed_out);\n"
        "    let answered = Instant::now() + Duration::from_millis(100);\n"
        "    assert!(!wait(answered).is_err());\n"
        "    let flagged = Instant::now() + Duration::from_millis(100);\n"
        "    assert_eq!(poll_until(flagged).timed_out, false);\n"
        "    let late = Instant::now() + Duration::from_millis(100);\n"
        "    assert!(poll_until(late).timed_out);\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == [
        f"crates/a/tests/timeout.rs:{line}" for line in (4, 7, 9)
    ]


def test_a_wait_checked_to_run_out_by_taking_its_error_is_no_budget(tmp_path):
    # Review 2026-11-13 r95 P3: a test that takes the timeout's error with `unwrap_err`,
    # `expect_err` or let-else `Err`, and inspects it, was flagged as a budget.
    write(tmp_path, "crates/a/tests/ran_out.rs", (
        "fn ran_out() {\n"
        "    let late = Instant::now() + Duration::from_millis(100);\n"
        "    let error = wait(late).unwrap_err();\n"
        "    assert_eq!(error.kind(), ErrorKind::WouldBlock);\n"
        "    let gone = Instant::now() + Duration::from_millis(100);\n"
        "    let Err(error) = wait(gone) else { panic!(\"it answered\") };\n"
        "    assert!(error.to_string().contains(\"gone\"));\n"
        "    let over = Instant::now() + Duration::from_millis(100);\n"
        "    assert!(matches!(wait(over).expect_err(\"ran out\").kind(), ErrorKind::WouldBlock));\n"
        "    replies.recv_timeout(Duration::from_millis(250)).unwrap_err();\n"
        "    let Err(_) = replies.recv_timeout(Duration::from_millis(250)) else { panic!() };\n"
        "    let answered = Instant::now() + Duration::from_millis(100);\n"
        "    assert_eq!(wait(answered).unwrap(), 1);\n"
        "}\n"
    ))
    assert checker.violations(tmp_path) == ["crates/a/tests/ran_out.rs:12"]


def test_a_closure_is_a_helper_only_in_the_file_that_binds_it(tmp_path):
    # Review 2026-11-13 r95 P3: closures were counted by name in the whole repository.
    write(tmp_path, "crates/a/tests/bound.rs", (
        "fn bound() {\n"
        "    let reply = |time: Duration| replies.recv_timeout(time).unwrap();\n"
        "    reply(Duration::from_millis(500));\n"
        "}\n"
    ))
    write(tmp_path, "crates/a/tests/other.rs", (
        "fn reply(time: Duration) { sent.push(time); }\n"
        "fn other() { reply(Duration::from_millis(500)); }\n"
    ))
    assert checker.violations(tmp_path) == ["crates/a/tests/bound.rs:3"]


def test_deadlines_of_waits_and_computed_durations_pass(tmp_path):
    write(tmp_path, "crates/a/tests/wait.rs", (
        "fn wait() {\n"
        "    let deadline = Instant::now() + Duration::from_secs(60);\n"
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
