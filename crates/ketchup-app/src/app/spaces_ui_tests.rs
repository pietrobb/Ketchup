use super::spaces_ui::SpaceRow;
use crate::*;
use egui_kittest::kittest::{Queryable, by};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantClearanceOwner, AssistantClearanceSeverity,
};
use ketchup_model::space::{ClearanceVolumeId, SpaceId};

fn apply(app: &mut KetchupApp, operations: Vec<AssistantCadEditOperation>) {
    let proposal = app
        .derive_assistant_cad_edit_proposal(&AssistantCadEditProgram { operations })
        .unwrap();
    app.commit_verified_proposal_with_work_recovery(&proposal)
        .unwrap();
}

/// A board, a bathroom and the required free volume in front of its door.
fn harness() -> egui_kittest::Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new();
    let source = ketchup_model::document::RuleProgramSource {
        file_name: "board.star".into(),
        source: "box(\"doska\", (600, 18, 400))\n".into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(source, true).unwrap();
    apply(
        &mut app,
        vec![
            AssistantCadEditOperation::UpsertSpace {
                space_id: 1,
                purpose: "kúpeľňa".into(),
                min_mm: [0.0, 0.0, 0.0],
                max_mm: [2000.0, 1500.0, 2500.0],
                adjacent_to: Vec::new(),
                accessible_to: Vec::new(),
            },
            AssistantCadEditOperation::UpsertClearanceVolume {
                clearance_volume_id: 2,
                reason: "pred dverami".into(),
                owner: AssistantClearanceOwner::Space { space_id: 1 },
                min_mm: [0.0, 0.0, 0.0],
                max_mm: [800.0, 600.0, 2000.0],
                severity: AssistantClearanceSeverity::Required,
                tolerance_mm: None,
            },
        ],
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness
}

fn click_row(harness: &mut egui_kittest::Harness<'static, KetchupApp>, text: &str) {
    harness.get(by().label_contains(text)).click();
    harness.run_steps(2);
}

fn picked_box(
    harness: &egui_kittest::Harness<'static, KetchupApp>,
) -> Option<(SpaceRow, [f64; 3], [f64; 3])> {
    harness
        .state()
        .picked_space_row()
        .map(|(row, volume)| (row, volume.min(), volume.max()))
}

/// The dock lists the space and the clearance volume by name; picking one
/// outlines its box. Delete removes the clearance volume with the typed
/// operation MCP uses, as one Undo step; a space a clearance volume still
/// belongs to is refused and nothing changes.
#[test]
fn dock_lists_outlines_and_deletes_spaces_and_clearance_volumes() {
    let mut harness = harness();
    let revision = harness.state().document.current().revision_id();
    let digest = harness.state().document.current().canonical_digest();

    click_row(&mut harness, "kúpeľňa");
    assert_eq!(
        picked_box(&harness),
        Some((
            SpaceRow::Space(SpaceId(1)),
            [0.0; 3],
            [2000.0, 1500.0, 2500.0]
        ))
    );
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    assert_eq!(harness.state().document.current().revision_id(), revision);
    assert_eq!(harness.state().document.current().spaces().count(), 1);
    let refused = harness.state().catalog.format(
        "spaces-delete-rejected",
        &BTreeMap::from([("reason", String::new())]),
    );
    assert!(
        harness.state().digest.starts_with(refused.trim_end()),
        "{}",
        harness.state().digest
    );

    click_row(&mut harness, "pred dverami");
    assert_eq!(
        picked_box(&harness),
        Some((
            SpaceRow::Clearance(ClearanceVolumeId(2)),
            [0.0; 3],
            [800.0, 600.0, 2000.0]
        ))
    );
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    let snapshot = harness.state().document.current();
    assert_eq!(snapshot.clearance_volumes().count(), 0);
    assert_eq!(snapshot.spaces().count(), 1);
    assert!(picked_box(&harness).is_none());
    assert!(harness.query(by().label_contains("pred dverami")).is_none());

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    let snapshot = harness.state().document.current();
    assert_eq!(snapshot.clearance_volumes().count(), 1);
    assert_eq!(snapshot.canonical_digest(), digest);
    harness.get(by().label_contains("pred dverami"));
}

/// A part selected after the row takes the outline and Delete back.
#[test]
fn a_selected_part_takes_over_from_the_picked_space() {
    let mut harness = harness();
    click_row(&mut harness, "pred dverami");
    assert!(picked_box(&harness).is_some());
    harness.state_mut().select_all();
    harness.run_steps(2);
    assert!(picked_box(&harness).is_none());
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    let snapshot = harness.state().document.current();
    assert_eq!(snapshot.clearance_volumes().count(), 1);
    assert_eq!(snapshot.occurrences().count(), 0);
}
