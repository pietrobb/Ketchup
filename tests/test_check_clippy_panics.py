import importlib.util
import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_clippy_panics", ROOT / "scripts" / "check_clippy_panics.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)
HYGIENE_SPEC = importlib.util.spec_from_file_location(
    "check_error_hygiene", ROOT / "scripts" / "check_error_hygiene.py"
)
hygiene = importlib.util.module_from_spec(HYGIENE_SPEC)
HYGIENE_SPEC.loader.exec_module(hygiene)


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
    checker.write_baseline(tmp_path, "linux", {"crates/a/src/lib.rs panic": 1}, {"a", "b"})
    assert checker.read_baseline(tmp_path, "linux") == ({"crates/a/src/lib.rs panic": 1}, {"a", "b"})
    assert checker.read_baseline(tmp_path, "windows") == ({}, set())


def test_a_count_on_one_system_is_compared_with_that_systems_count(tmp_path, monkeypatch, capsys):
    # Review 2026-11-08 CI-250: ketchup-mcp's Windows-only code is seen only by clippy on
    # Windows, so an unwrap added there must fail the Windows run, whatever Linux counts.
    package(tmp_path, "a")
    (tmp_path / "scripts").mkdir()
    checker.write_baseline(tmp_path, "linux", {"crates/a/src/acl.rs unwrap_used": 3}, {"a"})
    checker.write_baseline(tmp_path, "windows", {}, {"a"})
    found = [message("unwrap_used", "crates/a/src/acl.rs", 7)]
    monkeypatch.setattr(checker.subprocess, "run", lambda *_, **__: type(
        "Run", (), {"returncode": 0, "stdout": "\n".join(found)})())
    monkeypatch.setattr(checker, "compiled_packages", lambda packages, *_: set(packages))
    assert checker.check(["a"], root=tmp_path, system="linux") == 0
    assert checker.check(["a"], root=tmp_path, system="windows") == 1
    assert "crates/a/src/acl.rs unwrap_used: 1 (allowed 0)" in capsys.readouterr().out
    # --update does not let it in either; the Windows count stays as it was.
    assert checker.check(["a"], update=True, root=tmp_path, system="windows") == 1
    assert checker.read_baseline(tmp_path, "windows") == ({}, {"a"})


def test_the_workspace_packages_a_run_compiles_are_counted_too(tmp_path, monkeypatch, capsys):
    # Review 2026-11-09 R1109-TST-302: clippy reported the workspace crates the named ones
    # depend on, and the check dropped them, so code only they compiled was counted by no run.
    for name in ("app", "lib", "other"):
        package(tmp_path, name)
    (tmp_path / "scripts").mkdir()
    checker.write_baseline(tmp_path, "linux-occt", {}, {"app"})
    found = [message("expect_used", "crates/lib/src/unix.rs", 7),
             message("expect_used", "crates/other/src/lib.rs", 1)]
    monkeypatch.setattr(checker.subprocess, "run", lambda *_, **__: type(
        "Run", (), {"returncode": 0, "stdout": "\n".join(found)})())
    monkeypatch.setattr(checker, "compiled_packages", lambda *_: {"app", "lib"})
    assert checker.check(["app"], root=tmp_path, system="linux-occt") == 1
    assert capsys.readouterr().out.splitlines()[1:] == ["crates/lib/src/unix.rs expect_used: 1 (allowed 0)"]
    # Recorded once as found, with the package; other, not compiled, stays out.
    assert checker.check(["app"], update=True, root=tmp_path, system="linux-occt") == 0
    assert checker.read_baseline(tmp_path, "linux-occt") == (
        {"crates/lib/src/unix.rs expect_used": 1}, {"app", "lib"})


def test_the_compiled_packages_follow_the_workspace_dependencies_of_the_target(monkeypatch):
    metadata = json.dumps({
        "workspace_members": ["app", "lib", "phone", "mac"],
        "packages": [{"id": each, "name": each} for each in ("app", "lib", "phone", "mac", "serde")],
        "resolve": {"nodes": [
            {"id": "app", "deps": [{"pkg": "lib", "dep_kinds": [{"kind": None}]},
                                   {"pkg": "serde", "dep_kinds": [{"kind": None}]},
                                   {"pkg": "mac", "dep_kinds": [{"kind": "dev"}]}]},
            {"id": "phone", "deps": [{"pkg": "lib", "dep_kinds": [{"kind": None}]}]},
        ]},
    })
    commands = []

    def run(command, **_):
        commands.append(command)
        return type("Run", (), {"returncode": 0, "stdout": metadata})()

    monkeypatch.setattr(checker.subprocess, "run", run)
    assert checker.compiled_packages(["app"], ROOT, "aarch64-linux-android") == {"app", "lib"}
    assert commands[0][-2:] == ["--filter-platform", "aarch64-linux-android"]


def test_a_target_is_compiled_for_and_compared_with_its_own_baseline():
    assert checker.clippy_command(["a"], "wasm32-unknown-unknown")[3:7] == [
        "-p", "a", "--target", "wasm32-unknown-unknown"]
    assert checker.target_and_baseline(["--target", "aarch64-linux-android", "-p", "a"]) == (
        "aarch64-linux-android", "android")
    assert checker.target_and_baseline(["--target", "wasm32-unknown-unknown"])[1] == "browser"
    assert checker.target_and_baseline(["--target", "aarch64-apple-darwin"])[1] == "macos"
    assert checker.target_and_baseline(["--baseline", "linux-occt", "-p", "a"]) == (None, "linux-occt")
    assert checker.target_and_baseline(["-p", "a"]) == (None, checker.SYSTEM)


def test_the_ci_jobs_baselines_are_per_system():
    assert checker.SYSTEMS == {"win32": "windows", "linux": "linux", "darwin": "macos"}
    for system, needed in (("linux", "ketchup-mcp"), ("windows", "ketchup-mcp")):
        _, recorded = checker.read_baseline(ROOT, system)
        assert needed in recorded
    assert not (ROOT / "scripts" / "clippy_panics_baseline.txt").exists()


PROBE = """\
macro_rules! take {
    ($x:expr) => {
        $x.unwrap()
    };
}

macro_rules! boom {
    () => {
        panic!("x")
    };
}

pub fn plain(x: Option<u8>) -> u8 {
    x.unwrap()
}

pub fn in_macro(x: Option<u8>) -> u8 {
    take!(x)
}

pub fn boom_in_macro() {
    boom!();
}

pub fn by_path(x: Vec<Option<u8>>) -> Vec<u8> {
    x.into_iter().map(Option::unwrap).collect()
}

#[allow(clippy::unwrap_used)]
pub fn allowed(x: Option<u8>) -> u8 {
    x.unwrap()
}
"""


@pytest.mark.skipif(shutil.which("cargo") is None, reason="needs cargo clippy")
def test_real_clippy_and_the_text_check_count_every_form_once_between_them(tmp_path):
    # Review 2026-11-09 R1109-TST-300: the forms clippy does not report were checked
    # against made-up JSON, not clippy's output; a panic in a `macro_rules!` and
    # `.map(Option::unwrap)` passed both checks. TST-301: `#[allow(clippy::unwrap_used)]`
    # hid an unwrap from the count.
    (tmp_path / "src").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2024"\n\n[workspace]\n',
        encoding="utf-8")
    (tmp_path / "src" / "lib.rs").write_text(PROBE, encoding="utf-8")
    command = [part for part in checker.clippy_command(["probe"]) if part != "--locked"]
    run = subprocess.run(command, cwd=tmp_path, stdout=subprocess.PIPE, text=True,
                         encoding="utf-8", env={**os.environ, "CARGO_TARGET_DIR": str(tmp_path / "target")})
    assert run.returncode == 0
    # clippy names the plain unwrap and the allowed one, nothing in a macro or by path.
    assert checker.counts_of(run.stdout.splitlines(), tmp_path) == {"src/lib.rs unwrap_used": 2}
    text_counts = {form: count for form, count in hygiene.file_counts(PROBE).items() if count}
    assert text_counts == {"macro_panic": 2, "unwrap_path": 1, "clippy_allow": 1}


def test_without_packages_named_the_systems_recorded_ones_are_checked(tmp_path):
    # Review 2026-11-09 R1109-TST-306: run without -p, as the pre-push hook runs it, the
    # check ended 0 having run nothing.
    package(tmp_path, "a")
    (tmp_path / "scripts").mkdir()
    checker.write_baseline(tmp_path, "linux", {}, {"b", "a"})
    assert checker.packages_to_check([], tmp_path, "linux") == ["a", "b"]
    assert checker.packages_to_check(["-p", "c"], tmp_path, "linux") == ["c"]
    assert checker.packages_to_check([], tmp_path, "macos") == []
    # A dependency the run compiled is recorded but not named: named, its binaries, which a
    # dependency's run never compiles, would be counted against it.
    checker.write_baseline(tmp_path, "windows", {}, {"a"}, {"a", "lib"})
    assert checker.packages_to_check([], tmp_path, "windows") == ["a"]
    assert checker.read_baseline(tmp_path, "windows") == ({}, {"a", "lib"})


def test_the_packages_come_from_the_command_line():
    assert checker.packages_of(["--update", "-p", "a", "-p", "b"]) == ["a", "b"]
    assert checker.clippy_command(["a"])[:8] == [
        "cargo", "clippy", "--locked", "-p", "a", "--lib", "--bins", "--message-format=json"]
