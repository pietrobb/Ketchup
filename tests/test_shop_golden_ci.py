"""The golden shop files are checked on every system (review 2026-10-29 proposal 5.1).

The job without OCCT tests ketchup-program and ketchup-manufacturing on Linux and
macOS; Windows runs their golden tests in a step of its own, which fails when the
filter matches no test.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_windows_checks_the_same_golden_shop_files_as_linux_and_macos():
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    windows = ci.split("\n  rust-with-occt:\n", 1)[1].split("\n  rust-with-occt-linux:\n", 1)[0]
    assert "runs-on: windows-latest" in windows
    assert re.search(r"cargo test --locked -p ketchup-program --test integration -- shop_golden::", windows)
    assert re.search(r"cargo test --locked -p ketchup-manufacturing --test integration -- fabrication_validation::", windows)
    assert windows.count('grep -q "test result: ok. 1 passed" "$RUNNER_TEMP/shop-golden.log"') == 1
    without_occt = ci.split("\n  rust-without-occt:\n", 1)[1].split("\n  rust-with-occt:\n", 1)[0]
    assert "os: [ubuntu-latest, macos-latest]" in without_occt
    assert "-p ketchup-program" in without_occt and "-p ketchup-manufacturing" in without_occt


def test_the_golden_shop_files_are_compared_as_bytes():
    golden = ROOT / "crates" / "ketchup-program" / "tests" / "fixtures" / "shop-golden"
    assert sorted(path.name for path in golden.iterdir()) == ["cnc-dxf.zip", "cutting-plan.csv", "quote.csv"]
    attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8")
    assert "crates/ketchup-program/tests/fixtures/shop-golden/** -text" in attributes
    assert "mod shop_golden;" in (ROOT / "crates" / "ketchup-program" / "tests" / "integration.rs").read_text(
        encoding="utf-8"
    )
