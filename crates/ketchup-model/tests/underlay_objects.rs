//! The objects of a DXF drawing read as an underlay: each entity with its kind, handle and
//! layer; an insert with its block, placement and attributes and the objects of its block
//! as children, nested blocks too; a dimension with its measurement and points; a hatch
//! with its pattern and outline. Every line, text and fill drawn belongs to an object, and
//! the objects survive the container blob.
use ketchup_model::import::{DxfImportOptions, DxfUnderlay, read_dxf_underlay};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::objects::{ObjectDetail, UnderlayObject, apply, even_odd_area};

const BLOCKS: &str = concat!(
    "0\nBLOCK\n8\n0\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n5\nA1\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n",
    "0\nCIRCLE\n5\nA2\n8\ndetail\n10\n5\n20\n5\n40\n2\n",
    "0\nENDBLK\n",
    "0\nBLOCK\n8\n0\n2\ndoor\n70\n0\n10\n0\n20\n0\n",
    "0\nINSERT\n5\nB1\n8\n0\n2\nleaf\n10\n100\n20\n0\n",
    "0\nLINE\n5\nB2\n8\n0\n10\n0\n20\n0\n11\n0\n21\n50\n",
    "0\nENDBLK\n",
    "0\nBLOCK\n8\n0\n2\n*D1\n70\n1\n10\n0\n20\n0\n",
    "0\nLINE\n5\nD1\n8\n0\n10\n0\n20\n500\n11\n2630\n21\n500\n",
    "0\nTEXT\n5\nD2\n8\n0\n10\n1300\n20\n510\n40\n100\n1\n2630\n",
    "0\nENDBLK\n",
);

/// A solid hatch of a 100 mm square with a 40 mm square island.
fn hatch() -> String {
    let path = |low: f64, high: f64| {
        let corners = [[low, low], [high, low], [high, high], [low, high]];
        let edges: String = corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .map(|([x0, y0], [x1, y1])| format!("72\n1\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n"))
            .collect();
        format!("92\n0\n93\n4\n{edges}97\n0\n")
    };
    format!(
        "0\nHATCH\n5\n4C\n8\nfloor\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n\
         91\n2\n{}{}75\n0\n76\n1\n98\n0\n",
        path(0.0, 100.0),
        path(30.0, 70.0)
    )
}

fn read() -> DxfUnderlay {
    let entities = [
        "0\nINSERT\n5\n1F\n8\ndoors\n2\ndoor\n10\n1000\n20\n0\n50\n90\n66\n1\n",
        "0\nATTRIB\n8\ndoors\n2\nNAME\n1\nD-01\n10\n1000\n20\n0\n40\n50\n0\nSEQEND\n",
        "0\nLINE\n5\n2A\n8\nwalls\n10\n0\n20\n0\n11\n500\n21\n0\n",
        "0\nDIMENSION\n5\n3B\n8\ndims\n2\n*D1\n10\n0\n20\n500\n13\n0\n23\n0\n\
         14\n2630\n24\n0\n42\n2630\n70\n32\n",
        &hatch(),
    ]
    .concat();
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nBLOCKS\n{BLOCKS}0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap()
}

/// The index and object of handle `handle` among `parent`'s children.
fn child(lines: &UnderlayLines, parent: Option<u32>, handle: u64) -> (u32, &UnderlayObject) {
    lines
        .objects()
        .objects
        .iter()
        .enumerate()
        .find(|(_, object)| object.parent == parent && object.handle == handle)
        .map(|(index, object)| (u32::try_from(index).unwrap(), object))
        .unwrap_or_else(|| panic!("no object {handle:X} under {parent:?}"))
}

/// Every point an object draws itself, and the texts.
fn drawn(lines: &UnderlayLines, object: &UnderlayObject) -> (Vec<[f64; 2]>, Vec<String>) {
    let (mut points, mut texts) = (Vec::new(), Vec::new());
    for part in &object.parts {
        let layer = &lines.layers()[part.layer as usize];
        points.extend_from_slice(
            &layer.points_mm()[part.points.start as usize..part.points.end as usize],
        );
        texts.extend(
            layer.texts()[part.texts.start as usize..part.texts.end as usize]
                .iter()
                .map(|text| text.text.clone()),
        );
    }
    (points, texts)
}

fn near(point: [f64; 2], expected: [f64; 2]) -> bool {
    (point[0] - expected[0]).abs() < 1e-6 && (point[1] - expected[1]).abs() < 1e-6
}

#[test]
fn an_insert_keeps_its_block_placement_attributes_and_nested_children() {
    let underlay = read();
    let lines = &underlay.lines;
    let (door, insert) = child(lines, None, 0x1F);
    assert_eq!(insert.kind.name(), "INSERT");
    assert_eq!(lines.objects().layer_name(insert), "doors");
    let ObjectDetail::Insert {
        block,
        transform,
        attributes,
        copies,
    } = &insert.detail
    else {
        panic!("an insert detail: {:?}", insert.detail)
    };
    assert_eq!(block, "door");
    assert_eq!(copies, &[1, 1]);
    assert_eq!(attributes, &[("NAME".to_owned(), "D-01".to_owned())]);
    // Turned a quarter: the block's x axis runs along the drawing's y.
    assert!(near(apply(transform, [0.0, 0.0]), [1000.0, 0.0]));
    assert!(near(apply(transform, [10.0, 0.0]), [1000.0, 10.0]));
    // The attribute is drawn by the insert itself.
    assert_eq!(drawn(lines, insert).1, ["D-01"]);

    let (_, own_line) = child(lines, Some(door), 0xB2);
    assert_eq!(own_line.kind.name(), "LINE");
    let (points, _) = drawn(lines, own_line);
    assert!(points.iter().any(|point| near(*point, [950.0, 0.0])));

    let (leaf, nested) = child(lines, Some(door), 0xB1);
    let ObjectDetail::Insert {
        block, transform, ..
    } = &nested.detail
    else {
        panic!("a nested insert detail")
    };
    assert_eq!(block, "leaf");
    assert!(near(apply(transform, [0.0, 0.0]), [1000.0, 100.0]));
    let (_, leaf_line) = child(lines, Some(leaf), 0xA1);
    let (points, _) = drawn(lines, leaf_line);
    assert!(points.iter().any(|point| near(*point, [1000.0, 100.0])));
    assert!(points.iter().any(|point| near(*point, [1000.0, 110.0])));
    // On layer 0 of both blocks it is drawn on the layer of the outer insert.
    let part_layer = leaf_line.parts[0].layer as usize;
    assert_eq!(underlay.layers[part_layer], "doors");
    let (_, circle) = child(lines, Some(leaf), 0xA2);
    assert_eq!(circle.kind.name(), "CIRCLE");
    assert_eq!(lines.objects().layer_name(circle), "detail");
}

#[test]
fn array_insert_copies_keep_distinct_affine_children_and_geometry() {
    let source = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
        "0\nSECTION\n2\nBLOCKS\n",
        "0\nBLOCK\n8\n0\n2\nleaf\n70\n0\n10\n2\n20\n3\n",
        "0\nLINE\n5\nA1\n8\n0\n10\n2\n20\n3\n11\n12\n21\n8\n0\nENDBLK\n",
        "0\nBLOCK\n8\n0\n2\narray\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n5\nB1\n8\n0\n2\nleaf\n10\n10\n20\n20\n50\n30\n",
        "70\n2\n71\n2\n44\n40\n45\n60\n0\nENDBLK\n0\nENDSEC\n",
        "0\nSECTION\n2\nENTITIES\n",
        "0\nINSERT\n5\nC1\n8\ncopies\n2\narray\n10\n1000\n20\n2000\n",
        "41\n-2\n42\n3\n50\n90\n70\n2\n71\n1\n44\n200\n",
        "0\nENDSEC\n0\nEOF\n"
    );
    let drawing = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    let lines = UnderlayLines::decode(&drawing.lines.encode()).unwrap();
    let roots: Vec<_> = lines
        .objects()
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.parent.is_none())
        .collect();
    assert_eq!(roots.len(), 2);
    let (sin, cos) = 30.0_f64.to_radians().sin_cos();
    let mut seen = Vec::new();
    for (outer, (root_id, _)) in roots.iter().enumerate() {
        let children: Vec<_> = lines
            .objects()
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.parent == Some(*root_id as u32))
            .collect();
        assert_eq!(children.len(), 4);
        for (copy, (id, object)) in children.iter().enumerate() {
            let ObjectDetail::Insert {
                transform, copies, ..
            } = &object.detail
            else {
                panic!("insert")
            };
            assert_eq!(*copies, [1, 1]);
            let col = (copy % 2) as f64;
            let row = (copy / 2) as f64;
            let start = [
                10.0 + col * 40.0 * cos - row * 60.0 * sin,
                20.0 + col * 40.0 * sin + row * 60.0 * cos,
            ];
            let end = [
                start[0] + 10.0 * cos - 5.0 * sin,
                start[1] + 10.0 * sin + 5.0 * cos,
            ];
            let world = |p: [f64; 2]| {
                [
                    1000.0 - 3.0 * p[1],
                    2000.0 + outer as f64 * 200.0 - 2.0 * p[0],
                ]
            };
            let (_, line) = child(&lines, Some(*id as u32), 0xA1);
            let points = drawn(&lines, line).0;
            assert!(points.iter().any(|p| near(*p, world(start))), "{points:?}");
            assert!(points.iter().any(|p| near(*p, world(end))), "{points:?}");
            assert!(near(apply(transform, [2.0, 3.0]), world(start)));
            assert!(near(apply(transform, [12.0, 8.0]), world(end)));
            assert!(!seen.iter().any(|p| near(*p, world(start))));
            seen.push(world(start));
        }
    }
    assert_eq!(seen.len(), 8);
}

#[test]
fn a_dimension_keeps_its_measurement_points_and_drawn_text() {
    let underlay = read();
    let lines = &underlay.lines;
    let (index, dimension) = child(lines, None, 0x3B);
    let ObjectDetail::Dimension {
        dimension_type,
        measurement,
        points_mm,
        ..
    } = &dimension.detail
    else {
        panic!("a dimension detail")
    };
    assert_eq!(*dimension_type, 0);
    assert_eq!(*measurement, Some(2630.0));
    assert!(points_mm.contains(&(13, [0.0, 0.0])));
    assert!(points_mm.contains(&(14, [2630.0, 0.0])));
    let (_, text) = child(lines, Some(index), 0xD2);
    assert_eq!(drawn(lines, text).1, ["2630"]);
}

#[test]
fn a_hatch_keeps_its_pattern_and_outline_with_its_islands() {
    let underlay = read();
    let (_, hatch) = child(&underlay.lines, None, 0x4C);
    let ObjectDetail::Hatch {
        pattern,
        solid,
        rings,
    } = &hatch.detail
    else {
        panic!("a hatch detail")
    };
    assert_eq!(pattern, "SOLID");
    assert!(solid);
    assert!((even_odd_area(rings) - 8_400.0).abs() < 1e-6);
    assert_eq!(hatch.parts[0].fills, 0..1);
}

#[test]
fn every_point_drawn_belongs_to_an_object_and_objects_survive_the_blob() {
    let underlay = read();
    let lines = &underlay.lines;
    for (index, layer) in lines.layers().iter().enumerate() {
        let mut owned = vec![false; layer.points_mm().len()];
        for object in &lines.objects().objects {
            for part in object
                .parts
                .iter()
                .filter(|part| part.layer as usize == index)
            {
                owned[part.points.start as usize..part.points.end as usize].fill(true);
            }
        }
        assert!(
            owned.iter().all(|owned| *owned),
            "layer {}",
            underlay.layers[index]
        );
    }
    let top = lines
        .objects()
        .objects
        .iter()
        .filter(|object| object.parent.is_none())
        .map(|object| object.handle)
        .collect::<Vec<_>>();
    assert_eq!(top, [0x1F, 0x2A, 0x3B, 0x4C]);
    let decoded = UnderlayLines::decode(&lines.encode()).unwrap();
    assert_eq!(&decoded, lines);
}
