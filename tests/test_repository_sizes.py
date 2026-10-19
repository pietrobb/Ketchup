"""Large files stay out of the Git tree (review REPO-1, structural point 9).

Sample documents and artwork that no build or test reads are attached to the
`sample-files` GitHub release instead. A tracked file above `LIMIT_BYTES` must be
one that tests or builds read at every checkout, listed below with its size,
and it may only shrink. Program reports are computed by the tests, so only
their condensed <name>.expected.json files are tracked."""

import re
import subprocess
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
LIMIT_BYTES = 1024 * 1024

# Path -> largest allowed size. What reads it, so it cannot leave the tree:
NEEDED_LARGE_FILES = {
    # viewer_export_house, large_document_history
    "examples/house-project.ketchup": 6_162_575,
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
    assert reports == [], "example_programs computes the reports; commit only <name>.expected.json"


def test_external_run_evidence_is_released_not_tracked():
    evidence = [
        name
        for name in tracked_files()
        if name.endswith((".validation.json", ".blender-reference.json"))
    ]
    assert evidence == [], "attach run evidence to the sample-files release instead"


# Release-hosted files that code still names. Only opt-in tests (#[ignore] or
# skipTest) may read them, because a clone does not have them.
RELEASED_READ_BY_OPT_IN_TESTS = {
    "examples/garden-studio-exact.blender-reference.json",
    "examples/garden-studio-exact.validation.json",
}
OPT_IN_MARKERS = ("#[ignore", "skipTest")
# A literal relative to the repository (optionally after ../ steps), or a bare
# examples/ path joined to a parent directory of the reading file.
EXAMPLE_PATH = re.compile(r"(?:\.\./)*((?:[A-Za-z0-9_\-]+/)*examples/[A-Za-z0-9_.\-/]+\.[A-Za-z0-9]+)")


def resolve(literal, reader, tracked):
    bases = [str(parent) for parent in PurePosixPath(reader).parents if str(parent) != "."]
    for base in ["", *bases]:
        path = f"{base}/{literal}" if base else literal
        if path in tracked:
            return path
    return None


def test_every_example_file_that_code_reads_is_in_a_clone():
    tracked = set(tracked_files())
    missing = {}
    for name in tracked:
        if not name.endswith((".rs", ".py")) or not name.startswith(("crates/", "sdk/", "tests/", "scripts/")):
            continue
        text = (ROOT / name).read_text(encoding="utf-8", errors="replace")
        for literal in set(EXAMPLE_PATH.findall(text)):
            path = resolve(literal, name, tracked)
            if path:
                head = (ROOT / path).read_bytes()[:40]
                assert not head.startswith(b"version https://git-lfs"), f"{path} is an LFS pointer"
            elif literal in RELEASED_READ_BY_OPT_IN_TESTS and any(m in text for m in OPT_IN_MARKERS):
                continue
            else:
                missing.setdefault(literal, []).append(name)
    assert missing == {}, "a test or build reads these, so they must stay tracked"
