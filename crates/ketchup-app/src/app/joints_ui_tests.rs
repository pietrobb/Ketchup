use crate::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{Queryable, by};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadPartFeature, AssistantInstancePath,
    AssistantPrincipalPlane, AssistantSketchConstraint, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use ketchup_geometry::prismatic::JointId;
use ketchup_model::document::JointParticipant;

type Shell = egui_kittest::Harness<'static, KetchupApp>;

fn apply(app: &mut KetchupApp, operations: Vec<AssistantCadEditOperation>) {
    let proposal = app
        .derive_assistant_cad_edit_proposal(&AssistantCadEditProgram { operations })
        .unwrap();
    app.commit_verified_proposal_with_work_recovery(&proposal)
        .unwrap();
}

/// A round part 40 mm across and `height_mm` tall standing at `x_mm`.
fn post(name: &str, x_mm: f64, height_mm: f64) -> AssistantCadEditOperation {
    AssistantCadEditOperation::CreatePart {
        holes: Vec::new(),
        pockets: Vec::new(),
        name: name.into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 20.0,
        }],
        constraints: vec![AssistantSketchConstraint::Radius {
            id: 1,
            entity_id: 1,
            value_mm: 20.0,
        }],
        feature: AssistantCadPartFeature::Extrusion {
            distance_mm: height_mm,
        },
        translation_mm: [x_mm, 0.0, 0.0],
        rotation: None,
    }
}

/// A 300 mm post with a 200 mm one overlapping it by 10 mm along X, and a
/// board standing apart, beside the new document's box.
fn harness() -> Shell {
    let mut app = KetchupApp::new();
    apply(
        &mut app,
        vec![
            post("stojka", 0.0, 300.0),
            post("priečka", 30.0, 200.0),
            post("doska", 400.0, 100.0),
        ],
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness
}

fn part(harness: &Shell, name: &str) -> OccurrenceId {
    let snapshot = harness.state().document.current();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name);
    occurrence.expect("the part").id()
}

fn select(harness: &mut Shell, parts: &[OccurrenceId]) {
    for (index, part) in parts.iter().enumerate() {
        harness
            .state_mut()
            .select_from_outliner(InstancePath::root(*part), index > 0);
    }
    harness.run_steps(2);
}

fn click_button(harness: &mut Shell, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    harness.run_steps(2);
}

fn retype(harness: &mut Shell, label: &str, text: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    harness.run_steps(2);
}

fn size_field(harness: &Shell, axis: &str) -> String {
    harness.state().catalog.format(
        "spaces-field-size",
        &BTreeMap::from([("axis", axis.to_owned())]),
    )
}

fn press(harness: &mut Shell, key: egui::Key) {
    harness.key_press(key);
    harness.run_steps(2);
}

fn undo(harness: &mut Shell) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
}

fn digest(harness: &Shell) -> String {
    harness.state().document.current().canonical_digest()
}

fn refused(harness: &Shell) -> bool {
    let refused = harness.state().catalog.format(
        "spaces-rejected",
        &BTreeMap::from([("reason", String::new())]),
    );
    harness.state().digest.starts_with(refused.trim_end())
}

/// The one joint as the document has it: the parts it joins and its volume.
fn joint(harness: &Shell) -> Option<(BTreeSet<OccurrenceId>, [f64; 3], [f64; 3])> {
    let snapshot = harness.state().document.current();
    let mut joints = snapshot.joints();
    let joint = joints.next()?;
    assert!(joints.next().is_none());
    let parts = [joint.participant_a(), joint.participant_b()]
        .into_iter()
        .map(|participant| participant.part().expect("a part").root_occurrence())
        .collect();
    Some((parts, joint.volume().min(), joint.volume().max()))
}

fn close(actual: [f64; 3], expected: [f64; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| (actual - expected).abs() < 0.5)
}

/// Two selected overlapping posts are joined over their overlap; the joint
/// is picked with both posts outlined. Its volume changes by number, a zero
/// size is refused without a change, and Delete removes it. Each change is
/// one Undo step back to the same document.
#[test]
fn a_joint_of_two_selected_parts_is_made_edited_and_deleted_in_undo_steps() {
    let mut harness = harness();
    let start = digest(&harness);
    let (stojka, priecka) = (part(&harness, "stojka"), part(&harness, "priečka"));
    select(&mut harness, &[stojka, priecka]);

    let create = harness.state().catalog.text("joints-create");
    click_button(&mut harness, &create);
    let (parts, min, max) = joint(&harness).expect("the new joint");
    assert_eq!(parts, BTreeSet::from([stojka, priecka]));
    assert!(
        close(min, [10.0, -20.0, 0.0]) && close(max, [20.0, 20.0, 200.0]),
        "{min:?} {max:?}"
    );
    let id = harness.state().picked_joint_id().expect("picked");
    assert_eq!(
        harness.state().joint_parts(id),
        BTreeSet::from([InstancePath::root(stojka), InstancePath::root(priecka)])
    );
    let created = digest(&harness);

    let height = size_field(&harness, "Z");
    retype(&mut harness, &height, "150");
    let apply_label = harness.state().catalog.text("spaces-apply");
    click_button(&mut harness, &apply_label);
    let (_, edited_min, edited_max) = joint(&harness).unwrap();
    assert!(
        close(edited_min, min) && close(edited_max, [max[0], max[1], edited_min[2] + 150.0]),
        "{edited_min:?} {edited_max:?}"
    );
    assert!((edited_max[2] - edited_min[2] - 150.0).abs() < 1e-9);
    let edited = digest(&harness);

    retype(&mut harness, &height, "0");
    click_button(&mut harness, &apply_label);
    assert_eq!(digest(&harness), edited);
    assert!(refused(&harness), "{}", harness.state().digest);

    // A click on the row takes the keys back from the field.
    harness.get(by().label_contains("stojka ↔ priečka")).click();
    harness.run_steps(2);
    press(&mut harness, egui::Key::Delete);
    assert_eq!(joint(&harness), None);
    assert_eq!(harness.state().document.current().occurrences().count(), 4);

    undo(&mut harness);
    assert_eq!(digest(&harness), edited);
    undo(&mut harness);
    assert_eq!(digest(&harness), created);
    undo(&mut harness);
    assert_eq!(digest(&harness), start);
}

/// A joint MCP declared is listed with the parts it joins; picking it
/// outlines both and changes nothing. Parts that do not overlap are refused
/// a joint without a change. A part selected afterwards takes Delete first,
/// and the pick does not come back for the next Delete; deleting a joined
/// part drops its joint, and Undo brings both back.
#[test]
fn picking_a_joint_outlines_both_parts_and_delete_takes_parts_first() {
    let mut harness = harness();
    let (stojka, priecka, doska) = (
        part(&harness, "stojka"),
        part(&harness, "priečka"),
        part(&harness, "doska"),
    );
    let path = |id: OccurrenceId| AssistantInstancePath {
        root_occurrence_id: id.0,
        steps: Vec::new(),
    };
    apply(
        harness.state_mut(),
        vec![AssistantCadEditOperation::UpsertJoint {
            joint_id: 4,
            part_a: path(stojka),
            part_b: path(priecka),
            min_mm: [10.0, -20.0, 0.0],
            max_mm: [20.0, 20.0, 200.0],
        }],
    );
    harness.run_steps(2);
    let declared = digest(&harness);

    harness.get(by().label_contains("stojka ↔ priečka")).click();
    harness.run_steps(2);
    assert_eq!(harness.state().picked_joint_id(), Some(JointId(4)));
    assert_eq!(
        harness.state().joint_parts(JointId(4)),
        BTreeSet::from([InstancePath::root(stojka), InstancePath::root(priecka)])
    );
    assert_eq!(digest(&harness), declared);

    select(&mut harness, &[stojka, doska]);
    let create = harness.state().catalog.text("joints-create");
    click_button(&mut harness, &create);
    assert_eq!(digest(&harness), declared);
    assert!(refused(&harness), "{}", harness.state().digest);

    harness.get(by().label_contains("stojka ↔ priečka")).click();
    harness.run_steps(2);
    select(&mut harness, &[doska]);
    assert_eq!(harness.state().picked_joint_id(), None);
    press(&mut harness, egui::Key::Delete);
    let snapshot = harness.state().document.current();
    assert!(
        snapshot
            .occurrences()
            .all(|occurrence| occurrence.id() != doska)
    );
    assert_eq!(snapshot.joints().count(), 1);
    // The pick the deleted board took over does not come back for the next
    // Delete to remove unseen.
    assert_eq!(harness.state().picked_joint_id(), None);
    press(&mut harness, egui::Key::Delete);
    assert_eq!(digest(&harness), snapshot.canonical_digest());
    assert_eq!(harness.state().document.current().joints().count(), 1);

    select(&mut harness, &[priecka]);
    press(&mut harness, egui::Key::Delete);
    let pruned = harness.state().document.current();
    assert_eq!(pruned.joints().count(), 0);
    undo(&mut harness);
    assert_eq!(digest(&harness), snapshot.canonical_digest());
    let restored = harness.state().document.current();
    let restored = restored.joint(JointId(4)).expect("the joint is back");
    assert_eq!(
        restored.participant_b(),
        &JointParticipant::Part(InstancePath::root(priecka))
    );
}
