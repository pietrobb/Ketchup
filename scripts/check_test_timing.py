#!/usr/bin/env python3
"""Fail when a test outside `perf_*` asserts a wall-clock budget.

A test that asserts how long something took fails when the machine is busy (a parallel
suite, a debug build, a software GPU), not when the code is wrong. Ordinary tests check
the outcome instead: the work finished, was cancelled, or reported a timeout. A time
budget belongs only in a function named `perf_*` (the test and its helpers), marked
`#[ignore]` and run nightly in release (review 2026-10-17 TST-3). A deadline that only
stops a wait on a hung condition (`Instant::now() < deadline`, `recv_timeout`) is not a
budget and is not counted.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_test_sleeps import is_test_file  # noqa: E402
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
# A measured duration compared against an upper bound: `elapsed() <`, `.as_millis() <`,
# or anything `< Duration::…`, `< …_BUDGET`, `< …DEADLINE`.
BUDGET = re.compile(
    r"(?:\belapsed\(\)|\.as_(?:millis|micros|nanos|secs(?:_f32|_f64)?)\(\))\s*<=?\s"
    r"|\s<=?\s*(?:std::time::)?Duration::"
    r"|\s<=?\s*[A-Z][A-Z0-9_]*(?:BUDGET|DEADLINE)\b"
)
FUNCTION = re.compile(r"\bfn\s+(\w+)")
# A `perf_*` test and the attributes above it, which must ignore it in the PR suite.
PERF_TEST = re.compile(r"((?:^[ \t]*#\[[^\n]*\n|^[ \t]*///[^\n]*\n)+)[ \t]*fn\s+perf_\w+", re.M)
# Code that reads the clock; elsewhere a `Duration` compared is a computed value
# (a retry delay), not a measured one.
CLOCK = re.compile(r"Instant::now\(\)|\.elapsed\(\)")


def budgets(text: str) -> list[int]:
    """Lines of `text` with a budget outside functions named `perf_*`."""
    if not CLOCK.search(text):
        return []
    found = []
    function = ""
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("//"):
            continue
        named = FUNCTION.search(line)
        if named:
            function = named.group(1)
        if not function.startswith("perf_") and BUDGET.search(line):
            found.append(number)
    return found


def unignored_perf_tests(text: str) -> list[int]:
    """Lines of `#[test] fn perf_*` without `#[ignore`: such a test would time the PR suite."""
    return [text.count("\n", 0, test.end()) + 1 for test in PERF_TEST.finditer(text)
            if "#[test]" in test.group(1) and "#[ignore" not in test.group(1)]


def violations(root: Path = ROOT) -> list[str]:
    found = []
    for path in sorted(root.glob("crates/*/**/*.rs")):
        relative = path.relative_to(root)
        if "target" in relative.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        found += [f"{relative.as_posix()}:{line} perf test not ignored"
                  for line in unignored_perf_tests(text)]
        if is_test_file(relative):
            found += [f"{relative.as_posix()}:{line}" for line in budgets(text)]
        else:
            # Unit tests inside production files; their own line numbers.
            found += [f"{relative.as_posix()} (unit tests)" for module in TEST_MODULE.finditer(text)
                      for _ in budgets(module.group(0))]
    return found


def check(root: Path = ROOT) -> int:
    found = violations(root)
    if found:
        print("Tests assert wall-clock budgets, or a perf_* test is not ignored; assert the "
              "outcome instead, or move the budget into an ignored perf_* test run nightly in "
              "release:")
        print(*found, sep="\n  ")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
