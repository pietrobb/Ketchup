import importlib.util
import json
import os
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "run_production_tests", ROOT / "scripts" / "run_production_tests.py"
)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def test_native_paths_are_explicit_debug_artifacts(tmp_path):
    suffix = ".exe" if os.name == "nt" else ""
    assert runner.native_paths(tmp_path) == (
        tmp_path / "debug" / f"ketchup-headless{suffix}",
        tmp_path / "debug" / f"ketchup-exact-worker{suffix}",
    )


def test_the_live_mcp_tests_get_the_debug_app_beside_its_worker(tmp_path):
    # Review 2026-10-24 CI-22: nothing set KETCHUP_APP, so these tests ran nowhere.
    assert runner.app_tests() == ["tests/test_mcp_live_measurement.py"]
    for path in (ROOT / "tests").glob("test_*.py"):
        if "KETCHUP_APP" in path.read_text(encoding="utf-8") and path.name != Path(__file__).name:
            assert path.relative_to(ROOT).as_posix() in runner.app_tests(), path
    assert runner.app_path(tmp_path).parent == runner.native_paths(tmp_path)[1].parent
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    assert "cargo build --locked -p ketchup-app --bin ketchup-app" in workflow
    # The live tests stop the window the server started through psutil.
    assert "psutil==" in (ROOT / "tests" / "requirements-ci.txt").read_text(encoding="utf-8")


def test_the_release_app_tests_exist_and_run_nightly():
    # The debug run leaves them out, so the nightly job must run them.
    for test in runner.RELEASE_APP_TESTS:
        path, name = test.split("::")
        assert path in runner.app_tests(), test
        assert f"def {name}(" in (ROOT / path).read_text(encoding="utf-8"), test
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    assert "cargo build --locked --release -p ketchup-app --bin ketchup-app" in workflow
    assert "python scripts/run_production_tests.py --release-app" in workflow


def test_the_macos_bundle_runs_every_live_test():
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    job = workflow.split("\n  macos-app:\n", 1)[1].split("\n\n  ", 1)[0]
    assert "scripts/macos/build-app.sh" in job
    assert "mv third_party/occt-install-r0-v1" in job
    assert "python scripts/run_production_tests.py --packaged-app" in job
    assert "macos-app" in workflow.split("\n  rust-required:\n", 1)[1]


def test_missing_required_component_fails_closed(tmp_path):
    missing = tmp_path / "ketchup-headless"
    with pytest.raises(runner.RunnerError, match="Missing required headless"):
        runner.require_component(missing, "headless")


def test_skip_plugin_changes_success_to_failure():
    plugin = runner.FailOnSkipPlugin()
    plugin.pytest_collectreport(SimpleNamespace(skipped=True, nodeid="tests/test_module.py"))
    plugin.pytest_runtest_logreport(SimpleNamespace(skipped=True, nodeid="tests/test_native.py::test_real"))
    session = SimpleNamespace(exitstatus=pytest.ExitCode.OK)

    plugin.pytest_sessionfinish(session, int(pytest.ExitCode.OK))

    assert session.exitstatus == pytest.ExitCode.TESTS_FAILED
    assert plugin.skipped == ["tests/test_module.py", "tests/test_native.py::test_real"]


def _child_writing(payload: bytes, monkeypatch):
    """Every command the runner starts becomes a child writing `payload`, with
    the runner's own decoding arguments."""
    real_run = runner.subprocess.run
    script = f"import sys; sys.stdout.buffer.write({payload!r})"

    def run(command, **kwargs):
        return real_run([sys.executable, "-c", script], **kwargs)

    monkeypatch.setattr(runner.subprocess, "run", run)


def test_cargo_metadata_is_read_as_utf_8(monkeypatch):
    # The Windows CI runner decoded `cargo metadata` with its ANSI code page
    # and failed on the bytes of a non-ASCII character.
    target = "C:\\Kečup\\target"
    _child_writing(json.dumps({"target_directory": target}, ensure_ascii=False).encode(), monkeypatch)
    assert runner.cargo_target_directory("cargo") == Path(target).resolve()


def test_command_output_is_read_as_utf_8(monkeypatch):
    _child_writing("Kečup ✓".encode(), monkeypatch)
    assert runner.run(["tool"], capture_output=True).stdout == "Kečup ✓"


def test_the_native_set_is_every_file_that_skips_without_the_built_cli():
    # Review 2026-10-17 TST-2: five of the eight such files ran nowhere.
    tests = runner.native_tests()
    assert tests == [
        "tests/test_headless_acceptance.py",
        "tests/test_model_tools_errors.py",
        "tests/test_model_tools_protocol.py",
        "tests/test_opposing_holes_headless.py",
        "tests/test_production_setups.py",
        "tests/test_production_stock_frames.py",
        "tests/test_production_workflow.py",
        "tests/test_program_headless.py",
    ]
    for path in (ROOT / "tests").glob("test_*.py"):
        text = path.read_text(encoding="utf-8")
        skips = "skip" in text and "KETCHUP_HEADLESS" in text
        if skips and path.relative_to(ROOT).as_posix() not in tests:
            # Unit tests of the client with a mocked CLI, and this file.
            assert path.name in ("test_ketchup_headless.py", Path(__file__).name), path
