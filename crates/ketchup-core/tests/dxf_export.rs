use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, ProfileSegment, Transform,
};
use ketchup_core::dxf_export::{DxfProfileExportError, export_visible_profiles_dxf};
use ketchup_core::import::{DxfImportOptions, inspect_dxf, plan_dxf_import};

fn representative_dxf() -> Vec<u8> {
    b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n\
0\nLWPOLYLINE\n8\ncut\n90\n4\n70\n1\n10\n0\n20\n0\n10\n4\n20\n0\n42\n1\n10\n4\n20\n4\n10\n0\n20\n4\n\
0\nCIRCLE\n8\nbores\n10\n20\n20\n20\n40\n2\n\
0\nENDSEC\n0\nEOF\n"
        .to_vec()
}

fn commit(document: &mut DocumentStore, batch: CommandBatch) {
    let proposal = document.prepare_proposal(batch).unwrap();
    document.commit_verified_proposal(&proposal).unwrap();
}

#[test]
fn visible_profile_dxf_round_trip_is_deterministic_layered_and_non_mutating() {
    let source = representative_dxf();
    let mut document = DocumentStore::new();
    let batch = plan_dxf_import(
        &document.current(),
        &source,
        "representative.dxf",
        DxfImportOptions::new(None),
    )
    .unwrap();
    commit(&mut document, batch);
    commit(
        &mut document,
        CommandBatch::new(vec![CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(1),
            transform: Transform::from_translation(50.0, 25.0, 0.0).unwrap(),
        }]),
    );
    let snapshot = document.current();
    let before = snapshot.canonical_digest();

    let exported = export_visible_profiles_dxf(&snapshot).unwrap();
    let repeated = export_visible_profiles_dxf(&snapshot).unwrap();
    assert_eq!(exported, repeated);
    assert_eq!(snapshot.canonical_digest(), before);
    assert!(exported.dxf.starts_with(b"0\nSECTION\n2\nHEADER\n"));
    assert!(
        exported
            .loss_report
            .contains(&format!("source_digest={before}"))
    );
    assert!(exported.loss_report.contains("unit=millimetre"));
    assert!(exported.loss_report.contains("dwg=unsupported"));

    let original = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    let round_trip = inspect_dxf(&exported.dxf, DxfImportOptions::new(None)).unwrap();
    assert_eq!(round_trip.layers(), original.layers());
    assert_eq!(round_trip.profiles().len(), original.profiles().len());
    assert_eq!(
        round_trip
            .profiles()
            .iter()
            .map(|profile| (profile.layer(), profile.closed(), profile.segments().len()))
            .collect::<Vec<_>>(),
        vec![("bores", true, 2), ("cut", true, 4), ("outline", false, 2),]
    );
    let moved = round_trip
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "bores")
        .unwrap();
    assert!(moved.segments()[0].start_mm()[0] >= 68.0);
    assert!(moved.segments()[0].start_mm()[1] >= 43.0);
}

#[test]
fn unsupported_curves_and_non_planar_occurrences_fail_closed() {
    let mut bezier_document = DocumentStore::new();
    commit(
        &mut bezier_document,
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "curves".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "curve".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::CubicBezier {
                        start_mm: [0.0, 0.0],
                        control_1_mm: [1.0, 2.0],
                        control_2_mm: [2.0, 2.0],
                        end_mm: [3.0, 0.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "curve".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]),
    );
    assert_eq!(
        export_visible_profiles_dxf(&bezier_document.current()),
        Err(DxfProfileExportError::UnsupportedCurve)
    );

    let source = representative_dxf();
    let mut non_planar_document = DocumentStore::new();
    let batch = plan_dxf_import(
        &non_planar_document.current(),
        &source,
        "representative.dxf",
        DxfImportOptions::new(None),
    )
    .unwrap();
    commit(&mut non_planar_document, batch);
    commit(
        &mut non_planar_document,
        CommandBatch::new(vec![CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(1),
            transform: Transform::from_translation(0.0, 0.0, 1.0).unwrap(),
        }]),
    );
    assert_eq!(
        export_visible_profiles_dxf(&non_planar_document.current()),
        Err(DxfProfileExportError::UnsupportedTransform)
    );
}
