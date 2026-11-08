//! `file action=import as_underlay=true revision_of=N` (review 2026-11-08 §5.3): the
//! architect's revision B of the plan traced as underlay N is compared with it entity by
//! entity, by handle, and what changed, was added or removed lies on N as an underlay of
//! its own, on three layers, as one Undo step, wherever N was placed.

use super::underlay_objects::{ask, imported};
use super::*;

/// A national-grid easting: the plan lies far from the origin, as a surveyed one does.
const EAST_MM: f64 = 500_000_000.0;

fn line(handle: &str, from: [f64; 2], to: [f64; 2]) -> String {
    format!(
        "0\nLINE\n5\n{handle}\n100\nAcDbEntity\n8\nwalls\n100\nAcDbLine\n\
         10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
        from[0] + EAST_MM,
        from[1],
        to[0] + EAST_MM,
        to[1]
    )
}

fn drawing(entities: &[String]) -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{}0\nENDSEC\n0\nEOF\n",
        entities.concat()
    )
}

/// Revision A: a room 6 × 4 m.
fn revision_a() -> String {
    drawing(&[
        line("A0", [0.0, 0.0], [6000.0, 0.0]),
        line("A1", [6000.0, 0.0], [6000.0, 4000.0]),
        line("A2", [6000.0, 4000.0], [0.0, 4000.0]),
        line("A3", [0.0, 4000.0], [0.0, 0.0]),
    ])
}

/// Revision B: the east wall moved 300 mm out, the north wall with it, a partition added
/// and the west wall taken out.
fn revision_b() -> String {
    drawing(&[
        line("A0", [0.0, 0.0], [6000.0, 0.0]),
        line("A1", [6300.0, 0.0], [6300.0, 4000.0]),
        line("A2", [6300.0, 4000.0], [0.0, 4000.0]),
        line("B0", [3000.0, 0.0], [3000.0, 4000.0]),
    ])
}

fn call(app: &mut KetchupApp, bridge: &mut LiveBridge, method: &str, arguments: Value) -> Value {
    let mut arguments = arguments;
    arguments["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    bridge
        .execute(app, ask(method, arguments), false)
        .unwrap_or_else(|code| panic!("{method}: {code}: {:?}", take_error_details()))
}

fn import_revision(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    source: &str,
    arguments: Value,
) -> Result<Value, &'static str> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan-rev-b.dxf");
    std::fs::write(&path, source).unwrap();
    let mut arguments = arguments;
    arguments["path"] = json!(path.display().to_string());
    arguments["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    bridge.execute(app, ask("import", arguments), false)
}

/// Where the point `local_mm` of underlay `id`'s drawing lies in the document.
fn in_document(app: &KetchupApp, id: u64, local_mm: [f64; 2]) -> [f64; 2] {
    let snapshot = app.document.current();
    let underlay = snapshot
        .underlay(ketchup_model::underlay::UnderlayId(id))
        .unwrap();
    let [x, y, _] = underlay
        .placement
        .transform_point([local_mm[0], local_mm[1], 0.0]);
    [x, y]
}

#[test]
fn the_architects_revision_lies_on_the_traced_plan_as_its_changes() {
    let (mut app, mut bridge, offset) = imported(&revision_a());
    // The plan was placed on the model: its south-west corner at the origin.
    let at = |[x, y]: [f64; 2]| [x + EAST_MM - offset[0], y - offset[1]];
    call(
        &mut app,
        &mut bridge,
        "fit_underlay",
        json!({"underlay_id": 1, "from_mm": [at([0.0, 0.0]), at([6000.0, 0.0])], "to_mm": [[0.0, 0.0], [6000.0, 0.0]], "scale": false}),
    );
    let steps = app.undo_step_count();
    let answer = import_revision(
        &mut app,
        &mut bridge,
        &revision_b(),
        json!({"as_underlay": true, "revision_of": 1}),
    )
    .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    let comparison = &answer["comparison"];
    let handles = |kind: &str| {
        comparison[kind]
            .as_array()
            .unwrap()
            .iter()
            .map(|entity| entity["handle"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(handles("changed"), ["A1", "A2"], "{answer}");
    assert_eq!(handles("added"), ["B0"]);
    assert_eq!(handles("removed"), ["A3"]);
    assert_eq!(comparison["unchanged"], 1);
    assert_eq!(answer["overlay_underlay_id"], 2);
    assert_eq!(app.undo_step_count(), steps + 1, "one Undo step");

    let snapshot = app.document.current();
    let overlay = snapshot
        .underlay(ketchup_model::underlay::UnderlayId(2))
        .unwrap()
        .clone();
    assert_eq!(overlay.name, "plan · changes in plan-rev-b");
    let layers = overlay
        .layers
        .iter()
        .map(|layer| layer.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(layers, ["Changed", "Added", "Removed"]);
    // The partition, added at x 3000 of the drawing, lies where the plan has x 3000.
    let lines = app.cached_underlay_lines(&overlay).unwrap();
    let added = lines.layers()[1].polylines().collect::<Vec<_>>();
    assert_eq!(added.len(), 1);
    let ends = added[0]
        .iter()
        .map(|&point| in_document(&app, 2, point))
        .collect::<Vec<_>>();
    let on_plan = [[3000.0, 0.0], [3000.0, 4000.0]]
        .map(|[x, y]| in_document(&app, 1, [x + EAST_MM - offset[0], y - offset[1]]));
    for (end, expected) in ends.iter().zip(on_plan) {
        assert!(
            (end[0] - expected[0]).abs() < 0.01 && (end[1] - expected[1]).abs() < 0.01,
            "{end:?} on the plan at {expected:?}"
        );
    }
    assert!(
        (on_plan[0][0] - 3000.0).abs() < 0.01 && on_plan[0][1].abs() < 0.01,
        "the plan was placed with its corner at the origin: {on_plan:?}"
    );
    assert_eq!(
        lines.layers()[2].polylines().count(),
        1,
        "the west wall removed"
    );

    app.dispatch_command(AppCommand::Undo);
    assert_eq!(app.document.current().underlays().count(), 1);
}

#[test]
fn a_revision_without_changes_adds_nothing_and_a_wrong_request_is_refused() {
    let (mut app, mut bridge, _) = imported(&revision_a());
    let steps = app.undo_step_count();
    let same = import_revision(
        &mut app,
        &mut bridge,
        &revision_a(),
        json!({"as_underlay": true, "revision_of": 1}),
    )
    .unwrap();
    assert_eq!(same["overlay_underlay_id"], Value::Null, "{same}");
    assert_eq!(same["comparison"]["unchanged"], 4);
    assert_eq!(app.undo_step_count(), steps);

    let no_underlay = import_revision(
        &mut app,
        &mut bridge,
        &revision_b(),
        json!({"as_underlay": true, "revision_of": 9}),
    );
    assert_eq!(no_underlay, Err("entity_not_found"));
    let as_model = import_revision(
        &mut app,
        &mut bridge,
        &revision_b(),
        json!({"revision_of": 1}),
    );
    assert_eq!(as_model, Err("invalid_params"));
    assert_eq!(app.document.current().underlays().count(), 1);
}
