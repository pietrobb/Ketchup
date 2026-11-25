//! Review 2026-11-25 og PG-1002 and lo R1125lo-2, R1125lo-3: values a program builds while it
//! runs, nested far deeper than its source could, still overflowed the stack and killed the
//! process with no answer through `str()`, `repr()`, the message of `fail()` and of a missing
//! key, and hashing a key; `enumerate`, `sorted` and `reversed` over a huge range panicked. Each now writes `...` past
//! the nesting bound or is refused with its line.
use ketchup_program::run;
use std::collections::BTreeMap;

/// `x` built `depth` times by `step`, then `use` (its line is 4), then a part.
fn program(step: &str, depth: usize, used: &str) -> String {
    format!("x = 1\nfor i in range({depth}):\n    x = {step}\n{used}\nbox(\"a\", (1, 2, 3))\n")
}

#[test]
fn str_and_repr_of_a_deeply_nested_value_end_in_an_ellipsis() {
    for (step, used) in [
        ("[x]", "s = str((x,))"),
        ("[x]", "s = repr([x])"),
        ("struct(a = x)", "s = str(struct(a = x))"),
        ("(x,)", "s = str(x)"),
        ("{\"k\": x}", "s = repr(x)"),
    ] {
        let used = format!("{used}\nif \"...\" not in s or len(s) > 10000:\n    fail(len(s))");
        let source = program(step, 300_000, &used);
        run("deep.star", &source, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{step} {used}: {error}"));
    }
    // A value within the bound is written whole.
    let shallow = program("[x]", 150, "if \"...\" in str(x):\n    fail(\"cut\")");
    run("shallow.star", &shallow, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn fail_and_a_missing_key_with_a_deeply_nested_value_name_their_line() {
    for (step, used) in [("[x]", "fail(x)"), ("(x,)", "v = {}[x]")] {
        let source = program(step, 1_000_000, used);
        let error = run("deep.star", &source, &BTreeMap::new()).unwrap_err();
        assert_eq!(
            error.location.map(|at| at.at.line),
            Some(4),
            "{used}: {}",
            error.message
        );
    }
}

#[test]
fn enumerate_sorted_and_reversed_over_a_huge_range_are_refused_by_the_memory_bound() {
    for call in ["enumerate", "sorted", "reversed"] {
        let source = format!("n = len({call}(range(1000000000)))\nbox(\"a\", (1, 2, 3))\n");
        let error = run("huge.star", &source, &BTreeMap::new()).unwrap_err();
        assert!(
            error.message.contains("program memory limit exceeded"),
            "{call}: {}",
            error.message
        );
        assert_eq!(error.location.map(|at| at.at.line), Some(1), "{call}");
    }
}
