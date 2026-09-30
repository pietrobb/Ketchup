use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use serde_json::json;

fn program(workplane: AssistantWorkplaneSpec) -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1,
            name: "Frame sketch".into(),
            workplane,
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 1.0,
            }],
            constraints: vec![],
        }],
    }
}

#[test]
fn public_frame_schema_is_exact_and_uses_strict_canonical_validation() {
    let value = json!({"type":"frame","origin_mm":[120.0,-40.0,70.0],"x_axis":[0.8,0.6,0.0],"y_axis":[0.0,0.0,1.0]});
    let spec: AssistantWorkplaneSpec = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&spec).unwrap(), value);
    program(spec).validate().unwrap();
    for (key, bad) in [
        ("normal", json!([0, 0, 1])),
        ("origin_mm", json!([0, 0])),
        ("x_axis", json!([1, 0, 0, 0])),
        ("type", json!("free")),
    ] {
        let mut invalid = value.clone();
        invalid[key] = bad;
        assert!(serde_json::from_value::<AssistantWorkplaneSpec>(invalid).is_err());
    }
    let mut missing = value;
    missing.as_object_mut().unwrap().remove("y_axis");
    assert!(serde_json::from_value::<AssistantWorkplaneSpec>(missing).is_err());
    for (origin_mm, x_axis, y_axis) in [
        ([f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1_000_001.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0]),
        ([0.0; 3], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ([0.0; 3], [1.0, 0.0, 0.0], [0.6, 0.8, 0.0]),
        ([0.0; 3], [f64::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ] {
        assert!(
            program(AssistantWorkplaneSpec::Frame {
                origin_mm,
                x_axis,
                y_axis
            })
            .validate()
            .is_err()
        );
    }
}
