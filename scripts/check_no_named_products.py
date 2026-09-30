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
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "named_products_baseline.txt"
SOURCES = ["crates/*/src/**/*.rs", "crates/*/src/**/*.cc", "crates/*/src/**/*.hxx", "crates/*/include/**/*.hxx",
           "sdk/python/**/*.py", "skills/**/*.py"]
EXCLUDED_PARTS = {"tests", "examples", "fixtures"}
# Readers of old file formats must name the old fields and role categories they convert;
# they are the one exception.
OLD_FORMAT_READERS = {
    "crates/ketchup-model/src/document/digest_v3.rs",
    "crates/ketchup-model/src/persistence/legacy.rs",
    "crates/ketchup-model/src/persistence/snapshot_codec.rs",
}
WORDS = [
    "bottle", "teapot", "balloon_text", "balloon_glyph", "gable_roof", "staircase",
    "oriented_beam", "hettich", "quadro", "nightstand", "capsule", "d_profile",
    "squeeze", "ketchup_bottle", "drawer", "cabinet", "wardrobe", "dowel", "hinge",
    "shelf", "shelves", "furniture",
]
PATTERN = re.compile("(?<![A-Za-z])(?:" + "|".join(re.escape(word) for word in WORDS) + ")",
                     re.IGNORECASE)


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for pattern in SOURCES:
        for path in sorted(root.glob(pattern)):
            relative = path.relative_to(root)
            if (EXCLUDED_PARTS & set(relative.parts[:-1]) or path.stem.endswith("tests")
                    or relative.as_posix() in OLD_FORMAT_READERS):
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            if path.suffix == ".rs":
                text = TEST_MODULE.sub("", text)
            for match in PATTERN.finditer(text):
                key = f"{relative.as_posix()}:{match.group(0).lower()}"
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
