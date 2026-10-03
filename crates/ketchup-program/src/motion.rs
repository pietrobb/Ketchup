//! Program poses use the canonical kinematics, separately from reference geometry.
use crate::{ProgramModel, frame, model::ProgramInstance};
use ketchup_model::{
    assembly_joint::{AssemblyJointAxis, AssemblyJointKind, AssemblyJointLimits},
    document::Transform,
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramMotion {
    pub name: String,
    pub endpoints: [String; 2],
    /// Expanded copy of a shared motion; the compiler edits its original only.
    pub instanced: bool,
    /// Canonical motion without travel limits; limits are reported, never clamped.
    pub kind: AssemblyJointKind,
    pub limits: AssemblyJointLimits,
    pub position: f64,
}

impl ProgramMotion {
    pub fn transform(&self) -> Option<Transform> {
        self.kind.transform_from_zero()
    }

    pub(crate) fn instanced(&self, instance: &ProgramInstance) -> Self {
        let axis = |axis: AssemblyJointAxis| {
            let pivot = frame::apply(&instance.rotation, axis.pivot_in_parent_mm());
            AssemblyJointAxis::new(
                frame::apply(&instance.rotation, axis.direction_in_parent()),
                std::array::from_fn(|i| pivot[i] + instance.at_mm[i]),
            )
        };
        let kind = match self.kind {
            AssemblyJointKind::Prismatic {
                axis: a,
                limits,
                position_mm,
            } => AssemblyJointKind::Prismatic {
                axis: axis(a),
                limits,
                position_mm,
            },
            AssemblyJointKind::Revolute {
                axis: a,
                limits,
                position_degrees,
            } => AssemblyJointKind::Revolute {
                axis: axis(a),
                limits,
                position_degrees,
            },
            other => other,
        };
        Self {
            name: format!("{}/{}", instance.name, self.name),
            endpoints: self
                .endpoints
                .each_ref()
                .map(|name| format!("{}/{name}", instance.name)),
            instanced: true,
            kind,
            limits: self.limits,
            position: self.position,
        }
    }
}

impl ProgramModel {
    /// Includes the endpoint itself, groups and all their leaf members.
    pub fn member_names(&self, endpoint: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let mut pending = vec![endpoint.to_owned()];
        while let Some(name) = pending.pop() {
            if names.insert(name.clone())
                && let Some(group) = self.groups.iter().find(|group| group.name == name)
            {
                pending.extend(group.members.iter().cloned());
            }
        }
        names
    }
}

fn validate_shared(model: &ProgramModel, motion: &ProgramMotion) -> anyhow::Result<()> {
    if motion.instanced {
        return Ok(());
    }
    let target = &motion.endpoints[1];
    if model
        .instances
        .iter()
        .any(|i| &i.name != target && model.member_names(&i.name).contains(target))
    {
        anyhow::bail!(
            "joint {:?}: cannot drive a copied member separately; declare its motion before component() in the shared definition",
            motion.name
        );
    }
    if let Some(owner) = model
        .components
        .iter()
        .find(|c| c.groups.iter().any(|g| g.members.contains(target)))
        && !owner.motions.contains(motion)
    {
        anyhow::bail!(
            "joint {:?}: shared member {target:?} requires both endpoints and its motion before component({:?}); move the whole instance for independent placement",
            motion.name,
            owner.name
        );
    }
    Ok(())
}

pub(crate) fn validate(model: &ProgramModel) -> anyhow::Result<()> {
    let moving = model
        .motions
        .iter()
        .map(|m| model.member_names(&m.endpoints[1]))
        .collect::<Vec<_>>();
    for (index, motion) in model.motions.iter().enumerate() {
        let [anchor, target] = &motion.endpoints;
        for name in [anchor, target] {
            if model.part(name).is_none() && !model.groups.iter().any(|g| &g.name == name) {
                anyhow::bail!("joint {:?}: unknown endpoint {name:?}", motion.name);
            }
        }
        validate_shared(model, motion)?;
        let anchored = model.member_names(anchor);
        if !moving[index].is_disjoint(&anchored) {
            anyhow::bail!(
                "joint {:?}: anchor and moving endpoint overlap; choose disjoint assemblies",
                motion.name
            );
        }
        for (other, names) in moving.iter().enumerate() {
            if other == index {
                continue;
            }
            if !moving[index].is_disjoint(names)
                && (moving[index] == *names
                    || (!moving[index].is_subset(names) && !names.is_subset(&moving[index])))
            {
                anyhow::bail!(
                    "joint {:?}: moving endpoints overlap; use one motion per assembly",
                    motion.name
                );
            }
            // A parent carries both sides of an internal joint. Separate driven-anchor
            // chains need the canonical constraint solver, not an order-dependent pose.
            if !anchored.is_disjoint(names)
                && !(anchored.is_subset(names) && moving[index].is_subset(names))
            {
                anyhow::bail!(
                    "joint {:?}: its anchor is driven by another joint; coupled motion chains are not yet supported",
                    motion.name
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn pose(reference: &ProgramModel) -> anyhow::Result<ProgramModel> {
    validate(reference)?;
    let mut posed = reference.clone();
    let mut motions = reference
        .motions
        .iter()
        .map(|m| (m, reference.member_names(&m.endpoints[1])))
        .collect::<Vec<_>>();
    // Descendant motion is expressed in the reference frame, then carried by its parent.
    motions.sort_by_key(|(_, names)| names.len());
    for (motion, names) in motions {
        let transform = motion.transform().ok_or_else(|| {
            anyhow::anyhow!("joint {:?}: invalid motion axis or position", motion.name)
        })?;
        let matrix = transform.matrix();
        let rotation = std::array::from_fn(|row| std::array::from_fn(|col| matrix[row * 4 + col]));
        let shift = [matrix[3], matrix[7], matrix[11]];
        for part in &mut posed.parts {
            if names.contains(&part.name) {
                crate::eval::pose_part(part, &rotation, shift);
            }
        }
        for joint in &mut posed.joints {
            if joint.parts.iter().all(|name| names.contains(name)) {
                for point in &mut joint.fasteners_mm {
                    *point = transform.transform_point(*point);
                }
                joint.volume_mm = joint
                    .volume_mm
                    .map(|(min, max)| frame::Obb::new(shift, &rotation, min, max).world_bounds());
            }
        }
    }
    Ok(posed)
}

pub(crate) fn issues(model: &ProgramModel, issues: &mut Vec<crate::Issue>) {
    for motion in &model.motions {
        if !motion.limits.contains(motion.position) {
            issues.push(crate::Issue {
                severity: crate::Severity::Error,
                kind: "joint_out_of_range",
                parts: motion.endpoints.to_vec(),
                message: format!(
                    "joint {} position {} is outside [{}, {}]",
                    motion.name,
                    motion.position,
                    motion.limits.min(),
                    motion.limits.max()
                ),
                where_mm: None,
                hint:
                    "Set the position inside the declared travel range; the pose was not clamped."
                        .to_owned(),
            });
        }
    }
}
