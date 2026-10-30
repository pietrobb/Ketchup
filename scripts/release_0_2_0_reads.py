#!/usr/bin/env python3
"""Kečup 0.2.0 reads what Save writes for it (review 2026-10-29 proposal 5.4, MD-60).

Save writes a document 0.2.0 can hold as format 103, and 0.2.0 reads 103. A file it
takes for damaged it does not refuse: it opens the older `.recovery` in its place, and
its next Save overwrites the newer file. Two checks keep that from happening again:

1. Variants. A serde enum of the saved model (`ketchup-model`, `ketchup-geometry`,
   `ketchup-tolerance`) that 0.2.0 has may gain a variant only if a document using it
   is not written as 103 (`fits_format_103`); `REVIEWED` lists those checked. A new
   unlisted variant or a removed one fails.
2. Reading. Every file the tests save, not a list picked to stand for them: a check
   relaxed since 0.2.0 changes no enum (review 2026-10-26 MD-41). With the feature
   `ketchup-model/saved-files` and `KETCHUP_SAVED_FILES_DIR` set, Save also writes each
   file there, and a reader built from `ketchup-model` of the `v0.2.0` tag
   (`release_0_2_0_reader.rs`) opens them: 103 in container schema 1 within 64 MiB must
   open, any other must be refused, and none may open its `.recovery` (a file with an
   entry over 32 MiB is written as container schema 2 for that). `--files DIR` opens the files
   a test run already saved in DIR (CI keeps them from its test step); without it the
   tests of `ketchup-model` run here to save them.

Needs the tag `v0.2.0` (`git fetch --depth=1 origin tag v0.2.0` in a shallow clone).
The old crates and their build go to `<target>/release-0.2.0`; `--variants-only` skips
the build.
"""

from __future__ import annotations

import io
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TAG = "v0.2.0"
SAVED_CRATES = ("crates/ketchup-model", "crates/ketchup-geometry", "crates/ketchup-tolerance")
# `ketchup-test-env` is a dev-dependency of the model: the workspace needs its manifest.
OLD_CRATES = (*SAVED_CRATES, "crates/ketchup-test-env")

# Variants added since 0.2.0 to its enums, each checked to keep a document out of
# format 103.
REVIEWED = {
    # MD-60: `fits_format_103` writes an import in one of these units as 104.
    "ImportLengthUnit": {"Decimetre", "Kilometre", "Mile", "Yard"},
}

ENUM = re.compile(
    r"#\[derive\(([^)]*)\)\]\s*(?:#\[[^\]]*\]\s*)*pub(?:\([^)]*\))?\s+enum\s+(\w+)[^{]*\{",
    re.S,
)


def git(*args: str, binary: bool = False) -> str | bytes:
    result = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, check=False)
    if result.returncode != 0:
        sys.exit(f"git {' '.join(args)}: {result.stderr.decode(errors='replace').strip()}")
    return result.stdout if binary else result.stdout.decode("utf-8")


def enum_body(source: str, start: int) -> str:
    depth, end = 1, start
    while depth:
        depth += {"{": 1, "}": -1}.get(source[end], 0)
        end += 1
    return source[start : end - 1]


def variants(body: str) -> set[str]:
    body = re.sub(r"//[^\n]*", "", body)
    body = re.sub(r"#\[[^\]]*\]", "", body)
    # Fields of struct and tuple variants, innermost first.
    while True:
        flat = re.sub(r"\{[^{}]*\}|\([^()]*\)", "", body)
        if flat == body:
            break
        body = flat
    names = (part.split("=")[0].strip() for part in body.split(","))
    return {name for name in names if re.fullmatch(r"[A-Z]\w*", name)}


def serde_enums(ref: str) -> dict[str, set[str]]:
    files = git("ls-tree", "-r", "--name-only", ref, *(f"{crate}/src" for crate in SAVED_CRATES))
    enums: dict[str, set[str]] = {}
    for path in files.split():
        if not path.endswith(".rs"):
            continue
        source = git("show", f"{ref}:{path}")
        for match in ENUM.finditer(source):
            if "Deserialize" in match.group(1):
                enums.setdefault(match.group(2), set()).update(
                    variants(enum_body(source, match.end()))
                )
    return enums


def variant_problems(old: dict[str, set[str]], new: dict[str, set[str]]) -> list[str]:
    problems = []
    for name, known in sorted(old.items()):
        current = new.get(name)
        if current is None:
            problems.append(f"{name}: 0.2.0 has this enum, HEAD no longer does")
            continue
        for variant in sorted(known - current):
            problems.append(f"{name}::{variant}: removed, so a file of 0.2.0 using it no longer opens")
        for variant in sorted(current - known - REVIEWED.get(name, set())):
            problems.append(
                f"{name}::{variant}: new since 0.2.0; keep a document using it out of format 103 "
                "(fits_format_103) and add it to REVIEWED"
            )
    return problems


def old_workspace(target: Path) -> Path:
    """The crates of the tag under `target`, in a workspace of their own and the reader."""
    source = target / "release-0.2.0"
    commit = git("rev-parse", f"{TAG}^{{commit}}").strip()
    marker = source / "commit"
    if not (marker.is_file() and marker.read_text().strip() == commit):
        shutil.rmtree(source, ignore_errors=True)
        archive = git("archive", TAG, "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", *OLD_CRATES, binary=True)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter="data")
        manifest = (source / "Cargo.toml").read_text(encoding="utf-8")
        members = ", ".join(f'"{crate}"' for crate in (*OLD_CRATES, "reader"))
        manifest = re.sub(r"^members = \[.*?\]$", f"members = [{members}]", manifest, flags=re.M | re.S)
        manifest = re.sub(r"^exclude = .*?$\n", "", manifest, flags=re.M)
        manifest = re.sub(r"^\[patch\.crates-io\]\n(?:[^\[\n].*\n)*", "", manifest, flags=re.M)
        (source / "Cargo.toml").write_text(manifest, encoding="utf-8")
        marker.write_text(commit)
    reader = source / "reader"
    (reader / "src").mkdir(parents=True, exist_ok=True)
    (reader / "Cargo.toml").write_text(
        '[package]\nname = "release-0-2-0-reader"\nversion = "0.0.0"\nedition = "2024"\n'
        'publish = false\n\n[dependencies]\nketchup-model = { path = "../crates/ketchup-model" }\n',
        encoding="utf-8",
    )
    main = reader / "src" / "main.rs"
    text = (ROOT / "scripts" / "release_0_2_0_reader.rs").read_text(encoding="utf-8")
    if not main.is_file() or main.read_text(encoding="utf-8") != text:
        main.write_text(text, encoding="utf-8")
    return source


def write_files(directory: Path) -> None:
    env = dict(os.environ, KETCHUP_SAVED_FILES_DIR=str(directory))
    subprocess.run(
        ["cargo", "test", "--locked", "-q", "-p", "ketchup-model", "--features", "ketchup-model/saved-files"],
        cwd=ROOT, env=env, check=True,
    )


def read_files(source: Path, files: Path) -> int:
    env = {key: value for key, value in os.environ.items() if key != "CARGO_TARGET_DIR"}
    result = subprocess.run(
        ["cargo", "run", "-q", "-p", "release-0-2-0-reader", "--target-dir", str(source / "target"), "--", files],
        cwd=source, env=env, check=False,
    )
    return result.returncode


def main(arguments: list[str]) -> int:
    problems = variant_problems(serde_enums(TAG), serde_enums("HEAD"))
    for problem in problems:
        print(f"variant: {problem}")
    if problems:
        return 1
    print(f"variants: the serde enums of {TAG} gained only reviewed variants")
    if "--variants-only" in arguments:
        return 0
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    source = old_workspace(target)
    if "--files" in arguments:
        return read_files(source, Path(arguments[arguments.index("--files") + 1]).resolve())
    with tempfile.TemporaryDirectory(prefix="ketchup-release-0.2.0-") as files:
        write_files(Path(files))
        return read_files(source, Path(files))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
