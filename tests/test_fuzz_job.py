"""The fuzz job covers every reader of untrusted bytes and builds on what earlier runs found.

Review 2026-10-17 §3 P3: two targets only, a corpus thrown away after every run, and a
build that may rewrite fuzz/Cargo.lock.
"""

import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FUZZ = ROOT / "fuzz"
READER = re.compile(r"^pub fn ((?:inspect|parse|read)_\w+)\(\s*source: &\[u8\]", re.MULTILINE)


def targets():
    return [binary["name"] for binary in tomllib.loads((FUZZ / "Cargo.toml").read_text(encoding="utf-8"))["bin"]]


def fuzz_job():
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    return workflow.split("\n  fuzz:\n", 1)[1].split("\n  rust-required:\n", 1)[0]


def test_every_import_reader_has_a_fuzz_target():
    # Review 2026-10-21 P3: the DXF underlay reader, in import/dxf/, was not fuzzed.
    sources = [ROOT / "crates/ketchup-model/src/import.rs", *(ROOT / "crates/ketchup-model/src/import").rglob("*.rs")]
    readers = {name for path in sources for name in READER.findall(path.read_text(encoding="utf-8"))}
    assert {"inspect_dxf", "read_dxf_underlay", "parse_stl", "inspect_glb", "inspect_sketchup_scene"} <= readers
    fuzzed = "".join(path.read_text(encoding="utf-8") for path in (FUZZ / "fuzz_targets").glob("*.rs"))
    assert sorted(reader for reader in readers if f"::{reader}(" not in fuzzed) == []


def test_dwg_files_and_comments_read_without_their_package_are_fuzzed():
    # Review 2026-10-19 P3: neither the DWG reader nor Comments::read_unbound had a target.
    fuzzed = "".join(path.read_text(encoding="utf-8") for path in (FUZZ / "fuzz_targets").glob("*.rs"))
    assert "ketchup_dwg::dwg_to_dxf(" in fuzzed
    assert "Comments::read_unbound(" in fuzzed


def test_a_dwg_section_size_past_the_malloc_limit_is_refused_not_a_crash():
    # bg108: acadrust allocates a section of the size the header declares; the worker refuses
    # such a file at its memory limit, the dwg target at the fuzzer's malloc limit.
    target = (FUZZ / "fuzz_targets/dwg.rs").read_text(encoding="utf-8")
    assert "#[global_allocator]" in target
    assert "const MALLOC_LIMIT: usize = 2048 << 20;" in target
    assert "std::alloc::set_alloc_error_hook(" in target
    job = fuzz_job()
    assert "-malloc_limit_mb=2048" in job.split("LIMITS:", 1)[1].split("\n", 1)[0]
    assert (FUZZ / "seeds/dwg/section-size-past-the-malloc-limit.dwg").is_file()


def test_a_found_crash_is_uploaded():
    steps = fuzz_job().split("\n      - ")
    upload = [step for step in steps if "actions/upload-artifact@" in step]
    assert len(upload) == 1 and "path: fuzz/artifacts" in upload[0] and "if: failure()" in upload[0]


def test_only_nightly_and_manual_runs_look_for_new_inputs():
    # Review 2026-10-19 P3: a crash new inputs find in code a pull request did not touch
    # blocked that pull request. Pushes and pull requests replay the seeds and kept corpus.
    job = fuzz_job()
    assert "EXPLORE: ${{ github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' }}" in job
    assert "-runs=0" in job and "-max_total_time=" in job
    assert "-max_total_time=" not in job.split("LIMITS:", 1)[1].split("\n", 1)[0]


def test_every_target_has_seeds_and_is_run():
    job = fuzz_job()
    for target in targets():
        seeds = FUZZ / "seeds" / target
        assert seeds.is_dir() and any(seeds.iterdir()) or f"fuzz/seeds/{target}/" in job, target
    assert "fuzz list" in job, "the job runs every target cargo fuzz lists"


def test_the_corpus_is_kept_between_runs():
    steps = fuzz_job().split("\n      - ")
    restore = [step for step in steps if "actions/cache/restore@" in step]
    save = [step for step in steps if "actions/cache/save@" in step]
    assert len(restore) == 1 and "path: fuzz/corpus" in restore[0]
    assert len(save) == 1 and "path: fuzz/corpus" in save[0]
    assert "if: always()" in save[0], "a run that finds a crash keeps its corpus too"


def test_the_build_uses_the_committed_lock_file():
    job = fuzz_job()
    fetch = job.find("cargo fetch --locked --manifest-path fuzz/Cargo.toml")
    assert 0 <= fetch < job.find("fuzz build")
