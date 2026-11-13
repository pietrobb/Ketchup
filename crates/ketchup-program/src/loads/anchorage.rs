//! Where an anchorage holds a member sideways from: across the member and
//! beyond its side, standing on the ground through something besides the
//! member (review 2026-11-12 R1112-PG-500, PG-501: grounded parts wider than
//! the member, set aside on it, or sheets on its top anchoring a diaphragm,
//! held up by the member alone, held it).

use super::{Frame, depth};
use crate::load_path::on_floor;
use crate::model::{Part, ProgramModel};
use ketchup_geometry::linalg::{self, cross, dot, sub};
use ketchup_model::tolerance::CONTACT_FIT_MM;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// What each part touches face to face and where the floor lies: whether an
/// anchorage reaches the ground by a path of its own.
pub(super) struct Footing<'a> {
    pub(super) parts: &'a [Part],
    frames: &'a [Frame],
    /// The load-path members, none of which is ground for another.
    member: &'a [bool],
    grounded: Vec<bool>,
    floor: f64,
    touching: Vec<Vec<usize>>,
    /// Answers by anchor, member and the parts kept apart.
    known: RefCell<BTreeMap<Asked, bool>>,
}

type Asked = (usize, usize, Vec<usize>);

impl<'a> Footing<'a> {
    pub(super) fn of(model: &'a ProgramModel, member: &'a [bool], frames: &'a [Frame]) -> Self {
        let parts = &model.parts;
        let boxes: Vec<_> = parts.iter().map(Part::obb).collect();
        let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
        let square: Vec<bool> = boxes
            .iter()
            .map(|obb| obb.axes.iter().all(on_axis))
            .collect();
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|a, b| bounds[*a].0[0].total_cmp(&bounds[*b].0[0]));
        let mut touching = vec![Vec::new(); parts.len()];
        for (position, &left) in order.iter().enumerate() {
            for &right in &order[position + 1..] {
                if bounds[right].0[0] > bounds[left].1[0] + CONTACT_FIT_MM {
                    break;
                }
                // Parts meeting at an edge or a corner only hold each other
                // nowhere (R1113-PG-600); boxes turned off the axes are taken
                // as touching where their boxes meet.
                let (a, b) = (bounds[left], bounds[right]);
                let edge_only = square[left]
                    && square[right]
                    && (0..3)
                        .filter(|axis| {
                            a.1[*axis].min(b.1[*axis]) - a.0[*axis].max(b.0[*axis]) > CONTACT_FIT_MM
                        })
                        .count()
                        < 2;
                if edge_only
                    || model.are_alternatives(&parts[left], &parts[right])
                    || boxes[left].separation(&boxes[right]) > CONTACT_FIT_MM
                {
                    continue;
                }
                touching[left].push(right);
                touching[right].push(left);
            }
        }
        let names = model.grounded_parts();
        Self {
            parts,
            frames,
            member,
            grounded: parts
                .iter()
                .map(|part| names.contains(&part.name))
                .collect(),
            floor: model.floor_z_mm.unwrap_or(0.0),
            touching,
            known: RefCell::default(),
        }
    }

    /// Whether `anchor` stands on the ground through something besides
    /// `member` and the parts `apart` (the brace it holds, the sheets of its
    /// diaphragm): a path of parts touching face to face to a grounded part
    /// standing on the floor, through no load-path member, each part on it but
    /// the anchor as deep across `member` as the member (review 2026-11-13
    /// R1113-PG-600: a 1 mm grounded nub, or a free rod on the floor, made an
    /// anchorage ground; PG-601: so did a second free beam on its posts;
    /// PG-602: so did a grounded part standing on nothing modelled). Held up
    /// by the member, by parts hanging from it or resting on it (capability
    /// reviews 751, 758), it sways with the member.
    pub(super) fn stands_clear(&self, anchor: usize, member: usize, apart: &[usize]) -> bool {
        let mut apart = apart.to_vec();
        apart.sort_unstable();
        apart.dedup();
        let key = (anchor, member, apart);
        if let Some(known) = self.known.borrow().get(&key) {
            return *known;
        }
        let frame = &self.frames[member];
        let deep = depth(&self.parts[member], frame);
        let mut seen = BTreeSet::from([anchor]);
        let mut queue = vec![anchor];
        let mut clear = false;
        while let Some(item) = queue.pop() {
            if self.grounded[item] && on_floor(&self.parts[item], self.floor) {
                clear = true;
                break;
            }
            for &next in &self.touching[item] {
                let kept =
                    next == member || self.member[next] || key.2.binary_search(&next).is_ok();
                if !kept && depth(&self.parts[next], frame) >= deep && seen.insert(next) {
                    queue.push(next);
                }
            }
        }
        self.known.borrow_mut().insert(key, clear);
        clear
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
                "{held}: grounded, but reaches the floor only through {target}, another load_path() member{besides} or parts shallower across {target} than it is deep"
            ));
        }
        format!("{}, so it sways with {target}", why.join("; "))
    }
}

/// Whether `axis` runs along a world axis.
fn on_axis(axis: &[f64; 3]) -> bool {
    axis.iter().filter(|value| value.abs() > 1e-9).count() == 1
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
