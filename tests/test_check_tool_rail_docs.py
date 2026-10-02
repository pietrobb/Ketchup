import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_tool_rail_docs", ROOT / "scripts" / "check_tool_rail_docs.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def fixture(root, documented):
    write(
        root,
        checker.RAIL,
        "const TOOLS: [(AppCommand, u8); 3] = [\n"
        "    (AppCommand::Select, 0),\n"
        "    (AppCommand::Line, 1),\n"
        "    (AppCommand::Move, 2),\n"
        "];\n",
    )
    write(
        root,
        checker.KEYMAP,
        "    on_canvas(AppCommand::Select, &[chord(NONE, Key::Space)]),\n"
        "    on_canvas(AppCommand::Move, &[chord(NONE, Key::M)]),\n"
        "    on_canvas(AppCommand::Line, &[chord(NONE, Key::L)]),\n",
    )
    write(root, checker.DOC, "Order (tooltip / shortcut):\n" + documented + "\n\nNext.\n")


def test_matching_order_and_shortcuts_pass(tmp_path):
    fixture(tmp_path, "1. Select — Space (arrow)\n2. Line — L\n3. Move — M (Ctrl = copy)\n4. Spacer, then Delete.")
    assert checker.code_shortcuts(tmp_path) == ["Space", "L", "M"]
    assert checker.check(tmp_path) == 0


def test_a_missing_or_reordered_tool_fails(tmp_path):
    fixture(tmp_path, "1. Select — Space\n2. Move — M")
    assert checker.check(tmp_path) == 1
    fixture(tmp_path, "1. Select — Space\n2. Move — M\n3. Line — L")
    assert checker.check(tmp_path) == 1


def test_the_repository_rail_is_documented():
    assert checker.check() == 0
