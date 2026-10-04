use ketchup_application::{
    AssistantValidationSelection, DocumentSession, SaveOptions, SessionSettings,
};
use ketchup_model::document::{
    CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    RuleProgramSource, Snapshot,
};
use ketchup_model::validation::MATERIAL_DIMENSION_V1;
use std::collections::BTreeMap;

fn apply(session: &mut DocumentSession, source: &str) {
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "metadata.star".into(),
                source: source.into(),
                overrides: BTreeMap::new(),
            },
            false,
        )
        .unwrap();
}

fn classified(snapshot: &Snapshot, dimension: &str) -> BTreeMap<String, String> {
    let Some(dimension) = snapshot.classification_dimension_named(dimension).unwrap() else {
        return BTreeMap::new();
    };
    snapshot
        .occurrence_category_names(dimension)
        .map(|entry| {
            let (id, _, name) = entry.unwrap();
            (
                snapshot.occurrence(id).unwrap().name().to_owned(),
                name.to_owned(),
            )
        })
        .collect()
}

const SOURCE: &str = "box('shelf',(1000,300,20),material='steel',attributes={'classification:ketchup.validator-role.v1':'physics.beam.xy','classification:finish':'painted'})";

#[test]
fn program_material_and_role_drive_real_validation_without_rebuilding() {
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let initial = session.snapshot();
    let features = initial.features().cloned().collect::<Vec<_>>();
    let ids = initial
        .occurrences()
        .map(|part| (part.id(), part.definition_id()))
        .collect::<Vec<_>>();
    let selection = AssistantValidationSelection::only(&["beam_deflection"]);
    for (material, state) in [
        ("steel", "passed"),
        ("engineered_wood", "failed"),
        ("unknown", "not_evaluated"),
    ] {
        apply(
            &mut session,
            &SOURCE.replace("'steel'", &format!("'{material}'")),
        );
        let before = session.snapshot();
        let report = session.validators(&selection);
        assert_eq!(report["beam_deflection"]["state"], state, "{report:#}");
        assert_eq!(
            classified(&before, MATERIAL_DIMENSION_V1)["shelf"],
            material
        );
        assert_eq!(classified(&before, "finish")["shelf"], "painted");
        assert_eq!(before.features().cloned().collect::<Vec<_>>(), features);
        assert_eq!(
            before
                .occurrences()
                .map(|part| (part.id(), part.definition_id()))
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(
            before.canonical_digest(),
            session.snapshot().canonical_digest()
        );
        assert!(session.rule_program().is_some());
        if material != "unknown" {
            let measured = &report["beam_deflection"]["evaluations"][0];
            assert_eq!(measured["material"], material);
            assert_eq!(measured["material_source"], "classification");
            assert_eq!(measured["span_mm"], 1000.0);
            assert!(measured["predicted_deflection_mm"].as_f64().unwrap() > 0.0);
        }
    }
    session.undo().unwrap();
    assert_eq!(
        classified(&session.snapshot(), MATERIAL_DIMENSION_V1)["shelf"],
        "engineered_wood"
    );
    session.redo().unwrap();
    assert_eq!(
        classified(&session.snapshot(), MATERIAL_DIMENSION_V1)["shelf"],
        "unknown"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        classified(&reopened.snapshot(), MATERIAL_DIMENSION_V1)["shelf"],
        "unknown"
    );
    assert_eq!(
        reopened.validators(&selection)["beam_deflection"]["state"],
        "not_evaluated"
    );
    assert!(reopened.rule_program().is_some());
}

#[test]
fn metadata_removal_rebuild_and_addition_reconcile_final_occurrence_ids() {
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let original_id = session.snapshot().occurrences().next().unwrap().id();
    apply(
        &mut session,
        &format!(
            "{SOURCE}\nfillet('shelf',edges=[['x+','y+']],radius=2)\nbox('new',(10,10,10),at=(2000,0,0),material='aluminium')"
        ),
    );
    assert_eq!(
        session.snapshot().occurrence(original_id).unwrap().name(),
        "shelf"
    );
    assert_eq!(
        classified(&session.snapshot(), MATERIAL_DIMENSION_V1),
        BTreeMap::from([
            ("shelf".into(), "steel".into()),
            ("new".into(), "aluminium".into())
        ])
    );
    apply(
        &mut session,
        "box('shelf',(1000,300,20),attributes={'classification:finish':'oiled'})",
    );
    assert!(classified(&session.snapshot(), MATERIAL_DIMENSION_V1).is_empty());
    assert!(classified(&session.snapshot(), "ketchup.validator-role.v1").is_empty());
    assert_eq!(classified(&session.snapshot(), "finish")["shelf"], "oiled");
    assert_eq!(session.snapshot().occurrences().count(), 1);
    session.undo().unwrap();
    assert_eq!(
        classified(&session.snapshot(), MATERIAL_DIMENSION_V1).len(),
        2
    );
}

#[test]
fn program_metadata_preserves_unrelated_dimensions_and_existing_category_ids() {
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let snapshot = session.snapshot();
    let occurrence = snapshot.occurrences().next().unwrap().id();
    let finish = snapshot
        .classification_dimension_named("finish")
        .unwrap()
        .unwrap();
    let painted = snapshot
        .occurrence_classification(occurrence, finish.id())
        .unwrap();
    let unrelated = ClassificationDimensionId(100);
    session
        .apply_rule_commands_with_source(
            CommandBatch::new(vec![
                CanonicalCommand::UpsertClassificationDimension {
                    id: unrelated,
                    name: "review".into(),
                    categories: vec![(ClassificationCategoryId(1), "approved".into())],
                },
                CanonicalCommand::SetOccurrenceClassification {
                    occurrence_id: occurrence,
                    dimension_id: unrelated,
                    category_id: Some(ClassificationCategoryId(1)),
                },
            ]),
            session.rule_program().unwrap().clone(),
        )
        .unwrap();
    apply(&mut session, &SOURCE.replace("painted", "oiled"));
    let changed = session.snapshot();
    let updated = changed
        .classification_dimension_named("finish")
        .unwrap()
        .unwrap();
    assert_eq!(updated.id(), finish.id());
    assert_eq!(updated.category(painted).unwrap().name(), "painted");
    assert_eq!(classified(&changed, "finish")["shelf"], "oiled");
    assert_eq!(classified(&changed, "review")["shelf"], "approved");
    apply(&mut session, "box('shelf',(1000,300,20))");
    assert!(classified(&session.snapshot(), "finish").is_empty());
    assert_eq!(
        classified(&session.snapshot(), "review")["shelf"],
        "approved"
    );
}

#[test]
fn numeric_inputs_update_remove_and_survive_history_without_stale_loads() {
    let mut session = DocumentSession::default();
    let source = "box('load',(10,10,10),attributes={'classification:ketchup.validator-role.v1':'physics.static.load:test','classification:ketchup.static-load-mode.v1':'compression','input:physics.mass_kg.occurrence.{occurrence}':'100','input:physics.applied_load_n.occurrence.{occurrence}':'200','input:physics.gravity_x_m_s2':'0','input:physics.gravity_y_m_s2':'0','input:physics.gravity_z_m_s2':'-9.81'})\nbox('support',(10,10,10),at=(20,0,0),attributes={'classification:ketchup.validator-role.v1':'physics.static.support:test','classification:ketchup.support-capacity.v1':'{\"source\":\"synthetic test capacity, not design data\",\"units\":\"N\",\"mode\":\"compression\",\"direction_world\":[0,0,-1],\"assumptions\":\"test fixture only\",\"additive\":false}','input:physics.support_capacity_n.occurrence.{occurrence}':'2000'})";
    apply(&mut session, source);
    let initial = session.snapshot();
    assert_eq!(initial.evaluator_node_count(), 6);
    let selection = AssistantValidationSelection::only(&["static_load"]);
    let report = session.validators(&selection);
    assert_eq!(report["static_load"]["state"], "passed", "{report:#}");
    apply(&mut session, &source.replace("'2000'", "'500'"));
    assert_eq!(
        session.snapshot().evaluator_node_ids().collect::<Vec<_>>(),
        initial.evaluator_node_ids().collect::<Vec<_>>()
    );
    let report = session.validators(&selection);
    assert_eq!(report["static_load"]["state"], "failed", "{report:#}");
    apply(
        &mut session,
        &source.replace("'input:physics.mass_kg.occurrence.{occurrence}':'100',", ""),
    );
    assert_eq!(session.snapshot().evaluator_node_count(), 5);
    let report = session.validators(&selection);
    assert_eq!(
        report["static_load"]["state"], "not_evaluated",
        "{report:#}"
    );
    session.undo().unwrap();
    assert_eq!(session.snapshot().evaluator_node_count(), 6);
    assert_eq!(
        session.validators(&selection)["static_load"]["state"],
        "failed"
    );
    session.redo().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("loads.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().evaluator_node_count(), 5);
    assert_eq!(
        reopened.validators(&selection)["static_load"]["state"],
        "not_evaluated"
    );
}

#[test]
fn renamed_part_drops_source_owned_classifications_when_attributes_are_removed() {
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let id = session.snapshot().occurrences().next().unwrap().id();
    apply(
        &mut session,
        "p=box('renamed',(1000,300,20))\ncontinue_part(p,was='shelf')",
    );
    assert_eq!(session.snapshot().occurrence(id).unwrap().name(), "renamed");
    assert!(classified(&session.snapshot(), MATERIAL_DIMENSION_V1).is_empty());
    assert!(classified(&session.snapshot(), "ketchup.validator-role.v1").is_empty());
    assert!(classified(&session.snapshot(), "finish").is_empty());
    session.undo().unwrap();
    assert_eq!(
        classified(&session.snapshot(), MATERIAL_DIMENSION_V1)["shelf"],
        "steel"
    );
}

#[test]
fn numeric_inputs_follow_renames_and_removed_parts_without_orphan_values() {
    let mut session = DocumentSession::default();
    let source = "p=box('part',(10,10,10),attributes={'input:custom.occurrence.{occurrence}':'12'})\nbox('other',(10,10,10),at=(20,0,0),attributes={'input:global':'7'})";
    apply(&mut session, source);
    let initial = session.snapshot();
    let ids = initial.evaluator_node_ids().collect::<Vec<_>>();
    apply(
        &mut session,
        &format!(
            "{}\ncontinue_part(p,was='part')",
            source.replace("'part'", "'renamed'")
        ),
    );
    assert_eq!(
        session.snapshot().evaluator_node_ids().collect::<Vec<_>>(),
        ids
    );
    apply(
        &mut session,
        "box('other',(10,10,10),at=(20,0,0),attributes={'input:global':'9'})",
    );
    let remaining = session.snapshot();
    assert_eq!(remaining.evaluator_node_count(), 1);
    let input = remaining.evaluator_nodes().next().unwrap();
    assert_eq!(input.name(), "global");
    assert_eq!(input.dimension().unwrap().millimetres(), 9.0);
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().evaluator_node_ids().collect::<Vec<_>>(),
        ids
    );
    session.redo().unwrap();
    assert_eq!(session.snapshot().evaluator_node_count(), 1);
}

#[test]
fn conflicting_or_nonfinite_program_inputs_leave_the_previous_model_unchanged() {
    let mut session = DocumentSession::default();
    let source = "box('part',(10,10,10),attributes={'input:global':'12'})";
    apply(&mut session, source);
    let before = session.snapshot();
    for invalid in [
        source.replace("'12'", "'nan'"),
        source.replace("'12'", "'not-a-number'"),
        format!("{source}\nbox('other',(10,10,10),at=(20,0,0),attributes={{'input:global':'13'}})"),
    ] {
        assert!(
            session
                .apply_rule_program(
                    RuleProgramSource {
                        file_name: "invalid.star".into(),
                        source: invalid,
                        overrides: BTreeMap::new(),
                    },
                    false
                )
                .is_err()
        );
        assert_eq!(
            session.snapshot().canonical_digest(),
            before.canonical_digest()
        );
        assert_eq!(
            session
                .snapshot()
                .evaluator_nodes()
                .next()
                .unwrap()
                .dimension()
                .unwrap()
                .millimetres(),
            12.0
        );
        assert_eq!(session.rule_program().unwrap().source, source);
    }
    apply(
        &mut session,
        &format!(
            "{source}\nbox('other',(10,10,10),at=(20,0,0),attributes={{'input:global':'12'}})"
        ),
    );
    assert_eq!(session.snapshot().evaluator_node_count(), 1);
}

#[test]
fn removing_a_program_input_cannot_break_an_existing_graph_dependency() {
    let mut session = DocumentSession::default();
    let source = "box('part',(10,10,10),attributes={'input:custom.load':'12'})";
    apply(&mut session, source);
    let node = session.snapshot().evaluator_node_ids().next().unwrap();
    session
        .apply_rule_commands_with_source(
            CommandBatch::new(vec![CanonicalCommand::CreateEvaluatorNode {
                id: ketchup_model::document::NodeId(100),
                name: "dependent".into(),
                dimension: ketchup_model::document::Dimension::new("4", 4.0).unwrap(),
                dependencies: vec![node],
            }]),
            session.rule_program().unwrap().clone(),
        )
        .unwrap();
    let before = session.snapshot();
    let result = session.apply_rule_program(
        RuleProgramSource {
            file_name: "metadata.star".into(),
            source: "box('part',(10,10,10))".into(),
            overrides: BTreeMap::new(),
        },
        false,
    );
    assert!(result.is_err());
    assert_eq!(session.snapshot().evaluator_node_count(), 2);
    assert_eq!(
        session.snapshot().canonical_digest(),
        before.canonical_digest()
    );
}

#[test]
fn mixed_root_and_nested_roles_never_hide_unchecked_members() {
    let mut session = DocumentSession::default();
    let source = format!(
        "a={SOURCE}\nc=component('shared',[a])\ninstance('copy',c,at=(2000,0,0))\n{}",
        SOURCE
            .replace("'shelf'", "'root'")
            .replace("(1000,300,20)", "(1000,300,20),at=(4000,0,0)")
    );
    let selection = AssistantValidationSelection::only(&["beam_deflection"]);
    for (material, state) in [("steel", "not_evaluated"), ("engineered_wood", "failed")] {
        apply(&mut session, &source.replace("steel", material));
        let report = session.validators(&selection);
        let detail = &report["beam_deflection"];
        assert_eq!(report["state"], state, "{report:#}");
        assert_eq!(detail["state"], state, "{report:#}");
        assert_eq!(detail["complete"], false);
        assert_eq!(detail["evaluations"].as_array().unwrap().len(), 1);
        assert_eq!(detail["evaluations"][0]["name"], "root");
        let missing = detail["not_evaluated"].as_array().unwrap();
        assert!(
            missing.iter().any(|row| row["name"]
                .as_str()
                .is_some_and(|name| name.contains("shelf"))
                && row["reason"] == "nested_role_inputs_unsupported"),
            "{detail:#}"
        );
        assert!(missing.iter().all(|row| row["instance_path"].is_object()));
    }
}

#[test]
fn missing_material_is_an_explicit_library_assumption_not_a_measured_property() {
    let mut session = DocumentSession::default();
    apply(&mut session, &SOURCE.replace(",material='steel'", ""));
    let report = session.validators(&AssistantValidationSelection::only(&["beam_deflection"]));
    let detail = &report["beam_deflection"];
    assert_eq!(detail["evaluations"][0]["material_source"], "rules_default");
    assert!(
        detail["assumptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row.as_str().unwrap().contains("default material"))
    );
}

#[test]
fn nested_metadata_never_classifies_the_assembly_root() {
    let mut session = DocumentSession::default();
    let source = format!("a={SOURCE}\nc=component('shared',[a])\ninstance('copy',c,at=(2000,0,0))");
    apply(&mut session, &source);
    let before = session.snapshot().features().cloned().collect::<Vec<_>>();
    apply(&mut session, &source.replace("steel", "aluminium"));
    assert!(classified(&session.snapshot(), MATERIAL_DIMENSION_V1).is_empty());
    assert!(classified(&session.snapshot(), "ketchup.validator-role.v1").is_empty());
    assert_eq!(
        session.snapshot().features().cloned().collect::<Vec<_>>(),
        before
    );
    let selection = AssistantValidationSelection::only(&["beam_deflection"]);
    assert_eq!(
        session.validators(&selection)["beam_deflection"]["state"],
        "not_evaluated"
    );
}
