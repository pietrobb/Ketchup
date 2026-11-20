use super::*;
use std::f64::consts::PI;

const RADIUS_MM: f64 = 10.0;
const HEIGHT_MM: f64 = 20.0;
const PRESSURE_N_PER_MM2: f64 = 0.01;
const STEEL_KG_PER_M3: f64 = 7_850.0;

/// Two occurrences of one steel cylinder, the second standing on the first,
/// meshed and evaluated: the server and the bottom and top face ordinals.
fn stacked_cylinders() -> (Server, u64, u64) {
    use ketchup_model::document::{Dimension, FeatureKind, ProfileSegment, Transform};

    let worker_path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before the headless FEA integration test");
    let mut server = Server::new(SessionSettings {
        exact_worker_path: Some(worker_path),
        ..SessionSettings::default()
    });
    let definition = DefinitionId(81);
    let occurrence = |id: u64, z: f64| CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: definition,
        name: format!("Cylinder {id}"),
        transform: Transform::from_translation(0.0, 0.0, z).unwrap(),
        parent: None,
        tags: Default::default(),
        visible: true,
    };
    let seed = server
        .session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "FEA cylinder".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(82),
                definition_id: definition,
                name: "Circular section".into(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [RADIUS_MM, 0.0],
                            end_mm: [-RADIUS_MM, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [-RADIUS_MM, 0.0],
                            end_mm: [RADIUS_MM, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(83),
                definition_id: definition,
                name: "Cylinder".into(),
                kind: FeatureKind::extrusion(
                    FeatureId(82),
                    Dimension::new("20", HEIGHT_MM).unwrap(),
                ),
            },
            occurrence(84, 0.0),
            occurrence(85, HEIGHT_MM),
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
    let height = |face: &&Value| face["geometry"]["centroid_mm"][2].as_f64().unwrap();
    let by_height = |left: &&Value, right: &&Value| height(left).total_cmp(&height(right));
    let ordinal = |face: &Value| face["ordinal"].as_u64().unwrap();
    let bottom = ordinal(items.iter().min_by(by_height).unwrap());
    let top = ordinal(items.iter().max_by(by_height).unwrap());
    assert_ne!(bottom, top);
    (server, bottom, top)
}

/// The study's request against the document `state`, with `extra` fields
/// replacing the defaults.
fn params(state: &Value, bottom: u64, top: u64, joint: &Value, extra: &Value) -> Value {
    let part = |occurrence: u64, held: &[u64], loaded: &[u64]| {
        json!({
            "occurrence_id":occurrence,
            "feature_id":83,
            "material":{"id":1,"youngs_modulus_mpa":200000.0,"poisson_ratio":0.3,"yield_strength_mpa":250.0,"density_kg_per_m3":STEEL_KG_PER_M3},
            "constrained_face_ordinals":held,
            "face_tractions":loaded.iter().map(|face| json!({"face_ordinal":face,"traction_local_n_per_mm2":[0.0,0.0,-PRESSURE_N_PER_MM2]})).collect::<Vec<_>>(),
        })
    };
    let mut params = json!({
        "expected_revision":state["revision"],
        "expected_digest":state["canonical_digest"],
        "case_id":"stacked-cylinders",
        "parts":[part(84, &[bottom], &[]), part(85, &[], &[top])],
        "joints":[{"faces":[{"part":0,"face_ordinal":top},{"part":1,"face_ordinal":bottom}],"kind":joint}],
        "mesh_levels":[
            {"surface_deflection_mm":0.3,"angular_deflection_rad":0.3,"max_tetrahedra":512,"max_relative_volume_error":0.05,"min_tetrahedron_quality":0.000001},
            {"surface_deflection_mm":0.12,"angular_deflection_rad":0.12,"max_tetrahedra":1024,"max_relative_volume_error":0.02,"min_tetrahedron_quality":0.000001}
        ],
        "solve_settings":{"maximum_nodes":4096,"relative_pivot_tolerance":1.0e-12,"maximum_small_deformation_ratio":0.05,"convergence_relative_tolerance":0.5},
        "confirmed":true,
    });
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    params
}

fn finest_displacement(review: &Value) -> f64 {
    let levels = review["result"]["levels"].as_array().unwrap();
    levels.last().unwrap()["maximum_displacement_mm"]
        .as_f64()
        .unwrap()
}

/// R5.5: an assembly of two occurrences joined face to face is reviewed
/// through the headless session like one part, with its natural frequencies
/// when asked, and the document stays as it was.
#[test]
fn an_assembly_of_joined_occurrences_is_reviewed_with_its_natural_frequencies() {
    let (mut server, bottom, top) = stacked_cylinders();
    let state = server.state();
    let rigid = json!({"type":"rigid"});

    let refused = request(
        &mut server,
        "fea_assembly_review",
        params(&state, bottom, top, &rigid, &json!({"confirmed":false})),
    );
    assert_eq!(refused["error"]["code"], "confirmation_required");
    assert_eq!(server.state(), state);

    let reviewed = request(
        &mut server,
        "fea_assembly_review",
        params(&state, bottom, top, &rigid, &json!({"mode_count":2})),
    );
    assert!(reviewed.get("error").is_none(), "{reviewed}");
    let result = &reviewed["result"];
    assert_eq!(result["case_id"], "stacked-cylinders");
    assert_eq!(result["convergence"]["converged"], true, "{result}");
    assert_eq!(result["document_mutated"], false);
    assert_eq!(server.state(), state);
    let levels = result["levels"].as_array().unwrap();
    assert_eq!(levels.len(), 2);
    for level in levels {
        assert!(
            level["maximum_free_dof_residual_n"].as_f64().unwrap() <= 1.0e-6,
            "{level}"
        );
    }
    // Both cylinders' mass, up to how closely the mesh fills them.
    let modes = &result["modes"];
    let volume_error = levels[1]["relative_volume_error"].as_f64().unwrap();
    let mass = 2.0 * PI * RADIUS_MM * RADIUS_MM * HEIGHT_MM * STEEL_KG_PER_M3 * 1.0e-9;
    let found_mass = modes["total_mass_kg"].as_f64().unwrap();
    assert!(
        (found_mass - mass).abs() <= (volume_error + 1.0e-12) * mass,
        "{found_mass} against {mass}"
    );
    let frequencies = modes["frequencies_hz"].as_array().unwrap();
    assert_eq!(frequencies.len(), 2);
    let [lowest, next] = [0, 1].map(|index| frequencies[index].as_f64().unwrap());
    assert!(0.0 < lowest && lowest <= next, "{frequencies:?}");
    assert!(modes["maximum_relative_residual"].as_f64().unwrap() <= 1.0e-6);
    // R1120ac-FEA-900: the modes of the level before, how much they changed,
    // whether that is within the tolerance, and what modes leave out.
    let previous = modes["previous_level_frequencies_hz"].as_array().unwrap();
    assert_eq!(previous.len(), 2, "{modes}");
    let change = (0..2)
        .map(|index| {
            let [was, is] = [&previous[index], &frequencies[index]].map(|f| f.as_f64().unwrap());
            (is - was).abs() / is.abs().max(was.abs())
        })
        .fold(0.0, f64::max);
    let reported = modes["maximum_relative_frequency_change"].as_f64().unwrap();
    assert!((reported - change).abs() <= 1.0e-12, "{modes}");
    let tolerance = result["convergence"]["required_relative_tolerance"]
        .as_f64()
        .unwrap();
    assert_eq!(modes["converged"], change <= tolerance, "{modes}");
    let not_checked = modes["not_checked"].as_array().unwrap();
    assert_eq!(not_checked.len(), 4, "{modes}");
    assert!(
        not_checked[0].as_str().unwrap().contains("too stiff"),
        "{modes}"
    );

    // A soft spring joint lets the upper cylinder sink by its load over the
    // spring's stiffness, on top of how both cylinders deform.
    let stiffness = 1_000.0;
    let spring = json!({"type":"spring","translation_n_per_mm":[stiffness, stiffness, stiffness],"rotation_n_mm_per_rad":[1.0e9, 1.0e9, 1.0e9]});
    let sprung = request(
        &mut server,
        "fea_assembly_review",
        params(&state, bottom, top, &spring, &json!({})),
    );
    assert!(sprung.get("error").is_none(), "{sprung}");
    assert!(sprung["result"]["modes"].is_null());
    let sink = PI * RADIUS_MM * RADIUS_MM * PRESSURE_N_PER_MM2 / stiffness;
    let elastic = finest_displacement(&reviewed);
    let found = finest_displacement(&sprung);
    assert!(
        (found - sink).abs() <= 0.03 * sink + elastic,
        "{found} against {sink} + {elastic}"
    );
    assert_eq!(server.state(), state);
}

/// The lower cylinder's top mated face to face to the upper one's bottom.
fn mate_the_stack(server: &mut Server, mate_id: u64) {
    use ketchup_model::assembly::{
        AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
    };
    use ketchup_model::exact_product::{ExactBodyPackage, ExactFaceRole};

    let snapshot = server.session.snapshot();
    let package = server
        .session
        .exact_results()
        .get_render(&snapshot, DefinitionId(81))
        .unwrap();
    let ExactBodyPackage::Graph(package) = package.as_ref() else {
        panic!("the cylinder has exact graph evidence");
    };
    let face = |role| {
        package
            .planar_face_attachments
            .iter()
            .find(|attachment| attachment.reference().role() == Some(role))
            .unwrap()
            .clone()
    };
    let mate = AssemblyMate::new(
        AssemblyMateId(mate_id),
        AssemblyMateEndpoint::resolved_planar_face(OccurrenceId(84), face(ExactFaceRole::Top)),
        AssemblyMateEndpoint::resolved_planar_face(OccurrenceId(85), face(ExactFaceRole::Bottom)),
        AssemblyMateKind::CoincidentPlanar {
            offset_mm: 0.0,
            reversed: true,
        },
    );
    let proposal = server
        .session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(mate),
        ]))
        .unwrap();
    server.session.apply_proposal(&proposal).unwrap();
}

/// R5.5: a face-to-face mate of the document joins the faces it names, so the
/// study need not name them again.
#[test]
fn a_face_to_face_mate_joins_an_assembly_review_like_the_faces_it_names() {
    let (mut server, bottom, top) = stacked_cylinders();
    mate_the_stack(&mut server, 86);
    let state = server.state();
    let rigid = json!({"type":"rigid"});

    let named = request(
        &mut server,
        "fea_assembly_review",
        params(&state, bottom, top, &rigid, &json!({})),
    );
    assert!(named.get("error").is_none(), "{named}");
    let mated = request(
        &mut server,
        "fea_assembly_review",
        params(
            &state,
            bottom,
            top,
            &rigid,
            &json!({"joints":[],"mate_joints":[{"mate_id":86,"kind":rigid}]}),
        ),
    );
    assert!(mated.get("error").is_none(), "{mated}");
    assert_eq!(mated["result"]["levels"], named["result"]["levels"]);

    let unknown = request(
        &mut server,
        "fea_assembly_review",
        params(
            &state,
            bottom,
            top,
            &rigid,
            &json!({"joints":[],"mate_joints":[{"mate_id":87,"kind":rigid}]}),
        ),
    );
    assert_eq!(unknown["error"]["code"], "fea_setup_rejected", "{unknown}");
    assert_eq!(server.state(), state);
}

#[test]
fn an_assembly_review_that_cannot_be_built_is_refused_without_changing_the_document() {
    let (mut server, bottom, top) = stacked_cylinders();
    let state = server.state();
    let rigid = json!({"type":"rigid"});

    let one_part = params(&state, bottom, top, &rigid, &json!({}));
    let mut one_part = one_part;
    one_part["joints"][0]["faces"][1]["part"] = json!(0);
    let refused = request(&mut server, "fea_assembly_review", one_part);
    assert_eq!(refused["error"]["code"], "fea_setup_rejected", "{refused}");

    let no_density = params(&state, bottom, top, &rigid, &json!({"mode_count":1}));
    let mut no_density = no_density;
    for part in 0..2 {
        no_density["parts"][part]["material"]
            .as_object_mut()
            .unwrap()
            .remove("density_kg_per_m3");
    }
    let refused = request(&mut server, "fea_assembly_review", no_density);
    assert_eq!(refused["error"]["code"], "fea_solve_rejected", "{refused}");

    let unknown = params(&state, bottom, top, &json!({"type":"glued"}), &json!({}));
    let refused = request(&mut server, "fea_assembly_review", unknown);
    assert_eq!(refused["error"]["code"], "invalid_params", "{refused}");
    assert_eq!(server.state(), state);
}
