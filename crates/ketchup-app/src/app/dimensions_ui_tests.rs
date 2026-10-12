use crate::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{Queryable, by};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadPartFeature, AssistantPrincipalPlane,
    AssistantSketchConstraint, AssistantSketchEntity, AssistantWorkplaneSpec,
};
use ketchup_model::document::{
    FeatureParameterTarget, ParameterValueType, PersistentDimensionTarget,
};

type Shell = egui_kittest::Harness<'static, KetchupApp>;

fn apply(app: &mut KetchupApp, operations: Vec<AssistantCadEditOperation>) {
    let proposal = app
        .derive_assistant_cad_edit_proposal(&AssistantCadEditProgram { operations })
        .unwrap();
    app.commit_verified_proposal_with_work_recovery(&proposal)
        .unwrap();
}

/// A round part `height_mm` tall standing at `x_mm`.
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

/// Two posts, 300 mm and 500 mm tall, beside the new document's box.
fn harness() -> Shell {
    let mut app = KetchupApp::new();
    apply(
        &mut app,
        vec![post("noha", 0.0, 300.0), post("stĺpik", 400.0, 500.0)],
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

/// The extrusion height of the part.
fn height_of(harness: &Shell, part: OccurrenceId) -> FeatureParameterTarget {
    let snapshot = harness.state().document.current();
    let occurrence = snapshot.occurrence(part).unwrap();
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    definition
        .feature_ids()
        .iter()
        .map(|id| {
            FeatureParameterTarget::new(*id, "extent.distance", ParameterValueType::Length).unwrap()
        })
        .find(|target| snapshot.feature_parameter_value(target).is_some())
        .expect("an extrusion")
}

fn select(harness: &mut Shell, part: OccurrenceId) {
    harness
        .state_mut()
        .select_from_outliner(InstancePath::root(part), false);
    harness.run_steps(2);
}

fn click_button(harness: &mut Shell, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    harness.run_steps(2);
}

fn retype(harness: &mut Shell, key: &str, text: &str) {
    let label = harness.state().catalog.text(key);
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

fn digest(harness: &Shell) -> String {
    harness.state().document.current().canonical_digest()
}

/// The one dimension as the document has it: name, target and shown text.
fn dimension(harness: &Shell) -> Option<(String, PersistentDimensionTarget, Option<String>)> {
    let snapshot = harness.state().document.current();
    let mut dimensions = snapshot.persistent_dimensions();
    let dimension = dimensions.next()?;
    assert!(dimensions.next().is_none());
    let text = snapshot
        .project_persistent_dimension(dimension.id)
        .and_then(|projection| projection.display_text);
    Some((dimension.name.clone(), dimension.target.clone(), text))
}

/// The selected post offers its height; the dimension made from it shows
/// 300 mm and outlines the post. Renamed and shown in cm with two decimals,
/// it reads 30.00 cm; eleven decimals are refused without a change; Delete
/// removes it. Each change is one Undo step back to the same document.
#[test]
fn a_dimension_of_the_selected_part_is_made_edited_and_deleted_in_undo_steps() {
    let mut harness = harness();
    let start = digest(&harness);
    let noha = part(&harness, "noha");
    let height = height_of(&harness, noha);
    select(&mut harness, noha);

    let create = harness.state().catalog.format(
        "measured-dimension-create",
        &BTreeMap::from([
            ("parameter", "extent.distance".to_owned()),
            ("value", "300".to_owned()),
        ]),
    );
    click_button(&mut harness, &create);
    let (name, target, text) = dimension(&harness).expect("the new dimension");
    assert_eq!(name, "noha extent.distance");
    assert_eq!(target, PersistentDimensionTarget::FeatureParameter(height));
    assert_eq!(text.as_deref(), Some("300.0 mm"));
    // What it measures: the post, as tall as the value.
    let id = harness.state().picked_dimension_id().expect("picked");
    let parts = harness.state().dimension_parts(id);
    assert_eq!(parts, BTreeSet::from([InstancePath::root(noha)]));
    let measures = harness.state().measures_of(&parts).unwrap();
    assert!(
        (measures.world_size_mm[2] - 300.0).abs() < 1e-6,
        "{measures:?}"
    );
    let created = digest(&harness);

    retype(&mut harness, "measured-dimension-field-name", "Výška nohy");
    harness
        .get_by_role_and_label(Role::RadioButton, "cm")
        .click();
    harness.run_steps(2);
    retype(&mut harness, "measured-dimension-field-decimals", "2");
    let apply = harness.state().catalog.text("measured-dimension-apply");
    click_button(&mut harness, &apply);
    let (name, _, text) = dimension(&harness).unwrap();
    assert_eq!(
        (name.as_str(), text.as_deref()),
        ("Výška nohy", Some("30.00 cm"))
    );
    let edited = digest(&harness);

    retype(&mut harness, "measured-dimension-field-decimals", "11");
    click_button(&mut harness, &apply);
    assert_eq!(digest(&harness), edited);
    let refused = harness.state().catalog.format(
        "spaces-rejected",
        &BTreeMap::from([("reason", String::new())]),
    );
    assert!(
        harness.state().digest.starts_with(refused.trim_end()),
        "{}",
        harness.state().digest
    );

    // A click on the row takes the keys back from the field.
    harness.get(by().label_contains("Výška nohy:")).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    assert_eq!(dimension(&harness), None);
    assert_eq!(harness.state().document.current().occurrences().count(), 3);

    undo(&mut harness);
    assert_eq!(digest(&harness), edited);
    undo(&mut harness);
    assert_eq!(digest(&harness), created);
    undo(&mut harness);
    assert_eq!(digest(&harness), start);
}

/// A dimension MCP declared on the taller post is listed with its value;
/// picking it outlines that post and changes nothing, and a part selected
/// afterwards takes the outline and Delete back.
#[test]
fn picking_a_dimension_outlines_the_part_it_measures() {
    let mut harness = harness();
    let stlpik = part(&harness, "stĺpik");
    let height = height_of(&harness, stlpik);
    apply(
        harness.state_mut(),
        vec![AssistantCadEditOperation::UpsertDimension {
            dimension_id: 7,
            name: "Výška stĺpika".into(),
            feature_id: height.feature_id.0,
            parameter_path: "extent.distance".into(),
            unit: None,
            decimal_places: Some(0),
        }],
    );
    harness.run_steps(2);
    let before = digest(&harness);

    harness.get(by().label_contains("Výška stĺpika:")).click();
    harness.run_steps(2);
    let id = harness.state().picked_dimension_id();
    assert_eq!(id, Some(PersistentDimensionId(7)));
    assert_eq!(
        harness.state().dimension_parts(PersistentDimensionId(7)),
        BTreeSet::from([InstancePath::root(stlpik)])
    );
    harness.get(by().label_contains("500 mm"));
    assert_eq!(digest(&harness), before);

    let noha = part(&harness, "noha");
    select(&mut harness, noha);
    assert_eq!(harness.state().picked_dimension_id(), None);
    let parts = harness.state().document.current().occurrences().count();
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    let snapshot = harness.state().document.current();
    assert_eq!(snapshot.occurrences().count(), parts - 1);
    assert!(
        snapshot
            .occurrences()
            .all(|occurrence| occurrence.id() != noha)
    );
    // The pick the deleted post took over does not come back for the next
    // Delete to remove unseen.
    assert_eq!(harness.state().picked_dimension_id(), None);
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    assert_eq!(digest(&harness), snapshot.canonical_digest());
    assert_eq!(snapshot.persistent_dimensions().count(), 1);
}
