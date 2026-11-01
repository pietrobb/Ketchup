//! Project drawings of the timber-frame house: the set carries the floor plan,
//! both sections and four elevations with a title block on every sheet,
//! follows the visible layers and is written as PDF.
use crate::app::project_drawings::ProjectDrawingsError;
use crate::tests::shell::step_until;
use crate::*;
use ketchup_manufacturing::project_drawings::{DrawingSet, ProjectView, ViewSummary};
use ketchup_manufacturing::sheet_pdf::Mark;
use ketchup_manufacturing::title_block::{SheetFormat, TitleField};
use std::time::Duration;

const HOUSE: &str = include_str!("../../../../examples/programs/tiny-house.star");

/// The views of every sheet of the set.
fn views(set: &DrawingSet) -> impl Iterator<Item = &ViewSummary> {
    set.sheets.iter().flat_map(|sheet| &sheet.views)
}

/// The marks of every sheet of the set.
fn marks(set: &DrawingSet) -> impl Iterator<Item = &Mark> {
    set.sheets.iter().flat_map(|sheet| &sheet.page.marks)
}

/// The texts of every sheet of the set.
fn texts(set: &DrawingSet) -> Vec<&str> {
    set.sheets
        .iter()
        .flat_map(|sheet| sheet.page.texts())
        .collect()
}

fn cut_solids(set: &DrawingSet, view: ProjectView) -> usize {
    views(set)
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

/// Review 2026-10-19 P3: a drawing number too long for its corner cell ran
/// out of the title block. It stops the export, naming the field and what
/// fits in the window's language, and is never cut short.
#[test]
fn a_title_value_too_long_for_its_cell_stops_the_export_and_says_what_fits() {
    let mut app = KetchupApp::with_catalog(LocaleCatalog::slovak());
    add_box(&mut app, 903, "wall", 0.0);
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let mut settings = app.sheet_settings();
    settings
        .title_block
        .insert(TitleField::DrawingNumber, "D-2026-".repeat(40));
    app.store_sheet_settings(settings);
    let error = app.project_drawings().unwrap_err();
    let ProjectDrawingsError::Drawing(
        ketchup_manufacturing::project_drawings::ProjectDrawingError::TitleValueDoesNotFit(refused),
    ) = &error
    else {
        panic!("the drawing number must not be drawn past its cell: {error:?}");
    };
    assert_eq!(refused.field, TitleField::DrawingNumber);
    assert_eq!(refused.lines, 4, "a 13 mm row holds four lines of 1.6 mm");
    assert_eq!(refused.length, 280);
    let text = app.project_drawings_error_text(&error);
    for wanted in [
        app.catalog.text("title-field-drawing_number"),
        format!(
            "na 4 riadkoch sa zmestí {} znakov z 280",
            refused.characters
        ),
    ] {
        assert!(text.contains(&wanted), "{wanted:?} not in {text:?}");
    }
    let path = std::env::temp_dir().join(format!(
        "ketchup-drawings-long-title-{}.pdf",
        std::process::id()
    ));
    assert!(!app.export_project_drawings_to(&path));
    assert!(
        !path.exists(),
        "no sheet is written with the value cut short"
    );
}

/// P3-V: a visible part whose bodies cannot be told apart (two independent
/// extrusions both ending in its one body) stops the sheet, naming the part;
/// it was left off the sheet without a word.
#[test]
fn a_part_whose_bodies_do_not_resolve_stops_the_sheet() {
    let mut app = KetchupApp::new();
    add_box(&mut app, 903, "wall C", 0.0);
    let post = crate::app::drawing::create_box_batch(
        DefinitionId(903),
        [FeatureId(9301), FeatureId(9302)],
        OccurrenceId(903),
        ["wall C", "post profile", "post extrusion", "wall C"],
        Vec3::new(2000.0, 0.0, 0.0),
        Vec3::new(300.0, 200.0, 3000.0),
    );
    app.document
        .apply_batch(&CommandBatch::new(post.commands()[1..3].to_vec()))
        .unwrap();
    let snapshot = app.document.current();
    assert!(
        ketchup_model::exact_product::exact_body_terminal_features(&snapshot, DefinitionId(903))
            .is_err(),
        "the fixture must make the part's bodies unresolvable"
    );
    let error = app.visible_drawing_solids(&snapshot).unwrap_err();
    // Review 2026-10-19 P3 (0c7c426): the cause was dropped and the part
    // reported as one waiting for its exact solid, which no wait would bring.
    let ProjectDrawingsError::UnresolvedBodies(part, cause) = &error else {
        panic!("the sheet must not leave wall C out: {error:?}");
    };
    assert_eq!(part, "wall C");
    assert!(
        matches!(
            cause,
            ketchup_model::exact_product::ExactProductError::ConflictingBodyTerminals { .. }
        ),
        "{cause:?}"
    );
    let text = app.project_drawings_error_text(&error);
    assert!(
        text.contains("wall C") && text.contains(&cause.to_string()),
        "{text}"
    );
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
    let plan_cut = |set: &DrawingSet| {
        views(set)
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
    assert!(
        set.sheets[0].format <= default.sheets[0].format,
        "{:?}",
        set.sheets[0].format
    );
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
    for view in ProjectView::ALL {
        assert!(
            views(&concept).any(|summary| summary.view == view),
            "{view:?} missing"
        );
    }
    let texts = texts(&concept);
    for key in [
        "drawings-view-longitudinal",
        "drawings-view-cross",
        "drawings-view-front",
        "drawings-sheet-sections",
        "drawings-sheet-elevations",
        "title-field-client",
    ] {
        let title = harness.state().catalog.text(key);
        assert!(
            texts.iter().any(|text| text.contains(title.as_str())),
            "{key} missing"
        );
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
    let set = app.project_drawings().unwrap();
    for sheet in &set.sheets {
        assert_eq!(sheet.format, SheetFormat::A0);
        assert!(sheet.page.texts().any(|text| text == "Ján Novák"));
        assert!(sheet.page.texts().any(|text| text == "A0"));
    }
    assert!(
        app.digest.contains(&path.display().to_string()),
        "{}",
        app.digest
    );
}

/// A plate tapped M8 from its top and an M8 × 40 bolt lying along x.
const BOLTED: &str = "p = box('plate', (60, 40, 20))\n\
    hole(p, 'z+', at = (10, 10), diameter = 6.8, depth = 17, id = 'a', thread = ('M8', 10))\n\
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

    let set = app.project_drawings().unwrap();
    let thin = marks(&set)
        .filter(|mark| {
            matches!(mark, Mark::Line { stroke, .. }
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

/// A 4 m wall with a door and two equal windows cut into it.
const WALLED: &str = "wall = box('wall', (4000, 200, 2600))\n\
    door('entry', (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 200, 0, 900, 2100, cut = wall)\n\
    window('win', (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 1400, 900, 1200, 1250, cut = wall)\n\
    window('win 2', (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 2750, 900, 1200, 1250, cut = wall)\n";

/// R9-C step 14: the plan marks each window and door, a mark per type, and
/// the set's last sheet lists every type with the designation and count of
/// its row in the hardware list; each sheet also goes out as a DXF.
#[test]
fn the_plan_marks_each_opening_and_the_schedule_sheet_is_the_hardware_list() {
    use ketchup_manufacturing::project_drawings::SheetKind;
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "walled.star".into(),
        source: WALLED.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let set = app.project_drawings().unwrap();

    let mark = |kind: &str| {
        app.catalog.format(
            &format!("drawings-mark-{kind}"),
            &BTreeMap::from([("number", "1".to_owned())]),
        )
    };
    let (door, window) = (mark("door"), mark("window"));
    assert_eq!(
        set.sheets[0].openings,
        [door.clone(), window.clone(), window.clone()]
    );
    for page in [&set.sheets[0].page, &set.sheets[0].lines] {
        let count = |wanted: &str| page.texts().filter(|text| *text == wanted).count();
        assert_eq!((count(&door), count(&window)), (1, 2));
    }

    let schedule = set.sheets.last().unwrap();
    assert_eq!(schedule.kind, SheetKind::Openings);
    assert_eq!(schedule.number, set.sheets.len());
    let total = format!("{0} / {0}", set.sheets.len());
    let texts = schedule.page.texts().collect::<Vec<_>>();
    assert!(texts.contains(&total.as_str()), "{texts:?}");
    let evaluated = app.program_evaluations.get_blocking(&program).unwrap();
    let hardware = ketchup_program::bom(&evaluated.model).hardware;
    assert_eq!(hardware.len(), 2, "{hardware:?}");
    for row in &hardware {
        let at = texts
            .iter()
            .position(|text| *text == row.item)
            .unwrap_or_else(|| panic!("{} not in the schedule: {texts:?}", row.item));
        assert_eq!(texts[at + 1], row.count.to_string(), "{}", row.item);
    }
    assert_eq!(set.dxf().len(), set.sheets.len());
}

/// A 3 m timber-frame wall panel sheathed with OSB, a window 900 x 1200 in it.
const PANEL: &str = "buildup('wall S', (0, 0, 0), (1, 0, 0), (0, 0, 1), 3000, height = 2500,\n\
    layers = [{'name': 'frame', 'thickness': 140, 'material': 'C24', 'spacing': 625},\n\
              {'name': 'osb', 'thickness': 15, 'material': 'OSB'}],\n\
    openings = [(1000, 800, 900, 1200)])\n";

/// R9-C step 17: with stock for its materials, the set ends (before any
/// schedule) with the cutting plans: every bar and sheet of the report's plan
/// captioned, and every piece on it labelled or listed as too small.
#[test]
fn the_set_carries_a_cutting_plan_of_every_material_with_stock() {
    use ketchup_manufacturing::project_drawings::SheetKind;
    let mut app = KetchupApp::new();
    // The OSB laid from whole boards (capability review run 349): without
    // "sheet" its wall-high columns fit no sheet and were all left unplanned.
    // On the documented stock with its default trim (run 352): the boards as
    // long as the sheet keep its factory edges.
    let panel = PANEL.replace(
        "'material': 'OSB'}",
        "'material': 'OSB', 'sheet': (1250, 2500), 'grain': 'v'}",
    );
    let source = format!(
        "{panel}bar_stock('C24', [3000, 5000])\nsheet_stock('OSB', [(2500, 1250)], grain = True)\n"
    );
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "panel.star".into(),
        source,
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let set = app.project_drawings().unwrap();
    let evaluated = app.program_evaluations.get_blocking(&program).unwrap();
    let plans = ketchup_program::bom(&evaluated.model).cutting;
    assert_eq!(
        plans
            .iter()
            .map(|plan| plan.material.as_str())
            .collect::<Vec<_>>(),
        ["C24", "OSB"],
        "{plans:#?}"
    );
    let osb = &plans[1];
    assert_eq!(osb.unplaced, [], "{osb:#?}");
    let sawn = osb
        .layouts
        .iter()
        .flat_map(|layout| &layout.pieces)
        .map(|piece| piece.size_mm[0] * piece.size_mm[1])
        .sum::<f64>();
    assert!(
        (sawn - (3000.0 * 2500.0 - 900.0 * 1200.0)).abs() < 1e-3,
        "{sawn}"
    );

    let kinds = set
        .sheets
        .iter()
        .map(|sheet| sheet.kind)
        .collect::<Vec<_>>();
    let first = kinds
        .iter()
        .position(|kind| matches!(kind, SheetKind::Cutting { .. }))
        .unwrap_or_else(|| panic!("no cutting plan: {kinds:?}"));
    assert!(
        matches!(kinds[first - 1], SheetKind::Shop { .. }),
        "{kinds:?}"
    );
    let total = format!("{0} / {0}", set.sheets.len());
    for (index, plan) in plans.iter().enumerate() {
        let sheets = set
            .sheets
            .iter()
            .filter(|sheet| matches!(sheet.kind, SheetKind::Cutting { plan, .. } if plan == index))
            .collect::<Vec<_>>();
        assert!(!sheets.is_empty(), "{}", plan.material);
        assert!(
            sheets[0].title.contains(&plan.material),
            "{}",
            sheets[0].title
        );
        let summaries = sheets
            .iter()
            .map(|sheet| sheet.cutting.clone().unwrap())
            .collect::<Vec<_>>();
        let captions = summaries
            .iter()
            .map(|summary| summary.stock.len())
            .sum::<usize>();
        assert_eq!(captions, plan.layouts.len(), "{}", plan.material);
        let pieces = plan
            .layouts
            .iter()
            .map(|layout| layout.pieces.len())
            .sum::<usize>();
        let shown = summaries
            .iter()
            .map(|summary| summary.labels + summary.unlabelled.len())
            .sum::<usize>();
        assert_eq!(shown, pieces, "{}", plan.material);
        // Every piece of the plan is labelled with its part name somewhere.
        let texts = sheets
            .iter()
            .flat_map(|sheet| sheet.page.texts())
            .collect::<Vec<_>>();
        for piece in plan.layouts.iter().flat_map(|layout| &layout.pieces) {
            let named = texts
                .iter()
                .any(|text| text.starts_with(&format!("{} ", piece.part)))
                || summaries.iter().any(|summary| {
                    summary
                        .unlabelled
                        .iter()
                        .any(|label| label.starts_with(&format!("{} ", piece.part)))
                });
            assert!(named, "{} is not on the plan", piece.part);
        }
    }
    let last = set.sheets.last().unwrap();
    assert!(
        last.page.texts().any(|text| text == total),
        "numbered n / N"
    );
    assert_eq!(set.dxf().len(), set.sheets.len());
}

/// R9-C step 15: the set has a shop drawing of the panel. Every piece carries
/// its position number, the running dimensions give the edges of every stud
/// and of the window, and the table counts each position as the program does.
#[test]
fn a_panel_has_a_shop_drawing_dimensioning_its_studs_and_its_window() {
    use ketchup_manufacturing::project_drawings::SheetKind;
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "panel.star".into(),
        source: PANEL.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let set = app.project_drawings().unwrap();
    let evaluated = app.program_evaluations.get_blocking(&program).unwrap();
    let model = &evaluated.model;

    let shops = set
        .sheets
        .iter()
        .filter(|sheet| matches!(sheet.kind, SheetKind::Shop { .. }))
        .collect::<Vec<_>>();
    let [sheet] = shops[..] else {
        panic!(
            "one shop drawing: {:?}",
            set.sheets.iter().map(|s| s.kind).collect::<Vec<_>>()
        )
    };
    assert!(sheet.title.contains("wall S"), "{}", sheet.title);
    // The OSB covers the framing on the side the sheet looks at, so it has a
    // view of its own beside the framing's, which would otherwise be hidden.
    assert_eq!(
        sheet
            .views
            .iter()
            .map(|view| view.title.clone())
            .collect::<Vec<_>>(),
        [sheet.title.clone(), format!("{} - OSB", sheet.title)]
    );
    let shop = sheet.shop.as_ref().unwrap();
    let pieces = model
        .parts
        .iter()
        .filter(|part| part.name.starts_with("wall S/"))
        .collect::<Vec<_>>();
    assert_eq!(shop.marks.len(), pieces.len());
    let edges = |axis: usize, piece: &str| {
        pieces
            .iter()
            .filter(|part| part.name.contains(piece))
            .flat_map(|part| {
                let (min, max) = part.world_bounds();
                [min[axis].round(), max[axis].round()]
            })
            .collect::<Vec<_>>()
    };
    let studs = edges(0, "/stud ");
    assert!(studs.len() >= 10, "{studs:?}");
    for edge in studs.iter().chain(&[1000.0, 1900.0]) {
        assert!(
            shop.across.contains(edge),
            "{edge} not in {:?}",
            shop.across
        );
    }
    for edge in edges(2, "/header ")
        .iter()
        .chain(&edges(2, "/sill "))
        .chain(&[800.0, 2000.0])
    {
        assert!(shop.up.contains(edge), "{edge} not in {:?}", shop.up);
    }

    let positions = ketchup_program::shop_drawing::shop_drawings(model);
    let [drawing] = &positions[..] else {
        panic!("{positions:?}")
    };
    assert_eq!(
        drawing
            .positions
            .iter()
            .map(|position| position.parts.len())
            .sum::<usize>(),
        pieces.len()
    );
    let texts = sheet.page.texts().collect::<Vec<_>>();
    // Each table row: position, material, size, count.
    for (index, position) in drawing.positions.iter().enumerate() {
        let row = [
            position.mirror_of.map_or_else(
                || (index + 1).to_string(),
                |mirrored| format!("{} (mirror of {mirrored})", index + 1),
            ),
            position.material.clone(),
            position
                .dimensions_mm
                .map(|value| format!("{:.0}", value.round()))
                .join(" x "),
            position.parts.len().to_string(),
        ];
        assert!(
            texts.windows(4).any(|cells| *cells == row),
            "{row:?} not in {texts:?}"
        );
    }
    assert_eq!(set.dxf().len(), set.sheets.len());
}

/// An M5 × 10 bolt 7.3 mm into a plate tapped M5 10 mm deep, turned 37°.
const SCREWED: &str = "p = box('plate', (20, 20, 12))\n\
    hole(p, 'z+', at = (10, 10), depth = 12, through = True, id = 'a', thread = ('M5', 10))\n\
    s = bolt('s', 'M5', 10, at = (10, 10, 14.7))\n\
    place(s, origin = (10, 10, 14.7), z = (0, 0, 1), x = (0.7986355100472928, 0.6018150231520483, 0))\n";

/// Corners of `solids` named `name` inside a thread's groove on the axis
/// x = y = 10: between `radii` from it, within `heights` along it.
fn in_groove(
    solids: &[ketchup_manufacturing::project_drawings::DrawingSolid],
    name: &str,
    radii: (f64, f64),
    heights: (f64, f64),
) -> usize {
    solids
        .iter()
        .filter(|solid| solid.name == name)
        .flat_map(|solid| solid.triangles.iter().flatten())
        .filter(|[x, y, z]| {
            let radius = (x - 10.0).hypot(y - 10.0);
            radius > radii.0 && radius < radii.1 && *z > heights.0 && *z < heights.1
        })
        .count()
}

/// The 0.18 mm lines of a set (the solids' edges) and its filled faces.
fn edge_lines_and_faces(set: &DrawingSet) -> [usize; 2] {
    let count = |wanted: fn(&Mark) -> bool| marks(set).filter(|mark| wanted(mark)).count();
    [
        count(
            |mark| matches!(mark, Mark::Line { stroke, .. } if (stroke.width_mm - 0.18).abs() < 1e-9),
        ),
        count(|mark| matches!(mark, Mark::Fill { .. })),
    ]
}

/// A modelled thread is drawn by ISO 6410 exactly as a cosmetic one, not with
/// the edges of its helix: the sheet of the screwed bolt is the same in both
/// modes, while the modelled bodies' own triangles carry the grooves.
#[test]
fn modelled_threads_draw_the_same_sheet_as_cosmetic_ones_without_helix_edges() {
    use ketchup_manufacturing::project_drawings::{
        DrawingSetOptions, PlanSheet, drawing_set, storeys,
    };
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "screwed.star".into(),
        source: SCREWED.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    app.program_evaluations.get_blocking(&program).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let cosmetic = app.project_drawings().unwrap();
    let cosmetic_solids = app.visible_drawing_solids(&app.document.current()).unwrap();

    assert!(app.set_modeled_threads(true));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let snapshot = app.document.current();
    assert!(snapshot.modeled_threads());
    let modeled = app.project_drawings().unwrap();
    let drawn = app.visible_drawing_solids(&snapshot).unwrap();
    // The bodies as the exact kernel modelled them, grooves and all.
    let packages = app.exact.results.body_values(&snapshot).unwrap();
    let grooved = drawn
        .iter()
        .cloned()
        .map(|mut solid| {
            let occurrence = snapshot
                .scene_query()
                .into_iter()
                .find(|occurrence| occurrence.occurrence_name == solid.name)
                .unwrap();
            let (_, package) = packages
                .iter()
                .find(|(key, _)| key.definition_id == occurrence.definition_id)
                .unwrap();
            let vertices = package.vertices();
            solid.triangles = package
                .triangles()
                .iter()
                .map(|triangle| {
                    triangle.vertex_indices.map(|index| {
                        occurrence
                            .transform
                            .transform_point(vertices[index as usize].position_mm)
                    })
                })
                .collect();
            solid.faces = (0..package.triangles().len())
                .map(|index| package.triangle_face_ordinal(index).unwrap())
                .collect();
            solid
        })
        .collect::<Vec<_>>();

    // M5 × 0.8: the bolt's root at 2.01 mm from the axis, its crest at 2.5;
    // the hole's bore at 2.1 (4.2 drill), its 6H root at 2.5 or a little
    // past. The thread runs down the bolt's shank from z = 14.7 and down the
    // hole from z = 12.
    let bolt = ((2.05, 2.45), (5.2, 14.2));
    let hole = ((2.12, 2.6), (2.5, 11.5));
    for (solids, grooves) in [(&cosmetic_solids, false), (&drawn, false), (&grooved, true)] {
        for (part, (radii, heights)) in [("s", bolt), ("plate", hole)] {
            assert_eq!(
                in_groove(solids, part, radii, heights) > 0,
                grooves,
                "{part}: grooves {grooves}"
            );
        }
    }
    assert!(
        cosmetic == modeled,
        "the modelled sheet differs: {} vs {} marks, {:?} vs {:?} edge lines and faces",
        marks(&cosmetic).count(),
        marks(&modeled).count(),
        edge_lines_and_faces(&cosmetic),
        edge_lines_and_faces(&modeled)
    );
    let info = ketchup_manufacturing::sheet_pdf::PdfInfo::default();
    assert!(cosmetic.pdf(&info) == modeled.pdf(&info), "the PDFs differ");
    // Drawn from the grooved bodies, the sheet would carry the helix: its
    // edges and the facets of its flanks.
    let options = DrawingSetOptions {
        plans: storeys(&drawn, 1200.0)
            .into_iter()
            .map(|storey| PlanSheet {
                storey,
                view_title: String::new(),
                sheet_title: String::new(),
            })
            .collect(),
        ..DrawingSetOptions::default()
    };
    let [plain_edges, plain_faces] = edge_lines_and_faces(&drawing_set(&drawn, &options).unwrap());
    let [helix_edges, helix_faces] =
        edge_lines_and_faces(&drawing_set(&grooved, &options).unwrap());
    assert!(
        helix_edges > plain_edges && helix_faces > 2 * plain_faces,
        "edges {helix_edges} vs {plain_edges}, faces {helix_faces} vs {plain_faces}"
    );
}

/// Evaluating modelled threads makes only the grooved bodies: their cosmetic
/// form is made when a sheet is drawn, so without an exact worker then the
/// sheet is refused (naming the part), and with one it draws.
#[test]
fn modelled_threads_are_made_cosmetic_only_when_a_sheet_is_drawn() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "screwed.star".into(),
        source: SCREWED.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    app.program_evaluations.get_blocking(&program).unwrap();
    assert!(app.set_modeled_threads(true));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let snapshot = app.document.current();
    // Each evaluated body is the grooved one and nothing more.
    let drawn = app.visible_drawing_solids(&snapshot).unwrap();
    let mut grooved_bodies = 0;
    for (_, package) in app.exact.results.body_values(&snapshot).unwrap() {
        let ketchup_model::exact_product::ExactBodyPackage::Graph(package) = &**package else {
            panic!("a program part is a graph body");
        };
        if package.graph.without_thread_grooves().unwrap().is_some() {
            grooved_bodies += 1;
            assert!(
                drawn
                    .iter()
                    .all(|solid| solid.triangles.len() != package.triangles.len()),
                "a sheet draws the cosmetic body, not the evaluated one"
            );
        }
    }
    assert_eq!(grooved_bodies, 2);

    let worker = app.exact.worker_path.clone();
    app.exact.worker_path = Some(std::env::temp_dir().join("no-such-exact-worker.exe"));
    let Err(ProjectDrawingsError::CosmeticThreads(part, _)) = app.project_drawings() else {
        panic!("a sheet of modelled threads needs the exact worker");
    };
    assert!(part == "plate" || part == "s", "{part}");
    app.exact.worker_path = worker;
    assert!(app.project_drawings().is_ok());
}

/// Review 2026-10-18 UI-12: editing one field of the title block in the
/// window kept the project name, the sheet title in the window's language and
/// today's date in the document too, so every later sheet carried them; and
/// a default that changed (another language, Save As, midnight) was kept as
/// an edit. Only what the user wrote is kept; defaults are filled in when a
/// sheet is drawn, and a field still showing one follows it.
#[test]
fn the_title_block_keeps_only_what_the_user_wrote_and_defaults_follow_the_document() {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            KetchupApp::new(),
        );
    harness.state_mut().open_project_drawings_window();
    harness.step();
    assert!(!harness.state().drawings.unsaved, "opening changes nothing");

    harness
        .state_mut()
        .drawings
        .editing
        .title_block
        .insert(TitleField::Author, "Peter".to_owned());
    harness.step();
    let stored = |app: &KetchupApp| app.stored_sheet_settings().unwrap().title_block;
    assert_eq!(
        stored(harness.state()),
        BTreeMap::from([(TitleField::Author, "Peter".to_owned())])
    );
    assert!(harness.state().drawings.unsaved);

    harness
        .state_mut()
        .set_ui_language(crate::language::UiLanguage::Slovak);
    harness.state_mut().file.path = Some(PathBuf::from("Rodinný dom.ketchup"));
    harness.step();
    let app = harness.state();
    assert_eq!(
        stored(app),
        BTreeMap::from([(TitleField::Author, "Peter".to_owned())])
    );
    let shown = &app.drawings.editing.title_block;
    assert_eq!(shown[&TitleField::Project], "Rodinný dom");
    assert_eq!(
        shown[&TitleField::Drawing],
        app.catalog.text("drawings-default-sheet")
    );
    let drawn = app.sheet_settings().title_block;
    assert_eq!(drawn[&TitleField::Project], "Rodinný dom");
    assert_eq!(drawn[&TitleField::Author], "Peter");

    // A field the user wrote over its default is kept.
    harness
        .state_mut()
        .drawings
        .editing
        .title_block
        .insert(TitleField::Project, "Dom Novák".to_owned());
    harness.step();
    assert_eq!(stored(harness.state())[&TitleField::Project], "Dom Novák");
}

/// Review 2026-10-19 P3: the open window read the document's sheet settings
/// in every frame and wrote its own fields back whenever they differed, so
/// settings the AI wrote, or those of a document opened meanwhile, were
/// replaced by the window's in the next frame. The window takes up what
/// changed in the document and writes only what the user changes in it.
#[test]
fn the_open_drawings_window_takes_up_settings_changed_in_the_document() {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            KetchupApp::new(),
        );
    harness.state_mut().open_project_drawings_window();
    harness.step();
    let stored = |app: &KetchupApp| app.stored_sheet_settings().unwrap().title_block;

    // The AI writes a title block field while the window is open.
    let mut settings = harness.state().stored_sheet_settings().unwrap();
    settings
        .title_block
        .insert(TitleField::Author, "AI".to_owned());
    harness.state_mut().store_sheet_settings(settings);
    harness.step();
    assert_eq!(stored(harness.state())[&TitleField::Author], "AI");
    assert_eq!(
        harness.state().drawings.editing.title_block[&TitleField::Author],
        "AI"
    );

    // Another document's settings come in with its container.
    let mut container = ketchup_model::persistence::ContainerData::default();
    container.set_extension(
        ketchup_model::persistence::ExtensionEntry::new(
            "org.ketchup.drawings",
            "sheet-v1.json",
            false,
            br#"{"title_block": {"client": "Novak"}}"#.to_vec(),
        )
        .unwrap(),
    );
    harness.state_mut().file.container_data = container;
    harness.step();
    assert_eq!(
        stored(harness.state()),
        BTreeMap::from([(TitleField::Client, "Novak".to_owned())])
    );
    let shown = &harness.state().drawings.editing.title_block;
    assert_eq!(shown[&TitleField::Client], "Novak");
    assert!(shown.get(&TitleField::Author).is_none_or(String::is_empty));

    // What the user then writes in the window is kept with it.
    harness
        .state_mut()
        .drawings
        .editing
        .title_block
        .insert(TitleField::Author, "Peter".to_owned());
    harness.step();
    assert_eq!(
        stored(harness.state()),
        BTreeMap::from([
            (TitleField::Client, "Novak".to_owned()),
            (TitleField::Author, "Peter".to_owned()),
        ])
    );
}

/// The title block date is the user's: 23:30 UTC on 22 October is already 23 October
/// in Bratislava (+02:00), and still 22 October in New York (-04:00).
#[test]
fn the_drawing_date_is_the_date_on_the_users_clock() {
    use crate::app::project_drawings::date_at;
    // 2026-10-22 23:30:00 UTC.
    let late_evening = 1_792_711_800;
    assert_eq!(date_at(late_evening, 0), "22. 10. 2026");
    assert_eq!(date_at(late_evening, 2 * 3_600), "23. 10. 2026");
    assert_eq!(date_at(late_evening, -4 * 3_600), "22. 10. 2026");
    // A minute after midnight UTC on 1 January is still New Year's Eve west of it.
    assert_eq!(date_at(1_798_761_660, -3_600), "31. 12. 2026");
}

/// The project the sets of the MF-27 tests are drawn from.
const DOCUMENT: ketchup_model::document::DocumentId = ketchup_model::document::DocumentId(7);

/// Review 2026-10-24 MF-27: a set's DXF sheets were written only as new
/// files, so exporting the set again under the same name always failed.
#[test]
fn a_set_exported_again_under_its_name_replaces_the_sheets_it_wrote() {
    use crate::app::project_drawings::{
        SetFileFormat, project_drawings_targets, publish_project_drawings,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    let sheet = |number: u8| directory.path().join(format!("plan-0{number}.dxf"));
    let read = |number| std::fs::read(sheet(number)).unwrap();
    let publish = |files: [&[u8]; 2]| {
        let targets = project_drawings_targets(&path, SetFileFormat::Dxf, DOCUMENT).unwrap();
        let files = files.map(<[u8]>::to_vec);
        publish_project_drawings(&path, SetFileFormat::Dxf, &files, &targets, DOCUMENT)
            .map(|_| targets)
    };
    // Files named like no sheet of this set are not taken for one.
    for other in ["plan-1.dxf", "plan-ab.dxf", "planx-01.dxf", "plan-01.pdf"] {
        std::fs::write(directory.path().join(other), b"other").unwrap();
    }
    assert!(publish([b"first 1", b"first 2"]).unwrap().is_empty());

    let replaced = publish([b"second 1", b"second 2"]).unwrap();
    assert_eq!(
        replaced.keys().cloned().collect::<Vec<_>>(),
        [
            sheet(1),
            sheet(2),
            directory.path().join("plan.dxf.sheets.txt")
        ]
    );
    assert_eq!(
        (read(1), read(2)),
        (b"second 1".to_vec(), b"second 2".to_vec())
    );

    // A sheet saved meanwhile is kept, and the one the export had already
    // replaced gets its bytes back: the set is written whole or not at all.
    let targets = project_drawings_targets(&path, SetFileFormat::Dxf, DOCUMENT).unwrap();
    std::fs::write(sheet(2), b"saved meanwhile").unwrap();
    assert!(
        publish_project_drawings(
            &path,
            SetFileFormat::Dxf,
            &[b"third 1".to_vec(), b"third 2".to_vec()],
            &targets,
            DOCUMENT,
        )
        .is_err()
    );
    assert_eq!(
        (read(1), read(2)),
        (b"second 1".to_vec(), b"saved meanwhile".to_vec())
    );
}

/// A set exported again with fewer sheets leaves none of the old sheets beyond
/// its last beside it, unless one was saved meanwhile: then the export is
/// refused and the set stays as it was (review 2026-10-24 MF-27, run 323).
#[test]
fn a_set_exported_again_with_fewer_sheets_removes_the_sheets_beyond_its_last() {
    use crate::app::project_drawings::{
        SetFileFormat, project_drawings_targets, publish_project_drawings,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    let sheet = |number: u8| directory.path().join(format!("plan-0{number}.dxf"));
    let publish = |files: &[&[u8]]| {
        let targets = project_drawings_targets(&path, SetFileFormat::Dxf, DOCUMENT).unwrap();
        let files: Vec<Vec<u8>> = files.iter().map(|file| file.to_vec()).collect();
        publish_project_drawings(&path, SetFileFormat::Dxf, &files, &targets, DOCUMENT)
    };
    publish(&[b"a 1", b"a 2", b"a 3", b"a 4"]).unwrap();

    let paths = publish(&[b"b 1", b"b 2", b"b 3"]).unwrap();
    assert_eq!(paths, [sheet(1), sheet(2), sheet(3)]);
    assert!(!sheet(4).exists());

    let targets = project_drawings_targets(&path, SetFileFormat::Dxf, DOCUMENT).unwrap();
    std::fs::write(sheet(3), b"saved meanwhile").unwrap();
    let error = publish_project_drawings(
        &path,
        SetFileFormat::Dxf,
        &[b"c 1".to_vec(), b"c 2".to_vec()],
        &targets,
        DOCUMENT,
    )
    .unwrap_err();
    let crate::app::project_drawings::ProjectDrawingsError::Write(error) = error else {
        panic!("{error:?}");
    };
    assert!(error.to_string().contains("plan-03.dxf"), "{error}");
    let read = |number| std::fs::read(sheet(number)).unwrap();
    assert_eq!(
        (read(1), read(2), read(3)),
        (
            b"b 1".to_vec(),
            b"b 2".to_vec(),
            b"saved meanwhile".to_vec()
        )
    );
}

/// Review 2026-10-24 MF-27, run 325: a file named like a sheet of the set is
/// not taken for one by its name. Another project's sheets under the same name,
/// as two unsaved projects both suggest `Untitled`, sheets with no list, and a
/// sheet edited since the set listed it are each left as they are, and the
/// export is refused instead of replacing or removing them.
#[test]
fn sheets_of_another_project_under_the_same_name_are_neither_replaced_nor_removed() {
    use crate::app::project_drawings::{
        ProjectDrawingsError, SetFileFormat, project_drawings_targets, publish_project_drawings,
    };
    use ketchup_model::document::DocumentId;
    // Two new projects are two projects, though both are called Untitled.
    assert_ne!(
        KetchupApp::new().document.current().document_id(),
        KetchupApp::new().document.current().document_id()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Untitled.dxf");
    let sheet = |number: u8| directory.path().join(format!("Untitled-0{number}.dxf"));
    let list = directory.path().join("Untitled.dxf.sheets.txt");
    let publish = |document: DocumentId, files: &[&[u8]]| {
        let targets = project_drawings_targets(&path, SetFileFormat::Dxf, document).unwrap();
        let files: Vec<Vec<u8>> = files.iter().map(|file| file.to_vec()).collect();
        publish_project_drawings(&path, SetFileFormat::Dxf, &files, &targets, document)
    };
    let other = DocumentId(8);
    publish(other, &[b"A1", b"A2", b"A3", b"A4", b"A5"]).unwrap();
    let before = |count: u8| {
        (1..=count)
            .map(|number| std::fs::read(sheet(number)).unwrap())
            .collect::<Vec<_>>()
    };
    let other_sheets = before(5);
    let other_list = std::fs::read(&list).unwrap();

    let refused = |files: &[&[u8]]| {
        let Err(ProjectDrawingsError::Write(_)) = publish(DOCUMENT, files) else {
            panic!("another project's sheets were taken for this set's");
        };
    };
    refused(&[b"B1", b"B2"]);
    assert_eq!(before(5), other_sheets);
    assert_eq!(std::fs::read(&list).unwrap(), other_list);

    // Sheets with no list are no set's: they are kept too.
    std::fs::remove_file(&list).unwrap();
    refused(&[b"B1", b"B2"]);
    assert_eq!(before(5), other_sheets);
    assert!(!list.exists());

    // This project's own set is replaced, but not a sheet edited since.
    for number in 1..=5 {
        std::fs::remove_file(sheet(number)).unwrap();
    }
    publish(DOCUMENT, &[b"B1", b"B2", b"B3"]).unwrap();
    std::fs::write(sheet(3), b"edited").unwrap();
    refused(&[b"C1", b"C2"]);
    assert_eq!(
        before(3),
        [b"B1".to_vec(), b"B2".to_vec(), b"edited".to_vec()]
    );
    std::fs::write(sheet(3), b"B3").unwrap();
    assert_eq!(
        publish(DOCUMENT, &[b"C1", b"C2"]).unwrap(),
        [sheet(1), sheet(2)]
    );
    assert!(!sheet(3).exists());
}
