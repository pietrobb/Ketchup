//! Each controller's G-code is library data (`crates/ketchup-model/library/cam-gcode-dialects.json`):
//! the same rendering and reading code serves every controller, and a caller's own dialect
//! needs no code at all.

use super::{
    bidirectional_arc_operations, plan, sha256_hex, solid_document, successful_simulation,
};
use ketchup_model::cam::{
    CamGCodeDialect, CamGCodeDialectError, CamPostprocessedMotionKind, CamPostprocessorDialect,
    CamPostprocessorError, CamPostprocessorOutput, CamToolpath,
};

fn output(dialect: CamPostprocessorDialect) -> CamPostprocessorOutput {
    let document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let toolpath =
        CamToolpath::plan(&snapshot, &canonical, &bidirectional_arc_operations()).unwrap();
    let simulation = successful_simulation(&canonical, &toolpath);
    let output =
        CamPostprocessorOutput::generate(&snapshot, &canonical, &toolpath, &simulation, dialect)
            .unwrap();
    output
        .verify(&snapshot, &canonical, &toolpath, &simulation)
        .unwrap();
    output
}

fn library_text(id: &str) -> String {
    let output = output(CamPostprocessorDialect::library(id).unwrap());
    String::from_utf8(output.content).unwrap()
}

/// The words of a line without its parenthesised comments, as a controller reads them.
fn words(line: &str) -> Vec<String> {
    let mut code = String::new();
    let mut depth = 0;
    for character in line.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 => code.push(character),
            _ => {}
        }
    }
    code.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn every_library_dialect_writes_the_simulated_motions_and_reads_them_back() {
    let ids = CamPostprocessorDialect::library_ids().unwrap();
    for expected in [
        "iso_metric_gcode",
        "fanuc",
        "grbl",
        "linuxcnc",
        "controller_neutral_json",
    ] {
        assert!(
            ids.iter().any(|id| id == expected),
            "{expected} missing from {ids:?}"
        );
    }
    let neutral = output(CamPostprocessorDialect::ControllerNeutralJson);
    for id in &ids {
        let dialect = CamPostprocessorDialect::library(id).unwrap();
        assert_eq!(dialect.id(), id);
        let resolution = dialect.resolution_mm();
        let written = output(dialect);
        assert_eq!(written.parse().unwrap(), written.program, "{id}");
        assert_eq!(
            written.program.motions.len(),
            neutral.program.motions.len(),
            "{id}"
        );
        for (motion, exact) in written.program.motions.iter().zip(&neutral.program.motions) {
            assert_eq!(motion.kind, exact.kind, "{id}");
            for (value, exact) in motion.end_mm.iter().zip(exact.end_mm) {
                assert!(
                    (value - exact).abs() <= resolution / 2.0 + 1.0e-12,
                    "{id}: {value} vs {exact}"
                );
            }
        }
    }
}

/// R1119og-MF-801 and MF-804: on a controller with tool length offsets the Z of a program is the
/// spindle's unless G43 applies the tool's length, so the tip would run that length lower. Every
/// library G-code dialect resets the modal state, applies the length of the tool it changes to (or,
/// without a tool change, says the tool is touched off), reaches the safe Z on its own and only then
/// moves in X and Y, and keeps its comments in parentheses, which no controller reads as code.
#[test]
fn every_library_g_code_dialect_applies_the_tool_length_and_reaches_safe_z_before_moving_across() {
    let ids = CamPostprocessorDialect::library_ids().unwrap();
    for id in ids.iter().filter(|id| *id != "controller_neutral_json") {
        let written = output(CamPostprocessorDialect::library(id).unwrap());
        let text = std::str::from_utf8(&written.content).unwrap();
        assert!(!text.contains(';'), "{id}: ';' comment in {text}");
        let lines = text.lines().map(words).collect::<Vec<_>>();
        let has = |line: &[String], word: &str| line.iter().any(|w| w == word);
        let first_across = lines
            .iter()
            .position(|line| line.iter().any(|w| w.starts_with(['X', 'Y'])))
            .unwrap_or_else(|| panic!("{id}: no motion in {text}"));
        let before = &lines[..first_across];
        for reset in ["G40", "G49", "G80", "G94"] {
            assert!(
                before.iter().any(|line| line
                    .iter()
                    .any(|w| w.trim_start_matches(['G', '0']) == &reset[1..])),
                "{id}: no {reset} before the first motion in {text}"
            );
        }
        let safe_z = written.program.safe_retract_z_mm;
        let to_safe_z = before
            .iter()
            .position(|line| {
                line.iter().any(|w| w == "G0" || w == "G00")
                    && line.iter().any(|w| {
                        w.strip_prefix('Z')
                            .and_then(|z| z.parse::<f64>().ok())
                            .is_some_and(|z| (z - safe_z).abs() < 1.0e-3)
                    })
            })
            .unwrap_or_else(|| panic!("{id}: no rapid to Z{safe_z} alone before X/Y in {text}"));
        let changes_tool = before
            .iter()
            .any(|line| has(line, "M6") || has(line, "M06"));
        if changes_tool {
            assert!(
                before[..=to_safe_z]
                    .iter()
                    .any(|line| has(line, "G43") && has(line, "H1")),
                "{id}: no G43 H1 by the first Z move in {text}"
            );
        } else {
            assert!(text.contains("LOAD TOOL 1"), "{id}: {text}");
        }
        let first_motion = &written.program.motions[0];
        assert!(
            (first_motion.end_mm[2] - written.program.safe_retract_z_mm).abs() < 1.0e-9,
            "{id}: the first motion leaves safe Z: {first_motion:?}"
        );
    }
}

/// R1124og-CAM-900: the last retract leaves the tip at the safe Z, a few millimetres over the work.
/// A controller that moves when G49 cancels the tool length runs the spindle that length lower,
/// into the part. A program that applies the tool length sends Z to the machine's reference
/// (G28 or G53) before it cancels the length, or leaves the cancel to the end of the program.
#[test]
fn a_program_cancels_the_tool_length_only_at_the_machine_reference() {
    let ids = CamPostprocessorDialect::library_ids().unwrap();
    let mut checked = 0;
    for id in ids.iter().filter(|id| *id != "controller_neutral_json") {
        let written = output(CamPostprocessorDialect::library(id).unwrap());
        let text = std::str::from_utf8(&written.content).unwrap();
        let lines = text.lines().map(words).collect::<Vec<_>>();
        let is = |word: &str, code: &str| {
            word.strip_prefix(&code[..1])
                .is_some_and(|number| number.trim_start_matches('0') == &code[1..])
        };
        let has = |line: &[String], code: &str| line.iter().any(|word| is(word, code));
        if !lines.iter().any(|line| has(line, "G43")) {
            continue;
        }
        checked += 1;
        let last_motion = lines
            .iter()
            .rposition(|line| line.iter().any(|w| w.starts_with(['X', 'Y'])))
            .unwrap();
        let mut at_reference = false;
        for line in &lines[last_motion + 1..] {
            if (has(line, "G28") || has(line, "G53")) && line.iter().any(|w| w.starts_with('Z')) {
                at_reference = true;
            }
            assert!(
                at_reference || !has(line, "G49"),
                "{id}: G49 before Z reaches the machine reference in {text}"
            );
        }
        if let Some(incremental) = lines[last_motion + 1..]
            .iter()
            .rposition(|line| has(line, "G91"))
        {
            assert!(
                lines[last_motion + 1 + incremental..]
                    .iter()
                    .any(|line| has(line, "G90")),
                "{id}: the program ends incremental (G91) in {text}"
            );
        }
    }
    assert!(checked >= 3, "only {checked} dialects apply a tool length");
}

#[test]
fn grbl_program_uses_only_words_grbl_1_1_lists_and_names_the_tool_in_a_comment() {
    let text = library_text("grbl");
    assert!(!text.contains('%'), "{text}");
    assert!(text.contains("(LOAD TOOL 1 BEFORE START)"), "{text}");
    let supported = [
        "G0", "G1", "G2", "G3", "G4", "G17", "G21", "G40", "G49", "G54", "G80", "G90", "G94", "M3",
        "M5", "M30",
    ];
    for line in text.lines() {
        for word in words(line) {
            let letter = word.chars().next().unwrap();
            assert!("FGIJMPSXYZ".contains(letter), "{word} in {line:?}");
            if matches!(letter, 'G' | 'M') {
                assert!(supported.contains(&word.as_str()), "{word} in {line:?}");
            }
        }
    }
}

#[test]
fn fanuc_program_has_a_program_number_sequence_numbers_and_decimal_points() {
    let text = library_text("fanuc");
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.first(), Some(&"%"));
    assert!(lines[1].starts_with('O'), "{}", lines[1]);
    assert_eq!(lines.last(), Some(&"%"));
    assert!(!text.contains(';'), "{text}");
    assert!(!text.chars().any(|c| c.is_ascii_lowercase()), "{text}");
    assert!(text.contains("T1 M06"), "{text}");
    assert!(text.contains("G43 H1"), "{text}");
    let mut expected = 10;
    for line in &lines[2..lines.len() - 1] {
        let words = words(line);
        assert_eq!(words.first(), Some(&format!("N{expected}")), "{line}");
        expected += 10;
        for word in &words[1..] {
            if word.starts_with(['X', 'Y', 'Z', 'I', 'J', 'F']) {
                assert!(word.contains('.'), "{word} without decimal point in {line}");
            }
        }
    }
}

#[test]
fn linuxcnc_program_changes_the_tool_applies_its_length_and_ends_between_percent_lines() {
    let text = library_text("linuxcnc");
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.first(), Some(&"%"));
    assert_eq!(lines.last(), Some(&"%"));
    assert!(text.contains("T1 M6"), "{text}");
    assert!(text.contains("G43 H1"), "{text}");
    assert_eq!(lines[lines.len() - 2], "M2");
    assert!(lines.iter().all(|line| line.len() <= 256));
}

const CALLER_DIALECT: &str = r#"{
  "schema": "ketchup.cam-gcode-dialect.v1",
  "id": "caller_router",
  "name": "A caller's router",
  "sources": ["written for this test"],
  "decimals": 2,
  "decimal_point": "always",
  "uppercase": false,
  "line_numbers": {"prefix": "N", "first": 1, "step": 1, "last": 9},
  "start": [],
  "header": [
    "; plan {plan_digest} toolpath {toolpath_digest}",
    "; simulation {simulation_fingerprint}",
    "; feeds {cutting_feed}/{plunge_feed} safe {safe_z}",
    "G21 G90 {work_offset} T{tool} M6 S{spindle_rpm} M3"
  ],
  "motions": {
    "rapid": "G0 X{x} Y{y} Z{z}",
    "retract": "G0 Z{z} X{x} Y{y}",
    "plunge": "G1 Z{z} X{x} Y{y} F{feed}",
    "cut": "G1 X{x} Y{y} Z{z} F{feed}",
    "arc_clockwise": "G2 X{x} Y{y} Z{z} I{i} J{j} F{feed}",
    "arc_counterclockwise": "G3 X{x} Y{y} Z{z} I{i} J{j} F{feed}"
  },
  "footer": ["M5", "M30"],
  "end": []
}"#;

#[test]
fn a_callers_dialect_is_data_and_reads_back_like_the_library_ones() {
    let dialect = CamGCodeDialect::from_json(CALLER_DIALECT).unwrap();
    assert_eq!(dialect.id(), "caller_router");
    let written = output(CamPostprocessorDialect::GCode(dialect.into()));
    let text = std::str::from_utf8(&written.content).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert!(lines[0].starts_with("N1 ; plan "), "{text}");
    // Sequence numbers wrap from 9 back to 1.
    assert!(lines[9].starts_with("N1 "), "{text}");
    assert!(text.contains("G0 Z"), "{text}");
    assert!(
        written
            .program
            .motions
            .iter()
            .any(|motion| { motion.kind == CamPostprocessedMotionKind::Retract })
    );
}

#[test]
fn a_dialect_that_cannot_be_read_back_is_refused_before_any_program_is_written() {
    let same_rapid_and_retract = CALLER_DIALECT.replace("G0 Z{z} X{x} Y{y}", "G0 X{x} Y{y} Z{z}");
    assert!(matches!(
        CamGCodeDialect::from_json(&same_rapid_and_retract),
        Err(CamGCodeDialectError::Ambiguous { .. })
    ));
    let cut_without_feed = CALLER_DIALECT.replace("G1 X{x} Y{y} Z{z} F{feed}", "G1 X{x} Y{y} Z{z}");
    assert!(matches!(
        CamGCodeDialect::from_json(&cut_without_feed),
        Err(CamGCodeDialectError::Template { .. })
    ));
    let no_tool = CALLER_DIALECT.replace(" T{tool} M6", "");
    assert!(matches!(
        CamGCodeDialect::from_json(&no_tool),
        Err(CamGCodeDialectError::Template { .. })
    ));
    let joined_values = CALLER_DIALECT.replace("X{x} Y{y} Z{z}\"", "X{x}{y} Z{z}\"");
    assert!(matches!(
        CamGCodeDialect::from_json(&joined_values),
        Err(CamGCodeDialectError::Template { .. })
    ));
    let unknown_value = CALLER_DIALECT.replace("{spindle_rpm}", "{speed}");
    assert!(matches!(
        CamGCodeDialect::from_json(&unknown_value),
        Err(CamGCodeDialectError::Template { .. })
    ));
    assert!(matches!(
        CamGCodeDialect::from_json("{\"schema\": 1}"),
        Err(CamGCodeDialectError::Json { .. })
    ));
    assert!(matches!(
        CamPostprocessorDialect::library("no_such_controller"),
        Err(CamGCodeDialectError::UnknownLibraryDialect { .. })
    ));
}

#[test]
fn a_changed_word_in_a_controller_program_is_refused() {
    let fanuc = output(CamPostprocessorDialect::library("fanuc").unwrap());
    let text = std::str::from_utf8(&fanuc.content).unwrap();
    let rapid = text.lines().find(|line| line.contains("(RAPID)")).unwrap();
    for changed_line in [
        rapid.replacen("G00", "G01", 1),
        rapid.replacen('.', "", 1),
        rapid.replacen(" (RAPID)", "", 1),
    ] {
        let mut changed = fanuc.clone();
        changed.content = text.replacen(rapid, &changed_line, 1).into_bytes();
        changed.content_digest = sha256_hex(&changed.content);
        assert!(
            matches!(changed.parse(), Err(CamPostprocessorError::Parse(_))),
            "{changed_line}"
        );
    }
}

/// Angle a controller turns through for an I/J arc from `start` to `end`: it runs equal ends as a
/// full circle.
fn controller_sweep(start: [f64; 2], end: [f64; 2], center: [f64; 2], clockwise: bool) -> f64 {
    use std::f64::consts::TAU;
    if start == end {
        return TAU;
    }
    let angle = |point: [f64; 2]| (point[1] - center[1]).atan2(point[0] - center[0]);
    let counterclockwise = (angle(end) - angle(start)).rem_euclid(TAU);
    if clockwise {
        (TAU - counterclockwise).rem_euclid(TAU)
    } else {
        counterclockwise
    }
}

/// R1120ac-CAM-900: a corner arc whose chord is under half a step of the dialect has the same
/// start and end once rounded, and FANUC, Grbl and LinuxCNC run I/J between equal ends as a full
/// circle through the part. Read back as a controller reads it, no written arc may turn through
/// more than the arc that was planned and simulated.
#[test]
fn a_corner_arc_shorter_than_a_step_is_never_written_as_a_full_circle() {
    use ketchup_model::cam::{CamMotionPath, CamOperation, CamPath2d, CamPathSegment2d};
    let document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let center = [25.0, 10.0];
    let radius = 3.2;
    let at = |angle: f64| {
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    };
    let below = -std::f64::consts::FRAC_PI_2;
    for turn in [1.0e-4, 2.0e-5, 5.0e-6] {
        let start = at(below - turn / 2.0);
        let operations = vec![CamOperation::Contour {
            id: 1,
            center_path: CamPath2d {
                start_mm: start,
                segments: vec![
                    CamPathSegment2d::Arc {
                        to_mm: at(below + turn / 2.0),
                        center_mm: center,
                        clockwise: false,
                    },
                    CamPathSegment2d::Line {
                        to_mm: [40.0, 20.0],
                    },
                    CamPathSegment2d::Line { to_mm: start },
                ],
            },
            top_z_mm: -2.0,
            bottom_z_mm: -4.0,
            applied_radial_allowance_mm: 0.2,
        }];
        let toolpath = CamToolpath::plan(&snapshot, &canonical, &operations).unwrap();
        let simulation = successful_simulation(&canonical, &toolpath);
        for id in CamPostprocessorDialect::library_ids().unwrap() {
            let dialect = CamPostprocessorDialect::library(&id).unwrap();
            let written = CamPostprocessorOutput::generate(
                &snapshot,
                &canonical,
                &toolpath,
                &simulation,
                dialect,
            )
            .unwrap_or_else(|error| panic!("{id} at {turn}: {error}"));
            written
                .verify(&snapshot, &canonical, &toolpath, &simulation)
                .unwrap();
            let program = written.parse().unwrap();
            assert_eq!(program.motions.len(), toolpath.motions.len(), "{id}");
            let mut at_mm = [0.0, 0.0];
            for (motion, planned) in program.motions.iter().zip(&toolpath.motions) {
                let end = [motion.end_mm[0], motion.end_mm[1]];
                if let (
                    Some(offset),
                    CamMotionPath::Arc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    },
                ) = (motion.center_offset_mm, &planned.path)
                {
                    let written_sweep = controller_sweep(
                        at_mm,
                        end,
                        [at_mm[0] + offset[0], at_mm[1] + offset[1]],
                        motion.kind == CamPostprocessedMotionKind::ArcClockwise,
                    );
                    let planned_sweep = controller_sweep(
                        [start_mm[0], start_mm[1]],
                        [end_mm[0], end_mm[1]],
                        [center_mm[0], center_mm[1]],
                        *clockwise,
                    );
                    assert!(
                        (written_sweep - planned_sweep).abs() < 1.0e-2,
                        "{id} at {turn}: written arc turns {written_sweep} rad, planned {planned_sweep}: {motion:?}"
                    );
                }
                at_mm = end;
            }
        }
    }
}

/// Review 2026-11-21 ac P3: an arc no dialect's grid can write was refused without saying which;
/// the refusal names the operation, the motion and where the arc starts.
#[test]
fn an_unwritable_arc_is_refused_with_its_place() {
    use ketchup_model::cam::{CamMotionPath, CamOperation, CamPath2d, CamPathSegment2d};
    let document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let operations = vec![CamOperation::Contour {
        id: 7,
        center_path: CamPath2d {
            start_mm: [25.0, 10.0],
            segments: vec![
                CamPathSegment2d::Arc {
                    to_mm: [25.0, 10.004],
                    center_mm: [25.0, 10.002],
                    clockwise: false,
                },
                CamPathSegment2d::Line {
                    to_mm: [40.0, 20.0],
                },
                CamPathSegment2d::Line {
                    to_mm: [25.0, 10.0],
                },
            ],
        },
        top_z_mm: -2.0,
        bottom_z_mm: -4.0,
        applied_radial_allowance_mm: 0.2,
    }];
    let toolpath = CamToolpath::plan(&snapshot, &canonical, &operations).unwrap();
    let simulation = successful_simulation(&canonical, &toolpath);
    let mut refused = 0;
    for id in CamPostprocessorDialect::library_ids().unwrap() {
        let dialect = CamPostprocessorDialect::library(&id).unwrap();
        let Err(error) = CamPostprocessorOutput::generate(
            &snapshot,
            &canonical,
            &toolpath,
            &simulation,
            dialect,
        ) else {
            continue;
        };
        let CamPostprocessorError::UnwritableArc {
            operation,
            motion,
            start,
        } = &error
        else {
            panic!("{id}: {error}");
        };
        assert_eq!(*operation, 7, "{id}");
        let CamMotionPath::Arc { start_mm, .. } = toolpath.motions[*motion].path else {
            panic!("{id}: motion {motion} is not an arc");
        };
        assert_eq!(
            start,
            &ketchup_model::cam::place_mm([start_mm[0], start_mm[1]])
        );
        let text = error.to_string();
        assert!(
            text.contains(&format!("operation 7 (motion {motion}, from X ")),
            "{text}"
        );
        refused += 1;
    }
    assert!(refused > 0, "no dialect refused an arc of radius 0.002 mm");
}

/// R1125og-CAM-1000: a word whose meaning depends on the controller or the machine (G28 is the
/// reference return only on FANUC-style controls, a mirror image on Heidenhain ISO; G53 Z0 is the
/// top of Z only where Z homes to 0 at the top; G43 H is FANUC's tool length) is used only by a
/// dialect whose sources say what it assumes of the machine, so the user can tell before running it.
#[test]
fn every_machine_dependent_word_a_dialect_writes_has_its_assumption_in_the_sources() {
    let mut checked = 0;
    for dialect in CamGCodeDialect::library().unwrap() {
        let id = dialect.id().to_owned();
        let text = library_text(&id);
        let lines = text.lines().map(words).collect::<Vec<_>>();
        for word in ["G28", "G53", "G43"] {
            let used = lines.iter().flatten().any(|w| {
                w.strip_prefix('G')
                    .is_some_and(|n| n.trim_start_matches('0') == &word[1..])
            });
            if !used {
                continue;
            }
            checked += 1;
            assert!(
                dialect.sources().iter().any(|source| {
                    source.contains(word) && source.to_ascii_lowercase().contains("assumes")
                }),
                "{id} writes {word} but no source says what it assumes of the machine: {:?}",
                dialect.sources()
            );
        }
    }
    assert!(
        checked >= 6,
        "only {checked} machine-dependent words checked"
    );
}

#[test]
fn no_controller_is_named_in_the_postprocessor_code() {
    for source in [
        include_str!("../../src/cam.rs"),
        include_str!("../../src/cam_gcode.rs"),
    ] {
        let lower = source.to_ascii_lowercase();
        for controller in ["fanuc", "grbl", "linuxcnc"] {
            assert!(
                !lower.contains(controller),
                "{controller} named in Rust source"
            );
        }
    }
}
