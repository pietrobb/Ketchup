use std::collections::BTreeMap;

use crate::{dwg_to_whole_dxf, dxf_to_dwg};

/// A plan: walls A0, a line A1, a text A2 and a hatch A3 in model space (1F), the hatch sent
/// to the back by the model space's sort table F, so the drawing is drawn hatch, walls, line,
/// text.
fn plan() -> String {
    let entities = concat!(
        "0\nLWPOLYLINE\n5\nA0\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbPolyline\n90\n4\n70\n1\n",
        "10\n0\n20\n0\n10\n4000\n20\n0\n10\n4000\n20\n3000\n10\n0\n20\n3000\n",
        "0\nLINE\n5\nA1\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n",
        "10\n0\n20\n1500\n30\n0\n11\n4000\n21\n1500\n31\n0\n",
        "0\nTEXT\n5\nA2\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbText\n",
        "10\n500\n20\n500\n30\n0\n40\n250\n1\nKitchen\n100\nAcDbText\n",
        "0\nHATCH\n5\nA3\n330\n1F\n100\nAcDbEntity\n8\n0\n62\n8\n100\nAcDbHatch\n",
        "10\n0\n20\n0\n30\n0\n210\n0\n220\n0\n230\n1\n2\nSOLID\n70\n1\n71\n0\n91\n1\n",
        "92\n3\n72\n0\n73\n1\n93\n4\n10\n0\n20\n0\n10\n4000\n20\n0\n10\n4000\n20\n3000\n10\n0\n20\n3000\n",
        "97\n0\n75\n0\n76\n1\n98\n0\n",
    );
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$INSUNITS\n70\n4\n9\n$HANDSEED\n5\n200\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n\
         0\nTABLE\n2\nBLOCK_RECORD\n5\n1\n330\n0\n100\nAcDbSymbolTable\n70\n2\n\
         0\nBLOCK_RECORD\n5\n1F\n102\n{{ACAD_XDICTIONARY\n360\nE\n102\n}}\n330\n1\n100\nAcDbSymbolTableRecord\n\
         100\nAcDbBlockTableRecord\n2\n*Model_Space\n\
         0\nBLOCK_RECORD\n5\n1E\n330\n1\n100\nAcDbSymbolTableRecord\n100\nAcDbBlockTableRecord\n2\n*Paper_Space\n\
         0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n\
         0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n281\n1\n\
         0\nDICTIONARY\n5\nE\n330\n1F\n100\nAcDbDictionary\n280\n1\n281\n1\n3\nACAD_SORTENTS\n360\nF\n\
         0\nSORTENTSTABLE\n5\nF\n102\n{{ACAD_REACTORS\n330\nE\n102\n}}\n330\nE\n100\nAcDbSortentsTable\n\
         330\n1F\n331\nA3\n5\n9F\n0\nENDSEC\n0\nEOF\n"
    )
}

/// (code, value) pairs of a DXF text.
fn pairs(dxf: &[u8]) -> Vec<(i32, String)> {
    let text = String::from_utf8_lossy(dxf);
    let lines: Vec<&str> = text.lines().collect();
    lines
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| (pair[0].trim().parse().unwrap(), pair[1].trim().to_owned()))
        .collect()
}

/// The kinds of the model space's entities in the order AutoCAD draws them, last on top: by
/// their sort handle in a sort table of model space, otherwise their own handle, then file
/// order.
fn drawn(dxf: &[u8]) -> Vec<String> {
    let pairs = pairs(dxf);
    // The handle of the BLOCK_RECORD named *Model_Space.
    let model_space = pairs
        .iter()
        .position(|pair| *pair == (2, String::from("*Model_Space")))
        .and_then(|at| {
            let start = pairs[..at].iter().rposition(|(code, _)| *code == 0)?;
            pairs[start..at]
                .iter()
                .find(|(code, _)| *code == 5)
                .map(|(_, handle)| handle.clone())
        })
        .expect("a model space");
    let records: Vec<&[(i32, String)]> = {
        let starts: Vec<usize> = (0..pairs.len()).filter(|at| pairs[*at].0 == 0).collect();
        starts
            .iter()
            .zip(starts.iter().skip(1).chain([&pairs.len()]))
            .map(|(start, end)| &pairs[*start..*end])
            .collect()
    };
    let mut sorts = BTreeMap::new();
    for record in records.iter().filter(|r| r[0].1 == "SORTENTSTABLE") {
        let marker = record
            .iter()
            .position(|pair| *pair == (100, String::from("AcDbSortentsTable")))
            .expect("the subclass marker");
        let rest = &record[marker + 1..];
        if rest.first().map(|(_, owner)| owner) != Some(&model_space) {
            continue;
        }
        for entry in rest.windows(2) {
            if entry[0].0 == 331 && entry[1].0 == 5 {
                sorts.insert(entry[0].1.clone(), entry[1].1.clone());
            }
        }
    }
    let hex = |value: &str| u64::from_str_radix(value, 16).expect("a hex handle");
    let mut in_entities = false;
    let mut entities = Vec::new();
    for record in &records {
        if record[0].1 == "SECTION" {
            in_entities = record.get(1).is_some_and(|(_, name)| name == "ENTITIES");
            continue;
        }
        if !in_entities || record[0].1 == "ENDSEC" {
            continue;
        }
        let Some((_, handle)) = record.iter().find(|(code, _)| *code == 5) else {
            continue;
        };
        let key = sorts
            .get(handle)
            .map_or_else(|| hex(handle), |key| hex(key));
        entities.push((key, entities.len(), record[0].1.clone()));
    }
    entities.sort();
    entities.into_iter().map(|(_, _, kind)| kind).collect()
}

/// Review 2026-11-25 og DXF-1002: a DWG drawing's DXF keeps its draw order. A hatch sent to
/// the back was drawn over the walls, as acadrust gave the table's entries new sort handles
/// above every handle of the drawing.
#[test]
fn a_hatch_sent_to_the_back_in_a_dwg_stays_under_the_walls_in_its_dxf() {
    let source = plan();
    let expected = ["HATCH", "LWPOLYLINE", "LINE", "TEXT"];
    assert_eq!(drawn(source.as_bytes()), expected, "the source plan");
    let dwg = dxf_to_dwg(source.as_bytes()).expect("the plan is written as DWG");
    let whole = dwg_to_whole_dxf(&dwg, usize::MAX).expect("the DWG reads");
    assert_eq!(drawn(&whole.dxf), expected);
}
