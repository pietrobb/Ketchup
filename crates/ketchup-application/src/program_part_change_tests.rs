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
fn deleting_the_last_part_detaches_even_with_a_comment_or_param_left() {
    // Review 2026-10-19 P3: a comment or param() left behind kept the program
    // owning an empty document, so the window never said every part was gone.
    let kept = "# the only part\nW = param(\"w\", 10)\nbox(\"a\", (W, 20, 30))\n";
    let rewritten =
        rewrite_rule_program_parts(&program(kept), &[("a".to_owned(), PartChange::Delete)])
            .expect("changed");
    assert_eq!(rewritten, None);
}

#[test]
fn delete_of_a_part_another_line_uses_is_refused_with_that_line() {
    let used = "a = box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\nprint(a)\n";
    let message = refusal(used, "a", PartChange::Delete);
    assert!(
        message.contains("\"a\"") && message.contains("line 3"),
        "{message}"
    );
    // A string that only reads like the part's name is no use (review
    // 2026-10-24 WB-24).
    let named = "box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\nprint(\"a\")\n";
    let deleted = change(named, "a", PartChange::Delete);
    assert_eq!(
        deleted.source,
        "box(\"b\", (5, 5, 5), at=(50, 0, 0))\nprint(\"a\")\n"
    );
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

/// Review 2026-10-24 WB-23: a copy from its own `mirrored()` line took the
/// source's `box()` line as the call creating it, so deleting it said "a
/// changed too" and renaming it was refused as a name used elsewhere.
#[test]
fn a_mirrored_copy_is_deleted_and_renamed_on_its_own_line() {
    let source = "box(\"a\", (10, 20, 30), at=(100, 0, 0))\n\
                  mirrored(\"a\", \"a mirrored\", point=(0, 0, 0), normal=(1, 0, 0))\n";
    let deleted = change(source, "a mirrored", PartChange::Delete);
    assert_eq!(deleted.source, "box(\"a\", (10, 20, 30), at=(100, 0, 0))\n");
    let renamed = change(source, "a mirrored", PartChange::Rename("b".to_owned()));
    let after = model(&renamed);
    assert!(after.part("b").is_some(), "{}", renamed.source);
    assert_eq!(after.part("a"), model(&program(source)).part("a"));
    let coloured = change(source, "a mirrored", PartChange::Color(Some([1, 2, 3])));
    let after = model(&coloured);
    assert_eq!(after.part("a mirrored").unwrap().color, Some([1, 2, 3]));
    assert_eq!(after.part("a").unwrap().color, None, "{}", coloured.source);
    // The source and its copy coloured alike in one batch.
    let both =
        ["a", "a mirrored"].map(|name| (name.to_owned(), PartChange::Color(Some([9, 9, 9]))));
    let coloured = rewrite_rule_program_parts(&program(source), &both)
        .expect("coloured")
        .expect("a program");
    let after = model(&coloured);
    for name in ["a", "a mirrored"] {
        assert_eq!(
            after.part(name).unwrap().color,
            Some([9, 9, 9]),
            "{}",
            coloured.source
        );
    }
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
fn colour_goes_before_a_comment_after_the_last_argument() {
    let commented = "box(\"a\", (10, 20, 30),\n    at=(0, 0, 0)  # base\n)\n";
    let coloured = change(commented, "a", PartChange::Color(Some([200, 10, 0])));
    assert_eq!(
        coloured.source,
        "box(\"a\", (10, 20, 30),\n    at=(0, 0, 0), color=(200, 10, 0)  # base\n)\n"
    );
    let trailing = "box(\"a\", (10, 20, 30),  # size\n)\n";
    let coloured = change(trailing, "a", PartChange::Color(Some([1, 2, 3])));
    assert_eq!(
        coloured.source,
        "box(\"a\", (10, 20, 30), color=(1, 2, 3),  # size\n)\n"
    );
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

/// Review 2026-10-19 P3: a part renamed twice in one batch, a to p to q,
/// detached the program because q's old name p was no part of it yet.
#[test]
fn a_part_renamed_twice_in_one_batch_continues_its_document_name() {
    let chained = rewrite_rule_program_parts(
        &program(TWO),
        &[
            ("a".to_owned(), PartChange::Rename("p".to_owned())),
            ("p".to_owned(), PartChange::Rename("q".to_owned())),
        ],
    )
    .expect("renamed")
    .expect("still the program's part");
    assert_eq!(
        chained.source,
        "box(\"q\", (10, 20, 30), at=(0, 0, 0))\n\
         box(\"b\", (5, 5, 5), at=(50, 0, 0))\n\
         move(\"q\", by=(1, 0, 0))\n\
         continue_part(\"q\", was=\"a\")\n"
    );
    assert_eq!(model(&chained).continuations["q"], "a");
}

/// Review 2026-10-21 P3: a rename to the name of a group, a component, a
/// tool or a name another part continues was refused with the raw
/// evaluation error of the rewritten program.
#[test]
fn a_rename_to_a_name_the_program_uses_is_refused_with_that_use() {
    let parts = "box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\n";
    for (source, new, used) in [
        (format!("{parts}group(\"g\", [\"b\"])\n"), "g", "a group"),
        (
            format!("{parts}component(\"c\", [\"b\"])\n"),
            "c",
            "a component",
        ),
        (
            format!("{parts}subtract(\"b\", box(\"h\", (1, 1, 9), at=(51, 1, -2), tool=True))\n"),
            "h",
            "a tool",
        ),
        (
            "box(\"a\", (10, 20, 30))\nbox(\"side\", (5, 5, 5), at=(50, 0, 0))\n\
             continue_part(\"side\", was=\"b\")\n"
                .to_owned(),
            "b",
            "the earlier name of part \"side\"",
        ),
    ] {
        model(&program(&source));
        let message = refusal(&source, "a", PartChange::Rename(new.to_owned()));
        assert!(
            message.contains(&format!(
                "already uses {new:?} as {used}; choose another name"
            )),
            "{new}: {message}"
        );
    }
    // A part renamed back to its own earlier name is no other use of it.
    let side = change(TWO, "a", PartChange::Rename("side".to_owned()));
    let back = change(&side.source, "side", PartChange::Rename("a".to_owned()));
    assert!(model(&back).part("a").is_some(), "{}", back.source);
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

/// Review 2026-10-21 WB-11: a rename read every part named "<name> ..." as
/// derived from the renamed one, so "shelf" next to "shelf 2", a part with
/// a mirrored copy and "door" next to "door/handle" were refused.
#[test]
fn rename_keeps_the_parts_whose_names_only_start_with_the_old_name() {
    for (source, from, other) in [
        (
            "box(\"shelf\", (10, 18, 30))\nbox(\"shelf 2\", (10, 18, 30), at=(0, 50, 0))\n",
            "shelf",
            "shelf 2",
        ),
        (
            "box(\"a\", (10, 20, 30), at=(100, 0, 0))\n\
             mirrored(\"a\", \"a mirrored\", point=(0, 0, 0), normal=(1, 0, 0))\n",
            "a",
            "a mirrored",
        ),
        (
            "box(\"door\", (10, 18, 30))\nbox(\"door/handle\", (5, 5, 5), at=(0, 50, 0))\n",
            "door",
            "door/handle",
        ),
    ] {
        let before = model(&program(source));
        let renamed = change(source, from, PartChange::Rename("board".to_owned()));
        let after = model(&renamed);
        assert!(after.part("board").is_some(), "{}", renamed.source);
        assert_eq!(after.part(other), before.part(other), "{}", renamed.source);
        assert_eq!(after.continuations["board"], from);
    }
}

/// Review 2026-10-24 WB-24: the check after a rename read every string of the
/// model equal to the old name as the part's, so a part named like a text, a
/// feature kind or a face ("pad", "start", "x+") could not be renamed, and a
/// delete was refused by a text reading the part's name.
#[test]
fn a_text_feature_or_face_reading_the_name_neither_blocks_a_rename_nor_a_delete() {
    let texted = "box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\n\
                  text(\"t\", \"a\", at=(0, 0))\n";
    let deleted = change(texted, "a", PartChange::Delete);
    assert_eq!(
        deleted.source,
        "box(\"b\", (5, 5, 5), at=(50, 0, 0))\ntext(\"t\", \"a\", at=(0, 0))\n"
    );
    let renamed = change(texted, "a", PartChange::Rename("c".to_owned()));
    let after = model(&renamed);
    assert!(after.part("c").is_some(), "{}", renamed.source);
    assert_eq!(after.annotations, model(&program(texted)).annotations);

    for name in [
        "start",
        "end",
        "pad",
        "sketch",
        "transform",
        "extrusion",
        "x+",
    ] {
        let source = format!(
            "box({name:?}, (10, 20, 30))\npush_pull({name:?}, face=\"x+\", distance=5, name=\"Pull\")\n\
             box(\"b\", (5, 5, 5), at=(50, 0, 0))\n"
        );
        let before = model(&program(&source));
        let renamed = change(&source, name, PartChange::Rename("board".to_owned()));
        let after = model(&renamed);
        assert_eq!(
            after.part("board").map(|part| part.local_bounds()),
            before.part(name).map(|part| part.local_bounds()),
            "{name}: {}",
            renamed.source
        );
    }

    // A literal that does name the part still refuses the delete.
    let grouped = "box(\"a\", (10, 20, 30))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\n\
                   group(\"g\", [\"a\", \"b\"])\n";
    let message = refusal(grouped, "a", PartChange::Delete);
    assert!(message.contains("line 3"), "{message}");
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
    // A length the shared param() drives is this part's own push_pull(),
    // written against the parameter (WB-21).
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
        format!("{source}push_pull(\"a\", face=\"z+\", distance=23 - H, name=\"Length\")\n")
    );
    let after = model(&lengthened);
    assert_eq!(Reading::Length.of(&after, "a"), Some(23.0));
    others_same(&before, &after, "a");
}

/// Review 2026-10-21 WB-10: a param() that drove only this part was
/// overridden even when it also set another value of the part (a box's depth
/// with its width), a load or an expected gap.
#[test]
fn a_param_that_drives_more_of_the_part_or_the_program_is_not_overridden() {
    let width = |source: &str| {
        let before = model(&program(source));
        refusal(
            source,
            "a",
            PartChange::Parameter {
                feature: feature_with(&before, "a", "bounds.width"),
                path: "bounds.width".to_owned(),
                value: 150.0,
            },
        )
    };
    let message = width("W = param(\"w\", 100)\nbox(\"a\", (W, W, 10))\n");
    assert!(
        message.contains("param(\"w\")") && message.contains("bounds.height of part \"a\""),
        "{message}"
    );
    let message = width(
        "W = param(\"w\", 100)\nbox(\"a\", (W, 20, 10))\n\
         area_load(\"living\", kind=\"imposed\", kn_m2=W / 50, on=[\"a\"], source=\"test\")\n",
    );
    assert!(
        message.contains("param(\"w\")") && message.contains("the area loads of the program"),
        "{message}"
    );
    let message = width(
        "W = param(\"w\", 100)\na = box(\"a\", (W, 20, 10))\n\
         b = box(\"b\", (10, 20, 10), at=(300, 0, 0))\nexpect_gap(a, b, W + 100)\n",
    );
    assert!(
        message.contains("the expectations of the program"),
        "{message}"
    );
    // A length the param() also loads with is this part's own push_pull(),
    // and the load stays.
    let source = "H = param(\"h\", 18)\nbox(\"a\", (10, 20, H))\n\
                  area_load(\"living\", kind=\"imposed\", kn_m2=H / 9, on=[\"a\"], source=\"test\")\n";
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
    assert!(
        lengthened.overrides.is_empty(),
        "{:?}",
        lengthened.overrides
    );
    let after = model(&lengthened);
    assert_eq!(Reading::Length.of(&after, "a"), Some(23.0));
    assert_eq!(after.area_loads, before.area_loads);
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

/// Review 2026-10-24 WB-20: a mirrored copy made after the length line keeps
/// its length when the length is typed again; only part a changes.
#[test]
fn a_length_typed_again_leaves_a_later_copy_of_the_part_alone() {
    let first = set_length(&program(PADDED), 10.0).unwrap().unwrap();
    let mirrored = program(&format!(
        "{}mirrored(\"a\", \"a mirrored\", point=(0, 0, 0), normal=(1, 0, 0))\n",
        first.source
    ));
    let typed = set_length(&mirrored, 30.0).unwrap().unwrap();
    assert_eq!(length(&typed), 30.0);
    let (before, after) = (model(&mirrored), model(&typed));
    assert_eq!(after.part("a mirrored"), before.part("a mirrored"));
    assert_eq!(after.part("b"), before.part("b"));
}

/// Review 2026-10-24 WB-21: a length typed for a part whose length a shared
/// `param()` makes was written as the difference, so setting the parameter
/// later moved it (10 typed, h = 30, a 22 long). It is written against the
/// parameter, so the typed length stays and the parameter makes the rest.
#[test]
fn a_typed_length_stays_when_the_shared_parameter_that_made_it_changes() {
    let shared = program(
        "H = param(\"h\", 18)\nbox(\"a\", (10, 20, H), at=(0, 0, 0))\nbox(\"b\", (5, 5, H), at=(50, 0, 0))\n",
    );
    let typed = set_length(&shared, 10.0).unwrap().unwrap();
    assert_eq!(length(&typed), 10.0);
    let mut taller = typed.clone();
    taller.overrides.insert("h".to_owned(), 30.0);
    assert_eq!(length(&taller), 10.0, "{}", typed.source);
    let b = |source: &RuleProgramSource| Reading::Length.of(&model(source), "b").unwrap();
    assert_eq!(b(&taller), 30.0);
    // Typed again, the one line is written again.
    let again = set_length(&typed, 12.0).unwrap().unwrap();
    assert_eq!(again.source.matches("name=\"Length\"").count(), 1);
    taller = again.clone();
    taller.overrides.insert("h".to_owned(), 30.0);
    assert_eq!(length(&taller), 12.0, "{}", again.source);
}

fn push_pull(source: &RuleProgramSource, distance: f64) -> RuleProgramSource {
    let change = PartChange::PushPull {
        face: "z+".to_owned(),
        distance,
    };
    rewrite_rule_program_parts(source, &[("a".to_owned(), change)])
        .unwrap()
        .unwrap()
}

/// Review 2026-10-24 WB-22: a length line that goes away leaves its Push/Pull
/// number taken by a later Push/Pull, so the next one gets a free number
/// instead of a name the part already has.
#[test]
fn a_push_pull_after_a_removed_length_line_gets_a_free_name() {
    let pushed = push_pull(&program(PADDED), 5.0);
    let typed = set_length(&pushed, 10.0).unwrap().unwrap();
    let pushed_again = push_pull(&typed, 5.0);
    assert!(pushed_again.source.contains("GUI Push/Pull 3"));
    // 18 of the box and 10 of the two Push/Pulls: the length line goes.
    let back = set_length(&pushed_again, 28.0).unwrap().unwrap();
    assert!(!back.source.contains("\"Length\""), "{}", back.source);
    let third = push_pull(&back, 5.0);
    assert_eq!(length(&third), 33.0);
    assert!(third.source.contains("GUI Push/Pull 4"), "{}", third.source);
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
    assert!(
        refusal.to_string().contains("-5 mm long")
            && refusal.to_string().contains("the Push/Pull on line 3;"),
        "{refusal}"
    );
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

/// Review 2026-10-19 WB-5: a refusal of writing into the program had no
/// target and no fix, and a value no param() sets was refused with the line
/// of the part's creating call instead of the fillet() that writes it.
#[test]
fn a_refusal_names_the_program_line_to_change_and_its_fix() {
    let source = "box(\"p\", (40, 20, 10))\n\
                  move(\"p\", by=(1, 0, 0))\n\
                  fillet(\"p\", edges=[[\"z+\", \"x+\"]], radius=2, name=\"round\")\n";
    let made = model(&program(source));
    let round = (made.part("p").unwrap().features.iter())
        .find(|feature| feature.kind == ProgramFeatureKind::Fillet)
        .unwrap();
    let error = rewrite_rule_program_parts(
        &program(source),
        &[(
            "p".to_owned(),
            PartChange::Parameter {
                feature: round.name.clone(),
                path: round.parameters[0].path.clone(),
                value: 3.0,
            },
        )],
    )
    .expect_err("refused");
    let rejection = error.rejection();
    assert_eq!(rejection.code(), "program_parameter_not_driven");
    assert_eq!(rejection.target_name(), "program line 3");
    assert!(
        rejection.reason_text().contains("change it on line 3"),
        "{}",
        rejection.reason_text()
    );
    let entry = crate::rejections::host_rejection("program_parameter_not_driven").unwrap();
    assert_eq!(rejection.fix_hint_text(), entry.fix_hint);

    let looped = "for i in range(3):\n    box(\"s%d\" % i, (10, 10, 10), at=(0, 0, 20 * i))\n";
    let error =
        rewrite_rule_program_parts(&program(looped), &[("s1".to_owned(), PartChange::Delete)])
            .expect_err("refused");
    let rejection = error.rejection();
    assert_eq!(
        (rejection.code(), rejection.target_name()),
        ("program_part_in_loop", "program line 1")
    );
    let RuleProgramApplyError::Program {
        location: Some(location),
        ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(location.excerpt[0].text, "for i in range(3):");
}

#[test]
fn deleting_a_part_made_under_an_if_names_the_if() {
    let source = "W = 120\nbox(\"a\", (10, 10, 10))\nif W > 110:\n    box(\"extra\", (5, 5, 5), at=(0, 100, 0))\n";
    let error = rewrite_rule_program_parts(
        &program(source),
        &[("extra".to_owned(), PartChange::Delete)],
    )
    .expect_err("refused");
    let rejection = error.rejection();
    assert_eq!(
        (rejection.code(), rejection.target_name()),
        ("program_part_in_condition", "program line 3")
    );
    assert!(
        rejection.reason_text().contains("if on line 3"),
        "{}",
        rejection.reason_text()
    );
}

/// Review 2026-10-19 WB-5: a program that did not make the part as asked
/// was refused without saying what came out otherwise.
#[test]
fn a_mismatch_says_what_the_program_made_otherwise() {
    let before = model(&program(TWO));
    let placed = Expected::Placed {
        part: "a".to_owned(),
        at: [5.0, 0.0, 0.0],
        rotation: before.part("a").unwrap().rotation,
    };
    assert_eq!(
        placed.difference(&before, &before),
        "part \"a\" stands at (1, 0, 0) instead of (5, 0, 0)"
    );
    let mut after = before.clone();
    after.parts.retain(|part| part.name != "b");
    let same = Expected::Same(Box::new(before.clone()));
    assert_eq!(same.difference(&before, &after), "\"b\" changed too");
}

/// Review 2026-10-19 WB-6: moving many parts alike evaluated the whole
/// program two or three times for each part, so selecting all and moving
/// froze the window for seconds.
#[test]
fn many_parts_moved_alike_are_written_and_checked_at_once() {
    let boxes = (0..12)
        .map(|i| format!("box(\"p{i}\", (10, 10, 10), at=({}, 0, 0))\n", 20 * i))
        .collect::<String>();
    let moved_by = |names: &[String]| {
        names
            .iter()
            .map(|name| {
                let shift = Transform::from_translation(5.0, 0.0, 0.0).unwrap();
                (name.clone(), PartChange::Transform(shift))
            })
            .collect::<Vec<_>>()
    };
    let names = (0..12).map(|i| format!("p{i}")).collect::<Vec<_>>();
    let before = crate::rule_program::EVALUATIONS.get();
    let moved = rewrite_rule_program_parts(&program(&boxes), &moved_by(&names))
        .unwrap()
        .unwrap();
    let used = crate::rule_program::EVALUATIONS.get() - before;
    assert!(used <= 3, "{used} evaluations for 12 parts");
    // Each literal at= moves, as for one part.
    assert!(
        moved
            .source
            .contains("box(\"p3\", (10, 10, 10), at=(65, 0, 0))"),
        "{}",
        moved.source
    );
    let after = model(&moved);
    for (i, name) in names.iter().enumerate() {
        let x = 20.0 * f64::from(u8::try_from(i).unwrap()) + 5.0;
        assert_eq!(after.part(name).unwrap().at_mm, [x, 0.0, 0.0]);
    }
    // A part placed without a literal: every part gets its move() line.
    let mixed = format!("{boxes}box(\"q\", (10, 10, 10))\n");
    let mut all = names.clone();
    all.push("q".to_owned());
    let moved = rewrite_rule_program_parts(&program(&mixed), &moved_by(&all))
        .unwrap()
        .unwrap();
    assert!(moved.source.starts_with(&mixed), "{}", moved.source);
    assert_eq!(moved.source.matches("move(").count(), 13);
    let after = model(&moved);
    assert_eq!(after.part("q").unwrap().at_mm, [5.0, 0.0, 0.0]);
    assert_eq!(after.part("p11").unwrap().at_mm, [225.0, 0.0, 0.0]);
}

/// Review 2026-10-19 WB-6 (rest in review 2026-10-24): one part moved by its
/// literal `at=` was evaluated three times, the program it wrote once more after
/// the check that had already made it.
#[test]
fn one_part_moved_by_its_literal_costs_two_evaluations() {
    let source = program(TWO);
    let current = evaluate(&source).unwrap();
    let shift = Transform::from_translation(0.0, 5.0, 0.0).unwrap();
    let before = crate::rule_program::EVALUATIONS.get();
    let moved = rewrite_rule_program_parts_from(
        &source,
        Some(&current),
        &[("b".to_owned(), PartChange::Transform(shift))],
    )
    .unwrap()
    .unwrap();
    let used = crate::rule_program::EVALUATIONS.get() - before;
    assert_eq!(used, 2);
    assert!(
        moved.source.source.contains("at=(50, 5, 0)"),
        "{}",
        moved.source.source
    );
    assert_eq!(
        moved.evaluated.model.part("b").unwrap().at_mm,
        [50.0, 5.0, 0.0]
    );
}

/// Review 2026-10-19 (program writing) and 2026-10-21 (GUI): a Push/Pull distance or a
/// fillet or chamfer amount the window computed is written to the nanometre, as every
/// other written length is, not with the noise of its sum (0.30000000000000004).
#[test]
fn a_pulled_distance_and_a_finish_amount_are_written_without_noise() {
    let source = "box(\"p\", (40, 20, 10))\n";
    let noisy = 0.1 + 0.2;
    let pulled = change(
        source,
        "p",
        PartChange::PushPull {
            face: "z+".to_owned(),
            distance: noisy,
        },
    );
    assert!(pulled.source.contains("distance=0.3,"), "{}", pulled.source);
    for kind in [
        ProgramEdgeFinishKind::Fillet,
        ProgramEdgeFinishKind::Chamfer,
    ] {
        let finished = change(
            source,
            "p",
            PartChange::Finish {
                kind,
                edges: vec![["z+".to_owned(), "x+".to_owned()]],
                amount: noisy,
            },
        );
        assert!(finished.source.contains("=0.3)"), "{}", finished.source);
    }
}

const WINDOW: &str = "wall = box(\"wall\", (4000, 200, 2600))\n\
                      window(\"win\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 2200, 900, 1200, 1250, cut=wall)\n";

fn set(feature: &str, path: &str, value: f64) -> PartChange {
    PartChange::Parameter {
        feature: feature.to_owned(),
        path: path.to_owned(),
        value,
    }
}

/// Review 2026-10-24 WB-26: the parameter panel refused the place and the
/// sizes of a literal `box(...)`, though Move writes its literal `at=` in place.
#[test]
fn a_value_one_literal_of_the_creating_call_makes_is_written_in_place() {
    let source =
        "box(\"a\", (600, 18, 720), at=(10, 0, 0))\nbox(\"b\", (5, 5, 5), at=(900, 0, 0))\n";
    for (feature, path, value, written) in [
        ("a transform", "translation.x", 17.0, "at=(17, 0, 0)"),
        ("a transform", "translation.z", -4.5, "at=(10, 0, -4.5)"),
        ("a sketch", "bounds.width", 607.0, "(607, 18, 720)"),
        ("a sketch", "bounds.height", 25.0, "(600, 25, 720)"),
    ] {
        let changed = change(source, "a", set(feature, path, value));
        assert!(changed.source.contains(written), "{}", changed.source);
        assert_eq!(changed.source.lines().count(), 2, "{}", changed.source);
        let reading = Reading::Feature {
            feature: feature.to_owned(),
            path: path.to_owned(),
        };
        assert_eq!(reading.of(&model(&changed), "a"), Some(value));
        others_same(&model(&program(source)), &model(&changed), "a");
    }
    let profile = "extrude(\"e\", profile=[[0, 0], [100, 0], [100, 50]], distance=18)\n";
    let changed = change(profile, "e", set("e sketch", "entities.1.end.x", 107.0));
    assert_eq!(changed.source, profile.replace("[100, 0]", "[107, 0]"));
    // A literal whose change moves another part too stays refused.
    let read =
        "a = box(\"a\", (600, 18, 720))\nbox(\"b\", (5, 5, 5), at=(part_info(a).max[0], 0, 0))\n";
    let message = refusal(read, "a", set("a sketch", "bounds.width", 607.0));
    assert!(message.contains("not set by one param()"), "{message}");
}

/// Review 2026-10-24 WB-25: one part of a `window()` was deleted, coloured or
/// renamed through the whole call ("wall, win/frame changed too", "a name from
/// an expression"), and its pane moved alone left the opening in the wall.
#[test]
fn a_part_one_call_makes_with_others_is_refused_with_that_call() {
    let by = PartChange::Transform(Transform::from_translation(100.0, 0.0, 0.0).unwrap());
    for (name, edit) in [
        ("win/glass", PartChange::Delete),
        ("win/glass", PartChange::Rename("pane".to_owned())),
        ("win/frame left", PartChange::Color(Some([1, 2, 3]))),
    ] {
        let message = refusal(WINDOW, name, edit);
        assert!(
            message.contains("window() on line 2")
                && message.contains("\"win/frame top\"")
                && message.contains("changes \"wall\""),
            "{message}"
        );
    }
    let message = refusal(WINDOW, "win/glass", by.clone());
    assert!(
        message.contains("window() on line 2") && message.contains("also changes \"wall\""),
        "{message}"
    );
    // The wall moved with the window carries its opening along.
    let before = model(&program(WINDOW));
    let all = (before.parts.iter())
        .map(|part| (part.name.clone(), by.clone()))
        .collect::<Vec<_>>();
    let moved = rewrite_rule_program_parts(&program(WINDOW), &all)
        .expect("moved")
        .expect("a program");
    let after = model(&moved);
    for part in &before.parts {
        let x = after.part(&part.name).unwrap().at_mm[0];
        assert!((x - part.at_mm[0] - 100.0).abs() < 1e-9, "{}", moved.source);
    }
}

/// Review 2026-10-19 WB-5 (rest in review 2026-10-24): a name the program
/// already gives is refused with the line that gives it; the typed name stays
/// the target, as it is what changes.
#[test]
fn a_name_the_program_gives_is_refused_with_the_line_that_gives_it() {
    let message = refusal(TWO, "a", PartChange::Rename("b".to_owned()));
    assert!(
        message.contains("a part named \"b\" on line 2; choose another name"),
        "{message}"
    );
    let mirror = PartChange::Mirror {
        copy: "b".to_owned(),
        point: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    };
    let message = refusal(TWO, "a", mirror);
    assert!(message.contains("named \"b\" on line 2;"), "{message}");
    let message = refusal(TWO, "a", PartChange::Rename(" ".to_owned()));
    assert!(message.contains("a part needs a name"), "{message}");
}

/// Review 2026-10-19 WB-5 (rest in review 2026-10-24): a program the edit
/// rewrote that then failed was refused at the line of the rewritten text,
/// "program line 3" of a program of two lines, with a traceback quoting lines
/// the user never saw.
#[test]
fn a_program_that_fails_after_the_edit_is_refused_on_the_users_lines() {
    let source = "box(\"a\", (10, 10, 10))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\n";
    let pull = |part: &str, face: &str| {
        let change = PartChange::PushPull {
            face: face.to_owned(),
            distance: 5.0,
        };
        (part.to_owned(), change)
    };
    let error = rewrite_rule_program_parts(&program(source), &[pull("a", "z+"), pull("b", "nope")])
        .expect_err("refused");
    let rejection = error.rejection();
    assert_eq!(rejection.code(), "evaluation_error");
    assert!(
        !rejection.target_name().starts_with("program line"),
        "{}",
        rejection.target_name()
    );
    let reason = rejection.reason_text();
    assert!(
        reason.starts_with("the line the edit adds, `push_pull(\"b\", face=\"nope\"")
            && reason.contains("face \"nope\" does not exist")
            && !reason.contains("edit.star:4"),
        "{reason}"
    );

    // Line 2 of the rewritten program is line 3 of the user's, once the
    // delete earlier in the batch took out line 1.
    let source = "box(\"b\", (5, 5, 5), at=(50, 0, 0))\n\
                  box(\"a\", (10, 10, 10))\n\
                  move(chr(97), by=(1, 0, 0))\n";
    let error = rewrite_rule_program_parts(
        &program(source),
        &[
            ("b".to_owned(), PartChange::Delete),
            ("a".to_owned(), PartChange::Rename("c".to_owned())),
        ],
    )
    .expect_err("refused");
    let rejection = error.rejection();
    assert_eq!(rejection.target_name(), "program line 3", "{error:?}");
    assert!(
        (rejection.reason_text()).starts_with("after the edit the program fails on line 3: "),
        "{}",
        rejection.reason_text()
    );
    let RuleProgramApplyError::Program {
        location: Some(location),
        ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert!(
        (location.excerpt.iter()).any(|line| line.line == 1 && line.text.starts_with("box(\"b\"")),
        "{location:?}"
    );
}

/// Review 2026-10-24 capability finding on WB-5: a later change of a batch
/// was refused on the lines of the program as the earlier changes left it,
/// which the user never saw.
#[test]
fn a_later_change_of_a_batch_is_refused_on_the_users_lines() {
    let source = "box(\"a\", (5, 5, 5), at=(50, 0, 0))\n\
                  for i in range(2):\n    \
                  box(\"r%d\" % i, (10, 10, 10), at=(0, 0, 20 * i))\n";
    let error = rewrite_rule_program_parts(
        &program(source),
        &[
            ("a".to_owned(), PartChange::Delete),
            ("r0".to_owned(), PartChange::Color(Some([1, 2, 3]))),
        ],
    )
    .expect_err("refused");
    let rejection = error.rejection();
    assert_eq!(rejection.target_name(), "program line 2", "{error:?}");
    assert!(
        rejection.reason_text().contains("loop on line 2"),
        "{}",
        rejection.reason_text()
    );
    let RuleProgramApplyError::Program {
        location: Some(location),
        ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert!(
        (location.excerpt.iter()).any(|line| line.line == 1 && line.text.starts_with("box(\"a\"")),
        "{location:?}"
    );

    // A name an earlier change gave on a line it added: the user has no such
    // line to look at.
    let source = "box(\"a\", (5, 5, 5))\nbox(\"b\", (5, 5, 5), at=(50, 0, 0))\n";
    let error = rewrite_rule_program_parts(
        &program(source),
        &[
            (
                "a".to_owned(),
                PartChange::Mirror {
                    copy: "m".to_owned(),
                    point: [100.0, 0.0, 0.0],
                    normal: [1.0, 0.0, 0.0],
                },
            ),
            ("b".to_owned(), PartChange::Rename("m".to_owned())),
        ],
    )
    .expect_err("refused");
    let RuleProgramApplyError::Program {
        message, location, ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert!(location.is_none(), "{location:?}");
    assert!(
        message.contains("named \"m\" on a line an earlier change of the edit adds"),
        "{message}"
    );

    // The same after an earlier change deleted a line: the added line is not
    // the user's line 3, `box("b"…)` (capability finding run 319).
    let source = "box(\"a\", (5, 5, 5))\n\
                  box(\"c\", (5, 5, 5), at=(0, 50, 0))\n\
                  box(\"b\", (5, 5, 5), at=(50, 0, 0))\n";
    let error = rewrite_rule_program_parts(
        &program(source),
        &[
            ("c".to_owned(), PartChange::Delete),
            (
                "a".to_owned(),
                PartChange::Mirror {
                    copy: "m".to_owned(),
                    point: [100.0, 0.0, 0.0],
                    normal: [1.0, 0.0, 0.0],
                },
            ),
            ("b".to_owned(), PartChange::Rename("m".to_owned())),
        ],
    )
    .expect_err("refused");
    let RuleProgramApplyError::Program {
        message, location, ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert!(location.is_none(), "{location:?}");
    assert!(
        message.contains("named \"m\" on a line an earlier change of the edit adds"),
        "{message}"
    );
}
