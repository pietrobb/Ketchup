#![forbid(unsafe_code)]

use crate::{Ray, Vec3};
use ketchup_core::document::{DefinitionId, InstancePath, Snapshot};
use ketchup_core::exact_product::{AssemblySelectionTarget, ExactFaceRole, ExactRenderPackage};
use std::collections::BTreeMap;
use std::sync::Arc;

const RAY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug, PartialEq)]
pub struct DurableExactHit {
    pub target: AssemblySelectionTarget,
    pub position_mm: Vec3,
    pub ray_distance_mm: f64,
}

#[derive(Clone, Debug)]
struct ExactOccurrence {
    instance_path: InstancePath,
    origin_mm: Vec3,
    package: Arc<ExactRenderPackage>,
}

#[derive(Clone, Debug, Default)]
pub struct ExactInteractionProjection {
    occurrences: Vec<ExactOccurrence>,
}

impl ExactInteractionProjection {
    #[must_use]
    pub fn from_snapshot(
        snapshot: &Snapshot,
        packages: &BTreeMap<DefinitionId, Arc<ExactRenderPackage>>,
    ) -> Self {
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                let package = Arc::clone(packages.get(&occurrence.definition_id)?);
                if !package.is_current(snapshot)
                    || package.identity.definition_id != occurrence.definition_id
                {
                    return None;
                }
                let matrix = occurrence.transform.matrix();
                if matrix[0] != 1.0
                    || matrix[1] != 0.0
                    || matrix[2] != 0.0
                    || matrix[4] != 0.0
                    || matrix[5] != 1.0
                    || matrix[6] != 0.0
                    || matrix[8] != 0.0
                    || matrix[9] != 0.0
                    || matrix[10] != 1.0
                    || matrix[12] != 0.0
                    || matrix[13] != 0.0
                    || matrix[14] != 0.0
                    || matrix[15] != 1.0
                {
                    return None;
                }
                Some(ExactOccurrence {
                    instance_path: occurrence.instance_path,
                    origin_mm: Vec3::new(matrix[3], matrix[7], matrix[11]),
                    package,
                })
            })
            .collect();
        Self { occurrences }
    }

    #[must_use]
    pub fn occurrence_count(&self) -> usize {
        self.occurrences.len()
    }

    #[must_use]
    pub fn exact_pick(&self, ray: Ray) -> Option<DurableExactHit> {
        self.occurrences
            .iter()
            .filter_map(|occurrence| hit_occurrence(ray, occurrence))
            .min_by(|left, right| {
                left.ray_distance_mm
                    .total_cmp(&right.ray_distance_mm)
                    .then_with(|| left.target.instance_path.cmp(&right.target.instance_path))
            })
    }
}

fn hit_occurrence(ray: Ray, occurrence: &ExactOccurrence) -> Option<DurableExactHit> {
    let min = occurrence.package.bounds_mm[0];
    let max = occurrence.package.bounds_mm[1];
    let local_origin = ray.origin - occurrence.origin_mm;
    let origins = [local_origin.x, local_origin.y, local_origin.z];
    let directions = [ray.direction.x, ray.direction.y, ray.direction.z];
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    let mut near_face = (0_usize, false);
    let mut far_face = (0_usize, true);

    for axis in 0..3 {
        if directions[axis].abs() <= RAY_EPSILON {
            if origins[axis] < min[axis] || origins[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let first = (min[axis] - origins[axis]) / directions[axis];
        let second = (max[axis] - origins[axis]) / directions[axis];
        let (axis_near, axis_far, near_maximum, far_maximum) = if first <= second {
            (first, second, false, true)
        } else {
            (second, first, true, false)
        };
        if axis_near > near {
            near = axis_near;
            near_face = (axis, near_maximum);
        }
        if axis_far < far {
            far = axis_far;
            far_face = (axis, far_maximum);
        }
        if far < near {
            return None;
        }
    }

    let (distance, face) = if near >= 0.0 {
        (near, near_face)
    } else if far >= 0.0 {
        (far, far_face)
    } else {
        return None;
    };
    let role = match face {
        (2, true) => ExactFaceRole::Top,
        (2, false) => ExactFaceRole::Bottom,
        (0, true) => ExactFaceRole::East,
        _ => return None,
    };
    let reference = occurrence.package.reference(role)?.clone();
    Some(DurableExactHit {
        target: AssemblySelectionTarget {
            instance_path: occurrence.instance_path.clone(),
            body: reference,
        },
        position_mm: ray.at(distance),
        ray_distance_mm: distance,
    })
}
