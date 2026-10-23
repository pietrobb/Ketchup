"""The release version has one source: [workspace.package] version in Cargo.toml.

Review 2026-10-19 P3: the version stood in three places, Cargo.toml (what the programs
report), the Windows installer's -Version (never compared with it) and the Android
Viewer's manifest.
"""

import os
import re
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = ROOT / "scripts" / "windows"


def workspace_version():
    return tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]


def powershell(script):
    shell = shutil.which("pwsh") or shutil.which("powershell")
    if shell is None:
        if os.environ.get("CI"):
            pytest.fail("CI must run the release scripts' version checks; it has no PowerShell")
        pytest.skip("no PowerShell")
    return subprocess.run(
        [shell, "-NoProfile", "-NonInteractive", "-Command", script],
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )


def test_the_android_manifest_states_no_version_of_its_own():
    manifest = (ROOT / "viewer" / "android" / "AndroidManifest.xml").read_text(encoding="utf-8")
    assert not re.search(r"android:version(Name|Code)\s*=", manifest)


def test_the_scripts_read_the_version_from_cargo_toml():
    helper = (WINDOWS / "WorkspaceVersion.ps1").as_posix()
    result = powershell(
        f". '{helper}'; Get-WorkspaceVersion '{ROOT.as_posix()}'; "
        "Get-AndroidVersionCode '0.2.0'; Get-AndroidVersionCode '1.12.3'"
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.split() == [workspace_version(), "2000", "1012003"]


def test_the_android_build_gives_the_apk_the_workspace_version():
    script = (WINDOWS / "build-android-viewer.ps1").read_text(encoding="utf-8")
    link = next(line for line in script.splitlines() if "aapt2.exe" in line and " link " in line)
    assert "--version-name $version" in link
    assert "--version-code $versionCode" in link
    assert "Get-WorkspaceVersion" in script


def test_the_installer_refuses_a_version_other_than_cargo_toml():
    installer = (WINDOWS / "build-installer.ps1").as_posix()
    result = powershell(f"& '{installer}' -Version 9.9.9 -SkipBuild")
    assert result.returncode != 0
    assert "9.9.9" in result.stderr and workspace_version() in result.stderr, result.stderr
    assert not (ROOT / "artifacts" / "release" / "9.9.9").exists()
