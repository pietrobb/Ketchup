import importlib.util
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("changes_reach_the_house", ROOT / "scripts" / "changes_reach_the_house.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def test_a_change_of_the_library_the_examples_or_a_house_test_reaches_the_house(tmp_path):
    # Review 2026-10-24 CI-23: a pull request that changed prelude.star ran none of them.
    house_test = tmp_path / "crates/b/tests/house.rs"
    house_test.parent.mkdir(parents=True)
    house_test.write_text('const HOUSE: &str = include_str!("../../examples/programs/tiny-house.star");\n')
    (tmp_path / "crates/b/tests/other.rs").write_text("fn other() {}\n")
    for path in ("crates/ketchup-program/library/prelude.star", "crates/ketchup-program/src/loads.rs",
                 "examples/programs/tiny-house.star", "crates/b/tests/house.rs"):
        assert checker.reaches_the_house(["README.md", path], tmp_path), path
    assert not checker.reaches_the_house(["README.md", "crates/b/tests/other.rs", "crates/b/tests/gone.rs"], tmp_path)


def test_the_changed_files_are_those_of_the_last_commit(tmp_path):
    def git(*arguments):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *arguments], cwd=tmp_path, check=True,
                       capture_output=True)
    git("init", "-q")
    (tmp_path / "a.txt").write_text("a")
    git("add", "a.txt")
    git("commit", "-q", "-m", "a")
    # A single commit has no parent: git cannot tell, so the tests run.
    assert checker.changed_files(tmp_path) is None
    (tmp_path / "b.txt").write_text("b")
    git("add", "b.txt")
    git("commit", "-q", "-m", "b")
    assert checker.changed_files(tmp_path) == ["b.txt"]


def test_both_large_house_steps_also_run_when_the_change_reaches_the_house():
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    steps = workflow.split("- name: large-house tests")[1:]
    assert len(steps) == 2
    for step in steps:
        condition = step.split("\n        if:", 1)[1].split("\n", 1)[0]
        assert "steps.house.outputs.run == 'true'" in condition, condition
    assert workflow.count("scripts/changes_reach_the_house.py") == 2
    assert workflow.count("fetch-depth: 2") == 2
