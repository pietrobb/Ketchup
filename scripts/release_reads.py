#!/usr/bin/env python3
"""Every released Kečup reads what Save writes (review 2026-10-29 proposal 5.4, MD-60;
review 2026-10-31 CI-80).

A released version opens a file it can hold, and refuses a newer one as newer. A file it
takes for damaged it does not refuse: it opens the older `.recovery` in its place, and
its next Save overwrites the newer file. For each tag of `RELEASES` two checks keep that
from happening:

1. Variants. A serde enum of the saved model (`ketchup-model`, `ketchup-geometry`,
   `ketchup-tolerance`) that the release has may gain a variant only if a document using
   it is written in a format newer than the release reads (for 0.2.0 `fits_format_103`);
   `REVIEWED` lists those checked per release. A new unlisted variant or a removed one
   fails.
2. Reading. Every file the tests save, not a list picked to stand for them: a check
   relaxed since the release changes no enum (review 2026-10-26 MD-41). With the feature
   `ketchup-model/saved-files` and `KETCHUP_SAVED_FILES_DIR` set, Save also writes each
   file there, and a reader built from `ketchup-model` of the tag (`release_reader.rs`)
   opens them: a file in a format and container schema the release reads, within its
   file size, must open; any other must be refused as newer or as larger; none may open
   its `.recovery`. What the release reads is taken from its own source. `--files DIR`
   opens the files a test run already saved in DIR (CI keeps them from its test step);
   without it the tests of `ketchup-model` run here to save them.

`--tag vX` checks one release instead of all. Needs the tags (`git fetch --no-tags
--depth=1 origin tag v0.2.0 tag v0.3.0` in a shallow clone). The old crates and their
build go to `<target>/release-<version>`; `--variants-only` skips the build.
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
# Every release whose files Save may still write: add a tag here when it is published.
RELEASES = ("v0.2.0", "v0.3.0")
SAVED_CRATES = ("crates/ketchup-model", "crates/ketchup-geometry", "crates/ketchup-tolerance")
# `ketchup-test-env` is a dev-dependency of the model: the workspace needs its manifest.
OLD_CRATES = (*SAVED_CRATES, "crates/ketchup-test-env")

# Variants added since a release to its enums, each checked to keep a document using it
# out of the formats that release reads.
REVIEWED = {
    "v0.2.0": {
        # MD-60: `fits_format_103` writes an import in one of these units as 104.
        "ImportLengthUnit": {"Decimetre", "Kilometre", "Mile", "Yard"},
    },
    "v0.3.0": {
        # CD-61: `format_of` writes a document with a stretched line as 107.
        "AddedObject": {"Stretched"},
        # The program worker's answer to a drawing export, never part of a document.
        "DxfEditRefusal": {"ReadDifferently", "TooLarge"},
    },
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


def version(tag: str) -> str:
    return tag.removeprefix("v")


def variant_problems(
    old: dict[str, set[str]],
    new: dict[str, set[str]],
    reviewed: dict[str, set[str]],
    release: str = "0.2.0",
) -> list[str]:
    problems = []
    for name, known in sorted(old.items()):
        current = new.get(name)
        if current is None:
            problems.append(f"{name}: {release} has this enum, HEAD no longer does")
            continue
        for variant in sorted(known - current):
            problems.append(f"{name}::{variant}: removed, so a file of {release} using it no longer opens")
        for variant in sorted(current - known - reviewed.get(name, set())):
            problems.append(
                f"{name}::{variant}: new since {release}; keep a document using it out of the "
                f"formats {release} reads and add it to REVIEWED"
            )
    return problems


def product(expression: str) -> int:
    """The value of a constant written as a product of integers, e.g. `64 * 1024 * 1024`."""
    value = 1
    for factor in expression.split("*"):
        value *= int(factor.strip().replace("_", ""))
    return value


def what_it_reads(tag: str) -> list[str]:
    """The reader's arguments: the newest format, the container schemas and the file
    size (0 for none) the release reads, from its own source."""
    codec = git("show", f"{tag}:crates/ketchup-model/src/persistence/snapshot_codec.rs")
    persistence = git("show", f"{tag}:crates/ketchup-model/src/persistence.rs")
    newest = re.search(r"const SNAPSHOT_FORMAT: u16 = (\d+);", codec)
    schemas = re.findall(r"const \w*CONTAINER_SCHEMA: u16 = (\d+);", persistence)
    if not newest or not schemas:
        sys.exit(f"{tag}: no SNAPSHOT_FORMAT or CONTAINER_SCHEMA in its persistence")
    limit = re.search(r"const MAX_NATIVE_DOCUMENT_BYTES: usize = ([\d_ *]+);", persistence)
    return [newest.group(1), ",".join(schemas), str(product(limit.group(1))) if limit else "0"]


def old_workspace(tag: str, target: Path) -> Path:
    """The crates of the tag under `target`, in a workspace of their own and the reader."""
    source = target / f"release-{version(tag)}"
    commit = git("rev-parse", f"{tag}^{{commit}}").strip()
    marker = source / "commit"
    if not (marker.is_file() and marker.read_text().strip() == commit):
        shutil.rmtree(source, ignore_errors=True)
        archive = git("archive", tag, "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", *OLD_CRATES, binary=True)
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
        '[package]\nname = "release-reader"\nversion = "0.0.0"\nedition = "2024"\n'
        'publish = false\n\n[dependencies]\nketchup-model = { path = "../crates/ketchup-model" }\n',
        encoding="utf-8",
    )
    main = reader / "src" / "main.rs"
    text = (ROOT / "scripts" / "release_reader.rs").read_text(encoding="utf-8")
    if not main.is_file() or main.read_text(encoding="utf-8") != text:
        main.write_text(text, encoding="utf-8")
    return source


def write_files(directory: Path) -> None:
    env = dict(os.environ, KETCHUP_SAVED_FILES_DIR=str(directory))
    subprocess.run(
        ["cargo", "test", "--locked", "-q", "-p", "ketchup-model", "--features", "ketchup-model/saved-files"],
        cwd=ROOT, env=env, check=True,
    )


def read_files(tag: str, source: Path, files: Path) -> int:
    env = {key: value for key, value in os.environ.items() if key != "CARGO_TARGET_DIR"}
    print(f"Kečup {version(tag)} opens the saved files:", flush=True)
    result = subprocess.run(
        ["cargo", "run", "-q", "-p", "release-reader", "--target-dir", str(source / "target"),
         "--", files, *what_it_reads(tag)],
        cwd=source, env=env, check=False,
    )
    return result.returncode


def main(arguments: list[str]) -> int:
    # A Windows console (CI's included) writes cp1252, which has no "č" in "Kečup".
    sys.stdout.reconfigure(encoding="utf-8")
    tags = [arguments[arguments.index("--tag") + 1]] if "--tag" in arguments else list(RELEASES)
    failed = False
    for tag in tags:
        problems = variant_problems(
            serde_enums(tag), serde_enums("HEAD"), REVIEWED.get(tag, {}), version(tag)
        )
        for problem in problems:
            print(f"variant: {problem}")
        if problems:
            failed = True
        else:
            print(f"variants: the serde enums of {tag} gained only reviewed variants")
    if failed or "--variants-only" in arguments:
        return int(failed)
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    with tempfile.TemporaryDirectory(prefix="ketchup-saved-files-") as written:
        if "--files" in arguments:
            files = Path(arguments[arguments.index("--files") + 1]).resolve()
        else:
            files = Path(written)
            write_files(files)
        results = [read_files(tag, old_workspace(tag, target), files) for tag in tags]
    return 1 if any(results) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
