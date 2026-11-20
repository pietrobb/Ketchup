"""SHA256SUMS.txt names the release's files as they are downloaded beside it.

Review 2026-11-08 REL-251: the sums of 0.3.2 named payload/package-manifest.json,
which is no file of the release, so `sha256sum -c SHA256SUMS.txt` in the folder of
the downloads failed on it.
"""

import hashlib
import re
import struct

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
    # Review 2026-11-12 P3: the macOS lines of the 0.3.3 sums were added by hand.
    for path in ["$setupPath", "$zipPath", "$viewerZipPath", "$examplesPath", "$macPath"]:
        assert path in assets, path
    assert "$packageManifestPath" not in assets
    assert "[string[]]$MacPackages" in script
    assert "Assert-MacPackages $Version $MacPackages" in script


def test_a_file_named_twice_is_refused(tmp_path):
    # Review 2026-11-13 P3: a package given twice gave two lines of the same file.
    output = release_folder(tmp_path)
    setup = output / "Ketchup-9.9.9-Setup.exe"
    result = write(output, [setup, output / "Ketchup-9.9.9-Portable.zip", setup])
    assert result.returncode != 0
    assert "Ketchup-9.9.9-Setup.exe" in result.stderr and "twice" in result.stderr, result.stderr
    assert not (output / "SHA256SUMS.txt").exists()


SIGNER = b"Developer ID Application: CENTAURY Plus s.r.o. (HGHK355YL8)"


def disk_image(signer=SIGNER, ticket=b"s8ch\x01\x00\x00\x00ticket"):
    """A disk image laid out as the 0.3.3 packages are: data, an embedded signature with
    the CMS signature in slot 0x10000 and the stapled ticket in 0x10002, the `koly` trailer."""
    data = b"\0" * 4096
    slots = [(0x10000, b"0\x80cms " + signer)] + ([(0x10002, ticket)] if ticket is not None else [])
    blobs, at = b"", 12 + 8 * len(slots)
    index = b""
    for slot, content in slots:
        index += struct.pack(">II", slot, at + len(blobs))
        blobs += struct.pack(">II", 0xFADE0B01, 8 + len(content)) + content
    signature = struct.pack(">III", 0xFADE0CC0, 12 + len(index) + len(blobs), len(slots)) + index + blobs
    trailer = bytearray(512)
    trailer[:4] = b"koly"
    trailer[0x128:0x138] = struct.pack(">QQ", len(data), len(signature))
    return data + signature + bytes(trailer)


def mac_packages(tmp_path, names, image=None):
    packages = []
    for name in names:
        package = tmp_path / "mac" / name
        package.parent.mkdir(exist_ok=True)
        package.write_bytes(disk_image() if image is None else image)
        packages.append(f"'{package.as_posix()}'")
    helper = (WINDOWS / "MacPackages.ps1").as_posix()
    return powershell(f". '{helper}'; Assert-MacPackages '9.9.9' @({','.join(packages)})")


def test_the_macos_packages_are_both_or_none(tmp_path):
    # Review 2026-11-13 P3: $MacPackages took one of the two packages, so a release
    # could ship sums with the Viewer's line but not the app's.
    app, viewer = "Ketchup-9.9.9-macOS.dmg", "Ketchup-Viewer-9.9.9-macOS.dmg"
    assert mac_packages(tmp_path, []).returncode == 0
    both = mac_packages(tmp_path, [app, viewer])
    assert both.returncode == 0, both.stderr
    for names in [[app], [viewer], [app, app], [app, viewer, viewer]]:
        result = mac_packages(tmp_path, names)
        assert result.returncode != 0, names
        assert app in result.stderr and viewer in result.stderr, result.stderr
    other = mac_packages(tmp_path, [app, "Ketchup-Viewer-9.9.8-macOS.dmg"])
    assert other.returncode != 0 and "9.9.8" in other.stderr, other.stderr


def test_a_macos_package_is_signed_by_us_and_its_notarization_stapled(tmp_path):
    # Review 2026-11-13 r95 P3: notarization was only written down; nothing checked that
    # the packages -MacPackages takes are notarized.
    names = ["Ketchup-9.9.9-macOS.dmg", "Ketchup-Viewer-9.9.9-macOS.dmg"]
    refused = [
        ("no notarization ticket", disk_image(ticket=None)),
        ("no notarization ticket", disk_image(ticket=b"\0" * 12)),
        ("not signed with", disk_image(signer=b"Developer ID Application: Someone Else (AAAAAAAAAA)")),
        ("not signed with", b"dmg"),
    ]
    for words, image in refused:
        result = mac_packages(tmp_path, names, image)
        assert result.returncode != 0, words
        assert words in result.stderr and "Ketchup-9.9.9-macOS.dmg" in result.stderr, result.stderr
