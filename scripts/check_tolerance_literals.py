#!/usr/bin/env python3
"""Fail when a new tolerance or size-limit literal appears in crate source.

Tolerances and model limits have one home: crates/ketchup-tolerance/src/lib.rs
(the document's TolerancePolicy), handed to the exact kernel through its FFI,
and crates/ketchup-tolerance/src/limits.rs. A literal such as 1.0e-9 anywhere
else is a module-local tolerance. A length compared with a literal of 8 or more
(`.len() > 64`, `(2..=16).contains(&x.len())`) is an unnamed size limit; it is
counted as `<file> limit`.

A number that is not a tolerance (a view limit, a mesh setting) carries a
`not a tolerance: <reason>` comment on its line or the comment line above.

The check is a ratchet: scripts/tolerance_literals_baseline.txt records the
literals that still exist, per file. A count may only go down; a new file or a
higher count fails. Run with --update after removing literals to shrink the
baseline. Never add entries by hand.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE_NAME = "scripts/tolerance_literals_baseline.txt"
SOURCES = ["crates/*/src/**/*.rs", "crates/*/src/**/*.cc", "crates/*/src/**/*.hxx", "crates/*/include/**/*.hxx"]
EXCLUDED_PARTS = {"tests", "examples", "fixtures"}
HOME = "crates/ketchup-tolerance/src/lib.rs"
LITERAL = re.compile(r"(?<![\w.])\d+(?:\.\d+)?(?:_f64)?[eE]-\d+")
LIMIT_FLOOR = 8
# `.len() <op> N` / `N <op> x.len()` and `(a..=N).contains(&x.len())` with a literal N.
LEN_COMPARE = re.compile(r"\.len\(\)\s*(?:[<>]=?|[!=]=)\s*(\d[\d_]*)\b(?!\.)"
                         r"|(?<![\w.])(\d[\d_]*)\s*(?:[<>]=?|[!=]=)\s*[\w.]+\.len\(\)")
LEN_RANGE = re.compile(r"\(\s*[\w:]+\s*\.\.=?\s*(\d[\d_]*)\s*\)\s*\.contains\(\s*&[^)]*\.len\(\)")
# An inline `#[cfg(test)] mod name { ... }` at the top level of a Rust file, up to its
# closing brace in column 0; test assertions may state their own precision.
TEST_MODULE = re.compile(r"^#\[cfg\(test\)\]\s*\n(?:#\[[^\n]*\]\s*\n)*mod \w+ \{\n.*?^\}",
                         re.MULTILINE | re.DOTALL)


# A documented exception: a number that looks like a tolerance but is a view limit, a
# mesh setting or a probe step. The marker sits on the line or on a comment line above.
EXEMPT = "not a tolerance:"


def production_text(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="replace")
    if path.suffix == ".rs":
        text = TEST_MODULE.sub("", text)
    kept, previous = [], ""
    for line in text.splitlines():
        exempt = EXEMPT in line or (previous.lstrip().startswith(("//", "#")) and EXEMPT in previous)
        if not exempt:
            kept.append(line)
        previous = line
    return "\n".join(kept)


def limit_literals(text: str) -> int:
    """Size limits written as a literal of LIMIT_FLOOR or more next to a length."""
    numbers = [next(group for group in match.groups() if group)
               for pattern in (LEN_COMPARE, LEN_RANGE) for match in pattern.finditer(text)]
    return sum(int(number.replace("_", "")) >= LIMIT_FLOOR for number in numbers)


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for pattern in SOURCES:
        for path in sorted(root.glob(pattern)):
            relative = path.relative_to(root)
            key = relative.as_posix()
            if (key == HOME or EXCLUDED_PARTS & set(relative.parts[:-1])
                    or path.stem.endswith("tests")):
                continue
            text = production_text(path)
            found = len(LITERAL.findall(text))
            if found:
                counts[key] = found
            limits = limit_literals(text)
            if limits:
                counts[f"{key} limit"] = limits
    return counts


def read_baseline(root: Path = ROOT) -> dict[str, int]:
    baseline: dict[str, int] = {}
    path = root / BASELINE_NAME
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline


def check(root: Path = ROOT, update: bool = False, allow_growth: bool = False) -> int:
    counts = current_counts(root)
    old = read_baseline(root)
    if update:
        grown = {key: value for key, value in counts.items() if value > old.get(key, 0)}
        if old and grown and not allow_growth:
            print("Refusing to grow the baseline:", *sorted(grown), sep="\n  ")
            return 1
        lines = [f"{key} {value}" for key, value in sorted(counts.items())]
        (root / BASELINE_NAME).write_text(
            "# Remaining tolerance literals (<file>) and size-limit literals (<file> limit); may only shrink.\n"
            + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        print(f"baseline: {sum(counts.values())} literals in {len(counts)} files")
        return 0
    failures = [f"{key}: {value} (allowed {old.get(key, 0)})"
                for key, value in sorted(counts.items()) if value > old.get(key, 0)]
    if failures:
        print(f"New tolerance or limit literals; use TolerancePolicy, {HOME} or limits.rs:")
        print(*failures, sep="\n  ")
        return 1
    shrunk = sum(old.values()) - sum(counts.values())
    if shrunk > 0:
        print(f"OK; {shrunk} literals removed since the baseline. "
              "Run with --update to lock in the progress.")
    else:
        print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check(update="--update" in sys.argv, allow_growth="--allow-growth" in sys.argv))
