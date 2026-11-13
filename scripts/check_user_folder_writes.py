#!/usr/bin/env python3
"""Fail when code outside `ketchup-user-files` gains writes into the user's folders.

Under `sudo` root writing the user's files through their path follows the user's links
and lands in root's folders (reviews 2026-11-13 R1113-SEC-600..602 and r95
R1113r95-SEC-700/701, P3 register of windows). `ketchup-user-files` writes them as the
user who holds the way to them and opens no file through a link; the rest of the code
reaches the user's folders through it (`UserFolder::as_owner`, `side_file`,
`new_side_file`, `append_private`, `create_private_folder`).

A production source file that names a folder of the user's (`HOME`, an `XDG_*`
folder, `TMPDIR`, `APPDATA`, `LOCALAPPDATA`, `USERPROFILE`, `CLAUDE_CONFIG_DIR`,
`CODEX_HOME`, or a `UserFolder`) counts its direct writes: `fs::write`, `File::create`,
`OpenOptions`, `DirBuilder`, `create_dir_all`, `fs::rename`, `persist`/`persist_noclobber`.
Test code is not counted.

The check is a ratchet: scripts/user_folder_writes_baseline.txt records the writes that
still exist per file. A count may only go down; a new file or a higher count fails. Run
with --update after moving writes into the crate to shrink the baseline. Never add
entries by hand.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_files  # noqa: E402
from check_tolerance_literals import TEST_BUILD_MODULE as TEST_MODULE  # noqa: E402  (a Rust file's unit tests)
from check_large_house_tests import masked  # noqa: E402  (comments and strings blanked)

ROOT = Path(__file__).resolve().parents[1]
BASELINE_NAME = "user_folder_writes_baseline.txt"
# The crate that writes the user's files; its own writes are the point.
OWN_CRATE = "crates/ketchup-user-files/"
USER_FOLDERS = re.compile(
    r'"(?:HOME|XDG_[A-Z_]+|TMPDIR|APPDATA|LOCALAPPDATA|USERPROFILE|CLAUDE_CONFIG_DIR|CODEX_HOME)"'
    r"|\bUserFolder\b")
WRITES = re.compile(
    r"\bfs::write\s*\(|\bFile::create\s*\(|\bOpenOptions\b|\bDirBuilder\b|\bcreate_dir_all\s*\("
    r"|\bfs::rename\s*\(|\.persist(?:_noclobber)?\s*\(")


def is_test_file(relative: Path) -> bool:
    return "tests" in relative.parts[:-1] or relative.stem == "tests" or relative.stem.endswith("_tests")


def writes(source: str) -> int:
    """The direct writes of the production code of `source`, when it names a folder of
    the user's; none otherwise."""
    production = TEST_MODULE.sub("", source)
    if not USER_FOLDERS.search(production):
        return 0
    # `use std::fs::{self, OpenOptions}` imports, it writes nothing.
    plain = "\n".join(line for line in masked(production).splitlines() if not line.lstrip().startswith("use "))
    return len(WRITES.findall(plain))


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted(package_files(root)):
        relative = path.relative_to(root)
        if "target" in relative.parts or relative.as_posix().startswith(OWN_CRATE) or is_test_file(relative):
            continue
        count = writes(path.read_text(encoding="utf-8", errors="replace"))
        if count:
            counts[relative.as_posix()] = count
    return counts


def read_baseline(path: Path) -> dict[str, int]:
    baseline: dict[str, int] = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline


def check(root: Path = ROOT, update: bool = False, allow_growth: bool = False) -> int:
    counts = current_counts(root)
    baseline_path = root / "scripts" / BASELINE_NAME
    baseline = read_baseline(baseline_path)
    grown = [f"{key}: {value} (allowed {baseline.get(key, 0)})"
             for key, value in sorted(counts.items()) if value > baseline.get(key, 0)]
    if update:
        if baseline and grown and not allow_growth:
            print("Refusing to grow the baseline:", *grown, sep="\n  ")
            return 1
        lines = [f"{key} {value}" for key, value in sorted(counts.items())]
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text("# Direct writes into the user's folders outside ketchup-user-files; may only shrink.\n"
                                 + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        print(f"baseline: {sum(counts.values())} writes in {len(counts)} files")
        return 0
    if grown:
        print("Code gained direct writes into the user's folders; write them through ketchup-user-files"
              " (UserFolder::as_owner, side_file, new_side_file, append_private, create_private_folder):")
        print(*grown, sep="\n  ")
        return 1
    shrunk = sum(baseline.values()) - sum(counts.values())
    if shrunk > 0:
        print(f"OK; {shrunk} writes moved into ketchup-user-files since the baseline."
              " Run with --update to lock in the progress.")
    else:
        print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check(update="--update" in sys.argv, allow_growth="--allow-growth" in sys.argv))
