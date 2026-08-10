#![forbid(unsafe_code)]

use crate::{Ray, Vec3};
use ketchup_core::document::{DocumentId, Snapshot, Transform};
use std::fmt;

pub const SPATIAL_INDEX_V1: &str = "ketchup.spatial-bvh.v1";
const LEAF_CAPACITY: usize = 4;
const RAY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotBinding {
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
}

impl SnapshotBinding {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
        }
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SpatialQueryStats {
    pub indexed_items: usize,
    pub bounds_tested: usize,
    pub candidate_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpatialQueryError {
    StaleProjection,
}

impl fmt::Display for SpatialQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("spatial projection is bound to a different canonical snapshot")
    }
}

impl std::error::Error for SpatialQueryError {}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SpatialBounds {
    min: Vec3,
    max: Vec3,
}

impl SpatialBounds {
    pub(crate) fn from_min_max(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub(crate) fn from_origin_size(origin: Vec3, size: Vec3) -> Self {
        Self {
            min: origin,
            max: origin + size,
        }
    }

    pub(crate) fn around(point: Vec3, tolerance: f64) -> Self {
        let extent = Vec3::new(tolerance, tolerance, tolerance);
        Self {
            min: point - extent,
            max: point + extent,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: Vec3::new(
                self.min.x.min(other.min.x),
                self.min.y.min(other.min.y),
                self.min.z.min(other.min.z),
            ),
            max: Vec3::new(
                self.max.x.max(other.max.x),
                self.max.y.max(other.max.y),
                self.max.z.max(other.max.z),
            ),
        }
    }

    fn centroid_component(self, axis: usize) -> f64 {
        match axis {
            0 => (self.min.x + self.max.x) * 0.5,
            1 => (self.min.y + self.max.y) * 0.5,
            _ => (self.min.z + self.max.z) * 0.5,
        }
    }

    fn longest_axis(self) -> usize {
        let extents = [
            self.max.x - self.min.x,
            self.max.y - self.min.y,
            self.max.z - self.min.z,
        ];
        if extents[1] > extents[0] && extents[1] >= extents[2] {
            1
        } else if extents[2] > extents[0] && extents[2] > extents[1] {
            2
        } else {
            0
        }
    }

    fn intersects_ray(self, ray: Ray) -> bool {
        let origins = [ray.origin.x, ray.origin.y, ray.origin.z];
        let directions = [ray.direction.x, ray.direction.y, ray.direction.z];
        let minimum = [self.min.x, self.min.y, self.min.z];
        let maximum = [self.max.x, self.max.y, self.max.z];
        let mut near = f64::NEG_INFINITY;
        let mut far = f64::INFINITY;
        for axis in 0..3 {
            if directions[axis].abs() <= RAY_EPSILON {
                if origins[axis] < minimum[axis] || origins[axis] > maximum[axis] {
                    return false;
                }
                continue;
            }
            let first = (minimum[axis] - origins[axis]) / directions[axis];
            let second = (maximum[axis] - origins[axis]) / directions[axis];
            near = near.max(first.min(second));
            far = far.min(first.max(second));
            if far < near {
                return false;
            }
        }
        far >= 0.0
    }

    fn intersects_bounds(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }
}

#[derive(Clone, Copy, Debug)]
struct SpatialItem {
    source_index: usize,
    bounds: SpatialBounds,
}

#[derive(Clone, Debug)]
enum SpatialNode {
    Leaf {
        bounds: SpatialBounds,
        items: Vec<SpatialItem>,
    },
    Branch {
        bounds: SpatialBounds,
        left: Box<SpatialNode>,
        right: Box<SpatialNode>,
    },
}

impl SpatialNode {
    fn bounds(&self) -> SpatialBounds {
        match self {
            Self::Leaf { bounds, .. } | Self::Branch { bounds, .. } => *bounds,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SpatialIndex {
    root: Option<Box<SpatialNode>>,
    item_count: usize,
}

impl SpatialIndex {
    pub(crate) fn build(bounds: impl IntoIterator<Item = (usize, SpatialBounds)>) -> Self {
        let mut items = bounds
            .into_iter()
            .map(|(source_index, bounds)| SpatialItem {
                source_index,
                bounds,
            })
            .collect::<Vec<_>>();
        let item_count = items.len();
        let root = (!items.is_empty()).then(|| Box::new(build_node(&mut items)));
        Self { root, item_count }
    }

    pub(crate) fn query_ray(&self, ray: Ray) -> (Vec<usize>, SpatialQueryStats) {
        let mut candidates = Vec::new();
        let mut bounds_tested = 0;
        if let Some(root) = &self.root {
            query_ray_node(root, ray, &mut candidates, &mut bounds_tested);
        }
        candidates.sort_unstable();
        candidates.dedup();
        let stats = SpatialQueryStats {
            indexed_items: self.item_count,
            bounds_tested,
            candidate_count: candidates.len(),
        };
        (candidates, stats)
    }

    pub(crate) fn query_bounds(&self, bounds: SpatialBounds) -> Vec<usize> {
        let mut candidates = Vec::new();
        if let Some(root) = &self.root {
            query_bounds_node(root, bounds, &mut candidates);
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

fn build_node(items: &mut [SpatialItem]) -> SpatialNode {
    let bounds = items
        .iter()
        .skip(1)
        .fold(items[0].bounds, |combined, item| {
            combined.union(item.bounds)
        });
    if items.len() <= LEAF_CAPACITY {
        let mut leaf_items = items.to_vec();
        leaf_items.sort_by_key(|item| item.source_index);
        return SpatialNode::Leaf {
            bounds,
            items: leaf_items,
        };
    }
    let axis = bounds.longest_axis();
    items.sort_by(|left, right| {
        left.bounds
            .centroid_component(axis)
            .total_cmp(&right.bounds.centroid_component(axis))
            .then_with(|| left.source_index.cmp(&right.source_index))
    });
    let middle = items.len() / 2;
    let (left, right) = items.split_at_mut(middle);
    SpatialNode::Branch {
        bounds,
        left: Box::new(build_node(left)),
        right: Box::new(build_node(right)),
    }
}

fn query_ray_node(
    node: &SpatialNode,
    ray: Ray,
    candidates: &mut Vec<usize>,
    bounds_tested: &mut usize,
) {
    *bounds_tested += 1;
    if !node.bounds().intersects_ray(ray) {
        return;
    }
    match node {
        SpatialNode::Leaf { items, .. } => {
            for item in items {
                *bounds_tested += 1;
                if item.bounds.intersects_ray(ray) {
                    candidates.push(item.source_index);
                }
            }
        }
        SpatialNode::Branch { left, right, .. } => {
            query_ray_node(left, ray, candidates, bounds_tested);
            query_ray_node(right, ray, candidates, bounds_tested);
        }
    }
}

fn query_bounds_node(node: &SpatialNode, query: SpatialBounds, candidates: &mut Vec<usize>) {
    if !node.bounds().intersects_bounds(query) {
        return;
    }
    match node {
        SpatialNode::Leaf { items, .. } => candidates.extend(
            items
                .iter()
                .filter(|item| item.bounds.intersects_bounds(query))
                .map(|item| item.source_index),
        ),
        SpatialNode::Branch { left, right, .. } => {
            query_bounds_node(left, query, candidates);
            query_bounds_node(right, query, candidates);
        }
    }
}

pub(crate) fn transformed_bounds(transform: Transform, bounds: [[f64; 3]; 2]) -> SpatialBounds {
    let corners = [
        [bounds[0][0], bounds[0][1], bounds[0][2]],
        [bounds[1][0], bounds[0][1], bounds[0][2]],
        [bounds[0][0], bounds[1][1], bounds[0][2]],
        [bounds[1][0], bounds[1][1], bounds[0][2]],
        [bounds[0][0], bounds[0][1], bounds[1][2]],
        [bounds[1][0], bounds[0][1], bounds[1][2]],
        [bounds[0][0], bounds[1][1], bounds[1][2]],
        [bounds[1][0], bounds[1][1], bounds[1][2]],
    ]
    .map(|point| transform_point(transform, Vec3::new(point[0], point[1], point[2])));
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for point in corners {
        min = Vec3::new(min.x.min(point.x), min.y.min(point.y), min.z.min(point.z));
        max = Vec3::new(max.x.max(point.x), max.y.max(point.y), max.z.max(point.z));
    }
    SpatialBounds::from_min_max(min, max)
}

pub(crate) fn transform_point(transform: Transform, point: Vec3) -> Vec3 {
    let matrix = transform.matrix();
    Vec3::new(
        matrix[0] * point.x + matrix[1] * point.y + matrix[2] * point.z + matrix[3],
        matrix[4] * point.x + matrix[5] * point.y + matrix[6] * point.z + matrix[7],
        matrix[8] * point.x + matrix[9] * point.y + matrix[10] * point.z + matrix[11],
    )
}

pub(crate) fn ray_triangle_distance(
    ray: Ray,
    first: Vec3,
    second: Vec3,
    third: Vec3,
) -> Option<f64> {
    let first_edge = second - first;
    let second_edge = third - first;
    let determinant_vector = cross(ray.direction, second_edge);
    let determinant = dot(first_edge, determinant_vector);
    if determinant.abs() <= RAY_EPSILON {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    let origin_offset = ray.origin - first;
    let first_weight = dot(origin_offset, determinant_vector) * inverse_determinant;
    if !(-RAY_EPSILON..=1.0 + RAY_EPSILON).contains(&first_weight) {
        return None;
    }
    let weight_vector = cross(origin_offset, first_edge);
    let second_weight = dot(ray.direction, weight_vector) * inverse_determinant;
    if second_weight < -RAY_EPSILON || first_weight + second_weight > 1.0 + RAY_EPSILON {
        return None;
    }
    let distance = dot(second_edge, weight_vector) * inverse_determinant;
    (distance >= 0.0 && distance.is_finite()).then_some(distance)
}

fn dot(left: Vec3, right: Vec3) -> f64 {
    left.x * right.x + left.y * right.y + left.z * right.z
}

pub(crate) fn cross(left: Vec3, right: Vec3) -> Vec3 {
    Vec3::new(
        left.y * right.z - left.z * right.y,
        left.z * right.x - left.x * right.z,
        left.x * right.y - left.y * right.x,
    )
}
