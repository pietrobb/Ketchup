import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_ci_crates", ROOT / "scripts" / "check_ci_crates.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def test_only_crates_named_with_p_count_as_checked():
    workflow = "CRATES: >-\n  -p alpha -p beta\nrun: cargo build -p gamma --bins\n# delta is mentioned\n"
    named = checker.ci_crates([workflow])
    assert named == {"alpha", "beta", "gamma"}
    assert checker.unchecked_crates(["alpha", "delta"], named) == [
        "delta: no CI job names it with -p; add it to a job's crate list"
    ]


def test_every_workspace_crate_is_checked_in_ci():
    crates = checker.workspace_crates(ROOT)
    assert "ketchup-headless" in crates
    workflows = [path.read_text(encoding="utf-8") for path in (ROOT / ".github/workflows").glob("*.yml")]
    assert checker.unchecked_crates(crates, checker.ci_crates(workflows)) == []
