"""Every workspace crate is linted by some CI job, and tested by one if it has tests.

A crate no `cargo clippy` names with `-p` is never linted there, so an API change
that breaks only that crate still shows "CI passed" (ketchup-headless was left out
of the OCCT job this way). A crate that is only linted hides its failing tests
the same way (the desktop Viewer's tests ran nowhere). A crate without a test
of its own is enough to lint. A new crate must be added to a job's crate list.
Only `cargo clippy` lints and only `cargo test` tests: `-p` in `cargo build`,
`cargo tree` or after `--` does not count, nor does a job or step whose `if:` is
`false` (review 2026-10-19 P3).

A run counts only if it can fail a pull request's CI and runs all the tests it
names (review 2026-10-24 CI-20): not a job or step whose `if:` asks for the nightly
`schedule` or a manual `workflow_dispatch` (directly or through an env variable
set from it) without naming `pull_request` or `push`, not one with
`continue-on-error` other than `false`, and not a `cargo test` given a test name
filter, `--ignored` or `--list` after `--`.
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
# The cargo arguments of a command: up to `--`, the end of the line or the next shell
# command; then what follows `--`, the arguments of the test binary.
CARGO = re.compile(
    r"\bcargo\s+(?:\+\S+\s+)?(clippy|test)\b((?:(?!\s--(?:\s|$))[^\n;|&])*)(?:\s--(?=\s|$)([^\n;|&]*))?"
)
CONDITION = re.compile(r"^\s*(?:- )?if:(.*)$", re.MULTILINE)
TOLERATED = re.compile(r"^\s*(?:- )?continue-on-error:\s*(?!false\s*$)\S", re.MULTILINE)
NOT_A_PULL_REQUEST = re.compile(r"event_name\s*==\s*'(?:schedule|workflow_dispatch)'")
A_PULL_REQUEST = re.compile(r"event_name\s*==\s*'(?:pull_request|pull_request_target|push)'")
ENVIRONMENT = re.compile(r"^\s+(\w+):\s*\$\{\{(.*)\}\}\s*$", re.MULTILINE)
FOLDED = re.compile(r"^(\s*)(?:- )?run:\s*>-?\s*$")
# libtest options that take the next word as their value.
VALUE_OPTIONS = {"--skip", "--test-threads", "--format", "--color", "--logfile", "-Z", "--shuffle-seed"}


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


def nightly_variables(workflow):
    """The env variables set from an expression that holds only outside a pull request."""
    return {
        name
        for name, expression in ENVIRONMENT.findall(without_comments(workflow))
        if NOT_A_PULL_REQUEST.search(expression) and not A_PULL_REQUEST.search(expression)
    }


def fails_a_pull_request(text, nightly):
    """Whether a job head or step can run in a pull request's CI and fail it."""
    if NEVER.search(text) or TOLERATED.search(text):
        return False
    for condition in CONDITION.findall(text):
        outside = NOT_A_PULL_REQUEST.search(condition) or any(
            re.search(rf"\benv\.{re.escape(name)}\s*==\s*'true'", condition) for name in nightly
        )
        if outside and not A_PULL_REQUEST.search(condition):
            return False
    return True


def running_steps(job, nightly=frozenset()):
    """The steps of a job that can fail a pull request: none when the job's own head cannot."""
    head, *steps = STEP.split(job)
    if not fails_a_pull_request(head.split("\n    steps:", 1)[0], nightly):
        return []
    return [step for step in steps if fails_a_pull_request(step, nightly)]


def folded(step):
    """The step with each `run: >` block on one line, as YAML folds it."""
    lines, joined, index = step.split("\n"), [], 0
    while index < len(lines):
        line, index = lines[index], index + 1
        match = FOLDED.match(line)
        if not match:
            joined.append(line)
            continue
        # The first line of a step has lost its `      - ` to the split.
        indent = len(match.group(1)) if joined else 8
        block = []
        while index < len(lines) and (not lines[index].strip() or len(lines[index]) - len(lines[index].lstrip()) > indent):
            block.append(lines[index].strip())
            index += 1
        joined.append(line[: line.index("run:")] + "run: " + " ".join(part for part in block if part))
    return "\n".join(joined)


def narrowed(test_arguments):
    """Whether the test binary's arguments run only some of the tests."""
    words = test_arguments.split()
    index = 0
    while index < len(words):
        word = words[index]
        if word in ("--ignored", "--list") or not word.startswith("-"):
            return True
        index += 2 if word in VALUE_OPTIONS else 1
    return False


def cargo_crates(workflows, subcommand):
    """Every crate a running `cargo <subcommand>` names, directly or through its job's CRATES list."""
    crates = set()
    for workflow in workflows:
        nightly = nightly_variables(workflow)
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            for step in running_steps(job, nightly):
                for name, arguments, test_arguments in CARGO.findall(CONTINUATION.sub(" ", folded(step))):
                    if name != subcommand or (name == "test" and narrowed(test_arguments)):
                        continue
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
