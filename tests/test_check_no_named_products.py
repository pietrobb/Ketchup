import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_no_named_products", ROOT / "scripts" / "check_no_named_products.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_counts_domain_words_in_production_source_only(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "// a dowel, a Hinge cup, two shelves, one shelf, the furniture, its grain\n"
        "#[cfg(test)]\nmod tests {\n    const SHELF: &str = \"shelf\";\n}\n"
        "fn after() {} // a drawer\n",
    )
    write(tmp_path, "crates/a/src/view_tests.rs", "// dowel\n")
    write(tmp_path, "crates/a/tests/it.rs", "// dowel\n")
    write(tmp_path, "crates/ketchup-model/src/persistence/legacy.rs", "// dowel_joints\n")
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs:dowel": 1,
        "crates/a/src/lib.rs:hinge": 1,
        "crates/a/src/lib.rs:shelves": 1,
        "crates/a/src/lib.rs:shelf": 1,
        "crates/a/src/lib.rs:furniture": 1,
        "crates/a/src/lib.rs:grain": 1,
        "crates/a/src/lib.rs:drawer": 1,
    }


def test_counts_replaced_named_shape_types_under_any_prefix(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "enum ExactBRepSheetMetalEdge {}\nstruct AssistantThreadProfile;\n"
        "AppCommand::Thread => {}\nAppCommand::ThreadSafe => {}\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs:SheetMetalEdge": 1,
        "crates/a/src/lib.rs:ThreadProfile": 1,
        "crates/a/src/lib.rs:AppCommand::Thread": 1,
    }


def test_no_named_sheet_metal_edge_or_thread_shape_remains():
    assert [key for key in checker.current_counts() if key[key.rindex(":") + 1 :][0].isupper()] == []


# Words added by the 2026-10-01 review; their existing occurrences may only shrink.
SHRINKING = {"panel", "board", "beam", "timber", "lumber", "weldment", "cup_bore", "apron"}


def test_counts_board_and_frame_words_from_the_second_review(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "// a panel, a board, a beam, timber, lumber, a weldment, a cup_bore, an apron\n"
        "struct SidePanel;\n",
    )
    assert checker.current_counts(tmp_path) == {
        f"crates/a/src/lib.rs:{word}": 1
        for word in SHRINKING
    }


def test_core_and_application_crates_name_no_product_domain():
    assert [
        key
        for key in checker.current_counts()
        if key.startswith(("crates/ketchup-model/", "crates/ketchup-application/"))
        and key.rsplit(":", 1)[1] not in SHRINKING
    ] == []
