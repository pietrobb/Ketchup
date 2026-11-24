use super::*;

#[test]
fn thermal_review_conducts_heat_through_a_steel_part_without_mutation() {
    use ketchup_model::document::{
        ClassificationCategoryId, ClassificationDimensionId, Dimension, FeatureKind,
        ProfileSegment, Transform,
    };
    use ketchup_model::validation::MATERIAL_DIMENSION_V1;

    let worker_path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before the headless thermal integration test");
    let mut server = Server::new(SessionSettings {
        exact_worker_path: Some(worker_path),
        ..SessionSettings::default()
    });
    let definition = DefinitionId(81);
    let profile = FeatureId(82);
    let solid = FeatureId(83);
    let occurrence = OccurrenceId(84);
    let seed = server
        .session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Heated pin".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Circular section".into(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [10.0, 0.0],
                            end_mm: [-10.0, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [-10.0, 0.0],
                            end_mm: [10.0, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "Pin".into(),
                kind: FeatureKind::extrusion(profile, Dimension::new("20", 20.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Pin instance".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: MATERIAL_DIMENSION_V1.into(),
                categories: vec![(ClassificationCategoryId(1), "steel".into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: occurrence,
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
        ]))
        .unwrap();
    server.session.apply_proposal(&seed).unwrap();
    let evaluated = request(&mut server, "evaluate", json!({"timeout_ms":30000}));
    assert_eq!(evaluated["result"]["complete"], true, "{evaluated}");
    let faces = request(
        &mut server,
        "query",
        json!({"kind":"faces","limit":20,"definition_id":definition.0}),
    );
    let items = faces["result"]["items"].as_array().unwrap();
    let height = |item: &&Value| item["geometry"]["centroid_mm"][2].as_f64().unwrap();
    let bottom = items
        .iter()
        .min_by(|left, right| height(left).total_cmp(&height(right)))
        .unwrap()["ordinal"]
        .as_u64()
        .unwrap();
    let top = items
        .iter()
        .max_by(|left, right| height(left).total_cmp(&height(right)))
        .unwrap()["ordinal"]
        .as_u64()
        .unwrap();
    assert_ne!(bottom, top);
    let state = server.state();
    let params = |confirmed| {
        json!({
            "expected_revision":state["revision"],
            "expected_digest":state["canonical_digest"],
            "definition_id":definition.0,
            "feature_id":solid.0,
            "occurrence_id":occurrence.0,
            "case_id":"pin-between-two-temperatures",
            "held_faces":[
                {"face_ordinal":bottom,"temperature_c":20.0},
                {"face_ordinal":top,"temperature_c":120.0}
            ],
            "mesh_levels":[
                {"surface_deflection_mm":0.3,"angular_deflection_rad":0.3,"max_tetrahedra":512,"max_relative_volume_error":0.05,"min_tetrahedron_quality":0.000001},
                {"surface_deflection_mm":0.12,"angular_deflection_rad":0.12,"max_tetrahedra":1024,"max_relative_volume_error":0.02,"min_tetrahedron_quality":0.000001}
            ],
            "convergence_relative_tolerance":0.1,
            "confirmed":confirmed
        })
    };
    let refused = request(&mut server, "thermal_review", params(false));
    assert_eq!(
        refused["error"]["code"], "confirmation_required",
        "{refused}"
    );
    assert_eq!(server.state(), state);

    let reviewed = request(&mut server, "thermal_review", params(true));
    assert!(reviewed.get("error").is_none(), "{reviewed}");
    let result = &reviewed["result"];
    assert_eq!(result["units"], "mm,°C,W");
    assert_eq!(result["material"]["material"], "steel");
    assert!(
        result["material"]["source"]
            .as_str()
            .unwrap()
            .starts_with("EN 1993-1-2")
    );
    let levels = result["levels"].as_array().unwrap();
    assert_eq!(levels.len(), 2);
    assert_eq!(result["convergence"]["converged"], true, "{result}");
    assert_eq!(result["document_mutated"], false);
    assert_eq!(result["review_digest"].as_str().unwrap().len(), 64);
    // q = k ΔT A / h through the Ø20 × 20 mm pin.
    let conductivity = result["material"]["conductivity_w_per_m_k"]
        .as_f64()
        .unwrap();
    let exact_w = conductivity * 100.0 / 0.02 * std::f64::consts::PI * 0.01 * 0.01;
    for level in levels {
        let heat_w = level["heat_into_held_faces_w"].as_f64().unwrap();
        let volume_error = level["relative_volume_error"].as_f64().unwrap();
        assert!(
            heat_w <= exact_w * (1.0 + 1.0 / 1.0e9)
                && heat_w >= exact_w * (1.0 - volume_error - 1.0 / 1.0e9),
            "{heat_w} W against {exact_w} W"
        );
    }
    assert_eq!(server.state(), state);

    let unknown = request(&mut server, "thermal_review", {
        let mut bad = params(true);
        bad["held_faces"] = json!([{"face_ordinal":bottom,"temperature_c":20.0,"kelvin":true}]);
        bad
    });
    assert_eq!(unknown["error"]["code"], "invalid_params", "{unknown}");
}
