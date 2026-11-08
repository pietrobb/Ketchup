#!/usr/bin/env python3
"""Fail when production code gains a panic or a write to stderr that clippy sees.

clippy's `unwrap_used`, `expect_used`, `panic` and `print_stderr` find what a text check
missed (review 2026-11-08 TST-250): `.map(Result::unwrap)`, `Err(e) => panic!(..)`,
`eprint!`, and `eprintln!` inside a `macro_rules!` or a closure. They cannot be denied
outright while the code still holds them, so this check runs clippy with them as
warnings on the production targets (`--lib --bins`, no tests) of the packages it is given
and compares the count per file and lint with the baseline of the system it runs on,
scripts/clippy_panics_baseline_<system>.txt. clippy counts only the code the system
compiles, so a package with code for one system only (the ACL of ketchup-mcp's
credential files on Windows) is counted on each system, against that system's counts
(review 2026-11-08 CI-250). A count may only go down; only the packages of the run are
compared, so the job without OCCT and the one with it each check their own. Run with
--update after removing occurrences; it never grows a count.

A command line's entry point (`src/main.rs`, `src/bin/`) may write to stderr: that is
where it reports. `scripts/check_error_hygiene.py` holds the one of a window to its log.

    python scripts/check_clippy_panics.py [--update] -p ketchup-model -p ketchup-app ...
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_of  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SYSTEMS = {"win32": "windows", "linux": "linux", "darwin": "macos"}
SYSTEM = SYSTEMS.get(sys.platform, sys.platform)
LINTS = ("unwrap_used", "expect_used", "panic", "print_stderr")


def clippy_command(packages: list[str]) -> list[str]:
    selected = [argument for package in packages for argument in ("-p", package)]
    warnings = [argument for lint in LINTS for argument in ("-W", f"clippy::{lint}")]
    return ["cargo", "clippy", "--locked", *selected, "--lib", "--bins",
            "--message-format=json", "--", *warnings]


def entry_point(relative: Path) -> bool:
    return relative.name == "main.rs" or "bin" in relative.parts[:-1]


def counts_of(messages: list[str], root: Path) -> dict[str, int]:
    """`<file> <lint>` to the number of places clippy names, each place once (a module
    the lib and a bin both compile is reported twice)."""
    places = set()
    for line in messages:
        if not line.startswith("{"):
            continue
        record = json.loads(line)
        if record.get("reason") != "compiler-message":
            continue
        message = record["message"]
        code = (message.get("code") or {}).get("code") or ""
        lint = code.removeprefix("clippy::")
        if lint not in LINTS:
            continue
        for span in message["spans"]:
            if not span["is_primary"]:
                continue
            path = Path(span["file_name"])
            path = path if path.is_absolute() else root / path
            try:
                relative = path.resolve().relative_to(root.resolve())
            except ValueError:
                continue  # a dependency's file
            if lint == "print_stderr" and entry_point(relative):
                continue
            places.add((relative.as_posix(), lint, span["line_start"], span["column_start"]))
    counts: dict[str, int] = {}
    for file, lint, _, _ in places:
        counts[f"{file} {lint}"] = counts.get(f"{file} {lint}", 0) + 1
    return counts


PACKAGES_LINE = "# packages: "


def baseline_path(root: Path, system: str) -> Path:
    return root / "scripts" / f"clippy_panics_baseline_{system}.txt"


def read_baseline(root: Path, system: str) -> tuple[dict[str, int], set[str]]:
    """The counts on a system, and the packages they were recorded for."""
    baseline: dict[str, int] = {}
    recorded: set[str] = set()
    path = baseline_path(root, system)
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith(PACKAGES_LINE):
                recorded = set(line[len(PACKAGES_LINE):].split())
            elif line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline, recorded


def in_packages(key: str, packages: set[str], root: Path) -> bool:
    return package_of(root, root / key.rsplit(" ", 1)[0]) in packages


def compare(counts: dict[str, int], baseline: dict[str, int], packages: set[str],
            root: Path) -> list[str]:
    return [f"{key}: {value} (allowed {baseline.get(key, 0)})"
            for key, value in sorted(counts.items())
            if in_packages(key, packages, root) and value > baseline.get(key, 0)]


def updated(counts: dict[str, int], baseline: dict[str, int], packages: set[str],
            root: Path) -> dict[str, int]:
    """The baseline with the run's packages at their counts now; the others kept."""
    kept = {key: value for key, value in baseline.items()
            if not in_packages(key, packages, root)}
    return {**kept, **{key: value for key, value in counts.items()
                       if in_packages(key, packages, root)}}


def write_baseline(root: Path, system: str, baseline: dict[str, int],
                   packages: set[str]) -> None:
    lines = [f"{key} {value}" for key, value in sorted(baseline.items())]
    baseline_path(root, system).write_text(
        f"# Panics and stderr writes clippy still finds in production code on {system}"
        f" ({', '.join(LINTS)}; <file> <lint> <count>, scripts/check_clippy_panics.py);"
        " may only shrink.\n" + f"{PACKAGES_LINE}{' '.join(sorted(packages))}\n"
        + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def check(packages: list[str], update: bool = False, root: Path = ROOT,
          system: str = SYSTEM) -> int:
    run = subprocess.run(clippy_command(packages), cwd=root, stdout=subprocess.PIPE,
                         text=True, encoding="utf-8", errors="replace")
    if run.returncode != 0:
        print("clippy failed; run it as AGENTS.md says to see why")
        return 1
    # clippy also reports the workspace crates the packages depend on; they are not compared.
    counts = {key: value for key, value in counts_of(run.stdout.splitlines(), root).items()
              if in_packages(key, set(packages), root)}
    baseline, recorded = read_baseline(root, system)
    # A package the baseline has never counted is recorded once, as found.
    unrecorded = set(packages) - recorded
    failures = compare(counts, baseline, set(packages) - unrecorded if update else set(packages),
                       root)
    if failures:
        print("New panics or stderr writes in production code; return a "
              "ketchup_rejection::Rejection or log through ketchup_rejection::log! "
              "(AGENTS.md §2):")
        print(*failures, sep="\n  ")
        return 1
    if update:
        write_baseline(root, system, updated(counts, baseline, set(packages), root),
                       recorded | set(packages))
    shrunk = sum(value for key, value in baseline.items()
                 if in_packages(key, set(packages), root)) - sum(counts.values())
    print(f"OK; {sum(counts.values())} in {len(packages)} packages"
          + (f", {shrunk} fewer than the baseline" if shrunk > 0 and not update else ""))
    return 0


def packages_of(arguments: list[str]) -> list[str]:
    return [arguments[index + 1] for index, argument in enumerate(arguments)
            if argument == "-p" and index + 1 < len(arguments)]


if __name__ == "__main__":
    selected = packages_of(sys.argv[1:])
    if not selected:
        # The pre-push hook runs every check without arguments; the counts are per
        # platform, so each CI job names the packages it builds.
        print("No package named (-p); CI runs this check in each job for its packages")
        sys.exit(0)
    sys.exit(check(selected, update="--update" in sys.argv))
