use ketchup_core::assembly_joint::{
    ASSEMBLY_JOINT_SCHEMA_V1, AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind,
    AssemblyJointLimits, AssemblyKinematicPublishError, AssemblyKinematicSolveError,
    AssemblyKinematicSolveStatus, AssemblyMotionClearanceError, AssemblyMotionCollisionBody,
    AssemblyMotionCollisionPair, AssemblyMotionDriver, AssemblyMotionSamplingError,
    AssemblyMotionStudy, AssemblyMotionStudyId, MAX_ASSEMBLY_MOTION_SAMPLE_INTERVALS,
    analyze_assembly_motion_clearance, preview_assembly_motion_study_clearance,
    sample_assembly_motion_study, solve_assembly_joint_kinematics,
    solve_assembly_joint_kinematics_with_drivers, solve_assembly_motion_study,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, GroupId,
    OccurrenceId, ProposalCommitError, ProposalPrepareError, Transform,
};
use ketchup_core::persistence;
use ketchup_core::prismatic::Aabb;
use ketchup_core::state_view::encode_semantic_state;

const DEFINITION: DefinitionId = DefinitionId(1);
const FIRST: OccurrenceId = OccurrenceId(10);
const SECOND: OccurrenceId = OccurrenceId(11);
const THIRD: OccurrenceId = OccurrenceId(12);
const FOURTH: OccurrenceId = OccurrenceId(13);
const FIXED_JOINT: AssemblyJointId = AssemblyJointId(20);
const REVOLUTE_JOINT: AssemblyJointId = AssemblyJointId(21);
const PRISMATIC_JOINT: AssemblyJointId = AssemblyJointId(22);
const STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(40);

#[derive(Debug, Eq, PartialEq)]
struct StoreStamp {
    revision_id: u64,
    digest: String,
    revision_count: usize,
    undo_steps: usize,
    redo_steps: usize,
}

fn store_stamp(document: &DocumentStore) -> StoreStamp {
    StoreStamp {
        revision_id: document.current().revision_id(),
        digest: document.current().canonical_digest(),
        revision_count: document.revision_count(),
        undo_steps: document.visible_undo_steps(),
        redo_steps: document.visible_redo_steps(),
    }
}

fn occurrence(id: OccurrenceId, name: &str, x_mm: f64) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id,
        definition_id: DEFINITION,
        name: name.into(),
        transform: Transform::from_translation(x_mm, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    }
}

fn seeded_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Kinematic part".into(),
            },
            occurrence(FIRST, "Base", 0.0),
            occurrence(SECOND, "Link A", 20.0),
            occurrence(THIRD, "Link B", 40.0),
            occurrence(FOURTH, "Link C", 60.0),
        ]))
        .unwrap();
    document
}

fn axis_z() -> AssemblyJointAxis {
    AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0])
}

fn revolute(limits: Option<AssemblyJointLimits>, position_degrees: f64) -> AssemblyJointKind {
    AssemblyJointKind::Revolute {
        axis: axis_z(),
        limits,
        position_degrees,
    }
}

fn prismatic(limits: Option<AssemblyJointLimits>, position_mm: f64) -> AssemblyJointKind {
    AssemblyJointKind::Prismatic {
        axis: axis_z(),
        limits,
        position_mm,
    }
}

fn canonical_joints() -> [AssemblyJoint; 3] {
    [
        AssemblyJoint::new(FIXED_JOINT, FIRST, SECOND, AssemblyJointKind::Fixed),
        AssemblyJoint::new(
            REVOLUTE_JOINT,
            FIRST,
            THIRD,
            revolute(Some(AssemblyJointLimits::new(-90.0, 90.0)), 15.0),
        ),
        AssemblyJoint::new(
            PRISMATIC_JOINT,
            FIRST,
            FOURTH,
            prismatic(Some(AssemblyJointLimits::new(0.0, 100.0)), 25.0),
        ),
    ]
}

fn seeded_with_joints() -> DocumentStore {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(
            canonical_joints()
                .into_iter()
                .map(CanonicalCommand::CreateAssemblyJoint)
                .collect(),
        ))
        .unwrap();
    document
}

fn atomic_position_batch(
    document: &DocumentStore,
    id: AssemblyJointId,
    position: f64,
) -> CommandBatch {
    let source = document.current();
    let solution = solve_assembly_joint_kinematics_with_drivers(
        &source,
        &[AssemblyMotionDriver::new(id, position)],
    )
    .unwrap();
    let child_id = source.assembly_joint(id).unwrap().child_occurrence_id();
    CommandBatch::new(vec![
        CanonicalCommand::SetAssemblyJointPosition { id, position },
        CanonicalCommand::ApplyAssemblySolve {
            source_revision: source.revision_id(),
            source_digest: source.canonical_digest(),
            transforms: vec![(child_id, solution.pose(child_id).unwrap().local_transform())],
        },
    ])
}

#[test]
fn canonical_joint_kinds_commit_undo_redo_and_persist_losslessly() {
    let mut document = seeded_document();
    let before_digest = document.current().canonical_digest();
    let before_undo = document.visible_undo_steps();
    let [fixed, rev, pris] = canonical_joints();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyJoint(fixed.clone()),
            CanonicalCommand::CreateAssemblyJoint(rev.clone()),
            CanonicalCommand::CreateAssemblyJoint(pris.clone()),
        ]))
        .unwrap();

    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    assert_eq!(committed.assembly_joint(FIXED_JOINT), Some(&fixed));
    assert_eq!(committed.assembly_joint(REVOLUTE_JOINT), Some(&rev));
    assert_eq!(committed.assembly_joint(PRISMATIC_JOINT), Some(&pris));

    // Semantic state rows expose schema, topology, and kind for every joint.
    let state = encode_semantic_state(&committed);
    let complete = state.complete_v1();
    assert!(complete.contains(&format!(
        "assembly_joint.20=schema:{ASSEMBLY_JOINT_SCHEMA_V1:?},parent:10,child:11,kind:fixed"
    )));
    assert!(complete.contains("assembly_joint.21=schema:"));
    assert!(complete.contains("kind:revolute"));
    assert!(complete.contains("assembly_joint.22=schema:"));
    assert!(complete.contains("kind:prismatic"));

    // Save/load through the persistence API preserves the canonical digest,
    // the semantic state, and the joint payloads bit-for-bit.
    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        complete
    );
    assert_eq!(
        reopened.snapshot().assembly_joint(FIXED_JOINT),
        Some(&fixed)
    );
    assert_eq!(
        reopened.snapshot().assembly_joint(REVOLUTE_JOINT),
        Some(&rev)
    );
    assert_eq!(
        reopened.snapshot().assembly_joint(PRISMATIC_JOINT),
        Some(&pris)
    );

    // One-step undo returns to the pre-joint document; redo restores it.
    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert!(document.current().assembly_joint(FIXED_JOINT).is_none());
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    assert_eq!(document.current().assembly_joint(FIXED_JOINT), Some(&fixed));
}

#[test]
fn invalid_joints_and_dependent_occurrence_deletes_fail_without_partial_state() {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                FIXED_JOINT,
                FIRST,
                SECOND,
                AssemblyJointKind::Fixed,
            )),
        ]))
        .unwrap();

    let missing = OccurrenceId(999);
    let invalid = [
        // Self joint: parent and child are the same occurrence.
        AssemblyJoint::new(AssemblyJointId(30), THIRD, THIRD, AssemblyJointKind::Fixed),
        // Missing parent occurrence.
        AssemblyJoint::new(
            AssemblyJointId(31),
            missing,
            THIRD,
            AssemblyJointKind::Fixed,
        ),
        // Missing child occurrence.
        AssemblyJoint::new(
            AssemblyJointId(32),
            THIRD,
            missing,
            AssemblyJointKind::Fixed,
        ),
        // Cycle: SECOND is already the child of FIRST.
        AssemblyJoint::new(AssemblyJointId(33), SECOND, FIRST, AssemblyJointKind::Fixed),
        // Second parent for SECOND, which already has FIRST as parent.
        AssemblyJoint::new(AssemblyJointId(34), THIRD, SECOND, AssemblyJointKind::Fixed),
    ];
    for candidate in invalid {
        let before = store_stamp(&document);
        // Pair with a grounding side-effect to prove batches fail atomically.
        assert!(matches!(
            document.apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceGrounded {
                    id: FIRST,
                    grounded: true,
                },
                CanonicalCommand::CreateAssemblyJoint(candidate.clone()),
            ])),
            Err(CanonicalError::InvalidAssemblyJoint(id)) if id == candidate.id()
        ));
        assert_eq!(store_stamp(&document), before);
        assert!(!document.current().occurrence_is_grounded(FIRST));
    }

    let before_duplicate = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                FIXED_JOINT,
                FIRST,
                THIRD,
                AssemblyJointKind::Fixed,
            )),
        ])),
        Err(CanonicalError::AssemblyJointAlreadyExists(FIXED_JOINT))
    ));
    assert_eq!(store_stamp(&document), before_duplicate);

    // Occurrences referenced by a joint cannot be deleted.
    for id in [FIRST, SECOND] {
        let before = store_stamp(&document);
        assert!(matches!(
            document.apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteOccurrence { id }
            ])),
            Err(CanonicalError::OccurrenceInAssemblyJoint(blocked)) if blocked == id
        ));
        assert_eq!(store_stamp(&document), before);
    }
}

#[test]
fn set_position_and_limits_update_joints_and_reject_invalid_targets() {
    let mut document = seeded_with_joints();

    let before_unsynchronized = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointPosition {
                id: REVOLUTE_JOINT,
                position: 45.0,
            },
        ])),
        Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
            REVOLUTE_JOINT
        ))
    ));
    assert_eq!(store_stamp(&document), before_unsynchronized);

    document
        .apply_batch(&atomic_position_batch(&document, REVOLUTE_JOINT, 45.0))
        .unwrap();
    let kind = document
        .current()
        .assembly_joint(REVOLUTE_JOINT)
        .unwrap()
        .kind();
    assert_eq!(kind.position(), Some(45.0));
    assert_eq!(kind.limits(), Some(AssemblyJointLimits::new(-90.0, 90.0)));

    let replacement = Some(AssemblyJointLimits::new(0.0, 60.0));
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointLimits {
                id: REVOLUTE_JOINT,
                limits: replacement,
            },
        ]))
        .unwrap();
    let kind = document
        .current()
        .assembly_joint(REVOLUTE_JOINT)
        .unwrap()
        .kind();
    assert_eq!(kind.limits(), replacement);
    assert_eq!(kind.position(), Some(45.0));

    // Position outside current limits is rejected atomically.
    let before = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointPosition {
                id: REVOLUTE_JOINT,
                position: 90.0,
            },
        ])),
        Err(CanonicalError::InvalidAssemblyJoint(REVOLUTE_JOINT))
    ));
    assert_eq!(store_stamp(&document), before);

    // Limits that exclude the current position are rejected.
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointLimits {
                id: REVOLUTE_JOINT,
                limits: Some(AssemblyJointLimits::new(50.0, 60.0)),
            },
        ])),
        Err(CanonicalError::InvalidAssemblyJoint(REVOLUTE_JOINT))
    ));
    assert_eq!(store_stamp(&document), before);

    // Fixed joints have no position or limits to set.
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointPosition {
                id: FIXED_JOINT,
                position: 1.0,
            },
        ])),
        Err(CanonicalError::InvalidAssemblyJoint(FIXED_JOINT))
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointLimits {
                id: FIXED_JOINT,
                limits: Some(AssemblyJointLimits::new(0.0, 1.0)),
            },
        ])),
        Err(CanonicalError::InvalidAssemblyJoint(FIXED_JOINT))
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointPosition {
                id: AssemblyJointId(999),
                position: 1.0,
            },
        ])),
        Err(CanonicalError::AssemblyJointNotFound(AssemblyJointId(999)))
    ));
    assert_eq!(store_stamp(&document), before);

    // Clearing limits succeeds and preserves the position.
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointLimits {
                id: PRISMATIC_JOINT,
                limits: None,
            },
        ]))
        .unwrap();
    let kind = document
        .current()
        .assembly_joint(PRISMATIC_JOINT)
        .unwrap()
        .kind();
    assert_eq!(kind.limits(), None);
    assert_eq!(kind.position(), Some(25.0));
}

#[test]
fn position_geometry_atomicity_rejects_noop_duplicate_and_relabel_bypasses() {
    let mut document = seeded_with_joints();
    let source = document.current();
    let current_transform = source.occurrence(THIRD).unwrap().transform();

    let invalid_batches = [
        CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointPosition {
                id: REVOLUTE_JOINT,
                position: 45.0,
            },
            CanonicalCommand::ApplyAssemblySolve {
                source_revision: source.revision_id(),
                source_digest: source.canonical_digest(),
                transforms: vec![(THIRD, current_transform)],
            },
        ]),
        {
            let mut commands = atomic_position_batch(&document, REVOLUTE_JOINT, 45.0)
                .commands()
                .to_vec();
            commands.push(commands.last().unwrap().clone());
            CommandBatch::new(commands)
        },
        CommandBatch::new(vec![CanonicalCommand::SetAssemblyJointKind {
            id: REVOLUTE_JOINT,
            kind: prismatic(Some(AssemblyJointLimits::new(0.0, 100.0)), 15.0),
        }]),
        CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyJoint { id: REVOLUTE_JOINT },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                REVOLUTE_JOINT,
                FIRST,
                THIRD,
                revolute(Some(AssemblyJointLimits::new(-90.0, 90.0)), 45.0),
            )),
        ]),
        CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyJoint { id: REVOLUTE_JOINT },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(99),
                FIRST,
                THIRD,
                revolute(Some(AssemblyJointLimits::new(-90.0, 90.0)), 45.0),
            )),
        ]),
    ];

    for batch in invalid_batches {
        let before = store_stamp(&document);
        assert!(matches!(
            document.apply_batch(&batch),
            Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
                REVOLUTE_JOINT
            ))
        ));
        assert_eq!(store_stamp(&document), before);
    }
}

#[test]
fn matching_solve_allows_joint_kind_change_without_geometry_drift() {
    let mut document = seeded_with_joints();
    let source = document.current();
    let expected = solve_assembly_joint_kinematics_with_drivers(
        &source,
        &[AssemblyMotionDriver::new(REVOLUTE_JOINT, 0.0)],
    )
    .unwrap()
    .pose(THIRD)
    .unwrap()
    .local_transform();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointKind {
                id: REVOLUTE_JOINT,
                kind: AssemblyJointKind::Fixed,
            },
            CanonicalCommand::ApplyAssemblySolve {
                source_revision: source.revision_id(),
                source_digest: source.canonical_digest(),
                transforms: vec![(THIRD, expected)],
            },
        ]))
        .unwrap();
    let committed = document.current();
    assert_eq!(
        committed.assembly_joint(REVOLUTE_JOINT).unwrap().kind(),
        AssemblyJointKind::Fixed
    );
    assert_transform_near(committed.occurrence(THIRD).unwrap().transform(), expected);
    assert_transform_near(
        solve_assembly_joint_kinematics(&committed)
            .unwrap()
            .pose(THIRD)
            .unwrap()
            .local_transform(),
        expected,
    );
}

#[test]
fn sub_tolerance_motion_driver_still_publishes_its_joint_state_atomically() {
    let mut document = seeded_with_joints();
    let target = 25.000_000_000_001;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "mixed tolerance motion",
                vec![
                    AssemblyMotionDriver::new(REVOLUTE_JOINT, 45.0),
                    AssemblyMotionDriver::new(PRISMATIC_JOINT, target),
                ],
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    let publication = solution.publication_batch(&document.current()).unwrap();
    let transforms = publication
        .commands()
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::ApplyAssemblySolve { transforms, .. } => Some(transforms),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        transforms.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![THIRD, FOURTH]
    );
    document.apply_batch(&publication).unwrap();
    assert_eq!(
        document
            .current()
            .assembly_joint(PRISMATIC_JOINT)
            .unwrap()
            .kind()
            .position(),
        Some(target)
    );
}

#[test]
fn motion_studies_are_validated_and_protect_driven_joints_from_deletion() {
    let mut document = seeded_with_joints();

    let invalid_studies = [
        // Driver position outside the revolute limits.
        (
            AssemblyMotionStudy::new(
                STUDY,
                "over-limit",
                vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, 120.0)],
            ),
            CanonicalError::InvalidAssemblyMotionStudy(STUDY),
        ),
        // Fixed joints cannot be driven.
        (
            AssemblyMotionStudy::new(
                STUDY,
                "fixed-driver",
                vec![AssemblyMotionDriver::new(FIXED_JOINT, 1.0)],
            ),
            CanonicalError::InvalidAssemblyMotionStudy(STUDY),
        ),
        // Drivers must reference existing joints.
        (
            AssemblyMotionStudy::new(
                STUDY,
                "missing-joint",
                vec![AssemblyMotionDriver::new(AssemblyJointId(999), 1.0)],
            ),
            CanonicalError::AssemblyJointNotFound(AssemblyJointId(999)),
        ),
    ];
    for (study, expected) in invalid_studies {
        let before = store_stamp(&document);
        match document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(study),
        ])) {
            Ok(_) => panic!("invalid motion study unexpectedly committed"),
            Err(error) => assert_eq!(error, expected),
        }
        assert_eq!(store_stamp(&document), before);
    }

    let study = AssemblyMotionStudy::new(
        STUDY,
        "sweep",
        vec![
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 30.0),
            AssemblyMotionDriver::new(PRISMATIC_JOINT, 75.0),
        ],
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(study.clone()),
        ]))
        .unwrap();
    assert_eq!(
        document.current().assembly_motion_study(STUDY),
        Some(&study)
    );
    assert!(
        encode_semantic_state(&document.current())
            .complete_v1()
            .contains("assembly_motion_study.40=schema:")
    );

    let before_duplicate = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(study.clone()),
        ])),
        Err(CanonicalError::AssemblyMotionStudyAlreadyExists(STUDY))
    ));
    assert_eq!(store_stamp(&document), before_duplicate);

    // Joints referenced by a motion study cannot be deleted.
    let before_delete = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyJoint { id: REVOLUTE_JOINT },
        ])),
        Err(CanonicalError::AssemblyJointInMotionStudy(REVOLUTE_JOINT))
    ));
    assert_eq!(store_stamp(&document), before_delete);

    // Updates are re-validated in place.
    let updated = AssemblyMotionStudy::new(
        STUDY,
        "sweep",
        vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, -45.0)],
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateAssemblyMotionStudy(updated.clone()),
        ]))
        .unwrap();
    assert_eq!(
        document.current().assembly_motion_study(STUDY),
        Some(&updated)
    );

    // Removing the study unblocks joint deletion.
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMotionStudy { id: STUDY },
            CanonicalCommand::DeleteAssemblyJoint { id: REVOLUTE_JOINT },
        ]))
        .unwrap();
    assert!(document.current().assembly_motion_study(STUDY).is_none());
    assert!(document.current().assembly_joint(REVOLUTE_JOINT).is_none());
}

#[test]
fn motion_study_sampling_is_synchronized_deterministic_and_includes_both_endpoints() {
    let mut document = seeded_with_joints();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "synchronized sweep",
                vec![
                    AssemblyMotionDriver::new(PRISMATIC_JOINT, 85.0),
                    AssemblyMotionDriver::new(REVOLUTE_JOINT, 75.0),
                ],
            )),
        ]))
        .unwrap();
    let before = store_stamp(&document);

    let path = sample_assembly_motion_study(&document.current(), STUDY, 4).unwrap();
    assert_eq!(path.source_revision(), document.current().revision_id());
    assert_eq!(path.source_digest(), document.current().canonical_digest());
    assert_eq!(path.study_id(), STUDY);
    assert_eq!(path.sample_intervals(), 4);
    assert_eq!(path.samples().len(), 5);
    assert_eq!(
        path.samples()
            .iter()
            .map(|sample| sample.progress())
            .collect::<Vec<_>>(),
        vec![0.0, 0.25, 0.5, 0.75, 1.0]
    );
    assert_eq!(
        path.samples()[0]
            .drivers()
            .iter()
            .map(|driver| (driver.joint_id(), driver.position()))
            .collect::<Vec<_>>(),
        vec![(REVOLUTE_JOINT, 15.0), (PRISMATIC_JOINT, 25.0)]
    );
    assert_eq!(
        path.samples()[2]
            .drivers()
            .iter()
            .map(|driver| (driver.joint_id(), driver.position()))
            .collect::<Vec<_>>(),
        vec![(REVOLUTE_JOINT, 45.0), (PRISMATIC_JOINT, 55.0)]
    );
    assert_eq!(
        path.samples()[4]
            .drivers()
            .iter()
            .map(|driver| (driver.joint_id(), driver.position()))
            .collect::<Vec<_>>(),
        vec![(REVOLUTE_JOINT, 75.0), (PRISMATIC_JOINT, 85.0)]
    );

    let direct = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    assert_eq!(path.samples().last().unwrap().solution(), &direct);
    assert_eq!(
        sample_assembly_motion_study(&document.current(), STUDY, 4).unwrap(),
        path
    );
    assert_eq!(store_stamp(&document), before);
}

#[test]
fn motion_study_sampling_rejects_unbounded_work_without_mutating_the_document() {
    let mut document = seeded_with_joints();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "bounded sweep",
                vec![AssemblyMotionDriver::new(PRISMATIC_JOINT, 75.0)],
            )),
        ]))
        .unwrap();
    let before = store_stamp(&document);

    for invalid in [0, MAX_ASSEMBLY_MOTION_SAMPLE_INTERVALS + 1] {
        assert_eq!(
            sample_assembly_motion_study(&document.current(), STUDY, invalid),
            Err(AssemblyMotionSamplingError::InvalidSampleIntervals(invalid))
        );
        assert_eq!(store_stamp(&document), before);
    }
}

fn crossing_motion_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    let axis_x = AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Crossing motion".into(),
            },
            occurrence(FIRST, "Obstacle", 0.0),
            occurrence(SECOND, "Mover", -10.0),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                PRISMATIC_JOINT,
                FIRST,
                SECOND,
                AssemblyJointKind::Prismatic {
                    axis: axis_x,
                    limits: Some(AssemblyJointLimits::new(0.0, 20.0)),
                    position_mm: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "Cross obstacle",
                vec![AssemblyMotionDriver::new(PRISMATIC_JOINT, 20.0)],
            )),
        ]))
        .unwrap();
    document
}

#[test]
fn swept_clearance_detects_a_crossing_between_clear_endpoint_samples() {
    let document = crossing_motion_document();
    let before = store_stamp(&document);
    let path = sample_assembly_motion_study(&document.current(), STUDY, 1).unwrap();
    let unit = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap();
    let bodies = [
        AssemblyMotionCollisionBody::new(SECOND, unit),
        AssemblyMotionCollisionBody::new(FIRST, unit),
    ];

    let analysis = analyze_assembly_motion_clearance(&path, &bodies, 0.0).unwrap();
    assert_eq!(analysis.source_revision(), document.current().revision_id());
    assert_eq!(
        analysis.source_digest(),
        document.current().canonical_digest()
    );
    assert_eq!(analysis.minimum_clearance_mm(), 0.0);
    assert_eq!(
        analysis.minimum_clearance().pair(),
        AssemblyMotionCollisionPair::new(FIRST, SECOND).unwrap()
    );
    assert!((analysis.minimum_clearance().progress_start() - 0.45).abs() <= 1.0e-12);
    assert!((analysis.minimum_clearance().progress_end() - 0.45).abs() <= 1.0e-12);
    let first_contact = analysis.first_contact().unwrap();
    assert_eq!(first_contact.pair(), analysis.minimum_clearance().pair());
    assert!((first_contact.progress_start() - 0.45).abs() <= 1.0e-12);
    assert!((first_contact.progress_end() - 0.45).abs() <= 1.0e-12);
    assert_eq!(
        analyze_assembly_motion_clearance(&path, &bodies, 0.0).unwrap(),
        analysis
    );
    assert_eq!(store_stamp(&document), before);
}

#[test]
fn parallel_synchronized_motion_preserves_clearance_without_false_contact() {
    let mut document = DocumentStore::new();
    let axis_x = AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Parallel motion".into(),
            },
            occurrence(FIRST, "Root", -100.0),
            occurrence(SECOND, "Parallel A", 0.0),
            occurrence(THIRD, "Parallel B", 5.0),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                REVOLUTE_JOINT,
                FIRST,
                SECOND,
                AssemblyJointKind::Prismatic {
                    axis: axis_x,
                    limits: Some(AssemblyJointLimits::new(0.0, 10.0)),
                    position_mm: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                PRISMATIC_JOINT,
                FIRST,
                THIRD,
                AssemblyJointKind::Prismatic {
                    axis: axis_x,
                    limits: Some(AssemblyJointLimits::new(0.0, 10.0)),
                    position_mm: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "Parallel sweep",
                vec![
                    AssemblyMotionDriver::new(REVOLUTE_JOINT, 10.0),
                    AssemblyMotionDriver::new(PRISMATIC_JOINT, 10.0),
                ],
            )),
        ]))
        .unwrap();
    let path = sample_assembly_motion_study(&document.current(), STUDY, 1).unwrap();
    let unit = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap();
    let analysis = analyze_assembly_motion_clearance(
        &path,
        &[
            AssemblyMotionCollisionBody::new(SECOND, unit),
            AssemblyMotionCollisionBody::new(THIRD, unit),
        ],
        0.0,
    )
    .unwrap();

    assert_eq!(analysis.minimum_clearance_mm(), 4.0);
    assert_eq!(analysis.first_contact(), None);
}

#[test]
fn swept_clearance_inputs_fail_closed_without_document_mutation() {
    let document = crossing_motion_document();
    let before = store_stamp(&document);
    let path = sample_assembly_motion_study(&document.current(), STUDY, 1).unwrap();
    let unit = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap();
    let empty = Aabb::new([0.0, 0.0, 0.0], [0.0, 1.0, 1.0]).unwrap();

    assert_eq!(
        analyze_assembly_motion_clearance(
            &path,
            &[AssemblyMotionCollisionBody::new(FIRST, unit)],
            0.0,
        ),
        Err(AssemblyMotionClearanceError::InsufficientBodies)
    );
    assert_eq!(
        analyze_assembly_motion_clearance(
            &path,
            &[
                AssemblyMotionCollisionBody::new(FIRST, unit),
                AssemblyMotionCollisionBody::new(FIRST, unit),
            ],
            0.0,
        ),
        Err(AssemblyMotionClearanceError::DuplicateOccurrence(FIRST))
    );
    assert_eq!(
        analyze_assembly_motion_clearance(
            &path,
            &[
                AssemblyMotionCollisionBody::new(FIRST, unit),
                AssemblyMotionCollisionBody::new(SECOND, empty),
            ],
            0.0,
        ),
        Err(AssemblyMotionClearanceError::InvalidBodyBounds(SECOND))
    );
    assert_eq!(
        analyze_assembly_motion_clearance(
            &path,
            &[
                AssemblyMotionCollisionBody::new(FIRST, unit),
                AssemblyMotionCollisionBody::new(OccurrenceId(999), unit),
            ],
            0.0,
        ),
        Err(AssemblyMotionClearanceError::MissingPose(OccurrenceId(999)))
    );
    assert_eq!(
        analyze_assembly_motion_clearance(
            &path,
            &[
                AssemblyMotionCollisionBody::new(FIRST, unit),
                AssemblyMotionCollisionBody::new(SECOND, unit),
            ],
            -1.0,
        ),
        Err(AssemblyMotionClearanceError::InvalidClearanceTolerance)
    );
    let excessive = (1..=1_002)
        .map(|id| AssemblyMotionCollisionBody::new(OccurrenceId(id), unit))
        .collect::<Vec<_>>();
    assert_eq!(
        analyze_assembly_motion_clearance(&path, &excessive, 0.0),
        Err(AssemblyMotionClearanceError::AnalysisBudgetExceeded)
    );
    assert_eq!(store_stamp(&document), before);
}

#[test]
fn clearance_preview_is_read_only_and_rebuilds_identically_after_save_open() {
    let document = crossing_motion_document();
    let before = store_stamp(&document);
    let unit = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap();
    let bodies = [
        AssemblyMotionCollisionBody::new(FIRST, unit),
        AssemblyMotionCollisionBody::new(SECOND, unit),
    ];

    let preview =
        preview_assembly_motion_study_clearance(&document.current(), STUDY, 4, &bodies, 0.0)
            .unwrap();
    assert_eq!(preview.path().samples().len(), 5);
    assert_eq!(preview.clearance().minimum_clearance_mm(), 0.0);
    let proposal = preview
        .final_solution()
        .prepare_publication(&document)
        .unwrap();
    assert!(!proposal.batch().commands().is_empty());
    assert_eq!(store_stamp(&document), before);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    let rebuilt =
        preview_assembly_motion_study_clearance(&reopened.snapshot(), STUDY, 4, &bodies, 0.0)
            .unwrap();
    assert_eq!(rebuilt, preview);
    assert_eq!(
        rebuilt
            .final_solution()
            .publication_batch(&reopened.snapshot())
            .unwrap(),
        preview
            .final_solution()
            .publication_batch(&document.current())
            .unwrap()
    );
}

#[test]
fn integration_round_trip_dependencies_stale_replay_and_undo_redo_are_atomic() {
    let mut document = seeded_with_joints();
    let study = AssemblyMotionStudy::new(
        STUDY,
        "integration sweep",
        vec![
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 30.0),
            AssemblyMotionDriver::new(PRISMATIC_JOINT, 75.0),
        ],
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(study.clone()),
        ]))
        .unwrap();

    let saved = document.current();
    let reopened = persistence::load(&persistence::save(&saved)).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        saved.canonical_digest()
    );
    assert_eq!(
        reopened.snapshot().assembly_motion_study(STUDY),
        Some(&study)
    );
    assert_eq!(
        reopened.snapshot().assembly_joint(REVOLUTE_JOINT),
        saved.assembly_joint(REVOLUTE_JOINT)
    );
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        encode_semantic_state(&saved).complete_v1()
    );

    for (command, expected) in [
        (
            CanonicalCommand::DeleteOccurrence { id: FIRST },
            CanonicalError::OccurrenceInAssemblyJoint(FIRST),
        ),
        (
            CanonicalCommand::DeleteAssemblyJoint { id: REVOLUTE_JOINT },
            CanonicalError::AssemblyJointInMotionStudy(REVOLUTE_JOINT),
        ),
    ] {
        let before = store_stamp(&document);
        match document.apply_batch(&CommandBatch::new(vec![command])) {
            Ok(_) => panic!("dependent delete unexpectedly committed"),
            Err(error) => assert_eq!(error, expected),
        }
        assert_eq!(store_stamp(&document), before);
    }

    let stale = document
        .prepare_proposal(atomic_position_batch(&document, REVOLUTE_JOINT, 20.0))
        .unwrap();
    document
        .apply_batch(&atomic_position_batch(&document, REVOLUTE_JOINT, -20.0))
        .unwrap();
    let before_stale = store_stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store_stamp(&document), before_stale);

    let before_commit_digest = document.current().canonical_digest();
    let before_commit_undo = document.visible_undo_steps();
    let fresh = document
        .prepare_proposal(atomic_position_batch(&document, REVOLUTE_JOINT, 45.0))
        .unwrap();
    document.commit_proposal(&fresh).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_ne!(committed_digest, before_commit_digest);
    assert_eq!(document.visible_undo_steps(), before_commit_undo + 1);

    let before_replay = store_stamp(&document);
    assert!(matches!(
        document.commit_proposal(&fresh),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store_stamp(&document), before_replay);

    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        before_commit_digest
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn conflicting_proposals_are_stale_and_fresh_proposals_commit_reviewed() {
    let mut document = seeded_with_joints();

    // Preparing a proposal does not mutate the document.
    let before_prepare = store_stamp(&document);
    let proposal = document
        .prepare_proposal(atomic_position_batch(&document, REVOLUTE_JOINT, 45.0))
        .unwrap();
    assert_eq!(store_stamp(&document), before_prepare);

    // A concurrent edit of the same joint makes the proposal stale.
    document
        .apply_batch(&atomic_position_batch(&document, REVOLUTE_JOINT, -10.0))
        .unwrap();
    let before_stale = store_stamp(&document);
    assert!(matches!(
        document.commit_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store_stamp(&document), before_stale);
    assert_eq!(
        document
            .current()
            .assembly_joint(REVOLUTE_JOINT)
            .unwrap()
            .kind()
            .position(),
        Some(-10.0)
    );

    // A fresh proposal against the current revision commits as one undo step.
    let before_digest = document.current().canonical_digest();
    let before_undo = document.visible_undo_steps();
    let fresh = document
        .prepare_proposal(atomic_position_batch(&document, REVOLUTE_JOINT, 45.0))
        .unwrap();
    document.commit_proposal(&fresh).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    assert_eq!(
        document
            .current()
            .assembly_joint(REVOLUTE_JOINT)
            .unwrap()
            .kind()
            .position(),
        Some(45.0)
    );
}

fn assert_transform_near(actual: Transform, expected: Transform) {
    for (actual, expected) in actual.matrix().iter().zip(expected.matrix()) {
        assert!(
            (actual - expected).abs() <= 1.0e-10,
            "actual {actual:?} did not match expected {expected:?}"
        );
    }
}

#[test]
fn fixed_revolute_prismatic_chain_propagates_parent_before_child_deterministically() {
    let mut document = seeded_document();
    let axis_x = AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    document
        .apply_batch(&CommandBatch::new(vec![
            // IDs intentionally put the deepest edge first in map order.
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(1),
                THIRD,
                FOURTH,
                AssemblyJointKind::Prismatic {
                    axis: axis_x,
                    limits: Some(AssemblyJointLimits::new(0.0, 10.0)),
                    position_mm: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(2),
                SECOND,
                THIRD,
                AssemblyJointKind::Revolute {
                    axis: axis_z(),
                    limits: Some(AssemblyJointLimits::new(-180.0, 180.0)),
                    position_degrees: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(3),
                FIRST,
                SECOND,
                AssemblyJointKind::Fixed,
            )),
        ]))
        .unwrap();

    let drivers = [
        AssemblyMotionDriver::new(AssemblyJointId(1), 2.0),
        AssemblyMotionDriver::new(AssemblyJointId(2), 90.0),
    ];
    let first =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers).unwrap();
    let second =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .poses()
            .iter()
            .map(|pose| pose.occurrence_id())
            .collect::<Vec<_>>(),
        vec![FIRST, SECOND, THIRD, FOURTH]
    );
    assert_transform_near(
        first.pose(SECOND).unwrap().world_transform(),
        Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        first.pose(THIRD).unwrap().world_transform(),
        Transform::from_matrix([
            0.0, -1.0, 0.0, 20.0, 1.0, 0.0, 0.0, 20.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap(),
    );
    assert_transform_near(
        first.pose(FOURTH).unwrap().world_transform(),
        Transform::from_matrix([
            0.0, -1.0, 0.0, 20.0, 1.0, 0.0, 0.0, 42.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap(),
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "chain publication",
                drivers.to_vec(),
            )),
        ]))
        .unwrap();
    let publication_solution = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    let publication = publication_solution
        .publication_batch(&document.current())
        .unwrap();
    let mut child_only_commands = publication.commands().to_vec();
    let CanonicalCommand::ApplyAssemblySolve {
        source_revision,
        source_digest,
        transforms,
    } = child_only_commands.pop().unwrap()
    else {
        panic!("motion-study publication must end with the assembly solve");
    };
    assert_eq!(
        transforms.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![THIRD, FOURTH]
    );
    child_only_commands.push(CanonicalCommand::ApplyAssemblySolve {
        source_revision,
        source_digest,
        transforms: vec![transforms[0]],
    });
    let before_invalid = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(child_only_commands)),
        Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
            AssemblyJointId(1)
        ))
    ));
    assert_eq!(store_stamp(&document), before_invalid);

    let expected_third = publication_solution.pose(THIRD).unwrap().local_transform();
    let expected_fourth = publication_solution.pose(FOURTH).unwrap().local_transform();
    let proposal = publication_solution.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert_transform_near(
        document.current().occurrence(THIRD).unwrap().transform(),
        expected_third,
    );
    assert_transform_near(
        document.current().occurrence(FOURTH).unwrap().transform(),
        expected_fourth,
    );
    let driverless = solve_assembly_joint_kinematics(&document.current()).unwrap();
    assert_transform_near(
        driverless.pose(THIRD).unwrap().local_transform(),
        expected_third,
    );
    assert_transform_near(
        driverless.pose(FOURTH).unwrap().local_transform(),
        expected_fourth,
    );
}

#[test]
fn revolute_joint_rotates_about_its_parent_space_pivot() {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                REVOLUTE_JOINT,
                FIRST,
                SECOND,
                AssemblyJointKind::Revolute {
                    axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [10.0, 0.0, 0.0]),
                    limits: Some(AssemblyJointLimits::new(-180.0, 180.0)),
                    position_degrees: 0.0,
                },
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_joint_kinematics_with_drivers(
        &document.current(),
        &[AssemblyMotionDriver::new(REVOLUTE_JOINT, 90.0)],
    )
    .unwrap();
    assert_transform_near(
        solution.pose(SECOND).unwrap().world_transform(),
        Transform::from_matrix([
            0.0, -1.0, 0.0, 10.0, 1.0, 0.0, 0.0, 10.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap(),
    );
}

#[test]
fn small_but_invertible_parent_scale_remains_solvable() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Scaled kinematic part".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Scaled parent".into(),
                transform: Transform::from_matrix([
                    1.0e-6, 0.0, 0.0, 0.0, 0.0, 1.0e-6, 0.0, 0.0, 0.0, 0.0, 1.0e-6, 0.0, 0.0, 0.0,
                    0.0, 1.0,
                ])
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Child".into(),
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                FIXED_JOINT,
                FIRST,
                SECOND,
                AssemblyJointKind::Fixed,
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_joint_kinematics(&document.current()).unwrap();
    assert_transform_near(
        solution.pose(SECOND).unwrap().local_transform(),
        document.current().occurrence(SECOND).unwrap().transform(),
    );
}

#[test]
fn nested_group_child_is_returned_in_group_local_coordinates() {
    const OUTER: GroupId = GroupId(70);
    const INNER: GroupId = GroupId(71);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Nested component".into(),
            },
            CanonicalCommand::CreateGroup {
                id: OUTER,
                name: "Outer".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: INNER,
                name: "Inner".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: Some(OUTER),
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Driver".into(),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Nested child".into(),
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                PRISMATIC_JOINT,
                FIRST,
                SECOND,
                AssemblyJointKind::Prismatic {
                    axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                    limits: Some(AssemblyJointLimits::new(0.0, 20.0)),
                    position_mm: 0.0,
                },
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_joint_kinematics_with_drivers(
        &document.current(),
        &[AssemblyMotionDriver::new(PRISMATIC_JOINT, 10.0)],
    )
    .unwrap();
    assert_eq!(solution.source_revision(), document.current().revision_id());
    assert_eq!(
        solution.source_digest(),
        document.current().canonical_digest()
    );
    let child = solution.pose(SECOND).unwrap();
    assert_transform_near(
        child.world_transform(),
        Transform::from_translation(135.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        child.local_transform(),
        Transform::from_translation(15.0, 0.0, 0.0).unwrap(),
    );
}

#[test]
fn nested_limit_chain_is_topological_and_repeatable() {
    const OUTER: GroupId = GroupId(80);
    const INNER: GroupId = GroupId(81);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Nested limit chain".into(),
            },
            CanonicalCommand::CreateGroup {
                id: OUTER,
                name: "Outer".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: INNER,
                name: "Inner".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: Some(OUTER),
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Root".into(),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Nested parent".into(),
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: THIRD,
                definition_id: DEFINITION,
                name: "Nested child".into(),
                transform: Transform::from_translation(15.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER),
                tag: None,
                visible: true,
            },
            // The deeper edge sorts first, forcing the solver to defer it.
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(1),
                SECOND,
                THIRD,
                AssemblyJointKind::Fixed,
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(2),
                FIRST,
                SECOND,
                AssemblyJointKind::Prismatic {
                    axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                    limits: Some(AssemblyJointLimits::new(0.0, 10.0)),
                    position_mm: 0.0,
                },
            )),
        ]))
        .unwrap();

    let drivers = [AssemblyMotionDriver::new(AssemblyJointId(2), 10.0)];
    let first =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers).unwrap();
    let repeated =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers).unwrap();
    assert_eq!(first, repeated);

    let nested_parent = first.pose(SECOND).unwrap();
    assert_transform_near(
        nested_parent.world_transform(),
        Transform::from_translation(135.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        nested_parent.local_transform(),
        Transform::from_translation(15.0, 0.0, 0.0).unwrap(),
    );

    let nested_child = first.pose(THIRD).unwrap();
    assert_transform_near(
        nested_child.world_transform(),
        Transform::from_translation(145.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        nested_child.local_transform(),
        Transform::from_translation(25.0, 0.0, 0.0).unwrap(),
    );
}

#[test]
fn multiple_drivers_override_joint_positions_and_close_remaining_dof() {
    let document = seeded_with_joints();
    let unpowered = solve_assembly_joint_kinematics(&document.current()).unwrap();
    assert_eq!(
        unpowered.status(),
        AssemblyKinematicSolveStatus::UnderConstrained
    );
    assert_eq!(unpowered.remaining_dof(), 2);
    assert_eq!(
        unpowered
            .joint_diagnostics()
            .iter()
            .map(|diagnostic| (
                diagnostic.joint_id(),
                diagnostic.remaining_dof(),
                diagnostic.driver_count(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (FIXED_JOINT, 0, 0),
            (REVOLUTE_JOINT, 1, 0),
            (PRISMATIC_JOINT, 1, 0),
        ]
    );

    let drivers = [
        AssemblyMotionDriver::new(PRISMATIC_JOINT, 75.0),
        AssemblyMotionDriver::new(REVOLUTE_JOINT, 45.0),
    ];
    let powered =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers).unwrap();
    let reordered = solve_assembly_joint_kinematics_with_drivers(
        &document.current(),
        &[drivers[1], drivers[0]],
    )
    .unwrap();
    assert_eq!(powered, reordered);
    assert_eq!(
        powered.status(),
        AssemblyKinematicSolveStatus::FullyConstrained
    );
    assert_eq!(powered.remaining_dof(), 0);
    assert!(powered.redundant_driver_joint_ids().is_empty());
    assert_eq!(
        powered
            .joint_diagnostics()
            .iter()
            .map(|diagnostic| (
                diagnostic.joint_id(),
                diagnostic.remaining_dof(),
                diagnostic.driver_count(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (FIXED_JOINT, 0, 0),
            (REVOLUTE_JOINT, 0, 1),
            (PRISMATIC_JOINT, 0, 1),
        ]
    );
    let cosine = 30.0_f64.to_radians().cos();
    let sine = 30.0_f64.to_radians().sin();
    assert_transform_near(
        powered.pose(THIRD).unwrap().world_transform(),
        Transform::from_matrix([
            cosine,
            -sine,
            0.0,
            40.0 * cosine,
            sine,
            cosine,
            0.0,
            40.0 * sine,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
        .unwrap(),
    );
    assert_transform_near(
        powered.pose(FOURTH).unwrap().world_transform(),
        Transform::from_translation(60.0, 0.0, 50.0).unwrap(),
    );
}

#[test]
fn redundant_and_conflicting_drivers_have_deterministic_fail_closed_diagnostics() {
    let document = seeded_with_joints();
    let duplicate = AssemblyMotionDriver::new(REVOLUTE_JOINT, 30.0);
    let redundant =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &[duplicate, duplicate])
            .unwrap();
    assert_eq!(redundant.redundant_driver_joint_ids(), &[REVOLUTE_JOINT]);
    assert_eq!(redundant.remaining_dof(), 1);
    assert_eq!(redundant.joint_diagnostics()[1].driver_count(), 2);

    for drivers in [
        vec![
            AssemblyMotionDriver::new(PRISMATIC_JOINT, 20.0),
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 30.0),
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 10.0),
        ],
        vec![
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 10.0),
            AssemblyMotionDriver::new(PRISMATIC_JOINT, 20.0),
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 30.0),
        ],
    ] {
        assert_eq!(
            solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers),
            Err(AssemblyKinematicSolveError::OverConstrainedDriver(
                REVOLUTE_JOINT
            ))
        );
    }

    for (driver, expected) in [
        (
            AssemblyMotionDriver::new(AssemblyJointId(999), 1.0),
            AssemblyKinematicSolveError::UnknownDriverJoint(AssemblyJointId(999)),
        ),
        (
            AssemblyMotionDriver::new(FIXED_JOINT, 0.0),
            AssemblyKinematicSolveError::FixedJointDriven(FIXED_JOINT),
        ),
        (
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 120.0),
            AssemblyKinematicSolveError::InvalidDriverPosition(REVOLUTE_JOINT),
        ),
        (
            AssemblyMotionDriver::new(PRISMATIC_JOINT, f64::NAN),
            AssemblyKinematicSolveError::InvalidDriverPosition(PRISMATIC_JOINT),
        ),
    ] {
        assert_eq!(
            solve_assembly_joint_kinematics_with_drivers(&document.current(), &[driver]),
            Err(expected)
        );
    }
}

#[test]
fn driver_diagnostics_are_order_independent_repeatable_and_fail_closed() {
    let document = seeded_with_joints();
    let source_digest = document.current().canonical_digest();
    let drivers = [
        AssemblyMotionDriver::new(PRISMATIC_JOINT, 100.0),
        AssemblyMotionDriver::new(REVOLUTE_JOINT, -90.0),
        AssemblyMotionDriver::new(PRISMATIC_JOINT, 100.0),
        AssemblyMotionDriver::new(REVOLUTE_JOINT, -90.0),
    ];
    let reordered = [drivers[3], drivers[2], drivers[1], drivers[0]];

    let first =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &drivers).unwrap();
    let repeated =
        solve_assembly_joint_kinematics_with_drivers(&document.current(), &reordered).unwrap();
    assert_eq!(first, repeated);
    assert_eq!(
        first.status(),
        AssemblyKinematicSolveStatus::FullyConstrained
    );
    assert_eq!(first.remaining_dof(), 0);
    assert_eq!(
        first.redundant_driver_joint_ids(),
        &[REVOLUTE_JOINT, PRISMATIC_JOINT]
    );
    assert_eq!(
        first
            .joint_diagnostics()
            .iter()
            .map(|diagnostic| (
                diagnostic.joint_id(),
                diagnostic.remaining_dof(),
                diagnostic.driver_count(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (FIXED_JOINT, 0, 0),
            (REVOLUTE_JOINT, 0, 2),
            (PRISMATIC_JOINT, 0, 2),
        ]
    );

    let conflicting = [
        AssemblyMotionDriver::new(PRISMATIC_JOINT, 10.0),
        AssemblyMotionDriver::new(REVOLUTE_JOINT, 10.0),
        AssemblyMotionDriver::new(PRISMATIC_JOINT, 20.0),
        AssemblyMotionDriver::new(REVOLUTE_JOINT, 20.0),
    ];
    for candidate in [
        conflicting,
        [
            conflicting[3],
            conflicting[2],
            conflicting[1],
            conflicting[0],
        ],
    ] {
        assert_eq!(
            solve_assembly_joint_kinematics_with_drivers(&document.current(), &candidate),
            Err(AssemblyKinematicSolveError::OverConstrainedDriver(
                REVOLUTE_JOINT
            ))
        );
    }
    assert_eq!(document.current().canonical_digest(), source_digest);
}

#[test]
fn motion_study_preview_commit_semantic_state_and_save_open_are_atomic() {
    let mut document = seeded_with_joints();
    let study = AssemblyMotionStudy::new(
        STUDY,
        "canonical motion",
        vec![
            AssemblyMotionDriver::new(REVOLUTE_JOINT, 45.0),
            AssemblyMotionDriver::new(PRISMATIC_JOINT, 75.0),
        ],
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(study.clone()),
        ]))
        .unwrap();
    let source = document.current();
    let source_digest = source.canonical_digest();
    let source_semantic_state = encode_semantic_state(&source).complete_v1().to_owned();

    let solution = solve_assembly_motion_study(&source, STUDY).unwrap();
    assert_eq!(
        solution.status(),
        AssemblyKinematicSolveStatus::FullyConstrained
    );
    assert_eq!(
        solution.driven_joint_positions(),
        &[(REVOLUTE_JOINT, 45.0), (PRISMATIC_JOINT, 75.0)]
    );
    let expected_third = solution.pose(THIRD).unwrap().local_transform();
    let expected_fourth = solution.pose(FOURTH).unwrap().local_transform();
    let cosine = 30.0_f64.to_radians().cos();
    let sine = 30.0_f64.to_radians().sin();
    assert_transform_near(
        expected_third,
        Transform::from_matrix([
            cosine,
            -sine,
            0.0,
            40.0 * cosine,
            sine,
            cosine,
            0.0,
            40.0 * sine,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
        .unwrap(),
    );
    assert_transform_near(
        expected_fourth,
        Transform::from_translation(60.0, 0.0, 50.0).unwrap(),
    );
    assert_ne!(
        source.occurrence(THIRD).unwrap().transform(),
        expected_third
    );
    assert_ne!(
        source.occurrence(FOURTH).unwrap().transform(),
        expected_fourth
    );

    let proposal = solution.prepare_publication(&document).unwrap();
    assert_eq!(proposal.batch().commands().len(), 3);
    let undo_before = document.visible_undo_steps();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);

    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_eq!(
        committed
            .assembly_joint(REVOLUTE_JOINT)
            .unwrap()
            .kind()
            .position(),
        Some(45.0)
    );
    assert_eq!(
        committed
            .assembly_joint(PRISMATIC_JOINT)
            .unwrap()
            .kind()
            .position(),
        Some(75.0)
    );
    assert_eq!(
        committed.occurrence(THIRD).unwrap().transform(),
        expected_third
    );
    assert_eq!(
        committed.occurrence(FOURTH).unwrap().transform(),
        expected_fourth
    );
    let driverless = solve_assembly_joint_kinematics(&committed).unwrap();
    for pose in driverless.poses() {
        assert_transform_near(
            pose.local_transform(),
            committed
                .occurrence(pose.occurrence_id())
                .unwrap()
                .transform(),
        );
    }
    let committed_semantic_state = encode_semantic_state(&committed).complete_v1().to_owned();
    assert_ne!(committed_semantic_state, source_semantic_state);
    assert!(committed_semantic_state.contains("assembly_motion_study.40="));
    assert!(committed_semantic_state.contains("assembly_joint.21="));
    assert!(committed_semantic_state.contains("assembly_joint.22="));

    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        committed_semantic_state
    );
    assert_eq!(
        reopened.snapshot().assembly_motion_study(STUDY),
        Some(&study)
    );
    let repeated = solve_assembly_motion_study(&reopened.snapshot(), STUDY).unwrap();
    assert_eq!(
        repeated.publication_batch(&reopened.snapshot()),
        Err(AssemblyKinematicPublishError::NoCanonicalChanges)
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), source_digest);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn partial_motion_study_leaves_undriven_dof_stable_after_commit() {
    let mut document = seeded_with_joints();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "partial motion",
                vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, 45.0)],
            )),
        ]))
        .unwrap();
    let initial_fourth = document.current().occurrence(FOURTH).unwrap().transform();
    let absolute = solve_assembly_joint_kinematics(&document.current()).unwrap();
    assert_eq!(
        absolute.publication_batch(&document.current()),
        Err(AssemblyKinematicPublishError::NotMotionStudySolution)
    );

    let solution = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    assert_eq!(solution.remaining_dof(), 1);
    assert_eq!(
        solution.pose(FOURTH).unwrap().local_transform(),
        initial_fourth
    );
    let proposal = solution.prepare_publication(&document).unwrap();
    assert_eq!(proposal.batch().commands().len(), 2);
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(
        document.current().occurrence(FOURTH).unwrap().transform(),
        initial_fourth
    );

    let repeated = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    assert_eq!(
        repeated.publication_batch(&document.current()),
        Err(AssemblyKinematicPublishError::NoCanonicalChanges)
    );
}

#[test]
fn atomic_position_change_then_motion_study_reaches_target_without_double_motion() {
    let mut document = seeded_with_joints();
    let initial = document.current();
    let expected = solve_assembly_joint_kinematics_with_drivers(
        &initial,
        &[AssemblyMotionDriver::new(REVOLUTE_JOINT, 60.0)],
    )
    .unwrap()
    .pose(THIRD)
    .unwrap()
    .local_transform();

    document
        .apply_batch(&atomic_position_batch(&document, REVOLUTE_JOINT, 45.0))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "after direct position change",
                vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, 60.0)],
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    let proposal = solution.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    let committed = document.current();
    assert_transform_near(committed.occurrence(THIRD).unwrap().transform(), expected);
    assert_eq!(
        committed
            .assembly_joint(REVOLUTE_JOINT)
            .unwrap()
            .kind()
            .position(),
        Some(60.0)
    );
    let driverless = solve_assembly_joint_kinematics(&committed).unwrap();
    assert_transform_near(driverless.pose(THIRD).unwrap().local_transform(), expected);
    let repeated = solve_assembly_motion_study(&committed, STUDY).unwrap();
    assert_eq!(
        repeated.publication_batch(&committed),
        Err(AssemblyKinematicPublishError::NoCanonicalChanges)
    );
}

#[test]
fn rotated_nested_motion_study_reaches_a_tolerant_no_op_fixed_point() {
    const ROTATED_GROUP: GroupId = GroupId(90);
    let angle = 37.0_f64.to_radians();
    let cosine = angle.cos();
    let sine = angle.sin();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Rotated nested motion".into(),
            },
            CanonicalCommand::CreateGroup {
                id: ROTATED_GROUP,
                name: "Rotated group".into(),
                transform: Transform::from_matrix([
                    cosine, -sine, 0.0, 100.0, sine, cosine, 0.0, 20.0, 0.0, 0.0, 1.0, 0.0, 0.0,
                    0.0, 0.0, 1.0,
                ])
                .unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Root".into(),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Nested child".into(),
                transform: Transform::from_translation(5.0, 3.0, 0.0).unwrap(),
                parent: Some(ROTATED_GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                REVOLUTE_JOINT,
                FIRST,
                SECOND,
                revolute(Some(AssemblyJointLimits::new(-180.0, 180.0)), 10.0),
            )),
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "rotated nested study",
                vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, 20.0)],
            )),
        ]))
        .unwrap();

    let first = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    let proposal = first.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    let repeated = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    assert_eq!(
        repeated.publication_batch(&document.current()),
        Err(AssemblyKinematicPublishError::NoCanonicalChanges)
    );
}

#[test]
fn motion_study_publication_rejects_missing_stale_and_grounded_targets() {
    let mut document = seeded_with_joints();
    assert_eq!(
        solve_assembly_motion_study(&document.current(), STUDY),
        Err(AssemblyKinematicSolveError::MissingMotionStudy(STUDY))
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "guarded motion",
                vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, 45.0)],
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    let stale_proposal = solution.prepare_publication(&document).unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SECOND,
                visible: false,
            },
        ]))
        .unwrap();
    let before_stale = store_stamp(&document);
    assert_eq!(
        solution.prepare_publication(&document),
        Err(AssemblyKinematicPublishError::Stale)
    );
    assert!(matches!(
        document.commit_proposal(&stale_proposal),
        Err(ProposalCommitError::Preparation(
            ProposalPrepareError::Canonical(CanonicalError::StaleAssemblySolve)
        ))
    ));
    assert_eq!(store_stamp(&document), before_stale);

    let mut grounded = seeded_with_joints();
    grounded
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "grounded motion",
                vec![AssemblyMotionDriver::new(REVOLUTE_JOINT, 45.0)],
            )),
            CanonicalCommand::SetOccurrenceGrounded {
                id: THIRD,
                grounded: true,
            },
        ]))
        .unwrap();
    let grounded_solution = solve_assembly_motion_study(&grounded.current(), STUDY).unwrap();
    assert_eq!(
        grounded_solution.publication_batch(&grounded.current()),
        Err(AssemblyKinematicPublishError::GroundedOccurrenceWouldMove(
            THIRD
        ))
    );
}

#[test]
fn generic_telescopic_mechanism_proves_nested_limited_motion_and_swept_collision_end_to_end() {
    const OUTER_GROUP: GroupId = GroupId(100);
    const INNER_GROUP: GroupId = GroupId(101);
    const BASE: OccurrenceId = OccurrenceId(100);
    const GUIDE: OccurrenceId = OccurrenceId(101);
    const STAGE_ONE: OccurrenceId = OccurrenceId(102);
    const STAGE_TWO: OccurrenceId = OccurrenceId(103);
    const TIP: OccurrenceId = OccurrenceId(104);
    const OBSTACLE: OccurrenceId = OccurrenceId(105);
    const BASE_FIXED: AssemblyJointId = AssemblyJointId(100);
    const GUIDE_REVOLUTE: AssemblyJointId = AssemblyJointId(101);
    const EXTENSION_PRISMATIC: AssemblyJointId = AssemblyJointId(102);
    const TIP_FIXED: AssemblyJointId = AssemblyJointId(103);
    const TELESCOPIC_STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(100);
    const OVER_LIMIT_STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(101);

    let axis_x = AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Generic kinematic link".into(),
            },
            CanonicalCommand::CreateGroup {
                id: OUTER_GROUP,
                name: "Outer subassembly".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: INNER_GROUP,
                name: "Inner subassembly".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: Some(OUTER_GROUP),
            },
            CanonicalCommand::CreateOccurrence {
                id: BASE,
                definition_id: DEFINITION,
                name: "Base".into(),
                transform: Transform::from_translation(-20.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: GUIDE,
                definition_id: DEFINITION,
                name: "Guide".into(),
                transform: Transform::from_translation(-10.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: STAGE_ONE,
                definition_id: DEFINITION,
                name: "Stage one".into(),
                transform: Transform::from_translation(-130.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER_GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: STAGE_TWO,
                definition_id: DEFINITION,
                name: "Stage two".into(),
                transform: Transform::from_translation(-130.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER_GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: TIP,
                definition_id: DEFINITION,
                name: "Nested tip".into(),
                transform: Transform::from_translation(-128.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER_GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OBSTACLE,
                definition_id: DEFINITION,
                name: "Obstacle".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                BASE_FIXED,
                BASE,
                GUIDE,
                AssemblyJointKind::Fixed,
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                GUIDE_REVOLUTE,
                GUIDE,
                STAGE_ONE,
                AssemblyJointKind::Revolute {
                    axis: axis_z(),
                    limits: Some(AssemblyJointLimits::new(0.0, 0.0)),
                    position_degrees: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                EXTENSION_PRISMATIC,
                STAGE_ONE,
                STAGE_TWO,
                AssemblyJointKind::Prismatic {
                    axis: axis_x,
                    limits: Some(AssemblyJointLimits::new(0.0, 20.0)),
                    position_mm: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                TIP_FIXED,
                STAGE_TWO,
                TIP,
                AssemblyJointKind::Fixed,
            )),
        ]))
        .unwrap();

    let before_over_limit = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                OVER_LIMIT_STUDY,
                "Over-limit extension",
                vec![AssemblyMotionDriver::new(EXTENSION_PRISMATIC, 21.0)],
            )),
        ])),
        Err(CanonicalError::InvalidAssemblyMotionStudy(id)) if id == OVER_LIMIT_STUDY
    ));
    assert_eq!(store_stamp(&document), before_over_limit);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                TELESCOPIC_STUDY,
                "Generic telescopic extension",
                vec![
                    AssemblyMotionDriver::new(GUIDE_REVOLUTE, 0.0),
                    AssemblyMotionDriver::new(EXTENSION_PRISMATIC, 20.0),
                ],
            )),
        ]))
        .unwrap();
    let before_preview = store_stamp(&document);
    let unit = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap();
    let bodies = [
        AssemblyMotionCollisionBody::new(TIP, unit),
        AssemblyMotionCollisionBody::new(OBSTACLE, unit),
    ];

    let preview = preview_assembly_motion_study_clearance(
        &document.current(),
        TELESCOPIC_STUDY,
        1,
        &bodies,
        0.0,
    )
    .unwrap();
    assert_eq!(
        preview_assembly_motion_study_clearance(
            &document.current(),
            TELESCOPIC_STUDY,
            1,
            &bodies,
            0.0,
        )
        .unwrap(),
        preview
    );
    assert_eq!(store_stamp(&document), before_preview);
    assert_eq!(preview.path().samples().len(), 2);
    assert_transform_near(
        preview.path().samples()[0]
            .solution()
            .pose(TIP)
            .unwrap()
            .world_transform(),
        Transform::from_translation(-8.0, 0.0, 0.0).unwrap(),
    );
    let endpoint = preview.path().samples()[1].solution();
    assert_eq!(
        endpoint.driven_joint_positions(),
        &[(GUIDE_REVOLUTE, 0.0), (EXTENSION_PRISMATIC, 20.0)]
    );
    assert_transform_near(
        endpoint.pose(STAGE_TWO).unwrap().world_transform(),
        Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        endpoint.pose(STAGE_TWO).unwrap().local_transform(),
        Transform::from_translation(-110.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        endpoint.pose(TIP).unwrap().world_transform(),
        Transform::from_translation(12.0, 0.0, 0.0).unwrap(),
    );
    assert_transform_near(
        endpoint.pose(TIP).unwrap().local_transform(),
        Transform::from_translation(-108.0, 0.0, 0.0).unwrap(),
    );
    assert_eq!(preview.clearance().minimum_clearance_mm(), 0.0);
    let first_contact = preview.clearance().first_contact().unwrap();
    assert_eq!(
        first_contact.pair(),
        AssemblyMotionCollisionPair::new(TIP, OBSTACLE).unwrap()
    );
    assert!((first_contact.progress_start() - 0.35).abs() <= 1.0e-12);
    assert!((first_contact.progress_end() - 0.35).abs() <= 1.0e-12);

    let solution = solve_assembly_motion_study(&document.current(), TELESCOPIC_STUDY).unwrap();
    let proposal = solution.prepare_publication(&document).unwrap();
    let source_digest = document.current().canonical_digest();
    let undo_before = document.visible_undo_steps();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        document
            .current()
            .assembly_joint(EXTENSION_PRISMATIC)
            .unwrap()
            .kind()
            .position(),
        Some(20.0)
    );
    assert_transform_near(
        document.current().occurrence(TIP).unwrap().transform(),
        Transform::from_translation(-108.0, 0.0, 0.0).unwrap(),
    );

    let committed_digest = document.current().canonical_digest();
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened
            .snapshot()
            .assembly_joint(EXTENSION_PRISMATIC)
            .unwrap()
            .kind()
            .position(),
        Some(20.0)
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), source_digest);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}
