//! What the program can tell of an edge finish before OCCT builds it: `check`
//! needs no OCCT, so a finish OCCT refuses would otherwise pass it (review
//! 2026-11-20 ac P3).

use std::collections::BTreeSet;

use ketchup_model::tolerance::CONTACT_FIT_MM;

use crate::model::{Part, ProgramOperation, ProgramPartBody, ProgramProfileSegment};

/// Refuses an edge finish of `amount_mm` on `edges` of `part` that cannot be
/// built: an edge named by one face twice or listed twice, an edge an earlier
/// finish already rounded or cut away, and, on a plain cuboid, two opposite
/// faces, which share no edge, or an amount the part is not wider than across
/// the edge.
pub(crate) fn check_edge_finish(
    part: &Part,
    operation: &str,
    edges: &[[String; 2]],
    amount_mm: f64,
) -> anyhow::Result<()> {
    let part_name = &part.name;
    let pair = |[first, second]: &[String; 2]| -> anyhow::Result<[String; 2]> {
        let label = |face: &str| {
            part.exact_face_label(face)
                .map_err(|error| anyhow::anyhow!("{operation} on {part_name:?}: {error}"))
        };
        let mut pair = [label(first)?, label(second)?];
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
        let across = cuboid.size[first_axis].min(cuboid.size[second_axis]);
        if amount_mm >= across - CONTACT_FIT_MM {
            anyhow::bail!(
                "{operation} on {part_name:?}: {amount_mm} mm does not fit the edge ({first:?}, \
                 {second:?}), the part is {across} mm across it; use less than {across} mm"
            );
        }
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
