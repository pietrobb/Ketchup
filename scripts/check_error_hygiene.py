#!/usr/bin/env python3
"""Fail when crate source gains an error that throws away its cause (AGENTS.md §2).

Two forms are counted in production source (crates/*/src, test modules and
*tests.rs files excluded):

- discard: `map_err(|_| ...)` or `map_err(|_name| ...)` drops the error it
  receives. Only an error type that carries no content may be dropped, and the
  closure must name it: `map_err(|_: std::num::TryFromIntError| ...)`. The
  content-free types are listed in CONTENT_FREE; any other typed discard counts.
- string: `Err("...".to_owned())`, `.to_string()` or `.into()` answers with bare
  text instead of a typed error or `ketchup_rejection::Rejection`.
- unreachable: `unreachable!(...)` admits that a type allows a case the code
  says cannot happen; shape the type so the case does not exist instead.
- silent_unreachable: `unreachable!()` with no message also hides which
  assumption broke when it fires.
- string_result: `Result<_, String>` (or `Ok::<_, String>`) makes every caller unable to tell one
  failure from another.
- stringified_cause: `map_err(|e| e.to_string())` flattens a typed error into
  text, so the caller can no longer react to its kind.
- formatted_variant: `Variant(format!(...))` puts the facts of an error into a
  sentence instead of fields (`Some`, `Ok` and `Err` are not error variants;
  `Err(format!)` is already a string_result).
- unwrap: `.unwrap()` panics without saying which assumption broke.

The check is a ratchet: scripts/error_hygiene_baseline.txt records what still
exists, per file and form. A count may only go down; a new file or a higher count
fails. Run with --update after removing occurrences to shrink the baseline.
Never add entries by hand.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE_NAME = "scripts/error_hygiene_baseline.txt"
SOURCES = ["crates/*/src/**/*.rs"]
EXCLUDED_PARTS = {"tests", "examples", "fixtures"}
# Errors whose value says nothing beyond their type: dropping them loses nothing.
# An entry matches the named type by its trailing path, so "fmt::Error" accepts
# `std::fmt::Error` but not an unrelated `io::Error`.
CONTENT_FREE = {
    "TryFromIntError", "TryFromSliceError", "PoisonError", "TryLockError", "SendError",
    "TrySendError", "RecvError", "TryRecvError", "RecvTimeoutError", "Infallible",
    "fmt::Error",
    # A float parse failure only says the token was empty or not a number.
    "ParseFloatError",
    # An integer parse failure only says the digits were empty, invalid or out of range.
    "ParseIntError",
    # Says only that the path does not start with the prefix.
    "StripPrefixError",
    # wgpu reports a failed buffer map as a unit struct.
    "wgpu::BufferAsyncError",
    # Opaque by design: a failed signature check must not reveal why it failed.
    "ed25519_dalek::SignatureError",
}
DISCARD = re.compile(r"map_err\(\s*(?:move\s*)?\|\s*_\w*\s*(?::\s*([^|]+?)\s*)?\|")
STRING = re.compile(r"""Err\(\s*"(?:[^"\\]|\\.)*"\s*\.\s*(?:to_owned|to_string|into)\(\)\s*\)""")
UNREACHABLE = re.compile(r"\bunreachable!\s*\(")
SILENT_UNREACHABLE = re.compile(r"\bunreachable!\s*\(\s*\)")
# The Ok type may itself be generic up to two levels deep: Result<Vec<Option<T>>, String>.
# A turbofish `Ok::<_, String>(...)` names the same text error type.
STRING_RESULT = re.compile(r"\b(?:Result|(?:Ok|Err)::)<(?:[^<>,]|<[^<>]*(?:<[^<>]*>[^<>]*)*>)+,\s*String\s*>")
STRINGIFIED_CAUSE = re.compile(
    r"map_err\(\s*(?:move\s*)?\|\s*(\w+)\s*\|\s*\1\s*\.\s*to_string\(\)\s*\)")
FORMATTED_VARIANT = re.compile(r"\b(?!(?:Some|Ok|Err)\()[A-Z]\w*\(\s*format!\(")
UNWRAP = re.compile(r"\.unwrap\(\)")
# An inline `#[cfg(test)] mod name { ... }` at the top level of a Rust file.
TEST_MODULE = re.compile(r"^#\[cfg\(test\)\]\s*\n(?:#\[[^\n]*\]\s*\n)*mod \w+ \{\n.*?^\}",
                         re.MULTILINE | re.DOTALL)


def content_free(type_text: str) -> bool:
    path = type_text.split("<", 1)[0].strip()
    return any(path == entry or path.endswith("::" + entry) for entry in CONTENT_FREE)


def file_counts(text: str) -> dict[str, int]:
    text = TEST_MODULE.sub("", text)
    discards = sum(1 for match in DISCARD.finditer(text)
                   if match.group(1) is None or not content_free(match.group(1)))
    return {"discard": discards, "string": len(STRING.findall(text)),
            "unreachable": len(UNREACHABLE.findall(text)),
            "silent_unreachable": len(SILENT_UNREACHABLE.findall(text)),
            "string_result": len(STRING_RESULT.findall(text)),
            "stringified_cause": len(STRINGIFIED_CAUSE.findall(text)),
            "formatted_variant": len(FORMATTED_VARIANT.findall(text)),
            "unwrap": len(UNWRAP.findall(text))}


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for pattern in SOURCES:
        for path in sorted(root.glob(pattern)):
            relative = path.relative_to(root)
            if EXCLUDED_PARTS & set(relative.parts[:-1]) or path.stem.endswith("tests"):
                continue
            for form, found in file_counts(path.read_text(encoding="utf-8", errors="replace")).items():
                if found:
                    counts[f"{relative.as_posix()} {form}"] = found
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
            "# Remaining errors that drop their cause or cases the types still allow"
            " (<file> <form> <count>, forms in scripts/check_error_hygiene.py);"
            " may only shrink.\n"
            + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        print(f"baseline: {sum(counts.values())} occurrences in {len(counts)} entries")
        return 0
    failures = [f"{key}: {value} (allowed {old.get(key, 0)})"
                for key, value in sorted(counts.items()) if value > old.get(key, 0)]
    if failures:
        print("New errors that drop their cause; carry the source error or return a "
              "ketchup_rejection::Rejection (AGENTS.md §2):")
        print(*failures, sep="\n  ")
        return 1
    shrunk = sum(old.values()) - sum(counts.values())
    if shrunk > 0:
        print(f"OK; {shrunk} occurrences removed since the baseline. "
              "Run with --update to lock in the progress.")
    else:
        print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check(update="--update" in sys.argv, allow_growth="--allow-growth" in sys.argv))
