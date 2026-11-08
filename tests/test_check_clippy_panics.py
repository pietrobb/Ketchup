import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_clippy_panics", ROOT / "scripts" / "check_clippy_panics.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def package(root, name):
    directory = root / "crates" / name
    (directory / "src").mkdir(parents=True)
    (directory / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n', encoding="utf-8")


def message(lint, file, line, column=5, primary=True):
    return json.dumps({
        "reason": "compiler-message",
        "message": {
            "code": {"code": f"clippy::{lint}"},
            "spans": [{"file_name": file, "is_primary": primary,
                       "line_start": line, "column_start": column}],
        },
    })


def test_the_four_lints_count_per_file_each_place_once():
    # Review 2026-11-08 TST-250: `.map(Result::unwrap)`, `Err(e) => panic!`, `eprint!`
    # and an `eprintln!` in a macro or a closure slipped past the text check; clippy
    # names each of them.
    messages = [
        message("unwrap_used", "crates/a/src/lib.rs", 3),
        message("unwrap_used", "crates/a/src/lib.rs", 3),  # the lib and a bin both
        message("expect_used", "crates/a/src/lib.rs", 4),
        message("panic", "crates/a/src/lib.rs", 9),
        message("print_stderr", "crates/a/src/log.rs", 2),
        message("print_stderr", "crates/a/src/main.rs", 2),  # a command line reports so
        message("print_stderr", "crates/a/src/bin/tool.rs", 2),
        message("unwrap_used", "/home/u/.cargo/registry/x/src/lib.rs", 1),
        message("needless_return", "crates/a/src/lib.rs", 1),
        message("panic", "crates/a/src/lib.rs", 1, primary=False),
        '{"reason": "build-finished", "success": true}',
        "Compiling a v0.1.0",
    ]
    assert checker.counts_of(messages, ROOT) == {
        "crates/a/src/lib.rs unwrap_used": 1,
        "crates/a/src/lib.rs expect_used": 1,
        "crates/a/src/lib.rs panic": 1,
        "crates/a/src/log.rs print_stderr": 1,
    }


def test_a_count_only_shrinks_within_the_packages_of_the_run(tmp_path):
    package(tmp_path, "a")
    package(tmp_path, "b")
    baseline = {"crates/a/src/lib.rs panic": 1, "crates/b/src/lib.rs unwrap_used": 4}
    grown = {"crates/a/src/lib.rs panic": 2, "crates/a/src/new.rs expect_used": 1}
    assert checker.compare(grown, baseline, {"a"}, tmp_path) == [
        "crates/a/src/lib.rs panic: 2 (allowed 1)",
        "crates/a/src/new.rs expect_used: 1 (allowed 0)",
    ]
    # b was not in the run: its entries are neither compared nor dropped.
    assert checker.compare({}, baseline, {"a"}, tmp_path) == []
    assert checker.updated({}, baseline, {"a"}, tmp_path) == {
        "crates/b/src/lib.rs unwrap_used": 4,
    }


def test_the_baseline_records_its_packages(tmp_path):
    package(tmp_path, "a")
    (tmp_path / "scripts").mkdir()
    checker.write_baseline(tmp_path, {"crates/a/src/lib.rs panic": 1}, {"a", "b"})
    assert checker.read_baseline(tmp_path) == ({"crates/a/src/lib.rs panic": 1}, {"a", "b"})


def test_the_packages_come_from_the_command_line():
    assert checker.packages_of(["--update", "-p", "a", "-p", "b"]) == ["a", "b"]
    assert checker.clippy_command(["a"])[:8] == [
        "cargo", "clippy", "--locked", "-p", "a", "--lib", "--bins", "--message-format=json"]
