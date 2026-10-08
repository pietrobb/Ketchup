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
use crate::eval::TOLERANCE_MM;
use crate::load_path::{joint_points, on_floor};
use crate::model::{Part, ProgramModel, ProgramPartBody};
use ketchup_geometry::linalg::{self, cross, dot, length};
use serde::Serialize;
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MaterialWeight {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kg_m3: Option<f64>,
    /// Per m² of the part's largest face (a layer modelled as one slab).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kg_m2: Option<f64>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize)]
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lifted_off: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemberLoad {
    pub part: String,
    pub length_mm: f64,
    /// Unit direction of the member's longest axis; positions run along it from one end.
    pub axis: [f64; 3],
    /// Everything on the member, by kind.
    pub loads_n: Loads,
    pub patches: Vec<Patch>,
    pub reactions: Vec<Reaction>,
    /// Why the loads are incomplete; empty when every input was known.
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JointLoad {
    pub loads_n: Loads,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LoadReport {
    pub members: Vec<MemberLoad>,
    /// Load passed through each bearing joint, by joint name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub joints: BTreeMap<String, JointLoad>,
    /// Loaded parts (no members) that rest on nothing that carries them, with
    /// why: their load reaches no member, so no check is complete.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub unassigned: BTreeMap<String, String>,
}

impl LoadReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

fn add(into: &mut Loads, from: &Loads, factor: f64) {
    for (kind, value) in from {
        *into.entry(kind.clone()).or_default() += value * factor;
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
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

/// A member's longest axis: positions from 0 at one end to `length` at the other.
struct Frame {
    centre: [f64; 3],
    axis: [f64; 3],
    length: f64,
}

impl Frame {
    fn of(part: &Part) -> Self {
        let size = extents(part);
        let long = (0..3)
            .max_by(|a, b| size[*a].total_cmp(&size[*b]))
            .unwrap_or(0);
        let (min, max) = part.local_bounds();
        Self {
            centre: part.to_world(std::array::from_fn(|i| (min[i] + max[i]) / 2.0)),
            axis: axes(part)[long],
            length: size[long],
        }
    }

    fn along(&self, point: [f64; 3]) -> f64 {
        dot(sub(point, self.centre), self.axis) + self.length / 2.0
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
        .any(|pair| pair[1] - pair[0] > TOLERANCE_MM)
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
                points,
                joint: None,
            }
        })
        .collect()
}

/// A beam's loads shared among its supports, which bear on `ranges` along it
/// (same order), continuous over the inner ones. A support `held` by a joint
/// may pull the member down (a negative share); any other lets go of it.
fn beam_shares(patches: &[Patch], ranges: &[(f64, f64)], held: &[bool]) -> Shares {
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
    // A support that would have to pull the member down and holds it by no
    // joint lets go; the rest take its share. The worst one first.
    let held_cluster = |c: usize| clusters[c].1.iter().any(|member| held[*member]);
    let mut active: Vec<usize> = (0..clusters.len()).collect();
    let mut reactions = solve(&active);
    while active.len() > 1 {
        let worst = (0..active.len())
            .filter(|k| !held_cluster(active[*k]))
            .map(|k| (k, holding_down(&reactions[k])))
            .filter(|(_, value)| *value < 0.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((k, _)) = worst else {
            break;
        };
        active.remove(k);
        reactions = solve(&active);
    }
    let mut shares = vec![Loads::new(); ranges.len()];
    let mut lifted = vec![true; ranges.len()];
    for (k, cluster) in active.iter().enumerate() {
        let members = &clusters[*cluster].1;
        for member in members {
            add(
                &mut shares[*member],
                &reactions[k],
                1.0 / members.len() as f64,
            );
            lifted[*member] = false;
        }
    }
    // Lifting off down to one support of several, the member tips over it.
    let tips = clusters.len() > 1 && active.len() == 1;
    Shares {
        loads: shares,
        lifted,
        tips,
    }
}

/// What a beam passes to each of its supports.
struct Shares {
    loads: Vec<Loads>,
    /// The member lifts off this support (it carries nothing).
    lifted: Vec<bool>,
    /// It lifted off all but one of its supports and tips over that one.
    tips: bool,
}

/// The design value of a reaction for static equilibrium (EN 1990 table
/// A1.2(A)): permanent 0.9 holding down and 1.1 lifting, variable loads only
/// lifting, by 1.5. Negative: the member lifts off.
fn holding_down(reaction: &Loads) -> f64 {
    reaction
        .iter()
        .map(|(kind, value)| match (kind.as_str(), *value >= 0.0) {
            ("permanent", true) => 0.9 * value,
            ("permanent", false) => 1.1 * value,
            (_, true) => 0.0,
            (_, false) => 1.5 * value,
        })
        .sum()
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

/// Loads on every load-path member, when the program declares weights or area loads.
#[must_use]
pub fn loads(model: &ProgramModel) -> LoadReport {
    if model.load_paths.is_empty() || (model.weight_scope.is_empty() && model.area_loads.is_empty())
    {
        return LoadReport::default();
    }
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
    let mut faces = ContactFaces::default();
    let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|a, b| bounds[*a].0[0].total_cmp(&bounds[*b].0[0]));
    for (position, &left) in order.iter().enumerate() {
        for &right in &order[position + 1..] {
            if bounds[right].0[0] > bounds[left].1[0] + TOLERANCE_MM {
                break;
            }
            let (a, b) = (&parts[left], &parts[right]);
            if model.are_alternatives(a, b)
                || (0..3).any(|axis| {
                    bounds[left].0[axis] > bounds[right].1[axis] + TOLERANCE_MM
                        || bounds[right].0[axis] > bounds[left].1[axis] + TOLERANCE_MM
                })
            {
                continue;
            }
            for contact in faces.contacts(a, b) {
                let support = |supporter| Support {
                    supporter,
                    points: contact.points_mm.clone(),
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
                            leans[item].push(support(other));
                        }
                    }
                    continue;
                };
                if may_carry[lower] {
                    rests[upper].push(support(lower));
                } else if leans_on[upper] {
                    // Hangs from it (a ceiling board under a joist), unless it carries it.
                    leans[lower].push(support(upper));
                }
            }
        }
    }
    let index = |name: &str| parts.iter().position(|part| part.name == name);
    // Parts some joint ties together (a rafter tie, a screw): it holds them
    // against lifting off, whatever else it does.
    let joined_pairs: BTreeSet<(&str, &str)> = model
        .joints
        .iter()
        .flat_map(|joint| {
            let [a, b] = [joint.parts[0].as_str(), joint.parts[1].as_str()];
            [(a, b), (b, a)]
        })
        .collect();
    let joined = |a: &str, b: &str| joined_pairs.contains(&(a, b));
    for joint in model.joints.iter().filter(|joint| joint.bearing) {
        let (Some(a), Some(b)) = (index(&joint.parts[0]), index(&joint.parts[1])) else {
            continue;
        };
        let points = joint_points(joint, &mut faces, &parts[a], &parts[b]);
        rests[a].push(Support {
            supporter: b,
            points,
            area: 0.0,
            joint: Some(joint.name.clone()),
        });
    }
    let frames: Vec<Frame> = parts.iter().map(Frame::of).collect();
    let supports: Vec<Vec<Support>> = rests
        .into_iter()
        .zip(leans)
        .enumerate()
        .map(|(item, (rests, leans))| {
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
            } else {
                grouped(leans, None)
            }
        })
        .collect();

    // Own loads: weight of the parts in scope and the area loads on them.
    let mut patches: Vec<Vec<Patch>> = parts.iter().map(|_| Vec::new()).collect();
    let mut missing: Vec<BTreeSet<String>> = parts.iter().map(|_| BTreeSet::new()).collect();
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
    while let Some(item) = queue.pop() {
        done[item] = true;
        let list = &supports[item];
        if list.is_empty() {
            if !sink[item]
                && patches[item]
                    .iter()
                    .any(|p| p.loads_n.values().any(|v| *v > 0.0))
            {
                let reason = format!("{} rests on nothing that carries it", parts[item].name);
                if !member[item] {
                    report
                        .unassigned
                        .insert(parts[item].name.clone(), reason.clone());
                }
                missing[item].insert(reason);
            }
            continue;
        }
        // What each support takes, in pieces with where they act on the supporter
        // (None: along the whole bearing).
        let mut lifted = vec![false; list.len()];
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
            let shares = beam_shares(&patches[item], &ranges, &held);
            if shares.tips {
                let names: Vec<&str> = list
                    .iter()
                    .zip(&shares.lifted)
                    .filter(|(_, lifted)| **lifted)
                    .map(|(support, _)| parts[support.supporter].name.as_str())
                    .collect();
                missing[item].insert(format!(
                    "it lifts off {} and tips over its last support (EQU 0.9 G + 1.5 Q): no joint holds it down",
                    names.join(", ")
                ));
            }
            lifted = shares.lifted;
            shares
                .loads
                .into_iter()
                .map(|share| vec![(share, None)])
                .collect()
        } else {
            sheet_shares(&parts[item], &patches[item], list, &frames)
        };
        let carried = missing[item].clone();
        for ((support, pieces), lifted_off) in list.iter().zip(pieces).zip(lifted) {
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
            if let Some(joint) = &support.joint {
                report.joints.insert(
                    joint.clone(),
                    JointLoad {
                        loads_n: rounded(&share),
                        missing: carried.iter().cloned().collect(),
                    },
                );
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
    for (item, part) in parts.iter().enumerate() {
        if !member[item] {
            continue;
        }
        let mut reasons = missing[item].clone();
        if !done[item] {
            reasons.insert("the parts it carries lean on each other in a loop".to_owned());
        }
        let mut total = Loads::new();
        for patch in &patches[item] {
            add(&mut total, &patch.loads_n, 1.0);
        }
        report.members.push(MemberLoad {
            part: part.name.clone(),
            length_mm: round(frames[item].length),
            axis: frames[item].axis.map(|v| (v * 1e6).round() / 1e6 + 0.0),
            loads_n: rounded(&total),
            patches: patches[item]
                .iter()
                .map(|patch| Patch {
                    from_mm: round(patch.from_mm),
                    to_mm: round(patch.to_mm),
                    loads_n: rounded(&patch.loads_n),
                    source: patch.source.clone(),
                    footprint: Vec::new(),
                })
                .collect(),
            reactions: std::mem::take(&mut reactions[item]),
            missing: reasons.into_iter().collect(),
        });
    }
    report
}
