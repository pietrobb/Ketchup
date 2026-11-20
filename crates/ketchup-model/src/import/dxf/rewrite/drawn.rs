//! The entities of the shapes drawn over an underlay, as the drawing's version writes them:
//! LINE, LWPOLYLINE or POLYLINE, ARC, CIRCLE, TEXT or MTEXT, and HATCH.

use super::{Drawing, DxfEditRefusal, Frame, arc_angles, number, take_handle};
use crate::underlay::drawn::DrawnShape;

impl Drawing<'_> {
    /// The entity of `shape`, drawn on `layer`, in the drawing's units.
    pub(super) fn drawn(
        &self,
        layer: &str,
        shape: &DrawnShape,
        frame: &Frame,
        handles: &mut Option<u64>,
    ) -> Result<Vec<u8>, DxfEditRefusal> {
        let mut out = Vec::new();
        let mut pair = |code: i32, value: &[u8]| {
            out.extend_from_slice(format!("{code:>3}").as_bytes());
            out.extend_from_slice(self.eol);
            out.extend_from_slice(value);
            out.extend_from_slice(self.eol);
        };
        let layer_bytes = self
            .layers
            .iter()
            .find(|(name, _)| name == layer)
            .map_or_else(|| self.name(layer), |(_, raw)| raw.clone());
        // The record's head, owned by `owner`; the handle it took.
        let owned_head = |pair: &mut dyn FnMut(i32, &[u8]),
                          kind: &str,
                          handles: &mut Option<u64>,
                          owner: Option<&[u8]>| {
            pair(0, kind.as_bytes());
            let handle = take_handle(handles);
            if let Some(handle) = handle {
                pair(5, format!("{handle:X}").as_bytes());
            }
            if self.subclassed {
                if let Some(owner) = owner {
                    pair(330, owner);
                }
                pair(100, b"AcDbEntity");
            }
            pair(8, &layer_bytes);
            handle
        };
        let head = |pair: &mut dyn FnMut(i32, &[u8]), kind: &str, handles: &mut Option<u64>| {
            owned_head(pair, kind, handles, self.owner.as_deref());
        };
        let subclass = |pair: &mut dyn FnMut(i32, &[u8]), name: &str| {
            if self.subclassed {
                pair(100, name.as_bytes());
            }
        };
        let point = |pair: &mut dyn FnMut(i32, &[u8]), code: i32, at: [f64; 2]| {
            let [x, y] = frame.point(at);
            pair(code, number(x).as_bytes());
            pair(code + 10, number(y).as_bytes());
            pair(code + 20, b"0");
        };
        let length = |value: f64| number(value / frame.mm_per_unit);
        match shape {
            DrawnShape::Polyline { points_mm, closed } if points_mm.len() == 2 && !closed => {
                head(&mut pair, "LINE", handles);
                subclass(&mut pair, "AcDbLine");
                point(&mut pair, 10, points_mm[0]);
                point(&mut pair, 11, points_mm[1]);
            }
            DrawnShape::Polyline { points_mm, closed } if self.lightweight => {
                head(&mut pair, "LWPOLYLINE", handles);
                subclass(&mut pair, "AcDbPolyline");
                pair(90, points_mm.len().to_string().as_bytes());
                pair(70, if *closed { b"1" } else { b"0" });
                for at in points_mm {
                    let [x, y] = frame.point(*at);
                    pair(10, number(x).as_bytes());
                    pair(20, number(y).as_bytes());
                }
            }
            DrawnShape::Polyline { points_mm, closed } => {
                // DXF 12, or 13, which marks its records as subclasses.
                let polyline = owned_head(&mut pair, "POLYLINE", handles, self.owner.as_deref());
                subclass(&mut pair, "AcDb2dPolyline");
                pair(66, b"1");
                pair(70, if *closed { b"1" } else { b"0" });
                point(
                    &mut pair,
                    10,
                    [0.0 - frame.offset_mm[0], 0.0 - frame.offset_mm[1]],
                );
                let owner = polyline.map(|handle| format!("{handle:X}").into_bytes());
                for at in points_mm {
                    owned_head(&mut pair, "VERTEX", handles, owner.as_deref());
                    subclass(&mut pair, "AcDbVertex");
                    subclass(&mut pair, "AcDb2dVertex");
                    point(&mut pair, 10, *at);
                }
                owned_head(&mut pair, "SEQEND", handles, owner.as_deref());
            }
            DrawnShape::Arc {
                center_mm,
                radius_mm,
                start_deg,
                end_deg,
            } => {
                head(&mut pair, "ARC", handles);
                subclass(&mut pair, "AcDbCircle");
                point(&mut pair, 10, *center_mm);
                pair(40, length(*radius_mm).as_bytes());
                subclass(&mut pair, "AcDbArc");
                // A whole turn is `0 / 360`, not `0 / 0` (review 2026-11-08 UL-241).
                let (start_deg, end_deg) = arc_angles(*start_deg, *end_deg, 360.0);
                pair(50, number(start_deg).as_bytes());
                pair(51, number(end_deg).as_bytes());
            }
            DrawnShape::Circle {
                center_mm,
                radius_mm,
            } => {
                head(&mut pair, "CIRCLE", handles);
                subclass(&mut pair, "AcDbCircle");
                point(&mut pair, 10, *center_mm);
                pair(40, length(*radius_mm).as_bytes());
            }
            DrawnShape::Text {
                anchor_mm,
                height_mm,
                rotation_deg,
                text,
            } => {
                let characters = self.text_characters(text);
                let rotation = number(rotation_deg.rem_euclid(360.0));
                if characters <= super::super::MAX_DXF_LINE_CHARS {
                    head(&mut pair, "TEXT", handles);
                    subclass(&mut pair, "AcDbText");
                    point(&mut pair, 10, *anchor_mm);
                    pair(40, length(*height_mm).as_bytes());
                    pair(1, &self.text(text));
                    pair(50, rotation.as_bytes());
                    subclass(&mut pair, "AcDbText");
                } else if self.subclassed {
                    // Too long for a TEXT's one line: an MTEXT, which the line limit cuts
                    // into pieces (review 2026-11-07 r26 UL-240), from its bottom left.
                    head(&mut pair, "MTEXT", handles);
                    subclass(&mut pair, "AcDbMText");
                    point(&mut pair, 10, *anchor_mm);
                    pair(40, length(*height_mm).as_bytes());
                    pair(71, b"7");
                    pair(72, b"1");
                    pair(1, &self.mtext(text));
                    pair(50, rotation.as_bytes());
                } else {
                    return Err(DxfEditRefusal::DrawnTextTooLong {
                        starts: text.chars().take(20).collect(),
                        characters,
                        written: None,
                    });
                }
            }
            // DXF 12 has no HATCH: the fill is refused rather than left out.
            DrawnShape::Hatch { .. } if !self.subclassed => {
                return Err(DxfEditRefusal::DrawnHatchInDxf12 {
                    layer: layer.to_owned(),
                });
            }
            DrawnShape::Hatch { rings_mm } => {
                // A solid, not associative fill of polyline paths, the first outermost. In a
                // light grey of its own (ACI 254) and, where the drawing can say so, 60 %
                // transparent (by value, 0x02000000, and 102 of 255 opaque): the lines and
                // texts over it show, and the area is not printed solid in the layer's colour.
                head(&mut pair, "HATCH", handles);
                pair(62, b"254");
                if self.transparency {
                    pair(440, b"33554534");
                }
                subclass(&mut pair, "AcDbHatch");
                point(
                    &mut pair,
                    10,
                    [0.0 - frame.offset_mm[0], 0.0 - frame.offset_mm[1]],
                );
                pair(210, b"0");
                pair(220, b"0");
                pair(230, b"1");
                pair(2, b"SOLID");
                pair(70, b"1");
                pair(71, b"0");
                pair(91, rings_mm.len().to_string().as_bytes());
                for (at, ring) in rings_mm.iter().enumerate() {
                    // Polyline (2), the first also external (1) and outermost (16).
                    pair(92, if at == 0 { b"19" } else { b"2" });
                    pair(72, b"0");
                    pair(73, b"1");
                    pair(93, ring.len().to_string().as_bytes());
                    for corner in ring {
                        let [x, y] = frame.point(*corner);
                        pair(10, number(x).as_bytes());
                        pair(20, number(y).as_bytes());
                    }
                    pair(97, b"0");
                }
                pair(75, b"0");
                pair(76, b"1");
                pair(98, b"0");
            }
        }
        Ok(out)
    }
}
