//! Applying a program in the window (Apply, Push/Pull, a drawn shape) evaluates
//! it in the program worker: one that takes all the memory is refused with
//! `memory_limit`, the model stays as it was and the window keeps working
//! (review PRG-M1).
use crate::*;
use ketchup_application::RuleProgramApplyError;
use ketchup_model::document::RuleProgramSource;

fn program(source: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "box.star".into(),
        source: source.into(),
        overrides: BTreeMap::new(),
    }
}

#[test]
fn applying_a_program_past_the_memory_limit_leaves_the_window_and_model_as_they_were() {
    let mut app = KetchupApp::new();
    app.apply_program_source(program("box(\"a\", (10, 20, 30))\n"), true)
        .unwrap();
    let before = app.document.current();
    let error = app
        .apply_program_source(
            program("s = \"ab\" * 50000\nt = s.replace(\"\", s)\nbox(\"a\", (10, 20, 30))\n"),
            false,
        )
        .unwrap_err();
    let RuleProgramApplyError::Program { code, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(code, "memory_limit", "{error:?}");
    assert_eq!(before.revision_id(), app.document.current().revision_id());
    app.apply_program_source(program("box(\"a\", (10, 20, 40))\n"), false)
        .unwrap();
    assert_eq!(
        app.document.current_rule_program().unwrap().source,
        "box(\"a\", (10, 20, 40))\n"
    );
}
