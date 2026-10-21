use super::*;

#[test]
fn assistant_sheet_metal_reaches_exact_worker_and_file_export_through_accesskit() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("sheet-metal-source.ketchup");
    let flat_pattern = directory.path().join("sheet-metal-flat.dxf");
    let bend_table = flat_pattern.with_extension("bends.csv");
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(80),
                name: "Assistant sheet metal".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(80),
                definition_id: DefinitionId(80),
                name: "Assistant sheet metal".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
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

    let request = "Create a 100 by 50 millimetre sheet-metal bracket with opposite bends.";
    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 80.into(),
            name: "Opposite flanges".into(),
            feature: AssistantCadBodyFeature::SheetMetal {
                base_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]],
                thickness_mm: 2.0,
                k_factor: 0.4,
                bends: vec![
                    AssistantCadSheetMetalBend {
                        parent: None,
                        edge: 1,
                        length_mm: 30.0,
                        angle_degrees: -45.0,
                        inner_radius_mm: 3.0,
                    },
                    AssistantCadSheetMetalBend {
                        parent: None,
                        edge: 3,
                        length_mm: 20.0,
                        angle_degrees: 90.0,
                        inner_radius_mm: 3.0,
                    },
                ],
            },
        }],
    };
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_export(&flat_pattern)
        .always_confirm_high_risk_as(405);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    assert_eq!(shell.app().document_snapshot().features().count(), 0);
    apply_program(&mut shell, request, &program_request);

    let feature_id = shell
        .app()
        .document_snapshot()
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::SheetMetal(_)))
        .map(ketchup_model::document::Feature::id)
        .expect("the applied Assistant program must create one canonical sheet-metal feature");
    wait_for_current_exact_body(&mut shell);
    assert_eq!(shell.app().exact_current_producer_ids(), [feature_id]);
    let before_export = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportSheetMetalManufacturing);
    assert!(flat_pattern.is_file(), "{}", shell.app().action_digest());
    assert!(bend_table.is_file());
    let parsed = inspect_dxf(
        &std::fs::read(&flat_pattern).unwrap(),
        DxfImportOptions::new(None),
    )
    .unwrap();
    assert!(!parsed.profiles().is_empty());
    let bends = std::fs::read_to_string(&bend_table).unwrap();
    assert!(bends.contains("\n0,base,1,down,-45,3,"));
    assert!(bends.contains("\n1,base,3,up,90,3,"));
    assert_state_and_history_unchanged(&mut shell, &before_export, &before_history);
}

#[test]
fn assistant_cam_setup_reviews_exact_simulation_and_exports_through_accesskit() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cam-source.ketchup");
    let output = directory.path().join("reviewed-facing.nc");
    let definition = DefinitionId(95);
    let profile = FeatureId(950);
    let solid = FeatureId(951);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "CAM target".into(),
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
                name: "20x10x5 target".into(),
                kind: FeatureKind::extrusion(profile, Dimension::new("5", 5.0).unwrap()),
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

    let request = "Set up a reviewed 2.5D facing operation.";
    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::UpsertCamPlan {
            plan_id: 95,
            name: "Reviewed facing".into(),
            target_definition_id: definition.0,
            target_feature_id: solid.0,
            stock_minimum_mm: [0.0, 0.0, 0.0],
            stock_maximum_mm: [20.0, 10.0, 7.0],
            tool_number: 1,
            tool_kind: AssistantCamToolKind::FlatEndMill,
            tool_diameter_mm: 2.0,
            flute_length_mm: 5.0,
            overall_length_mm: 10.0,
            holder_diameter_mm: 6.0,
            holder_length_mm: 10.0,
            spindle_rpm: 10_000,
            feed_mm_per_min: 600.0,
            plunge_mm_per_min: 200.0,
            work_offset: AssistantCamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 10.0,
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        }],
    };
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_export(&output);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let baseline = canonical_state(&shell);

    apply_program(&mut shell, request, &program_request);
    assert!(
        shell
            .app()
            .document_snapshot()
            .cam_plan(ketchup_model::cam::CamPlanId(95))
            .is_some()
    );
    let setup_state = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ReviewCamExport);
    let cancel = shell.catalog().text("dialog-cancel");
    wait_for_visible_label(&mut shell, &cancel);
    shell.click_button_label(&cancel);
    assert!(!output.exists());
    assert_eq!(canonical_state(&shell), setup_state);

    shell.click_menu_command("menu-file", AppCommand::ReviewCamExport);
    let preview = shell.catalog().text("cam-review-preview");
    shell.click_button_label(&preview);
    assert!(shell.has_visible_label("mm: G54"));
    assert!(shell.app().action_digest().contains("safe"));
    assert_eq!(canonical_state(&shell), setup_state);
    let confirm = shell.catalog().text("cam-review-confirm-export");
    shell.click_button_label(&confirm);
    assert!(output.is_file(), "{}", shell.app().action_digest());
    let artifact = std::fs::read_to_string(&output).unwrap();
    assert!(artifact.contains("\nG21\nG90\nG17\nG54\nT1 M6\nS10000 M3\n"));
    assert_eq!(canonical_state(&shell), setup_state);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline.digest);
    assert!(
        shell
            .app()
            .document_snapshot()
            .cam_plan(ketchup_model::cam::CamPlanId(95))
            .is_none()
    );
}

#[test]
fn static_fea_review_runs_through_offscreen_accesskit_without_mutation() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("fea-source.ketchup");
    let definition = DefinitionId(71);
    let profile = FeatureId(72);
    let solid = FeatureId(73);
    let occurrence = OccurrenceId(74);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "FEA cylinder".into(),
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
                name: "Cylinder".into(),
                kind: FeatureKind::extrusion(profile, Dimension::new("20", 20.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Cylinder instance".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
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

    shell.click_menu_command("menu-file", AppCommand::ReviewStaticFea);
    let cancel = shell.catalog().text("dialog-cancel");
    wait_for_visible_label(&mut shell, &cancel);
    shell.click_button_label(&cancel);
    assert_eq!(canonical_state(&shell), baseline);

    shell.click_menu_command("menu-file", AppCommand::ReviewStaticFea);
    let run = shell.catalog().text("fea-review-run-confirmed");
    wait_for_visible_label(&mut shell, &run);
    assert_eq!(canonical_state(&shell), baseline);
    shell.click_button_label(&run);
    assert!(
        shell.app().action_digest().contains("without changing")
            || shell.app().action_digest().contains("bez zmeny"),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(canonical_state(&shell), baseline);
}

#[test]
fn local_pdm_root_child_and_verified_open_run_through_offscreen_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("pdm-source.ketchup");
    let repository = directory.path().join(".ketchup-pdm");
    let definition = DefinitionId(301);
    let profile = FeatureId(302);
    let solid = FeatureId(303);
    let occurrence = OccurrenceId(304);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Released bracket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "80x40 profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "80x40x12 bracket".into(),
                kind: FeatureKind::extrusion(profile, Dimension::new("12", 12.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Bracket instance".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
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
    let root_state = canonical_state(&shell);
    let root_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    let cancel = shell.catalog().text("dialog-cancel");
    wait_for_visible_label(&mut shell, &cancel);
    shell.click_button_label(&cancel);
    assert!(!repository.exists());
    assert_state_and_history_unchanged(&mut shell, &root_state, &root_history);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    let confirm = shell.catalog().text("pdm-confirm-create");
    wait_for_visible_label(&mut shell, &confirm);
    shell.click_button_label(&confirm);
    assert!(digest_starts_like(&shell, "pdm-release-created"));
    assert_eq!(
        std::fs::read_dir(repository.join("releases"))
            .unwrap()
            .count(),
        1
    );
    assert_state_and_history_unchanged(&mut shell, &root_state, &root_history);
    shell.click_button_label(&cancel);

    assert!(shell.app_mut().headless_select_occurrence(occurrence));
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    shell.settle();
    let child_state = canonical_state(&shell);
    let child_history = reachable_history_digests(&mut shell);
    assert_ne!(child_state.digest, root_state.digest);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    let refresh = shell.catalog().text("pdm-catalog-refresh");
    wait_for_visible_label(&mut shell, &refresh);
    shell.click_button_label(&refresh);
    assert!(digest_starts_like(&shell, "pdm-catalog-ready"));
    shell.click_button_label(&confirm);
    assert!(digest_starts_like(&shell, "pdm-release-created"));
    assert_eq!(
        std::fs::read_dir(repository.join("releases"))
            .unwrap()
            .count(),
        2
    );
    assert_state_and_history_unchanged(&mut shell, &child_state, &child_history);

    shell.click_button_label(&confirm);
    assert_eq!(
        shell.app().action_digest(),
        "local PDM request no longer matches the current document"
    );
    assert_eq!(
        std::fs::read_dir(repository.join("releases"))
            .unwrap()
            .count(),
        2
    );
    shell.click_button_label(&cancel);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    wait_for_visible_label(&mut shell, &refresh);
    shell.click_button_label(&refresh);
    let verify = shell.catalog().text("pdm-open-release");
    shell.click_button_label(&verify);
    assert!(digest_starts_like(&shell, "pdm-open-verified"));
    assert_state_and_history_unchanged(&mut shell, &child_state, &child_history);
}

#[test]
fn assistant_weldment_recomputes_and_exports_cut_list_through_accesskit() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("weldment-source.ketchup");
    let cut_list = directory.path().join("weldment-cut-list.csv");
    let drawing = cut_list.with_extension("svg");
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(80),
                name: "Assistant weldment".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(80),
                definition_id: DefinitionId(80),
                name: "Shared square profile".into(),
                kind: FeatureKind::polygon(&[[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(81),
                definition_id: DefinitionId(80),
                name: "Horizontal path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [-100.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 0.0],
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(82),
                definition_id: DefinitionId(80),
                name: "Vertical path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 100.0, 0.0],
                    }],
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(80),
                definition_id: DefinitionId(80),
                name: "Assistant weldment".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(200),
                name: ketchup_manufacturing::fabrication::FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![(
                    ClassificationCategoryId(201),
                    ketchup_manufacturing::fabrication::MANUFACTURED_ITEM_ROLE_V1.into(),
                )],
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(210),
                name: ketchup_manufacturing::fabrication::MATERIAL_DIMENSION_V1.into(),
                categories: vec![(
                    ClassificationCategoryId(211),
                    "ketchup.material.steel.s355.v1".into(),
                )],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(80),
                dimension_id: ClassificationDimensionId(200),
                category_id: Some(ClassificationCategoryId(201)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(80),
                dimension_id: ClassificationDimensionId(210),
                category_id: Some(ClassificationCategoryId(211)),
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

    let create_request = "Create two square-section weldment members and miter their corner.";
    let resize_request = "Change the shared weldment profile width to 6 millimetres.";
    let body_output = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    let program_create_request = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::AppendFeature {
                definition_id: 80.into(),
                name: "Horizontal member".into(),
                feature: AssistantCadBodyFeature::WeldmentMember {
                    profile_feature_id: 80,
                    path_feature_id: 81,
                    orientation_degrees: 0.0,
                },
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: 80.into(),
                name: "Vertical member".into(),
                feature: AssistantCadBodyFeature::WeldmentMember {
                    profile_feature_id: 80,
                    path_feature_id: 82,
                    orientation_degrees: 0.0,
                },
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: 80.into(),
                name: "Miter corner".into(),
                feature: AssistantCadBodyFeature::WeldmentJoint {
                    first_member_id: body_output(0),
                    second_member_id: body_output(1),
                    policy: AssistantCadWeldmentJointPolicy::Miter,
                    primary: AssistantCadWeldmentJointPrimary::First,
                },
            },
        ],
    };
    let program_resize_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::SetFeatureParameter {
            feature_id: 80,
            parameter_path: "bounds.width".into(),
            value_type: AssistantCadParameterValueType::Length,
            value: 6.0.into(),
        }],
    };
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_export(&cut_list)
        .queue_export(&cut_list)
        .queue_refused_high_risk()
        .always_confirm_high_risk_as(406);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    apply_program(&mut shell, create_request, &program_create_request);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::WeldmentMember(_)))
            .count(),
        2,
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::WeldmentJoint(_)))
            .count(),
        1,
        "{}",
        shell.app().action_digest()
    );
    wait_for_current_exact_body(&mut shell);
    let joint_id = shell
        .app()
        .document_snapshot()
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::WeldmentJoint(_)))
        .map(ketchup_model::document::Feature::id)
        .unwrap();
    assert_eq!(shell.app().exact_current_producer_ids(), [joint_id]);
    let before = shell
        .app_mut()
        .current_general_fabrication_projection()
        .unwrap()
        .weldment
        .unwrap();
    let stable_rows = before
        .rows
        .iter()
        .map(|row| row.stable_row_id.clone())
        .collect::<Vec<_>>();
    let before_digest = before.cut_list_envelope.source_digest;

    apply_program(&mut shell, resize_request, &program_resize_request);
    wait_for_current_exact_body(&mut shell);
    assert_eq!(shell.app().exact_current_producer_ids(), [joint_id]);
    let after = shell
        .app_mut()
        .current_general_fabrication_projection()
        .unwrap()
        .weldment
        .unwrap();
    assert_eq!(
        after
            .rows
            .iter()
            .map(|row| row.stable_row_id.clone())
            .collect::<Vec<_>>(),
        stable_rows
    );
    assert_ne!(after.cut_list_envelope.source_digest, before_digest);
    let before_export = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportWeldmentCutList);
    assert!(!cut_list.exists());
    assert!(!drawing.exists());
    assert!(shell.app().last_side_effect_receipt().is_none());
    shell.click_menu_command("menu-file", AppCommand::ExportWeldmentCutList);
    assert!(cut_list.is_file(), "{}", shell.app().action_digest());
    assert!(drawing.is_file());
    let csv = std::fs::read_to_string(&cut_list).unwrap();
    let svg = std::fs::read_to_string(&drawing).unwrap();
    assert!(csv.contains(ketchup_manufacturing::fabrication::WELDMENT_CUT_LIST_EXPORT_V1));
    assert!(csv.contains("position=1"));
    assert!(csv.contains("position=2"));
    assert!(csv.contains("material=ketchup.material.steel.s355.v1"));
    assert!(svg.contains(ketchup_manufacturing::fabrication::WELDMENT_DRAWING_SVG_V1));
    assert_eq!(
        shell.app().last_side_effect_receipt().unwrap().operation(),
        "release-weldment-cut-list-and-drawing"
    );
    assert_state_and_history_unchanged(&mut shell, &before_export, &before_history);
}

#[test]
fn save_as_then_new_then_open_restores_the_same_canonical_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("composed.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());

    compose_two_shared_occurrences(&mut shell);
    let composed_digest = shell.app().canonical_digest();
    let composed_revision = shell.app().document_revision();
    assert!(shell.app().is_dirty(), "an edited document must be dirty");

    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    assert!(path.is_file(), "Save As must write the document to disk");
    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(
        inspection.schema_version,
        ketchup_model::persistence::CURRENT_SCHEMA
    );
    assert_eq!(inspection.definitions, 1);
    assert_eq!(inspection.root_occurrences, 2);
    assert_eq!(inspection.profiles, 1);
    assert_eq!(inspection.extrusions, 1);
    assert_eq!(inspection.profile_extrusion_definitions, 1);
    assert_eq!(inspection.visible_profile_extrusion_root_occurrences, 2);
    assert_eq!(inspection.canonical_digest, composed_digest);
    assert_eq!(
        inspection.container_sha256,
        ketchup_model::graph::sha256_hex(&std::fs::read(&path).unwrap())
    );
    assert!(!shell.app().is_dirty(), "a saved document must be clean");
    assert_eq!(shell.app().document_path(), Some(path.as_path()));

    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(
        shell.app().active_box_count(),
        0,
        "New must replace the composed model with an empty document"
    );
    assert_eq!(shell.app().document_path(), None);

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(
        shell.app().canonical_digest(),
        composed_digest,
        "a reopened document must keep its IDs, hierarchy, transforms, \
         parameters, and sharing"
    );
    assert_eq!(shell.app().document_revision(), composed_revision);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
    assert!(
        !shell.app().is_dirty(),
        "a freshly opened document is clean"
    );
    assert_eq!(
        script.suggested_names(),
        vec!["Untitled.ketchup".to_owned()],
        "Save As must propose the current document name"
    );
}

#[test]
fn save_open_preserves_the_complete_undo_redo_history_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .queue_open(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    let expected = canonical_state(&shell);
    let expected_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    for cycle in 0..3 {
        shell.click_menu_command("menu-file", AppCommand::New);
        shell.click_menu_command("menu-file", AppCommand::Open);

        let reopened = canonical_state(&shell);
        assert_eq!(reopened.revision, expected.revision, "cycle {cycle}");
        assert_eq!(reopened.digest, expected.digest, "cycle {cycle}");
        assert_eq!(reopened.undo_steps, expected.undo_steps, "cycle {cycle}");
        assert_eq!(reopened.redo_steps, expected.redo_steps, "cycle {cycle}");
        assert!(!reopened.dirty, "cycle {cycle}");
        assert_eq!(
            reachable_history_digests(&mut shell),
            expected_history,
            "cycle {cycle}"
        );

        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), expected_history.1[0]);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), expected.digest);
        if cycle < 2 {
            shell.click_menu_command("menu-file", AppCommand::Save);
        }
    }
}

#[test]
fn corrupt_persisted_history_fails_open_without_replacing_the_active_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("corrupt-history.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    let active = canonical_state(&shell);
    let mut corrupt = std::fs::read(&path).unwrap();
    *corrupt.last_mut().unwrap() ^= 0xff;
    std::fs::write(&path, corrupt).unwrap();

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(canonical_state(&shell), active);
    assert!(digest_starts_like(&shell, "error-open-document"));
}

#[test]
fn recovered_open_warns_with_the_actual_source_and_requires_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("damaged.ketchup");
    let recovery = requested.with_extension("ketchup.recovery");
    let destination = directory.path().join("recovered-copy.ketchup");

    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&requested));
    compose_two_shared_occurrences(&mut author);
    let expected = author.app().canonical_digest();
    author.click_menu_command("menu-file", AppCommand::SaveAs);
    std::fs::copy(&requested, &recovery).unwrap();
    std::fs::write(&requested, b"corrupt primary").unwrap();

    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&requested)
        .queue_save(&destination)
        .always_discard();
    let mut recovered = Shell::with_dialogs(dialogs);
    recovered.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(recovered.app().canonical_digest(), expected);
    assert_eq!(recovered.app().document_path(), None);
    assert_eq!(
        recovered.app().recovery_requested_path(),
        Some(requested.as_path())
    );
    assert_eq!(
        recovered.app().recovery_source_path(),
        Some(recovery.as_path())
    );
    assert!(recovered.app().is_dirty());
    assert!(digest_starts_like(&recovered, "digest-opened-recovery"));
    assert!(
        recovered
            .app()
            .action_digest()
            .contains(&recovery.display().to_string())
    );
    assert!(
        recovered
            .app()
            .action_digest()
            .contains(&requested.display().to_string())
    );

    recovered.click_menu_command("menu-file", AppCommand::Save);
    assert_eq!(recovered.app().document_path(), Some(destination.as_path()));
    assert_eq!(recovered.app().recovery_requested_path(), None);
    assert_eq!(recovered.app().recovery_source_path(), None);
    assert!(!recovered.app().is_dirty());
    assert_eq!(std::fs::read(&requested).unwrap(), b"corrupt primary");
    assert_eq!(
        ketchup_model::persistence::load_file(&destination)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        expected
    );
}

#[test]
fn native_document_inspection_counts_only_visible_modeled_root_occurrences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("hidden-modeled-root.ketchup");
    let script = ScriptedFileDialogs::new().queue_save(&path);
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    assert!(shell.app_mut().set_selection_visibility(false));
    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(inspection.root_occurrences, 2);
    assert_eq!(inspection.profile_extrusion_definitions, 1);
    assert_eq!(inspection.visible_profile_extrusion_root_occurrences, 1);
}

#[test]
fn native_document_inspection_refuses_a_lossy_legacy_file_with_its_audit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-inspection.ketchup");
    std::fs::write(&path, lossy_legacy_document()).unwrap();

    let error = ketchup_app::inspect_native_document(&path).unwrap_err();
    let ketchup_app::NativeDocumentInspectionError::NotCurrentLossless(audit) = &error else {
        panic!("a legacy file must be refused with its load audit: {error:?}");
    };
    assert_ne!(
        audit.source_schema,
        ketchup_model::persistence::CURRENT_SCHEMA
    );
    assert!(!audit.migration_losses.is_empty(), "{audit:?}");
    assert!(
        error
            .to_string()
            .contains(&format!("schema {}", audit.source_schema)),
        "{error}"
    );
}

#[test]
fn native_document_inspection_hashes_the_recovered_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recovered-inspection.ketchup");
    let recovery = path.with_extension("ketchup.recovery");
    let script = ScriptedFileDialogs::new().queue_save(&path);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    std::fs::copy(&path, &recovery).unwrap();
    let recovery_bytes = std::fs::read(&recovery).unwrap();
    std::fs::write(&path, b"corrupt primary").unwrap();

    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(
        inspection.container_sha256,
        ketchup_model::graph::sha256_hex(&recovery_bytes)
    );
}

#[test]
fn confirmed_legacy_migration_writes_and_activates_only_a_new_copy() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy.ketchup");
    let destination = directory.path().join("migrated.ketchup");
    let occupied_destination = directory.path().join("occupied.ketchup");
    let replay_destination = directory.path().join("replayed.ketchup");
    let source_bytes = lossy_legacy_document();
    std::fs::write(&source, &source_bytes).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    let active_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(
        shell.app().has_review_candidate(),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&source)
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(directory.path())
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);

    std::fs::write(&source, b"tampered after review").unwrap();
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);

    std::fs::File::create(&source)
        .unwrap()
        .set_len(1024 * 1024)
        .unwrap();
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    std::fs::write(&source, &source_bytes).unwrap();

    let occupied_bytes = b"preserve existing migration destination";
    std::fs::write(&occupied_destination, occupied_bytes).unwrap();
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&occupied_destination)
    );
    assert_eq!(
        std::fs::read(&occupied_destination).unwrap(),
        occupied_bytes
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);

    assert!(
        shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!shell.app().has_review_candidate());
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&replay_destination)
    );
    assert!(!replay_destination.exists());
    assert_eq!(shell.app().document_path(), Some(destination.as_path()));
    assert_eq!(shell.app().document_revision(), 8);
    assert!(!shell.app().is_dirty());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(
        ketchup_model::persistence::load_file(&source)
            .unwrap()
            .disposition(),
        ketchup_model::persistence::LoadDisposition::ReviewOnly
    );
    assert_eq!(
        ketchup_model::persistence::load_file(&destination)
            .unwrap()
            .disposition(),
        ketchup_model::persistence::LoadDisposition::EditableLossless
    );
}

#[test]
fn recovered_migration_review_revalidates_the_recovery_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("recovered-legacy.ketchup");
    let recovery = source.with_extension("ketchup.recovery");
    let destination = directory.path().join("recovered-migrated.ketchup");
    std::fs::write(&source, b"corrupt primary").unwrap();
    std::fs::write(&recovery, lossy_legacy_document()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app().has_review_candidate());
    std::fs::File::create(&recovery)
        .unwrap()
        .set_len(1024 * 1024)
        .unwrap();

    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
}

#[test]
fn legacy_migration_review_rejects_a_stale_active_document_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy-stale.ketchup");
    let destination = directory.path().join("stale-migration.ketchup");
    std::fs::write(&source, lossy_legacy_document()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app().has_review_candidate());
    assert!(shell.app_mut().create_box());
    let stale_state = canonical_state(&shell);

    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
    assert_eq!(canonical_state(&shell), stale_state);
}

#[test]
fn optional_unknown_extension_survives_app_open_edit_and_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("extended.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard()
        .always_confirm_high_risk_as(7);
    let mut shell = Shell::with_dialogs(script);
    shell.click_menu_command("menu-file", AppCommand::Save);

    let snapshot = ketchup_model::persistence::load_file(&path)
        .unwrap()
        .snapshot();
    let mut sidecars = ketchup_model::persistence::ContainerData::default();
    sidecars
        .insert_extension(
            ketchup_model::persistence::ExtensionEntry::new(
                "org.example.optional",
                "opaque.bin",
                false,
                vec![7, 8, 9],
            )
            .unwrap(),
        )
        .unwrap();
    std::fs::write(
        &path,
        ketchup_model::persistence::save_container(&snapshot, &sidecars).unwrap(),
    )
    .unwrap();

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    shell.click_menu_command("menu-file", AppCommand::Save);

    let reopened = ketchup_model::persistence::load_file(&path).unwrap();
    let extension = reopened.container_data().extensions().next().unwrap();
    assert_eq!(extension.namespace(), "org.example.optional");
    assert_eq!(extension.path(), "opaque.bin");
    assert_eq!(extension.bytes(), &[7, 8, 9]);
    assert!(!extension.required());
}

#[test]
fn save_writes_to_the_known_path_without_asking_again() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resaved.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .always_confirm_high_risk_as(8);
    let mut shell = Shell::with_dialogs(script.clone());

    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(path.is_file());

    shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0));
    shell.settle();
    let edited_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Save);

    assert_eq!(
        script.suggested_names().len(),
        1,
        "a document with a path must be saved without a second dialog"
    );
    let on_disk = ketchup_model::persistence::load_file(&path)
        .expect("the saved document reloads")
        .snapshot()
        .canonical_digest();
    assert_eq!(
        on_disk, edited_digest,
        "Save must persist the current state of the document"
    );
    assert!(!shell.app().is_dirty());
}

/// An open from the `.recovery` backup whose Undo history no longer opens
/// names both (review 2026-10-15 S1).
#[test]
fn gui_open_of_a_backup_without_its_history_warns_of_both() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("damaged.ketchup");
    let backup = requested.with_extension("ketchup.recovery");
    let unreadable_history = b"KETCHUPHST\x05\x00".to_vec();
    std::fs::write(
        &backup,
        ketchup_model::testing::save_with_history_entry(
            &ketchup_model::document::DocumentStore::new(),
            unreadable_history,
        ),
    )
    .unwrap();
    std::fs::write(&requested, b"corrupt primary").unwrap();

    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&requested));
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(digest_starts_like(&shell, "digest-opened-recovery"));
    assert!(digest_has(&shell, "digest-opened-without-history"));
    assert_eq!(shell.app().recovery_source_path(), Some(backup.as_path()));
}

#[test]
fn dirty_gui_document_recovers_after_crash_and_requires_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("crashed.ketchup");
    let recovered_copy = directory.path().join("recovered.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    author.click_menu_command("menu-file", AppCommand::Save);
    let clean_digest = author.app().canonical_digest();

    let mut crashed = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&primary));
    crashed.click_menu_command("menu-file", AppCommand::Open);
    assert!(crashed.app_mut().create_box());
    crashed.settle();
    let dirty_digest = crashed.app().canonical_digest();
    let dirty_undo = crashed.app().undo_step_count();
    assert!(ketchup_model::persistence::work_recovery_path(&primary).is_file());
    drop(crashed);

    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&primary)
        .queue_save(&recovered_copy);
    let mut recovered = Shell::with_dialogs(dialogs);
    recovered.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(recovered.app().canonical_digest(), dirty_digest);
    assert_eq!(recovered.app().undo_step_count(), dirty_undo);
    assert_eq!(recovered.app().document_path(), None);
    assert!(recovered.app().is_dirty());
    // The file is intact: the warning names the unsaved work it restored, not a
    // damaged file (review 2026-10-15 S1).
    assert!(digest_starts_like(
        &recovered,
        "digest-opened-work-recovery"
    ));
    assert!(!digest_has(&recovered, "digest-opened-recovery"));
    assert_eq!(
        recovered.app().recovery_source_path(),
        Some(ketchup_model::persistence::work_recovery_path(&primary).as_path())
    );
    assert_eq!(
        ketchup_model::persistence::load(&std::fs::read(&primary).unwrap())
            .unwrap()
            .snapshot()
            .canonical_digest(),
        clean_digest
    );

    recovered.click_menu_command("menu-file", AppCommand::Save);
    assert_eq!(
        recovered.app().document_path(),
        Some(recovered_copy.as_path())
    );
    assert!(!recovered.app().is_dirty());
    assert!(!ketchup_model::persistence::work_recovery_path(&primary).exists());
}

/// Review 2026-10-17 M-7: a window opened from a work-recovery checkpoint
/// wrote no more checkpoints, so a second crash lost every edit since the open.
#[test]
fn a_window_opened_from_a_work_recovery_checkpoint_keeps_checkpointing() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("crashed.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    author.click_menu_command("menu-file", AppCommand::Save);
    let mut crashed = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&primary));
    crashed.click_menu_command("menu-file", AppCommand::Open);
    assert!(crashed.app_mut().create_box());
    crashed.settle();
    drop(crashed);

    let mut recovered = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&primary));
    recovered.click_menu_command("menu-file", AppCommand::Open);
    assert!(recovered.app().recovery_source_path().is_some());
    assert!(recovered.app_mut().create_box());
    recovered.settle();
    let second_crash = recovered.app().canonical_digest();
    let second_undo = recovered.app().undo_step_count();
    // Back at the state it was opened in, the checkpoint still holds that work:
    // the file beside it does not.
    assert!(recovered.app_mut().undo());
    recovered.settle();
    assert!(ketchup_model::persistence::work_recovery_path(&primary).is_file());
    assert!(recovered.app_mut().redo());
    recovered.settle();
    drop(recovered);

    let mut reopened = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&primary));
    reopened.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(reopened.app().canonical_digest(), second_crash);
    assert_eq!(reopened.app().undo_step_count(), second_undo);
}

/// A damaged work-recovery checkpoint beside the file is moved aside and named
/// on open, and the first edit is checkpointed instead of being refused.
#[test]
fn gui_open_sets_an_unusable_work_recovery_aside_and_keeps_editing() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("damaged-checkpoint.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&primary);
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    author.click_menu_command("menu-file", AppCommand::Save);
    drop(author);
    std::fs::write(&recovery, b"KETCHUPWRK damaged").unwrap();

    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&primary));
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(digest_starts_like(
        &shell,
        "digest-opened-work-recovery-set-aside"
    ));
    let mut set_aside = recovery.as_os_str().to_owned();
    set_aside.push(".stale");
    assert_eq!(
        std::fs::read(&set_aside).unwrap(),
        b"KETCHUPWRK damaged".to_vec()
    );
    assert!(shell.app_mut().create_box());
    shell.settle();
    assert!(shell.app().is_dirty());
    assert!(recovery.is_file());
}

#[test]
fn gui_save_rejects_replaced_work_recovery_before_publication() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("retry-save-cleanup.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&primary);
    let mut shell = Shell::with_dialogs(
        ScriptedFileDialogs::new()
            .queue_save(&primary)
            .always_confirm_high_risk_as(8),
    );
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(shell.app_mut().create_box());
    shell.settle();
    let owned_checkpoint = std::fs::read(&recovery).unwrap();

    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    let primary_before = std::fs::read(&primary).unwrap();
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(shell.app().is_dirty());
    assert_eq!(std::fs::read(&primary).unwrap(), primary_before);
    assert!(digest_starts_like(&shell, "error-save-document"));
    std::fs::remove_dir(&recovery).unwrap();
    std::fs::write(&recovery, owned_checkpoint).unwrap();
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!recovery.exists());
}

#[test]
fn gui_undo_and_redo_roll_back_when_work_recovery_checkpoint_fails() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("transactional-undo.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&primary);
    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().create_box());
    shell.settle();
    assert!(recovery.is_file());

    let before = canonical_state(&shell);
    let checkpoint = std::fs::read(&recovery).unwrap();
    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-save-document"));

    std::fs::remove_dir(&recovery).unwrap();
    std::fs::write(&recovery, checkpoint).unwrap();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(recovery.is_file());
    let undone = canonical_state(&shell);
    assert_eq!(
        ketchup_model::persistence::load_file(&primary)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        undone.digest
    );
    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(canonical_state(&shell), undone);
    assert!(digest_starts_like(&shell, "error-save-document"));
}

#[test]
fn gui_published_checkpoint_error_preserves_edit_undo_redo_and_recovery() {
    use ketchup_model::persistence::headless_with_parent_sync_failure;
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("published-checkpoint.ketchup");
    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(shell.app_mut().create_box());
    let first_edit = shell.app().canonical_digest();
    assert!(!headless_with_parent_sync_failure(|| shell
        .app_mut()
        .create_box()));
    let edited = shell.app().canonical_digest();
    assert_ne!(edited, first_edit);
    assert!(shell.app().action_digest().contains("published"));
    assert!(shell.app().is_dirty());
    assert_eq!(
        ketchup_model::persistence::load_file(&primary)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        edited
    );
    assert!(!headless_with_parent_sync_failure(|| shell
        .app_mut()
        .undo()));
    assert_eq!(shell.app().canonical_digest(), first_edit);
    assert!(shell.app().action_digest().contains("published"));
    assert!(!headless_with_parent_sync_failure(|| shell
        .app_mut()
        .redo()));
    assert_eq!(shell.app().canonical_digest(), edited);
    assert!(shell.app().action_digest().contains("published"));
    drop(shell);
    let mut recovered = Shell::with_dialogs(ScriptedFileDialogs::new());
    assert!(recovered.app_mut().open_document_path(&primary));
    assert_eq!(recovered.app().canonical_digest(), edited);
}

#[test]
fn gui_published_save_as_error_keeps_binding_and_allows_next_save() {
    use ketchup_model::persistence::headless_with_parent_sync_failure;
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("published-save.ketchup");
    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    let expected = shell.app().canonical_digest();
    headless_with_parent_sync_failure(|| shell.click_menu_command("menu-file", AppCommand::SaveAs));
    assert_eq!(shell.app().document_path(), Some(primary.as_path()));
    assert_eq!(
        ketchup_model::persistence::load_file(&primary)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        expected
    );
    assert!(shell.app().action_digest().contains("published"));
    assert!(!shell.app().is_dirty());
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!shell.app().is_dirty());
}

#[test]
fn gui_canonical_edit_rolls_back_when_work_recovery_checkpoint_fails() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("transactional-edit.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&primary);
    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    shell.click_menu_command("menu-file", AppCommand::Save);
    let before = canonical_state(&shell);

    std::fs::create_dir(&recovery).unwrap();
    assert!(!shell.app_mut().create_box());
    shell.settle();

    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-save-document"));
}

#[test]
fn step_import_publishes_its_blob_in_the_same_work_recovery_transaction() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("step-import-recovery.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&primary);
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&primary)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let baseline = canonical_state(&shell);

    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert_eq!(canonical_state(&shell), baseline);
    std::fs::remove_dir(&recovery).unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));

    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(recovery.is_file());
    let recovered = ketchup_model::persistence::load_file_with_source(&primary).unwrap();
    assert_eq!(recovered.source_path(), recovery);
    assert_eq!(recovered.outcome().snapshot().import_receipts().count(), 1);
}

#[test]
fn gui_recovery_owner_rejects_second_writer_open_and_save_but_allows_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let shared = directory.path().join("shared-recovery.ketchup");
    let alternate = directory.path().join("first-copy.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&shared);
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&shared));
    author.click_menu_command("menu-file", AppCommand::Save);

    let mut first = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&shared));
    let mut second = Shell::with_dialogs(
        ScriptedFileDialogs::new()
            .queue_open(&shared)
            .queue_save(&alternate),
    );
    first.click_menu_command("menu-file", AppCommand::Open);
    second.click_menu_command("menu-file", AppCommand::Open);
    assert!(first.app_mut().create_box());
    first.settle();
    let first_digest = first.app().canonical_digest();
    let checkpoint = std::fs::read(&recovery).unwrap();
    let second_before = canonical_state(&second);
    assert!(!second.app_mut().create_box());
    assert_eq!(canonical_state(&second), second_before);
    assert!(digest_starts_like(&second, "error-save-document"));
    assert!(!second.app_mut().open_document_path(&shared));
    assert!(!first.app_mut().open_document_path(&shared));
    second.click_menu_command("menu-file", AppCommand::Save);
    assert!(digest_starts_like(&second, "error-save-document"));
    assert_eq!(std::fs::read(&recovery).unwrap(), checkpoint);
    second.click_menu_command("menu-file", AppCommand::SaveAs);
    assert_eq!(second.app().document_path(), Some(alternate.as_path()));
    assert!(second.app_mut().create_box());
    second.click_menu_command("menu-file", AppCommand::Save);
    assert_eq!(std::fs::read(&recovery).unwrap(), checkpoint);
    drop(first); // Simulate process exit: release ownership, not the checkpoint.
    let mut recovered = Shell::with_dialogs(ScriptedFileDialogs::new());
    assert!(recovered.app_mut().open_document_path(&shared));
    assert_eq!(recovered.app().canonical_digest(), first_digest);
    assert!(recovered.app().is_dirty());
    assert!(!author.app_mut().open_document_path(&shared));
}

#[test]
fn two_open_windows_detect_external_save_and_offer_safe_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let shared = directory.path().join("shared.ketchup");
    let alternate = directory.path().join("second-copy.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&shared));
    author.click_menu_command("menu-file", AppCommand::Save);

    let first_dialogs = ScriptedFileDialogs::new()
        .queue_open(&shared)
        .always_confirm_high_risk_as(31);
    let second_dialogs = ScriptedFileDialogs::new()
        .queue_open(&shared)
        .queue_save(&alternate)
        .always_confirm_high_risk_as(32);
    let mut first = Shell::with_dialogs(first_dialogs);
    let mut second = Shell::with_dialogs(second_dialogs.clone());
    first.click_menu_command("menu-file", AppCommand::Open);
    second.click_menu_command("menu-file", AppCommand::Open);

    assert!(first.app_mut().create_box());
    first.click_menu_command("menu-file", AppCommand::Save);
    let first_bytes = std::fs::read(&shared).unwrap();
    let first_digest = first.app().canonical_digest();

    let stale_state = canonical_state(&second);
    assert!(!second.app_mut().create_box());

    assert_eq!(std::fs::read(&shared).unwrap(), first_bytes);
    assert!(!second.app().is_dirty());
    assert!(digest_starts_like(&second, "error-save-document"));
    assert!(
        second
            .app()
            .action_digest()
            .contains("changed outside this session")
    );
    assert_eq!(canonical_state(&second), stale_state);
    assert_eq!(
        second_dialogs.high_risk_prompts().len(),
        0,
        "a stale file must be rejected before requesting overwrite consent"
    );

    second.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(!second.app().is_dirty());
    assert_eq!(second.app().document_path(), Some(alternate.as_path()));
    assert!(second.app_mut().create_box());
    second.click_menu_command("menu-file", AppCommand::Save);
    let independent_digest = second.app().canonical_digest();
    assert_eq!(
        ketchup_model::persistence::load_file(&shared)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        first_digest
    );
    assert_eq!(
        ketchup_model::persistence::load_file(&alternate)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        independent_digest
    );
}

/// Grows a document to the largest earlier revision a history keeps (32 MiB), so
/// it runs with the large-house tests.
#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_save_as_requires_explicit_consent_before_preserving_current_revision_without_full_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let boundary = directory.path().join("history-boundary.ketchup");
    let saved = directory.path().join("current-only.ketchup");
    // About 31 MiB of names; node 1's name then sets how large the document is.
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(
            (1..=32_u64)
                .map(|id| CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(id),
                    name: if id == 1 {
                        "history counter".to_owned()
                    } else {
                        format!("{id:04}{}", "x".repeat(1024 * 1024 - 8))
                    },
                    dimension: Dimension::new("0", 0.0).unwrap(),
                    dependencies: Vec::new(),
                })
                .collect(),
        ))
        .unwrap();
    let container = ketchup_model::persistence::ContainerData::default();
    let save = |document: &DocumentStore| match ketchup_model::persistence::save_document_store(
        document, &container,
    ) {
        Ok(bytes) => Some(bytes),
        Err(ketchup_model::persistence::PersistenceError::ResourceLimit) => None,
        Err(error) => panic!("unexpected save error {error:?}"),
    };
    let rename = |document: &mut DocumentStore, len: usize| {
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::RenameEvaluatorNode {
                    id: NodeId(1),
                    name: "n".repeat(len),
                },
            ]))
            .unwrap();
    };
    // Whether the document renamed to `len` still fits once an edit makes it an
    // earlier revision; the file written is the one without that edit.
    let fits_as_earlier = |document: &mut DocumentStore, len: usize| {
        rename(document, len);
        let written = save(document).expect("the current revision has no size limit");
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(2),
                    dimension: Dimension::new("1", 1.0).unwrap(),
                },
            ]))
            .unwrap();
        let fits = save(document).is_some();
        assert!(document.undo().is_some());
        let undo_steps = document.visible_undo_steps();
        assert!(document.undo().is_some());
        fits.then_some((written, undo_steps))
    };
    let (mut fits, mut outgrows) = (1, 1024 * 1024 - 8);
    let mut fitting = fits_as_earlier(&mut document, fits);
    assert!(
        fitting.is_some(),
        "about 31 MiB fits as an earlier revision"
    );
    assert!(
        fits_as_earlier(&mut document, outgrows).is_none(),
        "over 32 MiB does not"
    );
    // The current revision then comes to within 1 KiB of the limit, so any
    // edit of the opened file makes it an earlier revision that still fits,
    // and a few more one that does not.
    while outgrows - fits > 1024 {
        let len = usize::midpoint(fits, outgrows);
        match fits_as_earlier(&mut document, len) {
            Some(written) => {
                fits = len;
                fitting = Some(written);
            }
            None => outgrows = len,
        }
    }
    let (fitting, fitting_undo_steps) = fitting.unwrap();
    std::fs::write(&boundary, fitting).unwrap();

    // A box adds some hundred bytes to the document; under 1 KiB is left, so a
    // few boxes outgrow it.
    const BOXES: usize = 16;
    let mut script = ScriptedFileDialogs::new()
        .queue_open(&boundary)
        .queue_open(&saved);
    for _ in 0..BOXES + 1 {
        script = script.queue_save(&saved);
    }
    let script = script
        .queue_history_truncation_approval(false)
        .queue_history_truncation_approval(true);
    let mut shell = Shell::with_dialogs(script.clone());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().undo_step_count(), fitting_undo_steps);
    let mut boxes = 0;
    let (expected_digest, expected_revision, expected_undo_steps, previous_save) = loop {
        assert!(boxes < BOXES, "{BOXES} boxes never outgrew the file");
        assert!(
            shell.app_mut().create_box(),
            "box {boxes}: {}",
            shell.app().action_digest()
        );
        boxes += 1;
        shell.settle();
        let expected = (
            shell.app().canonical_digest(),
            shell.app().document_revision(),
            shell.app().undo_step_count(),
            std::fs::read(&saved).ok(),
        );
        shell.click_menu_command("menu-file", AppCommand::SaveAs);
        if !script.history_truncation_prompts().is_empty() {
            break expected;
        }
        assert!(saved.is_file(), "a history that fits saves whole");
        assert!(!shell.app().is_dirty());
    };

    assert_eq!(
        std::fs::read(&saved).ok(),
        previous_save,
        "a refused Save As writes nothing"
    );
    assert!(shell.app().is_dirty());
    assert_eq!(shell.app().canonical_digest(), expected_digest);
    assert_eq!(shell.app().undo_step_count(), expected_undo_steps);
    assert_eq!(script.history_truncation_prompts().len(), 1);
    assert!(script.history_truncation_prompts()[0].contains(&expected_undo_steps.to_string()));

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    assert!(!shell.app().is_dirty());
    assert_eq!(shell.app().canonical_digest(), expected_digest);
    assert_eq!(shell.app().document_revision(), expected_revision);
    assert_eq!(shell.app().undo_step_count(), 0);
    assert_eq!(shell.app().redo_step_count(), 0);
    assert_eq!(script.history_truncation_prompts().len(), 2);
    assert!(digest_starts_like(
        &shell,
        "digest-saved-document-current-only"
    ));

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), expected_digest);
    assert_eq!(shell.app().document_revision(), expected_revision);
    assert_eq!(shell.app().undo_step_count(), 0);
    assert_eq!(shell.app().redo_step_count(), 0);
    assert!(!shell.app().is_dirty());
    assert!(shell.app_mut().create_box());
    assert!(shell.app().document_revision() > expected_revision);
}

#[test]
fn overwrite_save_requires_payload_bound_human_receipt_before_disk_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.ketchup");
    let original = b"existing file must survive refusal".to_vec();
    std::fs::write(&path, &original).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_save(&path)
        .queue_refused_high_risk()
        .queue_high_risk_approval(42);
    let mut shell = Shell::with_dialogs(script.clone());
    compose_two_shared_occurrences(&mut shell);
    let canonical = shell.app().canonical_digest();
    let revision = shell.app().document_revision();

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(shell.app().is_dirty());
    assert!(shell.app().last_side_effect_receipt().is_none());

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let receipt = shell
        .app()
        .last_side_effect_receipt()
        .expect("approved overwrite returns an authorization receipt");
    assert_eq!(receipt.approving_human(), 42);
    assert_eq!(receipt.revision_id(), revision);
    assert_eq!(receipt.operation(), "overwrite-native-document");
    assert_eq!(
        receipt.scope().path(),
        Some(path.display().to_string().as_str())
    );
    assert_eq!(shell.app().canonical_digest(), canonical);
    assert_eq!(shell.app().document_revision(), revision);
    assert!(!shell.app().is_dirty());
    assert_eq!(script.high_risk_prompts().len(), 2);
    assert!(script.high_risk_prompts()[0].contains("Payload SHA-256:"));
    assert_eq!(
        ketchup_model::persistence::load_file(&path)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        canonical
    );
}

#[test]
fn saving_the_opened_document_again_asks_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("own.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&path));
    author.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new().queue_open(&path);
    let mut shell = Shell::with_dialogs(script.clone());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!shell.app().is_dirty(), "{}", shell.app().action_digest());
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-file", AppCommand::Save);

    assert!(!shell.app().is_dirty(), "{}", shell.app().action_digest());
    assert!(script.high_risk_prompts().is_empty());
    assert_eq!(
        ketchup_model::persistence::load_file(&path)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        shell.app().canonical_digest()
    );
}

#[test]
fn save_as_rejects_a_target_changed_after_overwrite_approval() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.ketchup");
    std::fs::write(&path, b"original external file").unwrap();
    let concurrent = b"concurrent external update".to_vec();
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_high_risk_approval_after_write(42, &path, concurrent.clone());
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    assert_eq!(std::fs::read(&path).unwrap(), concurrent);
    assert!(shell.app().is_dirty());
    assert!(digest_starts_like(&shell, "error-save-document"));
}

#[test]
fn a_failed_open_leaves_the_active_document_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.ketchup");
    std::fs::write(&malformed, b"not a ketchup document").unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&malformed)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    let before = shell.app().canonical_digest();
    let revision = shell.app().document_revision();

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(
        shell.app().canonical_digest(),
        before,
        "a failed Open must not replace the active document"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().document_path(), None);
    assert!(
        digest_starts_like(&shell, "error-open-document"),
        "a failed Open must report the localized open error, digest was {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn a_failed_save_keeps_the_document_dirty_and_reports_the_reason() {
    let directory = tempfile::tempdir().unwrap();
    let script = ScriptedFileDialogs::new().queue_save(directory.path());
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    let before = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Save);

    assert_eq!(shell.app().canonical_digest(), before);
    assert!(
        shell.app().is_dirty(),
        "a failed Save must leave the document unsaved"
    );
    assert_eq!(shell.app().document_path(), None);
    assert!(
        digest_starts_like(&shell, "error-save-document"),
        "a failed Save must report the localized save error, digest was {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn discarding_unsaved_work_is_confirmed_before_new_replaces_it() {
    let refused = ScriptedFileDialogs::new();
    let mut shell = Shell::with_dialogs(refused.clone());
    compose_two_shared_occurrences(&mut shell);
    let composed = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::New);

    assert_eq!(refused.discard_prompts(), 1, "the shell must ask first");
    assert_eq!(
        shell.app().canonical_digest(),
        composed,
        "a refused prompt must keep the composed model"
    );
    assert_eq!(shell.app().active_box_count(), 2);
}

#[test]
fn file_menu_exports_fail_closed_when_current_exact_evidence_is_unavailable() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let stl = directory.path().join("preserved.stl");
    let stl_loss = stl.with_extension("stl.loss.txt");
    let step = directory.path().join("preserved.step");
    let step_loss = step.with_extension("step.loss.txt");
    let unavailable_worker = directory.path().join("not-an-exact-worker");
    let original_stl = b"preserve STL when exact evidence is unavailable";
    let original_stl_loss = b"preserve STL loss report when exact evidence is unavailable";
    let original_step = b"preserve STEP when exact evidence is unavailable";
    let original_step_loss = b"preserve STEP loss report when exact evidence is unavailable";
    std::fs::write(&stl, original_stl).unwrap();
    std::fs::write(&stl_loss, original_stl_loss).unwrap();
    std::fs::write(&step, original_step).unwrap();
    std::fs::write(&step_loss, original_step_loss).unwrap();
    std::fs::write(&unavailable_worker, b"not an executable worker").unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_export(&stl)
        .queue_export(&step)
        .always_confirm_high_risk_as(91);
    let mut shell = Shell::with_dialogs(script.clone());
    shell
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    shell.settle();
    assert_eq!(shell.app().exact_render_body_count(), 0);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);

    assert_eq!(std::fs::read(&stl).unwrap(), original_stl);
    assert_eq!(std::fs::read(&stl_loss).unwrap(), original_stl_loss);
    assert_eq!(std::fs::read(&step).unwrap(), original_step);
    assert_eq!(std::fs::read(&step_loss).unwrap(), original_step_loss);
    assert!(
        script.high_risk_prompts().is_empty(),
        "missing current evidence must be rejected before lossy confirmation"
    );
    assert!(shell.app().last_side_effect_receipt().is_none());
}

#[test]
fn file_menu_exports_current_visible_exact_model_without_mutating_canonical_state() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let protected_stl = directory.path().join("protected.stl");
    let protected_loss = protected_stl.with_extension("stl.loss.txt");
    let first_stl = directory.path().join("visible-a.stl");
    let second_stl = directory.path().join("visible-b.stl");
    let step = directory.path().join("visible.step");
    let script = ScriptedFileDialogs::new()
        .queue_cancelled_export()
        .queue_cancelled_export()
        .queue_export(&protected_stl)
        .queue_export(&protected_stl)
        .queue_export(&first_stl)
        .queue_export(&second_stl)
        .queue_export(&step)
        .queue_refused_high_risk()
        .queue_high_risk_approval(40)
        .queue_high_risk_approval(41)
        .queue_high_risk_approval(42)
        .queue_high_risk_approval(43)
        .queue_high_risk_approval(44)
        .queue_high_risk_approval(45);
    let mut shell = Shell::with_dialogs(script.clone());
    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        0,
        "cancelling either export dialog must not create an artifact"
    );
    assert!(script.high_risk_prompts().is_empty());

    let original_stl = b"existing STL must survive refusal";
    let original_loss = b"existing STL loss report must survive refusal";
    std::fs::write(&protected_stl, original_stl).unwrap();
    std::fs::write(&protected_loss, original_loss).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(std::fs::read(&protected_stl).unwrap(), original_stl);
    assert_eq!(std::fs::read(&protected_loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_eq!(canonical_state(&shell), before);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_ne!(std::fs::read(&protected_stl).unwrap(), original_stl);
    assert_ne!(std::fs::read(&protected_loss).unwrap(), original_loss);
    assert_eq!(canonical_state(&shell), before);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    let first_stl_bytes = std::fs::read(&first_stl).unwrap();
    let second_stl_bytes = std::fs::read(&second_stl).unwrap();
    assert_eq!(
        first_stl_bytes, second_stl_bytes,
        "the same visible exact model must produce byte-identical STL"
    );
    assert_eq!(
        ascii_stl_facet_count(&first_stl_bytes),
        12,
        "the hidden shared occurrence must not contribute facets"
    );
    let stl_loss = std::fs::read_to_string(first_stl.with_extension("stl.loss.txt")).unwrap();
    assert!(stl_loss.contains("format=ASCII STL"));
    assert!(stl_loss.contains("editability_loss="));
    assert!(stl_loss.contains("topology_loss="));
    assert!(stl_loss.contains(&format!("source_digest={}", before.digest)));

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(
        step.is_file(),
        "STEP export failed with digest {:?}",
        shell.app().action_digest()
    );
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    let step_loss = std::fs::read_to_string(step.with_extension("step.loss.txt")).unwrap();
    assert!(step_loss.contains("format=ISO 10303 STEP"));
    assert!(step_loss.contains("editability_loss="));
    assert!(step_loss.contains(&format!("source_digest={}", before.digest)));

    let requests = script.export_requests();
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.extension.as_str())
            .collect::<Vec<_>>(),
        vec!["stl", "step", "stl", "stl", "stl", "stl", "step"]
    );
    for request in &requests {
        assert!(
            request
                .filter_label
                .to_ascii_lowercase()
                .contains(request.extension.as_str()),
            "the recorded filter must identify its format: {request:?}"
        );
        assert_eq!(
            request.suggested_name,
            format!("Untitled.{}", request.extension)
        );
    }
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 7);
    assert!(prompts[2].contains(&protected_stl.display().to_string()));
    assert!(prompts[3].contains(&protected_loss.display().to_string()));
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
}

#[test]
fn file_menu_three_mf_export_is_localized_atomic_and_non_mutating() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let three_mf = directory.path().join("protected.3mf");
    let loss = three_mf.with_extension("3mf.loss.txt");
    let original_model = b"preserve 3MF until overwrite approval";
    let original_loss = b"preserve 3MF loss report until overwrite approval";
    std::fs::write(&three_mf, original_model).unwrap();
    std::fs::write(&loss, original_loss).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_export(&three_mf)
        .queue_export(&three_mf)
        .queue_refused_high_risk()
        .queue_high_risk_approval(121)
        .queue_high_risk_approval(122)
        .queue_high_risk_approval(123);
    let mut shell = Shell::with_dialogs(script.clone());
    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportPrintThreeMf);
    assert_eq!(std::fs::read(&three_mf).unwrap(), original_model);
    assert_eq!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    shell.click_menu_command("menu-file", AppCommand::ExportPrintThreeMf);
    let package = std::fs::read(&three_mf).unwrap();
    assert_eq!(&package[..4], b"PK\x03\x04");
    assert!(
        package
            .windows(b"3D/3dmodel.model".len())
            .any(|window| window == b"3D/3dmodel.model")
    );
    assert!(
        package
            .windows(b"unit=\"millimeter\"".len())
            .any(|window| window == b"unit=\"millimeter\"")
    );
    let report = std::fs::read_to_string(&loss).unwrap();
    assert!(report.contains("format=3MF Core 1.3 package"));
    assert!(report.contains("hierarchy=canonical global groups"));
    assert!(report.contains(&format!("source_digest={}", before.digest)));
    assert!(digest_starts_like(&shell, "digest-exported-3mf"));
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    let requests = script.export_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.extension == "3mf"
            && request.filter_label.to_ascii_lowercase().contains("3mf")
            && request.suggested_name == "Untitled.3mf"
    }));
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
}

#[test]
fn file_menu_round_trips_mixed_exact_and_imported_mesh_scene_to_stl_glb_and_three_mf() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let directory = tempfile::tempdir().unwrap();
    let stl = directory.path().join("mixed.stl");
    let glb = directory.path().join("mixed.glb");
    let three_mf = directory.path().join("mixed.3mf");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Glb, &fixture)
        .queue_export(&stl)
        .queue_export(&glb)
        .queue_export(&three_mf)
        .always_confirm_high_risk_as(131);
    let mut shell = Shell::with_dialogs(script.clone());
    compose_two_shared_occurrences(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::ExportBlenderGlb);
    shell.click_menu_command("menu-file", AppCommand::ExportPrintThreeMf);

    let stl_bytes = std::fs::read(&stl).unwrap();
    assert!(ascii_stl_facet_count(&stl_bytes) > 1_000);
    let stl_report = std::fs::read_to_string(stl.with_extension("stl.loss.txt")).unwrap();
    assert!(stl_report.contains("canonical_mesh_occurrence_count=140"));
    assert!(stl_report.contains("color_loss=STL does not preserve occurrence colors"));

    let glb_bytes = std::fs::read(&glb).unwrap();
    let glb_scene = ketchup_model::import::inspect_glb(&glb_bytes).unwrap();
    assert_eq!(glb_scene.instance_count(), 142);
    let mut round_trip = DocumentStore::new();
    let batch = ketchup_model::import::plan_glb_import(
        &round_trip.current(),
        &glb_bytes,
        "mixed-round-trip.glb",
    )
    .unwrap();
    round_trip.apply_batch(&batch).unwrap();
    let round_trip_digest = round_trip.current().canonical_digest();
    let persisted = ketchup_model::persistence::save(&round_trip.current());
    assert_eq!(
        ketchup_model::persistence::load(&persisted)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        round_trip_digest
    );
    let glb_report = std::fs::read_to_string(glb.with_extension("glb.loss.txt")).unwrap();
    assert!(glb_report.contains("canonical_mesh_occurrence_count=140"));
    assert!(glb_report.contains("materials=resolved occurrence sRGB colors"));

    let three_mf_bytes = std::fs::read(&three_mf).unwrap();
    assert_eq!(&three_mf_bytes[..4], b"PK\x03\x04");
    assert!(
        three_mf_bytes
            .windows(b"displaycolor=\"#".len())
            .any(|bytes| bytes == b"displaycolor=\"#")
    );
    let three_mf_report = std::fs::read_to_string(three_mf.with_extension("3mf.loss.txt")).unwrap();
    assert!(three_mf_report.contains("canonical_mesh_occurrence_count=140"));
    assert!(three_mf_report.contains("materials=resolved occurrence sRGB colors"));

    assert_eq!(script.high_risk_prompts().len(), 3);
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_menu_step_export_requires_bound_overwrite_approval_without_canonical_mutation() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("protected.step");
    let loss = step.with_extension("step.loss.txt");
    let original_step = b"preserve STEP until overwrite approval";
    let original_loss = b"preserve STEP loss report until overwrite approval";
    std::fs::write(&step, original_step).unwrap();
    std::fs::write(&loss, original_loss).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_export(&step)
        .queue_export(&step)
        .queue_refused_high_risk()
        .queue_high_risk_approval(111)
        .queue_high_risk_approval(112)
        .queue_high_risk_approval(113);
    let mut shell = Shell::with_dialogs(script.clone());
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(std::fs::read(&step).unwrap(), original_step);
    assert_eq!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_ne!(std::fs::read(&step).unwrap(), original_step);
    assert_ne!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(&step.display().to_string()))
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(&loss.display().to_string()))
    );
}

#[test]
fn file_menu_iges_export_and_localized_import_are_bound_atomic_and_renderable() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let step_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let import_source = directory.path().join("source.iges");
    let cancelled = AtomicBool::new(false);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    inspector
        .convert_step_to_iges_with_cancellation(&step_source, &import_source, &cancelled)
        .unwrap();
    let source_bytes = std::fs::read(&import_source).unwrap();
    let source_evidence = inspector
        .inspect_iges_import_with_cancellation(
            &import_source,
            &ketchup_model::graph::sha256_hex(&source_bytes),
            &cancelled,
        )
        .unwrap();

    let exported = directory.path().join("protected.iges");
    let exported_loss = exported.with_extension("iges.loss.txt");
    let original_export = b"preserve IGES until overwrite approval";
    let original_loss = b"preserve IGES loss report until overwrite approval";
    std::fs::write(&exported, original_export).unwrap();
    std::fs::write(&exported_loss, original_loss).unwrap();
    let export_dialogs = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &step_source)
        .queue_export(&exported)
        .queue_export(&exported)
        .queue_refused_high_risk()
        .queue_high_risk_approval(201)
        .queue_high_risk_approval(202)
        .queue_high_risk_approval(203);
    let mut export_shell = Shell::with_dialogs(export_dialogs.clone());
    export_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    export_shell.click_at(export_shell.viewport_rect().center());
    assert!(export_shell.app_mut().delete_selected());
    export_shell.settle();
    export_shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    export_shell.click_button_label(&export_shell.catalog().text("dialog-import-step-confirm"));
    let export_feature = imported_exact_feature_id(&export_shell);
    for _ in 0..100 {
        export_shell.settle();
        if export_shell.app().exact_current_producer_ids() == [export_feature] {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        export_shell.app().exact_current_producer_ids(),
        [export_feature]
    );
    let export_before = canonical_state(&export_shell);
    let export_history = reachable_history_digests(&mut export_shell);
    for _ in 0..100 {
        export_shell.settle();
        if export_shell.app().exact_current_producer_ids() == [export_feature] {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        export_shell.app().exact_current_producer_ids(),
        [export_feature]
    );

    export_shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    assert_eq!(std::fs::read(&exported).unwrap(), original_export);
    assert_eq!(std::fs::read(&exported_loss).unwrap(), original_loss);
    assert_state_and_history_unchanged(&mut export_shell, &export_before, &export_history);
    for _ in 0..100 {
        export_shell.settle();
        if export_shell.app().exact_current_producer_ids() == [export_feature] {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        export_shell.app().exact_current_producer_ids(),
        [export_feature]
    );

    export_shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    let exported_bytes = std::fs::read(&exported).unwrap();
    assert_ne!(
        exported_bytes,
        original_export,
        "IGES export failed with digest {:?}",
        export_shell.app().action_digest()
    );
    let report = std::fs::read_to_string(&exported_loss).unwrap();
    assert!(report.contains("format=IGES 5.3"));
    assert!(report.contains("assembly_loss="));
    assert!(report.contains(&format!("source_digest={}", export_before.digest)));
    let exported_evidence = inspector
        .inspect_iges_import_with_cancellation(
            &exported,
            &ketchup_model::graph::sha256_hex(&exported_bytes),
            &cancelled,
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, ImportLengthUnit::Millimetre);
    assert_eq!(exported_evidence.solid_count, 1);
    assert_state_and_history_unchanged(&mut export_shell, &export_before, &export_history);
    assert_eq!(export_dialogs.high_risk_prompts().len(), 4);

    let import_dialogs = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Iges, &import_source)
        .queue_import(ImportFormat::Iges, &import_source);
    let mut import_shell = Shell::with_catalog_and_dialogs(
        ketchup_interaction::LocaleCatalog::slovak(),
        import_dialogs.clone(),
    );
    import_shell.app_mut().enable_headless_instanced_scene();
    import_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    import_shell.click_at(import_shell.viewport_rect().center());
    assert!(import_shell.app_mut().delete_selected());
    import_shell.settle();
    let import_before = canonical_state(&import_shell);
    let import_history = reachable_history_digests(&mut import_shell);

    import_shell.click_menu_command("menu-file", AppCommand::ImportExactIges);
    let confirm = import_shell.catalog().text("dialog-import-iges-confirm");
    assert!(import_shell.has_role_and_label(Role::Button, &confirm));
    std::fs::write(&import_source, b"changed after IGES preview").unwrap();
    import_shell.click_button_label(&confirm);
    assert!(digest_starts_like(&import_shell, "error-import-iges"));
    assert_state_and_history_unchanged(&mut import_shell, &import_before, &import_history);

    std::fs::write(&import_source, &source_bytes).unwrap();
    import_shell.click_menu_command("menu-file", AppCommand::ImportExactIges);
    import_shell.click_button_label(&confirm);
    assert_eq!(
        import_shell.app().document_revision(),
        import_before.revision + 1
    );
    assert_eq!(import_shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&import_shell, "digest-imported-iges"));
    let imported_snapshot = import_shell.app().document_snapshot();
    let receipt = imported_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Iges)
        .unwrap();
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    let imported_feature = imported_exact_feature_id(&import_shell);
    for _ in 0..100 {
        import_shell.settle();
        if import_shell.app().exact_current_producer_ids() == [imported_feature]
            && import_shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(import_shell.app().exact_render_body_count(), 1);
    assert!(import_shell.app().exact_render_triangle_count() > 0);
    assert_eq!(
        import_dialogs.import_requests(),
        vec![
            ketchup_app::dialogs::ImportDialogRequestRecord {
                format: ImportFormat::Iges,
                filter_label: import_shell.catalog().text("file-filter-iges"),
                extensions: vec!["iges".to_owned(), "igs".to_owned()],
            },
            ketchup_app::dialogs::ImportDialogRequestRecord {
                format: ImportFormat::Iges,
                filter_label: import_shell.catalog().text("file-filter-iges"),
                extensions: vec!["iges".to_owned(), "igs".to_owned()],
            },
        ]
    );
}

#[test]
fn file_import_sketchup_scene_preserves_shared_instances_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let source = valid_sketchup_scene_bridge();
    let path = directory.path().join("shared.kscene");
    let document_path = directory.path().join("shared.ketchup");
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::SketchupScene, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportSketchupScene);
    shell.click_button_label(&shell.catalog().text("dialog-import-sketchup-scene-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 2);
    assert_eq!(shell.app().mesh_body_count(), before.mesh_bodies + 1);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-sketchup-scene"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::SketchupScene,
            filter_label: shell.catalog().text("file-filter-sketchup-scene"),
            extensions: vec!["kscene".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    let loaded = ketchup_model::persistence::load_file(&document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::SketchupScene);
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Inch);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert_eq!(snapshot.definitions().count(), before.definitions + 1);
    assert_eq!(snapshot.occurrences().count(), before.occurrences + 2);
    let imported_mesh = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh)
                if matches!(
                    mesh.authority,
                    ketchup_model::document::MeshAuthority::ImportedSketchupScene { .. }
                ) =>
            {
                Some(mesh)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(imported_mesh.vertices_mm[1], [25.4, 0.0, 0.0]);
}

#[test]
fn file_import_blender_glb_reviews_and_commits_one_mesh_scene_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let source = std::fs::read(&source_path).unwrap();
    let review = ketchup_model::import::inspect_glb(&source).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Glb, &source_path);
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(
        shell.app().definition_count(),
        before.definitions + review.mesh_primitive_count()
    );
    assert_eq!(
        shell.app().occurrence_count(),
        before.occurrences + review.instance_count()
    );
    assert_eq!(
        shell.app().mesh_body_count(),
        before.mesh_bodies + review.mesh_primitive_count()
    );
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-glb"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Glb,
            filter_label: shell.catalog().text("file-filter-glb"),
            extensions: vec!["glb".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .import_receipts()
            .find(|receipt| receipt.format() == ImportFormat::Glb)
            .unwrap()
            .source_sha256(),
        &sha256_bytes(&source)
    );
}

#[test]
fn file_import_blender_glb_cancel_source_change_and_stale_review_do_not_mutate() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let original = std::fs::read(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.glb");
    std::fs::write(&source, &original).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Glb, &source)
        .queue_import(ImportFormat::Glb, &source)
        .queue_import(ImportFormat::Glb, &source);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    let mut replacement = original.clone();
    replacement[0] ^= 1;
    std::fs::write(&source, replacement).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    assert!(digest_starts_like(&shell, "error-import-glb"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    std::fs::write(&source, original).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    assert!(digest_starts_like(&shell, "error-import-glb"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);
}

#[test]
fn blender_mesh_exact_conversion_requires_review_and_one_confirmed_undo_step() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Glb, &source);
    let mut shell = Shell::with_dialogs(script);
    shell
        .app_mut()
        .headless_force_exact_worker_path(exact_worker_path());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    let snapshot = shell.app().document_snapshot();
    let recognized = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh)
                if matches!(
                    recognize_mesh_body(mesh, 0.25),
                    MeshRecognition::Candidate { .. }
                ) =>
            {
                Some((feature.id(), feature.definition_id()))
            }
            _ => None,
        })
        .expect("the real Blender fixture must contain a supported simple mesh");
    let occurrence_id = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == recognized.1)
        .expect("the recognized Blender mesh must have a placed occurrence")
        .id();
    drop(snapshot);
    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    let imported = canonical_state(&shell);
    let imported_history = reachable_history_digests(&mut shell);

    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::ConvertSelectedMeshToExact);
    assert_eq!(canonical_state(&shell), imported);
    assert!(
        shell.has_visible_label(&shell.catalog().text("dialog-mesh-conversion-title")),
        "{}",
        shell.app().action_digest()
    );
    shell.click_button_label(&shell.catalog().text("dialog-mesh-conversion-cancel"));
    assert_state_and_history_unchanged(&mut shell, &imported, &imported_history);

    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::ConvertSelectedMeshToExact);
    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().undo());
    let after_aba = canonical_state(&shell);
    let after_aba_history = reachable_history_digests(&mut shell);
    assert!(!shell.has_visible_label(&shell.catalog().text("dialog-mesh-conversion-confirm")));
    assert_state_and_history_unchanged(&mut shell, &after_aba, &after_aba_history);

    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::ConvertSelectedMeshToExact);
    assert_eq!(canonical_state(&shell), after_aba);
    let confirm_label = shell.catalog().text("dialog-mesh-conversion-confirm");
    wait_for_visible_label(&mut shell, &confirm_label);
    shell.click_button_label(&confirm_label);
    assert!(shell.app().document_revision() > after_aba.revision);
    assert_eq!(shell.app().mesh_body_count(), after_aba.mesh_bodies - 1);
    assert_eq!(shell.app().undo_step_count(), after_aba.undo_steps + 1);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(recognized.0)
            .expect("converted feature keeps the source feature ID")
            .kind(),
        FeatureKind::Profile { .. } | FeatureKind::Workplane(_)
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), after_aba.digest);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(recognized.0)
            .expect("Undo restores the source mesh")
            .kind(),
        FeatureKind::MeshBody(_)
    ));
}

#[test]
fn imported_blender_mesh_blocks_exact_and_btlx_exports_before_side_effects() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("blocked.step");
    let btlx = directory.path().join("blocked.btlx");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Glb, &source)
        .queue_export(&step)
        .queue_export(&btlx)
        .always_confirm_high_risk_as(91);
    let mut shell = Shell::with_dialogs(script.clone());

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    let imported = canonical_state(&shell);
    let imported_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert!(digest_starts_like(&shell, "error-export-step"));
    assert!(
        shell.app().action_digest().contains("mesh body"),
        "{}",
        shell.app().action_digest()
    );
    assert!(!step.exists());
    assert_state_and_history_unchanged(&mut shell, &imported, &imported_history);

    shell.click_menu_command("menu-file", AppCommand::ExportHundeggerBtlx);
    assert!(digest_starts_like(&shell, "error-export-btlx"));
    assert!(shell.app().action_digest().contains("mesh body"));
    assert!(!btlx.exists());
    assert!(!btlx.with_extension("btlx.support.txt").exists());
    assert!(script.high_risk_prompts().is_empty());
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &imported, &imported_history);
}

#[test]
fn file_import_sketchup_scene_rejects_source_substitution_after_review() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed.kscene");
    std::fs::write(&path, valid_sketchup_scene_bridge()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::SketchupScene, &path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportSketchupScene);
    std::fs::write(&path, b"{}" as &[u8]).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-sketchup-scene-confirm"));

    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-import-sketchup-scene"));
}

#[test]
fn file_import_stl_commits_one_canonical_mesh_transaction_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron.stl");
    let document_path = directory.path().join("imported.ketchup");
    std::fs::write(&path, valid_ascii_tetrahedron()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);
    let before_definitions = shell.app().definition_count();

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));

    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert_eq!(shell.app().definition_count(), before_definitions + 1);
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert!(shell.app().can_undo());
    assert!(digest_starts_like(&shell, "digest-imported-stl"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Stl,
            filter_label: shell.catalog().text("file-filter-stl"),
            extensions: vec!["stl".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().mesh_body_count(), 0);
    assert_eq!(shell.app().import_receipt_count(), 0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);

    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!shell.app().is_dirty());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert_persisted_stl(
        &document_path,
        valid_ascii_tetrahedron(),
        ImportLengthUnit::Millimetre,
        "stl.ascii",
    );
}
