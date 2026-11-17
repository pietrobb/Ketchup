//! A hole read from the finished exact faces about a cut's axis: the coaxial
//! cylinders, cones and flat rings that bound it, where it opens and how it
//! ends. Only the body is measured; which cut or chamfer made a face does not
//! matter, so a counterbore of two cuts or a chamfered entry reads as one hole.
use crate::exact_product::{ExactBRepGraphEdgeEvidence, ExactBRepGraphFaceEvidence};
use ketchup_geometry::linalg::{dot, scale, sub};
use std::collections::{BTreeMap, BTreeSet};

/// How the bore ends below its entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum HoleEnd {
    /// It opens again on another face of the body.
    Through,
    FlatBottom,
    /// A drill point of this included angle, in degrees.
    Point(f64),
    /// The evidence names no face beyond its last rim.
    Unknown,
}

/// One cylinder of the hole, its depths from the cutter's start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bore {
    pub diameter: f64,
    pub from: f64,
    pub to: f64,
    /// The included angle of the cone leading down from it to the next
    /// cylinder; `None` for a flat shoulder or the last cylinder.
    pub step_angle: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct HoleProfile {
    /// Every face bounding the hole; the first identifies it on the body.
    pub faces: BTreeSet<u32>,
    /// Every coaxial circular edge of the hole.
    pub rims: BTreeSet<u32>,
    /// Depth of the entry from the cutter's start.
    pub entry: f64,
    /// An entry cone: its diameter at the entry and its included angle.
    pub countersink: Option<(f64, f64)>,
    /// The cylinders from the entry down; the last is the bore.
    pub bores: Vec<Bore>,
    pub end: HoleEnd,
}

/// Why the body about a cut's axis is no hole a callout can name.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum HoleUnsupported {
    BrokenWall,
    NoBore,
    OpensNowhere,
    OpensMoreThanTwice,
    UnnamedCone,
    UnnamedRing,
    NoEnd,
    NotOneRing,
    Widens,
    TurnsBack,
    NotOneDiameter,
    NotOneBore,
    TwoThreads,
    ThreadNotCircle,
    ModeledThread,
    NoThreadBore,
    ThreadStartsElsewhere,
    /// The thread does not suit the bore it is tapped into.
    Thread(ketchup_geometry::tapping::TappedHoleMismatch),
}

impl std::fmt::Display for HoleUnsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::BrokenWall => "Another cut or face breaks the wall of the hole",
            Self::NoBore => "The finished body has no complete bore of the cut's diameter about its axis",
            Self::OpensNowhere => "The bore opens nowhere on the body",
            Self::OpensMoreThanTwice => "The hole opens in more than two places",
            Self::UnnamedCone => "The hole has cones the callout cannot name",
            Self::UnnamedRing => "The hole has a flat ring the callout cannot name",
            Self::NoEnd => "The hole neither runs through nor ends in a flat bottom or a drill point",
            Self::NotOneRing => "A face of the hole is not one ring of the bore",
            Self::Widens => "The bore widens below its entry (an undercut or a groove); use a reviewed manual note",
            Self::TurnsBack => "A face of the hole turns back towards its entry",
            Self::NotOneDiameter => "A cylinder of the hole is not one diameter",
            Self::NotOneBore => "The hole's faces do not form one bore from its entry",
            Self::TwoThreads => "More than one thread is tapped into this hole",
            Self::ThreadNotCircle => "The thread's hole is not one circle cut from its profile plane",
            Self::ModeledThread => "The thread is modelled; make it cosmetic for its callout",
            Self::NoThreadBore => "The hole has no bore of the thread's diameter",
            Self::ThreadStartsElsewhere => "The thread does not start where its bore does",
            Self::Thread(mismatch) => return mismatch.fmt(formatter),
        })
    }
}

impl std::error::Error for HoleUnsupported {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Thread(mismatch) => Some(mismatch),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Cylinder,
    Cone,
    Plane,
}

/// One face along the hole's section: (depth, radius) at each end.
type Segment = (Shape, [f64; 2], [f64; 2]);

/// The hole whose cylinder of `diameter` lies about the axis from `start`
/// along `direction`, or why the body there is no hole a callout can name.
pub(super) fn hole_profile(
    start: [f64; 3],
    direction: [f64; 3],
    diameter: f64,
    edges: &[ExactBRepGraphEdgeEvidence],
    faces: &[ExactBRepGraphFaceEvidence],
) -> Result<HoleProfile, HoleUnsupported> {
    let tolerance = ketchup_tolerance::APPROXIMATION;
    let on_axis = |origin: [f64; 3]| {
        let displacement = sub(origin, start);
        let depth = dot(displacement, direction);
        let radial = sub(displacement, scale(direction, depth));
        (dot(radial, radial) < tolerance * tolerance).then_some(depth)
    };
    let parallel = |axis: [f64; 3]| (dot(axis, direction).abs() - 1.0).abs() < tolerance;
    let rim = |edge: &ExactBRepGraphEdgeEvidence| {
        let radius = edge.circle_radius_mm?;
        let depth = on_axis(edge.axis_origin_mm?)?;
        (edge.closed && edge.curve_kind == "circle" && parallel(edge.unit_axis_direction?))
            .then_some((depth, radius))
    };
    let apex = |edge: &ExactBRepGraphEdgeEvidence| {
        (edge.curve_kind == "degenerate")
            .then(|| on_axis(edge.centroid_mm))
            .flatten()
    };
    let by_ordinal = faces
        .iter()
        .map(|face| (face.face_ordinal, face))
        .collect::<BTreeMap<_, _>>();
    let bounding = |ordinal: u32| {
        edges
            .iter()
            .filter(move |edge| edge.adjacent_face_ordinals.contains(&ordinal))
    };
    // A face about the axis bounded only by coaxial rims, its seam and a
    // cone's apex; `Err` when such a cylinder or cone has other edges too.
    let hole_face = |ordinal: u32| -> Result<Option<Shape>, HoleUnsupported> {
        let Some(face) = by_ordinal.get(&ordinal) else {
            return Ok(None);
        };
        let shape = match face.surface_kind.as_str() {
            "cylinder"
                if face.unit_axis_direction.is_some_and(parallel)
                    && face.axis_origin_mm.and_then(on_axis).is_some() =>
            {
                Shape::Cylinder
            }
            "cone"
                if face
                    .cone
                    .is_some_and(|cone| parallel(cone.unit_axis_direction)) =>
            {
                Shape::Cone
            }
            "plane" if parallel(face.unit_normal) => Shape::Plane,
            _ => return Ok(None),
        };
        let mut rims = 0;
        for edge in bounding(ordinal) {
            if rim(edge).is_some() {
                rims += 1;
            } else if !(edge.curve_kind == "line"
                && edge.adjacent_face_ordinals.iter().all(|id| *id == ordinal)
                || shape == Shape::Cone && apex(edge).is_some())
            {
                return match shape {
                    // The face the hole opens on.
                    Shape::Plane => Ok(None),
                    _ => Err(HoleUnsupported::BrokenWall),
                };
            }
        }
        Ok((rims > 0).then_some(shape))
    };

    let seed = faces
        .iter()
        .find(|face| {
            face.surface_kind == "cylinder"
                && bounding(face.face_ordinal).any(|edge| {
                    rim(edge).is_some_and(|(_, radius)| (radius * 2.0 - diameter).abs() < tolerance)
                })
                && matches!(hole_face(face.face_ordinal), Ok(Some(Shape::Cylinder)))
        })
        .ok_or(HoleUnsupported::NoBore)?;
    let mut shapes = BTreeMap::from([(seed.face_ordinal, Shape::Cylinder)]);
    let mut queue = vec![seed.face_ordinal];
    let mut rims = BTreeSet::new();
    // Rims the hole opens at: (depth, radius, whether a face lies beyond).
    let mut openings = Vec::new();
    while let Some(ordinal) = queue.pop() {
        for edge in bounding(ordinal) {
            let Some((depth, radius)) = rim(edge) else {
                continue;
            };
            rims.insert(edge.edge_ordinal);
            let beyond = edge
                .adjacent_face_ordinals
                .iter()
                .copied()
                .filter(|id| *id != ordinal)
                .collect::<Vec<_>>();
            let mut inside = false;
            for &other in &beyond {
                if let Some(shape) = hole_face(other)? {
                    inside = true;
                    if shapes.insert(other, shape).is_none() {
                        queue.push(other);
                    }
                }
            }
            if !inside
                && !openings
                    .iter()
                    .any(|(_, _, _, id)| *id == edge.edge_ordinal)
            {
                openings.push((depth, radius, !beyond.is_empty(), edge.edge_ordinal));
            }
        }
    }
    openings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (entry, entry_radius) = match openings.as_slice() {
        [(depth, radius, ..)] | [(depth, radius, ..), _] => (*depth, *radius),
        [] => return Err(HoleUnsupported::OpensNowhere),
        _ => return Err(HoleUnsupported::OpensMoreThanTwice),
    };

    let segments = section(&shapes, bounding, rim, apex, (entry, entry_radius))?;
    let mut profile = HoleProfile {
        faces: shapes.keys().copied().collect(),
        rims,
        entry,
        countersink: None,
        bores: Vec::new(),
        end: match openings.as_slice() {
            [_, (.., true, _)] => HoleEnd::Through,
            [_, _] => HoleEnd::Unknown,
            _ => HoleEnd::FlatBottom,
        },
    };
    let last = segments.len() - 1;
    for (index, &(shape, [d0, r0], [d1, r1])) in segments.iter().enumerate() {
        let angle = || 2.0 * ((r0 - r1) / (d1 - d0)).atan().to_degrees();
        match shape {
            // Two cuts of one diameter can leave the bore as two faces.
            Shape::Cylinder
                if profile.bores.last().is_some_and(|bore| {
                    bore.step_angle.is_none()
                        && (bore.diameter - 2.0 * r0).abs() < ketchup_tolerance::APPROXIMATION
                }) =>
            {
                if let Some(bore) = profile.bores.last_mut() {
                    bore.to = d1;
                }
            }
            Shape::Cylinder => profile.bores.push(Bore {
                diameter: 2.0 * r0,
                from: d0,
                to: d1,
                step_angle: None,
            }),
            Shape::Cone if index == 0 => profile.countersink = Some((2.0 * r0, angle())),
            Shape::Cone if index == last && r1 == 0.0 && openings.len() == 1 => {
                profile.end = HoleEnd::Point(angle());
            }
            Shape::Cone => match profile.bores.last_mut() {
                Some(bore) if bore.step_angle.is_none() && index < last => {
                    bore.step_angle = Some(angle());
                }
                _ => return Err(HoleUnsupported::UnnamedCone),
            },
            Shape::Plane if r1 == 0.0 && index == last && openings.len() == 1 => {}
            Shape::Plane if index > 0 && index < last => {}
            Shape::Plane => return Err(HoleUnsupported::UnnamedRing),
        }
    }
    let bottomed =
        matches!(segments[last].0, Shape::Plane | Shape::Cone) && segments[last].2[1] == 0.0;
    if profile.bores.is_empty() || bottomed != (openings.len() == 1) {
        return Err(HoleUnsupported::NoEnd);
    }
    Ok(profile)
}

/// The hole's faces in order from the entry rim, each from the depth and
/// radius it starts at to where it ends: a bore that only narrows.
fn section<'a, B, I>(
    shapes: &BTreeMap<u32, Shape>,
    bounding: B,
    rim: impl Fn(&ExactBRepGraphEdgeEvidence) -> Option<(f64, f64)>,
    apex: impl Fn(&ExactBRepGraphEdgeEvidence) -> Option<f64>,
    entry: (f64, f64),
) -> Result<Vec<Segment>, HoleUnsupported>
where
    B: Fn(u32) -> I,
    I: Iterator<Item = &'a ExactBRepGraphEdgeEvidence>,
{
    let tolerance = ketchup_tolerance::APPROXIMATION;
    let same = |a: (f64, f64), b: (f64, f64)| {
        (a.0 - b.0).abs() < tolerance && (a.1 - b.1).abs() < tolerance
    };
    let mut segments = Vec::new();
    let mut visited = BTreeSet::new();
    let mut at = entry;
    loop {
        let next = shapes.iter().find_map(|(&ordinal, &shape)| {
            if visited.contains(&ordinal) {
                return None;
            }
            let ends = bounding(ordinal)
                .filter_map(|edge| rim(edge).or_else(|| apex(edge).map(|depth| (depth, 0.0))))
                .collect::<Vec<_>>();
            let start = ends.iter().position(|end| same(*end, at))?;
            Some((ordinal, shape, ends, start))
        });
        let Some((ordinal, shape, ends, start)) = next else {
            break;
        };
        visited.insert(ordinal);
        let to = match (shape, ends.as_slice()) {
            // A flat bottom: a disc bounded by its one rim.
            (Shape::Plane, [_]) => (at.0, 0.0),
            (_, [a, b]) => {
                if start == 0 {
                    *b
                } else {
                    *a
                }
            }
            _ => return Err(HoleUnsupported::NotOneRing),
        };
        if to.1 > at.1 + tolerance {
            return Err(HoleUnsupported::Widens);
        }
        if to.0 < at.0 - tolerance {
            return Err(HoleUnsupported::TurnsBack);
        }
        if shape == Shape::Cylinder && (to.1 - at.1).abs() >= tolerance {
            return Err(HoleUnsupported::NotOneDiameter);
        }
        segments.push((shape, [at.0, at.1], [to.0, to.1]));
        at = to;
        if to.1 == 0.0 {
            break;
        }
    }
    if visited.len() != shapes.len() || segments.is_empty() {
        return Err(HoleUnsupported::NotOneBore);
    }
    Ok(segments)
}
