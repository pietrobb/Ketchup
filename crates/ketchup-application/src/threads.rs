//! Where the threads of a document run, as cylinders in their definition's
//! frame: every tapped hole the document records, and the outside thread of
//! every program part that names one (`thread`, `thread_from_mm`,
//! `thread_to_mm` along its local z). Geometry stays a plain cylinder; these
//! say where a view draws the thread on it.

use ketchup_geometry::linalg;
use ketchup_geometry::sketch::{CutStart, PadOperation, PadSpec, SketchEntity, WorkplaneFrame};
use ketchup_model::document::{
    BooleanOperation, DefinitionId, FeatureId, FeatureKind, HoleThread, ProfileSegment, Snapshot,
    Transform,
};
use ketchup_model::tolerance::ROUNDING;
use std::collections::BTreeSet;

/// One thread on a cylinder of a definition.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadCylinder {
    pub definition_id: DefinitionId,
    /// The ISO metric size, e.g. "M8".
    pub size: String,
    pub pitch_mm: f64,
    /// The nominal (major) diameter of the thread.
    pub nominal_diameter_mm: f64,
    /// The diameter of the modelled cylinder: a tapped hole's bore, a
    /// fastener's nominal diameter.
    pub surface_diameter_mm: f64,
    /// Where the thread starts, on its axis, in the definition's frame.
    pub start_mm: [f64; 3],
    /// The unit direction the thread runs in from `start_mm`.
    pub direction: [f64; 3],
    pub length_mm: f64,
    /// A thread inside a hole rather than on the outside of a shaft.
    pub internal: bool,
}

/// Not a tolerance: ISO 724's minor diameter of a shaft's thread is
/// d3 = d − 1.226869·P.
const SHAFT_ROOT_DEPTH_PER_PITCH: f64 = 1.226_869;

impl ThreadCylinder {
    /// The diameter at the thread's root: the major diameter in a hole, the
    /// minor diameter on a shaft.
    #[must_use]
    pub fn root_diameter_mm(&self) -> f64 {
        if self.internal {
            self.nominal_diameter_mm
        } else {
            self.nominal_diameter_mm - SHAFT_ROOT_DEPTH_PER_PITCH * self.pitch_mm
        }
    }
}

/// Every thread of `snapshot`; the outside threads of program parts only
/// with the program's `evaluated` model.
#[must_use]
pub fn thread_cylinders(
    snapshot: &Snapshot,
    evaluated: Option<&ketchup_program::Evaluated>,
) -> Vec<ThreadCylinder> {
    let mut threads = snapshot
        .hole_threads()
        .filter_map(|(id, thread)| tapped_hole(snapshot, id, thread))
        .collect::<Vec<_>>();
    let Some(evaluated) = evaluated else {
        return threads;
    };
    let parts = evaluated.model.parts_by_name();
    let mut seen = BTreeSet::new();
    for occurrence in snapshot.scene_query() {
        if !seen.insert(occurrence.definition_id) {
            continue;
        }
        let Some(part) = crate::rule_program_part_name(snapshot, &occurrence.instance_path)
            .and_then(|name| parts.get(name.as_str()).copied())
        else {
            continue;
        };
        threads.extend(outside_thread(occurrence.definition_id, &part.attributes));
    }
    threads
}

/// The outside thread `attributes` name along local z: size and span.
fn outside_thread(
    definition_id: DefinitionId,
    attributes: &std::collections::BTreeMap<String, String>,
) -> Option<ThreadCylinder> {
    let size = attributes.get("thread")?;
    let bound = |key: &str| attributes.get(key)?.parse::<f64>().ok();
    let (from, to) = (bound("thread_from_mm")?, bound("thread_to_mm")?);
    let pitch_mm = ketchup_program::eval::library_metric_threads()
        .ok()?
        .get(size)?
        .pitch_mm;
    let nominal_diameter_mm = nominal_diameter(size)?;
    (to - from > ROUNDING).then(|| ThreadCylinder {
        definition_id,
        size: size.clone(),
        pitch_mm,
        nominal_diameter_mm,
        surface_diameter_mm: nominal_diameter_mm,
        start_mm: [0.0, 0.0, from],
        direction: [0.0, 0.0, 1.0],
        length_mm: to - from,
        internal: false,
    })
}

fn nominal_diameter(size: &str) -> Option<f64> {
    size.strip_prefix('M')?
        .parse::<f64>()
        .ok()
        .filter(|diameter| *diameter > 0.0)
}

/// The tapped hole `id` cuts: its circle swept from the profile plane. A cut
/// measured from the target face, or a hole that is not one circle, has no
/// place for its thread here.
fn tapped_hole(snapshot: &Snapshot, id: FeatureId, thread: &HoleThread) -> Option<ThreadCylinder> {
    let feature = snapshot.feature(id)?;
    let (pad, placement) = match feature.kind() {
        FeatureKind::Pad(pad) if matches!(pad.operation, PadOperation::Cut { .. }) => {
            (pad, Transform::identity())
        }
        FeatureKind::Boolean {
            operation: BooleanOperation::Cut,
            tool,
            ..
        } => placed_pad(snapshot, *tool)?,
        _ => return None,
    };
    if matches!(
        pad.operation,
        PadOperation::Cut {
            start: CutStart::TargetFace,
            ..
        }
    ) {
        return None;
    }
    let (frame, center, radius) = profile_circle(snapshot, pad.profile.feature_id())?;
    let start = linalg::add(
        frame.origin_mm,
        linalg::add(
            linalg::scale(frame.x_axis, center[0]),
            linalg::scale(frame.y_axis, center[1]),
        ),
    );
    let direction =
        linalg::normalize(placement.transform_vector(pad.direction.vector(frame.normal)?))?;
    Some(ThreadCylinder {
        definition_id: feature.definition_id(),
        size: thread.size.clone(),
        pitch_mm: thread.pitch_mm,
        nominal_diameter_mm: nominal_diameter(&thread.size)?,
        surface_diameter_mm: 2.0 * radius,
        start_mm: placement.transform_point(start),
        direction,
        length_mm: thread.depth_mm,
        internal: true,
    })
}

/// The pad a cut's tool is, through the rigid placements on it.
fn placed_pad(snapshot: &Snapshot, mut id: FeatureId) -> Option<(&PadSpec, Transform)> {
    let mut placement = Transform::identity();
    loop {
        match snapshot.feature(id)?.kind() {
            FeatureKind::Pad(pad) => return Some((pad, placement)),
            FeatureKind::RigidTransform { target, transform } => {
                placement = placement.compose(*transform);
                id = *target;
            }
            _ => return None,
        }
    }
}

/// The frame, centre and radius of a profile that is one circle: a sketch on
/// its workplane, or a plain profile in the definition's XY plane.
fn profile_circle(snapshot: &Snapshot, id: FeatureId) -> Option<(WorkplaneFrame, [f64; 2], f64)> {
    let arcs: Vec<([f64; 2], [f64; 2])> = match snapshot.feature(id)?.kind() {
        FeatureKind::Sketch(sketch) => {
            let FeatureKind::Workplane(workplane) = snapshot.feature(sketch.workplane)?.kind()
            else {
                return None;
            };
            let arcs = sketch
                .entities
                .iter()
                .map(|entity| match entity {
                    SketchEntity::Circle {
                        center_mm,
                        radius_mm,
                        ..
                    } => Some((*center_mm, [center_mm[0] + radius_mm, center_mm[1]])),
                    SketchEntity::Arc {
                        center_mm,
                        start_mm,
                        ..
                    } => Some((*center_mm, *start_mm)),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            return one_circle(&arcs).map(|(center, radius)| (workplane.frame, center, radius));
        }
        FeatureKind::Profile {
            segments,
            closed: true,
        } => segments
            .iter()
            .map(|segment| match segment {
                ProfileSegment::CircularArc {
                    center_mm,
                    start_mm,
                    ..
                } => Some((*center_mm, *start_mm)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    let frame = WorkplaneFrame::principal(ketchup_geometry::sketch::PrincipalPlane::Xy);
    one_circle(&arcs).map(|(center, radius)| (frame, center, radius))
}

/// The centre and radius shared by every (centre, point on it) arc.
fn one_circle(arcs: &[([f64; 2], [f64; 2])]) -> Option<([f64; 2], f64)> {
    let (center, point) = *arcs.first()?;
    let radius = (point[0] - center[0]).hypot(point[1] - center[1]);
    arcs.iter()
        .all(|(other, point)| {
            (other[0] - center[0]).hypot(other[1] - center[1]) <= ROUNDING
                && ((point[0] - other[0]).hypot(point[1] - other[1]) - radius).abs() <= ROUNDING
        })
        .then_some((center, radius))
        .filter(|(_, radius)| *radius > ROUNDING)
}
