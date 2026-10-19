use super::*;
use std::collections::BTreeMap;

fn program(source: &str) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "edit.star".to_owned(),
        source: source.to_owned(),
        overrides: BTreeMap::new(),
        variants: BTreeMap::default(),
    }
}

fn change(source: &str, name: &str, change: PartChange) -> RuleProgramSource {
    rewrite_rule_program_parts(&program(source), &[(name.to_owned(), change)])
        .expect("changed")
        .expect("a program part")
}

fn refusal(source: &str, name: &str, change: PartChange) -> String {
    rewrite_rule_program_parts(&program(source), &[(name.to_owned(), change)])
        .expect_err("refused")
        .to_string()
}

fn model(source: &RuleProgramSource) -> ProgramModel {
    evaluate(source).expect("evaluates").model
}

const TWO: &str = "box(\"a\", (10, 20, 30), at=(0, 0, 0))\n\
                   box(\"b\", (5, 5, 5), at=(50, 0, 0))\n\
                   move(\"a\", by=(1, 0, 0))\n";

#[test]
fn delete_removes_the_creating_line_and_the_lines_that_only_change_the_part() {
    let deleted = change(TWO, "a", PartChange::Delete);
    assert_eq!(deleted.source, "box(\"b\", (5, 5, 5), at=(50, 0, 0))\n");
    let after = model(&deleted);
    assert_eq!(after.parts.len(), 1);
    assert_eq!(after.parts[0].at_mm, [50.0, 0.0, 0.0]);
}

#[test]
fn delete_of_a_part_another_line_uses_is_refused_with_that_line() {
    let used = "a = box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\nprint(a)\n";
    let message = refusal(used, "a", PartChange::Delete);
    assert!(
        message.contains("\"a\"") && message.contains("line 3"),
        "{message}"
    );
    let named = "box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\nprint(\"a\")\n";
    let message = refusal(named, "a", PartChange::Delete);
    assert!(message.contains("line 3"), "{message}");
}

#[test]
fn a_part_of_a_loop_or_of_a_function_is_refused_with_the_line_to_change() {
    let looped = "for i in range(3):\n    box(\"s%d\" % i, (10, 10, 10), at=(0, 0, 20 * i))\n";
    for change in [
        PartChange::Delete,
        PartChange::Color(Some([1, 2, 3])),
        PartChange::Rename("x".to_owned()),
    ] {
        let message = refusal(looped, "s1", change);
        assert!(message.contains("loop on line 1"), "{message}");
    }
    let made =
        "def shelf(name, z):\n    box(name, (100, 18, 400), at=(0, 0, z))\nshelf(\"top\", 500)\n";
    let message = refusal(made, "top", PartChange::Delete);
    assert!(
        message.contains("shelf()") && message.contains("line 3"),
        "{message}"
    );
}

#[test]
fn colour_is_added_changed_and_removed_in_the_creating_call() {
    let coloured = change(TWO, "b", PartChange::Color(Some([200, 10, 0])));
    assert_eq!(
        coloured.source,
        TWO.replace("(50, 0, 0))", "(50, 0, 0), color=(200, 10, 0))")
    );
    assert_eq!(
        model(&coloured).part("b").unwrap().color,
        Some([200, 10, 0])
    );
    let changed = change(&coloured.source, "b", PartChange::Color(Some([0, 0, 255])));
    assert!(
        changed.source.contains("at=(50, 0, 0), color=(0, 0, 255))"),
        "{}",
        changed.source
    );
    let cleared = change(&changed.source, "b", PartChange::Color(None));
    assert_eq!(cleared.source, TWO);
}

#[test]
fn rename_changes_the_name_in_the_source_and_keeps_the_part() {
    let renamed = change(TWO, "a", PartChange::Rename("side".to_owned()));
    assert_eq!(
        renamed.source,
        "box(\"side\", (10, 20, 30), at=(0, 0, 0))\n\
         box(\"b\", (5, 5, 5), at=(50, 0, 0))\n\
         move(\"side\", by=(1, 0, 0))\n\
         continue_part(\"side\", was=\"a\")\n"
    );
    let after = model(&renamed);
    assert_eq!(after.part("side").unwrap().at_mm, [1.0, 0.0, 0.0]);
    assert_eq!(after.continuations["side"], "a");
    // Renamed again, the claim names the previous name, not the first one.
    let again = change(
        &renamed.source,
        "side",
        PartChange::Rename("end".to_owned()),
    );
    assert!(
        again
            .source
            .ends_with("continue_part(\"end\", was=\"side\")\n"),
        "{}",
        again.source
    );
    let message = refusal(TWO, "a", PartChange::Rename("b".to_owned()));
    assert!(
        message.contains("already makes a part named \"b\""),
        "{message}"
    );
}

#[test]
fn mirror_adds_a_mirrored_copy_across_the_plane() {
    let source = "box(\"a\", (10, 20, 30), at=(100, 0, 0))\n";
    let mirrored = change(
        source,
        "a",
        PartChange::Mirror {
            copy: "a mirrored".to_owned(),
            point: [0.0, 0.0, 0.0],
            normal: [2.0, 0.0, 0.0],
        },
    );
    assert!(
        mirrored
            .source
            .ends_with("mirrored(\"a\", \"a mirrored\", point=(0, 0, 0), normal=(1, 0, 0))\n"),
        "{}",
        mirrored.source
    );
    // x 100..110 mirrored across x = 0 is x -110..-100; y and z stay.
    let after = model(&mirrored);
    let (min, max) = after.part("a mirrored").unwrap().world_bounds();
    assert!(
        (0..3).all(|i| (min[i] - [-110.0, 0.0, 0.0][i]).abs() < 1.0e-9),
        "{min:?}"
    );
    assert!(
        (0..3).all(|i| (max[i] - [-100.0, 20.0, 30.0][i]).abs() < 1.0e-9),
        "{max:?}"
    );
    assert_eq!(after.part("a").unwrap().at_mm, [100.0, 0.0, 0.0]);
}

#[test]
fn a_value_one_param_drives_is_overridden_to_the_typed_size() {
    let source = "W = param(\"width\", 100)\nbox(\"p\", (W * 2, 20, 10))\n";
    let made = model(&program(source));
    let feature = made.part("p").unwrap().features.iter().find(|feature| {
        (feature.parameters.iter()).any(|parameter| parameter.path == "bounds.width")
    });
    let path = "bounds.width".to_owned();
    let feature = feature.unwrap().name.clone();
    let value = 140.0;
    let resized = change(
        source,
        "p",
        PartChange::Parameter {
            feature,
            path,
            value,
        },
    );
    // W * 2 = 140, so W = 70; the source stays as written.
    assert_eq!(
        (resized.source.as_str(), resized.overrides["width"]),
        (source, 70.0)
    );
    assert_eq!(
        model(&resized).part("p").unwrap().size_mm,
        [140.0, 20.0, 10.0]
    );
}

#[test]
fn a_part_of_a_loop_is_moved_after_the_loop_alone() {
    let source = "for i in range(3):\n    box(\"s%d\" % i, (10, 10, 10), at=(0, 0, 20 * i))\n";
    let by = Transform::from_translation(5.0, 0.0, 0.0).unwrap();
    let moved = change(source, "s1", PartChange::Transform(by));
    assert_eq!(
        moved.source,
        format!("{source}move(\"s1\", by=(5, 0, 0))\n")
    );
    // s1 goes from (0, 0, 20) to (5, 0, 20); s2 stays at (0, 0, 40).
    let after = model(&moved);
    assert_eq!(after.part("s1").unwrap().at_mm, [5.0, 0.0, 20.0]);
    assert_eq!(after.part("s2").unwrap().at_mm, [0.0, 0.0, 40.0]);
}

#[test]
fn nothing_is_written_for_what_the_program_does_not_make_as_a_part() {
    let source = program(TWO);
    assert!(
        rewrite_rule_program_parts(&source, &[("copy".to_owned(), PartChange::Delete)])
            .unwrap()
            .is_none()
    );
}

#[test]
fn deleting_every_part_writes_no_empty_program() {
    let source = program(TWO);
    let every = ["a", "b"].map(|name| (name.to_owned(), PartChange::Delete));
    assert!(
        rewrite_rule_program_parts(&source, &every)
            .unwrap()
            .is_none()
    );
    // A program with a line left keeps owning the parts it still makes.
    let kept = program(&format!("size = param(\"size\", 5)\n{TWO}"));
    let rewritten = rewrite_rule_program_parts(&kept, &every).unwrap().unwrap();
    assert_eq!(rewritten.source, "size = param(\"size\", 5)\n");
    assert!(model(&rewritten).parts.is_empty());
}

const PADDED: &str =
    "box(\"a\", (10, 20, 18), at=(0, 0, 0))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\n";

/// Part a's length typed in the parameter editor.
fn set_length(
    source: &RuleProgramSource,
    value: f64,
) -> Result<Option<RuleProgramSource>, RuleProgramApplyError> {
    let change = PartChange::Parameter {
        feature: "a feature".to_owned(),
        path: "extent.distance".to_owned(),
        value,
    };
    rewrite_rule_program_parts(source, &[("a".to_owned(), change)])
}

fn length(source: &RuleProgramSource) -> f64 {
    Reading::Length
        .of(&model(source), "a")
        .expect("an extrusion")
}

#[test]
fn a_length_typed_again_replaces_the_first_value_instead_of_adding_to_it() {
    let first = set_length(&program(PADDED), 10.0).unwrap().unwrap();
    assert_eq!(length(&first), 10.0);
    let second = set_length(&first, 12.0).unwrap().unwrap();
    // 18 -> 10 -> 12 is 12 long, not 18 - 8 - 6 = 4: the one line is written again.
    assert_eq!(
        second.source,
        format!("{PADDED}push_pull(\"a\", face=\"z+\", distance=-6, name=\"Length\")\n")
    );
    assert_eq!(length(&second), 12.0);
    assert_eq!(model(&second).part("b"), model(&program(PADDED)).part("b"));
    // Typed back to the length the program writes, the line goes.
    assert_eq!(set_length(&second, 18.0).unwrap().unwrap().source, PADDED);
}

#[test]
fn a_typed_length_keeps_the_push_pulls_of_other_lines() {
    let pushed = program(&format!(
        "{PADDED}push_pull(\"a\", face=\"z-\", distance=2, name=\"Lower\")\n"
    ));
    let typed = set_length(&set_length(&pushed, 30.0).unwrap().unwrap(), 25.0)
        .unwrap()
        .unwrap();
    // The extrusion is 25 long and the 2 below it stays: 27 in all.
    assert_eq!(length(&typed), 27.0);
    assert_eq!(typed.source.matches("name=\"Length\"").count(), 1);
}

#[test]
fn a_length_of_zero_or_less_is_refused_and_the_program_stays() {
    for value in [0.0, -5.0, f64::NAN] {
        let refusal = set_length(&program(PADDED), value).expect_err("refused");
        assert!(
            refusal.to_string().contains("type a length above 0"),
            "{refusal}"
        );
    }
    // A pull of 15 below leaves a length of 10 at -5.
    let pulled = program(&format!(
        "{PADDED}push_pull(\"a\", face=\"z-\", distance=-15, name=\"Lower\")\n"
    ));
    let refusal = set_length(&pulled, 10.0).expect_err("refused");
    assert!(refusal.to_string().contains("-5 mm long"), "{refusal}");
}
