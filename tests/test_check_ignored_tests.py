import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_ignored_tests", ROOT / "scripts" / "check_ignored_tests.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

WORKFLOW = """name: CI
jobs:
  nightly:
    env:
      CRATES: >-
        -p alpha
    steps:
      - name: perf
        run: >-
          cargo test --locked --release $CRATES --no-fail-fast --
          --ignored perf_
          --skip perf_needs_a_document
      - name: houses
        run: cargo test --locked --release -p beta -- --ignored houses:: large_house
      # - run: cargo test -p alpha -- --ignored forgotten
      - name: switched off
        if: false
        run: cargo test -p alpha -- --ignored forgotten
"""

SOURCE = """
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_frame() {}

#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_needs_a_document() {}

#[test]
#[ignore = "measurement: run on demand"]
fn forgotten() {}

#[test]
#[ignore = "run in a child process by the next test"]
fn child() {}

#[test]
fn starts_the_child() { test_process("frames::tests::child"); }

#[test]
#[ignore = "flaky under load, its subprocess times out"]
fn only_mentions_a_subprocess() {}

#[test]
#[ignore = "perf: release timing budget, hardware GPU"]
fn perf_on_a_real_gpu() {}
"""


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_an_ignored_test_no_step_runs_is_reported(tmp_path):
    write(tmp_path, "crates/alpha/Cargo.toml", '[package]\nname = "alpha"\n')
    write(tmp_path, "crates/alpha/src/lib.rs", "")
    write(tmp_path, "crates/alpha/src/frames/tests.rs", SOURCE)
    write(tmp_path, "crates/beta/Cargo.toml", '[package]\nname = "beta"\n')
    write(tmp_path, "crates/beta/tests/houses.rs", '#[test]\n#[ignore = "release-only"]\nfn whole() {}\n')
    write(tmp_path, "crates/beta/tests/other.rs", '#[test]\n#[ignore = "release-only"]\nfn whole() {}\n')
    tests = checker.ignored_tests(tmp_path)
    assert ("alpha", "frames::tests::perf_frame", "perf: release timing budget, runs nightly") in tests
    assert ("beta", "houses::whole", "release-only") in tests
    started = checker.started_by_tests(tmp_path)
    assert ("alpha", "child") in started
    runs = checker.ignored_runs([checker.without_comments(WORKFLOW)])
    assert checker.unrun_tests(tests, runs, started) == [
        "alpha frames::tests::perf_needs_a_document: ignored, but no CI step runs it; add it to a nightly step or delete it",
        "alpha frames::tests::forgotten: ignored, but no CI step runs it; add it to a nightly step or delete it",
        "alpha frames::tests::only_mentions_a_subprocess: ignored as a child process, but no test of alpha names it "
        "to start it; start it from a test or run it in a CI step",
        "beta other::whole: ignored, but no CI step runs it; add it to a nightly step or delete it",
    ]
    # The same child, when no test of its crate starts it, is reported too.
    assert "alpha frames::tests::child" in " ".join(checker.unrun_tests(tests, runs))


def test_every_ignored_test_of_the_repository_runs_in_ci():
    assert checker.main() == 0
