import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_large_house_tests", ROOT / "scripts" / "check_large_house_tests.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

HOUSE = 'const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");\n'
IGNORE = '#[ignore = "large-house: runs in full suite"]\n'


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_a_house_test_run_in_every_step_is_found_however_it_reaches_the_house(tmp_path):
    write(tmp_path, "crates/a/tests/house.rs", (
        HOUSE
        + "fn model() -> Model {\n    evaluate(HOUSE)\n}\n"
        + "#[test]\nfn through_the_constant() {\n    evaluate(HOUSE);\n}\n"
        + "#[test]\nfn through_a_helper() {\n    let _ = model();\n}\n"
        + '#[test]\nfn directly() {\n    run(include_str!("x/tiny-house.star"));\n}\n'
        + "#[test]\n" + IGNORE + "fn ignored_but_not_named() {\n    model();\n}\n"
        + "#[test]\nfn large_house_named_but_not_ignored() {\n    model();\n}\n"
    ))
    assert checker.violations(tmp_path) == [
        "crates/a/tests/house.rs:6 through_the_constant",
        "crates/a/tests/house.rs:10 through_a_helper",
        "crates/a/tests/house.rs:14 directly",
        "crates/a/tests/house.rs:19 ignored_but_not_named",
        "crates/a/tests/house.rs:23 large_house_named_but_not_ignored",
    ]


def test_a_public_helper_is_followed_into_the_other_files_of_its_crate_only(tmp_path):
    write(tmp_path, "crates/a/src/fixtures.rs", HOUSE + "pub(crate) fn house() -> App {\n    load(HOUSE)\n}\n")
    write(tmp_path, "crates/a/src/uses.rs", "#[test]\nfn pulls_the_house() {\n    house();\n}\n")
    # Its own `house()` shadows the shared one; another crate does not see it.
    write(tmp_path, "crates/a/src/own.rs", (
        "fn house() -> App {\n    small()\n}\n#[test]\nfn own_house() {\n    house();\n}\n"
    ))
    write(tmp_path, "crates/b/src/other.rs", "#[test]\nfn another_crate() {\n    house();\n}\n")
    assert checker.violations(tmp_path) == ["crates/a/src/uses.rs:2 pulls_the_house"]


def test_ignored_large_house_perf_and_release_only_tests_and_mentions_pass(tmp_path):
    write(tmp_path, "crates/a/tests/house.rs", (
        HOUSE
        + "#[test]\n" + IGNORE + "fn large_house_checked() {\n    evaluate(HOUSE);\n}\n"
        + '#[test]\n#[ignore = "perf: release timing budget, runs nightly"]\nfn perf_frames() {\n    evaluate(HOUSE);\n}\n'
        + '#[test]\n#[ignore = "release-only: exact geometry of the whole house"]\nfn the_panel() {\n'
        + "    evaluate(HOUSE);\n}\n"
        # Naming the file, in a comment or a string, is not evaluating it.
        + '#[test]\nfn skips_the_house() {\n    // HOUSE is too slow here; include_str!("tiny-house.star")\n'
        + '    assert!(!LARGE.contains(&"tiny-house.star"));\n}\n'
        + '#[test]\nfn braces_in_strings() {\n    let _ = format!("{{ {}", 1);\n    let _ = \'{\';\n}\n'
        + "#[test]\nfn after_the_braces() {\n    small();\n}\n"
    ))
    assert checker.violations(tmp_path) == []


def test_the_repository_runs_no_large_house_test_in_every_step():
    assert checker.check(ROOT) == 0
