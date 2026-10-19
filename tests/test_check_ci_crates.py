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


def test_only_crates_named_with_p_count_as_checked():
    named = checker.ci_crates([WORKFLOW])
    assert named == {"alpha", "beta", "gamma", "epsilon", "zeta"}
    assert checker.unchecked_crates(["alpha", "delta"], named, set(), set()) == [
        "delta: no CI job names it with -p; add it to a job's crate list"
    ]


def test_a_crate_with_tests_needs_a_job_that_tests_it():
    tested = checker.tested_crates([WORKFLOW])
    assert tested == {"epsilon", "zeta"}
    named = checker.ci_crates([WORKFLOW])
    assert checker.unchecked_crates(["alpha", "beta", "epsilon"], named, tested, {"alpha", "epsilon"}) == [
        "alpha: has tests, but no CI job runs cargo test on it; add it to a test job"
    ]


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
    assert checker.ci_crates([COMMENTED]) == {"alpha", "epsilon"}
    assert checker.tested_crates([COMMENTED]) == {"alpha", "epsilon"}


def test_every_workspace_crate_is_checked_in_ci():
    crates = checker.workspace_crates(ROOT)
    assert "ketchup-headless" in crates
    assert "ketchup-viewer" in checker.crates_with_tests(ROOT)
    workflows = [path.read_text(encoding="utf-8") for path in (ROOT / ".github/workflows").glob("*.yml")]
    problems = checker.unchecked_crates(
        crates, checker.ci_crates(workflows), checker.tested_crates(workflows), checker.crates_with_tests(ROOT)
    )
    assert problems == []
