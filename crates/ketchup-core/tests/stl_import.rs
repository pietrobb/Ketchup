use ketchup_core::document::{DocumentStore, FeatureKind, MeshAuthority};
use ketchup_core::import::{
    ImportFormat, ImportLengthUnit, ImportUnitAuthority, ImportUnitDecision, StlImportError,
    parse_stl, plan_stl_import,
};
use ketchup_core::persistence;

fn millimetres() -> ImportUnitDecision {
    ImportUnitDecision::new(
        ImportLengthUnit::Millimetre,
        ImportUnitAuthority::UserDeclared,
    )
}

fn tetrahedron_facets() -> [([[f32; 3]; 3], [f32; 3]); 4] {
    [
        (
            [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            [0.0, 0.0, -1.0],
        ),
        (
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [0.0, -1.0, 0.0],
        ),
        (
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            [-1.0, 0.0, 0.0],
        ),
        (
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [1.0, 1.0, 1.0],
        ),
    ]
}

fn binary_tetrahedron() -> Vec<u8> {
    let facets = tetrahedron_facets();
    let mut bytes = vec![0_u8; 80];
    bytes[..21].copy_from_slice(b"deterministic binary ");
    bytes.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for (vertices, normal) in facets {
        for value in normal.into_iter().chain(vertices.into_iter().flatten()) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u16.to_le_bytes());
    }
    bytes
}

fn ascii_tetrahedron() -> Vec<u8> {
    let mut text = String::from("solid tetrahedron\n");
    for (vertices, normal) in tetrahedron_facets() {
        text.push_str(&format!(
            " facet normal {} {} {}\n  outer loop\n",
            normal[0], normal[1], normal[2]
        ));
        for vertex in vertices {
            text.push_str(&format!(
                "   vertex {} {} {}\n",
                vertex[0], vertex[1], vertex[2]
            ));
        }
        text.push_str("  endloop\n endfacet\n");
    }
    text.push_str("endsolid tetrahedron\n");
    text.into_bytes()
}

#[test]
fn binary_and_ascii_stl_normalize_to_the_same_canonical_mesh() {
    let binary = parse_stl(&binary_tetrahedron(), millimetres()).unwrap();
    let ascii = parse_stl(&ascii_tetrahedron(), millimetres()).unwrap();

    assert_eq!(binary.vertices_mm(), ascii.vertices_mm());
    assert_eq!(binary.triangles(), ascii.triangles());
    assert_eq!(binary.vertices_mm().len(), 4);
    assert_eq!(binary.triangles().len(), 4);
    assert_eq!(binary.diagnostics()[0].code(), "stl.binary");
    assert_eq!(ascii.diagnostics()[0].code(), "stl.ascii");
}

#[test]
fn declared_units_are_applied_before_canonical_validation() {
    let inches = ImportUnitDecision::new(ImportLengthUnit::Inch, ImportUnitAuthority::UserDeclared);
    let mesh = parse_stl(&binary_tetrahedron(), inches).unwrap();
    assert!(
        mesh.vertices_mm()
            .iter()
            .flatten()
            .all(|value| matches!(*value, 0.0 | 25.4))
    );
}

#[test]
fn invalid_topology_and_structure_are_actionable_refusals() {
    let mut open = ascii_tetrahedron();
    let start = std::str::from_utf8(&open)
        .unwrap()
        .find(" facet normal 1 1 1")
        .unwrap();
    open.truncate(start);
    open.extend_from_slice(b"endsolid tetrahedron\n");
    assert_eq!(
        parse_stl(&open, millimetres()),
        Err(StlImportError::NonManifoldEdge)
    );

    let mut truncated = binary_tetrahedron();
    truncated.pop();
    assert_eq!(
        parse_stl(&truncated, millimetres()),
        Err(StlImportError::InvalidBinaryLength)
    );

    let mut inverted = ascii_tetrahedron();
    let text = String::from_utf8(inverted).unwrap();
    inverted = text
        .replace(
            "vertex 0 1 0\n   vertex 1 0 0",
            "vertex 1 0 0\n   vertex 0 1 0",
        )
        .into_bytes();
    assert!(matches!(
        parse_stl(&inverted, millimetres()),
        Err(StlImportError::InconsistentOrientation | StlImportError::NonPositiveVolume)
    ));
}

#[test]
fn stl_plan_is_one_reviewed_persistent_undoable_transaction() {
    let source = binary_tetrahedron();
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch = plan_stl_import(
        &document.current(),
        &source,
        "tetrahedron.stl",
        millimetres(),
    )
    .unwrap();
    let repeated = plan_stl_import(
        &document.current(),
        &source,
        "tetrahedron.stl",
        millimetres(),
    )
    .unwrap();
    assert_eq!(batch.digest(), repeated.digest());

    let proposal = document.prepare_proposal(batch).unwrap();
    assert_eq!(document.current().canonical_digest(), before);
    document.commit_verified_proposal(&proposal).unwrap();
    let committed = document.current();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(committed.import_receipts().count(), 1);
    let receipt = committed.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::Stl);
    let mesh = committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        mesh.authority,
        MeshAuthority::ImportedStl {
            import_id: receipt.id()
        }
    );
    assert_eq!(mesh.vertices_mm.len(), 4);
    assert_eq!(mesh.triangles.len(), 4);

    let committed_digest = committed.canonical_digest();
    let encoded = persistence::save(&committed);
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}
