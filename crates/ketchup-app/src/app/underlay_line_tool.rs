//! The Trim, Extend and Offset line tools on an underlay's drawing: a click on a line of
//! the drawing trims away its piece between the lines that cross it there, or extends its
//! nearer end to the next line it meets; Offset takes a click on the line, then one on the
//! side (a typed length sets the distance). Mirror with objects of a drawing picked flips
//! them across a line of two clicks. Each is one Undo step.

use crate::app::underlay_edit::{FindRegions, UnderlayEditError};
use crate::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::line_edit::LineEdit;
use ketchup_model::underlay::objects::Affine;

impl KetchupApp {
    /// The drawing, the object of it whose line is nearest `pointer`, and the point of
    /// that line under it in the millimetres of the drawing's lines.
    pub(crate) fn underlay_line_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(UnderlayId, u32, [f64; 2])> {
        let (underlay, segment, point) = self
            .underlay_snaps_at_screen(pointer, rect, 8.0)
            .into_iter()
            .filter_map(|(reference, _, point)| match reference.element {
                ElementId::Underlay { underlay, segment } => Some((
                    self.project(point, rect).distance(pointer),
                    underlay,
                    segment,
                    point,
                )),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, underlay, segment, point)| (underlay, segment, point))?;
        let object = self.underlay_object_at(underlay, segment)?;
        let local = self
            .document
            .current()
            .underlay(underlay)?
            .to_local([point.x, point.y]);
        Some((underlay, object, local))
    }

    /// One click of the Trim, Extend or Offset line tool.
    pub(crate) fn click_underlay_line_tool(&mut self, pointer: Pos2, rect: Rect) {
        if self.active_tool == ActiveTool::OffsetUnderlayLine
            && let Some((underlay, object)) = self.gesture.underlay_offset
        {
            self.offset_underlay_line_to(underlay, object, pointer, rect);
            return;
        }
        let Some((underlay, object, pick_mm)) = self.underlay_line_at_screen(pointer, rect) else {
            self.digest = self.catalog.text("digest-underlay-line-required");
            return;
        };
        let (edit, done) = match self.active_tool {
            ActiveTool::TrimUnderlayLine => {
                (LineEdit::Trim { pick_mm }, "digest-underlay-line-trimmed")
            }
            ActiveTool::ExtendUnderlayLine => (
                LineEdit::Extend { pick_mm },
                "digest-underlay-line-extended",
            ),
            _ => {
                self.gesture.underlay_offset = Some((underlay, object));
                self.status_key = "status-underlay-offset-side";
                return;
            }
        };
        self.apply_underlay_line_edit(underlay, object, edit, done);
    }

    /// The offset of the picked line to the side of `pointer`: as far as typed, else
    /// through the point clicked.
    fn offset_underlay_line_to(
        &mut self,
        underlay: UnderlayId,
        object: u32,
        pointer: Pos2,
        rect: Rect,
    ) {
        let Some(point) = self.measurement_point_at_screen(pointer, rect, 0.0) else {
            return;
        };
        let Some(found) = self.document.current().underlay(underlay).cloned() else {
            self.gesture.underlay_offset = None;
            self.status_key = "status-underlay-offset-line";
            return;
        };
        let typed = self.value_box.input.trim();
        let distance_mm = if typed.is_empty() {
            None
        } else {
            let Some(length) = parse_distance_mm(typed, self.value_box.decimal) else {
                self.digest = self.catalog.text("digest-sketch-invalid-length");
                return;
            };
            Some(length / found.scale())
        };
        let edit = LineEdit::Offset {
            distance_mm,
            side_mm: found.to_local([point.x, point.y]),
        };
        if self.apply_underlay_line_edit(underlay, object, edit, "digest-underlay-line-offset") {
            self.gesture.underlay_offset = None;
            self.status_key = "status-underlay-offset-line";
            self.value_box.input.clear();
        }
    }

    /// One click of Mirror while objects of a drawing are picked: the first fixes a point
    /// of the mirror line, the second its other point and flips the objects across it, or
    /// with `copy` adds flipped copies and keeps them.
    pub(crate) fn click_underlay_mirror(&mut self, pointer: Pos2, rect: Rect, copy: bool) {
        let Some(point) = self.measurement_point_at_screen(pointer, rect, 0.0) else {
            return;
        };
        let Some(from) = self.gesture.underlay_mirror_from.take() else {
            self.gesture.underlay_mirror_from = Some([point.x, point.y]);
            self.status_key = "status-underlay-mirror-second";
            return;
        };
        self.status_key = "status-underlay-mirror-first";
        let Some((underlay, ids)) = self.picked_underlay_objects() else {
            return;
        };
        let Some(found) = self.document.current().underlay(underlay).cloned() else {
            return;
        };
        let Some(map) = mirror_across(found.to_local(from), found.to_local([point.x, point.y]))
        else {
            self.digest = self.catalog.text("digest-underlay-mirror-line-required");
            return;
        };
        self.mirror_underlay_objects_in_window(underlay, ids, map, copy);
    }

    fn mirror_underlay_objects_in_window(
        &mut self,
        underlay: UnderlayId,
        ids: Vec<u32>,
        map: Affine,
        copy: bool,
    ) {
        match self.transform_underlay_objects(underlay, &ids, &map, copy, FindRegions::OffUiThread)
        {
            Err(UnderlayEditError::FindingRegions(batch)) => {
                self.await_followed_regions(batch, move |app| {
                    app.mirror_underlay_objects_in_window(underlay, ids, map, copy);
                });
            }
            Ok(changes) => {
                self.digest = self.catalog.text("digest-underlay-objects-mirrored")
                    + &self.followed_note(&changes);
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "digest-underlay-edit-failed",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
            }
        }
    }

    fn apply_underlay_line_edit(
        &mut self,
        underlay: UnderlayId,
        object: u32,
        edit: LineEdit,
        done: &'static str,
    ) -> bool {
        match self.edit_underlay_line(underlay, object, edit, FindRegions::OffUiThread) {
            Err(UnderlayEditError::FindingRegions(batch)) => {
                self.await_followed_regions(batch, move |app| {
                    app.apply_underlay_line_edit(underlay, object, edit, done);
                });
                true
            }
            Ok(changes) => {
                self.digest = self.catalog.text(done) + &self.followed_note(&changes);
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

/// The flip of the plane across the line through `a` and `b`; none when they coincide.
fn mirror_across(a: [f64; 2], b: [f64; 2]) -> Option<Affine> {
    let length = (b[0] - a[0]).hypot(b[1] - a[1]);
    if !length.is_finite() || length <= limits::MIN_LENGTH_MM {
        return None;
    }
    let [x, y] = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
    let [m00, m01, m11] = [x * x - y * y, 2.0 * x * y, y * y - x * x];
    Some([
        [m00, m01, a[0] - m00 * a[0] - m01 * a[1]],
        [m01, m11, a[1] - m01 * a[0] - m11 * a[1]],
    ])
}
