#!/usr/bin/env python3
"""Fail when a test hook is compiled into production code.

Methods that let a test reach past the user's path (`headless_*`: force a worker path,
install a result, hold a request) exist only for tests. In crate sources they must sit
behind `#[cfg(feature = "testing")]`, which only test builds enable, so the shipped
application cannot call them. Any `#[cfg(...)]` gates a hook that the production build,
without `test` and without the feature, leaves out whatever the other options are:
`all(windows, feature = "testing")` does, `not(feature = "testing")` does not (review
2026-10-31 TST-80). The crates and the Viewer apps are scanned (scripts/source_roots.py).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)
from source_roots import package_files  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
HOOK = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn (headless_\w+)", re.MULTILINE)
ATTRIBUTE = re.compile(r"^\s*(#\[|///)")
CFG = re.compile(r"^\s*#\[cfg\((.*)\)\]\s*$")
TOKEN = re.compile(r'\s*(all|any|not|\(|\)|,|[A-Za-z_]\w*(?:\s*=\s*"[^"]*")?)')
OFF_IN_PRODUCTION = {"test", 'feature = "testing"'}


def production_value(predicate: str) -> bool | None:
    """The cfg `predicate` in the production build: `test` and the testing feature off,
    every other option unknown (`None`)."""
    tokens = [token for token in TOKEN.findall(predicate) if token]
    at = 0

    def parse() -> bool | None:
        nonlocal at
        token = tokens[at]
        at += 1
        if token not in ("all", "any", "not"):
            return False if re.sub(r"\s*=\s*", " = ", token) in OFF_IN_PRODUCTION else None
        at += 1  # (
        values = []
        while tokens[at] != ")":
            values.append(parse())
            if tokens[at] == ",":
                at += 1
        at += 1  # )
        if token == "not":
            return None if values[0] is None else not values[0]
        if token == "all":
            return False if False in values else (None if None in values else True)
        return True if True in values else (None if None in values else False)

    try:
        return parse()
    except IndexError:
        return None


def gates(attribute: str) -> bool:
    cfg = CFG.match(attribute)
    return bool(cfg) and production_value(cfg.group(1)) is False


def ungated_hooks(root: Path = ROOT) -> list[str]:
    found = []
    for path in package_files(root, "src/**/*.rs"):
        relative = path.relative_to(root)
        if "tests" in relative.parts[:-1] or relative.stem == "tests" or relative.stem.endswith("_tests"):
            continue
        text = TEST_MODULE.sub("", path.read_text(encoding="utf-8", errors="replace"))
        lines = text.splitlines()
        for match in HOOK.finditer(text):
            line = text.count("\n", 0, match.start())
            attributes = []
            above = line - 1
            while above >= 0 and ATTRIBUTE.match(lines[above]):
                attributes.append(lines[above])
                above -= 1
            if not any(gates(attribute) for attribute in attributes):
                found.append(f"{relative.as_posix()}: {match.group(1)}")
    return found


def check(root: Path = ROOT) -> int:
    found = ungated_hooks(root)
    if found:
        print('Test hooks compiled into production code; gate them with #[cfg(feature = "testing")]:')
        print(*found, sep="\n  ")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
