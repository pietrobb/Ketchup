"""Every workspace crate is linted by some CI job, and tested by one if it has tests.

A crate no `cargo clippy` names with `-p` is never linted there, so an API change
that breaks only that crate still shows "CI passed" (ketchup-headless was left out
of the OCCT job this way). A crate that is only linted hides its failing tests
the same way (the desktop Viewer's tests ran nowhere). A crate without a test
of its own is enough to lint. A new crate must be added to a job's crate list.
Only `cargo clippy` lints and only `cargo test` tests: `-p` in `cargo build`,
`cargo tree` or after `--` does not count, nor does a job or step whose `if:` is
`false` (review 2026-10-19 P3).
"""

import re
import sys
import tomllib
from pathlib import Path

PACKAGE_FLAG = re.compile(r"(?:^|\s)-p\s+([\w-]+)")
JOB = re.compile(r"^  [\w-]+:\s*$", re.MULTILINE)
CRATE_LIST = re.compile(r"CRATES:\s*>-\n((?:[ \t]+-p\b.*\n)+)")
# A `#` at a line start or after a space opens a comment in YAML and in shell.
COMMENT = re.compile(r"(?:^|(?<=\s))#.*$", re.MULTILINE)
STEP = re.compile(r"\n {6}- ")
NEVER = re.compile(r"^\s*(?:- )?if:\s*(?:false|\$\{\{\s*false\s*\}\})\s*$", re.MULTILINE)
# A line ending in `\` (bash) or a backtick (PowerShell) continues on the next one.
CONTINUATION = re.compile(r"[\\`][ \t]*\n[ \t]*")
# The cargo arguments of a command: up to `--`, the end of the line or the next shell command.
CARGO = re.compile(r"\bcargo\s+(?:\+\S+\s+)?(clippy|test)\b((?:(?!\s--(?:\s|$))[^\n;|&])*)")


def without_comments(workflow):
    """The workflow with its comments removed, so a crate named only in one does not count."""
    return COMMENT.sub("", workflow)


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


def jobs(workflow):
    """The text of every job of a workflow, from its name to the next job's."""
    text = without_comments(workflow).split("\njobs:\n", 1)[-1]
    starts = [match.start() for match in JOB.finditer(text)] + [len(text)]
    return [text[start:end] for start, end in zip(starts, starts[1:])]


def running_steps(job):
    """The steps of a job that can run: none when the job's own `if:` is false."""
    head, *steps = STEP.split(job)
    if NEVER.search(head.split("\n    steps:", 1)[0]):
        return []
    return [step for step in steps if not NEVER.search(step)]


def cargo_crates(workflows, subcommand):
    """Every crate a running `cargo <subcommand>` names, directly or through its job's CRATES list."""
    crates = set()
    for workflow in workflows:
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            for step in running_steps(job):
                for name, arguments in CARGO.findall(CONTINUATION.sub(" ", step)):
                    if name == subcommand:
                        crates |= set(PACKAGE_FLAG.findall(arguments))
                        if "CRATES" in arguments:
                            crates |= listed
    return crates


def linted_crates(workflows):
    return cargo_crates(workflows, "clippy")


def tested_crates(workflows):
    return cargo_crates(workflows, "test")


def unchecked_crates(crates, linted, tested, with_tests):
    problems = []
    for crate in crates:
        if crate not in linted:
            problems.append(f"{crate}: no CI job runs cargo clippy on it; add it to a job's crate list")
        elif crate in with_tests and crate not in tested:
            problems.append(f"{crate}: has tests, but no CI job runs cargo test on it; add it to a test job")
    return problems


def main():
    root = Path(__file__).resolve().parents[1]
    workflows = [path.read_text(encoding="utf-8") for path in sorted((root / ".github/workflows").glob("*.yml"))]
    problems = unchecked_crates(
        workspace_crates(root), linted_crates(workflows), tested_crates(workflows), crates_with_tests(root)
    )
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
