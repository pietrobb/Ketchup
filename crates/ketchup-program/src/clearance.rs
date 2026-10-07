//! Free space a program keeps clear (`keep_clear()`): a tool body that parts
//! must stay out of, such as the landing in front of a stair, room to open
//! and use a door, or a zone where one kind of part never goes. A part that
//! reaches into it is an error, like a collision; editing is never blocked.

use crate::contact::polygon::{area, boolean};
use crate::eval::TOLERANCE_MM;
use crate::model::{
    Part, ProgramBooleanKind, ProgramModel, ProgramPartBody, ProgramProfileSegment,
};
use crate::validate::{
    Issue, Severity, booleans_leave_overlap, is_box, needs_exact_shapes, overlap,
};
use ketchup_tolerance::APPROXIMATION;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FreeSpace {
    pub name: String,
    /// The tool body that is the space.
    pub zone: String,
    /// Only parts carrying one of these tags count; empty: every part.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub only_tags: BTreeSet<String>,
    /// Parts that belong in the space (the thing it serves).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub ignore: BTreeSet<String>,
    pub hint: String,
}

/// Whether the solid of `part` reaches into the box `zone` by more than the
/// tolerance; `None` when boxes and profiles cannot tell.
fn reaches_in(zone: &Part, part: &Part) -> Option<bool> {
    if !needs_exact_shapes(zone, part) {
        return Some(true);
    }
    let removes_only = part
        .booleans()
        .all(|b| b.kind == ProgramBooleanKind::Subtract);
    if !is_box(zone) || zone.booleans().next().is_some() || part.has_shaping() || !removes_only {
        return None;
    }
    let ProgramPartBody::Extrusion {
        segments,
        distance_mm,
        ..
    } = &part.body
    else {
        return None;
    };
    if !segments.iter().all(ProgramProfileSegment::is_line) {
        return None;
    }
    // The zone in the part's frame; only an axis-aligned result is exact.
    let (min, max) = zone.local_bounds();
    let corners = (0..8).map(|corner: usize| {
        let local = std::array::from_fn(|axis| {
            if corner >> axis & 1 == 0 {
                min[axis]
            } else {
                max[axis]
            }
        });
        part.to_local(zone.to_world(local))
    });
    let (lo, hi) = corners.fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(lo, hi), p| {
            (
                std::array::from_fn(|i| lo[i].min(p[i])),
                std::array::from_fn(|i| hi[i].max(p[i])),
            )
        },
    );
    let volume = |lo: [f64; 3], hi: [f64; 3]| (0..3).map(|i| hi[i] - lo[i]).product::<f64>();
    if (volume(lo, hi) - volume(min, max)).abs() > APPROXIMATION * volume(min, max) {
        return None;
    }
    if hi[2].min(*distance_mm) - lo[2].max(0.0) <= TOLERANCE_MM {
        return Some(false);
    }
    let section = vec![segments.iter().map(|segment| segment.start_mm).collect()];
    let window = vec![vec![
        [lo[0], lo[1]],
        [hi[0], lo[1]],
        [hi[0], hi[1]],
        [lo[0], hi[1]],
    ]];
    let shared = area(&boolean(&section, &window, false));
    if shared <= TOLERANCE_MM * ((hi[0] - lo[0]) + (hi[1] - lo[1])) {
        return Some(false);
    }
    // Holes, pockets and subtracted tools might still empty the shared volume.
    (part.booleans().next().is_none()
        && part.holes().next().is_none()
        && part.pockets().next().is_none())
    .then_some(true)
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub(crate) fn issues(model: &ProgramModel, issues: &mut Vec<Issue>) {
    for space in &model.free_spaces {
        let Some(zone) = model.tool(&space.zone) else {
            continue;
        };
        for part in &model.parts {
            if space.ignore.contains(&part.name)
                || (!space.only_tags.is_empty() && part.tags.is_disjoint(&space.only_tags))
            {
                continue;
            }
            let Some((min, max)) = overlap(zone, part) else {
                continue;
            };
            if !booleans_leave_overlap(part, zone) {
                continue;
            }
            let size = format!(
                "{} x {} x {} mm",
                round(max[0] - min[0]),
                round(max[1] - min[1]),
                round(max[2] - min[2])
            );
            let (severity, kind, message) = match reaches_in(zone, part) {
                Some(false) => continue,
                Some(true) => (
                    Severity::Error,
                    "free_space_occupied",
                    format!(
                        "{} reaches into the free space {} ({size})",
                        part.name, space.name
                    ),
                ),
                None => (
                    Severity::Warning,
                    "free_space_unverified",
                    format!(
                        "the bounding box of {} reaches into the free space {} ({size}); \
                         whether its solid does was not decided",
                        part.name, space.name
                    ),
                ),
            };
            issues.push(Issue {
                severity,
                kind,
                parts: vec![part.name.clone()],
                message,
                where_mm: Some((min.map(round), max.map(round))),
                hint: space.hint.clone(),
            });
        }
    }
}
