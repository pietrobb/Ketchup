//! The viewport draws a program's annotations where they are, keeps their text one
//! size on screen at every zoom, follows their tag, and picks one by a click.
use crate::*;

type Shell = egui_kittest::Harness<'static, KetchupApp>;

/// A plate, a numbered point on it and a 5 m dimension along its south edge.
const PLAN: &str = "box('plate', (6000, 3000, 20))\n\
                    text('S-05', 'S-05', at=(2000, 1000, 20), height_mm=3.5, tags='plan')\n\
                    dim('south', (0, 0, 20), (5000, 0, 20), offset_mm=-1000, tags='plan')\n";

fn shell() -> Shell {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "plan.star".into(),
            source: PLAN.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    // The plate's exact solid is there before the first frame, so Zoom Fit frames it.
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    harness.state_mut().dispatch_command(AppCommand::ViewTop);
    harness.state_mut().dispatch_command(AppCommand::ZoomFit);
    harness.run_steps(2);
    harness
}

fn drawn(harness: &Shell) -> Vec<super::annotations_view::DrawnAnnotation> {
    let rect = harness.state().viewport_rect().unwrap();
    harness.state().drawn_annotations(&harness.ctx, rect)
}

fn on_screen(harness: &Shell, point: [f64; 3]) -> Pos2 {
    harness
        .state()
        .viewport_position(Vec3::new(point[0], point[1], point[2]))
        .unwrap()
}

/// The one text of the annotation `name`: its string, where its baseline starts
/// and how tall its line is on screen.
fn text_of(drawn: &[super::annotations_view::DrawnAnnotation], name: &str) -> (String, Pos2, f32) {
    let annotation = drawn.iter().find(|drawn| drawn.name == name).unwrap();
    let [text] = annotation.texts.as_slice() else {
        panic!("{name} writes one text");
    };
    let row = &text.galley.rows[0];
    let baseline = row.pos.y + row.row.glyphs[0].pos.y;
    let turned = egui::emath::Rot2::from_angle(text.angle) * egui::vec2(0.0, baseline);
    (
        text.galley.text().to_owned(),
        text.pos + turned,
        text.galley.rect.height(),
    )
}

fn click(harness: &mut Shell, at: Pos2) {
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
}

#[test]
fn annotations_are_drawn_at_their_points_and_their_text_keeps_its_size_at_every_zoom() {
    let mut harness = shell();
    let before = drawn(&harness);
    assert_eq!(
        before
            .iter()
            .map(|drawn| drawn.name.as_str())
            .collect::<Vec<_>>(),
        ["S-05", "south"]
    );
    let (text, baseline, height) = text_of(&before, "S-05");
    assert_eq!(text, "S-05");
    assert!(
        harness.state().viewport_rect().unwrap().contains(baseline),
        "{baseline:?}"
    );
    assert!(
        baseline.distance(on_screen(&harness, [2000.0, 1000.0, 20.0])) < 1.0,
        "the text starts at its point: {baseline:?}"
    );
    // The dimension writes the length the document measured, over the middle of
    // its line 1 m south of the measured edge.
    let (value, value_at, _) = text_of(&before, "south");
    assert_eq!(value, "5000");
    let middle = on_screen(&harness, [2500.0, -1000.0, 20.0]);
    assert!(
        (value_at.y - middle.y).abs() < 12.0 && value_at.y < middle.y,
        "{value_at:?} {middle:?}"
    );
    let south = before.iter().find(|drawn| drawn.name == "south").unwrap();
    assert!(
        south.lines.iter().any(|(points, _)| points.len() == 2
            && points[0].distance(on_screen(&harness, [0.0, -1000.0, 20.0])) < 1.0
            && points[1].distance(on_screen(&harness, [5000.0, -1000.0, 20.0])) < 1.0),
        "the dimension line runs between its points: {:?}",
        south.lines
    );

    let spacing = |harness: &Shell| {
        on_screen(harness, [0.0, 0.0, 20.0]).distance(on_screen(harness, [5000.0, 0.0, 20.0]))
    };
    let far = spacing(&harness);
    for _ in 0..3 {
        harness.state_mut().dispatch_command(AppCommand::ZoomIn);
    }
    harness.step();
    assert!(spacing(&harness) > far * 1.5, "zoomed in");
    let (_, baseline, zoomed) = text_of(&drawn(&harness), "S-05");
    assert!((zoomed - height).abs() < 0.01, "{height} then {zoomed}");
    assert!(baseline.distance(on_screen(&harness, [2000.0, 1000.0, 20.0])) < 1.0);
}

#[test]
fn a_click_picks_an_annotation_and_its_tag_hides_it() {
    let mut harness = shell();
    let plate = on_screen(&harness, [5000.0, 2500.0, 20.0]);
    click(&mut harness, plate);
    assert_eq!(harness.state().selected_instance_paths().len(), 1);
    // Picking the annotation right after the plate is no double click into the
    // plate's definition.
    let (_, baseline, height) = text_of(&drawn(&harness), "S-05");
    click(
        &mut harness,
        baseline + egui::vec2(height / 2.0, -height / 3.0),
    );
    assert_eq!(harness.state().picked_annotation_name(), Some("S-05"));
    assert!(
        harness.state().selected_instance_paths().is_empty(),
        "no part"
    );
    assert!(harness.state().action_digest().contains("S-05"));
    assert_eq!(harness.state().edit_context_depth(), 0);
    // What `inspect status` tells a client as picked_annotation.
    let summary = harness.state().picked_annotation_summary().unwrap();
    assert_eq!(summary["name"], "S-05");
    assert_eq!(summary["kind"], "text");
    assert_eq!(summary["text"], "S-05");
    assert_eq!(summary["tags"][0], "plan");

    // A click beside every annotation lets it go and selects the part under it,
    // also as the second click of a double click that began on the annotation.
    harness.run_steps(60);
    click(
        &mut harness,
        baseline + egui::vec2(height / 2.0, -height / 3.0),
    );
    click(&mut harness, plate);
    assert_eq!(harness.state().picked_annotation_name(), None);
    assert_eq!(harness.state().selected_instance_paths().len(), 1);
    assert_eq!(harness.state().edit_context_depth(), 0);

    click(
        &mut harness,
        baseline + egui::vec2(height / 2.0, -height / 3.0),
    );
    assert_eq!(harness.state().picked_annotation_name(), Some("S-05"));
    let plan = harness
        .state()
        .document_snapshot()
        .tags()
        .find(|tag| tag.name() == "plan")
        .map(|tag| tag.id())
        .unwrap();
    assert!(harness.state_mut().set_tag_visibility(plan, false));
    harness.step();
    assert!(drawn(&harness).is_empty(), "the tag hides them");
    assert_eq!(harness.state().picked_annotation_name(), None);

    // Two clicks on the plate itself still open it; a second of quiet first, so
    // egui counts them as a double click, not a sixth click in a row.
    harness.run_steps(60);
    click(&mut harness, plate);
    click(&mut harness, plate);
    assert_eq!(harness.state().edit_context_depth(), 1);
}
