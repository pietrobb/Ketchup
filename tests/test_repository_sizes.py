"""Large files stay out of the Git tree (review REPO-1, structural point 9).

Sample documents and artwork that no build or test reads are attached to the
`sample-files` GitHub release instead. A tracked file above `LIMIT_BYTES` must be
one that tests or builds read at every checkout, listed below with its size,
and it may only shrink. Program reports are computed by the tests, so only
their small digests are tracked."""

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIMIT_BYTES = 1024 * 1024

# Path -> largest allowed size. What reads it, so it cannot leave the tree:
NEEDED_LARGE_FILES = {
    # viewer_export_house, viewer_release_smoke, large_document_history
    "examples/house-project.ketchup": 9_408_002,
    # performance_ai_house
    "examples/wardrobe-chair-table.ketchup": 2_797_689,
    # scripts/macos/build-viewer-app.sh cuts the Viewer icon from it (ADR 0003)
    "pic/splash.png": 1_907_517,
    # fast_assembly fixtures of ketchup-application
    "crates/ketchup-application/tests/fixtures/fast_assembly/nightstand_v10_interrupted.ketchup": 1_894_287,
    "crates/ketchup-application/tests/fixtures/fast_assembly/nightstand_v9_retention.ketchup": 1_341_247,
    # production_performance
    "examples/hettich-quadro-v6-drawer.ketchup": 1_191_576,
}


def tracked_files():
    listed = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout.decode("utf-8")
    return [name for name in listed.split("\0") if name and (ROOT / name).is_file()]


def test_only_needed_files_are_larger_than_the_limit():
    oversized = {}
    for name in tracked_files():
        size = (ROOT / name).stat().st_size
        if size > NEEDED_LARGE_FILES.get(name, LIMIT_BYTES):
            oversized[name] = size
    assert oversized == {}, "attach large samples to the sample-files release instead"


def test_the_list_of_needed_large_files_names_only_tracked_files():
    tracked = set(tracked_files())
    assert sorted(set(NEEDED_LARGE_FILES) - tracked) == []


def test_no_generated_program_report_is_tracked():
    reports = [name for name in tracked_files() if name.endswith(".report.json")]
    assert reports == [], "example_programs computes the reports; commit only <name>.digest.json"
