#!/usr/bin/env python3
"""Fail when a test hook is compiled into production code.

Methods that let a test reach past the user's path (`headless_*`: force a worker path,
install a result, hold a request) exist only for tests. In crate sources they must sit
behind `#[cfg(feature = "testing")]`, which only test builds enable, so the shipped
application cannot call them.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
HOOK = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn (headless_\w+)", re.MULTILINE)
ATTRIBUTE = re.compile(r"^\s*(#\[|///)")
GATE = 'feature = "testing"'


def ungated_hooks(root: Path = ROOT) -> list[str]:
    found = []
    for path in sorted(root.glob("crates/*/src/**/*.rs")):
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
            if not any(GATE in attribute for attribute in attributes):
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
