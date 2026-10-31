"""Review 2026-10-29 PL-65: the Windows OCCT math gate (scripts/windows/check-occt-math.ps1)
saw only the libraries' imports. The macOS gate also refuses a library exporting its
private crmath copy, which another library could import instead of its own. The gate
runs here on stand-in libraries with a stand-in dumpbin printing what dumpbin would."""

from test_release_version import WINDOWS, powershell

GATE = (WINDOWS / "check-occt-math.ps1").as_posix()
BUILD = WINDOWS / "build-occt.ps1"

IMPORTS = """
Dump of file {dll}

File Type: DLL

  Section contains the following imports:

    KERNEL32.dll
                         2A1 GetLastError
    api-ms-win-crt-runtime-l1-1-0.dll
                          16 _initterm
{extra}
"""

EXPORTS = """
Dump of file {dll}

File Type: DLL

  Section contains the following exports for {name}

    ordinal hint RVA      name

          1    0 00012340 ??0Standard_Failure@@QEAA@XZ
          2    1 00012350 ?Sin@Math@@SANN@Z
{extra}
"""

# A dumpbin that prints <dll>.imports or <dll>.exports beside each stand-in library.
DUMPBIN = """param($nologo, $mode, $path)
Get-Content -LiteralPath ($path + '.' + $mode.TrimStart('/'))
"""


def gate(tmp_path, imports="", exports=""):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name, extra_imports, extra_exports in [
        ("TKernel.dll", "", ""),
        ("TKMath.dll", imports, exports),
    ]:
        dll = bin_dir / name
        dll.write_bytes(b"")
        (bin_dir / f"{name}.imports").write_text(
            IMPORTS.format(dll=dll, extra=extra_imports), encoding="utf-8"
        )
        (bin_dir / f"{name}.exports").write_text(
            EXPORTS.format(dll=dll, name=name[:-4], extra=extra_exports), encoding="utf-8"
        )
    dumpbin = tmp_path / "dumpbin.ps1"
    dumpbin.write_text(DUMPBIN, encoding="utf-8")
    return powershell(f"& '{GATE}' -Bin '{bin_dir.as_posix()}' -Dumpbin '{dumpbin.as_posix()}'")


def test_libraries_with_their_own_private_math_pass(tmp_path):
    result = gate(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr


def test_a_library_taking_sin_from_the_crt_fails(tmp_path):
    result = gate(tmp_path, imports="    api-ms-win-crt-math-l1-1-0.dll\n                          9B sin\n")
    assert result.returncode != 0
    assert "TKMath.dll imports 9B sin" in (result.stdout + result.stderr).splitlines()


def test_a_library_exporting_its_crmath_copy_fails(tmp_path):
    result = gate(tmp_path, exports="          3    2 00012360 sin\n          4    3 00012370 atan2f\n")
    assert result.returncode != 0
    output = result.stdout + result.stderr
    # Whole lines, as written: pwsh wraps, indents and colours a thrown message (CI on Linux).
    lines = output.splitlines()
    assert "TKMath.dll exports 3    2 00012360 sin" in lines, output
    assert "TKMath.dll exports 4    3 00012370 atan2f" in lines, output
    assert "TKernel.dll" not in output, output


def test_the_build_runs_the_gate():
    script = BUILD.read_text(encoding="utf-8")
    assert "check-occt-math.ps1" in script
    assert "/imports" not in script, "the build keeps a gate of its own"
