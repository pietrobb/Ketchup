import importlib.util
import os
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


def test_required_production_set_covers_acceptance_and_model_tools():
    assert runner.NATIVE_TESTS == (
        "tests/test_headless_acceptance.py",
        "tests/test_ketchup_model_skill.py",
        "tests/test_model_tools_errors.py",
        "tests/test_model_tools_protocol.py",
    )
    assert all((ROOT / relative).is_file() for relative in runner.NATIVE_TESTS)
