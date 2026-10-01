use super::*;

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn bend(parent: Option<usize>, edge: usize, length: f64, angle_degrees: f64) -> SheetMetalBend {
    SheetMetalBend {
        parent,
        edge,
        length: dimension(length),
        angle_degrees,
        inner_radius: dimension(3.0),
    }
}

fn sheet(base_mm: Vec<[f64; 2]>, bends: Vec<SheetMetalBend>) -> SheetMetalSpec {
    SheetMetalSpec {
        base_mm,
        thickness: dimension(2.0),
        k_factor: 0.4,
        bends,
    }
}

fn rectangle() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]]
}

/// A quadrilateral whose edge 1 runs at a slant from (100, 0) to (70, 60).
fn slanted() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [100.0, 0.0], [70.0, 60.0], [0.0, 60.0]]
}

fn close(left: Vec3, right: [f64; 3]) -> bool {
    (0..3).all(|axis| (left.component(axis) - right[axis]).abs() < 1.0e-9)
}

#[test]
fn flat_pattern_preserves_neutral_area_and_signed_opposite_bends() {
    let spec = sheet(
        rectangle(),
        vec![bend(None, 1, 30.0, -45.0), bend(None, 3, 20.0, 90.0)],
    );
    let pattern = spec.flat_pattern().unwrap();
    let max_x = spec.bend_allowance_mm(0).unwrap();
    let min_x = spec.bend_allowance_mm(1).unwrap();

    assert_eq!(pattern.base_corners_mm, rectangle());
    assert_eq!(
        pattern.blank_bounds_mm,
        [[-min_x - 20.0, 0.0], [100.0 + max_x + 30.0, 50.0]]
    );
    assert_eq!(pattern.outline_mm.len(), 4, "{:?}", pattern.outline_mm);
    assert_eq!(pattern.bends[0].angle_degrees, -45.0);
    assert_eq!(pattern.bends[1].angle_degrees, 90.0);
    assert_eq!(
        pattern.material_area_mm2,
        spec.folded_neutral_area_mm2().unwrap()
    );
    assert!((shoelace_area(&pattern.outline_mm) - pattern.material_area_mm2).abs() < 1.0e-9);
}

#[test]
fn bends_on_edges_sharing_a_corner_are_rejected_as_self_intersecting() {
    let spec = sheet(
        rectangle(),
        vec![bend(None, 0, 20.0, 90.0), bend(None, 3, 20.0, 90.0)],
    );
    assert_eq!(
        spec.flat_pattern(),
        Err(SheetMetalError::AdjacentFlangesRequireCornerRelief)
    );
    let on_flange_side = sheet(
        rectangle(),
        vec![bend(None, 1, 20.0, 90.0), bend(Some(0), 1, 10.0, 90.0)],
    );
    assert_eq!(
        on_flange_side.validate(),
        Err(SheetMetalError::AdjacentFlangesRequireCornerRelief)
    );
}

#[test]
fn the_bend_tree_is_canonical_and_names_existing_faces_and_edges() {
    for (bends, error) in [
        (
            vec![bend(Some(1), 2, 10.0, 90.0), bend(None, 1, 20.0, 90.0)],
            SheetMetalError::InvalidBendParent,
        ),
        (
            vec![bend(None, 1, 20.0, 90.0), bend(Some(0), 0, 10.0, 90.0)],
            SheetMetalError::DuplicateEdge,
        ),
        (
            vec![bend(None, 1, 20.0, 90.0), bend(None, 1, 10.0, 90.0)],
            SheetMetalError::DuplicateEdge,
        ),
        (
            vec![bend(None, 3, 20.0, 90.0), bend(None, 1, 10.0, 90.0)],
            SheetMetalError::NonCanonicalBendOrder,
        ),
        (
            vec![bend(None, 4, 20.0, 90.0)],
            SheetMetalError::UnknownEdge,
        ),
    ] {
        assert_eq!(sheet(rectangle(), bends).validate(), Err(error));
    }
    let clockwise = rectangle().into_iter().rev().collect();
    assert_eq!(
        sheet(clockwise, Vec::new()).validate(),
        Err(SheetMetalError::InvalidBase)
    );
    let crossed = vec![[0.0, 0.0], [100.0, 50.0], [100.0, 0.0], [0.0, 50.0]];
    assert_eq!(
        sheet(crossed, Vec::new()).validate(),
        Err(SheetMetalError::InvalidBase)
    );
    let thin = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 1.5], [0.0, 1.5]];
    assert_eq!(
        sheet(thin, Vec::new()).validate(),
        Err(SheetMetalError::ThicknessExceedsBase)
    );
}

#[test]
fn a_bend_on_a_slanted_edge_carries_a_further_bend_folded_and_unrolled() {
    let spec = sheet(
        slanted(),
        vec![bend(None, 1, 20.0, 90.0), bend(Some(0), 2, 10.0, 90.0)],
    );
    let shape = spec.shape();
    let folded = shape.fold().unwrap();
    let span = 30.0_f64.hypot(60.0);
    let edge = Vec3::new(-30.0, 60.0, 0.0).normalized().unwrap();
    let outward = Vec3::new(edge.y, -edge.x, 0.0);
    // The first flange stands up along +z over the slanted edge, its normal facing the base.
    assert!(close(folded[0].flange.x, [0.0, 0.0, 1.0]));
    assert!(close(folded[0].flange.y, edge.to_array()));
    assert!(close(folded[0].flange.z, (-outward).to_array()));
    // Its flange starts at the outer face of the bend: inner radius plus thickness out of the edge.
    assert!(close(
        folded[0].flange.origin,
        (Vec3::new(100.0, 0.0, 0.0) + outward * 5.0 + Vec3::new(0.0, 0.0, 5.0)).to_array()
    ));
    // The second bend folds the far edge back over the base: a channel lip.
    assert!(close(folded[1].flange.x, (-outward).to_array()));
    assert!(close(folded[1].flange.y, edge.to_array()));
    assert!((folded[1].span_mm - span).abs() < 1.0e-9);

    let pattern = spec.flat_pattern().unwrap();
    let first = &pattern.bends[0];
    let second = &pattern.bends[1];
    let line = |bend: &SheetMetalFlatBend| {
        [
            bend.bend_line_end_mm[0] - bend.bend_line_start_mm[0],
            bend.bend_line_end_mm[1] - bend.bend_line_start_mm[1],
        ]
    };
    let (first_line, second_line) = (line(first), line(second));
    assert!(
        (first_line[0] * second_line[1] - first_line[1] * second_line[0]).abs() < 1.0e-9,
        "unrolled bend lines stay parallel"
    );
    assert!((first_line[0].hypot(first_line[1]) - span).abs() < 1.0e-9);
    assert!((pattern.material_area_mm2 - spec.folded_neutral_area_mm2().unwrap()).abs() < 1.0e-9);
    assert!((shoelace_area(&pattern.outline_mm) - pattern.material_area_mm2).abs() < 1.0e-6);
    let bounds = shape.folded_bounds_mm().unwrap();
    assert!(bounds[0][2] <= 0.0 && bounds[1][2] >= 5.0 + 20.0);
}

#[test]
fn manufacturing_export_is_associative_tamper_evident_and_history_stable() {
    use crate::document::{
        CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureParameterTarget,
        ParameterValueType,
    };
    use crate::persistence::{ContainerData, load, save_document_store};

    let feature_id = FeatureId(41);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(40),
                name: "Manufactured bracket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id: DefinitionId(40),
                name: "Opposite flanges".into(),
                kind: FeatureKind::SheetMetal(sheet(
                    rectangle(),
                    vec![bend(None, 1, 30.0, -45.0), bend(None, 3, 20.0, 90.0)],
                )),
            },
        ]))
        .unwrap();
    let original_projection =
        project_sheet_metal_manufacturing(&document.current(), feature_id).unwrap();
    let original = original_projection.artifacts(&document.current()).unwrap();
    let dxf = String::from_utf8(original.flat_pattern_dxf.clone()).unwrap();
    let bends = String::from_utf8(original.bend_table_csv.clone()).unwrap();
    assert!(dxf.contains(SHEET_METAL_FLAT_PATTERN_DXF_V1));
    assert_eq!(dxf.matches("0\nLINE\n8\nCUT\n").count(), 4);
    assert_eq!(dxf.matches("0\nLINE\n8\nBEND_UP\n").count(), 1);
    assert_eq!(dxf.matches("0\nLINE\n8\nBEND_DOWN\n").count(), 1);
    assert!(bends.contains("\n0,base,1,down,-45,3,"), "{bends}");
    assert!(bends.contains("\n1,base,3,up,90,3,"), "{bends}");

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: FeatureParameterTarget::new(
                    feature_id,
                    "bends.1.length",
                    ParameterValueType::Length,
                )
                .unwrap(),
                dimension: dimension(40.0),
            },
        ]))
        .unwrap();
    assert_eq!(
        original_projection.artifacts(&document.current()),
        Err(SheetMetalManufacturingExportError::StaleProjection)
    );
    let edited_projection =
        project_sheet_metal_manufacturing(&document.current(), feature_id).unwrap();
    let edited = edited_projection.artifacts(&document.current()).unwrap();
    assert_ne!(edited, original);

    let mut tampered = edited_projection.clone();
    tampered.flat_pattern.bends[0].bend_allowance_mm += 1.0;
    assert_eq!(
        tampered.artifacts(&document.current()),
        Err(SheetMetalManufacturingExportError::TamperedProjection)
    );
    let mut tampered_digest = edited_projection.clone();
    tampered_digest.result_digest.replace_range(..1, "0");
    assert_eq!(
        tampered_digest.artifacts(&document.current()),
        Err(SheetMetalManufacturingExportError::TamperedProjection)
    );

    document.undo().unwrap();
    assert_eq!(
        original_projection.artifacts(&document.current()).unwrap(),
        original
    );
    document.redo().unwrap();
    assert_eq!(
        edited_projection.artifacts(&document.current()).unwrap(),
        edited
    );

    let saved = save_document_store(&document, &ContainerData::default()).unwrap();
    let (mut reopened, _) = load(&saved)
        .unwrap()
        .into_editable_with_container()
        .ok()
        .expect("sheet metal must reopen editable");
    assert_eq!(
        project_sheet_metal_manufacturing(&reopened.current(), feature_id)
            .unwrap()
            .artifacts(&reopened.current())
            .unwrap(),
        edited
    );
    reopened.undo().unwrap();
    assert_eq!(
        project_sheet_metal_manufacturing(&reopened.current(), feature_id)
            .unwrap()
            .artifacts(&reopened.current())
            .unwrap(),
        original
    );
}
