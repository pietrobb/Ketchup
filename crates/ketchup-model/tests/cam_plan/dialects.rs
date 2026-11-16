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
