//! Timber member check after EN 1995-1-1 (EC5) on the loads of the load path:
//! bending, shear (with notches at supports), deflection and compression
//! perpendicular to the grain at bearings for beams; compression with
//! buckling for columns. Strength classes come from `timber_strength()`;
//! loads are combined after EN 1990 6.10 with recommended factors. A member
//! whose inputs are incomplete reports its utilization from the known loads
//! as `not_verified`, never as passing.

use crate::continuous_span;
use crate::loads::{LOAD_KINDS, LoadReport, Loads, MemberLoad, axes, extents};
use crate::model::{Part, ProgramModel, ProgramPartBody, ProgramProfileSegment};
use ketchup_geometry::linalg::{dot, length};
use ketchup_tolerance::ROUNDING;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TimberClass {
    pub strength_class: String,
    pub service_class: u8,
    pub glulam: bool,
    pub fm_k: f64,
    pub fv_k: f64,
    pub fc0_k: f64,
    pub fc90_k: f64,
    pub e0_mean: f64,
    pub e0_05: f64,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Check {
    /// bending, shear, shear_notch, deflection_inst, deflection_fin, compression, bearing.
    pub name: &'static str,
    pub utilization: f64,
    pub combination: String,
    /// The span ("span 70-3068 mm") or the part it bears on / that bears on it.
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemberCheck {
    pub part: String,
    /// "beam" or "column".
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength_class: Option<String>,
    /// Width and depth (bending) or the two sides (column).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_mm: Option<[f64; 2]>,
    /// Largest utilization of the checks, from the known loads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub governing: Option<String>,
    pub checks: Vec<Check>,
    /// "pass", "fail" (some check over 1 even with incomplete loads) or "not_verified".
    pub status: &'static str,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DesignReport {
    pub basis: Vec<&'static str>,
    pub members: Vec<MemberCheck>,
}

impl DesignReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Counts by status, the most utilized member and the first failing ones.
    #[must_use]
    pub fn summary(&self) -> DesignSummary {
        let count = |status: &str| self.members.iter().filter(|m| m.status == status).count();
        let highest = self
            .members
            .iter()
            .filter(|m| m.utilization.is_some())
            .max_by(|a, b| {
                a.utilization
                    .unwrap_or(0.0)
                    .total_cmp(&b.utilization.unwrap_or(0.0))
            })
            .map(|m| Highest {
                part: m.part.clone(),
                utilization: m.utilization.unwrap_or(0.0),
                governing: m.governing.clone().unwrap_or_default(),
                status: m.status,
            });
        DesignSummary {
            members: self.members.len(),
            with_utilization: self
                .members
                .iter()
                .filter(|m| m.utilization.is_some())
                .count(),
            pass: count("pass"),
            fail: count("fail"),
            not_verified: count("not_verified"),
            highest,
            failing: self
                .members
                .iter()
                .filter(|m| m.status == "fail")
                .take(10)
                .map(|m| m.part.clone())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Highest {
    pub part: String,
    pub utilization: f64,
    pub governing: String,
    pub status: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DesignSummary {
    pub members: usize,
    pub with_utilization: usize,
    pub pass: usize,
    pub fail: usize,
    pub not_verified: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highest: Option<Highest>,
    /// The first ten failing members.
    pub failing: Vec<String>,
}

/// What the check leaves out: a pass covers vertical (gravity) loads only.
pub const NOT_CHECKED: [&str; 3] = [
    "wind",
    "horizontal_stability",
    "bending_with_axial_force_6.23_6.24",
];

pub const BASIS: [&str; 9] = [
    "scope: vertical (gravity) loads only, permanent, imposed, roof and snow; wind, horizontal stability (bracing, racking) and bending combined with axial force (EN 1995-1-1 6.2.3, 6.2.4, 6.3.2 (6.23, 6.24)) are not checked",
    "EN 1990 6.4.3.2 (6.10): 1.35 G + 1.5 Q1 + 1.5 psi0 Qi; SLS characteristic, w_fin with kdef and psi2 (recommended values, the national annex may differ)",
    "psi0/psi2 (EN 1990 table A1.1): imposed category A 0.7/0.3, roof category H 0/0 and never with snow (EN 1991-1-1 3.3.2), snow below 1000 m 0.5/0",
    "load duration (EN 1995-1-1 2.3.1.2): permanent G, medium-term imposed, short-term roof and snow; kmod table 3.1, kdef table 3.2, gamma_M 1.3 solid / 1.25 glulam (table 2.3)",
    "bending with kh (3.2, 3.3), kcrit = 1: members assumed braced against lateral torsional buckling by the decking",
    "shear with kcr = 0.67 (6.1.7); notches at bearings with kv (6.5.2), right-angled, the reaction at half the bearing length from the notch",
    "compression perpendicular to the grain with kc90 = 1 on the contact area (6.1.5, without the 30 mm extension); compression with kc (6.3.2) on the full member length about both axes, sheathing not counted, and without kc on the net section at notches in a side (6.1.4) more than twice the member width from its ends; axial force in sloped beams neglected",
    "moments, shear and deflection: spans continuous over the supports (three-moment equation, constant section) and cantilevers clamped at their support, as the loads are passed down; w_inst <= l/300 and w_fin <= l/250, cantilevers l/150 and l/125 (EN 1995-1-1 table 7.2, the lenient ends of the ranges)",
    "a computed check, not an authorized structural design",
];

/// Load duration classes: permanent, long, medium, short, instantaneous.
const KMOD: [[f64; 5]; 2] = [[0.6, 0.7, 0.8, 0.9, 1.1], [0.5, 0.55, 0.65, 0.7, 0.9]];
const KDEF: [f64; 3] = [0.6, 0.8, 2.0];
const GAMMA_G: f64 = 1.35;
const GAMMA_Q: f64 = 1.5;
const KCR: f64 = 0.67;
/// A bearing longer than this supports the member along it instead of at a point.
const BED_MM: f64 = 400.0;
/// A member closer to vertical than this (cosine of its slope) is a column.
const COLUMN_COS: f64 = 0.44;
const STEPS: usize = 64;

/// Duration class, psi0 and psi2 of each load kind.
fn action(kind: &str) -> (usize, f64, f64) {
    match kind {
        "imposed" => (2, 0.7, 0.3),
        "roof" => (3, 0.0, 0.0),
        "snow" => (3, 0.5, 0.0),
        _ => (0, 0.0, 0.0),
    }
}

struct Combination {
    label: String,
    factors: Loads,
    duration: usize,
}

fn factor(factors: &Loads, loads: &Loads) -> f64 {
    loads
        .iter()
        .map(|(kind, value)| factors.get(kind).copied().unwrap_or(0.0) * value)
        .sum()
}

fn label(parts: &[(f64, &str)]) -> String {
    parts
        .iter()
        .map(|(f, kind)| format!("{} {kind}", (f * 1000.0).round() / 1000.0))
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Ultimate (6.10) and characteristic/final serviceability combinations of
/// the variable kinds present.
fn combinations(
    present: &[&'static str],
    kdef: f64,
) -> (Vec<Combination>, Vec<(Combination, Loads)>) {
    let mut uls = vec![Combination {
        label: label(&[(GAMMA_G, "permanent")]),
        factors: Loads::from([("permanent".to_owned(), GAMMA_G)]),
        duration: 0,
    }];
    let mut sls = vec![(
        Combination {
            label: "permanent".to_owned(),
            factors: Loads::from([("permanent".to_owned(), 1.0)]),
            duration: 0,
        },
        Loads::from([("permanent".to_owned(), 1.0 + kdef)]),
    )];
    for mask in 1..(1usize << present.len()) {
        let set: Vec<&str> = (0..present.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| present[i])
            .collect();
        if set.contains(&"roof") && set.contains(&"snow") {
            continue;
        }
        let duration = set.iter().map(|kind| action(kind).0).max().unwrap_or(0);
        for lead in &set {
            let mut parts = vec![(GAMMA_G, "permanent")];
            let mut char_parts = vec![(1.0, "permanent")];
            let mut fin = Loads::from([("permanent".to_owned(), 1.0 + kdef)]);
            for kind in &set {
                let (_, psi0, psi2) = action(kind);
                if kind == lead {
                    parts.push((GAMMA_Q, kind));
                    char_parts.push((1.0, kind));
                    fin.insert((*kind).to_owned(), 1.0 + psi2 * kdef);
                } else {
                    parts.push((GAMMA_Q * psi0, kind));
                    char_parts.push((psi0, kind));
                    fin.insert((*kind).to_owned(), psi0 + psi2 * kdef);
                }
            }
            let to_loads = |parts: &[(f64, &str)]| {
                parts
                    .iter()
                    .map(|(f, kind)| ((*kind).to_owned(), *f))
                    .collect::<Loads>()
            };
            uls.push(Combination {
                label: label(&parts),
                factors: to_loads(&parts),
                duration,
            });
            sls.push((
                Combination {
                    label: label(&char_parts),
                    factors: to_loads(&char_parts),
                    duration,
                },
                fin,
            ));
        }
    }
    (uls, sls)
}

/// gamma_M of connections (EN 1995-1-1 table 2.3).
pub(crate) const GAMMA_M_CONNECTION: f64 = 1.3;

/// kmod (table 3.1) of a service class and load-duration class.
pub(crate) fn kmod(service_class: u8, duration: usize) -> f64 {
    KMOD[usize::from(service_class == 3)][duration]
}

/// `loads` in every ultimate combination (6.10): design value, load-duration
/// class and label.
pub(crate) fn ultimate(loads: &Loads) -> Vec<(f64, usize, String)> {
    let present: Vec<&'static str> = LOAD_KINDS[1..]
        .iter()
        .copied()
        .filter(|kind| loads.get(*kind).is_some_and(|v| *v != 0.0))
        .collect();
    combinations(&present, 0.0)
        .0
        .into_iter()
        .map(|c| (factor(&c.factors, loads), c.duration, c.label))
        .collect()
}

fn horizontal(v: [f64; 3]) -> f64 {
    length([v[0], v[1], 0.0])
}

/// Total length of the profile's cuts by the line local[long] = u.
fn chord(segments: &[ProgramProfileSegment], long: usize, u: f64) -> f64 {
    let other = 1 - long;
    let mut hits: Vec<f64> = segments
        .iter()
        .filter_map(|s| {
            let (a, b) = (s.start_mm, s.end_mm);
            let crosses = (a[long] <= u && u < b[long]) || (b[long] <= u && u < a[long]);
            crosses.then(|| a[other] + (u - a[long]) / (b[long] - a[long]) * (b[other] - a[other]))
        })
        .collect();
    hits.sort_by(f64::total_cmp);
    hits.chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| pair[1] - pair[0])
        .sum()
}

/// Area, second moment about the centroidal axis across `depth` (a local
/// profile axis) and the farthest fibre's distance from it, of a line profile.
fn section(segments: &[ProgramProfileSegment], depth: usize) -> (f64, f64, f64) {
    let (mut area, mut first, mut second) = (0.0, 0.0, 0.0);
    for s in segments {
        let (a, b) = (s.start_mm, s.end_mm);
        let cross = a[1 - depth] * b[depth] - b[1 - depth] * a[depth];
        area += cross / 2.0;
        first += (a[depth] + b[depth]) * cross / 6.0;
        second += (a[depth] * a[depth] + a[depth] * b[depth] + b[depth] * b[depth]) * cross / 12.0;
    }
    let centre = first / area;
    let inertia = (second - area * centre * centre).abs();
    let reach = segments
        .iter()
        .map(|s| (s.start_mm[depth] - centre).abs())
        .fold(0.0, f64::max);
    (area.abs(), inertia, reach)
}

/// A member's section and directions.
struct Shape {
    /// Unit world direction of the member's grain.
    grain: [f64; 3],
    /// Width and depth for bending; the two sides of a column.
    b: f64,
    h: f64,
    /// Section area, second moment and modulus about the bending axis.
    area: f64,
    /// Area of the thinnest section away from the ends (notches cut into a side).
    net_area: f64,
    inertia: f64,
    modulus: f64,
    /// True length per unit of the load frame's position.
    arc: f64,
    /// Depth across the member at a load-frame position, when it varies (notches).
    depth_at: Option<Box<dyn Fn(f64) -> f64>>,
}

fn shape(part: &Part, frame_axis: [f64; 3]) -> Result<Shape, &'static str> {
    let ProgramPartBody::Extrusion {
        segments,
        distance_mm,
        ..
    } = &part.body
    else {
        return Err("the member is not a straight extrusion");
    };
    if !segments.iter().all(ProgramProfileSegment::is_line) {
        return Err("the member's profile has curved edges");
    }
    let size = extents(part);
    let world = axes(part);
    let long = (0..3)
        .max_by(|a, b| size[*a].total_cmp(&size[*b]))
        .unwrap_or(0);
    if long == 2 {
        // The profile is the cross-section; depth along its more vertical local axis.
        let depth = usize::from(world[1][2].abs() >= world[0][2].abs());
        let (area, inertia, reach) = section(segments, depth);
        return Ok(Shape {
            grain: frame_axis,
            b: size[1 - depth],
            h: size[depth],
            area,
            net_area: area,
            inertia,
            modulus: inertia / reach,
            arc: 1.0,
            depth_at: None,
        });
    }
    // The profile is a side or plan view: the grain runs along its longest edge.
    let edge = segments
        .iter()
        .map(|s| [s.end_mm[0] - s.start_mm[0], s.end_mm[1] - s.start_mm[1]])
        .max_by(|a, b| a[0].hypot(a[1]).total_cmp(&b[0].hypot(b[1])))
        .unwrap_or([1.0, 0.0]);
    let norm = edge[0].hypot(edge[1]);
    let d = [edge[0] / norm, edge[1] / norm];
    let along = d[long].abs();
    if along < 0.2 {
        return Err("the member's grain does not follow its length");
    }
    let mut grain: [f64; 3] = std::array::from_fn(|i| d[0] * world[0][i] + d[1] * world[1][i]);
    if dot(frame_axis, grain) < 0.0 {
        grain = grain.map(|v| -v);
    }
    let across: [f64; 3] = std::array::from_fn(|i| -d[1] * world[0][i] + d[0] * world[1][i]);
    let (min, _) = part.local_bounds();
    let start = min[long];
    let length = size[long];
    let samples = 41;
    let thickest = (0..samples)
        .map(|k| {
            chord(
                segments,
                long,
                start + length * (k as f64 + 0.5) / samples as f64,
            )
        })
        .fold(0.0, f64::max)
        * along;
    // The chord is linear between the profile's corners, so its least value lies at
    // one of them; the ends (cut to a slope, seated) stay out, twice the width deep.
    let mut corners: Vec<f64> = segments.iter().map(|s| s.start_mm[long]).collect();
    corners.sort_by(f64::total_cmp);
    corners.dedup_by(|a, b| (*a - *b).abs() < ROUNDING);
    let reach = 2.0 * thickest / along;
    let thinnest = corners
        .windows(2)
        .map(|pair| (pair[0] + pair[1]) / 2.0)
        .filter(|u| *u - start > reach && start + length - *u > reach)
        .map(|u| chord(segments, long, u) * along)
        .fold(thickest, f64::min);
    let depth_in_plane = across[2].abs() >= world[2][2].abs();
    let (b, h) = if depth_in_plane {
        (*distance_mm, thickest)
    } else {
        (thickest, *distance_mm)
    };
    let profile = segments.clone();
    Ok(Shape {
        grain,
        b,
        h,
        area: b * h,
        net_area: distance_mm * thinnest,
        inertia: b * h.powi(3) / 12.0,
        modulus: b * h * h / 6.0,
        arc: 1.0 / along,
        depth_at: depth_in_plane.then(|| {
            Box::new(move |x: f64| chord(&profile, long, start + x) * along)
                as Box<dyn Fn(f64) -> f64>
        }),
    })
}

/// A force spread over [a, b] (a == b: a point), vertical, newtons.
type Piece = (f64, f64, f64);

/// Force left of x within the pieces and its moment about x.
fn left_of(pieces: &[Piece], x: f64) -> (f64, f64) {
    let mut force = 0.0;
    let mut moment = 0.0;
    for &(a, b, f) in pieces {
        if b - a <= ROUNDING {
            if a < x {
                force += f;
                moment += f * (x - a);
            }
        } else if x > a {
            let end = b.min(x);
            let part = f * (end - a) / (b - a);
            force += part;
            moment += part * (x - (a + end) / 2.0);
        }
    }
    (force, moment)
}

fn right_of(pieces: &[Piece], x: f64) -> (f64, f64) {
    let mirrored: Vec<Piece> = pieces.iter().map(|&(a, b, f)| (-b, -a, f)).collect();
    left_of(&mirrored, -x)
}

/// Pieces of `patches` (one kind) within [p, q]; points on the ends only when `ends` admits them.
fn clipped(patches: &[(f64, f64, f64)], p: f64, q: f64, ends: (bool, bool)) -> Vec<Piece> {
    patches
        .iter()
        .filter_map(|&(a, b, f)| {
            if b - a <= ROUNDING {
                let inside = (a > p + 0.5 || (ends.0 && a >= p - 0.5))
                    && (a < q - 0.5 || (ends.1 && a <= q + 0.5));
                inside.then_some((a.clamp(p, q), a.clamp(p, q), f))
            } else {
                let (lo, hi) = (a.max(p), b.min(q));
                (hi > lo).then(|| (lo, hi, f * (hi - lo) / (b - a)))
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Span,
    /// Free end at p, fixed at q.
    LeftCantilever,
    /// Fixed at p, free end at q.
    RightCantilever,
}

/// Bending moments (N mm), end shears and deflections (mm, for EI = 1) per load kind.
struct Segment {
    p: f64,
    q: f64,
    kind: Kind,
    moments: Vec<Vec<f64>>,
    shears: Vec<[f64; 2]>,
    deflections: Vec<Vec<f64>>,
}

/// `end_moments`: per load kind, the moments over the supports at p and q of
/// a span continuous over them (hogging negative; along the member, without
/// `lever`); cantilevers ignore them.
fn segment(
    p: f64,
    q: f64,
    kind: Kind,
    per_kind: &[Vec<(f64, f64, f64)>],
    end_moments: &[[f64; 2]],
    lever: f64,
    arc: f64,
) -> Segment {
    let ends = match kind {
        Kind::Span => (false, false),
        Kind::LeftCantilever => (true, false),
        Kind::RightCantilever => (false, true),
    };
    let xs: Vec<f64> = (0..=STEPS)
        .map(|j| p + (q - p) * j as f64 / STEPS as f64)
        .collect();
    let mut moments = Vec::new();
    let mut shears = Vec::new();
    let mut deflections = Vec::new();
    for (patches, [m_p, m_q]) in per_kind.iter().zip(end_moments) {
        let pieces = clipped(patches, p, q, ends);
        let total: f64 = pieces.iter().map(|piece| piece.2).sum();
        let (m, v): (Vec<f64>, [f64; 2]) = match kind {
            Kind::Span => {
                let l = q - p;
                let r_left: f64 = pieces
                    .iter()
                    .map(|&(a, b, f)| f * (q - (a + b) / 2.0) / l)
                    .sum::<f64>()
                    + (m_q - m_p) / l;
                let m = xs
                    .iter()
                    .map(|&x| (m_p + r_left * (x - p) - left_of(&pieces, x).1) * lever)
                    .collect();
                (m, [r_left, total - r_left])
            }
            Kind::LeftCantilever => (
                xs.iter().map(|&x| -left_of(&pieces, x).1 * lever).collect(),
                [0.0, total],
            ),
            Kind::RightCantilever => (
                xs.iter()
                    .map(|&x| -right_of(&pieces, x).1 * lever)
                    .collect(),
                [total, 0.0],
            ),
        };
        // w'' = -M / EI along the true length.
        let h = (q - p) / STEPS as f64 * arc;
        let curvature: Vec<f64> = m.iter().map(|value| -value).collect();
        let integrate = |values: &[f64]| {
            let mut out = vec![0.0; values.len()];
            for j in 1..values.len() {
                out[j] = out[j - 1] + (values[j - 1] + values[j]) / 2.0 * h;
            }
            out
        };
        let w = match kind {
            Kind::Span => {
                let raw = integrate(&integrate(&curvature));
                let end = raw[STEPS];
                raw.iter()
                    .enumerate()
                    .map(|(j, value)| value - end * j as f64 / STEPS as f64)
                    .collect()
            }
            Kind::RightCantilever => integrate(&integrate(&curvature)),
            Kind::LeftCantilever => {
                let reversed: Vec<f64> = curvature.iter().rev().copied().collect();
                let mut w = integrate(&integrate(&reversed));
                w.reverse();
                w
            }
        };
        moments.push(m);
        shears.push(v);
        deflections.push(w);
    }
    Segment {
        p,
        q,
        kind,
        moments,
        shears,
        deflections,
    }
}

fn combine(factors: &Loads, values: &[f64]) -> f64 {
    LOAD_KINDS
        .iter()
        .zip(values)
        .map(|(kind, value)| factors.get(*kind).copied().unwrap_or(0.0) * value)
        .sum()
}

struct Best(Option<Check>);

impl Best {
    fn offer(&mut self, name: &'static str, utilization: f64, combination: &str, at: &str) {
        if self
            .0
            .as_ref()
            .is_none_or(|best| utilization > best.utilization)
        {
            self.0 = Some(Check {
                name,
                utilization,
                combination: combination.to_owned(),
                at: at.to_owned(),
            });
        }
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

struct Design<'a> {
    class: &'a TimberClass,
    gamma_m: f64,
    kdef: f64,
}

impl Design<'_> {
    fn kmod(&self, duration: usize) -> f64 {
        kmod(self.class.service_class, duration)
    }

    fn strength(&self, characteristic: f64, duration: usize) -> f64 {
        self.kmod(duration) * characteristic / self.gamma_m
    }
}

fn beam_checks(
    load: &MemberLoad,
    shape: &Shape,
    design: &Design,
    uls: &[Combination],
    sls: &[(Combination, Loads)],
) -> (Vec<Check>, bool) {
    let class = design.class;
    let lever = horizontal(load.axis);
    let cos_t = horizontal(shape.grain);
    let per_kind: Vec<Vec<(f64, f64, f64)>> = LOAD_KINDS
        .iter()
        .map(|kind| {
            load.patches
                .iter()
                .filter_map(|patch| {
                    let f = patch.loads_n.get(*kind).copied().unwrap_or(0.0);
                    (f > 0.0).then_some((
                        patch.from_mm.min(patch.to_mm),
                        patch.from_mm.max(patch.to_mm),
                        f,
                    ))
                })
                .collect()
        })
        .collect();
    // Bearings closer than the member is deep hold it as one: over so short a
    // gap the load goes straight from one to the other, it does not bend.
    let zones = zones(load);
    let mut bearings: Vec<(f64, f64)> = Vec::new();
    for zone in &zones {
        match bearings.last_mut() {
            Some(last) if (zone.lo - last.1) * shape.arc < shape.h => last.1 = last.1.max(zone.hi),
            _ => bearings.push((zone.lo, zone.hi)),
        }
    }
    // A short bearing supports the member at its middle, a long one along it.
    let merged: Vec<(f64, f64)> = bearings
        .iter()
        .map(|&(lo, hi)| {
            if (hi - lo) * shape.arc > BED_MM {
                (lo, hi)
            } else {
                let middle = (lo + hi) / 2.0;
                (middle, middle)
            }
        })
        .collect();
    // The member is continuous over its supports: a long bearing holds it at
    // both of its ends, with no span checked between them.
    let mut supports: Vec<f64> = Vec::new();
    let mut bedded: Vec<bool> = Vec::new();
    for &(lo, hi) in &merged {
        if !supports.is_empty() {
            bedded.push(false);
        }
        supports.push(lo);
        if hi > lo {
            bedded.push(true);
            supports.push(hi);
        }
    }
    let support_moments: Vec<Vec<f64>> = if supports.is_empty() {
        Vec::new()
    } else {
        per_kind
            .iter()
            .map(|pieces| continuous_span::solve(&supports, &bedded, pieces).moments)
            .collect()
    };
    let no_moments = vec![[0.0; 2]; per_kind.len()];
    let mut segments = Vec::new();
    if let (Some(first), Some(last)) = (supports.first(), supports.last()) {
        if *first > 1.0 {
            segments.push(segment(
                0.0,
                *first,
                Kind::LeftCantilever,
                &per_kind,
                &no_moments,
                lever,
                shape.arc,
            ));
        }
        for (j, pair) in supports.windows(2).enumerate() {
            if !bedded[j] && pair[1] - pair[0] > 1.0 {
                let ends: Vec<[f64; 2]> = support_moments
                    .iter()
                    .map(|moments| [moments[j], moments[j + 1]])
                    .collect();
                segments.push(segment(
                    pair[0],
                    pair[1],
                    Kind::Span,
                    &per_kind,
                    &ends,
                    lever,
                    shape.arc,
                ));
            }
        }
        if load.length_mm - last > 1.0 {
            segments.push(segment(
                *last,
                load.length_mm,
                Kind::RightCantilever,
                &per_kind,
                &no_moments,
                lever,
                shape.arc,
            ));
        }
    }
    let spans_checked = bearings.len() < 2 || segments.iter().any(|seg| seg.kind == Kind::Span);
    let (b, h) = (shape.b, shape.h);
    let w_section = shape.modulus;
    let ei = class.e0_mean * shape.inertia;
    let kh = if class.glulam {
        (600.0 / h).powf(0.1).clamp(1.0, 1.1)
    } else {
        (150.0 / h).powf(0.2).clamp(1.0, 1.3)
    };
    let mut bending = Best(None);
    let mut shear = Best(None);
    let mut inst = Best(None);
    let mut fin = Best(None);
    for seg in &segments {
        let at = format!(
            "{} {}-{} mm",
            if seg.kind == Kind::Span {
                "span"
            } else {
                "cantilever"
            },
            seg.p.round(),
            seg.q.round()
        );
        for combination in uls {
            let moment = (0..=STEPS)
                .map(|j| {
                    let values: Vec<f64> = seg.moments.iter().map(|m| m[j]).collect();
                    combine(&combination.factors, &values).abs()
                })
                .fold(0.0, f64::max);
            let fm = design.strength(class.fm_k, combination.duration) * kh;
            bending.offer("bending", moment / w_section / fm, &combination.label, &at);
            let ends: Vec<f64> = (0..2)
                .map(|side| {
                    let values: Vec<f64> = seg.shears.iter().map(|v| v[side]).collect();
                    combine(&combination.factors, &values)
                })
                .collect();
            let force = ends[0].max(ends[1]) * cos_t;
            let tau = 1.5 * force / (KCR * shape.area);
            shear.offer(
                "shear",
                tau / design.strength(class.fv_k, combination.duration),
                &combination.label,
                &at,
            );
        }
        let true_len = (seg.q - seg.p) * shape.arc;
        let (inst_limit, fin_limit) = if seg.kind == Kind::Span {
            (true_len / 300.0, true_len / 250.0)
        } else {
            (true_len / 150.0, true_len / 125.0)
        };
        if true_len < 1.0 {
            continue;
        }
        for (characteristic, final_factors) in sls {
            let largest = |factors: &Loads| {
                (0..=STEPS)
                    .map(|j| {
                        let values: Vec<f64> = seg.deflections.iter().map(|w| w[j]).collect();
                        combine(factors, &values).abs() / ei
                    })
                    .fold(0.0, f64::max)
            };
            inst.offer(
                "deflection_inst",
                largest(&characteristic.factors) / inst_limit,
                &characteristic.label,
                &at,
            );
            fin.offer(
                "deflection_fin",
                largest(final_factors) / fin_limit,
                &characteristic.label,
                &at,
            );
        }
    }
    let mut checks: Vec<Check> = [bending, shear, inst, fin]
        .into_iter()
        .filter_map(|best| best.0)
        .collect();
    let mut notch = Best(None);
    let mut bearing = Best(None);
    for zone in &zones {
        let names = zone.names(load);
        let reaction = zone.total(load, false);
        // Notched bearings (6.5.2).
        if let Some(depth_at) = &shape.depth_at
            && (zone.hi - zone.lo) * shape.arc <= BED_MM
        {
            let h_ef = (0..=8)
                .map(|k| depth_at(zone.lo + (zone.hi - zone.lo) * f64::from(k) / 8.0))
                .filter(|depth| *depth > 1.0)
                .fold(f64::MAX, f64::min);
            if h_ef < 0.95 * h {
                let kn = if class.glulam { 6.5 } else { 5.0 };
                let alpha = h_ef / h;
                let x = (zone.hi - zone.lo) / 2.0 * shape.arc;
                let kv = (kn
                    / (h.sqrt()
                        * ((alpha * (1.0 - alpha)).sqrt()
                            + 0.8 * x / h * (1.0 / alpha - alpha * alpha).sqrt())))
                .min(1.0);
                for combination in uls {
                    let force = factor(&combination.factors, &reaction) * cos_t;
                    let tau = 1.5 * force / (KCR * b * h_ef);
                    let strength = kv * design.strength(class.fv_k, combination.duration);
                    notch.offer("shear_notch", tau / strength, &combination.label, &names);
                }
            }
        }
        // Compression perpendicular to the grain on the faces it bears on.
        let area = zone.area(load);
        if area > 0.0 {
            let pressed = zone.total(load, true);
            for combination in uls {
                let sigma = factor(&combination.factors, &pressed) / area;
                let strength = design.strength(class.fc90_k, combination.duration);
                bearing.offer("bearing", sigma / strength, &combination.label, &names);
            }
        }
    }
    checks.extend(notch.0);
    checks.extend(bearing.0);
    (checks, spans_checked)
}

/// Reactions of a member whose bearings touch or overlap, along its load frame.
struct Zone {
    lo: f64,
    hi: f64,
    reactions: Vec<usize>,
}

impl Zone {
    fn names(&self, load: &MemberLoad) -> String {
        self.reactions
            .iter()
            .map(|r| load.reactions[*r].part.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The reactions' loads; only those on faces when `on_faces`.
    fn total(&self, load: &MemberLoad, on_faces: bool) -> Loads {
        let mut total = Loads::new();
        for r in &self.reactions {
            let reaction = &load.reactions[*r];
            if on_faces && (reaction.joint.is_some() || reaction.contact_mm2 <= 0.0) {
                continue;
            }
            for (kind, value) in &reaction.loads_n {
                *total.entry(kind.clone()).or_default() += value;
            }
        }
        total
    }

    fn area(&self, load: &MemberLoad) -> f64 {
        self.reactions
            .iter()
            .map(|r| &load.reactions[*r])
            .filter(|r| r.joint.is_none())
            .map(|r| r.contact_mm2)
            .sum()
    }
}

/// Bearing zones of the supports the member stays on.
fn zones(load: &MemberLoad) -> Vec<Zone> {
    let mut order: Vec<usize> = (0..load.reactions.len())
        .filter(|r| !load.reactions[*r].lifted_off)
        .collect();
    let span = |r: usize| {
        let reaction = &load.reactions[r];
        (
            reaction.at_mm - reaction.length_mm / 2.0,
            reaction.at_mm + reaction.length_mm / 2.0,
        )
    };
    order.sort_by(|a, b| span(*a).0.total_cmp(&span(*b).0));
    let mut zones: Vec<Zone> = Vec::new();
    for r in order {
        let (lo, hi) = span(r);
        match zones.last_mut() {
            Some(zone) if lo <= zone.hi + 1.0 => {
                zone.hi = zone.hi.max(hi);
                zone.reactions.push(r);
            }
            _ => zones.push(Zone {
                lo,
                hi,
                reactions: vec![r],
            }),
        }
    }
    zones
}

fn column_check(load: &MemberLoad, shape: &Shape, design: &Design, uls: &[Combination]) -> Check {
    let class = design.class;
    let length = load.length_mm * shape.arc;
    let lambda_rel = |side: f64| {
        length * 12f64.sqrt() / side / std::f64::consts::PI * (class.fc0_k / class.e0_05).sqrt()
    };
    let beta = if class.glulam { 0.1 } else { 0.2 };
    let kc = |lr: f64| {
        if lr <= 0.3 {
            return 1.0;
        }
        let k = 0.5 * (1.0 + beta * (lr - 0.3) + lr * lr);
        1.0 / (k + (k * k - lr * lr).sqrt())
    };
    let kc = kc(lambda_rel(shape.b.min(shape.h)));
    let mut best = Best(None);
    for combination in uls {
        let force = factor(&combination.factors, &load.loads_n);
        let strength = design.strength(class.fc0_k, combination.duration);
        let buckling = force / (shape.b * shape.h) / (kc * strength);
        let net = force / shape.net_area / strength;
        let detail = if net > buckling {
            format!("net section {} mm² at a notch", shape.net_area.round())
        } else {
            format!("length {} mm", length.round())
        };
        best.offer(
            "compression",
            buckling.max(net),
            &combination.label,
            &detail,
        );
    }
    best.0.expect("at least the permanent combination")
}

/// Every load-path member checked as a timber beam or column.
#[must_use]
pub fn member_checks(model: &ProgramModel, loads: &LoadReport) -> DesignReport {
    if loads.is_empty() || model.strength_classes.is_empty() {
        return DesignReport::default();
    }
    let mut members: Vec<MemberCheck> = Vec::new();
    let mut bearing_on: BTreeMap<String, Vec<Check>> = BTreeMap::new();
    let mut beams: BTreeMap<String, (TimberClass, f64)> = BTreeMap::new();
    let mut pending_bearings: Vec<(String, f64, Loads, String)> = Vec::new();
    for load in &loads.members {
        let part = model.part(&load.part);
        let material = part.and_then(|p| p.material.clone()).unwrap_or_default();
        let class = model.strength_classes.get(&material);
        let mut missing = load.missing.clone();
        let shape = part.map_or_else(|| Err("the part is not found"), |p| shape(p, load.axis));
        let role = match &shape {
            Ok(s) if horizontal(s.grain) < COLUMN_COS => "column",
            _ => "beam",
        };
        let mut check = MemberCheck {
            part: load.part.clone(),
            role,
            strength_class: class.map(|c| c.strength_class.clone()),
            section_mm: shape.as_ref().ok().map(|s| [round3(s.b), round3(s.h)]),
            utilization: None,
            governing: None,
            checks: Vec::new(),
            status: "not_verified",
            missing: Vec::new(),
        };
        let (Some(class), Ok(shape)) = (class, shape.as_ref()) else {
            if class.is_none() {
                missing.push(format!(
                    "material {material:?} has no timber strength class (timber_strength)"
                ));
            }
            if let Err(reason) = &shape {
                missing.push((*reason).to_owned());
            }
            check.missing = missing;
            members.push(check);
            continue;
        };
        let design = Design {
            class,
            gamma_m: if class.glulam { 1.25 } else { 1.3 },
            kdef: KDEF[usize::from(class.service_class) - 1],
        };
        let present: Vec<&'static str> = LOAD_KINDS[1..]
            .iter()
            .copied()
            .filter(|kind| load.loads_n.get(*kind).is_some_and(|v| *v > 0.0))
            .collect();
        let (uls, sls) = combinations(&present, design.kdef);
        if load.reactions.is_empty() {
            missing.push("the member has no supports to check against".to_owned());
        } else if role == "column" {
            check.checks.push(column_check(load, shape, &design, &uls));
        } else {
            let (checks, spans_checked) = beam_checks(load, shape, &design, &uls, &sls);
            check.checks = checks;
            // Separate bearings always leave a span; without one, bending,
            // shear and deflection were never checked.
            if !spans_checked {
                missing.push("no span between its separate bearings was checked".to_owned());
            }
            beams.insert(load.part.clone(), (class.clone(), design.gamma_m));
        }
        for reaction in load
            .reactions
            .iter()
            .filter(|r| r.joint.is_none() && r.contact_mm2 > 0.0)
        {
            pending_bearings.push((
                reaction.part.clone(),
                reaction.contact_mm2,
                reaction.loads_n.clone(),
                load.part.clone(),
            ));
        }
        check.missing = missing;
        members.push(check);
    }
    // Compression perpendicular to the grain in the beams others bear on.
    for (supporter, area, reaction, from) in pending_bearings {
        let Some((class, gamma_m)) = beams.get(&supporter) else {
            continue;
        };
        let design = Design {
            class,
            gamma_m: *gamma_m,
            kdef: 0.0,
        };
        let present: Vec<&'static str> = LOAD_KINDS[1..]
            .iter()
            .copied()
            .filter(|kind| reaction.get(*kind).is_some_and(|v| *v > 0.0))
            .collect();
        let (uls, _) = combinations(&present, 0.0);
        let mut best = Best(None);
        for combination in &uls {
            let sigma = factor(&combination.factors, &reaction) / area;
            best.offer(
                "bearing",
                sigma / design.strength(class.fc90_k, combination.duration),
                &combination.label,
                &from,
            );
        }
        bearing_on.entry(supporter).or_default().extend(best.0);
    }
    for member in &mut members {
        if let Some(extra) = bearing_on.remove(&member.part) {
            member.checks.extend(extra);
        }
        settle(member);
    }
    DesignReport {
        basis: BASIS.to_vec(),
        members,
    }
}

/// Governing check and status of a member from its checks. A check that is
/// not a number leaves the member unverified unless another one fails it.
fn settle(member: &mut MemberCheck) {
    for check in &mut member.checks {
        check.utilization = round3(check.utilization);
    }
    if member.checks.iter().any(|c| c.utilization.is_nan()) {
        member
            .missing
            .push("a utilization is not a number".to_owned());
    }
    let governing = member
        .checks
        .iter()
        .filter(|c| !c.utilization.is_nan())
        .max_by(|a, b| a.utilization.total_cmp(&b.utilization));
    member.utilization = governing.map(|c| c.utilization);
    member.governing = governing.map(|c| c.name.to_owned());
    member.status = match member.utilization {
        Some(u) if u > 1.0 => "fail",
        Some(_) if member.missing.is_empty() => "pass",
        _ => "not_verified",
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(utilizations: &[f64]) -> MemberCheck {
        let mut member = MemberCheck {
            part: "beam".to_owned(),
            role: "beam",
            strength_class: None,
            section_mm: None,
            utilization: None,
            governing: None,
            checks: ["bending", "shear"]
                .iter()
                .zip(utilizations)
                .map(|(name, utilization)| Check {
                    name,
                    utilization: *utilization,
                    combination: String::new(),
                    at: String::new(),
                })
                .collect(),
            status: "not_verified",
            missing: Vec::new(),
        };
        settle(&mut member);
        member
    }

    #[test]
    fn a_utilization_that_is_not_a_number_never_passes_and_never_hides_a_failure() {
        let unknown = settled(&[f64::NAN, 0.5]);
        assert_eq!(unknown.status, "not_verified");
        assert_eq!(unknown.utilization, Some(0.5));
        assert_eq!(unknown.missing, ["a utilization is not a number"]);
        assert_eq!(settled(&[f64::NAN, 1.2]).status, "fail");
        assert_eq!(settled(&[0.4, 0.5]).status, "pass");
    }
}
