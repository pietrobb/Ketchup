"""Every Windows package of the desktop app is complete (review 2026-11-04 P3).

The old build-release-candidate.ps1 copied ketchup-app.exe with the exact worker and
OCCT but without the program worker, the Visual C++ runtime and the licence notices,
so its package failed every program evaluation and broke the licences on a clean
Windows. A script that packages the app ships all of them, as build-installer.ps1 does.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WINDOWS_SCRIPTS = ROOT / "scripts" / "windows"

PACKAGES_THE_APP = re.compile(r"Join-Path\s+\$BinaryDir\s+\"ketchup-app\.exe\"")
COMPLETE_PACKAGE = {
    "the program worker": re.compile(r"ketchup-program-worker\.exe"),
    "the Visual C++ runtime": re.compile(r"\bCopy-MissingPeImports\b"),
    "a check that no import is missing": re.compile(r"\bGet-UnresolvedPeImports\b"),
    "the licence notices": re.compile(r"\bCopy-PackageNotices\b"),
}


def test_every_script_that_packages_the_app_ships_what_the_app_needs():
    packagers = [path for path in sorted(WINDOWS_SCRIPTS.glob("*.ps1"))
                 if PACKAGES_THE_APP.search(path.read_text(encoding="utf-8-sig"))]
    assert packagers, "no Windows script packages ketchup-app.exe any more"
    incomplete = {
        path.name: [what for what, pattern in COMPLETE_PACKAGE.items()
                    if not pattern.search(path.read_text(encoding="utf-8-sig"))]
        for path in packagers
    }
    assert {name: missing for name, missing in incomplete.items() if missing} == {}
