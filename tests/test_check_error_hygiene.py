import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_error_hygiene", ROOT / "scripts" / "check_error_hygiene.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_counts_discarded_causes_and_bare_text_errors_in_production_source(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'let a = x.map_err(|_| "bad")?;\n'
        "let b = y.map_err(|_error| Error::Io)?;\n"
        "let c = z.map_err(move |_| Error::Io)?;\n"
        'return Err("no such part".to_owned());\n'
        'return Err( "a \\"quoted\\" name" .into() );\n'
        'Err("x".to_string())\n'
        "let kept = w.map_err(|error| Error::Io(error))?;\n"
        'let formatted = Err(format!("{x}"));\n',
    )
    write(tmp_path, "crates/a/src/scene_tests.rs", 'x.map_err(|_| "bad")?;\n')
    write(tmp_path, "crates/a/tests/it.rs", 'x.map_err(|_| "bad")?;\n')
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs discard": 3,
        "crates/a/src/lib.rs string": 3,
    }


def test_counts_unreachable_cases_in_production_source_only(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'Kind::Circle => unreachable!("handled above"),\n'
        "_ => unreachable! ()\n"
        "// not_unreachable!(x) and the word unreachable alone do not count\n"
        '#[cfg(test)]\nmod tests {\n    fn g() { unreachable!() }\n}\n',
    )
    write(tmp_path, "crates/a/tests/it.rs", "unreachable!();\n")
    assert checker.current_counts(tmp_path) == {"crates/a/src/lib.rs unreachable": 2}


def test_only_named_content_free_error_types_may_be_dropped(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "let a = u32::try_from(n).map_err(|_: std::num::TryFromIntError| Error::Range)?;\n"
        "let b = lock.map_err(|_: PoisonError<Guard<'_>>| Error::Poisoned)?;\n"
        "let c = s.parse::<u32>().map_err(|_: std::num::ParseIntError| Error::Number)?;\n"
        "let d = write!(out, \"x\").map_err(|_: std::fmt::Error| Error::Limit)?;\n"
        "let e = file.read(b).map_err(|_: std::io::Error| Error::Io)?;\n"
        "let f = key.verify(m, s).map_err(|_: ed25519_dalek::SignatureError| Error::Forged)?;\n"
        "let g = other.check().map_err(|_: SignatureError| Error::Forged)?;\n"
        "let h = path.strip_prefix(root).map_err(|_: std::path::StripPrefixError| Error::Outside)?;\n",
    )
    # Only the io::Error (e) and the unqualified SignatureError (g) drop content.
    assert checker.current_counts(tmp_path) == {"crates/a/src/lib.rs discard": 2}


def test_inline_test_modules_are_not_production_code(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'fn f() -> Result<(), String> { Err("x".to_owned()) }\n'
        '#[cfg(test)]\nmod tests {\n    fn g() { x.map_err(|_| "bad"); }\n}\n',
    )
    assert checker.current_counts(tmp_path) == {"crates/a/src/lib.rs string": 1}


def test_ratchet_refuses_growth_and_locks_in_shrinking(tmp_path, capsys):
    (tmp_path / "scripts").mkdir()
    write(tmp_path, "crates/a/src/lib.rs", 'a.map_err(|_| 1);\nb.map_err(|_| 2);\n')
    assert checker.check(tmp_path, update=True) == 0
    assert checker.check(tmp_path) == 0

    write(tmp_path, "crates/b/src/lib.rs", 'return Err("bad".into());\n')
    assert checker.check(tmp_path) == 1
    assert "crates/b/src/lib.rs string: 1 (allowed 0)" in capsys.readouterr().out
    assert checker.check(tmp_path, update=True) == 1

    (tmp_path / "crates/b/src/lib.rs").unlink()
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.read_baseline(tmp_path) == {"crates/a/src/lib.rs discard": 1}
