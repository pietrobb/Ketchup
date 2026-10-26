#!/usr/bin/env python3
"""Whether a change can break a large-house test, so CI runs them in its pull request too.

The large-house tests (AGENTS.md section 6) are ignored in a step and run nightly. A
change of the rule library, the program evaluator, the example programs or a test
that reads `tiny-house.star` runs them in the pull request as well (review 2026-10-24
CI-23: a pull request that changed `prelude.star` could break all 21 and pass). A
change elsewhere still waits for the nightly run.

Prints `run=true` or `run=false` and appends it to `$GITHUB_OUTPUT`. The changed files
are those between `HEAD^1` and `HEAD`: for a pull request its merge commit against
the base, for a push the last commit. When git cannot tell, the tests run.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PREFIXES = ("crates/ketchup-program/", "examples/programs/")
HOUSE = "tiny-house.star"


def reaches_the_house(paths: list[str], root: Path = ROOT) -> bool:
    for path in paths:
        if path.startswith(PREFIXES):
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


def main() -> int:
    changed = changed_files()
    line = f"run={'true' if changed is None or reaches_the_house(changed) else 'false'}"
    print(line)
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as file:
            file.write(line + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
