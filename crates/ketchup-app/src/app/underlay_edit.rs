//! Editing an underlay's drawing as a 2D CAD does: a click on one of its lines picks the
//! object that drew it (a line, polyline, arc, text, the whole insert of a block, a
//! dimension, a hatch), Shift adds or takes one away, the picked ones are drawn in the
//! accent color, and Delete takes them out of the drawing as one Undo step. The blob of
//! the drawing is not touched: the underlay carries the ids it took out
//! ([`ketchup_model::underlay::edits`]).

use crate::*;
use ketchup_model::underlay::drawn::DrawnShape;
use ketchup_model::underlay::edits::{
    UnderlayObjectError, plan_underlay_drawing, plan_underlay_object_removal,
    plan_underlay_object_transform,
};
use ketchup_model::underlay::object_query::ObjectIndex;
use ketchup_model::underlay::objects::Affine;
use ketchup_model::underlay::{UnderlayId, UnderlayLines, UnderlayPlanError};

/// The most points the picked objects are drawn with in one frame.
const MAX_PICKED_POINTS: usize = 100_000;

/// Why objects of a drawing were not taken out.
#[derive(Debug)]
pub(crate) enum UnderlayEditError {
    /// The document holds no readable lines of the underlay.
    NoLines,
    Plan(UnderlayObjectError),
    Commit(WorkRecoveryMutationError<CanonicalError>),
}

impl std::fmt::Display for UnderlayEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLines => formatter.write_str("the document holds no lines of the underlay"),
            Self::Plan(error) => error.fmt(formatter),
            Self::Commit(error) => write!(formatter, "the change was not kept: {error}"),
        }
    }
}

impl std::error::Error for UnderlayEditError {}

impl KetchupApp {
    /// The top-level object of `underlay` that drew its straight piece `segment`, counted
    /// as the snap index counts them.
    pub(crate) fn underlay_object_at(&self, underlay: UnderlayId, segment: u32) -> Option<u32> {
        let snapshot = self.document.current();
        let lines = self.cached_underlay_lines(snapshot.underlay(underlay)?)?;
        let (layer, point) = lines.segment_start(usize::try_from(segment).ok()?)?;
        let index = ObjectIndex::new(&lines);
        Some(index.top_level_of(index.object_drawing(layer, point)?))
    }

    /// Selects `underlay` and picks the object that drew its piece `segment`; `additive`
    /// adds it to the objects picked in the same underlay or takes it away.
    pub(crate) fn pick_underlay_object(
        &mut self,
        underlay: UnderlayId,
        segment: u32,
        additive: bool,
    ) {
        let keep = additive && self.selected_underlay() == Some(underlay);
        if !keep {
            self.select_underlay(underlay);
        }
        let Some(object) = self.underlay_object_at(underlay, segment) else {
            return;
        };
        self.selection.pick_underlay_object(object, keep);
        self.digest = self.picked_underlay_objects_readout(underlay);
    }

    /// What the status line says of the objects picked in `underlay`.
    fn picked_underlay_objects_readout(&self, underlay: UnderlayId) -> String {
        let picked = self.selection.underlay_objects();
        let snapshot = self.document.current();
        let lines = snapshot
            .underlay(underlay)
            .and_then(|underlay| self.cached_underlay_lines(underlay));
        let described = match (picked.len(), picked.first(), lines.as_deref()) {
            (1, Some(object), Some(lines)) => describe_object(lines, *object),
            _ => None,
        };
        match described {
            Some((kind, layer)) => self.catalog.format(
                "digest-underlay-object-picked",
                &BTreeMap::from([("kind", kind), ("layer", layer)]),
            ),
            None => self.catalog.format(
                "digest-underlay-objects-picked",
                &BTreeMap::from([("count", picked.len().to_string())]),
            ),
        }
    }

    /// The selected underlay and the objects picked in it, while there are any.
    pub(crate) fn picked_underlay_objects(&self) -> Option<(UnderlayId, Vec<u32>)> {
        let id = self.selected_underlay()?;
        let picked = self.selection.underlay_objects();
        (!picked.is_empty()).then(|| (id, picked.iter().copied().collect()))
    }

    /// Takes the objects `ids` out of the drawing of `underlay`, as one Undo step.
    pub(crate) fn remove_underlay_objects(
        &mut self,
        underlay: UnderlayId,
        ids: &[u32],
    ) -> Result<(), UnderlayEditError> {
        self.edit_underlay_drawing(underlay, |snapshot, lines| {
            plan_underlay_object_removal(snapshot, underlay, lines, ids)
        })
    }

    /// Draws `shapes`, in the millimetres of its lines, on layer `layer` of the drawing of
    /// `underlay`, as one Undo step.
    pub(crate) fn draw_into_underlay(
        &mut self,
        underlay: UnderlayId,
        layer: &str,
        shapes: Vec<DrawnShape>,
    ) -> Result<(), UnderlayEditError> {
        self.edit_underlay_drawing(underlay, |snapshot, _| {
            plan_underlay_drawing(snapshot, underlay, layer, shapes)
        })
    }

    /// Moves the objects `ids` of the drawing of `underlay` by `map`, in the millimetres of
    /// its lines, or adds copies of them so moved when `copy`, as one Undo step.
    pub(crate) fn transform_underlay_objects(
        &mut self,
        underlay: UnderlayId,
        ids: &[u32],
        map: &Affine,
        copy: bool,
    ) -> Result<(), UnderlayEditError> {
        self.edit_underlay_drawing(underlay, |snapshot, lines| {
            plan_underlay_object_transform(snapshot, underlay, lines, ids, map, copy)
        })
    }

    /// Commits the batch `plan` makes of the document and the lines of `underlay` as drawn
    /// now.
    fn edit_underlay_drawing(
        &mut self,
        underlay: UnderlayId,
        plan: impl FnOnce(&Snapshot, &UnderlayLines) -> Result<CommandBatch, UnderlayObjectError>,
    ) -> Result<(), UnderlayEditError> {
        let snapshot = self.document.current();
        let found = snapshot.underlay(underlay).ok_or(UnderlayEditError::Plan(
            UnderlayObjectError::Plan(UnderlayPlanError::NotFound(underlay)),
        ))?;
        let lines = self
            .cached_underlay_lines(found)
            .ok_or(UnderlayEditError::NoLines)?;
        let batch = plan(&snapshot, &lines).map_err(UnderlayEditError::Plan)?;
        self.apply_batch_with_work_recovery(&batch)
            .map_err(UnderlayEditError::Commit)
    }

    /// Delete on objects picked in a drawing: takes them out, one Undo step, and keeps the
    /// drawing selected.
    pub(crate) fn delete_picked_underlay_objects(&mut self) -> bool {
        let Some((underlay, ids)) = self.picked_underlay_objects() else {
            return false;
        };
        match self.remove_underlay_objects(underlay, &ids) {
            Ok(()) => {
                self.select_underlay(underlay);
                self.digest = self.catalog.format(
                    "digest-underlay-objects-removed",
                    &BTreeMap::from([("count", ids.len().to_string())]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "digest-underlay-objects-not-removed",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    /// Draws the lines of the picked objects, and of all they placed, in the accent color
    /// over the model.
    pub(crate) fn paint_picked_underlay_objects(&self, painter: &egui::Painter, rect: Rect) {
        let Some((id, picked)) = self.picked_underlay_objects() else {
            return;
        };
        let snapshot = self.document.current();
        let Some(underlay) = snapshot.underlay(id) else {
            return;
        };
        let Some(lines) = self.cached_underlay_lines(underlay) else {
            return;
        };
        let index = ObjectIndex::new(&lines);
        // A live move or turn draws them where it takes them.
        let placement = self
            .picked_objects_drag_edit()
            .map_or(underlay.placement, |edit| edit.compose(underlay.placement));
        let stroke = Stroke::new(2.5_f32, self.palette().accent);
        let painter = painter.with_clip_rect(rect);
        let mut budget = MAX_PICKED_POINTS;
        let mut objects = picked;
        while let Some(object) = objects.pop() {
            objects.extend_from_slice(index.children(object));
            for run in index.own_polylines(object) {
                if run.len() > budget {
                    return;
                }
                budget -= run.len();
                let points = run
                    .iter()
                    .map(|[x, y]| {
                        let world = transform_model_point(placement, Vec3::new(*x, *y, 0.0));
                        self.project(world, rect)
                    })
                    .collect::<Vec<_>>();
                painter.add(egui::Shape::line(points, stroke));
            }
        }
    }
}

/// The kind and layer of object `id` of `lines`.
fn describe_object(lines: &UnderlayLines, id: u32) -> Option<(String, String)> {
    let object = lines
        .objects()
        .objects
        .get(usize::try_from(id).ok()?)
        .filter(|object| !object.removed)?;
    Some((
        object.kind.name().to_owned(),
        lines.objects().layer_name(object).to_owned(),
    ))
}
