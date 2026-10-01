use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantInstancePath,
    AssistantInstanceReference,
};
use serde_json::{Value, json};

fn face(instance_path: Value) -> Value {
    json!({
        "instance_path": instance_path,
        "face_origin_local_mm": [0.0, 0.0, 18.0],
        "inward_unit_local": [0.0, 0.0, -1.0],
        "bounds_min_local_mm": [0.0, 0.0, 0.0],
        "bounds_max_local_mm": [100.0, 50.0, 18.0]
    })
}

fn path(root_occurrence_id: u64) -> Value {
    json!({"root_occurrence_id": root_occurrence_id, "steps": []})
}

fn output(operation_index: u32, output: &str) -> Value {
    json!({"operation_index": operation_index, "output": output})
}

fn part() -> Value {
    let line = |id: u64, start: [f64; 2], end: [f64; 2]| json!({"type": "line", "id": id, "start_mm": start, "end_mm": end});
    json!({
        "operation": "create_part",
        "name": "Part",
        "workplane": {"type": "principal", "plane": "xy"},
        "entities": [
            line(1, [0.0, 0.0], [100.0, 0.0]),
            line(2, [100.0, 0.0], [100.0, 50.0]),
            line(3, [100.0, 50.0], [0.0, 50.0]),
            line(4, [0.0, 50.0], [0.0, 0.0])
        ],
        "constraints": [],
        "feature": {"type": "extrusion", "distance_mm": 18.0},
        "translation_mm": [0.0, 0.0, 0.0]
    })
}

fn joint(first: Value, second: Value, count: u32, holes: Value) -> Value {
    json!({
        "operation": "create_pin_joint",
        "name": "Row",
        "first": face(first),
        "second": face(second),
        "first_center_local_mm": [20.0, 20.0, 18.0],
        "row_unit_first_local": [1.0, 0.0, 0.0],
        "count": count,
        "spacing_mm": 25.0,
        "pin": {"diameter_mm": 8.0, "length_mm": 30.0, "hole_clearance_mm": 1.0},
        "holes": holes
    })
}

fn validate(operations: Vec<Value>) -> bool {
    serde_json::from_value::<AssistantCadEditProgram>(json!({ "operations": operations }))
        .is_ok_and(|program| program.validate().is_ok())
}

#[test]
fn one_pin_joint_covers_logical_existing_and_drilled_holes_for_paths_and_outputs() {
    let logical = json!({"type": "logical"});
    let drill = json!({"type": "drill", "first_insertion_mm": 10.0});
    assert!(validate(vec![joint(path(1), path(2), 3, logical.clone())]));
    assert!(validate(vec![joint(path(1), path(2), 3, drill.clone())]));
    let existing = json!({"type": "existing", "pairs": [
        {"first_pocket_feature_id": 11, "second_pocket_feature_id": 21},
        {"first_pocket_feature_id": 12, "second_pocket_feature_id": 22}
    ]});
    assert!(validate(vec![joint(path(1), path(2), 2, existing.clone())]));
    // The same operation reaches parts and pockets created earlier in the batch.
    let from_outputs = json!({"type": "existing", "pairs": [{
        "first_pocket_feature_id": output(0, "body_feature"),
        "second_pocket_feature_id": output(1, "body_feature")
    }]});
    assert!(validate(vec![
        part(),
        part(),
        joint(
            output(0, "occurrence"),
            output(1, "occurrence"),
            1,
            from_outputs
        ),
    ]));

    let parsed: AssistantCadEditProgram = serde_json::from_value(json!({"operations": [
        part(), joint(output(0, "occurrence"), path(2), 1, logical.clone())
    ]}))
    .unwrap();
    let AssistantCadEditOperation::CreatePinJoint { first, second, .. } = &parsed.operations[1]
    else {
        panic!("create_pin_joint parses as CreatePinJoint");
    };
    assert!(matches!(
        first.instance_path,
        AssistantInstanceReference::ProgramOutput(_)
    ));
    assert_eq!(
        second.instance_path,
        AssistantInstanceReference::Existing(AssistantInstancePath {
            root_occurrence_id: 2,
            steps: Vec::new()
        })
    );
}

#[test]
fn pin_joint_rejects_mismatched_pairs_forward_outputs_and_replacing_undrilled_joints() {
    let one_pair = json!({"type": "existing", "pairs": [
        {"first_pocket_feature_id": 11, "second_pocket_feature_id": 21}
    ]});
    let repeated_pocket = json!({"type": "existing", "pairs": [
        {"first_pocket_feature_id": 11, "second_pocket_feature_id": 21},
        {"first_pocket_feature_id": 11, "second_pocket_feature_id": 22}
    ]});
    let zero_pocket = json!({"type": "existing", "pairs": [
        {"first_pocket_feature_id": 0, "second_pocket_feature_id": 21}
    ]});
    assert!(!validate(vec![joint(
        path(1),
        path(2),
        2,
        one_pair.clone()
    )]));
    assert!(!validate(vec![joint(path(1), path(2), 2, repeated_pocket)]));
    assert!(!validate(vec![joint(path(1), path(2), 1, zero_pocket)]));
    assert!(!validate(vec![joint(
        path(1),
        path(1),
        1,
        one_pair.clone()
    )]));
    assert!(!validate(vec![
        joint(
            output(1, "occurrence"),
            path(2),
            1,
            json!({"type": "logical"})
        ),
        part(),
    ]));
    assert!(!validate(vec![
        part(),
        joint(
            output(0, "body_feature"),
            path(2),
            1,
            json!({"type": "logical"})
        ),
    ]));
    for holes in [
        json!({"type": "drill", "first_insertion_mm": 0.0}),
        json!({"type": "drill", "first_insertion_mm": -5.0}),
    ] {
        assert!(!validate(vec![joint(path(1), path(2), 1, holes)]));
    }
    let mut replace_logical = joint(path(1), path(2), 1, json!({"type": "logical"}));
    replace_logical["joint_id"] = json!(4);
    assert!(!validate(vec![replace_logical]));
    let mut replace_drilled = joint(path(1), path(2), 1, json!({"type": "drill"}));
    replace_drilled["joint_id"] = json!(4);
    assert!(validate(vec![replace_drilled]));
    let mut missing_holes = joint(path(1), path(2), 1, Value::Null);
    missing_holes.as_object_mut().unwrap().remove("holes");
    assert!(!validate(vec![missing_holes]));
    for retired in ["create_physical_pin_joint", "create_program_pin_joint"] {
        let mut operation = joint(path(1), path(2), 1, json!({"type": "drill"}));
        operation["operation"] = json!(retired);
        assert!(!validate(vec![operation]));
    }
}

#[test]
fn create_part_holes_and_pockets_need_unique_ids_unit_axes_and_positive_sizes() {
    let with_cuts = |holes: Value, pockets: Value| {
        let mut operation = part();
        operation["holes"] = holes;
        operation["pockets"] = pockets;
        operation
    };
    let hole = |id: &str, inward: [f64; 3], diameter: f64| {
        json!({"id": id, "entry_local_mm": [20.0, 20.0, 0.0], "inward_unit_local": inward,
               "diameter_mm": diameter, "depth_mm": 10.0})
    };
    let pocket = |id: &str, max_z: f64| {
        json!({"id": id, "min_local_mm": [0.0, 0.0, 0.0], "max_local_mm": [20.0, 10.0, max_z],
               "inward_unit_local": [0.0, 0.0, 1.0]})
    };
    let up = [0.0, 0.0, 1.0];
    assert!(validate(vec![with_cuts(
        json!([hole("a", up, 8.0), hole("b", up, 8.0)]),
        json!([pocket("a", 5.0)])
    )]));
    for (holes, pockets) in [
        (json!([hole("a", up, 8.0), hole("a", up, 8.0)]), json!([])),
        (json!([hole("a", [0.0, 0.6, 0.8], 8.0)]), json!([])),
        (json!([hole("a", up, 0.0)]), json!([])),
        (json!([hole(" ", up, 8.0)]), json!([])),
        (json!([]), json!([pocket("a", 5.0), pocket("a", 5.0)])),
        (json!([]), json!([pocket("a", 0.0)])),
    ] {
        assert!(!validate(vec![with_cuts(holes, pockets)]));
    }
}
