use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    PrincipalPlane, SketchError, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};
use serde_json::json;

fn frame() -> WorkplaneFrame {
    WorkplaneFrame::from_axes([120.0, -40.0, 70.0], [0.8, 0.6, 0.0], [0.0, 0.0, 1.0]).unwrap()
}

fn document(spec: WorkplaneSpec) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Frame test".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Workplane".into(),
                kind: FeatureKind::Workplane(spec),
            },
        ]))
        .unwrap();
    document
}

#[test]
fn free_frame_is_dependency_free_lossless_and_digest_authoritative() {
    let spec = WorkplaneSpec {
        support: WorkplaneSupport::Free,
        frame: frame(),
    };
    assert_eq!(spec.frame.normal, [0.6, -0.8, 0.0]);
    assert!(
        FeatureKind::Workplane(spec.clone())
            .dependencies()
            .is_empty()
    );
    let mut document = document(spec.clone());
    let snapshot = document.current();
    let bytes = persistence::save(&snapshot);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), 52);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        reopened.snapshot().feature(FeatureId(1)).unwrap().kind(),
        &FeatureKind::Workplane(spec.clone())
    );
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(document.current().features().count(), 0);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );

    // The support tag and all frame coordinates affect canonical identity.
    let original = self::document(spec.clone()).current().canonical_digest();
    let mut translated = spec.clone();
    translated.frame.origin_mm[0] += 1.0;
    assert_ne!(
        original,
        self::document(translated).current().canonical_digest()
    );
    let mut rotated = spec;
    rotated.frame =
        WorkplaneFrame::from_axes(frame().origin_mm, [0.0, 0.0, 1.0], [0.8, 0.6, 0.0]).unwrap();
    assert_ne!(
        original,
        self::document(rotated).current().canonical_digest()
    );
    let principal = WorkplaneSpec::principal(PrincipalPlane::Xy);
    let free_xy = WorkplaneSpec {
        support: WorkplaneSupport::Free,
        frame: principal.frame,
    };
    assert_ne!(
        self::document(principal).current().canonical_digest(),
        self::document(free_xy).current().canonical_digest()
    );
}

#[test]
fn canonical_free_frames_reject_invalid_inputs_atomically_and_do_not_relax_principal() {
    let valid = frame();
    let mut invalid = Vec::new();
    for value in [f64::NAN, f64::INFINITY, 1_000_001.0] {
        let mut candidate = valid;
        candidate.origin_mm[0] = value;
        invalid.push(candidate);
    }
    let mut candidate = valid;
    candidate.x_axis = [0.0; 3];
    invalid.push(candidate);
    candidate = valid;
    candidate.x_axis = [1.6, 1.2, 0.0];
    invalid.push(candidate);
    candidate = valid;
    candidate.y_axis = candidate.x_axis;
    invalid.push(candidate);
    candidate = valid;
    candidate.normal = candidate.normal.map(|v| -v);
    invalid.push(candidate);
    for frame in invalid {
        let spec = WorkplaneSpec {
            support: WorkplaneSupport::Free,
            frame,
        };
        assert_eq!(
            spec.validate_local(),
            Err(SketchError::InvalidWorkplaneFrame)
        );
        let mut document = DocumentStore::new();
        let before = document.current().canonical_digest();
        assert!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateDefinition {
                        id: DefinitionId(1),
                        name: "Rejected".into()
                    },
                    CanonicalCommand::CreateFeature {
                        id: FeatureId(1),
                        definition_id: DefinitionId(1),
                        name: "Invalid frame".into(),
                        kind: FeatureKind::Workplane(spec),
                    },
                ]))
                .is_err()
        );
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), 0);
    }
    assert_eq!(
        WorkplaneSpec {
            support: WorkplaneSupport::Principal(PrincipalPlane::Xy),
            frame: valid
        }
        .validate_local(),
        Err(SketchError::InvalidWorkplaneFrame)
    );
}

#[test]
fn schema_51_keeps_principal_offset_semantics_and_rejects_the_new_tag() {
    let mut legacy = document(WorkplaneSpec::principal(PrincipalPlane::Xy));
    legacy
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(1),
            name: "Offset".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset {
                    base: FeatureId(1),
                    distance: ketchup_core::document::Dimension::from_decimal("5").unwrap(),
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(5.0),
            }),
        }]))
        .unwrap();
    let snapshot = legacy.current();
    let mut bytes = persistence::save(&snapshot);
    // Only the envelope version changes; the legacy payload and checksum stay intact.
    bytes[10..12].copy_from_slice(&51_u16.to_le_bytes());
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(loaded.source_schema(), 51);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    for id in [FeatureId(1), FeatureId(2)] {
        assert_eq!(
            loaded.snapshot().feature(id).unwrap().kind(),
            snapshot.feature(id).unwrap().kind()
        );
    }
    let free = document(WorkplaneSpec {
        support: WorkplaneSupport::Free,
        frame: frame(),
    });
    let mut bytes = persistence::save(&free.current());
    bytes[10..12].copy_from_slice(&51_u16.to_le_bytes());
    assert!(matches!(
        persistence::load(&bytes),
        Err(persistence::PersistenceError::InvalidFeatureKind(4))
    ));
}

fn program(workplane: AssistantWorkplaneSpec) -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1,
            name: "Frame sketch".into(),
            workplane,
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 1.0,
            }],
            constraints: vec![],
        }],
    }
}

#[test]
fn public_frame_schema_is_exact_and_uses_strict_canonical_validation() {
    let value = json!({"type":"frame","origin_mm":[120.0,-40.0,70.0],"x_axis":[0.8,0.6,0.0],"y_axis":[0.0,0.0,1.0]});
    let spec: AssistantWorkplaneSpec = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&spec).unwrap(), value);
    program(spec).validate().unwrap();
    for (key, bad) in [
        ("normal", json!([0, 0, 1])),
        ("origin_mm", json!([0, 0])),
        ("x_axis", json!([1, 0, 0, 0])),
        ("type", json!("free")),
    ] {
        let mut invalid = value.clone();
        invalid[key] = bad;
        assert!(serde_json::from_value::<AssistantWorkplaneSpec>(invalid).is_err());
    }
    let mut missing = value;
    missing.as_object_mut().unwrap().remove("y_axis");
    assert!(serde_json::from_value::<AssistantWorkplaneSpec>(missing).is_err());
    for (origin_mm, x_axis, y_axis) in [
        ([f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1_000_001.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0]),
        ([0.0; 3], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ([0.0; 3], [1.0, 0.0, 0.0], [0.6, 0.8, 0.0]),
        ([0.0; 3], [f64::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ] {
        assert!(
            program(AssistantWorkplaneSpec::Frame {
                origin_mm,
                x_axis,
                y_axis
            })
            .validate()
            .is_err()
        );
    }
}
