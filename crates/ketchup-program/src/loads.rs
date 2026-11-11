//! Loads on the members of a load path (`material_weight()`, `weight_scope()`,
//! `area_load()`). Every part passes its own weight, the area loads on it and
//! what it carries down to the parts it rests on: a member (a beam, a stud) to
//! what it rests on or hangs from through bearing joints, along its longest
//! axis as a continuous member over its supports with free overhangs (an
//! upward reaction is reported, not passed down); any other part (a deck, insulation, a
//! ceiling board) shares its load among the parts it rests on, or failing that
//! hangs from or leans against, by the area of it nearest to each (a standing
//! sheet by contact area). Loads are characteristic and
//! unfactored, by kind. Missing data is listed with the members it reaches,
//! never guessed.

use crate::contact::ContactFaces;
use crate::continuous_span::{self, Piece};
use crate::load_path::{joint_points, on_floor};
use crate::model::{Joint, JointFastening, Part, ProgramModel, ProgramPartBody};
use crate::validate::{Issue, Severity};
use ketchup_geometry::linalg::{self, cross, dot, length, sub};
use ketchup_model::tolerance::CONTACT_FIT_MM;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LOAD_KINDS: [&str; 4] = ["permanent", "imposed", "roof", "snow"];
const GRAVITY: f64 = 9.81;
/// not a tolerance: unit conversion, mm³ to m³.
const M3_PER_MM3: f64 = 1e-9;
/// not a tolerance: unit conversion, mm² to m².
const M2_PER_MM2: f64 = 1e-6;
/// A contact face looking at least this much downward lets the part rest on the other.
const RESTS_ON: f64 = -0.5;
/// A sheet's own load is shared out on a raster of at most this many cells
/// along its longer side...
const TRIBUTARY_CELLS: f64 = 100.0;
/// ...and no finer than this, mm.
const TRIBUTARY_CELL_MM: f64 = 25.0;
/// A bearing under a member is probed for gaps at this step along it, mm.
const BEARING_PROBE_MM: f64 = 5.0;

/// Newtons by load kind.
pub type Loads = BTreeMap<String, f64>;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MaterialWeight {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kg_m3: Option<f64>,
    /// Per m² of the part's largest face (a layer modelled as one slab).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kg_m2: Option<f64>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AreaLoad {
    pub name: String,
    pub kind: String,
    /// None: declared, but its value is not known.
    pub kn_m2: Option<f64>,
    pub on: BTreeSet<String>,
    /// Per plan area (snow) rather than per face area.
    pub projected: bool,
    pub source: String,
}

/// A load spread evenly over `from_mm..to_mm` along a member (equal ends: a point load).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Patch {
    pub from_mm: f64,
    pub to_mm: f64,
    pub loads_n: Loads,
    /// "own weight", an area load's name, or the part it carries.
    pub source: String,
    /// Where a carried part bears on this one (world points); empty for own loads.
    #[serde(skip)]
    pub footprint: Vec<[f64; 3]>,
}

/// What a member passes to one of its supports.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Reaction {
    pub part: String,
    pub at_mm: f64,
    /// Length of the bearing along the member.
    pub length_mm: f64,
    /// Area of the faces it bears on (0 through a joint).
    pub contact_mm2: f64,
    pub loads_n: Loads,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joint: Option<String>,
    /// The member only rests here and lifts off under its loads (static
    /// equilibrium, EN 1990 EQU): this support carries nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lifted_off: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MemberLoad {
    pub part: String,
    pub length_mm: f64,
    /// Unit direction of the member's load axis ([`Part::load_axis`]: the
    /// one its `length_axis` names, else its longest); positions run along it from one end.
    pub axis: [f64; 3],
    /// Everything on the member, by kind.
    pub loads_n: Loads,
    pub patches: Vec<Patch>,
    pub reactions: Vec<Reaction>,
    /// Where parts hold the member sideways along it (decking, battens, a
    /// ceiling), each on the edge it touches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub braced_by: Vec<Brace>,
    /// Why the loads are incomplete; empty when every input was known.
    pub missing: Vec<String>,
}

/// A part holding a member sideways at one edge (review 2026-11-02 PG-120,
/// 2026-11-04 PG-140 to PG-142, r19 PG-160 to PG-163): it bears on the member
/// and on a part declared grounded across the member, and is fastened to both
/// by a joint() of a kind that fastens. It holds the member at its fasteners
/// where the two touch, or at the middle of each place they touch when the
/// joint names none; `from_mm` equals `to_mm` then.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Brace {
    pub part: String,
    /// `top` (compressed under a sagging moment) or `bottom` (under hogging).
    pub edge: String,
    pub from_mm: f64,
    pub to_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct JointLoad {
    pub loads_n: Loads,
    pub missing: Vec<String>,
    /// The pull it takes holding a member down, when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tension: Option<Tension>,
}

/// The pull a joint takes holding a member down against lifting, in newtons
/// (positive), each load kind counted apart on every stretch of the member.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Tension {
    /// For static equilibrium (EN 1990 A1.2(A)): permanent 0.9 where it holds
    /// the member down and 1.1 where it lifts it, variable loads 1.5 only where
    /// they lift.
    pub design_n: f64,
    /// Permanent whole, variable loads only where they lift.
    pub characteristic_n: f64,
    /// The load kinds that lift the member there.
    pub lifted_by: Vec<String>,
}

impl Tension {
    fn add(&mut self, other: &Tension) {
        self.design_n = round(self.design_n + other.design_n);
        self.characteristic_n = round(self.characteristic_n + other.characteristic_n);
        for kind in &other.lifted_by {
            if !self.lifted_by.contains(kind) {
                self.lifted_by.push(kind.clone());
            }
        }
        self.lifted_by.sort();
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct LoadReport {
    pub members: Vec<MemberLoad>,
    /// Load passed through each bearing joint, and the pull of every joint
    /// that holds a member down, by joint name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub joints: BTreeMap<String, JointLoad>,
    /// Loaded parts (no members) that rest on nothing that carries them, with
    /// why: their load reaches no member, so no check is complete.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unassigned: BTreeMap<String, String>,
    /// What each unassigned part only touches, without a joint.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub touched: BTreeMap<String, Vec<String>>,
    /// Fasteners joints name away from where their parts touch, which brace nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stray_fasteners: Vec<StrayFastener>,
    /// The area loads each part takes none of, standing on edge with no broad face up.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edge_on: BTreeMap<String, Vec<String>>,
    /// Members, each with the grounded part bracing it that is smaller in
    /// every direction than the member is deep.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub small_anchorages: Vec<[String; 2]>,
    /// Joints of a kind the catalogue does not know ([`crate::joint_kind`]), each
    /// with the kind that fastens nothing it reads like, if one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doubtful_joint_kinds: Vec<DoubtfulJointKind>,
    /// Declared braces and diaphragms that hold nothing, each with its parts and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored_bracing: Vec<IgnoredBracing>,
}

/// A part declared to hold members' edge sideways (`brace()`, review
/// 2026-11-08 §5.1): it holds each member it is fastened to by a joint that
/// fastens, on `edge`, every `spacing_mm` along where it crosses the member,
/// or at the joint's fasteners there (the middle when it names none), when it
/// is grounded, in an anchored diaphragm or fastened to a grounded part.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DeclaredBrace {
    pub members: Vec<String>,
    pub by: Vec<String>,
    /// `top` or `bottom`.
    pub edge: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing_mm: Option<f64>,
}

/// Sheets declared to act as one stiff plane anchored to grounded parts
/// (`diaphragm()`, review 2026-11-08 §5.1): each sheet holds the members it
/// is fastened to as if it reached the anchorage itself, when every part of
/// `anchored_to` is grounded and fastened to one of the sheets.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Diaphragm {
    pub sheets: Vec<String>,
    pub anchored_to: Vec<String>,
}

/// A declared brace or diaphragm that holds nothing, why, and what to do about
/// that reason (review 2026-11-09 PG-310: one hint for every reason).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct IgnoredBracing {
    pub parts: Vec<String>,
    pub reason: String,
    #[serde(default)]
    pub hint: String,
}

/// A joint of `parts` whose `kind` the catalogue does not know (review
/// 2026-11-08 PG-241), and reads like `like`, a kind that fastens nothing
/// (review 2026-11-06 P3): it is taken to fasten them, and every check
/// standing on it is not verified.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DoubtfulJointKind {
    pub parts: [String; 2],
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
}

/// Issue kind of a loaded part whose load reaches no member: the checks of
/// what it touches, and of what that rests on, are not complete.
pub const LOAD_UNASSIGNED: &str = "load_unassigned";
/// Issue kind of a joint fastener away from where its parts touch: it holds nothing.
pub const FASTENER_OFF_CONTACT: &str = "fastener_off_contact";
/// Issue kind of an area load on a part standing on edge, which takes none of it.
pub const AREA_LOAD_EDGE_ON: &str = "area_load_edge_on";
/// Issue kind of a member braced from a grounded part smaller than it is deep.
pub const ANCHORAGE_SMALL: &str = "anchorage_small";
/// Issue kind of a joint of a kind that reads like one that does not fasten.
pub const JOINT_KIND_DOUBTFUL: &str = "joint_kind_doubtful";
/// Issue kind of a joint of a kind the catalogue does not know.
pub const JOINT_KIND_UNKNOWN: &str = "joint_kind_unknown";
/// Issue kind of a declared brace or diaphragm that holds nothing.
pub const DECLARED_BRACING_IGNORED: &str = "declared_bracing_ignored";

impl LoadReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// A warning for every unassigned part, naming it and what it touches,
    /// and for every joint's fasteners away from where its parts touch.
    #[must_use]
    pub fn issues(&self) -> Vec<Issue> {
        let stray = self.stray_fasteners.iter().map(|stray| {
            let [a, b] = &stray.parts;
            let [x, y, z] = stray.at_mm[0].map(round);
            // Worded for either part, the member or what would brace it
            // (review 2026-11-06 PG-181).
            let message = match stray.at_mm.len() {
                1 => format!(
                    "A fastener joining {a} and {b} lies away from where they touch (at {x}, {y}, {z} mm); it holds nothing, so the joint braces nothing there."
                ),
                count => format!(
                    "{count} of the fasteners joining {a} and {b} lie away from where they touch (the first at {x}, {y}, {z} mm); they hold nothing, so the joint braces nothing there."
                ),
            };
            Issue {
                severity: Severity::Warning,
                kind: FASTENER_OFF_CONTACT,
                parts: stray.parts.to_vec(),
                message,
                where_mm: None,
                hint: format!("Give fasteners=[...] at points where {a} and {b} meet."),
                source_lines: Vec::new(),
            }
        });
        self.unassigned
            .iter()
            .map(|(part, reason)| {
                let mut parts = vec![part.clone()];
                parts.extend(self.touched.get(part).into_iter().flatten().cloned());
                Issue {
                    severity: Severity::Warning,
                    kind: LOAD_UNASSIGNED,
                    parts,
                    message: format!("The load of {part} reaches no member: {reason}."),
                    where_mm: None,
                    hint: "Join it to what carries it with joint(), or rest it on a member."
                        .to_owned(),
                    source_lines: Vec::new(),
                }
            })
            .chain(stray)
            .chain(self.ignored_bracing.iter().map(|ignored| Issue {
                severity: Severity::Warning,
                kind: DECLARED_BRACING_IGNORED,
                parts: ignored.parts.clone(),
                message: format!("{}.", ignored.reason),
                where_mm: None,
                hint: ignored.hint.clone(),
                source_lines: Vec::new(),
            }))
            .chain(self.edge_on.iter().map(|(part, loads)| Issue {
                severity: Severity::Warning,
                kind: AREA_LOAD_EDGE_ON,
                parts: vec![part.clone()],
                message: format!(
                    "{part} stands on edge with no broad face up, so it takes none of the area load {}.",
                    loads.join(", ")
                ),
                where_mm: None,
                hint: "Lay it flat if it carries the load, or put the load on the part it carries."
                    .to_owned(),
                source_lines: Vec::new(),
            }))
            .chain(self.small_anchorages.iter().map(|[member, anchorage]| Issue {
                severity: Severity::Warning,
                kind: ANCHORAGE_SMALL,
                parts: vec![member.clone(), anchorage.clone()],
                message: format!(
                    "{member} is braced sideways from {anchorage}, which is smaller in every direction than {member} is deep; the check takes it to hold."
                ),
                where_mm: None,
                hint: format!(
                    "Brace {member} from a wall, post or frame, or make sure {anchorage} is anchored to hold it."
                ),
                source_lines: Vec::new(),
            }))
            .chain(self.doubtful_joint_kinds.iter().map(|doubtful| {
                let [a, b] = &doubtful.parts;
                let kind = &doubtful.kind;
                let (issue, message, hint) = match &doubtful.like {
                    Some(like) => (
                        JOINT_KIND_DOUBTFUL,
                        format!(
                            "The joint of {a} and {b} is of kind \"{kind}\", which reads like \"{like}\", which fastens nothing; the checks standing on it are not verified."
                        ),
                        format!(
                            "Write kind=\"{like}\" if they only meet, or name the fastening, e.g. kind=\"screw\"."
                        ),
                    ),
                    None => (
                        JOINT_KIND_UNKNOWN,
                        format!(
                            "The joint of {a} and {b} is of kind \"{kind}\", which is not a kind the checks know to fasten or not; the checks standing on it are not verified."
                        ),
                        "Name the fastening, e.g. kind=\"screw\", \"nail\", \"vrut\" or \"Schraube\", or kind=\"none\" (\"kontakt\", \"volně\") if the parts only meet.".to_owned(),
                    ),
                };
                Issue {
                    severity: Severity::Warning,
                    kind: issue,
                    parts: doubtful.parts.to_vec(),
                    message,
                    where_mm: None,
                    hint,
                    source_lines: Vec::new(),
                }
            }))
            .collect()
    }
}

fn add(into: &mut Loads, from: &Loads, factor: f64) {
    for (kind, value) in from {
        *into.entry(kind.clone()).or_default() += value * factor;
    }
}

/// Area of a planar polygon in space.
fn polygon_area(points: &[[f64; 3]]) -> f64 {
    let mut sum = [0.0; 3];
    for (index, a) in points.iter().enumerate() {
        sum = linalg::add(sum, cross(*a, points[(index + 1) % points.len()]));
    }
    length(sum) / 2.0
}

/// Area of a straight-sided extrusion's profile.
pub(crate) fn profile_area(part: &Part) -> Option<f64> {
    let ProgramPartBody::Extrusion { segments, .. } = &part.body else {
        return None;
    };
    if !segments.iter().all(|segment| segment.is_line()) {
        return None;
    }
    let twice: f64 = segments
        .iter()
        .map(|s| s.start_mm[0] * s.end_mm[1] - s.end_mm[0] * s.start_mm[1])
        .sum();
    Some(twice.abs() / 2.0)
}

pub(crate) fn extents(part: &Part) -> [f64; 3] {
    let (min, max) = part.local_bounds();
    std::array::from_fn(|axis| max[axis] - min[axis])
}

/// World directions of the part's local axes.
pub(crate) fn axes(part: &Part) -> [[f64; 3]; 3] {
    let origin = part.to_world([0.0; 3]);
    std::array::from_fn(|axis| {
        sub(
            part.to_world(std::array::from_fn(|i| if i == axis { 1.0 } else { 0.0 })),
            origin,
        )
    })
}

/// The extrusion's profile area when its local z is `axis`, else the box face.
fn face_area_mm2(part: &Part, axis: usize) -> f64 {
    let size = extents(part);
    match profile_area(part) {
        Some(area) if axis == 2 => area,
        _ => (0..3).filter(|i| *i != axis).map(|i| size[i]).product(),
    }
}

fn volume_m3(part: &Part) -> f64 {
    let size = extents(part);
    let mm3 = match (&part.body, profile_area(part)) {
        (ProgramPartBody::Extrusion { distance_mm, .. }, Some(area)) => area * distance_mm,
        _ => size.iter().product(),
    };
    mm3 * M3_PER_MM3
}

/// Largest face of a slab-like part, m².
fn slab_area_m2(part: &Part) -> f64 {
    let size = extents(part);
    let thin = (0..3)
        .min_by(|a, b| size[*a].total_cmp(&size[*b]))
        .unwrap_or(2);
    face_area_mm2(part, thin) * M2_PER_MM2
}

/// The broad face of a slab lying flat or sloped up to 60 degrees, m²; per
/// plan area when projected. A standing slab (a wall board) takes none.
fn upward_area_m2(part: &Part, projected: bool) -> f64 {
    let size = extents(part);
    let Some(axis) = (0..3).min_by(|a, b| size[*a].total_cmp(&size[*b])) else {
        return 0.0;
    };
    let up = axes(part)[axis][2].abs();
    if up < 0.5 {
        return 0.0;
    }
    face_area_mm2(part, axis) * M2_PER_MM2 * if projected { up } else { 1.0 }
}

/// A member's load frame along [`Part::load_axis`], as the member check takes
/// it: positions from 0 at one end to `length` at the other. Which edge a
/// contact holds is read in the member's own bar frame ([`Part::bar_frame`]):
/// its true length axis and its section's two sides with their sizes.
struct Frame {
    centre: [f64; 3],
    axis: [f64; 3],
    length: f64,
    bar_centre: [f64; 3],
    bar_axis: [f64; 3],
    section: [([f64; 3], f64); 2],
}

impl Frame {
    fn of(part: &Part) -> Self {
        let size = extents(part);
        let long = part.load_axis();
        let (min, max) = part.local_bounds();
        let bar = part.bar_frame();
        let local = axes(part);
        let world = |d: [f64; 3]| -> [f64; 3] {
            let v = std::array::from_fn(|i| (0..3).map(|k| d[k] * local[k][i]).sum());
            linalg::normalize(v).unwrap_or(v)
        };
        Self {
            centre: part.to_world(std::array::from_fn(|i| (min[i] + max[i]) / 2.0)),
            axis: local[long],
            length: size[long],
            bar_centre: part.to_world(bar.centre),
            bar_axis: world(bar.axes[0]),
            section: [1, 2].map(|side| (world(bar.axes[side]), bar.sizes[side])),
        }
    }

    fn along(&self, point: [f64; 3]) -> f64 {
        dot(sub(point, self.centre), self.axis) + self.length / 2.0
    }

    /// The edge `points` touch: Some(true) on top, Some(false) underneath,
    /// None from the side. Their middle holds an edge only in the outer quarter
    /// of the section's depth from that edge (review 2026-11-08 PG-240: a lath
    /// on the side of an 80 x 400 beam 13 mm above its middle held its top,
    /// though bracing at mid-depth does not stop it twisting).
    fn edge(&self, points: &[[f64; 3]]) -> Option<bool> {
        let (height, depth) = self.height(points)?;
        (height.abs() >= depth / 4.0).then_some(height > 0.0)
    }

    /// How high the middle of `points` lies above the section's middle, and the
    /// section's depth; None for none or a standing member, which has no top.
    fn height(&self, points: &[[f64; 3]]) -> Option<(f64, f64)> {
        if points.is_empty() {
            return None;
        }
        // The middle of the points' box: on the face they lie on.
        let middle: [f64; 3] = std::array::from_fn(|i| {
            let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
                (lo.min(p[i]), hi.max(p[i]))
            });
            (lo + hi) / 2.0
        });
        // Up in the section's plane; a standing member has no top or bottom.
        let rise = self.bar_axis[2];
        let up = linalg::normalize(std::array::from_fn(|i| {
            f64::from(u8::from(i == 2)) - rise * self.bar_axis[i]
        }))?;
        let height = dot(sub(middle, self.bar_centre), up);
        let depth: f64 = self
            .section
            .iter()
            .map(|(side, size)| dot(*side, up).abs() * size)
            .sum();
        Some((height, depth))
    }

    fn range(&self, points: &[[f64; 3]]) -> (f64, f64) {
        points
            .iter()
            .map(|point| self.along(*point))
            .fold((f64::MAX, f64::MIN), |(lo, hi), at| {
                (lo.min(at), hi.max(at))
            })
    }

    /// Positions of plan points (x, y) projected along the member seen from
    /// above; None for an upright member.
    fn plan_range(&self, points: &[[f64; 2]]) -> Option<(f64, f64)> {
        let flat = self.axis[0] * self.axis[0] + self.axis[1] * self.axis[1];
        if flat < 0.09 {
            return None;
        }
        let along = |p: &[f64; 2]| {
            ((p[0] - self.centre[0]) * self.axis[0] + (p[1] - self.centre[1]) * self.axis[1]) / flat
                + self.length / 2.0
        };
        Some(
            points
                .iter()
                .map(along)
                .fold((f64::MAX, f64::MIN), |(lo, hi), at| {
                    (lo.min(at), hi.max(at))
                }),
        )
    }
}

/// Plan bounding box [min x, min y, max x, max y] of points, grown by `margin`.
fn plan_box(points: &[[f64; 3]], margin: f64) -> [f64; 4] {
    points
        .iter()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
            [
                b[0].min(p[0] - margin),
                b[1].min(p[1] - margin),
                b[2].max(p[0] + margin),
                b[3].max(p[1] + margin),
            ]
        })
}

fn plan_overlap(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let o = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (o[2] > o[0] && o[3] > o[1]).then_some(o)
}

/// Where a part bears on one supporter.
struct Support {
    supporter: usize,
    points: Vec<[f64; 3]>,
    /// Each contact polygon `points` merges, apart (review 2026-11-04 r19
    /// PG-161): the middle of two far contacts is no place they touch.
    pieces: Vec<Vec<[f64; 3]>>,
    area: f64,
    joint: Option<String>,
}

/// Merges the supports on one supporter, keeping a joint's name. Along a
/// member only bearings that touch merge: a beam on both ends of one ring
/// beam bears twice, with a span between.
fn grouped(supports: Vec<Support>, member: Option<&Frame>) -> Vec<Support> {
    let mut groups: Vec<Support> = Vec::new();
    for support in supports {
        let touches = |group: &Support| {
            member.is_none_or(|frame| {
                let (a, b) = (frame.range(&group.points), frame.range(&support.points));
                a.0 <= b.1 + 1.0 && b.0 <= a.1 + 1.0
            })
        };
        match groups
            .iter_mut()
            .find(|group| group.supporter == support.supporter && touches(group))
        {
            Some(group) => {
                group.points.extend(support.points);
                group.pieces.extend(support.pieces);
                group.area += support.area;
                group.joint = group.joint.take().or(support.joint);
            }
            None => groups.push(support),
        }
    }
    groups
}

/// Where along `frame` the contact `points` has material: crossings of the
/// polygon with the plane at `at`, paired by the even-odd rule (the doubled
/// connectors between its rings pair up to nothing).
fn material_at(points: &[[f64; 3]], frame: &Frame, across: [f64; 3], at: f64) -> bool {
    let mut crossings: Vec<f64> = Vec::new();
    for (index, p) in points.iter().enumerate() {
        let q = points[(index + 1) % points.len()];
        let (a, b) = (frame.along(*p), frame.along(q));
        if (a > at) != (b > at) {
            let t = (at - a) / (b - a);
            crossings.push(dot(linalg::add(*p, linalg::scale(sub(q, *p), t)), across));
        }
    }
    crossings.sort_by(f64::total_cmp);
    crossings
        .chunks_exact(2)
        .any(|pair| pair[1] - pair[0] > CONTACT_FIT_MM)
}

/// The part of a contact polygon between `lo` and `hi` along `frame`.
fn clipped_polygon(points: &[[f64; 3]], frame: &Frame, lo: f64, hi: f64) -> Vec<[f64; 3]> {
    let mut polygon = points.to_vec();
    for (limit, keep_above) in [(lo, true), (hi, false)] {
        let inside = |p: &[f64; 3]| (frame.along(*p) >= limit) == keep_above;
        let mut next = Vec::new();
        for (index, p) in polygon.iter().enumerate() {
            let q = polygon[(index + 1) % polygon.len()];
            if inside(p) {
                next.push(*p);
            }
            if inside(p) != inside(&q) {
                let (a, b) = (frame.along(*p), frame.along(q));
                next.push(linalg::add(
                    *p,
                    linalg::scale(sub(q, *p), (limit - a) / (b - a)),
                ));
            }
        }
        polygon = next;
    }
    polygon
}

/// One contact under a member split where it has no material along the
/// member (a beam across both sides of a ring beam's opening).
fn bearing_runs(support: Support, frame: &Frame) -> Vec<Support> {
    let (lo, hi) = frame.range(&support.points);
    let normal = support
        .points
        .iter()
        .enumerate()
        .fold([0.0; 3], |sum, (index, p)| {
            linalg::add(
                sum,
                cross(*p, support.points[(index + 1) % support.points.len()]),
            )
        });
    let across = linalg::normalize(cross(normal, frame.axis));
    let (Some(across), false) = (
        across,
        support.joint.is_some() || hi - lo < 2.0 * BEARING_PROBE_MM,
    ) else {
        return vec![support];
    };
    let count = ((hi - lo) / BEARING_PROBE_MM).ceil() as usize;
    let step = (hi - lo) / count as f64;
    let mut runs: Vec<(f64, f64)> = Vec::new();
    let mut open = false;
    for index in 0..count {
        let material = material_at(
            &support.points,
            frame,
            across,
            lo + (index as f64 + 0.5) * step,
        );
        let start = lo + index as f64 * step;
        match (material, open) {
            (true, true) => runs.last_mut().expect("an open run").1 = start + step,
            (true, false) => runs.push((start, start + step)),
            _ => {}
        }
        open = material;
    }
    if runs.len() < 2 {
        return vec![support];
    }
    runs.into_iter()
        .map(|(from, to)| {
            let points = clipped_polygon(&support.points, frame, from, to);
            Support {
                supporter: support.supporter,
                area: polygon_area(&points),
                pieces: vec![points.clone()],
                points,
                joint: None,
            }
        })
        .collect()
}

/// A beam's loads shared among its supports, which bear on `ranges` along it
/// (same order), continuous over the inner ones. A support `held` by a joint
/// may pull the member down (a negative share); any other lets go of it.
/// `across_gravity`: the horizontal part of the member's unit axis (0 for a
/// post, which its loads press along rather than turn).
fn beam_shares(
    patches: &[Patch],
    ranges: &[(f64, f64)],
    held: &[bool],
    across_gravity: f64,
) -> Shares {
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by(|a, b| ranges[*a].0.total_cmp(&ranges[*b].0));
    // Bearings that touch (a doubled stud) are one support and share what
    // reaches it equally.
    let mut clusters: Vec<((f64, f64), Vec<usize>)> = Vec::new();
    for index in order {
        let (lo, hi) = ranges[index];
        match clusters.last_mut() {
            Some((range, members)) if lo <= range.1 + 1.0 => {
                range.1 = range.1.max(hi);
                members.push(index);
            }
            _ => clusters.push(((lo, hi), vec![index])),
        }
    }
    let bearings: Vec<(f64, f64)> = clusters.iter().map(|(range, _)| *range).collect();
    let clusters: Vec<(f64, Vec<usize>)> = clusters
        .into_iter()
        .map(|((lo, hi), members)| (f64::midpoint(lo, hi), members))
        .collect();
    let kinds: BTreeSet<&String> = patches
        .iter()
        .flat_map(|patch| patch.loads_n.keys())
        .collect();
    let pieces: Vec<(&String, Vec<Piece>)> = kinds
        .into_iter()
        .map(|kind| {
            let pieces = patches
                .iter()
                .filter_map(|patch| {
                    let force = patch.loads_n.get(kind).copied().unwrap_or(0.0);
                    (force != 0.0).then_some((
                        patch.from_mm.min(patch.to_mm),
                        patch.from_mm.max(patch.to_mm),
                        force,
                    ))
                })
                .collect();
            (kind, pieces)
        })
        .collect();
    // Reactions on the supports still in contact, by cluster.
    let solve = |active: &[usize]| -> Vec<Loads> {
        let at: Vec<f64> = active.iter().map(|c| clusters[*c].0).collect();
        let mut out = vec![Loads::new(); active.len()];
        for (kind, pieces) in pieces.iter().filter(|(_, pieces)| !pieces.is_empty()) {
            let solution = continuous_span::solve(&at, &[], pieces);
            for (k, reaction) in solution.reactions.into_iter().enumerate() {
                out[k].insert((*kind).clone(), reaction);
            }
        }
        out
    };
    // The design value of each reaction for static equilibrium (the member as
    // a rigid body): every kind apart on the overhang beyond the first bearing
    // still in contact, between the outer bearings and beyond the last.
    // Negative: the support has to pull the member down, by `Tension`.
    let equilibrium = |active: &[usize]| -> Vec<Tension> {
        let at: Vec<f64> = active.iter().map(|c| clusters[*c].0).collect();
        let (Some(first), Some(last)) = (active.first(), active.last()) else {
            return Vec::new();
        };
        let outer = [bearings[*first].0, bearings[*last].1];
        let mut holding = vec![
            Tension {
                design_n: 0.0,
                characteristic_n: 0.0,
                lifted_by: Vec::new(),
            };
            active.len()
        ];
        for (kind, pieces) in &pieces {
            for stretch in continuous_span::by_stretch(&outer, pieces) {
                if stretch.is_empty() {
                    continue;
                }
                let solution = continuous_span::solve(&at, &[], &stretch);
                for (k, reaction) in solution.reactions.into_iter().enumerate() {
                    let held = &mut holding[k];
                    held.design_n += holding_down(kind, reaction);
                    if kind.as_str() == "permanent" || reaction < 0.0 {
                        held.characteristic_n += reaction;
                    }
                    if reaction < 0.0 && !held.lifted_by.contains(*kind) {
                        held.lifted_by.push((*kind).clone());
                    }
                }
            }
        }
        holding
    };
    // A support that would have to pull the member down and holds it by no
    // joint lets go; the rest take its share. The worst one first.
    let held_cluster = |c: usize| clusters[c].1.iter().any(|member| held[*member]);
    let mut active: Vec<usize> = (0..clusters.len()).collect();
    while active.len() > 1 {
        let holding = equilibrium(&active);
        let worst = (0..active.len())
            .filter(|k| !held_cluster(active[*k]))
            .map(|k| (k, holding[k].design_n))
            .filter(|(_, value)| *value < 0.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((k, _)) = worst else {
            break;
        };
        active.remove(k);
    }
    let reactions = solve(&active);
    let holding = if active.len() > 1 {
        equilibrium(&active)
    } else {
        Vec::new()
    };
    let mut shares = vec![Loads::new(); ranges.len()];
    let mut lifted = vec![true; ranges.len()];
    let mut tension = vec![None; ranges.len()];
    for (k, cluster) in active.iter().enumerate() {
        let members = &clusters[*cluster].1;
        let part = 1.0 / members.len() as f64;
        for member in members {
            add(&mut shares[*member], &reactions[k], part);
            lifted[*member] = false;
            tension[*member] = holding
                .get(k)
                .filter(|held| held.design_n < 0.0)
                .map(|held| Tension {
                    design_n: round(-held.design_n * part),
                    characteristic_n: round((-held.characteristic_n * part).max(0.0)),
                    lifted_by: {
                        let mut kinds = held.lifted_by.clone();
                        kinds.sort();
                        kinds
                    },
                });
        }
    }
    // Lifting off down to one support of several, the member tips over it. On
    // one support only it tips over an edge of the bearing when its loads
    // beyond that edge outweigh the rest for static equilibrium: every kind
    // apart on the stretches before, on and after the bearing, as above (PG-41).
    // No joint takes a moment (a hanger or screws do not clamp it).
    let tips = match active.as_slice() {
        [_] if clusters.len() > 1 => true,
        [only] => {
            let (lo, hi) = bearings[*only];
            let force: f64 = pieces
                .iter()
                .flat_map(|(_, pieces)| pieces)
                .map(|(_, _, value)| value.abs())
                .sum();
            // The moment holding the member on the bearing about one edge,
            // `lever` the arm of a load towards the bearing (negative beyond it).
            let holding = |lever: &dyn Fn(f64) -> f64| -> f64 {
                pieces
                    .iter()
                    .flat_map(|(kind, pieces)| {
                        continuous_span::by_stretch(&[lo, hi], pieces)
                            .into_iter()
                            .map(move |stretch| {
                                let moment = stretch
                                    .iter()
                                    .map(|(from, to, value)| {
                                        value * lever(f64::midpoint(*from, *to))
                                    })
                                    .sum::<f64>();
                                holding_down(kind, moment)
                            })
                    })
                    .sum()
            };
            let over_lo = holding(&|at| at - lo);
            let over_hi = holding(&|at| hi - at);
            force > 0.0 && over_lo.min(over_hi) * across_gravity < -force
        }
        _ => false,
    };
    Shares {
        loads: shares,
        lifted,
        tension,
        tips,
    }
}

/// What a beam passes to each of its supports.
struct Shares {
    loads: Vec<Loads>,
    /// The pull a support held by a joint takes holding the member down.
    tension: Vec<Option<Tension>>,
    /// The member lifts off this support (it carries nothing).
    lifted: Vec<bool>,
    /// It lifted off all but one of its supports and tips over that one.
    tips: bool,
}

/// The design value for static equilibrium (EN 1990 table A1.2(A)) of the
/// reaction one stretch of one load kind gives: permanent 0.9 holding down and
/// 1.1 lifting, a variable load only lifting, by 1.5. The favourable and the
/// unfavourable stretches of an action count apart (6.4.3.1(4)), so a load on
/// the span does not hide the uplift of the same load on an overhang.
/// Negative: the member lifts off.
fn holding_down(kind: &str, reaction: f64) -> f64 {
    match (kind, reaction >= 0.0) {
        ("permanent", true) => 0.9 * reaction,
        ("permanent", false) => 1.1 * reaction,
        (_, true) => 0.0,
        (_, false) => 1.5 * reaction,
    }
}

/// Where loads land on a supporter: (loads, range along it or None for its whole bearing).
type Pieces = Vec<(Loads, Option<(f64, f64)>)>;

/// Distance in plan from `point` to where a support bears (inside: negative).
fn plan_distance(points: &[[f64; 3]], point: [f64; 2]) -> f64 {
    let outline: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
    linalg::signed_outline_distance(&outline, point).unwrap_or_else(|| {
        outline
            .first()
            .map_or(f64::INFINITY, |p| (p[0] - point[0]).hypot(p[1] - point[1]))
    })
}

/// The fraction of a flat or sloped sheet's own load each support takes: every
/// cell of a raster over its broad face goes to the support nearest to it seen
/// from above, so a support carries the strip it is nearest to, whatever the
/// size of its contact. None for a standing sheet (a wall board).
fn tributary_shares(part: &Part, list: &[Support]) -> Option<Vec<f64>> {
    let size = extents(part);
    let thin = (0..3).min_by(|a, b| size[*a].total_cmp(&size[*b]))?;
    if axes(part)[thin][2].abs() < 0.5 {
        return None;
    }
    let [u, v] = match thin {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let cell = (size[u].max(size[v]) / TRIBUTARY_CELLS).max(TRIBUTARY_CELL_MM);
    let counts = [u, v].map(|axis| (size[axis] / cell).ceil().max(1.0) as usize);
    let (min, max) = part.local_bounds();
    let mut cells = vec![0usize; list.len()];
    for i in 0..counts[0] {
        for j in 0..counts[1] {
            let mut local = [(min[thin] + max[thin]) / 2.0; 3];
            local[u] = min[u] + (i as f64 + 0.5) * size[u] / counts[0] as f64;
            local[v] = min[v] + (j as f64 + 0.5) * size[v] / counts[1] as f64;
            let world = part.to_world(local);
            let nearest = list
                .iter()
                .map(|support| plan_distance(&support.points, [world[0], world[1]]))
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(&b.1))?;
            cells[nearest.0] += 1;
        }
    }
    let total = (counts[0] * counts[1]) as f64;
    Some(
        cells
            .into_iter()
            .map(|count| count as f64 / total)
            .collect(),
    )
}

/// A part that is no member (a deck, a sheet) shares its own loads among its
/// supports by the area of it each is nearest to (a standing sheet by contact
/// area), and passes each load it carries to the supports under where that
/// load bears on it seen from above (searching up to 1 m around it), at that
/// place along them.
fn sheet_shares(part: &Part, patches: &[Patch], list: &[Support], frames: &[Frame]) -> Vec<Pieces> {
    let mut out: Vec<Pieces> = list.iter().map(|_| Vec::new()).collect();
    let mut own = Loads::new();
    for patch in patches {
        if patch.footprint.is_empty() {
            add(&mut own, &patch.loads_n, 1.0);
            continue;
        }
        let found = [0.0, 0.5, 100.0, 300.0, 1000.0].iter().find_map(|margin| {
            let foot = plan_box(&patch.footprint, *margin);
            let overlaps: Vec<Option<[f64; 4]>> = list
                .iter()
                .map(|support| plan_overlap(foot, plan_box(&support.points, 0.5)))
                .collect();
            let weights: Vec<f64> = overlaps
                .iter()
                .map(|o| o.map_or(0.0, |o| (o[2] - o[0]) * (o[3] - o[1])))
                .collect();
            let total: f64 = weights.iter().sum();
            (total > 0.0).then_some((overlaps, weights, total))
        });
        let Some((overlaps, weights, total)) = found else {
            add(&mut own, &patch.loads_n, 1.0);
            continue;
        };
        for (k, support) in list.iter().enumerate() {
            let Some(o) = overlaps[k] else {
                continue;
            };
            let mut share = Loads::new();
            add(&mut share, &patch.loads_n, weights[k] / total);
            let corners = [[o[0], o[1]], [o[2], o[1]], [o[0], o[3]], [o[2], o[3]]];
            out[k].push((share, frames[support.supporter].plan_range(&corners)));
        }
    }
    if !own.is_empty() {
        let factors = tributary_shares(part, list).unwrap_or_else(|| {
            let areas: Vec<f64> = list.iter().map(|s| polygon_area(&s.points)).collect();
            let sum: f64 = areas.iter().sum();
            areas
                .iter()
                .map(|area| {
                    if sum > 0.0 {
                        area / sum
                    } else {
                        1.0 / list.len() as f64
                    }
                })
                .collect()
        });
        for (k, factor) in factors.into_iter().enumerate() {
            let mut share = Loads::new();
            add(&mut share, &own, factor);
            out[k].push((share, None));
        }
    }
    out
}

fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0 + 0.0
}

fn rounded(loads: &Loads) -> Loads {
    loads
        .iter()
        .map(|(kind, value)| (kind.clone(), round(*value)))
        .collect()
}

/// Own loads: the weight of the parts in scope and the area loads on them,
/// part by part, with what is missing to know them, and the area loads each
/// part standing on edge takes none of (review 2026-11-04 r19 P3).
fn own_loads(model: &ProgramModel, frames: &[Frame]) -> OwnLoads {
    let parts = &model.parts;
    let mut patches: Vec<Vec<Patch>> = parts.iter().map(|_| Vec::new()).collect();
    let mut missing: Vec<BTreeSet<String>> = parts.iter().map(|_| BTreeSet::new()).collect();
    let mut edge_on: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (item, part) in parts.iter().enumerate() {
        let whole = |loads_n: Loads, source: &str| Patch {
            from_mm: 0.0,
            to_mm: frames[item].length,
            loads_n,
            source: source.to_owned(),
            footprint: Vec::new(),
        };
        if !part.tags.is_disjoint(&model.weight_scope) {
            let material = part.material.as_deref().unwrap_or("");
            match model.material_weights.get(material) {
                Some(weight) => {
                    let kg = weight.kg_m3.map_or_else(
                        || weight.kg_m2.unwrap_or(0.0) * slab_area_m2(part),
                        |kg_m3| kg_m3 * volume_m3(part),
                    );
                    let loads = Loads::from([("permanent".to_owned(), kg * GRAVITY)]);
                    patches[item].push(whole(loads, "own weight"));
                }
                None => {
                    missing[item]
                        .insert(format!("the weight of material {material:?} is not known"));
                }
            }
        }
        for load in &model.area_loads {
            if part.tags.is_disjoint(&load.on) {
                continue;
            }
            let area = upward_area_m2(part, load.projected);
            if area <= 0.0 {
                edge_on
                    .entry(part.name.clone())
                    .or_default()
                    .push(load.name.clone());
                continue;
            }
            match load.kn_m2 {
                Some(kn_m2) => {
                    let loads = Loads::from([(load.kind.clone(), kn_m2 * 1000.0 * area)]);
                    patches[item].push(whole(loads, &load.name));
                }
                None => {
                    missing[item].insert(format!("{} ({}) has no value", load.name, load.kind));
                }
            }
        }
    }
    OwnLoads {
        patches,
        missing,
        edge_on,
    }
}

struct OwnLoads {
    patches: Vec<Vec<Patch>>,
    missing: Vec<BTreeSet<String>>,
    edge_on: BTreeMap<String, Vec<String>>,
}

/// Why a member over `list` tips: over the supports it lifts off (`lifted`),
/// or over its one support when it lifts off none.
fn tipping(list: &[Support], lifted: &[bool], parts: &[Part]) -> String {
    let names: Vec<&str> = list
        .iter()
        .zip(lifted)
        .filter(|(_, lifted)| **lifted)
        .map(|(support, _)| parts[support.supporter].name.as_str())
        .collect();
    if names.is_empty() {
        let mut on: Vec<&str> = list
            .iter()
            .map(|support| parts[support.supporter].name.as_str())
            .collect();
        on.dedup();
        format!(
            "it rests on {} alone and tips over it: its loads beside the bearing turn it over (EQU 0.9 G + 1.5 Q) and no joint takes a moment",
            on.join(", ")
        )
    } else {
        format!(
            "it lifts off {} and tips over its last support (EQU 0.9 G + 1.5 Q): no joint holds it down",
            names.join(", ")
        )
    }
}

/// Why `part`'s load goes nowhere: it rests on nothing, and what it `touches`
/// from the side or below holds nothing without a joint.
fn unsupported(part: &Part, touches: &BTreeSet<usize>, parts: &[Part]) -> String {
    let mut reason = format!("{} rests on nothing that carries it", part.name);
    if !touches.is_empty() {
        let names: Vec<&str> = touches
            .iter()
            .map(|other| parts[*other].name.as_str())
            .collect();
        reason.push_str(&format!(
            "; it only touches {} from the side or below, which holds nothing without a joint",
            names.join(", ")
        ));
    }
    reason
}

/// The members whose world bounds lie within [`crate::relations::GAP_LIMIT_MM`] of
/// part `item`'s, with the gap between the bounds: a part resting on nothing there
/// most likely was meant to rest on or hang from them.
fn nearly_met(item: usize, parts: &[Part], member: &[bool]) -> Vec<(usize, f64)> {
    let (low, high) = parts[item].world_bounds();
    parts
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != item && member[*other])
        .filter_map(|(other, part)| {
            let (other_low, other_high) = part.world_bounds();
            let gap = (0..3)
                .map(|axis| {
                    (low[axis] - other_high[axis])
                        .max(other_low[axis] - high[axis])
                        .max(0.0)
                        .powi(2)
                })
                .sum::<f64>()
                .sqrt();
            (gap <= crate::relations::GAP_LIMIT_MM).then_some((other, gap))
        })
        .collect()
}

/// What a member carries in all, its patches and reactions rounded for the report.
fn member_load(
    part: &Part,
    frame: &Frame,
    patches: &[Patch],
    reactions: Vec<Reaction>,
    braced: &[Bracing],
    parts: &[Part],
    missing: BTreeSet<String>,
) -> MemberLoad {
    let mut stretches: Vec<Bracing> = braced
        .iter()
        .map(|brace| Bracing {
            from: brace.from.max(0.0),
            to: brace.to.min(frame.length),
            ..*brace
        })
        .filter(|brace| brace.to >= brace.from)
        .collect();
    stretches.sort_by(|a, b| {
        (a.by, a.top)
            .cmp(&(b.by, b.top))
            .then(a.from.total_cmp(&b.from))
    });
    let mut merged: Vec<Bracing> = Vec::new();
    for brace in stretches {
        match merged.last_mut() {
            Some(last) if last.by == brace.by && last.top == brace.top && brace.from <= last.to => {
                last.to = last.to.max(brace.to);
            }
            _ => merged.push(brace),
        }
    }
    let mut total = Loads::new();
    for patch in patches {
        add(&mut total, &patch.loads_n, 1.0);
    }
    MemberLoad {
        part: part.name.clone(),
        length_mm: round(frame.length),
        axis: frame.axis.map(|v| (v * 1e6).round() / 1e6 + 0.0),
        loads_n: rounded(&total),
        patches: patches
            .iter()
            .map(|patch| Patch {
                from_mm: round(patch.from_mm),
                to_mm: round(patch.to_mm),
                loads_n: rounded(&patch.loads_n),
                source: patch.source.clone(),
                footprint: Vec::new(),
            })
            .collect(),
        reactions,
        braced_by: merged
            .into_iter()
            .map(|brace| Brace {
                part: parts[brace.by].name.clone(),
                edge: if brace.top { "top" } else { "bottom" }.to_owned(),
                from_mm: round(brace.from),
                to_mm: round(brace.to),
            })
            .collect(),
        missing: missing.into_iter().collect(),
    }
}

/// A stretch `from`..`to` of a member's top (or bottom) edge that part `by` holds.
#[derive(Clone, Copy)]
struct Bracing {
    by: usize,
    /// The grounded part `by` ties the member to there.
    anchor: usize,
    top: bool,
    from: f64,
    to: f64,
}

/// Where each member is held sideways, along it, and on which edge. A part
/// bearing on a member and on a grounded part ties the member to the ground
/// there (review 2026-11-01 PG-100), but only when it is fastened to both
/// (review 2026-11-04 PG-140, PG-141): a plank screwed to two free beams lets
/// both buckle together, and a loose lath lying along a beam's top moves with
/// it however firmly the lath's end leans on a wall. It holds the member at
/// its fasteners only, so the stretch between two is free (PG-142); a joint
/// naming none holds it at the middle of each place they touch. A fastener
/// holds only where the two parts it joins touch (review 2026-11-04 r19
/// PG-160); one elsewhere holds nothing and is listed in `stray`. Only a part
/// declared grounded anchors (PG-163: a free stud standing on the floor holds
/// nothing sideways), only a joint that fastens counts (PG-162), and the
/// anchorage must lie across the member beyond its side (PG-161). One resting
/// on a single member (a pad, a post) moves with it; one touching it from the
/// side holds no edge.
fn braced_stretches(
    supports: &[Vec<Support>],
    member: &[bool],
    anchorage: &[bool],
    parts: &[Part],
    frames: &[Frame],
    fastening: impl Fn(usize, usize) -> Option<Vec<[f64; 3]>>,
    stray: &mut Vec<StrayFastener>,
) -> Vec<Vec<Bracing>> {
    let mut braced: Vec<Vec<Bracing>> = frames.iter().map(|_| Vec::new()).collect();
    let mut reported = BTreeSet::new();
    // Where `item` is held to `other`: at the fasteners in any of their
    // contacts, each decided once for the pair over all of them (review
    // 2026-11-06 PG-181), else at the middle of each contact; None unfastened.
    let mut held_at = |item: usize, other: usize, list: &[Support]| {
        let fasteners = fastening(item, other)?;
        let pieces: Vec<&Vec<[f64; 3]>> = list
            .iter()
            .filter(|support| support.supporter == other)
            .flat_map(|support| &support.pieces)
            .collect();
        if fasteners.is_empty() {
            return Some(pieces.into_iter().filter_map(|p| middle(p)).collect());
        }
        let thinner = |normal: [f64; 3]| reach_across([&parts[item], &parts[other]], normal);
        let (inside, outside): (Vec<[f64; 3]>, Vec<[f64; 3]>) =
            fasteners.into_iter().partition(|point| {
                [item, other]
                    .iter()
                    .any(|part| parts[*part].obb().contains(*point, CONTACT_FIT_MM))
                    && pieces
                        .iter()
                        .any(|piece| over_contact(*point, piece, &thinner))
            });
        if !outside.is_empty() && reported.insert((item, other)) {
            stray.push(StrayFastener {
                parts: [parts[item].name.clone(), parts[other].name.clone()],
                at_mm: outside,
            });
        }
        Some(inside)
    };
    for (item, list) in supports.iter().enumerate() {
        let anchorages: BTreeSet<usize> = list
            .iter()
            .map(|support| support.supporter)
            .filter(|supporter| anchorage[*supporter])
            .collect();
        let anchors: Vec<(usize, [f64; 3])> = anchorages
            .into_iter()
            .filter_map(|other| {
                held_at(item, other, list)
                    .map(|points| points.into_iter().map(move |point| (other, point)))
            })
            .flatten()
            .collect();
        // Without an anchorage nothing is braced, but the fasteners into the
        // members are still looked at and reported (review 2026-11-06 P3).
        for support in list.iter().filter(|support| member[support.supporter]) {
            let frame = &frames[support.supporter];
            let Some(held_points) = held_at(item, support.supporter, list) else {
                continue;
            };
            let Some(top) = frame.edge(&support.points) else {
                continue;
            };
            // This bearing's share of the pair's points.
            let (lo, hi) = frame.range(&support.points);
            let points = held_points.into_iter().filter(|point| {
                let at = frame.along(*point);
                at >= lo - CONTACT_FIT_MM && at <= hi + CONTACT_FIT_MM
            });
            // Held sideways only from an anchorage across the member and
            // beyond its side: from one in line with it, the part sways with
            // the member (capability review 466 of PG-141).
            let Some(sideways) = linalg::normalize(cross(frame.axis, [0.0, 0.0, 1.0])) else {
                continue;
            };
            let obb = parts[support.supporter].obb();
            let side: f64 = (0..3)
                .map(|axis| dot(obb.axes[axis], sideways).abs() * obb.half[axis])
                .sum();
            // Of the anchorages that hold it, the largest across the member,
            // whatever order they are declared in (review 2026-11-06 r23 P3:
            // a small block declared before a wall holding the same point
            // was reported as the anchorage).
            let held = |point: [f64; 3]| {
                anchors
                    .iter()
                    .filter(|(_, anchor)| {
                        let path = sub(*anchor, point);
                        let lever = dot(path, sideways).abs();
                        lever >= dot(path, frame.axis).abs() && lever > side + CONTACT_FIT_MM
                    })
                    .map(|(other, _)| (*other, depth(&parts[*other], frame)))
                    .max_by(|(a, a_depth), (b, b_depth)| a_depth.total_cmp(b_depth).then(b.cmp(a)))
                    .map(|(other, _)| other)
            };
            braced[support.supporter].extend(
                points
                    .into_iter()
                    .filter_map(|point| held(point).map(|anchor| (frame.along(point), anchor)))
                    .map(|(at, anchor)| Bracing {
                        by: item,
                        anchor,
                        top,
                        from: at,
                        to: at,
                    }),
            );
        }
    }
    braced
}

/// How deep a fastener reaches from a contact of `normal` between `pair`: no
/// deeper than the thinner part is thick across it (review 2026-11-06 P3: one
/// at a wall's foot held a batten).
fn reach_across(pair: [&Part; 2], normal: [f64; 3]) -> f64 {
    pair.iter()
        .map(|part| {
            let obb = part.obb();
            (0..3)
                .map(|axis| 2.0 * dot(obb.axes[axis], normal).abs() * obb.half[axis])
                .sum::<f64>()
        })
        .fold(f64::INFINITY, f64::min)
}

/// Whether `point`, seen along the contact's normal, falls on the contact
/// polygon (even-odd, so the doubled connectors between its rings cancel) or
/// within the fit of its rim, no farther from its plane than `depth` of its normal.
fn over_contact(point: [f64; 3], polygon: &[[f64; 3]], depth: &dyn Fn([f64; 3]) -> f64) -> bool {
    let near_rim = |to_plane: &dyn Fn([f64; 3]) -> [f64; 3]| {
        (0..polygon.len()).any(|index| {
            let (a, b) = (
                to_plane(polygon[index]),
                to_plane(polygon[(index + 1) % polygon.len()]),
            );
            let p = to_plane(point);
            let ab = sub(b, a);
            let t = (dot(sub(p, a), ab) / dot(ab, ab).max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
            length(sub(p, linalg::add(a, linalg::scale(ab, t)))) <= CONTACT_FIT_MM
        })
    };
    let normal = polygon
        .iter()
        .enumerate()
        .fold([0.0; 3], |sum, (index, p)| {
            linalg::add(sum, cross(*p, polygon[(index + 1) % polygon.len()]))
        });
    let Some(normal) = linalg::normalize(normal) else {
        // A contact along a line or at a point.
        return near_rim(&|p| p);
    };
    if dot(sub(point, polygon[0]), normal).abs() > depth(normal) + CONTACT_FIT_MM {
        return false;
    }
    let flat = |p: [f64; 3]| sub(p, linalg::scale(normal, dot(p, normal)));
    let (u, v) = {
        let seed = if normal[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u = linalg::normalize(cross(normal, seed)).unwrap_or(seed);
        (u, cross(normal, u))
    };
    let (x, y) = (dot(point, u), dot(point, v));
    let mut inside = false;
    for (index, a) in polygon.iter().enumerate() {
        let b = polygon[(index + 1) % polygon.len()];
        let (ax, ay, bx, by) = (dot(*a, u), dot(*a, v), dot(b, u), dot(b, v));
        if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
            inside = !inside;
        }
    }
    inside || near_rim(&flat)
}

/// Fasteners a joint between `parts` names away from where they touch
/// (review 2026-11-04 r19 PG-160): they hold nothing.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct StrayFastener {
    pub parts: [String; 2],
    pub at_mm: Vec<[f64; 3]>,
}

/// The middle of the box of `points`, None for none.
fn middle(points: &[[f64; 3]]) -> Option<[f64; 3]> {
    (!points.is_empty()).then(|| {
        std::array::from_fn(|i| {
            let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
                (lo.min(p[i]), hi.max(p[i]))
            });
            (lo + hi) / 2.0
        })
    })
}

/// Whether each part is of a load path's `fixed` tags: fastened to what it leans on.
fn fixed_by_tag(model: &ProgramModel) -> Vec<bool> {
    model
        .parts
        .iter()
        .map(|part| {
            model
                .load_paths
                .iter()
                .any(|path| !part.tags.is_disjoint(&path.fixed_tags))
        })
        .collect()
}

/// Loads on every load-path member, when the program declares weights or area loads.
#[must_use]
pub fn loads(model: &ProgramModel) -> LoadReport {
    loads_with(model, &mut ContactFaces::default())
}

/// [`braced_stretches`] of the model: a brace is anchored by a declared
/// grounded part only, fastened by a joint() that fastens (`joined`); a
/// `fixed` tag passes weight, it ties nothing.
fn braces<'a>(
    model: &'a ProgramModel,
    supports: &[Vec<Support>],
    member: &[bool],
    frames: &[Frame],
    joined: &BTreeSet<(&str, &str)>,
    faces: &mut ContactFaces<'a>,
) -> Braces {
    let parts = &model.parts;
    let grounded = model.grounded_parts();
    let anchorage: Vec<bool> = parts
        .iter()
        .map(|part| grounded.contains(&part.name))
        .collect();
    let mut stray = Vec::new();
    let looked = std::cell::RefCell::new(BTreeSet::new());
    let mut braced = braced_stretches(
        supports,
        member,
        &anchorage,
        parts,
        frames,
        |item, held| {
            looked.borrow_mut().insert((item.min(held), item.max(held)));
            let (a, b) = (parts[item].name.as_str(), parts[held].name.as_str());
            joined
                .contains(&(a, b))
                .then(|| fasteners_between(model, a, b))
        },
        &mut stray,
    );
    stray.extend(unlooked_stray_fasteners(model, &looked.into_inner(), faces));
    let (ignored, chains) = declared_bracing(model, member, frames, joined, faces, &mut braced);
    // An anchorage smaller across the member than the member is deep (a 50 mm
    // block beside a 400 mm beam) still braces it, but is reported (review
    // 2026-11-06 P3; r23 P3: so is a rod 5 900 × 10 × 10 along it, measured
    // along its length before).
    let small: BTreeSet<(usize, usize)> = braced
        .iter()
        .enumerate()
        .flat_map(|(member, list)| list.iter().map(move |brace| (member, brace.anchor)))
        .filter(|(member, anchor)| {
            let frame = &frames[*member];
            depth(&parts[*anchor], frame) < depth(&parts[*member], frame)
        })
        .collect();
    let small = small
        .into_iter()
        .map(|(member, anchor)| [parts[member].name.clone(), parts[anchor].name.clone()])
        .collect();
    Braces {
        braced,
        stray,
        small,
        ignored,
        chains,
    }
}

/// What [`braces`] finds: each member's braced stretches, the fasteners
/// holding nothing, the small anchorages, the declarations holding nothing and
/// the parts of the diaphragm each anchored sheet belongs to.
struct Braces {
    braced: Vec<Vec<Bracing>>,
    stray: Vec<StrayFastener>,
    small: Vec<[String; 2]>,
    ignored: Vec<IgnoredBracing>,
    chains: BTreeMap<usize, BTreeSet<usize>>,
}

/// The points a brace holds at on one contact, and the contact's polygon.
type ContactHold = (Vec<[f64; 3]>, Vec<[f64; 3]>);

/// The parts `names` name: the part of each name, else the parts of that tag;
/// with the names that are neither.
fn named_parts<'n>(model: &ProgramModel, names: &'n [String]) -> (Vec<usize>, Vec<&'n str>) {
    let mut found = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
        let before = found.len();
        match model.parts.iter().position(|part| &part.name == name) {
            Some(item) => found.push(item),
            None => found.extend(
                (0..model.parts.len()).filter(|item| model.parts[*item].tags.contains(name)),
            ),
        }
        if found.len() == before {
            unknown.push(name.as_str());
        }
    }
    found.sort_unstable();
    found.dedup();
    (found, unknown)
}

/// Where `by` holds `held`, contact by contact: its points there and the
/// contact's own; at the joint's fasteners over the contact, at the middle of
/// each contact when it names none, or every `spacing` along it.
fn contact_holds<'a>(
    model: &'a ProgramModel,
    frames: &[Frame],
    faces: &mut ContactFaces<'a>,
    [by, held]: [usize; 2],
    spacing: Option<f64>,
) -> Vec<ContactHold> {
    let parts = &model.parts;
    let frame = &frames[held];
    let fasteners = fasteners_between(model, &parts[by].name, &parts[held].name);
    let thinner = |normal: [f64; 3]| reach_across([&parts[by], &parts[held]], normal);
    faces
        .contacts(&parts[by], &parts[held])
        .into_iter()
        .map(|contact| {
            let points = &contact.points_mm;
            let at: Vec<[f64; 3]> = if let Some(spacing) = spacing {
                let (lo, hi) = frame.range(points);
                let count = ((hi - lo) / spacing).floor();
                let start = lo + (hi - lo - count * spacing) / 2.0;
                let centre = middle(points).unwrap_or(frame.centre);
                let from = frame.along(centre);
                let last = start + count * spacing;
                std::iter::successors(Some(start), |at| {
                    Some(at + spacing).filter(|next| next > at)
                })
                .take_while(|at| *at <= last + spacing / 2.0)
                .map(|at| {
                    let shift = at - from;
                    std::array::from_fn(|i| centre[i] + frame.axis[i] * shift)
                })
                .collect()
            } else if fasteners.is_empty() {
                middle(points).into_iter().collect()
            } else {
                fasteners
                    .iter()
                    .copied()
                    .filter(|point| over_contact(*point, points, &thinner))
                    .collect()
            };
            (at, contact.points_mm)
        })
        .collect()
}

/// Where `a` is held to `b` by a joint that fastens: at its fasteners over
/// where they touch, at the middle of each contact when it names none (a
/// fastener beside the contact holds nothing, review 2026-11-09 PG-303).
fn fastened_at<'a>(
    model: &'a ProgramModel,
    frames: &[Frame],
    joined: &BTreeSet<(&str, &str)>,
    faces: &mut ContactFaces<'a>,
    [a, b]: [usize; 2],
) -> Vec<[f64; 3]> {
    let parts = &model.parts;
    if !joined.contains(&(parts[a].name.as_str(), parts[b].name.as_str())) {
        return Vec::new();
    }
    contact_holds(model, frames, faces, [a, b], None)
        .into_iter()
        .flat_map(|(at, _)| at)
        .collect()
}

/// The declared diaphragms that hold: each anchored sheet's first anchor, the
/// parts of the diaphragms it belongs to, and the spots where their anchors
/// hold it with how far apart they lie.
struct Diaphragms {
    sheet_anchor: BTreeMap<usize, usize>,
    chains: BTreeMap<usize, BTreeSet<usize>>,
    spots: BTreeMap<usize, (Vec<[f64; 3]>, f64)>,
}

impl Diaphragms {
    /// Why the diaphragm of `sheet` holds nothing of `held`, with what to do
    /// (review 2026-11-10 R1110-PG-401): its anchorage passes the rules of a
    /// brace's, across the member beyond its side and not a pin it turns
    /// about, and a member never braces itself as a sheet of its own diaphragm.
    fn refusal(
        &self,
        parts: &[Part],
        frames: &[Frame],
        sheet: usize,
        held: usize,
    ) -> Option<(String, String)> {
        let target = &parts[held].name;
        if self
            .chains
            .get(&sheet)
            .is_some_and(|chain| chain.contains(&held))
        {
            return Some((
                format!("{target} is a sheet or an anchor of the diaphragm that would brace it"),
                format!(
                    "Leave {target} out of diaphragm(); name the sheets fastened to it and anchor them beside it."
                ),
            ));
        }
        let frame = &frames[held];
        let (spots, spread) = self
            .spots
            .get(&sheet)
            .map_or((&[][..], 0.0), |(spots, spread)| {
                (spots.as_slice(), *spread)
            });
        if !spots
            .iter()
            .any(|spot| beyond_side(frame, &parts[held], *spot))
        {
            return Some((
                format!(
                    "its anchors hold it only in line with {target} or within its width, where it sways with {target}"
                ),
                format!(
                    "Anchor the diaphragm to a grounded part beside {target}, across it and beyond its side."
                ),
            ));
        }
        let deep = depth(&parts[held], frame);
        (spread < deep).then(|| {
            (
                format!(
                    "its anchors hold it within {} mm, less than {target} is deep ({} mm), so it turns about them",
                    spread.round(),
                    deep.round()
                ),
                format!(
                    "Anchor the diaphragm at places farther apart than {target} is deep, or along a grounded part."
                ),
            )
        })
    }
}

/// The diaphragms a program declares (`diaphragm()`, review 2026-11-08 §5.1)
/// that hold, with each one that holds nothing and why. A diaphragm holds when
/// each of its anchors is declared grounded and holds one of its sheets,
/// fastened where they touch; a sheet holds only when it reaches an anchor
/// through sheets fastened to each other where they touch (PG-301, R1110-PG-404).
fn anchored_diaphragms<'a>(
    model: &'a ProgramModel,
    frames: &[Frame],
    joined: &BTreeSet<(&str, &str)>,
    faces: &mut ContactFaces<'a>,
) -> (Diaphragms, Vec<IgnoredBracing>) {
    let parts = &model.parts;
    let grounded = model.grounded_parts();
    let is_grounded = |item: usize| grounded.contains(&parts[item].name);
    let fastened =
        |a: usize, b: usize| joined.contains(&(parts[a].name.as_str(), parts[b].name.as_str()));
    let holds = |faces: &mut ContactFaces<'a>, by: usize, held: usize, spacing: Option<f64>| {
        contact_holds(model, frames, faces, [by, held], spacing)
    };
    let tied_at = |faces: &mut ContactFaces<'a>, a: usize, b: usize| {
        fastened_at(model, frames, joined, faces, [a, b])
    };
    let mut ignored = Vec::new();
    let mut sheet_anchor: BTreeMap<usize, usize> = BTreeMap::new();
    let mut chains: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    // Each anchored sheet's anchorage spots and how far apart they lie.
    let mut sheet_spots: BTreeMap<usize, (Vec<[f64; 3]>, f64)> = BTreeMap::new();
    for diaphragm in &model.diaphragms {
        let (sheets, unknown_sheets) = named_parts(model, &diaphragm.sheets);
        let (anchors, unknown_anchors) = named_parts(model, &diaphragm.anchored_to);
        let named = |item: &usize| parts[*item].name.as_str();
        let whole = || -> Vec<String> {
            diaphragm
                .sheets
                .iter()
                .chain(&diaphragm.anchored_to)
                .cloned()
                .collect()
        };
        let mut reason = if let Some(name) = unknown_sheets.iter().chain(&unknown_anchors).next() {
            Some((
                format!("diaphragm(): {name} names no part or tag; it holds nothing"),
                format!("Name in diaphragm() a part or a tag of the model instead of {name}."),
            ))
        } else {
            anchors
                .iter()
                .find(|anchor| !is_grounded(**anchor))
                .map(|anchor| {
                    (
                        format!(
                            "diaphragm(): its anchor {} is not declared grounded; it holds nothing",
                            named(anchor)
                        ),
                        format!(
                            "Make {} with grounded = True, or anchor the diaphragm to a part that is.",
                            named(anchor)
                        ),
                    )
                })
        };
        // The sheets each anchor holds, fastened where they touch.
        let mut reached: BTreeMap<usize, usize> = BTreeMap::new();
        if reason.is_none() {
            for &anchor in &anchors {
                let held: Vec<usize> = sheets
                    .iter()
                    .copied()
                    .filter(|sheet| !tied_at(faces, *sheet, anchor).is_empty())
                    .collect();
                if held.is_empty() {
                    reason = Some((
                        format!(
                            "diaphragm(): its anchor {} is fastened to none of its sheets where they touch by a joint() that fastens; it holds nothing",
                            named(&anchor)
                        ),
                        format!(
                            "Fasten a sheet to {} with joint() of a kind that fastens, its fasteners where they touch.",
                            named(&anchor)
                        ),
                    ));
                    break;
                }
                for sheet in held {
                    reached.entry(sheet).or_insert(anchor);
                }
            }
        }
        if let Some((reason, hint)) = reason {
            ignored.push(IgnoredBracing {
                parts: whole(),
                reason,
                hint,
            });
            continue;
        }
        // From the anchored sheets on, through sheets fastened to each other
        // where they touch: sheets that only touch, as a loose board over a
        // butt joint, slide apart and hold nothing (R1110-PG-404).
        let mut queue: Vec<usize> = reached.keys().copied().collect();
        while let Some(sheet) = queue.pop() {
            let anchor = reached[&sheet];
            for &other in &sheets {
                if !reached.contains_key(&other) && !tied_at(faces, sheet, other).is_empty() {
                    reached.insert(other, anchor);
                    queue.push(other);
                }
            }
        }
        let chain: BTreeSet<usize> = sheets.iter().chain(&anchors).copied().collect();
        // Where the anchors hold the diaphragm: at the fasteners over each
        // contact, along the whole contact when its joint names none.
        let mut spots: Vec<[f64; 3]> = Vec::new();
        for &anchor in &anchors {
            for &sheet in sheets.iter().filter(|sheet| fastened(**sheet, anchor)) {
                let named =
                    !fasteners_between(model, &parts[sheet].name, &parts[anchor].name).is_empty();
                for (at, points) in holds(faces, sheet, anchor, None) {
                    if !at.is_empty() {
                        spots.extend(if named { at } else { points });
                    }
                }
            }
        }
        let spread = spots
            .iter()
            .flat_map(|a| spots.iter().map(move |b| length(sub(*a, *b))))
            .fold(0.0, f64::max);
        for &sheet in &sheets {
            match reached.get(&sheet) {
                Some(&anchor) => {
                    sheet_anchor.entry(sheet).or_insert(anchor);
                    chains.entry(sheet).or_default().extend(&chain);
                    let held = sheet_spots.entry(sheet).or_insert_with(|| (Vec::new(), 0.0));
                    held.0.extend(&spots);
                    held.1 = f64::max(held.1, spread);
                }
                None => ignored.push(IgnoredBracing {
                    parts: vec![parts[sheet].name.clone()],
                    reason: format!(
                        "diaphragm(): its sheet {} reaches none of its anchors through sheets fastened to them and to each other where they touch; it holds nothing",
                        parts[sheet].name
                    ),
                    hint: format!(
                        "Fasten {} with joint() where it touches a sheet of the diaphragm that reaches an anchor, or to an anchor.",
                        parts[sheet].name
                    ),
                }),
            }
        }
    }
    let diaphragms = Diaphragms {
        sheet_anchor,
        chains,
        spots: sheet_spots,
    };
    (diaphragms, ignored)
}

/// The bracing a program declares (`brace()`, `diaphragm()`, review
/// 2026-11-08 §5.1), added to `braced`, each declaration the model contradicts
/// or that holds nothing, and the parts of the diaphragm each anchored sheet
/// belongs to. A declaration holds only what the geometry does not refute
/// (review 2026-11-09). A diaphragm holds when each of its anchors is declared
/// grounded and holds one of its sheets, fastened where they touch; a sheet
/// holds only when it reaches an anchor through sheets fastened to each other
/// (PG-301), and then the members it is fastened to, on the edge it lies on. A
/// declared brace holds the edge it names where it touches each member it is
/// fastened to, unless the geometry reads the other edge there (PG-300), when
/// it is anchored: grounded, a sheet of a diaphragm that holds, or fastened to
/// a grounded part where they touch and across the member beyond its side, as
/// a brace the geometry finds (PG-303). Either holds at the joint's fasteners
/// over the contact, at the middle of each contact when it names none, or
/// every `spacing_mm` along it.
fn declared_bracing<'a>(
    model: &'a ProgramModel,
    member: &[bool],
    frames: &[Frame],
    joined: &BTreeSet<(&str, &str)>,
    faces: &mut ContactFaces<'a>,
    braced: &mut [Vec<Bracing>],
) -> (Vec<IgnoredBracing>, BTreeMap<usize, BTreeSet<usize>>) {
    let parts = &model.parts;
    let grounded = model.grounded_parts();
    let is_grounded = |item: usize| grounded.contains(&parts[item].name);
    let fastened =
        |a: usize, b: usize| joined.contains(&(parts[a].name.as_str(), parts[b].name.as_str()));
    let holds = |faces: &mut ContactFaces<'a>, by: usize, held: usize, spacing: Option<f64>| {
        contact_holds(model, frames, faces, [by, held], spacing)
    };
    let tied_at = |faces: &mut ContactFaces<'a>, a: usize, b: usize| {
        fastened_at(model, frames, joined, faces, [a, b])
    };
    let (anchored, mut ignored) = anchored_diaphragms(model, frames, joined, faces);
    let sheet_anchor = &anchored.sheet_anchor;
    let hold = |braced: &mut [Vec<Bracing>],
                held: usize,
                by: usize,
                anchor: usize,
                top: bool,
                at: &[[f64; 3]]| {
        let frame = &frames[held];
        braced[held].extend(at.iter().map(|point| {
            let along = frame.along(*point);
            Bracing {
                by,
                anchor,
                top,
                from: along,
                to: along,
            }
        }));
    };
    let refusal = |sheet: usize, held: usize| anchored.refusal(parts, frames, sheet, held);
    let mut refused: BTreeSet<(usize, String)> = BTreeSet::new();
    for (&sheet, &anchor) in sheet_anchor {
        for held in (0..parts.len()).filter(|held| member[*held] && fastened(sheet, *held)) {
            if let Some((why, hint)) = refusal(sheet, held) {
                if refused.insert((held, why.clone())) {
                    ignored.push(IgnoredBracing {
                        parts: vec![parts[sheet].name.clone(), parts[held].name.clone()],
                        reason: format!(
                            "diaphragm(): {why}; it holds nothing of {}",
                            parts[held].name
                        ),
                        hint,
                    });
                }
                continue;
            }
            for (at, points) in holds(faces, sheet, held, None) {
                if let Some(top) = frames[held].edge(&points) {
                    hold(braced, held, sheet, anchor, top, &at);
                }
            }
        }
    }
    let edge_name = |top: bool| if top { "top" } else { "bottom" };
    for declared in &model.declared_braces {
        let (members, unknown_members) = named_parts(model, &declared.members);
        let (bys, unknown_bys) = named_parts(model, &declared.by);
        let declared_top = declared.edge == "top";
        let mut ignore = |names: Vec<String>, reason: String, hint: String| {
            ignored.push(IgnoredBracing {
                parts: names,
                reason: format!("brace(): {reason}"),
                hint,
            });
        };
        for name in unknown_members.iter().chain(&unknown_bys) {
            ignore(
                vec![(*name).to_owned()],
                format!("{name} names no part or tag; it holds nothing"),
                format!("Name in brace() a part or a tag of the model instead of {name}."),
            );
        }
        for &by in &bys {
            // The grounded parts `by` is fastened to, with where they hold it.
            let ties: Vec<(usize, Vec<[f64; 3]>)> = if sheet_anchor.contains_key(&by) {
                Vec::new()
            } else {
                (0..parts.len())
                    .filter(|other| is_grounded(*other) && fastened(by, *other))
                    .map(|other| (other, tied_at(faces, by, other)))
                    .collect()
            };
            for &held in &members {
                let names = vec![parts[by].name.clone(), parts[held].name.clone()];
                let (brace, target) = (&parts[by].name, &parts[held].name);
                if !member[held] {
                    ignore(
                        names,
                        format!("{target} is not a load_path() member; it holds nothing"),
                        format!("Take {target} into load_path(), or leave it out of brace()."),
                    );
                    continue;
                }
                if !fastened(by, held) {
                    ignore(
                        names,
                        format!(
                            "{brace} is not fastened to {target} by a joint() that fastens; it holds nothing"
                        ),
                        format!("Join {brace} to {target} with joint() of a kind that fastens."),
                    );
                    continue;
                }
                let frame = &frames[held];
                let sheet_refusal = sheet_anchor
                    .contains_key(&by)
                    .then(|| refusal(by, held))
                    .flatten();
                // An anchor through a grounded part holds a point only across
                // the member and beyond its side: one in line with it, the
                // member's own post, or a grounded brace lying within its
                // width, sways with it (R1110-PG-400).
                let anchor_of = |point: [f64; 3]| -> Option<usize> {
                    if is_grounded(by) && reaches_beyond(frame, &parts[held], &parts[by], point) {
                        return Some(by);
                    }
                    if let Some(anchor) = sheet_anchor.get(&by) {
                        return sheet_refusal.is_none().then_some(*anchor);
                    }
                    ties.iter()
                        .find(|(_, at)| {
                            at.iter()
                                .any(|anchor| across_beyond(frame, &parts[held], point, *anchor))
                        })
                        .map(|(other, _)| *other)
                };
                let mut contradicted = false;
                let mut mid_depth = false;
                let mut anchored = Vec::new();
                let mut touched = false;
                for (at, points) in holds(faces, by, held, declared.spacing_mm) {
                    if at.is_empty() {
                        continue;
                    }
                    touched = true;
                    // Where the geometry reads an edge, the declaration may not
                    // name the other one (PG-300: laths under a beam declared
                    // on its top held both its edges).
                    if frame.edge(&points).is_some_and(|top| top != declared_top) {
                        contradicted = true;
                        continue;
                    }
                    // Nor hold an edge from mid-depth, where bracing does not
                    // stop the member twisting (R1110-PG-403: laths on the side
                    // 13 mm above the middle declared on the top held it).
                    if frame.edge(&points).is_none() && frame.height(&points).is_some() {
                        mid_depth = true;
                        continue;
                    }
                    for point in at {
                        if let Some(anchor) = anchor_of(point) {
                            anchored.push((anchor, point));
                        }
                    }
                }
                if !touched {
                    ignore(
                        names,
                        format!(
                            "{brace} touches {target} nowhere it is fastened; it holds nothing"
                        ),
                        format!(
                            "Give the joint of {brace} and {target} its fasteners=[...] where they touch."
                        ),
                    );
                    continue;
                }
                if contradicted {
                    ignore(
                        names.clone(),
                        format!(
                            "{brace} touches {target} on its {} edge, not the {} it declares; the declaration holds nothing there, the edge the geometry reads holds",
                            edge_name(!declared_top),
                            declared.edge
                        ),
                        format!(
                            "Declare edge=\"{}\" where {brace} lies, or brace the {} edge of {target} with a part touching it.",
                            edge_name(!declared_top),
                            declared.edge
                        ),
                    );
                }
                if mid_depth {
                    ignore(
                        names.clone(),
                        format!(
                            "{brace} touches the side of {target} nearer its middle height than the {} edge it declares, where bracing does not stop it twisting; it holds nothing there",
                            declared.edge
                        ),
                        format!(
                            "Move {brace} up or down so it touches {target} nearer its {} edge than its middle, or lay it on that edge.",
                            declared.edge
                        ),
                    );
                }
                if anchored.is_empty() && !contradicted && !mid_depth {
                    let (reason, hint) = if let Some((why, hint)) = sheet_refusal {
                        (
                            format!(
                                "{brace} is a sheet of a diaphragm() that holds nothing of {target}: {why}"
                            ),
                            hint,
                        )
                    } else if is_grounded(by) && ties.is_empty() {
                        (
                            format!(
                                "{brace} is grounded, but lies on {target}, mostly within its width, or reaches nowhere across it beyond its side, where it sways with {target}; it holds nothing"
                            ),
                            format!(
                                "Let {brace} reach across {target} beyond its side, or fasten it to a grounded part beside {target}."
                            ),
                        )
                    } else if ties.is_empty() {
                        (
                            format!(
                                "{brace} is not anchored: it is not declared grounded, fastened to a grounded part or a sheet of an anchored diaphragm(); it holds nothing"
                            ),
                            format!(
                                "Fasten {brace} with joint() to a grounded part or a sheet of an anchored diaphragm(), or make it grounded = True."
                            ),
                        )
                    } else {
                        (
                            format!(
                                "{brace} is not anchored: the grounded parts it is fastened to hold it nowhere they touch, or in line with {target} or within its width, where it sways with {target}; it holds nothing"
                            ),
                            format!(
                                "Fasten {brace} where it touches a grounded part beside {target}, across {target} and beyond its side."
                            ),
                        )
                    };
                    ignore(names, reason, hint);
                    continue;
                }
                for (anchor, point) in anchored {
                    hold(braced, held, by, anchor, declared_top, &[point]);
                }
            }
        }
    }
    (ignored, anchored.chains)
}

/// Whether an anchorage at `anchor` holds `point` of `member`, of `frame`,
/// sideways: across the member and beyond its side (capability review 466 of
/// PG-141), as [`braced_stretches`] takes a brace the geometry finds.
fn across_beyond(frame: &Frame, member: &Part, point: [f64; 3], anchor: [f64; 3]) -> bool {
    let Some((sideways, side)) = sideways_of(frame, member) else {
        return false;
    };
    let path = sub(anchor, point);
    let lever = dot(path, sideways).abs();
    lever >= dot(path, frame.axis).abs() && lever > side + CONTACT_FIT_MM
}

/// The level direction across `member`, of `frame`, and how far its side lies
/// from its middle that way; None for an upright one.
fn sideways_of(frame: &Frame, member: &Part) -> Option<([f64; 3], f64)> {
    let sideways = linalg::normalize(cross(frame.axis, [0.0, 0.0, 1.0]))?;
    let obb = member.obb();
    let side = (0..3)
        .map(|axis| dot(obb.axes[axis], sideways).abs() * obb.half[axis])
        .sum();
    Some((sideways, side))
}

/// Whether `spot` lies across `member` beyond its side, seen from its middle
/// (review 2026-11-10 R1110-PG-401: a diaphragm anchored in line with the
/// member, on its own posts or on a nub over it sways with it).
fn beyond_side(frame: &Frame, member: &Part, spot: [f64; 3]) -> bool {
    sideways_of(frame, member).is_some_and(|(sideways, side)| {
        dot(sub(spot, member.obb().centre), sideways).abs() > side + CONTACT_FIT_MM
    })
}

/// Whether the grounded `by` itself reaches across `member` beyond its side,
/// level with `point`, and lies mostly beside the member's width (review
/// 2026-11-10 R1110-PG-400: grounded blocks or laths on the member's top,
/// within its width or a millimetre wider, rest on it and sway with it).
fn reaches_beyond(frame: &Frame, member: &Part, by: &Part, point: [f64; 3]) -> bool {
    let Some((sideways, side)) = sideways_of(frame, member) else {
        return false;
    };
    let obb = by.obb();
    let middle = dot(sub(obb.centre, member.obb().centre), sideways);
    let width: f64 = (0..3)
        .map(|axis| dot(obb.axes[axis], sideways).abs() * obb.half[axis])
        .sum();
    let within = ((middle + width).min(side) - (middle - width).max(-side)).max(0.0);
    if within > width {
        return false;
    }
    let off = dot(sub(point, member.obb().centre), sideways);
    let reach = side + 2.0 * CONTACT_FIT_MM;
    [reach - off, -reach - off].into_iter().any(|shift| {
        obb.contains(
            std::array::from_fn(|i| point[i] + sideways[i] * shift),
            CONTACT_FIT_MM,
        )
    })
}

/// The fasteners of joints between parts no bearing looked at, as a batten
/// butting a wall end on, that do not reach from one part into the other:
/// they were not looked at at all (review 2026-11-06 r23 P3: one 1 950 mm
/// into the wall), or reach it beside where they touch (review 2026-11-07
/// P3: one 40 mm beside a batten's end in the wall it butts).
fn unlooked_stray_fasteners<'a>(
    model: &'a ProgramModel,
    looked: &BTreeSet<(usize, usize)>,
    faces: &mut ContactFaces<'a>,
) -> Vec<StrayFastener> {
    let index = |name: &str| model.parts.iter().position(|part| part.name == name);
    let mut stray: Vec<StrayFastener> = Vec::new();
    for joint in model.joints.iter().filter(|joint| joint.fastens()) {
        let (Some(a), Some(b)) = (index(&joint.parts[0]), index(&joint.parts[1])) else {
            continue;
        };
        if a == b || looked.contains(&(a.min(b), a.max(b))) {
            continue;
        }
        // In one part and over where the parts touch, no farther from it than
        // the thinner part is thick, as a sill anchor halfway up the plate
        // above the slab: as `braced_stretches` judges a looked-at pair's.
        let pair = [&model.parts[a], &model.parts[b]];
        let boxes = pair.map(Part::obb);
        let contacts = faces.contacts(pair[0], pair[1]);
        let thinner = |normal: [f64; 3]| reach_across(pair, normal);
        // Parts touching on no flat face (round ones) keep their boxes.
        let reach = boxes
            .iter()
            .flat_map(|obb| obb.half.iter().map(|half| 2.0 * half))
            .fold(f64::INFINITY, f64::min);
        let outside: Vec<[f64; 3]> = joint
            .fasteners_mm
            .iter()
            .copied()
            .filter(|point| {
                !(boxes.iter().any(|obb| obb.contains(*point, CONTACT_FIT_MM))
                    && if contacts.is_empty() {
                        boxes
                            .iter()
                            .all(|obb| obb.contains(*point, reach + CONTACT_FIT_MM))
                    } else {
                        contacts
                            .iter()
                            .any(|contact| over_contact(*point, &contact.points_mm, &thinner))
                    })
            })
            .collect();
        if outside.is_empty() {
            continue;
        }
        let parts = [joint.parts[0].clone(), joint.parts[1].clone()];
        match stray.iter_mut().find(|known| known.parts == parts) {
            Some(known) => known.at_mm.extend(outside),
            None => stray.push(StrayFastener {
                parts,
                at_mm: outside,
            }),
        }
    }
    stray
}

/// The largest size of `part` across the axis of the member of `frame`, mm:
/// its box measured along the two directions square to the axis, so a part
/// turned to the member counts with all its sides (review 2026-11-07 P3: one
/// at 45° was measured by its height alone). A side lying nearly along the
/// axis is its length, not its depth, and is left out (r26 P3: a rod 5 900 mm
/// long turned 4.2° was 432 mm across).
fn depth(part: &Part, frame: &Frame) -> f64 {
    const ALONG: f64 = 0.9;
    let obb = part.obb();
    let seed = if frame.axis[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let Some(across) = linalg::normalize(cross(frame.axis, seed)) else {
        return 0.0;
    };
    [across, cross(frame.axis, across)]
        .into_iter()
        .map(|direction| {
            (0..3)
                .filter(|axis| dot(obb.axes[*axis], frame.axis).abs() <= ALONG)
                .map(|axis| 2.0 * dot(obb.axes[axis], direction).abs() * obb.half[axis])
                .sum::<f64>()
        })
        .fold(0.0, f64::max)
}

/// The fasteners of every joint between the parts named `a` and `b`, in millimetres.
fn fasteners_between(model: &ProgramModel, a: &str, b: &str) -> Vec<[f64; 3]> {
    model
        .joints
        .iter()
        .filter(|joint| {
            let [x, y] = [joint.parts[0].as_str(), joint.parts[1].as_str()];
            joint.fastens() && ((x, y) == (a, b) || (x, y) == (b, a))
        })
        .flat_map(|joint| joint.fasteners_mm.iter().copied())
        .collect()
}

/// What each part of a model bears on, and what that read of it.
struct Bearings<'a> {
    member: Vec<bool>,
    /// Grounded or on the floor: takes its load out of the model.
    sink: Vec<bool>,
    /// What each part touches without a joint that would let it lean there.
    touches: Vec<BTreeSet<usize>>,
    frames: Vec<Frame>,
    supports: Vec<Vec<Support>>,
    /// Both orders of the parts a fastening joint ties together.
    joined_pairs: BTreeSet<(&'a str, &'a str)>,
}

/// Where each part rests, leans or hangs (a member through its bearing
/// joints too), measuring contacts into `faces`.
fn bearings<'a>(model: &'a ProgramModel, faces: &mut ContactFaces<'a>) -> Bearings<'a> {
    let parts = &model.parts;
    let floor = model.floor_z_mm.unwrap_or(0.0);
    let grounded = model.grounded_parts();
    let member: Vec<bool> = parts
        .iter()
        .map(|part| model.load_paths.iter().any(|path| path.member(part)))
        .collect();
    let sink: Vec<bool> = parts
        .iter()
        .map(|part| grounded.contains(&part.name) || on_floor(part, floor))
        .collect();
    let may_carry: Vec<bool> = parts
        .iter()
        .enumerate()
        .map(|(item, part)| {
            sink[item]
                || member[item]
                || model
                    .load_paths
                    .iter()
                    .any(|path| !part.tags.is_disjoint(&path.carrier_tags))
        })
        .collect();

    // A part resting on nothing hangs from or leans on members and the ground only,
    // so two such parts never lean on each other.
    let leans_on: Vec<bool> = (0..parts.len()).map(|i| sink[i] || member[i]).collect();
    // What each part rests on, and what else it touches from below or the side.
    let mut rests: Vec<Vec<Support>> = parts.iter().map(|_| Vec::new()).collect();
    let mut leans: Vec<Vec<Support>> = parts.iter().map(|_| Vec::new()).collect();
    // Parts some joint ties together (a rafter tie, a screw): it holds them
    // against lifting off, whatever else it does.
    let joined_pairs: BTreeSet<(&str, &str)> = model
        .joints
        .iter()
        .filter(|joint| joint.fastens())
        .flat_map(|joint| {
            let [a, b] = [joint.parts[0].as_str(), joint.parts[1].as_str()];
            [(a, b), (b, a)]
        })
        .collect();
    let joined = |a: &str, b: &str| joined_pairs.contains(&(a, b));
    // Failing both, a part that carries nothing is fixed to the carriers a joint ties
    // it to from the side or below (a gypsum board over an opening screwed through
    // the OSB to the header). A carrier never leans on such a part, so the weight
    // still runs one way down. Touching a carrier without a joint holds nothing.
    let mut fixed_to: Vec<Vec<Support>> = parts.iter().map(|_| Vec::new()).collect();
    let mut touches: Vec<BTreeSet<usize>> = parts.iter().map(|_| BTreeSet::new()).collect();
    // So too a part that carries nothing beside or under a member or the ground
    // leans on it or hangs from it only where a joint ties them; a carrier
    // (sheathing on its studs) and a part of the tags `fixed` lean as they are.
    let fixed_by_tag = fixed_by_tag(model);
    let fixed: Vec<bool> = fixed_by_tag
        .iter()
        .zip(&may_carry)
        .map(|(tagged, carries)| *tagged || *carries)
        .collect();
    let mut fix_to = |item: usize, carrier: usize, support: Support, lean: bool| {
        if lean && fixed[item] {
            leans[item].push(support);
        } else if joined(&parts[item].name, &parts[carrier].name) {
            let held = if lean { &mut leans } else { &mut fixed_to };
            held[item].push(support);
        } else {
            touches[item].insert(carrier);
        }
    };
    let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|a, b| bounds[*a].0[0].total_cmp(&bounds[*b].0[0]));
    for (position, &left) in order.iter().enumerate() {
        for &right in &order[position + 1..] {
            if bounds[right].0[0] > bounds[left].1[0] + CONTACT_FIT_MM {
                break;
            }
            let (a, b) = (&parts[left], &parts[right]);
            if model.are_alternatives(a, b)
                || (0..3).any(|axis| {
                    bounds[left].0[axis] > bounds[right].1[axis] + CONTACT_FIT_MM
                        || bounds[right].0[axis] > bounds[left].1[axis] + CONTACT_FIT_MM
                })
            {
                continue;
            }
            for contact in faces.contacts(a, b) {
                let support = |supporter| Support {
                    supporter,
                    points: contact.points_mm.clone(),
                    pieces: vec![contact.points_mm.clone()],
                    area: polygon_area(&contact.points_mm),
                    joint: None,
                };
                let (upper, lower) = if contact.normal[2] <= RESTS_ON {
                    (left, right)
                } else if contact.normal[2] >= -RESTS_ON {
                    (right, left)
                } else {
                    for (item, other) in [(left, right), (right, left)] {
                        if leans_on[other] {
                            fix_to(item, other, support(other), true);
                        } else if may_carry[other] && !may_carry[item] {
                            fix_to(item, other, support(other), false);
                        }
                    }
                    continue;
                };
                if may_carry[lower] {
                    rests[upper].push(support(lower));
                } else if leans_on[upper] {
                    // Hangs from it (a ceiling board under a joist), unless it carries it.
                    fix_to(lower, upper, support(upper), true);
                } else if may_carry[upper] {
                    fix_to(lower, upper, support(upper), false);
                }
            }
        }
    }
    let index = |name: &str| parts.iter().position(|part| part.name == name);
    for joint in model.joints.iter().filter(|joint| joint.bearing) {
        let (Some(a), Some(b)) = (index(&joint.parts[0]), index(&joint.parts[1])) else {
            continue;
        };
        let points = joint_points(joint, faces, &parts[a], &parts[b]);
        // Where the parts really touch, never the joint's own points (review
        // 2026-11-06 PG-180): a bearing joint across a gap holds nothing sideways.
        let pieces = faces
            .contacts(&parts[a], &parts[b])
            .into_iter()
            .map(|contact| contact.points_mm)
            .collect();
        rests[a].push(Support {
            supporter: b,
            pieces,
            points,
            area: 0.0,
            joint: Some(joint.name.clone()),
        });
    }
    let frames: Vec<Frame> = parts.iter().map(Frame::of).collect();
    let supports: Vec<Vec<Support>> = rests
        .into_iter()
        .zip(leans.into_iter().zip(fixed_to))
        .enumerate()
        .map(|(item, (rests, (leans, fixed_to)))| {
            if sink[item] {
                Vec::new()
            } else if member[item] {
                let frame = &frames[item];
                let runs = rests
                    .into_iter()
                    .flat_map(|support| bearing_runs(support, frame))
                    .collect();
                grouped(runs, Some(frame))
            } else if !rests.is_empty() {
                grouped(rests, None)
            } else if !leans.is_empty() {
                grouped(leans, None)
            } else {
                grouped(fixed_to, None)
            }
        })
        .collect();
    Bearings {
        member,
        sink,
        touches,
        frames,
        supports,
        joined_pairs,
    }
}

/// [`loads`] reusing the contacts the load path already measured into `faces`.
pub(crate) fn loads_with<'a>(model: &'a ProgramModel, faces: &mut ContactFaces<'a>) -> LoadReport {
    // Members to check without any load are listed as missing it (review
    // 2026-11-02 PG-105), never left out as if there were none.
    let unloaded = model.weight_scope.is_empty() && model.area_loads.is_empty();
    if model.load_paths.is_empty() || (unloaded && model.strength_classes.is_empty()) {
        return LoadReport::default();
    }
    let parts = &model.parts;
    let Bearings {
        member,
        sink,
        touches,
        frames,
        supports,
        joined_pairs,
    } = bearings(model, faces);
    let joined = |a: &str, b: &str| joined_pairs.contains(&(a, b));

    let own = own_loads(model, &frames);
    let (mut patches, mut missing) = (own.patches, own.missing);
    let Braces {
        braced,
        stray,
        small: small_anchorages,
        ignored: ignored_bracing,
        chains,
    } = braces(model, &supports, &member, &frames, &joined_pairs, faces);
    // The parts each member's bracing stands on, its braces, their anchors and
    // the diaphragms of the sheets among them (review 2026-11-09 PG-302).
    let standing: BTreeMap<String, BTreeSet<String>> = braced
        .iter()
        .enumerate()
        .filter(|(_, list)| !list.is_empty())
        .map(|(item, list)| {
            let mut on = BTreeSet::from([item]);
            for brace in list {
                on.extend([brace.by, brace.anchor]);
                on.extend(chains.get(&brace.by).into_iter().flatten());
            }
            (
                parts[item].name.clone(),
                on.into_iter()
                    .map(|part| parts[part].name.clone())
                    .collect(),
            )
        })
        .collect();

    // Pass the loads down, each part after everything it carries.
    let mut pending = vec![0usize; parts.len()];
    for list in &supports {
        for support in list {
            pending[support.supporter] += 1;
        }
    }
    let mut queue: Vec<usize> = (0..parts.len()).filter(|i| pending[*i] == 0).collect();
    let mut done = vec![false; parts.len()];
    let mut reactions: Vec<Vec<Reaction>> = parts.iter().map(|_| Vec::new()).collect();
    let mut report = LoadReport::default();
    // Loads that reach no member, by the part they only touch.
    let mut lost: Vec<(usize, String)> = Vec::new();
    while let Some(item) = queue.pop() {
        done[item] = true;
        let list = &supports[item];
        if list.is_empty() {
            if !sink[item]
                && patches[item]
                    .iter()
                    .any(|p| p.loads_n.values().any(|v| *v > 0.0))
            {
                let reason = unsupported(&parts[item], &touches[item], parts);
                if !member[item] {
                    report
                        .unassigned
                        .insert(parts[item].name.clone(), reason.clone());
                    report.touched.insert(
                        parts[item].name.clone(),
                        touches[item]
                            .iter()
                            .map(|other| parts[*other].name.clone())
                            .collect(),
                    );
                }
                missing[item].insert(reason);
                // What it touches does not carry it, and does not know it (PG-40).
                let newtons: f64 = patches[item]
                    .iter()
                    .flat_map(|patch| patch.loads_n.values())
                    .filter(|value| **value > 0.0)
                    .sum();
                for &carrier in &touches[item] {
                    lost.push((
                        carrier,
                        format!(
                            "{} touches it without a joint; its {:.2} kN is not counted",
                            parts[item].name,
                            newtons / 1000.0
                        ),
                    ));
                }
                // Nor does a member it nearly meets: hung 0.5 mm under a beam, it
                // touched nothing and the beam passed (review 2026-10-31 P3, PG-40).
                if touches[item].is_empty() {
                    for (carrier, gap) in nearly_met(item, parts, &member) {
                        lost.push((
                            carrier,
                            format!(
                                "{} lies {} mm from it, touching nothing that carries it; its {:.2} kN is not counted",
                                parts[item].name,
                                round(gap),
                                newtons / 1000.0
                            ),
                        ));
                    }
                }
            }
            continue;
        }
        // What each support takes, in pieces with where they act on the supporter
        // (None: along the whole bearing).
        let mut lifted = vec![false; list.len()];
        let mut tension = vec![None; list.len()];
        let pieces: Vec<Pieces> = if member[item] {
            let ranges: Vec<(f64, f64)> = list
                .iter()
                .map(|support| frames[item].range(&support.points))
                .collect();
            let held: Vec<bool> = list
                .iter()
                .map(|support| {
                    support.joint.is_some()
                        || joined(&parts[item].name, &parts[support.supporter].name)
                })
                .collect();
            let axis = frames[item].axis;
            let shares = beam_shares(&patches[item], &ranges, &held, axis[0].hypot(axis[1]));
            if shares.tips {
                missing[item].insert(tipping(list, &shares.lifted, parts));
            }
            lifted = shares.lifted;
            tension = shares.tension;
            shares
                .loads
                .into_iter()
                .map(|share| vec![(share, None)])
                .collect()
        } else {
            sheet_shares(&parts[item], &patches[item], list, &frames)
        };
        let carried = missing[item].clone();
        for (((support, pieces), lifted_off), tension) in
            list.iter().zip(pieces).zip(lifted).zip(tension)
        {
            let target = support.supporter;
            let (from_mm, to_mm) = frames[target].range(&support.points);
            let mut share = Loads::new();
            for (loads, _) in &pieces {
                add(&mut share, loads, 1.0);
            }
            if member[item] {
                let (lo, hi) = frames[item].range(&support.points);
                reactions[item].push(Reaction {
                    part: parts[target].name.clone(),
                    at_mm: round((lo + hi) / 2.0),
                    length_mm: round(hi - lo),
                    contact_mm2: round(support.area),
                    loads_n: rounded(&share),
                    joint: support.joint.clone(),
                    lifted_off,
                });
            }
            let entry = |joints: &mut BTreeMap<String, JointLoad>, joint: &str| {
                joints.entry(joint.to_owned()).or_insert_with(|| JointLoad {
                    loads_n: Loads::new(),
                    missing: carried.iter().cloned().collect(),
                    tension: None,
                });
            };
            if let Some(joint) = &support.joint {
                entry(&mut report.joints, joint);
                report.joints.get_mut(joint).expect("just added").loads_n = rounded(&share);
            }
            if let Some(tension) = tension {
                // The pull goes through the joint at this support, or else
                // through every joint that ties the two parts together.
                let holders: Vec<&str> = match &support.joint {
                    Some(joint) => vec![joint.as_str()],
                    None => model
                        .joints
                        .iter()
                        .filter(|joint| {
                            let names = [&parts[item].name, &parts[target].name];
                            joint.parts.iter().all(|part| names.contains(&part))
                        })
                        .map(|joint| joint.name.as_str())
                        .collect(),
                };
                for joint in holders {
                    entry(&mut report.joints, joint);
                    let load = report.joints.get_mut(joint).expect("just added");
                    match &mut load.tension {
                        Some(sum) => sum.add(&tension),
                        none => *none = Some(tension.clone()),
                    }
                }
            }
            for (loads, range) in pieces {
                // An upward pull is not passed down as relief of the supporter.
                let loads: Loads = loads
                    .into_iter()
                    .filter(|(_, value)| *value > 0.0)
                    .collect();
                if loads.is_empty() {
                    continue;
                }
                let (lo, hi) = range.unwrap_or((from_mm, to_mm));
                patches[target].push(Patch {
                    from_mm: lo.max(0.0),
                    to_mm: hi.min(frames[target].length),
                    loads_n: loads,
                    source: parts[item].name.clone(),
                    footprint: support.points.clone(),
                });
            }
            missing[target].extend(carried.iter().cloned());
            pending[target] -= 1;
            if pending[target] == 0 {
                queue.push(target);
            }
        }
    }
    // A touched part may have passed its loads down before the part touching
    // it came up, so the lost load goes down its supports here, to every part
    // and bearing joint below.
    for (carrier, reason) in lost {
        let mut below = vec![carrier];
        let mut seen = BTreeSet::new();
        while let Some(item) = below.pop() {
            if !seen.insert(item) {
                continue;
            }
            missing[item].insert(reason.clone());
            for support in &supports[item] {
                if let Some(load) = support
                    .joint
                    .as_ref()
                    .and_then(|joint| report.joints.get_mut(joint))
                    && !load.missing.contains(&reason)
                {
                    load.missing.push(reason.clone());
                }
                below.push(support.supporter);
            }
        }
    }
    for (item, part) in parts.iter().enumerate() {
        if !member[item] {
            continue;
        }
        let mut reasons = missing[item].clone();
        if !done[item] {
            reasons.insert("the parts it carries lean on each other in a loop".to_owned());
        }
        if unloaded {
            reasons.insert("no load is declared (self_weight() or area_load())".to_owned());
        }
        report.members.push(member_load(
            part,
            &frames[item],
            &patches[item],
            std::mem::take(&mut reactions[item]),
            &braced[item],
            parts,
            reasons,
        ));
    }
    report.stray_fasteners = stray;
    report.edge_on = own.edge_on;
    report.small_anchorages = small_anchorages;
    report.ignored_bracing = ignored_bracing;
    let unknown: Vec<&Joint> = model
        .joints
        .iter()
        .filter(|joint| joint.fastening() == JointFastening::Unknown)
        .collect();
    not_verified_on_unknown_kinds(&mut report, &unknown, &standing);
    report.doubtful_joint_kinds = unknown
        .into_iter()
        .map(|joint| DoubtfulJointKind {
            like: joint.misspelt_unfastened_kind().map(str::to_owned),
            parts: joint.parts.clone(),
            kind: joint.kind.clone(),
        })
        .collect();
    report
}

/// Leaves not verified each check standing on a joint of a kind the catalogue
/// does not know (review 2026-11-08 PG-241: "voľne", "kontakt" or "nichts"
/// braced a beam, which passed): that of a member it joins, a part bracing the
/// member joins, or two parts of the chain bracing it from the member to the
/// ground (`standing`: review 2026-11-09 PG-302, a diaphragm's anchor joined
/// to a strip of it so), and that of the joint itself.
fn not_verified_on_unknown_kinds(
    report: &mut LoadReport,
    unknown: &[&Joint],
    standing: &BTreeMap<String, BTreeSet<String>>,
) {
    for joint in unknown {
        let reason = format!(
            "joint {} is of kind \"{}\", not a kind the checks know to fasten or not",
            joint.name, joint.kind
        );
        let joins = |part: &str| joint.parts.iter().any(|joined| joined == part);
        for member in &mut report.members {
            let chain = standing.get(&member.part);
            let stands = joins(&member.part)
                || member.braced_by.iter().any(|brace| joins(&brace.part))
                || chain.is_some_and(|on| joint.parts.iter().all(|part| on.contains(part)));
            if stands && !member.missing.contains(&reason) {
                member.missing.push(reason.clone());
            }
        }
        if let Some(load) = report.joints.get_mut(&joint.name)
            && !load.missing.contains(&reason)
        {
            load.missing.push(reason);
        }
    }
}

#[cfg(test)]
mod shared_contact_tests {
    use super::*;
    use crate::exact::ExactShapes;

    /// The loads take the contacts the load path measured instead of
    /// measuring every pair a second time, and come out the same.
    #[test]
    fn the_loads_reuse_the_contacts_the_load_path_measured() {
        let source = "
load_path(only = [\"frame\"])
self_weight([\"frame\"])
box(\"post a\", (100, 100, 2000), material = \"C24\", tags = [\"frame\"])
box(\"post b\", (100, 100, 2000), at = (2900, 0, 0), material = \"C24\", tags = [\"frame\"])
box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
";
        let model = crate::evaluate("shared.star", source, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{error}"))
            .model;
        let mut faces = ContactFaces::default();
        crate::validate::validate_with_faces(&model, &ExactShapes::default(), &mut faces);
        let (measured, reused) = faces.measured_and_reused();
        assert!(measured > 0);
        let shared = loads_with(&model, &mut faces);
        // Every pair the loads look at was already measured.
        assert_eq!(faces.measured_and_reused(), (measured, reused + measured));
        assert_eq!(shared.members.len(), 3);
        assert_eq!(shared, loads(&model));
    }
}
