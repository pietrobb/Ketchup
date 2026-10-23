"""Every release build script builds with the committed Cargo.lock.

Review 2026-10-18 P3: build-android-viewer.ps1 and build-web-viewer.ps1 ran
`cargo build` without --locked, so a Viewer could ship dependencies that CI
and cargo deny never checked.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = [*sorted((ROOT / "scripts" / "windows").glob("*.ps1")), *sorted((ROOT / "scripts" / "macos").glob("*.sh"))]

# `cargo build ...` on one line, or a PowerShell argument list starting with "build".
CARGO_BUILD = re.compile(r'\bcargo\s+build\b|@\(\s*"build"')


def unlocked_builds(text):
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        code = line.split("#", 1)[0]
        if CARGO_BUILD.search(code) and "--locked" not in code:
            found.append(number)
    return found


def test_every_cargo_build_in_a_release_script_is_locked():
    unlocked = {
        script.relative_to(ROOT).as_posix(): lines
        for script in SCRIPTS
        if (lines := unlocked_builds(script.read_text(encoding="utf-8")))
    }
    assert unlocked == {}


def test_the_viewer_scripts_are_among_those_checked():
    names = {script.name for script in SCRIPTS}
    assert {"build-android-viewer.ps1", "build-web-viewer.ps1", "build-viewer-app.sh", "build-installer.ps1"} <= names


def test_an_unlocked_build_in_either_form_is_found():
    assert unlocked_builds('& cargo build -p a --release\n') == [1]
    assert unlocked_builds('$args = @("build", "-p", "a")\n') == [1]
    assert unlocked_builds('& cargo build --locked -p a\n$args = @("build", "--locked")\n') == []
    assert unlocked_builds('# cargo build in a comment\n') == []
