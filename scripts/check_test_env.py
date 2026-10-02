#!/usr/bin/env python3
"""Fail when test code reads an environment variable of its own.

What steers tests (the Python interpreter, refreshing golden files, where reports go)
has one name each in the `ketchup-test-env` crate; a test that reads another name
splits one setting into several (`PYTHON`, `KETCHUP_PYTHON`, `KETCHUP_LIVE_PYTHON`) and
the runner sets the wrong one. Test code may read a variable directly only when it
hands that same variable to a child process it starts (`.env(NAME, ..)`,
`OsString::from(NAME)`): that is the test's own wiring, not a setting.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_test_sleeps import is_test_file  # noqa: E402
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "crates/ketchup-test-env/"
READ = re.compile(r"\benv::var(?:_os)?\(\s*([^()]*?)\s*\)")


def handed_on(text: str, name: str) -> bool:
    return re.search(r"(?:\.env\(|::from\()\s*" + re.escape(name) + r"\s*[,)]", text) is not None


def own_variables(root: Path = ROOT) -> list[str]:
    found = []
    for path in sorted(root.glob("crates/*/**/*.rs")):
        relative = path.relative_to(root)
        if "target" in relative.parts or relative.as_posix().startswith(SOURCE):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if not is_test_file(relative):
            text = "".join(match.group(0) for match in TEST_MODULE.finditer(text))
        for match in READ.finditer(text):
            if not handed_on(text, match.group(1)):
                found.append(f"{relative.as_posix()}: env::var({match.group(1)})")
    return found


def check(root: Path = ROOT) -> int:
    found = own_variables(root)
    if found:
        print("Tests read environment variables of their own; use ketchup_test_env instead:")
        print(*found, sep="\n  ")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
