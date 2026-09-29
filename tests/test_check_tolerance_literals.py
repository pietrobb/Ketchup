import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_tolerance_literals", ROOT / "scripts" / "check_tolerance_literals.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_counts_literals_outside_the_tolerance_home_and_tests(tmp_path):
    write(tmp_path, "crates/a/src/lib.rs", "const E: f64 = 1.0e-9;\nlet x = 1e-7_f64; let y = 2.5e-3;\n")
    write(tmp_path, "crates/a/src/native.cc", "if (d <= 1.0e-12) {}\n")
    write(tmp_path, "crates/ketchup-core/src/tolerance.rs", "pub const T: f64 = 1.0e-7;\n")
    write(tmp_path, "crates/a/src/scene_tests.rs", "assert!(d < 1.0e-9);\n")
    write(tmp_path, "crates/a/tests/it.rs", "assert!(d < 1.0e-9);\n")
    write(tmp_path, "crates/a/src/names.rs", "let v1e_5 = x1e-5; let n = 10e5;\n")
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs": 3,
        "crates/a/src/native.cc": 1,
    }


def test_ratchet_refuses_growth_and_locks_in_shrinking(tmp_path, capsys):
    (tmp_path / "scripts").mkdir()
    write(tmp_path, "crates/a/src/lib.rs", "const A: f64 = 1.0e-9;\nconst B: f64 = 1.0e-6;\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.check(tmp_path) == 0

    write(tmp_path, "crates/b/src/lib.rs", "const C: f64 = 1.0e-8;\n")
    assert checker.check(tmp_path) == 1
    assert "crates/b/src/lib.rs: 1 (allowed 0)" in capsys.readouterr().out
    assert checker.check(tmp_path, update=True) == 1

    (tmp_path / "crates/b/src/lib.rs").unlink()
    write(tmp_path, "crates/a/src/lib.rs", "const A: f64 = 1.0e-9;\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.read_baseline(tmp_path) == {"crates/a/src/lib.rs": 1}
