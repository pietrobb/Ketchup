//! Project drawings of the timber-frame house: the sheet carries the floor plan,
//! both sections and four elevations with a title block, follows the visible
//! layers and is written as PDF.
use crate::app::project_drawings::ProjectDrawingsError;
use crate::tests::shell::step_until;
use crate::*;
use ketchup_manufacturing::project_drawings::{ProjectSheet, ProjectView};
use ketchup_manufacturing::title_block::{SheetFormat, TitleField};
use std::time::Duration;

const HOUSE: &str = include_str!("../../../../examples/programs/tiny-house.star");

fn cut_solids(sheet: &ProjectSheet, view: ProjectView) -> usize {
    sheet
        .views
        .iter()
        .find(|summary| summary.view == view)
        .map_or(0, |summary| summary.cut_solids)
}

/// Adds a visible 1000 x 200 x 2500 mm box named `name` at `x_mm`.
fn add_box(app: &mut KetchupApp, id: u64, name: &str, x_mm: f64) {
    app.document
        .apply_batch(&crate::app::drawing::create_box_batch(
            DefinitionId(id),
            [FeatureId(id * 10), FeatureId(id * 10 + 1)],
            OccurrenceId(id),
            [name, "profile", "extrusion", name],
            Vec3::new(x_mm, 0.0, 0.0),
            Vec3::new(1000.0, 200.0, 2500.0),
        ))
        .unwrap();
}

fn drawn(app: &KetchupApp, name: &str) -> usize {
    app.visible_drawing_solids(&app.document.current())
        .unwrap()
        .iter()
        .filter(|solid| solid.name == name)
        .count()
}

#[test]
fn every_visible_body_is_drawn_and_a_part_without_its_exact_solid_stops_the_sheet() {
    let mut app = KetchupApp::new();
    add_box(&mut app, 901, "wall A", 0.0);
    // A second body in the same part: a 300 x 200 x 3000 mm post.
    let post = crate::app::drawing::create_box_batch(
        DefinitionId(901),
        [FeatureId(9101), FeatureId(9102)],
        OccurrenceId(901),
        ["wall A", "post profile", "post extrusion", "wall A"],
        Vec3::ZERO,
        Vec3::new(300.0, 200.0, 3000.0),
    );
    app.document
        .apply_batch(&CommandBatch::new(
            [
                CanonicalCommand::CreateBody {
                    definition_id: DefinitionId(901),
                    id: BodyId(2),
                    name: "post".to_owned(),
                    visible: true,
                },
                CanonicalCommand::SetActiveBody {
                    definition_id: DefinitionId(901),
                    id: BodyId(2),
                },
            ]
            .into_iter()
            .chain(post.commands()[1..3].iter().cloned())
            .collect(),
        ))
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    assert_eq!(
        drawn(&app, "wall A"),
        2,
        "both bodies of the part are drawn"
    );
    assert!(app.project_drawings().is_ok());

    // A part added after the evaluation has no exact solid yet.
    add_box(&mut app, 902, "wall B", 2000.0);
    let error = app.project_drawings().unwrap_err();
    let ProjectDrawingsError::NotEvaluated(parts) = &error else {
        panic!("the sheet must not leave wall B out: {error:?}");
    };
    assert!(parts.contains(&"wall B".to_owned()), "{parts:?}");
    assert!(app.project_drawings_error_text(&error).contains("wall B"));
    let path = std::env::temp_dir().join(format!(
        "ketchup-drawings-missing-{}.pdf",
        std::process::id()
    ));
    assert!(!app.export_project_drawings_to(&path));
    assert!(!path.exists(), "no sheet is written without wall B");

    crate::drawn_shape::tests::evaluate_exact(&mut app);
    assert_eq!(drawn(&app, "wall B"), 1);
}

/// The plan cut height and the scale of the automatic format are document
/// settings, not constants: changing them moves the cut and the scale.
#[test]
fn the_plan_cut_height_and_the_automatic_scale_follow_the_sheet_settings() {
    let mut app = KetchupApp::new();
    add_box(&mut app, 901, "wall A", 0.0);
    add_box(&mut app, 902, "wall B", 9000.0);
    // The floor the walls stand on: its top at 0.
    app.document
        .apply_batch(&crate::app::drawing::create_box_batch(
            DefinitionId(903),
            [FeatureId(9030), FeatureId(9031)],
            OccurrenceId(903),
            ["slab", "profile", "extrusion", "slab"],
            Vec3::new(0.0, 0.0, -200.0),
            Vec3::new(10_000.0, 3000.0, 200.0),
        ))
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let plan_cut = |sheet: &ProjectSheet| {
        sheet
            .views
            .iter()
            .find(|summary| summary.view == ProjectView::Plan)
            .and_then(|summary| summary.cut_at_mm)
    };
    let default = app.project_drawings().unwrap();
    assert_eq!(plan_cut(&default), Some(1200.0));
    assert_eq!(default.scale, 50);

    let mut settings = app.sheet_settings();
    settings.plan_cut_above_floor_mm = Some(900);
    settings.coarsest_scale = Some(100);
    app.store_sheet_settings(settings);
    let stored = app.stored_sheet_settings().unwrap();
    assert_eq!(
        (stored.plan_cut_above_floor_mm, stored.coarsest_scale),
        (Some(900), Some(100))
    );
    let set = app.project_drawings().unwrap();
    assert_eq!(plan_cut(&set), Some(900.0));
    assert!(set.scale > 50 && set.scale <= 100, "{}", set.scale);
    assert!(set.format <= default.format, "{:?}", set.format);
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_the_house_sheet_has_plan_sections_and_elevations_of_the_visible_layers() {
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker alongside the app tests");
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "tiny-house.star".into(),
            source: HOUSE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app.headless_force_exact_worker_path(&worker);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    step_until(
        &mut harness,
        Duration::from_secs(300),
        "exact house never finished",
        |app| app.exact.task.is_none() && app.exact.source.is_some(),
    );

    let concept = harness.state().project_drawings().unwrap();
    assert_eq!(concept.views.len(), ProjectView::ALL.len());
    let texts = concept.page.texts().collect::<Vec<_>>();
    for key in [
        "drawings-view-longitudinal",
        "drawings-view-cross",
        "drawings-view-front",
        "drawings-default-sheet",
        "title-field-client",
    ] {
        let title = harness.state().catalog.text(key);
        assert!(texts.contains(&title.as_str()), "{key} missing");
    }
    assert!(
        texts.contains(&"tiny-house.star"),
        "the project defaults to the document"
    );
    for view in [
        ProjectView::Plan,
        ProjectView::LongitudinalSection,
        ProjectView::CrossSection,
    ] {
        assert!(cut_solids(&concept, view) > 0, "{view:?} cuts nothing");
    }

    let app = harness.state_mut();
    let tag = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "koncept")
        .map(|tag| tag.id())
        .unwrap();
    assert!(app.set_tag_visibility(tag, false));
    let construction = app.project_drawings().unwrap();
    let (all, without_concept) = (
        cut_solids(&concept, ProjectView::Plan),
        cut_solids(&construction, ProjectView::Plan),
    );
    assert!(
        without_concept > 50 && without_concept < all,
        "hiding the concept layer leaves the studs and layers: {without_concept} of {all}"
    );

    // The title block and format the user sets are kept in the document.
    assert!(!app.drawings.unsaved, "drawing the sheet changes nothing");
    let mut settings = app.sheet_settings();
    settings.format = Some(SheetFormat::A0);
    settings
        .title_block
        .insert(TitleField::Client, "Ján Novák".to_owned());
    app.store_sheet_settings(settings);
    assert!(app.drawings.unsaved && app.is_dirty());
    assert_eq!(
        app.stored_sheet_settings().unwrap().title_block[&TitleField::Client],
        "Ján Novák"
    );

    let path = std::env::temp_dir().join(format!("ketchup-drawings-{}.pdf", std::process::id()));
    assert!(app.export_project_drawings_to(&path));
    let written = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert!(written.starts_with(b"%PDF-"));
    assert!(written.windows(10).any(|window| window == b"/FontFile2"));
    let sheet = app.project_drawings().unwrap();
    assert_eq!(sheet.format, SheetFormat::A0);
    assert!(sheet.page.texts().any(|text| text == "Ján Novák"));
    assert!(app.digest.contains("A0"), "{}", app.digest);
}

/// A plate tapped M8 from its top and an M8 × 40 bolt lying along x.
const BOLTED: &str = "p = box('plate', (60, 40, 20))\n\
    hole(p, 'z+', at = (10, 10), diameter = 6.8, depth = 15, id = 'a', thread = ('M8', 10))\n\
    bolt('s', 'M8', 40, at = (100, 10, 5), axis = (1, 0, 0))\n";

/// Threads reach the sheet as ISO 6410 lines: the tapped hole's root at its
/// major diameter, the bolt's at its minor one, placed in the world; and the
/// sheet waits for the program rather than leaving the bolt's thread out.
#[test]
fn tapped_holes_and_bolts_are_drawn_with_their_threads() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "bolted.star".into(),
        source: BOLTED.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let solids = app.visible_drawing_solids(&app.document.current()).unwrap();
    let threads = |name: &str| {
        solids
            .iter()
            .filter(|solid| solid.name == name)
            .flat_map(|solid| solid.threads.clone())
            .collect::<Vec<_>>()
    };
    let close = |left: [f64; 3], right: [f64; 3]| {
        (0..3).all(|axis| (left[axis] - right[axis]).abs() < 1e-6)
    };

    let [hole] = threads("plate").try_into().unwrap();
    assert!(hole.internal);
    assert!(close(hole.start_mm, [10.0, 10.0, 20.0]) && close(hole.direction, [0.0, 0.0, -1.0]));
    assert!((hole.length_mm - 10.0).abs() < 1e-9);
    assert!((hole.root_diameter_mm - 8.0).abs() < 1e-9);

    let [bolt] = threads("s").try_into().unwrap();
    assert!(!bolt.internal);
    assert!(close(bolt.start_mm, [60.0, 10.0, 5.0]) && close(bolt.direction, [1.0, 0.0, 0.0]));
    assert!((bolt.length_mm - 40.0).abs() < 1e-9);
    assert!((bolt.major_diameter_mm - 8.0).abs() < 1e-9);
    assert!((bolt.root_diameter_mm - (8.0 - 1.226_869 * 1.25)).abs() < 1e-9);

    let sheet = app.project_drawings().unwrap();
    let thin = sheet
        .page
        .marks
        .iter()
        .filter(|mark| {
            matches!(mark, ketchup_manufacturing::sheet_pdf::Mark::Line { stroke, .. }
                if (stroke.width_mm - 0.13).abs() < 1e-9)
        })
        .count();
    assert!(thin > 0, "the sheet draws no thread root lines");

    app.program_evaluations.forget();
    assert!(matches!(
        app.project_drawings(),
        Err(ProjectDrawingsError::ProgramPlanning)
    ));
    app.program_evaluations.get_blocking(&program).unwrap();
    assert!(app.project_drawings().is_ok());
}
