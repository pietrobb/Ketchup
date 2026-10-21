use super::*;

#[test]
fn file_import_dxf_reviews_and_commits_one_canonical_profile_transaction_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    let document_path = directory.path().join("imported-dxf.ketchup");
    std::fs::write(&path, valid_dxf_subset()).unwrap();
    assert_eq!(
        inspect_dxf(valid_dxf_subset(), DxfImportOptions::new(None))
            .unwrap()
            .profiles()
            .len(),
        24
    );
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);
    let before_box_count = shell.app().active_box_count();
    // The starting part's own base outline is a profile too; only imported ones count.
    let profiles_before = shell
        .app()
        .document_snapshot()
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Profile { .. }))
        .count();

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(
        shell.app().document_revision() == before.revision + 1,
        "DXF import did not commit: {}",
        shell.app().action_digest()
    );
    assert!(shell.app().definition_count() >= 24);
    assert!(shell.app().feature_count() >= 24);
    assert!(shell.app().occurrence_count() >= 24);
    assert!(shell.app().active_box_count() >= before_box_count + 24);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&shell, "digest-imported-dxf"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Dxf,
            filter_label: shell.catalog().text("file-filter-dxf-dwg"),
            extensions: vec!["dxf".to_owned(), "dwg".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().definition_count(), before.definitions);
    assert_eq!(shell.app().feature_count(), before.features);
    assert_eq!(shell.app().occurrence_count(), before.occurrences);
    assert_eq!(shell.app().import_receipt_count(), before.import_receipts);
    assert!(shell.app().can_redo());
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    let loaded = ketchup_model::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::Dxf);
    assert_eq!(
        receipt.parser_version(),
        ketchup_model::import::DXF_PARSER_VERSION
    );
    assert_eq!(receipt.outputs().len(), 24 * 3);
    assert_eq!(receipt.source_sha256(), &sha256_bytes(valid_dxf_subset()));
    assert_eq!(receipt.source_byte_len(), valid_dxf_subset().len() as u64);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("TEXT")
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("ATTDEF")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("stamp")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.insert-attributes-dropped"
            && diagnostic.subject() == Some("wrapper")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.dimension-semantics-dropped"
            && diagnostic.subject() == Some("*D1")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.dimension-graphics"
            && diagnostic.subject() == Some("*D1")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-boundary-topology-dropped"
            && diagnostic.subject() == Some("hatches")
            && diagnostic.count() == 2
    }));
    for code in [
        "dxf.mpolygon-fill-dropped",
        "dxf.mpolygon-annotation-dropped",
        "dxf.mpolygon-boundary-topology-dropped",
        "dxf.mpolygon-geometry",
    ] {
        assert!(receipt.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == code && diagnostic.subject() == Some("mpolygons")
        }));
    }
    for (subject, codes) in [
        (
            "meshes",
            [
                "dxf.mesh-face-topology-dropped",
                "dxf.mesh-boundary-geometry",
            ],
        ),
        (
            "polyfaces",
            [
                "dxf.polyface-face-topology-dropped",
                "dxf.polyface-boundary-geometry",
            ],
        ),
        (
            "polygon-meshes",
            [
                "dxf.polygon-mesh-surface-topology-dropped",
                "dxf.polygon-mesh-boundary-geometry",
            ],
        ),
    ] {
        for code in codes {
            assert!(receipt.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == code && diagnostic.subject() == Some(subject)
            }));
        }
    }
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.polyface-invisible-edge-dropped"
            && diagnostic.subject() == Some("polyfaces")
            && diagnostic.count() == 1
    }));
    for code in [
        "dxf.leader-arrowhead-dropped",
        "dxf.leader-semantics-dropped",
        "dxf.leader-geometry",
    ] {
        assert!(
            receipt
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code() == code && diagnostic.count() == 1 })
        );
    }
    assert_eq!(
        loaded_snapshot
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::Profile { .. }))
            .count(),
        profiles_before + 24
    );
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [200.0, 0.0]
            && segments[0].end_mm() == [204.0, 0.0]
            && segments[1].end_mm() == [204.0, 3.0]
            && segments[2].end_mm() == [200.0, 3.0]
            && segments[3].end_mm() == [200.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        !*closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [170.0, 0.0],
                        end_mm: [174.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [174.0, 0.0],
                        end_mm: [174.0, 3.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        !*closed
            && matches!(
                segments.as_slice(),
                [ProfileSegment::Line {
                    start_mm: [165.0, 2.0],
                    end_mm: [169.0, 2.0]
                }]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 3
            && segments[0].start_mm() == [80.0, 0.0]
            && segments[0].end_mm() == [84.0, 0.0]
            && segments[1].end_mm() == [80.0, 3.0]
            && segments[2].end_mm() == [80.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [60.0, -10.0]
            && segments[0].end_mm() == [60.0, -14.0]
            && segments[1].end_mm() == [54.0, -14.0]
            && segments[3].end_mm() == [60.0, -10.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [90.0, 0.0]
            && segments[0].end_mm() == [94.0, 0.0]
            && segments[1].end_mm() == [94.0, 3.0]
            && segments[2].end_mm() == [90.0, 3.0]
            && segments[3].end_mm() == [90.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [100.0, 0.0],
                        end_mm: [104.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [104.0, 0.0],
                        end_mm: [100.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [100.0, 3.0],
                        end_mm: [100.0, 0.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        matches!(
            segments.as_slice(),
            [
                ProfileSegment::CircularArc {
                    start_mm: [35.0, 20.0],
                    end_mm: [25.0, 20.0],
                    center_mm: [30.0, 20.0],
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: [25.0, 20.0],
                    end_mm: [35.0, 20.0],
                    center_mm: [30.0, 20.0],
                    clockwise: false,
                }
            ] if *closed
        )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [40.0, 0.0]
            && segments[0].end_mm() == [50.0, 0.0]
            && segments[3].end_mm() == [40.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 3
            && segments[0].start_mm() == [60.0, 0.0]
            && segments[0].end_mm() == [60.0, -8.0]
            && segments[1].end_mm() == [48.0, 0.0]
            && segments[2].end_mm() == [60.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 3
            && segments[0].start_mm() == [60.0, 10.0]
            && segments[0].end_mm() == [60.0, 2.0]
            && segments[1].end_mm() == [48.0, 10.0]
            && segments[2].end_mm() == [60.0, 10.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        matches!(
            segments.as_slice(),
            [
                ProfileSegment::CircularArc {
                    start_mm: [110.0, 7.0],
                    end_mm: [110.0, 3.0],
                    center_mm: [110.0, 5.0],
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: [110.0, 3.0],
                    end_mm: [110.0, 7.0],
                    center_mm: [110.0, 5.0],
                    clockwise: false,
                }
            ] if *closed
        )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        !*closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [120.0, 0.0],
                        end_mm: [124.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [124.0, 0.0],
                        end_mm: [124.0, 3.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [130.0, 0.0],
                        end_mm: [134.0, 0.0]
                    },
                    ProfileSegment::CircularArc {
                        start_mm: [134.0, 0.0],
                        end_mm: [134.0, 3.0],
                        center_mm: [134.0, 1.5],
                        clockwise: false
                    },
                    ProfileSegment::Line {
                        start_mm: [134.0, 3.0],
                        end_mm: [130.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [130.0, 3.0],
                        end_mm: [130.0, 0.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [140.0, 0.0],
                        end_mm: [144.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [144.0, 0.0],
                        end_mm: [144.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [144.0, 3.0],
                        end_mm: [140.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [140.0, 3.0],
                        end_mm: [140.0, 0.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::Profile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [150.0, 0.0],
                        end_mm: [154.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [154.0, 0.0],
                        end_mm: [154.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [154.0, 3.0],
                        end_mm: [150.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [150.0, 3.0],
                        end_mm: [150.0, 0.0]
                    }
                ]
            )
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-fill-dropped"
            && diagnostic.subject() == Some("hatch-patterned")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-fill-dropped"
            && diagnostic.subject() == Some("hatches")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("wrapper")
            && diagnostic.count() == 2
    }));
}

#[test]
fn file_menu_dxf_export_is_layered_authorized_atomic_and_non_mutating_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("profiles.dxf");
    let exported_path = directory.path().join("round-trip.dxf");
    let report_path = exported_path.with_extension("dxf.loss.txt");
    std::fs::write(&source_path, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source_path)
        .queue_export(&exported_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(117);
    let mut shell = Shell::with_dialogs(script.clone());
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    let before_export = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportDrawingDxf);
    assert_eq!(canonical_state(&shell), before_export);
    let first_dxf = std::fs::read(&exported_path).unwrap();
    let first_report = std::fs::read_to_string(&report_path).unwrap();
    let inspected = inspect_dxf(&first_dxf, DxfImportOptions::new(None)).unwrap();
    assert!(inspected.profiles().len() >= 24);
    assert!(inspected.layers().iter().any(|layer| layer == "bores"));
    assert!(inspected.layers().iter().any(|layer| layer == "hatches"));
    assert!(first_report.contains("schema=ketchup.dxf-profile-export.v1"));
    assert!(first_report.contains("unit=millimetre"));
    assert!(first_report.contains("dwg=unsupported"));

    std::fs::write(&exported_path, b"stale primary").unwrap();
    std::fs::write(&report_path, b"stale report").unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportDrawingDxf);
    assert_eq!(canonical_state(&shell), before_export);
    assert_eq!(std::fs::read(&exported_path).unwrap(), first_dxf);
    assert_eq!(std::fs::read_to_string(&report_path).unwrap(), first_report);
    assert_eq!(
        script.export_requests(),
        vec![
            ketchup_app::dialogs::ExportRequestRecord {
                filter_label: shell.catalog().text("file-filter-dxf"),
                extension: "dxf".to_owned(),
                suggested_name: "Untitled.dxf".to_owned(),
            },
            ketchup_app::dialogs::ExportRequestRecord {
                filter_label: shell.catalog().text("file-filter-dxf"),
                extension: "dxf".to_owned(),
                suggested_name: "Untitled.dxf".to_owned(),
            },
        ]
    );
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(prompts[0].contains(&shell.catalog().text("dialog-export-dxf-title")));
    assert!(prompts[0].contains(&shell.catalog().text("dialog-export-dxf-risk")));
    assert!(prompts[2].contains(&shell.catalog().text("dialog-export-overwrite-title")));
    assert!(prompts[3].contains(&shell.catalog().text("dialog-export-overwrite-title")));
    assert!(digest_starts_like(&shell, "digest-exported-dxf"));
}

#[test]
fn file_menu_refuses_a_dxf_named_dwg_and_profiles_as_dwg_without_preview_authorization_or_side_effects()
 {
    let directory = tempfile::tempdir().unwrap();
    let renamed_source = directory.path().join("renamed.dwg");
    let destination = directory.path().join("misleading.dwg");
    std::fs::write(&renamed_source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &renamed_source)
        .queue_export(&destination)
        .always_confirm_high_risk_as(118);
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    // A .dwg file is read as DWG; a DXF under that name is no DWG.
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    let digest = shell.app().action_digest();
    assert!(digest.contains("DWG file"), "{digest}");

    shell.click_menu_command("menu-file", AppCommand::ExportDrawingDxf);
    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-export-dxf"));
    assert!(!destination.exists());
    assert!(!destination.with_extension("dxf.loss.txt").exists());
    assert!(script.high_risk_prompts().is_empty());
}

#[test]
fn exact_exchange_refuses_native_solidworks_and_parasolid_without_preview_or_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let source = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/self-authored-box.step"),
    )
    .unwrap();
    let solidworks_source = directory.path().join("part.sldprt");
    let parasolid_source = directory.path().join("assembly.x_t");
    let solidworks_destination = directory.path().join("export.sldasm");
    let parasolid_destination = directory.path().join("export.x_b");
    std::fs::write(&solidworks_source, &source).unwrap();
    std::fs::write(&parasolid_source, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &solidworks_source)
        .queue_import(ImportFormat::Iges, &parasolid_source)
        .queue_export(&solidworks_destination)
        .queue_export(&parasolid_destination)
        .always_confirm_high_risk_as(121);
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("SolidWorks"));

    shell.click_menu_command("menu-file", AppCommand::ImportExactIges);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("Parasolid"));

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("SolidWorks"));

    shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("Parasolid"));

    assert!(!solidworks_destination.exists());
    assert!(!parasolid_destination.exists());
    assert!(script.high_risk_prompts().is_empty());
}

#[test]
fn exact_worker_derives_step_units_from_representation_context() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let original = std::fs::read_to_string(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    for (name, source, expected) in [
        (
            "millimetre",
            original.replace(
                "END-ISO-10303-21;",
                "/* misleading SI_UNIT(.CENTI.,.METRE.) comment */\nEND-ISO-10303-21;",
            ),
            ImportLengthUnit::Millimetre,
        ),
        (
            "centimetre",
            original.replace(".MILLI.", ".CENTI."),
            ImportLengthUnit::Centimetre,
        ),
        (
            "metre",
            original.replace(".MILLI.", "$"),
            ImportLengthUnit::Metre,
        ),
    ] {
        let path = directory.path().join(format!("{name}.step"));
        std::fs::write(&path, source.as_bytes()).unwrap();
        let evidence = inspector
            .inspect_step_import_with_cancellation(
                &path,
                &ketchup_model::graph::sha256_hex(source.as_bytes()),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(evidence.source_unit, expected);
    }
}

#[test]
fn file_import_exact_step_commits_undoes_and_persists_source_blob_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let source = std::fs::read(&source_path).unwrap();
    let source_hash = ketchup_model::graph::sha256_hex(&source);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source_evidence = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    let repeated_evidence = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(repeated_evidence, source_evidence);
    assert!(!source_evidence.backend.is_empty());
    assert!(!source_evidence.tolerance.is_empty());
    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-step.ketchup");
    let exported_path = directory.path().join("reexported-step.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(91)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the focused STEP workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let imported_feature_id = imported_exact_feature_id(&shell);

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 1);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-step"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Step,
            filter_label: shell.catalog().text("file-filter-step"),
            extensions: vec!["step".to_owned(), "stp".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().import_receipt_count(), before.import_receipts);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );

    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    // An imported STEP body without triangles is invisible and unpickable, and an exact
    // product that arrives after its revision must still reach the painted scene.
    wait_for_exact_render(&mut shell, &[imported_feature_id]);
    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    let screen = shell.app().project_to_screen(centre, viewport);
    shell.click_at(screen);
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "an imported STEP body must be pickable where it is painted"
    );
    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "STEP re-export failed with digest {:?}",
        shell.app().action_digest()
    );
    assert!(
        std::fs::read_to_string(&exported_path)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_evidence = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &ketchup_model::graph::sha256_hex(&exported),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, source_evidence.source_unit);
    assert_eq!(exported_evidence.solid_count, source_evidence.solid_count);
    assert!((exported_evidence.volume_mm3 - source_evidence.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (exported_evidence.bounds_mm[0][axis] - source_evidence.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (exported_evidence.bounds_mm[1][axis] - source_evidence.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }

    let loaded = ketchup_model::persistence::load_file(&document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.source_byte_len(), source.len() as u64);
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert_eq!(receipt.parser_id(), ketchup_model::import::STEP_PARSER_ID);
    assert_eq!(
        receipt.parser_version(),
        ketchup_model::import::STEP_XDE_PARSER_VERSION
    );
    let spec = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(spec.source_sha256, sha256_bytes(&source));
    assert_eq!(spec.source_byte_len, source.len() as u64);
    assert_eq!(spec.source_part_index, Some(0));
    let hash = ketchup_model::graph::sha256_hex(&source);
    assert_eq!(loaded.container_data().blobs().get(&hash), Some(&source));
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn file_import_exact_step_preserves_a_real_nested_repeated_xde_assembly_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/independent-xde-assembly.step");
    let source = std::fs::read(&source_path).unwrap();
    let source_hash = ketchup_model::graph::sha256_hex(&source);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let evidence = inspector
        .inspect_step_xde_import_with_cancellation(
            &source_path,
            &source_hash,
            &AtomicBool::new(false),
        )
        .unwrap();
    let source_geometry = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(evidence.parts.len(), 2);
    assert_eq!(evidence.nodes.len(), 5);
    assert_eq!(
        evidence
            .nodes
            .iter()
            .filter(|node| node.part_index.is_some())
            .count(),
        3
    );

    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-xde-assembly.ketchup");
    let exported_path = directory.path().join("roundtrip-xde-assembly.step");
    let exported_iges_path = directory.path().join("flat-xde-assembly.iges");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .queue_export(&exported_iges_path)
        .always_confirm_high_risk_as(117)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the XDE assembly workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);
    let before_groups = shell.app().group_count();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 2);
    assert_eq!(shell.app().feature_count(), before.features + 2);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 3);
    assert_eq!(shell.app().group_count(), before_groups + 2);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);

    let snapshot = shell.app().document_snapshot();
    let root = snapshot
        .groups()
        .find(|group| group.name() == "Fixture root assembly")
        .unwrap();
    let carriage = snapshot
        .groups()
        .find(|group| group.name() == "Carriage nested instance")
        .unwrap();
    assert_eq!(root.parent(), None);
    assert_eq!(carriage.parent(), Some(root.id()));
    assert_eq!(carriage.transform().matrix()[7], 50.0);
    let left = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Bracket left instance")
        .unwrap();
    let right = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Bracket right instance")
        .unwrap();
    let pin = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Pin root instance")
        .unwrap();
    assert_eq!(left.parent(), Some(carriage.id()));
    assert_eq!(right.parent(), Some(carriage.id()));
    assert_eq!(pin.parent(), Some(root.id()));
    assert_eq!(left.definition_id(), right.definition_id());
    assert_ne!(left.definition_id(), pin.definition_id());
    assert_eq!(left.color(), Some([255, 0, 0]));
    assert_eq!(right.color(), Some([0, 255, 0]));
    assert_eq!(pin.color(), Some([0, 0, 255]));
    assert_eq!(right.transform().matrix()[3], 40.0);
    assert_eq!(pin.transform().matrix()[3], 20.0);
    assert_eq!(pin.transform().matrix()[7], 10.0);
    assert_eq!(pin.transform().matrix()[11], 5.0);
    let mut imported_features = snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some((feature.id(), spec.source_part_index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    imported_features.sort_unstable();
    assert_eq!(
        imported_features
            .iter()
            .map(|(_, part_index)| *part_index)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1)]
    );
    let mut expected_producers = imported_features
        .iter()
        .map(|(feature_id, _)| *feature_id)
        .collect::<Vec<_>>();
    expected_producers.sort_unstable();
    drop(snapshot);

    wait_for_exact_render(&mut shell, &expected_producers);
    assert!(
        shell.app().instanced_scene_triangle_count() > shell.app().exact_render_triangle_count(),
        "the shared bracket definition must be instanced twice in the painted scene"
    );
    let screen = shell
        .app()
        .project_to_screen(Vec3::new(45.0, 60.0, 15.0), shell.viewport_rect());
    shell.click_at(screen);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(shell.app().definition_count(), before.definitions + 2);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 3);
    assert_eq!(shell.app().group_count(), before_groups + 2);
    wait_for_exact_render(&mut shell, &expected_producers);

    let state = ketchup_model::state_view::encode_semantic_state(&shell.app().document_snapshot())
        .agent()
        .to_owned();
    assert_eq!(state.matches("ImportedExactBody:{").count(), 2);
    assert!(state.contains("format:\"Step\""));
    assert!(state.contains("source_part_index:0"));
    assert!(state.contains("source_part_index:1"));
    assert!(
        state.contains("code:\"step_parametric_reconstruction_unavailable\",subject:none,count:2")
    );

    let loaded = ketchup_model::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let loaded_receipt = loaded_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(loaded_receipt.source_sha256(), &sha256_bytes(&source));
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_hierarchy_preserved")
    );
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_color_metadata_preserved")
    );
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_name_metadata_preserved")
    );
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_parametric_reconstruction_unavailable")
    );
    assert_eq!(
        loaded.container_data().blobs().get(&source_hash),
        Some(&source)
    );

    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "XDE assembly export failed with digest {:?}",
        shell.app().action_digest()
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_hash = ketchup_model::graph::sha256_hex(&exported);
    let roundtrip = inspector
        .inspect_step_xde_import_with_cancellation(
            &exported_path,
            &exported_hash,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(roundtrip.parts.len(), 2);
    assert_eq!(
        roundtrip
            .parts
            .iter()
            .map(|part| part.name.as_str())
            .collect::<Vec<_>>(),
        evidence
            .parts
            .iter()
            .map(|part| part.name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(roundtrip.nodes.len(), 6);
    assert_eq!(
        roundtrip
            .nodes
            .iter()
            .filter(|node| node.part_index.is_some())
            .count(),
        3
    );
    assert_eq!(
        roundtrip
            .nodes
            .iter()
            .filter_map(|node| node.part_index)
            .collect::<Vec<_>>(),
        vec![0, 0, 1]
    );
    let roundtrip_node = |name: &str| {
        roundtrip
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap()
    };
    let root_node = roundtrip_node("Fixture root assembly");
    let carriage_node = roundtrip_node("Carriage nested instance");
    let left_node = roundtrip_node("Bracket left instance");
    let right_node = roundtrip_node("Bracket right instance");
    let pin_node = roundtrip_node("Pin root instance");
    assert_eq!(carriage_node.parent_id, Some(root_node.id));
    assert_eq!(left_node.parent_id, Some(carriage_node.id));
    assert_eq!(right_node.parent_id, Some(carriage_node.id));
    assert_eq!(pin_node.parent_id, Some(root_node.id));
    assert_eq!(left_node.color, Some([255, 0, 0]));
    assert_eq!(right_node.color, Some([0, 255, 0]));
    assert_eq!(pin_node.color, Some([0, 0, 255]));
    assert_eq!(carriage_node.transform.matrix()[7], 50.0);
    assert_eq!(right_node.transform.matrix()[3], 40.0);
    assert_eq!(pin_node.transform.matrix()[3], 20.0);
    assert_eq!(pin_node.transform.matrix()[7], 10.0);
    assert_eq!(pin_node.transform.matrix()[11], 5.0);
    let roundtrip_geometry = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &exported_hash,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(roundtrip_geometry.solid_count, source_geometry.solid_count);
    assert!((roundtrip_geometry.volume_mm3 - source_geometry.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (roundtrip_geometry.bounds_mm[0][axis] - source_geometry.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (roundtrip_geometry.bounds_mm[1][axis] - source_geometry.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }
    assert!(
        std::fs::read_to_string(exported_path.with_extension("step.loss.txt"))
            .unwrap()
            .contains("repeated part definitions")
    );

    shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    assert_eq!(canonical_state(&shell), before_export);
    let exported_iges = std::fs::read(&exported_iges_path).unwrap();
    let iges_roundtrip = inspector
        .inspect_iges_xde_import_with_cancellation(
            &exported_iges_path,
            &ketchup_model::graph::sha256_hex(&exported_iges),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(iges_roundtrip.parts.len(), 3);
    assert_eq!(iges_roundtrip.nodes.len(), 3);
    assert!(
        iges_roundtrip
            .nodes
            .iter()
            .all(|node| node.transform == Transform::identity())
    );
    assert_eq!(
        iges_roundtrip
            .nodes
            .iter()
            .map(|node| (node.name.as_str(), node.color))
            .collect::<Vec<_>>(),
        vec![
            ("Bracket left instance", Some([255, 0, 0])),
            ("Bracket right instance", Some([0, 255, 0])),
            ("Pin root instance", Some([0, 0, 255])),
        ]
    );
    assert_eq!(
        iges_roundtrip
            .parts
            .iter()
            .map(|part| part.exact.solid_count)
            .sum::<u32>(),
        source_geometry.solid_count
    );
    assert!(
        (iges_roundtrip
            .parts
            .iter()
            .map(|part| part.exact.volume_mm3)
            .sum::<f64>()
            - source_geometry.volume_mm3)
            .abs()
            < 1.0e-6
    );
    let iges_loss =
        std::fs::read_to_string(exported_iges_path.with_extension("iges.loss.txt")).unwrap();
    assert!(iges_loss.contains("occurrence names, sRGB colors"));
    assert!(iges_loss.contains("hierarchy, local transforms, and repeated shared definitions"));
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn moving_an_imported_step_body_keeps_it_painted_and_drops_it_when_the_import_is_undone() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .always_confirm_high_risk_as(103);
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the imported STEP move workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let imported_feature_id = imported_exact_feature_id(&shell);
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_current_producer_ids() == [imported_feature_id]
            && shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let painted = shell.app().instanced_scene_triangle_count();
    assert!(painted > 0);

    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let span = (bounds[1][0] - bounds[0][0]).max(bounds[1][1] - bounds[0][1]);
    let interaction_point = centre + Vec3::new(span * 0.17, span * 0.09, 0.0);
    let viewport = shell.viewport_rect();
    let interaction_screen = shell.app().project_to_screen(interaction_point, viewport);
    wait_for_hovered_pick(&mut shell, interaction_screen);
    shell.click_at(interaction_screen);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    // A move publishes a new revision without touching the import, and the
    // isolated worker needs seconds to re-derive the body. The body must stay
    // painted in the very next frame instead of blinking out until it catches up.
    // The move goes through a real viewport drag: a drag ends its preview in the
    // same frame that commits the revision, so the scene plan is rebuilt at the
    // moment the carried-forward products are still bound to the old revision.
    let offset = Vec3::new(40.0, 0.0, 0.0);
    shell.click_command(AppCommand::Move);
    let from = shell.app().project_to_screen(interaction_point, viewport);
    let to = shell
        .app()
        .project_to_screen(interaction_point + offset, viewport);
    // Every single frame of the gesture counts, not just the settled result: the
    // frame that commits the revision paints before the next one can repair it,
    // and a shell only repaints on demand, so one blank frame stays on screen.
    let mut blank_frames = 0;
    shell.drag_observing(from, to, |app| {
        if app.instanced_scene_triangle_count() == 0 {
            blank_frames += 1;
        }
    });
    assert_eq!(
        blank_frames, 0,
        "no frame of a move may paint an empty scene"
    );
    shell.settle();
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "moving an imported STEP body must not invalidate it"
    );
    assert_eq!(
        shell.app().instanced_scene_triangle_count(),
        painted,
        "an imported STEP body must stay painted across a move"
    );
    let viewport = shell.viewport_rect();
    let moved = shell
        .app()
        .project_to_screen(interaction_point + offset, viewport);
    shell.click_at(moved);
    assert!(
        shell.app().hovered_selection().is_some(),
        "a moved imported STEP body must be hoverable where it is painted"
    );
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "a moved imported STEP body must be pickable where it is painted"
    );

    // Move only starts on a hovered occurrence, so a pick projection that stops
    // following the carried-forward products leaves the body painted but dead:
    // the first move works and no later one does.
    let before_second_move = shell.app().document_revision();
    shell.click_command(AppCommand::Move);
    let further = shell
        .app()
        .project_to_screen(interaction_point + offset + offset, viewport);
    shell.drag(moved, further);
    shell.settle();
    assert_eq!(
        shell.app().document_revision(),
        before_second_move + 1,
        "a second move of an imported STEP body must still commit: {:?}",
        shell.app().action_digest()
    );

    // Carrying the product forward must stay fail-closed: undoing the import
    // removes the feature it was derived from, so it must disappear at once.
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.settle();
    assert_eq!(
        shell.app().exact_render_body_count(),
        0,
        "an imported body whose feature is gone must not be carried forward"
    );
    assert_eq!(shell.app().instanced_scene_triangle_count(), 0);
}

#[test]
fn rotating_an_imported_step_body_keeps_it_painted_pickable_and_turnable_again() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .always_confirm_high_risk_as(103);
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the imported STEP rotate workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let imported_feature_id = imported_exact_feature_id(&shell);
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_current_producer_ids() == [imported_feature_id]
            && shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let painted = shell.app().instanced_scene_triangle_count();
    assert!(painted > 0);

    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let span = (bounds[1][0] - bounds[0][0]).max(bounds[1][1] - bounds[0][1]);
    let interaction_point = centre + Vec3::new(span * 0.17, span * 0.09, 0.0);
    let viewport = shell.viewport_rect();
    let interaction_screen = shell.app().project_to_screen(interaction_point, viewport);
    wait_for_hovered_pick(&mut shell, interaction_screen);
    shell.click_at(interaction_screen);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    // An imported body carries no canonical box, so its pivot has to come from
    // the painted exact geometry. Turning it is a pure occurrence transform:
    // the accepted product stays valid and the worker is never asked again.
    shell.click_command(AppCommand::Rotate);
    let before_steps = shell.app().undo_step_count();
    let from = shell
        .app()
        .project_to_screen(centre + Vec3::new(span * 0.4, 0.0, 0.0), viewport);
    let to = shell
        .app()
        .project_to_screen(centre + Vec3::new(0.0, span * 0.4, 0.0), viewport);
    // Choose the pivot, then grab an arm and swing it a quarter turn.
    let pivot = shell.app().project_to_screen(centre, viewport);
    shell.click_at(pivot);
    let mut blank_frames = 0;
    shell.drag_observing(from, to, |app| {
        if app.instanced_scene_triangle_count() == 0 {
            blank_frames += 1;
        }
    });
    assert_eq!(
        blank_frames, 0,
        "no frame of a rotation may paint an empty scene"
    );
    shell.settle();
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "one Rotate drag must be exactly one undo step: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "rotating an imported STEP body must not invalidate it"
    );
    assert_eq!(
        shell.app().instanced_scene_triangle_count(),
        painted,
        "an imported STEP body must stay painted across a rotation"
    );

    // The pivot is the surface point under the cursor, not the body's centre, so
    // the body may have moved: aim at where it is painted now. A second rotation
    // must be able to start there. The render bounds are the body's own, placed
    // by its occurrence transform (the identity before the first turn).
    let snapshot = shell.app().document_snapshot();
    let occurrence = snapshot.occurrences().next().unwrap().id();
    let placed = snapshot
        .world_transform_for_occurrence(occurrence)
        .unwrap()
        .transform_point([centre.x, centre.y, centre.z]);
    let centre = Vec3::new(placed[0], placed[1], placed[2]);
    let interaction_point = centre + Vec3::new(span * 0.17, span * 0.09, 0.0);
    let viewport = shell.viewport_rect();
    let interaction_screen = shell.app().project_to_screen(interaction_point, viewport);
    let pivot = shell.app().project_to_screen(centre, viewport);
    let from = shell
        .app()
        .project_to_screen(centre + Vec3::new(span * 0.4, 0.0, 0.0), viewport);
    let to = shell
        .app()
        .project_to_screen(centre + Vec3::new(0.0, span * 0.4, 0.0), viewport);
    wait_for_hovered_pick(&mut shell, interaction_screen);
    shell.click_at(interaction_screen);
    assert!(
        shell.app().hovered_selection().is_some(),
        "a rotated imported STEP body must be hoverable where it is painted"
    );
    let before_steps = shell.app().undo_step_count();
    shell.click_command(AppCommand::Rotate);
    shell.click_at(pivot);
    shell.drag(from, to);
    shell.settle();
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "a second rotation of an imported STEP body must still commit: {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn file_import_transformed_multi_solid_step_round_trips_through_save_open_and_occt() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("transformed-multi-solid.step");
    let document_path = directory.path().join("transformed-multi-solid.ketchup");
    let exported_path = directory
        .path()
        .join("transformed-multi-solid-reexport.step");

    let source_script = ScriptedFileDialogs::new()
        .queue_export(&source_path)
        .always_confirm_high_risk_as(101);
    let mut source_shell = Shell::with_dialogs(source_script);
    source_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    compose_two_shared_occurrences(&mut source_shell);
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        source_shell.settle();
        if source_shell.app().exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(source_shell.app().exact_render_body_count(), 1);
    source_shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert!(
        source_path.is_file(),
        "multi-solid STEP source export failed with digest {:?}",
        source_shell.app().action_digest()
    );

    let source = std::fs::read(&source_path).unwrap();
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source_evidence = inspector
        .inspect_step_import_with_cancellation(
            &source_path,
            &ketchup_model::graph::sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(source_evidence.solid_count, 2);
    assert!(source_evidence.bounds_mm[1][0] > 150.0);
    assert!(source_evidence.bounds_mm[1][1] > 25.0);
    let source_xde_evidence = inspector
        .inspect_step_xde_import_with_cancellation(
            &source_path,
            &ketchup_model::graph::sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(source_xde_evidence.parts.len(), 1);
    assert_eq!(source_xde_evidence.nodes.len(), 3);
    assert_eq!(
        source_xde_evidence
            .nodes
            .iter()
            .filter_map(|node| node.part_index)
            .collect::<Vec<_>>(),
        vec![0, 0]
    );

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(102)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let mut imported_feature_ids = shell
        .app()
        .document_snapshot()
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
        .map(ketchup_model::document::Feature::id)
        .collect::<Vec<_>>();
    imported_feature_ids.sort_unstable();
    assert_eq!(imported_feature_ids.len(), 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 2);
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    // Under the full workspace run the worker can take longer than 2 s to
    // republish after Open; wait on its state, the asserts below decide.
    let _ = shell.wait_until(|shell| {
        shell.settle();
        let mut producers = shell.app().exact_current_producer_ids();
        producers.sort_unstable();
        producers == imported_feature_ids && shell.app().instanced_scene_triangle_count() > 0
    });
    let mut producers = shell.app().exact_current_producer_ids();
    producers.sort_unstable();
    assert_eq!(producers, imported_feature_ids);
    assert_eq!(shell.app().exact_render_body_count(), 1);
    shell.settle();
    assert!(
        shell.app().instanced_scene_triangle_count() > 0,
        "reopened shared STEP occurrences must reach the painted scene"
    );
    // XDE preserves two occurrences of one shared exact part definition.
    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    let screen = shell.app().project_to_screen(centre, viewport);
    shell.click_at(screen);
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "a reopened multi-solid STEP body must be pickable where it is painted"
    );

    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "multi-solid STEP re-export failed with digest {:?}",
        shell.app().action_digest()
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_evidence = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &ketchup_model::graph::sha256_hex(&exported),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, source_evidence.source_unit);
    assert_eq!(exported_evidence.solid_count, source_evidence.solid_count);
    assert!((exported_evidence.volume_mm3 - source_evidence.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (exported_evidence.bounds_mm[0][axis] - source_evidence.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (exported_evidence.bounds_mm[1][axis] - source_evidence.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }

    let loaded = ketchup_model::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn file_import_step_cancel_worker_source_and_stale_refuse_without_mutation() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let original = std::fs::read(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.step");
    let corrupt = directory.path().join("corrupt.step");
    let unsupported_unit = directory.path().join("unsupported-unit.step");
    let replacement = directory.path().join("replacement.ketchup");
    let unavailable_worker = directory.path().join("unavailable-worker");
    std::fs::write(&source, &original).unwrap();
    std::fs::write(&corrupt, b"not an ISO-10303-21 exchange file").unwrap();
    let unsupported_source = String::from_utf8(original.clone())
        .unwrap()
        .replace(".MILLI.", ".MICRO.")
        .into_bytes();
    assert_ne!(unsupported_source, original);
    std::fs::write(&unsupported_unit, unsupported_source).unwrap();
    std::fs::write(&unavailable_worker, b"not an exact worker").unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Step)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &corrupt)
        .queue_import(ImportFormat::Step, &unsupported_unit)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    let mut changed = original.clone();
    changed[0] ^= 1;
    std::fs::write(&source, &changed).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
    std::fs::write(&source, &original).unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    for _ in 0..2 {
        let before_refusal = canonical_state(&shell);
        let before_refusal_history = reachable_history_digests(&mut shell);
        shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
        assert!(digest_starts_like(&shell, "error-import-step"));
        assert_state_and_history_unchanged(&mut shell, &before_refusal, &before_refusal_history);
    }

    shell.click_at(shell.viewport_rect().center());
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_change_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(
        &mut shell,
        &after_document_change,
        &after_document_change_history,
    );

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

const OVER_32_MIB: usize = 32 * 1024 * 1024 + 1;

/// Comment lines, which a STEP reader skips, until the file holds at least `len` bytes.
fn padded_step(source: &[u8], len: usize) -> Vec<u8> {
    let text = std::str::from_utf8(source).unwrap();
    let line = format!("/* {} */\n", "padding ".repeat(9));
    let lines = len.saturating_sub(source.len()).div_ceil(line.len());
    let padded = text.replacen("DATA;\n", &format!("DATA;\n{}", line.repeat(lines)), 1);
    assert!(padded.len() >= len);
    padded.into_bytes()
}

/// Blank Start-section lines, which carry only the sender's prose, until the file holds at
/// least `len` bytes; the Terminate line counts them.
fn padded_iges(source: &[u8], len: usize) -> Vec<u8> {
    let text = std::str::from_utf8(source).unwrap();
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let section = |line: &str| line.as_bytes().get(72).copied();
    let starts = text
        .lines()
        .filter(|line| section(line) == Some(b'S'))
        .count();
    let extra = len
        .saturating_sub(source.len())
        .div_ceil(80 + newline.len());
    let mut padded = String::with_capacity(len + 1024);
    let mut seen = 0;
    for line in text.lines() {
        if section(line) == Some(b'T') {
            padded.push_str(&format!("S{:07}{}", starts + extra, &line[8..]));
        } else {
            padded.push_str(line);
        }
        padded.push_str(newline);
        if section(line) == Some(b'S') {
            seen += 1;
            if seen == starts {
                for number in starts + 1..=starts + extra {
                    padded.push_str(&format!("{:72}S{number:7}{newline}", ""));
                }
            }
        }
    }
    assert!(padded.len() >= len);
    padded.into_bytes()
}

/// Spaces at the end of the JSON chunk, which the glTF JSON allows, until the file holds at
/// least `len` bytes; the chunk and file lengths in the header grow with them.
fn padded_glb(source: &[u8], len: usize) -> Vec<u8> {
    let read = |at: usize| u32::from_le_bytes(source[at..at + 4].try_into().unwrap()) as usize;
    let json_len = read(12);
    let extra = len.saturating_sub(source.len()).next_multiple_of(4);
    let mut padded = Vec::with_capacity(source.len() + extra);
    padded.extend_from_slice(&source[..20 + json_len]);
    padded.resize(20 + json_len + extra, b' ');
    padded.extend_from_slice(&source[20 + json_len..]);
    let total = u32::try_from(padded.len()).unwrap();
    padded[8..12].copy_from_slice(&total.to_le_bytes());
    let json = u32::try_from(json_len + extra).unwrap();
    padded[12..16].copy_from_slice(&json.to_le_bytes());
    assert!(padded.len() >= len);
    padded
}

/// STEP, IGES, SketchUp scene and GLB sources have no size limit of their own: each, padded
/// past 32 MiB with what its format skips, imports as one Undo step whose receipt keeps the
/// whole length, and the document saves and opens with all four.
#[test]
fn step_iges_sketchup_scene_and_glb_sources_over_32_mib_import_save_and_open() {
    let directory = tempfile::tempdir().unwrap();
    let step_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let small_iges = directory.path().join("small.iges");
    ExactWorkerSupervisor::spawn(exact_worker_path())
        .unwrap()
        .convert_step_to_iges_with_cancellation(&step_fixture, &small_iges, &AtomicBool::new(false))
        .unwrap();
    let glb_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let mut scene = valid_sketchup_scene_bridge();
    scene.resize(scene.len().max(OVER_32_MIB), b' ');
    let sources = [
        (
            ImportFormat::Step,
            "large.step",
            padded_step(&std::fs::read(&step_fixture).unwrap(), OVER_32_MIB),
            AppCommand::ImportExactStep,
            "dialog-import-step-confirm",
        ),
        (
            ImportFormat::Iges,
            "large.iges",
            padded_iges(&std::fs::read(&small_iges).unwrap(), OVER_32_MIB),
            AppCommand::ImportExactIges,
            "dialog-import-iges-confirm",
        ),
        (
            ImportFormat::SketchupScene,
            "large.kscene",
            scene,
            AppCommand::ImportSketchupScene,
            "dialog-import-sketchup-scene-confirm",
        ),
        (
            ImportFormat::Glb,
            "large.glb",
            padded_glb(&std::fs::read(&glb_fixture).unwrap(), OVER_32_MIB),
            AppCommand::ImportBlenderGlb,
            "dialog-import-glb-confirm",
        ),
    ];
    let document_path = directory.path().join("large-imports.ketchup");
    let mut script = ScriptedFileDialogs::new();
    for (format, name, source, ..) in &sources {
        let path = directory.path().join(name);
        std::fs::write(&path, source).unwrap();
        script = script.queue_import(*format, &path);
    }
    let script = script
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    for (format, _, source, command, confirm) in &sources {
        let before = canonical_state(&shell);
        shell.click_menu_command("menu-file", *command);
        shell.click_button_label(&shell.catalog().text(confirm));
        assert_eq!(
            shell.app().import_receipt_count(),
            before.import_receipts + 1,
            "{format:?}: {}",
            shell.app().action_digest()
        );
        assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
        let snapshot = shell.app().document_snapshot();
        let receipt = snapshot
            .import_receipts()
            .find(|receipt| receipt.format() == *format)
            .unwrap();
        assert_eq!(receipt.source_byte_len(), source.len() as u64);
        assert_eq!(receipt.source_sha256(), &sha256_bytes(source));
    }

    let imported = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported);
    assert_eq!(shell.app().import_receipt_count(), sources.len());
}

#[test]
fn file_import_unitless_dxf_requires_and_persists_explicit_user_units() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unitless.dxf");
    let document_path = directory.path().join("unitless.ketchup");
    let source = std::str::from_utf8(valid_dxf_subset())
        .unwrap()
        .replace("9\n$INSUNITS\n70\n4\n", "")
        .into_bytes();
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &path)
        .queue_save(&document_path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert_eq!(canonical_state(&shell), before);

    shell.click_role_and_label(Role::RadioButton, &shell.catalog().text("unit-centimetre"));
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    shell.click_menu_command("menu-file", AppCommand::Save);

    let loaded = ketchup_model::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Centimetre);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::UserDeclared
    );
}

#[test]
fn file_import_dxf_dialog_and_review_cancel_leave_canonical_state_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    std::fs::write(&path, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Dxf)
        .queue_import(ImportFormat::Dxf, &path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert_state_and_history_unchanged(&mut shell, &before, &history);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_dxf_refusals_preserve_exact_state_and_reachable_history() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.dxf");
    let ambiguous = directory.path().join("ambiguous.dxf");
    std::fs::write(
        &malformed,
        b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n1\n",
    )
    .unwrap();
    std::fs::write(
        &ambiguous,
        b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n\
0\nLINE\n10\n0\n20\n0\n11\n0\n21\n1\n\
0\nLINE\n10\n0\n20\n0\n11\n-1\n21\n0\n\
0\nENDSEC\n0\nEOF\n",
    )
    .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &malformed)
        .queue_import(ImportFormat::Dxf, &ambiguous);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);
    assert!(before.redo_steps > 0);

    for _ in 0..2 {
        shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
        assert!(digest_starts_like(&shell, "error-import-dxf"));
        assert_state_and_history_unchanged(&mut shell, &before, &history);
    }
}

#[test]
fn file_import_dxf_over_8_mib_is_not_refused_for_its_size() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.dxf");
    // Comments (group 999) pad the drawing past the 8 MiB the reader once refused.
    let comment = format!("999\n{}\n", "x".repeat(500));
    let mut source = comment.repeat(17_000).into_bytes();
    source.extend_from_slice(valid_dxf_subset());
    assert!(source.len() > 8 * 1024 * 1024);
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(
        shell.app().document_revision() == before.revision + 1,
        "the large DXF did not import: {}",
        shell.app().action_digest()
    );
    assert!(digest_starts_like(&shell, "digest-imported-dxf"));
    assert_eq!(shell.app().import_receipt_count(), 1);
}

#[test]
fn file_import_dxf_rejects_source_document_history_and_open_staleness() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    let replacement = directory.path().join("replacement.ketchup");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    let before_source_change = canonical_state(&shell);
    let before_source_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    let mut same_length_change = valid_dxf_subset().to_vec();
    same_length_change[0] = b'1';
    std::fs::write(&source, same_length_change).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before_source_change, &before_source_history);

    std::fs::write(&source, valid_dxf_subset()).unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_document_change, &after_document_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
}

#[test]
fn file_import_dxf_undo_redo_round_trip_cannot_restore_confirmability() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &source);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(canonical_state(&shell), before);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_dxf_edit_context_navigation_invalidates_review() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &source);
    let mut shell = Shell::with_dialogs(script);
    let solid = shell.top_face_centre(1);
    shell.click_at(solid);
    shell.double_click_at(solid);
    assert_eq!(shell.app().edit_context_depth(), 1);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 0);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_binary_stl_commits_through_the_same_headless_workflow() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron-binary.stl");
    let document_path = directory.path().join("binary-import.ketchup");
    let source = valid_binary_tetrahedron();
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&shell, "digest-imported-stl"));

    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_persisted_stl(
        &document_path,
        &source,
        ImportLengthUnit::Millimetre,
        "stl.binary",
    );
}

#[test]
fn file_import_stl_review_applies_and_persists_every_declared_unit() {
    let directory = tempfile::tempdir().unwrap();
    for (index, unit, label_key) in [
        (0, ImportLengthUnit::Millimetre, "unit-millimetre"),
        (1, ImportLengthUnit::Centimetre, "unit-centimetre"),
        (2, ImportLengthUnit::Metre, "unit-metre"),
        (3, ImportLengthUnit::Inch, "unit-inch"),
        (4, ImportLengthUnit::Foot, "unit-foot"),
    ] {
        let source_path = directory.path().join(format!("unit-{index}.stl"));
        let document_path = directory.path().join(format!("unit-{index}.ketchup"));
        std::fs::write(&source_path, valid_ascii_tetrahedron()).unwrap();
        let script = ScriptedFileDialogs::new()
            .queue_import(ImportFormat::Stl, &source_path)
            .queue_save(&document_path);
        let mut shell = Shell::with_dialogs(script);

        shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
        shell.click_role_and_label(Role::RadioButton, &shell.catalog().text(label_key));
        shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
        shell.click_menu_command("menu-file", AppCommand::Save);

        assert_persisted_stl(&document_path, valid_ascii_tetrahedron(), unit, "stl.ascii");
    }
}

#[test]
fn file_import_stl_cancel_and_every_refusal_leave_canonical_state_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let valid = directory.path().join("valid.stl");
    let zero_facets = directory.path().join("zero-facets.stl");
    let malformed = directory.path().join("truncated-binary.stl");
    let non_manifold = directory.path().join("open-shell.stl");
    let oversized = directory.path().join("oversized.stl");
    std::fs::write(&valid, valid_ascii_tetrahedron()).unwrap();
    let mut zero_facet_source = vec![0_u8; 80];
    zero_facet_source.extend_from_slice(&0_u32.to_le_bytes());
    std::fs::write(&zero_facets, zero_facet_source).unwrap();
    let mut truncated_binary = valid_binary_tetrahedron();
    truncated_binary.pop();
    std::fs::write(&malformed, truncated_binary).unwrap();
    let last_facet = std::str::from_utf8(valid_ascii_tetrahedron())
        .unwrap()
        .find("facet normal 1 1 1")
        .unwrap();
    let mut open_shell = valid_ascii_tetrahedron()[..last_facet].to_vec();
    open_shell.extend_from_slice(b"endsolid tetrahedron\n");
    std::fs::write(&non_manifold, open_shell).unwrap();
    // One byte past the 208,000,260 bytes the reader once refused: read whole and judged by
    // what it holds, not by its size.
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(208_000_261)
        .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Stl)
        .queue_import(ImportFormat::Stl, &valid)
        .queue_import(ImportFormat::Stl, &zero_facets)
        .queue_import(ImportFormat::Stl, &malformed)
        .queue_import(ImportFormat::Stl, &non_manifold)
        .queue_import(ImportFormat::Stl, &oversized);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);
    assert!(before.redo_steps > 0);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert!(
        shell
            .app()
            .action_digest()
            .contains("binary STL triangle count does not match its byte length"),
        "{}",
        shell.app().action_digest()
    );
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
}

#[test]
fn file_import_stl_rejects_source_document_history_and_open_staleness() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.stl");
    let replacement = directory.path().join("replacement.ketchup");
    std::fs::write(&source, valid_ascii_tetrahedron()).unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    let before_source_change = canonical_state(&shell);
    let before_source_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    let mut same_length_change = valid_ascii_tetrahedron().to_vec();
    same_length_change[0] = b'S';
    std::fs::write(&source, same_length_change).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before_source_change, &before_source_history);

    std::fs::write(&source, valid_ascii_tetrahedron()).unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_document_change, &after_document_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
}
