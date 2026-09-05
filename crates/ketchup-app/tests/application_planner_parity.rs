mod harness;

use std::collections::BTreeSet;

use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_application::plan_assistant_cad_edit_program;
use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadPartFeature, AssistantCadRotation, AssistantPrincipalPlane, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use ketchup_core::document::{DocumentStore, OccurrenceId};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::persistence::{self, LoadOutcome};

fn same_document(shell: &Shell) -> DocumentStore {
    match persistence::load(&persistence::save(&shell.app().document_snapshot())).unwrap() {
        LoadOutcome::Editable { document, .. } => document,
        LoadOutcome::ReviewOnly(_) => panic!("current document must reload losslessly"),
    }
}

#[test]
fn gui_and_application_plan_identical_creation_and_selected_transforms() {
    let mut shell = Shell::new();
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    let document = same_document(&shell);
    let selection = document
        .current()
        .occurrences()
        .map(|occurrence| occurrence.id())
        .collect::<BTreeSet<_>>();
    assert_eq!(selection.len(), shell.app().selected_occurrence_count());
    let topology = ExactResultRegistry::default();
    let before = shell.app().canonical_digest();
    let revision = shell.app().document_revision();
    let undo = shell.app().undo_step_count();
    let programs = [
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Scripted cylinder".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities: vec![AssistantSketchEntity::Circle {
                    id: 1,
                    center_mm: [0.0, 0.0],
                    radius_mm: 12.0,
                }],
                constraints: vec![],
                feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
                translation_mm: [5.0, 6.0, 7.0],
                rotation: Some(AssistantCadRotation {
                    pivot_mm: [5.0, 6.0, 7.0],
                    axis: [0.0, 1.0, 0.0],
                    angle_degrees: 45.0,
                }),
            }],
        },
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::Transform {
                    selector: AssistantCadEntitySelector::CurrentSelection {},
                    translation_mm: [10.0, 20.0, 30.0],
                    rotation: None,
                },
                AssistantCadEditOperation::Copy {
                    selector: AssistantCadEntitySelector::CurrentSelection {},
                    translation_mm: [0.0, 20.0, 0.0],
                },
                AssistantCadEditOperation::LinearPattern {
                    selector: AssistantCadEntitySelector::CurrentSelection {},
                    instances: 3,
                    step_mm: [30.0, 0.0, 0.0],
                },
            ],
        },
    ];
    for program in programs {
        let direct = plan_assistant_cad_edit_program(&document, &selection, &topology, &program)
            .expect("shared application plan");
        let gui = shell
            .app()
            .plan_assistant_cad_edit_program(&program)
            .expect("GUI delegated plan");
        assert_eq!(gui, direct);
        assert!(!direct.commands().is_empty());
    }
    assert_eq!(shell.app().canonical_digest(), before);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().undo_step_count(), undo);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn gui_and_application_return_identical_missing_target_diagnostics() {
    let shell = Shell::new();
    let document = same_document(&shell);
    let missing = OccurrenceId(u64::MAX);
    assert!(document.current().occurrence(missing).is_none());
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Transform {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![missing.0],
            },
            translation_mm: [1.0, 0.0, 0.0],
            rotation: None,
        }],
    };
    let direct = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program,
    )
    .unwrap_err();
    let gui = shell
        .app()
        .plan_assistant_cad_edit_program(&program)
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(&gui).unwrap(),
        serde_json::to_value(&direct).unwrap()
    );
    assert_eq!(direct.target, format!("occurrence:{}", missing.0));
    assert_eq!(
        shell.app().canonical_digest(),
        document.current().canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
}
