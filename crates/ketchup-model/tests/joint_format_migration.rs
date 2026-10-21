//! Format 103 lets a joint join placed parts. A format-102 file, whose joints all
//! join values a rule derives, opens with the same joints in every revision and
//! under the digest its writer computed, and saves them unchanged in format 103.
//! A joint of placed parts needs both parts and goes with a deleted one.
use ketchup_geometry::prismatic::{Aabb, CanonicalJoint, JointId};
use ketchup_model::document::*;
use ketchup_model::graph::{DerivedIdentity, SlotPath, SlotSegment};
use ketchup_model::persistence::{self, ContainerData, LoadOutcome};

/// Written by format 102: a definition, joints 1 and 2, then joint 1 widened.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/rule-joints-format102.bin"
);

/// The digest format 102 computed for the current (last) revision.
const WRITER_DIGEST: &str = "e656ec99db7ef94a";

fn identity(key: &str) -> JointParticipant {
    DerivedIdentity::new(
        NodeId(7),
        SlotPath::new(vec![SlotSegment::new(NodeId(7), "parts", key).unwrap()]).unwrap(),
    )
    .unwrap()
    .into()
}

fn joint(id: u64, a: &str, b: &str, min: [f64; 3], max: [f64; 3]) -> DocumentJoint {
    CanonicalJoint::new(
        JointId(id),
        identity(a),
        identity(b),
        Aabb::new(min, max).unwrap(),
    )
    .unwrap()
}

/// The joints of the revision format 102 wrote last.
fn written_joints() -> Vec<DocumentJoint> {
    vec![
        joint(1, "bok-lavy", "dno", [0., 0., 0.], [18., 560., 18.]),
        joint(2, "bok-pravy", "dno", [782., 0., 0.], [800., 500., 18.]),
    ]
}

fn joints(document: &DocumentStore) -> Vec<DocumentJoint> {
    document.current().joints().cloned().collect()
}

fn editable(bytes: &[u8]) -> (DocumentStore, persistence::LoadAudit) {
    let LoadOutcome::Editable {
        document, audit, ..
    } = persistence::load(bytes).unwrap()
    else {
        panic!("the file is editable")
    };
    (document, audit)
}

#[test]
fn format_102_rule_joints_open_unchanged_and_save_as_format_103() {
    let (document, audit) = editable(&std::fs::read(FIXTURE).unwrap());
    assert_eq!(audit.source_schema, 102);
    assert_eq!(audit.history_discarded(), None);
    assert_eq!(audit.source_canonical_digest, WRITER_DIGEST);
    assert_eq!(joints(&document), written_joints());
    assert_eq!(document.visible_undo_steps(), 0);

    let saved = persistence::save_document_store(&document, &ContainerData::default()).unwrap();
    let (reopened, audit) = editable(&saved);
    assert_eq!(audit.source_schema, persistence::CURRENT_SCHEMA);
    assert_eq!(audit.history_discarded(), None);
    assert_eq!(joints(&reopened), written_joints());
}

fn part_joint(a: u64, b: u64) -> DocumentJoint {
    CanonicalJoint::new(
        JointId(5),
        InstancePath::root(OccurrenceId(a)).into(),
        InstancePath::root(OccurrenceId(b)).into(),
        Aabb::new([0., 0., 0.], [10., 10., 10.]).unwrap(),
    )
    .unwrap()
}

#[test]
fn a_part_joint_needs_placed_parts_and_goes_with_a_deleted_part() {
    let mut document = DocumentStore::new();
    let mut commands = vec![CanonicalCommand::CreateDefinition {
        id: DefinitionId(1),
        name: "Doska".into(),
    }];
    for id in [20, 21] {
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(1),
            name: format!("Diel {id}"),
            transform: Transform::identity(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let before = document.current().canonical_digest();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            part_joint(20, 99)
        )])),
        Err(CanonicalError::JointPartNotFound {
            joint: JointId(5),
            ..
        })
    ));
    assert_eq!(document.current().canonical_digest(), before);

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            part_joint(20, 21),
        )]))
        .unwrap();
    assert_eq!(joints(&document), [part_joint(20, 21)]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(21),
            },
        ]))
        .unwrap();
    assert!(joints(&document).is_empty());
    document.undo().unwrap();
    assert_eq!(joints(&document), [part_joint(20, 21)]);
}
