//! Hole construction and its reusable operation reference, separate from builtin dispatch.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn drill<'v>(
    part: Value<'v>,
    face: Value<'v>,
    at: Option<Value<'v>>,
    world: Option<Value<'v>>,
    diameter: Value<'v>,
    depth: Value<'v>,
    through: bool,
    id: Option<Value<'v>>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> anyhow::Result<Value<'v>> {
    let heap = eval.heap();
    let name = part_name(part, heap)?;
    let face = face_name(face, heap, Some(&name))?;
    let diameter = number(diameter, "diameter")?;
    let depth = number(depth, "depth")?;
    if diameter <= 0.0 || depth <= 0.0 {
        anyhow::bail!("hole in {name:?}: diameter and depth must be positive");
    }
    let at = given(at)
        .map(|at| numbers::<2>(at, heap, "at"))
        .transpose()?;
    let world = given(world)
        .map(|world| numbers::<3>(world, heap, "world"))
        .transpose()?;
    let id = text(id, "id")?;
    let state = state(eval)?;
    record_source(eval, &state, &[&name]);
    with_part(&state, &name, |part| {
        let frame = part
            .machining_frame(&face)
            .map_err(|error| anyhow::anyhow!("hole in {name:?}: {error}"))?;
        let at = match (at, world) {
            (Some(at), None) => at,
            (None, Some(world)) => frame.coordinates(part.to_local(world)),
            _ => {
                anyhow::bail!("hole in {name:?}: give exactly one of at=(u, v) or world=(x, y, z)")
            }
        };
        let id = id.unwrap_or_else(|| format!("h{}", part.holes().count() + 1));
        add_operation(
            part,
            ProgramOperation::Hole(Hole {
                id: id.clone(),
                face,
                at_mm: at,
                entry_mm: frame.point(at),
                inward: frame.normal_at(at).map(|value| -value),
                diameter_mm: diameter,
                depth_mm: depth,
                through,
            }),
        )?;
        Ok(heap.alloc(AllocStruct([
            ("part", heap.alloc(name.as_str())),
            ("id", heap.alloc(id)),
        ])))
    })
}
