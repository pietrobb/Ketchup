"""Build and run the required native Python production tests without skips.

`--prebuilt` runs only the native tests against the debug `ketchup-headless` and
`ketchup-exact-worker` a CI job already built, without the workspace build and
the release window check.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

import pytest


ROOT = Path(__file__).resolve().parents[1]
# A native test reads the built CLI from this variable and skips without it.
NATIVE_MARKER = 'os.environ.get("KETCHUP_HEADLESS")'


def native_tests() -> list[str]:
    """Every test file that needs the built CLI, so a new one cannot stay out."""
    return sorted(
        path.relative_to(ROOT).as_posix()
        for path in (ROOT / "tests").glob("test_*.py")
        if NATIVE_MARKER in path.read_text(encoding="utf-8")
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
        encoding="utf-8",
        errors="replace",
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


def worker_handshake(worker: Path) -> str:
    """Send the typed `Hello` frame (u32 LE length + CBOR) and return the protocol identity."""
    hello = b"\x65Hello"
    print("+", worker, "<Hello frame>", flush=True)
    result = subprocess.run(
        [str(worker)],
        cwd=ROOT,
        input=struct.pack("<I", len(hello)) + hello,
        capture_output=True,
        check=False,
    )
    reply = result.stdout
    if result.returncode != 0 or len(reply) < 4:
        raise RunnerError(f"Exact worker handshake failed with exit code {result.returncode}")
    (length,) = struct.unpack("<I", reply[:4])
    body = reply[4:]
    # {"Hello": {"protocol": <64-character text>}}
    prefix = b"\xa1\x65Hello\xa1\x68protocol\x78\x40"
    if length != len(body) or not body.startswith(prefix) or len(body) != len(prefix) + 64:
        raise RunnerError(f"Exact worker handshake returned {body!r}")
    return body[len(prefix):].decode("ascii")


def cargo_target_directory(cargo: str) -> Path:
    result = subprocess.run(
        [cargo, "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
        # Cargo writes UTF-8, not the code page of the Windows runner.
        encoding="utf-8",
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


def verify_app(cargo: str, target_directory: Path) -> None:
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
    ])
    suffix = ".exe" if os.name == "nt" else ""
    app = require_component(
        target_directory / "release" / f"ketchup-app{suffix}",
        "ketchup-app release executable",
    )
    with tempfile.TemporaryDirectory(prefix="ketchup-production-smoke-") as directory:
        persistence_path = Path(directory) / "install-roundtrip.ketchup"
        verification = run(
            [str(app), "--verify-install", str(persistence_path)],
            capture_output=True,
        ).stdout.strip()
        expected = "Ketchup production-test verified"
        if verification != expected or not persistence_path.is_file():
            raise RunnerError("ketchup-app startup/persistence verification failed")
    print(f"KETCHUP_APP={app} sha256={sha256(app)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--prebuilt",
        action="store_true",
        help="test the debug binaries already built, without building",
    )
    prebuilt = parser.parse_args().prebuilt

    cargo = shutil.which("cargo")
    if cargo is None:
        raise RunnerError("cargo is required on PATH")

    target_directory = cargo_target_directory(cargo)
    if not prebuilt:
        run([cargo, "test", "--locked", "--workspace", "--all-targets", "--no-run"])
        verify_app(cargo, target_directory)

    headless_path, worker_path = native_paths(target_directory)
    headless = require_component(headless_path, "ketchup-headless executable")
    worker = require_component(worker_path, "ketchup-exact-worker executable")
    worker_handshake(worker)

    os.environ["KETCHUP_HEADLESS"] = str(headless)
    os.environ["KETCHUP_EXACT_WORKER"] = str(worker)
    os.environ["KETCHUP_PYTHON"] = str(Path(sys.executable).resolve())
    print(f"Python: {Path(sys.executable).resolve()}")
    print(f"KETCHUP_HEADLESS={headless} sha256={sha256(headless)}")
    print(f"KETCHUP_EXACT_WORKER={worker} sha256={sha256(worker)}")

    tests = native_tests()
    if not tests:
        raise RunnerError(f"No test file reads {NATIVE_MARKER}")
    print("Native tests:", " ".join(tests))
    plugin = FailOnSkipPlugin()
    return int(pytest.main([*tests, "-ra"], plugins=[plugin]))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RunnerError as error:
        print(f"production test runner failed: {error}", file=sys.stderr)
        raise SystemExit(1)
