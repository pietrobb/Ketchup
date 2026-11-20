import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_user_folder_writes", ROOT / "scripts" / "check_user_folder_writes.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n', encoding="utf-8")
    return path


def test_counts_writes_of_files_that_name_a_users_folder(tmp_path):
    # Review 2026-11-13 r95 P3: the register of windows was written into
    # $XDG_RUNTIME_DIR under sudo -E as root, outside the crate.
    write(tmp_path, "crates/app/src/register.rs",
          'use std::fs::{self, OpenOptions};\n'
          'fn root() { std::env::var_os("XDG_RUNTIME_DIR"); }\n'
          'fn publish() { fs::write(a, b); fs::rename(a, b); }\n'
          '// fs::write(c, d) in a comment\n'
          '#[cfg(test)]\nmod tests {\n    fn t() {\n        std::fs::write(a, b);\n    }\n}\n')
    write(tmp_path, "crates/app/src/document.rs", "fn save() { std::fs::write(a, b); }\n")
    write(tmp_path, "crates/ketchup-user-files/src/lib.rs",
          'fn home() { std::env::var_os("HOME"); std::fs::write(a, b); }\n')
    assert checker.current_counts(tmp_path) == {"crates/app/src/register.rs": 2}


def test_growth_fails_and_shrinking_passes(tmp_path):
    path = write(tmp_path, "crates/app/src/settings.rs",
                 "fn f(folder: UserFolder) { std::fs::write(a, b); }\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.check(tmp_path) == 0
    path.write_text("fn f(folder: UserFolder) { std::fs::write(a, b); File::create(c); }\n", encoding="utf-8")
    assert checker.check(tmp_path) == 1
    assert checker.check(tmp_path, update=True) == 1
    path.write_text("fn f(folder: UserFolder) { folder.as_owner(w); }\n", encoding="utf-8")
    assert checker.check(tmp_path) == 0


def test_the_repository_is_within_its_baseline():
    assert checker.check(ROOT) == 0
