#!/usr/bin/env python3
"""Fail when a test creates its own wgpu instance.

A debug-build wgpu instance enables the D3D12 debug layer and releases its controller,
which unloads D3D12SDKLayers.dll while devices of tests running in parallel may still
use it; the Kečup Viewer tests so corrupted the heap (0xc0000374, #103). Tests take the
instance from their crate's `test_gpu.rs`, which creates it once per test process, and
create only their own devices.

`KNOWN` lists the files that still create instances, with their count; a count may only
go down. (An `egui_kittest` harness without an explicit renderer also creates one when it
renders; that cannot be seen here.)
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_test_sleeps import is_test_file  # noqa: E402
from check_tolerance_literals import TEST_MODULE  # noqa: E402  (a Rust file's unit tests)

ROOT = Path(__file__).resolve().parents[1]
SHARED = "test_gpu.rs"
CREATES = re.compile(
    r"\bInstance::new\(|\bcreate_render_state\(|\bnew_instance\(\)"
    r"|\bWgpuTestRenderer::(?:new|from_setup)\("
)
KNOWN = {
    "crates/ketchup-app/src/tests/interchange.rs": 1,
    "crates/ketchup-app/tests/instanced_rendering.rs": 2,
    "crates/ketchup-app/tests/viewer_release_smoke.rs": 1,
}


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted(root.glob("crates/*/**/*.rs")):
        relative = path.relative_to(root)
        if "target" in relative.parts or relative.name == SHARED:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if not is_test_file(relative):
            text = "".join(match.group(0) for match in TEST_MODULE.finditer(text))
        count = len(CREATES.findall(text))
        if count:
            counts[relative.as_posix()] = count
    return counts


def check(root: Path = ROOT) -> int:
    grown = [f"{key}: {value} (allowed {KNOWN.get(key, 0)})"
             for key, value in sorted(current_counts(root).items()) if value > KNOWN.get(key, 0)]
    if grown:
        print(f"Tests create their own wgpu instance; take it from the crate's {SHARED}:")
        print(*grown, sep="\n  ")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
