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

/// R1126og-PG-1105: the text of a value nested past the bound ended in `...`, so two lists 250
/// deep ending in 1 and in 2 had the same `str()` and a dict keyed by it kept one of them. Text
/// a program gets as a value is refused there with its line; only messages and printed lines
/// still end in `...`.
#[test]
fn text_of_a_deeply_nested_value_is_refused_with_its_line() {
    for depth in [250, 300_000] {
        for (step, used) in [
            ("[x]", "s = str((x,))"),
            ("[x]", "s = repr([x])"),
            ("struct(a = x)", "s = str(struct(a = x))"),
            ("(x,)", "s = str(x)"),
            ("{\"k\": x}", "s = repr(x)"),
            ("[x]", "s = \"%s\" % (x,)"),
            ("[x]", "s = \"a %r b %d\" % (x, 1)"),
            ("[x]", "s = \"{}\".format(x)"),
            ("[x]", "s = \"a {!r} b {}\".format(x, 1)"),
            ("[x]", "s = f\"a {x} b\""),
        ] {
            let source = program(step, depth, used);
            let error = run("deep.star", &source, &BTreeMap::new())
                .err()
                .unwrap_or_else(|| panic!("{depth} {step} {used}: no error"));
            assert!(
                error.message.contains("more than 200 deep"),
                "{depth} {used}: {}",
                error.message
            );
            assert_eq!(
                error.location.map(|at| at.at.line),
                Some(4),
                "{depth} {used}"
            );
        }
    }
    // A value within the bound is written whole, and printing a deeper one shows `...`.
    let shallow = program(
        "[x]",
        150,
        "if \"...\" in str(x) or str(x) == str([x]):\n    fail(\"cut\")",
    );
    run("shallow.star", &shallow, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let printed = program("[x]", 300_000, "print(x)");
    run("print.star", &printed, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
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
