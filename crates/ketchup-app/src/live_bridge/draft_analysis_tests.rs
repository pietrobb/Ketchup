//! Draft analysis in `query kind=faces`: without a direction the rows are as
//! before; with one, planes, cylinders and cones get their exact draft and
//! class, and any other face is `not_analyzed` with its reason.

use super::drill_holes::{centroid, edit, face, saved};
use super::program_edit::evaluate_exact;
use super::*;

/// Every face row of the query (all pages) and the first page.
fn faces(app: &mut KetchupApp, bridge: &mut LiveBridge, mut query: Value) -> (Vec<Value>, Value) {
    query["kind"] = json!("faces");
    let mut rows = Vec::new();
    let mut first = None;
    loop {
        let page = bridge
            .execute(
                app,
                Request::Query {
                    expected: None,
                    query: serde_json::from_value(query.clone()).unwrap(),
                },
                false,
            )
            .unwrap();
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        let next = page["next_cursor"].clone();
        first.get_or_insert(page);
        if next.is_null() {
            return (rows, first.unwrap());
        }
        query["cursor"] = next;
    }
}

/// The rows of the newest producer feature (the last edit).
fn newest(rows: &[Value]) -> Vec<&Value> {
    let last = rows
        .iter()
        .filter_map(|row| row["producer_feature_id"].as_u64())
        .max();
    rows.iter()
        .filter(|row| row["producer_feature_id"].as_u64() == last)
        .collect()
}

/// The draft of the row with the same reference as `face`.
fn draft_of(rows: &[Value], face: &Value) -> Value {
    rows.iter()
        .find(|row| row["reference_id"] == face["reference_id"])
        .unwrap_or_else(|| panic!("no row for {face}"))["draft"]
        .clone()
}

fn assert_draft(draft: &Value, surface: &str, degrees: f64, class: &str) {
    assert_eq!(draft["status"], "analyzed", "{draft}");
    assert_eq!(draft["surface"], surface, "{draft}");
    assert_eq!(draft["class"], class, "{draft}");
    for bound in ["min_angle_degrees", "max_angle_degrees"] {
        let angle = draft[bound].as_f64().unwrap();
        assert!((angle - degrees).abs() <= 1e-9, "{draft}");
    }
}

fn draft_faces(faces: &[&Value], neutral: &Value, angle_degrees: f64) -> Request {
    let reference = |face: &Value| face["reference_id"].as_str().unwrap().to_owned();
    edit(AssistantCadEditOperation::DraftFaces {
        definition_id: neutral["definition_id"].as_u64().unwrap(),
        name: "draft".to_owned(),
        target_feature_id: neutral["producer_feature_id"].as_u64().unwrap(),
        face_reference_ids: faces.iter().map(|face| reference(face)).collect(),
        neutral_face_reference_id: reference(neutral),
        angle_degrees,
    })
}

#[test]
fn a_pull_direction_classes_every_plane_of_the_block_and_leaves_rows_without_one_as_before() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let (plain, plain_page) = faces(&mut app, &mut bridge, json!({"limit": 2}));
    assert!(plain_page.get("draft_analysis").is_none(), "{plain_page}");
    assert!(plain.iter().all(|row| row.get("draft").is_none()));

    // A direction of any length is the unit pull; the rows are the same apart from `draft`.
    let (analysed, page) = faces(
        &mut app,
        &mut bridge,
        json!({"limit": 2, "direction": [0.0, 0.0, 2.0]}),
    );
    assert_eq!(page["draft_analysis"]["direction"], json!([0.0, 0.0, 1.0]));
    assert_eq!(page["draft_analysis"]["min_draft_degrees"], 1.0);
    let stripped = analysed
        .iter()
        .cloned()
        .map(|mut row| {
            row.as_object_mut().unwrap().remove("draft").unwrap();
            row
        })
        .collect::<Vec<_>>();
    assert_eq!(stripped, plain);

    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let sides = [
        [-1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 1.0, 0.0],
    ]
    .map(|normal| face(&mut app, &mut bridge, "plane", normal));
    assert_draft(&draft_of(&analysed, &top), "plane", 90.0, "positive");
    assert_draft(&draft_of(&analysed, &bottom), "plane", -90.0, "negative");
    for side in &sides {
        assert_draft(&draft_of(&analysed, side), "plane", 0.0, "vertical");
    }

    // A cursor carries its direction: the next page of another pull is refused.
    let next = page["next_cursor"].clone();
    assert!(next.is_string(), "{page}");
    let other = json!({"kind": "faces", "limit": 2, "direction": [0.0, 0.0, -1.0], "cursor": next});
    let refused = bridge.execute(
        &mut app,
        Request::Query {
            expected: None,
            query: serde_json::from_value(other).unwrap(),
        },
        false,
    );
    assert!(refused.is_err(), "{refused:?}");

    for (angle, minimum, class) in [
        (5.0, None, "positive"),
        (5.0, Some(10.0), "insufficient"),
        (-5.0, None, "negative"),
    ] {
        let sides = sides.iter().collect::<Vec<_>>();
        let applied = bridge
            .execute(&mut app, draft_faces(&sides, &bottom, angle), false)
            .unwrap();
        assert_eq!(applied["published"], true, "{applied}");
        evaluate_exact(&mut app);
        let mut query = json!({"limit": 100, "direction": [0.0, 0.0, 1.0]});
        if let Some(minimum) = minimum {
            query["min_draft_degrees"] = json!(minimum);
        }
        let (rows, _) = faces(&mut app, &mut bridge, query);
        let drafted = newest(&rows)
            .into_iter()
            .filter(|row| row["geometry"]["surface_kind"] == "plane")
            .filter(|row| row["geometry"]["unit_normal"][2].as_f64().unwrap().abs() < 0.5)
            .collect::<Vec<_>>();
        assert_eq!(drafted.len(), 4, "{applied}");
        for row in drafted {
            assert_draft(&row["draft"], "plane", angle, class);
        }
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
    }
}

#[test]
fn a_hole_wall_is_vertical_a_drafted_one_is_a_cone_and_a_torus_is_not_analyzed() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let [x, y, z] = centroid(&top);
    let drilled = bridge
        .execute(
            &mut app,
            edit(AssistantCadEditOperation::DrillHoles {
                definition_id: top["definition_id"].as_u64().unwrap(),
                name: "hole".to_owned(),
                target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
                face_reference_id: top["reference_id"].as_str().unwrap().to_owned(),
                points_local_mm: vec![[x, y, z]],
                diameter_mm: 20.0,
                depth_mm: None,
                through: true,
                thread: None,
            }),
            false,
        )
        .unwrap();
    assert_eq!(drilled["published"], true, "{drilled}");
    evaluate_exact(&mut app);
    let wall = face(&mut app, &mut bridge, "cylinder", [0.0; 3]);
    let along = json!({"limit": 100, "direction": [0.0, 0.0, 1.0]});
    let (rows, _) = faces(&mut app, &mut bridge, along.clone());
    assert_draft(&draft_of(&rows, &wall), "cylinder", 0.0, "vertical");
    let (rows, _) = faces(
        &mut app,
        &mut bridge,
        json!({"limit": 100, "direction": [1.0, 0.0, 0.0]}),
    );
    assert_eq!(
        draft_of(&rows, &wall),
        json!({"status":"not_analyzed","surface":"cylinder","reason":"axis_not_along_direction"})
    );

    // The drafted wall is a cone with one exact angle all round: +5 degrees, as
    // the same draft gives the block's sides.
    let hole_bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let applied = bridge
        .execute(&mut app, draft_faces(&[&wall], &hole_bottom, 5.0), false)
        .unwrap();
    assert_eq!(applied["published"], true, "{applied}");
    evaluate_exact(&mut app);
    let (rows, _) = faces(&mut app, &mut bridge, along.clone());
    let cone = newest(&rows)
        .into_iter()
        .find(|row| row["draft"]["surface"] == "cone")
        .unwrap_or_else(|| panic!("no cone in {rows:?}"));
    assert_draft(&cone["draft"], "cone", 5.0, "positive");
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();

    // A fillet round the hole's top edge is a torus: no draft is claimed for it.
    let circles = json!({"kind": "edges", "limit": 100, "search": "circle"});
    let page = bridge
        .execute(
            &mut app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(circles).unwrap(),
            },
            false,
        )
        .unwrap();
    let edges = page["items"].as_array().unwrap();
    let rim = newest(edges)
        .into_iter()
        .find(|edge| (edge["geometry"]["centroid_mm"][2].as_f64().unwrap() - z).abs() < 1e-6)
        .unwrap_or_else(|| panic!("no top rim in {edges:?}"));
    let filleted = bridge
        .execute(
            &mut app,
            edit(AssistantCadEditOperation::FilletEdges {
                definition_id: rim["definition_id"].as_u64().unwrap(),
                name: "rim".to_owned(),
                target_feature_id: rim["producer_feature_id"].as_u64().unwrap(),
                edge_reference_ids: vec![rim["reference_id"].as_str().unwrap().to_owned()],
                radius_mm: 2.0,
            }),
            false,
        )
        .unwrap();
    assert_eq!(filleted["published"], true, "{filleted}");
    evaluate_exact(&mut app);
    let (rows, _) = faces(&mut app, &mut bridge, along);
    let torus = newest(&rows)
        .into_iter()
        .find(|row| row["geometry"]["surface_kind"] == "other")
        .unwrap_or_else(|| panic!("no torus in {rows:?}"));
    assert_eq!(
        torus["draft"],
        json!({"status":"not_analyzed","surface":"other","reason":"unsupported_surface"})
    );
}

#[test]
fn a_draft_query_refuses_a_zero_pull_a_bad_minimum_and_rows_other_than_faces() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    for query in [
        json!({"kind": "faces", "direction": [0.0, 0.0, 0.0]}),
        json!({"kind": "faces", "direction": [0.0, 0.0, 1.0], "min_draft_degrees": 0.0}),
        json!({"kind": "faces", "direction": [0.0, 0.0, 1.0], "min_draft_degrees": 90.0}),
        json!({"kind": "faces", "min_draft_degrees": 2.0}),
        json!({"kind": "edges", "direction": [0.0, 0.0, 1.0]}),
        json!({"kind": "occurrences", "direction": [0.0, 0.0, 1.0]}),
    ] {
        let result = bridge.execute(
            &mut app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(query.clone()).unwrap(),
            },
            false,
        );
        assert!(result.is_err(), "{query}: {result:?}");
    }
}

#[test]
fn the_draft_analysis_view_colors_each_face_by_its_class_without_touching_the_document() {
    use crate::ViewFlag;
    use crate::app::draft::draft_color;
    use ketchup_application::draft_analysis::DraftClass;

    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let colors = |app: &KetchupApp| {
        let snapshot = app.document.current();
        let definition = snapshot.occurrences().next().unwrap().definition_id();
        let package = app.exact.results.get_render(&snapshot, definition).unwrap();
        app.draft_face_colors(package)
    };
    assert!(
        colors(&app).is_empty(),
        "the view starts without draft colors"
    );
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    app.dispatch_command(AppCommand::View(ViewFlag::DraftAnalysis));
    assert!(app.view_visible(ViewFlag::DraftAnalysis));
    assert_eq!(app.digest, app.catalog.text("digest-draft-analysis-shown"));
    assert_eq!(
        app.live_bridge_stamp().canonical_digest,
        stamp.canonical_digest
    );
    assert_eq!(app.undo_step_count(), undo);
    assert!(saved(&app) == bytes);

    let ordinal = |face: &Value| u32::try_from(face["ordinal"].as_u64().unwrap()).unwrap();
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let sides = [
        [-1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 1.0, 0.0],
    ]
    .map(|normal| face(&mut app, &mut bridge, "plane", normal));
    let plain = colors(&app);
    assert_eq!(plain.len(), 6, "{plain:?}");
    assert_eq!(plain[&ordinal(&top)], draft_color(DraftClass::Positive));
    assert_eq!(plain[&ordinal(&bottom)], draft_color(DraftClass::Negative));
    for side in &sides {
        assert_eq!(plain[&ordinal(side)], draft_color(DraftClass::Vertical));
    }

    // Sides drafted by 5 and by 0.5 degrees: enough draft, and too little.
    for (angle, class) in [(5.0, DraftClass::Positive), (0.5, DraftClass::Insufficient)] {
        let sides = sides.iter().collect::<Vec<_>>();
        let applied = bridge
            .execute(&mut app, draft_faces(&sides, &bottom, angle), false)
            .unwrap();
        assert_eq!(applied["published"], true, "{applied}");
        evaluate_exact(&mut app);
        let (rows, _) = faces(&mut app, &mut bridge, json!({"limit": 100}));
        let drafted = colors(&app);
        let walls = newest(&rows)
            .into_iter()
            .filter(|row| row["geometry"]["unit_normal"][2].as_f64().unwrap().abs() < 0.5)
            .collect::<Vec<_>>();
        assert_eq!(walls.len(), 4);
        for wall in walls {
            assert_eq!(
                drafted[&ordinal(wall)],
                draft_color(class),
                "{angle}: {wall}"
            );
        }
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
        evaluate_exact(&mut app);
    }

    app.dispatch_command(AppCommand::View(ViewFlag::DraftAnalysis));
    assert!(colors(&app).is_empty());
    assert!(saved(&app) == bytes);
}
