use ketchup_program::{Issue, run};
use std::collections::BTreeMap;

fn failures(source: &str) -> Vec<Issue> {
    run("test.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
        .issues
        .into_iter()
        .filter(|issue| issue.kind == "expectation_failed")
        .collect()
}

fn messages(source: &str) -> Vec<String> {
    failures(source)
        .into_iter()
        .map(|issue| issue.message)
        .collect()
}

// A tilted back stood on a turned seat by relation, as in P2.
const CHAIR: &str = "seat = box(\"seat\", (400, 380, 30), at = (100, 50, 420))\n\
rotate(seat, axis = (0, 0, 1), angle = 30, pivot = (300, 240, 0))\n\
back = box(\"back\", (360, 20, 400))\n\
rotate(back, axis = (1, 0, 0), angle = -12)\n\
rotate(back, axis = (0, 0, 1), angle = 30)\n\
on(back, seat, align = \"y+\", center = \"x\")\n";

#[test]
fn intent_that_holds_on_rotated_parts_reports_nothing() {
    let source = format!(
        "{CHAIR}\
         expect_contact(back, seat)\n\
         expect_flush(back, seat, \"y+\")\n\
         expect_symmetric(back, back, seat, axis = \"x\")\n\
         expect_gap(seat, back, 0)\n"
    );
    assert_eq!(messages(&source), Vec::<String>::new());
}

#[test]
fn intent_is_measured_after_the_whole_program() {
    // Stated before the move that makes it true.
    let source = "a = box(\"a\", (100, 100, 18))\n\
         b = box(\"b\", (100, 100, 18), at = (300, 0, 0))\n\
         expect_contact(b, a, face = \"x+\")\n\
         on(b, a, face = \"x+\")\n";
    assert_eq!(messages(source), Vec::<String>::new());
}

#[test]
fn broken_intent_is_an_error_with_measured_and_required_numbers() {
    let source = format!(
        "{CHAIR}\
         move(back, by = (0, 0, 5))\n\
         expect_contact(back, seat)\n"
    );
    let failed = failures(&source);
    assert_eq!(failed.len(), 1, "{failed:#?}");
    let issue = &failed[0];
    assert_eq!(issue.severity, ketchup_program::Severity::Error);
    assert_eq!(issue.parts, ["back", "seat"]);
    assert!(
        issue
            .message
            .starts_with("back touches seat: measured 5 mm, required == 0 mm"),
        "{}",
        issue.message
    );
    assert!(!issue.hint.is_empty());
}

#[test]
fn flush_on_a_rotated_target_reports_how_far_the_part_sticks_out() {
    let source = format!(
        "{CHAIR}\
         move(back, by = vec_scale(face_normal(seat, \"y+\"), 7))\n\
         expect_flush(back, seat, \"y+\")\n"
    );
    assert_eq!(
        messages(&source),
        ["back flush with seat y+: measured 7 mm, required == 0 mm (tolerance 0.1 mm)"]
    );
}

#[test]
fn gap_contact_face_symmetry_and_inside_each_fail_with_their_numbers() {
    let base = "top = box(\"top\", (1000, 600, 25), at = (0, 0, 700))\n\
         left = box(\"left\", (60, 60, 700), at = (40, 40, 0))\n\
         right = box(\"right\", (60, 60, 700), at = (890, 40, 0))\n\
         door = box(\"door\", (400, 18, 500), at = (103, 40, 50))\n\
         drawer = box(\"drawer\", (300, 500, 80), at = (350, 50, 610))\n";
    let holds = format!(
        "{base}expect_contact(left, top, face = \"z-\")\n\
         expect_gap(door, left, 3)\n\
         expect_inside(drawer, top)\n"
    );
    // The drawer is not inside the top: it only sits under it.
    assert_eq!(
        messages(&holds),
        ["drawer inside top at z-: measured 90 mm, required <= 0 mm (tolerance 0.1 mm)"]
    );
    let broken = format!(
        "{base}expect_gap(door, left, 2)\n\
         expect_symmetric(left, right, top)\n\
         expect_contact(door, top, face = \"z-\")\n"
    );
    assert_eq!(
        messages(&broken),
        [
            "door is 2 mm from left: measured 3 mm, required == 2 mm (tolerance 0.1 mm)",
            "left and right symmetric about top: measured -10 mm, required == 0 mm (tolerance 0.1 mm)",
            "door touches top: measured 150 mm, required == 0 mm (tolerance 0.1 mm)",
            "door touches top on z-: measured 0 mm², required > 0 mm² (tolerance 0 mm²)",
        ]
    );
}

#[test]
fn raw_expect_compares_any_sum_of_measures() {
    let source = "seat = box(\"seat\", (400, 400, 30), at = (0, 0, 420))\n\
         expect(\"seat height\", terms = [(1, (\"reach\", seat, (0, 0, 1)))], value = 450)\n\
         expect(\"seat thickness\", terms = [(1, (\"reach\", seat, (seat, \"z+\"))), (1, (\"reach\", seat, (seat, \"z-\")))], op = \">=\", value = 40)\n";
    assert_eq!(
        messages(source),
        ["seat thickness: measured 30 mm, required >= 40 mm (tolerance 0.1 mm)"]
    );
    let error = run(
        "bad.star",
        "a = box(\"a\", (1, 1, 1))\nexpect(\"x\", terms = [(1, (\"volume\", a))])\n",
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(
        error.message.contains("measure must be"),
        "{}",
        error.message
    );
}
