//! Drawing into an underlay's drawing with the Line, Rectangle, Circle and Arc tools: while
//! Draw into drawing is on, what they finish becomes a LINE, LWPOLYLINE, CIRCLE or ARC on a
//! layer of the drawing, one Undo step each, instead of a profile of the model.

use crate::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::drawn::{DrawnShape, inverse};
use ketchup_model::underlay::objects::apply;

/// The drawing and the layer of it the drawing tools draw into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnderlayDrawTarget {
    pub(crate) underlay: UnderlayId,
    pub(crate) layer: String,
}

impl KetchupApp {
    /// Turns drawing into the selected drawing on, on the layer of the object picked first
    /// in it (else layer "0", else its first layer) with the Line tool unless a drawing tool
    /// is on; or turns it off.
    pub(crate) fn toggle_underlay_draw(&mut self) {
        if self.gesture.underlay_draw.take().is_some() {
            self.digest = self.catalog.text("digest-underlay-draw-off");
            return;
        }
        let Some(underlay) = self.selected_underlay() else {
            return;
        };
        let Some(layer) = self.underlay_draw_layer(underlay) else {
            return;
        };
        if !matches!(
            self.active_tool,
            ActiveTool::Line | ActiveTool::Rectangle | ActiveTool::Circle | ActiveTool::Arc
        ) {
            self.dispatch_command(AppCommand::Line);
        }
        self.digest = self.catalog.format(
            "digest-underlay-draw-on",
            &BTreeMap::from([("layer", layer.clone())]),
        );
        self.gesture.underlay_draw = Some(UnderlayDrawTarget { underlay, layer });
    }

    /// The layer drawing into `underlay` starts on.
    fn underlay_draw_layer(&self, underlay: UnderlayId) -> Option<String> {
        let snapshot = self.document.current();
        let found = snapshot.underlay(underlay)?;
        let picked = self.selection.underlay_objects().first().copied();
        let picked_layer = picked.and_then(|id| {
            let lines = self.cached_underlay_lines(found)?;
            let object = lines.objects().objects.get(usize::try_from(id).ok()?)?;
            Some(lines.objects().layer_name(object).to_owned())
        });
        picked_layer
            .or_else(|| {
                found
                    .layers
                    .iter()
                    .find(|layer| layer.name == "0")
                    .map(|layer| layer.name.clone())
            })
            .or_else(|| found.layers.first().map(|layer| layer.name.clone()))
    }

    /// Where the drawing tools draw, while the drawing is still in the document.
    pub(crate) fn underlay_draw_target(&self) -> Option<UnderlayDrawTarget> {
        let target = self.gesture.underlay_draw.as_ref()?;
        self.document
            .current()
            .underlay(target.underlay)
            .is_some()
            .then(|| target.clone())
    }

    /// The finished line from `start` to `end`, drawn into the drawing as a LINE; the
    /// chain goes on from `end`, and ends with a line back to where it began.
    pub(crate) fn draw_underlay_line(&mut self, start: Vec3, end: Vec3) -> bool {
        let closes = self
            .gesture
            .sketch
            .chain_origin
            .is_some_and(|origin| length(end - origin) <= 0.1);
        if !self.draw_underlay_shapes(&[start, end], |points| {
            vec![DrawnShape::Polyline {
                points_mm: points.to_vec(),
                closed: false,
            }]
        }) {
            return false;
        }
        if closes {
            self.rearm_sketch();
        } else {
            let sketch = &mut self.gesture.sketch;
            sketch.chain_points.push(end);
            sketch.armed = true;
            sketch.start = Some(end);
            sketch.cursor = Some(end);
            self.mark_sketch_source();
            self.status_key = "status-line-end";
            self.value_box.input.clear();
        }
        true
    }

    /// The drawing tool ready for the first point of the next shape.
    fn rearm_sketch(&mut self) {
        self.cancel_rectangle_sketch();
        self.gesture.sketch.armed = true;
        self.status_key = match self.active_tool {
            ActiveTool::Line => "status-line-start",
            ActiveTool::Circle => "status-circle-center",
            ActiveTool::Arc => "status-arc-start",
            _ => "status-sketch-first-point",
        };
        self.value_box.input.clear();
    }

    /// The rectangle with the corners `start` and `end` across, its sides along the
    /// document's axes, drawn into the drawing as a closed LWPOLYLINE.
    pub(crate) fn draw_underlay_rectangle(&mut self, start: Vec3, end: Vec3) -> bool {
        let corners = [
            start,
            Vec3::new(end.x, start.y, start.z),
            end,
            Vec3::new(start.x, end.y, start.z),
        ];
        self.draw_underlay_shapes(&corners, |points| {
            vec![DrawnShape::Polyline {
                points_mm: points.to_vec(),
                closed: true,
            }]
        })
    }

    /// The circle about `center` of radius `radius_mm`, drawn into the drawing.
    pub(crate) fn draw_underlay_circle(&mut self, center: Vec3, radius_mm: f64) -> bool {
        let Some(target) = self.underlay_draw_target() else {
            return false;
        };
        let Some(scale) = self
            .document
            .current()
            .underlay(target.underlay)
            .map(|underlay| underlay.scale())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
        else {
            return false;
        };
        self.draw_underlay_shapes(&[center], |points| {
            vec![DrawnShape::Circle {
                center_mm: points[0],
                radius_mm: radius_mm / scale,
            }]
        })
    }

    /// The arc from `start` through `bulge_point` to `end`, drawn into the drawing.
    pub(crate) fn draw_underlay_arc(&mut self, start: Vec3, end: Vec3, bulge_point: Vec3) -> bool {
        self.draw_underlay_shapes(&[start, bulge_point, end], |points| {
            arc_through(points[0], points[1], points[2])
                .into_iter()
                .collect()
        })
    }

    /// Draws into the drawing the shapes `shape` makes of `points`, taken into the
    /// millimetres of its lines; says what was drawn or why not.
    fn draw_underlay_shapes(
        &mut self,
        points: &[Vec3],
        shape: impl FnOnce(&[[f64; 2]]) -> Vec<DrawnShape>,
    ) -> bool {
        let Some(target) = self.underlay_draw_target() else {
            return false;
        };
        let Some(to_lines) = self
            .document
            .current()
            .underlay(target.underlay)
            .and_then(|underlay| inverse(&underlay.plane_map()))
        else {
            return false;
        };
        let in_lines = points
            .iter()
            .map(|point| apply(&to_lines, [point.x, point.y]))
            .collect::<Vec<_>>();
        let shapes = shape(&in_lines);
        let Some(kind) = shapes.first().map(DrawnShape::kind) else {
            return false;
        };
        match self.draw_into_underlay(target.underlay, &target.layer, shapes) {
            Ok(()) => {
                // A line goes on from its end; other shapes are done.
                if self.active_tool != ActiveTool::Line {
                    self.rearm_sketch();
                }
                self.digest = self.catalog.format(
                    "digest-underlay-drawn",
                    &BTreeMap::from([("kind", kind.to_owned()), ("layer", target.layer)]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "digest-underlay-edit-failed",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }
}

/// The ARC from `start` through `through` to `end`, counterclockwise as a drawing keeps
/// it; none when the three points lie on one line.
fn arc_through(start: [f64; 2], through: [f64; 2], end: [f64; 2]) -> Option<DrawnShape> {
    let [ax, ay] = start;
    let [bx, by] = through;
    let [cx, cy] = end;
    let twice_area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    let size = (bx - ax).hypot(by - ay).max((cx - ax).hypot(cy - ay));
    if !twice_area.is_finite() || twice_area.abs() <= 1e-9 * size * size {
        return None;
    }
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    let a2 = ax * ax + ay * ay;
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let center = [
        (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d,
        (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d,
    ];
    let angle = |[x, y]: [f64; 2]| (y - center[1]).atan2(x - center[0]).to_degrees();
    // Start, through, end turning left run counterclockwise from start to end.
    let (from, to) = if twice_area > 0.0 {
        (angle(start), angle(end))
    } else {
        (angle(end), angle(start))
    };
    Some(DrawnShape::Arc {
        center_mm: center,
        radius_mm: (ax - center[0]).hypot(ay - center[1]),
        start_deg: from,
        end_deg: to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc_parts(shape: Option<DrawnShape>) -> ([f64; 2], f64, f64, f64) {
        match shape {
            Some(DrawnShape::Arc {
                center_mm,
                radius_mm,
                start_deg,
                end_deg,
            }) => (center_mm, radius_mm, start_deg, end_deg),
            other => panic!("not an arc: {other:?}"),
        }
    }

    #[test]
    fn an_arc_through_three_points_runs_counterclockwise_whichever_way_it_was_drawn() {
        let (center, radius, start, end) =
            arc_parts(arc_through([10.0, 0.0], [0.0, 10.0], [-10.0, 0.0]));
        assert!(center[0].abs() < 1e-9 && center[1].abs() < 1e-9);
        assert!((radius - 10.0).abs() < 1e-9);
        assert!(start.abs() < 1e-9 && (end - 180.0).abs() < 1e-9);
        // The same arc drawn from its other end.
        let (_, _, start, end) = arc_parts(arc_through([-10.0, 0.0], [0.0, 10.0], [10.0, 0.0]));
        assert!(start.abs() < 1e-9 && (end - 180.0).abs() < 1e-9);
        // Through the bottom instead: from 180° on round to 360°.
        let (_, _, start, end) = arc_parts(arc_through([10.0, 0.0], [0.0, -10.0], [-10.0, 0.0]));
        assert!((start - 180.0).abs() < 1e-9 && end.abs() < 1e-9);
    }

    #[test]
    fn three_points_on_one_line_make_no_arc() {
        assert_eq!(arc_through([0.0, 0.0], [1.0, 1.0], [2.0, 2.0]), None);
    }
}
