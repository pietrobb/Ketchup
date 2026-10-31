import importlib.util
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("changes_reach_the_house", ROOT / "scripts" / "changes_reach_the_house.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def workspace(root):
    """A workspace whose evaluator builds on a model crate and a patched starlark."""
    files = {
        "Cargo.toml": '[workspace]\nmembers = []\n\n[patch.crates-io]\nstarlark = { path = "third_party/starlark" }\n'
                      'other = { path = "third_party/other" }\n\n[profile.dev]\n',
        "crates/ketchup-program/Cargo.toml": '[dependencies]\nketchup-model = { path = "../ketchup-model" }\n'
                                             'starlark = "=0.13.0"\n',
        "crates/ketchup-model/Cargo.toml": '[dependencies]\nketchup-model = { path = ".", features = ["t"] }\n'
                                           'ketchup-geometry = { path = "../ketchup-geometry" }\n',
        "crates/ketchup-geometry/Cargo.toml": '[dependencies]\nketchup-model = { path = "../ketchup-model" }\n',
        "third_party/starlark/Cargo.toml": "[dependencies]\n",
    }
    for path, text in files.items():
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text(text)


def test_a_change_of_the_library_the_examples_or_a_house_test_reaches_the_house(tmp_path):
    # Review 2026-10-24 CI-23: a pull request that changed prelude.star ran none of them.
    workspace(tmp_path)
    house_test = tmp_path / "crates/b/tests/house.rs"
    house_test.parent.mkdir(parents=True)
    house_test.write_text('const HOUSE: &str = include_str!("../../examples/programs/tiny-house.star");\n')
    (tmp_path / "crates/b/tests/other.rs").write_text("fn other() {}\n")
    for path in ("crates/ketchup-program/library/prelude.star", "crates/ketchup-program/src/loads.rs",
                 "examples/programs/tiny-house.star", "crates/b/tests/house.rs"):
        assert checker.reaches_the_house(["README.md", path], tmp_path), path
    assert not checker.reaches_the_house(["README.md", "crates/b/tests/other.rs", "crates/b/tests/gone.rs"], tmp_path)


def test_a_change_of_what_the_evaluator_builds_on_reaches_the_house(tmp_path):
    # Review 2026-10-26 CI-43: ketchup-model, Cargo.lock or the toolchain did not count.
    workspace(tmp_path)
    assert checker.evaluator_directories(tmp_path) == [
        "crates/ketchup-geometry", "crates/ketchup-model", "crates/ketchup-program", "third_party/starlark"]
    for path in ("crates/ketchup-model/src/lib.rs", "crates/ketchup-geometry/src/lib.rs",
                 "third_party/starlark/src/eval.rs", "Cargo.lock", "Cargo.toml", "rust-toolchain.toml"):
        assert checker.reaches_the_house([path], tmp_path), path
    for path in ("third_party/other/src/lib.rs", "crates/ketchup-app/src/main.rs", "crates/Cargo.lock"):
        assert not checker.reaches_the_house([path], tmp_path), path
    assert checker.evaluator_directories()[-1] == "third_party/starlark"


def test_a_push_always_reaches_the_house(tmp_path, monkeypatch):
    # Review 2026-10-26 CI-43: the checkout of a push of 35 commits showed git only the last.
    workspace(tmp_path)
    monkeypatch.setattr(checker, "changed_files", lambda root: ["README.md"])
    assert checker.runs("push", tmp_path)
    assert not checker.runs("pull_request", tmp_path)
    monkeypatch.setattr(checker, "changed_files", lambda root: None)
    assert checker.runs("pull_request", tmp_path)


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
