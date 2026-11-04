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


def pe_image(*imports):
    """A minimal PE32+ image whose import directory names `imports`."""
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
    struct.pack_into("<H", optional, 0, 0x20B)
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
        f"Copy-Item (Join-Path (Get-VcRuntimeDirectory) 'vcruntime140.dll') '{tmp_path.as_posix()}'; "
        f"Get-PeFilesToSign '{tmp_path.as_posix()}'"
    )

    assert to_sign == ["Ketchup.exe", "TKernel.dll"]


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
    copied = script.index("Copy-MissingPeImports $payloadDir (Get-VcRuntimeDirectory)")
    gated = script.index("Get-UnresolvedPeImports $payloadDir")
    assert copied < gated < script.index("$files = @(") < script.index("& $InnoCompilerPath")
    assert gated < script.index("Compress-Archive -Path (Join-Path $payloadDir")
