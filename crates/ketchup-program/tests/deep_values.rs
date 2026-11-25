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

/// `source` is refused by the memory bound at line `line`, not a panic or the whole machine's
/// memory.
fn refused_at(source: &str, line: usize) {
    let error = run("huge.star", source, &BTreeMap::new()).unwrap_err();
    assert!(
        error.message.contains("program memory limit exceeded"),
        "{source}: {}",
        error.message
    );
    assert_eq!(error.location.map(|at| at.at.line), Some(line), "{source}");
}

/// Review 2026-11-26 og R1126og-PG-1103: a range longer than `i32::MAX` reported no length, so
/// the check before collecting it passed, and `enumerate` panicked (exit 101) while `reversed`,
/// `sorted` and `tuple` took memory until the process was killed.
#[test]
fn collecting_a_range_longer_than_i32_is_refused_by_the_memory_bound() {
    for call in [
        "enumerate(r)",
        "list(r)",
        "tuple(r)",
        "sorted(r)",
        "sorted(r, key = abs)",
        "reversed(r)",
    ] {
        refused_at(
            &format!(
                "r = range(2147483647, -2147483648, -1)\nn = len({call})\nbox(\"a\", (1, 2, 3))\n"
            ),
            2,
        );
    }
}

/// R1126og-PG-1103: a list grown by `extend` or `+=`, and the tuples `zip` builds, were not
/// checked against the memory bound at all.
#[test]
fn extending_a_list_and_zip_are_refused_by_the_memory_bound() {
    for used in [
        "l = []\nl.extend(range(1000000000))",
        "l = []\nl += range(1000000000)",
        "l = []\nl = zip(range(1000000000), range(1000000000))",
        "l = []\nl = zip(range(300000000))",
    ] {
        refused_at(&format!("{used}\nbox(\"a\", (1, 2, 3))\n"), 2);
    }
    // Within the bound they build what they did.
    let small = r#"
r = range(5, -1, -2)
checks = [
    (enumerate(r, 1), [(1, 5), (2, 3), (3, 1)]),
    (list(r), [5, 3, 1]),
    (tuple(r), (5, 3, 1)),
    (sorted(r), [1, 3, 5]),
    (sorted(r, key = lambda x: -x, reverse = True), [1, 3, 5]),
    (reversed(r), [1, 3, 5]),
    (zip(r, "ab".elems(), [7, 8, 9]), [(5, "a", 7), (3, "b", 8)]),
    (zip(), []),
]
l = [0]
l.extend(r)
l += r
checks.append((l, [0, 5, 3, 1, 5, 3, 1]))
for got, want in checks:
    if got != want:
        fail(got, want)
box("a", (1, 2, 3))
"#;
    run("small.star", small, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
}
