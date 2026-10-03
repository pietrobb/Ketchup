use crate::*;
use egui_kittest::kittest::Queryable;

const SOURCE: &str = "a=box(\"a\",(20,30,40))\nb=box(\"b\",(10,10,10),at=(100,0,0))\ng=group(\"inner\",[a])\nh=group(\"wrapper\",[g,b])\nc=component(\"assembly\",[h])\ninstance(\"second\",c,at=(200,0,0))\ngroup(\"outside\",[])";

fn app() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "groups.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let root = app
        .document
        .current()
        .occurrences()
        .find(|item| item.name() == "second")
        .unwrap()
        .id();
    assert!(app.enter_occurrence_context(InstancePath::root(root)));
    app
}

fn group_label(app: &KetchupApp, name: &str, count: usize) -> String {
    app.catalog.format(
        "outliner-group",
        &BTreeMap::from([("name", name.to_owned()), ("count", count.to_string())]),
    )
}

#[test]
fn component_tree_shows_local_group_hierarchy_and_selects_only_its_instance() {
    let app = app();
    let before = app.document.current().scene_query();
    let inner = group_label(&app, "inner", 1);
    let wrapper = group_label(&app, "wrapper", 2);
    let outside = group_label(&app, "outside", 0);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1800.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    assert!(
        harness.query_by_label(&wrapper).is_some(),
        "wrapper group missing"
    );
    assert!(
        harness.query_by_label(&inner).is_some(),
        "inner group missing"
    );
    assert!(
        harness.query_by_label(&outside).is_none(),
        "root groups leak into component context"
    );
    assert!(
        harness.get_by_label(&inner).rect().max.y < 1800.0,
        "inner outside viewport: {:?}",
        harness.get_by_label(&inner).rect()
    );
    harness.get_by_label(&inner).click();
    harness.run_steps(3);
    let selected = harness.state().selected_instance_paths();
    assert_eq!(selected.len(), 1);
    let path = selected.first().unwrap();
    let snapshot = harness.state().document.current();
    assert_eq!(
        snapshot.occurrence(path.root_occurrence()).unwrap().name(),
        "second"
    );
    assert_eq!(
        before
            .iter()
            .find(|part| part.instance_path == *path)
            .unwrap()
            .occurrence_name,
        "a"
    );
    assert_eq!(snapshot.scene_query(), before);
    harness.get_by_label(&wrapper).click();
    harness.run_steps(3);
    let all = harness.state().selected_instance_paths();
    assert_eq!(all.len(), 2);
    assert!(
        all.iter()
            .all(|item| item.root_occurrence() == path.root_occurrence())
    );
    assert_eq!(
        before
            .iter()
            .filter(|item| all.contains(&item.instance_path))
            .map(|item| item.occurrence_name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["a", "b"])
    );
    assert!(harness.state_mut().exit_edit_context());
    harness.run_steps(3);
    assert!(harness.query_by_label(&outside).is_some());
    assert!(harness.query_by_label(&inner).is_none());
}

#[test]
fn grounding_a_local_group_preserves_headless_selection_and_sibling_geometry() {
    let mut app = app();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "groups.star".into(),
            source: SOURCE.replace("group(\"inner\",[a])", "group(\"inner\",[a],grounded=True)"),
            overrides: BTreeMap::new(),
        },
        false,
    )
    .unwrap();
    let before = app.document.current().scene_query();
    let inner = group_label(&app, "inner", 1);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600., 1800.))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness.get_by_label(&inner).click();
    harness.run_steps(3);
    let selected = harness.state().selected_instance_paths();
    assert_eq!(selected.len(), 1);
    let snapshot = harness.state().document.current();
    assert!(snapshot.instance_is_grounded(selected.first().unwrap()));
    for leaf in &before {
        assert_eq!(
            snapshot.instance_is_grounded(&leaf.instance_path),
            leaf.occurrence_name == "a"
        );
    }
    assert_eq!(snapshot.scene_query(), before);
}

#[test]
fn posed_component_still_selects_its_member_in_headless_outliner() {
    let mut app = app();
    let source = format!(
        "{SOURCE}\nanchor=box(\"anchor\",(10,10,10),at=(-100,0,0))\njoint(anchor,\"second\",kind=\"motion\",motion=slide((0,1,0),0,100),position=40)"
    );
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "groups.star".into(),
            source,
            overrides: BTreeMap::new(),
        },
        false,
    )
    .unwrap();
    while app.exit_edit_context() {}
    let before = app.document.current();
    let root = before
        .occurrences()
        .find(|p| p.name() == "second")
        .unwrap()
        .id();
    assert!(app.enter_occurrence_context(InstancePath::root(root)));
    let inner = group_label(&app, "inner", 1);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1800.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness.get_by_label(&inner).click();
    harness.run_steps(3);
    let selected = harness.state().selected_instance_paths();
    assert_eq!(selected.len(), 1);
    let path = selected.first().unwrap();
    assert_eq!(path.root_occurrence(), root);
    let leaf = before
        .scene_query()
        .into_iter()
        .find(|p| &p.instance_path == path)
        .unwrap();
    assert_eq!(leaf.occurrence_name, "a");
    assert_eq!(leaf.transform.transform_point([0.0; 3]), [200.0, 40.0, 0.0]);
    assert_eq!(
        harness.state().document.current().scene_query(),
        before.scene_query()
    );
    assert!(harness.state().document.current_rule_program().is_some());
}

#[test]
fn internal_shared_motion_keeps_local_group_selection_and_world_placement() {
    let mut app = app();
    let source = SOURCE.replace(
        "c=component",
        "joint(b,g,kind='motion',motion=slide((0,1,0),0,100),position=40)\nc=component",
    );
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "groups.star".into(),
            source,
            overrides: BTreeMap::new(),
        },
        false,
    )
    .unwrap();
    while app.exit_edit_context() {}
    let before = app.document.current();
    let root = before
        .occurrences()
        .find(|p| p.name() == "second")
        .unwrap()
        .id();
    assert!(app.enter_occurrence_context(InstancePath::root(root)));
    let inner = group_label(&app, "inner", 1);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600., 1800.))
        .with_step_dt(1. / 60.)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness.get_by_label(&inner).click();
    harness.run_steps(3);
    let selected = harness.state().selected_instance_paths();
    assert_eq!(selected.len(), 1);
    let path = selected.first().unwrap();
    assert_eq!(path.root_occurrence(), root);
    let scene = before.scene_query();
    let leaf = scene.iter().find(|p| &p.instance_path == path).unwrap();
    assert_eq!(leaf.occurrence_name, "a");
    assert_eq!(leaf.transform.transform_point([0.; 3]), [200., 40., 0.]);
    let original = scene
        .iter()
        .find(|p| p.occurrence_name == "a" && p.instance_path.root_occurrence() != root)
        .unwrap();
    assert_eq!(original.transform.transform_point([0.; 3]), [0., 40., 0.]);
    assert_eq!(harness.state().document.current().scene_query(), scene);
    assert!(harness.state().document.current_rule_program().is_some());
}

#[test]
fn continued_shared_member_is_named_and_selected_in_the_outliner() {
    let mut app = app();
    let before = app.document.current();
    let root = before
        .occurrences()
        .find(|p| p.name() == "second")
        .unwrap()
        .id();
    let old_path = before
        .scene_query()
        .into_iter()
        .find(|p| p.occurrence_id == root && p.occurrence_name == "a")
        .unwrap()
        .instance_path;
    let source = SOURCE.replace(
        "a=box(\"a\",(20,30,40))",
        "a=box(\"renamed\",(20,30,40))\ncontinue_part(a,was=\"a\")",
    );
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "groups.star".into(),
            source,
            overrides: BTreeMap::new(),
        },
        false,
    )
    .unwrap();
    while app.exit_edit_context() {}
    assert!(app.enter_occurrence_context(InstancePath::root(root)));
    let label = |name: &str| {
        app.catalog.format(
            "outliner-instance",
            &BTreeMap::from([("name", name.to_owned()), ("visibility", "◉".to_owned())]),
        )
    };
    let old_label = label("a");
    let new_label = label("renamed");
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1800.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    assert!(harness.query_by_label(&old_label).is_none());
    harness.get_by_label(&new_label).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().selected_instance_paths(),
        BTreeSet::from([old_path])
    );
}

#[test]
fn nested_component_context_selects_only_leaves_of_the_chosen_inner_copy() {
    let mut app = KetchupApp::new();
    let source = "a=box(\"a\",(20,30,40))\nc=component(\"child\",[group(\"inside\",[a])])\ni=instance(\"inner\",c,at=(200,0,0))\np=component(\"parent\",[group(\"pair\",[c,i])])\ninstance(\"outer\",p,at=(800,500,0),x=(0,1,0))\n";
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "nested.star".into(),
            source: source.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let snapshot = app.document.current();
    let before = snapshot.scene_query();
    let root = snapshot
        .occurrences()
        .find(|part| part.name() == "outer")
        .unwrap()
        .id();
    let inner = before
        .iter()
        .find(|part| part.occurrence_id == root && part.occurrence_name == "inner")
        .unwrap()
        .instance_path
        .clone();
    assert!(app.enter_occurrence_context(InstancePath::root(root)));
    assert!(app.enter_occurrence_context(inner));
    let label = group_label(&app, "inside", 1);
    let pair = group_label(&app, "pair", 2);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1800.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    assert!(harness.query_by_label(&pair).is_none());
    assert!(harness.get_by_label(&label).rect().max.y < 1800.0);
    harness.get_by_label(&label).click();
    harness.run_steps(3);
    let selected = harness.state().selected_instance_paths();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        ketchup_application::rule_program_part_name(&snapshot, selected.first().unwrap())
            .as_deref(),
        Some("outer/inner/a")
    );
    assert_eq!(harness.state().document.current().scene_query(), before);
    assert!(harness.state_mut().exit_edit_context());
    harness.run_steps(3);
    assert!(harness.query_by_label(&label).is_none());
    assert!(harness.query_by_label(&pair).is_some());
}

#[test]
fn program_read_exposes_shared_groups_members_and_parent_keys() {
    let app = app();
    let view = app.program_source_view().unwrap();
    let component = &view["components"][0];
    let groups = component["groups"]
        .as_array()
        .expect("local groups missing from MCP read");
    assert_eq!(groups.len(), 2);
    let inner = groups
        .iter()
        .find(|group| group["name"] == "inner")
        .unwrap();
    let wrapper = groups
        .iter()
        .find(|group| group["name"] == "wrapper")
        .unwrap();
    assert_eq!(inner["parent_local_group_id"], wrapper["local_group_id"]);
    assert!(wrapper["parent_local_group_id"].is_null());
    let members = component["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    let a = members.iter().find(|part| part["name"] == "a").unwrap();
    assert_eq!(a["parent_local_group_id"], inner["local_group_id"]);
    let definition = a["definition_id"].as_u64().unwrap();
    assert_eq!(
        view["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["definition_id"] == definition)
            .count(),
        2
    );
    assert_eq!(component["instances"].as_array().unwrap().len(), 2);
}
