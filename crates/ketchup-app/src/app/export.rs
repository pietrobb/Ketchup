//! Exports of the current model and profiles (DXF, STL, 3MF, GLB, STEP, IGES, CAM, cut lists).

use crate::*;

impl KetchupApp {
    pub(crate) fn homag_mpr_program_name(stem: &str) -> String {
        let mut name = stem
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
            .map(|character| character.to_ascii_uppercase())
            .take(12)
            .collect::<String>();
        while name.len() < 12 {
            name.push('0');
        }
        name
    }

    pub(crate) fn validate_homag_mpr_path(path: &Path) -> Result<(), ExportError> {
        let invalid_name = || ExportError::Destination {
            requirement: "a program name of exactly 12 ASCII letters, digits, '-' or '_'",
        };
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mpr"))
        {
            return Err(ExportError::Destination {
                requirement: "an explicit .mpr destination",
            });
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(invalid_name)?;
        if stem.len() != 12
            || !stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(invalid_name());
        }
        Ok(())
    }

    pub(crate) fn choose_export_path(&mut self, extension: &str) -> Option<PathBuf> {
        let (filter_key, suffix) = match extension {
            "dxf" => ("file-filter-dxf", "dxf"),
            "step" => ("file-filter-step", "step"),
            "iges" => ("file-filter-iges", "iges"),
            "stl" => ("file-filter-stl", "stl"),
            "3mf" => ("file-filter-3mf", "3mf"),
            "glb" => ("file-filter-glb", "glb"),
            "ketchup-view" => ("file-filter-viewer", "ketchup-view"),
            "csv" => ("file-filter-general-bom", "csv"),
            "xlsx" => ("file-filter-cut-list-xlsx", "xlsx"),
            "nc" => ("file-filter-cam-gcode", "nc"),
            "mpr" => ("file-filter-homag-mpr", "mpr"),
            "btlx" => ("file-filter-btlx", "btlx"),
            "svg" => ("file-filter-svg", "svg"),
            "pdf" => ("file-filter-pdf", "pdf"),
            "dwg" => ("file-filter-dwg", "dwg"),
            _ => unreachable!(
                "the File menu and the takeoff and drawings windows export only DXF, STEP, IGES, STL, 3MF, GLB, KETCHUP-VIEW, CSV, XLSX, NC, MPR, BTLx, SVG, PDF and DWG"
            ),
        };
        let filter_label = self.catalog.text(filter_key);
        let stem = self
            .file
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.trim().is_empty())
            .unwrap_or("Untitled");
        let suggested_name = if extension == "mpr" {
            format!("{}.mpr", Self::homag_mpr_program_name(stem))
        } else {
            format!("{stem}.{suffix}")
        };
        self.dialogs.pick_export_path(ExportRequest {
            filter_label: &filter_label,
            extension,
            suggested_name: &suggested_name,
        })
    }

    pub(crate) fn export_current_profiles_dxf_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dxf"))
        {
            export_visible_profiles_dxf(&snapshot).map_err(ExportError::failed)
        } else {
            Err(ExportError::Destination {
                requirement: "an explicit .dxf destination; profiles are not written as DWG (a drawing sheet is, from File > Drawings)",
            })
        }
        .and_then(|bundle| {
            let report_path = path.with_extension("dxf.loss.txt");
            let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
            let evidence = dxf_profile_export_evidence(path, &bundle);
            self.authorize_export_bundle(
                ExportConsent {
                    class: HighRiskClass::LossyConversion,
                    operation: "export-current-profiles-dxf-with-loss-report",
                    title_key: "dialog-export-dxf-title",
                    risk_key: "dialog-export-dxf-risk",
                    overwrite_title_key: "dialog-export-overwrite-title",
                    overwrite_operations: [
                        "overwrite-current-profiles-dxf-export",
                        "overwrite-current-profiles-dxf-loss-report",
                    ],
                },
                path,
                &report_path,
                &precondition,
                &evidence,
            )?;
            write_export_bundle(path, &bundle.dxf, &report_path, bundle.loss_report.as_bytes(), &precondition)?;
            Ok(Written::with_loss_report(path, &report_path))
        });
        self.report_export_outcome("dxf", path, result)
    }

    pub(crate) fn export_current_model_stl_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = self
            .current_visible_mesh_scene(&snapshot)
            .and_then(|scene| {
                if scene.iter().any(|body| {
                    matches!(
                        body.source,
                        CurrentVisibleMeshSource::Exact(ref package)
                            if matches!(package.as_ref(), ExactBodyPackage::Imported(_))
                    )
                }) {
                    return Err(ExportError::UnsupportedContent {
                        reason: "imported STEP bounds proxies cannot be exported as STL",
                    });
                }
                let bodies = scene
                    .iter()
                    .map(|body| MeshExportBody {
                        source: body.source.as_export_source(),
                        transform: body.occurrence.transform,
                    })
                    .collect::<Vec<_>>();
                model_stl_export(&snapshot, &bodies).map_err(ExportError::failed)
            })
            .and_then(|bundle| {
                let report_path = path.with_extension("stl.loss.txt");
                let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
                let evidence = exact_stl_export_evidence(path, &bundle);
                self.authorize_export_bundle(
                    ExportConsent {
                        class: HighRiskClass::LossyConversion,
                        operation: "export-current-model-stl-with-loss-report",
                        title_key: "dialog-export-lossy-title",
                        risk_key: "dialog-export-lossy-risk",
                        overwrite_title_key: "dialog-export-overwrite-title",
                        overwrite_operations: [
                            "overwrite-current-model-stl-export",
                            "overwrite-current-model-stl-loss-report",
                        ],
                    },
                    path,
                    &report_path,
                    &precondition,
                    &evidence,
                )?;
                write_export_bundle(
                    path,
                    bundle.mesh_stl.as_bytes(),
                    &report_path,
                    bundle.loss_report.as_bytes(),
                    &precondition,
                )?;
                Ok(Written::with_loss_report(path, &report_path))
            });
        self.report_export_outcome("stl", path, result)
    }

    pub(crate) fn export_current_model_three_mf_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = self
            .current_visible_mesh_scene(&snapshot)
            .and_then(|scene| {
                let instances = scene
                    .iter()
                    .map(|body| MeshThreeMfInstance {
                        source: body.source.as_export_source(),
                        occurrence: &body.occurrence,
                    })
                    .collect::<Vec<_>>();
                model_three_mf_export(&snapshot, &instances).map_err(ExportError::failed)
            })
            .and_then(|bundle| {
                let report_path = path.with_extension("3mf.loss.txt");
                let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
                let evidence = exact_three_mf_export_evidence(path, &bundle);
                self.authorize_export_bundle(
                    ExportConsent {
                        class: HighRiskClass::LossyConversion,
                        operation: "export-current-model-3mf-with-loss-report",
                        title_key: "dialog-export-lossy-title",
                        risk_key: "dialog-export-lossy-risk",
                        overwrite_title_key: "dialog-export-overwrite-title",
                        overwrite_operations: [
                            "overwrite-current-model-3mf-export",
                            "overwrite-current-model-3mf-loss-report",
                        ],
                    },
                    path,
                    &report_path,
                    &precondition,
                    &evidence,
                )?;
                write_export_bundle(
                    path,
                    &bundle.three_mf,
                    &report_path,
                    bundle.loss_report.as_bytes(),
                    &precondition,
                )?;
                Ok(Written::with_loss_report(path, &report_path))
            });
        self.report_export_outcome("3mf", path, result)
    }

    pub(crate) fn export_current_model_glb_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = self
            .current_visible_mesh_scene(&snapshot)
            .and_then(|scene| {
                let instances = scene
                    .iter()
                    .map(|body| MeshGlbInstance {
                        source: body.source.as_export_source(),
                        occurrence: &body.occurrence,
                    })
                    .collect::<Vec<_>>();
                model_glb_export(&snapshot, &instances).map_err(ExportError::failed)
            })
            .and_then(|bundle| {
                let report_path = path.with_extension("glb.loss.txt");
                let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
                let evidence = exact_glb_export_evidence(path, &bundle);
                self.authorize_export_bundle(
                    ExportConsent {
                        class: HighRiskClass::LossyConversion,
                        operation: "export-current-model-blender-glb-with-loss-report",
                        title_key: "dialog-export-blender-title",
                        risk_key: "dialog-export-blender-risk",
                        overwrite_title_key: "dialog-export-overwrite-title",
                        overwrite_operations: [
                            "overwrite-current-model-blender-glb-export",
                            "overwrite-current-model-blender-glb-loss-report",
                        ],
                    },
                    path,
                    &report_path,
                    &precondition,
                    &evidence,
                )?;
                write_export_bundle(
                    path,
                    &bundle.glb,
                    &report_path,
                    bundle.loss_report.as_bytes(),
                    &precondition,
                )?;
                Ok(Written::with_loss_report(path, &report_path))
            });
        self.report_export_outcome("glb", path, result)
    }

    /// COLLADA for SketchUp from the same visible meshes as the GLB export.
    pub(crate) fn export_current_model_dae_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = self
            .current_visible_mesh_scene(&snapshot)
            .and_then(|scene| {
                let instances = scene
                    .iter()
                    .map(|body| MeshGlbInstance {
                        source: body.source.as_export_source(),
                        occurrence: &body.occurrence,
                    })
                    .collect::<Vec<_>>();
                model_dae_export(&snapshot, &instances).map_err(ExportError::failed)
            })
            .and_then(|bundle| {
                let report_path = path.with_extension("dae.loss.txt");
                let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
                let evidence = dae_export_evidence(path, &bundle);
                self.authorize_export_bundle(
                    ExportConsent {
                        class: HighRiskClass::LossyConversion,
                        operation: "export-current-model-dae-with-loss-report",
                        title_key: "dialog-export-lossy-title",
                        risk_key: "dialog-export-lossy-risk",
                        overwrite_title_key: "dialog-export-overwrite-title",
                        overwrite_operations: [
                            "overwrite-current-model-dae-export",
                            "overwrite-current-model-dae-loss-report",
                        ],
                    },
                    path,
                    &report_path,
                    &precondition,
                    &evidence,
                )?;
                write_export_bundle(
                    path,
                    bundle.dae.as_bytes(),
                    &report_path,
                    bundle.loss_report.as_bytes(),
                    &precondition,
                )?;
                Ok(Written::with_loss_report(path, &report_path))
            });
        self.report_export_outcome("dae", path, result)
    }

    pub(crate) fn btlx_export_options(&self) -> BtlxExportOptions {
        let profile_processing_request = match self.btlx_profile_strategy {
            BtlxProfileStrategy::PortableFreeContour => {
                BtlxProfileProcessingRequest::PortableFreeContour
            }
            BtlxProfileStrategy::EdgeSawCutsThenMillContour => {
                BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
                    intermediate_saw_cuts: self.btlx_intermediate_saw_cuts,
                }
            }
        };
        BtlxExportOptions {
            profile_processing_request,
        }
    }

    pub(crate) fn sole_exportable_sheet_metal_feature_id(&self) -> Result<FeatureId, ExportError> {
        let snapshot = self.document.current();
        let feature_ids = snapshot
            .features()
            .filter(|feature| {
                matches!(feature.kind(), FeatureKind::SheetMetal(_))
                    && !snapshot.feature_is_suppressed(feature.id())
            })
            .map(|feature| feature.id())
            .collect::<Vec<_>>();
        match feature_ids.as_slice() {
            [feature_id] => Ok(*feature_id),
            [] => Err(ExportError::NothingToExport {
                subject: "sheet-metal feature",
            }),
            _ => Err(ExportError::Ambiguous {
                subject: "unsuppressed sheet-metal feature",
            }),
        }
    }

    pub(crate) fn export_current_sheet_metal_manufacturing_to(
        &mut self,
        feature_id: FeatureId,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = (|| -> Result<Written, ExportError> {
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dxf"))
            {
                return Err(ExportError::Destination {
                    requirement: "an explicit .dxf destination",
                });
            }
            let projection = project_sheet_metal_manufacturing(&snapshot, feature_id)
                .map_err(ExportError::failed)?;
            let artifacts = projection
                .artifacts(&snapshot)
                .map_err(ExportError::failed)?;
            let bend_table_path = path.with_extension("bends.csv");
            let precondition = ExportBundlePrecondition::capture(path, &bend_table_path)?;
            let evidence = export_bundle_evidence(
                b"ketchup.sheet-metal-manufacturing-export.v1",
                path,
                &artifacts.flat_pattern_dxf,
                &bend_table_path,
                &artifacts.bend_table_csv,
            );
            self.authorize_export_bundle(
                ExportConsent {
                    class: HighRiskClass::ReleaseManufacturingExportWithWarnings,
                    operation: "release-sheet-metal-flat-pattern-and-bend-table",
                    title_key: "dialog-export-general-fabrication-title",
                    risk_key: "dialog-export-general-fabrication-risk",
                    overwrite_title_key: "dialog-export-overwrite-title",
                    overwrite_operations: [
                        "overwrite-sheet-metal-flat-pattern",
                        "overwrite-sheet-metal-bend-table",
                    ],
                },
                path,
                &bend_table_path,
                &precondition,
                &evidence,
            )?;
            projection
                .artifacts(&self.document.current())
                .map_err(ExportError::failed)?;
            write_export_bundle(
                path,
                &artifacts.flat_pattern_dxf,
                &bend_table_path,
                &artifacts.bend_table_csv,
                &precondition,
            )?;
            Ok(Written::with_companion(path, &bend_table_path))
        })();
        self.report_export_outcome("general-fabrication", path, result)
    }

    pub(crate) fn export_current_general_fabrication_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = (|| -> Result<Written, ExportError> {
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
            {
                return Err(ExportError::Destination {
                    requirement: "an explicit .csv destination",
                });
            }
            let projection = self.current_general_fabrication_projection()?;
            let bom = projection
                .bom_export(&snapshot)
                .map_err(ExportError::failed)?;
            let drawings = projection
                .drawing_svg(&snapshot)
                .map_err(ExportError::failed)?;
            let drawing_path = path.with_extension("drawings.svg");
            let precondition = ExportBundlePrecondition::capture(path, &drawing_path)?;
            let evidence = export_bundle_evidence(
                b"ketchup.general-fabrication-export.v1",
                path,
                &bom,
                &drawing_path,
                &drawings,
            );
            self.authorize_export_bundle(
                ExportConsent {
                    class: HighRiskClass::ReleaseManufacturingExportWithWarnings,
                    operation: "release-general-fabrication-bom-and-drawings",
                    title_key: "dialog-export-general-fabrication-title",
                    risk_key: "dialog-export-general-fabrication-risk",
                    overwrite_title_key: "dialog-export-overwrite-title",
                    overwrite_operations: [
                        "overwrite-general-fabrication-bom",
                        "overwrite-general-fabrication-drawings",
                    ],
                },
                path,
                &drawing_path,
                &precondition,
                &evidence,
            )?;
            write_export_bundle(path, &bom, &drawing_path, &drawings, &precondition)?;
            Ok(Written::with_companion(path, &drawing_path))
        })();
        self.report_export_outcome("general-fabrication", path, result)
    }

    pub(crate) fn export_current_member_cut_list_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = (|| -> Result<Written, ExportError> {
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
            {
                return Err(ExportError::Destination {
                    requirement: "an explicit .csv destination",
                });
            }
            let weldment = self
                .current_general_fabrication_projection()?
                .weldment
                .ok_or(ExportError::NothingToExport {
                    subject: "weldment",
                })?;
            let cut_list = weldment
                .cut_list_export(&snapshot)
                .map_err(ExportError::failed)?;
            let drawing = weldment
                .drawing_svg(&snapshot)
                .map_err(ExportError::failed)?;
            let drawing_path = path.with_extension("svg");
            let precondition = ExportBundlePrecondition::capture(path, &drawing_path)?;
            let evidence = export_bundle_evidence(
                b"ketchup.weldment-cut-list-export.v1",
                path,
                &cut_list,
                &drawing_path,
                &drawing,
            );
            self.authorize_export_bundle(
                ExportConsent {
                    class: HighRiskClass::ReleaseManufacturingExportWithWarnings,
                    operation: "release-weldment-cut-list-and-drawing",
                    title_key: "dialog-export-general-fabrication-title",
                    risk_key: "dialog-export-general-fabrication-risk",
                    overwrite_title_key: "dialog-export-overwrite-title",
                    overwrite_operations: [
                        "overwrite-weldment-cut-list",
                        "overwrite-weldment-drawing",
                    ],
                },
                path,
                &drawing_path,
                &precondition,
                &evidence,
            )?;
            let current = self.document.current();
            let current_weldment = self
                .current_general_fabrication_projection()?
                .weldment
                .ok_or(ExportError::NothingToExport {
                    subject: "weldment",
                })?;
            if current_weldment
                .cut_list_export(&current)
                .map_err(ExportError::failed)?
                != cut_list
                || current_weldment
                    .drawing_svg(&current)
                    .map_err(ExportError::failed)?
                    != drawing
            {
                return Err(ExportError::ChangedDuringConsent);
            }
            write_export_bundle(path, &cut_list, &drawing_path, &drawing, &precondition)?;
            Ok(Written::with_companion(path, &drawing_path))
        })();
        self.report_export_outcome("general-fabrication", path, result)
    }

    pub(crate) fn export_current_model_homag_mpr_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = (|| -> Result<Written, ExportError> {
            Self::validate_homag_mpr_path(path)?;
            let projection = self.current_general_fabrication_projection()?;
            let mpr = projection
                .woodwop_mpr_4_0_drill_export(&snapshot)
                .map_err(ExportError::failed)?;
            let expected_sha256 = export_target_sha256(path)?;
            let title = self.catalog.text("dialog-export-homag-mpr-title");
            let risk = self.catalog.text("dialog-export-homag-mpr-risk");
            self.authorize_path_side_effect(
                HighRiskClass::ReleaseManufacturingExportWithWarnings,
                "release-homag-woodwop-mpr",
                &title,
                &risk,
                path,
                &mpr,
            )?;
            if expected_sha256.is_some() {
                let title = self.catalog.text("dialog-export-overwrite-title");
                let risk = self.catalog.text("dialog-export-overwrite-risk");
                self.authorize_path_side_effect(
                    HighRiskClass::Overwrite,
                    "overwrite-homag-woodwop-mpr",
                    &title,
                    &risk,
                    path,
                    &mpr,
                )?;
            }
            let current = self.document.current();
            let current_projection = self.current_general_fabrication_projection()?;
            if current.document_id() != snapshot.document_id()
                || current.revision_id() != snapshot.revision_id()
                || current.canonical_digest() != snapshot.canonical_digest()
                || current_projection
                    .woodwop_mpr_4_0_drill_export(&current)
                    .map_err(ExportError::failed)?
                    != mpr
            {
                return Err(ExportError::ChangedDuringConsent);
            }
            write_export_artifact_if_unchanged(path, &mpr, expected_sha256.as_deref())?;
            Ok(Written::alone(path))
        })();
        self.report_export_outcome("homag-mpr", path, result)
    }

    pub(crate) fn export_current_model_btlx_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let options = self.btlx_export_options();
        let (selected_profile_request, intermediate_saw_cuts, selected_profile_contours) =
            match options.profile_processing_request {
                BtlxProfileProcessingRequest::PortableFreeContour => (
                    "portable FreeContour",
                    "not applicable".to_owned(),
                    "closed simple line/arc contours",
                ),
                BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
                    intermediate_saw_cuts,
                } => (
                    "edge SawContour cuts, then MillContour",
                    intermediate_saw_cuts.to_string(),
                    "rectangular only",
                ),
            };
        let result = (|| -> Result<Written, ExportError> {
            let projection = self.current_general_fabrication_projection()?;
            let btlx = projection
                .btlx_2_3_1_export_with_options(&snapshot, options)
                .map_err(ExportError::failed)?;
            let thread_loss = projection
                .btlx_thread_loss_report(&snapshot)
                .map_err(ExportError::failed)?;
            let support_report = format!(
                "schema=ketchup.btlx-support-report.v1\nsource_digest={}\nformat=BTLx 2.3.1\nstock=rectangular straight timber\ndrilling=circular\nportable_profile_removals=pocket, through-cut, extruded boolean-cut\nportable_profile_contours=closed simple line/arc contours\ndefault_profile_request=edge SawContour cuts, then MillContour\nselected_profile_request={selected_profile_request}\nintermediate_saw_cuts={intermediate_saw_cuts}\nselected_profile_contours={selected_profile_contours}\nunsupported=arc or non-rectangular profile with the edge-saw request; non-rectangular stock; other machining operations\nconcrete_importer_verified=false\nmachine_execution_order_guaranteed=false\n{}",
                snapshot.canonical_digest(),
                thread_loss
            );
            let report_path = path.with_extension("btlx.support.txt");
            let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
            let mut evidence = btlx.clone();
            evidence.extend_from_slice(support_report.as_bytes());
            self.authorize_export_bundle(
                ExportConsent {
                    class: HighRiskClass::ReleaseManufacturingExportWithWarnings,
                    operation: "release-hundegger-btlx-with-support-report",
                    title_key: "dialog-export-btlx-title",
                    risk_key: "dialog-export-btlx-risk",
                    overwrite_title_key: "dialog-export-overwrite-title",
                    overwrite_operations: [
                        "overwrite-hundegger-btlx-export",
                        "overwrite-hundegger-btlx-support-report",
                    ],
                },
                path,
                &report_path,
                &precondition,
                &evidence,
            )?;
            write_export_bundle(
                path,
                &btlx,
                &report_path,
                support_report.as_bytes(),
                &precondition,
            )?;
            Ok(Written::with_support_report(path, &report_path))
        })();
        self.report_export_outcome("btlx", path, result)
    }

    pub(crate) fn export_current_model_step_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.export_exchange_to(super::exchange_export::ExchangeFormat::Step, path)
    }

    pub(crate) fn export_current_model_iges_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.export_exchange_to(super::exchange_export::ExchangeFormat::Iges, path)
    }

    pub(crate) fn show_cam_export_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.cam_export_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("cam-review-title"))
            .id(egui::Id::new("cam-review-export"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(format!(
                    "{} {}",
                    self.catalog.text("cam-review-plan"),
                    pending.plan_id.0
                ));
                if let Some(review) = &pending.review {
                    ui.label(format!("{}: {}", review.units, review.work_offset));
                    ui.label(format!(
                        "T{} · S{} · F{} mm/min",
                        review.tool_number, review.spindle_rpm, review.cutting_feed_mm_per_min
                    ));
                    ui.label(format!(
                        "{}: {:.6} mm³ · {}: {:.6} mm³",
                        self.catalog.text("cam-review-removed"),
                        review.removed_stock_mm3,
                        self.catalog.text("cam-review-gouge"),
                        review.gouge_mm3
                    ));
                    ui.monospace(format!("SHA-256 {}", review.content_digest));
                } else {
                    ui.label(self.catalog.text("cam-review-not-previewed"));
                }
                ui.horizontal(|ui| {
                    if ui.button(self.catalog.text("cam-review-preview")).clicked() {
                        preview = true;
                    }
                    if ui
                        .add_enabled(
                            pending.review.is_some(),
                            egui::Button::new(self.catalog.text("cam-review-confirm-export")),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button(self.catalog.text("dialog-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if preview {
            let result = (|| {
                let snapshot = self.document.current();
                let mutation_epoch = self.document.mutation_epoch();
                let plan = snapshot
                    .cam_plan(pending.plan_id)
                    .ok_or_else(|| self.catalog.refusal("cam-review-no-plan"))?;
                let operation = plan
                    .default_facing_operation(&snapshot, 1)
                    .map_err(|error| failed("cam.operation", error))?;
                let worker_path = self.exact_worker_executable()?;
                self.reviews.cam_reviews.set_worker_path(worker_path);
                self.reviews
                    .cam_reviews
                    .preview(
                        &snapshot,
                        mutation_epoch,
                        CamReviewRequest {
                            plan_id: pending.plan_id,
                            operations: vec![operation],
                            fixtures: Vec::new(),
                            dialect: CamPostprocessorDialect::IsoMetricGCode,
                        },
                        &AtomicBool::new(false),
                    )
                    .map_err(|error| failed("cam.review", error))
            })();
            match result {
                Ok(review) => {
                    pending.review = Some(review);
                    self.digest = self.catalog.text("cam-review-ready");
                }
                Err(error) => self.digest = error.reason_text().to_owned(),
            }
        }
        if confirm
            && let Some(review) = &pending.review
            && let Some(path) = self.choose_export_path("nc")
        {
            let snapshot = self.document.current();
            let mutation_epoch = self.document.mutation_epoch();
            let result = self.reviews.cam_reviews.export(
                &snapshot,
                mutation_epoch,
                &review.token,
                &path,
                true,
                &AtomicBool::new(false),
            );
            match result {
                Ok(_) => {
                    self.digest = self.catalog.text("cam-review-exported");
                    cancel = true;
                }
                Err(error) => self.digest = error.to_string(),
            }
        }
        if open && !cancel {
            self.reviews.cam_export_dialog = Some(pending);
        }
    }
}

fn dxf_profile_export_evidence(path: &Path, bundle: &DxfProfileExport) -> Vec<u8> {
    export_bundle_evidence(
        b"ketchup.current-profiles-dxf-export.v1",
        path,
        &bundle.dxf,
        &path.with_extension("dxf.loss.txt"),
        bundle.loss_report.as_bytes(),
    )
}

fn exact_stl_export_evidence(path: &Path, bundle: &ExactStlExport) -> Vec<u8> {
    export_bundle_evidence(
        b"ketchup.current-model-stl-export.v1",
        path,
        bundle.mesh_stl.as_bytes(),
        &path.with_extension("stl.loss.txt"),
        bundle.loss_report.as_bytes(),
    )
}

fn exact_three_mf_export_evidence(path: &Path, bundle: &ExactThreeMfExport) -> Vec<u8> {
    export_bundle_evidence(
        b"ketchup.current-model-3mf-export.v1",
        path,
        &bundle.three_mf,
        &path.with_extension("3mf.loss.txt"),
        bundle.loss_report.as_bytes(),
    )
}

fn exact_glb_export_evidence(path: &Path, bundle: &ExactGlbExport) -> Vec<u8> {
    export_bundle_evidence(
        b"ketchup.current-model-blender-glb-export.v1",
        path,
        &bundle.glb,
        &path.with_extension("glb.loss.txt"),
        bundle.loss_report.as_bytes(),
    )
}

fn dae_export_evidence(path: &Path, bundle: &DaeExport) -> Vec<u8> {
    export_bundle_evidence(
        b"ketchup.current-model-dae-export.v1",
        path,
        bundle.dae.as_bytes(),
        &path.with_extension("dae.loss.txt"),
        bundle.loss_report.as_bytes(),
    )
}

pub(super) fn export_bundle_evidence(
    domain: &[u8],
    primary_path: &Path,
    primary: &[u8],
    report_path: &Path,
    report: &[u8],
) -> Vec<u8> {
    let mut evidence = domain.to_vec();
    for (path, artifact) in [(primary_path, primary), (report_path, report)] {
        let path = path.to_string_lossy();
        evidence.extend_from_slice(&(path.len() as u64).to_le_bytes());
        evidence.extend_from_slice(path.as_bytes());
        evidence.extend_from_slice(&(artifact.len() as u64).to_le_bytes());
        evidence.extend_from_slice(artifact);
    }
    evidence
}
