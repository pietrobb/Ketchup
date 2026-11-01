//! 2D-3: a drawing changed in Kečup written back as DXF. Its own text stays as it was —
//! header, tables, blocks, styles and the entities not changed — the entities taken out
//! are left out, the moved ones mapped, and the copies and shapes drawn come after them
//! with handles of their own. Read again, it draws what the underlay drew with the edits.
use ketchup_model::document::TagId;
use ketchup_model::import::{
    DxfEditRefusal, DxfImportOptions, DxfUnderlay, EditedDxfError, ImportLengthUnit,
    read_dxf_underlay, write_edited_dxf,
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

/// Review 2026-10-29 CD-69: one moved entity of extrusion −Z, as AutoCAD's Mirror leaves
/// arcs, polylines and texts, refused the export of the whole drawing. Its points are in
/// an object coordinate system whose x is the world's −x; it is mapped there, and a line,
/// whose points are the world's, as it is.
#[test]
fn moved_entities_of_extrusion_minus_z_are_written_where_they_now_lie() {
    let minus_z = "210\n0\n220\n0\n230\n-1\n";
    let entities = ENTITIES
        .replace(
            "10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n",
            &format!("10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n{minus_z}"),
        )
        .replace(
            "10\n4000\n20\n2000\n30\n0\n40\n1000\n100\nAcDbArc\n50\n0\n51\n90\n",
            &format!(
                "10\n-4000\n20\n2000\n30\n0\n40\n1000\n{minus_z}100\nAcDbArc\n50\n90\n51\n180\n"
            ),
        )
        .replace(
            "90\n3\n70\n1\n10\n0\n20\n3000\n42\n0.5\n10\n1000\n20\n3000\n10\n1000\n20\n4000\n",
            &format!(
                "90\n3\n70\n1\n10\n0\n20\n3000\n42\n-0.5\n10\n-1000\n20\n3000\n\
                 10\n-1000\n20\n4000\n{minus_z}"
            ),
        )
        .replace(
            "10\n500\n20\n500\n30\n0\n40\n250\n1\nWC 1.02\n",
            &format!("10\n-500\n20\n500\n30\n0\n40\n250\n1\nWC 1.02\n50\n180\n{minus_z}"),
        );
    let flipped = format!("{HEADER}{TABLES}{BLOCKS}{entities}0\nEOF\n");
    let original = read(flipped.as_bytes());
    let lines = &original.lines;
    let names = &original.layers;
    // The same drawing as the one of extrusion +Z.
    let plain = read(source().as_bytes());
    assert_same(&drawn(lines, names), &drawn(&plain.lines, &plain.layers));
    let layers = names
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    let turn = similarity([500.0, 200.0], 30.0, 1.0, [0.0, 0.0], None);
    let mirror = similarity(
        [0.0, 0.0],
        0.0,
        1.0,
        [0.0, 0.0],
        Some([[0.0, 5000.0], [1.0, 5000.0]]),
    );
    let mut edits = UnderlayEdits::default();
    edits.moved_objects.insert(id_of(lines, 0xA0), turn);
    edits.moved_objects.insert(id_of(lines, 0xA1), turn);
    edits.moved_objects.insert(id_of(lines, 0xA2), mirror);
    edits.moved_objects.insert(id_of(lines, 0xA3), turn);
    for handle in [0xA1, 0xA2, 0xA3] {
        edits.added.push(AddedObject::Copy {
            of: id_of(lines, handle),
            map: mirror,
        });
    }

    let written = write_edited_dxf(
        flipped.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let again = read(&written);
    let edited = lines.clone().edited(&edits, &layers);
    assert_same(&drawn(&again.lines, &again.layers), &drawn(&edited, names));
}

/// Review 2026-10-31 P3 (the rest of CD-69): a moved hatch of extrusion −Z still refused
/// the export of the whole drawing. Its boundaries, seed points and pattern lie in the
/// object coordinate system whose x is the world's −x, and are mapped there.
#[test]
fn moved_hatches_of_extrusion_minus_z_are_written_where_they_now_lie() {
    let minus_z = "210\n0\n220\n0\n230\n-1\n";
    let entities = ENTITIES
        .replace(
            "210\n0\n220\n0\n230\n1\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n2\n72\n0\n73\n1\n93\n4\n\
             10\n7000\n20\n0\n10\n8000\n20\n0\n10\n8000\n20\n1000\n10\n7000\n20\n1000\n",
            &format!(
                "{minus_z}2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n2\n72\n0\n73\n1\n93\n4\n\
                 10\n-7000\n20\n0\n10\n-8000\n20\n0\n10\n-8000\n20\n1000\n10\n-7000\n20\n1000\n"
            ),
        )
        .replace(
            "210\n0\n220\n0\n230\n1\n2\nUSER\n70\n0\n71\n0\n91\n1\n92\n1\n93\n4\n\
             72\n1\n10\n12000\n20\n0\n11\n13000\n21\n0\n\
             72\n1\n10\n13000\n20\n0\n11\n13000\n21\n1000\n\
             72\n1\n10\n13000\n20\n1000\n11\n12000\n21\n1000\n\
             72\n1\n10\n12000\n20\n1000\n11\n12000\n21\n0\n",
            &format!(
                "{minus_z}2\nUSER\n70\n0\n71\n0\n91\n1\n92\n1\n93\n4\n\
                 72\n1\n10\n-12000\n20\n0\n11\n-13000\n21\n0\n\
                 72\n1\n10\n-13000\n20\n0\n11\n-13000\n21\n1000\n\
                 72\n1\n10\n-13000\n20\n1000\n11\n-12000\n21\n1000\n\
                 72\n1\n10\n-12000\n20\n1000\n11\n-12000\n21\n0\n"
            ),
        )
        .replace(
            "98\n1\n10\n12500\n20\n500\n",
            "98\n1\n10\n-12500\n20\n500\n",
        );
    let flipped = format!("{HEADER}{TABLES}{BLOCKS}{entities}0\nEOF\n");
    assert_ne!(flipped, source());
    let original = read(flipped.as_bytes());
    let lines = &original.lines;
    let names = &original.layers;
    let plain = read(source().as_bytes());
    assert_same(&drawn(lines, names), &drawn(&plain.lines, &plain.layers));
    let layers = names
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    let turn = similarity([500.0, 200.0], 30.0, 1.0, [0.0, 0.0], None);
    let mirror = similarity(
        [0.0, 0.0],
        0.0,
        1.0,
        [0.0, 0.0],
        Some([[0.0, 5000.0], [1.0, 5000.0]]),
    );
    let mut edits = UnderlayEdits::default();
    for handle in [0xA8, 0xAB] {
        edits.moved_objects.insert(id_of(lines, handle), turn);
        edits.added.push(AddedObject::Copy {
            of: id_of(lines, handle),
            map: mirror,
        });
    }
    let written = write_edited_dxf(
        flipped.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let again = read(&written);
    let edited = lines.clone().edited(&edits, &layers);
    assert_same(&drawn(&again.lines, &again.layers), &drawn(&edited, names));
}

/// The number of group `code` in `record`, the `nth` of them.
fn group(record: &str, code: &str, nth: usize) -> f64 {
    let lines = record.lines().collect::<Vec<_>>();
    let values = lines
        .chunks(2)
        .filter(|pair| pair[0] == code)
        .map(|pair| pair[1].trim().parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    *values
        .get(nth)
        .unwrap_or_else(|| panic!("no group {code} #{nth} in {record}"))
}

/// Review 2026-10-29 CD-70: the MTEXT inside a multi-line attribute had its direction
/// mapped as a point; the world points, displacements, directions and distances of
/// extended data were left as read; and a mirrored text kept its slant.
#[test]
fn moved_entities_map_embedded_text_extended_data_and_slant() {
    let entities = ENTITIES
        .replace(
            "100\nAcDbAttribute\n2\nNAME\n70\n0\n",
            "100\nAcDbAttribute\n2\nNAME\n70\n0\n101\nEmbedded Object\n\
             10\n2100\n20\n100\n30\n0\n40\n100\n41\n800\n46\n0\n71\n1\n72\n5\n\
             11\n1\n21\n0\n31\n0\n",
        )
        .replace(
            "10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n",
            "10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n1001\nKETCHUP\n\
             1010\n7\n1020\n7\n1030\n0\n1011\n100\n1021\n0\n1031\n0\n\
             1012\n10\n1022\n0\n1032\n0\n1013\n1\n1023\n0\n1033\n0\n\
             1040\n3\n1041\n50\n1042\n2\n",
        )
        .replace("1\nWC 1.02\n", "1\nWC 1.02\n51\n15\n");
    let source = format!("{HEADER}{TABLES}{BLOCKS}{entities}0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let turn = similarity([1000.0, 0.0], 90.0, 2.0, [0.0, 0.0], None);
    let mirror = similarity(
        [0.0, 0.0],
        0.0,
        1.0,
        [0.0, 0.0],
        Some([[0.0, 5000.0], [1.0, 5000.0]]),
    );
    let mut edits = UnderlayEdits::default();
    edits.moved_objects.insert(id_of(lines, 0xA0), turn);
    edits.moved_objects.insert(id_of(lines, 0xA3), mirror);
    edits.moved_objects.insert(id_of(lines, 0xA4), turn);

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written).unwrap();
    let [[a, b, x], [c, d, y]] = turn;
    let close = |got: f64, wanted: f64| assert!((got - wanted).abs() < 1e-6, "{got} ≠ {wanted}");
    // The attribute's embedded text: its point moved, its direction only turned.
    let attribute = entity(&text, "A5");
    close(group(&attribute, "10", 1), a * 2100.0 + b * 100.0 + x);
    close(group(&attribute, "20", 1), c * 2100.0 + d * 100.0 + y);
    close(group(&attribute, "11", 0), a);
    close(group(&attribute, "21", 0), c);
    close(group(&attribute, "40", 1), 200.0);
    close(group(&attribute, "41", 0), 1600.0);
    // The line's extended data.
    let line = entity(&text, "A0");
    close(group(&line, "1010", 0), 7.0);
    close(group(&line, "1011", 0), a * 100.0 + x);
    close(group(&line, "1021", 0), c * 100.0 + y);
    close(group(&line, "1012", 0), a * 10.0);
    close(group(&line, "1022", 0), c * 10.0);
    close(group(&line, "1013", 0), a / 2.0);
    close(group(&line, "1023", 0), c / 2.0);
    close(group(&line, "1040", 0), 3.0);
    close(group(&line, "1041", 0), 100.0);
    close(group(&line, "1042", 0), 4.0);
    // The mirrored text slants the other way.
    close(group(&entity(&text, "A3"), "51", 0), 345.0);
}

/// Review 2026-10-31 P3: a stretched line (trim, extend) scaled the distances and
/// scale factors of its extended data (1041, 1042) and its displacements (1012) by how
/// much longer it became, as if the drawing had been scaled. A stretch reshapes the line;
/// its data keep their size, as its widths do.
#[test]
fn a_stretched_line_keeps_the_sizes_of_its_extended_data() {
    let entities = ENTITIES.replace(
        "10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n",
        "10\n0\n20\n0\n30\n0\n11\n4000\n21\n0\n31\n0\n1001\nKETCHUP\n\
         1012\n10\n1022\n0\n1032\n0\n1041\n50\n1042\n2\n",
    );
    let source = format!("{HEADER}{TABLES}{BLOCKS}{entities}0\nEOF\n");
    let original = read(source.as_bytes());
    let mut edits = UnderlayEdits::default();
    edits.stretched_objects.insert(
        id_of(&original.lines, 0xA0),
        [[0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
    );
    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        &original.lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let line = entity(&String::from_utf8(written).unwrap(), "A0");
    let close = |got: f64, wanted: f64| assert!((got - wanted).abs() < 1e-6, "{got} ≠ {wanted}");
    close(group(&line, "11", 0), 2000.0);
    close(group(&line, "1012", 0), 10.0);
    close(group(&line, "1041", 0), 50.0);
    close(group(&line, "1042", 0), 2.0);
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

/// Review 2026-10-29 CD-68: a removed entity was still named by the associative hatch
/// bounded by it and by the dimension association measuring it. The hatch keeps its fill
/// and stops following the entity; the association goes, and the dimension (here moved)
/// no longer names it as a reactor.
#[test]
fn removed_entities_leave_no_reference_in_hatches_and_dimension_associations() {
    let entities = ENTITIES.replace(
        "0\nDIMENSION\n5\nA9\n330\n1F\n",
        "0\nDIMENSION\n5\nA9\n102\n{ACAD_REACTORS\n330\nB5\n102\n}\n330\n1F\n",
    );
    let objects = OBJECTS.replace(
        "0\nENDSEC\n",
        concat!(
            "0\nDIMASSOC\n5\nB5\n330\nA9\n100\nAcDbDimAssoc\n330\nA9\n90\n1\n70\n0\n71\n0\n",
            "1\nAcDbOsnapPointRef\n72\n1\n331\nA0\n73\n0\n40\n0\n10\n0\n20\n0\n30\n0\n75\n0\n",
            "0\nENDSEC\n",
        ),
    );
    let source = format!("{HEADER}{TABLES}{BLOCKS}{entities}{objects}0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(id_of(lines, 0xA0));
    edits.removed_objects.insert(id_of(lines, 0xA7));
    edits
        .moved_objects
        .insert(id_of(lines, 0xA9), [[1.0, 0.0, 0.0], [0.0, 1.0, 1000.0]]);

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written.clone()).unwrap();
    for gone in ["A0", "A7", "B5"] {
        let named = records(&text)
            .into_iter()
            .filter(|record| {
                let lines = record.lines().collect::<Vec<_>>();
                lines.chunks(2).any(|pair| {
                    pair[1] == gone
                        && pair[0]
                            .parse::<i32>()
                            .is_ok_and(|code| code == 5 || (330..=369).contains(&code))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(named, Vec::<String>::new(), "{gone}");
    }
    let hatch = entity(&text, "AA");
    assert!(hatch.contains("\n71\n0\n"), "{hatch}");
    assert!(hatch.contains("\n97\n0\n"), "{hatch}");
    assert!(entity(&text, "A9").contains("\n100\nAcDbEntity\n"));
    let again = read(&written);
    assert_eq!(
        again.lines.objects().objects.len() + 2,
        lines.objects().objects.len()
    );
}

/// The drawing with the hatch AA following the circle A7 and the dimension A9 measuring
/// the line A0 through the association B5, each naming the other as a reactor.
fn associated() -> String {
    let entities = ENTITIES
        .replace(
            "0\nDIMENSION\n5\nA9\n330\n1F\n",
            "0\nDIMENSION\n5\nA9\n102\n{ACAD_REACTORS\n330\nB5\n102\n}\n330\n1F\n",
        )
        .replace(
            "0\nLINE\n5\nA0\n330\n1F\n",
            "0\nLINE\n5\nA0\n102\n{ACAD_REACTORS\n330\nB5\n102\n}\n330\n1F\n",
        )
        .replace(
            "0\nCIRCLE\n5\nA7\n330\n1F\n",
            "0\nCIRCLE\n5\nA7\n102\n{ACAD_REACTORS\n330\nAA\n102\n}\n330\n1F\n",
        );
    let objects = OBJECTS.replace(
        "0\nENDSEC\n",
        concat!(
            "0\nDIMASSOC\n5\nB5\n330\nA9\n100\nAcDbDimAssoc\n330\nA9\n90\n1\n70\n0\n71\n0\n",
            "1\nAcDbOsnapPointRef\n72\n1\n331\nA0\n73\n0\n40\n0\n10\n0\n20\n0\n30\n0\n75\n0\n",
            "0\nENDSEC\n",
        ),
    );
    format!("{HEADER}{TABLES}{BLOCKS}{entities}{objects}0\nEOF\n")
}

/// Review 2026-10-31 CD-80: a hatch kept its association with a boundary moved away from
/// it (71 1 and the 330 of the boundary), and a dimension association stayed with a
/// measured entity moved apart: AutoCAD would rebuild or break them at the next edit.
/// Moved apart, the hatch stops following (71 0, no boundary named either way) and the
/// association goes; moved together by one map, both stay.
#[test]
fn associations_of_entities_moved_apart_are_let_go() {
    let source = associated();
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let away = [[1.0, 0.0, 5000.0], [0.0, 1.0, 0.0]];
    let write = |moved: &[u64]| {
        let mut edits = UnderlayEdits::default();
        for handle in moved {
            edits.moved_objects.insert(id_of(lines, *handle), away);
        }
        let written = write_edited_dxf(
            source.as_bytes(),
            DxfImportOptions::new(None),
            lines,
            &edits,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        String::from_utf8(written).unwrap()
    };
    let names = |record: &str, handle: &str| {
        record
            .lines()
            .collect::<Vec<_>>()
            .chunks(2)
            .any(|pair| pair[1] == handle && pair[0].parse::<i32>().is_ok_and(|c| c >= 330))
    };

    // The boundary and the measured line moved alone.
    let text = write(&[0xA7, 0xA0]);
    let hatch = entity(&text, "AA");
    assert!(
        hatch.contains("\n71\n0\n") && hatch.contains("\n97\n0\n"),
        "{hatch}"
    );
    assert!(!names(&hatch, "A7"), "{hatch}");
    assert!(
        !names(&entity(&text, "A7"), "AA"),
        "the circle's reactor goes"
    );
    assert!(!text.contains("DIMASSOC"), "{text}");
    assert!(!names(&entity(&text, "A9"), "B5") && !names(&entity(&text, "A0"), "B5"));

    // The hatch and the dimension moved alone: the same.
    let text = write(&[0xAA, 0xA9]);
    assert!(entity(&text, "AA").contains("\n71\n0\n"));
    assert!(!names(&entity(&text, "A7"), "AA"));
    assert!(!text.contains("DIMASSOC"));

    // Moved together by one map, the association stays. A moved hatch is written as
    // following nothing, so its boundary names it no more either.
    let text = write(&[0xA7, 0xAA, 0xA0, 0xA9]);
    assert!(text.contains("DIMASSOC") && names(&entity(&text, "A9"), "B5"));
    assert!(names(&entity(&text, "A0"), "B5"));
    assert!(!names(&entity(&text, "A7"), "AA"));
    // Nothing moved: everything follows as read.
    let text = write(&[]);
    assert!(entity(&text, "AA").contains("\n71\n1\n"));
    assert!(names(&entity(&text, "A7"), "AA") && text.contains("DIMASSOC"));
}

/// Review 2026-10-31 CD-81: removing an entity took its handle out of every record of the
/// OBJECTS section, an application's XRECORD too, so `90 2 / 330 A0 / 330 A1` became
/// `90 2 / 330 A1`, its count and positions wrong for whoever reads it. Records of a
/// structure not known to Kečup keep their places, the removed handle written as 0.
#[test]
fn removing_an_entity_keeps_the_places_of_application_data() {
    let objects = OBJECTS.replace(
        "0\nENDSEC\n",
        concat!(
            "0\nXRECORD\n5\nB7\n330\nC\n100\nAcDbXrecord\n90\n2\n330\nA0\n330\nA1\n",
            "0\nACME_LINK\n5\nB8\n330\nC\n100\nAcmeLink\n340\nA0\n1\nfrom\n",
            "0\nENDSEC\n",
        ),
    );
    let source = format!("{HEADER}{TABLES}{BLOCKS}{ENTITIES}{objects}0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(id_of(lines, 0xA0));
    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written).unwrap();
    assert!(
        entity(&text, "B7").ends_with("\n90\n2\n330\n0\n330\nA1\n"),
        "{}",
        entity(&text, "B7")
    );
    assert!(
        entity(&text, "B8").contains("\n340\n0\n1\nfrom\n"),
        "{}",
        entity(&text, "B8")
    );
    // A group of known structure still loses the member.
    assert!(!names_member(&entity(&text, "E"), "A0"));
}

fn names_member(record: &str, handle: &str) -> bool {
    record.contains(&format!("\n340\n{handle}\n"))
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

/// Review 2026-10-29 CD-64: a source whose lines differ from the underlay's was blamed as
/// another drawing even when the import itself had changed. Another drawing is one whose
/// entities differ; the same entities that read as other lines are refused for what they
/// are, and say how to go on.
#[test]
fn another_drawing_and_one_read_differently_are_refused_apart() {
    let source = source();
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let refusal = |other: &str, lines: &UnderlayLines| match write_edited_dxf(
        other.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &UnderlayEdits::default(),
    ) {
        Err(EditedDxfError::Refused(refusal)) => refusal,
        other => panic!("{other:?}"),
    };
    // An entity of another handle: another drawing.
    let renamed = source.replace("0\nLINE\n5\nA0\n", "0\nLINE\n5\nF0\n");
    assert_eq!(refusal(&renamed, lines), DxfEditRefusal::NotTheSource);
    // The same entities, one of them longer: they read differently.
    let longer = source.replace("11\n4000\n21\n0\n", "11\n4100\n21\n0\n");
    assert_eq!(refusal(&longer, lines), DxfEditRefusal::ReadDifferently);
    // The same file, its lines as an older import drew them (here: moved).
    let mut older = UnderlayEdits::default();
    older
        .moved_objects
        .insert(id_of(lines, 0xA0), [[1.0, 0.0, 5.0], [0.0, 1.0, 0.0]]);
    let layers = original
        .layers
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    let as_drawn_then = lines.clone().edited(&older, &layers);
    let refused = refusal(&source, &as_drawn_then);
    assert_eq!(refused, DxfEditRefusal::ReadDifferently);
    assert!(
        refused.to_string().contains("import the drawing again"),
        "{refused}"
    );
}

/// Review 2026-10-29 CD-63: a drawing that declares no unit, read in centimetres and
/// written as if read in millimetres, is found in its unit by the size of its lines, not
/// read once in each unit.
#[test]
fn a_drawing_without_a_unit_is_written_in_the_unit_it_was_read_in() {
    let source = source().replace("9\n$INSUNITS\n70\n4\n", "");
    let in_centimetres = DxfImportOptions::new(Some(ImportLengthUnit::Centimetre));
    let original = read_dxf_underlay(source.as_bytes(), in_centimetres).unwrap();
    let lines = &original.lines;
    let layers = original
        .layers
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    let mut edits = UnderlayEdits::default();
    edits
        .moved_objects
        .insert(id_of(lines, 0xA0), [[1.0, 0.0, 500.0], [0.0, 1.0, 0.0]]);

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(Some(ImportLengthUnit::Millimetre)),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let again = read_dxf_underlay(&written, in_centimetres).unwrap();
    let edited = lines.clone().edited(&edits, &layers);
    assert_same(
        &drawn(&again.lines, &again.layers),
        &drawn(&edited, &original.layers),
    );
}

/// Review 2026-10-29 CD-67: with a `$HANDSEED` below the drawing's handles, the next
/// handle was found past the entities' own handles but not past those of their VERTEX,
/// ATTRIB and SEQEND, so a copy could take the handle of a door's attribute. Every handle
/// written is new.
#[test]
fn a_copy_never_takes_the_handle_of_an_attribute_vertex_or_seqend() {
    let entities = ENTITIES
        .replace("0\nATTRIB\n5\nA5\n", "0\nATTRIB\n5\nF00\n")
        .replace("0\nSEQEND\n5\nA6\n", "0\nSEQEND\n5\nF01\n");
    let header = HEADER.replace("$HANDSEED\n5\n200\n", "$HANDSEED\n5\n1\n");
    let source = format!("{header}{TABLES}{BLOCKS}{entities}0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut edits = UnderlayEdits::default();
    for handle in [0xA0, 0xA7] {
        edits.added.push(AddedObject::Copy {
            of: id_of(lines, handle),
            map: [[1.0, 0.0, 0.0], [0.0, 1.0, 5000.0]],
        });
    }

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written).unwrap();
    let mut handles = records(&text)
        .iter()
        .filter_map(|record| {
            let lines = record.lines().collect::<Vec<_>>();
            lines
                .chunks(2)
                .find(|pair| pair[0] == "5")
                .map(|pair| pair[1].to_owned())
        })
        .collect::<Vec<_>>();
    let count = handles.len();
    handles.sort();
    handles.dedup();
    assert_eq!(handles.len(), count, "{text}");
    assert!(handles.contains(&"F02".to_owned()), "{handles:?}");
}

/// Review 2026-10-31 P3: a copy of a leader kept group 340, the annotation of the
/// original, so two leaders claimed one note. The copy has no annotation of its own.
#[test]
fn a_copied_leader_does_not_claim_the_note_of_the_original() {
    let entities = ENTITIES.replace(
        "0\nENDSEC\n",
        "0\nMTEXT\n5\nAC\n330\n1F\n100\nAcDbEntity\n8\nnotes\n100\nAcDbMText\n\
         10\n600\n20\n-500\n30\n0\n40\n100\n1\nnote\n\
         0\nLEADER\n5\nAD\n330\n1F\n100\nAcDbEntity\n8\nnotes\n100\nAcDbLeader\n\
         71\n0\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n\
         10\n0\n20\n-500\n30\n0\n10\n500\n20\n-500\n30\n0\n340\nAC\n0\nENDSEC\n",
    );
    let source = format!("{HEADER}{TABLES}{BLOCKS}{entities}0\nEOF\n");
    let original = read(source.as_bytes());
    let mut edits = UnderlayEdits::default();
    edits.added.push(AddedObject::Copy {
        of: id_of(&original.lines, 0xAD),
        map: [[1.0, 0.0, 0.0], [0.0, 1.0, -1000.0]],
    });
    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        &original.lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written).unwrap();
    let leaders = records(&text)
        .into_iter()
        .filter(|record| record.starts_with("0\nLEADER\n"))
        .collect::<Vec<_>>();
    assert_eq!(leaders.len(), 2, "{text}");
    assert!(leaders[0].contains("\n340\nAC\n"), "{}", leaders[0]);
    assert!(!leaders[1].contains("\n340\n"), "{}", leaders[1]);
    assert!(leaders[1].contains("\n73\n3\n"), "{}", leaders[1]);
}

/// Review 2026-10-31 P3: only the ENTITIES section was cleared of references to removed
/// entities, so a leader on a layout (a block) kept naming a removed note and a reactor
/// a removed line. Every block's entities let go of them too.
#[test]
fn an_entity_in_a_layout_block_lets_go_of_a_removed_entity() {
    let blocks = BLOCKS.replace(
        "0\nENDSEC\n",
        "0\nBLOCK\n5\n40\n8\n0\n2\n*Paper_Space\n70\n0\n10\n0\n20\n0\n\
         0\nLEADER\n5\n41\n102\n{ACAD_REACTORS\n330\nA0\n102\n}\n8\n0\n\
         71\n0\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n\
         10\n0\n20\n0\n30\n0\n10\n500\n20\n0\n30\n0\n340\nA3\n\
         0\nENDBLK\n5\n42\n8\n0\n0\nENDSEC\n",
    );
    let source = format!("{HEADER}{TABLES}{blocks}{ENTITIES}0\nEOF\n");
    let original = read(source.as_bytes());
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(id_of(&original.lines, 0xA0));
    edits.removed_objects.insert(id_of(&original.lines, 0xA3));
    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        &original.lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written).unwrap();
    let leader = records(&text)
        .into_iter()
        .find(|record| record.starts_with("0\nLEADER\n"))
        .unwrap();
    assert!(
        leader.starts_with("0\nLEADER\n5\n41\n8\n0\n71\n"),
        "{leader}"
    );
    assert!(!leader.contains("\n340\n"), "{leader}");
    assert!(text.contains("0\nENDBLK\n5\n42\n"), "{text}");
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

/// Review 2026-10-31 P3: a character beyond U+FFFF written into a drawing before DXF 2007
/// became `?`. It is the pair of UTF-16 units it is, each as `\U+XXXX`, and reads back.
#[test]
fn a_character_beyond_u_ffff_is_written_before_dxf_2007_and_reads_back() {
    let source = source();
    let original = read(source.as_bytes());
    let mut edits = UnderlayEdits::default();
    edits.added.push(AddedObject::Drawn {
        layer: "notes".to_owned(),
        shape: DrawnShape::Text {
            anchor_mm: [0.0, -1000.0],
            height_mm: 100.0,
            rotation_deg: 0.0,
            text: "Byt 𝔸 😀".to_owned(),
        },
    });
    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        &original.lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written.clone()).unwrap();
    assert!(
        text.contains("Byt \\U+D835\\U+DD38 \\U+D83D\\U+DE00\n"),
        "{text}"
    );
    let again = read(&written);
    assert!(
        again
            .lines
            .layers()
            .iter()
            .flat_map(|layer| layer.texts())
            .any(|text| text.text == "Byt 𝔸 😀"),
        "{:?}",
        again
            .lines
            .layers()
            .iter()
            .flat_map(|layer| layer.texts())
            .map(|text| text.text.escape_unicode().to_string())
            .collect::<Vec<_>>()
    );
    let mtext = source.replace(
        "0\nENDSEC\n0\nEOF\n",
        "0\nMTEXT\n8\nnotes\n10\n0\n20\n-2000\n40\n100\n1\nM \\U+D83D\\U+DE00\\Pdva\n\
         0\nENDSEC\n0\nEOF\n",
    );
    assert!(
        read(mtext.as_bytes())
            .lines
            .layers()
            .iter()
            .flat_map(|layer| layer.texts())
            .any(|text| text.text == "M 😀\ndva")
    );
}

/// Review 2026-10-31 P3: a polyline drawn into an R13 drawing was written as LWPOLYLINE,
/// which came only with R14. It is a POLYLINE there, with its VERTEX and SEQEND marked as
/// R13 marks them and owned by it.
#[test]
fn a_polyline_drawn_into_an_r13_drawing_is_one_r13_knows() {
    let header = HEADER.replace("AC1015", "AC1012");
    let entities = ENTITIES.split("0\nARC\n").next().unwrap().to_owned();
    let source = format!("{header}{TABLES}{entities}0\nENDSEC\n0\nEOF\n");
    let original = read(source.as_bytes());
    let lines = &original.lines;
    let mut edits = UnderlayEdits::default();
    edits.added.push(AddedObject::Drawn {
        layer: "walls".to_owned(),
        shape: DrawnShape::Polyline {
            points_mm: vec![[0.0, -2000.0], [3000.0, -2000.0], [3000.0, -2500.0]],
            closed: true,
        },
    });

    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(written.clone()).unwrap();
    assert!(!text.contains("LWPOLYLINE"), "{text}");
    let records = records(&text);
    let at = records
        .iter()
        .position(|record| record.starts_with("0\nPOLYLINE\n"))
        .unwrap_or_else(|| panic!("{text}"));
    let polyline = &records[at];
    assert!(polyline.contains("100\nAcDb2dPolyline\n"), "{polyline}");
    let handle = |record: &str| {
        let lines = record.lines().collect::<Vec<_>>();
        lines
            .chunks(2)
            .find(|pair| pair[0] == "5")
            .map(|pair| pair[1].to_owned())
            .unwrap()
    };
    let owner = format!("330\n{}\n", handle(polyline));
    let kinds = records[at + 1..at + 5]
        .iter()
        .map(|record| record.lines().nth(1).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["VERTEX", "VERTEX", "VERTEX", "SEQEND"]);
    for vertex in &records[at + 1..at + 4] {
        assert!(
            vertex.contains("100\nAcDbVertex\n100\nAcDb2dVertex\n") && vertex.contains(&owner),
            "{vertex}"
        );
    }
    assert!(records[at + 4].contains(&owner), "{}", records[at + 4]);

    let again = read(&written);
    let layers = original
        .layers
        .iter()
        .enumerate()
        .map(|(index, name)| UnderlayLayer {
            name: name.clone(),
            tag: TagId(10 + index as u64),
        })
        .collect::<Vec<_>>();
    assert_same(
        &drawn(&again.lines, &again.layers),
        &drawn(&lines.clone().edited(&edits, &layers), &original.layers),
    );
}
