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
        "// a dowel, a Hinge cup, two shelves, one shelf, the furniture\n"
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
        "crates/a/src/lib.rs:drawer": 1,
    }


def test_core_and_application_crates_name_no_product_domain():
    assert [
        key
        for key in checker.current_counts()
        if key.startswith(("crates/ketchup-model/", "crates/ketchup-application/"))
    ] == []
