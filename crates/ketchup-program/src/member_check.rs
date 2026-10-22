//! Timber member check after EN 1995-1-1 (EC5) on the loads of the load path:
//! bending, shear (with notches at supports), deflection and compression
//! perpendicular to the grain at bearings for beams; compression with
//! buckling for columns. Strength classes come from `timber_strength()`;
//! loads are combined after EN 1990 6.10 with recommended factors. A member
//! whose inputs are incomplete reports its utilization from the known loads
//! as `not_verified`, never as passing.

use crate::continuous_span;
use crate::loads::{LOAD_KINDS, LoadReport, Loads, MemberLoad, axes, extents};
use crate::model::{
    Part, ProgramMirror, ProgramModel, ProgramOperation, ProgramPartBody, ProgramProfileSegment,
};
use ketchup_geometry::linalg::{cross, cross2, dot, length, normalize, scale, sub};
use ketchup_tolerance::ROUNDING;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Check {
    /// bending, shear, shear_notch, deflection_inst, deflection_fin, compression, bearing.
    #[serde(deserialize_with = "crate::wire_names::name")]
    pub name: crate::wire_names::Name,
    pub utilization: f64,
    pub combination: String,
    /// The span ("span 70-3068 mm") or the part it bears on / that bears on it.
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MemberCheck {
    pub part: String,
    /// "beam" or "column".
    #[serde(deserialize_with = "crate::wire_names::name")]
    pub role: crate::wire_names::Name,
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
    #[serde(deserialize_with = "crate::wire_names::name")]
    pub status: crate::wire_names::Name,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct DesignReport {
    #[serde(deserialize_with = "basis")]
    pub basis: Vec<&'static str>,
    pub members: Vec<MemberCheck>,
}

/// The basis comes back from the program worker as [`BASIS`] or empty.
fn basis<'de, D: serde::Deserializer<'de>>(reader: D) -> Result<Vec<&'static str>, D::Error> {
    let basis = Vec::<String>::deserialize(reader)?;
    if basis.is_empty() {
        Ok(Vec::new())
    } else if basis.iter().map(String::as_str).eq(BASIS) {
        Ok(BASIS.to_vec())
    } else {
        Err(serde::de::Error::custom(
            "the design basis is not this build's",
        ))
    }
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
    "shear with kcr = 0.67 (6.1.7); notches at bearings with kv (6.5.2), right-angled, x from the middle of the reaction to the corner of the notch; elsewhere a change in depth steeper than 1:10 on the side in tension is not verified (6.5.1(2))",
    "compression perpendicular to the grain with kc90 = 1 on the contact area (6.1.5, without the 30 mm extension); compression with kc (6.3.2) on the full member length about both axes, sheathing not counted, and without kc on the net section at notches in a side (6.1.4) more than twice the member width from its ends; axial force in sloped beams neglected",
    "moments, shear and deflection: bending stress, shear stress and curvature on the section at each place along the member (a notch drawn in its side profile included), taken in the vertical plane across its grain so that a vertical load bends a turned section about both its axes, and its deflection is the resultant of the vertical and the sideways part, each at its own worst load pattern; spans continuous over the supports (three-moment equation, constant section) and cantilevers clamped at their support, as the loads are passed down; variable loads laid span by span and overhang by overhang where they add to the effect (load patterns), permanent load with 1.35 or, where it relieves the effect, 1.0 (EN 1990 table A1.2(B)); w_inst <= l/300 and w_fin <= l/250, cantilevers l/150 and l/125 (EN 1995-1-1 table 7.2, the lenient ends of the ranges)",
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
pub(crate) fn action(kind: &str) -> (usize, f64, f64) {
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

/// Where the line local[long] = u crosses the profile, along the other local
/// axis, in order: the cuts run between the first and second, third and fourth...
fn crossings(segments: &[ProgramProfileSegment], long: usize, u: f64) -> Vec<f64> {
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
    hits
}

/// Total length of the profile's cuts by the line local[long] = u.
fn chord(segments: &[ProgramProfileSegment], long: usize, u: f64) -> f64 {
    crossings(segments, long, u)
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| pair[1] - pair[0])
        .sum()
}

/// Area integrals of a closed line profile taken counter-clockwise: area,
/// ∫x, ∫y, ∫x², ∫y² and ∫xy over it.
fn integrals(segments: &[ProgramProfileSegment]) -> [f64; 6] {
    let mut sum = [0.0; 6];
    for s in segments {
        let ([ax, ay], [bx, by]) = (s.start_mm, s.end_mm);
        let cross = ax * by - bx * ay;
        sum[0] += cross / 2.0;
        sum[1] += (ax + bx) * cross / 6.0;
        sum[2] += (ay + by) * cross / 6.0;
        sum[3] += (ax * ax + ax * bx + bx * bx) * cross / 12.0;
        sum[4] += (ay * ay + ay * by + by * by) * cross / 12.0;
        sum[5] += (ax * by + 2.0 * ax * ay + 2.0 * bx * by + bx * ay) * cross / 24.0;
    }
    if sum[0] < 0.0 { sum.map(|v| -v) } else { sum }
}

/// Section of a line profile less the line profiles `bores` cut through the
/// whole member inside it, under a load along `depth` (a local profile axis)
/// that bends it freely: about the axis across `depth` when that is a principal
/// axis, and sideways too when it is not (an angle, a parallelogram).
struct Section<'a> {
    segments: &'a [ProgramProfileSegment],
    bores: &'a [&'a [ProgramProfileSegment]],
    depth: usize,
    area: f64,
    centre: [f64; 2],
    /// Second moments about the centroidal axes across and along `depth`,
    /// and their product.
    across: f64,
    along: f64,
    product: f64,
}

impl<'a> Section<'a> {
    fn new(
        segments: &'a [ProgramProfileSegment],
        bores: &'a [&'a [ProgramProfileSegment]],
        depth: usize,
    ) -> Self {
        let mut sum = integrals(segments);
        for bore in bores {
            let hole = integrals(bore);
            sum = std::array::from_fn(|i| sum[i] - hole[i]);
        }
        let [area, first_x, first_y, second_x, second_y, product] = sum;
        let centre = [first_x / area, first_y / area];
        // About the centroidal axes along local y and x, and their product.
        let about_y = second_x - area * centre[0] * centre[0];
        let about_x = second_y - area * centre[1] * centre[1];
        let (across, along) = if depth == 1 {
            (about_x, about_y)
        } else {
            (about_y, about_x)
        };
        Self {
            segments,
            bores,
            depth,
            area,
            centre,
            across,
            along,
            product: product - area * centre[0] * centre[1],
        }
    }

    /// The least second moment, about the weak principal axis.
    fn least(&self) -> f64 {
        (self.across + self.along) / 2.0 - ((self.across - self.along) / 2.0).hypot(self.product)
    }

    /// The determinant of the section's inertia tensor, the product of its
    /// principal second moments.
    fn inertia_determinant(&self) -> f64 {
        cross2([self.across, self.product], [self.product, self.along])
    }

    /// Curvature per unit moment about the axis across `depth`, along `depth`
    /// and sideways: a turned section bends sideways too (review 2026-10-19
    /// PG-16), not at all when that axis is principal.
    fn flexibility(&self) -> [f64; 2] {
        let principal_product = self.inertia_determinant();
        [
            self.along / principal_product,
            -self.product / principal_product,
        ]
    }

    /// Bending stress per unit moment at offsets `along_depth` and `across_depth`
    /// from the centroid; with the first moments of an area beyond a cut
    /// instead, its shear flow per unit shear force.
    fn per_unit(&self, along_depth: f64, across_depth: f64) -> f64 {
        (self.along * along_depth - self.product * across_depth) / self.inertia_determinant()
    }

    /// Section modulus at the farthest fibre.
    fn modulus(&self) -> f64 {
        let other = 1 - self.depth;
        let farthest = self
            .segments
            .iter()
            .map(|s| {
                let p = s.start_mm;
                self.per_unit(
                    p[self.depth] - self.centre[self.depth],
                    p[other] - self.centre[other],
                )
                .abs()
            })
            .fold(0.0, f64::max);
        1.0 / farthest
    }

    /// Material width of the cut at local[depth] = t and the first moment of
    /// that strip about the centroid, across `depth`.
    fn strip(&self, t: f64) -> (f64, f64) {
        let centre = self.centre[1 - self.depth];
        let cuts = |segments: &[ProgramProfileSegment]| {
            crossings(segments, self.depth, t)
                .chunks(2)
                .filter(|pair| pair.len() == 2)
                .fold((0.0, 0.0), |(width, first), pair| {
                    let (a, b) = (pair[0] - centre, pair[1] - centre);
                    (width + b - a, first + (b * b - a * a) / 2.0)
                })
        };
        let (mut width, mut first) = cuts(self.segments);
        for bore in self.bores {
            let (w, f) = cuts(bore);
            width -= w;
            first -= f;
        }
        (width, first)
    }

    /// The area the shear force is divided by for the largest shear stress,
    /// the shear flow V Q / I over the material width b of a cut across
    /// `depth` (a rectangle: 2/3 of its area). Q and I as in free bending.
    fn shear_area(&self) -> f64 {
        let depth = self.depth;
        let mut levels: Vec<f64> = std::iter::once(self.segments)
            .chain(self.bores.iter().copied())
            .flatten()
            .map(|s| s.start_mm[depth])
            .collect();
        levels.sort_by(f64::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() < ROUNDING);
        // Between two levels the strip's width is linear in t and its first
        // moment quadratic, so two-point Gauss integrates both exactly.
        let centre = self.centre[depth];
        let beyond = |a: f64, b: f64| {
            let (mid, half) = ((a + b) / 2.0, (b - a) / 2.0);
            let offset = half / 3f64.sqrt();
            [mid - offset, mid + offset]
                .into_iter()
                .fold([0.0, 0.0], |sum, t| {
                    let (width, first) = self.strip(t);
                    [sum[0] + half * (t - centre) * width, sum[1] + half * first]
                })
        };
        // Cuts within each span and through the centroid, from the top down.
        let mut above = [0.0, 0.0];
        let mut largest: f64 = 0.0;
        for pair in levels.windows(2).rev() {
            let (a, b) = (pair[0], pair[1]);
            let steps = 32;
            let cuts = (1..steps)
                .map(|k| a + (b - a) * f64::from(k) / f64::from(steps))
                .chain((a < centre && centre < b).then_some(centre));
            for cut in cuts {
                let (width, _) = self.strip(cut);
                if width > ROUNDING {
                    let part = beyond(cut, b);
                    let flow = self.per_unit(above[0] + part[0], above[1] + part[1]);
                    largest = largest.max(flow.abs() / width);
                }
            }
            let part = beyond(a, b);
            above = [above[0] + part[0], above[1] + part[1]];
        }
        1.0 / largest
    }

    /// Where the material's edges cross the line local[depth] = t, in order.
    fn edges_at(&self, t: f64) -> Vec<f64> {
        let mut hits: Vec<f64> = std::iter::once(self.segments)
            .chain(self.bores.iter().copied())
            .flat_map(|segments| crossings(segments, self.depth, t))
            .collect();
        hits.sort_by(f64::total_cmp);
        hits
    }

    /// Whether a load along `depth` through the centroid passes beside the
    /// shear centre and twists the member: unless the section is symmetric
    /// about its centroid, or `upright` (`depth` vertical) and symmetric about
    /// the vertical line through it. An angle or a channel open to the side
    /// twists; a sheared rectangle or a channel open upward does not.
    fn twists(&self, upright: bool) -> bool {
        let (centre, other) = (self.centre[self.depth], self.centre[1 - self.depth]);
        let mut levels: Vec<f64> = std::iter::once(self.segments)
            .chain(self.bores.iter().copied())
            .flatten()
            .flat_map(|s| {
                [
                    s.start_mm[self.depth],
                    2.0 * centre - s.start_mm[self.depth],
                ]
            })
            .collect();
        levels.sort_by(f64::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() < ROUNDING);
        // Edges at t mirrored across the centre line onto edges at `there`.
        let mirrored = |here: &[f64], there: &[f64]| {
            here.len() == there.len()
                && here
                    .iter()
                    .zip(there.iter().rev())
                    .all(|(a, b)| (a + b - 2.0 * other).abs() <= ROUNDING)
        };
        // The edges move linearly between levels, so two cuts a span settle it.
        let (mut about_centre, mut about_line) = (true, upright);
        for pair in levels.windows(2) {
            for t in [
                (2.0 * pair[0] + pair[1]) / 3.0,
                (pair[0] + 2.0 * pair[1]) / 3.0,
            ] {
                let here = self.edges_at(t);
                about_centre &= mirrored(&here, &self.edges_at(2.0 * centre - t));
                about_line &= mirrored(&here, &here);
            }
        }
        !(about_centre || about_line)
    }
}

/// Whether `point` lies inside the closed line profile (even-odd rule).
fn inside(segments: &[ProgramProfileSegment], point: [f64; 2]) -> bool {
    segments
        .iter()
        .filter(|s| {
            let (a, b) = (s.start_mm, s.end_mm);
            (a[1] > point[1]) != (b[1] > point[1])
                && point[0] < a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        })
        .count()
        % 2
        == 1
}

/// Whether the closed line profile `inner` lies within `outer` without
/// touching or crossing its edges.
fn encloses(outer: &[ProgramProfileSegment], inner: &[ProgramProfileSegment]) -> bool {
    let side = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let crosses = |e: &ProgramProfileSegment, f: &ProgramProfileSegment| {
        let (p, q, r, s) = (e.start_mm, e.end_mm, f.start_mm, f.end_mm);
        side(p, q, r) * side(p, q, s) <= 0.0 && side(r, s, p) * side(r, s, q) <= 0.0
    };
    inner.iter().all(|s| inside(outer, s.start_mm))
        && !inner.iter().any(|e| outer.iter().any(|f| crosses(e, f)))
}

/// The world directions a member's section is taken in, across its `grain`:
/// horizontal, and the vertical's component across the grain, along which a
/// vertical load bends it (review 2026-10-18 PG-9). Any two across a vertical
/// grain, where no vertical load bends it.
fn across_grain(grain: [f64; 3]) -> [[f64; 3]; 2] {
    let towards = |axis: [f64; 3]| normalize(sub(axis, scale(grain, dot(axis, grain))));
    let vertical = towards([0.0, 0.0, 1.0])
        .filter(|_| horizontal(grain) > ROUNDING)
        .or_else(|| towards([1.0, 0.0, 0.0]))
        .unwrap_or([0.0, 1.0, 0.0]);
    [cross(grain, vertical), vertical]
}

/// A closed polygon of `corners` as profile lines.
fn polygon(corners: &[[f64; 2]]) -> Vec<ProgramProfileSegment> {
    (0..corners.len())
        .map(|i| {
            ProgramProfileSegment::line(String::new(), corners[i], corners[(i + 1) % corners.len()])
        })
        .collect()
}

/// What bending reads from a member's section at one place along it, taken
/// in the world's vertical plane across the grain (`across_grain`) so that a
/// vertical load bends it freely.
#[derive(Clone, Copy)]
struct Bending {
    /// Curvature per unit moment, vertical and sideways.
    flexibility: [f64; 2],
    modulus: f64,
    shear_area: f64,
}

impl Bending {
    fn of(section: &Section) -> Self {
        Self {
            flexibility: section.flexibility(),
            modulus: section.modulus(),
            shear_area: section.shear_area(),
        }
    }
}

/// A member's section along it (review 2026-10-18 PG-8: one drawn as its side
/// profile is as deep as the profile at each place, not its deepest).
enum Along {
    /// The same section all along: its profile is the cross-section.
    Prismatic(Bending),
    /// Cut across from a side or plan `profile` at local[`long`] = `start` +
    /// the load-frame position: each stretch of the cut, `distance` thick
    /// along the extrusion. `long_axis`, `other` and `extrusion` are the
    /// section-plane coordinates (`across_grain`) of a unit along the
    /// profile's `long` and other local axis and along its extrusion; `corners` the load-frame positions where
    /// the section changes.
    Profile {
        profile: Vec<ProgramProfileSegment>,
        long: usize,
        start: f64,
        long_axis: [f64; 2],
        other: [f64; 2],
        extrusion: [f64; 2],
        distance: f64,
        corners: Vec<f64>,
    },
}

impl Along {
    /// The stretches of a profile's cut at load-frame position `x`, along its
    /// other local axis; none for a prismatic section.
    fn cut(&self, x: f64) -> Vec<[f64; 2]> {
        let Self::Profile {
            profile,
            long,
            start,
            corners,
            ..
        } = self
        else {
            return Vec::new();
        };
        // An end face of the member is cut just inside it.
        let (first, last) = (corners[0], corners[corners.len() - 1]);
        let x = x.clamp(first + ROUNDING, last - ROUNDING);
        crossings(profile, *long, start + x)
            .chunks(2)
            .filter(|pair| pair.len() == 2 && pair[1] - pair[0] > ROUNDING)
            .map(|pair| [pair[0], pair[1]])
            .collect()
    }

    /// The outline of a profile's `cut` in the section plane; None where the
    /// member has no material (a pointed end).
    fn outline(&self, cut: &[[f64; 2]]) -> Option<Vec<ProgramProfileSegment>> {
        let Self::Profile {
            other,
            extrusion,
            distance,
            ..
        } = self
        else {
            return None;
        };
        let at = |c: f64, e: f64| {
            [
                c * other[0] + e * extrusion[0],
                c * other[1] + e * extrusion[1],
            ]
        };
        let outline: Vec<ProgramProfileSegment> = cut
            .iter()
            .flat_map(|[from, to]| {
                polygon(&[
                    at(*from, 0.0),
                    at(*to, 0.0),
                    at(*to, *distance),
                    at(*from, *distance),
                ])
            })
            .collect();
        (!outline.is_empty()).then_some(outline)
    }

    /// The section's bending properties at load-frame position `x`, computed
    /// once per distinct cut (`known`).
    fn at(&self, x: f64, known: &mut HashMap<Vec<u64>, Option<Bending>>) -> Option<Bending> {
        if let Self::Prismatic(bending) = self {
            return Some(*bending);
        }
        let cut = self.cut(x);
        let key = cut.iter().flatten().map(|c| c.to_bits()).collect();
        *known.entry(key).or_insert_with(|| {
            self.outline(&cut)
                .map(|outline| Bending::of(&Section::new(&outline, &[], 1)))
        })
    }

    /// The lowest and highest level of the section at load-frame position `x`,
    /// along the vertical across the grain, so that a face along the grain
    /// keeps its level; None for a prismatic section and where there is no
    /// material.
    fn edges(&self, x: f64) -> Option<[f64; 2]> {
        let Self::Profile {
            start,
            long_axis,
            other,
            extrusion,
            distance,
            ..
        } = self
        else {
            return None;
        };
        let base = (start + x) * long_axis[1];
        let levels: Vec<f64> = self
            .cut(x)
            .iter()
            .flatten()
            .flat_map(|c| {
                let level = base + c * other[1];
                [level, level + distance * extrusion[1]]
            })
            .collect();
        (!levels.is_empty()).then(|| {
            [
                levels.iter().copied().fold(f64::MAX, f64::min),
                levels.iter().copied().fold(f64::MIN, f64::max),
            ]
        })
    }

    fn corners(&self) -> &[f64] {
        match self {
            Self::Prismatic(_) => &[],
            Self::Profile { corners, .. } => corners,
        }
    }
}

/// A member's section and directions.
struct Shape {
    /// Unit world direction of the member's grain.
    grain: [f64; 3],
    /// Width and depth for bending; the two sides of a column.
    b: f64,
    h: f64,
    /// Area of the deepest section.
    area: f64,
    /// Area of the thinnest section anywhere along the member, ends included
    /// (notches and laps cut into a side).
    net_area: f64,
    /// Area of the thinnest section away from the ends, for buckling.
    buckling_area: f64,
    /// The section bending reads along the member.
    bending: Along,
    /// Least radius of gyration of the `buckling_area` section, about its
    /// weak principal axis.
    radius: f64,
    /// Why the section above is not the member's along its whole length.
    unmodelled: Option<&'static str>,
    /// Why the member twists, which the check does not model: as a beam under
    /// a vertical load, or as a column in flexural-torsional buckling.
    twists: Option<&'static str>,
    /// True length per unit of the load frame's position.
    arc: f64,
    /// Depth across the member at a load-frame position, when it varies (notches).
    depth_at: Option<Box<dyn Fn(f64) -> f64>>,
}

/// The mirrors among `operations` (review 2026-10-19 PG-13: they reflect the
/// solid as shaped before them, so a profile mirrored along the grain has its
/// notch at the other end).
fn mirrors(operations: &[ProgramOperation]) -> Vec<&ProgramMirror> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            ProgramOperation::Mirror(mirror) => Some(mirror),
            _ => None,
        })
        .collect()
}

/// A point of the part's frame where `mirrors` leave it.
fn reflected(mut point: [f64; 3], mirrors: &[&ProgramMirror]) -> [f64; 3] {
    for mirror in mirrors {
        point[mirror.axis] = 2.0 * mirror.center_mm - point[mirror.axis];
    }
    point
}

/// A line profile in the part's local XY where `mirrors` leave it; a mirror
/// across the extrusion leaves it as it is.
fn reflected_profile(
    profile: &[ProgramProfileSegment],
    mirrors: &[&ProgramMirror],
) -> Vec<ProgramProfileSegment> {
    let at = |p: [f64; 2]| {
        let [x, y, _] = reflected([p[0], p[1], 0.0], mirrors);
        [x, y]
    };
    profile
        .iter()
        .map(|s| ProgramProfileSegment::line(s.name.clone(), at(s.start_mm), at(s.end_mm)))
        .collect()
}

/// The through-cuts of `part` that bore along its whole grain (when `outline`
/// is its cross-section) and, when another operation changes its section along
/// the grain in a way the check does not model, why. `outline`, `grain` and
/// the bores are where the part's mirrors leave them; `narrower` is the
/// narrower side of the section.
fn changes(
    part: &Part,
    outline: Option<&[ProgramProfileSegment]>,
    grain: [f64; 3],
    narrower: f64,
) -> (Vec<Vec<ProgramProfileSegment>>, Option<&'static str>) {
    let mut bores = Vec::new();
    let mut unmodelled = None;
    // A hole across the grain or a rounded or chamfered edge up to 5 % of
    // the narrower side takes at most 5 % of the section at one place
    // (a 10 mm bolt hole or an eased arris in a 200 mm column).
    let small = 0.05 * narrower;
    // (start, end, diameter) along the grain of each such hole.
    let mut across = Vec::new();
    for (index, operation) in part.operations.iter().enumerate() {
        let later = mirrors(&part.operations[index + 1..]);
        match operation {
            ProgramOperation::Cut(cut)
                if outline.is_some_and(|outline| {
                    cut.depth_mm.is_none()
                        && cut.segments.iter().all(ProgramProfileSegment::is_line)
                        && encloses(outline, &reflected_profile(&cut.segments, &later))
                }) =>
            {
                bores.push(reflected_profile(&cut.segments, &later));
            }
            ProgramOperation::Cut(_)
            | ProgramOperation::Pocket(_)
            | ProgramOperation::Shell(_)
            | ProgramOperation::Boolean(_)
            | ProgramOperation::FaceOffset(_) => {
                unmodelled = Some(
                    "the member's section changes along its length (a cut, pocket, shell, \
                     boolean or face offset other than a bore through it)",
                );
            }
            ProgramOperation::Hole(hole)
                if dot(
                    sub(reflected(hole.inward, &later), reflected([0.0; 3], &later)),
                    grain,
                )
                .abs()
                    <= ROUNDING * length(hole.inward)
                    && hole.diameter_mm <= small =>
            {
                let at = dot(reflected(hole.entry_mm, &later), grain);
                let radius = hole.diameter_mm / 2.0;
                across.push((at - radius, at + radius, hole.diameter_mm));
            }
            ProgramOperation::Finish(finish) if finish.radius_mm <= small => {}
            ProgramOperation::Hole(_) => {
                unmodelled = Some(
                    "the member's section changes along its length (a hole along it, or one \
                     across it wider than 5 % of its narrower side)",
                );
            }
            ProgramOperation::Finish(_) => {
                unmodelled = Some(
                    "the member's section changes along its length (a rounded or chamfered \
                     edge larger than 5 % of its narrower side)",
                );
            }
            // Taken by `reflected` above and in `shape`.
            ProgramOperation::Mirror(_) => {}
        }
    }
    // Holes whose spans along the grain overlap meet in one section, so the
    // 5 % holds for their diameters together.
    let crowded = across.iter().any(|(start, ..)| {
        let together: f64 = across
            .iter()
            .filter(|(from, to, _)| *from <= *start && *start < *to)
            .map(|(.., diameter)| diameter)
            .sum();
        together > small
    });
    if crowded {
        unmodelled = Some(
            "the member's section changes along its length (holes across it that together \
             take more than 5 % of its narrower side in one section)",
        );
    }
    (bores, unmodelled)
}

/// Why a member whose section `twists` is not checked.
fn twisting(column: bool) -> &'static str {
    if column {
        "the section's shear centre is off its centroid (an angle or a channel), so it \
         buckles in a flexural-torsional mode; flexural-torsional buckling is not checked"
    } else {
        "a vertical load passes beside the section's shear centre (an angle, or a channel \
         open to the side) and twists the member; torsion is not checked"
    }
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
    let segments = &reflected_profile(segments, &mirrors(&part.operations));
    let size = extents(part);
    let world = axes(part);
    let long = (0..3)
        .max_by(|a, b| size[*a].total_cmp(&size[*b]))
        .unwrap_or(0);
    if long == 2 {
        // The profile is the cross-section; b and h along its more vertical local axis.
        let depth = usize::from(world[1][2].abs() >= world[0][2].abs());
        let (bores, unmodelled) =
            changes(part, Some(segments), [0.0, 0.0, 1.0], size[0].min(size[1]));
        // Bending takes it turned as placed: in the vertical plane across the grain.
        let [across, vertical] = across_grain(world[2]);
        let turned = |profile: &[ProgramProfileSegment]| -> Vec<ProgramProfileSegment> {
            let at = |p: [f64; 2]| {
                let point: [f64; 3] =
                    std::array::from_fn(|i| p[0] * world[0][i] + p[1] * world[1][i]);
                [dot(point, across), dot(point, vertical)]
            };
            profile
                .iter()
                .map(|s| ProgramProfileSegment::line(String::new(), at(s.start_mm), at(s.end_mm)))
                .collect()
        };
        let outline = turned(segments);
        let bored: Vec<Vec<ProgramProfileSegment>> =
            bores.iter().map(|bore| turned(bore)).collect();
        let bores: Vec<&[ProgramProfileSegment]> = bored.iter().map(Vec::as_slice).collect();
        let section = Section::new(&outline, &bores, 1);
        let area = section.area;
        let radius = (section.least() / area).sqrt();
        if !(area > 0.0 && radius > 0.0 && radius.is_finite()) {
            return Err("the member's section has no area");
        }
        let column = horizontal(frame_axis) < COLUMN_COS;
        // A beam's section axis is vertical, so a section symmetric about it
        // only bends; a column's is not, and only a section symmetric about
        // its centroid has its shear centre there.
        let twists = section.twists(!column).then_some(twisting(column));
        return Ok(Shape {
            grain: frame_axis,
            b: size[1 - depth],
            h: size[depth],
            area,
            net_area: area,
            buckling_area: area,
            bending: Along::Prismatic(Bending::of(&section)),
            radius,
            unmodelled,
            twists,
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
    // The chord is linear between the profile's corners, so its least value lies
    // at an end of a stretch between two of them: it is measured just inside
    // both ends of every stretch. A notch or lap anywhere, ends included, takes the
    // whole force on its net section; buckling, which the pinned ends hardly
    // affect, takes the thinnest section more than twice the width from them.
    let mut corners: Vec<f64> = segments.iter().map(|s| s.start_mm[long]).collect();
    corners.sort_by(f64::total_cmp);
    corners.dedup_by(|a, b| (*a - *b).abs() < ROUNDING);
    let reach = 2.0 * thickest / along;
    // An end cut on a slope has no face across the grain: the force enters
    // along the slope, so its tip is a bearing, not a net section.
    let faced = |end: f64| {
        segments.iter().any(|s| {
            (s.start_mm[long] - end).abs() < ROUNDING && (s.end_mm[long] - end).abs() < ROUNDING
        })
    };
    let pointed: Vec<f64> = [corners[0], corners[corners.len() - 1]]
        .into_iter()
        .filter(|end| !faced(*end))
        .collect();
    let stations: Vec<(f64, f64)> = corners
        .windows(2)
        .flat_map(|pair| [pair[0] + ROUNDING, pair[1] - ROUNDING])
        .filter(|u| pointed.iter().all(|end| (u - end).abs() > 2.0 * ROUNDING))
        .map(|u| (u, chord(segments, long, u) * along))
        .collect();
    let thinnest = stations.iter().map(|s| s.1).fold(thickest, f64::min);
    let thinnest_inside = stations
        .iter()
        .filter(|(u, _)| *u - start > reach && start + length - *u > reach)
        .map(|s| s.1)
        .fold(thickest, f64::min);
    let depth_in_plane = across[2].abs() >= world[2][2].abs();
    let (b, h) = if depth_in_plane {
        (*distance_mm, thickest)
    } else {
        (thickest, *distance_mm)
    };
    // The grain runs in the profile plane, so no cut along the extrusion bores along it.
    let (_, unmodelled) = changes(part, None, [d[0], d[1], 0.0], b.min(h));
    let profile = segments.clone();
    // Bending takes the section cut across the profile at each place, turned
    // as placed: in the vertical plane across the grain.
    let section_axes = across_grain(grain);
    let in_section = |axis: [f64; 3]| section_axes.map(|s| dot(axis, s));
    let bending = Along::Profile {
        profile: segments.clone(),
        long,
        start,
        long_axis: in_section(world[long]),
        other: in_section(world[1 - long]),
        extrusion: in_section(world[2]),
        distance: *distance_mm,
        corners: corners.iter().map(|u| u - start).collect(),
    };
    // One stretch of the cut is a rectangle, whose shear centre is its
    // centroid; stretches side by side may not be symmetric.
    let column = horizontal(grain) < COLUMN_COS;
    let twists = corners
        .windows(2)
        .filter_map(|pair| bending.outline(&bending.cut((pair[0] + pair[1]) / 2.0 - start)))
        .any(|outline| Section::new(&outline, &[], 1).twists(!column))
        .then_some(twisting(column));
    Ok(Shape {
        grain,
        b,
        h,
        area: b * h,
        net_area: distance_mm * thinnest,
        buckling_area: distance_mm * thinnest_inside,
        bending,
        radius: distance_mm.min(thinnest_inside) / 12f64.sqrt(),
        unmodelled,
        twists,
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

/// Bending moments (N mm), shear forces (N) and deflections (mm, for E = 1;
/// vertical and sideways) per load component at the stations `xs`.
struct Segment {
    p: f64,
    q: f64,
    kind: Kind,
    xs: Vec<f64>,
    moments: Vec<Vec<f64>>,
    shears: Vec<Vec<f64>>,
    deflections: [Vec<Vec<f64>>; 2],
}

/// `end_moments`: per load component, the moments over the supports at p and q of
/// a span continuous over them (hogging negative; along the member, without
/// `lever`); cantilevers ignore them. `xs` are the stations from p to q and
/// `flexibility` the curvature per unit moment at each (vertical and
/// sideways), None where there is no material.
fn segment(
    kind: Kind,
    components: &[Vec<Piece>],
    end_moments: &[[f64; 2]],
    lever: f64,
    arc: f64,
    xs: Vec<f64>,
    flexibility: &[Option<[f64; 2]>],
) -> Segment {
    let ends = match kind {
        Kind::Span => (false, false),
        Kind::LeftCantilever => (true, false),
        Kind::RightCantilever => (false, true),
    };
    let last = xs.len() - 1;
    let (p, q) = (xs[0], xs[last]);
    let widths: Vec<f64> = xs
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) * arc)
        .collect();
    let mut moments = Vec::new();
    let mut shears = Vec::new();
    let mut deflections = [Vec::new(), Vec::new()];
    for (patches, [m_p, m_q]) in components.iter().zip(end_moments) {
        let pieces = clipped(patches, p, q, ends);
        let total: f64 = pieces.iter().map(|piece| piece.2).sum();
        // The shear at the ends takes a point load on a support into it.
        let (m, mut v, end_shears): (Vec<f64>, Vec<f64>, [f64; 2]) = match kind {
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
                let v = xs.iter().map(|&x| r_left - left_of(&pieces, x).0).collect();
                (m, v, [r_left, total - r_left])
            }
            Kind::LeftCantilever => (
                xs.iter().map(|&x| -left_of(&pieces, x).1 * lever).collect(),
                xs.iter().map(|&x| left_of(&pieces, x).0).collect(),
                [0.0, total],
            ),
            Kind::RightCantilever => (
                xs.iter()
                    .map(|&x| -right_of(&pieces, x).1 * lever)
                    .collect(),
                xs.iter().map(|&x| right_of(&pieces, x).0).collect(),
                [total, 0.0],
            ),
        };
        (v[0], v[last]) = (end_shears[0], end_shears[1]);
        let integrate = |values: &[f64], widths: &[f64]| {
            let mut out = vec![0.0; values.len()];
            for j in 1..values.len() {
                out[j] = out[j - 1] + (values[j - 1] + values[j]) / 2.0 * widths[j - 1];
            }
            out
        };
        for (direction, deflections) in deflections.iter_mut().enumerate() {
            // w'' = -M / EI along the true length, vertical and sideways; no
            // material, no curvature (only at a pointed free end, where M is 0).
            let curvature: Vec<f64> = m
                .iter()
                .zip(flexibility)
                .map(|(value, flexibility)| flexibility.map_or(0.0, |f| -value * f[direction]))
                .collect();
            let w = match kind {
                Kind::Span => {
                    let raw = integrate(&integrate(&curvature, &widths), &widths);
                    let end = raw[last];
                    raw.iter()
                        .zip(&xs)
                        .map(|(value, x)| value - end * (x - p) / (q - p))
                        .collect()
                }
                Kind::RightCantilever => integrate(&integrate(&curvature, &widths), &widths),
                Kind::LeftCantilever => {
                    let reversed: Vec<f64> = curvature.iter().rev().copied().collect();
                    let back: Vec<f64> = widths.iter().rev().copied().collect();
                    let mut w = integrate(&integrate(&reversed, &back), &back);
                    w.reverse();
                    w
                }
            };
            deflections.push(w);
        }
        moments.push(m);
        shears.push(v);
    }
    Segment {
        p,
        q,
        kind,
        xs,
        moments,
        shears,
        deflections,
    }
}

/// The largest magnitude of an effect, from its `values` under the load
/// components of `kinds` (indices into `LOAD_KINDS`) with `factors`. The
/// permanent load takes its factor where it adds to the effect and `relieving`
/// where it relieves it (1.0 at the ultimate limit state, EN 1990 table
/// A1.2(B)); each stretch of a variable load counts only where it adds.
fn envelope(factors: &Loads, kinds: &[usize], values: &[f64], relieving: Option<f64>) -> f64 {
    let (largest, smallest) = extremes(factors, kinds, values, relieving);
    largest.abs().max(smallest.abs())
}

/// The largest and the smallest signed effect that `envelope` takes the
/// magnitude of.
fn extremes(
    factors: &Loads,
    kinds: &[usize],
    values: &[f64],
    relieving: Option<f64>,
) -> (f64, f64) {
    let mut permanent = 0.0;
    let (mut adding, mut relieving_variable) = (0.0, 0.0);
    for (kind, value) in kinds.iter().zip(values) {
        if *kind == 0 {
            permanent += value;
            continue;
        }
        let effect = factors.get(LOAD_KINDS[*kind]).copied().unwrap_or(0.0) * value;
        if effect > 0.0 {
            adding += effect;
        } else {
            relieving_variable += effect;
        }
    }
    let gamma = factors.get(LOAD_KINDS[0]).copied().unwrap_or(0.0);
    let low = relieving.unwrap_or(gamma);
    (
        (gamma * permanent).max(low * permanent) + adding,
        (gamma * permanent).min(low * permanent) + relieving_variable,
    )
}

/// gamma_G of a permanent load that relieves an ultimate effect.
const GAMMA_G_FAVOURABLE: f64 = 1.0;

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
) -> (Vec<Check>, bool, Vec<String>) {
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
    // The permanent load is one action (one source); every variable kind is laid
    // stretch by stretch between the supports, and each effect takes a stretch
    // only where it adds to it (load patterns, EN 1990 6.4.3.1(4)).
    let mut kinds: Vec<usize> = Vec::new();
    let mut components: Vec<Vec<Piece>> = Vec::new();
    for (kind, pieces) in per_kind.into_iter().enumerate() {
        let stretches = if kind == 0 || supports.is_empty() {
            vec![pieces]
        } else {
            continuous_span::by_stretch(&supports, &pieces)
        };
        for stretch in stretches.into_iter().filter(|stretch| !stretch.is_empty()) {
            kinds.push(kind);
            components.push(stretch);
        }
    }
    let support_moments: Vec<Vec<f64>> = if supports.is_empty() {
        Vec::new()
    } else {
        components
            .iter()
            .map(|pieces| continuous_span::solve(&supports, &bedded, pieces).moments)
            .collect()
    };
    let no_moments = vec![[0.0; 2]; components.len()];
    // Each segment is checked evenly along it and either side of every place
    // its section changes, on the section there.
    let mut known = HashMap::new();
    let mut build = |p: f64, q: f64, kind: Kind, end_moments: &[[f64; 2]]| {
        let mut xs: Vec<f64> = (0..=STEPS)
            .map(|j| p + (q - p) * j as f64 / STEPS as f64)
            .chain(
                shape
                    .bending
                    .corners()
                    .iter()
                    .flat_map(|x| [x - ROUNDING, x + ROUNDING])
                    .filter(|x| p < *x && *x < q),
            )
            .collect();
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let sections: Vec<Option<Bending>> = xs
            .iter()
            .map(|x| shape.bending.at(*x, &mut known))
            .collect();
        let flexibility: Vec<Option<[f64; 2]>> =
            sections.iter().map(|s| s.map(|s| s.flexibility)).collect();
        let seg = segment(
            kind,
            &components,
            end_moments,
            lever,
            shape.arc,
            xs,
            &flexibility,
        );
        (seg, sections)
    };
    let mut segments = Vec::new();
    if let (Some(first), Some(last)) = (supports.first(), supports.last()) {
        if *first > 1.0 {
            segments.push(build(0.0, *first, Kind::LeftCantilever, &no_moments));
        }
        for (j, pair) in supports.windows(2).enumerate() {
            if !bedded[j] && pair[1] - pair[0] > 1.0 {
                let ends: Vec<[f64; 2]> = support_moments
                    .iter()
                    .map(|moments| [moments[j], moments[j + 1]])
                    .collect();
                segments.push(build(pair[0], pair[1], Kind::Span, &ends));
            }
        }
        if load.length_mm - last > 1.0 {
            segments.push(build(
                *last,
                load.length_mm,
                Kind::RightCantilever,
                &no_moments,
            ));
        }
    }
    let spans_checked =
        bearings.len() < 2 || segments.iter().any(|(seg, _)| seg.kind == Kind::Span);
    let (b, h) = (shape.b, shape.h);
    // kh is of the member's depth (EC5 3.2(3)), not of a notched section's.
    let kh = if class.glulam {
        (600.0 / h).powf(0.1).clamp(1.0, 1.1)
    } else {
        (150.0 / h).powf(0.2).clamp(1.0, 1.3)
    };
    let mut bending = Best(None);
    let mut shear = Best(None);
    let mut inst = Best(None);
    let mut fin = Best(None);
    for (seg, sections) in &segments {
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
            // The largest utilization along the segment by an effect (one
            // value per load component at each station) over the section's
            // `resistance` there. Rounding left of a zero effect is none (the
            // moment over an end support, at a free end); a station with no
            // material carries nothing, and an effect there is not a number.
            let largest = |effects: &[Vec<f64>], resistance: &dyn Fn(&Bending) -> f64| {
                let values: Vec<f64> = (0..seg.xs.len())
                    .map(|j| {
                        let components: Vec<f64> = effects.iter().map(|e| e[j]).collect();
                        envelope(
                            &combination.factors,
                            &kinds,
                            &components,
                            Some(GAMMA_G_FAVOURABLE),
                        )
                    })
                    .collect();
                let scale = values.iter().fold(1.0, |most: f64, v| most.max(*v));
                values
                    .iter()
                    .zip(sections)
                    .map(|(value, section)| match section {
                        _ if *value <= ROUNDING * scale => 0.0,
                        Some(section) => value / resistance(section),
                        None => f64::NAN,
                    })
                    .fold(0.0, |most: f64, u| {
                        if most.is_nan() || u.is_nan() {
                            f64::NAN
                        } else {
                            most.max(u)
                        }
                    })
            };
            let fm = design.strength(class.fm_k, combination.duration);
            let fv = design.strength(class.fv_k, combination.duration);
            let in_bending = largest(&seg.moments, &|s| s.modulus * fm * kh);
            bending.offer("bending", in_bending, &combination.label, &at);
            let in_shear = largest(&seg.shears, &|s| KCR * s.shear_area * fv / cos_t);
            shear.offer("shear", in_shear, &combination.label, &at);
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
            // The resultant of the vertical and the sideways deflection, each
            // at its own worst load pattern.
            let largest = |factors: &Loads| {
                (0..seg.xs.len())
                    .map(|j| {
                        let [vertical, sideways] = seg.deflections.each_ref().map(|direction| {
                            let values: Vec<f64> = direction.iter().map(|w| w[j]).collect();
                            envelope(factors, &kinds, &values, None)
                        });
                        vertical.hypot(sideways) / class.e0_mean
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
    let mut unchecked = Vec::new();
    for zone in &zones {
        let names = zone.names(load);
        let reaction = zone.total(load, false);
        // Notched bearings (6.5.2).
        if let Some(depth_at) = &shape.depth_at {
            let h_ef = (0..=8)
                .map(|k| depth_at(zone.lo + (zone.hi - zone.lo) * f64::from(k) / 8.0))
                .filter(|depth| *depth > 1.0)
                .fold(f64::MAX, f64::min);
            if h_ef < 0.95 * h && (zone.hi - zone.lo) * shape.arc > BED_MM {
                // A long bearing supports the member along it: x of fig. 6.11
                // has no reaction line to start from.
                unchecked.push(format!(
                    "it is notched over {names}, a bearing longer than {BED_MM} mm, where the line of its reaction is not known: the notch is not checked (EN 1995-1-1 6.5.2)"
                ));
            } else if h_ef < 0.95 * h {
                let kn = if class.glulam { 6.5 } else { 5.0 };
                let alpha = h_ef / h;
                let x = notch_corner(depth_at, zone, load.length_mm, 0.95 * h, shape.arc);
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
    unchecked.extend(tension_notch(shape, &segments, &zones, uls, &kinds));
    (checks, spans_checked, unchecked)
}

/// EN 1995-1-1 6.5.1(2): the stress concentration at a change in depth may be
/// ignored where the change slopes at 1:10 or flatter, or lies on the side
/// in compression.
const NOTCH_SLOPE: f64 = 0.1;

/// Review 2026-10-19 PG-15: where a member's depth changes more steeply than
/// 1:10 on a side that is in tension, away from its bearings (a notch over a
/// bearing is checked with kv, 6.5.2), the stress concentration is not
/// checked; the first such place, as a reason.
fn tension_notch(
    shape: &Shape,
    segments: &[(Segment, Vec<Option<Bending>>)],
    zones: &[Zone],
    uls: &[Combination],
    kinds: &[usize],
) -> Option<String> {
    let corners = shape.bending.corners();
    if corners.len() < 3 {
        return None;
    }
    // The signed moments (sagging positive) at the stations within `near` of x.
    let moments = |x: f64, near: f64| {
        let mut bounds = Vec::new();
        for (seg, _) in segments {
            for (j, station) in seg.xs.iter().enumerate() {
                if (station - x).abs() > near {
                    continue;
                }
                let values: Vec<f64> = seg.moments.iter().map(|m| m[j]).collect();
                for combination in uls {
                    bounds.push(extremes(
                        &combination.factors,
                        kinds,
                        &values,
                        Some(GAMMA_G_FAVOURABLE),
                    ));
                }
            }
        }
        bounds
    };
    let scale = moments(0.0, f64::MAX)
        .iter()
        .fold(0.0, |most: f64, (a, b)| most.max(a.abs()).max(b.abs()));
    let inner = &corners[1..corners.len() - 1];
    for (i, &c) in inner.iter().enumerate() {
        let (before, after) = (corners[i], corners[i + 2]);
        // Underside (0) and top (1); `inward` is the way the edge moves into the member.
        for (side, inward) in [(0, 1.0), (1, -1.0)] {
            let level = |x: f64| shape.bending.edges(x).map(|e| e[side]);
            let stations = [
                (before + c) / 2.0,
                c - ROUNDING,
                c + ROUNDING,
                (c + after) / 2.0,
            ];
            let Some(levels) = stations
                .iter()
                .map(|x| level(*x))
                .collect::<Option<Vec<f64>>>()
            else {
                continue;
            };
            // How fast the edge moves into the member before c, across it (a
            // step) and after it. Where that drops by more than 1:10 the edge
            // has a re-entrant corner: a step, or the floor of a sloped notch;
            // not where it turns in (a cut end tapering off).
            let inwards = |a: usize, b: usize| {
                (levels[b] - levels[a]) * inward / ((stations[b] - stations[a]) * shape.arc)
            };
            let (into, across, out) = (inwards(0, 1), inwards(1, 2), inwards(2, 3));
            if (into - across).max(across - out).max(into - out) <= NOTCH_SLOPE {
                continue;
            }
            // The side the larger moment there pulls; a small reversed moment
            // (variable load on the overhangs only) does not count.
            let (sagging, hogging) = moments(c, 2.0 * ROUNDING)
                .iter()
                .fold((0.0, 0.0), |(s, h): (f64, f64), (largest, smallest)| {
                    (s.max(*largest), h.max(-*smallest))
                });
            let in_tension = sagging.max(hogging) > ROUNDING * scale
                && if side == 0 {
                    sagging >= hogging
                } else {
                    hogging >= sagging
                };
            if !in_tension {
                continue;
            }
            // A notch over a bearing: the edge stays set back from c to it.
            let outer = levels
                .iter()
                .copied()
                .fold(f64::MAX, |most, l| most.min(l * inward))
                * inward;
            let set_back = |x: f64| level(x).is_none_or(|l| (l - outer) * inward > ROUNDING);
            let over_bearing = zones.iter().any(|zone| {
                if zone.lo <= c && c <= zone.hi {
                    return true;
                }
                let (lo, hi) = if zone.hi < c {
                    (zone.hi, c)
                } else {
                    (c, zone.lo)
                };
                let mut probes: Vec<f64> = corners
                    .iter()
                    .filter(|x| lo < **x && **x < hi)
                    .flat_map(|x| [x - ROUNDING, x + ROUNDING])
                    .chain(
                        corners
                            .windows(2)
                            .map(|pair| (pair[0] + pair[1]) / 2.0)
                            .filter(|x| lo < *x && *x < hi),
                    )
                    .collect();
                probes.push(if zone.hi < c {
                    c - ROUNDING
                } else {
                    c + ROUNDING
                });
                probes.into_iter().all(set_back)
            });
            if !over_bearing {
                return Some(format!(
                    "its depth changes more steeply than 1:10 on its {}, which is in tension, at {} mm, away from its bearings: the stress concentration there is not checked (EN 1995-1-1 6.5.1(2))",
                    if side == 0 { "underside" } else { "top" },
                    c.round()
                ));
            }
        }
    }
    None
}

/// EC5 6.5.2, fig. 6.11: `x`, mm, from the middle of the reaction in `zone`
/// to the corner of the notch, where the member is `full` deep again. A
/// notch with a corner on each side takes the farther one. `depth_at` and
/// `length` are along the load frame, `arc` mm per unit of it (review
/// 2026-10-17 PG-3: the half bearing length was taken instead).
fn notch_corner(
    depth_at: &dyn Fn(f64) -> f64,
    zone: &Zone,
    length: f64,
    full: f64,
    arc: f64,
) -> f64 {
    let middle = (zone.lo + zone.hi) / 2.0;
    let step = 1.0 / arc;
    let corner = |towards: f64| {
        let mut notched = middle;
        loop {
            let next = notched + towards * step;
            if !(0.0..=length).contains(&next) {
                return 0.0;
            }
            if depth_at(next) >= full {
                // The corner lies between the last notched step and this one.
                let (mut a, mut b) = (notched, next);
                for _ in 0..40 {
                    let half = (a + b) / 2.0;
                    if depth_at(half) >= full {
                        b = half;
                    } else {
                        a = half;
                    }
                }
                return (b - middle).abs() * arc;
            }
            notched = next;
        }
    };
    corner(1.0).max(corner(-1.0))
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
    let lambda_rel =
        |radius: f64| length / radius / std::f64::consts::PI * (class.fc0_k / class.e0_05).sqrt();
    let beta = if class.glulam { 0.1 } else { 0.2 };
    let kc = |lr: f64| {
        if lr <= 0.3 {
            return 1.0;
        }
        let k = 0.5 * (1.0 + beta * (lr - 0.3) + lr * lr);
        1.0 / (k + (k * k - lr * lr).sqrt())
    };
    let kc = kc(lambda_rel(shape.radius));
    // The loads are vertical; a leaning column pinned at its ends carries
    // them along its grain as V / cos of its slope.
    let along = 1.0 / shape.grain[2].abs();
    let mut best = Best(None);
    for combination in uls {
        let force = factor(&combination.factors, &load.loads_n) * along;
        let strength = design.strength(class.fc0_k, combination.duration);
        let buckling = force / shape.buckling_area / (kc * strength);
        let net = force / shape.net_area / strength;
        let detail = if net > buckling {
            format!("net section {} mm² at a notch", shape.net_area.round())
        } else if shape.buckling_area < shape.area - ROUNDING {
            format!(
                "length {} mm, thinnest section {} mm²",
                length.round(),
                shape.buckling_area.round()
            )
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
    let parts = model.parts_by_name();
    for load in &loads.members {
        let part = parts.get(load.part.as_str()).copied();
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
            match shape.unmodelled.or(shape.twists) {
                Some(reason) => missing.push(reason.to_owned()),
                None => check.checks.push(column_check(load, shape, &design, &uls)),
            }
        } else {
            if let Some(reason) = shape.unmodelled.or(shape.twists) {
                // Bending, shear, deflection and notches on the full section
                // would overstate a beam that a cut, pocket or hole weakens,
                // or that the load twists.
                missing.push(reason.to_owned());
            } else {
                let (checks, spans_checked, unchecked) =
                    beam_checks(load, shape, &design, &uls, &sls);
                check.checks = checks;
                missing.extend(unchecked);
                // Separate bearings always leave a span; without one, bending,
                // shear and deflection were never checked.
                if !spans_checked {
                    missing.push("no span between its separate bearings was checked".to_owned());
                }
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
/// The status is decided on the unrounded values; rounding is for the report.
fn settle(member: &mut MemberCheck) {
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
    for check in &mut member.checks {
        check.utilization = round3(check.utilization);
    }
    member.utilization = member.utilization.map(round3);
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

    #[test]
    fn a_utilization_just_over_one_fails_even_though_it_rounds_to_one() {
        let over = settled(&[1.0004, 0.5]);
        assert_eq!(over.status, "fail");
        assert_eq!(over.governing.as_deref(), Some("bending"));
        assert_eq!(settled(&[1.0, 0.5]).status, "pass");
    }
}
