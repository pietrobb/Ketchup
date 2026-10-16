"""Every workspace crate is linted and tested by some CI job.

A crate CI never names with `-p` is never built there, so an API change that
breaks only that crate still shows "CI passed" (ketchup-headless was left out
of the OCCT job this way). A new crate must be added to a job's crate list.
"""

import re
import sys
import tomllib
from pathlib import Path

PACKAGE_FLAG = re.compile(r"(?:^|\s)-p\s+([\w-]+)")


def workspace_crates(root):
    """The package names of the workspace members of the root Cargo.toml."""
    members = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["members"]
    return sorted(
        tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]["name"]
        for member in members
    )


def ci_crates(workflows):
    """Every crate a workflow names with `-p`."""
    return {name for text in workflows for name in PACKAGE_FLAG.findall(text)}


def unchecked_crates(crates, named):
    return [f"{crate}: no CI job names it with -p; add it to a job's crate list" for crate in crates if crate not in named]


def main():
    root = Path(__file__).resolve().parents[1]
    workflows = [path.read_text(encoding="utf-8") for path in sorted((root / ".github/workflows").glob("*.yml"))]
    problems = unchecked_crates(workspace_crates(root), ci_crates(workflows))
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
