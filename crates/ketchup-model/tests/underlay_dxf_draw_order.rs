//! Review 2026-11-24 og DXF-900: the fills drawn into a drawing (the areas of rooms) lie under
//! the architect's lines in the order AutoCAD draws, not only in the file. AutoCAD and ezdxf
//! draw a space by the sort handles of its `ACAD_SORTENTS` table, an entity not in it by its
//! own handle; a fill written first in ENTITIES but with the highest handle was drawn over
//! every line, and in DXF 2000–2007, without transparency, hid the walls and texts under it.
use ketchup_model::import::{DxfImportOptions, read_dxf_underlay, write_edited_dxf};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::drawn::{AddedObject, DrawnShape};
use ketchup_model::underlay::edits::UnderlayEdits;

const MODEL_SPACE: &str = "0\nBLOCK_RECORD\n5\n1F\n{XDICTIONARY}330\n1\n100\nAcDbSymbolTableRecord\n\
                           100\nAcDbBlockTableRecord\n2\n*Model_Space\n";

/// A plan of version `version`: walls A0, a line A1 and a text A2 in model space, a line A3
/// in paper space, the model space's extension dictionary `xdictionary` (its 102 group) and
/// the records `objects` in the OBJECTS section after the root dictionary.
fn plan(version: &str, xdictionary: &str, objects: &str) -> String {
    let model_space = MODEL_SPACE.replace("{XDICTIONARY}", xdictionary);
    let entities = concat!(
        "0\nLWPOLYLINE\n5\nA0\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbPolyline\n90\n4\n70\n1\n",
        "10\n0\n20\n0\n10\n4000\n20\n0\n10\n4000\n20\n3000\n10\n0\n20\n3000\n",
        "0\nLINE\n5\nA1\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbLine\n",
        "10\n0\n20\n1500\n30\n0\n11\n4000\n21\n1500\n31\n0\n",
        "0\nTEXT\n5\nA2\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbText\n",
        "10\n500\n20\n500\n30\n0\n40\n250\n1\nKitchen\n100\nAcDbText\n",
        "0\nLINE\n5\nA3\n330\n1E\n100\nAcDbEntity\n67\n1\n8\nwalls\n100\nAcDbLine\n",
        "10\n0\n20\n0\n30\n0\n11\n100\n21\n0\n31\n0\n",
    );
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\n{version}\n9\n$INSUNITS\n70\n4\n9\n$HANDSEED\n5\n200\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n5\n2\n70\n1\n\
         0\nLAYER\n5\n10\n100\nAcDbSymbolTableRecord\n100\nAcDbLayerTableRecord\n2\nwalls\n70\n0\n62\n1\n6\nCONTINUOUS\n\
         0\nENDTAB\n0\nTABLE\n2\nBLOCK_RECORD\n5\n1\n70\n2\n{model_space}\
         0\nBLOCK_RECORD\n5\n1E\n330\n1\n100\nAcDbSymbolTableRecord\n100\nAcDbBlockTableRecord\n2\n*Paper_Space\n\
         0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n\
         0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n281\n1\n3\nACAD_GROUP\n350\nD\n\
         0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n281\n1\n{objects}0\nENDSEC\n0\nEOF\n"
    )
}

/// The records of `text` with the section each is in, as (code, value) pairs.
fn records(text: &str) -> Vec<(String, Vec<(i32, String)>)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut records: Vec<(String, Vec<(i32, String)>)> = Vec::new();
    let mut section = String::new();
    for pair in lines.chunks(2) {
        let (code, value) = (pair[0].trim().parse::<i32>().unwrap(), pair[1].trim());
        if code == 0 {
            records.push((section.clone(), Vec::new()));
        }
        if let Some((_, record)) = records.last_mut() {
            record.push((code, value.to_owned()));
        }
        if code == 2 && records.last().is_some_and(|(_, r)| r[0].1 == "SECTION") {
            section = value.to_owned();
        }
    }
    records
}

fn group(record: &[(i32, String)], code: i32) -> Option<&str> {
    record
        .iter()
        .find(|(at, _)| *at == code)
        .map(|(_, value)| value.as_str())
}

/// The first 330 of `record` outside its 102 groups.
fn owner(record: &[(i32, String)]) -> Option<&str> {
    let mut in_group = false;
    record.iter().find_map(|(code, value)| {
        if *code == 102 {
            in_group = value.starts_with('{');
        }
        (*code == 330 && !in_group).then_some(value.as_str())
    })
}

/// The handles of model space's entities in the order AutoCAD draws them, last on top: by
/// their sort handle in the model space's sort table, otherwise their own, then file order.
/// And the model space's sort tables, each by its handle.
fn draw_order(text: &str) -> (Vec<String>, Vec<String>) {
    let records = records(text);
    let mut sorts = std::collections::BTreeMap::new();
    let mut tables = Vec::new();
    for (_, record) in records.iter().filter(|(_, r)| r[0].1 == "SORTENTSTABLE") {
        let marker = record
            .iter()
            .position(|(code, value)| *code == 100 && value == "AcDbSortentsTable")
            .unwrap();
        let rest = &record[marker + 1..];
        if group(rest, 330) != Some("1F") {
            continue;
        }
        tables.push(group(record, 5).unwrap().to_owned());
        for entry in rest.windows(2) {
            if entry[0].0 == 331 && entry[1].0 == 5 {
                sorts.insert(entry[0].1.clone(), entry[1].1.clone());
            }
        }
    }
    let hex = |value: &str| u64::from_str_radix(value, 16).unwrap();
    let mut entities: Vec<(u64, usize, String)> = records
        .iter()
        .filter(|(section, record)| section == "ENTITIES" && group(record, 5).is_some())
        // Paper space's are owned by its block record or marked by 67; an export before
        // DXF-1000 gave a model space entity a group's handle for its owner.
        .filter(|(_, record)| owner(record) != Some("1E") && group(record, 67) != Some("1"))
        .enumerate()
        .map(|(at, (_, record))| {
            let handle = group(record, 5).unwrap().to_owned();
            let key = sorts.get(&handle).map_or(hex(&handle), |key| hex(key));
            (key, at, handle)
        })
        .collect();
    entities.sort();
    (entities.into_iter().map(|(_, _, h)| h).collect(), tables)
}

/// The record of handle `handle`.
fn record_of(text: &str, handle: &str) -> Vec<(i32, String)> {
    records(text)
        .into_iter()
        .map(|(_, record)| record)
        .find(|record| group(record, 5) == Some(handle))
        .unwrap_or_else(|| panic!("no record {handle}"))
}

/// The handle the model space's extension dictionary names for `ACAD_SORTENTS`.
fn sort_table_named(text: &str) -> String {
    let space = record_of(text, "1F");
    let at = space
        .iter()
        .position(|(code, value)| *code == 102 && value == "{ACAD_XDICTIONARY")
        .expect("model space has an extension dictionary");
    assert_eq!(space[at + 1].0, 360);
    let dictionary = record_of(text, &space[at + 1].1);
    let entry = dictionary
        .iter()
        .position(|(code, value)| *code == 3 && value == "ACAD_SORTENTS")
        .expect("ACAD_SORTENTS");
    dictionary[entry + 1].1.clone()
}

/// The CLASS records of SORTENTSTABLE in the CLASSES section of `text`.
fn sort_table_classes(text: &str) -> Vec<Vec<(i32, String)>> {
    records(text)
        .into_iter()
        .filter(|(section, record)| section == "CLASSES" && record[0].1 == "CLASS")
        .filter(|(_, record)| group(record, 1) == Some("SORTENTSTABLE"))
        .map(|(_, record)| record)
        .collect()
}

fn written(source: &str, edits: impl FnOnce(&UnderlayLines) -> UnderlayEdits) -> String {
    let read = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    let edits = edits(&read.lines);
    assert!(edits.is_valid());
    let written = write_edited_dxf(
        source.as_bytes(),
        DxfImportOptions::new(None),
        &read.lines,
        &edits,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    String::from_utf8(written.dxf).unwrap()
}

fn id_of(lines: &UnderlayLines, handle: u64) -> u32 {
    let at = lines
        .objects()
        .objects
        .iter()
        .position(|object| object.parent.is_none() && object.handle == handle)
        .unwrap_or_else(|| panic!("no object {handle:X}"));
    u32::try_from(at).unwrap()
}

fn fill(corner: f64) -> AddedObject {
    AddedObject::Drawn {
        layer: "walls".to_owned(),
        shape: DrawnShape::Hatch {
            rings_mm: vec![vec![
                [corner, corner],
                [corner + 4000.0, corner],
                [corner + 4000.0, corner + 3000.0],
                [corner, corner + 3000.0],
            ]],
        },
    }
}

/// The handles of the HATCH records of `text`, in file order.
fn hatches(text: &str) -> Vec<String> {
    records(text)
        .into_iter()
        .filter(|(section, record)| section == "ENTITIES" && record[0].1 == "HATCH")
        .map(|(_, record)| group(&record, 5).unwrap().to_owned())
        .collect()
}

#[test]
fn fills_drawn_into_a_drawing_lie_under_its_lines_in_the_order_autocad_draws() {
    for version in ["AC1015", "AC1018", "AC1024", "AC1032"] {
        let source = plan(version, "", "");
        // A copy and a line drawn before the fill in the edits: they still lie over it all.
        let text = written(&source, |lines| {
            let mut edits = UnderlayEdits::default();
            edits.added.push(AddedObject::Copy {
                of: id_of(lines, 0xA1),
                map: [[1.0, 0.0, 0.0], [0.0, 1.0, 500.0]],
            });
            edits.added.push(AddedObject::Drawn {
                layer: "walls".to_owned(),
                shape: DrawnShape::Polyline {
                    points_mm: vec![[0.0, 0.0], [100.0, 100.0]],
                    closed: false,
                },
            });
            edits.added.push(fill(0.0));
            edits
        });
        let [hatch] = hatches(&text).try_into().unwrap();
        let (order, tables) = draw_order(&text);
        assert_eq!(order.len(), 6, "{version}: {order:?}");
        assert_eq!(
            order[..4],
            [hatch.as_str(), "A0", "A1", "A2"],
            "{version}: the fill under every line of the drawing, theirs kept"
        );
        assert_eq!(tables.len(), 1, "{version}: one sort table");
        assert_eq!(sort_table_named(&text), tables[0], "{version}");
        // Its class listed, in a CLASSES section before TABLES (DXF-1004).
        assert_eq!(sort_table_classes(&text).len(), 1, "{text}");
        let sections: Vec<_> = records(&text)
            .into_iter()
            .filter(|(_, record)| record[0].1 == "SECTION")
            .filter_map(|(_, record)| group(&record, 2).map(str::to_owned))
            .collect();
        assert_eq!(sections[..3], ["HEADER", "CLASSES", "TABLES"], "{version}");
        // Paper space is not in model space's table.
        let table = record_of(&text, &tables[0]);
        assert!(
            !table
                .iter()
                .any(|(code, value)| *code == 331 && value == "A3")
        );
        assert_eq!(owner(&table), group(&record_of(&text, "1F"), 360));
        // The new records' handles lie below the handle seed.
        let seed = text.split("$HANDSEED\n5\n").nth(1).unwrap().lines().next();
        let seed = u64::from_str_radix(seed.unwrap().trim(), 16).unwrap();
        for record in records(&text) {
            if let Some(handle) = group(&record.1, 5).filter(|_| record.1[0].1 != "SECTION") {
                assert!(u64::from_str_radix(handle, 16).unwrap() < seed, "{handle}");
            }
        }

        // Written again with another fill: one table still, the new fill under the old.
        let again = written(&text, |_| {
            let mut edits = UnderlayEdits::default();
            edits.added.push(fill(1000.0));
            edits
        });
        let all = hatches(&again);
        let newer = all.iter().find(|h| **h != hatch).unwrap();
        let (order, tables) = draw_order(&again);
        assert_eq!(
            order[..5],
            [newer.as_str(), hatch.as_str(), "A0", "A1", "A2"],
            "{version}"
        );
        assert_eq!(tables.len(), 1, "{version}: {again}");
        assert_eq!(again.matches("ACAD_SORTENTS").count(), 1, "{version}");
        assert_eq!(sort_table_classes(&again).len(), 1, "{version}");
    }
}

/// The drawing's own draw order is kept over the fill, in the table it has or in an
/// extension dictionary it has without one; an entity taken out leaves no entry.
#[test]
fn a_drawings_own_draw_order_is_kept_over_the_fills() {
    let xdictionary = "102\n{ACAD_XDICTIONARY\n360\nE\n102\n}\n";
    let dictionary = "0\nDICTIONARY\n5\nE\n330\n1F\n100\nAcDbDictionary\n280\n1\n281\n1\n";
    let sorted = format!(
        "{dictionary}3\nACAD_SORTENTS\n360\nF\n\
         0\nSORTENTSTABLE\n5\nF\n102\n{{ACAD_REACTORS\n330\nE\n102\n}}\n330\nE\n100\nAcDbSortentsTable\n\
         330\n1F\n331\nA2\n5\nA0\n331\nA0\n5\nA2\n"
    );
    // A2 drawn first, then A1, then A0.
    let source = plan("AC1015", xdictionary, &sorted);
    assert_eq!(draw_order(&source).0, ["A2", "A1", "A0"]);
    let text = written(&source, |lines| {
        let mut edits = UnderlayEdits::default();
        edits.removed_objects.insert(id_of(lines, 0xA1));
        edits.added.push(fill(0.0));
        edits
    });
    let [hatch] = hatches(&text).try_into().unwrap();
    let (order, tables) = draw_order(&text);
    assert_eq!(order, [hatch.as_str(), "A2", "A0"], "{text}");
    assert_eq!(tables, ["F"], "the drawing's own table, rewritten");
    assert!(!record_of(&text, "F").iter().any(|(_, value)| value == "A1"));
    assert_eq!(text.matches("ACAD_SORTENTS").count(), 1);

    // An extension dictionary without a sort table gets one.
    let source = plan("AC1024", xdictionary, dictionary);
    let text = written(&source, |_| {
        let mut edits = UnderlayEdits::default();
        edits.added.push(fill(0.0));
        edits
    });
    let [hatch] = hatches(&text).try_into().unwrap();
    let (order, tables) = draw_order(&text);
    assert_eq!(order, [hatch.as_str(), "A0", "A1", "A2"], "{text}");
    assert_eq!(sort_table_named(&text), tables[0]);
    assert_eq!(owner(&record_of(&text, &tables[0])), Some("E"));
    assert_eq!(text.matches("{ACAD_XDICTIONARY").count(), 1);
}

/// Review 2026-11-25 og DXF-1000: when the first entity of model space is in a group (its
/// reactor names the GROUP), the entities added are still owned by model space, so a second
/// export keeps the earlier fill in model space's table under the lines.
#[test]
fn entities_added_are_owned_by_model_space_when_the_first_entity_is_in_a_group() {
    let group = "0\nGROUP\n5\n3C\n102\n{ACAD_REACTORS\n330\nD\n102\n}\n330\nD\n100\nAcDbGroup\n\
                 300\n\n70\n0\n71\n1\n340\nA0\n";
    let classes = "0\nSECTION\n2\nCLASSES\n0\nCLASS\n1\nGROUP\n2\nAcDbGroup\n3\nObjectDBX Classes\n\
                   90\n0\n91\n1\n280\n0\n281\n0\n0\nENDSEC\n";
    for version in ["AC1015", "AC1032"] {
        let source = plan(version, "", group)
            .replacen(
                "5\nA0\n330\n1F\n",
                "5\nA0\n102\n{ACAD_REACTORS\n330\n3C\n102\n}\n330\n1F\n",
                1,
            )
            .replacen(
                "0\nSECTION\n2\nTABLES\n",
                &format!("{classes}0\nSECTION\n2\nTABLES\n"),
                1,
            );
        assert_eq!(draw_order(&source).0, ["A0", "A1", "A2"]);
        let text = written(&source, |_| {
            let mut edits = UnderlayEdits::default();
            edits.added.push(fill(0.0));
            edits
        });
        // The class of the new sort table added to the drawing's classes, counted as they are
        // (DXF-1004).
        let class = [
            (0, "CLASS"),
            (1, "SORTENTSTABLE"),
            (2, "AcDbSortentsTable"),
            (3, "ObjectDBX Classes"),
            (90, "0"),
            (91, "1"),
            (280, "0"),
            (281, "0"),
        ]
        .map(|(code, value)| (code, value.to_owned()));
        assert_eq!(sort_table_classes(&text), [class.to_vec()], "{text}");
        assert_eq!(text.matches("CLASSES").count(), 1, "{text}");
        let [hatch] = hatches(&text).try_into().unwrap();
        assert_eq!(
            owner(&record_of(&text, &hatch)),
            Some("1F"),
            "{version}: {text}"
        );
        // Also over an export before the fix, whose fill is owned by the group.
        let before = text.replacen(
            &format!("5\n{hatch}\n330\n1F\n"),
            &format!("5\n{hatch}\n330\n3C\n"),
            1,
        );
        assert_ne!(before, text);
        for earlier in [text, before] {
            let again = written(&earlier, |_| {
                let mut edits = UnderlayEdits::default();
                edits.added.push(fill(1000.0));
                edits
            });
            let all = hatches(&again);
            let newer = all.iter().find(|h| **h != hatch).unwrap();
            assert_eq!(owner(&record_of(&again, newer)), Some("1F"), "{version}");
            assert_eq!(
                draw_order(&again).0,
                [newer.as_str(), hatch.as_str(), "A0", "A1", "A2"],
                "{version}: both fills under the lines"
            );
        }
    }
}
