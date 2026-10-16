"""Every workspace crate is linted by some CI job, and tested by one if it has tests.

A crate CI never names with `-p` is never built there, so an API change that
breaks only that crate still shows "CI passed" (ketchup-headless was left out
of the OCCT job this way). A crate that is only linted hides its failing tests
the same way (the desktop Viewer's tests ran nowhere). A crate without a test
of its own is enough to lint. A new crate must be added to a job's crate list.
"""

import re
import sys
import tomllib
from pathlib import Path

PACKAGE_FLAG = re.compile(r"(?:^|\s)-p\s+([\w-]+)")
JOB = re.compile(r"^  [\w-]+:\s*$", re.MULTILINE)
CRATE_LIST = re.compile(r"CRATES:\s*>-\n((?:[ \t]+-p\b.*\n)+)")


def workspace_members(root):
    """The package name and directory of every workspace member of the root Cargo.toml."""
    members = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["members"]
    return {
        tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]["name"]: root / member
        for member in members
    }


def workspace_crates(root):
    return sorted(workspace_members(root))


def crates_with_tests(root):
    """The members with at least one `#[test]` in their sources or tests."""
    return {
        name
        for name, directory in workspace_members(root).items()
        if any("#[test]" in path.read_text(encoding="utf-8") for path in directory.rglob("*.rs"))
    }


def ci_crates(workflows):
    """Every crate a workflow names with `-p`."""
    return {name for text in workflows for name in PACKAGE_FLAG.findall(text)}


def jobs(workflow):
    """The text of every job of a workflow, from its name to the next job's."""
    text = workflow.split("\njobs:\n", 1)[-1]
    starts = [match.start() for match in JOB.finditer(text)] + [len(text)]
    return [text[start:end] for start, end in zip(starts, starts[1:])]


def tested_crates(workflows):
    """Every crate a `cargo test` line names, directly or through its job's CRATES list."""
    tested = set()
    for workflow in workflows:
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            for line in job.splitlines():
                if "cargo test" in line:
                    tested |= set(PACKAGE_FLAG.findall(line))
                    if "CRATES" in line:
                        tested |= listed
    return tested


def unchecked_crates(crates, named, tested, with_tests):
    problems = []
    for crate in crates:
        if crate not in named:
            problems.append(f"{crate}: no CI job names it with -p; add it to a job's crate list")
        elif crate in with_tests and crate not in tested:
            problems.append(f"{crate}: has tests, but no CI job runs cargo test on it; add it to a test job")
    return problems


def main():
    root = Path(__file__).resolve().parents[1]
    workflows = [path.read_text(encoding="utf-8") for path in sorted((root / ".github/workflows").glob("*.yml"))]
    problems = unchecked_crates(
        workspace_crates(root), ci_crates(workflows), tested_crates(workflows), crates_with_tests(root)
    )
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
