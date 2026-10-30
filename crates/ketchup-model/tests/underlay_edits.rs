//! Objects taken out of an underlay's drawing: their lines, texts and fills leave it, a
//! polyline they were part of splits where they were, an insert takes what it placed with
//! it, and every other object keeps its id and its lines. The edits are a few ids the
//! underlay carries, one Undo step each, and name the lines they give.
use ketchup_model::document::DocumentStore;
use ketchup_model::import::{DxfImportOptions, read_dxf_underlay};
use ketchup_model::underlay::drawn::{AddedObject, DrawnShape};
use ketchup_model::underlay::edits::{
    UnderlayEdits, UnderlayObjectError, plan_underlay_drawing, plan_underlay_object_removal,
    plan_underlay_object_transform,
};
use ketchup_model::underlay::line_edit::{LineEdit, plan_underlay_line_edit};
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};
use ketchup_model::underlay::plan_underlay;
use ketchup_model::underlay::{UnderlayId, UnderlayLines};

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
fn id_of(lines: &UnderlayLines, handle: u64) -> u32 {
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
        ..UnderlayEdits::default()
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
    let middle = id_of(&lines, 0x11);
    let edited = lines.clone().edited(&removing(&[middle]), &[]);
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
        index.own_polylines(id_of(&lines, 0x10)),
        vec![&[[0.0, 0.0], [100.0, 0.0]][..]]
    );
    assert_eq!(
        index.own_polylines(id_of(&lines, 0x12)),
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
    let door = id_of(&lines, 0x20);
    let before = ObjectIndex::new(&lines);
    let children = before.children(door).to_vec();
    assert!(!children.is_empty());
    assert!(lines.text_count() >= 2);
    let edited = lines.clone().edited(&removing(&[door]), &[]);
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
        .edited(&removing(&[id_of(&lines, 0x30), id_of(&lines, 0x40)]), &[]);
    assert_eq!(edited.text_count(), lines.text_count() - 1);
    assert_eq!(lines.fill_count(), 1);
    assert_eq!(edited.fill_count(), 0);
    // What the walls draw is untouched.
    assert_eq!(
        ObjectIndex::new(&edited).own_polylines(id_of(&lines, 0x11)),
        vec![&[[100.0, 0.0], [100.0, 100.0]][..]]
    );
}

#[test]
fn no_edits_are_the_lines_as_read() {
    let lines = lines();
    assert_eq!(lines.clone().edited(&UnderlayEdits::default(), &[]), lines);
}

/// A document with an underlay of the layers of [`lines`].
fn document() -> (DocumentStore, UnderlayId) {
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
    (document, underlay_id)
}

/// The lines of underlay `id` in `document` as drawn now.
fn drawn(document: &DocumentStore, id: UnderlayId) -> UnderlayLines {
    let underlay = document.current().underlay(id).unwrap().clone();
    lines().edited(&underlay.edits, &underlay.layers)
}

fn moving(by: [f64; 2]) -> [[f64; 3]; 2] {
    [[1.0, 0.0, by[0]], [0.0, 1.0, by[1]]]
}

fn text_at(lines: &UnderlayLines, text: &str) -> Vec<[f64; 2]> {
    lines
        .layers()
        .iter()
        .flat_map(|layer| layer.texts())
        .filter(|drawn| drawn.text == text)
        .map(|drawn| drawn.anchor_mm)
        .collect()
}

#[test]
fn drawn_shapes_are_new_objects_after_the_drawings_own() {
    let (mut document, id) = document();
    let read = lines().objects().objects.len();
    let shapes = vec![
        DrawnShape::Polyline {
            points_mm: vec![[0.0, 200.0], [300.0, 200.0]],
            closed: false,
        },
        DrawnShape::Circle {
            center_mm: [50.0, 50.0],
            radius_mm: 20.0,
        },
        DrawnShape::Text {
            anchor_mm: [10.0, 300.0],
            height_mm: 25.0,
            rotation_deg: 0.0,
            text: "Kuchyňa".into(),
        },
    ];
    let snapshot = document.current();
    assert_eq!(
        plan_underlay_drawing(&snapshot, id, "roof", shapes.clone()).unwrap_err(),
        UnderlayObjectError::UnknownLayer("roof".into())
    );
    let flat = DrawnShape::Circle {
        center_mm: [0.0, 0.0],
        radius_mm: 0.0,
    };
    assert_eq!(
        plan_underlay_drawing(&snapshot, id, "walls", vec![shapes[0].clone(), flat]).unwrap_err(),
        UnderlayObjectError::InvalidShape(1)
    );
    let steps = document.visible_undo_steps();
    document
        .apply_batch(&plan_underlay_drawing(&snapshot, id, "walls", shapes).unwrap())
        .unwrap();
    assert_eq!(document.visible_undo_steps(), steps + 1);

    let edited = drawn(&document, id);
    let index = ObjectIndex::new(&edited);
    let new = |offset: usize| u32::try_from(read + offset).unwrap();
    let kinds = (0..3)
        .map(|offset| index.object(new(offset)).unwrap().kind.name())
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["LINE", "CIRCLE", "TEXT"]);
    assert_eq!(
        index.own_polylines(new(0)),
        vec![&[[0.0, 200.0], [300.0, 200.0]][..]]
    );
    let [low, high] = index.bounds(new(1)).unwrap();
    for (got, want) in low.iter().chain(&high).zip([30.0, 30.0, 70.0, 70.0]) {
        assert!((got - want).abs() < 1e-6, "{low:?} {high:?}");
    }
    assert_eq!(text_at(&edited, "Kuchyňa"), [[10.0, 300.0]]);
    assert_eq!(index.own_texts(new(2))[0].text, "Kuchyňa");
    assert_eq!(
        edited.segment_count(),
        lines().segment_count() + 1 + {
            let circle = index.own_polylines(new(1));
            circle[0].len() - 1
        }
    );

    // A drawn object is removed like a read one, and the ids after it stay.
    let snapshot = document.current();
    document
        .apply_batch(&plan_underlay_object_removal(&snapshot, id, &edited, &[new(0)]).unwrap())
        .unwrap();
    let edited = drawn(&document, id);
    let index = ObjectIndex::new(&edited);
    assert!(index.object(new(0)).is_none());
    assert_eq!(index.object(new(2)).unwrap().kind.name(), "TEXT");
    document.undo().unwrap();
    document.undo().unwrap();
    assert!(document.current().underlay(id).unwrap().edits.is_empty());
}

#[test]
fn a_moved_object_keeps_its_id_and_moves_again_from_where_it_is() {
    let (mut document, id) = document();
    let lines = lines();
    let text = id_of(&lines, 0x30);
    let walls = id_of(&lines, 0x10);
    for _ in 0..2 {
        let snapshot = document.current();
        let batch = plan_underlay_object_transform(
            &snapshot,
            id,
            &drawn(&document, id),
            &[text],
            &moving([100.0, 0.0]),
            false,
        )
        .unwrap();
        document.apply_batch(&batch).unwrap();
    }
    let edited = drawn(&document, id);
    assert_eq!(text_at(&edited, "WC"), [[250.0, 50.0]]);
    let index = ObjectIndex::new(&edited);
    assert!(index.object(text).is_some());
    assert_eq!(index.present(), ObjectIndex::new(&lines).present());
    assert_eq!(
        index.own_polylines(walls),
        vec![&[[0.0, 0.0], [100.0, 0.0]][..]]
    );
    let shear = [[1.0, 0.5, 0.0], [0.0, 1.0, 0.0]];
    assert_eq!(
        plan_underlay_object_transform(&document.current(), id, &edited, &[text], &shear, false)
            .unwrap_err(),
        UnderlayObjectError::InvalidMap
    );
}

#[test]
fn a_copy_of_an_insert_places_the_block_again_and_a_turned_copy_turns_its_lines() {
    let (mut document, id) = document();
    let lines = lines();
    let door = id_of(&lines, 0x20);
    let walls = id_of(&lines, 0x10);
    let quarter = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0]];
    let snapshot = document.current();
    document
        .apply_batch(
            &plan_underlay_object_transform(&snapshot, id, &lines, &[door, walls], &quarter, true)
                .unwrap(),
        )
        .unwrap();
    let edited = drawn(&document, id);
    let index = ObjectIndex::new(&edited);
    assert_eq!(
        index.block_counts().get("leaf").map(|counts| counts[0]),
        Some(2)
    );
    // Copies come in the order of the ids: the walls' line, then the door.
    let copied_walls = u32::try_from(lines.objects().objects.len()).unwrap();
    assert_eq!(
        index.own_polylines(copied_walls),
        vec![&[[0.0, 0.0], [0.0, 100.0]][..]]
    );
    assert_eq!(text_at(&edited, "D-01").len(), 2);
    // The original stays where it was.
    assert_eq!(
        index.own_polylines(walls),
        vec![&[[0.0, 0.0], [100.0, 0.0]][..]]
    );

    // A copy of the copy goes from where the copy is.
    let snapshot = document.current();
    document
        .apply_batch(
            &plan_underlay_object_transform(
                &snapshot,
                id,
                &edited,
                &[copied_walls],
                &moving([0.0, 10.0]),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    let edited = drawn(&document, id);
    let last = u32::try_from(edited.objects().objects.len() - 1).unwrap();
    assert_eq!(
        ObjectIndex::new(&edited).own_polylines(last),
        vec![&[[0.0, 10.0], [0.0, 110.0]][..]]
    );
}

#[test]
fn moving_a_drawn_shape_rewrites_it() {
    let (mut document, id) = document();
    let read = u32::try_from(lines().objects().objects.len()).unwrap();
    let snapshot = document.current();
    let arc = DrawnShape::Arc {
        center_mm: [0.0, 0.0],
        radius_mm: 10.0,
        start_deg: 0.0,
        end_deg: 90.0,
    };
    document
        .apply_batch(&plan_underlay_drawing(&snapshot, id, "notes", vec![arc]).unwrap())
        .unwrap();
    let mirror = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let snapshot = document.current();
    document
        .apply_batch(
            &plan_underlay_object_transform(
                &snapshot,
                id,
                &drawn(&document, id),
                &[read],
                &mirror,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    let edits = document.current().underlay(id).unwrap().edits.clone();
    assert_eq!(edits.added.len(), 1);
    let AddedObject::Drawn { shape, .. } = &edits.added[0] else {
        panic!("{edits:?}");
    };
    let DrawnShape::Arc {
        start_deg, end_deg, ..
    } = shape
    else {
        panic!("{shape:?}");
    };
    // Mirrored, the quarter from 0° to 90° runs from 90° to 180°.
    assert!((start_deg - 90.0).abs() < 1e-9 && (end_deg - 180.0).abs() < 1e-9);
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

    let door = id_of(&lines, 0x20);
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
    let edited = lines.clone().edited(&after.edits, &after.layers);
    assert_eq!(
        plan_underlay_object_removal(&document.current(), underlay_id, &edited, &[door])
            .unwrap_err(),
        UnderlayObjectError::NotFound(door)
    );
    document.undo().unwrap();
    assert_eq!(document.current().underlay(underlay_id), Some(&before));
}

/// A drawing whose one insert, handle 50 on `doors`, places a block of `lines` lines.
fn an_insert_of(lines: usize) -> UnderlayLines {
    let block = (0..lines)
        .map(|line| format!("0\nLINE\n8\n0\n10\n0\n20\n{line}\n11\n10\n21\n{line}\n"))
        .collect::<String>();
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n8\n0\n2\nmany\n70\n0\n10\n0\n20\n0\n{block}0\nENDBLK\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n0\nINSERT\n5\n50\n8\ndoors\n2\nmany\n10\n0\n20\n0\n0\nENDSEC\n0\nEOF\n"
    );
    read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None))
        .unwrap()
        .lines
}

fn copies(of: u32, count: usize) -> UnderlayEdits {
    UnderlayEdits {
        added: (0..count)
            .map(|copy| AddedObject::Copy {
                of,
                map: moving([20.0 * copy as f64, 0.0]),
            })
            .collect(),
        ..UnderlayEdits::default()
    }
}

/// Review 2026-10-29 MD-61: a copy of an insert places its block again, so a few kB of
/// edits, 100 copies of an insert of 50 000 lines, made 5.1 GB of lines each time they
/// were drawn: after every edit, at Undo and on opening the file. A drawing with its
/// copies is held to what a drawing read may hold, counted before any copy is made: a
/// copy past it is not planned, and lines whose edits ask for more are not drawn.
#[test]
fn copies_of_an_insert_are_held_to_what_a_drawing_may_hold() {
    use ketchup_model::underlay::UnderlayLinesError;
    use ketchup_tolerance::limits::DRAWING_ENTITIES;
    // What is counted is what the copies make.
    let small = lines();
    let edits = UnderlayEdits {
        added: vec![
            AddedObject::Copy {
                of: id_of(&small, 0x20),
                map: moving([0.0, 50.0]),
            },
            AddedObject::Copy {
                of: id_of(&small, 0x10),
                map: moving([0.0, 50.0]),
            },
        ],
        ..UnderlayEdits::default()
    };
    assert_eq!(
        small.edited_size(&edits).objects,
        small.clone().edited(&edits, &[]).objects().objects.len()
    );

    let per_insert = 20_000;
    let big = an_insert_of(per_insert);
    let insert = id_of(&big, 0x50);
    let fit = DRAWING_ENTITIES / (per_insert + 1) - 1;
    let (mut document, id) = document();
    let mut underlay = document.current().underlay(id).unwrap().clone();
    let blob = big.encode();
    // Within it the copies are drawn.
    underlay.edits = copies(insert, fit);
    let drawn = underlay.read_lines(&blob).unwrap();
    assert_eq!(drawn.objects().objects.len(), (fit + 1) * (per_insert + 1));
    // One more is not planned.
    document
        .apply_batch(&ketchup_model::document::CommandBatch::new(vec![
            ketchup_model::document::CanonicalCommand::UpsertUnderlay(underlay.clone()),
        ]))
        .unwrap();
    let refused = plan_underlay_object_transform(
        &document.current(),
        id,
        &drawn,
        &[insert],
        &moving([0.0, 5.0]),
        true,
    )
    .unwrap_err();
    assert!(
        matches!(refused, UnderlayObjectError::TooLarge(size) if size.objects > DRAWING_ENTITIES),
        "{refused:?}"
    );
    // Lines whose edits ask for a hundred are refused before they are made.
    underlay.edits = copies(insert, 100);
    let error = underlay.read_lines(&blob).unwrap_err();
    assert!(
        matches!(error, UnderlayLinesError::EditedTooLarge(size) if size.objects > 2_000_000),
        "{error:?}"
    );
}

/// The straight lines a drawing's object `id` draws now.
fn own_lines(document: &DocumentStore, id: UnderlayId, object: u32) -> Vec<Vec<[f64; 2]>> {
    let edited = drawn(document, id);
    ObjectIndex::new(&edited)
        .own_polylines(object)
        .into_iter()
        .map(<[[f64; 2]]>::to_vec)
        .collect()
}

/// 2D-2: a line of the drawing is trimmed back to the line that crosses it, extended to
/// the next line ahead of an end, or offset beside itself; one Undo step each, on its
/// layer, and an object that is not one straight line is refused by name.
#[test]
fn a_line_of_the_drawing_is_trimmed_extended_and_offset() {
    let (mut document, id) = document();
    let read = u32::try_from(lines().objects().objects.len()).unwrap();
    let top = id_of(&lines(), 0x12); // (100, 100) to (0, 100) on walls
    let line = |points: &[[f64; 2]]| DrawnShape::Polyline {
        points_mm: points.to_vec(),
        closed: false,
    };
    // A cross line at x = 50 and a short one inside the walls.
    let crossing = vec![
        line(&[[50.0, -50.0], [50.0, 150.0]]),
        line(&[[60.0, 50.0], [80.0, 50.0]]),
    ];
    let snapshot = document.current();
    document
        .apply_batch(&plan_underlay_drawing(&snapshot, id, "walls", crossing).unwrap())
        .unwrap();
    let short = read + 1;
    let steps = document.visible_undo_steps();
    let edit = |document: &mut DocumentStore, object: u32, edit: LineEdit| {
        let snapshot = document.current();
        let batch = plan_underlay_line_edit(&snapshot, id, &drawn(document, id), object, edit)?;
        document.apply_batch(&batch).unwrap();
        Ok::<_, UnderlayObjectError>(())
    };

    // Trimmed at (80, 100): the piece from the wall's end to the cross line goes.
    edit(
        &mut document,
        top,
        LineEdit::Trim {
            pick_mm: [80.0, 100.0],
        },
    )
    .unwrap();
    assert_eq!(document.visible_undo_steps(), steps + 1);
    assert!(own_lines(&document, id, top).is_empty());
    assert_eq!(
        own_lines(&document, id, read + 2),
        vec![vec![[50.0, 100.0], [0.0, 100.0]]]
    );
    let edited = drawn(&document, id);
    let index = ObjectIndex::new(&edited);
    let layer = edited.objects().layer_name(index.object(read + 2).unwrap());
    assert_eq!(layer, "walls");

    // The short line reaches on to the wall at x = 100, or back to the cross line.
    edit(
        &mut document,
        short,
        LineEdit::Extend {
            pick_mm: [79.0, 51.0],
        },
    )
    .unwrap();
    assert_eq!(
        own_lines(&document, id, read + 3),
        vec![vec![[60.0, 50.0], [100.0, 50.0]]]
    );
    edit(
        &mut document,
        read + 3,
        LineEdit::Extend {
            pick_mm: [61.0, 50.0],
        },
    )
    .unwrap();
    assert_eq!(
        own_lines(&document, id, read + 4),
        vec![vec![[50.0, 50.0], [100.0, 50.0]]]
    );

    // Offset 25 above the trimmed wall, and through a point below it; the wall stays.
    let offset = |distance_mm, side_mm| LineEdit::Offset {
        distance_mm,
        side_mm,
    };
    edit(&mut document, read + 2, offset(Some(25.0), [10.0, 300.0])).unwrap();
    assert_eq!(
        own_lines(&document, id, read + 5),
        vec![vec![[50.0, 125.0], [0.0, 125.0]]]
    );
    edit(&mut document, read + 2, offset(None, [20.0, 60.0])).unwrap();
    assert_eq!(
        own_lines(&document, id, read + 6),
        vec![vec![[50.0, 60.0], [0.0, 60.0]]]
    );
    assert_eq!(own_lines(&document, id, read + 2).len(), 1);
    assert_eq!(document.visible_undo_steps(), steps + 5);

    // Refused by name: a text, nothing ahead of an end, an offset onto the line.
    let text = id_of(&lines(), 0x30);
    let refused = |document: &mut DocumentStore, object, line_edit| {
        edit(document, object, line_edit).unwrap_err()
    };
    assert_eq!(
        refused(
            &mut document,
            text,
            LineEdit::Trim {
                pick_mm: [50.0, 50.0]
            }
        ),
        UnderlayObjectError::NotALine(text)
    );
    assert_eq!(
        refused(
            &mut document,
            read + 5,
            LineEdit::Extend {
                pick_mm: [1.0, 125.0]
            }
        ),
        UnderlayObjectError::NoCrossing(read + 5)
    );
    assert_eq!(
        refused(&mut document, read + 5, offset(None, [20.0, 125.0])),
        UnderlayObjectError::InvalidDistance
    );
    assert_eq!(document.visible_undo_steps(), steps + 5);

    for _ in 0..5 {
        document.undo().unwrap();
    }
    assert_eq!(
        own_lines(&document, id, top),
        vec![vec![[100.0, 100.0], [0.0, 100.0]]]
    );
}
