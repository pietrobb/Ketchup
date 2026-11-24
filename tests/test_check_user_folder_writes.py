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


def test_every_way_of_writing_counts_and_folders_from_the_environment_too(tmp_path):
    # Review 2026-11-20 ac P3: these writes, and folders from home_dir()/temp_dir(),
    # were not seen.
    write(tmp_path, "crates/app/src/copies.rs",
          'use std::fs::write;\n'
          'fn f() {\n'
          '    let home = dirs::home_dir();\n'
          '    File::create_new(a); fs::copy(a, b); File::options().write(true).open(a);\n'
          '    std::os::unix::fs::symlink(a, b); std::os::windows::fs::symlink_file(a, b);\n'
          '    tempfile::NamedTempFile::new_in(a); tempfile::Builder::new().tempdir_in(a);\n'
          '    write(a, b);\n'
          '    out.write(b); write!(out, "x");\n'
          '}\n'
          'fn write(a: u8) {}\n')
    write(tmp_path, "crates/app/src/scratch.rs", "fn f() { let t = std::env::temp_dir(); fs::copy(t, b); }\n")
    assert checker.current_counts(tmp_path) == {
        "crates/app/src/copies.rs": 8, "crates/app/src/scratch.rs": 1}


def test_folders_links_owners_and_permissions_count_and_dirs_folders_too(tmp_path):
    # Review 2026-11-21 ac P3: these writes, and folders from dirs::config_dir(), were
    # not seen.
    write(tmp_path, "crates/app/src/setup.rs",
          'fn f() {\n'
          '    let config = dirs::config_dir();\n'
          '    std::fs::create_dir(a); tempfile::Builder::new().make_in(a, |path| Ok(()));\n'
          '    std::fs::hard_link(a, b); std::os::unix::fs::chown(a, None, None);\n'
          '    std::os::unix::fs::lchown(a, None, None); std::fs::set_permissions(a, p);\n'
          '}\n')
    assert checker.current_counts(tmp_path) == {"crates/app/src/setup.rs": 6}


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
