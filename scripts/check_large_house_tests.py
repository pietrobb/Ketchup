#!/usr/bin/env python3
"""Fail when a test that evaluates the large house runs in every step.

AGENTS.md §6: a test that needs the 750-part `tiny-house.star` is named
`large_house_*` (or `perf_*`, a release timing budget) and ignored, so the
filtered tests of a step skip it and the full suite and the nightly run take it.
Review 2026-10-18 P3 found unignored ones in `program_evaluation`, `takeoff_house`,
`free_space`, `member_design` and `member_loads`.

A test reaches the house when its body, a constant it names or a function it
calls holds `include_str!("…tiny-house.star")`. Functions are followed within
the file and, for `pub` helpers, across the files of the same crate; a function
defined in the file shadows one of the same name elsewhere.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HOUSE = re.compile(r'include_str!\(\s*"[^"]*tiny-house\.star"\s*\)')
CONSTANT = re.compile(r"\b(?:const|static)\s+(\w+)\s*:[^=]*=\s*include_str!\(\s*\"[^\"]*tiny-house\.star\"")
FUNCTION = re.compile(
    r"((?:^[ \t]*#\[[^\n]*\n|^[ \t]*///[^\n]*\n)*)^[ \t]*(pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)", re.M)
CALL = re.compile(r"\b(\w+)\s*(?:::<[^>]*>)?\s*\(")
ALLOWED = ("large_house_", "perf_")
# A whole-house test run in release by its own nightly step keeps its name.
RELEASE_ONLY = "release-only"
IGNORE = re.compile(r'#\[ignore(?:\s*=\s*"([^"]*)")?\]')


IGNORED_TEXT = re.compile(r"//[^\n]*|\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^\\'\n])'")
BRACE = re.compile(r"[{}]")


def masked(text: str) -> str:
    """`text` with comments and the insides of strings and chars blanked, lengths kept."""
    def blank(match: re.Match) -> str:
        found = match.group(0)
        if found.startswith("//"):
            return " " * len(found)
        return found[0] + " " * (len(found) - 2) + found[-1]
    return IGNORED_TEXT.sub(blank, text)


def closing_braces(plain: str) -> dict[int, int]:
    """Where each `{` of `plain` is closed."""
    pairs, open_at = {}, []
    for brace in BRACE.finditer(plain):
        if brace.group(0) == "{":
            open_at.append(brace.start())
        elif open_at:
            pairs[open_at.pop()] = brace.start()
    return pairs


def functions(text: str) -> list[dict]:
    """Every `fn` with a body: name, attributes, visibility, raw body, line."""
    plain = masked(text)
    pairs = closing_braces(plain)
    found = []
    for match in FUNCTION.finditer(plain):
        start = plain.find("{", match.end())
        semicolon = plain.find(";", match.end())
        if start < 0 or 0 <= semicolon < start or start not in pairs:
            continue
        end = pairs[start]
        found.append({
            "name": match.group(3),
            "attributes": text[match.start(1):match.end(1)],
            "public": bool(match.group(2)),
            "body": text[start:end + 1],
            "code": plain[start:end + 1],
            "calls": set(CALL.findall(plain[start:end + 1])),
            "line": text.count("\n", 0, match.start(3)) + 1,
        })
    return found


def holds_the_house(item: dict, constants: set[str]) -> bool:
    """An `include_str!` of the house in code (not a comment), or a constant holding it."""
    if any(item["code"].startswith("include_str!(", found.start()) for found in HOUSE.finditer(item["body"])):
        return True
    return any(re.search(rf"\b{re.escape(name)}\b", item["code"]) for name in constants)


def crate_of(path: Path, root: Path) -> str:
    return path.relative_to(root).parts[1]


def violations(root: Path = ROOT) -> list[str]:
    files = {}
    for path in sorted(root.glob("crates/*/**/*.rs")):
        if "target" in path.relative_to(root).parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        constants = set(CONSTANT.findall(text))
        items = functions(text)
        for item in items:
            item["house"] = holds_the_house(item, constants)
        files[path] = items
    # Follow calls until nothing new reaches the house.
    changed = True
    while changed:
        changed = False
        exported: dict[str, set[str]] = {}
        for path, items in files.items():
            for item in items:
                if item["house"] and item["public"]:
                    exported.setdefault(crate_of(path, root), set()).add(item["name"])
        for path, items in files.items():
            local = {item["name"]: item for item in items}
            shared = exported.get(crate_of(path, root), set())
            for item in items:
                if item["house"]:
                    continue
                if any(local[name]["house"] if name in local else name in shared for name in item["calls"]):
                    item["house"] = changed = True
    found = []
    for path, items in files.items():
        for item in items:
            if not item["house"] or "#[test]" not in item["attributes"]:
                continue
            reason = IGNORE.search(item["attributes"])
            if not reason:
                found.append(f"{path.relative_to(root).as_posix()}:{item['line']} {item['name']}")
            elif not (item["name"].startswith(ALLOWED) or (reason.group(1) or "").startswith(RELEASE_ONLY)):
                found.append(f"{path.relative_to(root).as_posix()}:{item['line']} {item['name']}")
    return found


def check(root: Path = ROOT) -> int:
    found = violations(root)
    if found:
        print("Tests evaluate the 750-part tiny-house.star in every step; name them large_house_* and mark them"
              ' #[ignore = "large-house: runs in full suite"], or use tests/fixtures/small-house.star'
              " (AGENTS.md section 6):")
        print("  " + "\n  ".join(found))
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
