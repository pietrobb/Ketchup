"""The Rust packages every repository check scans: the workspace crates and the
Viewer apps (review 2026-10-31 TST-80: four checks skipped viewer/*)."""
from __future__ import annotations

import functools
import tomllib
from pathlib import Path

PACKAGE_ROOTS = ("crates", "viewer")


@functools.cache
def _package_name(directory: Path) -> str:
    manifest = tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8"))
    return manifest["package"]["name"]


def package_of(root: Path, path: Path) -> str:
    """The Cargo package name of the package directory `path` lies in."""
    relative = path.relative_to(root)
    directory = root / relative.parts[0] / relative.parts[1]
    if (directory / "Cargo.toml").exists():
        return _package_name(directory)
    return relative.parts[1]


def package_files(root: Path, pattern: str = "**/*.rs") -> list[Path]:
    """The files matching `pattern` inside each package directory under the package
    roots (`src/**/*.rs`, `**/*.rs`, `include/**/*.hxx` …), build output left out."""
    found = set()
    for package_root in PACKAGE_ROOTS:
        for path in (root / package_root).glob(f"*/{pattern}"):
            if "target" not in path.relative_to(root).parts:
                found.add(path)
    return sorted(found)
