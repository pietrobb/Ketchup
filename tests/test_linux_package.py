"""The Linux release package: CI builds Ketchup and the Viewer for Linux x86_64 on a
release tag, with the workers and the OCCT libraries beside them, checks that it
starts in a clean container, and sums it. The release script takes it beside the
Windows and macOS packages into SHA256SUMS.txt. This runs on any system; the
workflow runs the scripts themselves."""
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("third_party_notices", ROOT / "scripts" / "third_party_notices.py")
notices = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = notices
SPEC.loader.exec_module(notices)
LINUX = ROOT / "scripts" / "linux"
WORKERS = sorted(path.stem for path in (ROOT / "crates" / "ketchup-scheduler" / "src" / "bin").glob("*.rs"))
WORKFLOW_PATH = ROOT / ".github" / "workflows" / "linux-package.yml"


def workflow() -> str:
    return WORKFLOW_PATH.read_text(encoding="utf-8")


def build_script() -> str:
    return (LINUX / "build-package.sh").read_text(encoding="utf-8")


def check_script() -> str:
    return (LINUX / "check-package.sh").read_text(encoding="utf-8")


def test_the_package_builds_and_ships_the_app_the_viewer_and_every_worker():
    script = build_script()
    build = next(line for line in script.splitlines() if line.startswith("cargo build"))
    shipped = re.search(r"^programs=\((.*)\)$", script, re.MULTILINE).group(1).split()
    for program in ["ketchup-app", "ketchup-viewer", *WORKERS]:
        assert f"--bin {program}" in build, program
        assert program in shipped, program


def test_the_check_greets_every_worker_and_opens_a_document_and_a_view():
    script = check_script()
    greeted = re.search(r"^workers=\((.*)\)$", script, re.MULTILINE).group(1).split()
    assert sorted(greeted) == WORKERS
    assert 'for worker in "${workers[@]}"' in script
    assert "ketchup-app --verify-install" in script
    assert "ketchup-app --inspect-native-document" in script
    assert "ketchup-viewer --verify-open" in script
    # Every library a program needs is in the package or the documented system packages.
    assert "not found" in script and "ldd" in script


def test_the_programs_find_occt_in_the_package_not_on_the_build_machine():
    script = build_script()
    assert "$ORIGIN/lib" in script, "the programs load OCCT from lib/ beside them"
    assert "patchelf --set-rpath '$ORIGIN'" in script, "each library finds the others beside it"
    assert "references the build machine" in script


def test_the_package_carries_the_notices_of_the_programs_it_builds():
    package = notices.PACKAGES["ketchup-linux"]
    assert package.target == "x86_64-unknown-linux-gnu"
    script = build_script()
    build = next(line for line in script.splitlines() if line.startswith("cargo build"))
    assert set(package.programs) == set(re.findall(r"-p (\S+)", build))
    assert "scripts/third_party_notices.py --package ketchup-linux --output" in script
    assert set(package.sections) == {"occt", "acadrust", "starlark", "core-math"}
    licenses = notices.merge([])
    text = notices.render(package, licenses)
    assert "libTK*.so" in text and "scripts/linux/build-occt.sh" in text
    assert ".dll" not in text and "{" not in text


def test_ci_builds_it_on_a_release_tag_and_starts_it_in_a_clean_container():
    on = re.search(r"^on:\n((?:  .*\n)+)", workflow(), re.MULTILINE).group(1)
    assert re.search(r'tags: \["v\*"\]', on), on
    assert "scripts/linux/build-package.sh" in workflow()
    runtime = (LINUX / "runtime-packages.txt").read_text(encoding="utf-8").split()
    assert runtime
    container = re.search(r"docker run[^\n]*(?:\n\s+[^\n]*)*", workflow()).group(0)
    assert "ubuntu:24.04" in container
    assert "runtime-packages.txt" in container
    assert "check-package.sh" in container
    assert "sha256sum" in workflow() and "SHA256SUMS.txt" in workflow()


def test_every_action_of_the_package_workflow_is_pinned_to_a_commit():
    uses = re.findall(r"^\s*-?\s*uses:\s*(\S+)", workflow(), re.MULTILINE)
    assert uses
    for action in uses:
        assert re.fullmatch(r"[\w.-]+/[\w./-]+@[0-9a-f]{40}", action), action


def test_the_occt_cache_key_matches_the_linux_ci_job():
    """The package job reuses the OCCT the Linux CI job keeps, built by the same
    script and clang, instead of an hour of its own."""
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    ubuntu = [key for key in re.findall(r"key:\s*(occt-.+)", ci) if "-ubuntu" in key]
    assert ubuntu and ubuntu[0] in workflow()


def test_the_release_checksums_name_the_linux_package():
    script = (ROOT / "scripts" / "windows" / "build-installer.ps1").read_text(encoding="utf-8")
    assert "[string]$LinuxPackage" in script
    assert "Ketchup-$Version-Linux-x86_64.tar.gz" in script
    assert "$releaseAssets += $linuxPath" in script


def test_the_readme_offers_the_linux_package():
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    assert "have no release package" not in readme
    assert "Ketchup-<version>-Linux-x86_64.tar.gz" in readme
