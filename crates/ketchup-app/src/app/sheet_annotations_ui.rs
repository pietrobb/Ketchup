//! Sheet-row editor: geometry choices come from the exact projection, not typed IDs.
use super::drawing_sheet_export::SheetFileFormat;
use crate::*;
use ketchup_model::drawing::annotation::{AnnotationTolerance, DrawingAnnotation};
use ketchup_model::drawing::*;

#[derive(Clone)]
struct Fields {
    open: bool,
    view: usize,
    kind: usize,
    line: usize,
    second: usize,
    circle: usize,
    offset: [f64; 2],
    radius: f64,
    angle: f64,
    tolerance: f64,
    label: String,
    datums: String,
    characteristic: DrawingGeometricCharacteristic,
    material: DrawingMaterialCondition,
    diameter_zone: bool,
    geometry: Option<OrthographicDrawing>,
    error: Option<ketchup_rejection::Rejection>,
}
impl Default for Fields {
    fn default() -> Self {
        Self {
            open: false,
            view: 0,
            kind: 0,
            line: 0,
            second: 1,
            circle: 0,
            offset: [8.0, 8.0],
            radius: 8.0,
            angle: 45.0,
            tolerance: 0.0,
            label: "A".into(),
            datums: String::new(),
            characteristic: DrawingGeometricCharacteristic::Position,
            material: DrawingMaterialCondition::None,
            diameter_zone: false,
            geometry: None,
            error: None,
        }
    }
}
const KINDS: [&str; 5] = [
    "sheet-angular",
    "sheet-radius",
    "sheet-diameter",
    "sheet-datum",
    "sheet-frame",
];

impl KetchupApp {
    pub(crate) fn show_annotatable_sheet_row(
        &mut self,
        ui: &mut egui::Ui,
        sheet: &DrawingSheet,
        export: &mut Option<(DrawingSheetId, SheetFileFormat)>,
    ) {
        let key = egui::Id::new((
            "sheet-annotations",
            self.document.current().document_id(),
            sheet.id(),
        ));
        let mut fields = ui
            .ctx()
            .data_mut(|data| data.get_temp_mut_or_default::<Fields>(key).clone());
        ui.push_id(key, |ui| {
            ui.horizontal(|ui| {
                ui.label(sheet.name());
                for format in SheetFileFormat::ALL {
                    let label = match format {
                        SheetFileFormat::Dxf => "drawings-export-sheet-dxf",
                        SheetFileFormat::Dwg => "drawings-export-sheet-dwg",
                        SheetFileFormat::Svg => "drawings-export-sheet-svg",
                        SheetFileFormat::Pdf => "drawings-export-sheet-pdf",
                    };
                    if ui.button(self.catalog.text(label)).clicked() {
                        *export = Some((sheet.id(), format));
                    }
                }
                if ui.button(self.catalog.text("sheet-annotate")).clicked() {
                    fields.open = !fields.open;
                }
            });
            if fields.open {
                if ui
                    .add_enabled(
                        self.exact.task.is_none(),
                        egui::Button::new(self.catalog.text("sheet-hole-callouts")),
                    )
                    .clicked()
                {
                    match self.annotate_drawing_sheet(sheet.id(), &[], true) {
                        Ok(_) => {
                            fields.geometry = None;
                            fields.error = None;
                        }
                        Err(error) => fields.error = Some(error),
                    }
                }
                let current = self.document.current();
                if fields
                    .geometry
                    .as_ref()
                    .is_none_or(|drawing| !drawing.is_current(&current))
                    && self.exact.task.is_none()
                {
                    match self.drawing_sheet_geometry(sheet.id()) {
                        Ok(drawing) => {
                            fields.geometry = Some(drawing);
                            fields.error = None;
                        }
                        Err(error) => fields.error = Some(error),
                    }
                }
                let geometry = fields.geometry.clone();
                if let Some(drawing) = geometry {
                    ui.add_enabled_ui(self.exact.task.is_none(), |ui| {
                        self.annotation_fields(ui, &drawing, &mut fields);
                        if ui.button(self.catalog.text("sheet-place")).clicked() {
                            if let Some(annotation) = fields.annotation(&drawing) {
                                match self.annotate_drawing_sheet(sheet.id(), &[annotation], false)
                                {
                                    Ok(_) => {
                                        fields.geometry = None;
                                        fields.error = None;
                                    }
                                    Err(error) => fields.error = Some(error),
                                }
                            } else {
                                fields.error = Some(
                                    ketchup_application::rejections::rejected("invalid_params")
                                        .target("annotations")
                                        .reason("Select the required projected edges or circle."),
                                );
                            }
                        }
                    });
                }
                if let Some(error) = &fields.error {
                    ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    for cause in error.causes() {
                        ui.label(cause);
                    }
                }
            }
        });
        ui.ctx().data_mut(|data| data.insert_temp(key, fields));
    }

    fn annotation_fields(
        &self,
        ui: &mut egui::Ui,
        drawing: &OrthographicDrawing,
        fields: &mut Fields,
    ) {
        egui::ComboBox::from_label(self.catalog.text("sheet-view"))
            .selected_text(
                drawing
                    .views
                    .get(fields.view)
                    .map(|view| view.kind.stable_name())
                    .unwrap_or_else(|| self.catalog.text("sheet-view")),
            )
            .show_ui(ui, |ui| {
                for (index, view) in drawing.views.iter().enumerate() {
                    if ui
                        .selectable_value(&mut fields.view, index, view.kind.stable_name())
                        .changed()
                    {
                        fields.line = 0;
                        fields.second = 1;
                        fields.circle = 0;
                    }
                }
            });
        ui.horizontal_wrapped(|ui| {
            for (index, key) in KINDS.iter().enumerate() {
                ui.selectable_value(&mut fields.kind, index, self.catalog.text(key));
            }
        });
        let Some(view) = drawing.views.get(fields.view) else {
            return;
        };
        let lines: Vec<_> = view
            .visible_lines
            .iter()
            .chain(&view.hidden_lines)
            .collect();
        if matches!(fields.kind, 1 | 2) {
            egui::ComboBox::from_label(self.catalog.text("sheet-circle"))
                .selected_text(
                    view.circles
                        .get(fields.circle)
                        .map(|circle| {
                            format!("{} (R{} mm)", circle.stable_circle_id, circle.radius_mm)
                        })
                        .unwrap_or_else(|| self.catalog.text("sheet-circle")),
                )
                .show_ui(ui, |ui| {
                    for (index, circle) in view.circles.iter().enumerate() {
                        ui.selectable_value(
                            &mut fields.circle,
                            index,
                            format!("{} (R{} mm)", circle.stable_circle_id, circle.radius_mm),
                        );
                    }
                });
        } else {
            for (key, selected) in [
                ("sheet-line", &mut fields.line),
                ("sheet-line-two", &mut fields.second),
            ]
            .into_iter()
            .take(if fields.kind == 0 { 2 } else { 1 })
            {
                egui::ComboBox::from_label(self.catalog.text(key))
                    .selected_text(
                        lines
                            .get(*selected)
                            .map(|line| line.stable_line_id.clone())
                            .unwrap_or_else(|| self.catalog.text(key)),
                    )
                    .show_ui(ui, |ui| {
                        for (index, line) in lines.iter().enumerate() {
                            ui.selectable_value(
                                selected,
                                index,
                                format!(
                                    "{} ({:?} – {:?} mm)",
                                    line.stable_line_id, line.start_mm, line.end_mm
                                ),
                            );
                        }
                    });
            }
        }
        ui.label(self.catalog.text("sheet-selected"));
        preview(ui, view, fields);
        for (key, value) in [
            ("sheet-offset-x", &mut fields.offset[0]),
            ("sheet-radius-placeholder", &mut fields.radius),
        ] {
            // The arc radius is independent of the leader offsets.
            let key = if key == "sheet-radius-placeholder" {
                "sheet-arc"
            } else {
                key
            };
            ui.horizontal(|ui| {
                ui.label(self.catalog.text(key));
                ui.add(egui::DragValue::new(value).speed(0.1));
            });
        }
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("sheet-offset-y"));
            ui.add(egui::DragValue::new(&mut fields.offset[1]).speed(0.1));
        });
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("sheet-angle"));
            ui.add(egui::DragValue::new(&mut fields.angle));
        });
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("sheet-tolerance"));
            ui.add(
                egui::DragValue::new(&mut fields.tolerance)
                    .speed(0.01)
                    .range(0.0..=1000.0),
            );
        });
        if fields.kind == 3 {
            ui.horizontal(|ui| {
                ui.label(self.catalog.text("sheet-label"));
                ui.text_edit_singleline(&mut fields.label);
            });
        }
        if fields.kind == 4 {
            self.frame_fields(ui, fields);
        }
        for text in drawing
            .layout
            .angular_dimensions
            .iter()
            .map(|d| &d.label)
            .chain(drawing.layout.circular_dimensions.iter().map(|d| &d.label))
            .chain(drawing.layout.datum_symbols.iter().map(|d| &d.label))
        {
            ui.label(text);
        }
    }

    fn frame_fields(&self, ui: &mut egui::Ui, fields: &mut Fields) {
        use DrawingGeometricCharacteristic::*;
        egui::ComboBox::from_label(self.catalog.text("sheet-characteristic"))
            .selected_text(fields.characteristic.stable_name())
            .show_ui(ui, |ui| {
                for value in [
                    Straightness,
                    Flatness,
                    Circularity,
                    Cylindricity,
                    ProfileOfLine,
                    ProfileOfSurface,
                    Angularity,
                    Perpendicularity,
                    Parallelism,
                    Position,
                    Concentricity,
                    Symmetry,
                    CircularRunout,
                    TotalRunout,
                ] {
                    ui.selectable_value(&mut fields.characteristic, value, value.stable_name());
                }
            });
        egui::ComboBox::from_label(self.catalog.text("sheet-material"))
            .selected_text(fields.material.stable_name())
            .show_ui(ui, |ui| {
                for value in [
                    DrawingMaterialCondition::None,
                    DrawingMaterialCondition::MaximumMaterial,
                    DrawingMaterialCondition::LeastMaterial,
                    DrawingMaterialCondition::RegardlessOfFeatureSize,
                ] {
                    ui.selectable_value(&mut fields.material, value, value.stable_name());
                }
            });
        ui.checkbox(&mut fields.diameter_zone, self.catalog.text("sheet-zone"));
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("sheet-datums"));
            ui.text_edit_singleline(&mut fields.datums);
        });
    }
}

impl Fields {
    fn annotation(&self, drawing: &OrthographicDrawing) -> Option<DrawingAnnotation> {
        let projected = drawing.views.get(self.view)?;
        let view = projected.kind.stable_name();
        let lines: Vec<_> = projected
            .visible_lines
            .iter()
            .chain(&projected.hidden_lines)
            .collect();
        let tolerance = if self.tolerance == 0.0 {
            AnnotationTolerance::None
        } else {
            AnnotationTolerance::Symmetric {
                deviation: self.tolerance,
            }
        };
        Some(match self.kind {
            0 => DrawingAnnotation::Angular {
                view,
                source_line_ids: [
                    lines.get(self.line)?.stable_line_id.clone(),
                    lines.get(self.second)?.stable_line_id.clone(),
                ],
                arc_radius_page_mm: self.radius,
                tolerance,
            },
            1 => DrawingAnnotation::Radius {
                view,
                source_circle_id: projected.circles.get(self.circle)?.stable_circle_id.clone(),
                leader_angle_degrees: self.angle,
                offset_page_mm: self.offset[0],
                tolerance,
            },
            2 => DrawingAnnotation::Diameter {
                view,
                source_circle_id: projected.circles.get(self.circle)?.stable_circle_id.clone(),
                leader_angle_degrees: self.angle,
                offset_page_mm: self.offset[0],
                tolerance,
            },
            3 => DrawingAnnotation::Datum {
                view,
                source_line_id: lines.get(self.line)?.stable_line_id.clone(),
                label: self.label.clone(),
                offset_page_mm: self.offset,
            },
            _ => DrawingAnnotation::FeatureControlFrame {
                view,
                source_line_id: lines.get(self.line)?.stable_line_id.clone(),
                characteristic: self.characteristic,
                tolerance_mm: self.tolerance,
                diameter_zone: self.diameter_zone,
                material_condition: self.material,
                datum_labels: self
                    .datums
                    .split(',')
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .map(str::to_owned)
                    .collect(),
                offset_page_mm: self.offset,
            },
        })
    }
}

fn preview(ui: &mut egui::Ui, view: &OrthographicView, fields: &Fields) {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width().min(500.0), 180.0),
        egui::Sense::hover(),
    );
    let rect = response.rect.shrink(10.0);
    let [[xmin, ymin], [xmax, ymax]] = view.bounds_mm;
    let scale = (f64::from(rect.width()) / (xmax - xmin).max(1.0))
        .min(f64::from(rect.height()) / (ymax - ymin).max(1.0));
    let point = |p: [f64; 2]| {
        let [x, y, _] =
            crate::renderer::single_precision([(p[0] - xmin) * scale, (p[1] - ymin) * scale, 0.0]);
        egui::pos2(rect.left() + x, rect.bottom() - y)
    };
    let normal = ui.visuals().text_color();
    let selected = ui.visuals().selection.stroke.color;
    for (index, line) in view
        .visible_lines
        .iter()
        .chain(&view.hidden_lines)
        .enumerate()
    {
        let active = !matches!(fields.kind, 1 | 2)
            && (index == fields.line || fields.kind == 0 && index == fields.second);
        painter.line_segment(
            [point(line.start_mm), point(line.end_mm)],
            egui::Stroke::new(
                if active { 2.5_f32 } else { 1.0_f32 },
                if active { selected } else { normal },
            ),
        );
    }
    for (index, circle) in view.circles.iter().enumerate() {
        let active = matches!(fields.kind, 1 | 2) && index == fields.circle;
        let points = (0..=64)
            .map(|step| {
                let angle = circle.start_rad + circle.span_rad * f64::from(step) / 64.0;
                point([
                    circle.center_mm[0] + circle.radius_mm * angle.cos(),
                    circle.center_mm[1] + circle.radius_mm * angle.sin(),
                ])
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(
                if active { 2.5_f32 } else { 1.0_f32 },
                if active { selected } else { normal },
            ),
        ));
    }
}
