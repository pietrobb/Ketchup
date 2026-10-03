use super::*;

#[test]
fn opposing_holes_report_world_gap_and_parameter_repair_through_native_mcp() {
    let source = "depth=param('depth',10)\np=box('divider',(100,100,18))\nhole(p,'z-',at=(40,50),diameter=8,depth=8,id='left')\nhole(p,'z+',at=(40,50),diameter=8,depth=depth,id='right')\nc=component('c',[p])\ninstance('copy',c,at=(200,0,0),x=(0,0,-1),z=(1,0,0))";
    let mut wire = Wire::new();
    let response = wire.call_within(apply(source, true), Duration::from_secs(60));
    assert!(response.ok, "{response:?}");
    let first = response.result.unwrap();
    let issues = |value: &serde_json::Value| {
        value["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"].as_str().unwrap().starts_with("opposing_holes"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let found = issues(&first);
    assert_eq!(found.len(), 2, "{first:#}");
    assert!(found.iter().all(|i| i["severity"] == "error"));
    let copy = found
        .iter()
        .find(|i| i["parts"] == json!(["copy/divider"]))
        .unwrap();
    assert_eq!(
        copy["where_mm"],
        json!([[208., 50., -40.], [208., 50., -40.]])
    );
    let original = wire.app.document.current().scene_query();
    let undo_steps = wire.app.undo_step_count();
    let mut request = apply(source, false);
    if let Request::ApplyProgram { overrides, .. } = &mut request {
        overrides.insert("depth".into(), 7.);
    }
    let response = wire.call_within(request, Duration::from_secs(60));
    assert!(response.ok, "{response:?}");
    let repaired = response.result.unwrap();
    assert!(issues(&repaired).is_empty(), "{repaired:#}");
    assert_eq!(wire.app.undo_step_count(), undo_steps + 1);
    let identities = |scene: &[ketchup_model::document::SceneOccurrence]| {
        scene
            .iter()
            .map(|o| o.instance_path.clone())
            .collect::<Vec<_>>()
    };
    let current = wire.app.document.current().scene_query();
    assert_eq!(identities(&current), identities(&original));
    assert!(wire.call(Request::Undo { expected: None }).ok);
    assert_eq!(wire.app.document.current().scene_query(), original);
    assert!(wire.call(Request::Redo { expected: None }).ok);
    assert_eq!(wire.app.document.current().scene_query(), current);
}
