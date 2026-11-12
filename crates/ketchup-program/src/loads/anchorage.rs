//! Where an anchorage holds a member sideways from: across the member and
//! beyond its side, standing on something besides the member (review
//! 2026-11-12 R1112-PG-500, PG-501: grounded parts wider than the member, set
//! aside on it, or sheets on its top anchoring a diaphragm, held up by the
//! member alone, held it).

use super::Frame;
use crate::load_path::on_floor;
use crate::model::{Part, ProgramModel};
use ketchup_geometry::linalg::{self, cross, dot, sub};
use ketchup_model::tolerance::CONTACT_FIT_MM;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// What each part touches and where the floor lies: whether a grounded part
/// stands on something besides the member it would anchor.
pub(super) struct Footing<'a> {
    pub(super) parts: &'a [Part],
    floor: f64,
    touching: Vec<Vec<usize>>,
    /// Answers by anchor, member and the parts kept apart.
    known: RefCell<BTreeMap<Asked, bool>>,
}

type Asked = (usize, usize, Vec<usize>);

impl<'a> Footing<'a> {
    pub(super) fn of(model: &'a ProgramModel) -> Self {
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
                    || boxes[left].separation(&boxes[right]) > CONTACT_FIT_MM
                {
                    continue;
                }
                touching[left].push(right);
                touching[right].push(left);
            }
        }
        Self {
            parts,
            floor: model.floor_z_mm.unwrap_or(0.0),
            touching,
            known: RefCell::default(),
        }
    }

    /// Whether `anchor` stands on something besides `member` and the parts
    /// `apart` (the brace it holds, the sheets of its diaphragm): on the
    /// floor, or touching, through other parts than those, one that reaches
    /// lower than it does. Held up by the member (on it, or on one of `apart`
    /// reaching lower) and nothing else it sways with the member, however far
    /// it reaches beyond its side; one resting on neither is taken as declared
    /// (review 2026-11-06 P3: a grounded part under a brace beside the member).
    pub(super) fn stands_clear(&self, anchor: usize, member: usize, apart: &[usize]) -> bool {
        let mut apart = apart.to_vec();
        apart.sort_unstable();
        apart.dedup();
        let key = (anchor, member, apart);
        if let Some(known) = self.known.borrow().get(&key) {
            return *known;
        }
        let bottom = |item: usize| self.parts[item].world_bounds().0[2];
        let lower = bottom(anchor) - CONTACT_FIT_MM;
        let mut seen = BTreeSet::from([anchor]);
        let mut queue = vec![anchor];
        let mut clear = false;
        let mut rests = false;
        while let Some(item) = queue.pop() {
            if on_floor(&self.parts[item], self.floor) || bottom(item) < lower {
                clear = true;
                break;
            }
            for &next in &self.touching[item] {
                let kept = next == member || key.2.binary_search(&next).is_ok();
                if kept {
                    rests |= next == member || bottom(next) < bottom(item) - CONTACT_FIT_MM;
                } else if seen.insert(next) {
                    queue.push(next);
                }
            }
        }
        let clear = clear || !rests;
        self.known.borrow_mut().insert(key, clear);
        clear
    }
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
