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

/// Review 2026-10-19 WB-3: renaming part "oak" also renamed the material
/// "oak" of another part, the parameter "oak" and a text reading "oak".
#[test]
fn rename_changes_only_the_literals_that_name_the_part() {
    let source = "w = param(\"oak\", 40)\n\
                  box(\"oak\", (w, 18, 30), material=\"oak\")\n\
                  box(\"b\", (10, 18, 30), at=(0, 100, 0), material=\"oak\")\n\
                  move(\"oak\", by=(0, 0, 10))\n\
                  tag(\"oak\", \"oak\")\n\
                  for name in [\"oak\"]:\n    move(name, by=(1, 0, 0))\n\
                  label = \"oak\"\n\
                  if label == \"oak\":\n    box(\"c\", (10, 10, 10), at=(0, 300, 0))\n";
    let renamed = change(source, "oak", PartChange::Rename("lid".to_owned()));
    assert_eq!(
        renamed.source,
        "w = param(\"oak\", 40)\n\
         box(\"lid\", (w, 18, 30), material=\"oak\")\n\
         box(\"b\", (10, 18, 30), at=(0, 100, 0), material=\"oak\")\n\
         move(\"lid\", by=(0, 0, 10))\n\
         tag(\"lid\", \"oak\")\n\
         for name in [\"lid\"]:\n    move(name, by=(1, 0, 0))\n\
         label = \"oak\"\n\
         if label == \"oak\":\n    box(\"c\", (10, 10, 10), at=(0, 300, 0))\n\
         continue_part(\"lid\", was=\"oak\")\n"
    );
    let before = model(&program(source));
    let after = model(&renamed);
    let mut expected = before.clone();
    expected
        .parts
        .iter_mut()
        .filter(|part| part.name == "oak")
        .for_each(|part| {
            part.name = "lid".to_owned();
            for feature in &mut part.features {
                feature.name = feature.name.replacen("oak ", "lid ", 1);
            }
        });
    expected
        .continuations
        .insert("lid".to_owned(), "oak".to_owned());
    assert_eq!(after, expected);
    assert_eq!(after.params[0].name, "oak");
    assert_eq!(after.part("b").unwrap().material.as_deref(), Some("oak"));
}

#[test]
fn rename_is_refused_with_the_line_that_still_needs_the_old_name() {
    let source = "box(\"oak\", (10, 18, 30))\n\
                  for name in [\"oak\", \"b\"]:\n    if name == \"oak\":\n        move(name, by=(0, 0, 5))\n";
    let message = refusal(source, "oak", PartChange::Rename("lid".to_owned()));
    assert!(
        message.contains("lines 2, 3 of the program read \"oak\""),
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

fn feature_with(model: &ProgramModel, part: &str, path: &str) -> String {
    let part = model.part(part).unwrap();
    let feature = part
        .features
        .iter()
        .find(|feature| (feature.parameters.iter()).any(|parameter| parameter.path == path));
    feature.unwrap().name.clone()
}

/// Every part and tool but `part` is made as before.
fn others_same(before: &ProgramModel, after: &ProgramModel, part: &str) {
    let others = |model: &ProgramModel| {
        (model.parts.iter().chain(&model.tools))
            .filter(|made| made.name != part)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(others(after), others(before));
}

/// Review 2026-10-19 WB-2: a value one param() drives for several parts
/// changed them all, and could make a new part, without a word.
#[test]
fn a_param_that_drives_other_parts_too_is_not_overridden() {
    let source = "W = param(\"width\", 100)\n\
                  box(\"a\", (W, 20, 10))\n\
                  box(\"b\", (W, 20, 10), at=(0, 50, 0))\n\
                  if W > 110:\n    box(\"extra\", (5, 5, 5), at=(0, 100, 0))\n";
    let before = model(&program(source));
    let message = refusal(
        source,
        "a",
        PartChange::Parameter {
            feature: feature_with(&before, "a", "bounds.width"),
            path: "bounds.width".to_owned(),
            value: 120.0,
        },
    );
    assert!(
        message.contains("param(\"width\")")
            && message.contains("\"b\", \"extra\"")
            && message.contains("line 2"),
        "{message}"
    );
    // A length the shared param() drives is this part's own push_pull().
    let source =
        "H = param(\"h\", 18)\nbox(\"a\", (10, 20, H))\nbox(\"b\", (10, 20, H), at=(0, 50, 0))\n";
    let before = model(&program(source));
    let lengthened = change(
        source,
        "a",
        PartChange::Parameter {
            feature: feature_with(&before, "a", "extent.distance"),
            path: "extent.distance".to_owned(),
            value: 23.0,
        },
    );
    assert!(lengthened.overrides.is_empty());
    assert_eq!(
        lengthened.source,
        format!("{source}push_pull(\"a\", face=\"z+\", distance=5, name=\"Length\")\n")
    );
    let after = model(&lengthened);
    assert_eq!(Reading::Length.of(&after, "a"), Some(23.0));
    others_same(&before, &after, "a");
}

#[test]
fn a_side_of_the_cabinet_changes_alone() {
    let source = include_str!("../../../examples/programs/cabinet.star");
    let before = model(&program(source));
    let changed = change(
        source,
        "carcass/left",
        PartChange::PushPull {
            face: "z+".to_owned(),
            distance: 80.0,
        },
    );
    assert!(changed.overrides.is_empty(), "{:?}", changed.overrides);
    let after = model(&changed);
    others_same(&before, &after, "carcass/left");
    let (low, high) = after.part("carcass/left").unwrap().world_bounds();
    assert!(
        (high[2] - low[2] - 800.0).abs() < 1.0e-6,
        "{low:?} {high:?}"
    );
}

#[test]
fn an_overridden_param_is_written_without_probe_noise() {
    let source = "W = param(\"width\", 100)\nbox(\"p\", (W * 3, 20, 10))\n";
    let before = model(&program(source));
    let resized = change(
        source,
        "p",
        PartChange::Parameter {
            feature: feature_with(&before, "p", "bounds.width"),
            path: "bounds.width".to_owned(),
            value: 250.3,
        },
    );
    assert_eq!(resized.overrides["width"], 83.433333333);
    let whole = change(
        source,
        "p",
        PartChange::Parameter {
            feature: feature_with(&before, "p", "bounds.width"),
            path: "bounds.width".to_owned(),
            value: 270.0,
        },
    );
    assert_eq!(whole.overrides["width"], 90.0);
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

/// WB-9: a literal `at=` alone would leave the tool of the part's hole
/// behind in the world; the move is a move() line that carries it along.
#[test]
fn a_moved_part_keeps_its_holes_where_they_are_on_the_part() {
    let source = "a = box(\"a\", (100, 100, 18), at=(0, 0, 0))\n\
                  subtract(a, box(\"h\", (10, 10, 30), at=(45, 45, -5), tool=True))\n";
    let by = Transform::from_translation(20.0, 0.0, 0.0).unwrap();
    let moved = change(source, "a", PartChange::Transform(by));
    assert_eq!(
        moved.source,
        format!("{source}move(\"a\", by=(20, 0, 0))\n")
    );
    let after = model(&moved);
    let part = after.part("a").unwrap();
    assert_eq!(part.at_mm, [20.0, 0.0, 0.0]);
    let tools = part
        .booleans()
        .map(|boolean| boolean.tool.at_mm)
        .collect::<Vec<_>>();
    assert_eq!(tools, [[65.0, 45.0, -5.0]]);
    // Without booleans the literal is still the one written.
    let plain = change(PADDED, "a", PartChange::Transform(by));
    assert!(
        plain
            .source
            .starts_with("box(\"a\", (10, 20, 18), at=(20, 0, 0))\n")
    );
    assert!(!plain.source.contains("move("), "{}", plain.source);
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
    // The part is 25 long: the 2 below it stays and the extrusion is 23.
    assert_eq!(length(&typed), 25.0);
    assert_eq!(typed.source.matches("name=\"Length\"").count(), 1);
    assert!(typed.source.contains("distance=2, name=\"Lower\""));
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
    // A push of 15 below leaves the extrusion -5 for a part 10 long.
    let pushed = program(&format!(
        "{PADDED}push_pull(\"a\", face=\"z-\", distance=15, name=\"Lower\")\n"
    ));
    let refusal = set_length(&pushed, 10.0).expect_err("refused");
    assert!(refusal.to_string().contains("-5 mm long"), "{refusal}");
    // A pull of 15 below makes the extrusion 25 for a part 10 long.
    let pulled = program(&format!(
        "{PADDED}push_pull(\"a\", face=\"z-\", distance=-15, name=\"Lower\")\n"
    ));
    assert_eq!(length(&set_length(&pulled, 10.0).unwrap().unwrap()), 10.0);
}

/// WB-8: once a param() drives the length alone again, the length line is
/// written again instead of the parameter, so 10 then 12 is 12, not 4.
#[test]
fn a_length_typed_after_its_param_became_its_own_rewrites_the_length_line() {
    let shared = "H = param(\"h\", 18)\n\
                  box(\"a\", (10, 20, H), at=(0, 0, 0))\n\
                  box(\"b\", (10, 20, H), at=(0, 50, 0))\n";
    let first = set_length(&program(shared), 10.0).unwrap().unwrap();
    assert_eq!(length(&first), 10.0);
    let alone = rewrite_rule_program_parts(&first, &[("b".to_owned(), PartChange::Delete)])
        .unwrap()
        .unwrap();
    let second = set_length(&alone, 12.0).unwrap().unwrap();
    assert_eq!(length(&second), 12.0);
    assert!(second.overrides.is_empty(), "{:?}", second.overrides);
    assert_eq!(second.source.matches("name=\"Length\"").count(), 1);
}

/// WB-12: a typed value its one param() could only make past the range the
/// program declares for it is refused with that range, not written as a
/// length line that passes the range by.
#[test]
fn a_value_past_the_range_of_its_param_is_refused_with_the_range() {
    let source = program(
        "H = param(\"h\", 18, min=12, max=40)\n\
         box(\"a\", (10, 20, H), at=(0, 0, 0))\n",
    );
    for value in [10.0, 60.0] {
        let refusal = set_length(&source, value).expect_err("refused");
        assert!(refusal.to_string().contains("from 12 to 40"), "{refusal}");
    }
    let wide = "W = param(\"w\", 100, max=150)\nbox(\"p\", (W, 20, 10))\n";
    let before = model(&program(wide));
    let refusal = refusal(
        wide,
        "p",
        PartChange::Parameter {
            feature: feature_with(&before, "p", "bounds.width"),
            path: "bounds.width".to_owned(),
            value: 200.0,
        },
    );
    assert!(refusal.contains("at most 150"), "{refusal}");
    // Within the range the parameter is overridden as before.
    assert_eq!(
        set_length(&source, 30.0).unwrap().unwrap().overrides["h"],
        30.0
    );
}

/// WB-8: a param() that drives the length alone is set so the part with the
/// Push/Pulls of other lines is as long as typed; only a parameter of 0 or
/// less is refused.
#[test]
fn a_param_driven_length_makes_the_typed_length_with_other_pulls() {
    let source = program(
        "H = param(\"h\", 30)\n\
         box(\"a\", (10, 20, H), at=(0, 0, 0))\n\
         push_pull(\"a\", face=\"z+\", distance=-15, name=\"Shorter\")\n",
    );
    for (value, h) in [(10.0, 25.0), (25.0, 40.0)] {
        let typed = set_length(&source, value).unwrap().unwrap();
        assert_eq!(typed.overrides["h"], h);
        assert_eq!(length(&typed), value);
    }
    let pushed = program(
        "H = param(\"h\", 30)\n\
         box(\"a\", (10, 20, H), at=(0, 0, 0))\n\
         push_pull(\"a\", face=\"z+\", distance=15, name=\"Longer\")\n",
    );
    let refusal = set_length(&pushed, 10.0).expect_err("refused");
    assert!(refusal.to_string().contains("-5 mm long"), "{refusal}");
}
