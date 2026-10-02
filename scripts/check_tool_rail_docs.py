#!/usr/bin/env python3
"""Fail when the documented tool rail no longer matches the window.

The rail order lives in `TOOLS` in crates/ketchup-app/src/app/drawing.rs and each
tool's shortcut in `KEYMAP` in crates/ketchup-app/src/keymap.rs. The numbered
"Order (tooltip / shortcut)" list in docs/design/README.md describes the same rail;
this check compares the shortcut of every documented entry with the code, in order,
so a tool added to or moved in the rail must be documented in the same change.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RAIL = "crates/ketchup-app/src/app/drawing.rs"
KEYMAP = "crates/ketchup-app/src/keymap.rs"
DOC = "docs/design/README.md"

TOOLS_BLOCK = re.compile(r"const TOOLS: \[\(AppCommand, u8\); \d+\] = \[(.*?)\];", re.DOTALL)
TOOL = re.compile(r"\(AppCommand::(\w+), \d+\)")
BINDING = re.compile(r"on_canvas\(AppCommand::(\w+), &\[chord\(NONE, Key::(\w+)\)")
# "12. Rotate — Q" or "11. Move — M (Ctrl = copy)": the shortcut is the first word after the dash.
DOC_ENTRY = re.compile(r"^\d+\. [^—\n]+ — (\w+)", re.MULTILINE)
DOC_SECTION = re.compile(r"^Order \(tooltip / shortcut\).*?\n(.*?)\n\n", re.MULTILINE | re.DOTALL)


def code_shortcuts(root: Path = ROOT) -> list[str]:
    block = TOOLS_BLOCK.search((root / RAIL).read_text(encoding="utf-8"))
    if block is None:
        raise SystemExit(f"{RAIL}: no TOOLS array")
    keys = dict(BINDING.findall((root / KEYMAP).read_text(encoding="utf-8")))
    return [keys.get(command, f"<{command} has no shortcut>") for command in TOOL.findall(block.group(1))]


def documented_shortcuts(root: Path = ROOT) -> list[str]:
    section = DOC_SECTION.search((root / DOC).read_text(encoding="utf-8"))
    if section is None:
        raise SystemExit(f"{DOC}: no 'Order (tooltip / shortcut)' list")
    return DOC_ENTRY.findall(section.group(1))


def check(root: Path = ROOT) -> int:
    code, documented = code_shortcuts(root), documented_shortcuts(root)
    if code != documented:
        print(f"{DOC} rail order does not match {RAIL} TOOLS and {KEYMAP}:")
        print(f"  window:     {' '.join(code)}")
        print(f"  documented: {' '.join(documented)}")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
