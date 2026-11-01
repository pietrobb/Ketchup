//! A drawing of one mirrored part made from the window shows the hand the part
//! is placed in, as the Assistant's drawing does (review 2026-10-31 MF-81).

use super::*;
use crate::drawn_shape::tests::evaluate_exact;
use ketchup_model::drawing::DrawingSource;

const PART: OccurrenceId = OccurrenceId(1);

fn drawing_source_of_selected_part(mirror: bool) -> DrawingSource {
    let mut app = KetchupApp::new();
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
    app.document
        .current()
        .drawing_sheets()
        .next()
        .unwrap()
        .source()
        .clone()
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
