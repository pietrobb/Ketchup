#!/usr/bin/env python3
"""Fail when named products or bespoke shapes appear in core source.

Rust crates and the Python SDK may contain only generic geometry. Domain names
(bottle, teapot, cabinet, ...) belong in the program-language library, in
examples and in test fixtures. See AGENTS.md, section 1.

The check is a ratchet: scripts/named_products_baseline.txt records the
occurrences that still exist. A count may only go down; a new file or a higher
count fails. Run with --update after removing occurrences to shrink the
baseline. Never add entries by hand.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_files  # noqa: E402
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "named_products_baseline.txt"
# Inside every crate and Viewer app (scripts/source_roots.py), then the SDK and skills.
PACKAGE_SOURCES = ["src/**/*.rs", "src/**/*.cc", "src/**/*.hxx", "include/**/*.hxx"]
SOURCES = [*PACKAGE_SOURCES, "sdk/python/**/*.py", "skills/**/*.py"]
EXCLUDED_PARTS = {"tests", "examples", "fixtures"}
# Readers of old file formats must name the old fields and role categories they convert;
# they are the one exception.
OLD_FORMAT_READERS = {
    "crates/ketchup-model/src/document/digest_v3.rs",
    "crates/ketchup-model/src/persistence/legacy.rs",
    "crates/ketchup-model/src/persistence/snapshot_codec.rs",
}
# Validators against a design standard (EN 1995-1-1 for timber members) only read finished parts
# and create no geometry; a standard is not a product. Their files, and lines that name the
# standard or its program-language inputs, are the second exception.
STANDARD_VALIDATORS = {
    "crates/ketchup-program/src/load_path.rs",
    "crates/ketchup-program/src/loads.rs",
    "crates/ketchup-program/src/member_check.rs",
}
STANDARD_VALIDATOR_LINE = re.compile(r"^.*(?:EN 1995|timber_strength|timber_design|TimberClass).*$",
                                     re.MULTILINE)
# The Windows registry key holding the user's number format is named by the operating system.
OS_REGISTRY_LINE = re.compile(r"^.*Control Panel\\International.*$", re.MULTILINE)
WORDS = [
    "bottle", "teapot", "balloon_text", "balloon_glyph", "gable_roof", "staircase",
    "oriented_beam", "hettich", "quadro", "nightstand", "capsule", "d_profile",
    "squeeze", "ketchup_bottle", "drawer", "cabinet", "wardrobe", "dowel", "hinge",
    "shelf", "shelves", "furniture", "grain", "panel", "board", "beam", "timber", "lumber",
    "weldment", "cup_bore", "apron",
]
PATTERN = re.compile("(?<![A-Za-z])(?:" + "|".join(re.escape(word) for word in WORDS) + ")",
                     re.IGNORECASE)
# Building words from the 2026-10-07 review. They are short enough to start generic words
# (motion study, household, staircase), so they count only as a whole word or a
# CamelCase/snake_case part, singular or plural.
WHOLE_WORDS = ["stud", "rafter", "joist", "house", "roof", "stair"]
WHOLE_WORD_PATTERN = re.compile("(?<![A-Za-z])(" + "|".join(WHOLE_WORDS) + ")(?:e?s)?", re.IGNORECASE)


def whole_words(text: str):
    """The building words that end where a word or a CamelCase/CONSTANT part ends."""
    for match in WHOLE_WORD_PATTERN.finditer(text):
        following = text[match.end():match.end() + 1]
        if following.islower() or (match.group(0).isupper() and following.isalpha()):
            continue
        yield match.group(1).lower()
# Named shape types that a general operation replaced (sheet metal bends on any polygon edge,
# threads as any profile swept along a helix); they may not come back under any prefix.
NAMED_SHAPE_TYPES = re.compile(r"SheetMetalEdge|MAX_SHEET_METAL_FLANGES|ThreadProfile|CreateThread"
                               r"|CreateHelixPath|AppCommand::Thread\b")


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for pattern in SOURCES:
        for path in package_files(root, pattern) if pattern in PACKAGE_SOURCES else sorted(root.glob(pattern)):
            relative = path.relative_to(root)
            if (EXCLUDED_PARTS & set(relative.parts[:-1]) or path.stem.endswith("tests")
                    or relative.as_posix() in OLD_FORMAT_READERS | STANDARD_VALIDATORS):
                continue
            text = STANDARD_VALIDATOR_LINE.sub("", path.read_text(encoding="utf-8", errors="replace"))
            text = OS_REGISTRY_LINE.sub("", text)
            if path.suffix == ".rs":
                text = TEST_MODULE.sub("", text)
            for match in PATTERN.finditer(text):
                key = f"{relative.as_posix()}:{match.group(0).lower()}"
                counts[key] = counts.get(key, 0) + 1
            for word in whole_words(text):
                key = f"{relative.as_posix()}:{word}"
                counts[key] = counts.get(key, 0) + 1
            for match in NAMED_SHAPE_TYPES.finditer(text):
                key = f"{relative.as_posix()}:{match.group(0)}"
                counts[key] = counts.get(key, 0) + 1
    return counts


def read_baseline() -> dict[str, int]:
    baseline: dict[str, int] = {}
    if BASELINE.exists():
        for line in BASELINE.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline


def main() -> int:
    counts = current_counts()
    if "--update" in sys.argv:
        old = read_baseline()
        grown = {key: value for key, value in counts.items() if value > old.get(key, 0)}
        if old and grown and "--allow-growth" not in sys.argv:
            print("Refusing to grow the baseline:", *sorted(grown), sep="\n  ")
            return 1
        lines = [f"{key} {value}" for key, value in sorted(counts.items())]
        BASELINE.write_text("# Remaining named-product occurrences; may only shrink.\n"
                            + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        print(f"baseline: {sum(counts.values())} occurrences in {len(counts)} entries")
        return 0
    baseline = read_baseline()
    failures = [f"{key}: {value} (allowed {baseline.get(key, 0)})"
                for key, value in sorted(counts.items()) if value > baseline.get(key, 0)]
    if failures:
        print("Named products or bespoke shapes in core source (see AGENTS.md section 1):")
        print(*failures, sep="\n  ")
        return 1
    shrunk = sum(baseline.values()) - sum(counts.values())
    if shrunk > 0:
        print(f"OK; {shrunk} occurrences removed since the baseline. "
              "Run with --update to lock in the progress.")
    else:
        print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
