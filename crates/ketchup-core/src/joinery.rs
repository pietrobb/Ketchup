use crate::document::{InstancePath, Snapshot, Transform};
use std::fmt;

pub const DOWEL_JOINERY_PROJECTION_V1: &str = "ketchup.dowel-joinery-projection.v1";
const GEOMETRY_TOLERANCE: f64 = 1.0e-8;
const MAX_DOWELS_PER_JOINT: u32 = 128;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DowelJointId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct DowelJointFace {
    pub instance_path: InstancePath,
    pub face_origin_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
    pub bounds_min_local_mm: [f64; 3],
    pub bounds_max_local_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DowelJointContract {
    pub id: DowelJointId,
    pub name: String,
    pub first: DowelJointFace,
    pub second: DowelJointFace,
    pub first_center_local_mm: [f64; 3],
    pub row_unit_first_local: [f64; 3],
    pub count: u32,
    pub spacing_mm: f64,
    pub dowel: DowelSpec,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StandardDowel {
    D6x30,
    D8x30,
    D8x40,
    D10x40,
}

impl StandardDowel {
    #[must_use]
    pub const fn dimensions_mm(self) -> (f64, f64) {
        match self {
            Self::D6x30 => (6.0, 30.0),
            Self::D8x30 => (8.0, 30.0),
            Self::D8x40 => (8.0, 40.0),
            Self::D10x40 => (10.0, 40.0),
        }
    }

    #[must_use]
    pub fn symmetric_spec(self) -> DowelSpec {
        let (diameter_mm, length_mm) = self.dimensions_mm();
        DowelSpec {
            diameter_mm,
            length_mm,
            first_insertion_mm: length_mm / 2.0,
            second_insertion_mm: length_mm / 2.0,
            bottom_clearance_mm: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DowelSpec {
    pub diameter_mm: f64,
    pub length_mm: f64,
    pub first_insertion_mm: f64,
    pub second_insertion_mm: f64,
    pub bottom_clearance_mm: f64,
}

impl DowelSpec {
    fn validate(self) -> Result<(), DowelJointError> {
        if [
            self.diameter_mm,
            self.length_mm,
            self.first_insertion_mm,
            self.second_insertion_mm,
            self.bottom_clearance_mm,
        ]
        .into_iter()
        .any(|value| !value.is_finite())
            || self.diameter_mm <= 0.0
            || self.length_mm <= 0.0
            || self.first_insertion_mm <= 0.0
            || self.second_insertion_mm <= 0.0
            || self.bottom_clearance_mm < 0.0
            || (self.first_insertion_mm + self.second_insertion_mm - self.length_mm).abs()
                > GEOMETRY_TOLERANCE
        {
            return Err(DowelJointError::InvalidDowel);
        }
        Ok(())
    }

    #[must_use]
    pub fn first_hole_depth_mm(self) -> f64 {
        self.first_insertion_mm + self.bottom_clearance_mm
    }

    #[must_use]
    pub fn second_hole_depth_mm(self) -> f64 {
        self.second_insertion_mm + self.bottom_clearance_mm
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DowelJointSide {
    pub instance_path: InstancePath,
    pub world_from_local: Transform,
    pub face_origin_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
    pub bounds_min_local_mm: [f64; 3],
    pub bounds_max_local_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DowelJointRequest {
    pub stable_joint_id: String,
    pub first: DowelJointSide,
    pub second: DowelJointSide,
    pub first_center_world_mm: [f64; 3],
    pub row_unit_world: [f64; 3],
    pub count: u32,
    pub spacing_mm: f64,
    pub dowel: DowelSpec,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DowelHole {
    pub stable_hole_id: String,
    pub instance_path: InstancePath,
    pub entry_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
    pub diameter_mm: f64,
    pub depth_mm: f64,
    pub shared_center_world_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DowelPair {
    pub index: u32,
    pub first: DowelHole,
    pub second: DowelHole,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DowelJointProjection {
    pub schema: &'static str,
    pub stable_joint_id: String,
    pub pairs: Vec<DowelPair>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DowelJointError {
    InvalidJointId,
    SameParticipant,
    InvalidDowel,
    InvalidRow,
    NonRigidTransform,
    InvalidParticipantGeometry,
    FacesDoNotMate,
    HoleOutsidePart,
}

impl fmt::Display for DowelJointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidJointId => "dowel joint ID is invalid",
            Self::SameParticipant => "dowel joint requires two different part instances",
            Self::InvalidDowel => "dowel dimensions or insertion depths are invalid",
            Self::InvalidRow => "dowel row direction, count, or spacing is invalid",
            Self::NonRigidTransform => "dowel participants require rigid instance transforms",
            Self::InvalidParticipantGeometry => "dowel participant face or bounds are invalid",
            Self::FacesDoNotMate => "dowel participant faces are not coincident and opposed",
            Self::HoleOutsidePart => "a derived dowel hole leaves its host part",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for DowelJointError {}

pub fn project_dowel_joint_contract(
    snapshot: &Snapshot,
    contract: &DowelJointContract,
) -> Result<DowelJointProjection, DowelJointError> {
    if contract.id.0 == 0
        || contract.name.trim().is_empty()
        || contract.name.len() > 128
        || !contract
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'-' | b'_' | b'/'))
    {
        return Err(DowelJointError::InvalidJointId);
    }
    let first_resolved = snapshot
        .resolve_instance_path(&contract.first.instance_path)
        .map_err(|_| DowelJointError::InvalidParticipantGeometry)?;
    let second_resolved = snapshot
        .resolve_instance_path(&contract.second.instance_path)
        .map_err(|_| DowelJointError::InvalidParticipantGeometry)?;
    let first_center_world_mm = transform_point(
        first_resolved.world_transform,
        contract.first_center_local_mm,
    );
    let row_unit_world = transform_vector(
        first_resolved.world_transform,
        contract.row_unit_first_local,
    );
    project_dowel_joint(&DowelJointRequest {
        stable_joint_id: format!("dowel-{:016x}", contract.id.0),
        first: DowelJointSide {
            instance_path: contract.first.instance_path.clone(),
            world_from_local: first_resolved.world_transform,
            face_origin_local_mm: contract.first.face_origin_local_mm,
            inward_unit_local: contract.first.inward_unit_local,
            bounds_min_local_mm: contract.first.bounds_min_local_mm,
            bounds_max_local_mm: contract.first.bounds_max_local_mm,
        },
        second: DowelJointSide {
            instance_path: contract.second.instance_path.clone(),
            world_from_local: second_resolved.world_transform,
            face_origin_local_mm: contract.second.face_origin_local_mm,
            inward_unit_local: contract.second.inward_unit_local,
            bounds_min_local_mm: contract.second.bounds_min_local_mm,
            bounds_max_local_mm: contract.second.bounds_max_local_mm,
        },
        first_center_world_mm,
        row_unit_world,
        count: contract.count,
        spacing_mm: contract.spacing_mm,
        dowel: contract.dowel,
    })
}

pub fn project_dowel_joint(
    request: &DowelJointRequest,
) -> Result<DowelJointProjection, DowelJointError> {
    if request.stable_joint_id.trim().is_empty()
        || request.stable_joint_id.len() > 128
        || !request
            .stable_joint_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/'))
    {
        return Err(DowelJointError::InvalidJointId);
    }
    if request.first.instance_path == request.second.instance_path {
        return Err(DowelJointError::SameParticipant);
    }
    request.dowel.validate()?;
    if request.count == 0
        || request.count > MAX_DOWELS_PER_JOINT
        || !request.spacing_mm.is_finite()
        || request.spacing_mm < 0.0
        || (request.count > 1 && request.spacing_mm < request.dowel.diameter_mm)
        || request
            .first_center_world_mm
            .iter()
            .any(|value| !value.is_finite())
        || !is_unit(request.row_unit_world)
    {
        return Err(DowelJointError::InvalidRow);
    }

    let first_inverse = validate_side(&request.first)?;
    let second_inverse = validate_side(&request.second)?;
    let first_inward_world = transform_vector(
        request.first.world_from_local,
        request.first.inward_unit_local,
    );
    let second_inward_world = transform_vector(
        request.second.world_from_local,
        request.second.inward_unit_local,
    );
    if !is_unit(first_inward_world)
        || !is_unit(second_inward_world)
        || dot(first_inward_world, second_inward_world) > -1.0 + GEOMETRY_TOLERANCE
        || dot(first_inward_world, request.row_unit_world).abs() > GEOMETRY_TOLERANCE
        || dot(second_inward_world, request.row_unit_world).abs() > GEOMETRY_TOLERANCE
    {
        return Err(DowelJointError::FacesDoNotMate);
    }
    let first_face_world = transform_point(
        request.first.world_from_local,
        request.first.face_origin_local_mm,
    );
    let second_face_world = transform_point(
        request.second.world_from_local,
        request.second.face_origin_local_mm,
    );
    if distance_along(
        first_face_world,
        request.first_center_world_mm,
        first_inward_world,
    )
    .abs()
        > GEOMETRY_TOLERANCE
        || distance_along(
            second_face_world,
            request.first_center_world_mm,
            second_inward_world,
        )
        .abs()
            > GEOMETRY_TOLERANCE
        || distance_along(first_face_world, second_face_world, first_inward_world).abs()
            > GEOMETRY_TOLERANCE
    {
        return Err(DowelJointError::FacesDoNotMate);
    }

    let mut pairs = Vec::with_capacity(request.count as usize);
    for index in 0..request.count {
        let center_world_mm = add(
            request.first_center_world_mm,
            scale(
                request.row_unit_world,
                request.spacing_mm * f64::from(index),
            ),
        );
        let first = derive_hole(
            &request.stable_joint_id,
            index,
            "first",
            &request.first,
            first_inverse,
            center_world_mm,
            request.dowel.diameter_mm,
            request.dowel.first_hole_depth_mm(),
        )?;
        let second = derive_hole(
            &request.stable_joint_id,
            index,
            "second",
            &request.second,
            second_inverse,
            center_world_mm,
            request.dowel.diameter_mm,
            request.dowel.second_hole_depth_mm(),
        )?;
        pairs.push(DowelPair {
            index,
            first,
            second,
        });
    }
    Ok(DowelJointProjection {
        schema: DOWEL_JOINERY_PROJECTION_V1,
        stable_joint_id: request.stable_joint_id.clone(),
        pairs,
    })
}

fn validate_side(side: &DowelJointSide) -> Result<Transform, DowelJointError> {
    let inverse = side
        .world_from_local
        .rigid_inverse()
        .ok_or(DowelJointError::NonRigidTransform)?;
    if side
        .face_origin_local_mm
        .iter()
        .chain(&side.inward_unit_local)
        .chain(&side.bounds_min_local_mm)
        .chain(&side.bounds_max_local_mm)
        .any(|value| !value.is_finite())
        || !is_unit(side.inward_unit_local)
        || (0..3).any(|axis| {
            side.bounds_min_local_mm[axis] >= side.bounds_max_local_mm[axis]
                || side.face_origin_local_mm[axis]
                    < side.bounds_min_local_mm[axis] - GEOMETRY_TOLERANCE
                || side.face_origin_local_mm[axis]
                    > side.bounds_max_local_mm[axis] + GEOMETRY_TOLERANCE
        })
    {
        return Err(DowelJointError::InvalidParticipantGeometry);
    }
    Ok(inverse)
}

#[allow(clippy::too_many_arguments)]
fn derive_hole(
    joint_id: &str,
    index: u32,
    side_name: &str,
    side: &DowelJointSide,
    local_from_world: Transform,
    center_world_mm: [f64; 3],
    diameter_mm: f64,
    depth_mm: f64,
) -> Result<DowelHole, DowelJointError> {
    let entry_local_mm = transform_point(local_from_world, center_world_mm);
    let end_local_mm = add(entry_local_mm, scale(side.inward_unit_local, depth_mm));
    let radius = diameter_mm / 2.0;
    for point in [entry_local_mm, end_local_mm] {
        for (axis, coordinate) in point.into_iter().enumerate() {
            let direction_component = side.inward_unit_local[axis].abs();
            let margin = radius * (1.0 - direction_component).sqrt();
            if coordinate < side.bounds_min_local_mm[axis] + margin - GEOMETRY_TOLERANCE
                || coordinate > side.bounds_max_local_mm[axis] - margin + GEOMETRY_TOLERANCE
            {
                return Err(DowelJointError::HoleOutsidePart);
            }
        }
    }
    Ok(DowelHole {
        stable_hole_id: format!("{joint_id}/{index}/{side_name}"),
        instance_path: side.instance_path.clone(),
        entry_local_mm,
        inward_unit_local: side.inward_unit_local,
        diameter_mm,
        depth_mm,
        shared_center_world_mm: center_world_mm,
    })
}

fn transform_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

fn transform_vector(transform: Transform, vector: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * vector[0] + matrix[1] * vector[1] + matrix[2] * vector[2],
        matrix[4] * vector[0] + matrix[5] * vector[1] + matrix[6] * vector[2],
        matrix[8] * vector[0] + matrix[9] * vector[1] + matrix[10] * vector[2],
    ]
}

fn is_unit(vector: [f64; 3]) -> bool {
    vector.into_iter().all(f64::is_finite)
        && (dot(vector, vector) - 1.0).abs() <= GEOMETRY_TOLERANCE
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] + right[axis])
}

fn scale(vector: [f64; 3], factor: f64) -> [f64; 3] {
    vector.map(|value| value * factor)
}

fn distance_along(origin: [f64; 3], point: [f64; 3], direction: [f64; 3]) -> f64 {
    dot(
        std::array::from_fn(|axis| point[axis] - origin[axis]),
        direction,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, OccurrenceId,
    };
    use crate::persistence;

    fn side(
        occurrence: u64,
        inward_unit_local: [f64; 3],
        bounds_min_local_mm: [f64; 3],
        bounds_max_local_mm: [f64; 3],
    ) -> DowelJointSide {
        DowelJointSide {
            instance_path: InstancePath::root(OccurrenceId(occurrence)),
            world_from_local: Transform::identity(),
            face_origin_local_mm: [0.0, 0.0, 0.0],
            inward_unit_local,
            bounds_min_local_mm,
            bounds_max_local_mm,
        }
    }

    fn valid_request() -> DowelJointRequest {
        DowelJointRequest {
            stable_joint_id: "cabinet/left-row".to_owned(),
            first: side(1, [0.0, 0.0, -1.0], [0.0, 0.0, -19.0], [600.0, 400.0, 0.0]),
            second: side(2, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0], [600.0, 400.0, 19.0]),
            first_center_world_mm: [50.0, 20.0, 0.0],
            row_unit_world: [1.0, 0.0, 0.0],
            count: 3,
            spacing_mm: 32.0,
            dowel: StandardDowel::D8x30.symmetric_spec(),
        }
    }

    #[test]
    fn one_joint_derives_coaxial_holes_for_both_parts() {
        let projection = project_dowel_joint(&valid_request()).unwrap();
        assert_eq!(projection.schema, DOWEL_JOINERY_PROJECTION_V1);
        assert_eq!(projection.pairs.len(), 3);
        for (expected_index, pair) in projection.pairs.iter().enumerate() {
            assert_eq!(pair.index, expected_index as u32);
            assert_eq!(
                pair.first.shared_center_world_mm,
                pair.second.shared_center_world_mm
            );
            assert_eq!(
                pair.first.entry_local_mm,
                [50.0 + 32.0 * expected_index as f64, 20.0, 0.0]
            );
            assert_eq!(pair.second.entry_local_mm, pair.first.entry_local_mm);
            assert_eq!(pair.first.inward_unit_local, [0.0, 0.0, -1.0]);
            assert_eq!(pair.second.inward_unit_local, [0.0, 0.0, 1.0]);
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
        }
    }

    #[test]
    fn instance_transform_recomputes_local_hole_without_breaking_shared_center() {
        let mut request = valid_request();
        request.second.world_from_local = Transform::from_translation(10.0, 0.0, 0.0).unwrap();
        request.second.face_origin_local_mm = [-10.0, 0.0, 0.0];
        request.second.bounds_min_local_mm = [-10.0, 0.0, 0.0];
        request.second.bounds_max_local_mm = [590.0, 400.0, 19.0];

        let projection = project_dowel_joint(&request).unwrap();
        assert_eq!(projection.pairs[0].first.entry_local_mm, [50.0, 20.0, 0.0]);
        assert_eq!(projection.pairs[0].second.entry_local_mm, [40.0, 20.0, 0.0]);
        assert_eq!(
            projection.pairs[0].first.shared_center_world_mm,
            projection.pairs[0].second.shared_center_world_mm
        );
    }

    #[test]
    fn invalid_mating_or_out_of_bounds_rows_fail_closed() {
        let mut not_opposed = valid_request();
        not_opposed.second.inward_unit_local = [0.0, 0.0, -1.0];
        assert_eq!(
            project_dowel_joint(&not_opposed),
            Err(DowelJointError::FacesDoNotMate)
        );

        let mut outside = valid_request();
        outside.first_center_world_mm = [2.0, 20.0, 0.0];
        assert_eq!(
            project_dowel_joint(&outside),
            Err(DowelJointError::HoleOutsidePart)
        );
    }

    #[test]
    fn standard_presets_keep_dowel_length_and_hole_clearance_explicit() {
        for preset in [
            StandardDowel::D6x30,
            StandardDowel::D8x30,
            StandardDowel::D8x40,
            StandardDowel::D10x40,
        ] {
            let spec = preset.symmetric_spec();
            spec.validate().unwrap();
            assert_eq!(
                spec.first_insertion_mm + spec.second_insertion_mm,
                spec.length_mm
            );
            assert!(spec.first_hole_depth_mm() > spec.first_insertion_mm);
            assert!(spec.second_hole_depth_mm() > spec.second_insertion_mm);
        }
    }

    #[test]
    fn persistent_contract_roundtrips_and_guards_both_participants() {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Panel".to_owned(),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "First".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "Second".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let contract = DowelJointContract {
            id: DowelJointId(7),
            name: "Cabinet left row".to_owned(),
            first: DowelJointFace {
                instance_path: InstancePath::root(OccurrenceId(1)),
                face_origin_local_mm: [0.0, 0.0, 0.0],
                inward_unit_local: [0.0, 0.0, -1.0],
                bounds_min_local_mm: [0.0, 0.0, -19.0],
                bounds_max_local_mm: [600.0, 400.0, 0.0],
            },
            second: DowelJointFace {
                instance_path: InstancePath::root(OccurrenceId(2)),
                face_origin_local_mm: [0.0, 0.0, 0.0],
                inward_unit_local: [0.0, 0.0, 1.0],
                bounds_min_local_mm: [0.0, 0.0, 0.0],
                bounds_max_local_mm: [600.0, 400.0, 19.0],
            },
            first_center_local_mm: [50.0, 20.0, 0.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 3,
            spacing_mm: 32.0,
            dowel: StandardDowel::D8x30.symmetric_spec(),
        };
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::UpsertDowelJoint(contract.clone()),
            ]))
            .unwrap();
        let snapshot = document.current();
        assert_eq!(
            project_dowel_joint_contract(&snapshot, snapshot.dowel_joint(contract.id).unwrap())
                .unwrap()
                .pairs
                .len(),
            3
        );

        let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
        assert_eq!(
            reopened.snapshot().dowel_joint(contract.id),
            Some(&contract)
        );
        assert_eq!(
            reopened.snapshot().canonical_digest(),
            snapshot.canonical_digest()
        );

        assert_eq!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::DeleteOccurrence {
                        id: OccurrenceId(1),
                    }
                ]))
                .err(),
            Some(CanonicalError::OccurrenceInDowelJoint(OccurrenceId(1)))
        );
        assert_eq!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetOccurrenceTransform {
                        id: OccurrenceId(2),
                        transform: Transform::from_translation(0.0, 0.0, 1.0).unwrap(),
                    },
                ]))
                .err(),
            Some(CanonicalError::DowelJoint(DowelJointError::FacesDoNotMate))
        );
    }
}
