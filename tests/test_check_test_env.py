import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_test_env", ROOT / "scripts" / "check_test_env.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_a_test_reading_its_own_setting_fails(tmp_path):
    write(tmp_path, "crates/a/tests/flow.rs", 'let python = std::env::var_os("PYTHON");\n')
    write(tmp_path, "crates/a/src/lib.rs", (
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn update() -> bool { std::env::var(name).is_ok() }\n"
        "}\n"
    ))
    assert checker.own_variables(tmp_path) == [
        "crates/a/src/lib.rs: env::var(name)",
        'crates/a/tests/flow.rs: env::var("PYTHON")',
    ]
    assert checker.check(tmp_path) == 1


def test_variables_handed_to_a_child_and_production_reads_pass(tmp_path):
    write(tmp_path, "crates/a/tests/flow.rs", (
        'const PROBE: &str = "KETCHUP_PROBE";\n'
        "if std::env::var_os(PROBE).is_some() {}\n"
        'command.env(PROBE, "1");\n'
        'if let Some(root) = std::env::var_os("SYSTEMROOT") {\n'
        '    environment.push((OsString::from("SYSTEMROOT"), root));\n'
        "}\n"
    ))
    write(tmp_path, "crates/a/src/lib.rs", 'let models = std::env::var_os("KETCHUP_MODELS");\n')
    write(tmp_path, "crates/ketchup-test-env/src/lib.rs", "std::env::var_os(PYTHON)\n")
    assert checker.check(tmp_path) == 0


def test_the_repository_tests_read_no_variables_of_their_own():
    assert checker.own_variables(ROOT) == []
