use crate::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    AssemblyMotionDriver, AssemblyMotionStudy, AssemblyMotionStudyId,
};
use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, Transform,
};

pub const HETTICH_EXAMPLE_BOTTOM: OccurrenceId = OccurrenceId(1);
pub const HETTICH_EXAMPLE_LEFT_SIDE: OccurrenceId = OccurrenceId(2);
pub const HETTICH_EXAMPLE_RIGHT_SIDE: OccurrenceId = OccurrenceId(3);
pub const HETTICH_EXAMPLE_BACK: OccurrenceId = OccurrenceId(4);
pub const HETTICH_EXAMPLE_DRAWER: OccurrenceId = OccurrenceId(5);
pub const HETTICH_EXAMPLE_LEFT_RUNNER: OccurrenceId = OccurrenceId(6);
pub const HETTICH_EXAMPLE_RIGHT_RUNNER: OccurrenceId = OccurrenceId(7);
pub const HETTICH_EXAMPLE_DRAWER_LEFT_SIDE: OccurrenceId = OccurrenceId(8);
pub const HETTICH_EXAMPLE_DRAWER_RIGHT_SIDE: OccurrenceId = OccurrenceId(9);
pub const HETTICH_EXAMPLE_DRAWER_BACK: OccurrenceId = OccurrenceId(10);
pub const HETTICH_EXAMPLE_DRAWER_FRONT: OccurrenceId = OccurrenceId(11);
pub const HETTICH_EXAMPLE_DRAWER_JOINT: AssemblyJointId = AssemblyJointId(104);
pub const HETTICH_EXAMPLE_MOTION_STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(200);

const HETTICH_DRAWER_SIDE_BOTTOM_Z_MM: f64 = 144.241_910_702_799_1;
const HETTICH_DRAWER_BOTTOM_UNDERSIDE_Z_MM: f64 = HETTICH_DRAWER_SIDE_BOTTOM_Z_MM + 12.0;
const HETTICH_DRAWER_BOTTOM_TOP_Z_MM: f64 = HETTICH_DRAWER_BOTTOM_UNDERSIDE_Z_MM + 13.0;

fn append_box(
    commands: &mut Vec<CanonicalCommand>,
    definition_id: DefinitionId,
    profile_id: FeatureId,
    extrusion_id: FeatureId,
    occurrence_id: OccurrenceId,
    name: &str,
    geometry: ([f64; 3], [f64; 3]),
) {
    let (size_mm, position_mm) = geometry;
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: profile_id,
            definition_id,
            name: format!("{name} profile"),
            kind: FeatureKind::Profile {
                points_mm: vec![
                    [0.0, 0.0],
                    [size_mm[0], 0.0],
                    [size_mm[0], size_mm[1]],
                    [0.0, size_mm[1]],
                ],
            },
        },
        CanonicalCommand::CreateFeature {
            id: extrusion_id,
            definition_id,
            name: format!("{name} body"),
            kind: FeatureKind::Extrusion {
                profile: profile_id,
                height: Dimension::from_decimal(size_mm[2].to_string()).unwrap(),
            },
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: name.to_owned(),
            transform: Transform::from_translation(position_mm[0], position_mm[1], position_mm[2])
                .unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
    ]);
}

#[must_use]
pub fn hettich_quadro_v6_drawer_example() -> DocumentStore {
    let mut commands = Vec::new();
    append_box(
        &mut commands,
        DefinitionId(1),
        FeatureId(1),
        FeatureId(2),
        HETTICH_EXAMPLE_BOTTOM,
        "Cabinet bottom",
        ([600.0, 562.0, 18.0], [0.0, 38.0, 0.0]),
    );
    append_box(
        &mut commands,
        DefinitionId(2),
        FeatureId(3),
        FeatureId(4),
        HETTICH_EXAMPLE_LEFT_SIDE,
        "Cabinet left side",
        ([600.0, 18.0, 350.0], [0.0, 20.0, 0.0]),
    );
    append_box(
        &mut commands,
        DefinitionId(3),
        FeatureId(5),
        FeatureId(6),
        HETTICH_EXAMPLE_RIGHT_SIDE,
        "Cabinet right side",
        ([600.0, 18.0, 350.0], [0.0, 600.0, 0.0]),
    );
    append_box(
        &mut commands,
        DefinitionId(4),
        FeatureId(7),
        FeatureId(8),
        HETTICH_EXAMPLE_BACK,
        "Cabinet back",
        ([18.0, 562.0, 332.0], [582.0, 38.0, 18.0]),
    );
    append_box(
        &mut commands,
        DefinitionId(5),
        FeatureId(9),
        FeatureId(10),
        HETTICH_EXAMPLE_DRAWER,
        "Drawer bottom 490 mm",
        (
            [450.0, 490.0, 13.0],
            [-450.0, 74.0, HETTICH_DRAWER_BOTTOM_UNDERSIDE_Z_MM],
        ),
    );
    append_box(
        &mut commands,
        DefinitionId(8),
        FeatureId(15),
        FeatureId(16),
        HETTICH_EXAMPLE_DRAWER_LEFT_SIDE,
        "Drawer left side",
        (
            [450.0, 16.0, 130.0],
            [-450.0, 58.0, HETTICH_DRAWER_SIDE_BOTTOM_Z_MM],
        ),
    );
    append_box(
        &mut commands,
        DefinitionId(9),
        FeatureId(17),
        FeatureId(18),
        HETTICH_EXAMPLE_DRAWER_RIGHT_SIDE,
        "Drawer right side",
        (
            [450.0, 16.0, 130.0],
            [-450.0, 564.0, HETTICH_DRAWER_SIDE_BOTTOM_Z_MM],
        ),
    );
    append_box(
        &mut commands,
        DefinitionId(10),
        FeatureId(19),
        FeatureId(20),
        HETTICH_EXAMPLE_DRAWER_BACK,
        "Drawer back",
        (
            [16.0, 490.0, 130.0],
            [-16.0, 74.0, HETTICH_DRAWER_BOTTOM_TOP_Z_MM],
        ),
    );
    append_box(
        &mut commands,
        DefinitionId(11),
        FeatureId(21),
        FeatureId(22),
        HETTICH_EXAMPLE_DRAWER_FRONT,
        "Drawer front",
        ([18.0, 596.0, 200.0], [-468.0, 20.0, 120.0]),
    );
    append_box(
        &mut commands,
        DefinitionId(6),
        FeatureId(11),
        FeatureId(12),
        HETTICH_EXAMPLE_LEFT_RUNNER,
        "Hettich Quadro left runner envelope",
        ([450.0, 20.0, 45.0], [-450.0, 38.0, 100.0]),
    );
    append_box(
        &mut commands,
        DefinitionId(7),
        FeatureId(13),
        FeatureId(14),
        HETTICH_EXAMPLE_RIGHT_RUNNER,
        "Hettich Quadro right runner envelope",
        ([450.0, 20.0, 45.0], [-450.0, 580.0, 100.0]),
    );

    let axis_x = AssemblyJointAxis::new([-1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    commands.extend([
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(100),
            HETTICH_EXAMPLE_BOTTOM,
            HETTICH_EXAMPLE_LEFT_SIDE,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(101),
            HETTICH_EXAMPLE_BOTTOM,
            HETTICH_EXAMPLE_RIGHT_SIDE,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(102),
            HETTICH_EXAMPLE_BOTTOM,
            HETTICH_EXAMPLE_BACK,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            HETTICH_EXAMPLE_DRAWER_JOINT,
            HETTICH_EXAMPLE_BOTTOM,
            HETTICH_EXAMPLE_DRAWER,
            AssemblyJointKind::Prismatic {
                axis: axis_x,
                limits: Some(AssemblyJointLimits::new(0.0, 450.0)),
                position_mm: 450.0,
            },
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(105),
            HETTICH_EXAMPLE_DRAWER,
            HETTICH_EXAMPLE_LEFT_RUNNER,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(107),
            HETTICH_EXAMPLE_DRAWER,
            HETTICH_EXAMPLE_DRAWER_LEFT_SIDE,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(108),
            HETTICH_EXAMPLE_DRAWER,
            HETTICH_EXAMPLE_DRAWER_RIGHT_SIDE,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(109),
            HETTICH_EXAMPLE_DRAWER,
            HETTICH_EXAMPLE_DRAWER_BACK,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(110),
            HETTICH_EXAMPLE_DRAWER,
            HETTICH_EXAMPLE_DRAWER_FRONT,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(106),
            HETTICH_EXAMPLE_DRAWER,
            HETTICH_EXAMPLE_RIGHT_RUNNER,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
            HETTICH_EXAMPLE_MOTION_STUDY,
            "Close Hettich Quadro V6 drawer",
            vec![AssemblyMotionDriver::new(HETTICH_EXAMPLE_DRAWER_JOINT, 0.0)],
        )),
    ]);

    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly_joint::{
        AssemblyMotionCollisionBody, preview_assembly_motion_study_clearance,
    };
    use crate::linear_hardware::{
        HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450, LinearRunnerInstallation,
        LinearRunnerInstallationIssue, validate_linear_runner_installation,
    };
    use crate::persistence;
    use crate::prismatic::Aabb;

    #[test]
    fn reference_document_is_canonical_persistent_and_contains_the_dynamic_assembly() {
        let document = hettich_quadro_v6_drawer_example();
        let snapshot = document.current();
        assert_eq!(snapshot.definitions().count(), 11);
        assert_eq!(snapshot.features().count(), 22);
        assert_eq!(snapshot.occurrences().count(), 11);
        assert_eq!(snapshot.assembly_joints().count(), 10);
        assert_eq!(snapshot.assembly_motion_studies().count(), 1);
        assert_eq!(
            snapshot
                .assembly_joint(HETTICH_EXAMPLE_DRAWER_JOINT)
                .unwrap()
                .kind()
                .limits()
                .unwrap(),
            AssemblyJointLimits::new(0.0, 450.0)
        );

        let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
        assert_eq!(
            reopened.snapshot().canonical_digest(),
            snapshot.canonical_digest()
        );
    }

    #[test]
    fn reference_drawer_has_valid_fit_and_moves_with_clearance_over_the_full_travel() {
        let document = hettich_quadro_v6_drawer_example();
        let installation = LinearRunnerInstallation {
            cabinet_inner_width_mm: 562.0,
            cabinet_depth_mm: 600.0,
            drawer_outer_width_mm: 522.0,
            drawer_side_thickness_mm: 16.0,
            left_runner_length_mm: 450.0,
            right_runner_length_mm: 450.0,
            left_runner_height_mm: 100.0,
            right_runner_height_mm: 100.0,
            requested_travel_mm: 450.0,
        };
        assert!(
            validate_linear_runner_installation(
                HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450,
                installation,
            )
            .unwrap()
            .is_valid()
        );

        let drawer_bounds =
            Aabb::bounded_volume([0.0, -16.0, -12.0], [450.0, 506.0, 118.0]).unwrap();
        let cabinet_side_bounds =
            Aabb::bounded_volume([0.0, 0.0, 0.0], [600.0, 18.0, 350.0]).unwrap();
        let preview = preview_assembly_motion_study_clearance(
            &document.current(),
            HETTICH_EXAMPLE_MOTION_STUDY,
            10,
            &[
                AssemblyMotionCollisionBody::new(HETTICH_EXAMPLE_DRAWER, drawer_bounds),
                AssemblyMotionCollisionBody::new(HETTICH_EXAMPLE_LEFT_SIDE, cabinet_side_bounds),
                AssemblyMotionCollisionBody::new(HETTICH_EXAMPLE_RIGHT_SIDE, cabinet_side_bounds),
            ],
            0.0,
        )
        .unwrap();
        assert_eq!(preview.path().samples().len(), 11);
        assert_eq!(preview.clearance().minimum_clearance_mm(), 20.0);
        assert!(preview.clearance().first_contact().is_none());
        let endpoint = preview.path().samples().last().unwrap().solution();
        assert_eq!(
            endpoint
                .pose(HETTICH_EXAMPLE_DRAWER)
                .unwrap()
                .world_transform(),
            Transform::from_translation(0.0, 74.0, HETTICH_DRAWER_BOTTOM_UNDERSIDE_Z_MM).unwrap()
        );
        assert_eq!(
            endpoint
                .pose(HETTICH_EXAMPLE_LEFT_RUNNER)
                .unwrap()
                .world_transform(),
            Transform::from_translation(0.0, 38.0, 100.0).unwrap()
        );
        assert_eq!(
            endpoint
                .pose(HETTICH_EXAMPLE_RIGHT_RUNNER)
                .unwrap()
                .world_transform(),
            Transform::from_translation(0.0, 580.0, 100.0).unwrap()
        );

        let invalid = validate_linear_runner_installation(
            HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450,
            LinearRunnerInstallation {
                cabinet_depth_mm: 450.0,
                drawer_outer_width_mm: 565.0,
                right_runner_height_mm: 103.0,
                requested_travel_mm: 460.0,
                ..installation
            },
        )
        .unwrap();
        assert_eq!(invalid.issues().len(), 4);
        assert!(matches!(
            invalid.issues()[1],
            LinearRunnerInstallationIssue::DrawerWidthMismatch { .. }
        ));
    }
}
