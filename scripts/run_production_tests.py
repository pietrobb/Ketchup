"""Build and run the required native Python production tests without skips."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

import pytest


ROOT = Path(__file__).resolve().parents[1]
NATIVE_TESTS = (
    "tests/test_headless_acceptance.py",
    "tests/test_ketchup_model_skill.py",
    "tests/test_model_tools_errors.py",
    "tests/test_model_tools_protocol.py",
)


class RunnerError(RuntimeError):
    pass


class FailOnSkipPlugin:
    def __init__(self) -> None:
        self.skipped: list[str] = []

    def pytest_collectreport(self, report: pytest.CollectReport) -> None:
        if report.skipped:
            self.skipped.append(report.nodeid)

    def pytest_runtest_logreport(self, report: pytest.TestReport) -> None:
        if report.skipped:
            self.skipped.append(report.nodeid)

    def pytest_sessionfinish(self, session: pytest.Session, exitstatus: int) -> None:
        if self.skipped:
            session.exitstatus = pytest.ExitCode.TESTS_FAILED
            print("Production test runner rejects skipped tests:", file=sys.stderr)
            for nodeid in self.skipped:
                print(f"  {nodeid}", file=sys.stderr)


def run(
    command: list[str], *, input_text: str | None = None, capture_output: bool = False
) -> subprocess.CompletedProcess[str]:
    print("+", subprocess.list2cmdline(command), flush=True)
    result = subprocess.run(
        command,
        cwd=ROOT,
        input=input_text,
        text=True,
        capture_output=input_text is not None or capture_output,
        check=False,
    )
    if result.returncode != 0:
        if result.stdout:
            print(result.stdout, file=sys.stderr)
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        raise RunnerError(f"Command failed with exit code {result.returncode}: {command[0]}")
    return result


def cargo_target_directory(cargo: str) -> Path:
    result = subprocess.run(
        [cargo, "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RunnerError(f"cargo metadata failed: {result.stderr.strip()}")
    try:
        return Path(json.loads(result.stdout)["target_directory"]).resolve()
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise RunnerError("cargo metadata did not return target_directory") from error


def native_paths(target_directory: Path) -> tuple[Path, Path]:
    suffix = ".exe" if os.name == "nt" else ""
    binary_directory = target_directory / "debug"
    return (
        binary_directory / f"ketchup-headless{suffix}",
        binary_directory / f"ketchup-exact-worker{suffix}",
    )


def require_component(path: Path, label: str) -> Path:
    resolved = path.resolve()
    if not resolved.is_file():
        raise RunnerError(f"Missing required {label}: {resolved}")
    return resolved


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    cargo = shutil.which("cargo")
    if cargo is None:
        raise RunnerError("cargo is required on PATH")

    target_directory = cargo_target_directory(cargo)
    run([cargo, "build", "--locked", "-p", "ketchup-headless", "--bin", "ketchup-headless"])
    run([cargo, "build", "--locked", "-p", "ketchup-scheduler", "--bin", "ketchup-exact-worker"])
    os.environ["KETCHUP_BUILD_VERSION"] = "production-test"
    run([
        cargo,
        "build",
        "--locked",
        "--release",
        "-p",
        "ketchup-app",
        "--bin",
        "ketchup-app",
        "--no-default-features",
        "--features",
        "manual-alpha",
    ])

    headless_path, worker_path = native_paths(target_directory)
    headless = require_component(headless_path, "ketchup-headless executable")
    worker = require_component(worker_path, "ketchup-exact-worker executable")
    pong = run([str(worker)], input_text="PING\n").stdout.strip()
    if pong != "PONG":
        raise RunnerError(f"Exact worker handshake returned {pong!r}, expected 'PONG'")
    suffix = ".exe" if os.name == "nt" else ""
    manual_alpha = require_component(
        target_directory / "release" / f"ketchup-app{suffix}",
        "OAuth-enabled Manual Alpha executable",
    )
    with tempfile.TemporaryDirectory(prefix="ketchup-production-smoke-") as directory:
        persistence_path = Path(directory) / "manual-alpha-roundtrip.ketchup"
        verification = run(
            [str(manual_alpha), "--verify-manual-alpha", str(persistence_path)],
            capture_output=True,
        ).stdout.strip()
        expected = (
            "Ketchup Manual Alpha production-test verified; "
            "private-oauth codex-oauth gpt-5.6-sol"
        )
        if verification != expected or not persistence_path.is_file():
            raise RunnerError("Manual Alpha startup/persistence verification failed")

    os.environ["KETCHUP_HEADLESS"] = str(headless)
    os.environ["KETCHUP_EXACT_WORKER"] = str(worker)
    os.environ["KETCHUP_LIVE_PYTHON"] = str(Path(sys.executable).resolve())
    run([sys.executable, "-c", "import anthropic"])
    print(f"Python: {Path(sys.executable).resolve()}")
    print(f"KETCHUP_HEADLESS={headless} sha256={sha256(headless)}")
    print(f"KETCHUP_EXACT_WORKER={worker} sha256={sha256(worker)}")
    print(f"MANUAL_ALPHA={manual_alpha} sha256={sha256(manual_alpha)}")
    run([
        cargo,
        "test",
        "--locked",
        "-p",
        "ketchup-app",
        "--no-default-features",
        "--test",
        "live_bridge_python",
        "--",
        "--ignored",
        "--nocapture",
    ])

    missing_tests = [relative for relative in NATIVE_TESTS if not (ROOT / relative).is_file()]
    if missing_tests:
        raise RunnerError(f"Missing required production tests: {', '.join(missing_tests)}")

    plugin = FailOnSkipPlugin()
    return int(pytest.main([*NATIVE_TESTS, "-ra"], plugins=[plugin]))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RunnerError as error:
        print(f"production test runner failed: {error}", file=sys.stderr)
        raise SystemExit(1)
