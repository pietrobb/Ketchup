//! Where an anchorage holds a member sideways from: across the member and
//! beyond its side, standing on the ground through something besides the
//! member (review 2026-11-12 R1112-PG-500, PG-501: grounded parts wider than
//! the member, set aside on it, or sheets on its top anchoring a diaphragm,
//! held up by the member alone, held it); and how stiffly, and up to what
//! force, it holds there ([`stiffness`]; review 2026-11-13 r95 structural
//! point 1: every threshold of slenderness was passed by the next shape past
//! it).

mod stiffness;

use super::{BraceAnchorage, Frame, Support, depth, middle};
use crate::frame::{Obb, intersection_vertices};
use crate::load_path::on_floor;
use crate::model::{Part, ProgramModel, convex_hull};
use ketchup_geometry::linalg::{self, cross, dot, length, sub};
use ketchup_model::tolerance::{CONTACT_FIT_MM, ROUNDING};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use stiffness::{Tie, joint_give, part_capacity, part_give};

/// The modulus of elasticity, mean and fifth percentile, N/mm², of a part of no
/// timber strength class: softwood C24 (EN 338).
const SOFTWOOD_MODULI: (f64, f64) = (11_000.0, 7_400.0);

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
}

/// What each part touches face to face and where the floor lies: whether an
/// anchorage reaches the ground by a path of its own, and how stiffly.
pub(super) struct Footing<'a> {
    pub(super) parts: &'a [Part],
    frames: &'a [Frame],
    /// The load-path members, none of which is ground for another.
    member: &'a [bool],
    /// The long axis of each part spanning between its supports, which bends
    /// across it like a member (R1113r95-PG-705: a second beam outside the
    /// load path was ground).
    spanning: Vec<Option<[f64; 3]>>,
    grounded: Vec<bool>,
    /// Each part's modulus of elasticity, mean and fifth percentile, N/mm².
    moduli: Vec<(f64, f64)>,
    floor: f64,
    touching: Vec<Vec<usize>>,
    /// The fasteners the fastening joint()s between each pair of parts, lesser
    /// index first, name.
    joined: BTreeMap<(usize, usize), Vec<[f64; 3]>>,
    /// Answers by anchor, member and the parts kept apart.
    known: RefCell<BTreeMap<Asked, Option<Anchorage>>>,
    /// The parts each anchor's search met turning about one fastener.
    met_hinges: RefCell<BTreeMap<usize, BTreeSet<usize>>>,
}

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

type Asked = (usize, usize, Vec<usize>);

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
                        .map_or(SOFTWOOD_MODULI, |class| (class.e0_mean, class.e0_05))
                })
                .collect(),
            floor: model.floor_z_mm.unwrap_or(0.0),
            touching,
            joined,
            known: RefCell::default(),
            met_hinges: RefCell::default(),
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
        let mut apart = apart.to_vec();
        apart.sort_unstable();
        apart.dedup();
        let key = (anchor, member, apart);
        if let Some(known) = self.known.borrow().get(&key) {
            return *known;
        }
        // Where the anchor holds: where it meets the member, else the brace
        // it holds.
        let obb = self.parts[anchor].obb();
        let held = std::iter::once(member)
            .chain(key.2.iter().copied())
            .find_map(|other| meeting(&obb, &self.parts[other].obb()))
            .unwrap_or(obb.centre);
        let directions = match sideways_of(&self.frames[member], &self.parts[member]) {
            Some((sideways, _)) => vec![sideways],
            None => vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        };
        let mut found: Option<Anchorage> = None;
        for sideways in directions {
            let Some(this) = self.search(anchor, member, &key.2, held, sideways) else {
                found = None;
                break;
            };
            found = Some(match found {
                Some(known) => weaker(known, this),
                None => this,
            });
        }
        self.known.borrow_mut().insert(key, found);
        found
    }

    /// Whether `item` ends a path: grounded, standing on the floor.
    fn ends(&self, item: usize) -> bool {
        self.grounded[item] && on_floor(&self.parts[item], self.floor)
    }

    /// [`Self::anchorage`] by a force `sideways` at `held`: the path giving
    /// least, found as the shortest by its give, what each part and joint
    /// gives adding up along it.
    fn search(
        &self,
        anchor: usize,
        member: usize,
        apart: &[usize],
        held: [f64; 3],
        sideways: [f64; 3],
    ) -> Option<Anchorage> {
        let count = self.parts.len();
        let frame = &self.frames[member];
        let deep = depth(&self.parts[member], frame);
        let mut give = vec![f64::INFINITY; count];
        let mut entry = vec![held; count];
        // How far the path has run to each part, and the force it takes
        // before buckling up to there.
        let mut reach = vec![0.0; count];
        let mut capacity = vec![f64::INFINITY; count];
        let mut buckling = vec![anchor; count];
        // Which part the path came from to each, and what that part gave.
        let mut came: Vec<Option<(usize, f64)>> = vec![None; count];
        give[anchor] = 0.0;
        let mut queue = BinaryHeap::from([Reverse((0_u64, anchor))]);
        let mut hinges = BTreeSet::new();
        // The path's whole give, its capacity and the part buckling first,
        // its last part and that part's give.
        let mut best: Option<(f64, (f64, usize), usize, f64)> = None;
        while let Some(Reverse((bits, item))) = queue.pop() {
            let so_far = f64::from_bits(bits);
            if so_far > give[item] || best.is_some_and(|(total, ..)| so_far >= total) {
                continue;
            }
            let obb = self.parts[item].obb();
            let (modulus, modulus_05) = self.moduli[item];
            let from = entry[item];
            if self.ends(item) {
                let foot = foot(&obb, from, self.floor);
                let own = part_give(&obb, modulus, from, foot, held, sideways);
                let run = reach[item] + length(sub(from, foot));
                let own_capacity = part_capacity(&obb, modulus_05, from, foot, sideways, run);
                let holds = if own_capacity < capacity[item] {
                    (own_capacity, item)
                } else {
                    (capacity[item], buckling[item])
                };
                if best.is_none_or(|(total, ..)| so_far + own < total) {
                    best = Some((so_far + own, holds, item, own));
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
                let Some(joint) = joint_give(tie, at, held, sideways, thin) else {
                    hinges.insert(next);
                    continue;
                };
                let own = part_give(&obb, modulus, from, at, held, sideways) + joint;
                let further = so_far + own;
                if further < give[next] {
                    let run = reach[item] + length(sub(from, at));
                    give[next] = further;
                    entry[next] = at;
                    reach[next] = run;
                    let own_capacity = part_capacity(&obb, modulus_05, from, at, sideways, run);
                    (capacity[next], buckling[next]) = if own_capacity < capacity[item] {
                        (own_capacity, item)
                    } else {
                        (capacity[item], buckling[item])
                    };
                    came[next] = Some((item, own));
                    queue.push(Reverse((further.to_bits(), next)));
                }
            }
        }
        let Some((total, (holds, buckles), end, own)) = best else {
            self.met_hinges
                .borrow_mut()
                .entry(anchor)
                .or_default()
                .extend(hinges);
            return None;
        };
        let (mut weakest, mut most, mut at) = (end, own, end);
        while let Some((from, given)) = came[at] {
            if given > most {
                (weakest, most) = (from, given);
            }
            at = from;
        }
        Some(Anchorage {
            stiffness_n_mm: if total > 0.0 {
                1.0 / total
            } else {
                f64::INFINITY
            },
            capacity_n: holds,
            weakest,
            buckling: buckles,
        })
    }

    /// How touching parts `a` and `b` hold each other: rigidly when both are
    /// grounded, else on the fasteners of a joint() fastening them, where it
    /// names none on two when two fit ([`Self::fitting`]), one being a hinge.
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
            1 => Tie::Hinge,
            count => {
                let count = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
                let middle: [f64; 3] = std::array::from_fn(|i| {
                    listed.iter().map(|point| point[i]).sum::<f64>() / count
                });
                Tie::Fasteners {
                    count,
                    turning: listed
                        .iter()
                        .map(|point| dot(sub(*point, middle), sub(*point, middle)))
                        .sum(),
                }
            }
        })
    }

    /// The fasteners a joint naming none is taken to have between `a` and
    /// `b`: two at the ends of their contact where it is three times as long
    /// as the thinner part is thick, else one (as the sheets of a diaphragm,
    /// review 2026-11-13 P3).
    fn fitting(&self, a: usize, b: usize) -> Tie {
        let (first, second) = (self.parts[a].obb(), self.parts[b].obb());
        let corners = shared(&first, &second);
        let long = corners
            .iter()
            .flat_map(|p| corners.iter().map(move |q| length(sub(*p, *q))))
            .fold(0.0, f64::max);
        let thin = least_side(&first).min(least_side(&second));
        if long >= 3.0 * thin {
            let apart = (long - thin) / 2.0;
            Tie::Fasteners {
                count: 2.0,
                turning: 2.0 * apart * apart,
            }
        } else {
            Tie::Hinge
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
        let met = self.met_hinges.borrow();
        let mut hinges: Vec<&str> = resting
            .iter()
            .filter_map(|anchor| met.get(anchor))
            .flatten()
            .map(|part| self.parts[*part].name.as_str())
            .collect();
        hinges.sort_unstable();
        hinges.dedup();
        if !hinges.is_empty() {
            why.push(format!(
                "{} held by a single fastener, a hinge the path turns about",
                hinges.join(", ")
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

/// The area of the convex region with corners `points`, seen along whichever
/// of `directions` it is thinnest along.
fn shared_area(points: &[[f64; 3]], directions: impl Iterator<Item = [f64; 3]>) -> f64 {
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
    let Some(across) = directions.min_by(|a, b| extent(*a).total_cmp(&extent(*b))) else {
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
