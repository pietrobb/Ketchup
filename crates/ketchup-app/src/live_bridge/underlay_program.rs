//! `program action=from_underlay`: what an underlay draws, written into the window's
//! program as named data in document millimetres, so the program builds on the drawing
//! without anyone copying coordinates: the outline and holes of the closed region around a
//! point or of one closed object (`extrude("floor", profile = NAME, …)`), or every
//! affine instance of a shared block's sampled linework. Declarations sit between two comment lines
//! with a persistent selector in drawing coordinates. Underlay edits regenerate these sections
//! through the same planner as `program apply`, publishing input and derived geometry in one Undo
//! step. A missing selection or a program that no longer evaluates changes nothing.

use super::program_access::invalid;
use super::*;
use ketchup_geometry::linalg::cross2;
use ketchup_model::underlay::Underlay;
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};
use ketchup_model::underlay::objects::{Affine, ObjectDetail};

/// The most points, or placements, one piece of data holds.
const MAX_ITEMS: usize = 4_000;
const POINTS_PER_LINE: usize = 4;
const FILE_NAME: &str = "drawing.star";

impl LiveBridge {
    /// The program apply that writes the data `request` asks for, or `request` itself.
    pub(super) fn expand_program_from_underlay(
        app: &KetchupApp,
        request: Request,
    ) -> Result<Request, &'static str> {
        let Request::ProgramFromUnderlay {
            expected,
            name,
            underlay_id,
            point_mm,
            closing_segments_mm,
            ..
        } = &request
        else {
            return Ok(request);
        };
        Self::guard(app, &Some(expected.clone()))?;
        if !closing_segments_mm.is_empty() && point_mm.is_none() {
            return Err(invalid(
                "closing_segments_mm",
                "Closing segments apply only to a point-selected region.",
                "Use point_mm with closing_segments_mm, not block or object_id.",
            ));
        }
        if !is_identifier(name) {
            return Err(invalid(
                "name",
                format!("{name:?} is not a program name."),
                "Use letters, digits and _, starting with a letter, e.g. ROOM_1_01.",
            ));
        }
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let expected = expected.clone();
        let name = name.clone();
        let binding = super::underlay_binding::Binding::new(app, &snapshot, underlay, request)?;
        let written = binding.written(app, &snapshot)?;
        let (source, overrides, file_name) = match app.document.current_rule_program() {
            Some(program) => (
                with_data(&program.source, &name, &written),
                Some(program.overrides.clone()),
                program.file_name.clone(),
            ),
            None => (written, None, FILE_NAME.to_owned()),
        };
        Ok(Request::ApplyProgram {
            expected: Some(expected),
            source,
            overrides,
            file_name: Some(file_name),
            replace_document: false,
        })
    }
}

pub(super) fn binding_data(
    app: &KetchupApp,
    snapshot: &Snapshot,
    binding: &super::underlay_binding::Binding,
) -> Result<String, &'static str> {
    let underlay = super::underlay_regions::one_underlay(snapshot, Some(binding.underlay_id))?;
    let name = &binding.name;
    let document_point = |point: [f64; 2]| {
        let offset = underlay.offset_mm();
        underlay.to_document([point[0] - offset[0], point[1] - offset[1]])
    };
    let point_mm = binding.point.map(document_point);
    let layers = &binding.layers;
    let tolerance_mm = binding.tolerance.map(|value| value * underlay.scale());
    let closing_segments_mm = binding
        .closures
        .iter()
        .map(|ends| ends.map(document_point))
        .collect::<Vec<_>>();
    let lines = app
        .cached_underlay_lines(underlay)
        .ok_or("entity_not_found")?;
    let object_id = binding.object(&lines)?;
    let (origin, data) = match (point_mm, object_id, binding.block.as_deref()) {
        (Some(point), None, None) => {
            let region = super::underlay_regions::region_at(
                snapshot,
                underlay,
                &lines,
                layers.as_deref(),
                point,
                tolerance_mm,
                &closing_segments_mm,
            )?;
            (
                format!(
                    "the region around ({}) of {}; layers {layers:?}; closing_segments_mm {closing_segments_mm:?}",
                    point_text(point),
                    underlay.name
                ),
                outline_data(name, underlay, &region.outline, &region.holes)?,
            )
        }
        (None, Some(id), None) => {
            let index = ObjectIndex::new(&lines);
            let (outline, holes) = object_outline(&index, id)?;
            let handle = index.object(id).map_or(0, |object| object.handle);
            (
                format!("object {id} (handle {handle:X}) of {}", underlay.name),
                outline_data(name, underlay, &outline, &holes)?,
            )
        }
        (None, None, Some(block)) => (
            format!(
                "the shared linework of block {block:?} in {}",
                underlay.name
            ),
            block_data(name, underlay, &ObjectIndex::new(&lines), block)?,
        ),
        _ => {
            return Err(invalid(
                "point_mm",
                "Give exactly one of point_mm, object_id or block.",
                "point_mm for the region around a point, object_id for one closed object, block for every placement of a block.",
            ));
        }
    };
    Ok(format!(
        "{BEGIN} {name}: {origin}. Follows the referenced underlay.\n{}{}\n{data}{END} {name}\n",
        super::underlay_binding::PREFIX,
        serde_json::to_string(binding).map_err(|error| failed_because("invalid_params", error))?,
    ))
}

const BEGIN: &str = "# underlay data";
const END: &str = "# end of underlay data";

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|next| next.is_ascii_alphanumeric() || next == '_')
        && name.len() <= ketchup_model::tolerance::limits::NAME_BYTES
}

/// `source` with the data named `name` replaced by `written`, or with `written` first.
pub(super) fn with_data(source: &str, name: &str, written: &str) -> String {
    let begin = format!("{BEGIN} {name}:");
    let end = format!("{END} {name}");
    let mut offset = 0;
    let mut start = None;
    for line in source.split_inclusive('\n') {
        let text = line.trim_end();
        if start.is_none() && text.starts_with(&begin) {
            start = Some(offset);
        } else if let Some(first) = start
            && text == end
        {
            let after = offset + line.len();
            return format!("{}{written}{}", &source[..first], &source[after..]);
        }
        offset += line.len();
    }
    format!("{written}\n{source}")
}

/// A number as the program reads it: to the thousandth of a millimetre, no trailing zeros.
fn number(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

fn point_text(point: [f64; 2]) -> String {
    format!("{}, {}", number(point[0]), number(point[1]))
}

fn ring_text(underlay: &Underlay, ring: &[[f64; 2]], indent: &str) -> String {
    let points = ring
        .iter()
        .map(|point| format!("({})", point_text(underlay.to_document(*point))))
        .collect::<Vec<_>>();
    points
        .chunks(POINTS_PER_LINE)
        .map(|chunk| format!("{indent}{},\n", chunk.join(", ")))
        .collect()
}

/// `NAME` the outline and `NAME_HOLES` its holes, in document millimetres.
fn outline_data(
    name: &str,
    underlay: &Underlay,
    outline: &[[f64; 2]],
    holes: &[Vec<[f64; 2]>],
) -> Result<String, &'static str> {
    let total = outline.len() + holes.iter().map(Vec::len).sum::<usize>();
    if total > MAX_ITEMS {
        return Err(invalid(
            "point_mm",
            format!(
                "The outline has {total} corners, more than the {MAX_ITEMS} one piece of data holds."
            ),
            "Pick a smaller region or object.",
        ));
    }
    let mut data = format!(
        "{name} = [\n{}]\n{name}_HOLES = [\n",
        ring_text(underlay, outline, "    ")
    );
    for hole in holes {
        data.push_str(&format!(
            "    [\n{}    ],\n",
            ring_text(underlay, hole, "        ")
        ));
    }
    data.push_str(&format!("]\ncomponent({}, [profile_part({}, points={name}, closed=True)] + [profile_part({} % (i + 1), points=hole, closed=True) for i, hole in enumerate({name}_HOLES)])\n", quoted(&format!("{name}_sketch")), quoted(&format!("{name}/outline")), quoted(&format!("{name}/hole%d"))));
    Ok(data)
}

fn signed_area(ring: &[[f64; 2]]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| cross2(*a, *b))
        .sum::<f64>()
        / 2.0
}

type Ring = Vec<[f64; 2]>;

/// The outline (counterclockwise) and holes (clockwise) of object `id`: a hatch's
/// boundary, or the closed polylines it draws, the largest as the outline.
fn object_outline(index: &ObjectIndex<'_>, id: u32) -> Result<(Ring, Vec<Ring>), &'static str> {
    let object = index.object(id).ok_or("entity_not_found")?;
    let mut rings = match &object.detail {
        ObjectDetail::Hatch { rings, .. } => rings.clone(),
        _ => index
            .own_polylines(id)
            .into_iter()
            .filter(|run| run.len() >= 4 && run.first() == run.last())
            .map(|run| run[..run.len() - 1].to_vec())
            .collect(),
    };
    rings.retain(|ring| ring.len() >= 3);
    let largest = (0..rings.len())
        .max_by(|a, b| {
            signed_area(&rings[*a])
                .abs()
                .total_cmp(&signed_area(&rings[*b]).abs())
        })
        .ok_or_else(|| {
            invalid(
                "object_id",
                format!(
                    "Object {id} ({}) draws no closed outline.",
                    object.kind.name()
                ),
                "Use point_mm for the region its lines enclose with the lines around it.",
            )
        })?;
    let mut outline = rings.swap_remove(largest);
    if signed_area(&outline) < 0.0 {
        outline.reverse();
    }
    for hole in &mut rings {
        if signed_area(hole) > 0.0 {
            hole.reverse();
        }
    }
    Ok((outline, rings))
}

/// Shared sampled linework, with the first placement as the component's original
/// occurrence. Other occurrences map that geometry by the full relative affine map.
fn block_data(
    name: &str,
    underlay: &Underlay,
    index: &ObjectIndex<'_>,
    block: &str,
) -> Result<String, &'static str> {
    let filter = ObjectFilter {
        kind: Some("INSERT".to_owned()),
        block: Some(block.to_owned()),
        ..ObjectFilter::default()
    };
    let placed = index
        .find(&filter)
        .into_iter()
        .filter_map(|id| {
            let object = index.object(id)?;
            match &object.detail {
                ObjectDetail::Insert {
                    block: placed,
                    transform,
                    attributes,
                    copies,
                } if placed.eq_ignore_ascii_case(block) => {
                    Some((id, object, transform, attributes, copies))
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    let Some((first_id, _, first_map, _, _)) = placed.first() else {
        return Err(invalid(
            "block",
            format!("No insert places a block named {block:?}."),
            "Take the name from inspect action=underlay_objects (its blocks list).",
        ));
    };
    if placed.len() > MAX_ITEMS || placed.iter().any(|(_, _, _, _, copies)| **copies != [1, 1]) {
        return Err(invalid(
            "block",
            "Too many placements or an older array without separate copy maps.",
            "Reimport the drawing to recover each array copy; convert at most 4000 placements.",
        ));
    }
    let first = block_transform(underlay, first_map)?;
    let inverse = first.inverse().ok_or_else(|| {
        invalid(
            "block",
            "The first block placement is singular.",
            "Choose an insert with nonzero scale on both axes.",
        )
    })?;
    let mut pending = index.children(*first_id).to_vec();
    let mut runs = Vec::new();
    let mut points = 0;
    while let Some(id) = pending.pop() {
        pending.extend(index.children(id).iter().rev());
        for run in index.own_polylines(id) {
            points += run.len();
            if points > MAX_ITEMS {
                return Err(invalid(
                    "block",
                    "The block exceeds 4000 sampled points.",
                    "Choose a smaller block.",
                ));
            }
            runs.push(run);
        }
    }
    if runs.is_empty() {
        return Err(invalid(
            "block",
            "The block has no linework to convert to profiles.",
            "Choose a block with lines or polylines; text and fills remain in the underlay.",
        ));
    }
    let mut data = format!(
        "# Sampled linework only; texts, attributes and fills remain in the underlay.\n{name} = component({}, [\n",
        quoted(name)
    );
    for (i, run) in runs.iter().enumerate() {
        let closed = run.len() > 2 && run.first() == run.last();
        let run = if closed { &run[..run.len() - 1] } else { run };
        let coordinates = run
            .iter()
            .map(|point| {
                let [x, y] = underlay.to_document(*point);
                format!("({x:?}, {y:?})")
            })
            .collect::<Vec<_>>()
            .join(", ");
        data.push_str(&format!(
            "    profile_part({}, points=[{coordinates}], closed={}),\n",
            quoted(&format!("{name}/curve_{}", i + 1)),
            if closed { "True" } else { "False" }
        ));
    }
    data.push_str("])\n");
    for (copy, (_, object, map, attributes, _)) in placed.iter().enumerate() {
        data.push_str(&format!(
            "# Placement {}: handle {:X}, attributes {}\n",
            copy + 1,
            object.handle,
            quoted(&format!("{attributes:?}"))
        ));
        if copy == 0 {
            continue;
        }
        let relative = block_transform(underlay, map)?.compose(inverse);
        data.push_str(&format!(
            "instance({}, {name}, matrix={:?})\n",
            quoted(&format!("{name}_copy_{}", copy + 1)),
            relative.matrix()
        ));
    }
    Ok(data)
}

fn block_transform(
    underlay: &Underlay,
    map: &Affine,
) -> Result<ketchup_model::document::Transform, &'static str> {
    let transform = ketchup_model::document::Transform::from_matrix([
        map[0][0], map[0][1], 0.0, map[0][2], map[1][0], map[1][1], 0.0, map[1][2], 0.0, 0.0, 1.0,
        0.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .map_err(|error| {
        invalid(
            "block",
            format!("Invalid block placement: {error}"),
            "Reimport a drawing with finite placements.",
        )
    })?;
    Ok(underlay.placement.compose(transform))
}

/// `text` as a program string literal.
fn quoted(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    for character in text.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            control if control.is_control() => {
                literal.push_str(&format!("\\x{:02x}", u32::from(control)));
            }
            other => literal.push(other),
        }
    }
    literal.push('"');
    literal
}
