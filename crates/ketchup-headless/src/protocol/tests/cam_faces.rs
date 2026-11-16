use super::*;

#[test]
fn cam_preview_machines_faces_of_the_exact_target_named_by_ordinal() {
    use ketchup_model::cam::{
        CamCutParameters, CamPlan, CamSetup, CamStock, CamTool, CamToolKind, CamWorkOffset,
    };
    use ketchup_model::document::{Dimension, FeatureKind, Transform};

    let worker_path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before the headless CAM integration test");
    let mut server = Server::new(SessionSettings {
        exact_worker_path: Some(worker_path),
        ..SessionSettings::default()
    });
    let definition = DefinitionId(97);
    let profile = FeatureId(970);
    let solid = FeatureId(971);
    let seed = server
        .session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "CAM face target".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "20x10 profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "20x10x5 block".into(),
                kind: FeatureKind::extrusion(profile, Dimension::new("5", 5.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(972),
                definition_id: definition,
                name: "Block instance".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    server.session.apply_proposal(&seed).unwrap();
    let plan = CamPlan::new(
        &server.session.snapshot(),
        CamPlanId(97),
        "Profile from the bottom face",
        CamStock {
            minimum_mm: [-3.0, -3.0, 0.0],
            maximum_mm: [23.0, 13.0, 5.0],
        },
        CamTool {
            number: 2,
            kind: CamToolKind::FlatEndMill,
            diameter_mm: 2.0,
            flute_length_mm: 6.0,
            overall_length_mm: 30.0,
            holder_diameter_mm: 6.0,
            holder_length_mm: 10.0,
            spindle_rpm: 10_000,
            feed_mm_per_min: 600.0,
            plunge_mm_per_min: 200.0,
        },
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 10.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        definition,
        solid,
    )
    .unwrap();
    let proposal = server
        .session
        .plan_commands(CommandBatch::new(vec![CanonicalCommand::UpsertCamPlan(
            plan,
        )]))
        .unwrap();
    server.session.apply_proposal(&proposal).unwrap();
    let evaluated = request(&mut server, "evaluate", json!({"timeout_ms":30000}));
    assert_eq!(evaluated["result"]["complete"], true, "{evaluated}");
    let faces = request(
        &mut server,
        "query",
        json!({"kind":"faces","limit":20,"definition_id":definition.0}),
    );
    let bottom = faces["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .min_by(|left, right| {
            left["geometry"]["centroid_mm"][2]
                .as_f64()
                .unwrap()
                .total_cmp(&right["geometry"]["centroid_mm"][2].as_f64().unwrap())
        })
        .unwrap()["ordinal"]
        .as_u64()
        .unwrap();
    let state = server.state();
    let preview = |server: &mut Server, operation: Value| {
        request(
            server,
            "cam_preview",
            json!({
                "expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],
                "plan_id":97,
                "operations":[operation],
                "fixtures":[],
                "dialect":"iso_metric_gcode"
            }),
        )
    };

    // The caller names the face; Ketchup reads its boundary and depth from
    // the exact part and cuts one tool radius outside it, down to the face.
    let profiled = preview(
        &mut server,
        json!({"type":"face_contour","id":1,"face_ordinal":bottom,"side":"outside"}),
    );
    assert!(profiled.get("error").is_none(), "{profiled}");
    let band_mm2 = 60.0 * 2.0 + std::f64::consts::PI * 4.0;
    let removed = profiled["result"]["removed_stock_mm3"].as_f64().unwrap();
    assert!((removed - band_mm2 * 5.0).abs() < 0.5, "{profiled}");
    assert!(profiled["result"]["gouge_mm3"].as_f64().unwrap() < 1.0e-5);

    // A flat face is not a hole, and a face the part does not have is refused.
    for operation in [
        json!({"type":"face_drill","id":2,"face_ordinal":bottom}),
        json!({"type":"face_contour","id":3,"face_ordinal":9_999,"side":"outside"}),
        json!({"type":"face_pocket","id":4,"face_ordinal":9_999}),
    ] {
        let refused = preview(&mut server, operation);
        assert_eq!(
            refused["error"]["code"], "cam_planning_rejected",
            "{refused}"
        );
    }
    let malformed = preview(
        &mut server,
        json!({"type":"face_contour","id":5,"face_ordinal":bottom,"side":"diagonal"}),
    );
    assert_eq!(malformed["error"]["code"], "invalid_params");
    assert_eq!(server.state(), state);
}
