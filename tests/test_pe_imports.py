"""Every executable and library of the Windows package finds the DLLs it imports.

Review 2026-11-02 REL-120: Ketchup.exe, both workers and the OCCT libraries of 0.3.1
import the Visual C++ runtime, which neither the installer nor the portable zip
shipped, so a clean Windows could not start Ketchup. Only the Viewer was checked.
"""

import struct
import sys

import pytest

from test_release_version import WINDOWS, powershell

PE_IMPORTS = (WINDOWS / "PeImports.ps1").as_posix()


def pe_image(*imports, linker=(14, 44)):
    """A minimal PE32+ image whose import directory names `imports`, linked by `linker`."""
    rva, raw = 0x1000, 0x200
    table = 20 * (len(imports) + 1)
    names, descriptors = b"", b""
    for name in imports:
        descriptors += struct.pack("<IIIII", 0, 0, 0, rva + table + len(names), 0)
        names += name.encode() + b"\0"
    data = descriptors + b"\0" * 20 + names
    data += b"\0" * (-len(data) % 0x200 or 0x200)
    dos = b"MZ" + b"\0" * 0x3A + struct.pack("<I", 0x40)
    coff = b"PE\0\0" + struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 240, 0x22)
    optional = bytearray(240)
    struct.pack_into("<HBB", optional, 0, 0x20B, *linker)
    struct.pack_into("<II", optional, 112 + 8, rva if imports else 0, table)
    section = b".idata\0\0" + struct.pack("<IIII", len(data), rva, len(data), raw) + b"\0" * 16
    header = dos + coff + bytes(optional) + section
    return header + b"\0" * (raw - len(header)) + data


def run(script):
    result = powershell(f"$ErrorActionPreference = 'Stop'; . '{PE_IMPORTS}'; {script}")
    assert result.returncode == 0, result.stderr
    return result.stdout.split()


def test_an_import_neither_shipped_nor_part_of_windows_is_reported(tmp_path):
    payload, system = tmp_path / "payload", tmp_path / "system"
    payload.mkdir()
    system.mkdir()
    (payload / "Ketchup.exe").write_bytes(pe_image("vcruntime140.dll", "api-ms-win-crt-runtime-l1-1-0.dll", "TKernel.dll"))
    (payload / "TKernel.dll").write_bytes(pe_image("msvcp140.dll"))
    # A runtime some other program put in the system directory is not part of Windows.
    (system / "msvcp140.dll").write_bytes(pe_image())

    missing = run(f"Get-UnresolvedPeImports '{payload.as_posix()}' '{system.as_posix()}' | ForEach-Object {{ $_ -replace ' ', '' }}")

    assert missing == ["Ketchup.exe:vcruntime140.dll", "TKernel.dll:msvcp140.dll"]


def test_the_runtime_and_what_it_needs_in_turn_ship_beside_the_program(tmp_path):
    payload, runtime, system = tmp_path / "payload", tmp_path / "runtime", tmp_path / "system"
    for folder in (payload, runtime, system):
        folder.mkdir()
    (payload / "Ketchup.exe").write_bytes(pe_image("vcruntime140.dll"))
    (payload / "ketchup-exact-worker.exe").write_bytes(pe_image("msvcp140_2.dll"))
    (runtime / "msvcp140_2.dll").write_bytes(pe_image("msvcp140.dll"))
    (runtime / "msvcp140.dll").write_bytes(pe_image("vcruntime140.dll", "vcruntime140_1.dll"))
    (runtime / "vcruntime140.dll").write_bytes(pe_image())
    (runtime / "vcruntime140_1.dll").write_bytes(pe_image())
    (runtime / "concrt140.dll").write_bytes(pe_image())

    copied = run(
        f"Copy-MissingPeImports '{payload.as_posix()}' '{runtime.as_posix()}' '{system.as_posix()}'; "
        f"'unresolved=' + @(Get-UnresolvedPeImports '{payload.as_posix()}' '{system.as_posix()}').Count"
    )

    # Sorted here: Sort-Object orders "_" against "." by culture, differently on Linux.
    assert sorted(copied[:-1]) == ["msvcp140.dll", "msvcp140_2.dll", "vcruntime140.dll", "vcruntime140_1.dll"]
    assert copied[-1] == "unresolved=0"
    assert not (payload / "concrt140.dll").exists()


@pytest.mark.skipif(sys.platform != "win32", reason="reads the real Windows system directory")
def test_windows_itself_counts_and_the_visual_cpp_runtime_does_not():
    assert run("Test-WindowsSystemDll kernel32.dll; Test-WindowsSystemDll opengl32.dll; Test-WindowsSystemDll vcruntime140.dll") == [
        "True",
        "True",
        "False",
    ]


@pytest.mark.skipif(sys.platform != "win32", reason="reads Authenticode signatures")
def test_every_unsigned_program_and_library_is_to_be_signed_and_the_runtime_is_not(tmp_path):
    # Review 2026-11-04 REL-142: 0.3.2 signed the four programs only, so the OCCT TK*.dll
    # went out unsigned, which Smart App Control blocks.
    (tmp_path / "Ketchup.exe").write_bytes(pe_image("TKernel.dll"))
    (tmp_path / "TKernel.dll").write_bytes(pe_image())
    (tmp_path / "README.txt").write_text("not a program")

    to_sign = run(
        f"Copy-Item (Join-Path (Get-VcRuntimeDirectory ([version]'14.0')) 'vcruntime140.dll') '{tmp_path.as_posix()}'; "
        f"Get-PeFilesToSign '{tmp_path.as_posix()}'"
    )

    assert to_sign == ["Ketchup.exe", "TKernel.dll"]


@pytest.mark.skipif(sys.platform != "win32", reason="reads Authenticode signatures")
def test_a_library_signed_only_through_a_catalog_of_this_machine_is_to_be_signed(tmp_path):
    # Review 2026-11-04 r19 P3 (REL-142): Get-AuthenticodeSignature calls a file valid
    # when its hash is in a catalog of the build machine, which the user's machine does
    # not have, so such a file went out without a signature of its own.
    to_sign = run(
        "$catalog = Get-ChildItem \"$env:SystemRoot\\System32\" -Filter *.dll | "
        "Where-Object { $_.Length -lt 300kb } | Where-Object { "
        "$s = Get-AuthenticodeSignature $_.FullName; $s.Status -eq 'Valid' -and $s.SignatureType -eq 'Catalog' } | "
        "Select-Object -First 1; "
        f"Copy-Item $catalog.FullName (Join-Path '{tmp_path.as_posix()}' 'TKMath.dll'); "
        f"(Get-AuthenticodeSignature (Join-Path '{tmp_path.as_posix()}' 'TKMath.dll')).Status; "
        f"Get-PeFilesToSign '{tmp_path.as_posix()}'"
    )

    assert to_sign == ["Valid", "TKMath.dll"]


def test_the_signing_checks_ask_for_a_signature_in_the_file_itself():
    # Review 2026-11-04 r19 P3 (REL-142): a catalog signature passed the signing checks too.
    script = (WINDOWS / "build-installer.ps1").read_text(encoding="utf-8")
    assert script.count("Get-AuthenticodeSignature") == script.count("Test-EmbeddedSignature")


def test_a_signed_installer_signs_the_whole_payload_and_ships_nothing_unsigned():
    script = (WINDOWS / "build-installer.ps1").read_text(encoding="utf-8")
    signed = script.index("foreach ($name in @(Get-PeFilesToSign $payloadDir))")
    checked = script.index("$unsigned = @(Get-PeFilesToSign $payloadDir)", signed)
    assert script.index("Copy-MissingPeImports $payloadDir") < signed
    assert checked < script.index("& $InnoCompilerPath")
    assert checked < script.index("Compress-Archive -Path (Join-Path $payloadDir")
    assert 'Invoke-AzureSigning (Join-Path $payloadDir "Ketchup.exe")' not in script


def test_the_installer_gates_the_whole_payload_before_packaging_it():
    script = (WINDOWS / "build-installer.ps1").read_text(encoding="utf-8")
    copied = script.index("Copy-MissingPeImports $payloadDir (Get-VcRuntimeDirectory $toolset)")
    assert script.index("$toolset = Get-PeToolsetVersion $payloadDir") < copied
    gated = script.index("Get-UnresolvedPeImports $payloadDir")
    assert copied < gated < script.index("$files = @(") < script.index("& $InnoCompilerPath")
    assert gated < script.index("Compress-Archive -Path (Join-Path $payloadDir")


def redist(root, version):
    crt = root / version / "x64" / "Microsoft.VC143.CRT"
    crt.mkdir(parents=True)
    return crt


def test_the_runtime_shipped_is_the_one_of_the_toolset_that_linked_the_package(tmp_path):
    # Review 2026-11-04 P3 platform: the newest installed Visual Studio's runtime was
    # shipped, whatever toolset had linked the programs, an older one too.
    payload, redists = tmp_path / "payload", tmp_path / "MSVC"
    payload.mkdir()
    (payload / "Ketchup.exe").write_bytes(pe_image("vcruntime140.dll", linker=(14, 44)))
    (payload / "TKernel.dll").write_bytes(pe_image(linker=(14, 35)))
    (payload / "README.txt").write_text("not a program")
    for version in ("14.29.30133", "14.44.35112", "14.44.35208", "14.50.35710"):
        redist(redists, version)
    (redists / "14.38.33130").mkdir()  # a version without an x64 runtime
    found = lambda toolset: run(
        f"Get-VcRuntimeDirectory ([version]'{toolset}') @(Get-ChildItem '{redists.as_posix()}' -Directory | "
        "ForEach-Object FullName) | Split-Path -Parent | Split-Path -Parent | Split-Path -Leaf"
    )

    assert run(f"Get-PeToolsetVersion '{payload.as_posix()}' | ForEach-Object ToString") == ["14.44"]
    assert found("14.44") == ["14.44.35208"]
    assert found("14.36") == ["14.44.35208"]
    assert found("14.45") == ["14.50.35710"]
    refused = powershell(
        f"$ErrorActionPreference = 'Stop'; . '{PE_IMPORTS}'; "
        f"Get-VcRuntimeDirectory ([version]'14.51') @('{(redists / '14.50.35710').as_posix()}')"
    )
    assert refused.returncode != 0 and "14.51" in refused.stderr


def test_the_runtime_visual_studio_installed_with_the_toolset_ships_for_it(tmp_path):
    # Release 0.3.2: Visual Studio 2022 17.5 installs tools 14.35.32215 with the runtime
    # 14.34.31931, whose DLLs did not change for 14.35, so a package it linked found no
    # runtime "14.35 or newer" to ship and the release could not be built.
    installation = tmp_path / "BuildTools"
    build = installation / "VC" / "Auxiliary" / "Build"
    build.mkdir(parents=True)
    (build / "Microsoft.VCToolsVersion.default.txt").write_text("14.35.32215\n")
    (build / "Microsoft.VCRedistVersion.default.txt").write_text("14.34.31931\n")
    redists = installation / "VC" / "Redist" / "MSVC"
    redist(redists, "14.34.31931")
    redist(redists, "14.33.31629")  # installed without a toolset of its own here
    found = lambda toolset: powershell(
        f"$ErrorActionPreference = 'Stop'; . '{PE_IMPORTS}'; $vs = @('{installation.as_posix()}'); "
        f"Get-VcRuntimeDirectory ([version]'{toolset}') (Get-VcRedistDirectories $vs) (Get-VcRedistToolsets $vs) | "
        "Split-Path -Parent | Split-Path -Parent | Split-Path -Leaf"
    )

    assert found("14.35").stdout.split() == ["14.34.31931"]
    assert found("14.34").stdout.split() == ["14.34.31931"]
    refused = found("14.36")
    assert refused.returncode != 0 and "14.36" in refused.stderr


def test_a_machine_without_visual_studio_has_no_installations():
    # Release 0.3.2 CI: Linux and macOS have no ProgramFiles(x86), and looking for
    # Visual Studio there failed every runtime lookup instead of finding none.
    assert run("${env:ProgramFiles(x86)} = $null; @(Get-VsInstallations).Count") == ["0"]
