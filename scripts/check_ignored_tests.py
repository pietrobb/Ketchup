"""Every ignored test runs in some CI step.

An `#[ignore]` test runs only where a `cargo test` step passes `--ignored` with a
filter that names it, for a crate the step tests, and no `--skip` drops it. One no
step names checks nothing, while its name suggests it guards something (review
2026-10-17 §3 P3: four such tests ran nowhere). Only two kinds may stay out of CI,
and their ignore reason says so: the child process another test starts, and a test
that needs a hardware GPU, which the runners do not have.
"""

import re
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_ci_crates import CRATE_LIST, PACKAGE_FLAG, jobs, without_comments  # noqa: E402

IGNORED = re.compile(r'#\[ignore(?:\s*=\s*"([^"]*)")?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+(\w+)')
EXEMPT = ("child process", "subprocess", "hardware GPU")
STEP = re.compile(r"\n {6}- ")


def crate_of(path):
    """The package name of the crate `path` belongs to, and the crate's directory."""
    for directory in path.parents:
        manifest = directory / "Cargo.toml"
        if manifest.is_file():
            package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package")
            if package:
                return package["name"], directory
    return None, None


def module_path(relative):
    """`a::b` for `src/a/b.rs` or `tests/a/b.rs`; a crate or test root adds nothing."""
    parts = list(relative.with_suffix("").parts[1:])
    if parts and parts[-1] in ("lib", "main", "mod", "integration"):
        parts.pop()
    return "::".join(parts)


def ignored_tests(root):
    """(crate, test path, ignore reason) of every ignored test of the workspace."""
    tests = []
    for path in sorted([*root.glob("crates/*/**/*.rs"), *root.glob("viewer/*/**/*.rs")]):
        if "target" in path.relative_to(root).parts:
            continue
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
            for step in STEP.split(job):
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


def unrun_tests(tests, runs):
    return [
        f"{crate} {name}: ignored, but no CI step runs it; add it to a nightly step or delete it"
        for crate, name, reason in tests
        if not any(kind in reason for kind in EXEMPT) and not runs_somewhere(crate, name, runs)
    ]


def main():
    root = Path(__file__).resolve().parents[1]
    workflows = [
        without_comments(path.read_text(encoding="utf-8"))
        for path in sorted((root / ".github/workflows").glob("*.yml"))
    ]
    problems = unrun_tests(ignored_tests(root), ignored_runs(workflows))
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
