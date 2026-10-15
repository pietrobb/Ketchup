use super::spaces_ui::SpaceRow;
use crate::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{Queryable, by};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantClearanceOwner, AssistantClearanceSeverity,
};
use ketchup_model::space::{ClearanceOwner, ClearanceSeverity, ClearanceVolumeId, SpaceId};

type Shell = egui_kittest::Harness<'static, KetchupApp>;

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
        variants: Default::default(),
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
    // The board's exact solid is there before the first frame: arriving later
    // it would add the mass rows above the buttons these tests click.
    crate::drawn_shape::tests::evaluate_exact(&mut app);
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
        "spaces-rejected",
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

fn click_button(harness: &mut Shell, key: &str) {
    let label = harness.state().catalog.text(key);
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(2);
}

fn retype(harness: &mut Shell, key: &str, axis: Option<&str>, text: &str) {
    let label = match axis {
        Some(axis) => harness
            .state()
            .catalog
            .format(key, &BTreeMap::from([("axis", axis.to_owned())])),
        None => harness.state().catalog.text(key),
    };
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .type_text(text);
    harness.run_steps(2);
}

fn undo(harness: &mut Shell) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
}

/// "Space from selection" puts a space around the selected board and
/// "Clearance in front of part" a required volume owned by it, 600 mm deep
/// toward -Y; each is one Undo step back to the same document.
#[test]
fn the_selected_part_gives_a_space_and_a_clearance_in_front() {
    let mut harness = harness();
    let digest = harness.state().document.current().canonical_digest();
    harness.state_mut().select_all();
    harness.run_steps(2);
    let measures = harness.state().selection_measures().unwrap();
    let min = measures.world_min_mm;
    let max: [f64; 3] = std::array::from_fn(|axis| min[axis] + measures.world_size_mm[axis]);

    click_button(&mut harness, "spaces-create-from-selection");
    let snapshot = harness.state().document.current();
    let space = snapshot.spaces().find(|space| space.id() == SpaceId(2));
    let space = space.expect("the new space");
    assert_eq!((space.volume().min(), space.volume().max()), (min, max));
    assert_eq!(
        picked_box(&harness).map(|(row, ..)| row),
        Some(SpaceRow::Space(SpaceId(2)))
    );
    undo(&mut harness);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );

    harness.state_mut().select_all();
    harness.run_steps(2);
    let board = harness.state().selected_instance_paths().first().cloned();
    let board = board.expect("the board is selected");
    click_button(&mut harness, "spaces-create-clearance-in-front");
    let snapshot = harness.state().document.current();
    let volume = snapshot
        .clearance_volumes()
        .find(|volume| volume.id() == ClearanceVolumeId(3));
    let volume = volume.expect("the new clearance volume");
    assert_eq!(volume.owner(), &ClearanceOwner::Occurrence(board));
    assert_eq!(volume.severity(), ClearanceSeverity::Required);
    assert_eq!(
        (volume.volume().min(), volume.volume().max()),
        ([min[0], min[1] - 600.0, min[2]], [max[0], min[1], max[2]])
    );
    undo(&mut harness);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
}

fn clearance_findings(harness: &mut Shell) -> Vec<ValidatorPanelFinding> {
    let app = harness.state_mut();
    app.validator_panel.selection = BTreeSet::from(["clearance_occupancy"]);
    app.run_validator_panel();
    harness.run_steps(2);
    let report = harness.state().validator_panel_report();
    report.expect("a report on this revision").findings.clone()
}

/// The board stands in the clearance in front of the bathroom door. Its
/// finding in the validation panel selects the board and outlines the
/// volume; moving the volume away by number clears the finding in one Undo
/// step, and showing the finding changes nothing.
#[test]
fn a_clearance_finding_shows_the_part_and_the_volume_in_its_way() {
    let mut harness = harness();
    let digest = harness.state().document.current().canonical_digest();
    let snapshot = harness.state().document.current();
    let board = snapshot.occurrences().next().expect("the board").id();

    let findings = clearance_findings(&mut harness);
    let [finding] = findings.as_slice() else {
        panic!("one occupant expected: {findings:#?}");
    };
    assert_eq!(
        (finding.occurrences.as_slice(), finding.clearance_volume),
        ([board].as_slice(), Some(ClearanceVolumeId(2)))
    );
    let title = harness.state().catalog.text("validators-title");
    harness.get_by_role_and_label(Role::Button, &title).click();
    harness.run_steps(2);
    click_row(&mut harness, "clearance.occupied");
    assert_eq!(
        harness.state().selected_instance_paths(),
        BTreeSet::from([InstancePath::root(board)])
    );
    assert_eq!(
        picked_box(&harness),
        Some((
            SpaceRow::Clearance(ClearanceVolumeId(2)),
            [0.0; 3],
            [800.0, 600.0, 2000.0]
        ))
    );
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );

    // The validation panel stays open: the dock scrolls the fields into view.
    retype(&mut harness, "spaces-field-position", Some("Y"), "1000");
    click_button(&mut harness, "spaces-apply");
    assert_ne!(
        harness.state().document.current().canonical_digest(),
        digest
    );
    assert!(harness.state().validator_panel_report().is_none());
    assert_eq!(clearance_findings(&mut harness), Vec::new());

    undo(&mut harness);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
    assert_eq!(clearance_findings(&mut harness).len(), 1);
}

/// With the board and its clearance volume both picked from the finding,
/// Delete removes the board and keeps the volume; with no part selected the
/// next Delete removes the volume. Two Undo steps restore the document.
#[test]
fn delete_takes_the_selected_part_before_the_picked_volume() {
    let mut harness = harness();
    let digest = harness.state().document.current().canonical_digest();
    clearance_findings(&mut harness);
    let title = harness.state().catalog.text("validators-title");
    harness.get_by_role_and_label(Role::Button, &title).click();
    harness.run_steps(2);
    click_row(&mut harness, "clearance.occupied");
    harness.get_by_role_and_label(Role::Button, &title).click();
    harness.run_steps(2);
    assert_eq!(harness.state().selected_instance_paths().len(), 1);
    assert!(picked_box(&harness).is_some());

    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    let snapshot = harness.state().document.current();
    assert_eq!(snapshot.occurrences().count(), 0);
    assert_eq!(snapshot.clearance_volumes().count(), 1);

    click_row(&mut harness, "pred dverami");
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    let snapshot = harness.state().document.current();
    assert_eq!(snapshot.clearance_volumes().count(), 0);
    assert_eq!(snapshot.spaces().count(), 1);

    undo(&mut harness);
    undo(&mut harness);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
}

/// The picked clearance volume's reason, size and severity change by typed
/// numbers in one Undo step; a zero size is refused and changes nothing.
#[test]
fn the_picked_row_is_edited_by_number() {
    let mut harness = harness();
    let digest = harness.state().document.current().canonical_digest();
    click_row(&mut harness, "pred dverami");

    retype(&mut harness, "spaces-field-reason", None, "pred sprchou");
    retype(&mut harness, "spaces-field-size", Some("Y"), "900");
    let required = harness.state().catalog.text("spaces-field-required");
    harness
        .get_by_role_and_label(Role::CheckBox, &required)
        .click();
    harness.run_steps(2);
    let revision = harness.state().document.current().revision_id();
    click_button(&mut harness, "spaces-apply");
    let snapshot = harness.state().document.current();
    assert_ne!(snapshot.revision_id(), revision);
    let volume = snapshot.clearance_volumes().next().unwrap();
    assert_eq!(volume.reason(), "pred sprchou");
    assert_eq!(volume.severity(), ClearanceSeverity::Advisory);
    assert_eq!(
        (volume.volume().min(), volume.volume().max()),
        ([0.0; 3], [800.0, 900.0, 2000.0])
    );
    assert_eq!(volume.owner(), &ClearanceOwner::Space(SpaceId(1)));
    let edited = snapshot.canonical_digest();

    retype(&mut harness, "spaces-field-size", Some("X"), "0");
    click_button(&mut harness, "spaces-apply");
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        edited
    );
    let refused = harness.state().catalog.format(
        "spaces-rejected",
        &BTreeMap::from([("reason", String::new())]),
    );
    assert!(
        harness.state().digest.starts_with(refused.trim_end()),
        "{}",
        harness.state().digest
    );

    undo(&mut harness);
    assert_eq!(
        harness.state().document.current().canonical_digest(),
        digest
    );
}
