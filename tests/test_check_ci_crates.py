import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_ci_crates", ROOT / "scripts" / "check_ci_crates.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

WORKFLOW = """name: CI
jobs:
  linted:
    env:
      CRATES: >-
        -p alpha -p beta
    steps:
      - run: cargo clippy $CRATES
      - run: cargo build -p gamma --bins
  tested:
    env:
      CRATES: >-
        -p epsilon
    steps:
      - run: cargo test --locked $CRATES
      - run: cargo test --locked -p zeta
# delta is mentioned
"""


def test_only_crates_cargo_clippy_names_count_as_linted():
    linted = checker.linted_crates([WORKFLOW])
    assert linted == {"alpha", "beta"}
    assert checker.unchecked_crates(["alpha", "gamma", "delta"], linted, set(), set()) == [
        "gamma: no CI job runs cargo clippy on it; add it to a job's crate list",
        "delta: no CI job runs cargo clippy on it; add it to a job's crate list",
    ]


def test_a_crate_with_tests_needs_a_job_that_tests_it():
    tested = checker.tested_crates([WORKFLOW])
    assert tested == {"epsilon", "zeta"}
    linted = checker.linted_crates([WORKFLOW])
    assert checker.unchecked_crates(["alpha", "beta"], linted, tested, {"alpha"}) == [
        "alpha: has tests, but no CI job runs cargo test on it; add it to a test job"
    ]


DISABLED = """name: CI
jobs:
  switched-off:
    runs-on: ubuntu-latest
    if: false
    steps:
      - run: cargo clippy -p alpha
      - run: cargo test -p alpha
  running:
    runs-on: ubuntu-latest
    if: always()
    steps:
      - name: Lint beta
        if: ${{ false }}
        run: cargo clippy -p beta
      - if: false
        run: cargo test -p beta
      - run: cargo tree -p gamma -i js-sys | grep -q sys-locale
      - run: cargo test -p delta -- -p eta
      - run: cargo build -p theta && cargo clippy -p iota
      - run: |
          cargo clippy --locked \\
            -p kappa -- -D warnings
          cargo test --locked `
            -p lambda
"""


def test_build_tree_test_arguments_and_steps_that_never_run_do_not_count():
    assert checker.linted_crates([DISABLED]) == {"iota", "kappa"}
    # `-p eta` after `--` names no crate; `eta` there is a test filter, so delta counts neither.
    assert checker.tested_crates([DISABLED]) == {"lambda"}


COMMENTED = """name: CI
jobs:
  tested:
    env:
      CRATES: >-
        -p alpha
    steps:
      # - run: cargo test -p beta
      - run: cargo test --locked $CRATES  # later also -p gamma
      - run: |
          # cargo clippy -p delta
          cargo test --locked -p epsilon
"""


def test_a_crate_named_only_in_a_comment_is_not_checked():
    assert checker.linted_crates([COMMENTED]) == set()
    assert checker.tested_crates([COMMENTED]) == {"alpha", "epsilon"}


PULL_REQUEST_ONLY_FAILS = """name: CI
jobs:
  nightly:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'
    steps:
      - run: cargo test -p alpha
  tolerant:
    runs-on: ubuntu-latest
    continue-on-error: true
    steps:
      - run: cargo clippy -p beta
      - run: cargo test -p beta
  per-step:
    runs-on: ubuntu-latest
    env:
      NIGHTLY: ${{ github.event_name == 'schedule' }}
    steps:
      - if: env.NIGHTLY == 'true'
        run: cargo test -p gamma
      - if: runner.os == 'Linux' && github.event_name == 'schedule'
        run: cargo test -p delta
      - continue-on-error: true
        run: cargo test -p epsilon
      - run: cargo test -p zeta -- --exact zeta::one
      - run: cargo test -p eta -- --ignored
      - run: >-
          cargo test -p theta --
          only_these
      - run: cargo test -p iota --no-fail-fast -- --skip slow --test-threads 2
      - if: github.event_name != 'schedule'
        continue-on-error: false
        run: cargo clippy -p kappa && cargo test -p kappa
"""


def test_a_run_that_cannot_fail_a_pull_request_or_runs_some_tests_only_does_not_count():
    # Review 2026-10-24 CI-20: nightly only, allowed to fail, or narrowed by a filter.
    assert checker.tested_crates([PULL_REQUEST_ONLY_FAILS]) == {"iota", "kappa"}
    assert checker.linted_crates([PULL_REQUEST_ONLY_FAILS]) == {"kappa"}


def test_every_workspace_crate_is_checked_in_ci():
    crates = checker.workspace_crates(ROOT)
    assert "ketchup-headless" in crates
    assert "ketchup-viewer" in checker.crates_with_tests(ROOT)
    workflows = [path.read_text(encoding="utf-8") for path in (ROOT / ".github/workflows").glob("*.yml")]
    problems = checker.unchecked_crates(
        crates, checker.linted_crates(workflows), checker.tested_crates(workflows), checker.crates_with_tests(ROOT)
    )
    assert problems == []
