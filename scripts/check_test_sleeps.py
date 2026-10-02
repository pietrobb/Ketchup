#!/usr/bin/env python3
"""Fail when tests gain `sleep` calls.

A test that sleeps guesses how long the code under test needs; under load the guess is
wrong and the test flakes. Wait on the condition instead (`wait_until(predicate,
deadline)` in the UI harness, a channel, a join).

The check is a ratchet: scripts/test_sleeps_baseline.txt records the sleeps that still
exist per test file. A count may only go down; a new file or a higher count fails. Run
with --update after removing sleeps to shrink the baseline. Never add entries by hand.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "test_sleeps_baseline.txt"
SLEEP = re.compile(r"\bsleep\(")


def is_test_file(relative: Path) -> bool:
    return "tests" in relative.parts[:-1] or relative.stem == "tests" or relative.stem.endswith("_tests")


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted(root.glob("crates/*/**/*.rs")):
        relative = path.relative_to(root)
        if "target" in relative.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if not is_test_file(relative):
            text = "".join(match.group(0) for match in TEST_MODULE.finditer(text))
        count = len(SLEEP.findall(text))
        if count:
            counts[relative.as_posix()] = count
    return counts


def read_baseline(path: Path = BASELINE) -> dict[str, int]:
    baseline: dict[str, int] = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline


def check(root: Path = ROOT, update: bool = False, allow_growth: bool = False) -> int:
    counts = current_counts(root)
    baseline_path = root / "scripts" / "test_sleeps_baseline.txt"
    baseline = read_baseline(baseline_path)
    grown = [f"{key}: {value} (allowed {baseline.get(key, 0)})"
             for key, value in sorted(counts.items()) if value > baseline.get(key, 0)]
    if update:
        if baseline and grown and not allow_growth:
            print("Refusing to grow the baseline:", *grown, sep="\n  ")
            return 1
        lines = [f"{key} {value}" for key, value in sorted(counts.items())]
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text("# Remaining sleep calls in tests; may only shrink.\n"
                                 + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        print(f"baseline: {sum(counts.values())} sleeps in {len(counts)} files")
        return 0
    if grown:
        print("Tests gained sleep calls; wait on the condition instead (wait_until, a channel, a join):")
        print(*grown, sep="\n  ")
        return 1
    shrunk = sum(baseline.values()) - sum(counts.values())
    if shrunk > 0:
        print(f"OK; {shrunk} sleeps removed since the baseline. Run with --update to lock in the progress.")
    else:
        print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check(update="--update" in sys.argv, allow_growth="--allow-growth" in sys.argv))
