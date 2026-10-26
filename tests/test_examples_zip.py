"""The release's Ketchup-<version>-Examples.zip is built by a script from README.md.

Review 2026-10-18 P3: README.md tells users to download the Examples zip from the
release, but no script made it.
"""

import re
import shutil
import subprocess
import zipfile
from pathlib import Path

import pytest

from test_release_version import ROOT, WINDOWS, powershell, workspace_version

SCRIPT = (WINDOWS / "build-examples-zip.ps1").as_posix()


def readme_examples():
    text = (ROOT / "README.md").read_text(encoding="utf-8")
    section = text.split("## Examples you can open right away", 1)[1].split("\n## ", 1)[0]
    return re.findall(r"^\|\s*\[`([^`]+)`\]\(([^)]+)\)", section, re.M)


def build(output, downloads, extra=""):
    return powershell(
        "Set-StrictMode -Version Latest; "
        f"& '{SCRIPT}' -OutputDir '{output.as_posix()}' -DownloadedExamples '{downloads.as_posix()}' {extra}"
    )


def downloads_in(tmp_path):
    downloads = tmp_path / "downloads"
    downloads.mkdir()
    for name, target in readme_examples():
        if target.startswith("https://"):
            (downloads / name).write_bytes(b"downloaded " + name.encode())
    return downloads


def test_the_zip_holds_every_readme_example_and_every_program(tmp_path):
    downloads = downloads_in(tmp_path)
    result = build(tmp_path / "out", downloads)
    assert result.returncode == 0, result.stderr
    folder = f"Ketchup-{workspace_version()}-Examples"
    with zipfile.ZipFile(tmp_path / "out" / f"{folder}.zip") as archive:
        names = {info.filename for info in archive.infolist() if not info.is_dir()}
        examples = readme_examples()
        programs = sorted((ROOT / "examples" / "programs").glob("*.star"))
        assert names == {f"{folder}/{name}" for name, _ in examples} | {
            f"{folder}/programs/{program.name}" for program in programs}
        for name, target in examples:
            source = downloads / name if target.startswith("https://") else ROOT / target
            assert archive.read(f"{folder}/{name}") == source.read_bytes(), name
    assert len(examples) >= 8 and len(programs) >= 8


@pytest.mark.parametrize("shell", ["pwsh", "powershell"])
def test_every_powershell_writes_the_entry_names_with_forward_slashes(tmp_path, shell):
    # Review 2026-10-24 DOC-20: Windows PowerShell 5.1 wrote Ketchup-0.2.0-Examples.zip
    # with backslashes, so unzip on macOS and Linux made no folder. zipfile on Windows
    # turns them into slashes, so only the raw names show it.
    found = shutil.which(shell)
    if found is None:
        pytest.skip(f"no {shell}")
    downloads = downloads_in(tmp_path)
    result = subprocess.run(
        [found, "-NoProfile", "-NonInteractive", "-Command",
         f"& '{SCRIPT}' -OutputDir '{(tmp_path / 'out').as_posix()}' -DownloadedExamples '{downloads.as_posix()}'"],
        capture_output=True, text=True, timeout=120, check=False,
    )
    assert result.returncode == 0, result.stderr
    folder = f"Ketchup-{workspace_version()}-Examples"
    with zipfile.ZipFile(tmp_path / "out" / f"{folder}.zip") as archive:
        raw = [info.orig_filename for info in archive.infolist()]
    assert raw and all("\\" not in name and name.startswith(f"{folder}/") for name in raw), raw
    assert f"{folder}/programs/table.star" in raw


def test_a_missing_download_names_the_file_and_where_to_get_it(tmp_path):
    downloads = tmp_path / "downloads"
    downloads.mkdir()
    result = build(tmp_path / "out", downloads)
    assert result.returncode != 0
    name, url = next((name, target) for name, target in readme_examples() if target.startswith("https://"))
    assert name in result.stderr and url in result.stderr.replace("\n", ""), result.stderr
    assert not list((tmp_path / "out").glob("*.zip")) if (tmp_path / "out").exists() else True


def test_another_version_or_an_existing_zip_is_refused(tmp_path):
    downloads = downloads_in(tmp_path)
    result = build(tmp_path / "out", downloads, "-Version 9.9.9")
    assert result.returncode != 0 and "9.9.9" in result.stderr, result.stderr
    assert build(tmp_path / "out", downloads).returncode == 0
    again = build(tmp_path / "out", downloads)
    assert again.returncode != 0 and "overwrite" in again.stderr, again.stderr


def test_the_installer_builds_the_examples_before_the_programs_and_lists_their_checksum():
    script = (WINDOWS / "build-installer.ps1").read_text(encoding="utf-8")
    assert script.index("build-examples-zip.ps1") < script.index("cargo build --locked")
    checksums = script[script.index("$checksums = @("):]
    assert "$examplesPath" in checksums
