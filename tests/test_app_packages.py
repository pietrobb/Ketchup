"""Every worker the application starts goes into each package: the macOS bundle
and the Windows installer build it, ship it and check that it answers Hello.
The macOS bundle once left out the program worker, and opening a document,
the only check then, starts no worker (review 2026-10-30). This runs on any
system; the macOS CI job also runs the live tests against the built bundle."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKERS = sorted(path.stem for path in (ROOT / "crates" / "ketchup-scheduler" / "src" / "bin").glob("*.rs"))


def test_the_workers_are_found():
    assert {"ketchup-exact-worker", "ketchup-program-worker"} <= set(WORKERS)


def test_the_macos_bundle_builds_ships_and_greets_every_worker():
    script = (ROOT / "scripts" / "macos" / "build-app.sh").read_text(encoding="utf-8")
    build = next(line for line in script.splitlines() if line.startswith("cargo build"))
    bundled = re.search(r"^workers=\((.*)\)$", script, re.MULTILINE).group(1)
    for worker in WORKERS:
        assert f"--bin {worker}" in build, worker
        assert f"target/release/{worker}" in script, worker
        assert f'"$app/Contents/MacOS/{worker}"' in bundled, worker
    # The signed and handshaken list is the one above.
    assert 'sign "${workers[@]}"' in script
    assert 'for worker in "${workers[@]}"' in script


def test_the_windows_installer_builds_ships_and_greets_every_worker():
    script = (ROOT / "scripts" / "windows" / "build-installer.ps1").read_text(encoding="utf-8")
    for worker in WORKERS:
        assert f"--bin {worker}" in script, worker
        assert f'(Join-Path $payloadDir "{worker}.exe")' in script, worker
        assert f'Assert-ExactWorkerHandshake (Join-Path $installedDir "{worker}.exe")' in script, worker
