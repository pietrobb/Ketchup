"""Tracked file names carry no whitespace, so shell pipelines over
`git ls-files` (xargs, wc, for loops) see each file as one word (review REPO-1)."""

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_no_tracked_file_name_contains_whitespace():
    listed = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout.decode("utf-8")
    named_with_whitespace = [name for name in listed.split("\0") if name and any(c.isspace() for c in name)]
    assert named_with_whitespace == []
