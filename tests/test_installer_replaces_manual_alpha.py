"""The release installer takes over from Manual Alpha (feedback 2026-11-10).

Manual Alpha (scripts/windows/build-manual-alpha.ps1, removed since) installed
under AppId KetchupManualAlpha with the shortcut "Ketchup Manual Alpha" in its
Start menu group "Ketchup Manual Alpha" and, as a task, on the desktop. After
the release installer had removed Manual Alpha, a tester still found that
desktop shortcut, starting a Ketchup.exe that no longer existed."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = (ROOT / "scripts" / "windows" / "build-installer.ps1").read_text(encoding="utf-8")


def section(name):
    found = re.search(rf"^\[{name}\]\r?\n(.*?)(?=^\[|\Z)", SCRIPT, re.MULTILINE | re.DOTALL)
    assert found, f"no [{name}] section"
    return [line for line in found.group(1).splitlines() if line and not line.startswith(";")]


def test_manual_alpha_is_uninstalled_before_the_release_installs():
    assert "KetchupManualAlpha_is1" in SCRIPT


def test_every_manual_alpha_shortcut_is_deleted_for_the_user_and_for_all_users():
    deleted = section("InstallDelete")
    for desktop in ("{userdesktop}", "{commondesktop}"):
        assert f'Type: files; Name: "{desktop}\\Ketchup Manual Alpha.lnk"' in deleted, desktop
    for programs in ("{userprograms}", "{commonprograms}"):
        assert f'Type: filesandordirs; Name: "{programs}\\Ketchup Manual Alpha"' in deleted, programs


def test_only_manual_alpha_shortcuts_are_deleted():
    # Nothing of the release, nor of the user, matches these names.
    for line in section("InstallDelete"):
        assert "Ketchup Manual Alpha" in line, line
