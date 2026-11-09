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

Each crate is also checked on its systems (review 2026-10-29 CI-62: the crates
with OCCT were compiled on Windows only, so code for the other systems broke
unseen): every crate on Linux, a crate that builds on OCCT on Windows too, and a
crate for one system only on that one. A job runs on the systems of its
`runs-on` (or its `matrix.os`), a step on those its `runner.os` condition leaves.

A feature a crate declares off by default is linted with it on by some `cargo
clippy` (review 2026-11-01 TST-102: the code behind `saved-files` was never
linted), except `testing`, which a crate's own tests turn on through its
dev-dependencies.

A crate with code only Windows compiles has its panics and stderr writes counted
(scripts/check_clippy_panics.py) by a job on Windows, since clippy elsewhere never
sees that code (review 2026-11-08 CI-250); so does one with code only Linux, macOS,
a unix system, Android or the browser compiles, on that system or with its `--target`
(review 2026-11-09 R1109-TST-302). A crate is counted where a crate depending on it is.
"""

import re
import sys
import tomllib
from pathlib import Path

PACKAGE_FLAG = re.compile(r"(?:^|\s)-p\s+([\w-]+)")
FEATURES_FLAG = re.compile(r"(?:^|\s)(?:--features|-F)[\s=]+([\w/,-]+)")
# Features the crate's tests turn on through its dev-dependencies, so --all-targets lints them.
TEST_FEATURES = {"default", "testing"}
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
RUNS_ON = re.compile(r"^\s+runs-on:\s*(.+)$", re.MULTILINE)
MATRIX_OS = re.compile(r"^\s+os:\s*\[(.*)\]\s*$", re.MULTILINE)
RUNNER_OS = re.compile(r"runner\.os\s*(==|!=)\s*'(\w+)'")
# GitHub's runner labels and the `runner.os` of each.
SYSTEMS = {"ubuntu": "Linux", "windows": "Windows", "macos": "macOS"}
# The crate whose build needs OCCT; every crate that depends on it does too.
OCCT_CRATE = "ketchup-exact"
OCCT_SYSTEMS = {"Linux", "Windows"}
# Crates whose code compiles for one system only.
SYSTEM_ONLY = {"ketchup-viewer-macos": {"macOS"}, "ketchup-macos-open": {"macOS"}}
PANICS = re.compile(r"\bcheck_clippy_panics\.py\b([^\n;|&]*)")
# Code only Windows compiles, which clippy on another system never sees (review 2026-11-08 CI-250).
# `cfg!(windows)` is not: both of its branches compile everywhere.
# Over several lines as rustfmt breaks a long one, and `not(unix)` too (review 2026-11-09
# R1109-CI-300: both were missed).
CFG = r"#!?\[cfg(?:_attr)?\([^\]]*?"
# Code for one system or target only, which clippy elsewhere never sees (review 2026-11-09
# R1109-TST-302: the unix code of the crates with OCCT, and the macOS, Android and browser
# code, were counted nowhere). `unix` code is seen on any unix system.
SYSTEM_CODE = {
    "Windows": re.compile(CFG + r'(?:\bwindows\b|target_os\s*=\s*"windows"|\bnot\s*\(\s*unix\s*\))'),
    "Linux": re.compile(CFG + r'target_os\s*=\s*"linux"'),
    "macOS": re.compile(CFG + r'target_os\s*=\s*"macos"'),
    "Android": re.compile(CFG + r'target_os\s*=\s*"android"'),
    "Browser": re.compile(CFG + r'(?:target_arch\s*=\s*"wasm32"|target_family\s*=\s*"wasm")'),
    "unix": re.compile(CFG + r"(?<!not\()\bunix\b"),
}
UNIX = {"Linux", "macOS", "Android"}
# The system a `--target` triple compiles for, and the words of a `[target.'cfg(..)']`
# dependency's condition that hold on each system.
TARGET_SYSTEMS = (("android", "Android"), ("wasm32", "Browser"), ("apple", "macOS"),
                  ("windows", "Windows"), ("linux", "Linux"))
TARGET_FLAG = re.compile(r"(?:^|\s)--target[\s=]+(\S+)")
BASELINE_FLAG = re.compile(r"(?:^|\s)--baseline[\s=]+(\S+)")
CONDITION_WORDS = {"Linux": ("linux", "unix"), "macOS": ("macos", "unix"), "Windows": ("windows",),
                   "Android": ("android", "unix"), "Browser": ("wasm",)}
TEST_FILE = re.compile(r"(?:^|_)tests?\.rs$")
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


def running_steps(job):
    """The steps of a job that can run: none when the job's own `if:` is false."""
    head, *steps = STEP.split(job)
    if NEVER.search(head.split("\n    steps:", 1)[0]):
        return []
    return [step for step in steps if not NEVER.search(step)]


def pull_request_steps(job, nightly):
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


def job_systems(job):
    """The systems a job runs on: its `runs-on` label, or each label of its `matrix.os`."""
    head = job.split("\n    steps:", 1)[0]
    runs_on = RUNS_ON.search(head)
    if not runs_on:
        return set()
    labels = [runs_on.group(1)]
    matrix = MATRIX_OS.search(head)
    if "matrix.os" in runs_on.group(1) and matrix:
        labels = matrix.group(1).split(",")
    return {
        system for label in labels for prefix, system in SYSTEMS.items() if label.strip().startswith(prefix)
    }


def step_systems(step, systems):
    """The systems of its job a step runs on, after its `runner.os` conditions."""
    for condition in CONDITION.findall(step):
        tests = RUNNER_OS.findall(condition)
        only = {system for operator, system in tests if operator == "=="}
        if only:
            systems = systems & only
        systems = systems - {system for operator, system in tests if operator == "!="}
    return systems


def pull_request_runs(workflows, system=None):
    """The job's CRATES list and the step's commands, each on one line, of every step
    that can fail a pull request; with a system, only the steps on that system."""
    for workflow in workflows:
        nightly = nightly_variables(workflow)
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            systems = job_systems(job)
            for step in pull_request_steps(job, nightly):
                if system is None or system in step_systems(step, systems):
                    yield listed, CONTINUATION.sub(" ", folded(step))


def cargo_crates(workflows, subcommand, system=None):
    """Every crate a running `cargo <subcommand>` names, directly or through its job's CRATES list.

    With a system, only the runs on that system count.
    """
    crates = set()
    for listed, step in pull_request_runs(workflows, system):
        for name, arguments, test_arguments in CARGO.findall(step):
            if name != subcommand or (name == "test" and narrowed(test_arguments)):
                continue
            crates |= set(PACKAGE_FLAG.findall(arguments))
            if "CRATES" in arguments:
                crates |= listed
    return crates


def target_system(target):
    return next((system for part, system in TARGET_SYSTEMS if part in target), target)


def workspace_dependencies(root, system):
    """Each member's workspace dependencies on the system: its plain ones and those of a
    `[target.'cfg(..)']` table whose condition holds there."""
    members = workspace_members(root)
    dependencies = {}
    for name, directory in members.items():
        manifest = tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8"))
        used = set(manifest.get("dependencies", {}))
        for condition, table in manifest.get("target", {}).items():
            if "not(" not in condition and any(word in condition for word in CONDITION_WORDS.get(system, ())):
                used |= set(table.get("dependencies", {}))
        dependencies[name] = used & set(members)
    return dependencies


def panics_counted(workflows, system, root=None):
    """Every crate whose panics and stderr writes a step counts on the system: on its job's
    systems, or the one of its `--target`. With the workspace root, also the workspace
    crates they depend on there, which scripts/check_clippy_panics.py counts too."""
    crates = set()
    for workflow in workflows:
        nightly = nightly_variables(workflow)
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            for step in pull_request_steps(job, nightly):
                step_line = CONTINUATION.sub(" ", folded(step))
                for arguments in PANICS.findall(step_line):
                    target = TARGET_FLAG.search(arguments)
                    systems = {target_system(target.group(1))} if target else step_systems(step, job_systems(job))
                    if system in systems:
                        crates |= set(PACKAGE_FLAG.findall(arguments)) | (listed if "CRATES" in arguments else set())
    if root is not None:
        dependencies = workspace_dependencies(root, system)
        reached = list(crates)
        while reached:
            for dependency in dependencies.get(reached.pop(), set()) - crates:
                crates.add(dependency)
                reached.append(dependency)
    return crates


def panic_baselines(workflows):
    """The baseline each running scripts/check_clippy_panics.py step compares with: its
    `--baseline`, else the system of its `--target`, else that of each system it runs on."""
    names = set()
    for workflow in workflows:
        nightly = nightly_variables(workflow)
        for job in jobs(workflow):
            for step in pull_request_steps(job, nightly):
                for arguments in PANICS.findall(CONTINUATION.sub(" ", folded(step))):
                    named, target = BASELINE_FLAG.search(arguments), TARGET_FLAG.search(arguments)
                    if named:
                        names.add(named.group(1))
                    elif target:
                        names.add(target_system(target.group(1)).lower())
                    else:
                        names |= {system.lower() for system in step_systems(step, job_systems(job))}
    return names


def missing_panic_baselines(names, root):
    """A step whose baseline file is missing allows nothing, so it fails on the first count
    (review 2026-11-09 R1109-TST-302: the macOS app job named one never recorded)."""
    return [
        f"scripts/clippy_panics_baseline_{name}.txt: a CI step counts panics against it, but it does"
        " not exist; record it with scripts/check_clippy_panics.py --update on that system"
        for name in sorted(names)
        if not (root / "scripts" / f"clippy_panics_baseline_{name}.txt").exists()
    ]


def crates_with_system_code(root):
    """Each member whose sources, outside a test file, have code for one system only, with
    those systems (`unix` for code every unix system compiles)."""
    found = {}
    for name, directory in workspace_members(root).items():
        for path in (directory / "src").rglob("*.rs"):
            if TEST_FILE.search(path.name):
                continue
            text = path.read_text(encoding="utf-8")
            found.setdefault(name, set()).update(system for system, code in SYSTEM_CODE.items() if code.search(text))
        for system in SYSTEM_ONLY.get(name, ()):
            found.setdefault(name, set()).add(system)
    return {name: systems for name, systems in found.items() if systems}


def system_code_uncounted(with_system_code, counted):
    """counted maps a system to the crates whose panics a step there counts."""
    problems = []
    for crate, systems in sorted(with_system_code.items()):
        for system in sorted(systems):
            where = UNIX if system == "unix" else {system}
            if not any(crate in counted.get(each, set()) for each in where):
                problems.append(
                    f"{crate}: has code only {system} compiles, but no CI job on {system} counts its panics"
                    " and stderr writes; add it (or a crate depending on it) to a scripts/check_clippy_panics.py"
                    f" step on {system}" + (" or with its --target" if system in ("Android", "Browser") else "")
                )
    return problems


def crate_features(root):
    """Every (crate, feature) a workspace member declares off by default, but those its tests turn on."""
    return {
        (name, feature)
        for name, directory in workspace_members(root).items()
        for feature in tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8")).get("features", {})
        if feature not in TEST_FEATURES
    }


def linted_features(workflows):
    """Every (crate, feature) a running `cargo clippy` turns on: `crate/feature`, or a bare
    feature of each crate the run names."""
    features = set()
    for workflow in workflows:
        nightly = nightly_variables(workflow)
        for job in jobs(workflow):
            listed = set(PACKAGE_FLAG.findall("".join(CRATE_LIST.findall(job))))
            for step in pull_request_steps(job, nightly):
                for name, arguments, _ in CARGO.findall(CONTINUATION.sub(" ", folded(step))):
                    if name != "clippy":
                        continue
                    crates = set(PACKAGE_FLAG.findall(arguments)) | (listed if "CRATES" in arguments else set())
                    for value in FEATURES_FLAG.findall(arguments):
                        for feature in filter(None, value.split(",")):
                            crate, _, own = feature.rpartition("/")
                            features |= {(crate, own)} if crate else {(each, own) for each in crates}
    return features


def unlinted_features(declared, linted):
    return [
        f"{crate}: feature {feature} is off by default and no CI job runs cargo clippy with it on"
        for crate, feature in sorted(declared - linted)
    ]


def linted_crates(workflows, system=None):
    return cargo_crates(workflows, "clippy", system)


def tested_crates(workflows, system=None):
    return cargo_crates(workflows, "test", system)


def required_systems(root):
    """The systems CI must check each workspace crate on."""
    members = workspace_members(root)
    dependencies = {
        name: set(tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8")).get("dependencies", {}))
        for name, directory in members.items()
    }
    with_occt = {OCCT_CRATE}
    grown = True
    while grown:
        reached = {name for name, used in dependencies.items() if used & with_occt}
        grown = not reached <= with_occt
        with_occt |= reached
    return {
        name: SYSTEM_ONLY.get(name, OCCT_SYSTEMS if name in with_occt else {"Linux"}) for name in members
    }


def unchecked_on_systems(required, linted, tested, with_tests):
    """What CI leaves unchecked on a system a crate needs; linted and tested map a system to its crates."""
    problems = []
    for crate, systems in required.items():
        for system in sorted(systems):
            if crate not in linted.get(system, set()):
                problems.append(
                    f"{crate}: no CI job on {system} runs cargo clippy on it; add it to a {system} job's crate list"
                )
            elif crate in with_tests and crate not in tested.get(system, set()):
                problems.append(
                    f"{crate}: has tests, but no CI job on {system} runs cargo test on it; add it to a {system} test job"
                )
    return problems


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
    problems += unchecked_on_systems(
        required_systems(root),
        {system: linted_crates(workflows, system) for system in SYSTEMS.values()},
        {system: tested_crates(workflows, system) for system in SYSTEMS.values()},
        crates_with_tests(root),
    )
    problems += unlinted_features(crate_features(root), linted_features(workflows))
    problems += system_code_uncounted(
        crates_with_system_code(root),
        {system: panics_counted(workflows, system, root) for system in (*SYSTEMS.values(), "Android", "Browser")},
    )
    problems += missing_panic_baselines(panic_baselines(workflows), root)
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
