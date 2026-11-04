import importlib.util
import re
import sys
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "third_party_notices", ROOT / "scripts" / "third_party_notices.py"
)
notices = importlib.util.module_from_spec(SPEC)
# Its dataclasses resolve their annotations through the module registry.
sys.modules[SPEC.name] = notices
SPEC.loader.exec_module(notices)

MIT = "MIT License\n\nCopyright (c) 2020 Jane Doe\n\nPermission is hereby granted, free of charge"


def report(*licenses):
    return {
        "licenses": [
            {
                "id": license_id,
                "name": f"{license_id} License",
                "text": text,
                "used_by": [
                    {"crate": {"name": name, "version": "1.0.0", "authors": authors}}
                    for name, authors in crates
                ],
            }
            for license_id, text, crates in licenses
        ]
    }


def viewer():
    return notices.PACKAGES["viewer"]


def test_a_crate_in_two_programs_is_listed_once_under_its_text():
    first = report(("MIT", MIT, [("a", ["Jane"])]))
    second = report(("MIT", MIT, [("a", ["Jane"]), ("b", ["Bob"])]))
    merged = notices.merge([first, second])
    assert len(merged) == 1
    assert sorted(merged[0]["crates"]) == [("a", "1.0.0"), ("b", "1.0.0")]


def test_the_notice_carries_each_copyright_and_permission_text():
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane Doe <jane@example.com>"])]))])
    text = notices.render(viewer(), licenses)
    assert "Copyright (c) 2020 Jane Doe" in text
    assert "Permission is hereby granted" in text
    assert "  a 1.0.0 - Jane Doe <jane@example.com>" in text
    # The Viewer contains no component with its own section.
    assert "OCCT" not in text and "starlark" not in text


def test_the_kecup_notice_names_its_components_before_the_crates():
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane"])]))])
    text = notices.render(notices.PACKAGES["ketchup"], licenses)
    headings = [
        "1. Open CASCADE",
        "2. acadrust",
        "3. starlark",
        "4. CORE-MATH",
        "5. Microsoft Visual C++ runtime",
        "6. Rust crates",
    ]
    order = [text.index(f"\n{heading}") for heading in headings]
    assert order == sorted(order)


def test_a_compiled_crate_without_a_license_text_is_refused():
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane"])]))])
    compiled = {("a", "1.0.0", False), ("b", "2.0.0", False), ("ketchup-x", "0.3.1", True)}
    with pytest.raises(notices.NoticeError, match="b 2.0.0"):
        notices.verify(viewer(), licenses, compiled)


def test_a_vendored_crate_without_a_license_text_is_refused():
    """Review 2026-11-02 CI-121: only crates from a registry were checked, so a
    crate patched in from third_party/ (starlark, process-wrap) or taken from
    git could ship without its license text. Only the workspace's own
    packages are exempt."""
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane"])]))])
    compiled = {("a", "1.0.0", False), ("process-wrap", "10.0.0", False), ("ketchup-x", "0.3.1", True)}
    with pytest.raises(notices.NoticeError, match="process-wrap 10.0.0"):
        notices.verify(viewer(), licenses, compiled)


def test_a_crate_of_the_workspace_is_ours_and_any_other_is_third_party(monkeypatch):
    out = "\n".join(
        [
            r"ketchup-app v0.3.1 (C:\k\crates\ketchup-app)",
            "serde v1.0.229",
            "serde v1.0.229 (*)",
            r"process-wrap v10.0.0 (C:\k\third_party\process-wrap)",
            "gitcrate v0.1.0 (https://github.com/x/gitcrate#0123abcd)",
        ]
    )
    monkeypatch.setattr(notices, "run", lambda args: out)
    monkeypatch.setattr(notices, "workspace", lambda: {"ketchup-app": "Cargo.toml"})
    assert notices.tree("ketchup-app", notices.TARGET) == {
        ("ketchup-app", "0.3.1", True),
        ("serde", "1.0.229", False),
        ("process-wrap", "10.0.0", False),
        ("gitcrate", "0.1.0", False),
    }


def test_an_mit_text_without_copyright_needs_the_crate_authors():
    bare = "Permission is hereby granted, free of charge"
    licenses = notices.merge([report(("MIT", bare, [("a", ["Jane"])]))])
    notices.verify(viewer(), licenses, {("a", "1.0.0", False)})
    licenses = notices.merge([report(("MIT", bare, [("a", [])]))])
    with pytest.raises(notices.NoticeError, match="no copyright holder"):
        notices.verify(viewer(), licenses, {("a", "1.0.0", False)})


def test_a_component_section_is_there_exactly_when_its_crate_is_compiled_in():
    licenses = notices.merge([report(("MIT", MIT, [("starlark", ["Meta"])]))])
    with pytest.raises(notices.NoticeError, match="starlark section must be there"):
        notices.verify(viewer(), licenses, {("starlark", "1.0.0", False)})
    with pytest.raises(notices.NoticeError, match="must be left out"):
        notices.verify(notices.PACKAGES["ketchup"], licenses, {("starlark", "1.0.0", False)})


def test_the_windows_package_names_the_visual_cpp_runtime_it_ships_on_microsofts_terms():
    """Review 2026-11-04 REL-141: the Windows package ships Microsoft's Visual C++
    runtime (REL-120), but its notices did not name it, nor the terms Microsoft
    sets for its distributable code. The section is there exactly for the
    Windows package that compiles in OCCT, and names each runtime library."""
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane"])]))])
    windows = notices.render(notices.PACKAGES["ketchup"], licenses)
    for needle in ("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "Microsoft Corporation",
                   "Distributable Code", "https://visualstudio.microsoft.com/license-terms/"):
        assert needle in windows, needle
    assert "Visual C++ runtime" not in notices.render(notices.PACKAGES["ketchup-macos"], licenses)
    compiled = {("a", "1.0.0", False), ("ketchup-exact", "0.3.2", True)}
    without = notices.Package("Kecup", ("ketchup-app",), ("occt", "core-math"))
    with pytest.raises(notices.NoticeError, match="msvc-runtime section must be there"):
        notices.verify(without, licenses, compiled)
    compiled |= {("acadrust", "0.1.0", True), ("starlark", "0.13.0", True)}
    notices.verify(notices.PACKAGES["ketchup-macos"], licenses, compiled)


def test_cargo_about_accepts_what_cargo_deny_allows():
    about = tomllib.loads((ROOT / "about.toml").read_text(encoding="utf-8"))
    deny = tomllib.loads((ROOT / "deny.toml").read_text(encoding="utf-8"))
    allowed = set(deny["licenses"]["allow"])
    allowed |= {license for exception in deny["licenses"]["exceptions"] for license in exception["allow"]}
    assert set(about["accepted"]) == allowed


def test_ci_checks_the_notices_with_a_verified_cargo_about():
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    assert "ABOUT_SHA256:" in workflow
    assert "$ABOUT_SHA256  $RUNNER_TEMP/about.tar.gz\" | sha256sum --check --strict" in workflow
    assert "python3 scripts/third_party_notices.py --check" in workflow


def test_every_section_has_its_text():
    for name in notices.SECTIONS:
        title, _, body = (notices.NOTICES / f"{name}.txt").read_text(encoding="utf-8").partition("\n")
        assert title and body.strip()


def test_each_macos_bundle_carries_the_notices_of_the_programs_it_builds():
    """Review 2026-11-01 PL-62: the .app and the .dmg carried no notices. Each
    macOS build script makes them for macOS and for exactly the packages it
    builds, and ships them in the bundle and beside it in the .dmg."""
    for script_name, name in (("build-app.sh", "ketchup-macos"), ("build-viewer-app.sh", "viewer-macos")):
        script = (ROOT / "scripts" / "macos" / script_name).read_text(encoding="utf-8")
        package = notices.PACKAGES[name]
        assert package.target.endswith("-apple-darwin"), name
        build = next(line for line in script.splitlines() if line.strip().startswith("cargo build"))
        assert set(package.programs) == set(re.findall(r"-p (\S+)", build)), name
        assert f"scripts/third_party_notices.py --package {name} --output" in script, script_name
        assert 'for notice in "${notices[@]}"' in script, script_name


def test_the_apk_and_the_web_viewer_carry_the_notices_of_what_they_build():
    """Review 2026-11-02 REL-121: the Android APK and the web Viewer carried no
    notices. Each build script makes them for its own program and target and
    ships them: inside the APK as an asset, beside the WebAssembly module."""
    for script_name, name, triple, shipped in (
        ("build-android-viewer.ps1", "viewer-android", "aarch64-linux-android", '"assets/THIRD_PARTY_NOTICES.txt"'),
        ("build-web-viewer.ps1", "viewer-web", "wasm32-unknown-unknown", "--output $OutputDir"),
    ):
        script = (ROOT / "scripts" / "windows" / script_name).read_text(encoding="utf-8-sig")
        package = notices.PACKAGES[name]
        assert package.target == triple, name
        assert triple in script, script_name
        built = set(re.findall(r'-p"?,? "?(ketchup-[\w-]+)', script))
        assert set(package.programs) == built, name
        assert f"scripts\\third_party_notices.py\") --package {name} --output" in script, script_name
        assert shipped in script, script_name


def test_the_component_sections_name_the_files_of_their_platform():
    """Review 2026-11-01 PL-62: OCCT's LGPL notice names the libraries a user may
    replace and how they are built, on each platform as it ships them."""
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane"])]))])
    windows = notices.render(notices.PACKAGES["ketchup"], licenses)
    assert "TK*.dll in the program folder" in windows
    assert "scripts/windows/build-occt.ps1" in windows
    macos = notices.render(notices.PACKAGES["ketchup-macos"], licenses)
    assert "libTK*.dylib in Contents/Frameworks" in macos
    assert "scripts/macos/build-occt.sh" in macos
    assert ".dll" not in macos and "{" not in macos
