"""The Assistant protocol's limits live in crates/ketchup-assistant/src/protocol.rs.

The Python protocol carries them only in the block generated from that file; the Rust
test `python_protocol_block_matches_rust` keeps the block current, and this test keeps
hand-written protocol constants from reappearing beside it.
"""

import ast
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROTOCOL = ROOT / "sdk" / "python" / "ketchup_assistant_protocol.py"
PROTOCOL_NAME = re.compile(r"^(MAX_[A-Z0-9_]+|PROTOCOL_VERSION|[A-Z0-9_]+_SCHEMA|LOCAL_[A-Z0-9_]+)$")
# Arithmetic facts rather than protocol choices.
NOT_PROTOCOL = {"MAX_U64"}


def generated_line_range(lines):
    begin = next(i for i, line in enumerate(lines) if line.startswith("# BEGIN generated from crates/ketchup-assistant/src/protocol.rs"))
    end = next(i for i in range(begin, len(lines)) if lines[i] == "# END generated")
    return begin + 1, end + 1


def handwritten_protocol_constants(source):
    lines = source.splitlines()
    first, last = generated_line_range(lines)
    found = []
    for node in ast.parse(source).body:
        if not isinstance(node, (ast.Assign, ast.AnnAssign)):
            continue
        targets = node.targets if isinstance(node, ast.Assign) else [node.target]
        for target in targets:
            if (
                isinstance(target, ast.Name)
                and PROTOCOL_NAME.match(target.id)
                and target.id not in NOT_PROTOCOL
                and not first < node.lineno <= last
            ):
                found.append(f"{target.id} (line {node.lineno})")
    return found


class AssistantProtocolConstantsTest(unittest.TestCase):
    def test_protocol_constants_come_only_from_the_generated_block(self):
        self.assertEqual(handwritten_protocol_constants(PROTOCOL.read_text(encoding="utf-8")), [])

    def test_generated_block_defines_the_limits(self):
        source = PROTOCOL.read_text(encoding="utf-8")
        first, last = generated_line_range(source.splitlines())
        names = [line.split(" = ")[0] for line in source.splitlines()[first:last - 1]]
        self.assertIn("PROTOCOL_VERSION", names)
        self.assertIn("MAX_CAD_EDIT_OPERATIONS", names)

    def test_guard_reports_a_handwritten_limit(self):
        source = (
            "# BEGIN generated from crates/ketchup-assistant/src/protocol.rs; do not edit.\n"
            "MAX_CAD_EDIT_OPERATIONS = 64\n"
            "# END generated\n"
            "MAX_U64 = (1 << 64) - 1\n"
            "MAX_CAD_SELECTOR_TARGETS = 100\n"
        )
        self.assertEqual(handwritten_protocol_constants(source), ["MAX_CAD_SELECTOR_TARGETS (line 5)"])


if __name__ == "__main__":
    unittest.main()
