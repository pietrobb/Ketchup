//! Physical connections and independent rigid assembly motion declarations.
use super::*;
use crate::model::{JointLink, JointOperationRef};

fn link_field<'v>(value: Value<'v>, name: &str, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
    value
        .get_attr(name, heap)
        .map_err(|error| anyhow::anyhow!("joint link field {name}: {error}"))?
        .ok_or_else(|| anyhow::anyhow!("joint link is missing {name}; use joint_link() and operation references from hole() or struct(part=..., id=...)"))
}

fn parse_links<'v>(value: Option<Value<'v>>, heap: &'v Heap) -> anyhow::Result<Vec<JointLink>> {
    let Some(value) = given(value) else {
        return Ok(Vec::new());
    };
    iterate(
        value,
        heap,
        format_args!("links must be a list of joint_link() values"),
    )?
    .map(|link| {
        let operations = iterate(
            link_field(link, "operations", heap)?,
            heap,
            format_args!("operations must be a list of operation references"),
        )?
        .map(|operation| {
            let part = part_name(link_field(operation, "part", heap)?, heap)?;
            let id = link_field(operation, "id", heap)?
                .unpack_str()
                .ok_or_else(|| {
                    anyhow::anyhow!("operation id must be text; use the result of hole()")
                })?
                .to_owned();
            Ok(JointOperationRef { part, id })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
        let hardware_parts = iterate(
            link_field(link, "hardware", heap)?,
            heap,
            format_args!("hardware must be a list of existing physical parts"),
        )?
        .map(|part| part_name(part, heap))
        .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(JointLink {
            operations,
            hardware_parts,
        })
    })
    .collect()
}

/// Referential integrity only: ownership is explicit, never reconstructed by proximity.
pub(super) fn validate(model: &ProgramModel) -> anyhow::Result<()> {
    let mut owned = BTreeSet::new();
    let mut hardware = BTreeSet::new();
    for joint in &model.joints {
        for part in &joint.parts {
            if model.part(part).is_none() {
                anyhow::bail!(
                    "joint {:?} refers to missing part {part:?}; keep the part or remove its joint",
                    joint.name
                );
            }
        }
        for link in &joint.links {
            for operation in &link.operations {
                let part = model.part(&operation.part).ok_or_else(|| {
                    anyhow::anyhow!(
                        "joint {:?} operation refers to unknown part {:?}; declare the part first",
                        joint.name,
                        operation.part
                    )
                })?;
                if !joint.parts.contains(&operation.part)
                    && !link.hardware_parts.contains(&operation.part)
                {
                    anyhow::bail!(
                        "joint {:?}: operation part {:?} is neither an endpoint nor linked hardware; correct the operation reference",
                        joint.name,
                        operation.part
                    );
                }
                if !part
                    .operations
                    .iter()
                    .any(|item| item.name() == operation.id)
                {
                    anyhow::bail!(
                        "joint {:?}: unknown operation {:?} on {:?}; declare the operation first and use its exact id",
                        joint.name,
                        operation.id,
                        operation.part
                    );
                }
                if !owned.insert(operation) {
                    anyhow::bail!(
                        "joint {:?}: operation {:?} on {:?} is already owned; associate each operation with only one joint link",
                        joint.name,
                        operation.id,
                        operation.part
                    );
                }
            }
            for part in &link.hardware_parts {
                if model.part(part).is_none() {
                    anyhow::bail!(
                        "joint {:?}: unknown hardware part {part:?}; declare a physical part first or leave hardware empty for metadata only",
                        joint.name
                    );
                }
                if joint.parts.contains(part) || !hardware.insert(part) {
                    anyhow::bail!(
                        "joint {:?}: hardware part {part:?} is an endpoint or already linked; use a separate physical part for each link",
                        joint.name
                    );
                }
            }
        }
    }
    Ok(())
}
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

    /// Append an insertion path. Geometry conflicts are reported by validation,
    /// not rejected while editing. Use the named motion's units for from/to.
    fn assembly_step<'v>(
        #[starlark(require = pos)] motion: &str,
        #[starlark(require = named)] start: Value<'v>,
        #[starlark(require = named)] end: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let step = crate::motion::ProgramAssemblyStep {
            motion: motion.to_owned(),
            from: number(start, "assembly_step start")?,
            to: number(end, "assembly_step end")?,
        };
        state(eval)?.model.borrow_mut().assembly_steps.push(step);
        Ok(NoneType)
    }

    /// Check an auxiliary volume approaching its zero-position working pose.
    fn tool_access<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] envelope: Option<Value<'v>>,
        #[starlark(require = named)] motion: Option<Value<'v>>,
        #[starlark(require = named)] start: Option<Value<'v>>,
        #[starlark(require = named)] end: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let parsed = given(motion)
            .map(|v| parse_motion(v, 0.0, heap))
            .transpose()?;
        let access = crate::motion::ProgramToolAccess {
            name: name.to_owned(),
            envelope: given(envelope).map(|v| part_name(v, heap)).transpose()?,
            kind: parsed.map(|(kind, _)| kind),
            limits: parsed.map(|(_, limits)| limits),
            start: given(start)
                .map(|v| number(v, "tool_access start"))
                .transpose()?,
            end: given(end).map_or(Ok(0.0), |v| number(v, "tool_access end"))?,
        };
        state(eval)?.model.borrow_mut().tool_access.push(access);
        Ok(NoneType)
    }

    fn joint<'v>(
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        #[starlark(require = named)] kind: &str,
        #[starlark(require = named)] fasteners: Option<Value<'v>>,
        #[starlark(require = named)] links: Option<Value<'v>>,
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
            if [fasteners, fastener, volume, max_gap, links]
                .into_iter()
                .any(|v| given(v).is_some())
            {
                anyhow::bail!(
                    "joint {name:?}: motion cannot also specify physical fasteners, links, volume or max_gap; declare physical joints separately"
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
            links: parse_links(links, heap)?,
            volume_mm: volume,
            fasteners_mm: fasteners,
            fastener: text(fastener, "fastener")?,
            max_gap_mm: max_gap,
        });
        Ok(NoneType)
    }
}
