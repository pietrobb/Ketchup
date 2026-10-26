//! Objects taken out of an underlay's drawing: their lines, texts and fills leave it, a
//! polyline they were part of splits where they were, an insert takes what it placed with
//! it, and every other object keeps its id and its lines. The edits are a few ids the
//! underlay carries, one Undo step each, and name the lines they give.
use ketchup_model::document::DocumentStore;
use ketchup_model::import::{DxfImportOptions, read_dxf_underlay};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::edits::{
    UnderlayEdits, UnderlayObjectError, plan_underlay_object_removal,
};
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};
use ketchup_model::underlay::plan_underlay;

const BLOCKS: &str = concat!(
    "0\nBLOCK\n8\n0\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n5\nA1\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n",
    "0\nENDBLK\n",
);

/// Three lines on `walls` that meet end to end, so the drawing joins them into one
/// polyline; an insert of `leaf` with an attribute; a text; a solid hatch.
fn lines() -> UnderlayLines {
    let line = |handle: &str, from: [i32; 2], to: [i32; 2]| {
        format!(
            "0\nLINE\n5\n{handle}\n8\nwalls\n10\n{}\n20\n{}\n11\n{}\n21\n{}\n",
            from[0], from[1], to[0], to[1]
        )
    };
    let entities = [
        line("10", [0, 0], [100, 0]),
        line("11", [100, 0], [100, 100]),
        line("12", [100, 100], [0, 100]),
        "0\nINSERT\n5\n20\n8\ndoors\n2\nleaf\n10\n500\n20\n0\n66\n1\n".to_owned(),
        "0\nATTRIB\n8\ndoors\n2\nNAME\n1\nD-01\n10\n500\n20\n0\n40\n50\n0\nSEQEND\n".to_owned(),
        "0\nTEXT\n5\n30\n8\nnotes\n10\n50\n20\n50\n40\n10\n1\nWC\n".to_owned(),
        "0\nHATCH\n5\n40\n8\nfloor\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n\
         92\n0\n93\n4\n72\n1\n10\n0\n20\n0\n11\n10\n21\n0\n72\n1\n10\n10\n20\n0\n11\n10\n21\n10\n\
         72\n1\n10\n10\n20\n10\n11\n0\n21\n10\n72\n1\n10\n0\n20\n10\n11\n0\n21\n0\n\
         97\n0\n75\n0\n76\n1\n98\n0\n"
            .to_owned(),
    ]
    .concat();
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nBLOCKS\n{BLOCKS}0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None))
        .unwrap()
        .lines
}

/// The id of the top-level object of handle `handle`.
fn id(lines: &UnderlayLines, handle: u64) -> u32 {
    let index = lines
        .objects()
        .objects
        .iter()
        .position(|object| object.parent.is_none() && object.handle == handle)
        .unwrap_or_else(|| panic!("no object {handle:X}"));
    u32::try_from(index).unwrap()
}

fn removing(ids: &[u32]) -> UnderlayEdits {
    UnderlayEdits {
        removed_objects: ids.iter().copied().collect(),
    }
}

/// Every polyline of every layer.
fn polylines(lines: &UnderlayLines) -> Vec<Vec<[f64; 2]>> {
    lines
        .layers()
        .iter()
        .flat_map(|layer| layer.polylines().map(<[[f64; 2]]>::to_vec))
        .collect()
}

#[test]
fn removing_the_middle_of_joined_lines_splits_their_polyline() {
    let lines = lines();
    let joined = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    assert!(polylines(&lines).contains(&joined.to_vec()));
    let middle = id(&lines, 0x11);
    let edited = lines.clone().edited(&removing(&[middle]));
    let after = polylines(&edited);
    assert!(after.contains(&vec![[0.0, 0.0], [100.0, 0.0]]), "{after:?}");
    assert!(
        after.contains(&vec![[100.0, 100.0], [0.0, 100.0]]),
        "{after:?}"
    );
    assert!(
        !after
            .iter()
            .any(|polyline| polyline.contains(&[100.0, 50.0]))
    );
    assert_eq!(edited.segment_count(), lines.segment_count() - 1);
    // The others keep their ids and their lines; the removed one is gone.
    let index = ObjectIndex::new(&edited);
    assert!(index.object(middle).is_none());
    assert_eq!(
        index.own_polylines(id(&lines, 0x10)),
        vec![&[[0.0, 0.0], [100.0, 0.0]][..]]
    );
    assert_eq!(
        index.own_polylines(id(&lines, 0x12)),
        vec![&[[100.0, 100.0], [0.0, 100.0]][..]]
    );
    assert_eq!(index.present(), ObjectIndex::new(&lines).present() - 1);
    let all = ObjectFilter {
        top_level: false,
        ..ObjectFilter::default()
    };
    assert!(!index.find(&all).contains(&middle));
}

#[test]
fn an_insert_takes_its_block_and_attributes_with_it() {
    let lines = lines();
    let door = id(&lines, 0x20);
    let before = ObjectIndex::new(&lines);
    let children = before.children(door).to_vec();
    assert!(!children.is_empty());
    assert!(lines.text_count() >= 2);
    let edited = lines.clone().edited(&removing(&[door]));
    let index = ObjectIndex::new(&edited);
    for object in std::iter::once(door).chain(children) {
        assert!(index.object(object).is_none(), "{object}");
        assert_eq!(index.bounds(object), None);
    }
    let texts = edited
        .layers()
        .iter()
        .flat_map(|layer| layer.texts())
        .map(|text| text.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["WC"]);
    assert_eq!(edited.segment_count(), lines.segment_count() - 1);
    assert_eq!(index.block_counts().get("leaf"), None);
}

#[test]
fn a_text_and_a_hatch_leave_with_their_objects() {
    let lines = lines();
    let edited = lines
        .clone()
        .edited(&removing(&[id(&lines, 0x30), id(&lines, 0x40)]));
    assert_eq!(edited.text_count(), lines.text_count() - 1);
    assert_eq!(lines.fill_count(), 1);
    assert_eq!(edited.fill_count(), 0);
    // What the walls draw is untouched.
    assert_eq!(
        ObjectIndex::new(&edited).own_polylines(id(&lines, 0x11)),
        vec![&[[100.0, 0.0], [100.0, 100.0]][..]]
    );
}

#[test]
fn no_edits_are_the_lines_as_read() {
    let lines = lines();
    assert_eq!(lines.clone().edited(&UnderlayEdits::default()), lines);
}

#[test]
fn a_removal_is_one_undo_step_that_names_new_lines() {
    let lines = lines();
    let mut document = DocumentStore::new();
    let (batch, underlay_id) = plan_underlay(
        &document.current(),
        "plan",
        &[
            "walls".into(),
            "doors".into(),
            "notes".into(),
            "floor".into(),
        ],
        &[],
        [7; 32],
        [0.0, 0.0],
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let before = document.current().underlay(underlay_id).unwrap().clone();
    assert_eq!(before.lines_key(), before.lines_sha256);

    let door = id(&lines, 0x20);
    let child = ObjectIndex::new(&lines).children(door)[0];
    let snapshot = document.current();
    let plan = |ids: &[u32]| plan_underlay_object_removal(&snapshot, underlay_id, &lines, ids);
    assert_eq!(plan(&[]).unwrap_err(), UnderlayObjectError::Empty);
    assert_eq!(
        plan(&[9_999]).unwrap_err(),
        UnderlayObjectError::NotFound(9_999)
    );
    assert_eq!(
        plan(&[child]).unwrap_err(),
        UnderlayObjectError::Placed {
            object: child,
            parent: door
        }
    );
    assert_eq!(
        plan_underlay_object_removal(&snapshot, underlay_id, &UnderlayLines::default(), &[0])
            .unwrap_err(),
        UnderlayObjectError::NoObjects
    );

    let steps = document.visible_undo_steps();
    document.apply_batch(&plan(&[door, door]).unwrap()).unwrap();
    assert_eq!(document.visible_undo_steps(), steps + 1);
    let after = document.current().underlay(underlay_id).unwrap().clone();
    assert_eq!(after.edits, removing(&[door]));
    assert_eq!(after.lines_sha256, before.lines_sha256);
    assert_ne!(after.lines_key(), before.lines_key());

    // Removed once, it is not found again.
    let edited = lines.clone().edited(&after.edits);
    assert_eq!(
        plan_underlay_object_removal(&document.current(), underlay_id, &edited, &[door])
            .unwrap_err(),
        UnderlayObjectError::NotFound(door)
    );
    document.undo().unwrap();
    assert_eq!(document.current().underlay(underlay_id), Some(&before));
}
