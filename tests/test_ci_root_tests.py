"""Every test that proves something only as root is run as root in CI.

Review 2026-11-13: the tests of writing the user's files under sudo return early
unless they are root, and no CI step ran them as root, so CI passed them unchecked.
"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
ROOT_ONLY = re.compile(r"if effective_user\(\) != 0 \{")
TEST = re.compile(r"\bfn (\w+)\(")
AS_ROOT = re.compile(r'sudo "\$test"(?: ([\w:]+))?')


def root_only_tests():
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for found in ROOT_ONLY.finditer(text):
            crate = path.relative_to(ROOT / "crates").parts[0]
            yield crate, TEST.findall(text[: found.start()])[-1]


def runs_as_root(crate, test):
    for step in re.split(r"\n(?=      - )", WORKFLOW):
        run = AS_ROOT.search(step)
        built = f"-p {crate} " in step or f'"{crate.replace("-", "_")}"' in step
        if run and built and (run.group(1) is None or run.group(1).rsplit("::", 1)[-1] in test):
            return True
    return False


def test_every_root_only_test_runs_as_root_in_ci():
    tests = list(root_only_tests())
    assert any(crate == "ketchup-app" for crate, _ in tests), "the scan finds no root-only test"
    skipped = [f"{crate}: {test}" for crate, test in tests if not runs_as_root(crate, test)]
    assert not skipped, "\n".join(skipped)
