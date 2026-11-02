//! A drawing of one mirrored part made from the window shows the hand the part
//! is placed in, as the Assistant's drawing does (review 2026-10-31 MF-81).

use super::*;
use crate::app::drawing_sheet_export::SheetFileFormat;
use crate::drawn_shape::tests::evaluate_exact;
use ketchup_model::drawing::DrawingSource;
use std::collections::BTreeSet;

const PART: OccurrenceId = OccurrenceId(1);

fn drawing_source_of_selected_part(mirror: bool) -> DrawingSource {
    drawing_of_selected_part(mirror, None).0
}

/// The drawing of the default part, placed mirrored when `mirror`, with the
/// corner `notch` (x from, x to) cut through it: its source and its sheet's
/// lines as DXF writes them.
fn drawing_of_selected_part(
    mirror: bool,
    notch: Option<[f64; 2]>,
) -> (DrawingSource, BTreeSet<(String, [i64; 4])>) {
    let mut app = KetchupApp::new();
    if let Some([from, to]) = notch {
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(1),
                    name: "notch".to_owned(),
                    kind: FeatureKind::polygon(&[[from, 0.0], [to, 0.0], [to, 20.0], [from, 20.0]]),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(1),
                    name: "notch cut".to_owned(),
                    kind: FeatureKind::through_cut(FeatureId(2), FeatureId(3)),
                },
            ]))
            .unwrap();
    }
    if mirror {
        let mut matrix = *app
            .document
            .current()
            .occurrence(PART)
            .unwrap()
            .transform()
            .matrix();
        matrix[0] = -matrix[0];
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: PART,
                    transform: Transform::from_matrix(matrix).unwrap(),
                },
            ]))
            .unwrap();
    }
    evaluate_exact(&mut app);
    app.selection.select_occurrence(PART, false);
    assert!(app.preview_selection_drawing(), "{}", app.action_digest());
    assert!(app.confirm_assembly_preview(), "{}", app.action_digest());
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheets().next().unwrap();
    let (_, dxf) = app
        .drawing_sheet_file(sheet.id(), SheetFileFormat::Dxf)
        .unwrap();
    (sheet.source().clone(), dxf_lines(&dxf))
}

/// The edges a DXF sheet draws of its part: each LINE of the VISIBLE and
/// HIDDEN layers as its layer and its two ends in hundredths of a mm, the
/// lower end first, once each.
fn dxf_lines(dxf: &[u8]) -> BTreeSet<(String, [i64; 4])> {
    let text = String::from_utf8_lossy(dxf);
    let pairs = text.lines().map(str::trim).collect::<Vec<_>>();
    let mut lines = BTreeSet::new();
    let mut current: Option<(String, [i64; 4])> = None;
    let mut keep = |line: Option<(String, [i64; 4])>| {
        if let Some((layer, [x1, y1, x2, y2])) = line
            && (layer == "VISIBLE" || layer == "HIDDEN")
        {
            let [a, b] = if [x1, y1] <= [x2, y2] {
                [[x1, y1], [x2, y2]]
            } else {
                [[x2, y2], [x1, y1]]
            };
            lines.insert((layer, [a[0], a[1], b[0], b[1]]));
        }
    };
    for pair in pairs.chunks(2) {
        let [code, value] = pair else { break };
        if *code == "0" {
            keep(current.take());
            current = (*value == "LINE").then(|| (String::new(), [0; 4]));
            continue;
        }
        let Some((layer, line)) = current.as_mut() else {
            continue;
        };
        if *code == "8" {
            *layer = (*value).to_owned();
        } else if let Some(slot) = ["10", "20", "11", "21"]
            .iter()
            .position(|known| known == code)
        {
            line[slot] = (value.parse::<f64>().unwrap() * 100.0).round() as i64;
        }
    }
    keep(current);
    lines
}

#[test]
fn a_drawing_of_one_mirrored_part_keeps_its_hand() {
    assert!(matches!(
        drawing_source_of_selected_part(false),
        DrawingSource::Definition(_)
    ));
    assert_eq!(
        drawing_source_of_selected_part(true),
        DrawingSource::RigidAssemblyInstances {
            instance_paths: vec![InstancePath::root(PART)],
        },
        "the definition is the opposite hand of a mirrored copy"
    );
}

/// Review 2026-11-01 MF-81: the hand is checked in what the sheet draws, not
/// only in its source. A part notched at its right end, placed mirrored, is
/// drawn notched at its left end, line for line as the part notched there.
#[test]
fn a_mirrored_part_is_drawn_in_its_hand() {
    let (_, mirrored) = drawing_of_selected_part(true, Some([80.0, 100.0]));
    let (_, opposite) = drawing_of_selected_part(false, Some([0.0, 20.0]));
    let (_, original) = drawing_of_selected_part(false, Some([80.0, 100.0]));
    assert!(!mirrored.is_empty());
    assert_ne!(original, opposite, "the two hands draw alike");
    assert_eq!(mirrored, opposite);
}
