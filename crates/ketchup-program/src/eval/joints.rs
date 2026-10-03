//! Physical connections and independent rigid assembly motion declarations.
use super::*;
use ketchup_model::assembly_joint::{AssemblyJointAxis, AssemblyJointKind, AssemblyJointLimits};

fn motion_value<'v>(
    kind: &str,
    axis: Value<'v>,
    min: Value<'v>,
    max: Value<'v>,
    pivot: Option<Value<'v>>,
    heap: &'v Heap,
) -> anyhow::Result<Value<'v>> {
    let direction = numbers::<3>(axis, heap, "motion axis")?;
    let pivot = given(pivot)
        .map(|v| numbers::<3>(v, heap, "motion pivot"))
        .transpose()?
        .unwrap_or([0.0; 3]);
    let axis = AssemblyJointAxis::new(direction, pivot);
    let (min, max) = (number(min, "motion min")?, number(max, "motion max")?);
    if !axis.is_valid() || min > max {
        anyhow::bail!("motion requires a non-zero axis, finite pivot and min <= max");
    }
    Ok(heap.alloc(AllocStruct([
        ("kind", heap.alloc(kind)),
        ("axis", heap.alloc(axis.direction_in_parent().to_vec())),
        ("pivot", heap.alloc(pivot.to_vec())),
        ("min", heap.alloc(min)),
        ("max", heap.alloc(max)),
    ])))
}

fn parse_motion<'v>(
    value: Value<'v>,
    position: f64,
    heap: &'v Heap,
) -> anyhow::Result<(AssemblyJointKind, AssemblyJointLimits)> {
    let field = |name: &str| {
        value
            .get_attr(name, heap)
            .map_err(|error| anyhow::anyhow!("motion field {name}: {error}"))?
            .ok_or_else(|| {
                anyhow::anyhow!("motion is missing {name}; use slide() or rotate_motion()")
            })
    };
    let axis = AssemblyJointAxis::new(
        numbers::<3>(field("axis")?, heap, "motion axis")?,
        numbers::<3>(field("pivot")?, heap, "motion pivot")?,
    );
    let min = number(field("min")?, "motion min")?;
    let max = number(field("max")?, "motion max")?;
    if min > max {
        anyhow::bail!("motion requires min <= max");
    }
    let kind = match field("kind")?.unpack_str() {
        Some("slide") => AssemblyJointKind::Prismatic {
            axis,
            limits: None,
            position_mm: position,
        },
        Some("rotate") => AssemblyJointKind::Revolute {
            axis,
            limits: None,
            position_degrees: position,
        },
        _ => anyhow::bail!("unknown motion; use slide() or rotate_motion()"),
    };
    if kind.transform_from_zero().is_none() {
        anyhow::bail!(
            "invalid motion axis, pivot or position; use a non-zero direction and finite coordinates within the model range"
        );
    }
    Ok((kind, AssemblyJointLimits::new(min, max)))
}

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Describes travel in millimetres along a program-world axis.
    fn slide<'v>(
        #[starlark(require = pos)] axis: Value<'v>,
        #[starlark(require = pos)] min: Value<'v>,
        #[starlark(require = pos)] max: Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<Value<'v>> {
        motion_value("slide", axis, min, max, None, heap)
    }

    /// Describes right-hand travel in degrees about a program-world axis and pivot.
    fn rotate_motion<'v>(
        #[starlark(require = pos)] axis: Value<'v>,
        #[starlark(require = pos)] min: Value<'v>,
        #[starlark(require = pos)] max: Value<'v>,
        #[starlark(require = named)] pivot: Option<Value<'v>>,
        heap: &'v Heap,
    ) -> anyhow::Result<Value<'v>> {
        motion_value("rotate", axis, min, max, pivot, heap)
    }

    fn joint<'v>(
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        #[starlark(require = named)] kind: &str,
        #[starlark(require = named)] fasteners: Option<Value<'v>>,
        #[starlark(require = named)] fastener: Option<Value<'v>>,
        #[starlark(require = named)] volume: Option<Value<'v>>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        #[starlark(require = named)] max_gap: Option<Value<'v>>,
        #[starlark(require = named)] motion: Option<Value<'v>>,
        #[starlark(require = named)] position: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let (a, b) = (part_name(a, heap)?, part_name(b, heap)?);
        let name = text(name, "name")?.unwrap_or_else(|| format!("{kind}:{a}+{b}"));
        let state = state(eval)?;
        record_source(eval, &state, &[&a, &b]);
        let mut model = state.model.borrow_mut();
        if let Some(motion) = given(motion) {
            if [fasteners, fastener, volume, max_gap]
                .into_iter()
                .any(|v| given(v).is_some())
            {
                anyhow::bail!(
                    "joint {name:?}: motion cannot also specify physical fasteners, volume or max_gap; declare physical joints separately"
                );
            }
            let position = given(position).map_or(Ok(0.0), |v| number(v, "position"))?;
            let (kind, limits) = parse_motion(motion, position, heap)?;
            model.motions.push(crate::motion::ProgramMotion {
                name,
                endpoints: [a, b],
                instanced: false,
                kind,
                limits,
                position,
            });
            return Ok(NoneType);
        }
        if given(position).is_some() {
            anyhow::bail!(
                "joint {name:?}: position requires motion=slide(...) or rotate_motion(...)"
            );
        }
        let max_gap = given(max_gap).map_or(Ok(0.0), |gap| number(gap, "max_gap"))?;
        if max_gap < 0.0 {
            anyhow::bail!("max_gap must not be negative");
        }
        let fasteners = match given(fasteners) {
            None => Vec::new(),
            Some(list) => iterate(
                list,
                heap,
                format_args!("fasteners must be a list of (x, y, z) points"),
            )?
            .enumerate()
            .map(|(index, point)| numbers::<3>(point, heap, &format!("fasteners[{index}]")))
            .collect::<anyhow::Result<Vec<_>>>()?,
        };
        let volume = given(volume)
            .map(|volume| {
                let corners = iterate(volume, heap, format_args!("volume must be (min, max)"))?
                    .collect::<Vec<_>>();
                let [min, max] = corners.as_slice() else {
                    anyhow::bail!("volume must be (min, max)");
                };
                Ok((
                    numbers::<3>(*min, heap, "volume min")?,
                    numbers::<3>(*max, heap, "volume max")?,
                ))
            })
            .transpose()?;
        for part in [&a, &b] {
            if model.part(part).is_none() {
                anyhow::bail!("joint refers to unknown part {part:?}");
            }
        }
        model.joints.push(Joint {
            name,
            kind: kind.to_owned(),
            parts: [a, b],
            volume_mm: volume,
            fasteners_mm: fasteners,
            fastener: text(fastener, "fastener")?,
            max_gap_mm: max_gap,
        });
        Ok(NoneType)
    }
}
