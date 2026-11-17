//! Tools > Analysis > Steady heat conduction on a real exact steel part,
//! driven through the offscreen AccessKit tree: the conductivity is steel's
//! in the library rules, and the document is never changed.

use super::*;

/// The exact faces of the body with the lowest and the highest centroid.
fn end_faces(
    snapshot: &ketchup_model::document::Snapshot,
    definition: DefinitionId,
    solid: FeatureId,
) -> (u32, u32) {
    let graph =
        ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(snapshot, definition, solid)
            .unwrap();
    let mut worker = ketchup_scheduler::ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker
        .evaluate_exact_brep_graph_volume_mesh(
            &graph,
            ketchup_scheduler::ExactVolumeMeshWireOptions {
                surface_deflection_mm: 0.3,
                angular_deflection_rad: 0.3,
                max_tetrahedra: 512,
                max_relative_volume_error: 0.05,
                min_tetrahedron_quality: 1.0e-6,
            },
        )
        .unwrap();
    let faces = &package.source.face_evidence;
    let bottom = faces
        .iter()
        .min_by(|left, right| left.centroid_mm[2].total_cmp(&right.centroid_mm[2]))
        .unwrap();
    let top = faces
        .iter()
        .max_by(|left, right| left.centroid_mm[2].total_cmp(&right.centroid_mm[2]))
        .unwrap();
    (bottom.face_ordinal, top.face_ordinal)
}

#[test]
fn steady_thermal_review_runs_through_offscreen_accesskit_without_mutation() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("thermal-source.ketchup");
    let definition = DefinitionId(81);
    let profile = FeatureId(82);
    let solid = FeatureId(83);
    let occurrence = OccurrenceId(84);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
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
                id: ketchup_model::document::ClassificationDimensionId(1),
                name: ketchup_model::validation::MATERIAL_DIMENSION_V1.into(),
                categories: vec![(
                    ketchup_model::document::ClassificationCategoryId(1),
                    "steel".into(),
                )],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: occurrence,
                dimension_id: ketchup_model::document::ClassificationDimensionId(1),
                category_id: Some(ketchup_model::document::ClassificationCategoryId(1)),
            },
        ]))
        .unwrap();
    std::fs::write(
        &source,
        ketchup_model::persistence::save_document_store(
            &fixture,
            &ketchup_model::persistence::ContainerData::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let dialogs = ScriptedFileDialogs::new().queue_open(&source);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    assert!(shell.app_mut().headless_select_occurrence(occurrence));
    shell.settle();
    let baseline = canonical_state(&shell);

    shell.click_submenu_command(
        "menu-tools",
        "menu-tools-analysis",
        AppCommand::ReviewSteadyThermal,
    );
    let run = shell.catalog().text("thermal-review-run-confirmed");
    wait_for_visible_label(&mut shell, &run);
    assert_eq!(canonical_state(&shell), baseline);
    // Hold the pin's bottom face at 20 °C and its top face at 100 °C.
    let (bottom, top) = end_faces(&fixture.current(), definition, solid);
    for (label, value) in [
        ("thermal-review-hot-faces", top.to_string()),
        ("thermal-review-cold-faces", bottom.to_string()),
    ] {
        let label = shell.catalog().text(label);
        shell.focus_text_input(&label);
        shell.key(Key::A, eframe::egui::Modifiers::COMMAND);
        shell.type_text(&value);
    }
    shell.click_button_label(&run);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("thermal-review-ready")
    );
    // The default setup holds two end faces of the pin at 100 and 20 °C;
    // the result names the material its conductivity is of, with its source.
    let steel =
        ketchup_application::validation_rules::ValidationRules::library().materials["steel"]
            .thermal_conductivity
            .clone()
            .unwrap();
    let material = format!(
        "{}: steel · {} W/(m·K) · {}",
        shell.catalog().text("thermal-review-material"),
        steel.w_m_k,
        steel.source
    );
    assert!(shell.has_visible_label(&material), "{material}");
    assert_eq!(canonical_state(&shell), baseline);

    let cancel = shell.catalog().text("dialog-cancel");
    shell.click_button_label(&cancel);
    assert!(!shell.has_visible_label(&run));
    assert_eq!(canonical_state(&shell), baseline);
}
