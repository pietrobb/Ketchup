//! Snapping to and measuring on the shown lines of the underlays: their endpoints,
//! intersections, midpoints and nearest points join the snap points of the model, so drawing, Measure
//! and the bridge's `underlay_snap` all find the same points.

use crate::*;
use ketchup_model::underlay::object_query::ObjectIndex;
use ketchup_model::underlay::snap::{
    UnderlayPiece, UnderlaySnap, UnderlaySnapIndex, UnderlaySnapKind,
};
use ketchup_model::underlay::{Underlay, UnderlayId, UnderlayLines};
use std::sync::mpsc::{Receiver, TryRecvError};

/// How many times longer than wide the snap window on an underlay may be: a plane seen at
/// a slant stretches the pointer's tolerance up and down the screen, without end as it is
/// seen edge-on (review 2026-10-21 P3).
const MAX_SNAP_WINDOW_STRETCH: f64 = 8.0;

/// What a snap on an underlay line references: no definition and no occurrence.
pub(crate) fn underlay_reference(underlay: UnderlayId, segment: usize) -> SelectionId {
    SelectionId {
        definition_id: DefinitionId(0),
        instance_path: InstancePath::root(OccurrenceId(0)),
        element: ElementId::Underlay {
            underlay,
            segment: u32::try_from(segment).unwrap_or(u32::MAX),
        },
    }
}

/// The underlay `reference` names, if it names one.
pub(crate) const fn underlay_of(reference: &SelectionId) -> Option<UnderlayId> {
    match reference.element {
        ElementId::Underlay { underlay, .. } => Some(underlay),
        _ => None,
    }
}

/// The snap kind of the model's snaps a snap on an underlay line ranks as.
pub(crate) fn snap_kind(kind: UnderlaySnapKind) -> SnapKind {
    match kind {
        UnderlaySnapKind::Endpoint => SnapKind::Endpoint,
        UnderlaySnapKind::Intersection => SnapKind::Intersection,
        UnderlaySnapKind::Midpoint => SnapKind::Midpoint,
        UnderlaySnapKind::Nearest => SnapKind::Edge,
    }
}

/// The snap index of one blob of underlay lines, never built on the window thread, since a
/// large drawing takes about 100 ms: an imported drawing brings it from the thread that read
/// it, an opened document's underlay builds it on a thread of its own and gives no snaps
/// until it is there.
pub(crate) enum UnderlaySnapSlot {
    Building(Receiver<UnderlaySnapIndex>),
    Ready(Arc<UnderlaySnapIndex>),
}

impl KetchupApp {
    /// The snap index of `underlay`'s lines, or `None` while it is still being built; the
    /// first ask starts building it. Lines that do not decode give no snaps; drawing them
    /// reports why.
    pub(crate) fn underlay_snap_index(
        &self,
        underlay: &Underlay,
    ) -> Option<Arc<UnderlaySnapIndex>> {
        let key = underlay.lines_key();
        let mut slots = self.render.underlay_snaps.borrow_mut();
        let built = match slots.get(&key) {
            Some(UnderlaySnapSlot::Ready(index)) => return Some(Arc::clone(index)),
            Some(UnderlaySnapSlot::Building(receiver)) => match receiver.try_recv() {
                Ok(index) => index,
                Err(TryRecvError::Empty) => return None,
                // The build panicked: no snaps rather than another try every frame.
                Err(TryRecvError::Disconnected) => {
                    UnderlaySnapIndex::new(&UnderlayLines::default())
                }
            },
            None => {
                drop(slots);
                let lines = self
                    .file
                    .container_data
                    .blobs()
                    .get(&underlay.lines_blob())
                    .and_then(|bytes| underlay.read_lines(bytes).ok())
                    .unwrap_or_default();
                self.build_underlay_snap_index(key, lines);
                return None;
            }
        };
        let index = Arc::new(built);
        slots.insert(key, UnderlaySnapSlot::Ready(Arc::clone(&index)));
        Some(index)
    }

    /// Starts building the snap index of the lines of blob `key` on a thread of its own.
    pub(crate) fn build_underlay_snap_index(&self, key: [u8; 32], lines: UnderlayLines) {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(UnderlaySnapIndex::new(&lines));
        });
        self.render
            .underlay_snaps
            .borrow_mut()
            .insert(key, UnderlaySnapSlot::Building(receiver));
    }

    /// Whether the snap index of every underlay of the document is there, none still being
    /// built or not started.
    #[cfg(test)]
    pub(crate) fn underlay_snaps_built(&self) -> bool {
        let slots = self.render.underlay_snaps.borrow();
        self.document.current().underlays().all(|underlay| {
            matches!(
                slots.get(&underlay.lines_key()),
                Some(UnderlaySnapSlot::Ready(_))
            )
        })
    }

    /// Waits until the snap index of every underlay of the document is built.
    #[cfg(test)]
    pub(crate) fn wait_for_underlay_snaps(&self) {
        for underlay in self.document.current().underlays() {
            if self.underlay_snap_index(underlay).is_some() {
                continue;
            }
            let key = underlay.lines_key();
            let mut slots = self.render.underlay_snaps.borrow_mut();
            if let Some(UnderlaySnapSlot::Building(receiver)) = slots.get(&key) {
                let index = receiver
                    .recv()
                    .expect("the snap index thread sends its index");
                slots.insert(key, UnderlaySnapSlot::Ready(Arc::new(index)));
            }
        }
    }

    /// Whether `underlay` and its layer `layer` are shown where the window's tools reach
    /// them: in the ground plane, not set up as a view.
    pub(super) fn underlay_layer_shown(
        snapshot: &Snapshot,
        underlay: &Underlay,
        layer: usize,
    ) -> bool {
        let shown = |tag| snapshot.tag(tag).is_some_and(|tag| tag.visible());
        underlay.lies_flat()
            && shown(underlay.tag)
            && underlay
                .layers
                .get(layer)
                .is_some_and(|layer| shown(layer.tag))
    }

    /// The best endpoint, intersection, midpoint and nearest point of each underlay's shown
    /// lines, where its placement puts them, within `radius_mm` of `point_mm`, in the
    /// document's XY plane, and whether the snap index of every shown underlay was there to
    /// ask.
    pub(crate) fn underlay_snaps_near(
        &self,
        snapshot: &Snapshot,
        point_mm: [f64; 2],
        radius_mm: f64,
    ) -> (Vec<(UnderlayId, UnderlaySnap)>, bool) {
        self.underlay_snaps_near_of(snapshot, point_mm, radius_mm, |_| true)
    }

    /// [`Self::underlay_snaps_near`] of the underlays `of` keeps only.
    fn underlay_snaps_near_of(
        &self,
        snapshot: &Snapshot,
        point_mm: [f64; 2],
        radius_mm: f64,
        of: impl Fn(&Underlay) -> bool,
    ) -> (Vec<(UnderlayId, UnderlaySnap)>, bool) {
        let mut found = Vec::new();
        let mut ready = true;
        for underlay in snapshot.underlays().filter(|underlay| of(underlay)) {
            if !snapshot.tag(underlay.tag).is_some_and(|tag| tag.visible()) {
                continue;
            }
            let Some(index) = self.underlay_snap_index(underlay) else {
                ready = false;
                continue;
            };
            let scale = underlay.scale();
            let snaps = index.snaps(underlay.to_local(point_mm), radius_mm / scale, |layer| {
                Self::underlay_layer_shown(snapshot, underlay, layer)
            });
            found.extend(snaps.iter().map(|snap| {
                let placed = UnderlaySnap {
                    point_mm: underlay.to_document(snap.point_mm),
                    distance_mm: snap.distance_mm * scale,
                    ..*snap
                };
                (underlay.id, placed)
            }));
        }
        (found, ready)
    }

    /// The line or arc of underlay `id` its straight piece `segment` belongs to, where the
    /// underlay's placement puts it.
    pub(crate) fn underlay_piece(&self, id: UnderlayId, segment: usize) -> Option<UnderlayPiece> {
        let snapshot = self.document.current();
        let underlay = snapshot.underlay(id)?;
        let piece = self.underlay_snap_index(underlay)?.piece(segment)?;
        Some(UnderlayPiece {
            points_mm: piece
                .points_mm
                .iter()
                .map(|point| underlay.to_document(*point))
                .collect(),
            length_mm: piece.length_mm * underlay.scale(),
            ..piece
        })
    }

    /// Where the pointer meets the plane of plans at `height_mm`, and how far from there a
    /// point of that plane may lie to be within `tolerance_px` of the pointer: up and down
    /// the screen as far as that reaches, but at most [`MAX_SNAP_WINDOW_STRETCH`] times its
    /// reach across the screen, so that a plane seen almost edge-on is not searched to
    /// its horizon.
    pub(crate) fn underlay_snap_window(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f32,
        height_mm: f64,
    ) -> Option<([f64; 2], f64)> {
        let at = |offset: Vec2| self.screen_to_plane(pointer + offset, rect, height_mm);
        let center = at(Vec2::ZERO)?;
        let reach = |offsets: [Vec2; 2]| {
            offsets
                .into_iter()
                .filter_map(at)
                .map(|point| point.distance(center))
                .fold(0.0, f64::max)
        };
        let across = reach([Vec2::new(tolerance_px, 0.0), Vec2::new(-tolerance_px, 0.0)]);
        let along = reach([Vec2::new(0.0, tolerance_px), Vec2::new(0.0, -tolerance_px)]);
        let radius = if across > 0.0 {
            along.min(MAX_SNAP_WINDOW_STRETCH * across).max(across)
        } else {
            along
        };
        Some(([center.x, center.y], radius))
    }

    /// The snaps on the shown underlay lines within `tolerance_px` of the pointer, as the
    /// model's snaps rank them. An underlay the Move tool is moving has none: it is drawn
    /// where the pointer takes it, not where its lines were (review 2026-10-24 UI-25).
    pub(crate) fn underlay_snaps_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f32,
    ) -> Vec<(SelectionId, SnapKind, Vec3)> {
        let snapshot = self.document.current();
        // Each plan snaps where the pointer meets the plane it lies in, on the ground or
        // raised off it (review 2026-10-29 UO-69).
        let mut heights = snapshot
            .underlays()
            .filter_map(Underlay::plan_height)
            .collect::<Vec<_>>();
        heights.sort_by(f64::total_cmp);
        heights.dedup();
        if heights.is_empty() {
            return Vec::new();
        }
        // Objects picked in a drawing move or turn alone, and the rest of their drawing
        // snaps; a turn moves them once its arm is set (review 2026-10-29 UI-63).
        let dragged = match self.transform_tool.session.as_ref() {
            Some(ToolSession::Move { drag, .. }) => Some(&drag.selection),
            Some(ToolSession::Rotate { drag, .. }) if drag.reference_mm.is_some() => {
                Some(&drag.selection)
            }
            _ => None,
        };
        let (moving, picked) = match dragged {
            Some(selection) => match self.dragged_underlay_objects(selection) {
                Some(picked) => (None, Some(picked)),
                _ => (underlay_of(selection), None),
            },
            None => (None, None),
        };
        let picked_lines = picked.as_ref().and_then(|(id, objects)| {
            let lines = self.cached_underlay_lines(snapshot.underlay(*id)?)?;
            Some((*id, objects, lines))
        });
        let picked_index = picked_lines
            .as_ref()
            .map(|(id, objects, lines)| (*id, *objects, lines, ObjectIndex::new(lines)));
        let is_picked = |id: UnderlayId, segment: usize| {
            picked_index
                .as_ref()
                .filter(|(picked, ..)| *picked == id)
                .is_some_and(|(_, objects, lines, index)| {
                    lines
                        .segment_start(segment)
                        .and_then(|(layer, point)| index.object_drawing(layer, point))
                        .is_some_and(|object| objects.contains(&index.top_level_of(object)))
                })
        };
        let mut snaps = Vec::new();
        for height in heights {
            let Some((point, radius)) =
                self.underlay_snap_window(pointer, rect, tolerance_px, height)
            else {
                continue;
            };
            let at_height = |underlay: &Underlay| underlay.plan_height() == Some(height);
            let (found, _) = self.underlay_snaps_near_of(&snapshot, point, radius, at_height);
            snaps.extend(
                found
                    .into_iter()
                    .filter(|(id, snap)| moving != Some(*id) && !is_picked(*id, snap.segment))
                    .map(|(id, snap)| {
                        (
                            underlay_reference(id, snap.segment),
                            snap_kind(snap.kind),
                            Vec3::new(snap.point_mm[0], snap.point_mm[1], height),
                        )
                    }),
            );
        }
        snaps
    }

    /// Per underlay, its shown line or arc nearest the pointer within `tolerance_px`, as
    /// world points, and its length; `edge_at_screen` picks the nearest on screen.
    pub(crate) fn underlay_pieces_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f32,
    ) -> Vec<(Vec<Vec3>, f64)> {
        self.underlay_snaps_at_screen(pointer, rect, tolerance_px)
            .into_iter()
            .filter_map(|(reference, kind, at)| match reference.element {
                ElementId::Underlay { underlay, segment } if kind == SnapKind::Edge => self
                    .underlay_piece(underlay, usize::try_from(segment).ok()?)
                    .map(|piece| (piece, at.z)),
                _ => None,
            })
            .map(|(piece, height)| {
                (
                    piece
                        .points_mm
                        .iter()
                        .map(|point| Vec3::new(point[0], point[1], height))
                        .collect(),
                    piece.length_mm,
                )
            })
            .collect()
    }
}
