//! `file action=import` of a SketchUp `.skp` file: it imports as the GLB `ketchup-skp`
//! converts it to, in one Undo step and as a job the window does not wait for; the review
//! and the receipt name the SketchUp version, and a damaged file is refused without
//! changing the document.

use super::*;
use crate::live_bridge::dwg_import::imports_as_job;
use ketchup_model::import::ImportFormat;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../ketchup-skp/fixtures")
            .join(name),
    )
    .unwrap()
}

fn import(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: false,
        revision_of: None,
        code_page: None,
    }
}

#[test]
fn a_sketchup_file_imports_as_its_glb_in_one_undo_step_noting_its_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("solid.skp");
    let source = fixture("SU_File.skp");
    std::fs::write(&path, &source).unwrap();
    assert!(
        imports_as_job(&import(&path)),
        "read off the window's thread"
    );

    let (mut app, mut bridge) = setup();
    let (occurrences, undo) = (app.occurrence_count(), app.undo_step_count());
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));

    assert_eq!(answer["imported"], true, "{answer}");
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    assert!(app.occurrence_count() > occurrences, "{answer}");
    let version = answer["review"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "skp_version_verified")
        .unwrap_or_else(|| panic!("no SketchUp version in {answer}"));
    assert!(
        version["subject"].as_str().unwrap().starts_with("{25."),
        "{version}"
    );

    let snapshot = app.document_snapshot();
    let receipt = snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Glb)
        .unwrap();
    assert_eq!(receipt.source_name(), "solid.skp");
    assert!(
        receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "skp_version_verified"),
        "the receipt keeps the version"
    );
}

/// The boxes on the tag "Walls", hidden in the file, import on a hidden tag "Walls": showing
/// the tag shows them. Before, every part imported untagged and the tag was only a note.
#[test]
fn the_parts_of_a_tagged_component_import_on_that_tag_hidden_as_in_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tagged.skp");
    std::fs::write(&path, fixture("tagged_boxes.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let before = app.document_snapshot();
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["imported"], true, "{answer}");

    let snapshot = app.document_snapshot();
    let walls = snapshot
        .tags()
        .find(|tag| tag.name() == "Walls")
        .unwrap_or_else(|| panic!("no tag Walls after {answer}"));
    assert!(!walls.visible(), "hidden as in the file");
    let imported: Vec<_> = snapshot
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .collect();
    assert_eq!(imported.len(), 2, "both boxes");
    for occurrence in imported {
        assert_eq!(
            occurrence.tags(),
            &std::collections::BTreeSet::from([walls.id()])
        );
    }
}

/// A box placed hidden in SketchUp (Edit > Hide) imports hidden, the other shown (review
/// 2026-11-10 SKP-402). Before, both imported visible.
#[test]
fn a_component_hidden_in_the_file_imports_hidden() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("hidden.skp");
    std::fs::write(&path, fixture("hidden_boxes.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let before = app.document_snapshot();
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["imported"], true, "{answer}");

    let snapshot = app.document_snapshot();
    let mut visible: Vec<bool> = snapshot
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .map(|occurrence| occurrence.visible())
        .collect();
    visible.sort_unstable();
    assert_eq!(
        visible,
        [false, true],
        "one box hidden, one shown: {answer}"
    );
}

/// The answer to importing `file` into a new document, which answered `imported`, the
/// document after it and the occurrences it added.
fn import_fixture(
    file: &str,
) -> (
    Value,
    ketchup_model::document::Snapshot,
    Vec<ketchup_model::document::Occurrence>,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(file);
    std::fs::write(&path, fixture(file)).unwrap();
    let (mut app, mut bridge) = setup();
    let before = app.document_snapshot();
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{file}: {code}: {}", app.action_digest()));
    assert_eq!(answer["imported"], true, "{file}: {answer}");
    let snapshot = app.document_snapshot();
    let imported = snapshot
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .cloned()
        .collect();
    (answer, snapshot, imported)
}

/// A face on its own and a box without its top close no solid, and are SketchUp content
/// like any other: both come into the document, as sheets, and the review counts them.
/// Before, the file was refused as having nothing to import.
#[test]
fn faces_that_close_no_solid_come_into_the_document() {
    let (answer, snapshot, imported) = import_fixture("open_pieces.skp");
    let mut names: Vec<_> = imported
        .iter()
        .map(|occurrence| occurrence.name())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["Panel", "Tray"]);
    for occurrence in &imported {
        let body = snapshot
            .definition(occurrence.definition_id())
            .unwrap()
            .feature_ids()
            .iter()
            .find_map(|id| snapshot.feature(*id))
            .unwrap();
        assert_eq!(
            body.kind().body_kind(),
            Some(ketchup_model::document::BodyKind::Surface),
            "{} is a sheet",
            occurrence.name()
        );
    }
    let open = answer["review"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "glb_open_surfaces_imported")
        .unwrap_or_else(|| panic!("open surfaces not counted in {answer}"));
    assert_eq!(open["count"], 2, "{open}");
}

/// What needs a solid refuses an imported sheet by name: it is not made exact, and a
/// check of made parts (collision, fabrication, mass) does not take it as a body.
#[test]
fn what_needs_a_solid_refuses_an_imported_sheet_by_name() {
    use ketchup_application::mesh_conversion::{MeshConversionError, prepare_mesh_conversion};
    use ketchup_model::exact_product::ExactResultRegistry;
    use ketchup_model::exact_validation::{GeneralBodyParticipant, GeneralBodyValidationError};
    use ketchup_model::tolerance::TolerancePolicy;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("open.skp");
    std::fs::write(&path, fixture("open_pieces.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let before = app.document_snapshot();
    bridge.execute(&mut app, import(&path), false).unwrap();
    let snapshot = app.document_snapshot();
    let sheets = snapshot
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .map(|occurrence| occurrence.definition_id())
        .collect::<Vec<_>>();
    assert_eq!(sheets.len(), 2);
    let registry = ExactResultRegistry::accept(&snapshot, []).unwrap();
    for definition_id in sheets {
        let [feature_id] = snapshot.definition(definition_id).unwrap().feature_ids() else {
            panic!("a sheet is its definition's one feature");
        };
        assert_eq!(
            prepare_mesh_conversion(&app.document, *feature_id, 0.25).err(),
            Some(MeshConversionError::SourceIsSheet)
        );
        let placed = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.definition_id == definition_id)
            .unwrap();
        assert_eq!(
            GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                placed.instance_path,
                TolerancePolicy::default(),
            )
            .err(),
            Some(GeneralBodyValidationError::OpenSheet)
        );
    }
}

/// A textured SketchUp face comes into the document with its picture: every triangle of
/// the box says which image it shows and where its corners lie on it, and the image is the
/// file's PNG, kept in the document. Before, the texture was dropped and only counted.
#[test]
fn a_textured_face_comes_into_the_document_with_its_image() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("textured.skp");
    std::fs::write(&path, fixture("textured_box.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let before = app.document_snapshot();
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let codes: Vec<_> = answer["review"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned())
        .collect();
    for dropped in [
        "skp_textures_dropped",
        "glb_material_textures_ignored",
        "glb_images_ignored",
        "glb_textures_ignored",
    ] {
        assert!(!codes.iter().any(|code| code == dropped), "{answer}");
    }
    let snapshot = app.document_snapshot();
    let box_body = snapshot
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .find_map(|occurrence| {
            let definition = snapshot.definition(occurrence.definition_id())?;
            definition
                .feature_ids()
                .iter()
                .find_map(|id| match snapshot.feature(*id)?.kind() {
                    ketchup_model::document::FeatureKind::MeshBody(mesh) => Some(mesh.clone()),
                    _ => None,
                })
        })
        .unwrap_or_else(|| panic!("no mesh body after {answer}"));
    let look = box_body
        .look
        .as_ref()
        .unwrap_or_else(|| panic!("the box carries no texture: {answer}"));
    assert_eq!(look.images.len(), 1);
    assert_eq!(look.images[0].mime_type, "image/png");
    assert_eq!(look.triangles.len(), box_body.triangles.len());
    // One tile covers 5 × 5 inches, unstretched: along every edge of every triangle the
    // picture moves one tile per 127 mm.
    for (corners, textured) in box_body.triangles.iter().zip(&look.triangles) {
        let textured = textured.expect("every face of the box is textured");
        assert_eq!(textured.image, 0);
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let [p, q] = [corners[a], corners[b]].map(|index| box_body.vertices_mm[index as usize]);
            let length_mm = (0..3)
                .map(|axis| (p[axis] - q[axis]).powi(2))
                .sum::<f64>()
                .sqrt();
            let tiles = f64::from(
                (textured.uv[a][0] - textured.uv[b][0])
                    .hypot(textured.uv[a][1] - textured.uv[b][1]),
            );
            assert!(
                (tiles * 127.0 - length_mm).abs() < 0.01,
                "{length_mm} mm, {tiles} tiles"
            );
        }
    }
    let image = app
        .file
        .container_data
        .blobs()
        .get(&look.images[0].blob)
        .unwrap_or_else(|| panic!("the document holds no image {}", look.images[0].blob));
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n");
}

/// The presentation image shows a SketchUp texture where it lies: the textured box comes
/// out in the picture's red and white squares. Before, each face showed one flat colour.
#[test]
fn a_texture_shows_in_the_presentation_image() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("textured.skp");
    std::fs::write(&path, fixture("textured_box.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let hide = app
        .document_snapshot()
        .occurrences()
        .map(|occurrence| CanonicalCommand::SetOccurrenceVisibility {
            id: occurrence.id(),
            visible: false,
        })
        .collect();
    app.document.apply_batch(&CommandBatch::new(hide)).unwrap();
    bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let image = directory.path().join("textured.png");
    let options = json!({
        "width": 240, "height": 160, "background": "transparent",
        "shadows": false, "ambient_occlusion": false, "outlines": false,
    });
    let answer = bridge
        .execute(
            &mut app,
            Request::Export {
                expected: None,
                path: image.display().to_string(),
                export_format: "presentation_png".to_owned(),
                drawing_scale: None,
                image: Some(serde_json::from_value(options).unwrap()),
                overwrite: false,
            },
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["exported"], true, "{answer}");
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&image).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut rgba).unwrap();
    // Shading scales a face's colour, so a flat colour keeps one green-to-red ratio.
    let ratios = rgba
        .chunks(4)
        .filter(|pixel| pixel[3] == u8::MAX && pixel[0] > 60)
        .map(|pixel| f64::from(pixel[1]) / f64::from(pixel[0]))
        .collect::<Vec<_>>();
    let red = ratios.iter().filter(|ratio| **ratio < 0.6).count();
    let white = ratios.iter().filter(|ratio| **ratio > 0.8).count();
    assert!(
        red * 20 > ratios.len() && white * 20 > ratios.len(),
        "{red} red and {white} white of {} model pixels",
        ratios.len()
    );
}

/// The groups nested in each other in files of SketchUp 2013 and 2020 come in as that
/// assembly: each group's geometry under the group that holds it. Before, both files were
/// refused.
#[test]
fn the_nested_groups_of_classic_files_come_in_as_an_assembly() {
    for file in ["nesting_2013.skp", "nesting_2020.skp"] {
        let (_, _, imported) = import_fixture(file);
        assert_eq!(imported.len(), 3, "{file}: Outer, Shelf and Inner");
    }
}

#[test]
fn a_damaged_sketchup_file_is_refused_and_changes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged.skp");
    let source = fixture("SU_File.skp");
    std::fs::write(&path, &source[..source.len() / 3]).unwrap();
    let (mut app, mut bridge) = setup();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());

    assert_eq!(
        bridge.execute(&mut app, import(&path), false),
        Err("import_failed")
    );
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo);
}

/// Imported sheets go out again in every mesh format that carries an open surface; a 3MF
/// print package, which holds closed solids, refuses them and writes nothing.
#[test]
fn imported_sheets_export_as_meshes_and_3mf_refuses_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("open.skp");
    std::fs::write(&path, fixture("open_pieces.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    bridge.execute(&mut app, import(&path), false).unwrap();
    for format in ["glb", "dae", "stl", "3mf"] {
        let out = directory.path().join(format!("out.{format}"));
        let answer = bridge.execute(
            &mut app,
            Request::Export {
                expected: None,
                path: out.display().to_string(),
                export_format: format.to_owned(),
                drawing_scale: None,
                image: None,
                overwrite: false,
            },
            false,
        );
        if format == "3mf" {
            assert_eq!(answer.err(), Some("export_refused"));
            assert!(
                app.action_digest().contains("open sheet"),
                "{}",
                app.action_digest()
            );
            assert!(!out.exists());
        } else {
            answer.unwrap_or_else(|code| panic!("{format}: {code}: {}", app.action_digest()));
            assert!(std::fs::metadata(&out).unwrap().len() > 0, "{format}");
        }
    }
    // The GLB holds the sheets: read back, it brings the box and both of them.
    let (mut again, mut reader) = setup();
    let before = again.occurrence_count();
    reader
        .execute(&mut again, import(&directory.path().join("out.glb")), false)
        .unwrap_or_else(|code| panic!("{code}: {}", again.action_digest()));
    assert_eq!(again.occurrence_count(), before + 3);
}
