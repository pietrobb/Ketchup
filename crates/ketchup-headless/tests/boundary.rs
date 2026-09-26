use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn exchange(lines: &[String]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ketchup-headless"))
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for line in lines {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn request(id: u64, method: &str, params: Value) -> String {
    json!({"protocol":"ketchup.headless.v1","id":id,"method":method,"params":params}).to_string()
}

#[test]
fn program_source_survives_native_save_open_and_follows_document_undo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rule.ketchup");
    let path = path.to_str().unwrap();
    let source = "W = param(\"width\", 100)\nbox(\"part\", [W, 20, 10])";
    let responses = exchange(&[
        request(
            1,
            "program_apply",
            json!({"source":source,"params":{"width":120}}),
        ),
        request(2, "program_source", json!({})),
        request(3, "save", json!({"path":path})),
        request(4, "open", json!({"path":path})),
        request(5, "program_source", json!({})),
        request(6, "undo", json!({})),
        request(7, "program_source", json!({})),
        request(8, "redo", json!({})),
        request(9, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let expected = json!({"file_name":"program.star","source":source,"overrides":{"width":120.0}});
    assert_eq!(responses[1]["result"]["source"], expected);
    assert_eq!(responses[4]["result"]["source"], expected);
    assert!(responses[6]["result"]["source"].is_null());
    assert_eq!(responses[8]["result"]["source"], expected);
}

#[test]
fn canonical_program_history_keeps_ten_changes_across_save_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bounded.ketchup");
    let mut requests = (0..=15)
        .map(|version| {
            request(
                version + 1,
                "program_apply",
                json!({"source":format!("# version {version}\nbox(\"part\", [100, 20, 10])")}),
            )
        })
        .collect::<Vec<_>>();
    requests.push(request(17, "save", json!({"path":path.to_str().unwrap()})));
    requests.push(request(18, "open", json!({"path":path.to_str().unwrap()})));
    for id in 19..=28 {
        requests.push(request(id, "undo", json!({})));
    }
    requests.push(request(29, "program_source", json!({})));
    requests.push(request(30, "redo", json!({})));
    requests.push(request(31, "program_source", json!({})));
    let responses = exchange(&requests);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(responses[15]["result"]["state"]["undo_steps"], 10);
    assert_eq!(responses[17]["result"]["state"]["undo_steps"], 10);
    assert_eq!(responses[27]["result"]["state"]["undo_steps"], 0);
    assert_eq!(
        responses[28]["result"]["source"]["source"],
        "# version 5\nbox(\"part\", [100, 20, 10])"
    );
    assert_eq!(
        responses[30]["result"]["source"]["source"],
        "# version 6\nbox(\"part\", [100, 20, 10])"
    );
}

#[test]
fn canonical_program_history_bounds_geometry_edits_too() {
    let source = "W = param(\"width\", 100)\nbox(\"part\", [W, 20, 10])";
    let requests = (0..=12)
        .map(|step| {
            request(
                step + 1,
                "program_apply",
                json!({"source":source,"params":{"width":100 + step}}),
            )
        })
        .collect::<Vec<_>>();
    let responses = exchange(&requests);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(responses[12]["result"]["state"]["undo_steps"], 10);
    assert_eq!(
        responses[12]["result"]["state"]["occurrences"],
        responses[0]["result"]["state"]["occurrences"]
    );
    assert_ne!(
        responses[12]["result"]["state"]["canonical_digest"],
        responses[0]["result"]["state"]["canonical_digest"]
    );
}

#[test]
fn reapplying_equivalent_program_preserves_document_geometry_and_undo_history() {
    let first = "box(\"part\", [100, 20, 10])";
    let edited = "# harmless source edit\nbox(\"part\", [100, 20, 10])";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source": first})),
        request(2, "program_apply", json!({"source": edited})),
        request(3, "program_source", json!({})),
        request(4, "undo", json!({})),
        request(5, "program_source", json!({})),
        request(6, "redo", json!({})),
        request(7, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let after = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], after["document_id"]);
    assert_eq!(before["canonical_digest"], after["canonical_digest"]);
    assert_ne!(before["revision"], after["revision"]);
    assert_eq!(responses[2]["result"]["source"]["source"], edited);
    assert_eq!(responses[4]["result"]["source"]["source"], first);
    assert_eq!(responses[6]["result"]["source"]["source"], edited);
}

#[test]
fn moving_a_program_part_keeps_other_parts_and_one_shared_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moved.ketchup");
    let source = "X = param(\"x\", 10)\nbox(\"fixed\", [20, 20, 20])\nbox(\"moving\", [10, 10, 10], at=[X, 30, 0])";
    let responses = exchange(&[
        request(
            1,
            "program_apply",
            json!({"source":source,"params":{"x":10}}),
        ),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"x":45}}),
        ),
        request(3, "program_source", json!({})),
        request(4, "undo", json!({})),
        request(5, "program_source", json!({})),
        request(6, "redo", json!({})),
        request(7, "program_source", json!({})),
        request(8, "save", json!({"path":path.to_str().unwrap()})),
        request(9, "open", json!({"path":path.to_str().unwrap()})),
        request(10, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let moved = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], moved["document_id"]);
    assert_eq!(before["definitions"], moved["definitions"]);
    assert_eq!(before["features"], moved["features"]);
    let old = before["occurrences"].as_array().unwrap();
    let new = moved["occurrences"].as_array().unwrap();
    assert_eq!(old.len(), 2);
    assert_eq!(
        old.iter().map(|item| &item["id"]).collect::<Vec<_>>(),
        new.iter().map(|item| &item["id"]).collect::<Vec<_>>()
    );
    assert_eq!(old[0], new[0]);
    assert_ne!(old[1]["transform"], new[1]["transform"]);
    assert_eq!(responses[2]["result"]["source"]["overrides"]["x"], 45.0);
    assert_eq!(
        responses[3]["result"]["state"]["occurrences"],
        before["occurrences"]
    );
    assert_eq!(responses[4]["result"]["source"]["overrides"]["x"], 10.0);
    assert_eq!(
        responses[5]["result"]["state"]["occurrences"],
        moved["occurrences"]
    );
    assert_eq!(responses[6]["result"]["source"]["overrides"]["x"], 45.0);
    assert_eq!(
        responses[8]["result"]["state"]["occurrences"],
        moved["occurrences"]
    );
    assert_eq!(responses[9]["result"]["source"]["overrides"]["x"], 45.0);
}

#[test]
fn resizing_a_program_part_preserves_identity_and_shared_undo() {
    let source = "W = param(\"width\", 100)\nbox(\"part\", [W, 20, 10])";
    let responses = exchange(&[
        request(
            1,
            "program_apply",
            json!({"source":source,"params":{"width":100}}),
        ),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"width":140},"discard_unsaved":true}),
        ),
        request(3, "program_source", json!({})),
        request(4, "undo", json!({})),
        request(5, "program_source", json!({})),
        request(6, "redo", json!({})),
        request(
            7,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":1,"steps":[]}]}),
        ),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let after = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], after["document_id"]);
    assert_eq!(before["occurrences"], after["occurrences"]);
    assert_eq!(before["definitions"], after["definitions"]);
    assert_eq!(before["features"][0]["id"], after["features"][0]["id"]);
    assert_ne!(before["canonical_digest"], after["canonical_digest"]);
    assert_eq!(
        responses[2]["result"]["source"]["overrides"]["width"],
        140.0
    );
    assert_eq!(
        responses[3]["result"]["state"]["canonical_digest"],
        before["canonical_digest"]
    );
    assert_eq!(
        responses[4]["result"]["source"]["overrides"]["width"],
        100.0
    );
    assert_eq!(
        responses[5]["result"]["state"]["canonical_digest"],
        after["canonical_digest"]
    );
    let features = responses[6]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    assert!(
        features
            .iter()
            .flat_map(|feature| feature["parameters"].as_array().unwrap())
            .any(|parameter| {
                parameter["path"] == "bounds.width" && parameter["value"] == 140.0
            })
    );
}

#[test]
fn changing_all_dimensions_keeps_the_other_part_and_survives_save_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resized.ketchup");
    let source = "W = param(\"w\", 100)\nD = param(\"d\", 40)\nT = param(\"t\", 12)\nbox(\"fixed\", [30, 30, 30])\nbox(\"resized\", [W, D, T], at=[0, 50, 0])";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"w":130,"d":55,"t":18}}),
        ),
        request(
            3,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":2,"steps":[]}]}),
        ),
        request(4, "save", json!({"path":path.to_str().unwrap()})),
        request(5, "open", json!({"path":path.to_str().unwrap()})),
        request(6, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let after = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], after["document_id"]);
    assert_eq!(before["occurrences"], after["occurrences"]);
    assert_eq!(before["definitions"], after["definitions"]);
    let features = responses[2]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    let params = features
        .iter()
        .flat_map(|f| f["parameters"].as_array().unwrap())
        .collect::<Vec<_>>();
    for (path, value) in [
        ("bounds.width", 130.0),
        ("bounds.height", 55.0),
        ("extent.distance", 18.0),
    ] {
        assert!(
            params
                .iter()
                .any(|p| p["path"] == path && p["value"] == value),
            "{path}: {params:?}"
        );
    }
    assert_eq!(
        after["canonical_digest"],
        responses[4]["result"]["state"]["canonical_digest"]
    );
    assert_eq!(
        responses[5]["result"]["source"]["overrides"],
        json!({"w":130.0,"d":55.0,"t":18.0})
    );
}

#[test]
fn adding_a_named_part_preserves_existing_ids_and_shared_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("added.ketchup");
    let first = "box(\"fixed\", [100, 20, 10])\nbox(\"moving\", [20, 20, 10], at=[0, 50, 0])";
    let added = format!("{first}\nbox(\"new\", [30, 40, 18], at=[0, 100, 0])");
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":first})),
        request(2, "program_apply", json!({"source":added})),
        request(3, "undo", json!({})),
        request(4, "program_source", json!({})),
        request(5, "redo", json!({})),
        request(6, "save", json!({"path":path.to_str().unwrap()})),
        request(7, "open", json!({"path":path.to_str().unwrap()})),
        request(8, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let after = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], after["document_id"]);
    let old = before["occurrences"].as_array().unwrap();
    let new = after["occurrences"].as_array().unwrap();
    assert_eq!(new.len(), 3);
    assert_eq!(&new[..2], old);
    assert_eq!(new[2]["name"], "new");
    assert_eq!(
        responses[2]["result"]["state"]["occurrences"],
        before["occurrences"]
    );
    assert_eq!(responses[3]["result"]["source"]["source"], first);
    assert_eq!(
        responses[4]["result"]["state"]["occurrences"],
        after["occurrences"]
    );
    assert_eq!(
        responses[6]["result"]["state"]["occurrences"],
        after["occurrences"]
    );
    assert_eq!(responses[7]["result"]["source"]["source"], added);
}

#[test]
fn removing_a_named_part_preserves_remaining_ids_and_shared_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("removed.ketchup");
    let first = "box(\"keep\", [100, 20, 10])\nbox(\"remove\", [20, 20, 10], at=[0, 50, 0])";
    let remaining = "box(\"keep\", [100, 20, 10])";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":first})),
        request(2, "program_apply", json!({"source":remaining})),
        request(3, "undo", json!({})),
        request(4, "program_source", json!({})),
        request(5, "redo", json!({})),
        request(6, "save", json!({"path":path.to_str().unwrap()})),
        request(7, "open", json!({"path":path.to_str().unwrap()})),
        request(8, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let after = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], after["document_id"]);
    assert_eq!(after["occurrences"], json!([before["occurrences"][0]]));
    assert_eq!(after["definitions"], json!([before["definitions"][0]]));
    assert_eq!(
        responses[2]["result"]["state"]["occurrences"],
        before["occurrences"]
    );
    assert_eq!(responses[3]["result"]["source"]["source"], first);
    assert_eq!(
        responses[4]["result"]["state"]["occurrences"],
        after["occurrences"]
    );
    assert_eq!(
        responses[6]["result"]["state"]["occurrences"],
        after["occurrences"]
    );
    assert_eq!(responses[7]["result"]["source"]["source"], remaining);
}

#[test]
fn replacing_one_part_and_moving_another_is_one_program_revision() {
    let first = "box(\"keep\", [100, 20, 10])\nbox(\"old\", [20, 20, 10], at=[0, 50, 0])";
    let next =
        "box(\"keep\", [100, 20, 10], at=[10, 0, 0])\nbox(\"new\", [30, 40, 18], at=[0, 100, 0])";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":first})),
        request(2, "program_apply", json!({"source":next})),
        request(3, "undo", json!({})),
        request(4, "program_source", json!({})),
        request(5, "redo", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    let before = &responses[0]["result"]["state"];
    let after = &responses[1]["result"]["state"];
    assert_eq!(before["document_id"], after["document_id"]);
    assert_eq!(
        before["occurrences"][0]["id"],
        after["occurrences"][0]["id"]
    );
    assert_ne!(
        before["occurrences"][0]["transform"],
        after["occurrences"][0]["transform"]
    );
    assert_eq!(after["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(after["occurrences"][1]["name"], "new");
    assert_eq!(
        responses[2]["result"]["state"]["occurrences"],
        before["occurrences"]
    );
    assert_eq!(responses[3]["result"]["source"]["source"], first);
    assert_eq!(
        responses[4]["result"]["state"]["occurrences"],
        after["occurrences"]
    );
}

#[test]
fn editing_hole_dimensions_preserves_part_identity_and_shared_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edited-hole.ketchup");
    let source = "R = param(\"diameter\", 8)\nD = param(\"depth\", 10)\np = box(\"part\", [100, 60, 18])\nhole(p, \"z+\", at=(20, 20), diameter=R, depth=D)";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"diameter":12,"depth":14}}),
        ),
        request(
            3,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":1,"steps":[]}]}),
        ),
        request(4, "save", json!({"path":path.to_str().unwrap()})),
        request(5, "open", json!({"path":path.to_str().unwrap()})),
        request(6, "program_source", json!({})),
        request(7, "undo", json!({})),
        request(8, "program_source", json!({})),
        request(9, "redo", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(
        responses[0]["result"]["state"]["document_id"],
        responses[1]["result"]["state"]["document_id"]
    );
    assert_eq!(
        responses[0]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
    let features = responses[2]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    let params = features
        .iter()
        .flat_map(|feature| feature["parameters"].as_array().unwrap())
        .collect::<Vec<_>>();
    assert!(
        params
            .iter()
            .any(|p| p["path"] == "entities.1.radius" && p["value"] == 6.0),
        "{params:?}"
    );
    assert!(
        params
            .iter()
            .any(|p| p["path"] == "depth" && p["value"] == 14.0),
        "{params:?}"
    );
    assert_eq!(
        responses[5]["result"]["source"]["overrides"],
        json!({"diameter":12.0,"depth":14.0})
    );
    assert_eq!(responses[7]["result"]["source"]["overrides"], json!({}));
    assert_eq!(
        responses[8]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
}

#[test]
fn moving_a_hole_preserves_part_identity_and_shared_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moved-hole.ketchup");
    let source = "p = box(\"part\", [100, 60, 18])\nhole(p, \"z+\", at=(20, 20), diameter=8, depth=10)\nhole(p, \"z+\", at=(60, 20), diameter=8, depth=10)\nhole(p, \"x+\", at=(30, 9), diameter=8, depth=10)";
    let moved = "p = box(\"part\", [100, 60, 18])\nhole(p, \"z+\", at=(25, 30), diameter=8, depth=10)\nhole(p, \"z+\", at=(60, 20), diameter=8, depth=10)\nhole(p, \"x+\", at=(32, 10), diameter=8, depth=10)";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(2, "program_apply", json!({"source":moved})),
        request(
            3,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":1,"steps":[]}]}),
        ),
        request(4, "save", json!({"path":path.to_str().unwrap()})),
        request(5, "open", json!({"path":path.to_str().unwrap()})),
        request(6, "program_source", json!({})),
        request(7, "undo", json!({})),
        request(8, "program_source", json!({})),
        request(9, "redo", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(
        responses[0]["result"]["state"]["document_id"],
        responses[1]["result"]["state"]["document_id"]
    );
    assert_eq!(
        responses[0]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
    let features = responses[2]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    let origins = features
        .iter()
        .map(|feature| {
            feature["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| {
                    p["path"]
                        .as_str()
                        .is_some_and(|path| path.starts_with("frame.origin."))
                })
                .map(|p| p["value"].as_f64().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(origins.contains(&vec![25.0, 30.0, 18.0]), "{origins:?}");
    assert!(origins.contains(&vec![60.0, 20.0, 18.0]), "{origins:?}");
    assert!(origins.contains(&vec![100.0, 32.0, 10.0]), "{origins:?}");
    assert_eq!(responses[5]["result"]["source"]["source"], moved);
    assert_eq!(responses[7]["result"]["source"]["source"], source);
    assert_eq!(
        responses[8]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
}

#[test]
fn resizing_a_drilled_part_updates_face_anchors_in_one_shared_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resized-drilled-part.ketchup");
    let source = "W = param(\"width\", 100)\nL = param(\"length\", 60)\nT = param(\"thickness\", 18)\np = box(\"part\", [W, L, T])\nhole(p, \"z+\", at=(20, 20), diameter=8, depth=10)\nhole(p, \"x+\", at=(30, 9), diameter=8, depth=10)\nhole(p, \"y+\", at=(40, 9), diameter=8, depth=10)";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"width":140,"length":80,"thickness":24}}),
        ),
        request(
            3,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":1,"steps":[]}]}),
        ),
        request(4, "save", json!({"path":path.to_str().unwrap()})),
        request(5, "open", json!({"path":path.to_str().unwrap()})),
        request(6, "undo", json!({})),
        request(7, "program_source", json!({})),
        request(8, "redo", json!({})),
        request(9, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(
        responses[0]["result"]["state"]["document_id"],
        responses[1]["result"]["state"]["document_id"]
    );
    assert_eq!(
        responses[0]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
    let features = responses[2]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    let origins = features
        .iter()
        .map(|feature| {
            feature["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| {
                    p["path"]
                        .as_str()
                        .is_some_and(|path| path.starts_with("frame.origin."))
                })
                .map(|p| p["value"].as_f64().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(origins.contains(&vec![20.0, 20.0, 24.0]), "{origins:?}");
    assert!(origins.contains(&vec![140.0, 30.0, 9.0]), "{origins:?}");
    assert!(origins.contains(&vec![40.0, 80.0, 9.0]), "{origins:?}");
    assert_eq!(responses[6]["result"]["source"]["overrides"], json!({}));
    assert_eq!(
        responses[8]["result"]["source"]["overrides"],
        json!({"width":140.0,"length":80.0,"thickness":24.0})
    );
    assert_eq!(
        responses[7]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
}

#[test]
fn editing_pocket_depth_preserves_part_and_shared_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocket-depth.ketchup");
    let source = "D = param(\"depth\", 5)\np = box(\"part\", [100, 60, 18])\npocket(p, \"z+\", rect=(20, 20, 40, 40), depth=D)";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"depth":8}}),
        ),
        request(
            3,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":1,"steps":[]}]}),
        ),
        request(4, "save", json!({"path":path.to_str().unwrap()})),
        request(5, "open", json!({"path":path.to_str().unwrap()})),
        request(6, "undo", json!({})),
        request(7, "program_source", json!({})),
        request(8, "redo", json!({})),
        request(9, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(
        responses[0]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
    let features = responses[2]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    assert!(
        features.iter().any(|feature| feature["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["path"] == "depth" && p["value"] == 8.0)),
        "{features:?}"
    );
    assert_eq!(responses[6]["result"]["source"]["overrides"], json!({}));
    assert_eq!(
        responses[8]["result"]["source"]["overrides"],
        json!({"depth":8.0})
    );
}

#[test]
fn resizing_and_moving_programmed_pockets_keeps_identity_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moved-pockets.ketchup");
    let source = "W = param(\"width\", 100)\nL = param(\"length\", 60)\nT = param(\"thickness\", 18)\nU = param(\"start\", 20)\nS = param(\"span\", 20)\nV = param(\"side\", 4)\np = box(\"part\", [W, L, T])\npocket(p, \"z+\", rect=(U, 20, U+S, 40), depth=5)\npocket(p, \"y+\", rect=(25, V, 45, V+6), depth=3)\nhole(p, \"x+\", at=(30, 9), diameter=8, depth=10)";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(
            2,
            "program_apply",
            json!({"source":source,"params":{"width":140,"length":80,"thickness":24,"start":25,"span":30,"side":6}}),
        ),
        request(
            3,
            "edit_context",
            json!({"targets":[{"root_occurrence_id":1,"steps":[]}]}),
        ),
        request(4, "save", json!({"path":path.to_str().unwrap()})),
        request(5, "open", json!({"path":path.to_str().unwrap()})),
        request(6, "undo", json!({})),
        request(7, "program_source", json!({})),
        request(8, "redo", json!({})),
        request(9, "program_source", json!({})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(
        responses[0]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
    let features = responses[2]["result"]["targets"][0]["features"]
        .as_array()
        .unwrap();
    let origins = features
        .iter()
        .map(|feature| {
            feature["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| {
                    p["path"]
                        .as_str()
                        .is_some_and(|path| path.starts_with("frame.origin."))
                })
                .map(|p| p["value"].as_f64().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(origins.contains(&vec![40.0, 30.0, 24.0]), "{origins:?}");
    assert!(origins.contains(&vec![35.0, 80.0, 9.0]), "{origins:?}");
    assert!(origins.contains(&vec![140.0, 30.0, 9.0]), "{origins:?}");
    let sketch = features
        .iter()
        .find(|feature| {
            feature["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["path"] == "entities.1.start.x" && p["value"] == -15.0)
        })
        .unwrap();
    assert!(
        sketch["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["path"] == "entities.1.end.x" && p["value"] == 15.0)
    );
    assert_eq!(responses[6]["result"]["source"]["overrides"], json!({}));
    assert_eq!(
        responses[8]["result"]["source"]["overrides"],
        json!({"width":140.0,"length":80.0,"thickness":24.0,"start":25.0,"span":30.0,"side":6.0})
    );
    assert_eq!(
        responses[7]["result"]["state"]["occurrences"],
        responses[1]["result"]["state"]["occurrences"]
    );
}

#[test]
fn updated_pocket_evaluates_as_exact_geometry() {
    let source =
        "p = box(\"part\", [100, 60, 18])\npocket(p, \"z+\", rect=(20, 20, 40, 40), depth=5)";
    let changed =
        "p = box(\"part\", [110, 60, 18])\npocket(p, \"z+\", rect=(25, 20, 50, 40), depth=7)";
    let responses = exchange(&[
        request(1, "program_apply", json!({"source":source})),
        request(2, "program_apply", json!({"source":changed})),
        request(3, "evaluate", json!({"timeout_ms":30000})),
    ]);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response:?}");
    }
    assert_eq!(
        responses[2]["result"]["complete"], true,
        "{:?}",
        responses[2]
    );
    assert_eq!(
        responses[2]["result"]["topology_complete"], true,
        "{:?}",
        responses[2]
    );
    let exact = responses[2]["result"]["topology_geometry"]
        .as_array()
        .unwrap()
        .iter()
        .max_by_key(|body| body["feature_id"].as_u64().unwrap())
        .unwrap();
    let volume = exact["native_evidence"]["volume_mm3"].as_f64().unwrap();
    assert!((volume - 115_300.0).abs() < 0.01, "{exact:?}");
    let fresh = exchange(&[
        request(1, "program_apply", json!({"source":changed})),
        request(2, "evaluate", json!({"timeout_ms":30000})),
    ]);
    assert!(
        fresh.iter().all(|response| response.get("error").is_none()),
        "{fresh:?}"
    );
    assert_eq!(fresh[1]["result"]["topology_complete"], true);
    let rebuilt = fresh[1]["result"]["topology_geometry"]
        .as_array()
        .unwrap()
        .iter()
        .max_by_key(|body| body["feature_id"].as_u64().unwrap())
        .unwrap();
    assert_eq!(exact["bounds_mm"], rebuilt["bounds_mm"]);
    assert_eq!(exact["native_evidence"], rebuilt["native_evidence"]);
}

#[test]
fn changing_pocket_face_does_not_replace_document_or_history() {
    let source = "W = param(\"width\", 100)\np = box(\"part\", [W, 60, 18])\npocket(p, \"z+\", rect=(20, 20, 40, 40), depth=5)";
    let responses = exchange(&[
        request(
            1,
            "program_apply",
            json!({"source":source,"params":{"width":100}}),
        ),
        request(
            2,
            "program_apply",
            json!({"source":source.replace("\"z+\"", "\"z-\""),"params":{"width":140},"discard_unsaved":true}),
        ),
        request(3, "state", json!({})),
        request(4, "program_source", json!({})),
    ]);
    assert!(responses[0].get("error").is_none(), "{:?}", responses[0]);
    assert_eq!(
        responses[1]["error"]["code"],
        "program_incremental_unsupported"
    );
    assert_eq!(
        responses[0]["result"]["state"],
        responses[2]["result"]["state"]
    );
    assert_eq!(
        responses[3]["result"]["source"]["overrides"]["width"],
        100.0
    );
}

#[test]
fn live_protocol_rejects_duplicate_permissions_and_stays_synchronized() {
    let responses = exchange(&[
        request(1, "state", json!({})),
        r#"{"protocol":"ketchup.headless.v1","id":2,"method":"save","params":{"overwrite":false,"overwrite":true}}"#.to_owned(),
        r#"{"protocol":"ketchup.headless.v1","id":3,"method":"new","params":{"discard_unsaved":false,"discard_unsaved":true}}"#.to_owned(),
        r#"{"protocol":"ketchup.headless.v1","id":4,"method":"state","method":"new","params":{}}"#.to_owned(),
        request(5, "state", json!({})),
    ]);
    assert_eq!(responses.len(), 5);
    for response in &responses[1..4] {
        assert_eq!(response["error"]["code"], "invalid_json");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("duplicate JSON field")
        );
    }
    assert_eq!(
        responses[0]["result"]["state"],
        responses[4]["result"]["state"]
    );
    assert_eq!(responses[4]["id"], 5);
}

#[test]
fn live_protocol_honors_nondefault_deadline_and_rejects_out_of_range() {
    let responses = exchange(&[
        request(1, "state", json!({})),
        request(2, "evaluate", json!({"timeout_ms":2500})),
        request(3, "evaluate", json!({"timeout_ms":0})),
        request(4, "evaluate", json!({"timeout_ms":300001})),
        request(5, "state", json!({})),
    ]);
    assert_eq!(responses.len(), 5);
    assert!(responses[1].get("error").is_none(), "{:?}", responses[1]);
    assert_eq!(responses[1]["result"]["complete"], false);
    assert!(responses[1]["result"]["not_evaluated"].is_string());
    for response in &responses[2..4] {
        assert_eq!(response["error"]["code"], "invalid_params");
    }
    assert_eq!(
        responses[0]["result"]["state"],
        responses[4]["result"]["state"]
    );
}

#[test]
fn advertised_color_schema_is_nullable_bounded_rgb() {
    let responses = exchange(&[request(1, "capabilities", json!({}))]);
    fn locate(value: &Value) -> Option<&Value> {
        if value.pointer("/properties/operation/const") == Some(&json!("set_color")) {
            return Some(value);
        }
        match value {
            Value::Object(map) => map.values().find_map(locate),
            Value::Array(items) => items.iter().find_map(locate),
            _ => None,
        }
    }
    let operation = locate(&responses[0]).expect("advertised set_color operation");
    let color = &operation["properties"]["color"]["anyOf"];
    assert_eq!(color[0]["minItems"], 3);
    assert_eq!(color[0]["maxItems"], 3);
    assert_eq!(
        color[0]["items"],
        json!({"type":"integer","minimum":0,"maximum":255})
    );
    assert_eq!(color[1], json!({"type":"null"}));
}
