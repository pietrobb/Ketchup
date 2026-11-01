#!/usr/bin/env python3
"""Fail when tests gain `sleep` calls.

A test that sleeps guesses how long the code under test needs; under load the guess is
wrong and the test flakes. Wait on the condition instead (`wait_until(predicate,
deadline)` in the UI harness, a channel, a join). The Python tests (`tests/`,
`sdk/*/tests/`) count their `sleep(...)` calls, not the word in a string.

A helper in the same file that sleeps, or calls a helper that does, counts at each
call of it as many sleeps as it makes, and not again where it is defined (review 2026-10-29
TST-42: moving three polling loops into one `ask_until` lowered the count while the
tests slept as before). Only plain calls are followed, not methods or other files.

The check is a ratchet: scripts/test_sleeps_baseline.txt records the sleeps that still
exist per test file. A count may only go down; a new file or a higher count fails. Run
with --update after removing sleeps to shrink the baseline. Never add entries by hand.
"""
from __future__ import annotations

import ast
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_files  # noqa: E402
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)
from check_large_house_tests import functions, masked  # noqa: E402  (Rust fns, comments blanked)

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "test_sleeps_baseline.txt"
# `thread::park_timeout` sleeps as well when nothing unparks the thread; a space before
# the parenthesis is still a call (review 2026-10-19 P3).
SLEEPS = ("sleep", "sleep_ms", "park_timeout", "park_timeout_ms")
# `use std::thread::sleep as nap;` calls it `nap` (review 2026-10-24).
RUST_ALIAS = re.compile(rf"\b(?:{'|'.join(SLEEPS)})\s+as\s+(\w+)")


def free_call(name: str) -> re.Pattern:
    """A call of the function `name` itself: not a method, a path or its definition."""
    return re.compile(rf"(?<![\w.:])(?<!fn ){re.escape(name)}\s*\(")


def rust_sleeps(text: str, aliases_from: str) -> int:
    """The sleep calls of `text` (not in comments or strings), under the names
    `aliases_from` imports them as too, a helper that sleeps counted at each call."""
    names = [*SLEEPS, *RUST_ALIAS.findall(aliases_from)]
    sleep = re.compile(rf"\b(?:{'|'.join(map(re.escape, names))})\s*\(")
    plain = masked(text)
    items = functions(text)
    sleeping: set[str] = set()
    changed = True
    while changed:
        changed = False
        for item in items:
            if item["name"] not in sleeping and (
                    sleep.search(item["code"]) or any(free_call(name).search(item["code"]) for name in sleeping)):
                sleeping.add(item["name"])
                changed = True
    helpers = {name for name in sleeping if free_call(name).search(plain)}
    spans = {item["name"]: item["span"] for item in items if item["name"] in helpers}
    places = [(found.start(), None) for found in sleep.finditer(plain)]
    places += [(found.start(), name) for name in helpers for found in free_call(name).finditer(plain)]
    weights: dict[str, int] = {}

    def weight(name: str | None, seen: frozenset = frozenset()) -> int:
        if name is None:
            return 1
        if name in seen:
            return 1
        if name not in weights:
            start, end = spans[name]
            weights[name] = max(1, within(start, end, seen | {name}))
        return weights[name]

    def within(start: int, end: int, seen: frozenset) -> int:
        """The sleeps in [start, end) that no helper nested deeper holds."""
        total = 0
        for place, name in places:
            if not start <= place < end:
                continue
            if any(start <= inner_start and inner_end <= end and (inner_start, inner_end) != (start, end)
                   and inner_start <= place < inner_end for inner_start, inner_end in spans.values()):
                continue
            total += weight(name, seen)
        return total

    outside = [(place, name) for place, name in places
               if not any(start <= place < end for start, end in spans.values())]
    return sum(weight(name) for _, name in outside)


def is_test_file(relative: Path) -> bool:
    return "tests" in relative.parts[:-1] or relative.stem == "tests" or relative.stem.endswith("_tests")


def python_sleeps(text: str) -> int:
    """The `sleep(...)` and `time.sleep(...)` calls of a Python file, `sleep` imported
    under another name included, and the wait of an Event made in the same call
    (`Event().wait(0.5)`), which nothing can set."""
    tree = ast.parse(text)
    names = {"sleep"} | {alias.asname for node in ast.walk(tree) if isinstance(node, ast.ImportFrom)
                         for alias in node.names if alias.name == "sleep" and alias.asname}

    def sleeps(node: ast.AST) -> bool:
        return isinstance(node, ast.Call) and (
            isinstance(node.func, ast.Name) and node.func.id in names
            or isinstance(node.func, ast.Attribute) and node.func.attr == "sleep"
            or isinstance(node.func, ast.Attribute) and node.func.attr == "wait"
            and isinstance(node.func.value, ast.Call))

    def calls(node: ast.AST, of: set[str]) -> bool:
        return isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id in of

    # A helper that sleeps, or calls one that does, counts at each call (review
    # 2026-10-29 TST-42); its own sleeps are then counted there, not again inside it.
    defined = [node for node in ast.walk(tree) if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))]
    sleeping: set[str] = set()
    changed = True
    while changed:
        changed = False
        for node in defined:
            if node.name not in sleeping and any(sleeps(inner) or calls(inner, sleeping) for inner in ast.walk(node)):
                sleeping.add(node.name)
                changed = True
    helpers = {node.func.id for node in ast.walk(tree) if calls(node, sleeping)}
    bodies = {node.name: node for node in reversed(defined) if node.name in helpers}

    def weight(name: str, seen: frozenset) -> int:
        """The sleeps one call of the helper `name` makes, at least one."""
        if name in seen:
            return 1
        return max(1, sum(count(child, seen | {name}) for child in ast.iter_child_nodes(bodies[name])))

    def count(node: ast.AST, seen: frozenset = frozenset()) -> int:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in helpers:
            return 0
        own = 1 if sleeps(node) else weight(node.func.id, seen) if calls(node, helpers) else 0
        return own + sum(count(child, seen) for child in ast.iter_child_nodes(node))

    return count(tree)


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted([*root.glob("tests/**/*.py"), *root.glob("sdk/*/tests/**/*.py")]):
        count = python_sleeps(path.read_text(encoding="utf-8"))
        if count:
            counts[path.relative_to(root).as_posix()] = count
    paths = package_files(root)
    for path in sorted(paths):
        relative = path.relative_to(root)
        if "target" in relative.parts:
            continue
        source = path.read_text(encoding="utf-8", errors="replace")
        text = source
        if not is_test_file(relative):
            text = "".join(match.group(0) for match in TEST_MODULE.finditer(source))
        count = rust_sleeps(text, source)
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
