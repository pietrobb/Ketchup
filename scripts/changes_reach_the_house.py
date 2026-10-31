#!/usr/bin/env python3
"""Whether a change can break a large-house test, so CI runs them in its pull request too.

The large-house tests (AGENTS.md section 6) are ignored in a step and run nightly. A
change of the program evaluator or of a crate it builds on, the example programs, a
test that reads `tiny-house.star`, the lock file, the workspace manifest or the
toolchain runs them in the pull request as well (review 2026-10-24 CI-23: a pull
request that changed `prelude.star` could break all 21 and pass; review 2026-10-26
CI-43: a change of `ketchup-model` or `Cargo.lock` did not count). A change elsewhere
still waits for the nightly run.

Prints `run=true` or `run=false` and appends it to `$GITHUB_OUTPUT`. A push to `main`
always runs them: with the checkout's two commits git sees only the last commit of a
push, and a push of 35 commits that changed `prelude.star` in another skipped them
(CI-43). For a pull request the changed files are those between `HEAD^1` and `HEAD`,
its merge commit against the base. When git cannot tell, the tests run.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EVALUATOR = "crates/ketchup-program"
PREFIXES = ("examples/programs/",)
FILES = ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml")
HOUSE = "tiny-house.star"

PATH_DEPENDENCY = re.compile(r'^([A-Za-z0-9_-]+)\s*=\s*\{[^}\n]*\bpath\s*=\s*"([^"]+)"', re.MULTILINE)
DEPENDENCY = re.compile(r"^([A-Za-z0-9_-]+)\s*=", re.MULTILINE)


def evaluator_directories(root: Path = ROOT) -> list[str]:
    """The evaluator and every crate it depends on by path, the patched ones included."""
    manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
    patch = manifest.split("[patch.crates-io]", 1)[1] if "[patch.crates-io]" in manifest else ""
    patched = {name: path for name, path in PATH_DEPENDENCY.findall(patch.split("\n[", 1)[0])}
    found, pending = [], [EVALUATOR]
    while pending:
        directory = pending.pop()
        if directory in found:
            continue
        found.append(directory)
        text = (root / directory / "Cargo.toml").read_text(encoding="utf-8")
        for _, path in PATH_DEPENDENCY.findall(text):
            pending.append(os.path.normpath(Path(directory) / path).replace(os.sep, "/"))
        pending.extend(path for name, path in patched.items() if name in DEPENDENCY.findall(text))
    return sorted(found)


def reaches_the_house(paths: list[str], root: Path = ROOT) -> bool:
    prefixes = PREFIXES + tuple(directory + "/" for directory in evaluator_directories(root))
    for path in paths:
        if path.startswith(prefixes) or path in FILES:
            return True
        file = root / path
        if path.endswith(".rs") and file.is_file() and HOUSE in file.read_text(encoding="utf-8", errors="replace"):
            return True
    return False


def changed_files(root: Path = ROOT) -> list[str] | None:
    result = subprocess.run(
        ["git", "diff", "--name-only", "HEAD^1", "HEAD"],
        cwd=root, capture_output=True, text=True, encoding="utf-8", check=False,
    )
    return result.stdout.splitlines() if result.returncode == 0 else None


def runs(event: str | None, root: Path = ROOT) -> bool:
    if event == "push":
        return True
    changed = changed_files(root)
    return changed is None or reaches_the_house(changed, root)


def main() -> int:
    line = f"run={'true' if runs(os.environ.get('GITHUB_EVENT_NAME')) else 'false'}"
    print(line)
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as file:
            file.write(line + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
