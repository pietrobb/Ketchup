//! Loads on the members of a load path (`material_weight()`, `weight_scope()`,
//! `area_load()`). Every part passes its own weight, the area loads on it and
//! what it carries down to the parts it rests on: a member (a beam, a stud) to
//! what it rests on or hangs from through bearing joints, along its longest
//! axis by the lever rule (simple spans between neighbouring supports, an
//! overhang onto its last support); any other part (a deck, insulation, a
//! ceiling board) shares its load among the parts it rests on, or failing that
//! hangs from or leans against, by contact area. Loads are characteristic and
//! unfactored, by kind. Missing data is listed with the members it reaches,
//! never guessed.

use crate::contact::ContactFaces;
use crate::eval::TOLERANCE_MM;
use crate::load_path::{centre_of_mass, on_floor};
use crate::model::{Part, ProgramModel, ProgramPartBody};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const LOAD_KINDS: [&str; 4] = ["permanent", "imposed", "roof", "snow"];
const GRAVITY: f64 = 9.81;
/// A contact face looking at least this much downward lets the part rest on the other.
const RESTS_ON: f64 = -0.5;

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

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Area of a planar polygon in space.
fn polygon_area(points: &[[f64; 3]]) -> f64 {
    let mut sum = [0.0; 3];
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        sum[0] += a[1] * b[2] - a[2] * b[1];
        sum[1] += a[2] * b[0] - a[0] * b[2];
        sum[2] += a[0] * b[1] - a[1] * b[0];
    }
    dot(sum, sum).sqrt() / 2.0
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
    mm3 * 1e-9
}

/// Largest face of a slab-like part, m².
fn slab_area_m2(part: &Part) -> f64 {
    let size = extents(part);
    let thin = (0..3)
        .min_by(|a, b| size[*a].total_cmp(&size[*b]))
        .unwrap_or(2);
    face_area_mm2(part, thin) * 1e-6
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
    face_area_mm2(part, axis) * 1e-6 * if projected { up } else { 1.0 }
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

/// Merges the supports on one supporter, keeping a joint's name.
fn grouped(supports: Vec<Support>) -> Vec<Support> {
    let mut groups: Vec<Support> = Vec::new();
    for support in supports {
        match groups
            .iter_mut()
            .find(|group| group.supporter == support.supporter)
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

/// A beam's loads shared among its supports at `positions` (same order).
fn beam_shares(patches: &[Patch], positions: &[f64]) -> Vec<Loads> {
    let mut shares = vec![Loads::new(); positions.len()];
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by(|a, b| positions[*a].total_cmp(&positions[*b]));
    // Supports at one place share what reaches it equally.
    let mut clusters: Vec<(f64, Vec<usize>)> = Vec::new();
    for index in order {
        match clusters.last_mut() {
            Some((at, members)) if positions[index] - *at <= 1.0 => members.push(index),
            _ => clusters.push((positions[index], vec![index])),
        }
    }
    let mut give = |cluster: usize, loads: &Loads, factor: f64| {
        let members = &clusters[cluster].1;
        for member in members {
            add(&mut shares[*member], loads, factor / members.len() as f64);
        }
    };
    let last = clusters.len() - 1;
    for patch in patches {
        let (a, b) = (
            patch.from_mm.min(patch.to_mm),
            patch.from_mm.max(patch.to_mm),
        );
        let mut cuts = vec![a];
        cuts.extend(
            clusters
                .iter()
                .map(|(at, _)| *at)
                .filter(|at| *at > a && *at < b),
        );
        cuts.push(b);
        let pieces: Vec<(f64, f64)> = if b - a <= f64::EPSILON {
            vec![(a, 1.0)]
        } else {
            cuts.windows(2)
                .map(|pair| ((pair[0] + pair[1]) / 2.0, (pair[1] - pair[0]) / (b - a)))
                .collect()
        };
        for (at, fraction) in pieces {
            if at <= clusters[0].0 {
                give(0, &patch.loads_n, fraction);
            } else if at >= clusters[last].0 {
                give(last, &patch.loads_n, fraction);
            } else {
                let k = clusters
                    .windows(2)
                    .position(|pair| pair[0].0 <= at && at <= pair[1].0)
                    .unwrap_or(0);
                let t = (at - clusters[k].0) / (clusters[k + 1].0 - clusters[k].0);
                give(k, &patch.loads_n, fraction * (1.0 - t));
                give(k + 1, &patch.loads_n, fraction * t);
            }
        }
    }
    shares
}

/// Where loads land on a supporter: (loads, range along it or None for its whole bearing).
type Pieces = Vec<(Loads, Option<(f64, f64)>)>;

/// A part that is no member (a deck, a sheet) shares its own loads among its
/// supports by contact area, and passes each load it carries to the supports
/// under where that load bears on it seen from above (searching up to 1 m
/// around it), at that place along them.
fn sheet_shares(patches: &[Patch], list: &[Support], frames: &[Frame]) -> Vec<Pieces> {
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
        let areas: Vec<f64> = list.iter().map(|s| polygon_area(&s.points)).collect();
        let sum: f64 = areas.iter().sum();
        for (k, area) in areas.iter().enumerate() {
            let factor = if sum > 0.0 {
                area / sum
            } else {
                1.0 / list.len() as f64
            };
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
    for joint in model.joints.iter().filter(|joint| joint.bearing) {
        let (Some(a), Some(b)) = (index(&joint.parts[0]), index(&joint.parts[1])) else {
            continue;
        };
        let points = if joint.fasteners_mm.is_empty() {
            faces.contact(&parts[a], &parts[b]).map_or_else(
                || vec![centre_of_mass(&parts[a])],
                |contact| contact.points_mm,
            )
        } else {
            joint.fasteners_mm.clone()
        };
        rests[a].push(Support {
            supporter: b,
            points,
            area: 0.0,
            joint: Some(joint.name.clone()),
        });
    }
    let supports: Vec<Vec<Support>> = rests
        .into_iter()
        .zip(leans)
        .enumerate()
        .map(|(item, (rests, leans))| {
            if sink[item] {
                Vec::new()
            } else if member[item] || !rests.is_empty() {
                grouped(rests)
            } else {
                grouped(leans)
            }
        })
        .collect();

    // Own loads: weight of the parts in scope and the area loads on them.
    let frames: Vec<Frame> = parts.iter().map(Frame::of).collect();
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
                missing[item].insert(format!(
                    "{} rests on nothing that carries it",
                    parts[item].name
                ));
            }
            continue;
        }
        // What each support takes, in pieces with where they act on the supporter
        // (None: along the whole bearing).
        let pieces: Vec<Pieces> = if member[item] {
            let positions: Vec<f64> = list
                .iter()
                .map(|support| {
                    let (lo, hi) = frames[item].range(&support.points);
                    (lo + hi) / 2.0
                })
                .collect();
            beam_shares(&patches[item], &positions)
                .into_iter()
                .map(|share| vec![(share, None)])
                .collect()
        } else {
            sheet_shares(&patches[item], list, &frames)
        };
        let carried = missing[item].clone();
        for (support, pieces) in list.iter().zip(pieces) {
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
