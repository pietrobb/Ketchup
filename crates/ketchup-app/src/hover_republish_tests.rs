//! An exact result that arrives in the background replaces a body's box stand-in
//! under an unmoved pointer. The body the user hovers must stay hovered, or the
//! next click acts on a different solid than the highlighted one.
use super::*;

fn last_feature(app: &KetchupApp, definition_id: DefinitionId) -> FeatureId {
    *app.document
        .current()
        .definition(definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap()
}

#[test]
fn hovered_body_survives_exact_results_replacing_box_stand_ins() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let snapshot = app.document.current();
    let definition = |occurrence: u64| {
        snapshot
            .occurrence(OccurrenceId(occurrence))
            .unwrap()
            .definition_id()
    };
    let (first, second) = (definition(1), definition(2));
    assert_ne!(first, second);
    let context = egui::Context::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
        ..Default::default()
    };
    let _ = context.run(input(), |context| app.ui(context));
    // Both top faces lie at z = 20 here, so the ray reaches them at one distance.
    let pointer = app.viewport_position(Vec3::new(60.0, 45.0, 20.0)).unwrap();
    let mut raw = input();
    raw.events.push(egui::Event::PointerMoved(pointer));
    let _ = context.run(raw, |context| app.ui(context));
    let hovered = |app: &KetchupApp| {
        app.hovered_selection()
            .map(|selection| selection.instance_path.clone())
    };
    let tool = Some(InstancePath::root(OccurrenceId(2)));
    for _ in 0..4 {
        if hovered(&app) == tool {
            break;
        }
        assert!(app.cycle_hover_overlap());
    }
    assert_eq!(hovered(&app), tool);

    // The other body's exact result arrives first, then the hovered body's own,
    // which gives the hovered body new face references.
    for definition_id in [first, second] {
        let producer = last_feature(&app, definition_id);
        install_graph_result(&mut app, definition_id, producer, None);
        let _ = context.run(input(), |context| app.ui(context));
        assert_eq!(
            hovered(&app),
            tool,
            "exact result of {definition_id:?} moved the hover to {:?}, choice {:?}",
            app.hovered_selection(),
            app.hovered_overlap_choice()
        );
    }
}

/// The line through two parts enters and leaves each of them. A choice of the
/// face a part is left through stays on that face when the exact result renames
/// it, not on the face of the same part the line enters through.
#[test]
fn a_chosen_far_face_survives_exact_results_renaming_it() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let snapshot = app.document.current();
    let definitions = [1, 2].map(|occurrence| {
        snapshot
            .occurrence(OccurrenceId(occurrence))
            .unwrap()
            .definition_id()
    });
    let context = egui::Context::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
        ..Default::default()
    };
    let _ = context.run(input(), |context| app.ui(context));
    let pointer = app.viewport_position(Vec3::new(60.0, 45.0, 20.0)).unwrap();
    let mut raw = input();
    raw.events.push(egui::Event::PointerMoved(pointer));
    let _ = context.run(raw, |context| app.ui(context));
    let candidates = |app: &KetchupApp| {
        app.hover
            .pick
            .as_ref()
            .unwrap()
            .overlapping
            .iter()
            .map(|hit| hit.reference.clone())
            .collect::<Vec<_>>()
    };
    let before = candidates(&app);
    let far = (1..before.len())
        .find(|index| {
            before[..*index]
                .iter()
                .any(|earlier| earlier.instance_path == before[*index].instance_path)
        })
        .expect("the line must cross one part twice");
    while app.hovered_overlap_choice().unwrap().0 != far {
        assert!(app.cycle_hover_overlap());
    }
    let part = before[far].instance_path.clone();

    for definition_id in definitions {
        let producer = last_feature(&app, definition_id);
        install_graph_result(&mut app, definition_id, producer, None);
        let _ = context.run(input(), |context| app.ui(context));
    }
    assert_ne!(
        candidates(&app),
        before,
        "the exact faces must replace the box faces"
    );
    assert_eq!(app.hovered_overlap_choice().unwrap().0, far);
    assert_eq!(app.hovered_selection().unwrap().instance_path, part);
}
