"""Every test that proves something only as root is run as root in CI.

Review 2026-11-13: the tests of writing the user's files under sudo return early
unless they are root, and no CI step ran them as root, so CI passed them unchecked.

Review 2026-11-20 ac P3: the steps were read as text, so a filter ending in `::`
covered any test of the crate and only the first `sudo` of a step counted. Now
the workflow is read as YAML, job by job, and each `sudo "$test" <filter>` is
tied to the `cargo test --no-run` that built its binaries; the filter is matched
as libtest does, as a substring of the test's full path.

Review 2026-11-21 ac P3: `--skip x` and a quoted filter were read as no filter,
and a step with `if: false`, a commented-out `sudo` and a run under `|| true`
counted. Now the arguments after `sudo "$test"` are split as the shell splits
them and read as libtest reads them, and a run that is never made, or whose
failure is let through, does not count.
"""
import re
import shlex
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_PATH = ROOT / ".github" / "workflows" / "ci.yml"
ROOT_ONLY = re.compile(r"if (?:ketchup_user_files::)?effective_user\(\) != 0 \{")
# A part of a test run only as root (review 2026-11-13 r95 P3: the Viewer's
# `== 0` branch was not found). Production code asks the same, so only in tests.
ROOT_PART = re.compile(r"if (?:ketchup_user_files::)?effective_user\(\) == 0 \{")
TEST = re.compile(r"\bfn (\w+)\(")
AS_ROOT = re.compile(r'sudo "\$test"')
PACKAGE = re.compile(r"(?:-p|--package) ([\w-]+)")
TARGET_NAME = re.compile(r'\.target\.name == "(\w+)"')
SHELL_COMMENT = re.compile(r"(?:^|\s)#.*")
# libtest's options that take the next argument as their value.
VALUED = {"--skip", "--test-threads", "--format", "--color", "--logfile", "-Z"}
# What ends the test's command: a pipe, a list operator, a redirection.
SHELL_OPERATOR = re.compile(r"^[()<>|&;]+$")


def module_path(relative):
    """The module path of a file under `crates/<crate>/src` or its test binary."""
    parts = list(relative.parts[2:])
    if relative.parts[1] == "tests":
        # tests/<binary>.rs is the binary's root; tests/<binary>/<module>.rs a module of it.
        parts = parts[1:]
    elif relative.parts[1] != "src":
        return None
    if parts:
        parts[-1] = Path(parts[-1]).stem
        if parts[-1] in ("mod", "lib", "main"):
            parts = parts[:-1]
    return parts


def root_only_tests():
    """(crate, full test path, in the library) of every root-only test."""
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT / "crates")
        crate = relative.parts[0]
        module = module_path(relative)
        if module is None:
            continue
        in_test_file = "tests" in relative.parts or "tests" in path.stem
        tests_from = 0 if in_test_file else text.find("mod tests {")
        found = list(ROOT_ONLY.finditer(text))
        if tests_from >= 0:
            found += [part for part in ROOT_PART.finditer(text) if part.start() > tests_from]
        for each in found:
            name = TEST.findall(text[: each.start()])[-1]
            inner = ["tests"] if not in_test_file and each.start() > tests_from >= 0 else []
            yield crate, "::".join(module + inner + [name]), relative.parts[1] == "src"


def never_made(part):
    """A job or step with `if: false`, or one whose failure does not fail CI."""
    condition = str(part.get("if", "")).replace(" ", "").lower()
    return condition in ("false", "${{false}}") or part.get("continue-on-error") is True


def libtest_run(rest):
    """(filters, skips, exact) of the arguments after `sudo "$test"` on its line, as libtest
    reads them; None when the run takes only ignored tests or a `||` after it, as
    `done || true` of its loop, may let its failure through."""
    lexer = shlex.shlex(rest, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    tokens = list(lexer)
    if "||" in tokens:
        return None
    filters, skips, exact = [], [], False
    arguments = iter(tokens)
    for token in arguments:
        if SHELL_OPERATOR.match(token):
            break
        option, _, value = token.partition("=")
        if option in VALUED and not value:
            value = next(arguments, "")
        if option == "--skip":
            skips.append(value)
        elif option == "--exact":
            exact = True
        elif option == "--ignored":
            return None
        elif not token.startswith("-"):
            filters.append(token)
    return filters, skips, exact


def root_runs(workflow):
    """(crates, library only, (filters, skips, exact)) of every `sudo "$test"` of every job."""
    for job in workflow["jobs"].values():
        if never_made(job):
            continue
        built = (frozenset(), False)
        for step in job.get("steps", []):
            if never_made(step):
                continue
            run = step.get("run", "")
            for line in run.replace("\\\n", " ").splitlines():
                line = SHELL_COMMENT.sub("", line)
                if "cargo test" in line and "--no-run" in line:
                    named = TARGET_NAME.findall(line)
                    crates = [name.replace("_", "-") for name in named] or PACKAGE.findall(line)
                    built = (frozenset(crates), 'index("lib")' in line)
                for match in AS_ROOT.finditer(line):
                    arguments = libtest_run(line[match.end():])
                    if arguments is not None:
                        yield built[0], built[1], arguments


def selects(arguments, test):
    """Whether libtest given `arguments` runs `test`, named by its full path."""
    filters, skips, exact = arguments
    matches = (lambda name: name == test) if exact else (lambda name: name in test)
    return (not filters or any(map(matches, filters))) and not any(map(matches, skips))


def runs_as_root(runs, crate, test, in_library):
    return any(
        crate in crates and (in_library or not library_only) and selects(arguments, test)
        for crates, library_only, arguments in runs
    )


def load_workflow():
    return yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))


def test_every_root_only_test_runs_as_root_in_ci():
    tests = list(root_only_tests())
    assert any(crate == "ketchup-app" for crate, _, _ in tests), "the scan finds no root-only test"
    runs = list(root_runs(load_workflow()))
    skipped = [
        f"{crate}: {test}"
        for crate, test, in_library in tests
        if not runs_as_root(runs, crate, test, in_library)
    ]
    assert not skipped, "\n".join(skipped)


SAMPLE = """
jobs:
  one:
    steps:
      - name: as root
        run: |
          tests=$(cargo test -p ketchup-user-files --no-run \\
              --message-format=json | jq -r '.executable')
          for test in $tests; do sudo "$test"; done
          tests=$(cargo test -p ketchup-mcp --no-run --message-format=json | jq -r '.executable')
          for test in $tests; do sudo "$test" under_sudo; done
  two:
    steps:
      - name: build
        run: cargo test -p ketchup-app --no-run
      - name: viewer as root
        run: |
          test=$(cargo test -p ketchup-viewer --no-run --message-format=json \\
            | jq -r 'select(.target.name == "ketchup_viewer" and (.target.kind | index("lib"))) | .executable')
          sudo "$test" preferences::tests::
"""


def test_a_filter_covers_only_the_tests_it_names():
    runs = list(root_runs(yaml.safe_load(SAMPLE)))
    assert runs_as_root(runs, "ketchup-user-files", "home::tests::anything", True)
    # The second sudo of a step counts, with its own filter and its own build.
    assert runs_as_root(runs, "ketchup-mcp", "tests::under_sudo_writes", True)
    assert not runs_as_root(runs, "ketchup-mcp", "tests::as_the_runner", True)
    # A filter ending in `::` names a module, not every test of every crate.
    assert runs_as_root(runs, "ketchup-viewer", "preferences::tests::as_root", True)
    assert not runs_as_root(runs, "ketchup-viewer", "camera::tests::as_root", True)
    assert not runs_as_root(runs, "ketchup-app", "preferences::tests::as_root", True)
    # The Viewer's step runs its library only, not its integration tests.
    assert not runs_as_root(runs, "ketchup-viewer", "preferences::tests::as_root", False)


UNMADE = """
jobs:
  one:
    steps:
      - name: build
        run: cargo test -p ketchup-mcp --no-run
      - name: as root
        run: |
          sudo "$test" --skip under_sudo
          sudo "$test" "home::tests::" --test-threads 1 | tee out.txt
          sudo "$test" --exact tests::under_sudo_exactly
          # sudo "$test"
          echo done  # sudo "$test" commented
          sudo "$test" setup::tests:: || true
          for test in $tests; do sudo "$test" folders::; done || true
          sudo "$test" --ignored
      - name: never
        if: false
        run: sudo "$test" skipped_step
      - name: let fail
        continue-on-error: true
        run: sudo "$test" failing_step
  two:
    if: ${{ false }}
    steps:
      - run: cargo test -p ketchup-app --no-run
      - run: sudo "$test"
"""


def test_a_run_that_is_skipped_never_made_or_let_fail_does_not_count():
    runs = list(root_runs(yaml.safe_load(UNMADE)))
    covered = lambda test: runs_as_root(runs, "ketchup-mcp", test, True)
    # `--skip x` runs every test but those, not every test.
    assert covered("tests::as_the_runner")
    assert not covered("tests::under_sudo_writes")
    # A quoted filter is a filter, its options are not.
    assert covered("home::tests::under_sudo_writes")
    # `--exact` takes the whole path only.
    assert covered("tests::under_sudo_exactly")
    assert not covered("tests::under_sudo_exactly_more")
    # A comment, `|| true`, `--ignored`, `if: false` and `continue-on-error` run nothing.
    for unmade in [
        "other::under_sudo",
        "setup::tests::under_sudo",
        "folders::under_sudo",
        "skipped_step::under_sudo",
        "failing_step::under_sudo",
    ]:
        assert not covered(unmade), unmade
    assert not runs_as_root(runs, "ketchup-app", "anything", True)
