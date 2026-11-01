import importlib.util
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("release_reads", ROOT / "scripts" / "release_reads.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

SOURCE = """
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum Shape {
    /// A point.
    Point,
    Line { from: [f64; 2], to: [f64; 2] },
    Arc(Center, #[serde(default)] Option<f64>),
    Text { lines: Vec<(String, f64)>, style: Style { size: f64 } },
}

#[derive(Clone, Debug)]
pub enum NotSaved {
    Kept,
}
"""


def enums_of(source):
    return {
        match.group(2): checker.variants(checker.enum_body(source, match.end()))
        for match in checker.ENUM.finditer(source)
        if "Deserialize" in match.group(1)
    }


def test_the_variants_of_a_saved_enum_are_its_names_without_their_fields():
    assert enums_of(SOURCE) == {"Shape": {"Point", "Line", "Arc", "Text"}}


def test_a_new_variant_fails_until_reviewed_and_a_removed_one_always_fails():
    # Review 2026-10-29 MD-60: Decimetre was new since 0.2.0 and written as 103.
    old = {"ImportLengthUnit": {"Millimetre", "Metre"}, "Shape": {"Point", "Line"}}
    new = {"ImportLengthUnit": {"Millimetre", "Metre", "Decimetre", "Furlong"}, "Shape": {"Point"}}
    reviewed = {"ImportLengthUnit": {"Decimetre"}}
    problems = checker.variant_problems(old, new, reviewed)
    assert [problem.split(": ")[0] for problem in problems] == ["ImportLengthUnit::Furlong", "Shape::Line"]
    assert checker.variant_problems(old, {"ImportLengthUnit": old["ImportLengthUnit"]}, {}, "0.3.0") == [
        "Shape: 0.3.0 has this enum, HEAD no longer does"
    ]


def test_what_a_release_reads_is_a_product_of_integers():
    assert checker.product("64 * 1024 * 1024") == 64 * 1024 * 1024
    assert checker.product("1_000") == 1000


def test_the_reader_is_named_by_the_script_and_built_outside_the_workspace():
    workspace = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    assert "release_reader" not in workspace
    script = (ROOT / "scripts" / "release_reads.py").read_text(encoding="utf-8")
    assert "release_reader.rs" in script
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    fetches = re.findall(r"git fetch --no-tags --depth=1 origin ((?:tag v[\d.]+ ?)+)", ci)
    assert fetches
    # Review 2026-10-31 CI-80: CI fetches every release the script checks.
    for fetch in fetches:
        assert set(re.findall(r"v[\d.]+", fetch)) == set(checker.RELEASES)
    assert set(checker.REVIEWED) == set(checker.RELEASES)


def test_every_published_tag_is_a_release_the_script_checks():
    # Review 2026-10-31 CI-80: only 0.2.0 was checked when 0.3.0 had shipped.
    tags = subprocess.run(
        ["git", "tag", "--list", "v*"], cwd=ROOT, capture_output=True, text=True, check=False
    ).stdout.split()
    if not tags:
        pytest.skip("no tags in this clone")
    numbered = (re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", tag) for tag in tags)
    released = [m.group(0) for m in numbered if m and tuple(map(int, m.groups())) >= (0, 2, 0)]
    assert sorted(released) == sorted(checker.RELEASES)


def test_the_script_speaks_in_a_console_without_the_products_letters(tmp_path):
    # PR #8 CI: the Windows runner writes cp1252, which has no "č" in "Kečup".
    stubbed = f"""
import importlib.util, pathlib, sys, types
spec = importlib.util.spec_from_file_location("release_reads", {str(ROOT / "scripts" / "release_reads.py")!r})
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
checker.serde_enums = lambda tag: {{}}
checker.version = lambda tag: "0.3.0"
checker.what_it_reads = lambda tag: []
checker.old_workspace = lambda tag, target: pathlib.Path(".")
checker.subprocess.run = lambda *args, **kwargs: types.SimpleNamespace(returncode=0)
sys.exit(checker.main(["--tag", "v0.3.0", "--files", {str(tmp_path)!r}]))
"""
    result = subprocess.run(
        [sys.executable, "-c", stubbed],
        capture_output=True,
        env={**os.environ, "PYTHONIOENCODING": "cp1252", "PYTHONUTF8": "0"},
        check=False,
    )
    assert result.returncode == 0, result.stderr.decode("utf-8", "replace")
    assert "Kečup 0.3.0 opens the saved files" in result.stdout.decode("utf-8")


def test_every_job_that_tests_its_crates_keeps_their_saved_files_for_the_releases():
    # Review 2026-10-29 proposal 5.4: the application tests save documents too.
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    runs = re.findall(r"cargo test --locked \$(?:env:)?CRATES[^\n]*", ci)
    tests = [run for run in runs if "--ignored" not in run]
    # Crates without OCCT, and those with it on Windows and on Linux.
    assert len(tests) == 3
    assert all("--features ketchup-model/saved-files" in run for run in tests)
    assert ci.count("scripts/release_reads.py --files") == len(tests)
