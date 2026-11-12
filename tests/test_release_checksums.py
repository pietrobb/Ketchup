"""SHA256SUMS.txt names the release's files as they are downloaded beside it.

Review 2026-11-08 REL-251: the sums of 0.3.2 named payload/package-manifest.json,
which is no file of the release, so `sha256sum -c SHA256SUMS.txt` in the folder of
the downloads failed on it.
"""

import hashlib
import re

from test_release_version import WINDOWS, powershell

SCRIPT = (WINDOWS / "write-checksums.ps1").as_posix()


def write(output, assets):
    listed = ",".join(f"'{asset.as_posix()}'" for asset in assets)
    return powershell(f"& '{SCRIPT}' -OutputDir '{output.as_posix()}' -Assets @({listed})")


def release_folder(tmp_path):
    output = tmp_path / "release"
    (output / "payload").mkdir(parents=True)
    for name in ["Ketchup-9.9.9-Setup.exe", "Ketchup-9.9.9-Portable.zip"]:
        (output / name).write_bytes(name.encode())
    (output / "payload" / "package-manifest.json").write_text("{}")
    return output


def test_each_line_names_a_file_beside_the_sums_with_its_hash(tmp_path):
    output = release_folder(tmp_path)
    assets = [output / "Ketchup-9.9.9-Setup.exe", output / "Ketchup-9.9.9-Portable.zip"]
    result = write(output, assets)
    assert result.returncode == 0, result.stderr
    sums = (output / "SHA256SUMS.txt").read_bytes().decode("utf-8")
    assert "\r" not in sums, "GNU sha256sum -c reads a CR as part of the file name"
    lines = re.findall(r"^([0-9a-f]{64})  (\S+)$", sums, re.M)
    assert [name for _, name in lines] == [asset.name for asset in assets], sums
    for digest, name in lines:
        assert digest == hashlib.sha256((output / name).read_bytes()).hexdigest(), name


def test_a_file_not_downloaded_beside_the_sums_is_refused(tmp_path):
    output = release_folder(tmp_path)
    inside = output / "payload" / "package-manifest.json"
    result = write(output, [output / "Ketchup-9.9.9-Setup.exe", inside])
    assert result.returncode != 0
    assert "package-manifest.json" in result.stderr, result.stderr
    assert not (output / "SHA256SUMS.txt").exists()
    missing = write(output, [output / "Ketchup-9.9.9-Examples.zip"])
    assert missing.returncode != 0 and "Examples" in missing.stderr, missing.stderr


def test_the_installer_lists_the_files_it_releases_and_nothing_else():
    script = (WINDOWS / "build-installer.ps1").read_text(encoding="utf-8")
    assert script.count("SHA256SUMS") == 0, "the sums are written by write-checksums.ps1 alone"
    call = script[script.index("write-checksums.ps1"):].split("\n", 1)[0]
    assets = script[script.index("$releaseAssets = @("):script.index("write-checksums.ps1")]
    assert "-Assets $releaseAssets" in call
    for path in ["$setupPath", "$zipPath", "$viewerZipPath", "$examplesPath"]:
        assert path in assets, path
    assert "$packageManifestPath" not in assets
