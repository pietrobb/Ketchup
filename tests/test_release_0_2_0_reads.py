import importlib.util
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("release_0_2_0_reads", ROOT / "scripts" / "release_0_2_0_reads.py")
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
    problems = checker.variant_problems(old, new)
    assert [problem.split(": ")[0] for problem in problems] == ["ImportLengthUnit::Furlong", "Shape::Line"]
    assert checker.variant_problems(old, {"ImportLengthUnit": old["ImportLengthUnit"]}) == [
        "Shape: 0.2.0 has this enum, HEAD no longer does"
    ]


def test_the_reader_is_named_by_the_script_and_built_outside_the_workspace():
    workspace = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    assert "release_0_2_0_reader" not in workspace
    script = (ROOT / "scripts" / "release_0_2_0_reads.py").read_text(encoding="utf-8")
    assert "release_0_2_0_reader.rs" in script
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    assert re.search(r"git fetch .*tag v0\.2\.0", ci) and "scripts/release_0_2_0_reads.py" in ci


def test_every_job_that_tests_its_crates_keeps_their_saved_files_for_kecup_0_2_0():
    # Review 2026-10-29 proposal 5.4: the application tests save documents too.
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    runs = re.findall(r"cargo test --locked \$(?:env:)?CRATES[^\n]*", ci)
    tests = [run for run in runs if "--ignored" not in run]
    assert len(tests) == 2
    assert all("--features ketchup-model/saved-files" in run for run in tests)
    assert ci.count("scripts/release_0_2_0_reads.py --files") == len(tests)
