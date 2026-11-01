"""The fuzz job covers every reader of untrusted bytes and builds on what earlier runs found.

Review 2026-10-17 §3 P3: two targets only, a corpus thrown away after every run, and a
build that may rewrite fuzz/Cargo.lock.
"""

import os
import re
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
FUZZ = ROOT / "fuzz"
READER = re.compile(r"^pub fn ((?:inspect|parse|read|write)_\w+)\(\s*source: &\[u8\]", re.MULTILINE)


def targets():
    return [binary["name"] for binary in tomllib.loads((FUZZ / "Cargo.toml").read_text(encoding="utf-8"))["bin"]]


def fuzz_job():
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    return workflow.split("\n  fuzz:\n", 1)[1].split("\n  rust-required:\n", 1)[0]


def test_every_import_reader_has_a_fuzz_target():
    # Review 2026-10-21 P3: the DXF underlay reader, in import/dxf/, was not fuzzed; review
    # 2026-10-29 TST-61: nor was write_edited_dxf, which parses the drawing's file again.
    sources = [ROOT / "crates/ketchup-model/src/import.rs", *(ROOT / "crates/ketchup-model/src/import").rglob("*.rs")]
    readers = {name for path in sources for name in READER.findall(path.read_text(encoding="utf-8"))}
    expected = {"inspect_dxf", "read_dxf_underlay", "parse_stl", "inspect_glb", "inspect_sketchup_scene", "write_edited_dxf"}
    assert expected <= readers
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


def every_target_step():
    step = next(step for step in fuzz_job().split("\n      - ") if step.startswith("name: every target"))
    assert "shell:" not in step, "the probe below runs the step the way GitHub does without shell:"
    return "\n".join(line.removeprefix("          ") for line in step.split("run: |\n", 1)[1].splitlines())


def test_every_fuzz_run_in_the_loop_records_its_failure():
    # Capability review 2026-10-23: under bash -e the first crashing target ended the step,
    # and no later target was fuzzed while that crash stayed open.
    body = every_target_step()
    runs = [line for line in body.splitlines() if " fuzz run " in line]
    assert runs and all(line.rstrip().endswith('|| failed+=("$target")') for line in runs)
    assert 'echo "::error::fuzz targets that crashed: ${failed[*]}"' in body and "exit 1" in body


@pytest.mark.skipif(os.name == "nt" or shutil.which("bash") is None, reason="needs a POSIX bash")
def test_one_crashing_target_does_not_hide_the_others(tmp_path):
    names = targets()
    crashing = names[len(names) // 2]
    cargo = tmp_path / "bin" / "cargo"
    cargo.parent.mkdir()
    cargo.write_text(
        "#!/usr/bin/env bash\n"
        f"case \"$*\" in\n  *'fuzz list'*) printf '%s\\n' {' '.join(names)} ;;\n"
        f"  *'fuzz run'*' {crashing} '*) exit 1 ;;\nesac\n",
        encoding="utf-8",
    )
    cargo.chmod(0o755)
    (tmp_path / "step.sh").write_text(every_target_step() + "\n", encoding="utf-8")
    env = {**os.environ, "PATH": f"{cargo.parent}{os.pathsep}{os.environ['PATH']}", "NIGHTLY": "x", "EXPLORE": "false", "LIMITS": ""}
    # GitHub runs a step without shell: as `bash -e {0}`.
    result = subprocess.run(["bash", "-e", "step.sh"], cwd=tmp_path, env=env, capture_output=True, text=True, timeout=60)
    assert result.returncode == 1
    assert re.findall(r"^::group::(\S+)$", result.stdout, re.MULTILINE) == names
    assert f"::error::fuzz targets that crashed: {crashing}" in result.stdout


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


def stale_workspace_crates(fuzz_lock, version):
    """The repository's crates in the fuzz lock file whose version is not the
    workspace's `version`, or which lack a dependency their Cargo.toml requires, as
    `cargo fetch --locked` would refuse them."""
    stale = []
    for package in tomllib.loads(fuzz_lock)["package"]:
        manifest = ROOT / "crates" / package["name"] / "Cargo.toml"
        if "source" in package or not manifest.is_file():
            continue
        required = {
            name
            for name, spec in tomllib.loads(manifest.read_text(encoding="utf-8")).get("dependencies", {}).items()
            if not (isinstance(spec, dict) and spec.get("optional"))
        }
        locked = {name.split(" ")[0] for name in package.get("dependencies", [])}
        if package["version"] != version or not required <= locked:
            stale.append(package["name"])
    return sorted(stale)


def test_the_fuzz_lock_file_follows_the_workspace():
    """Review 2026-10-31 R11: a version bump and two dependencies of ketchup-model left
    fuzz/Cargo.lock stale, and only the fuzz job in CI found it."""
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    fuzz_lock = (FUZZ / "Cargo.lock").read_text(encoding="utf-8")
    assert stale_workspace_crates(fuzz_lock, version) == []
    without = fuzz_lock.replace(' "ttf-parser",\n', "", 1)
    assert stale_workspace_crates(without, version) == ["ketchup-model"]
