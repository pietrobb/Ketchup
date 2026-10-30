//! `program action=from_underlay`: what an imported drawing draws becomes data of the
//! window's program, so a part follows the drawing: a floor extruded from the room its
//! walls enclose, rewritten in place when asked again, an object's outline and every
//! placement of a block, each as one Undo step.

use super::program_edit::{evaluate_exact, exact_volume};
use super::underlay_objects::{DRAWING, ask, imported, walled_room};
use super::*;
use ketchup_model::document::CanonicalCommand;

fn source(app: &KetchupApp) -> String {
    app.document.current_rule_program().unwrap().source.clone()
}

fn from_underlay(app: &mut KetchupApp, bridge: &mut LiveBridge, arguments: Value) -> Value {
    let mut arguments = arguments;
    arguments["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    bridge
        .execute(app, ask("program_from_underlay", arguments), false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()))
}

fn assert_profile_lines(app: &KetchupApp, expected: &[[[f64; 3]; 2]]) {
    let lines = app.open_profile_line_segments();
    assert_eq!(lines.len(), expected.len(), "native viewport segments");
    for expected in expected {
        assert!(
            lines.iter().any(|(_, points)| {
                points.iter().zip(expected).all(|(point, expected)| {
                    [point.x, point.y, point.z]
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| (actual - expected).abs() < 1e-6)
                })
            }),
            "missing segment {expected:?}: {lines:?}"
        );
    }
}

const FLOOR: &str = "
def floor_of(outline, holes):
    floor = extrude(\"floor\", profile = outline, distance = 200)
    for index, hole in enumerate(holes):
        cut(floor, profile = hole, through = True, name = \"hole%d\" % (index + 1))
    return floor

floor_of(ROOM, ROOM_HOLES)
";

fn replace_drawing(
    app: &mut KetchupApp,
    drawing: &str,
) -> Result<
    ketchup_model::document::VerifiedProposalCommit,
    crate::program_edit::WorkRecoveryMutationError<ketchup_model::document::ProposalCommitError>,
> {
    let (replacement, _, _) = imported(drawing);
    let incoming = replacement
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .clone();
    let mut underlay = app.document.current().underlays().next().unwrap().clone();
    let mut container = app.file.container_data.clone();
    container
        .insert_import_blob(
            replacement.file.container_data.blobs()[&incoming.lines_blob()].to_vec(),
        )
        .unwrap();
    underlay.lines_sha256 = incoming.lines_sha256;
    underlay.offset_mm_bits = incoming.offset_mm_bits;
    let proposal = app
        .document
        .prepare_proposal(CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(
            underlay,
        )]))
        .unwrap();
    app.commit_verified_proposal_with_container_work_recovery(&proposal, container)
}

#[test]
fn underlay_binding_updates_the_profile_and_exact_slab_from_changed_source() {
    let drawing = walled_room();
    let (mut app, mut bridge, offset) = imported(&drawing);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":[2000.0-offset[0], 2000.0-offset[1]], "layers":["walls"]}),
    );
    let apply = Request::ApplyProgram {
        expected: Some(app.live_bridge_stamp()),
        source: format!("{}{FLOOR}", source(&app)),
        overrides: None,
        file_name: None,
        replace_document: false,
    };
    bridge.execute(&mut app, apply, false).unwrap();
    evaluate_exact(&mut app);
    let before_volume = exact_volume(&app, "floor");
    let before_source = source(&app);
    let before_underlay = app.document.current().underlays().next().unwrap().clone();
    let steps = app.undo_step_count();
    replace_drawing(
        &mut app,
        &drawing.replace("9700", "10700").replace("10000", "11000"),
    )
    .unwrap();
    assert_eq!(app.undo_step_count(), steps + 1);
    assert!(source(&app).ends_with(FLOOR));
    let area = 10_400.0 * 5_400.0;
    let snapshot = app.document.current();
    assert!(
        snapshot.features().any(|feature| match feature.kind() {
            FeatureKind::Profile {
                segments,
                closed: true,
            } => {
                let actual = segments
                    .iter()
                    .map(|segment| {
                        let [x, y] = segment.start_mm();
                        let [u, v] = segment.end_mm();
                        (x * v - y * u) / 2.0
                    })
                    .sum::<f64>()
                    .abs();
                (actual - area).abs() < 1.0
            }
            _ => false,
        }),
        "native profile must follow the wider source drawing"
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - area * 200.0).abs() < 1.0);
    assert!(app.undo());
    assert_eq!(source(&app), before_source);
    assert_eq!(
        app.document.current().underlays().next().unwrap(),
        &before_underlay
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - before_volume).abs() < 1.0);
    assert!(app.redo());
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - area * 200.0).abs() < 1.0);
    // Placement changes use the same binding; scaling the input changes area, not slab height.
    let id = before_underlay.id;
    let steps = app.undo_step_count();
    app.fit_underlay(
        id,
        [[0.0, 0.0], [1000.0, 0.0]],
        [[100.0, 200.0], [2100.0, 200.0]],
        true,
    )
    .unwrap();
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - 4.0 * area * 200.0).abs() < 1.0);
    assert_eq!(app.undo_step_count(), steps + 1);
    // A broken dependency cannot publish a new input beside stale derived geometry.
    let steps = app.undo_step_count();
    assert!(app.remove_underlay(id).is_err());
    assert_eq!(app.undo_step_count(), steps);
    assert!(app.document.current().underlay(id).is_some());
    assert!((exact_volume(&app, "floor") - 4.0 * area * 200.0).abs() < 1.0);
}

/// The floor of a room named in the drawing in one call: no lookup of the text or the
/// region first, no second apply for the extrusion; it follows the room when it grows.
#[test]
fn a_labelled_room_becomes_a_slab_in_one_call_and_follows_the_drawing() {
    let drawing = walled_room();
    let (mut app, mut bridge, offset) = imported(&drawing);
    let steps = app.undo_step_count();
    let answer = from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "label":"KITCHEN", "layers":["walls"], "thickness_mm":200}),
    );
    assert_eq!(app.undo_step_count(), steps + 1, "one Undo step");
    let written = source(&app);
    assert!(
        written.contains("the region labelled \"KITCHEN\"")
            && written.contains("area 50.760 m², perimeter 29.600 m, 0 holes"),
        "{written}"
    );
    assert!(
        answer["source_diff"].to_string().contains("area 50.760 m²"),
        "the answer shows what was found: {answer}"
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 9_400.0 * 5_400.0 * 200.0).abs() < 1.0);

    // In the walls' ring the room is a hole, cut through the slab.
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"RING", "point_mm":[150.0 - offset[0], 3_000.0 - offset[1]], "layers":["walls"], "thickness_mm":100}),
    );
    assert!(source(&app).contains("RING_SOLID = extrude(\"RING\""));
    evaluate_exact(&mut app);
    let ring = |x: f64| (x * 6_000.0 - (x - 600.0) * 5_400.0) * 100.0;
    assert!((exact_volume(&app, "RING") - ring(10_000.0)).abs() < 1.0);

    replace_drawing(
        &mut app,
        &drawing.replace("9700", "10700").replace("10000", "11000"),
    )
    .unwrap();
    evaluate_exact(&mut app);
    assert!(
        (exact_volume(&app, "ROOM") - 10_400.0 * 5_400.0 * 200.0).abs() < 1.0,
        "the slab follows the labelled room"
    );
    assert!((exact_volume(&app, "RING") - ring(11_000.0)).abs() < 1.0);

    let digest = app.canonical_digest();
    for (arguments, field) in [
        (json!({"name":"X", "label":"Bathroom"}), "label"),
        (json!({"name":"X", "label":"  "}), "label"),
        (
            json!({"name":"X", "label":"Kitchen", "point_mm":[0, 0]}),
            "point_mm",
        ),
        (
            json!({"name":"X", "label":"Kitchen", "thickness_mm":-1}),
            "thickness_mm",
        ),
    ] {
        let mut request = arguments.clone();
        request["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
        let code = bridge
            .execute(&mut app, ask("program_from_underlay", request), false)
            .unwrap_err();
        assert_eq!(code, "invalid_params", "{arguments}");
        let details = take_error_details().unwrap_or_default().to_string();
        assert!(details.contains(field), "{arguments}: {details}");
    }
    assert_eq!(
        app.canonical_digest(),
        digest,
        "a refused call changes nothing"
    );
}

/// Two rooms with the same text: refused with both, until the whole label names one.
#[test]
fn a_label_in_several_regions_is_refused_with_the_candidates() {
    let drawing = walled_room().replace(
        "0\nENDSEC\n0\nEOF\n",
        "0\nLWPOLYLINE\n8\nwalls\n90\n4\n70\n1\n10\n20000\n20\n0\n10\n23000\n20\n0\n10\n23000\n20\n2000\n10\n20000\n20\n2000\n\
         0\nMTEXT\n8\nrooms\n10\n21000\n20\n1000\n40\n100\n1\nKitchen\\P2\n0\nENDSEC\n0\nEOF\n",
    );
    let (mut app, mut bridge, _) = imported(&drawing);
    let mut request = json!({"name":"ROOM", "label":"kitch", "layers":["walls"]});
    request["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    let code = bridge
        .execute(&mut app, ask("program_from_underlay", request), false)
        .unwrap_err();
    assert_eq!(code, "invalid_params");
    let details = take_error_details().unwrap_or_default().to_string();
    assert!(details.contains("2 regions"), "{details}");
    // The exact text wins over the one that only contains it.
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "label":"kitchen 2", "layers":["walls"], "thickness_mm":200}),
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 3_000.0 * 2_000.0 * 200.0).abs() < 1.0);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "label":"Kitchen", "layers":["walls"], "thickness_mm":200}),
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 9_400.0 * 5_400.0 * 200.0).abs() < 1.0);
}

#[test]
fn underlay_binding_without_handle_never_retargets_a_replaced_drawing() {
    use std::error::Error;

    let rectangle = |x, width| {
        format!(
            "0\nLWPOLYLINE\n8\nprofiles\n90\n4\n70\n1\n10\n{x}\n20\n0\n10\n{}\n20\n0\n10\n{}\n20\n500\n10\n{x}\n20\n500\n",
            x + width,
            x + width,
        )
    };
    let linked = rectangle(0, 1000);
    let other = rectangle(2000, 200);
    let unrelated = rectangle(3000, 300);
    let drawing = |entities: String| {
        format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
        )
    };
    let (mut app, mut bridge, _) = imported(&drawing(format!("{linked}{other}")));
    let underlay = app.document.current().underlays().next().unwrap().clone();
    let lines = app.cached_underlay_lines(&underlay).unwrap();
    let index = ketchup_model::underlay::object_query::ObjectIndex::new(&lines);
    let id = index.find(&Default::default())[0];
    assert_eq!(index.object(id).unwrap().handle, 0);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "object_id":id}),
    );
    let apply = Request::ApplyProgram {
        expected: Some(app.live_bridge_stamp()),
        source: format!("{}{FLOOR}", source(&app)),
        overrides: None,
        file_name: None,
        replace_document: false,
    };
    bridge.execute(&mut app, apply, false).unwrap();
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - 1000.0 * 500.0 * 200.0).abs() < 1.0);
    // The persisted reference must be safe too, not just the in-memory selection.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handleless.ketchup");
    assert!(app.save_document_to(&path));
    let (mut reopened, _) = setup();
    assert!(reopened.open_document_path(&path));
    app = reopened;
    evaluate_exact(&mut app);
    let before_source = source(&app);
    let profiles = |app: &KetchupApp| {
        app.document
            .current()
            .features()
            .filter_map(|feature| match feature.kind() {
                FeatureKind::Profile {
                    segments,
                    closed: true,
                } => Some(segments.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let geometry = profiles(&app);
    assert!(!geometry.is_empty());
    let area = |profiles: &Vec<Vec<ketchup_model::document::ProfileSegment>>| {
        profiles
            .iter()
            .map(|segments| {
                segments
                    .iter()
                    .map(|segment| {
                        let [x, y] = segment.start_mm();
                        let [u, v] = segment.end_mm();
                        (x * v - y * u) / 2.0
                    })
                    .sum::<f64>()
                    .abs()
            })
            .sum::<f64>()
    };
    assert!((area(&geometry) - 1000.0 * 500.0).abs() < 1.0);
    let before = app.document.current().underlays().next().unwrap().clone();
    let steps = app.undo_step_count();
    for entities in [
        format!("{unrelated}{linked}{other}"),
        format!("{other}{linked}"),
        format!("{linked}{linked}{other}"),
        other.clone(),
    ] {
        let error = replace_drawing(&mut app, &drawing(entities))
            .err()
            .expect("lost identity must reject the replacement");
        let program_error = error
            .source()
            .unwrap()
            .downcast_ref::<ketchup_application::RuleProgramApplyError>()
            .unwrap();
        let rejection = program_error.rejection();
        assert_eq!(rejection.target_name(), "object_id");
        assert!(rejection.reason_text().contains("identity is lost"));
        assert!(rejection.fix_hint_text().contains("from_underlay"));
        assert_eq!(profiles(&app), geometry);
        assert_eq!(source(&app), before_source);
        assert_eq!(app.document.current().underlays().next().unwrap(), &before);
        assert_eq!(app.undo_step_count(), steps);
        evaluate_exact(&mut app);
        assert!((exact_volume(&app, "floor") - 1000.0 * 500.0 * 200.0).abs() < 1.0);
    }
    // The same blob retains positional identity: placement still updates the model atomically.
    app.fit_underlay(
        before.id,
        [[0.0, 0.0], [1000.0, 0.0]],
        [[0.0, 0.0], [2000.0, 0.0]],
        true,
    )
    .unwrap();
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - 2000.0 * 1000.0 * 200.0).abs() < 1.0);
    assert!((area(&profiles(&app)) - 2000.0 * 1000.0).abs() < 1.0);
    assert_eq!(app.undo_step_count(), steps + 1);
    assert!(app.undo());
    assert_eq!(profiles(&app), geometry);
    assert_eq!(app.document.current().underlays().next().unwrap(), &before);
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - 1000.0 * 500.0 * 200.0).abs() < 1.0);
}

#[test]
fn underlay_binding_errors_keep_the_cause_and_leave_geometry_unchanged() {
    use std::error::Error;

    let (mut app, mut bridge, offset) = imported(&walled_room());
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":[2000.0-offset[0], 2000.0-offset[1]], "layers":["walls"]}),
    );
    let valid = app.document.current_rule_program().unwrap().clone();
    let mut malformed = valid.clone();
    malformed.source.push_str("\n# underlay binding {\n");
    app.apply_program_source(malformed, false).unwrap();
    let before = app.document.current().underlays().next().unwrap().clone();
    let geometry = app.open_profile_line_segments();
    let steps = app.undo_step_count();
    let batch = CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(before.clone())]);
    let error = app.apply_batch_with_work_recovery(&batch).unwrap_err();
    let program_error = error.source().unwrap();
    let rejection = program_error
        .source()
        .unwrap()
        .downcast_ref::<Rejection>()
        .unwrap();
    let json_error = rejection
        .source()
        .unwrap()
        .downcast_ref::<serde_json::Error>()
        .unwrap();
    assert!(json_error.is_eof());
    assert_eq!(rejection.target_name(), "program.underlay_binding");
    assert!(rejection.fix_hint_text().contains("from_underlay"));
    assert_eq!(app.open_profile_line_segments(), geometry);
    assert_eq!(app.document.current().underlays().next().unwrap(), &before);
    assert_eq!(app.undo_step_count(), steps);

    let mut no_region = valid;
    assert!(no_region.source.contains("\"layers\":[\"walls\"]"));
    no_region.source = no_region
        .source
        .replace("\"layers\":[\"walls\"]", "\"layers\":[]");
    app.apply_program_source(no_region, false).unwrap();
    let steps = app.undo_step_count();
    let error = app.apply_batch_with_work_recovery(&batch).unwrap_err();
    let program_error = error
        .source()
        .unwrap()
        .downcast_ref::<ketchup_application::RuleProgramApplyError>()
        .unwrap();
    let rejection = program_error.rejection();
    assert_eq!(rejection.target_name(), "point_mm");
    assert_eq!(
        rejection.reason_text(),
        "ROOM follows this drawing, and the edit leaves it no outline: No closed region of the chosen layers holds the point."
    );
    assert!(rejection.fix_hint_text().contains("underlay_regions"));
    assert!(rejection.fix_hint_text().contains("To keep ROOM as it is"));
    assert_eq!(app.open_profile_line_segments(), geometry);
    assert_eq!(app.document.current().underlays().next().unwrap(), &before);
    assert_eq!(app.undo_step_count(), steps);
}

#[test]
fn underlay_binding_updates_shared_block_geometry_from_changed_source() {
    let (mut app, mut bridge, _) = imported(DRAWING);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"DOORS", "block":"DOOR_80"}),
    );
    let lengths = |app: &KetchupApp| {
        app.open_profile_line_segments()
            .into_iter()
            .map(|(_, points)| {
                let a = points[0];
                let b = points[1];
                ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(lengths(&app), vec![800.0, 800.0]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("linked.ketchup");
    assert!(app.save_document_to(&path));
    let (mut reopened, _) = setup();
    assert!(reopened.open_document_path(&path));
    app = reopened;
    let steps = app.undo_step_count();
    replace_drawing(&mut app, &DRAWING.replace("11\n800\n", "11\n1200\n")).unwrap();
    assert_eq!(lengths(&app), vec![1200.0, 1200.0]);
    assert_eq!(app.undo_step_count(), steps + 1);
    assert!(app.undo());
    assert_eq!(lengths(&app), vec![800.0, 800.0]);
    assert!(app.redo());
    assert_eq!(lengths(&app), vec![1200.0, 1200.0]);
}

#[test]
fn a_floor_follows_the_room_its_walls_enclose_and_the_data_is_rewritten_in_place() {
    let (mut app, mut bridge, offset) = imported(&walled_room());
    let document = |[x, y]: [f64; 2]| json!([x - offset[0], y - offset[1]]);

    let written = from_underlay(
        &mut app,
        &mut bridge,
        json!({"name": "ROOM", "point_mm": document([2_000.0, 2_000.0])}),
    );
    assert_eq!(
        app.document.current().features().filter(|f| matches!(f.kind(), FeatureKind::Profile { segments, closed: true } if segments.len() == 4)).count(), 1,
        "one native closed sketch outline: {written}"
    );
    assert_eq!(
        app.document.current().underlays().count(),
        1,
        "the drawing stays beneath the program"
    );
    assert_eq!(
        app.document.current().features().count(),
        1,
        "no solid before extrusion"
    );
    let sketch = app.document.current();
    let FeatureKind::Profile { segments, .. } = sketch.features().next().unwrap().kind() else {
        panic!("native sketch required")
    };
    let area: f64 = segments
        .iter()
        .map(|segment| {
            let [x, y] = segment.start_mm();
            let [u, v] = segment.end_mm();
            (x * v - y * u) / 2.0
        })
        .sum();
    assert!(
        (area.abs() - 9_400.0 * 5_400.0).abs() < 1.0,
        "native profile area {area}"
    );
    let data = source(&app);
    assert!(
        data.starts_with("# underlay data ROOM: the region around ("),
        "{data}"
    );
    assert!(data.contains("ROOM_HOLES = [\n]"), "{data}");
    assert_eq!(data.matches("), (").count() + 1, 4, "four corners: {data}");

    // The program builds on the data like on any value of its own.
    let apply = Request::ApplyProgram {
        expected: Some(app.live_bridge_stamp()),
        source: format!("{data}{FLOOR}"),
        overrides: None,
        file_name: None,
        replace_document: false,
    };
    bridge.execute(&mut app, apply, false).unwrap();
    evaluate_exact(&mut app);
    let room = 9_400.0 * 5_400.0 * 200.0;
    assert!(
        (exact_volume(&app, "floor") - room).abs() < 1.0,
        "the room's floor"
    );

    // Asked again for the walls, the same name is rewritten where it stands and the
    // floor becomes the walls' ring with the room cut out of it.
    let steps = app.undo_step_count();
    let before = source(&app);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name": "ROOM", "point_mm": document([150.0, 3_000.0]), "layers": ["walls"]}),
    );
    assert_eq!(app.undo_step_count(), steps + 1, "one Undo step");
    let snapshot = app.document.current();
    assert!(snapshot.definitions().any(|d| d.name() == "ROOM_sketch"));
    let rings = snapshot.local_occurrences().count();
    assert_eq!(rings, 2, "native outline and hole, not just program data");
    let rewritten = source(&app);
    assert_eq!(
        rewritten.matches("# underlay data ROOM:").count(),
        1,
        "{rewritten}"
    );
    assert!(
        rewritten.ends_with(FLOOR),
        "the program around it is kept: {rewritten}"
    );
    assert!(
        !rewritten.contains("ROOM_HOLES = [\n]"),
        "the walls hold the room"
    );
    evaluate_exact(&mut app);
    let walls = (10_000.0 * 6_000.0 - 9_400.0 * 5_400.0) * 200.0;
    assert!(
        (exact_volume(&app, "floor") - walls).abs() < 1.0,
        "the walls' ring"
    );

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(source(&app), before, "Undo brings the room back");

    // A closed object: the hatch over the room, by its id.
    let hatches = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"object_kind": "HATCH"})),
            false,
        )
        .unwrap();
    let hatch = hatches["objects"][0]["id"].clone();
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name": "HATCHED", "object_id": hatch}),
    );
    let both = source(&app);
    assert!(
        both.starts_with("# underlay data HATCHED: object "),
        "{both}"
    );
    assert!(
        both.contains("# underlay data ROOM:"),
        "the room data stays: {both}"
    );
}

#[test]
fn an_explicit_gap_closure_makes_a_profile_without_changing_the_drawing() {
    let drawing = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\nboundary\n90\n4\n70\n0\n10\n2400\n20\n0\n10\n0\n20\n0\n10\n0\n20\n1250\n10\n2400\n20\n1250\n0\nENDSEC\n0\nEOF\n";
    let (mut app, mut bridge, offset) = imported(drawing);
    let point = [1200.0 - offset[0], 600.0 - offset[1]];
    let closure = json!([[
        [2400.0 - offset[0], -offset[1]],
        [2400.0 - offset[0], 1250.0 - offset[1]]
    ]]);
    let open = bridge
        .execute(
            &mut app,
            ask("underlay_regions", json!({"point_mm":point})),
            false,
        )
        .unwrap();
    assert_eq!(open["matched"], 0);
    let region = bridge
        .execute(
            &mut app,
            ask(
                "underlay_regions",
                json!({"point_mm":point, "closing_segments_mm":closure}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(region["matched"], 1);
    assert_eq!(region["regions"][0]["area_m2"], 3.0);
    assert_eq!(region["closing_segments_mm"], closure);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":point, "closing_segments_mm":closure}),
    );
    let data = source(&app);
    let expected = app.live_bridge_stamp();
    bridge
        .execute(
            &mut app,
            Request::ApplyProgram {
                expected: Some(expected),
                source: format!("{data}{FLOOR}"),
                overrides: None,
                file_name: None,
                replace_document: false,
            },
            false,
        )
        .unwrap();
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "floor") - 600_000_000.0).abs() < 1.0);
    let still_open = bridge
        .execute(
            &mut app,
            ask("underlay_regions", json!({"point_mm":point})),
            false,
        )
        .unwrap();
    assert_eq!(
        still_open["matched"], 0,
        "helper segment must not edit the drawing"
    );
}

#[test]
fn array_block_conversion_keeps_affine_geometry_and_updates_the_shared_definition() {
    let drawing = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
        "0\nSECTION\n2\nBLOCKS\n",
        "0\nBLOCK\n8\n0\n2\nleaf\n70\n0\n10\n2\n20\n3\n",
        "0\nLINE\n5\nA1\n8\n0\n10\n2\n20\n3\n11\n12\n21\n8\n0\nENDBLK\n",
        "0\nBLOCK\n8\n0\n2\narray\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n5\nB1\n8\n0\n2\nleaf\n10\n10\n20\n20\n50\n30\n",
        "70\n2\n71\n2\n44\n40\n45\n60\n0\nENDBLK\n0\nENDSEC\n",
        "0\nSECTION\n2\nENTITIES\n",
        "0\nINSERT\n5\nC1\n8\ncopies\n2\narray\n10\n1000\n20\n2000\n",
        "41\n-2\n42\n3\n50\n90\n",
        "0\nINSERT\n5\nC2\n8\ncopies\n2\narray\n10\n2000\n20\n1000\n",
        "0\nENDSEC\n0\nEOF\n"
    );
    let (mut app, mut bridge, offset) = imported(drawing);
    // The first component is added to an already bound, data-only program.
    let expected_stamp = app.live_bridge_stamp();
    bridge
        .execute(
            &mut app,
            Request::ApplyProgram {
                expected: Some(expected_stamp),
                source: "kept = 7\n".to_owned(),
                overrides: None,
                file_name: None,
                replace_document: false,
            },
            false,
        )
        .unwrap();
    let steps = app.undo_step_count();
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"COPIES", "block":"leaf"}),
    );
    assert_eq!(app.undo_step_count(), steps + 1);
    let (sin, cos) = 30.0_f64.to_radians().sin_cos();
    let mut expected = Vec::new();
    for outer in 0..2 {
        for row in 0..2 {
            for col in 0..2 {
                let p = [
                    10.0 + f64::from(col) * 40.0 * cos - f64::from(row) * 60.0 * sin,
                    20.0 + f64::from(col) * 40.0 * sin + f64::from(row) * 60.0 * cos,
                ];
                let q = [p[0] + 10.0 * cos - 5.0 * sin, p[1] + 10.0 * sin + 5.0 * cos];
                let world = |p: [f64; 2]| {
                    if outer == 0 {
                        [
                            1000.0 - 3.0 * p[1] - offset[0],
                            2000.0 - 2.0 * p[0] - offset[1],
                            0.0,
                        ]
                    } else {
                        [2000.0 + p[0] - offset[0], 1000.0 + p[1] - offset[1], 0.0]
                    }
                };
                expected.push([world(p), world(q)]);
            }
        }
    }
    assert_profile_lines(&app, &expected);
    let snapshot = app.document.current();
    assert_eq!(snapshot.features().count(), 1, "one shared profile");
    assert_eq!(snapshot.occurrences().count(), 8);
    let shared = snapshot.occurrences().next().unwrap().definition_id();
    assert!(
        snapshot
            .occurrences()
            .all(|item| item.definition_id() == shared)
    );
    assert!(source(&app).ends_with("kept = 7\n"));
    // Replacing the named source section does not append duplicate instances.
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"COPIES", "block":"leaf"}),
    );
    assert_eq!(app.document.current().occurrences().count(), 8);
    assert_profile_lines(&app, &expected);
    let steps = app.undo_step_count();
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"COPIES", "block":"array"}),
    );
    assert_eq!(app.undo_step_count(), steps + 1);
    assert_eq!(app.document.current().occurrences().count(), 2);
    assert_eq!(app.document.current().features().count(), 4);
    assert_profile_lines(&app, &expected);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current().occurrences().count(), 8);
    assert_eq!(app.document.current().features().count(), 1);
    assert_profile_lines(&app, &expected);
}

#[test]
fn block_linework_becomes_shared_native_instances_and_a_wrong_question_changes_nothing() {
    let (mut app, mut bridge, offset) = imported(DRAWING);
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name": "DOORS", "block": "door_80"}),
    );
    let snapshot = app.document.current();
    assert_eq!(snapshot.underlays().count(), 1);
    assert_eq!(
        snapshot.features().count(),
        1,
        "one shared profile, no solid"
    );
    assert!(
        snapshot
            .features()
            .all(|feature| matches!(feature.kind(), FeatureKind::Profile { closed: false, .. }))
    );
    let instances = snapshot.occurrences().collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].definition_id(), instances[1].definition_id());
    let expected = [
        [
            [1000.0 - offset[0], -offset[1], 0.0],
            [1000.0 - offset[0], 800.0 - offset[1], 0.0],
        ],
        [
            [5000.0 - offset[0], -offset[1], 0.0],
            [5800.0 - offset[0], -offset[1], 0.0],
        ],
    ];
    assert_profile_lines(&app, &expected);
    assert!(
        source(&app).contains("D-01"),
        "attribute provenance retained"
    );
    let published = source(&app);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current().features().count(), 0);
    assert_eq!(app.document.current().underlays().count(), 1);
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(source(&app), published);
    assert_profile_lines(&app, &expected);

    let digest = app.canonical_digest();
    let steps = app.undo_step_count();
    for (arguments, field) in [
        (json!({"name": "1DOORS", "block": "door_80"}), "name"),
        (json!({"name": "DOORS", "block": "WINDOW"}), "block"),
        (
            json!({"name": "DOORS", "block": "door_80", "point_mm": [0, 0]}),
            "point_mm",
        ),
        (
            json!({"name": "DOORS", "point_mm": [-1e7, -1e7]}),
            "point_mm",
        ),
    ] {
        let mut request = arguments.clone();
        request["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
        let code = bridge
            .execute(&mut app, ask("program_from_underlay", request), false)
            .unwrap_err();
        assert_eq!(code, "invalid_params", "{arguments}");
        let details = take_error_details().unwrap_or_default();
        assert!(
            details.to_string().contains(field),
            "{arguments}: {details}"
        );
    }
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), steps);
}

/// Two rooms of four walls and a partition between them, drawn as lines.
fn two_rooms() -> String {
    let line = |handle: &str, [x, y]: [u32; 2], [u, v]: [u32; 2]| {
        format!("0\nLINE\n5\n{handle}\n8\nwalls\n10\n{x}\n20\n{y}\n11\n{u}\n21\n{v}\n")
    };
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{}{}{}{}{}0\nENDSEC\n0\nEOF\n",
        line("A1", [0, 0], [8000, 0]),
        line("A2", [8000, 0], [8000, 4000]),
        line("A3", [8000, 4000], [0, 4000]),
        line("A4", [0, 4000], [0, 0]),
        line("A5", [4000, 0], [4000, 4000]),
    )
}

/// After an area is pulled, an edit of its drawing that joins two rooms says the solid grew,
/// one that opens the room is refused naming it and how to unlink it, and once unlinked the
/// solid keeps its outline whatever the drawing does (review 2026-10-29 UO-64).
#[test]
fn a_drawing_edit_after_a_pull_names_the_area_it_changes_or_would_lose() {
    let (mut app, mut bridge, offset) = imported(&two_rooms());
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":[2000.0-offset[0], 2000.0-offset[1]], "layers":["walls"], "thickness_mm":200}),
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 4_000.0 * 4_000.0 * 200.0).abs() < 1.0);
    let objects = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"limit": 10})),
            false,
        )
        .unwrap();
    let id = |handle: &str| {
        objects["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["handle"] == handle)
            .unwrap()["id"]
            .clone()
    };
    let (partition, left) = (id("A5"), id("A4"));

    // The partition taken out: the room is the union of both, and the answer says so.
    let answer = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"object_ids": [partition]})),
            false,
        )
        .unwrap();
    assert_eq!(
        answer["areas_changed"],
        json!([{
            "name": "ROOM",
            "was": "area 16.000 m², perimeter 16.000 m, 0 holes",
            "now": "area 32.000 m², perimeter 24.000 m, 0 holes",
        }]),
        "{answer}"
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 8_000.0 * 4_000.0 * 200.0).abs() < 1.0);
    assert!(app.undo());

    // The room opened: refused, naming the area and the way to keep it.
    let digest = app.canonical_digest();
    let steps = app.undo_step_count();
    let code = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"object_ids": [left.clone()]})),
            false,
        )
        .unwrap_err();
    assert_eq!(code, "commit_rejected");
    let details = take_error_details().unwrap_or_default().to_string();
    assert!(details.contains("ROOM follows this drawing"), "{details}");
    assert!(
        details.contains(&app.catalog.text("menu-unlink-pulled-areas")),
        "{details}"
    );
    assert_eq!(
        app.canonical_digest(),
        digest,
        "a refused edit changes nothing"
    );
    assert_eq!(app.undo_step_count(), steps);

    // Unlinked, one Undo step: the slab keeps its outline when the drawing changes.
    assert!(app.has_pulled_areas());
    assert!(app.unlink_pulled_areas());
    assert!(!app.has_pulled_areas());
    assert_eq!(app.undo_step_count(), steps + 1);
    assert!(
        !source(&app).contains(super::underlay_binding::PREFIX),
        "{}",
        source(&app)
    );
    let answer = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"object_ids": [left]})),
            false,
        )
        .unwrap();
    assert!(answer.get("areas_changed").is_none(), "{answer}");
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 4_000.0 * 4_000.0 * 200.0).abs() < 1.0);
}
