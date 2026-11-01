#!/usr/bin/env python3
"""Fail when a test that evaluates the large house runs in every step.

AGENTS.md §6: a test that needs the 750-part `tiny-house.star` is named
`large_house_*` (or `perf_*`, a release timing budget) and ignored, so the
filtered tests of a step skip it and the full suite and the nightly run take it.
Review 2026-10-18 P3 found unignored ones in `program_evaluation`, `takeoff_house`,
`free_space`, `member_design` and `member_loads`. Review 2026-10-29 TST-62: the
1 855-part `house-project.star` is a large house too.

A test reaches the house when its body, a constant it names or a function or
`macro_rules!` it calls holds `include_str!("…tiny-house.star")` or a path to a
house read any other way (`concat!`, `fs::read_to_string`, `.join("tiny-house.star")`).
`#[test]` counts above comment, doc and blank lines and on the `fn` line; an
`#[ignore]` in a comment does not. Functions are followed within
the file and, for `pub` helpers, across the files of the same crate; a function
defined in the file shadows one of the same name elsewhere. A function that lists
`examples/programs` with `read_dir` reaches the house too, unless it names
`tiny-house.star` (directly or through a constant) to set it apart (review
2026-10-24 TST-20: the golden test of every example read it in every step).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_files  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
# Both large houses: tiny-house.star (750 parts) and house-project.star (1 855 parts).
FILE = r"(?:tiny-house|house-project)\.star"
HOUSE = re.compile(rf'include_str!\(\s*"[^"]*{FILE}"\s*\)')
# A path to a house, read however (include_str!, concat!, fs::read_to_string, join):
# a literal with a directory before the file, or the bare file joined to a directory.
HOUSE_PATH = re.compile(rf'"[^"\n]*/{FILE}"|\.join\(\s*"{FILE}"')
CONSTANT = re.compile(rf"\b(?:const|static)\s+(\w+)\s*:[^=]*=[^;]*(?:\"[^\"\n]*/{FILE}\")")
# Attribute, comment, doc and blank lines above a fn (comments are blank in the masked
# text), and attributes on the fn line itself.
FUNCTION = re.compile(
    r"((?:^[ \t]*(?:#\[[^\n]*)?\n)*^[ \t]*(?:#\[[^\n]*?\][ \t]*)*)"
    r"(pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)", re.M)
MACRO = re.compile(r"((?:^[ \t]*(?:#\[[^\n]*)?\n)*)^[ \t]*macro_rules!\s*(\w+)\s*\{", re.M)
CALL = re.compile(r"\b(\w+)\s*(?:::<[^>]*>)?\s*\(")
MACRO_CALL = re.compile(r"\b(\w+)!\s*[(\[{]")
EXAMPLES = re.compile(r'"[^"\n]*examples/programs[^"\n]*"')
NAMED = re.compile(rf'"[^"\n]*{FILE}"')
NAMING = re.compile(rf'\b(?:const|static)\s+(\w+)\s*:[^=]*=[^;]*"[^"\n]*{FILE}"')
ALLOWED = ("large_house_", "perf_")
# A whole-house test run in release by its own nightly step keeps its name.
RELEASE_ONLY = "release-only"
# Matched on the masked attributes, so an #[ignore] in a comment does not count;
# the reason is then read from the same place in the text.
IGNORE = re.compile(r'#\[ignore(?:\s*=\s*("[^"]*"))?\]')
TEST = re.compile(r"#\[test\]")


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
    """Every `fn` with a body and every `macro_rules!`: name, attributes, visibility, raw body, line."""
    plain = masked(text)
    pairs = closing_braces(plain)
    found = []

    def add(match: re.Match, name_group: int, start: int, public: bool) -> None:
        end = pairs[start]
        code = plain[start:end + 1]
        found.append({
            "name": match.group(name_group),
            "attributes": text[match.start(1):match.end(1)],
            "plain_attributes": plain[match.start(1):match.end(1)],
            "public": public,
            "body": text[start:end + 1],
            "code": code,
            "calls": set(CALL.findall(code)) | set(MACRO_CALL.findall(code)),
            "line": text.count("\n", 0, match.start(name_group)) + 1,
            "span": (start, end + 1),
        })

    for match in FUNCTION.finditer(plain):
        start = plain.find("{", match.end())
        semicolon = plain.find(";", match.end())
        if start < 0 or 0 <= semicolon < start or start not in pairs:
            continue
        add(match, 3, start, bool(match.group(2)))
    for match in MACRO.finditer(plain):
        start = match.end() - 1
        if start in pairs:
            add(match, 2, start, "macro_export" in plain[match.start(1):match.end(1)])
    return found


def holds_the_house(item: dict, constants: set[str]) -> bool:
    """An `include_str!` of a house or a path to one in code (not a comment), or a constant holding it."""
    if any(item["code"].startswith("include_str!(", found.start()) for found in HOUSE.finditer(item["body"])):
        return True
    if any(item["code"][found.start()] == item["body"][found.start()] for found in HOUSE_PATH.finditer(item["body"])):
        return True
    return any(re.search(rf"\b{re.escape(name)}\b", item["code"]) for name in constants)


def lists_the_house(item: dict, naming: set[str]) -> bool:
    """A `read_dir` of `examples/programs` (in code) that does not name the house to set it apart."""
    def in_code(pattern: re.Pattern) -> bool:
        return any(item["code"][found.start()] == '"' for found in pattern.finditer(item["body"]))
    if not re.search(r"\bread_dir\b", item["code"]) or not in_code(EXAMPLES):
        return False
    return not (in_code(NAMED) or any(re.search(rf"\b{re.escape(name)}\b", item["code"]) for name in naming))


def crate_of(path: Path, root: Path) -> str:
    return path.relative_to(root).parts[1]


def violations(root: Path = ROOT) -> list[str]:
    files = {}
    for path in package_files(root):
        if "target" in path.relative_to(root).parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        constants = set(CONSTANT.findall(text))
        naming = set(NAMING.findall(text))
        items = functions(text)
        for item in items:
            item["house"] = holds_the_house(item, constants) or lists_the_house(item, naming)
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
            if not item["house"] or not TEST.search(item["plain_attributes"]):
                continue
            ignore = IGNORE.search(item["plain_attributes"])
            reason = item["attributes"][ignore.start(1) + 1:ignore.end(1) - 1] if ignore and ignore.group(1) else ""
            if not ignore:
                found.append(f"{path.relative_to(root).as_posix()}:{item['line']} {item['name']}")
            elif not (item["name"].startswith(ALLOWED) or reason.startswith(RELEASE_ONLY)):
                found.append(f"{path.relative_to(root).as_posix()}:{item['line']} {item['name']}")
    return found


def check(root: Path = ROOT) -> int:
    found = violations(root)
    if found:
        print("Tests evaluate a large house (tiny-house.star, house-project.star) in every step; name them large_house_* and mark them"
              ' #[ignore = "large-house: runs in full suite"], or use tests/fixtures/small-house.star'
              " (AGENTS.md section 6):")
        print("  " + "\n  ".join(found))
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
