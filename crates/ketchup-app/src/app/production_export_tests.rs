//! Review 2026-10-09 MF-N2/MF-N3: the machine files make the parts of the
//! program's `production(tags=…)` scope whether their layers are shown or
//! hidden; without a scope they make the shown parts and leave hidden ones out.
use crate::*;

const TIMBER: &str =
    "attributes = {'classification:ketchup.fabrication-role.v1': 'fabrication.timber-member.v1'}";
const BOUGHT: &str =
    "attributes = {'classification:ketchup.fabrication-role.v1': 'fabrication.purchased-item.v1'}";

/// Two floors of timber, a bought angle bracket and a concept wall through the
/// ground floor beam, each on its own layer, with the given layers hidden and
/// evaluated to exact solids.
fn house(scope: &str, hidden: &[&str]) -> KetchupApp {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "house.star".into(),
            source: format!(
                "box('prízemie/trám', (60, 140, 2000), at = (0, 0, 0), tags = ['prízemie'], {TIMBER})\n\
                 box('poschodie/trám', (60, 140, 2000), at = (0, 0, 3000), tags = ['poschodie'], {TIMBER})\n\
                 box('kovanie/uholník', (40, 40, 40), at = (500, 0, 0), tags = ['kovanie'], {BOUGHT})\n\
                 box('obal/stena', (3000, 160, 2500), at = (0, 50, 0), tags = ['koncept'], {TIMBER})\n\
                 {scope}"
            ),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    for name in hidden {
        let tag = app
            .document
            .current()
            .tags()
            .find(|tag| tag.name() == *name)
            .unwrap()
            .id();
        assert!(app.set_tag_visibility(tag, false), "{name}");
    }
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    app
}

/// The program names of the parts the machine files make, and how many
/// timber parts the BTLx file holds.
fn made(app: &mut KetchupApp) -> (Vec<String>, usize) {
    let projection = app.current_general_fabrication_projection().unwrap();
    let snapshot = app.document.current();
    let mut names = projection
        .bom
        .rows
        .iter()
        .flat_map(|row| &row.instances)
        .map(|path| ketchup_application::rule_program_part_name(&snapshot, path).unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert!(
        projection.bom_export(&snapshot).is_ok()
            && projection.manufacturing_export(&snapshot).is_ok(),
        "{:?} {:?} {:?} {:?} {:?}",
        projection.bom_export(&snapshot).err(),
        projection.manufacturing_export(&snapshot).err(),
        projection.bom.envelope.status,
        projection.manufacturing.envelope.status,
        projection.manufacturing.validation_state,
    );
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    (names, btlx.matches("<Part ").count())
}

#[test]
fn without_a_production_scope_the_shown_floor_is_made_and_hidden_hardware_does_not_block_it() {
    let mut app = house("", &["poschodie", "kovanie", "koncept"]);
    assert_eq!(made(&mut app), (vec!["prízemie/trám".to_owned()], 1));
}

#[test]
fn a_production_scope_makes_its_hidden_parts_and_never_the_shown_concept() {
    let mut app = house(
        "production(tags = ['prízemie', 'poschodie', 'kovanie'])\n",
        &["poschodie", "kovanie"],
    );
    assert_eq!(
        made(&mut app),
        (
            vec![
                "kovanie/uholník".to_owned(),
                "poschodie/trám".to_owned(),
                "prízemie/trám".to_owned(),
            ],
            2
        )
    );
}

/// After an Open or Undo the program may still be evaluated in the background:
/// the export says so instead of waiting for it on the window's thread.
#[test]
fn an_export_while_the_program_is_evaluated_does_not_wait_for_it() {
    let mut app = house("production(tags = ['prízemie'])\n", &[]);
    let program = app.document.current_rule_program().unwrap().clone();
    let _held = app.program_evaluations.hold(&program);
    let rejection = app.current_general_fabrication_projection().err().unwrap();
    assert_eq!(rejection.code(), "fabrication.program_planning");
}

/// A hidden part is still checked when it is made: a bracket inside the hidden
/// upper floor beam blocks the machine files.
#[test]
fn a_hidden_production_part_colliding_with_another_blocks_the_machine_files() {
    let mut app = house(
        &format!(
            "box('kovanie/v trame', (40, 40, 40), at = (10, 10, 3100), tags = ['kovanie'], {BOUGHT})\n\
             production(tags = ['prízemie', 'poschodie', 'kovanie'])\n"
        ),
        &["poschodie", "kovanie"],
    );
    let projection = app.current_general_fabrication_projection().unwrap();
    assert_eq!(
        projection.manufacturing.validation_state,
        ketchup_model::validation::ValidationState::Failed
    );
    assert!(
        projection
            .manufacturing_export(&app.document.current())
            .is_err()
    );
}
