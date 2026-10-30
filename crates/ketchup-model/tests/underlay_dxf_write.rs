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
    "0\nENDTAB\n",
    "0\nTABLE\n2\nBLOCK_RECORD\n5\n1\n70\n1\n",
    "0\nBLOCK_RECORD\n5\n31\n330\n1\n100\nAcDbSymbolTableRecord\n100\nAcDbBlockTableRecord\n2\n*D1\n",
    "0\nENDTAB\n0\nENDSEC\n",
);

/// The block `leaf` of a door, and the block `*D1` the dimension A9 draws.
const BLOCKS: &str = concat!(
    "0\nSECTION\n2\nBLOCKS\n",
    "0\nBLOCK\n5\n20\n8\n0\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n5\n21\n8\n0\n10\n0\n20\n0\n11\n800\n21\n0\n",
    "0\nARC\n5\n22\n8\n0\n10\n0\n20\n0\n40\n800\n50\n0\n51\n90\n",
    "0\nENDBLK\n5\n23\n8\n0\n",
    "0\nBLOCK\n5\n30\n330\n31\n100\nAcDbEntity\n8\n0\n100\nAcDbBlockBegin\n2\n*D1\n70\n1\n",
    "10\n0\n20\n0\n30\n0\n3\n*D1\n",
    "0\nLINE\n5\n32\n330\n31\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n",
    "10\n8000\n20\n2000\n30\n0\n11\n9000\n21\n2000\n31\n0\n",
    "0\nLINE\n5\n33\n330\n31\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n",
    "10\n8000\n20\n1500\n30\n0\n11\n8000\n21\n2100\n31\n0\n",
    "0\nLINE\n5\n34\n330\n31\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n",
    "10\n9000\n20\n1500\n30\n0\n11\n9000\n21\n2100\n31\n0\n",
    "0\nTEXT\n5\n35\n330\n31\n100\nAcDbEntity\n8\n0\n100\nAcDbText\n",
    "10\n8400\n20\n2050\n30\n0\n40\n100\n1\n1000\n100\nAcDbText\n",
    "0\nENDBLK\n5\n36\n330\n31\n100\nAcDbEntity\n8\n0\n100\nAcDbBlockEnd\n",
    "0\nENDSEC\n",
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
    "0\nDIMENSION\n5\nA9\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbDimension\n2\n*D1\n",
    "10\n9000\n20\n2000\n30\n0\n11\n8500\n21\n2100\n31\n0\n70\n33\n42\n1000\n",
    "100\nAcDbAlignedDimension\n13\n8000\n23\n1500\n33\n0\n14\n9000\n24\n1500\n34\n0\n",
    // A solid fill bounded by edges, one a clockwise arc, made from the circle A7.
    "0\nHATCH\n5\nAA\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbHatch\n10\n0\n20\n0\n30\n0\n",
    "210\n0\n220\n0\n230\n1\n2\nSOLID\n70\n1\n71\n1\n91\n1\n92\n1\n93\n4\n",
    "72\n1\n10\n10000\n20\n0\n11\n11000\n21\n0\n",
    "72\n2\n10\n11000\n20\n500\n40\n500\n50\n270\n51\n90\n73\n0\n",
    "72\n1\n10\n11000\n20\n1000\n11\n10000\n21\n1000\n",
    "72\n1\n10\n10000\n20\n1000\n11\n10000\n21\n0\n",
    "97\n1\n330\nA7\n75\n0\n76\n1\n98\n1\n10\n10500\n20\n500\n",
    // Lines 300 apart across a square, as the hatch carries them.
    "0\nHATCH\n5\nAB\n330\n1F\n100\nAcDbEntity\n8\nnotes\n100\nAcDbHatch\n10\n0\n20\n0\n30\n0\n",
    "210\n0\n220\n0\n230\n1\n2\nUSER\n70\n0\n71\n0\n91\n1\n92\n1\n93\n4\n",
    "72\n1\n10\n12000\n20\n0\n11\n13000\n21\n0\n",
    "72\n1\n10\n13000\n20\n0\n11\n13000\n21\n1000\n",
    "72\n1\n10\n13000\n20\n1000\n11\n12000\n21\n1000\n",
    "72\n1\n10\n12000\n20\n1000\n11\n12000\n21\n0\n",
    "97\n0\n75\n0\n76\n0\n52\n0\n41\n300\n77\n0\n78\n1\n",
    "53\n0\n43\n0\n44\n150\n45\n0\n46\n300\n79\n0\n98\n1\n10\n12500\n20\n500\n",
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

/// What a drawing draws, by layer name: its straight pieces, both ways round, its texts,
/// and its fills with the edges of their rings.
#[derive(Debug, Default)]
struct Drawn {
    pieces: Vec<[[f64; 2]; 2]>,
    texts: Vec<(String, [f64; 2], f64, f64)>,
    fills: usize,
    fill_edges: Vec<[[f64; 2]; 2]>,
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
        for fill in layer.fills() {
            for ring in fill.rings() {
                for (at, point) in ring.iter().enumerate() {
                    drawn.fill_edges.push([*point, ring[(at + 1) % ring.len()]]);
                }
            }
        }
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
    ketchup_geometry::linalg::point_segment_distance2(point, a, b)
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
        for (from, to) in [(got, expected), (expected, got)] {
            for point in from.fill_edges.iter().flatten() {
                assert!(
                    to.fill_edges.iter().any(|edge| on(*point, *edge) < 0.25),
                    "{name}: fill corner {point:?} not on the other's rings"
                );
            }
        }
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

/// A room is moved with its fill and its dimensions: hatches (a polyline boundary, edges
/// with a clockwise arc, a pattern of lines) and a dimension moved, turned, mirrored and
/// copied read back where the underlay drew them. Each dimension draws a block of its own,
/// with its record; the hatches no longer name what they were made from.
#[test]
fn moved_and_copied_hatches_and_dimensions_are_written_where_they_now_lie() {
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
    let shift = similarity([100.0, 0.0], 0.0, 1.0, [0.0, 0.0], None);
    let turn = similarity([500.0, 200.0], 30.0, 1.0, [0.0, 0.0], None);
    let mirror = similarity(
        [0.0, 0.0],
        0.0,
        1.0,
        [0.0, 0.0],
        Some([[0.0, 5000.0], [1.0, 5000.0]]),
    );
    let mut edits = UnderlayEdits::default();
    edits.moved_objects.insert(id(0xA8), shift);
    edits.moved_objects.insert(id(0xA9), turn);
    edits.moved_objects.insert(id(0xAA), mirror);
    edits.moved_objects.insert(id(0xAB), turn);
    for (of, map) in [(0xA9, mirror), (0xAA, turn), (0xAB, mirror), (0xA8, turn)] {
        edits.added.push(AddedObject::Copy { of: id(of), map });
    }
    assert!(edits.is_valid());

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap();
    let text = String::from_utf8(written.clone()).unwrap();
    assert!(text.contains(BLOCKS.trim_end_matches("0\nENDSEC\n")));
    // Named by its record, its BLOCK (groups 2 and 3) and its dimension.
    for name in ["*D2", "*D3"] {
        assert_eq!(text.matches(&format!("\n{name}\n")).count(), 4, "{name}");
    }
    assert_eq!(text.matches("BLOCK_RECORD\n").count(), 4);
    assert!(!text.contains("97\n1\n330\nA7\n"), "{text}");
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
    let drawn_again = drawn(&again.lines, &again.layers);
    assert_same(&drawn_again, &drawn(&edited, names));
    assert_eq!(drawn_again["walls"].fills, 4);
}

/// Groups, draw order and the extension dictionary of the line A0, which has data.
const OBJECTS: &str = concat!(
    "0\nSECTION\n2\nOBJECTS\n",
    "0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n3\nACAD_GROUP\n350\nD\n",
    "0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n3\nWALL1\n350\nE\n3\nDOOR\n350\nF\n",
    "0\nGROUP\n5\nE\n102\n{ACAD_REACTORS\n330\nD\n102\n}\n330\nD\n100\nAcDbGroup\n",
    "300\n\n70\n0\n71\n1\n340\nA0\n340\nA1\n",
    "0\nGROUP\n5\nF\n330\nD\n100\nAcDbGroup\n300\n\n70\n0\n71\n1\n340\nA4\n",
    "0\nDICTIONARY\n5\nB0\n330\nA0\n100\nAcDbDictionary\n3\nDATA\n360\nB1\n",
    "0\nXRECORD\n5\nB1\n330\nB0\n100\nAcDbXrecord\n1\nhello\n",
    "0\nSORTENTSTABLE\n5\nB2\n330\nC\n100\nAcDbSortentsTable\n331\nA0\n5\nB3\n331\nA1\n5\nB4\n",
    "0\nENDSEC\n",
);

/// Review 2026-10-29 CD-60: entities taken out leave no reference in the OBJECTS section —
/// a group keeps its other members, draw order its other entries, and the extension
/// dictionary of a removed entity goes with it, with what that dictionary owns.
#[test]
fn removed_entities_leave_no_reference_in_the_objects() {
    let entities = ENTITIES.replace(
        "0\nLINE\n5\nA0\n330\n1F\n",
        "0\nLINE\n5\nA0\n102\n{ACAD_XDICTIONARY\n360\nB0\n102\n}\n330\n1F\n",
    );
    let source = format!("{HEADER}{TABLES}{BLOCKS}{entities}{OBJECTS}0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(id_of(lines, 0xA0));
    edits.removed_objects.insert(id_of(lines, 0xA4));

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap();
    let text = String::from_utf8(written).unwrap();
    let objects = &text[text.find("OBJECTS\n").unwrap()..];
    assert_eq!(
        objects,
        concat!(
            "OBJECTS\n",
            "0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n3\nACAD_GROUP\n350\nD\n",
            "0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n3\nWALL1\n350\nE\n3\nDOOR\n350\nF\n",
            "0\nGROUP\n5\nE\n102\n{ACAD_REACTORS\n330\nD\n102\n}\n330\nD\n100\nAcDbGroup\n",
            "300\n\n70\n0\n71\n1\n340\nA1\n",
            "0\nGROUP\n5\nF\n330\nD\n100\nAcDbGroup\n300\n\n70\n0\n71\n1\n",
            "0\nSORTENTSTABLE\n5\nB2\n330\nC\n100\nAcDbSortentsTable\n331\nA1\n5\nB4\n",
            "0\nENDSEC\n0\nEOF\n",
        )
    );
}

/// A dashed red LINE of lineweight 50 with extended data, two lines across it, and an
/// LWPOLYLINE 20 wide along it; a group names the line and the polyline.
const SHAPED: &str = concat!(
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n5\nC0\n330\n1F\n100\nAcDbEntity\n8\nwalls\n6\nDASHED\n62\n1\n370\n50\n",
    "100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n1001\nKETCHUP\n1000\nwall-7\n",
    "0\nLINE\n5\nC1\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbLine\n",
    "10\n1000\n20\n-500\n30\n0\n11\n1000\n21\n1500\n31\n0\n",
    "0\nLINE\n5\nC2\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbLine\n",
    "10\n3000\n20\n-500\n30\n0\n11\n3000\n21\n1500\n31\n0\n",
    "0\nLWPOLYLINE\n5\nC3\n330\n1F\n100\nAcDbEntity\n8\nwalls\n62\n3\n100\nAcDbPolyline\n",
    "90\n2\n70\n0\n43\n20\n10\n0\n20\n1000\n10\n4000\n20\n1000\n",
    "0\nENDSEC\n",
    "0\nSECTION\n2\nOBJECTS\n",
    "0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n3\nACAD_GROUP\n350\nD\n",
    "0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n3\nWALL1\n350\nE\n",
    "0\nGROUP\n5\nE\n330\nD\n100\nAcDbGroup\n300\n\n70\n0\n71\n1\n340\nC0\n340\nC3\n",
    "0\nENDSEC\n",
);

/// The records of `text`, each from its `0` group to the next, with group codes unpadded.
fn records(text: &str) -> Vec<String> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut records = Vec::<String>::new();
    for pair in lines.chunks(2) {
        let code = pair[0].trim();
        if code == "0" {
            records.push(String::new());
        }
        if let Some(record) = records.last_mut() {
            record.push_str(&format!("{code}\n{}\n", pair.get(1).unwrap_or(&"")));
        }
    }
    records
}

/// The record of handle `handle` in `text`.
fn entity(text: &str, handle: &str) -> String {
    records(text)
        .into_iter()
        .find(|record| record.contains(&format!("\n5\n{handle}\n")))
        .unwrap_or_else(|| panic!("no entity {handle}"))
}

/// Review 2026-10-29 CD-61: a trimmed, extended or offset line was taken out and drawn
/// again on its layer, so a dashed red LINE of lineweight 50 with extended data came back
/// as a line by layer with a new handle and no data, a polyline lost its width, and a group
/// lost its member. The line keeps its entity: only its ends are written; a second piece of
/// it and an offset are copies of it with all it has, and widths are not scaled.
#[test]
fn a_trimmed_extended_or_offset_line_keeps_its_entity_and_properties() {
    use ketchup_model::document::DocumentStore;
    use ketchup_model::underlay::line_edit::{LineEdit, plan_underlay_line_edit};
    use ketchup_model::underlay::plan_underlay;

    let source = format!("{HEADER}{TABLES}{SHAPED}0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut document = DocumentStore::new();
    let (batch, underlay) = plan_underlay(
        &document.current(),
        "plan",
        &original.layers,
        &[],
        [7; 32],
        [0.0, 0.0],
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let edits = |document: &DocumentStore| document.current().underlay(underlay).unwrap().clone();
    let edit = |document: &mut DocumentStore, object: u32, line_edit: LineEdit| {
        let current = edits(document);
        let drawn = lines.clone().edited(&current.edits, &current.layers);
        let batch =
            plan_underlay_line_edit(&document.current(), underlay, &drawn, object, line_edit)
                .unwrap();
        document.apply_batch(&batch).unwrap();
    };
    let line = id_of(lines, 0xC0);
    let polyline = id_of(lines, 0xC3);
    // The line between the two across it goes; the polyline's piece before the first.
    edit(
        &mut document,
        line,
        LineEdit::Trim {
            pick_mm: [2000.0, 0.0],
        },
    );
    edit(
        &mut document,
        polyline,
        LineEdit::Trim {
            pick_mm: [500.0, 1000.0],
        },
    );
    edit(
        &mut document,
        line,
        LineEdit::Offset {
            distance_mm: Some(500.0),
            side_mm: [500.0, 600.0],
        },
    );
    let current = edits(&document);
    assert!(current.edits.is_valid());

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &current.edits,
    )
    .unwrap();
    let text = String::from_utf8(written.clone()).unwrap();
    let properties = "8\nwalls\n6\nDASHED\n62\n1\n370\n50\n100\nAcDbLine\n";
    let data = "1001\nKETCHUP\n1000\nwall-7\n";
    assert_eq!(
        entity(&text, "C0"),
        format!(
            "0\nLINE\n5\nC0\n330\n1F\n100\nAcDbEntity\n{properties}\
             10\n0\n20\n0\n30\n0\n11\n1000\n21\n0\n31\n0\n{data}"
        )
    );
    assert_eq!(
        entity(&text, "C3"),
        "0\nLWPOLYLINE\n5\nC3\n330\n1F\n100\nAcDbEntity\n8\nwalls\n62\n3\n100\nAcDbPolyline\n\
         90\n2\n70\n0\n43\n20\n10\n1000\n20\n1000\n10\n4000\n20\n1000\n"
    );
    // The piece past the second line across, and the offset: lines with all the line has.
    let records = records(&text);
    for (from, to) in [
        ("3000\n20\n0", "4000\n21\n0"),
        ("0\n20\n500", "1000\n21\n500"),
    ] {
        let body =
            format!("100\nAcDbEntity\n{properties}10\n{from}\n30\n0\n11\n{to}\n31\n0\n{data}");
        let found = records.iter().filter(|record| record.ends_with(&body));
        assert_eq!(found.count(), 1, "{body}\n{text}");
    }
    let kept = records
        .iter()
        .filter(|record| record.starts_with("0\nLINE\n"));
    assert_eq!(kept.count(), 5, "{text}");
    // The group still names both.
    assert!(text.contains("340\nC0\n340\nC3\n"), "{text}");

    let again = read(&written);
    let edited = lines.clone().edited(&current.edits, &current.layers);
    assert_same(
        &drawn(&again.lines, &again.layers),
        &drawn(&edited, &original.layers),
    );
}

#[test]
fn another_drawing_is_refused() {
    let source = source();
    let original = read(source.as_bytes());
    let lines = &original.lines;
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

/// A DXF R12 drawing: no entity has a handle. A line in paper space is not read.
const R12: &str = concat!(
    "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
    "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n2\n",
    "0\nLAYER\n2\nwalls\n70\n0\n62\n1\n6\nCONTINUOUS\n",
    "0\nLAYER\n2\nnotes\n70\n0\n62\n3\n6\nCONTINUOUS\n",
    "0\nENDTAB\n0\nENDSEC\n",
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n8\nwalls\n10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n",
    "0\nLINE\n8\nwalls\n67\n1\n10\n0\n20\n0\n30\n0\n11\n100\n21\n100\n31\n0\n",
    "0\nLINE\n8\nwalls\n10\n0\n20\n1000\n30\n0\n11\n4000\n21\n1000\n31\n0\n",
    "0\nCIRCLE\n8\nwalls\n10\n6000\n20\n0\n30\n0\n40\n300\n",
    "0\nTEXT\n8\nnotes\n10\n500\n20\n500\n30\n0\n40\n250\n1\nWC\n",
    "0\nPOLYLINE\n8\nwalls\n66\n1\n10\n0\n20\n0\n30\n0\n",
    "0\nVERTEX\n8\nwalls\n10\n0\n20\n2000\n30\n0\n",
    "0\nVERTEX\n8\nwalls\n10\n4000\n20\n2000\n30\n0\n",
    "0\nSEQEND\n8\nwalls\n",
    "0\nENDSEC\n0\nEOF\n",
);

/// Review 2026-10-29 CD-66: in a drawing without handles every removal, move and copy was
/// refused only on export ("object 0 has no handle"). Its entities are found by their
/// order: the n-th object is the n-th entity of model space.
#[test]
fn a_drawing_without_handles_is_written_with_its_edits() {
    let original = read(R12.as_bytes());
    let lines = &original.lines;
    let objects = &lines.objects().objects;
    let tops = (0..objects.len())
        .filter(|index| objects[*index].parent.is_none())
        .map(|index| u32::try_from(index).unwrap())
        .collect::<Vec<_>>();
    let kinds = tops
        .iter()
        .map(|id| objects[*id as usize].kind.name())
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["LINE", "LINE", "CIRCLE", "TEXT", "POLYLINE"]);
    assert!(objects.iter().all(|object| object.handle == 0));
    let layers = original
        .layers
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    let shift = |by: [f64; 2]| [[1.0, 0.0, by[0]], [0.0, 1.0, by[1]]];
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(tops[1]);
    edits.moved_objects.insert(tops[2], shift([1000.0, 0.0]));
    edits.moved_objects.insert(tops[4], shift([0.0, 500.0]));
    edits.added.push(AddedObject::Copy {
        of: tops[3],
        map: shift([0.0, 2000.0]),
    });

    let written = write_edited_dxf(R12.as_bytes(), DxfImportOptions::new(None), lines, &edits)
        .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written.clone()).unwrap();
    // The first line and the one in paper space are as they were; the second is gone.
    assert!(text.contains("0\nLINE\n8\nwalls\n10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n"));
    assert!(text.contains("67\n1\n"), "{text}");
    assert!(
        !text.contains("20\n1000\n30\n0\n11\n4000\n21\n1000\n"),
        "{text}"
    );
    let again = read(&written);
    let edited = lines.clone().edited(&edits, &layers);
    assert_same(
        &drawn(&again.lines, &again.layers),
        &drawn(&edited, &original.layers),
    );

    // An insert placed in two columns is two objects of one entity: the entities do not
    // line up with the objects one by one, and a change is refused by name.
    let array = R12
        .replace(
            "0\nSECTION\n2\nENTITIES\n",
            "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n8\n0\n2\npost\n70\n0\n10\n0\n20\n0\n\
             0\nLINE\n8\n0\n10\n0\n20\n0\n11\n0\n21\n100\n0\nENDBLK\n8\n0\n0\nENDSEC\n\
             0\nSECTION\n2\nENTITIES\n",
        )
        .replace(
            "0\nENDSEC\n0\nEOF\n",
            "0\nINSERT\n8\nwalls\n2\npost\n10\n0\n20\n3000\n70\n2\n44\n1000\n0\nENDSEC\n0\nEOF\n",
        );
    let read_array = read(array.as_bytes());
    let mut removal = UnderlayEdits::default();
    removal.removed_objects.insert(0);
    let error = write_edited_dxf(
        array.as_bytes(),
        DxfImportOptions::new(None),
        &read_array.lines,
        &removal,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            EditedDxfError::Refused(DxfEditRefusal::NoHandle { object: 0 })
        ),
        "{error}"
    );
}
