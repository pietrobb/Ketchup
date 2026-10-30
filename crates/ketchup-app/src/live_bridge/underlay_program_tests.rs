//! `program action=from_underlay`: what an imported drawing draws becomes data of the
//! window's program, so a part follows the drawing: a floor extruded from the room its
//! walls enclose, rewritten in place when asked again, an object's outline and every
//! placement of a block, each as one Undo step.

use super::program_edit::{evaluate_exact, exact_volume};
use super::underlay_objects::{DRAWING, ask, imported, outline, walled_room};
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

/// Review 2026-10-26 UO-41: in a room stand its name in a stamp frame, a wardrobe and a
/// column. With the layers left to the shown ones the name still names the room, and a
/// point in the room, on the stamp or in the wardrobe takes the room, in the program and in
/// the window alike: only the column, drawn with the walls, is cut through the floor; the
/// frame and the wardrobe, drawn on layers of their own, are not.
#[test]
fn what_stands_in_a_room_on_other_layers_than_its_walls_is_not_cut_through_its_floor() {
    let rectangle =
        |[x0, y0]: [f64; 2], [x1, y1]: [f64; 2]| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let drawing = walled_room().replace(
        "0\nENDSEC\n0\nEOF\n",
        &format!(
            "{}{}{}0\nENDSEC\n0\nEOF\n",
            outline("stamps", &rectangle([1_900.0, 1_850.0], [2_900.0, 2_250.0])),
            outline(
                "furniture",
                &rectangle([5_000.0, 1_000.0], [6_000.0, 1_600.0])
            ),
            outline("walls", &rectangle([7_000.0, 3_000.0], [7_300.0, 3_300.0])),
        ),
    );
    let (mut app, mut bridge, offset) = imported(&drawing);
    let floor = (9_400.0 * 5_400.0 - 300.0 * 300.0) * 200.0;
    let measure = "area 50.670 m², perimeter 29.600 m, 1 holes";
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "label":"Kitchen", "thickness_mm":200}),
    );
    let written = source(&app);
    assert!(written.contains(measure), "{written}");
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - floor).abs() < 1.0);
    let id = app.document.current().underlays().next().unwrap().id;
    for (name, [x, y]) in [
        ("IN_ROOM", [4_000.0, 4_000.0]),
        ("ON_STAMP", [2_700.0, 2_200.0]),
        ("IN_WARDROBE", [5_500.0, 1_300.0]),
    ] {
        let point = [x - offset[0], y - offset[1]];
        from_underlay(
            &mut app,
            &mut bridge,
            json!({"name": name, "point_mm": point}),
        );
        let written = source(&app);
        let heading = written
            .lines()
            .find(|line| line.contains(&format!(" {name}:")))
            .unwrap();
        assert!(heading.contains(measure), "{heading}");
        let (_, holes) = crate::live_bridge::underlay_program::underlay_area_outline(
            &app,
            id,
            Some(point),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(holes.len(), 1, "{name}: the window cuts the column only");
    }
}

/// Review 2026-10-26 UO-42: an underlay imported before Kečup kept its objects cannot tell
/// hatch and dimension lines from walls, so they may cut its regions; finding regions in it
/// and taking a room from it say so and ask for the drawing to be imported again.
#[test]
fn regions_of_an_underlay_imported_before_its_objects_were_kept_warn_to_import_it_again() {
    use ketchup_model::document::ProfileSegment;
    use ketchup_model::underlay::UnderlayLinesBuilder;
    let (mut app, mut bridge, _) = imported(&walled_room());
    let snapshot = app.document.current();
    let underlay = snapshot.underlays().next().unwrap();
    let lines = app.cached_underlay_lines(underlay).unwrap();
    assert!(!lines.objects().is_empty());
    let mut builder = UnderlayLinesBuilder::default();
    for (layer, drawn) in underlay.layers.iter().zip(lines.layers()) {
        for polyline in drawn.polylines() {
            let segments = polyline
                .windows(2)
                .map(|ends| ProfileSegment::Line {
                    start_mm: ends[0],
                    end_mm: ends[1],
                })
                .collect::<Vec<_>>();
            builder.add(&layer.name, &segments);
        }
        for text in drawn.texts() {
            builder.add_text(&layer.name, text.clone());
        }
    }
    let (names, old) = builder.finish();
    let present = underlay
        .layers
        .iter()
        .zip(lines.layers())
        .filter(|(_, drawn)| drawn.polylines().next().is_some() || !drawn.texts().is_empty())
        .map(|(layer, _)| layer.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, present);
    assert!(old.objects().is_empty());
    let key = underlay.lines_key();
    drop(snapshot);
    app.render
        .underlay_lines
        .borrow_mut()
        .insert(key, std::sync::Arc::new(old));
    let mut regions = json!({});
    regions["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    let answer = bridge
        .execute(&mut app, ask("underlay_regions", regions), false)
        .unwrap();
    let warnings = answer["warnings"].to_string();
    assert!(warnings.contains("import it again"), "{answer}");
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "label":"Kitchen", "thickness_mm":200}),
    );
    assert!(source(&app).contains("import it again"), "{}", source(&app));
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
    assert_eq!(
        data.matches("), (").count(),
        2 * 3,
        "four corners, in ROOM and in ROOM_POINTS: {data}"
    );

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

/// What was made of a pulled area after it was pulled, a part renamed or a distance changed
/// by hand or by the window, stays when the area follows its drawing: only its outline is
/// written anew (review 2026-10-26 UO-45).
#[test]
fn a_followed_area_keeps_the_name_and_distance_given_to_its_part() {
    let (mut app, mut bridge, offset) = imported(&two_rooms());
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":[2000.0-offset[0], 2000.0-offset[1]], "layers":["walls"], "thickness_mm":200}),
    );
    let generated = "ROOM_SOLID = extrude(\"ROOM\", profile = ROOM, distance = 200)";
    let edited = "ROOM_SOLID = extrude(\"SLAB\", profile = ROOM, distance = 300)";
    let mut program = app.document.current_rule_program().unwrap().clone();
    assert!(program.source.contains(generated), "{}", program.source);
    program.source = program.source.replace(generated, edited);
    app.apply_program_source(program, false).unwrap();
    let id = app.document.current().underlays().next().unwrap().id;
    app.fit_underlay(
        id,
        [[0.0, 0.0], [1000.0, 0.0]],
        [[500.0, 0.0], [1500.0, 0.0]],
        false,
    )
    .unwrap_or_else(|error| panic!("{error}: {:?}", take_error_details()));
    assert!(source(&app).contains(edited), "{}", source(&app));
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "SLAB") - 4_000.0 * 4_000.0 * 300.0).abs() < 1.0);
}

/// A room taken by its label still follows the drawing after the layer of room names is
/// hidden: the binding keeps the layers its label was read from (review 2026-10-26 UO-44).
#[test]
fn hiding_the_room_names_does_not_lose_a_labelled_room_when_its_drawing_moves() {
    let (mut app, mut bridge, _) = imported(&walled_room());
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "label":"Kitchen", "layers":["walls"], "thickness_mm":200}),
    );
    evaluate_exact(&mut app);
    let volume = exact_volume(&app, "ROOM");
    let snapshot = app.document.current();
    let underlay = snapshot.underlays().next().unwrap();
    let id = underlay.id;
    let rooms = underlay
        .layers
        .iter()
        .find(|layer| layer.name == "rooms")
        .unwrap()
        .tag;
    drop(snapshot);
    assert!(app.set_tag_visibility(rooms, false));
    app.fit_underlay(
        id,
        [[0.0, 0.0], [1000.0, 0.0]],
        [[500.0, 0.0], [1500.0, 0.0]],
        false,
    )
    .unwrap_or_else(|error| panic!("{error}: {:?}", take_error_details()));
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - volume).abs() < 1.0);
}

/// Removing an underlay a section follows is refused naming the section, saying the drawing
/// it follows would be gone, and how to unlink it; once unlinked the underlay goes and the
/// slab stays (review 2026-10-26 UO-43).
#[test]
fn removing_a_followed_underlay_names_the_section_and_how_to_unlink_it() {
    let (mut app, mut bridge, offset) = imported(&two_rooms());
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":[2000.0-offset[0], 2000.0-offset[1]], "layers":["walls"], "thickness_mm":200}),
    );
    let id = app.document.current().underlays().next().unwrap().id.0;
    let digest = app.canonical_digest();
    let remove = |app: &KetchupApp| {
        let mut arguments = json!({"underlay_id": id});
        arguments["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
        ask("remove_underlay", arguments)
    };
    let request = remove(&app);
    let code = bridge.execute(&mut app, request, false).unwrap_err();
    assert_eq!(code, "commit_rejected");
    let details = take_error_details().unwrap_or_default().to_string();
    assert!(details.contains("ROOM follows this drawing"), "{details}");
    assert!(details.contains("removes the drawing"), "{details}");
    assert!(
        details.contains(&app.catalog.text("menu-unlink-pulled-areas")),
        "{details}"
    );
    assert_eq!(
        app.canonical_digest(),
        digest,
        "a refused removal changes nothing"
    );

    assert!(app.unlink_pulled_areas());
    let request = remove(&app);
    bridge.execute(&mut app, request, false).unwrap();
    assert_eq!(app.document.current().underlays().count(), 0);
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "ROOM") - 4_000.0 * 4_000.0 * 200.0).abs() < 1.0);
}

/// Code that reads the outline as points reads `NAME_POINTS`: when an edit of the drawing
/// reveals an arc and `NAME` becomes named segments, `NAME_POINTS` still gives the corners
/// and the edit is kept (review 2026-10-29 UO-65).
#[test]
fn name_points_stay_points_when_an_edit_reveals_an_arc() {
    let (mut app, mut bridge, offset) = imported(&two_rooms());
    from_underlay(
        &mut app,
        &mut bridge,
        json!({"name":"ROOM", "point_mm":[2000.0-offset[0], 2000.0-offset[1]], "layers":["walls"]}),
    );
    // Its box: code that takes each item of the outline as a point.
    let pad = "
xs = [x for x, y in ROOM_POINTS]
ys = [y for x, y in ROOM_POINTS]
pad = extrude(\"pad\", profile = [(min(xs), min(ys)), (max(xs), min(ys)), (max(xs), max(ys)), (min(xs), max(ys))], distance = 100)
";
    let apply = Request::ApplyProgram {
        expected: Some(app.live_bridge_stamp()),
        source: format!("{}{pad}", source(&app)),
        overrides: None,
        file_name: None,
        replace_document: false,
    };
    bridge.execute(&mut app, apply, false).unwrap();
    assert!(
        source(&app).contains("ROOM_POINTS = [\n    ("),
        "{}",
        source(&app)
    );
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "pad") - 4_000.0 * 4_000.0 * 100.0).abs() < 1.0);

    // The left wall becomes a half circle: ROOM is named segments, one of them an arc.
    let objects = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"limit": 10})),
            false,
        )
        .unwrap();
    let left = objects["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["handle"] == "A4")
        .unwrap()["id"]
        .clone();
    bridge
        .execute(
            &mut app,
            ask(
                "drawing_draw",
                json!({"layer": "walls", "shapes": [{"shape": "arc", "center_mm": [-offset[0], 2000.0 - offset[1]], "radius_mm": 2000.0, "start_deg": 90.0, "end_deg": 270.0}]}),
            ),
            false,
        )
        .unwrap();
    bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"object_ids": [left]})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    let written = source(&app);
    assert!(written.contains("{\"through\": "), "{written}");
    assert!(written.ends_with(pad), "{written}");
    evaluate_exact(&mut app);
    // The half circle reaches 2 m left of the old wall.
    let volume = exact_volume(&app, "pad");
    let expected = 6_000.0 * 4_000.0 * 100.0;
    assert!(
        (volume - expected).abs() < 0.005 * expected,
        "{volume} != {expected}"
    );
}
