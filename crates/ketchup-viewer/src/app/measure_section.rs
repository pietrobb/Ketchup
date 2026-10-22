//! The Measure tool in the window: its toolbar switch, the result beside the
//! model, and the points and line drawn over the 3D view.

use super::{ViewerApp, millimetres};
use crate::{
    camera::Camera,
    measure::{Measure, Measured, Snap, SnapKind},
    text::Language,
};
use eframe::egui;
use num_traits::ToPrimitive;

/// Drawn over the model, so it stays visible behind faces.
const COLOUR: egui::Color32 = egui::Color32::from_rgb(80, 230, 120);

impl ViewerApp {
    /// The toolbar switch: on starts measuring, off drops the measurement.
    pub(super) fn measure_switch(&mut self, ui: &mut egui::Ui) {
        if self.model.is_some()
            && ui
                .selectable_label(self.measure.is_some(), self.t("viewer-measure"))
                .clicked()
        {
            self.measure = self.measure.take().is_none().then(Measure::default);
        }
    }

    /// What to click next, the result and why it may be approximate.
    pub(super) fn measure_info(&self, ui: &mut egui::Ui) {
        let Some(measure) = &self.measure else {
            return;
        };
        ui.heading(self.t("viewer-measure-heading"));
        match (measure.first, measure.done) {
            (None, Some(done)) => {
                let distance = format!("{}{} mm", about(&done), millimetres(done.distance()));
                ui.label(
                    egui::RichText::new(
                        self.language
                            .format("viewer-measure-distance", &[("distance", &distance)]),
                    )
                    .strong()
                    .size(18.0),
                );
                for (axis, value) in ["ΔX", "ΔY", "ΔZ"].into_iter().zip(done.delta()) {
                    ui.label(format!("{axis}: {} mm", millimetres(value)));
                }
                if let Some(note) = approximate_note(self.language, &done) {
                    ui.label(egui::RichText::new(note).color(egui::Color32::LIGHT_YELLOW));
                }
                ui.small(self.t("viewer-measure-again"));
            }
            (Some(_), _) => {
                ui.label(self.t("viewer-measure-second"));
            }
            (None, None) => {
                ui.label(self.t("viewer-measure-first"));
            }
        }
        ui.small(self.t("viewer-measure-cancel"));
        ui.separator();
    }
}

/// `≈ ` before an approximate value.
fn about(measured: &Measured) -> &'static str {
    if measured.approximate() { "≈ " } else { "" }
}

/// Why the result is approximate, naming the point or points on a curved face.
fn approximate_note(language: Language, measured: &Measured) -> Option<String> {
    match (measured.a.approximate, measured.b.approximate) {
        (true, true) => Some(language.text("viewer-measure-approximate-both").to_owned()),
        (true, false) => Some(language.format("viewer-measure-approximate-one", &[("point", "A")])),
        (false, true) => Some(language.format("viewer-measure-approximate-one", &[("point", "B")])),
        (false, false) => None,
    }
}

/// The measurement over the 3D view: A, B and the line between them with its
/// value; while B is awaited, the line runs to where the pointer would snap,
/// and that point says what it snaps to.
pub(super) fn paint_measure(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: &Camera,
    measure: &Measure,
    language: Language,
) {
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    let screen = |point: [f64; 3]| {
        let p = camera.project(point.map(|v| v.to_f32().unwrap_or(f32::MAX)), aspect);
        ((0.0..=1.0).contains(&p[2]) && p[0].is_finite() && p[1].is_finite()).then(|| {
            egui::pos2(
                rect.left() + (p[0] + 1.0) * 0.5 * rect.width(),
                rect.top() + (1.0 - p[1]) * 0.5 * rect.height(),
            )
        })
    };
    let shown = measure.done.or_else(|| {
        measure
            .first
            .zip(measure.hover)
            .map(|(a, b)| Measured { a, b })
    });
    if let Some(measured) = shown
        && let (Some(a), Some(b)) = (screen(measured.a.point), screen(measured.b.point))
    {
        painter.line_segment([a, b], egui::Stroke::new(5.0_f32, egui::Color32::BLACK));
        painter.line_segment([a, b], egui::Stroke::new(2.5_f32, COLOUR));
        let [x, y, z] = measured.delta().map(millimetres);
        let text = format!(
            "{}{} mm\nΔX {x} · ΔY {y} · ΔZ {z}",
            about(&measured),
            millimetres(measured.distance())
        );
        label(painter, a.lerp(b, 0.5), text);
    }
    let placed = measure
        .done
        .map(|done| [Some(done.a), Some(done.b)])
        .unwrap_or([measure.first, None]);
    for (name, snap) in ["A", "B"].into_iter().zip(placed) {
        if let Some(at) = snap.and_then(|snap| screen(snap.point)) {
            painter.circle(
                at,
                5.0,
                COLOUR,
                egui::Stroke::new(1.5_f32, egui::Color32::BLACK),
            );
            label(painter, at + egui::vec2(12.0, -12.0), name.to_owned());
        }
    }
    if let Some(hover) = measure.hover
        && let Some(at) = screen(hover.point)
    {
        paint_snap(painter, at, hover);
        let kind = language.text(match hover.kind {
            SnapKind::Vertex => "viewer-snap-vertex",
            SnapKind::Edge => "viewer-snap-edge",
            SnapKind::Face => "viewer-snap-face",
        });
        label(painter, at + egui::vec2(14.0, 14.0), kind.to_owned());
    }
}

/// A square on a vertex, a diamond on an edge, a ring on a face.
fn paint_snap(painter: &egui::Painter, at: egui::Pos2, snap: Snap) {
    let stroke = egui::Stroke::new(2.0_f32, COLOUR);
    match snap.kind {
        SnapKind::Vertex => {
            painter.rect_stroke(
                egui::Rect::from_center_size(at, egui::vec2(12.0, 12.0)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        SnapKind::Edge => {
            let corners = [(0.0, -7.0), (7.0, 0.0), (0.0, 7.0), (-7.0, 0.0)]
                .map(|(x, y)| at + egui::vec2(x, y))
                .to_vec();
            painter.add(egui::Shape::closed_line(corners, stroke));
        }
        SnapKind::Face => {
            painter.circle_stroke(at, 6.0, stroke);
        }
    }
}

/// White text on a dark plate, centred on `at`.
fn label(painter: &egui::Painter, at: egui::Pos2, text: String) {
    let galley =
        painter.layout_no_wrap(text, egui::FontId::proportional(14.0), egui::Color32::WHITE);
    let plate = egui::Rect::from_center_size(at, galley.size()).expand(3.0);
    painter.rect_filled(plate, 3.0, egui::Color32::from_black_alpha(200));
    painter.galley(
        plate.min + egui::vec2(3.0, 3.0),
        galley,
        egui::Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure_tests::{block, half_cylinder, package, scene};
    use egui_kittest::kittest::Queryable as _;

    type Harness = egui_kittest::Harness<'static, ViewerApp>;

    /// Where world `point` shows in the 3D view.
    fn on_screen(harness: &Harness, point: [f32; 3]) -> egui::Pos2 {
        let rect = harness.get_by_label("Pohľad na model").rect();
        let camera = harness.state().camera.as_ref().expect("camera");
        let p = camera.project(point, rect.width() / rect.height());
        egui::pos2(
            rect.left() + (p[0] + 1.0) * 0.5 * rect.width(),
            rect.top() + (1.0 - p[1]) * 0.5 * rect.height(),
        )
    }

    fn click(harness: &mut Harness, point: [f32; 3]) {
        let at = on_screen(harness, point);
        for pressed in [true, false] {
            harness.input_mut().events.extend([
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            harness.run();
        }
    }

    #[test]
    fn measure_snaps_two_points_shows_the_result_and_esc_or_the_button_ends_it() {
        let mut bytes = Vec::new();
        package(&[block(), half_cylinder()])
            .write(&mut bytes)
            .expect("fixture");
        let mut app = ViewerApp::new(None).with_language(Language::from_tag("sk").expect("sk"));
        app.open(bytes.as_slice()).expect("open");
        app.camera = Some(scene().1);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
        harness.run();
        harness.get_by_label("Zmerať").click();
        harness.run();
        harness.get_by_label("Kliknite alebo ťuknite na prvý bod (A).");

        // Two quick clicks are two points, and pick no component.
        click(&mut harness, [0.0, 0.0, 50.0]);
        assert_eq!(
            harness
                .state()
                .measure
                .as_ref()
                .and_then(|m| m.hover)
                .map(|s| s.kind),
            Some(SnapKind::Vertex),
            "the pointer shows it is on a vertex"
        );
        harness.get_by_label("Kliknite alebo ťuknite na druhý bod (B).");
        let camera = harness.state().camera.clone().expect("camera");
        click(&mut harness, [100.0, 100.0, 50.0]);
        harness.get_by_label("Vzdialenosť: 141.42 mm");
        harness.get_by_label("ΔX: 100 mm");
        harness.get_by_label("ΔZ: 0 mm");
        assert!(harness.query_by_label_contains("Približne").is_none());
        assert_eq!(harness.state().selected, None);
        let after = harness.state().camera.as_ref().expect("camera");
        assert_eq!(
            (after.center, after.scale),
            (camera.center, camera.scale),
            "no double-click fit"
        );

        // A new measurement replaces the finished one; a rim vertex is approximate.
        click(&mut harness, [350.0, 0.0, 50.0]);
        assert!(harness.query_by_label_contains("Vzdialenosť").is_none());
        click(&mut harness, [0.0, 0.0, 50.0]);
        harness.get_by_label("Vzdialenosť: ≈ 350 mm");
        harness.get_by_label_contains("≈ Približne: bod A leží na zakrivenej ploche.");

        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        assert!(harness.state().measure.is_none(), "Esc ends measuring");
        assert!(harness.query_by_label("Meranie").is_none());
        harness.get_by_label("Zmerať").click();
        harness.run();
        assert!(harness.state().measure.is_some());
        harness.get_by_label("Zmerať").click();
        harness.run();
        assert!(harness.state().measure.is_none(), "the button ends it too");
        click(&mut harness, [50.0, 50.0, 50.0]);
        assert_eq!(harness.state().selected, Some(0), "clicks pick parts again");
    }
}
