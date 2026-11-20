"""Every test that proves something only as root is run as root in CI.

Review 2026-11-13: the tests of writing the user's files under sudo return early
unless they are root, and no CI step ran them as root, so CI passed them unchecked.

Review 2026-11-20 ac P3: the steps were read as text, so a filter ending in `::`
covered any test of the crate and only the first `sudo` of a step counted. Now
the workflow is read as YAML, job by job, and each `sudo "$test" <filter>` is
tied to the `cargo test --no-run` that built its binaries; the filter is matched
as libtest does, as a substring of the test's full path.
"""
import re
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_PATH = ROOT / ".github" / "workflows" / "ci.yml"
ROOT_ONLY = re.compile(r"if (?:ketchup_user_files::)?effective_user\(\) != 0 \{")
# A part of a test run only as root (review 2026-11-13 r95 P3: the Viewer's
# `== 0` branch was not found). Production code asks the same, so only in tests.
ROOT_PART = re.compile(r"if (?:ketchup_user_files::)?effective_user\(\) == 0 \{")
TEST = re.compile(r"\bfn (\w+)\(")
AS_ROOT = re.compile(r'sudo "\$test"(?: ([\w:]+))?')
PACKAGE = re.compile(r"(?:-p|--package) ([\w-]+)")
TARGET_NAME = re.compile(r'\.target\.name == "(\w+)"')


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


def root_runs(workflow):
    """(crates, library only, filter or None) of every `sudo "$test"` of every job."""
    for job in workflow["jobs"].values():
        built = (frozenset(), False)
        for step in job.get("steps", []):
            run = step.get("run", "")
            for line in run.replace("\\\n", " ").splitlines():
                if "cargo test" in line and "--no-run" in line:
                    named = TARGET_NAME.findall(line)
                    crates = [name.replace("_", "-") for name in named] or PACKAGE.findall(line)
                    built = (frozenset(crates), 'index("lib")' in line)
                for match in AS_ROOT.finditer(line):
                    yield built[0], built[1], match.group(1)


def runs_as_root(runs, crate, test, in_library):
    return any(
        crate in crates
        and (in_library or not library_only)
        and (test_filter is None or test_filter in test)
        for crates, library_only, test_filter in runs
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
