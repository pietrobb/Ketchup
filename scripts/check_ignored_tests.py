"""Every ignored test runs in some CI step.

An `#[ignore]` test runs only where a `cargo test` step passes `--ignored` with a
filter that names it, for a crate the step tests, and no `--skip` drops it. One no
step names checks nothing, while its name suggests it guards something (review
2026-10-17 §3 P3: four such tests ran nowhere). Only two kinds may stay out of CI,
and their ignore reason says so: the child process another test starts, and a test
that needs a hardware GPU, which the runners do not have. A child process must also
be named in a string by a test of its crate, the one that starts it: a reason that
only mentions a subprocess exempts nothing (review 2026-10-19 P3). A step whose
`if:` is false runs nothing.
"""

import functools
import re
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_ci_crates import CRATE_LIST, PACKAGE_FLAG, jobs, running_steps, without_comments  # noqa: E402

IGNORED = re.compile(r'#\[ignore(?:\s*=\s*"([^"]*)")?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+(\w+)')
CHILD = ("child process", "subprocess")
GPU = "hardware GPU"
# A string that names a function, alone or by its module path: how a test starts another as a child.
QUOTED = re.compile(r'"(?:\w+::)*(\w+)"')


@functools.cache
def package_in(directory):
    """The package name of the Cargo.toml in `directory`, or None."""
    manifest = directory / "Cargo.toml"
    if manifest.is_file():
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package")
        if package:
            return package["name"]
    return None


def crate_of(path):
    """The package name of the crate `path` belongs to, and the crate's directory."""
    for directory in path.parents:
        name = package_in(directory)
        if name:
            return name, directory
    return None, None


def rust_sources(root):
    paths = sorted([*root.glob("crates/*/**/*.rs"), *root.glob("viewer/*/**/*.rs")])
    return [path for path in paths if "target" not in path.relative_to(root).parts]


def started_by_tests(root):
    """(crate, function) of every function a string of its crate names, as the test starting it does."""
    started = set()
    for path in rust_sources(root):
        names = set(QUOTED.findall(path.read_text(encoding="utf-8", errors="replace")))
        if names:
            crate, _ = crate_of(path)
            started |= {(crate, name) for name in names}
    return started


def module_path(relative):
    """`a::b` for `src/a/b.rs` or `tests/a/b.rs`; a crate or test root adds nothing."""
    parts = list(relative.with_suffix("").parts[1:])
    if parts and parts[-1] in ("lib", "main", "mod", "integration"):
        parts.pop()
    return "::".join(parts)


def ignored_tests(root):
    """(crate, test path, ignore reason) of every ignored test of the workspace."""
    tests = []
    for path in rust_sources(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        matches = list(IGNORED.finditer(text))
        if not matches:
            continue
        crate, directory = crate_of(path)
        module = module_path(path.relative_to(directory))
        for match in matches:
            name = f"{module}::{match.group(2)}" if module else match.group(2)
            tests.append((crate, name, match.group(1) or ""))
    return tests


def ignored_runs(workflows):
    """(crates, filters, skips) of every `cargo test ... --ignored` step."""
    runs = []
    for workflow in workflows:
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            for step in running_steps(job):
                if "cargo test" not in step or "--ignored" not in step:
                    continue
                command = step[step.index("cargo test") :]
                crates = set(PACKAGE_FLAG.findall(command))
                if "CRATES" in command:
                    crates |= listed
                tokens = command.split()
                start = tokens.index("--ignored") + 1
                filters = []
                for token in tokens[start:]:
                    if token.startswith("-"):
                        break
                    filters.append(token)
                skips = {tokens[i + 1] for i, token in enumerate(tokens[:-1]) if token == "--skip"}
                runs.append((crates, filters, skips))
    return runs


def runs_somewhere(crate, name, runs):
    return any(
        crate in crates
        and any(wanted in name for wanted in filters)
        and not any(skipped in name for skipped in skips)
        for crates, filters, skips in runs
    )


def unrun_tests(tests, runs, started=frozenset()):
    problems = []
    for crate, name, reason in tests:
        if GPU in reason or runs_somewhere(crate, name, runs):
            continue
        child = any(kind in reason for kind in CHILD)
        if child and (crate, name.rsplit("::", 1)[-1]) in started:
            continue
        if child:
            problems.append(
                f"{crate} {name}: ignored as a child process, but no test of {crate} names it to start it; "
                "start it from a test or run it in a CI step"
            )
        else:
            problems.append(f"{crate} {name}: ignored, but no CI step runs it; add it to a nightly step or delete it")
    return problems


def main():
    root = Path(__file__).resolve().parents[1]
    workflows = [
        without_comments(path.read_text(encoding="utf-8"))
        for path in sorted((root / ".github/workflows").glob("*.yml"))
    ]
    problems = unrun_tests(ignored_tests(root), ignored_runs(workflows), started_by_tests(root))
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
