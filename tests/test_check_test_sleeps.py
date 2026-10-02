import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_test_sleeps", ROOT / "scripts" / "check_test_sleeps.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


PRODUCTION = (
    "fn poll() { std::thread::sleep(POLL); }\n"
    "#[cfg(test)]\n"
    "mod tests {\n"
    "    fn wait() { std::thread::sleep(TICK); }\n"
    "}\n"
)


def test_counts_only_test_code(tmp_path):
    write(tmp_path, "crates/a/src/lib.rs", PRODUCTION)
    write(tmp_path, "crates/a/src/worker_tests.rs", "fn t() { sleep(A); sleep(B); }\n")
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { thread::sleep(A); }\n")
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs": 1,
        "crates/a/src/worker_tests.rs": 2,
        "crates/a/tests/flow.rs": 1,
    }


def test_growth_fails_and_shrinking_passes(tmp_path):
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { sleep(A); sleep(B); }\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.check(tmp_path) == 0
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { sleep(A); sleep(B); sleep(C); }\n")
    assert checker.check(tmp_path) == 1
    assert checker.check(tmp_path, update=True) == 1
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { sleep(A); }\n")
    assert checker.check(tmp_path) == 0
    write(tmp_path, "crates/a/tests/new.rs", "fn t() { sleep(A); }\n")
    assert checker.check(tmp_path) == 1


def test_the_repository_is_within_its_baseline():
    assert checker.check(ROOT) == 0
