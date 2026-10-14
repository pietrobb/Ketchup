#!/usr/bin/env python3
"""Fail when a test creates its own wgpu instance.

A debug-build wgpu instance enables the D3D12 debug layer and releases its controller,
which unloads D3D12SDKLayers.dll while devices of tests running in parallel may still
use it; the Kečup Viewer tests so corrupted the heap (0xc0000374, #103). Tests take the
instance from their crate's `test_gpu.rs`, which creates it once per test process, and
create only their own devices.

An `egui_kittest` harness without an explicit renderer creates an instance of its own
when it renders, so a harness that is rendered (`<name>.render()` or a snapshot; any
unnamed harness in a file that renders) must be built with `.renderer(...)`.
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
    r"|\bWgpuTestRenderer::(?:new|from_setup|default)\(|\.wgpu(?:_setup)?\("
)
HARNESS = re.compile(
    r"(?:let\s+(?:mut\s+)?(\w+)\s*(?::[^=;]*)?=\s*)?(?:egui_kittest::)?Harness::(builder\(\)|new\w*\()"
)
BUILD = re.compile(r"\.build\w*\(")
RENDERS = r"\s*\.\s*(?:render\(\)|try_snapshot|snapshot_options\(|snapshot\((?!\)))"


def renderless_harnesses(text: str) -> int:
    count = 0
    for match in HARNESS.finditer(text):
        name, constructor = match.groups()
        if constructor == "builder()":
            build = BUILD.search(text, match.end())
            if ".renderer(" in text[match.end(): build.start() if build else len(text)]:
                continue
        receiver = rf"\b{name}" if name else r"[\w)\]]"
        if re.search(receiver + RENDERS, text):
            count += 1
    return count


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted(root.glob("crates/*/**/*.rs")):
        relative = path.relative_to(root)
        if "target" in relative.parts or relative.name == SHARED:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if not is_test_file(relative):
            text = "".join(match.group(0) for match in TEST_MODULE.finditer(text))
        count = len(CREATES.findall(text)) + renderless_harnesses(text)
        if count:
            counts[relative.as_posix()] = count
    return counts


def check(root: Path = ROOT) -> int:
    found = [f"{key}: {value}" for key, value in sorted(current_counts(root).items())]
    if found:
        print(f"Tests create their own wgpu instance; take it from the crate's {SHARED}:")
        print(*found, sep="\n  ")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
