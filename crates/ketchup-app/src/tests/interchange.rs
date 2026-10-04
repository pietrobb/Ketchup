//! Import and export: STL, DXF, STEP, SketchUp, GLB, BTLX, MPR and fabrication.

use super::*;

#[test]
fn homag_mpr_program_names_are_exactly_twelve_machine_safe_characters() {
    assert_eq!(
        KetchupApp::homag_mpr_program_name("door-side"),
        "DOOR-SIDE000"
    );
    assert_eq!(
        KetchupApp::homag_mpr_program_name("123456789012345"),
        "123456789012"
    );
    assert_eq!(
        KetchupApp::homag_mpr_program_name("čelo dverí"),
        "ELODVER00000"
    );
    assert!(KetchupApp::validate_homag_mpr_path(Path::new("123456789012.mpr")).is_ok());
    assert!(KetchupApp::validate_homag_mpr_path(Path::new("ABCDEF_12345.MPR")).is_ok());
    assert!(KetchupApp::validate_homag_mpr_path(Path::new("short.mpr")).is_err());
    assert!(KetchupApp::validate_homag_mpr_path(Path::new("123456789012.nc")).is_err());
    assert!(KetchupApp::validate_homag_mpr_path(Path::new("12345678901!.mpr")).is_err());
}

#[test]
fn homag_mpr_file_command_requests_a_twelve_character_mpr_name() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("UNTITLED0000.mpr");
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_export(&path);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));

    app.dispatch_file_command(AppCommand::ExportHomagMpr);

    assert_eq!(script.suggested_names(), vec!["UNTITLED0000.mpr"]);
    let requests = script.export_requests();
    let [request] = requests.as_slice() else {
        panic!("the HOMAG command must open one export dialog");
    };
    assert_eq!(request.extension, "mpr");
    assert_eq!(request.suggested_name, "UNTITLED0000.mpr");
    assert!(!path.exists());
}

#[test]
fn only_body_producing_features_are_export_candidates() {
    assert!(!FeatureKind::polygon(&[]).produces_body());
    assert!(
        FeatureKind::extrusion(FeatureId(1), Dimension::new("1", 1.0).unwrap()).produces_body()
    );
}

#[test]
fn blender_glb_file_command_exports_current_scene_with_loss_report() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("current-model.glb");
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_export(&path)
        .always_confirm_high_risk_as(83);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    install_initial_graph_result(&mut app);
    let revision = app.document.current().revision_id();
    let digest = app.document.current().canonical_digest();

    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let file_menu = harness.state().catalog.text("menu-file");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    let export = harness.state().command_label(AppCommand::ExportBlenderGlb);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    harness.run();

    assert_eq!(harness.state().document.current().revision_id(), revision);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
    let glb = std::fs::read(&path).unwrap();
    assert_eq!(&glb[0..4], b"glTF");
    assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
    let report = std::fs::read_to_string(path.with_extension("glb.loss.txt")).unwrap();
    assert!(report.contains("format=glTF 2.0 binary (GLB)"));
    assert!(report.contains("unit_conversion=millimetres to metres"));
    assert!(report.contains("axis_conversion=Ketchup Z-up to glTF Y-up"));
    assert!(report.contains("blender_background_import_verified=false"));
    assert_eq!(script.export_requests()[0].extension, "glb");
    let receipt = harness.state().last_side_effect_receipt().unwrap();
    assert_eq!(receipt.scope().class(), HighRiskClass::LossyConversion);
    assert_eq!(
        receipt.operation(),
        "export-current-model-blender-glb-with-loss-report"
    );
}

#[test]
fn sheet_metal_manufacturing_export_requires_release_and_bound_overwrite_consent() {
    use ketchup_model::sheet_metal::{SheetMetalBend, SheetMetalSpec};
    let bend = |edge, length, angle_degrees| SheetMetalBend {
        parent: None,
        edge,
        length: Dimension::from_decimal(length).unwrap(),
        angle_degrees,
        inner_radius: Dimension::from_decimal("3").unwrap(),
    };

    let directory = tempfile::tempdir().unwrap();
    let flat_pattern = directory.path().join("bracket.dxf");
    let bend_table = flat_pattern.with_extension("bends.csv");
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_export(&flat_pattern)
        .queue_export(&flat_pattern)
        .queue_export(&flat_pattern)
        .queue_refused_high_risk()
        .queue_high_risk_approval(301)
        .queue_high_risk_approval(302)
        .queue_high_risk_approval(303)
        .queue_high_risk_approval(304);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let feature_id = FeatureId(80);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(80),
                name: "Sheet metal bracket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id: DefinitionId(80),
                name: "Opposite flanges".into(),
                kind: FeatureKind::SheetMetal(SheetMetalSpec {
                    base_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]],
                    thickness: Dimension::from_decimal("2").unwrap(),
                    k_factor: 0.4,
                    bends: vec![bend(1, "30", -45.0), bend(3, "20", 90.0)],
                }),
            },
        ]))
        .unwrap();
    let revision = app.document.current().revision_id();
    let digest = app.document.current().canonical_digest();
    let undo_steps = app.undo_step_count();
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let file_menu = harness.state().catalog.text("menu-file");
    let export = harness
        .state()
        .command_label(AppCommand::ExportSheetMetalManufacturing);

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    harness.run();
    assert!(!flat_pattern.exists());
    assert!(!bend_table.exists());
    assert!(harness.state().side_effect_receipts.is_empty());

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    harness.run();
    let initial_dxf = std::fs::read(&flat_pattern).unwrap();
    let initial_bends = std::fs::read(&bend_table).unwrap();
    assert!(
        String::from_utf8_lossy(&initial_dxf).contains("ketchup.sheet-metal-flat-pattern-dxf.v1")
    );
    assert!(
        String::from_utf8_lossy(&initial_bends).contains("ketchup.sheet-metal-bend-table-csv.v1")
    );
    assert_eq!(harness.state().side_effect_receipts.len(), 1);
    assert_eq!(
        harness.state().side_effect_receipts[0].operation(),
        "release-sheet-metal-flat-pattern-and-bend-table"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    harness.run();
    assert_eq!(std::fs::read(&flat_pattern).unwrap(), initial_dxf);
    assert_eq!(std::fs::read(&bend_table).unwrap(), initial_bends);
    assert_eq!(harness.state().side_effect_receipts.len(), 3);
    assert_eq!(
        harness.state().side_effect_receipts[1].scope().class(),
        HighRiskClass::Overwrite
    );
    assert_eq!(
        harness.state().side_effect_receipts[2].scope().class(),
        HighRiskClass::Overwrite
    );
    assert_eq!(script.high_risk_prompts().len(), 5);
    assert_eq!(harness.state().document.current().revision_id(), revision);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
    assert_eq!(harness.state().undo_step_count(), undo_steps);
}

#[test]
fn general_fabrication_file_command_exports_one_authoritative_mixed_nested_package() {
    let build_app = |dialogs: dialogs::ScriptedFileDialogs| {
        let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: INITIAL_BOX_DEFINITION,
                    name: "Second leaf".to_owned(),
                    transform: Transform::from_translation(120.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
                CanonicalCommand::CreateGroup {
                    id: GroupId(100),
                    name: "Two-part subassembly".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                },
                CanonicalCommand::SetOccurrenceParent {
                    id: OccurrenceId(1),
                    parent: Some(GroupId(100)),
                },
                CanonicalCommand::SetOccurrenceParent {
                    id: OccurrenceId(2),
                    parent: Some(GroupId(100)),
                },
            ]))
            .unwrap();
        let assembly = app
            .document
            .convert_group_to_component(GroupId(100), "Reusable two-part subassembly")
            .unwrap();
        let assembly_copy = OccurrenceId(assembly.component_occurrence_id.0 + 1);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateOccurrence {
                    id: assembly_copy,
                    definition_id: assembly.component_definition_id,
                    name: "Purchased subassembly copy".to_owned(),
                    transform: Transform::from_translation(300.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
                CanonicalCommand::UpsertClassificationDimension {
                    id: ClassificationDimensionId(200),
                    name: ketchup_manufacturing::fabrication::FABRICATION_ROLE_DIMENSION_V1
                        .to_owned(),
                    categories: vec![
                        (
                            ClassificationCategoryId(201),
                            ketchup_manufacturing::fabrication::MANUFACTURED_ITEM_ROLE_V1
                                .to_owned(),
                        ),
                        (
                            ClassificationCategoryId(202),
                            ketchup_manufacturing::fabrication::PURCHASED_ITEM_ROLE_V1.to_owned(),
                        ),
                    ],
                },
                CanonicalCommand::UpsertClassificationDimension {
                    id: ClassificationDimensionId(210),
                    name: ketchup_manufacturing::fabrication::MATERIAL_DIMENSION_V1.to_owned(),
                    categories: vec![
                        (
                            ClassificationCategoryId(211),
                            "ketchup.material.steel.s355.v1".to_owned(),
                        ),
                        (
                            ClassificationCategoryId(212),
                            "ketchup.material.bearing.6202.v1".to_owned(),
                        ),
                    ],
                },
                CanonicalCommand::SetOccurrenceClassification {
                    occurrence_id: assembly.component_occurrence_id,
                    dimension_id: ClassificationDimensionId(200),
                    category_id: Some(ClassificationCategoryId(201)),
                },
                CanonicalCommand::SetOccurrenceClassification {
                    occurrence_id: assembly.component_occurrence_id,
                    dimension_id: ClassificationDimensionId(210),
                    category_id: Some(ClassificationCategoryId(211)),
                },
            ]))
            .unwrap();
        let cad_edit_program = AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::SetOccurrenceClassification {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![assembly_copy.0],
                    },
                    dimension_id: 200,
                    category_id: Some(202),
                },
                AssistantCadEditOperation::SetOccurrenceClassification {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![assembly_copy.0],
                    },
                    dimension_id: 210,
                    category_id: Some(212),
                },
            ],
        };
        app.assistant.pending_execution = Some(AssistantPendingExecution {
            cad_edit_program: Some(cad_edit_program),
            result: AssistantChatResult {
                message: "Classified the purchased subassembly for review.".to_owned(),
                model_intent: None,
            },
            message: "Classify the second subassembly as purchased bearing stock.".to_owned(),
            replan_attempted: false,
            document_id: app.document.current().document_id(),
            revision_id: app.document.current().revision_id(),
            canonical_digest: app.document.current().canonical_digest(),
            source: "test assistant transport".to_owned(),
        });
        app.poll_assistant_chat(&egui::Context::default());
        assert!(app.assistant.proposal.is_some());
        assert!(app.confirm_assistant_proposal());
        install_initial_graph_result(&mut app);
        app
    };

    let refused_directory = tempfile::tempdir().unwrap();
    let refused_path = refused_directory.path().join("mixed.csv");
    let refused_dialogs = dialogs::ScriptedFileDialogs::new().queue_export(&refused_path);
    let mut refused = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            build_app(refused_dialogs),
        );
    refused.run();
    let file_menu = refused.state().catalog.text("menu-file");
    let export = refused
        .state()
        .command_label(AppCommand::ExportGeneralFabrication);
    refused
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    refused.run();
    refused
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    refused.run();
    assert!(!refused_path.exists());
    assert!(!refused_path.with_extension("drawings.svg").exists());
    assert!(refused.state().side_effect_receipts.is_empty());

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mixed.csv");
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_export(&path)
        .always_confirm_high_risk_as(85);
    let script = dialogs.clone();
    let app = build_app(dialogs);
    let revision = app.document.current().revision_id();
    let digest = app.document.current().canonical_digest();
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let file_menu = harness.state().catalog.text("menu-file");
    let export = harness
        .state()
        .command_label(AppCommand::ExportGeneralFabrication);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    harness.run();

    assert_eq!(harness.state().document.current().revision_id(), revision);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
    assert!(path.exists(), "{}", harness.state().digest);
    let bom = std::fs::read_to_string(&path).unwrap();
    let drawings = std::fs::read_to_string(path.with_extension("drawings.svg")).unwrap();
    assert!(bom.contains("ketchup.general-bom-export.v2"));
    assert!(bom.contains("position=1;definition=1;kind=manufactured;quantity=2"));
    assert!(bom.contains("material=ketchup.material.steel.s355.v1"));
    assert!(bom.contains("position=2;definition=1;kind=purchased;quantity=2"));
    assert!(bom.contains("material=ketchup.material.bearing.6202.v1"));
    assert!(drawings.contains("ketchup.general-drawing-svg.v3"));
    assert!(drawings.contains(
        "position: 1, quantity: 2, kind: manufactured, material: ketchup.material.steel.s355.v1"
    ));
    assert!(drawings.contains(
        "position: 2, quantity: 2, kind: purchased, material: ketchup.material.bearing.6202.v1"
    ));
    assert_eq!(script.export_requests()[0].extension, "csv");
    let receipt = harness.state().last_side_effect_receipt().unwrap();
    assert_eq!(
        receipt.scope().class(),
        HighRiskClass::ReleaseManufacturingExportWithWarnings
    );
    assert_eq!(
        receipt.operation(),
        "release-general-fabrication-bom-and-drawings"
    );

    assert!(
        harness
            .state_mut()
            .export_current_general_fabrication_to(&path)
    );
    assert_eq!(harness.state().side_effect_receipts.len(), 3);
    assert_eq!(
        harness.state().side_effect_receipts[0].scope().class(),
        HighRiskClass::ReleaseManufacturingExportWithWarnings
    );
    assert!(
        harness.state().side_effect_receipts[1..]
            .iter()
            .all(|receipt| receipt.scope().class() == HighRiskClass::Overwrite)
    );
    assert_eq!(script.high_risk_prompts().len(), 4);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bom);
    assert_eq!(
        std::fs::read_to_string(path.with_extension("drawings.svg")).unwrap(),
        drawings
    );
    assert_eq!(harness.state().document.current().revision_id(), revision);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
}

#[test]
fn hundegger_btlx_file_command_exports_validated_timber_with_support_report() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("single-timber.btlx");
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_export(&path)
        .always_confirm_high_risk_as(84);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(100),
                name: ketchup_manufacturing::fabrication::FABRICATION_ROLE_DIMENSION_V1.to_owned(),
                categories: vec![(
                    ClassificationCategoryId(101),
                    ketchup_manufacturing::fabrication::TIMBER_MEMBER_ROLE_V1.to_owned(),
                )],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(100),
                category_id: Some(ClassificationCategoryId(101)),
            },
        ]))
        .unwrap();
    install_initial_graph_result(&mut app);
    let revision = app.document.current().revision_id();
    let digest = app.document.current().canonical_digest();

    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let file_menu = harness.state().catalog.text("menu-file");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    let export = harness
        .state()
        .command_label(AppCommand::ExportHundeggerBtlx);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &export)
        .click();
    harness.run();

    assert_eq!(harness.state().document.current().revision_id(), revision);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
    let btlx = std::fs::read_to_string(&path).unwrap();
    assert!(btlx.contains("<BTLx"));
    assert!(btlx.contains("Version=\"2.3.1\""));
    assert!(btlx.contains("Material=\"ketchup.material.timber.unspecified.v1\""));
    let support = std::fs::read_to_string(path.with_extension("btlx.support.txt")).unwrap();
    assert!(support.contains("default_profile_request=edge SawContour cuts, then MillContour"));
    assert!(support.contains("selected_profile_request=edge SawContour cuts, then MillContour"));
    assert!(
        support.contains("portable_profile_removals=pocket, through-cut, extruded boolean-cut")
    );
    assert!(support.contains("intermediate_saw_cuts=0"));
    assert!(support.contains("concrete_importer_verified=false"));
    assert!(support.contains("machine_execution_order_guaranteed=false"));
    assert_eq!(script.export_requests()[0].extension, "btlx");
    let receipt = harness.state().last_side_effect_receipt().unwrap();
    assert_eq!(
        receipt.scope().class(),
        HighRiskClass::ReleaseManufacturingExportWithWarnings
    );
    assert_eq!(
        receipt.operation(),
        "release-hundegger-btlx-with-support-report"
    );
}

#[test]
fn hundegger_btlx_file_menu_configures_profile_strategy_and_intermediate_cuts() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            KetchupApp::new(),
        );
    harness.run();

    let file_menu = harness.state().catalog.text("menu-file");
    let options = format!(
        "{} ⏵",
        harness.state().catalog.text("file-export-btlx-options")
    );
    let portable = harness
        .state()
        .catalog
        .text("file-export-btlx-portable-contour");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &file_menu)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &options)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &portable)
        .click();
    harness.run();

    assert_eq!(
        harness
            .state()
            .btlx_export_options()
            .profile_processing_request,
        BtlxProfileProcessingRequest::PortableFreeContour
    );

    harness.state_mut().btlx_profile_strategy = BtlxProfileStrategy::EdgeSawCutsThenMillContour;
    harness.state_mut().btlx_intermediate_saw_cuts = 32;
    assert_eq!(
        harness
            .state()
            .btlx_export_options()
            .profile_processing_request,
        BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
            intermediate_saw_cuts: 32,
        }
    );
}

#[test]
fn exact_occurrence_reference_and_mesh_export_use_the_canonical_world_transform() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transformed.obj");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(62),
    ));
    let transform = Transform::from_matrix([
        0.0, -1.0, 0.0, 10.0, 1.0, 0.0, 0.0, 20.0, 0.0, 0.0, 1.0, 30.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform,
            },
        ]))
        .unwrap();
    let package = current_box_package(&app);
    let snapshot = app.document.current();
    app.exact
        .results
        .insert_current(&snapshot, Arc::new(package))
        .unwrap();
    let instance_path = InstancePath::root(OccurrenceId(1));

    let reference = app
        .exact_reference_for_occurrence(&instance_path, ExactFaceRole::Top)
        .unwrap();
    assert_eq!(reference.instance_path, instance_path);
    assert_eq!(reference.body.role(), Some(ExactFaceRole::Top));
    assert!(app.export_exact_occurrence_mesh_to(&instance_path, &path));

    let mesh = std::fs::read_to_string(&path).unwrap();
    let first_vertex = mesh
        .lines()
        .find_map(|line| line.strip_prefix("v "))
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(first_vertex, vec![10.0, 20.0, 30.0]);
    assert!(mesh.contains("g topological.face."));
    assert!(mesh.lines().any(|line| line.starts_with("f ")));
    let loss = std::fs::read_to_string(path.with_extension("obj.loss.txt")).unwrap();
    assert!(loss.contains("exact-body-to-world-space-mesh"));
    assert!(loss.contains("producer_feature_id=2"));
}

#[test]
fn sketchup_scene_import_confirmation_rederives_the_exact_reviewed_plan_atomically() {
    fn pending_for(app: &KetchupApp, path: &Path, source: &[u8]) -> PendingSketchupSceneImport {
        let snapshot = app.document.current();
        let source = ImportSourcePlan::seal(
            ImportFormat::SketchupScene,
            path.to_owned(),
            source.to_vec(),
            (),
            &snapshot,
        );
        PendingSketchupSceneImport {
            plan: app
                .prepare_sketchup_scene_import_preview_plan(source)
                .unwrap(),
            invalidated: false,
        }
    }

    fn state(app: &KetchupApp) -> (u64, String, usize, usize, usize, usize, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.definition_count(),
            app.occurrence_count(),
            app.import_receipt_count(),
        )
    }

    let source = br#"{"schema":"ketchup.sketchup-scene.v1","units":"inch","definitions":[{"id":"component:solid:1","name":"Reviewed solid","vertices":[[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],"triangles":[[0,2,1],[0,1,3],[0,3,2],[1,2,3]]}],"instances":[{"definition":"component:solid:1","name":"Reviewed instance","transform":[1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0],"visible":true}],"metadata":{"material_assignments":0,"textures":0,"tags":0,"scenes":0,"unsupported_entities":0}}"#;
    let alternate_source = source
        .windows(b"Reviewed instance".len())
        .position(|window| window == b"Reviewed instance")
        .map(|offset| {
            let mut alternate = source.to_vec();
            alternate.splice(
                offset..offset + b"Reviewed instance".len(),
                b"Tampered instance".iter().copied(),
            );
            alternate
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed-plan.kscene");
    std::fs::write(&path, source).unwrap();

    let mut app = KetchupApp::new();
    let pending = pending_for(&app, &path, source);
    let alternate = pending_for(&app, &path, &alternate_source);
    let baseline = state(&app);

    let mut review_tamper = pending.clone();
    review_tamper.plan.review = alternate.plan.review.clone();
    assert!(!app.import_sketchup_scene_from(&review_tamper));
    assert_eq!(state(&app), baseline);

    let mut proposal_tamper = pending.clone();
    proposal_tamper.plan.proposal = alternate.plan.proposal;
    assert!(!app.import_sketchup_scene_from(&proposal_tamper));
    assert_eq!(state(&app), baseline);

    let mut source_tamper = pending.clone();
    source_tamper.plan.source.source[0] ^= 1;
    assert!(!app.import_sketchup_scene_from(&source_tamper));
    assert_eq!(state(&app), baseline);

    let mut stale = pending.clone();
    stale.plan.source.revision_id += 1;
    assert!(!app.import_sketchup_scene_from(&stale));
    assert_eq!(state(&app), baseline);

    assert!(
        app.import_sketchup_scene_from(&pending),
        "{}",
        app.action_digest()
    );
    assert_eq!(app.document_revision(), baseline.0 + 1);
    assert_eq!(app.undo_step_count(), baseline.2 + 1);
    let committed = state(&app);
    assert!(!app.import_sketchup_scene_from(&pending));
    assert_eq!(state(&app), committed);
}

#[test]
fn exact_step_preview_plan_rejects_tamper_stale_and_replay_atomically() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/independent-xde-assembly.step");
    let source = std::fs::read(&path).unwrap();
    let executable = exact_worker_executable();
    assert!(executable.is_file(), "{}", executable.display());
    let mut app = KetchupApp::new();
    app.connect_exact_worker(executable).unwrap();
    let snapshot = app.document.current();
    let source_plan = ImportSourcePlan::seal(ImportFormat::Step, path, source, (), &snapshot);
    let pending = PendingStepImport {
        plan: app.prepare_step_import_preview_plan(source_plan).unwrap(),
        invalidated: false,
    };
    let baseline_revision = app.document_revision();
    let baseline_digest = app.canonical_digest();
    let baseline_undo = app.undo_step_count();
    let baseline_container = app.file.container_data.clone();
    let assert_unchanged = |app: &KetchupApp| {
        assert_eq!(app.document_revision(), baseline_revision);
        assert_eq!(app.canonical_digest(), baseline_digest);
        assert_eq!(app.undo_step_count(), baseline_undo);
        assert_eq!(app.file.container_data, baseline_container);
    };

    let mut evidence_tamper = pending.clone();
    evidence_tamper.plan.evidence.parts[0].exact.solid_count += 1;
    assert!(!app.import_step_from(&evidence_tamper));
    assert_unchanged(&app);

    let mut proposal_tamper = pending.clone();
    proposal_tamper.plan.proposal = app
        .document
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            }]),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert!(!app.import_step_from(&proposal_tamper));
    assert_unchanged(&app);

    let mut blob_tamper = pending.clone();
    blob_tamper.plan.blob_hash = "tampered".to_owned();
    assert!(!app.import_step_from(&blob_tamper));
    assert_unchanged(&app);

    let mut source_tamper = pending.clone();
    source_tamper.plan.source.source[0] ^= 1;
    assert!(!app.import_step_from(&source_tamper));
    assert_unchanged(&app);

    let mut stale = pending.clone();
    stale.plan.source.revision_id += 1;
    assert!(!app.import_step_from(&stale));
    assert_unchanged(&app);

    assert!(app.import_step_from(&pending), "{}", app.action_digest());
    assert_eq!(app.document_revision(), baseline_revision + 1);
    assert_eq!(app.undo_step_count(), baseline_undo + 1);
    assert_eq!(
        app.file.container_data.blobs().get(&pending.plan.blob_hash),
        Some(&pending.plan.source.source)
    );
    let committed_revision = app.document_revision();
    let committed_digest = app.canonical_digest();
    let committed_undo = app.undo_step_count();
    let committed_container = app.file.container_data.clone();
    assert!(!app.import_step_from(&pending));
    assert_eq!(app.document_revision(), committed_revision);
    assert_eq!(app.canonical_digest(), committed_digest);
    assert_eq!(app.undo_step_count(), committed_undo);
    assert_eq!(app.file.container_data, committed_container);
}

#[test]
fn stl_import_confirmation_rederives_the_exact_reviewed_plan_atomically() {
    fn pending_for(
        app: &KetchupApp,
        path: &Path,
        source: &[u8],
        unit: ImportLengthUnit,
    ) -> PendingStlImport {
        let snapshot = app.document.current();
        let source = ImportSourcePlan::seal(
            ImportFormat::Stl,
            path.to_owned(),
            source.to_vec(),
            unit,
            &snapshot,
        );
        PendingStlImport {
            plan: app.prepare_stl_import_preview_plan(source).unwrap(),
            review_error: None,
            invalidated: false,
        }
    }

    fn state(app: &KetchupApp) -> (u64, String, usize, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.redo_step_count(),
        )
    }

    let source = b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 1\n   vertex 0 1 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 1 0 0\n   vertex 0 1 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
endsolid tetrahedron\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed-plan.stl");
    std::fs::write(&path, source).unwrap();

    let mut app = KetchupApp::new();
    let pending = pending_for(&app, &path, source, ImportLengthUnit::Millimetre);
    let baseline = state(&app);

    let mut unit_tamper = pending.clone();
    unit_tamper.plan.source.unit = ImportLengthUnit::Centimetre;
    assert!(!app.import_stl_from(&unit_tamper));
    assert_eq!(state(&app), baseline);

    let alternate = pending_for(&app, &path, source, ImportLengthUnit::Centimetre);
    let mut review_tamper = pending.clone();
    review_tamper.plan.review = alternate.plan.review.clone();
    assert!(!app.import_stl_from(&review_tamper));
    assert_eq!(state(&app), baseline);

    let mut proposal_tamper = pending.clone();
    proposal_tamper.plan.proposal = alternate.plan.proposal;
    assert!(!app.import_stl_from(&proposal_tamper));
    assert_eq!(state(&app), baseline);

    let mut source_tamper = pending.clone();
    source_tamper.plan.source.source[0] ^= 1;
    assert!(!app.import_stl_from(&source_tamper));
    assert_eq!(state(&app), baseline);

    let mut stale = pending.clone();
    stale.plan.source.revision_id += 1;
    assert!(!app.import_stl_from(&stale));
    assert_eq!(state(&app), baseline);

    assert!(app.import_stl_from(&pending), "{}", app.action_digest());
    assert_eq!(app.document_revision(), baseline.0 + 1);
    assert_eq!(app.undo_step_count(), baseline.2 + 1);
    let committed = state(&app);
    assert!(!app.import_stl_from(&pending));
    assert_eq!(state(&app), committed);
}

#[test]
fn imported_stl_mesh_is_rendered_picked_and_outlined_from_canonical_geometry() {
    let source = b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 1\n   vertex 0 1 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 1 0 0\n   vertex 0 1 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
endsolid tetrahedron\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron.stl");
    std::fs::write(&path, source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let source_plan = ImportSourcePlan::seal(
        ImportFormat::Stl,
        path,
        source.to_vec(),
        ImportLengthUnit::Millimetre,
        &snapshot,
    );
    let pending = PendingStlImport {
        plan: app.prepare_stl_import_preview_plan(source_plan).unwrap(),
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_stl_from(&pending));

    let snapshot = app.document.current();
    let occurrence = snapshot.occurrences().next().unwrap().id();
    let projection = MeshInteractionProjection::from_snapshot(&snapshot);
    let hit = projection
        .exact_surface_pick(Ray::new(Vec3::new(0.2, 0.2, 2.0), Vec3::new(0.0, 0.0, -1.0)).unwrap())
        .expect("the imported tetrahedron must be pickable by its canonical triangles");
    assert_eq!(hit.instance_path, InstancePath::root(occurrence));

    let mut render_cache = renderer::DerivedRenderCache::default();
    let render_plan = renderer::InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact.results,
        &mut render_cache,
    );
    let imported_batch = render_plan
        .batches()
        .iter()
        .find(|batch| batch.definition_id == hit.definition_id)
        .expect("the imported mesh must produce a render batch");
    assert_eq!(imported_batch.geometry.index_count(), 12);
    assert!(imported_batch.geometry.vertex_count() >= 4);
    assert_eq!(imported_batch.instances.len(), 1);

    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.zoom_fit();
    app.selection.select_occurrence(occurrence, false);
    assert!(selection_stroke_segments(&context, &mut app) > 0);
}

/// Hovering or selecting a body must not re-derive its feature edges every
/// frame: on an imported mesh that cost seconds per frame and froze the
/// viewport as soon as the pointer touched the model.
#[test]
fn selected_imported_mesh_paints_its_outline_without_per_frame_edge_derivation() {
    let source = jagged_sphere_binary_stl(90, 90);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jagged-sphere.stl");
    std::fs::write(&path, &source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let source_plan = ImportSourcePlan::seal(
        ImportFormat::Stl,
        path,
        source.clone(),
        ImportLengthUnit::Millimetre,
        &snapshot,
    );
    let pending = PendingStlImport {
        plan: app.prepare_stl_import_preview_plan(source_plan).unwrap(),
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_stl_from(&pending), "{}", app.action_digest());

    let snapshot = app.document.current();
    let mesh = definition_mesh_body(&snapshot, snapshot.definitions().next().unwrap().id())
        .expect("the import produces a canonical mesh body");
    assert!(mesh.triangles.len() > 10_000);
    let occurrence = snapshot.occurrences().next().unwrap().id();
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.zoom_fit();
    app.selection.select_occurrence(occurrence, false);
    assert!(selection_stroke_segments(&context, &mut app) > 0);

    let cached = Arc::clone(
        &app.render
            .overlay_edge_cache
            .borrow()
            .values()
            .next()
            .unwrap()
            .edges,
    );
    let started = Instant::now();
    for _ in 0..8 {
        let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    }
    let elapsed = started.elapsed();
    assert!(
        Arc::ptr_eq(
            &cached,
            &app.render
                .overlay_edge_cache
                .borrow()
                .values()
                .next()
                .unwrap()
                .edges
        ),
        "the feature edges of an unchanged body must be derived once, not per frame"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "eight frames of a selected {} triangle mesh took {elapsed:?}",
        mesh.triangles.len()
    );
}

#[test]
fn dxf_import_confirmation_rederives_the_exact_reviewed_plan_atomically() {
    fn pending_for(
        app: &KetchupApp,
        path: &Path,
        source: &[u8],
        unit: ImportLengthUnit,
    ) -> PendingDxfImport {
        let snapshot = app.document.current();
        let source = ImportSourcePlan::seal(
            ImportFormat::Dxf,
            path.to_owned(),
            source.to_vec(),
            unit,
            &snapshot,
        );
        PendingDxfImport {
            plan: app.prepare_dxf_import_preview_plan(source).unwrap(),
            unit_confirmed: true,
            review_error: None,
            invalidated: false,
        }
    }

    fn state(app: &KetchupApp) -> (u64, String, usize, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.redo_step_count(),
        )
    }

    let source = b"0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nreviewed\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nENDSEC\n0\nEOF\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed-plan.dxf");
    std::fs::write(&path, source).unwrap();

    let mut valid = KetchupApp::new();
    let pending = pending_for(&valid, &path, source, ImportLengthUnit::Millimetre);
    let initial = state(&valid);
    assert!(valid.import_dxf_from(&pending));
    assert_eq!(valid.document_revision(), initial.0 + 1);
    assert_eq!(valid.undo_step_count(), initial.2 + 1);
    let committed = state(&valid);
    assert!(!valid.import_dxf_from(&pending));
    assert_eq!(state(&valid), committed);

    let mut unit_tampered = KetchupApp::new();
    let mut pending = pending_for(&unit_tampered, &path, source, ImportLengthUnit::Millimetre);
    pending.plan.source.unit = ImportLengthUnit::Centimetre;
    let before = state(&unit_tampered);
    assert!(!unit_tampered.import_dxf_from(&pending));
    assert_eq!(state(&unit_tampered), before);

    let mut review_tampered = KetchupApp::new();
    let mut pending = pending_for(
        &review_tampered,
        &path,
        source,
        ImportLengthUnit::Millimetre,
    );
    let alternate = pending_for(
        &review_tampered,
        &path,
        source,
        ImportLengthUnit::Centimetre,
    );
    pending.plan.review = alternate.plan.review;
    let before = state(&review_tampered);
    assert!(!review_tampered.import_dxf_from(&pending));
    assert_eq!(state(&review_tampered), before);

    let mut proposal_tampered = KetchupApp::new();
    let mut pending = pending_for(
        &proposal_tampered,
        &path,
        source,
        ImportLengthUnit::Millimetre,
    );
    let alternate = pending_for(
        &proposal_tampered,
        &path,
        source,
        ImportLengthUnit::Centimetre,
    );
    pending.plan.proposal = alternate.plan.proposal;
    let before = state(&proposal_tampered);
    assert!(!proposal_tampered.import_dxf_from(&pending));
    assert_eq!(state(&proposal_tampered), before);

    let mut unconfirmed = KetchupApp::new();
    let mut pending = pending_for(&unconfirmed, &path, source, ImportLengthUnit::Millimetre);
    pending.unit_confirmed = false;
    let before = state(&unconfirmed);
    assert!(!unconfirmed.import_dxf_from(&pending));
    assert_eq!(state(&unconfirmed), before);

    let mut stale = KetchupApp::new();
    let pending = pending_for(&stale, &path, source, ImportLengthUnit::Millimetre);
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let after_drift = state(&stale);
    assert!(!stale.import_dxf_from(&pending));
    assert_eq!(state(&stale), after_drift);
}

#[test]
fn imported_dxf_profiles_remain_projected_and_pickable_after_persistence() {
    let source = b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nopen\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nLWPOLYLINE\n8\nclosed\n90\n3\n70\n1\n\
10\n20\n20\n0\n10\n30\n20\n0\n10\n20\n20\n10\n\
0\nCIRCLE\n8\nround\n10\n50\n20\n5\n40\n4\n\
0\nENDSEC\n0\nEOF\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    std::fs::write(&path, source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let source_plan = ImportSourcePlan::seal(
        ImportFormat::Dxf,
        path,
        source.to_vec(),
        ImportLengthUnit::Millimetre,
        &snapshot,
    );
    let pending = PendingDxfImport {
        plan: app.prepare_dxf_import_preview_plan(source_plan).unwrap(),
        unit_confirmed: true,
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_dxf_from(&pending));

    let committed = app.document.current();
    let reopened =
        ketchup_model::persistence::load(&ketchup_model::persistence::save(&committed)).unwrap();
    let snapshot = reopened.snapshot();
    let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
    let scene = projection.scene().unwrap();
    assert_eq!(scene.occurrence_count(), 3);
    assert!(snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        exact_circle_geometry(segments, *closed) == Some(([50.0, 5.0], 4.0))
    }));
    let open_hit = scene
        .exact_pick(
            Ray::new(Vec3::new(5.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .expect("the persisted open DXF profile must remain pickable");
    let closed_hit = scene
        .exact_pick(
            Ray::new(Vec3::new(25.0, 5.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .expect("the persisted closed DXF profile must remain pickable");
    let circle_hit = scene
        .exact_pick(
            Ray::new(Vec3::new(54.0, 5.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .expect("the persisted exact DXF circle must remain pickable");
    assert_ne!(
        open_hit.primary.reference.instance_path,
        closed_hit.primary.reference.instance_path
    );
    assert_ne!(
        closed_hit.primary.reference.instance_path,
        circle_hit.primary.reference.instance_path
    );
}
