//! `drawing action=remove`: a client takes objects out of an imported drawing by the ids
//! `underlay_objects` gave or by a box, one Undo step; the others keep their ids, an object placed by an
//! insert is refused with the insert to remove instead, and Save and Open keep the edit.

use super::underlay_objects::{DRAWING, ask, imported};
use super::*;

fn top_level(bridge: &mut LiveBridge, app: &mut KetchupApp) -> Vec<(u64, String)> {
    let answer = bridge
        .execute(app, ask("underlay_objects", json!({"limit": 100})), false)
        .unwrap();
    answer["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_u64().unwrap(),
                row["handle"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn id_of(objects: &[(u64, String)], handle: &str) -> u64 {
    objects.iter().find(|(_, h)| h == handle).unwrap().0
}

#[test]
fn a_client_removes_objects_of_a_drawing_in_one_undo_step_that_save_and_open_keep() {
    let (mut app, mut bridge, offset) = imported(DRAWING);
    let before = top_level(&mut bridge, &mut app);
    assert_eq!(before.len(), 5, "{before:?}");
    let door = id_of(&before, "1F");
    let wall = id_of(&before, "2A");
    let digest = app.canonical_digest();
    let undo = app.undo_step_count();

    // A line the door's insert placed is refused, naming the insert.
    let child = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"object_id": door})),
            false,
        )
        .unwrap()["object"]["child_ids"][0]
        .clone();
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_remove", json!({"object_ids": [child]})),
                false
            )
            .unwrap_err(),
        "invalid_params"
    );
    let details = take_error_details().unwrap();
    assert_eq!(details["details"]["remove_instead"], door, "{details}");
    assert_eq!(app.canonical_digest(), digest);

    let removed = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"object_ids": [door, wall, door]})),
            false,
        )
        .unwrap();
    assert_eq!(removed["removed"], 2, "{removed}");
    assert_eq!(removed["objects_left"], 6);
    assert_eq!(app.undo_step_count(), undo + 1);
    let after = top_level(&mut bridge, &mut app);
    assert_eq!(after.len(), 3, "{after:?}");
    // The others keep their ids.
    for (id, handle) in &after {
        assert_eq!(id_of(&before, handle), *id);
    }
    let summary = bridge
        .execute(&mut app, ask("underlay_objects", json!({})), false)
        .unwrap();
    assert_eq!(summary["kinds"]["INSERT"], json!([1, 1]), "{summary}");
    assert!(summary["layers"].get("walls").is_none(), "{summary}");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_remove", json!({"object_ids": [wall]})),
                false
            )
            .unwrap_err(),
        "entity_not_found"
    );

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(top_level(&mut bridge, &mut app), before);
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(top_level(&mut bridge, &mut app), after);

    // By a box: what meets it goes, here the second door.
    let near = [5000.0 - offset[0], -offset[1]];
    let around = |reach: f64| {
        json!([
            [near[0] - reach, near[1] - reach],
            [near[0] + reach, near[1] + reach]
        ])
    };
    let boxed = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"within_mm": around(10.0)})),
            false,
        )
        .unwrap();
    assert_eq!(
        boxed["object_ids"],
        json!([id_of(&before, "20")]),
        "{boxed}"
    );
    let after = top_level(&mut bridge, &mut app);
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_remove", json!({"within_mm": around(10.0)})),
                false
            )
            .unwrap_err(),
        "entity_not_found",
        "nothing is left there"
    );
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask(
                    "drawing_remove",
                    json!({"object_ids": [after[0].0], "within_mm": around(1.0)})
                ),
                false
            )
            .unwrap_err(),
        "invalid_params"
    );
    let edited = app.canonical_digest();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.ketchup");
    bridge
        .execute(
            &mut app,
            Request::SaveAs {
                expected: None,
                path: path.display().to_string(),
            },
            false,
        )
        .unwrap();
    drop(bridge);
    drop(app);
    let (mut opened, mut bridge) = setup();
    bridge
        .execute(
            &mut opened,
            Request::Open {
                expected: None,
                path: path.display().to_string(),
            },
            false,
        )
        .unwrap();
    assert_eq!(opened.canonical_digest(), edited);
    assert_eq!(top_level(&mut bridge, &mut opened), after);
}
