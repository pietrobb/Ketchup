//! Threads of a document: the threads tapped into holes, where they run, and
//! which threads are modelled as helical geometry rather than drawn cosmetic
//! on their plain cylinder.
use super::*;
use ketchup_geometry::linalg;
use ketchup_geometry::sketch::{CutStart, PadSpec};
use ketchup_geometry::tapping::{TappedHole, TappedHoleMismatch, metric_coarse_tapping};

/// One thread on a cylinder of a definition, in its frame.
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
    /// Modelled as helical geometry, not only drawn on the cylinder.
    pub modeled: bool,
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

/// The nominal diameter an ISO metric `size` ("M8") names.
#[must_use]
pub fn metric_nominal_diameter(size: &str) -> Option<f64> {
    size.strip_prefix('M')?
        .parse::<f64>()
        .ok()
        .filter(|diameter| *diameter > 0.0)
}

impl Snapshot {
    /// Whether the document models threads that do not say themselves.
    #[must_use]
    pub fn modeled_threads(&self) -> bool {
        self.product.modeled_threads
    }

    /// The modelled outside threads, by the feature whose body they are cut into.
    pub fn shaft_threads(&self) -> impl Iterator<Item = (FeatureId, &ShaftThread)> {
        self.product
            .shaft_threads
            .iter()
            .map(|(id, thread)| (*id, thread))
    }

    /// The features whose body changes because a thread turned modelled or
    /// cosmetic, or a modelled one changed, between `before` and this.
    #[must_use]
    pub fn thread_changed_features_since(&self, before: &Self) -> BTreeSet<FeatureId> {
        thread_changed_features(&before.product, &self.product)
    }

    /// The modelled outside thread cut into the body feature `id` makes.
    #[must_use]
    pub fn shaft_thread(&self, id: FeatureId) -> Option<&ShaftThread> {
        self.product.shaft_threads.get(&id)
    }

    /// Whether the thread tapped into the hole `id` cuts is modelled: its own
    /// choice, else the document's.
    #[must_use]
    pub fn hole_thread_is_modeled(&self, id: FeatureId) -> bool {
        self.product
            .hole_threads
            .get(&id)
            .is_some_and(|thread| thread.modeled.unwrap_or(self.product.modeled_threads))
    }

    /// The tapped hole `id` cuts: its circle swept from the profile plane. A
    /// cut measured from the target face, or a hole that is not one circle,
    /// has no place for its thread here.
    #[must_use]
    pub fn tapped_hole(&self, id: FeatureId) -> Option<ThreadCylinder> {
        let thread = self.product.hole_threads.get(&id)?;
        let feature = self.feature(id)?;
        let (pad, placement) = cut_pad(&self.product, id)?;
        if matches!(
            pad.operation,
            PadOperation::Cut {
                start: CutStart::TargetFace,
                ..
            }
        ) {
            return None;
        }
        let (frame, center, radius) = profile_circle(&self.product, pad.profile.feature_id())?;
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
            nominal_diameter_mm: metric_nominal_diameter(&thread.size)?,
            surface_diameter_mm: 2.0 * radius,
            start_mm: placement.transform_point(start),
            direction,
            length_mm: thread.depth_mm,
            internal: true,
            modeled: self.hole_thread_is_modeled(id),
        })
    }
}

/// The features whose body a change of thread mode rebuilds between
/// `before` and `after`: the holes whose modelled thread differs, and the
/// features a modelled outside thread is recorded on, added or removed.
pub(crate) fn thread_changed_features(
    before: &ProductModel,
    after: &ProductModel,
) -> BTreeSet<FeatureId> {
    let modeled = |product: &ProductModel| {
        product
            .hole_threads
            .iter()
            .filter(|(_, thread)| thread.modeled.unwrap_or(product.modeled_threads))
            .map(|(id, thread)| (*id, thread.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let (holes_before, holes_after) = (modeled(before), modeled(after));
    let mut changed = holes_before
        .keys()
        .chain(holes_after.keys())
        .filter(|id| holes_before.get(id) != holes_after.get(id))
        .copied()
        .collect::<BTreeSet<_>>();
    changed.extend(
        before
            .shaft_threads
            .keys()
            .chain(after.shaft_threads.keys())
            .filter(|id| before.shaft_threads.get(id) != after.shaft_threads.get(id)),
    );
    changed
}

/// Applies a thread command to `product`.
pub(super) fn apply(
    product: &mut ProductModel,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::SetHoleThread { id, thread } => {
            if !product.features.contains_key(id) {
                return Err(CanonicalError::FeatureNotFound(*id));
            }
            match thread {
                Some(thread) if !thread.is_valid() => {
                    return Err(CanonicalError::InvalidHoleThread(*id));
                }
                Some(thread) => {
                    if let Some(mismatch) = hole_thread_mismatch(product, *id, thread) {
                        return Err(CanonicalError::HoleThreadMismatch { id: *id, mismatch });
                    }
                    product.hole_threads.insert(*id, thread.clone())
                }
                None => product.hole_threads.remove(id),
            };
        }
        CanonicalCommand::SetModeledThreads { modeled } => product.modeled_threads = *modeled,
        CanonicalCommand::SetShaftThread { id, thread } => {
            if !product.features.contains_key(id) {
                return Err(CanonicalError::FeatureNotFound(*id));
            }
            match thread {
                Some(thread) if !thread.is_valid() => {
                    return Err(CanonicalError::InvalidShaftThread(*id));
                }
                Some(thread) => product.shaft_threads.insert(*id, thread.clone()),
                None => product.shaft_threads.remove(id),
            };
        }
        _ => {}
    }
    Ok(())
}

/// The features a thread command describes: its own, or for the document's
/// mode every tapped hole.
fn described_features(snapshot: &Snapshot, command: &CanonicalCommand) -> Vec<FeatureId> {
    match command {
        CanonicalCommand::SetHoleThread { id, .. }
        | CanonicalCommand::SetShaftThread { id, .. } => {
            vec![*id]
        }
        CanonicalCommand::SetModeledThreads { .. } => {
            snapshot.product.hole_threads.keys().copied().collect()
        }
        _ => Vec::new(),
    }
}

/// The features a thread command writes.
pub(super) fn add_writes(
    snapshot: &Snapshot,
    command: &CanonicalCommand,
    writes: &mut BTreeSet<AuthoritativeDependency>,
) {
    writes.extend(
        described_features(snapshot, command)
            .into_iter()
            .map(AuthoritativeDependency::Feature),
    );
}

/// What a thread command reads: the features it describes and theirs.
pub(super) fn add_dependencies(
    snapshot: &Snapshot,
    command: &CanonicalCommand,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    for id in described_features(snapshot, command) {
        add_feature_dependency_closure(snapshot, id, dependencies);
    }
}

fn feature(product: &ProductModel, id: FeatureId) -> Option<&Feature> {
    product.features.get(&id).map(Arc::as_ref)
}

/// The pad that cuts the hole `id`: the cut itself, or a cut Boolean's tool
/// through the rigid placements on it.
fn cut_pad(product: &ProductModel, id: FeatureId) -> Option<(&PadSpec, Transform)> {
    match feature(product, id)?.kind() {
        FeatureKind::Pad(pad) if matches!(pad.operation, PadOperation::Cut { .. }) => {
            Some((pad, Transform::identity()))
        }
        FeatureKind::Boolean {
            operation: BooleanOperation::Cut,
            tool,
            ..
        } => placed_pad(product, *tool),
        _ => None,
    }
}

/// Why the hole `id` cuts does not take `thread`, by the rule every export
/// checks it by again ([`TappedHole::check`]), as far as the document tells
/// without the exact body: the size and its coarse pitch, the tap drill when
/// the hole is one circle, and a thread no deeper than a blind cut runs.
/// Whether that cut ends inside the part, so the tap needs its runout below
/// the thread, only the exact body tells; the export checks that.
pub(crate) fn hole_thread_mismatch(
    product: &ProductModel,
    id: FeatureId,
    thread: &HoleThread,
) -> Option<TappedHoleMismatch> {
    let tolerance_mm = product.tolerance.linear_mm();
    let Some(tapping) = metric_coarse_tapping(&thread.size) else {
        return Some(TappedHoleMismatch::UnknownSize);
    };
    let pad = cut_pad(product, id).map(|(pad, _)| pad);
    // A hole that is not one circle has its size in the exact body only, so
    // it is taken here as the tap drill and left to the export.
    let hole_diameter_mm = pad
        .and_then(|pad| profile_circle(product, pad.profile.feature_id()))
        .map_or(tapping.tap_drill_mm, |(_, _, radius)| 2.0 * radius);
    let hole_depth_mm = match pad.map(|pad| &pad.extent) {
        Some(FeatureExtent::Blind(depth)) => depth.millimetres(),
        _ => f64::INFINITY,
    };
    TappedHole {
        size: &thread.size,
        pitch_mm: thread.pitch_mm,
        thread_depth_mm: thread.depth_mm,
        hole_diameter_mm,
        hole_depth_mm,
        through: true,
    }
    .check(tolerance_mm)
    .err()
}

/// The pad a cut's tool is, through the rigid placements on it.
fn placed_pad(product: &ProductModel, mut id: FeatureId) -> Option<(&PadSpec, Transform)> {
    let mut placement = Transform::identity();
    loop {
        match feature(product, id)?.kind() {
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
fn profile_circle(
    product: &ProductModel,
    id: FeatureId,
) -> Option<(WorkplaneFrame, [f64; 2], f64)> {
    let arcs: Vec<([f64; 2], [f64; 2])> = match feature(product, id)?.kind() {
        FeatureKind::Sketch(sketch) => {
            let FeatureKind::Workplane(workplane) = feature(product, sketch.workplane)?.kind()
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
