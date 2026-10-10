//! File > Export for Viewer: collects the current model, the author's Viewer
//! settings and the shared scenes, then writes the package and its loss report.

use super::viewer_settings::ViewerSettings;
use crate::program_evaluation::Lookup;
use crate::*;
use ketchup_manufacturing::viewer_export::{
    DefinitionMetadata, OccurrenceMetadata, ViewerExport, ViewerExportData, model_viewer_export,
};
use ketchup_model::document::SavedViewId;

impl KetchupApp {
    /// Write the current model to `path`; `false` with a reported error otherwise.
    pub(crate) fn export_current_model_viewer_to(&mut self, path: &Path) -> bool {
        self.side_effect_receipts.clear();
        let result = (|| -> Result<(), ExportError> {
            if !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("ketchup-view"))
            {
                return Err(ExportError::Destination {
                    requirement: "an explicit .ketchup-view destination",
                });
            }
            let viewport = self.camera.viewport_rect.ok_or_else(|| {
                source_error("viewport", "The 3D viewport has not been laid out yet.")
            })?;
            let settings = self.stored_viewer_settings().map_err(|error| {
                source_error("viewer_settings", "The stored Viewer notes cannot be read.")
                    .caused_by(error)
            })?;
            let saved_views = self.viewer_export_scenes(&settings);
            if saved_views.is_empty() {
                return Err(
                    source_error("scenes", "No shared saved scene is available.")
                        .fix_hint("Save a view under Scenes and tick Viewer, then export again.")
                        .into(),
                );
            }
            let bundle = self.current_model_viewer_export(&saved_views, viewport)?;
            let mut bytes = Vec::new();
            bundle.package.write(&mut bytes)?;
            let report_path = path.with_extension("ketchup-view.loss.txt");
            let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
            let evidence = super::export::export_bundle_evidence(
                b"ketchup.current-model-viewer-export.v1",
                path,
                &bytes,
                &report_path,
                bundle.loss_report.as_bytes(),
            );
            self.authorize_export_bundle(
                ExportConsent {
                    class: HighRiskClass::LossyConversion,
                    operation: "export-current-model-viewer-with-loss-report",
                    title_key: "dialog-export-viewer-title",
                    risk_key: "dialog-export-viewer-risk",
                    overwrite_title_key: "dialog-export-viewer-overwrite-title",
                    overwrite_operations: [
                        "overwrite-current-model-viewer-export",
                        "overwrite-current-model-viewer-loss-report",
                    ],
                },
                path,
                &report_path,
                &precondition,
                &evidence,
            )?;
            Ok(write_export_bundle(
                path,
                &bytes,
                &report_path,
                bundle.loss_report.as_bytes(),
                &precondition,
            )?)
        })();
        self.report_export_outcome("viewer", path, result)
    }

    /// Build a read-only package from current accepted geometry, including hidden bodies.
    /// The first selected saved view is the opening scene; no CAD evaluation is started.
    pub fn current_model_viewer_export(
        &self,
        saved_views: &[SavedViewId],
        viewport: Rect,
    ) -> Result<ViewerExport, Rejection> {
        let start_scene = saved_views.first().ok_or_else(|| {
            source_error(
                "scenes",
                "Select at least one saved scene for the opening view.",
            )
        })?;
        let snapshot = self.document.current();
        let settings = self.stored_viewer_settings().map_err(|error| {
            source_error("viewer_settings", "The stored Viewer notes cannot be read.")
                .caused_by(error)
        })?;
        let mut scene = self.current_mesh_scene(&snapshot, true).map_err(|error| {
            source_error(
                "geometry",
                "A component has no current exportable body geometry.",
            )
            .caused_by(error)
        })?;
        // Unshared components leave the package entirely, not only the scenes.
        scene.retain(|body| {
            !settings
                .excluded_roots
                .contains(&body.occurrence.instance_path.root_occurrence().0)
        });
        if scene.is_empty() {
            return Err(source_error(
                "components",
                "Every component is excluded from Viewer sharing.",
            ));
        }
        let paths = scene
            .iter()
            .map(|body| body.occurrence.instance_path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut data = self.viewer_source_data(&paths, saved_views, viewport)?;
        data.start_scene = Some(start_scene.0);
        apply_author_notes(&settings, &mut data);
        let instances = scene
            .iter()
            .map(|body| MeshGlbInstance {
                source: body.source.as_export_source(),
                occurrence: &body.occurrence,
            })
            .collect::<Vec<_>>();
        let mut export = model_viewer_export(&snapshot, &instances, &data)?;
        export.loss_report.push_str("viewer_geometry=display tessellation, not manufacturing CAD; all shared body occurrences, including hidden components; scene visibility is not removal of confidential geometry\n");
        if !settings.excluded_roots.is_empty() {
            use std::fmt::Write;
            writeln!(
                export.loss_report,
                "viewer_excluded_components={}; not in the package",
                settings.excluded_roots.len()
            )
            .expect("writing to a String cannot fail");
        }
        let omitted_dimensions = snapshot.persistent_dimensions().count();
        if omitted_dimensions > 0 {
            use std::fmt::Write;
            writeln!(
                export.loss_report,
                "viewer_omitted_dimensions={omitted_dimensions}; source dimensions have no stored 3D anchors or label offsets"
            )
            .expect("writing to a String cannot fail");
        }
        Ok(export)
    }

    /// Prepare metadata for evaluated export paths without activating scenes or changing CAD.
    pub fn viewer_source_data(
        &self,
        paths: &[InstancePath],
        saved_views: &[SavedViewId],
        viewport: Rect,
    ) -> Result<ViewerExportData, Rejection> {
        let mut data = ViewerExportData {
            saved_views: self.viewer_saved_view_contexts(saved_views, viewport)?,
            ..ViewerExportData::default()
        };
        let snapshot = self.document.current();
        let resolved = paths
            .iter()
            .map(|path| {
                snapshot
                    .resolve_instance_path(path)
                    .map(|item| (path, item.definition_id))
                    .map_err(|error| {
                        source_error(
                            "occurrences.path",
                            "An export path no longer exists in this document.",
                        )
                        .caused_by(error)
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let Some(source) = self.document.current_rule_program() else {
            return Ok(data);
        };
        let evaluated = match self.program_evaluations.try_get(source) {
            Lookup::Pending => {
                return Err(source_error(
                    "program",
                    "The current program metadata is still being evaluated.",
                ));
            }
            Lookup::Ready(Err(error)) => {
                return Err(source_error(
                    "program",
                    "The current program metadata could not be evaluated.",
                )
                .caused_by(error));
            }
            Lookup::Ready(Ok(evaluated)) => evaluated,
        };
        let parts = evaluated.model.parts_by_name();
        for (path, definition_id) in resolved {
            let Some(name) = ketchup_application::rule_program_part_name(&snapshot, path) else {
                continue;
            };
            let Some(part) = parts.get(name.as_str()) else {
                // Assembly ancestors and manually drawn profiles have no program part metadata.
                continue;
            };
            let metadata =
                data.definitions
                    .entry(definition_id)
                    .or_insert_with(|| DefinitionMetadata {
                        authored_size_mm: Some(part.size_mm),
                        ..DefinitionMetadata::default()
                    });
            if metadata.authored_size_mm != Some(part.size_mm) {
                return Err(source_error(
                    &name,
                    "Shared component occurrences have conflicting authored dimensions.",
                ));
            }
            data.occurrences.insert(
                path.clone(),
                OccurrenceMetadata {
                    material: part.material.clone(),
                    attributes: part.attributes.clone(),
                    ..OccurrenceMetadata::default()
                },
            );
        }
        Ok(data)
    }
}

/// Notes of exported definitions and paths only; the exporter ignores notes of
/// components that are not in the package.
fn apply_author_notes(settings: &ViewerSettings, data: &mut ViewerExportData) {
    for (&id, note) in &settings.definition_notes {
        data.definitions.entry(DefinitionId(id)).or_default().note = Some(note.clone());
    }
    for (path, note) in &settings.occurrence_notes {
        data.occurrences.entry(path.clone()).or_default().note = Some(note.clone());
    }
}

fn source_error(target: &str, reason: &str) -> Rejection {
    Rejection::new("viewer.source_data", RejectionPhase::Validation)
        .target(target)
        .reason(reason)
        .fix_hint("Use current component paths and wait for the current program to finish evaluating before exporting.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::document::{RuleProgramSource, SavedViewId};

    fn viewport() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))
    }

    fn source() -> RuleProgramSource {
        RuleProgramSource {
            file_name: "viewer.star".into(),
            source: "a=box(\"a\", (20,30,40), material=\"HPL\", attributes={\"texture_axis\":\"x\",\"note\":\"inert attribute\"})\ng=group(\"inner\",[a])\nc=component(\"assembly\",[g])\ninstance(\"second\",c,at=(10000,0,0))".into(),
            overrides: BTreeMap::new(),
        }
    }

    #[test]
    fn shared_nested_parts_keep_source_material_attributes_and_local_axes() {
        let mut app = KetchupApp::new();
        app.apply_program_source(source(), true)
            .expect("apply program");
        let snapshot = app.document.current();
        let paths = snapshot
            .scene_query()
            .into_iter()
            .map(|item| item.instance_path)
            .collect::<Vec<_>>();
        let before = (
            snapshot.canonical_digest(),
            snapshot.revision_id(),
            app.undo_step_count(),
        );
        let data = app
            .viewer_source_data(&paths, &[], viewport())
            .expect("source data");
        assert_eq!(data.occurrences.len(), 2);
        assert_eq!(data.definitions.len(), 1);
        for (path, metadata) in &data.occurrences {
            assert!(!path.is_root());
            assert_eq!(metadata.material.as_deref(), Some("HPL"));
            assert_eq!(
                metadata.attributes.get("texture_axis").map(String::as_str),
                Some("x")
            );
            assert_eq!(
                metadata.attributes.get("note").map(String::as_str),
                Some("inert attribute")
            );
            assert!(
                metadata.note.is_none(),
                "attributes are not silently promoted to author notes"
            );
        }
        assert_eq!(
            data.definitions
                .values()
                .next()
                .expect("shared definition")
                .authored_size_mm,
            Some([20.0, 30.0, 40.0])
        );
        let after = app.document.current();
        assert_eq!(
            before,
            (
                after.canonical_digest(),
                after.revision_id(),
                app.undo_step_count()
            )
        );
        assert_eq!(app.program_evaluations.on_ui_thread(), 0);
    }

    #[test]
    fn detached_program_metadata_is_not_guessed_from_matching_part_names() {
        let mut app = KetchupApp::new();
        app.apply_program_source(source(), true)
            .expect("apply program");
        let snapshot = app.document.current();
        let root = snapshot.occurrences().next().expect("root occurrence").id();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: root,
                    transform: Transform::from_translation(500.0, 0.0, 0.0).expect("placement"),
                },
            ]))
            .expect("manual edit");
        assert!(app.document.current_rule_program().is_none());
        let paths = app
            .document
            .current()
            .scene_query()
            .into_iter()
            .map(|item| item.instance_path)
            .collect::<Vec<_>>();
        let data = app
            .viewer_source_data(&paths, &[], viewport())
            .expect("manual model metadata");
        assert!(data.definitions.is_empty());
        assert!(data.occurrences.is_empty());
        assert_eq!(app.program_evaluations.on_ui_thread(), 0);
    }

    #[test]
    fn package_roundtrip_connects_nested_geometry_metadata_and_scene_cameras() {
        let mut app = KetchupApp::new();
        app.apply_program_source(source(), true).expect("program");
        let snapshot = app.document.current();
        let definitions = snapshot
            .scene_query()
            .into_iter()
            .map(|item| item.definition_id)
            .collect::<BTreeSet<_>>();
        for definition in definitions {
            for producer in exact_body_terminal_features(&snapshot, definition)
                .expect("terminals")
                .values()
            {
                let package = ketchup_model::testing::box_package(
                    &snapshot,
                    definition,
                    *producer,
                    "viewer-fixture",
                    &[],
                )
                .expect("geometry fixture");
                assert!(app.headless_install_exact_package(package));
            }
        }
        let first = app.save_view("Parallel").expect("save view");
        app.camera.projection_mode = ProjectionMode::Perspective;
        app.camera.pan = Vec2::new(47.0, -23.0);
        let second = app.save_view("Perspective").expect("save view");
        let before = (
            app.document_revision(),
            app.undo_step_count(),
            app.camera.pan,
        );
        let export = app
            .current_model_viewer_export(&[second, first], viewport())
            .expect("integrated export");
        let mut bytes = Vec::new();
        export.package.write(&mut bytes).expect("write package");
        let reopened = ketchup_view_format::Package::read(bytes.as_slice()).expect("read package");
        assert_eq!(reopened, export.package);
        assert_eq!(reopened.manifest.start_scene, Some(second.0));
        assert_eq!(reopened.manifest.scenes.len(), 2);
        assert!(matches!(
            reopened
                .manifest
                .scenes
                .iter()
                .find(|s| s.id == second.0)
                .expect("perspective scene")
                .camera
                .projection,
            ketchup_view_format::Projection::Perspective { .. }
        ));
        let parts = reopened
            .manifest
            .occurrences
            .iter()
            .filter(|item| item.material.as_deref() == Some("HPL"))
            .collect::<Vec<_>>();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].definition_id, parts[1].definition_id);
        assert_ne!(
            parts[0].path.root_occurrence_id,
            parts[1].path.root_occurrence_id
        );
        for part in parts {
            assert!(!part.path.steps.is_empty());
            assert_eq!(part.attributes["texture_axis"], "x");
            let definition = reopened
                .manifest
                .definitions
                .iter()
                .find(|d| d.id == part.definition_id)
                .expect("definition");
            assert_eq!(
                definition.local_dimensions,
                Some(ketchup_view_format::LocalDimensions::AuthoredAxes {
                    size_mm: [20.0, 30.0, 40.0]
                })
            );
        }
        assert_eq!(
            before,
            (
                app.document_revision(),
                app.undo_step_count(),
                app.camera.pan
            )
        );
        assert_eq!(app.program_evaluations.on_ui_thread(), 0);
    }

    #[test]
    fn package_keeps_hidden_geometry_for_saved_scene_and_reports_unanchored_dimensions() {
        use ketchup_model::document::{
            DimensionDisplayUnit, DimensionPresentation, PersistentDimension,
            PersistentDimensionId, PersistentDimensionTarget, TagId,
        };
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: TagId(1),
                    name: "Part layer".into(),
                    visible: true,
                },
                CanonicalCommand::SetOccurrenceTags {
                    id: OccurrenceId(1),
                    tags: BTreeSet::from([TagId(1)]),
                },
                CanonicalCommand::UpsertPersistentDimension(
                    PersistentDimension::new(
                        PersistentDimensionId(1),
                        "Width",
                        PersistentDimensionTarget::FeatureParameter(
                            FeatureParameterTarget::new(
                                FeatureId(2),
                                "bounds.width",
                                ParameterValueType::Length,
                            )
                            .expect("target"),
                        ),
                        DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2)
                            .expect("presentation"),
                    )
                    .expect("dimension"),
                ),
            ]))
            .expect("tag and dimension");
        let shown = app.save_view("Shown").expect("save shown");
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetTagVisibility {
                    id: TagId(1),
                    visible: false,
                },
            ]))
            .expect("hide layer");
        let hidden = app.save_view("Hidden").expect("save hidden");
        crate::tests::install_initial_graph_result(&mut app);
        let snapshot = app.document.current();
        assert!(matches!(
            app.current_visible_mesh_scene(&snapshot),
            Err(ExportError::NothingToExport { .. })
        ));
        let before = (app.document_revision(), app.undo_step_count());
        let export = app
            .current_model_viewer_export(&[hidden, shown], viewport())
            .expect("export hidden geometry");
        let mut bytes = Vec::new();
        export.package.write(&mut bytes).expect("write");
        let reopened = ketchup_view_format::Package::read(bytes.as_slice()).expect("read");
        assert_eq!(reopened.manifest.occurrences.len(), 1);
        assert!(
            reopened
                .manifest
                .scenes
                .iter()
                .find(|s| s.id == shown.0)
                .expect("shown")
                .hidden
                .is_empty()
        );
        assert_eq!(
            reopened
                .manifest
                .scenes
                .iter()
                .find(|s| s.id == hidden.0)
                .expect("hidden")
                .hidden,
            vec![reopened.manifest.occurrences[0].path.clone()]
        );
        assert!(reopened.manifest.dimensions.is_empty());
        assert!(export.loss_report.contains("viewer_omitted_dimensions=1"));
        assert!(
            export
                .loss_report
                .contains("scene visibility is not removal of confidential geometry")
        );
        assert!(!snapshot.tag(TagId(1)).expect("tag").visible());
        assert_eq!(before, (app.document_revision(), app.undo_step_count()));
    }

    #[test]
    fn package_rejects_missing_or_stale_geometry_and_invalid_scene_selection() {
        let mut app = KetchupApp::new();
        let scene = app.save_view("Scene").expect("scene");
        let rejection = app
            .current_model_viewer_export(&[scene], viewport())
            .err()
            .expect("missing geometry");
        assert!(!rejection.causes().is_empty());
        crate::tests::install_initial_graph_result(&mut app);
        assert!(app.current_model_viewer_export(&[], viewport()).is_err());
        assert!(
            app.current_model_viewer_export(&[SavedViewId(u64::MAX)], viewport())
                .is_err()
        );
        assert!(
            app.current_model_viewer_export(&[scene], Rect::NOTHING)
                .is_err()
        );
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: FeatureId(2),
                    dimension: Dimension::from_decimal("120").expect("length"),
                },
            ]))
            .expect("change geometry");
        assert!(
            app.current_model_viewer_export(&[scene], viewport())
                .is_err()
        );
    }

    #[test]
    fn pending_metadata_does_not_wait_or_export_previous_program_values() {
        let mut app = KetchupApp::new();
        let source = source();
        app.apply_program_source(source.clone(), true)
            .expect("apply program");
        let paths = app
            .document
            .current()
            .scene_query()
            .into_iter()
            .map(|item| item.instance_path)
            .collect::<Vec<_>>();
        let held = app.program_evaluations.hold(&source);
        let rejection = app
            .viewer_source_data(&paths, &[], viewport())
            .expect_err("pending evaluation");
        assert!(rejection.reason_text().contains("still being evaluated"));
        assert_eq!(app.program_evaluations.on_ui_thread(), 0);
        held.send(crate::program_evaluation::ProgramEvaluations::evaluate_for_test(&source))
            .expect("finish evaluation");
        assert_eq!(
            app.viewer_source_data(&paths, &[], viewport())
                .expect("ready")
                .occurrences
                .len(),
            2
        );
        assert!(
            app.viewer_source_data(
                &[InstancePath::root(OccurrenceId(u64::MAX))],
                &[],
                viewport()
            )
            .is_err()
        );
        assert!(
            app.viewer_source_data(&paths, &[SavedViewId(u64::MAX)], viewport())
                .is_err()
        );
    }
}
