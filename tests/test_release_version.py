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
        "Get-AndroidVersionCode '0.2.0'; Get-AndroidVersionCode '1.12.3'; "
        "Get-AndroidVersionCode '0.3.2-dev'; Get-AndroidVersionCode '0.3.1'"
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.split() == [workspace_version(), "20009", "10120039", "30020", "30019"]


def test_a_pre_release_apk_is_below_its_release_and_above_the_one_before():
    # Review 2026-11-04 P3 platform: 0.3.2-dev had the versionCode of 0.3.2.
    helper = (WINDOWS / "WorkspaceVersion.ps1").as_posix()
    result = powershell(
        f". '{helper}'; "
        "(Get-AndroidVersionCode '0.3.1') -lt (Get-AndroidVersionCode '0.3.2-dev') -and "
        "(Get-AndroidVersionCode '0.3.2-dev') -lt (Get-AndroidVersionCode '0.3.2'); "
        "(Get-AndroidVersionCode '209.999.999') -le 2100000000"
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.split() == ["True", "True"]


def mac_bundle_versions(version):
    helper = (ROOT / "scripts" / "macos" / "bundle-version.sh").as_posix()
    return subprocess.run(
        ["bash", "-c", f'source "{helper}" && bundle_versions "$0"', version],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


@pytest.mark.skipif(os.name == "nt" or shutil.which("bash") is None, reason="needs a POSIX bash")
def test_a_mac_bundle_version_is_integers_and_a_pre_release_is_below_its_release():
    # Review 2026-11-04 P3 platform: CFBundleVersion was 0.3.2-dev, which is not
    # period-separated integers.
    pre = mac_bundle_versions("0.3.2-dev")
    assert pre.returncode == 0, pre.stderr
    assert pre.stdout.split() == ["0.3.2", "30020"]
    assert mac_bundle_versions("0.3.2").stdout.split() == ["0.3.2", "30029"]
    assert mac_bundle_versions(workspace_version()).returncode == 0
    refused = mac_bundle_versions("0.3")
    assert refused.returncode != 0 and "0.3" in refused.stderr


def test_the_mac_apps_take_their_bundle_versions_from_the_helper():
    for script in ("build-app.sh", "build-viewer-app.sh"):
        text = (ROOT / "scripts" / "macos" / script).read_text(encoding="utf-8")
        assert "<key>CFBundleShortVersionString</key><string>$short_version</string>" in text, script
        assert "<key>CFBundleVersion</key><string>$bundle_version</string>" in text, script
        assert 'versions="$(bundle_versions "$version")"' in text, script


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
