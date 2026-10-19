"""The fuzz job covers every reader of untrusted bytes and builds on what earlier runs found.

Review 2026-10-17 §3 P3: two targets only, a corpus thrown away after every run, and a
build that may rewrite fuzz/Cargo.lock.
"""

import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FUZZ = ROOT / "fuzz"
READER = re.compile(r"^pub fn ((?:inspect|parse)_\w+)\(\s*source: &\[u8\]", re.MULTILINE)


def targets():
    return [binary["name"] for binary in tomllib.loads((FUZZ / "Cargo.toml").read_text(encoding="utf-8"))["bin"]]


def fuzz_job():
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    return workflow.split("\n  fuzz:\n", 1)[1].split("\n  rust-required:\n", 1)[0]


def test_every_import_reader_has_a_fuzz_target():
    sources = [ROOT / "crates/ketchup-model/src/import.rs", *(ROOT / "crates/ketchup-model/src/import").glob("*.rs")]
    readers = {name for path in sources for name in READER.findall(path.read_text(encoding="utf-8"))}
    assert {"inspect_dxf", "parse_stl", "inspect_glb", "inspect_sketchup_scene"} <= readers
    fuzzed = "".join(path.read_text(encoding="utf-8") for path in (FUZZ / "fuzz_targets").glob("*.rs"))
    assert sorted(reader for reader in readers if f"::{reader}(" not in fuzzed) == []


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
