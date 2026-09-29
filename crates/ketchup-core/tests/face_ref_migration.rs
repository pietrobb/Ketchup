//! Features pick faces and edges through one `FaceRef`/`EdgeRef`: documents written
//! with separate recorded and program-named lists load into it unchanged, and every
//! pick is checked the same way whichever way it names the face.

use ketchup_core::document::{
    CanonicalCommand, CanonicalError, ChamferMode, CommandBatch, DefinitionId, Dimension,
    DocumentStore, EdgeFinishKind, EdgeRef, FaceRef, FeatureId, FeatureKind, ProfileEdgeReference,
    ProfileFaceReference, ShellDirection, Snapshot,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_core::persistence;
use ketchup_core::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceStability,
};

/// Written by the retired field-by-field codec: an extrusion (10) with a shell (11),
/// fillet (12) and face offset (13) picked by recorded topology, then a shell (14),
/// chamfer (15) and face offset (16) picked by program face names.
const FIXTURE: &[u8] = include_bytes!("fixtures/persistence/legacy/face-ref-records-schema97.bin");
/// The canonical digest the document had when the retired codec wrote it.
const DIGEST: &str = "d8a7904d3e3f5aed";
const DEFINITION: DefinitionId = DefinitionId(1);

fn side() -> ProfileFaceReference {
    ProfileFaceReference::Segment {
        entity_id: 2,
        source_name: "Profile".into(),
    }
}

fn recorded(face: &FaceRef, kind: TopologicalElementKind, producer: u64) -> bool {
    face.topological()
        .is_some_and(|face| face.kind == kind && face.producer_feature_id == FeatureId(producer))
}

#[test]
fn old_recorded_and_named_pick_records_load_as_one_reference_kind() {
    let loaded = persistence::load(FIXTURE).unwrap();
    assert!(loaded.migration_losses().is_empty());
    // The picks hash, under the frozen pre-serde digest, to what the old records did.
    assert_eq!(loaded.audit().source_canonical_digest, DIGEST);
    let snapshot = loaded.snapshot();
    let kind = |id| snapshot.feature(FeatureId(id)).unwrap().kind();

    let FeatureKind::Shell {
        removed_faces,
        direction: ShellDirection::Outward,
        ..
    } = kind(11)
    else {
        panic!("recorded shell lost its direction");
    };
    assert!(
        matches!(removed_faces.as_slice(), [face] if recorded(face, TopologicalElementKind::Face, 10))
    );
    let FeatureKind::EdgeFinish { edges, .. } = kind(12) else {
        panic!("recorded fillet changed kind");
    };
    assert!(matches!(
        edges.as_slice(),
        [EdgeRef::Topological(edge)]
            if edge.kind == TopologicalElementKind::Edge && edge.producer_feature_id == FeatureId(11)
    ));
    let FeatureKind::FaceOffset { face, .. } = kind(13) else {
        panic!("recorded offset changed kind");
    };
    assert!(recorded(face, TopologicalElementKind::Face, 12));

    assert!(matches!(
        kind(14),
        FeatureKind::Shell { removed_faces, direction: ShellDirection::Inward, .. }
            if removed_faces == &[FaceRef::Named(ProfileFaceReference::End), FaceRef::Named(side())]
    ));
    assert!(matches!(
        kind(15),
        FeatureKind::EdgeFinish { edges, kind: EdgeFinishKind::Chamfer, .. }
            if edges == &[EdgeRef::Named(ProfileEdgeReference {
                first: ProfileFaceReference::Start,
                second: side(),
            })]
    ));
    assert!(matches!(
        kind(16),
        FeatureKind::FaceOffset { face: FaceRef::Named(ProfileFaceReference::NamedResult(name)), .. }
            if name == "Named shell"
    ));

    // The exact kernel still receives recorded picks as selectors and named picks by name.
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, FeatureId(16)).unwrap();
    let shells = graph
        .nodes
        .iter()
        .filter_map(|node| match &node.operation {
            ExactBRepOperation::Shell {
                removed_faces,
                profile_faces,
                ..
            } => Some((removed_faces.len(), profile_faces.len())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(shells, [(1, 0), (0, 2)]);
    let offsets = graph
        .nodes
        .iter()
        .filter_map(|node| match &node.operation {
            ExactBRepOperation::FaceOffset {
                face, profile_face, ..
            } => Some((face.is_some(), profile_face.is_some())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(offsets, [(true, false), (false, true)]);

    let reloaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        reloaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
}

fn reference(
    snapshot: &Snapshot,
    producer: u64,
    kind: TopologicalElementKind,
) -> TopologicalElementRef {
    TopologicalElementRef::new(
        snapshot.document_id(),
        DEFINITION,
        FeatureId(producer),
        FeatureId(producer),
        kind,
        format!("generated-source/{}/1", kind.token()),
        format!("generated-result/{}/1", kind.token()),
        TopologicalReferenceStability::Guaranteed,
        "ketchup.exact-brep-graph-evaluator.v1",
        "occt.v1",
        "1e-7-mm",
        format!("result-{producer}"),
        format!("geometry-{}-1", kind.token()),
    )
    .unwrap()
}

fn extruded_block() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Block".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(9),
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 30.0], [0.0, 30.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(10),
                definition_id: DEFINITION,
                name: "Extrusion".into(),
                kind: FeatureKind::extrusion(FeatureId(9), Dimension::from_decimal("10").unwrap()),
            },
        ]))
        .unwrap();
    document
}

fn create(document: &mut DocumentStore, kind: FeatureKind) -> Result<(), CanonicalError> {
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(11),
            definition_id: DEFINITION,
            name: "Pick".into(),
            kind,
        }]))
        .map(|_| ())
}

#[test]
fn a_feature_picks_all_its_faces_or_edges_the_same_way() {
    let mut document = extruded_block();
    let snapshot = document.current();
    let mixed_shell = FeatureKind::Shell {
        target: FeatureId(10),
        removed_faces: vec![
            FaceRef::from(reference(&snapshot, 10, TopologicalElementKind::Face)),
            FaceRef::Named(ProfileFaceReference::End),
        ],
        thickness: Dimension::from_decimal("1").unwrap(),
        direction: ShellDirection::Inward,
    };
    assert_eq!(
        create(&mut document, mixed_shell),
        Err(CanonicalError::InvalidTopologicalFeatureReference)
    );
    let mixed_finish = FeatureKind::EdgeFinish {
        target: FeatureId(10),
        edges: vec![
            EdgeRef::from(reference(&snapshot, 10, TopologicalElementKind::Edge)),
            EdgeRef::Named(ProfileEdgeReference {
                first: ProfileFaceReference::Start,
                second: side(),
            }),
        ],
        kind: EdgeFinishKind::Fillet,
        amount: Dimension::from_decimal("1").unwrap(),
        fillet_radius_stations: Vec::new(),
        chamfer_mode: ChamferMode::Symmetric,
        chamfer_edge_sides: Vec::new(),
    };
    assert_eq!(
        create(&mut document, mixed_finish),
        Err(CanonicalError::InvalidTopologicalFeatureReference)
    );
    assert_eq!(document.current().revision_id(), snapshot.revision_id());
}

#[test]
fn a_shell_without_removed_faces_is_a_closed_shell() {
    for direction in [ShellDirection::Inward, ShellDirection::Outward] {
        let mut document = extruded_block();
        let closed = FeatureKind::Shell {
            target: FeatureId(10),
            removed_faces: Vec::new(),
            thickness: Dimension::from_decimal("1").unwrap(),
            direction,
        };
        assert_eq!(create(&mut document, closed), Ok(()));
    }
}

#[test]
fn a_named_pick_needs_the_same_solid_target_as_a_recorded_one() {
    // Both ways of naming the face are checked against the target: a profile
    // is not a solid to offset a face of.
    let named = |_: &Snapshot| FaceRef::Named(ProfileFaceReference::End);
    let recorded =
        |snapshot: &Snapshot| FaceRef::from(reference(snapshot, 9, TopologicalElementKind::Face));
    for pick in [&named as &dyn Fn(&Snapshot) -> FaceRef, &recorded] {
        let mut document = extruded_block();
        let offset = FeatureKind::FaceOffset {
            target: FeatureId(9),
            face: pick(&document.current()),
            distance: Dimension::from_decimal("1").unwrap(),
        };
        assert_eq!(
            create(&mut document, offset),
            Err(CanonicalError::InvalidFeatureOwnership(FeatureId(11)))
        );
    }
    let mut document = extruded_block();
    let offset = FeatureKind::FaceOffset {
        target: FeatureId(10),
        face: FaceRef::Named(ProfileFaceReference::End),
        distance: Dimension::from_decimal("1").unwrap(),
    };
    assert_eq!(create(&mut document, offset), Ok(()));
}
