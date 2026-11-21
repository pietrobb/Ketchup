//! What the program can tell of an edge finish before OCCT builds it: `check`
//! needs no OCCT, so a finish OCCT refuses would otherwise pass it (review
//! 2026-11-20 ac P3).

use std::collections::BTreeSet;

use ketchup_model::tolerance::CONTACT_FIT_MM;

use crate::model::{Part, ProgramOperation, ProgramPartBody, ProgramProfileSegment};

/// Refuses an edge finish of `amount_mm` on `edges` of `part` that cannot be
/// built: an amount too small to build, named by its parameter (R1121ac P3),
/// an edge named by one face twice or listed twice, an edge an earlier
/// finish already rounded or cut away, and, on a plain cuboid, two opposite
/// faces, which share no edge, or an amount that does not fit a face beside
/// the finishes on the face's other edge across the same width: a finish takes
/// its amount off each of its two faces, so finishes on both edges of a 10 mm
/// face must add up to less than 10 mm (R1121ac P3).
pub(crate) fn check_edge_finish(
    part: &Part,
    operation: &str,
    edges: &[[String; 2]],
    amount_mm: f64,
) -> anyhow::Result<()> {
    let part_name = &part.name;
    if amount_mm <= CONTACT_FIT_MM {
        let parameter = if operation == "fillet" {
            "radius"
        } else {
            "distance"
        };
        anyhow::bail!(
            "{operation} on {part_name:?}: {parameter} must be more than {CONTACT_FIT_MM} mm, \
             got {amount_mm}"
        );
    }
    let faces = |[first, second]: &[String; 2]| -> anyhow::Result<[String; 2]> {
        let label = |face: &str| {
            part.exact_face_label(face)
                .map_err(|error| anyhow::anyhow!("{operation} on {part_name:?}: {error}"))
        };
        Ok([label(first)?, label(second)?])
    };
    let pair = |edge: &[String; 2]| -> anyhow::Result<[String; 2]> {
        let mut pair = faces(edge)?;
        pair.sort_unstable();
        Ok(pair)
    };
    let mut finished = Vec::new();
    for finish in part.finishes() {
        for edge in &finish.edges {
            finished.push((pair(edge)?, &finish.name));
        }
    }
    let cuboid = PlainCuboid::of(part);
    // Each finished edge of the cuboid as (face label, axis of the other face,
    // amount), once per face: what the finish takes off that face.
    let mut taken: Vec<(String, usize, f64)> = Vec::new();
    if let Some(cuboid) = &cuboid {
        for finish in part.finishes() {
            for edge in &finish.edges {
                taken.extend(cuboid.takes(edge, &faces(edge)?, finish.radius_mm));
            }
        }
    }
    let mut unique = BTreeSet::new();
    for edge @ [first, second] in edges {
        if first == second {
            anyhow::bail!("{operation} edge faces must be different, got {first:?} twice");
        }
        let labels = pair(edge)?;
        if let Some((_, earlier)) = finished.iter().find(|(done, _)| *done == labels) {
            anyhow::bail!(
                "{operation} edge ({first:?}, {second:?}) of {part_name:?} is already finished \
                 by {earlier:?}, which took the edge away; an edge takes one finish, so change \
                 {earlier:?} instead"
            );
        }
        if !unique.insert(labels) {
            anyhow::bail!("{operation} edge ({first:?}, {second:?}) is listed more than once");
        }
        let Some(cuboid) = &cuboid else {
            continue;
        };
        let (Some(first_axis), Some(second_axis)) = (cuboid.axis(first), cuboid.axis(second))
        else {
            continue;
        };
        if first_axis == second_axis {
            anyhow::bail!(
                "{operation} on {part_name:?}: faces {first:?} and {second:?} are opposite sides \
                 of the part and share no edge; name two faces that meet"
            );
        }
        let labels = faces(edge)?;
        let room_on = |label: &String, across_axis: usize| {
            let across = cuboid.size[across_axis];
            let beside: f64 = taken
                .iter()
                .filter(|(other, axis, _)| other == label && *axis == across_axis)
                .map(|(_, _, amount)| amount)
                .sum();
            (across, beside, across - CONTACT_FIT_MM - beside)
        };
        // The face with less room left decides.
        let on_first = room_on(&labels[0], second_axis);
        let on_second = room_on(&labels[1], first_axis);
        let (face, (across, beside, room)) = if on_first.2 <= on_second.2 {
            (first, on_first)
        } else {
            (second, on_second)
        };
        if amount_mm >= room {
            // Rounded down to a micrometre, so the advice itself fits.
            let fits = (room * 1000.0).floor() / 1000.0;
            if beside == 0.0 {
                anyhow::bail!(
                    "{operation} on {part_name:?}: {amount_mm} mm does not fit the edge \
                     ({first:?}, {second:?}), the part is {across} mm across it; use less than \
                     {fits} mm"
                );
            }
            if fits <= 0.0 {
                anyhow::bail!(
                    "{operation} on {part_name:?}: the edge ({first:?}, {second:?}) has no room \
                     on face {face:?}: finishes on its other edge already take {beside} mm of \
                     the {across} mm across it; make them smaller first"
                );
            }
            anyhow::bail!(
                "{operation} on {part_name:?}: {amount_mm} mm does not fit the edge ({first:?}, \
                 {second:?}): face {face:?} is {across} mm across and finishes on its other edge \
                 take {beside} mm of it; use less than {fits} mm"
            );
        }
        taken.extend(cuboid.takes(edge, &labels, amount_mm));
    }
    Ok(())
}

/// A part that is a cuboid nothing but holes, pockets and finishes changed:
/// its size, and its faces by name.
struct PlainCuboid<'a> {
    size: [f64; 3],
    segments: &'a [ProgramProfileSegment],
    caps: &'a [String; 2],
}

impl<'a> PlainCuboid<'a> {
    fn of(part: &'a Part) -> Option<Self> {
        let size = part.body.cuboid_size()?;
        let plain = part.operations.iter().all(|operation| {
            matches!(
                operation,
                ProgramOperation::Finish(_)
                    | ProgramOperation::Hole(_)
                    | ProgramOperation::Pocket(_)
            )
        });
        match &part.body {
            ProgramPartBody::Extrusion { segments, caps, .. } if plain => Some(Self {
                size,
                segments,
                caps,
            }),
            _ => None,
        }
    }

    /// What a finish of `amount_mm` on `edge` (face labels `labels`, in the
    /// edge's order) takes off each of its faces: (face label, axis it takes along,
    /// amount). Nothing for an edge whose faces have no axis or share one.
    fn takes(
        &self,
        [first, second]: &[String; 2],
        labels: &[String; 2],
        amount_mm: f64,
    ) -> Vec<(String, usize, f64)> {
        let (Some(first_axis), Some(second_axis)) = (self.axis(first), self.axis(second)) else {
            return Vec::new();
        };
        if first_axis == second_axis {
            return Vec::new();
        }
        let [first_label, second_label] = labels;
        vec![
            (first_label.clone(), second_axis, amount_mm),
            (second_label.clone(), first_axis, amount_mm),
        ]
    }

    /// The axis `face` faces along: the rectangle runs y-, x+, y+, x-, the caps
    /// are z- and z+.
    fn axis(&self, face: &str) -> Option<usize> {
        if self.caps.iter().any(|cap| cap == face) {
            return Some(2);
        }
        self.segments
            .iter()
            .position(|segment| segment.name == face)
            .map(|index| if index % 2 == 0 { 1 } else { 0 })
    }
}
