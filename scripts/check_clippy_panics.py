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
compared, so the job without OCCT and the one with it each check their own. Every
workspace package the named ones depend on is compiled and counted too; code compiled
only for another target is counted with --target (its system's baseline), and a job
whose packages turn on other features of a shared dependency names its own --baseline
(review 2026-11-09 R1109-TST-302). Run with --update after removing occurrences; it never
grows a count.

A command line's entry point (`src/main.rs`, `src/bin/`) may write to stderr: that is
where it reports. `scripts/check_error_hygiene.py` holds the one of a window to its log.

    python scripts/check_clippy_panics.py [--update] [--target <triple>] [--baseline <name>]
        -p ketchup-model -p ketchup-app ...
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


def clippy_command(packages: list[str], target: str | None = None) -> list[str]:
    selected = [argument for package in packages for argument in ("-p", package)]
    # Forced, so an `#[allow(clippy::unwrap_used)]` does not hide what it allows (review
    # 2026-11-09 R1109-TST-301); scripts/check_error_hygiene.py counts the attribute too.
    warnings = [argument for lint in LINTS for argument in ("--force-warn", f"clippy::{lint}")]
    return ["cargo", "clippy", "--locked", *selected, *(["--target", target] if target else []),
            "--lib", "--bins", "--message-format=json", "--", *warnings]


# The baseline of a target named by its triple: the phone's and the browser's code is
# compiled only for them (review 2026-11-09 R1109-TST-302).
TARGET_SYSTEMS = (("android", "android"), ("wasm32", "browser"), ("apple", "macos"),
                  ("windows", "windows"), ("linux", "linux"))


def system_of(target: str) -> str:
    return next((system for part, system in TARGET_SYSTEMS if part in target), target)


def compiled_packages(packages: list[str], root: Path = ROOT,
                      target: str | None = None) -> set[str]:
    """The packages and every workspace package they depend on for the target: clippy
    compiles them all, so code compiled only through a dependency is counted too (review
    2026-11-09 R1109-TST-302: the workspace dependencies were left out)."""
    if target is None:
        host = subprocess.run(["rustc", "-vV"], stdout=subprocess.PIPE, text=True, check=True)
        target = next(line.split(":", 1)[1].strip() for line in host.stdout.splitlines()
                      if line.startswith("host:"))
    run = subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1",
                          "--filter-platform", target], cwd=root, stdout=subprocess.PIPE,
                         text=True, encoding="utf-8", check=True)
    metadata = json.loads(run.stdout)
    members = set(metadata["workspace_members"])
    names = {package["id"]: package["name"] for package in metadata["packages"]}
    depends = {node["id"]: [dependency["pkg"] for dependency in node["deps"]
                            if dependency["pkg"] in members
                            and any(kind["kind"] is None for kind in dependency["dep_kinds"])]
               for node in metadata["resolve"]["nodes"]}
    reached = [member for member in members if names[member] in packages]
    seen = set(reached)
    while reached:
        for dependency in depends.get(reached.pop(), []):
            if dependency not in seen:
                seen.add(dependency)
                reached.append(dependency)
    return {names[member] for member in seen}


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
# The workspace packages the named ones depend on: compiled as libraries only, so not named
# when the check runs without -p, where their binaries would be compiled too.
DEPENDENCIES_LINE = "# dependencies: "


def baseline_path(root: Path, system: str) -> Path:
    return root / "scripts" / f"clippy_panics_baseline_{system}.txt"


def baseline_lines(root: Path, system: str) -> tuple[dict[str, int], set[str], set[str]]:
    """The counts on a system, the packages named for them and those they depend on."""
    baseline: dict[str, int] = {}
    named: set[str] = set()
    dependencies: set[str] = set()
    path = baseline_path(root, system)
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith(PACKAGES_LINE):
                named = set(line[len(PACKAGES_LINE):].split())
            elif line.startswith(DEPENDENCIES_LINE):
                dependencies = set(line[len(DEPENDENCIES_LINE):].split())
            elif line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline, named, dependencies


def read_baseline(root: Path, system: str) -> tuple[dict[str, int], set[str]]:
    """The counts on a system, and the packages they were recorded for."""
    baseline, named, dependencies = baseline_lines(root, system)
    return baseline, named | dependencies


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
                   packages: set[str], dependencies: set[str] = frozenset()) -> None:
    lines = [f"{key} {value}" for key, value in sorted(baseline.items())]
    dependencies = set(dependencies) - set(packages)
    baseline_path(root, system).write_text(
        f"# Panics and stderr writes clippy still finds in production code on {system}"
        f" ({', '.join(LINTS)}; <file> <lint> <count>, scripts/check_clippy_panics.py);"
        " may only shrink.\n" + f"{PACKAGES_LINE}{' '.join(sorted(packages))}\n"
        + (f"{DEPENDENCIES_LINE}{' '.join(sorted(dependencies))}\n" if dependencies else "")
        + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8", newline="\n")


def check(packages: list[str], update: bool = False, root: Path = ROOT,
          system: str = SYSTEM, target: str | None = None) -> int:
    run = subprocess.run(clippy_command(packages, target), cwd=root, stdout=subprocess.PIPE,
                         text=True, encoding="utf-8", errors="replace")
    if run.returncode != 0:
        print("clippy failed; run it as AGENTS.md says to see why")
        return 1
    # The workspace packages the named ones depend on are compiled, so counted, too.
    compiled = compiled_packages(packages, root, target) | set(packages)
    counts = {key: value for key, value in counts_of(run.stdout.splitlines(), root).items()
              if in_packages(key, compiled, root)}
    baseline, recorded = read_baseline(root, system)
    # A package the baseline has never counted is recorded once, as found.
    unrecorded = compiled - recorded
    failures = compare(counts, baseline, compiled - unrecorded if update else compiled, root)
    if failures:
        print("New panics or stderr writes in production code; return a "
              "ketchup_rejection::Rejection or log through ketchup_rejection::log! "
              "(AGENTS.md §2):")
        print(*failures, sep="\n  ")
        return 1
    if update:
        _, named, dependencies = baseline_lines(root, system)
        write_baseline(root, system, updated(counts, baseline, compiled, root),
                       named | set(packages), dependencies | compiled)
    shrunk = sum(value for key, value in baseline.items()
                 if in_packages(key, compiled, root)) - sum(counts.values())
    print(f"OK; {sum(counts.values())} in {len(compiled)} packages"
          + (f", {shrunk} fewer than the baseline" if shrunk > 0 and not update else ""))
    return 0


def values_of(arguments: list[str], option: str) -> list[str]:
    return [arguments[index + 1] for index, argument in enumerate(arguments)
            if argument == option and index + 1 < len(arguments)]


def packages_of(arguments: list[str]) -> list[str]:
    return values_of(arguments, "-p")


def target_and_baseline(arguments: list[str]) -> tuple[str | None, str]:
    """The --target to compile for, and the baseline to compare with: --baseline, else the
    system of the target, else this one. A job whose packages compile a shared dependency
    with other features names its own baseline (the one with OCCT on Linux)."""
    target = next(iter(values_of(arguments, "--target")), None)
    named = next(iter(values_of(arguments, "--baseline")), None)
    return target, named or (system_of(target) if target else SYSTEM)


def packages_to_check(arguments: list[str], root: Path = ROOT, system: str = SYSTEM) -> list[str]:
    """The packages named with -p, or else those this system's baseline records: the
    pre-push hook runs every check without arguments, and nothing checked is not a pass
    (review 2026-11-09 R1109-TST-306: it ended 0 having run nothing)."""
    return packages_of(arguments) or sorted(baseline_lines(root, system)[1])


if __name__ == "__main__":
    TARGET, BASELINE = target_and_baseline(sys.argv[1:])
    selected = packages_to_check(sys.argv[1:], system=BASELINE)
    if not selected:
        print(f"No package named (-p) and none recorded in "
              f"{baseline_path(ROOT, BASELINE).relative_to(ROOT).as_posix()}; nothing was checked")
        sys.exit(1)
    sys.exit(check(selected, update="--update" in sys.argv, system=BASELINE, target=TARGET))
