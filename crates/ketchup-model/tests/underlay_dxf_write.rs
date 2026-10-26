//! 2D-3: a drawing changed in Kečup written back as DXF. Its own text stays as it was —
//! header, tables, blocks, styles and the entities not changed — the entities taken out
//! are left out, the moved ones mapped, and the copies and shapes drawn come after them
//! with handles of their own. Read again, it draws what the underlay drew with the edits.
use ketchup_model::document::TagId;
use ketchup_model::import::{
    DxfEditRefusal, DxfImportOptions, DxfUnderlay, EditedDxfError, read_dxf_underlay,
    write_edited_dxf,
};
use ketchup_model::underlay::drawn::{AddedObject, DrawnShape, similarity};
use ketchup_model::underlay::edits::UnderlayEdits;
use ketchup_model::underlay::{UnderlayLayer, UnderlayLines};

const HEADER: &str = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$INSUNITS\n70\n4\n\
                      9\n$HANDSEED\n5\n200\n0\nENDSEC\n";

const TABLES: &str = concat!(
    "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n5\n2\n70\n3\n",
    "0\nLAYER\n5\n10\n100\nAcDbSymbolTableRecord\n100\nAcDbLayerTableRecord\n2\nwalls\n70\n0\n62\n1\n6\nCONTINUOUS\n",
    "0\nLAYER\n5\n11\n100\nAcDbSymbolTableRecord\n100\nAcDbLayerTableRecord\n2\nnotes\n70\n0\n62\n3\n6\nCONTINUOUS\n",
    "0\nLAYER\n5\n12\n100\nAcDbSymbolTableRecord\n100\nAcDbLayerTableRecord\n2\ndoors\n70\n0\n62\n5\n6\nCONTINUOUS\n",
    "0\nENDTAB\n0\nENDSEC\n",
);

const BLOCKS: &str = concat!(
    "0\nSECTION\n2\nBLOCKS\n",
    "0\nBLOCK\n5\n20\n8\n0\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n5\n21\n8\n0\n10\n0\n20\n0\n11\n800\n21\n0\n",
    "0\nARC\n5\n22\n8\n0\n10\n0\n20\n0\n40\n800\n50\n0\n51\n90\n",
    "0\nENDBLK\n5\n23\n8\n0\n0\nENDSEC\n",
);

const ENTITIES: &str = concat!(
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n5\nA0\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbLine\n",
    "10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n",
    "0\nARC\n5\nA1\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbCircle\n",
    "10\n4000\n20\n2000\n30\n0\n40\n1000\n100\nAcDbArc\n50\n0\n51\n90\n",
    "0\nLWPOLYLINE\n5\nA2\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbPolyline\n90\n3\n70\n1\n",
    "10\n0\n20\n3000\n42\n0.5\n10\n1000\n20\n3000\n10\n1000\n20\n4000\n",
    "0\nTEXT\n5\nA3\n330\n1F\n100\nAcDbEntity\n8\nnotes\n100\nAcDbText\n",
    "10\n500\n20\n500\n30\n0\n40\n250\n1\nWC 1.02\n100\nAcDbText\n",
    "0\nINSERT\n5\nA4\n330\n1F\n100\nAcDbEntity\n8\ndoors\n100\nAcDbBlockReference\n66\n1\n",
    "2\nleaf\n10\n2000\n20\n0\n30\n0\n",
    "0\nATTRIB\n5\nA5\n330\nA4\n100\nAcDbEntity\n8\ndoors\n100\nAcDbText\n",
    "10\n2100\n20\n100\n30\n0\n40\n100\n1\nD-01\n100\nAcDbAttribute\n2\nNAME\n70\n0\n",
    "0\nSEQEND\n5\nA6\n330\nA4\n100\nAcDbEntity\n8\ndoors\n",
    "0\nCIRCLE\n5\nA7\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbCircle\n",
    "10\n6000\n20\n0\n30\n0\n40\n300\n",
    "0\nHATCH\n5\nA8\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbHatch\n10\n0\n20\n0\n30\n0\n",
    "210\n0\n220\n0\n230\n1\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n2\n72\n0\n73\n1\n93\n4\n",
    "10\n7000\n20\n0\n10\n8000\n20\n0\n10\n8000\n20\n1000\n10\n7000\n20\n1000\n",
    "97\n0\n75\n0\n76\n1\n98\n0\n",
    "0\nENDSEC\n",
);

fn source() -> String {
    format!("{HEADER}{TABLES}{BLOCKS}{ENTITIES}0\nEOF\n")
}

fn read(source: &[u8]) -> DxfUnderlay {
    read_dxf_underlay(source, DxfImportOptions::new(None)).unwrap()
}

/// The id of the top-level object of handle `handle`.
fn id_of(lines: &UnderlayLines, handle: u64) -> u32 {
    let index = lines
        .objects()
        .objects
        .iter()
        .position(|object| object.parent.is_none() && object.handle == handle)
        .unwrap_or_else(|| panic!("no object {handle:X}"));
    u32::try_from(index).unwrap()
}

/// What a drawing draws, by layer name: its straight pieces, both ways round, and its
/// texts and fills.
#[derive(Debug, Default)]
struct Drawn {
    pieces: Vec<[[f64; 2]; 2]>,
    texts: Vec<(String, [f64; 2], f64, f64)>,
    fills: usize,
}

fn drawn(lines: &UnderlayLines, names: &[String]) -> std::collections::BTreeMap<String, Drawn> {
    let mut by_layer = std::collections::BTreeMap::<String, Drawn>::new();
    for (layer, name) in lines.layers().iter().zip(names) {
        let drawn = by_layer.entry(name.clone()).or_default();
        for polyline in layer.polylines() {
            for piece in polyline.windows(2) {
                let mut piece = [piece[0], piece[1]];
                if (piece[1][0], piece[1][1]) < (piece[0][0], piece[0][1]) {
                    piece.swap(0, 1);
                }
                drawn.pieces.push(piece);
            }
        }
        for text in layer.texts() {
            drawn.texts.push((
                text.text.clone(),
                text.anchor_mm,
                text.height_mm,
                text.rotation_rad.rem_euclid(std::f64::consts::TAU),
            ));
        }
        drawn.fills += layer.fills().len();
    }
    by_layer
        .retain(|_, drawn| !drawn.pieces.is_empty() || !drawn.texts.is_empty() || drawn.fills > 0);
    by_layer
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6
}

/// How far `point` is from the straight piece `piece`.
fn on(point: [f64; 2], [a, b]: [[f64; 2]; 2]) -> f64 {
    let along = [b[0] - a[0], b[1] - a[1]];
    let length = along[0] * along[0] + along[1] * along[1];
    let t = if length > 0.0 {
        (((point[0] - a[0]) * along[0] + (point[1] - a[1]) * along[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point[0] - a[0] - t * along[0]).hypot(point[1] - a[1] - t * along[1])
}

/// Whether both draw the same, up to 1e-6 mm.
fn assert_same(
    written: &std::collections::BTreeMap<String, Drawn>,
    edited: &std::collections::BTreeMap<String, Drawn>,
) {
    assert_eq!(
        written.keys().collect::<Vec<_>>(),
        edited.keys().collect::<Vec<_>>()
    );
    for (name, expected) in edited {
        let got = &written[name];
        assert_eq!(got.pieces.len(), expected.pieces.len(), "pieces of {name}");
        // A circle has no start: turned, its chords start elsewhere on it. Every end
        // lies on the other's lines, within the chords' sag.
        for (from, to) in [(got, expected), (expected, got)] {
            for point in from.pieces.iter().flatten() {
                assert!(
                    to.pieces.iter().any(|piece| on(*point, *piece) < 0.25),
                    "{name}: {point:?} not on the lines"
                );
            }
        }
        assert_eq!(got.fills, expected.fills, "fills of {name}");
        assert_eq!(got.texts.len(), expected.texts.len(), "texts of {name}");
        for text in &expected.texts {
            assert!(
                got.texts.iter().any(|other| other.0 == text.0
                    && near(other.1, text.1)
                    && (other.2 - text.2).abs() < 1e-6
                    && ((other.3 - text.3 + 1.0).rem_euclid(std::f64::consts::TAU) - 1.0).abs()
                        < 1e-9),
                "{name}: {text:?} not written, {:?}",
                got.texts
            );
        }
    }
}

#[test]
fn an_edited_drawing_written_as_dxf_reads_back_as_the_underlay_drew_it() {
    let source = source();
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let names = &original.layers;
    let layers = names
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    let id = |handle| id_of(lines, handle);
    let turn = similarity([500.0, 200.0], 30.0, 1.0, [0.0, 0.0], None);
    let mirror = similarity(
        [0.0, 0.0],
        0.0,
        1.0,
        [0.0, 0.0],
        Some([[0.0, 5000.0], [1.0, 5000.0]]),
    );
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(id(0xA0));
    edits.removed_objects.insert(id(0xA8));
    edits.moved_objects.insert(id(0xA1), turn);
    edits.moved_objects.insert(id(0xA2), mirror);
    edits.moved_objects.insert(id(0xA3), mirror);
    edits.added.push(AddedObject::Copy {
        of: id(0xA4),
        map: mirror,
    });
    edits.added.push(AddedObject::Copy {
        of: id(0xA7),
        map: turn,
    });
    edits.added.push(AddedObject::Copy {
        of: id(0xA0),
        map: turn,
    });
    let draw = |layer: &str, shape| AddedObject::Drawn {
        layer: layer.to_owned(),
        shape,
    };
    edits.added.extend([
        draw(
            "walls",
            DrawnShape::Polyline {
                points_mm: vec![[0.0, -1000.0], [3000.0, -1000.0]],
                closed: false,
            },
        ),
        draw(
            "walls",
            DrawnShape::Polyline {
                points_mm: vec![[0.0, -2000.0], [3000.0, -2000.0], [3000.0, -2500.0]],
                closed: true,
            },
        ),
        draw(
            "walls",
            DrawnShape::Arc {
                center_mm: [5000.0, -2000.0],
                radius_mm: 400.0,
                start_deg: 45.0,
                end_deg: 200.0,
            },
        ),
        draw(
            "doors",
            DrawnShape::Circle {
                center_mm: [6000.0, -2000.0],
                radius_mm: 150.0,
            },
        ),
        draw(
            "notes",
            DrawnShape::Text {
                anchor_mm: [100.0, -3000.0],
                height_mm: 200.0,
                rotation_deg: 15.0,
                text: "Kúpeľňa 1.02, 50%%".to_owned(),
            },
        ),
    ]);
    // The circle drawn is taken out again; the copies before it took an id for each object
    // they hold.
    let circle = lines
        .clone()
        .edited(&edits, &layers)
        .objects()
        .objects
        .iter()
        .position(|object| object.added_by == Some(6))
        .unwrap();
    assert!(circle > lines.objects().objects.len() + 6);
    edits.removed_objects.insert(u32::try_from(circle).unwrap());
    assert!(edits.is_valid());

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap();
    let text = String::from_utf8(written.clone()).unwrap();
    // What was not changed is there as it was.
    for kept in [
        HEADER[..HEADER.find("5\n200").unwrap()].to_owned(),
        TABLES.to_owned(),
        BLOCKS.to_owned(),
    ] {
        assert!(text.contains(&kept), "{kept}");
    }
    assert!(!text.contains("$HANDSEED\n5\n200\n"), "the seed moved on");
    assert!(!text.contains("\n5\nA0\n") && !text.contains("\n5\nA8\n"));
    assert!(
        text.contains("K\\U+00FApe\\U+013E\\U+0148a 1.02, 50%%%%%%"),
        "{text}"
    );
    // Every handle is its own.
    let handles = text
        .split('\n')
        .collect::<Vec<_>>()
        .chunks(2)
        .filter(|pair| pair[0].trim() == "5")
        .map(|pair| pair[1].to_owned())
        .collect::<Vec<_>>();
    let mut unique = handles.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), handles.len(), "{handles:?}");

    let again = read(&written);
    let edited = lines.clone().edited(&edits, &layers);
    assert_same(&drawn(&again.lines, &again.layers), &drawn(&edited, names));
}

#[test]
fn a_moved_hatch_or_another_drawing_is_refused_by_name() {
    let source = source();
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut edits = UnderlayEdits::default();
    edits.moved_objects.insert(
        id_of(lines, 0xA8),
        similarity([100.0, 0.0], 0.0, 1.0, [0.0, 0.0], None),
    );
    let error = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_err();
    assert!(
        matches!(&error, EditedDxfError::Refused(DxfEditRefusal::NotMappable { kind, .. }) if kind == "HATCH"),
        "{error}"
    );

    let other = source.replace("11\n4000\n21\n0\n", "11\n4100\n21\n0\n");
    let error = write_edited_dxf(
        other.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &UnderlayEdits::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, EditedDxfError::Refused(DxfEditRefusal::NotTheSource)),
        "{error}"
    );
}
