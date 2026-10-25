//! The drawing annotations of the document in the viewport: texts, dimensions with
//! the value the document measured, leaders and drawn lines, laid out by the same
//! marks the plan sheet draws. Their points follow the model; their text and the
//! marks of a dimension keep one size on screen at every zoom. A click on one picks
//! it; it stays no part, so the part selection is empty while it is picked. A click on
//! the picked one picks what lies under it.

use crate::renderer::single_precision;
use crate::*;
use ketchup_manufacturing::project_drawings::annotation_marks;
use ketchup_manufacturing::sheet_pdf::{Anchor, Mark, SheetFont};
use ketchup_model::annotation::Annotation;

/// Screen points for one millimetre of the page the marks are laid out on: text
/// 2.5 mm tall on paper has capitals 10 points tall on screen.
const POINTS_PER_PAPER_MM: f64 = 4.0;
/// How near the pointer must come to a line of an annotation, or to the box of
/// its text, to pick it, in screen points.
const PICK_REACH_POINTS: f32 = 4.0;

/// One annotation as the viewport draws it this frame, in screen points.
pub(crate) struct DrawnAnnotation {
    pub(crate) name: String,
    pub(crate) lines: Vec<(Vec<Pos2>, bool)>,
    pub(crate) fills: Vec<Vec<Pos2>>,
    pub(crate) texts: Vec<DrawnText>,
}

pub(crate) struct DrawnText {
    /// The top left corner of the laid out text, which it is turned about.
    pub(crate) pos: Pos2,
    /// Clockwise, in radians, as egui turns text.
    pub(crate) angle: f32,
    pub(crate) galley: Arc<egui::Galley>,
}

impl DrawnText {
    fn contains(&self, pointer: Pos2) -> bool {
        let local = egui::emath::Rot2::from_angle(-self.angle) * (pointer - self.pos);
        self.galley
            .rect
            .expand(PICK_REACH_POINTS)
            .contains(local.to_pos2())
    }
}

impl DrawnAnnotation {
    fn reaches(&self, pointer: Pos2) -> bool {
        self.texts.iter().any(|text| text.contains(pointer))
            || self.lines.iter().any(|(points, closed)| {
                let count = points.len();
                let segments = if *closed {
                    count
                } else {
                    count.saturating_sub(1)
                };
                (0..segments).any(|index| {
                    let segment = [points[index], points[(index + 1) % count]];
                    distance_to_segment(pointer, segment) <= PICK_REACH_POINTS
                })
            })
    }
}

fn distance_to_segment(point: Pos2, [start, end]: [Pos2; 2]) -> f32 {
    let along = end - start;
    let length = along.length_sq();
    let share = if length > 0.0 {
        ((point - start).dot(along) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(start + along * share)
}

impl KetchupApp {
    /// The picked annotation's name while the part selection is the one it was
    /// picked with and the annotation is still shown.
    pub(crate) fn picked_annotation_name(&self) -> Option<&str> {
        let (name, with) = self.picked_annotation.as_ref()?;
        if *with != self.selected_instance_paths() || self.selection.selected_group.is_some() {
            return None;
        }
        let snapshot = self.document.current();
        snapshot
            .annotations()
            .iter()
            .any(|annotation| annotation.name == *name && snapshot.tags_visible(&annotation.tags))
            .then_some(name.as_str())
    }

    /// The picked annotation for a client: its name, kind, the text it writes and
    /// the names of its tags.
    pub(crate) fn picked_annotation_summary(&self) -> Option<serde_json::Value> {
        let name = self.picked_annotation_name()?;
        let snapshot = self.document.current();
        let annotation = snapshot
            .annotations()
            .iter()
            .find(|annotation| annotation.name == name)?;
        let tags: Vec<_> = annotation
            .tags
            .iter()
            .filter_map(|tag| snapshot.tag(*tag).map(|tag| tag.name().to_owned()))
            .collect();
        Some(serde_json::json!({
            "name": annotation.name,
            "kind": annotation.kind(),
            "text": annotation.text(),
            "tags": tags,
        }))
    }

    fn pick_annotation(&mut self, annotation: &Annotation) {
        self.clear_selection();
        self.picked_space = None;
        self.picked_dimension = None;
        self.picked_joint = None;
        self.picked_annotation = Some((annotation.name.clone(), BTreeSet::new()));
        self.digest = match annotation.text() {
            Some(text) => self.catalog.format(
                "digest-annotation-picked",
                &BTreeMap::from([("name", annotation.name.clone()), ("text", text)]),
            ),
            None => self.catalog.format(
                "digest-annotation-line-picked",
                &BTreeMap::from([("name", annotation.name.clone())]),
            ),
        };
    }

    /// Picks the shown annotation under `pointer`, the last drawn first, as it is
    /// drawn on top; a click beside every annotation lets go of the picked one.
    /// A click on the picked one goes through to what lies under it: the next annotation
    /// there, then the part, so a wall under labels that keep their size on screen as the
    /// plan is zoomed out is still picked (review 2026-10-24 UI-26). Whether one was picked.
    pub(crate) fn pick_annotation_at(
        &mut self,
        context: &egui::Context,
        pointer: Pos2,
        rect: Rect,
    ) -> bool {
        let under: Vec<String> = self
            .drawn_annotations(context, rect)
            .into_iter()
            .rev()
            .filter(|drawn| drawn.reaches(pointer))
            .map(|drawn| drawn.name)
            .collect();
        let picked = self
            .picked_annotation_name()
            .and_then(|picked| under.iter().position(|name| name == picked));
        let Some(name) = under.get(picked.map_or(0, |at| at + 1)).cloned() else {
            self.picked_annotation = None;
            return false;
        };
        let snapshot = self.document.current();
        let Some(annotation) = snapshot
            .annotations()
            .iter()
            .find(|annotation| annotation.name == name)
        else {
            return false;
        };
        self.pick_annotation(annotation);
        true
    }

    /// Where a world point is on the page the marks are laid out on, or `None`
    /// at or behind a converging eye.
    fn annotation_page_point(&self, point: [f64; 3], rect: Rect) -> Option<[f64; 2]> {
        let point = Vec3::new(point[0], point[1], point[2]);
        if self.camera.projection_mode == ProjectionMode::Perspective {
            let (_, _, forward) = self.camera_basis();
            let depth = self.camera_distance() + dot(point - self.camera_target(), forward);
            if depth <= PERSPECTIVE_NEAR_MM {
                return None;
            }
        }
        let screen = self.project(point, rect);
        Some([
            f64::from(screen.x) / POINTS_PER_PAPER_MM,
            f64::from(screen.y) / POINTS_PER_PAPER_MM,
        ])
    }

    /// Every shown annotation as the viewport draws it this frame.
    pub(crate) fn drawn_annotations(
        &self,
        context: &egui::Context,
        rect: Rect,
    ) -> Vec<DrawnAnnotation> {
        let snapshot = self.document.current();
        let screen = |[x, y]: [f64; 2]| {
            let [x, y, _] =
                single_precision([x * POINTS_PER_PAPER_MM, y * POINTS_PER_PAPER_MM, 0.0]);
            Pos2::new(x, y)
        };
        snapshot
            .annotations()
            .iter()
            .filter(|annotation| snapshot.tags_visible(&annotation.tags))
            .filter_map(|annotation| {
                let marks =
                    annotation_marks(annotation, |point| self.annotation_page_point(point, rect))?;
                let mut drawn = DrawnAnnotation {
                    name: annotation.name.clone(),
                    lines: Vec::new(),
                    fills: Vec::new(),
                    texts: Vec::new(),
                };
                for mark in marks {
                    match mark {
                        Mark::Line { points, closed, .. } => drawn
                            .lines
                            .push((points.into_iter().map(screen).collect(), closed)),
                        Mark::Fill { loops, .. } => drawn.fills.extend(
                            loops
                                .into_iter()
                                .map(|points| points.into_iter().map(screen).collect()),
                        ),
                        Mark::Text {
                            at,
                            text,
                            height_mm,
                            anchor,
                            angle_deg,
                            ..
                        } => drawn.texts.push(drawn_text(
                            context,
                            screen(at),
                            text,
                            height_mm,
                            anchor,
                            angle_deg,
                        )),
                    }
                }
                Some(drawn)
            })
            .collect()
    }

    /// Paints the shown annotations over the model, the picked one in the
    /// selection colour.
    pub(crate) fn paint_annotations(&self, painter: &egui::Painter, rect: Rect) {
        if self.document.current().annotations().is_empty() {
            return;
        }
        let palette = self.palette();
        let picked = self.picked_annotation_name();
        for drawn in self.drawn_annotations(painter.ctx(), rect) {
            let color = if picked == Some(drawn.name.as_str()) {
                palette.accent
            } else {
                palette.text
            };
            let stroke = Stroke::new(1.0_f32, color);
            for (points, closed) in drawn.lines {
                painter.add(if closed {
                    egui::Shape::closed_line(points, stroke)
                } else {
                    egui::Shape::line(points, stroke)
                });
            }
            for points in drawn.fills {
                painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
            }
            for text in drawn.texts {
                painter.add(
                    egui::epaint::TextShape::new(text.pos, text.galley, color)
                        .with_angle(text.angle),
                );
            }
        }
    }
}

/// A line of text with capitals `height_mm` of the page tall, its baseline at
/// `at` (screen points) and turned `angle_deg` counter-clockwise about it.
fn drawn_text(
    context: &egui::Context,
    at: Pos2,
    text: String,
    height_mm: f64,
    anchor: Anchor,
    angle_deg: f64,
) -> DrawnText {
    let [size, angle, _] = single_precision([
        SheetFont::standard().size_for(height_mm) * POINTS_PER_PAPER_MM,
        -angle_deg.to_radians(),
        0.0,
    ]);
    let galley = context.fonts(|fonts| {
        fonts.layout_no_wrap(text, egui::FontId::proportional(size), Color32::PLACEHOLDER)
    });
    let baseline = galley.rows.first().map_or(galley.rect.height(), |row| {
        row.pos.y + row.row.glyphs.first().map_or(0.0, |glyph| glyph.pos.y)
    });
    let width = galley.rect.width();
    let along = match anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => -width / 2.0,
        Anchor::End => -width,
    };
    let offset = egui::emath::Rot2::from_angle(angle) * egui::vec2(along, -baseline);
    DrawnText {
        pos: at + offset,
        angle,
        galley,
    }
}
