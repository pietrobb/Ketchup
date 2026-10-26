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
                let distance = distance_text(self.language, &done);
                ui.label(
                    egui::RichText::new(
                        self.language
                            .format("viewer-measure-distance", &[("distance", &distance)]),
                    )
                    .strong()
                    .size(18.0),
                );
                for (axis, value) in ["ΔX", "ΔY", "ΔZ"].into_iter().zip(done.delta()) {
                    ui.label(format!("{axis}: {} mm", millimetres(self.language, value)));
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
        // A phone has no Esc key to name (review 2026-10-24 DOC-21).
        let touch = ui.input(egui::InputState::has_touch_screen);
        ui.small(self.t(if touch {
            "viewer-measure-cancel-touch"
        } else {
            "viewer-measure-cancel"
        }));
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
    let line = shown.and_then(|measured| {
        Some((
            screen(measured.a.point)?,
            screen(measured.b.point)?,
            measured,
        ))
    });
    if let Some((a, b, _)) = line {
        painter.line_segment([a, b], egui::Stroke::new(5.0_f32, egui::Color32::BLACK));
        painter.line_segment([a, b], egui::Stroke::new(2.5_f32, COLOUR));
    }
    // What the distance must not cover: the points, their names and the snap mark.
    let mut taken = Vec::new();
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
            taken.push(mark(at));
            taken.push(label(
                painter,
                at + egui::vec2(12.0, -12.0),
                name.to_owned(),
            ));
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
        taken.push(mark(at));
        taken.push(label(painter, at + egui::vec2(14.0, 14.0), kind.to_owned()));
    }
    if let Some((a, b, measured)) = line {
        let galley = layout(painter, distance_text(language, &measured));
        let plate = distance_plate(a, b, galley.size() + egui::vec2(6.0, 6.0), &taken);
        paint_plate(painter, plate, galley);
    }
}

/// The text over the line: only the distance; ΔX, ΔY and ΔZ are listed beside the model.
fn distance_text(language: Language, measured: &Measured) -> String {
    format!(
        "{}{} mm",
        about(measured),
        millimetres(language, measured.distance())
    )
}

/// The space a point or a snap mark takes on the screen.
fn mark(at: egui::Pos2) -> egui::Rect {
    egui::Rect::from_center_size(at, egui::vec2(16.0, 16.0))
}

/// Points between the line and the distance beside it.
const GAP: f32 = 6.0;

/// Where the distance goes: a few points off the middle of the line, on the
/// side facing up the screen, else the other side, moving further out until it
/// covers nothing in `taken`; so beside a line shorter than the text it ends up
/// clear of both points. It never lies on the line itself.
fn distance_plate(
    a: egui::Pos2,
    b: egui::Pos2,
    size: egui::Vec2,
    taken: &[egui::Rect],
) -> egui::Rect {
    let along = b - a;
    let normal = if along.length() > 0.5 {
        let normal = egui::vec2(-along.y, along.x) / along.length();
        if normal.y > 0.0 { -normal } else { normal }
    } else {
        egui::vec2(0.0, -1.0)
    };
    // Half the plate measured across the line.
    let across = 0.5 * (normal.x.abs() * size.x + normal.y.abs() * size.y);
    let middle = a.lerp(b, 0.5);
    let beside = |side: egui::Vec2, out: f32| {
        egui::Rect::from_center_size(middle + side * (GAP + across + out), size)
    };
    (0..60_u8)
        .map(|step| f32::from(step) * GAP)
        .flat_map(|out| [beside(normal, out), beside(-normal, out)])
        .find(|plate| !taken.iter().any(|t| t.intersects(*plate)))
        .unwrap_or_else(|| beside(normal, 0.0))
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

/// White text on a dark plate, centred on `at`; returns the plate.
fn label(painter: &egui::Painter, at: egui::Pos2, text: String) -> egui::Rect {
    let galley = layout(painter, text);
    let plate = egui::Rect::from_center_size(at, galley.size()).expand(3.0);
    paint_plate(painter, plate, galley);
    plate
}

fn layout(painter: &egui::Painter, text: String) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(text, egui::FontId::proportional(14.0), egui::Color32::WHITE)
}

fn paint_plate(painter: &egui::Painter, plate: egui::Rect, galley: std::sync::Arc<egui::Galley>) {
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

    fn snap(point: [f64; 3], approximate: bool) -> Snap {
        Snap {
            point,
            kind: SnapKind::Vertex,
            approximate,
        }
    }

    #[test]
    fn the_text_over_the_line_is_only_the_distance() {
        let exact = Measured {
            a: snap([0.0, 0.0, 0.0], false),
            b: snap([100.0, 100.0, 50.0], false),
        };
        assert_eq!(distance_text(Language::ENGLISH, &exact), "150 mm");
        let curved = Measured {
            a: snap([0.0, 0.0, 0.0], true),
            ..exact
        };
        let text = distance_text(Language::ENGLISH, &curved);
        assert_eq!(text, "≈ 150 mm");
        assert!(!text.contains('Δ') && !text.contains('\n'), "{text}");
    }

    /// The plate is off the segment `a`–`b` by at least `GAP`.
    fn clear_of_the_line(plate: egui::Rect, a: egui::Pos2, b: egui::Pos2) -> bool {
        (0..=100_u8).all(|i| {
            let on = a.lerp(b, f32::from(i) / 100.0);
            plate.distance_to_pos(on) >= GAP - 0.01
        })
    }

    #[test]
    fn the_distance_sits_beside_a_long_line_a_few_points_off_it() {
        let size = egui::vec2(80.0, 20.0);
        for (a, b) in [
            (egui::pos2(100.0, 300.0), egui::pos2(500.0, 300.0)),
            (egui::pos2(300.0, 100.0), egui::pos2(300.0, 500.0)),
            (egui::pos2(100.0, 500.0), egui::pos2(500.0, 100.0)),
        ] {
            let taken = [mark(a), mark(b)];
            let plate = distance_plate(a, b, size, &taken);
            assert!(clear_of_the_line(plate, a, b), "{plate:?} on {a:?}-{b:?}");
            assert!(!taken.iter().any(|t| t.intersects(plate)));
            let middle = a.lerp(b, 0.5);
            assert!(
                (plate.center() - middle).length() <= GAP + 0.5 * size.length(),
                "{plate:?} is right beside the middle {middle:?}"
            );
        }
        // Above a level line, not below it.
        let plate = distance_plate(
            egui::pos2(100.0, 300.0),
            egui::pos2(500.0, 300.0),
            size,
            &[],
        );
        assert_eq!(plate.bottom(), 300.0 - GAP);
    }

    #[test]
    fn beside_a_line_shorter_than_the_text_the_distance_covers_neither_point() {
        let size = egui::vec2(80.0, 20.0);
        for (a, b) in [
            (egui::pos2(300.0, 300.0), egui::pos2(320.0, 300.0)),
            (egui::pos2(300.0, 300.0), egui::pos2(305.0, 290.0)),
            (egui::pos2(300.0, 300.0), egui::pos2(300.0, 300.0)),
        ] {
            // The points, their names above right and the snap name below right.
            let taken = [
                mark(a),
                mark(b),
                egui::Rect::from_center_size(a + egui::vec2(12.0, -12.0), egui::vec2(16.0, 22.0)),
                egui::Rect::from_center_size(b + egui::vec2(12.0, -12.0), egui::vec2(16.0, 22.0)),
                egui::Rect::from_center_size(b + egui::vec2(14.0, 14.0), egui::vec2(50.0, 22.0)),
            ];
            let plate = distance_plate(a, b, size, &taken);
            assert!(!taken.iter().any(|t| t.intersects(plate)), "{plate:?}");
            assert!(!plate.contains(a) && !plate.contains(b));
            assert!(clear_of_the_line(plate, a, b), "{plate:?} on {a:?}-{b:?}");
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
        // Review 2026-10-24 DOC-21: a decimal comma in Slovak.
        harness.get_by_label("Vzdialenosť: 141,42 mm");
        harness.get_by_label("ΔX: 100 mm");
        harness.get_by_label("ΔZ: 0 mm");
        harness.get_by_label("Esc alebo tlačidlo Zmerať meranie ukončí.");
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

        // DOC-21: on a touch screen (a phone) there is no Esc to name.
        let at = on_screen(&harness, [50.0, 50.0, 50.0]);
        for phase in [egui::TouchPhase::Start, egui::TouchPhase::End] {
            harness.input_mut().events.push(egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(1),
                phase,
                pos: at,
                force: None,
            });
            harness.run();
        }
        harness.get_by_label("Zmerať").click();
        harness.run();
        harness.get_by_label("Tlačidlo Zmerať meranie ukončí.");
        assert!(harness.query_by_label_contains("Esc").is_none());
    }

    #[test]
    fn millimetres_have_no_negative_zero_and_the_language_s_decimal_mark() {
        let slovak = Language::from_tag("sk").expect("sk");
        assert_eq!(millimetres(Language::ENGLISH, -0.001), "0");
        assert_eq!(millimetres(slovak, -0.004), "0");
        assert_eq!(millimetres(Language::ENGLISH, -18.5), "-18.5");
        assert_eq!(millimetres(slovak, 18.25), "18,25");
        assert_eq!(millimetres(slovak, 600.0), "600");
    }
}
