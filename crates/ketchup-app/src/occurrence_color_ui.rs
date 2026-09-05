use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrence_color_copy_and_patterns_preserve_overrides_in_one_step() {
        for operation in [
            "move-copy",
            "rotate-copy",
            "linear",
            "rectangular",
            "circular",
        ] {
            let mut geometry = None;
            for color in [None, Some([12, 70, 220])] {
                let mut app = KetchupApp::new();
                assert!(app.select_untagged_occurrences());
                if color.is_some() {
                    assert!(app.set_selected_occurrence_color(color));
                }
                let snapshot = app.document.current();
                let source = snapshot.occurrences().next().unwrap().id();
                let before = snapshot.canonical_digest();
                let features = format!("{:?}", snapshot.features().collect::<Vec<_>>());
                let undo = app.undo_step_count();
                match operation {
                    "move-copy" => assert!(app.copy_selected(Vec3::new(100.0, 0.0, 0.0))),
                    "rotate-copy" => {
                        let selection = app.selected_move_reference().unwrap();
                        assert!(app.rotate_occurrence(&selection, Vec3::ZERO, Axis::Z, 90.0, true));
                    }
                    "linear" => {
                        assert!(app.preview_linear_pattern(source, Axis::X, 100.0, 3));
                        assert_eq!(
                            app.occurrence_operation_preview
                                .as_ref()
                                .unwrap()
                                .boxes
                                .len(),
                            2
                        );
                        assert!(app.confirm_occurrence_operation_preview());
                    }
                    "rectangular" => {
                        assert!(app.preview_rectangular_pattern(
                            source,
                            RectangularPatternSpec {
                                primary_axis: Axis::X,
                                primary_spacing_mm: 100.0,
                                primary_count: 2,
                                secondary_axis: Axis::Y,
                                secondary_spacing_mm: 150.0,
                                secondary_count: 2,
                            }
                        ));
                        assert_eq!(
                            app.occurrence_operation_preview
                                .as_ref()
                                .unwrap()
                                .boxes
                                .len(),
                            3
                        );
                        assert!(app.confirm_occurrence_operation_preview());
                    }
                    "circular" => {
                        assert!(app.preview_circular_pattern(source, Axis::Z, Vec3::ZERO, 90.0, 3));
                        assert_eq!(
                            app.occurrence_operation_preview
                                .as_ref()
                                .unwrap()
                                .boxes
                                .len(),
                            2
                        );
                        assert!(app.confirm_occurrence_operation_preview());
                    }
                    _ => unreachable!(),
                }
                assert_eq!(app.undo_step_count(), undo + 1, "{operation}");
                let after = app.document.current();
                assert!(after.occurrences().count() > snapshot.occurrences().count());
                assert!(
                    after
                        .occurrences()
                        .all(|occurrence| occurrence.color() == color),
                    "{operation}"
                );
                assert_eq!(
                    format!("{:?}", after.features().collect::<Vec<_>>()),
                    features
                );
                let transforms = after
                    .occurrences()
                    .map(|occurrence| {
                        (
                            occurrence.id(),
                            occurrence.definition_id(),
                            occurrence.transform(),
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(expected) = &geometry {
                    assert_eq!(&transforms, expected, "{operation}");
                }
                geometry = Some(transforms);
                let digest = after.canonical_digest();
                app.document.undo().unwrap();
                assert_eq!(app.canonical_digest(), before);
                app.document.redo().unwrap();
                assert_eq!(app.canonical_digest(), digest);
            }
        }
    }

    #[test]
    fn occurrence_color_nested_cpu_conversion_and_reset_reveal_child_color() {
        let mut app = KetchupApp::new();
        assert!(app.select_untagged_occurrences());
        assert!(app.set_selected_occurrence_color(Some([12, 70, 220])));
        let mut root = app.document.current().occurrences().next().unwrap().id();
        for level in 1..=1 {
            let group = GroupId(900 + level);
            app.document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateGroup {
                        id: group,
                        name: "Nested color".into(),
                        transform: Transform::identity(),
                        parent: None,
                    },
                    CanonicalCommand::SetOccurrenceParent {
                        id: root,
                        parent: Some(group),
                    },
                ]))
                .unwrap();
            root = app
                .document
                .convert_group_to_component(group, "Colored component")
                .unwrap()
                .component_occurrence_id;
        }
        app.selection.clear();
        // Nested objects outside the active edit context intentionally use a neutral fill.
        assert!(app.enter_occurrence_context(InstancePath::root(root)));
        let context = egui::Context::default();
        for override_color in [None, Some([220, 40, 20]), None] {
            app.document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetOccurrenceColor {
                        id: root,
                        color: override_color,
                    },
                ]))
                .unwrap();
            let expected_rgb = override_color.unwrap_or([12, 70, 220]);
            let snapshot = app.document.current();
            let scene = snapshot.scene_query();
            assert!(
                scene
                    .iter()
                    .any(|occurrence| !occurrence.instance_path.is_root()
                        && occurrence.color() == Some(expected_rgb))
            );
            let revision = app.document_revision();
            for xray in [false, true] {
                app.xray_visible = xray;
                let expected = Color32::from_rgba_unmultiplied(
                    expected_rgb[0],
                    expected_rgb[1],
                    expected_rgb[2],
                    if xray { 72 } else { 255 },
                );
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1600.0, 1000.0),
                        )),
                        ..Default::default()
                    },
                    |context| app.ui(context),
                );
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.fill == expected)), "nested CPU color {override_color:?}, Xray {xray}");
                assert_eq!(app.document_revision(), revision);
            }
        }
        // A copy of a default component must retain local colors, not bake them into its root.
        app.selection.edit_context.clear();
        app.selection.select_occurrence(root, false);
        assert!(app.duplicate_selection());
        let snapshot = app.document.current();
        assert!(
            snapshot
                .occurrences()
                .all(|occurrence| occurrence.color().is_none())
        );
        let roots = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id())
            .collect::<BTreeSet<_>>();
        assert_eq!(roots.len(), 2);
        let scene = snapshot.scene_query();
        let children = scene
            .iter()
            .filter(|occurrence| !occurrence.instance_path.is_root())
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert_eq!(
            children
                .iter()
                .map(|occurrence| occurrence.instance_path.root_occurrence())
                .collect::<BTreeSet<_>>(),
            roots
        );
        assert!(
            children
                .iter()
                .all(|occurrence| occurrence.color() == Some([12, 70, 220]))
        );
    }

    #[test]
    fn occurrence_color_cpu_viewport_preserves_srgb_and_single_xray_fill() {
        let mut app = KetchupApp::new();
        assert!(app.select_untagged_occurrences());
        assert!(app.set_selected_occurrence_color(Some([12, 70, 220])));
        app.selection.clear();
        let revision = app.document_revision();
        let context = egui::Context::default();
        for xray in [false, true] {
            if xray {
                app.toggle_xray();
            }
            let expected =
                Color32::from_rgba_unmultiplied(12, 70, 220, if xray { 72 } else { 255 });
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                    ..Default::default()
                },
                |context| app.ui(context),
            );
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Path(path) if path.fill == expected
                )),
                "CPU viewport must paint the persisted sRGB color"
            );
            if xray {
                assert!(!output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|vertex| vertex.color == expected)
                )), "Xray must not also emit a matching mesh underlay");
            }
            assert_eq!(app.document_revision(), revision);
        }
    }
}

#[derive(Clone)]
struct ColorDraft {
    revision: u64,
    selection: BTreeSet<OccurrenceId>,
    rgb: [u8; 3],
}

impl KetchupApp {
    /// Set or reset all selected root objects in one canonical undo step.
    pub fn set_selected_occurrence_color(&mut self, color: Option<[u8; 3]>) -> bool {
        let snapshot = self.document.current();
        let commands = self
            .selected_occurrence_ids()
            .into_iter()
            .filter_map(|id| {
                let occurrence = snapshot.occurrence(id)?;
                (occurrence.color() != color)
                    .then_some(CanonicalCommand::SetOccurrenceColor { id, color })
            })
            .collect::<Vec<_>>();
        if commands.is_empty() {
            return false;
        }
        self.document
            .apply_batch(&CommandBatch::new(commands))
            .is_ok()
    }

    pub(super) fn show_occurrence_color_editor(&mut self, ui: &mut egui::Ui) {
        let selection = self.selected_occurrence_ids();
        if selection.is_empty() {
            return;
        }
        let snapshot = self.document.current();
        let colors = selection
            .iter()
            .filter_map(|id| snapshot.occurrence(*id))
            .map(|occurrence| occurrence.color())
            .collect::<Vec<_>>();
        let first = colors.first().copied().flatten();
        let mixed = colors.windows(2).any(|pair| pair[0] != pair[1]);
        let id = ui.id().with("occurrence-color-draft");
        let mut draft = ui
            .data_mut(|data| data.get_temp::<ColorDraft>(id))
            .filter(|draft| {
                draft.revision == snapshot.revision_id() && draft.selection == selection
            })
            .unwrap_or(ColorDraft {
                revision: snapshot.revision_id(),
                selection,
                rgb: first.unwrap_or([140, 160, 180]),
            });
        egui::CollapsingHeader::new(self.catalog.text("occurrence-color-title"))
            .id_salt("occurrence-color-editor")
            .default_open(true)
            .show(ui, |ui| {
                if mixed {
                    ui.label(self.catalog.text("occurrence-color-mixed"));
                }
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(&mut draft.rgb);
                    for (channel, label) in draft.rgb.iter_mut().zip(["R", "G", "B"]) {
                        let label = ui.label(label);
                        ui.add(egui::DragValue::new(channel).range(0..=255).speed(1))
                            .labelled_by(label.id);
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .button(self.catalog.text("occurrence-color-apply"))
                        .clicked()
                    {
                        self.set_selected_occurrence_color(Some(draft.rgb));
                    }
                    if ui
                        .add_enabled(
                            colors.iter().any(Option::is_some),
                            egui::Button::new(self.catalog.text("occurrence-color-reset")),
                        )
                        .clicked()
                    {
                        self.set_selected_occurrence_color(None);
                    }
                });
            });
        ui.data_mut(|data| data.insert_temp(id, draft));
    }
}
