import importlib.util
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
    headings = ["1. Open CASCADE", "2. acadrust", "3. starlark", "4. CORE-MATH", "5. Rust crates"]
    order = [text.index(f"\n{heading}") for heading in headings]
    assert order == sorted(order)


def test_a_compiled_crate_without_a_license_text_is_refused():
    licenses = notices.merge([report(("MIT", MIT, [("a", ["Jane"])]))])
    compiled = {("a", "1.0.0", True), ("b", "2.0.0", True), ("ketchup-x", "0.3.1", False)}
    with pytest.raises(notices.NoticeError, match="b 2.0.0"):
        notices.verify(viewer(), licenses, compiled)


def test_an_mit_text_without_copyright_needs_the_crate_authors():
    bare = "Permission is hereby granted, free of charge"
    licenses = notices.merge([report(("MIT", bare, [("a", ["Jane"])]))])
    notices.verify(viewer(), licenses, {("a", "1.0.0", True)})
    licenses = notices.merge([report(("MIT", bare, [("a", [])]))])
    with pytest.raises(notices.NoticeError, match="no copyright holder"):
        notices.verify(viewer(), licenses, {("a", "1.0.0", True)})


def test_a_component_section_is_there_exactly_when_its_crate_is_compiled_in():
    licenses = notices.merge([report(("MIT", MIT, [("starlark", ["Meta"])]))])
    with pytest.raises(notices.NoticeError, match="starlark section must be there"):
        notices.verify(viewer(), licenses, {("starlark", "1.0.0", True)})
    with pytest.raises(notices.NoticeError, match="must be left out"):
        notices.verify(notices.PACKAGES["ketchup"], licenses, {("starlark", "1.0.0", True)})


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
