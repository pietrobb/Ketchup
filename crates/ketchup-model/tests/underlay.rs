//! A DXF drawing read as an underlay: every line kept by layer, no profile limits, one
//! Undo step to add or remove it, and lines that survive their container blob.
use ketchup_model::document::{DocumentStore, TagId};
use ketchup_model::graph::sha256_bytes;
use ketchup_model::import::{DxfImportOptions, read_dxf_underlay};
use ketchup_model::underlay::{
    UnderlayLines, UnderlayLinesError, plan_underlay, plan_underlay_removal,
};

fn dxf(entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn line(layer: &str, from: [f64; 2], to: [f64; 2]) -> String {
    format!(
        "0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n11\n{}\n21\n{}\n",
        from[0], from[1], to[0], to[1]
    )
}

fn read(entities: &str) -> ketchup_model::import::DxfUnderlay {
    read_dxf_underlay(&dxf(entities), DxfImportOptions::new(None)).unwrap()
}

#[test]
fn a_drawing_past_every_profile_limit_reads_whole_and_by_layer() {
    // 12,000 lines on two layers: past the 10,000 entities, ~170 profiles and 1,024
    // segments a profile import keeps to.
    let mut entities = String::new();
    for index in 0..6_000 {
        let x = f64::from(index);
        entities.push_str(&line("walls", [x, 0.0], [x + 1.0, 0.0]));
        entities.push_str(&line("doors", [x, 10.0 + x], [x, 11.0 + x]));
    }
    let underlay = read(&entities);
    assert_eq!(underlay.layers, ["walls", "doors"]);
    assert_eq!(underlay.lines.segment_count(), 12_000);
    let [walls, doors] = underlay.lines.layers() else {
        panic!("two layers")
    };
    // The walls meet end to end: one polyline of 6,000 segments. The doors touch nothing.
    assert_eq!(walls.polylines().count(), 1);
    assert_eq!(walls.segment_count(), 6_000);
    assert_eq!(doors.polylines().count(), 6_000);
}

#[test]
fn lines_join_only_where_two_of_them_meet_alone() {
    // A ring of three, and a T: the stem meets two others at one point.
    let entities = [
        line("0", [0.0, 0.0], [10.0, 0.0]),
        line("0", [10.0, 0.0], [10.0, 10.0]),
        line("0", [0.0, 0.0], [10.0, 10.0]),
        line("0", [20.0, 0.0], [30.0, 0.0]),
        line("0", [30.0, 0.0], [40.0, 0.0]),
        line("0", [30.0, 0.0], [30.0, 10.0]),
    ]
    .concat();
    let underlay = read(&entities);
    let polylines = underlay.lines.layers()[0]
        .polylines()
        .map(<[_]>::to_vec)
        .collect::<Vec<_>>();
    assert_eq!(underlay.lines.segment_count(), 6);
    assert_eq!(polylines.len(), 4);
    let ring = polylines.iter().find(|points| points.len() == 4).unwrap();
    assert_eq!(ring.first(), ring.last());
}

#[test]
fn an_entity_it_cannot_read_is_left_out_and_counted() {
    let entities = [
        line("0", [0.0, 0.0], [10.0, 0.0]),
        "0\nLINE\n8\n0\n10\nnot-a-number\n20\n0\n11\n1\n21\n0\n".to_owned(),
    ]
    .concat();
    let underlay = read(&entities);
    assert_eq!(underlay.lines.segment_count(), 1);
    let unreadable = underlay
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == "dxf.entity-unreadable")
        .unwrap();
    assert_eq!(unreadable.count(), 1);
    assert!(unreadable.subject().unwrap().starts_with("LINE: "));
}

#[test]
fn lines_decode_as_they_were_encoded_and_damaged_bytes_are_refused() {
    let underlay = read(&line("0", [0.0, 0.0], [10.0, 0.0]));
    let bytes = underlay.lines.encode();
    assert_eq!(UnderlayLines::decode(&bytes).unwrap(), underlay.lines);
    assert_eq!(
        UnderlayLines::decode(&bytes[..bytes.len() - 1]),
        Err(UnderlayLinesError::Truncated)
    );
    assert_eq!(
        UnderlayLines::decode(b"not lines"),
        Err(UnderlayLinesError::NotUnderlayLines)
    );
}

#[test]
fn an_underlay_is_added_and_removed_with_its_tags_in_one_undo_step_each() {
    let underlay = read(
        &[
            line("walls", [0.0, 0.0], [1.0, 0.0]),
            line("doors", [5.0, 0.0], [5.0, 1.0]),
        ]
        .concat(),
    );
    let lines = sha256_bytes(&underlay.lines.encode());
    let mut document = DocumentStore::new();
    let (batch, id) = plan_underlay(&document.current(), "Plan", &underlay.layers, lines).unwrap();
    document.apply_batch(&batch).unwrap();
    let added = document.current();
    let stored = added.underlay(id).unwrap();
    assert_eq!(stored.layers.len(), 2);
    let tags = stored.tags().collect::<Vec<TagId>>();
    assert_eq!(tags.len(), 3);
    assert!(tags.iter().all(|tag| added.tag(*tag).is_some()));
    // No part: nothing that counts or makes parts meets it.
    assert_eq!(added.occurrences().count(), 0);

    document
        .apply_batch(&plan_underlay_removal(&added, id).unwrap())
        .unwrap();
    let removed = document.current();
    assert!(removed.underlay(id).is_none());
    assert!(tags.iter().all(|tag| removed.tag(*tag).is_none()));

    document.undo().unwrap();
    assert!(document.current().underlay(id).is_some());
    document.undo().unwrap();
    assert!(document.current().underlay(id).is_none());
    assert_eq!(document.current().tags().count(), 0);
}
