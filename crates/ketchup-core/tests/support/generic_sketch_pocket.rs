use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PrincipalPlane, SketchEntity, SketchEntityId,
    SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};

pub const DEFINITION: DefinitionId = DefinitionId(1);
pub const PRINCIPAL: FeatureId = FeatureId(10);
pub const PLANE: FeatureId = FeatureId(11);
pub const BASE_SKETCH: FeatureId = FeatureId(12);
pub const PAD: FeatureId = FeatureId(13);
pub const CUT_SKETCH: FeatureId = FeatureId(14);
pub const POCKET: FeatureId = FeatureId(15);

pub fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

pub fn rectangle(workplane: FeatureId, min: [f64; 2], max: [f64; 2]) -> SketchSpec {
    let points = [min, [max[0], min[1]], max, [min[0], max[1]]];
    SketchSpec {
        workplane,
        entities: (0..4)
            .map(|index| SketchEntity::Line {
                id: SketchEntityId(index as u64 + 1),
                start_mm: points[index],
                end_mm: points[(index + 1) % 4],
            })
            .collect(),
        constraints: vec![],
    }
}

pub fn feature(id: FeatureId, kind: FeatureKind) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id,
        definition_id: DEFINITION,
        name: format!("Feature {}", id.0),
        kind,
    }
}

pub fn pocket(profile: FeatureId, depth: f64) -> CanonicalCommand {
    feature(
        POCKET,
        FeatureKind::Pocket {
            target: PAD,
            profile,
            depth: dimension(depth),
        },
    )
}

pub fn base_document(plane: PrincipalPlane, offset: f64) -> DocumentStore {
    let sketch = rectangle(PLANE, [0.0, 0.0], [400.0, 200.0]);
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Generic part".into(),
            },
            feature(
                PRINCIPAL,
                FeatureKind::Workplane(WorkplaneSpec::principal(plane)),
            ),
            feature(
                PLANE,
                FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Offset {
                        base: PRINCIPAL,
                        distance: dimension(offset),
                    },
                    frame: WorkplaneFrame::principal(plane).offset(offset),
                }),
            ),
            feature(BASE_SKETCH, FeatureKind::Sketch(sketch)),
            feature(
                PAD,
                FeatureKind::Pad(PadSpec {
                    sketch: BASE_SKETCH,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(20.0)),
                }),
            ),
        ]))
        .unwrap();
    document
}

pub fn document(plane: PrincipalPlane, offset: f64) -> DocumentStore {
    let mut document = base_document(plane, offset);
    document
        .apply_batch(&CommandBatch::new(vec![
            feature(
                CUT_SKETCH,
                FeatureKind::Sketch(rectangle(PLANE, [40.0, 40.0], [80.0, 80.0])),
            ),
            pocket(CUT_SKETCH, 20.0),
        ]))
        .unwrap();
    document
}
