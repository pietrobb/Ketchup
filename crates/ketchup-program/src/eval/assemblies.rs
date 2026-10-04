//! Assembly declarations and rigid instances of shared definitions.
use super::*;
use crate::model::{ProgramComponent, ProgramGroup, ProgramInstance};

fn check_name(model: &ProgramModel, name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        anyhow::bail!("group name must be non-empty printable text");
    }
    if model.part(name).is_some()
        || model.tool(name).is_some()
        || model.groups.iter().any(|group| group.name == name)
    {
        anyhow::bail!("group {name:?} already exists; use a unique part or group name");
    }
    Ok(())
}

fn add_group(
    model: &mut ProgramModel,
    name: &str,
    members: Vec<String>,
    grounded: bool,
) -> anyhow::Result<()> {
    check_name(model, name)?;
    if model.groups.len() >= MAX_PARTS {
        anyhow::bail!("more than {MAX_PARTS} groups; check the program for a runaway loop");
    }
    let mut unique = BTreeSet::new();
    for member in &members {
        if !unique.insert(member) {
            anyhow::bail!("group {name:?} lists {member:?} twice; list each member once");
        }
        if model.part(member).is_none() && !model.groups.iter().any(|group| &group.name == member) {
            anyhow::bail!(
                "group {name:?} refers to unknown member {member:?}; declare the part or group first"
            );
        }
        if let Some(parent) = model
            .groups
            .iter()
            .find(|group| group.members.contains(member))
        {
            anyhow::bail!(
                "{member:?} already belongs to group {:?}; a member can have only one parent",
                parent.name
            );
        }
    }
    model.groups.push(ProgramGroup {
        name: name.to_owned(),
        grounded,
        members,
    });
    Ok(())
}

fn members<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<Vec<String>> {
    iterate(
        value,
        heap,
        format_args!("group members must be a list of parts or groups"),
    )?
    .map(|member| part_name(member, heap))
    .collect()
}

fn descendants(model: &ProgramModel, name: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut pending = vec![name.to_owned()];
    while let Some(name) = pending.pop() {
        if names.insert(name.clone())
            && let Some(group) = model.groups.iter().find(|group| group.name == name)
        {
            pending.extend(group.members.iter().cloned());
        }
    }
    names
}

fn capture(model: &ProgramModel, name: &str) -> anyhow::Result<ProgramComponent> {
    let names = descendants(model, name);
    for joint in &model.joints {
        if joint.parts.iter().all(|part| names.contains(part))
            && joint
                .links
                .iter()
                .flat_map(|link| &link.hardware_parts)
                .any(|part| !names.contains(part))
        {
            anyhow::bail!(
                "component {name:?}: joint {:?} links hardware outside its members; include the physical hardware in the component",
                joint.name
            );
        }
    }
    Ok(ProgramComponent {
        name: name.to_owned(),
        parts: model
            .parts
            .iter()
            .filter(|part| names.contains(&part.name))
            .cloned()
            .collect(),
        groups: model
            .groups
            .iter()
            .filter(|group| names.contains(&group.name))
            .cloned()
            .collect(),
        joints: model
            .joints
            .iter()
            .filter(|joint| joint.parts.iter().all(|part| names.contains(part)))
            .cloned()
            .collect(),
        motions: model
            .motions
            .iter()
            .filter(|motion| motion.endpoints.iter().all(|name| names.contains(name)))
            .cloned()
            .collect(),
    })
}

fn instance_part(part: &Part, instance: &ProgramInstance) -> Part {
    let mut part = part.clone();
    part.name = format!("{}/{}", instance.name, part.name);
    move_part(&mut part, &instance.rotation, [0.0; 3]);
    translate_part(&mut part, instance.at_mm);
    part.refresh_feature_tree();
    part
}

fn instance_groups(component: &ProgramComponent, instance: &ProgramInstance) -> Vec<ProgramGroup> {
    component
        .groups
        .iter()
        .map(|group| ProgramGroup {
            name: if group.name == component.name {
                instance.name.clone()
            } else {
                format!("{}/{}", instance.name, group.name)
            },
            grounded: group.grounded,
            members: group
                .members
                .iter()
                .map(|member| format!("{}/{}", instance.name, member))
                .collect(),
        })
        .collect()
}

fn instance_joint(joint: &Joint, instance: &ProgramInstance) -> Joint {
    let mut joint = joint.clone();
    joint.name = format!("{}/{}", instance.name, joint.name);
    joint.parts = joint
        .parts
        .map(|name| format!("{}/{}", instance.name, name));
    for link in &mut joint.links {
        for operation in &mut link.operations {
            operation.part = format!("{}/{}", instance.name, operation.part);
        }
        for part in &mut link.hardware_parts {
            *part = format!("{}/{}", instance.name, part);
        }
    }
    let point = |p| {
        let rotated = frame::apply(&instance.rotation, p);
        std::array::from_fn(|axis| rotated[axis] + instance.at_mm[axis])
    };
    joint.fasteners_mm = joint.fasteners_mm.into_iter().map(point).collect();
    joint.volume_mm = joint.volume_mm.map(|(min, max)| {
        frame::Obb::new(instance.at_mm, &instance.rotation, min, max).world_bounds()
    });
    joint
}

pub(super) fn validate(model: &ProgramModel) -> anyhow::Result<()> {
    for component in &model.components {
        for part in &component.parts {
            if model.part(&part.name) != Some(part) {
                anyhow::bail!(
                    "component {:?} member {:?} changed after component(); finish machining and placement before declaring the component",
                    component.name,
                    part.name
                );
            }
        }
        for instance in model
            .instances
            .iter()
            .filter(|item| item.component == component.name)
        {
            for part in &component.parts {
                let expected = instance_part(part, instance);
                if model.part(&expected.name) != Some(&expected) {
                    anyhow::bail!(
                        "instance {:?} member {:?} was edited separately; edit the shared parts before component(), or use copy() for an independent part",
                        instance.name,
                        expected.name
                    );
                }
            }
        }
    }
    Ok(())
}

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Copies geometry and the part's explicit support anchor, not joints or contacts.
    fn copy<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let source = part_name(part, heap)?;
        check_part_name(name)?;
        let state = state(eval)?;
        let (copy, tool) = {
            let model = state.model.borrow();
            match (model.part(&source), model.tool(&source)) {
                (Some(part), _) => (part.clone(), false),
                (None, Some(part)) => (part.clone(), true),
                (None, None) => anyhow::bail!("copy(): unknown part {source:?}"),
            }
        };
        let copy = insert_part(
            &state,
            Part {
                name: name.to_owned(),
                ..copy
            },
            tool,
        )?;
        {
            let mut sources = state.part_sources.borrow_mut();
            let lines = sources.get(&source).cloned().unwrap_or_default();
            sources.entry(name.to_owned()).or_default().extend(lines);
        }
        record_source(eval, &state, &[name]);
        Ok(part_value(&copy, heap))
    }

    /// Groups existing parts or groups without moving their world frames.
    fn group<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] items: Value<'v>,
        #[starlark(require = named, default = false)] grounded: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let members = members(items, heap)?;
        add_group(
            &mut state(eval)?.model.borrow_mut(),
            name,
            members,
            grounded,
        )?;
        Ok(heap.alloc(AllocStruct([("name", heap.alloc(name))])))
    }

    /// Adds document tags (layers) to parts: `items` is a part, a group (every
    /// part in it) or a list of them; `tags` one name or a list. Tag shared
    /// parts before component(), so every instance carries the same tags.
    fn tag<'v>(
        #[starlark(require = pos)] items: Value<'v>,
        #[starlark(require = pos)] tags: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let single =
            items.unpack_str().is_some() || items.get_attr("name", heap).ok().flatten().is_some();
        let items = if single {
            vec![part_name(items, heap)?]
        } else {
            members(items, heap)?
        };
        let tags = tag_names(tags, heap)?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        let mut names = BTreeSet::new();
        for item in &items {
            if model.part(item).is_none() && !model.groups.iter().any(|group| &group.name == item) {
                anyhow::bail!("tag(): unknown part or group {item:?}; declare it first");
            }
            names.extend(descendants(&model, item));
        }
        for part in model
            .parts
            .iter_mut()
            .filter(|part| names.contains(&part.name))
        {
            part.tags.extend(tags.iter().cloned());
        }
        Ok(NoneType)
    }

    /// Declares tags that are alternative representations of the same thing (a concept
    /// and a construction): parts carrying different ones of them are never reported
    /// as colliding with each other. Parts carrying none or the same one are checked.
    fn alternatives<'v>(
        #[starlark(require = pos)] tags: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let tags = tag_names(tags, eval.heap())?;
        if tags.len() < 2 {
            anyhow::bail!("alternatives(): name at least two tags, got {tags:?}");
        }
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if let Some(taken) = tags
            .iter()
            .find(|tag| model.alternative_tags.iter().any(|set| set.contains(*tag)))
        {
            anyhow::bail!(
                "alternatives(): tag {taken:?} is already in another set of alternatives"
            );
        }
        model.alternative_tags.push(tags.into_iter().collect());
        Ok(NoneType)
    }

    /// Declares a shared assembly, retaining its first instance at the current position.
    fn component<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] items: Value<'v>,
        #[starlark(require = named, default = false)] grounded: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let members = members(items, heap)?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        add_group(&mut model, name, members, grounded)?;
        let component = capture(&model, name)?;
        model.components.push(component);
        Ok(heap.alloc(AllocStruct([("name", heap.alloc(name))])))
    }

    /// Instantiates a component under a new name with a rigid transform from the program origin.
    fn instance<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] component: Value<'v>,
        #[starlark(require = named)] grounded: Option<bool>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named)] x: Option<Value<'v>>,
        #[starlark(require = named)] z: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let component_name = part_name(component, heap)?;
        let at_mm = given(at)
            .map(|value| numbers::<3>(value, heap, "instance at"))
            .transpose()?
            .unwrap_or([0.0; 3]);
        let x = given(x)
            .map(|value| numbers::<3>(value, heap, "instance x"))
            .transpose()?
            .unwrap_or([1.0, 0.0, 0.0]);
        let z = given(z)
            .map(|value| numbers::<3>(value, heap, "instance z"))
            .transpose()?
            .unwrap_or([0.0, 0.0, 1.0]);
        let rotation = frame::from_axes(x, z).ok_or_else(|| {
            anyhow::anyhow!("instance {name:?}: x and z must be non-zero and not parallel")
        })?;
        let instance = ProgramInstance {
            name: name.to_owned(),
            component: component_name.clone(),
            at_mm,
            rotation,
        };
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        let component = model
            .components
            .iter()
            .find(|item| item.name == component_name)
            .ok_or_else(|| {
                anyhow::anyhow!("unknown component {component_name:?}; declare component() first")
            })?
            .clone();
        let parts = component
            .parts
            .iter()
            .map(|part| instance_part(part, &instance))
            .collect::<Vec<_>>();
        let mut groups = instance_groups(&component, &instance);
        if let Some(grounded) = grounded {
            for group in groups.iter_mut().filter(|group| group.name == name) {
                group.grounded = grounded;
            }
        }
        if model.parts.len() + model.tools.len() + parts.len() > MAX_PARTS
            || model.groups.len() + groups.len() > MAX_PARTS
        {
            anyhow::bail!(
                "instance {name:?} exceeds {MAX_PARTS} parts or groups; reduce the instance count"
            );
        }
        let mut names = BTreeSet::new();
        for name in parts
            .iter()
            .map(|part| &part.name)
            .chain(groups.iter().map(|group| &group.name))
        {
            check_name(&model, name)?;
            if !names.insert(name) {
                anyhow::bail!(
                    "instance {name:?} has colliding member names; rename the source members"
                );
            }
        }
        for (original, part) in component.parts.iter().zip(&parts) {
            check_part_name(&part.name)?;
            let mut sources = state.part_sources.borrow_mut();
            let lines = sources.get(&original.name).cloned().unwrap_or_default();
            sources.entry(part.name.clone()).or_default().extend(lines);
            drop(sources);
            record_source(eval, &state, &[&part.name]);
        }
        model.parts.extend(parts);
        model.groups.extend(groups);
        model.joints.extend(
            component
                .joints
                .iter()
                .map(|joint| instance_joint(joint, &instance)),
        );
        model.motions.extend(
            component
                .motions
                .iter()
                .map(|motion| motion.instanced(&instance)),
        );
        model.instances.push(instance);
        Ok(heap.alloc(AllocStruct([("name", heap.alloc(name))])))
    }
}
