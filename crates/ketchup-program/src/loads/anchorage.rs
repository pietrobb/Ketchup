//! Where an anchorage holds a member sideways from: across the member and
//! beyond its side, standing on the ground through something besides the
//! member (review 2026-11-12 R1112-PG-500, PG-501: grounded parts wider than
//! the member, set aside on it, or sheets on its top anchoring a diaphragm,
//! held up by the member alone, held it); and how stiffly, and up to what
//! force, it holds there ([`stiffness`]; review 2026-11-13 r95 structural
//! point 1: every threshold of slenderness was passed by the next shape past
//! it).

mod joint;
mod sheet;
mod stiffness;

use super::{BraceAnchorage, Frame, Support, depth, middle};
use crate::frame::{Obb, intersection_vertices};
use crate::load_path::on_floor;
use crate::model::{Part, ProgramModel, convex_hull};
use joint::{Hold, Pivot, Strut, hold, strut_line};
use ketchup_geometry::linalg::{self, cross, dot, length, scale, sub};
use ketchup_model::tolerance::{CONTACT_FIT_MM, ROUNDING};
use sheet::{Spring, on_springs};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use stiffness::{SLIP_PER_FASTENER, Tie, brace_give, part_capacity, part_give, pushed_along};

/// The modulus of elasticity, mean and fifth percentile, and the compressive
/// strength f_c,0,k, N/mm², of a part of no wood strength class:
/// softwood C24 (EN 338).
const SOFTWOOD_MODULI: (f64, f64, f64) = (11_000.0, 7_400.0, 21.0);

/// How a path to the ground holds a member at a point sideways.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Anchorage {
    /// The force across the member per millimetre the point moves, N/mm.
    pub(super) stiffness_n_mm: f64,
    /// The largest force across the member before a part of the path buckles, N.
    pub(super) capacity_n: f64,
    /// The part giving the most, with the joint it leaves by.
    pub(super) weakest: usize,
    /// The part buckling first.
    pub(super) buckling: usize,
    /// How long the rod is the path starts with at the point held, mm: the
    /// length a brace pushing it buckles over.
    pub(super) over_mm: f64,
}

/// What each part touches face to face and where the floor lies: whether an
/// anchorage reaches the ground by a path of its own, and how stiffly.
pub(super) struct Footing<'a> {
    pub(super) parts: &'a [Part],
    frames: &'a [Frame],
    /// The load-path members, none of which is ground for another.
    member: &'a [bool],
    /// The long axis of each part spanning between its supports, which bends
    /// across it like a member (R1113r95-PG-705: a second member outside the
    /// load path was ground).
    spanning: Vec<Option<[f64; 3]>>,
    grounded: Vec<bool>,
    /// Each part's modulus of elasticity, mean and fifth percentile, and its
    /// compressive strength f_c,0,k, N/mm².
    moduli: Vec<(f64, f64, f64)>,
    floor: f64,
    touching: Vec<Vec<usize>>,
    /// The fasteners the fastening joint()s between each pair of parts, lesser
    /// index first, name.
    joined: BTreeMap<(usize, usize), Vec<[f64; 3]>>,
    /// Answers by anchor, member, the parts kept apart and the length run
    /// before the anchor, with the parts the paths run through.
    known: RefCell<BTreeMap<Asked, Option<Reached>>>,
    /// The parts each anchor's search met turning about one fastener.
    met_pivots: RefCell<BTreeMap<usize, BTreeSet<usize>>>,
}

/// An anchorage and the parts its paths to the ground run through.
type Reached = (Anchorage, BTreeSet<usize>);

/// Where boxes `a` and `b`, touching, meet: the middle of the corners of the
/// region they share within the fit.
fn meeting(a: &Obb, b: &Obb) -> Option<[f64; 3]> {
    let (sum, count) = shared(a, b)
        .into_iter()
        .fold(([0.0; 3], 0.0), |(sum, count), corner| {
            (std::array::from_fn(|i| sum[i] + corner[i]), count + 1.0)
        });
    (count > 0.0).then(|| sum.map(|sum| sum / count))
}

/// How far from `at` along `way` (unit) the line through it runs inside box
/// `obb`, within the fit: before and past `at`, or None where it misses it.
fn inside_along(obb: &Obb, at: [f64; 3], way: [f64; 3]) -> Option<(f64, f64)> {
    let mut stretch = (f64::NEG_INFINITY, f64::INFINITY);
    for k in 0..3 {
        let offset = dot(sub(at, obb.centre), obb.axes[k]);
        let half = obb.half[k] + CONTACT_FIT_MM;
        let rate = dot(way, obb.axes[k]);
        if rate.abs() <= f64::EPSILON {
            if offset.abs() > half {
                return None;
            }
            continue;
        }
        let (lo, hi) = ((-half - offset) / rate, (half - offset) / rate);
        stretch = (stretch.0.max(lo.min(hi)), stretch.1.min(lo.max(hi)));
    }
    (stretch.0 <= stretch.1).then_some(stretch)
}

/// The corners of the region boxes `a` and `b` share within the fit.
fn shared(a: &Obb, b: &Obb) -> Vec<[f64; 3]> {
    let grown = Obb {
        half: a.half.map(|half| half + CONTACT_FIT_MM),
        ..*a
    };
    intersection_vertices(&[grown, *b], ROUNDING)
}

/// The thinnest side of box `obb`.
fn least_side(obb: &Obb) -> f64 {
    2.0 * obb.half.into_iter().fold(f64::INFINITY, f64::min)
}

/// Where the grounded box `obb` standing on the floor at `floor` is held for a
/// path entering it at `from`: the point of its foot nearest under `from`.
fn foot(obb: &Obb, from: [f64; 3], floor: f64) -> [f64; 3] {
    let corners: Vec<[f64; 3]> = (0..8)
        .map(|bits: usize| {
            std::array::from_fn(|n| {
                obb.centre[n]
                    + (0..3)
                        .map(|i| {
                            let sign = if bits >> i & 1 == 0 { -1.0 } else { 1.0 };
                            obb.axes[i][n] * obb.half[i] * sign
                        })
                        .sum::<f64>()
            })
        })
        .collect();
    let low = corners.iter().map(|c| c[2]).fold(f64::INFINITY, f64::min);
    let bottom: Vec<[f64; 3]> = corners
        .into_iter()
        .filter(|c| c[2] <= low.max(floor) + CONTACT_FIT_MM)
        .collect();
    let under = [from[0], from[1], low];
    match bottom.as_slice() {
        [] => under,
        [corner] => *corner,
        [a, b] => {
            let edge = sub(*b, *a);
            let t = (dot(sub(under, *a), edge) / dot(edge, edge).max(f64::EPSILON)).clamp(0.0, 1.0);
            std::array::from_fn(|i| a[i] + edge[i] * t)
        }
        _ => {
            let offset = sub(under, obb.centre);
            let mut point = obb.centre;
            for axis in 0..3 {
                let along = dot(offset, obb.axes[axis]).clamp(-obb.half[axis], obb.half[axis]);
                point = std::array::from_fn(|i| point[i] + obb.axes[axis][i] * along);
            }
            point
        }
    }
}

impl Anchorage {
    /// This anchorage as a brace reports it, by the names of `parts`.
    pub(super) fn reported(self, parts: &[Part]) -> BraceAnchorage {
        let thousandths = |value: f64| (value * 1000.0).round() / 1000.0 + 0.0;
        let buckles = self.capacity_n.is_finite();
        BraceAnchorage {
            through: parts[self.weakest].name.clone(),
            stiffness_n_mm: thousandths(self.stiffness_n_mm.min(1.0e12)),
            capacity_n: buckles.then(|| thousandths(self.capacity_n)),
            buckling_in: buckles.then(|| parts[self.buckling].name.clone()),
        }
    }
}

/// The force a brace puts on its anchor for a unit force across the member at
/// the point it holds: acting `along` (unit), entering the anchor at `enter`
/// and turning it about `held`, through which it runs. A brace fixed at both
/// ends passes the force across the member on at the point of the member, the
/// brace's length a lever on the anchor; one turning about a joint passes on
/// only the force along its axis, through the joint (review 2026-11-21
/// R1121ac-PG-1000: the anchor was asked only how it holds a force across the
/// member where it meets the brace, and a column bending its weak way under a
/// strut at 40°, or twisting under a brace fixed 250 mm off its axis, held
/// the member a hundred times too stiffly).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Probe {
    held: [f64; 3],
    enter: [f64; 3],
    along: [f64; 3],
}

impl Probe {
    fn bits(self) -> [u64; 9] {
        let all = [self.held, self.enter, self.along];
        std::array::from_fn(|i| all[i / 3][i % 3].to_bits())
    }
}

type Asked = (usize, usize, Vec<usize>, u64, Option<[u64; 9]>);

/// Of two ways a point is held, the less stiff with the sooner buckling.
pub(super) fn weaker(a: Anchorage, b: Anchorage) -> Anchorage {
    let stiff = if b.stiffness_n_mm < a.stiffness_n_mm {
        b
    } else {
        a
    };
    let strong = if b.capacity_n < a.capacity_n { b } else { a };
    Anchorage {
        capacity_n: strong.capacity_n,
        buckling: strong.buckling,
        over_mm: a.over_mm.max(b.over_mm),
        ..stiff
    }
}

/// A part resting on supports far apart along its length spans between them
/// and bends like a member (R1113r95-PG-705): its long axis, for each part
/// not `skipped`.
pub(super) fn spanning(
    parts: &[Part],
    supports: &[Vec<Support>],
    skipped: impl Fn(usize) -> bool,
) -> Vec<Option<[f64; 3]>> {
    parts
        .iter()
        .enumerate()
        .map(|(item, part)| {
            if skipped(item) {
                return None;
            }
            let obb = part.obb();
            let long = (0..3)
                .max_by(|a, b| obb.half[*a].total_cmp(&obb.half[*b]))
                .unwrap_or(0);
            let axis = obb.axes[long];
            if axis[2].abs() >= std::f64::consts::FRAC_1_SQRT_2 {
                return None;
            }
            let at: Vec<f64> = supports[item]
                .iter()
                .filter_map(|support| middle(&support.points))
                .map(|point| dot(point, axis))
                .collect();
            let (lo, hi) = at
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), at| {
                    (lo.min(*at), hi.max(*at))
                });
            (at.len() >= 2 && hi - lo > obb.half[long]).then_some(axis)
        })
        .collect()
}

impl<'a> Footing<'a> {
    pub(super) fn of(
        model: &'a ProgramModel,
        member: &'a [bool],
        frames: &'a [Frame],
        spanning: Vec<Option<[f64; 3]>>,
    ) -> Self {
        let parts = &model.parts;
        let boxes: Vec<_> = parts.iter().map(Part::obb).collect();
        let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|a, b| bounds[*a].0[0].total_cmp(&bounds[*b].0[0]));
        let mut touching = vec![Vec::new(); parts.len()];
        for (position, &left) in order.iter().enumerate() {
            for &right in &order[position + 1..] {
                if bounds[right].0[0] > bounds[left].1[0] + CONTACT_FIT_MM {
                    break;
                }
                if model.are_alternatives(&parts[left], &parts[right])
                    || !face_to_face(&boxes[left], &boxes[right])
                {
                    continue;
                }
                touching[left].push(right);
                touching[right].push(left);
            }
        }
        let index = |name: &str| parts.iter().position(|part| part.name == name);
        let mut joined: BTreeMap<(usize, usize), Vec<[f64; 3]>> = BTreeMap::new();
        for joint in model.joints.iter().filter(|joint| joint.fastens()) {
            let (Some(a), Some(b)) = (index(&joint.parts[0]), index(&joint.parts[1])) else {
                continue;
            };
            joined
                .entry((a.min(b), a.max(b)))
                .or_default()
                .extend(&joint.fasteners_mm);
        }
        let names = model.grounded_parts();
        Self {
            parts,
            frames,
            member,
            spanning,
            grounded: parts
                .iter()
                .map(|part| names.contains(&part.name))
                .collect(),
            moduli: parts
                .iter()
                .map(|part| {
                    part.material
                        .as_ref()
                        .and_then(|material| model.strength_classes.get(material))
                        .map_or(SOFTWOOD_MODULI, |class| {
                            (class.e0_mean, class.e0_05, class.fc0_k)
                        })
                })
                .collect(),
            floor: model.floor_z_mm.unwrap_or(0.0),
            touching,
            joined,
            known: RefCell::default(),
            met_pivots: RefCell::default(),
        }
    }

    /// Whether `anchor` stands on the ground through something besides
    /// `member` and the parts `apart` ([`Self::anchorage`]).
    pub(super) fn stands_clear(&self, anchor: usize, member: usize, apart: &[usize]) -> bool {
        self.anchorage(anchor, member, apart).is_some()
    }

    /// How `anchor` holds `member` sideways from the ground through something
    /// besides it and the parts `apart` (the brace it holds, the sheets of its
    /// diaphragm), the stiffest way: a path of parts touching face to face,
    /// each pair fastened or both grounded ([`Self::tie`]), to a grounded part
    /// standing on the floor, through no load-path member and no part spanning
    /// between supports across the force, each part between the anchor and the last as deep
    /// across `member` as the member (review 2026-11-13 R1113-PG-600: a 1 mm
    /// grounded nub, or a free rod on the floor, made an anchorage ground;
    /// PG-601: so did a second free member on its posts; PG-602: so did a
    /// grounded part standing on nothing modelled; r95 P3: a strip footing
    /// shallower than the member was no ground). None where no path holds,
    /// or every path turns about a single fastener (R1113r95-PG-702). Held up
    /// by the member, by parts hanging from it or resting on it (capability
    /// reviews 751, 758), it sways with the member. Across an upright member
    /// it holds as it does the weaker way.
    pub(super) fn anchorage(
        &self,
        anchor: usize,
        member: usize,
        apart: &[usize],
    ) -> Option<Anchorage> {
        self.reached(anchor, member, apart, 0.0, None)
            .map(|(anchorage, _)| anchorage)
    }

    /// [`Self::anchorage`] of a path that has run `reach` before the anchor,
    /// over which it buckles as well (R1120ac-PG-902: a brace 1 200 long end
    /// to end with an anchor 1 200 long held the member where one rod 2 400 long
    /// buckled), with the parts the paths run through; under the force
    /// `probe` a brace puts on it, else a force across the member where the
    /// anchor meets it.
    fn reached(
        &self,
        anchor: usize,
        member: usize,
        apart: &[usize],
        reach: f64,
        probe: Option<Probe>,
    ) -> Option<(Anchorage, BTreeSet<usize>)> {
        let mut apart = apart.to_vec();
        apart.sort_unstable();
        apart.dedup();
        let key = (
            anchor,
            member,
            apart,
            reach.to_bits(),
            probe.map(Probe::bits),
        );
        if let Some(known) = self.known.borrow().get(&key) {
            return known.clone();
        }
        let probes = probe.map_or_else(
            || {
                // Where the anchor holds: where it meets the member, else the
                // parts kept apart.
                let obb = self.parts[anchor].obb();
                let held = std::iter::once(member)
                    .chain(key.2.iter().copied())
                    .find_map(|other| meeting(&obb, &self.parts[other].obb()))
                    .unwrap_or(obb.centre);
                self.sideways(member)
                    .into_iter()
                    .map(|along| Probe {
                        held,
                        enter: held,
                        along,
                    })
                    .collect()
            },
            |probe| vec![probe],
        );
        let mut found: Option<(Anchorage, BTreeSet<usize>)> = None;
        for probe in probes {
            let Some((this, route)) = self.search(anchor, member, &key.2, probe, reach) else {
                found = None;
                break;
            };
            found = Some(match found {
                Some((known, mut parts)) => {
                    parts.extend(route);
                    (weaker(known, this), parts)
                }
                None => (this, route.into_iter().collect()),
            });
        }
        self.known.borrow_mut().insert(key, found.clone());
        found
    }

    /// How `by`, a brace or a diaphragm's sheet holding `member` at `point`,
    /// holds it from the ground through `anchor`: the path starts at the
    /// member (R1119og-PG-800: three 3 × 3 mm strips screwed to a grounded
    /// wall held a 400 mm member). Its own give from `point` to where it meets
    /// the anchor, its joints with the member and the anchor, and Euler's load
    /// of its length come before [`Self::anchorage`] of the anchor, which the
    /// parts `apart` (the sheets of its diaphragm) are kept out of, buckling on
    /// from its length. A brace turning about either joint ([`hold`]) is a
    /// strut pushing only along its axis, and one lying across the force holds
    /// nothing (R1120ac-PG-900, PG-903). A grounded brace is its own anchor and
    /// its path starts at the member already.
    pub(super) fn through(
        &self,
        by: usize,
        anchor: usize,
        member: usize,
        point: [f64; 3],
        apart: &[usize],
    ) -> Option<Anchorage> {
        self.routed(by, anchor, member, point, apart, true)
            .map(|(anchorage, _)| anchorage)
    }

    /// Where `by`, or one of the parts `apart`, meets `anchor`.
    fn meets(&self, by: usize, anchor: usize, apart: &[usize]) -> [f64; 3] {
        let anchor_obb = self.parts[anchor].obb();
        std::iter::once(by)
            .chain(apart.iter().copied())
            .find_map(|part| meeting(&anchor_obb, &self.parts[part].obb()))
            .unwrap_or(anchor_obb.centre)
    }

    /// Where `by` turns about its joint with `other`, which it meets at
    /// `meets` (R1122ac-PG-1100): the middle of the fasteners joining them,
    /// else where they meet, anywhere along the contact's normal as deep as
    /// the thinner part is thick that way, within the two parts.
    fn pivot(&self, by: usize, other: usize, meets: [f64; 3]) -> Pivot {
        let at = self
            .joined
            .get(&(by.min(other), by.max(other)))
            .and_then(|fasteners| middle(fasteners))
            .unwrap_or(meets);
        let Some(normal) = self.contact_normal(by, other) else {
            return Pivot::point(at);
        };
        let boxes = [self.parts[by].obb(), self.parts[other].obb()];
        let deep = boxes
            .iter()
            .map(|obb| {
                (0..3)
                    .map(|k| 2.0 * obb.half[k] * dot(obb.axes[k], normal).abs())
                    .sum()
            })
            .fold(f64::INFINITY, f64::min);
        let (from, to) = boxes
            .iter()
            .filter_map(|obb| inside_along(obb, at, normal))
            .fold((0.0_f64, 0.0_f64), |(from, to), (lo, hi)| {
                (from.min(lo), to.max(hi))
            });
        Pivot {
            at,
            along: normal,
            from: from.max(-deep),
            to: to.min(deep),
        }
    }

    /// The level directions across `member` a force holding it acts in: the
    /// one across it, or both for an upright one.
    fn sideways(&self, member: usize) -> Vec<[f64; 3]> {
        match sideways_of(&self.frames[member], &self.parts[member]) {
            Some((sideways, _)) => vec![sideways],
            None => vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        }
    }

    /// [`Self::through`] with the parts the path runs through past `by`; a
    /// brace fixed at both ends turns its anchor about the point of the member
    /// it holds where `lever`, and passes the force on where it meets it
    /// otherwise (a diaphragm's sheet, which anchors apart from each other
    /// hold together).
    fn routed(
        &self,
        by: usize,
        anchor: usize,
        member: usize,
        point: [f64; 3],
        apart: &[usize],
        lever: bool,
    ) -> Option<(Anchorage, BTreeSet<usize>)> {
        if anchor == by {
            return self.reached(anchor, member, apart, 0.0, None);
        }
        let obb = self.parts[by].obb();
        let at = self.meets(by, anchor, apart);
        let run = length(sub(point, at));
        let mut route = BTreeSet::new();
        let mut found: Option<Anchorage> = None;
        for sideways in self.sideways(member) {
            // A part only bearing on another turns about it.
            let joint = |other: usize, at: [f64; 3]| {
                let thin = least_side(&obb).min(least_side(&self.parts[other].obb()));
                self.tie(by, other)
                    .map_or(Hold::Turning(1.0 / SLIP_PER_FASTENER), |tie| {
                        hold(tie, at, point, sideways, thin)
                    })
            };
            let this = match (joint(member, point), joint(anchor, at)) {
                (Hold::Fixed(to_member), Hold::Fixed(to_anchor)) => {
                    let probe = lever.then_some(Probe {
                        held: point,
                        enter: at,
                        along: sideways,
                    });
                    let (from_anchor, through) = self.reached(anchor, member, apart, run, probe)?;
                    route.extend(through);
                    let anchor_give = 1.0 / from_anchor.stiffness_n_mm;
                    let own = brace_give(&obb, self.moduli[by].0, point, at, sideways)
                        + to_member
                        + to_anchor;
                    let capacity = part_capacity(
                        &obb,
                        (self.moduli[by].1, self.moduli[by].2),
                        point,
                        at,
                        sideways,
                        from_anchor.over_mm,
                    );
                    Anchorage {
                        stiffness_n_mm: 1.0 / (own + anchor_give),
                        capacity_n: capacity.min(from_anchor.capacity_n),
                        weakest: if own > anchor_give {
                            by
                        } else {
                            from_anchor.weakest
                        },
                        buckling: if capacity < from_anchor.capacity_n {
                            by
                        } else {
                            from_anchor.buckling
                        },
                        over_mm: from_anchor.over_mm,
                    }
                }
                (to_member, to_anchor) => {
                    let ends = strut_line(
                        self.pivot(by, member, point),
                        self.pivot(by, anchor, at),
                        sideways,
                    );
                    let reach = length(sub(ends.0, ends.1));
                    let probe = Probe {
                        held: ends.1,
                        enter: ends.1,
                        along: Strut::of(&obb, self.moduli[by], ends, sideways, reach).axis,
                    };
                    let (from_anchor, through) =
                        self.reached(anchor, member, apart, reach, Some(probe))?;
                    route.extend(through);
                    let strut =
                        Strut::of(&obb, self.moduli[by], ends, sideways, from_anchor.over_mm);
                    let anchor_give = 1.0 / from_anchor.stiffness_n_mm;
                    let slip = to_member.give() + to_anchor.give();
                    let (stiffness_n_mm, capacity_n) =
                        strut.holds(slip, anchor_give, from_anchor.capacity_n);
                    Anchorage {
                        stiffness_n_mm,
                        capacity_n,
                        weakest: if strut.across() || strut.give + slip > anchor_give {
                            by
                        } else {
                            from_anchor.weakest
                        },
                        buckling: if strut.capacity < from_anchor.capacity_n {
                            by
                        } else {
                            from_anchor.buckling
                        },
                        over_mm: from_anchor.over_mm,
                    }
                }
            };
            found = Some(match found {
                Some(known) => weaker(known, this),
                None => this,
            });
        }
        found.map(|anchorage| (anchorage, route))
    }

    /// How a diaphragm's sheet `by` holds `member` at `point`: through each of
    /// `anchors` ([`Self::through`], the diaphragm's parts `chain` kept apart),
    /// side by side (R1119og-PG-801: sheets on four free posts held a member
    /// unchecked, though each post gives 0.8 N/mm against 58 asked). Paths
    /// sharing a part on their way to the ground are one path, the stiffest of
    /// them (R1120ac-PG-901: eight blocks on one lath on a single post held as
    /// eight posts). One path alone is turned by the sheet as a fixed brace
    /// turns its anchor (R1121ac-PG-1000: a deck on one plate over a column
    /// 20 x 400 held the member at 198.6 N/mm, the column's strong way, where
    /// its twist by the deck's 500 mm lets it give about 1.2). Paths apart from
    /// each other hold the sheet as a rigid sheet on springs ([`on_springs`]):
    /// on both sides of the point side by side, on one side by their spacing
    /// (capability review 977: two plates 220 mm apart on one side held as
    /// their strong ways added up).
    pub(super) fn diaphragm(
        &self,
        by: usize,
        anchors: impl Iterator<Item = usize>,
        member: usize,
        point: [f64; 3],
        chain: &[usize],
    ) -> Option<Anchorage> {
        let mut paths: Vec<(usize, Anchorage, BTreeSet<usize>)> = anchors
            .collect::<BTreeSet<usize>>()
            .into_iter()
            .filter_map(|anchor| {
                self.routed(by, anchor, member, point, chain, false)
                    .map(|(anchorage, route)| (anchor, anchorage, route))
            })
            .collect();
        paths.sort_by(|a, b| b.1.stiffness_n_mm.total_cmp(&a.1.stiffness_n_mm));
        let mut used = BTreeSet::new();
        let apart: Vec<(usize, Anchorage)> = paths
            .into_iter()
            .filter(|(_, _, route)| {
                let alone = route.is_disjoint(&used);
                if alone {
                    used.extend(route.iter().copied());
                }
                alone
            })
            .map(|(anchor, anchorage, _)| (anchor, anchorage))
            .collect();
        let turned = |anchor: usize| {
            self.routed(by, anchor, member, point, chain, true)
                .map(|(anchorage, _)| anchorage)
        };
        if let [(anchor, _)] = apart.as_slice() {
            return turned(*anchor);
        }
        // Each anchor turned alone, or holding no turn where it cannot be.
        let held: Vec<(Anchorage, Anchorage, [f64; 3])> = apart
            .into_iter()
            .map(|(anchor, at_contact)| {
                let turned = turned(anchor).unwrap_or(Anchorage {
                    stiffness_n_mm: 0.0,
                    ..at_contact
                });
                (
                    at_contact,
                    turned,
                    sub(self.meets(by, anchor, chain), point),
                )
            })
            .collect();
        self.sideways(member)
            .into_iter()
            .filter_map(|sideways| {
                let along = linalg::normalize(cross([0.0, 0.0, 1.0], sideways))?;
                let springs: Vec<Spring> = held
                    .iter()
                    .map(|(at_contact, turned, offset)| Spring {
                        at_contact: *at_contact,
                        turned: *turned,
                        arm_mm: dot(*offset, along),
                    })
                    .collect();
                on_springs(&springs)
            })
            .reduce(weaker)
    }

    /// Whether `item` ends a path: grounded, standing on the floor.
    fn ends(&self, item: usize) -> bool {
        self.grounded[item] && on_floor(&self.parts[item], self.floor)
    }

    /// [`Self::reached`] by the force `probe`: the path giving least, found
    /// as the shortest by its give, what each part and joint gives adding up
    /// along it, buckling as [`Self::rods`] after `start`; with the parts it
    /// runs through, the anchor first.
    fn search(
        &self,
        anchor: usize,
        member: usize,
        apart: &[usize],
        probe: Probe,
        start: f64,
    ) -> Option<(Anchorage, Vec<usize>)> {
        let Probe {
            held,
            enter,
            along: sideways,
        } = probe;
        let count = self.parts.len();
        let frame = &self.frames[member];
        let deep = depth(&self.parts[member], frame);
        let mut give = vec![f64::INFINITY; count];
        let mut entry = vec![enter; count];
        // Which part the path came from to each, and what that part gave.
        let mut came: Vec<Option<(usize, f64)>> = vec![None; count];
        give[anchor] = 0.0;
        let mut queue = BinaryHeap::from([Reverse((0_u64, anchor))]);
        let mut pivots = BTreeSet::new();
        // The path's whole give, its last part, that part's give and where it
        // stands on the floor.
        let mut best: Option<(f64, usize, f64, [f64; 3])> = None;
        while let Some(Reverse((bits, item))) = queue.pop() {
            let so_far = f64::from_bits(bits);
            if so_far > give[item] || best.is_some_and(|(total, ..)| so_far >= total) {
                continue;
            }
            let obb = self.parts[item].obb();
            let modulus = self.moduli[item].0;
            let from = entry[item];
            if self.ends(item) {
                let foot = foot(&obb, from, self.floor);
                let own = part_give(&obb, modulus, from, foot, held, sideways);
                if best.is_none_or(|(total, ..)| so_far + own < total) {
                    best = Some((so_far + own, item, own, foot));
                }
            }
            for &next in &self.touching[item] {
                // A part spanning across the force bends between its
                // supports; along it, it only pushes and pulls.
                let bends = self.spanning[next].is_some_and(|axis| {
                    dot(axis, sideways).abs() < std::f64::consts::FRAC_1_SQRT_2
                });
                let kept = next == member
                    || self.member[next]
                    || bends
                    || apart.binary_search(&next).is_ok();
                if kept || (!self.ends(next) && depth(&self.parts[next], frame) < deep) {
                    continue;
                }
                let Some(tie) = self.tie(item, next) else {
                    continue;
                };
                let other = self.parts[next].obb();
                let Some(at) = meeting(&obb, &other) else {
                    continue;
                };
                let thin = least_side(&obb).min(least_side(&other));
                let Hold::Fixed(joint) = hold(tie, at, held, sideways, thin) else {
                    pivots.insert(next);
                    continue;
                };
                let own = part_give(&obb, modulus, from, at, held, sideways) + joint;
                let further = so_far + own;
                if further < give[next] {
                    give[next] = further;
                    entry[next] = at;
                    came[next] = Some((item, own));
                    queue.push(Reverse((further.to_bits(), next)));
                }
            }
        }
        let Some((total, end, own, foot)) = best else {
            self.met_pivots
                .borrow_mut()
                .entry(anchor)
                .or_default()
                .extend(pivots);
            return None;
        };
        let (mut weakest, mut most, mut at) = (end, own, end);
        let mut route = vec![end];
        while let Some((from, given)) = came[at] {
            if given > most {
                (weakest, most) = (from, given);
            }
            at = from;
            route.push(at);
        }
        route.reverse();
        let exits = route
            .iter()
            .skip(1)
            .map(|&next| entry[next])
            .chain(std::iter::once(foot));
        let pieces: Vec<(usize, [f64; 3], [f64; 3])> = route
            .iter()
            .zip(exits)
            .map(|(&part, exit)| (part, entry[part], exit))
            .collect();
        let (capacity_n, buckling, over_mm) = self.rods(&pieces, sideways, start, anchor);
        let anchorage = Anchorage {
            stiffness_n_mm: if total > 0.0 {
                1.0 / total
            } else {
                f64::INFINITY
            },
            capacity_n,
            weakest,
            buckling,
            over_mm,
        };
        Some((anchorage, route))
    }

    /// How a path that has run `start` before its `pieces` (each part, where
    /// the path enters it and where it leaves) buckles under the force
    /// `sideways`. Pieces pushed along their axes end to end are one rod,
    /// ending where the path enters a part bearing the force across, which
    /// holds it; each piece buckles over the whole length of its rod, a stepped
    /// rod taken at its weakest piece, on the safe side (review 2026-11-21
    /// R1121ac-PG-1002: a brace 15 x 15 end to end with an anchor 30 x 30,
    /// 1 200 each, held 277.6 N over the brace's own length, where the stepped
    /// rod buckles at about 112). The force first buckling a piece, that part
    /// (else `anchor`), and the length of the rod the path starts with, mm.
    fn rods(
        &self,
        pieces: &[(usize, [f64; 3], [f64; 3])],
        sideways: [f64; 3],
        start: f64,
        anchor: usize,
    ) -> (f64, usize, f64) {
        let mut rods: Vec<(Vec<usize>, f64)> = Vec::new();
        let mut rod: (Vec<usize>, f64) = (Vec::new(), start);
        for (index, &(part, entry, exit)) in pieces.iter().enumerate() {
            let long = length(sub(entry, exit));
            if pushed_along(&self.parts[part].obb(), entry, exit, sideways).is_some() {
                rod.0.push(index);
                rod.1 += long;
            } else if long > f64::EPSILON {
                rods.push(std::mem::replace(&mut rod, (Vec::new(), 0.0)));
            }
        }
        rods.push(rod);
        let mut least = (f64::INFINITY, anchor);
        for (rod, long) in &rods {
            for &index in rod {
                let (part, entry, exit) = pieces[index];
                let capacity = part_capacity(
                    &self.parts[part].obb(),
                    (self.moduli[part].1, self.moduli[part].2),
                    entry,
                    exit,
                    sideways,
                    *long,
                );
                if capacity < least.0 {
                    least = (capacity, part);
                }
            }
        }
        (least.0, least.1, rods[0].1)
    }

    /// How touching parts `a` and `b` hold each other: rigidly when both are
    /// grounded, else on the fasteners of a joint() fastening them, where it
    /// names none on two when two fit ([`Self::fitting`]), one being a pivot.
    /// A free part only bearing on another turns about it freely, as on the
    /// floor (PG-163; capability review 771: a free post on a grounded slab
    /// was ground, on the bare floor not).
    fn tie(&self, a: usize, b: usize) -> Option<Tie> {
        if self.grounded[a] && self.grounded[b] {
            return Some(Tie::Rigid);
        }
        let listed = self.joined.get(&(a.min(b), a.max(b)))?;
        Some(match listed.len() {
            0 => self.fitting(a, b),
            1 => Tie::Pin,
            _ => Tie::fasteners(listed, self.contact_normal(a, b)),
        })
    }

    /// The way the contact between `a` and `b` faces: across the region they
    /// share, its thinnest way; None where they share none.
    fn contact_normal(&self, a: usize, b: usize) -> Option<[f64; 3]> {
        let (first, second) = (self.parts[a].obb(), self.parts[b].obb());
        thinnest(
            &shared(&first, &second),
            first.axes.into_iter().chain(second.axes),
        )
    }

    /// The fasteners a joint naming none is taken to have between `a` and
    /// `b`: two at the ends of their contact, along its longest way, where it
    /// is three times as long as the thinner part is thick, else one (as the
    /// sheets of a diaphragm, review 2026-11-13 P3).
    fn fitting(&self, a: usize, b: usize) -> Tie {
        let (first, second) = (self.parts[a].obb(), self.parts[b].obb());
        let corners = shared(&first, &second);
        let Some((p, q)) = corners
            .iter()
            .flat_map(|p| corners.iter().map(move |q| (*p, *q)))
            .max_by(|(p, q), (r, s)| length(sub(*p, *q)).total_cmp(&length(sub(*r, *s))))
        else {
            return Tie::Pin;
        };
        let long = length(sub(q, p));
        let thin = least_side(&first).min(least_side(&second));
        if long >= 3.0 * thin {
            let apart = (long - thin) / 2.0;
            let middle: [f64; 3] = std::array::from_fn(|i| (p[i] + q[i]) / 2.0);
            let way = scale(sub(q, p), apart / long);
            Tie::fasteners(
                &[sub(middle, way), linalg::add(middle, way)],
                self.contact_normal(a, b),
            )
        } else {
            Tie::Pin
        }
    }

    /// Whether the grounded `anchor` stands on nothing modelled: neither on
    /// the floor nor on any part under it (R1113-PG-602).
    pub(super) fn on_nothing(&self, anchor: usize) -> bool {
        let (low, _) = self.parts[anchor].world_bounds();
        !on_floor(&self.parts[anchor], self.floor)
            && !self.touching[anchor]
                .iter()
                .any(|next| self.parts[*next].world_bounds().1[2] <= low[2] + CONTACT_FIT_MM)
    }

    /// Why the anchors `resting` hold nothing of `target`, standing on the
    /// ground through nothing besides it and `besides`.
    pub(super) fn why_resting(&self, resting: &[usize], target: &str, besides: &str) -> String {
        let names = |floating: bool| {
            let mut list: Vec<&str> = resting
                .iter()
                .filter(|anchor| self.on_nothing(**anchor) == floating)
                .map(|anchor| self.parts[*anchor].name.as_str())
                .collect();
            list.dedup();
            list.join(", ")
        };
        let (floating, held) = (names(true), names(false));
        let mut why = Vec::new();
        if !floating.is_empty() {
            why.push(format!(
                "{floating}: grounded but stands on nothing modelled, neither on the floor nor on a part under it"
            ));
        }
        if !held.is_empty() {
            why.push(format!(
                "{held}: grounded, but reaches the floor only through {target}, another load_path() member{besides}, a part spanning between its supports or parts shallower across {target} than it is deep"
            ));
        }
        let met = self.met_pivots.borrow();
        let mut pivots: Vec<&str> = resting
            .iter()
            .filter_map(|anchor| met.get(anchor))
            .flatten()
            .map(|part| self.parts[*part].name.as_str())
            .collect();
        pivots.sort_unstable();
        pivots.dedup();
        if !pivots.is_empty() {
            why.push(format!(
                "{} held by a single fastener, a pivot the path turns about",
                pivots.join(", ")
            ));
        }
        format!("{}, so it sways with {target}", why.join("; "))
    }
}

/// Whether boxes `a` and `b` hold each other: where they meet within the fit
/// or run into each other, the region they share, seen across its thinnest
/// way, covers at least half the square of the thinner part's thinnest side,
/// however either is turned. Parts meeting at an edge, a corner, a speck or a
/// corner sunk a little into the other hold each other nowhere (R1113-PG-600;
/// capability review 768: a grounded column turned 1°, meeting a block along
/// its edge, was ground for it; review 2026-11-13 r95 R1113r95-PG-701: so was
/// one meeting it on a patch 2 x 2 or 0.05 x 0.05 mm, or sunk 0.05 mm into
/// it at a corner).
fn face_to_face(a: &Obb, b: &Obb) -> bool {
    if a.separation(b) > CONTACT_FIT_MM {
        return false;
    }
    let grown = Obb {
        half: a.half.map(|half| half + CONTACT_FIT_MM),
        ..*a
    };
    let shared = intersection_vertices(&[grown, *b], ROUNDING);
    let thinnest = a
        .half
        .into_iter()
        .chain(b.half)
        .fold(f64::INFINITY, f64::min)
        * 2.0;
    shared_area(&shared, a.axes.into_iter().chain(b.axes)) >= thinnest * thinnest / 2.0
}

/// Of `directions`, the one the region with corners `points` is thinnest
/// along; None for no region.
fn thinnest(points: &[[f64; 3]], directions: impl Iterator<Item = [f64; 3]>) -> Option<[f64; 3]> {
    if points.is_empty() {
        return None;
    }
    let extent = |direction: [f64; 3]| {
        let (low, high) =
            points
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                    let along = dot(*point, direction);
                    (low.min(along), high.max(along))
                });
        high - low
    };
    directions.min_by(|a, b| extent(*a).total_cmp(&extent(*b)))
}

/// The area of the convex region with corners `points`, seen along whichever
/// of `directions` it is thinnest along.
fn shared_area(points: &[[f64; 3]], directions: impl Iterator<Item = [f64; 3]>) -> f64 {
    let Some(across) = thinnest(points, directions) else {
        return 0.0;
    };
    let other = if across[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let Some(u) = linalg::normalize(cross(across, other)) else {
        return 0.0;
    };
    let v = cross(across, u);
    let hull = convex_hull(
        points
            .iter()
            .map(|point| [dot(*point, u), dot(*point, v)])
            .collect(),
    );
    let twice: f64 = hull
        .iter()
        .zip(hull.iter().cycle().skip(1))
        .map(|(p, q)| p[0] * q[1] - p[1] * q[0])
        .sum();
    twice.abs() / 2.0
}

/// Whether an anchorage at `anchor` holds `point` of `member`, of `frame`,
/// sideways: across the member and beyond its side (capability review 466 of
/// PG-141), as [`braced_stretches`] takes a brace the geometry finds.
pub(super) fn across_beyond(
    frame: &Frame,
    member: &Part,
    point: [f64; 3],
    anchor: [f64; 3],
) -> bool {
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
pub(super) fn beyond_side(frame: &Frame, member: &Part, spot: [f64; 3]) -> bool {
    sideways_of(frame, member).is_some_and(|(sideways, side)| {
        dot(sub(spot, member.obb().centre), sideways).abs() > side + CONTACT_FIT_MM
    })
}

/// Whether the grounded `by` itself reaches across `member` beyond its side,
/// level with `point`, and lies mostly beside the member's width (review
/// 2026-11-10 R1110-PG-400: grounded blocks or laths on the member's top,
/// within its width or a millimetre wider, rest on it and sway with it).
pub(super) fn reaches_beyond(frame: &Frame, member: &Part, by: &Part, point: [f64; 3]) -> bool {
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
