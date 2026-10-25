//! window() and door(): one definition gives the wall opening of every build-up
//! in that wall, the cut in a plain wall body and the frame, pane or leaf.
use ketchup_program::model::ProgramModel;
use ketchup_program::{Issue, Severity, run};
use std::collections::BTreeMap;

/// A wall facing -y, inside face at y = 200, 200 mm thick, with a window and a door.
const WALL: &str = "
wall = box(\"wall\", (4000, 200, 2600), material = \"timber frame\")
win = window(\"win\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 2200, 900, 1200, 1250, cut = wall)
entry = door(\"entry\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 600, 0, 900, 2100, cut = wall)
";

const LAYERS: &str = "
LAYERS = [
    {\"name\": \"gypsum\", \"thickness\": 12.5, \"material\": \"gypsum board\"},
    {\"name\": \"frame\", \"thickness\": 140, \"material\": \"C24\", \"spacing\": 625, \"infill\": \"mineral wool\"},
    {\"name\": \"osb\", \"thickness\": 15, \"material\": \"OSB\"},
]
";

fn evaluate(source: &str) -> (ProgramModel, Vec<Issue>) {
    let (evaluation, report) =
        run("openings.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    (evaluation.model, report.issues)
}

fn errors(issues: &[Issue]) -> Vec<String> {
    issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .map(|issue| format!("{}: {}", issue.kind, issue.message))
        .collect()
}

fn bounds(model: &ProgramModel, name: &str) -> [[f64; 3]; 2] {
    let (min, max) = model
        .part(name)
        .unwrap_or_else(|| panic!("no part {name}"))
        .world_bounds();
    [min, max].map(|corner| corner.map(|value| (value * 1000.0).round() / 1000.0))
}

/// Every part of the build-up `prefix` with its world bounds.
fn pieces(model: &ProgramModel, prefix: &str) -> Vec<(String, [[f64; 3]; 2])> {
    model
        .parts
        .iter()
        .filter(|part| part.name.starts_with(prefix))
        .map(|part| {
            (
                part.name[prefix.len()..].to_owned(),
                bounds(model, &part.name),
            )
        })
        .collect()
}

#[test]
fn a_window_and_a_door_are_framed_and_cut_into_the_wall() {
    let (model, issues) = evaluate(WALL);
    assert_eq!(errors(&issues), Vec::<String>::new());
    // The frame (80 x 78) is in the middle of the wall: y = 139 .. 61.
    assert_eq!(
        bounds(&model, "win/frame bottom"),
        [[2200.0, 61.0, 900.0], [3400.0, 139.0, 980.0]]
    );
    assert_eq!(
        bounds(&model, "win/frame top"),
        [[2200.0, 61.0, 2070.0], [3400.0, 139.0, 2150.0]]
    );
    assert_eq!(
        bounds(&model, "win/frame left"),
        [[2200.0, 61.0, 980.0], [2280.0, 139.0, 2070.0]]
    );
    assert_eq!(
        bounds(&model, "win/frame right"),
        [[3320.0, 61.0, 980.0], [3400.0, 139.0, 2070.0]]
    );
    // A 24 mm pane in the middle of the frame.
    assert_eq!(
        bounds(&model, "win/glass"),
        [[2280.0, 88.0, 980.0], [3320.0, 112.0, 2070.0]]
    );
    // A door: threshold instead of the bottom piece, a 40 mm leaf.
    assert_eq!(
        bounds(&model, "entry/threshold"),
        [[600.0, 61.0, 0.0], [1500.0, 139.0, 20.0]]
    );
    assert_eq!(
        bounds(&model, "entry/frame head"),
        [[600.0, 61.0, 2020.0], [1500.0, 139.0, 2100.0]]
    );
    assert_eq!(
        bounds(&model, "entry/frame left"),
        [[600.0, 61.0, 20.0], [680.0, 139.0, 2020.0]]
    );
    assert_eq!(
        bounds(&model, "entry/leaf"),
        [[680.0, 80.0, 20.0], [1420.0, 120.0, 2020.0]]
    );
    let glass = model.part("win/glass").unwrap();
    assert!(glass.tags.contains("window"), "{:?}", glass.tags);
    assert_eq!(glass.material.as_deref(), Some("insulating glass"));
    let leaf = model.part("entry/leaf").unwrap();
    assert!(
        leaf.tags.contains("door") && !leaf.tags.contains("window"),
        "{:?}",
        leaf.tags
    );
    // Each opening is cut into the wall once, under its own name; the tool is no part.
    let wall = model.part("wall").unwrap();
    let cuts: Vec<_> = wall
        .operations
        .iter()
        .map(|operation| operation.name())
        .collect();
    assert_eq!(cuts, ["win", "entry"]);
    assert!(model.part("win/opening").is_none());
    assert_eq!(
        model.parts.len(),
        11,
        "{:?}",
        model
            .parts
            .iter()
            .map(|part| &part.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_build_up_cuts_exactly_the_opening_a_window_defines() {
    let numbers = format!(
        "{LAYERS}
buildup(\"wall\", (100, 200, 0), (1, 0, 0), (0, 0, 1), 3800, LAYERS, height = 2600,
        openings = [(500, 0, 900, 2100), (2100, 900, 1200, 1250)])"
    );
    let openings = "
win = window(\"win\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 2200, 900, 1200, 1250)
entry = door(\"entry\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 600, 0, 900, 2100)
";
    let objects = format!(
        "{LAYERS}{openings}
buildup(\"wall\", (100, 200, 0), (1, 0, 0), (0, 0, 1), 3800, LAYERS, height = 2600, openings = [entry, win])
# Another wall facing +y and the storey above leave both out.
buildup(\"north\", (3900, 3800, 0), (-1, 0, 0), (0, 0, 1), 3800, LAYERS, height = 2600, openings = [win, entry])
buildup(\"attic\", (100, 200, 2600), (1, 0, 0), (0, 0, 1), 3800, LAYERS, height = 1200, openings = [win, entry])"
    );
    // The same wall built from its other end: u runs the other way.
    let mirrored = format!(
        "{LAYERS}{openings}
buildup(\"back\", (3900, 32.5, 0), (-1, 0, 0), (0, 0, 1), 3800, LAYERS, height = 2600, openings = [win, entry])"
    );
    let (by_numbers, _) = evaluate(&numbers);
    let (by_objects, issues) = evaluate(&objects);
    let (by_mirror, _) = evaluate(&mirrored);
    let expected = pieces(&by_numbers, "wall/");
    assert!(expected.len() > 20, "{expected:?}");
    assert_eq!(pieces(&by_objects, "wall/"), expected);
    // Mirrored along u, the back wall has the same openings: the same headers and sills.
    let span = |model: &ProgramModel, prefix: &str| {
        let mut found: Vec<_> = pieces(model, prefix)
            .into_iter()
            .filter(|(name, _)| name.contains("header") || name.contains("sill"))
            .map(|(_, [min, max])| [min[0], max[0], min[2], max[2]])
            .collect();
        found.sort_by(|a, b| a.partial_cmp(b).unwrap());
        found
    };
    assert_eq!(span(&by_mirror, "back/"), span(&by_numbers, "wall/"));
    for whole in ["north/", "attic/"] {
        assert!(
            pieces(&by_objects, whole)
                .iter()
                .all(|(name, _)| !name.contains("header") && !name.contains("sill")),
            "{whole} must have no opening: {:?}",
            pieces(&by_objects, whole)
        );
    }
    assert_eq!(errors(&issues), Vec::<String>::new());
}

#[test]
fn no_window_over_finds_a_window_without_naming_its_tag() {
    let source = format!(
        "{WALL}
bed = box(\"bed\", (2000, 900, 450), at = (2000, 200, 0), material = \"wood\")
no_window_over(bed)"
    );
    let (_, issues) = evaluate(&source);
    let mut found: Vec<_> = issues
        .iter()
        .filter(|issue| issue.kind == "free_space_occupied")
        .map(|issue| issue.parts[0].clone())
        .collect();
    found.sort();
    // Up to 1500 mm above the bed (z = 1950): the top frame piece is higher.
    assert_eq!(
        found,
        [
            "win/frame bottom",
            "win/frame left",
            "win/frame right",
            "win/glass"
        ]
    );
}

#[test]
fn each_window_and_door_is_one_bought_item_in_the_hardware_list() {
    let source = format!(
        "{WALL}
# The same window in the storey above, and a small one named in the program's words.
window(\"win 2\", (0, 200, 2600), (1, 0, 0), (0, 0, 1), 200, 2200, 900, 1200, 1250)
window(\"stair\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 1200, 400, 800, cut = wall, designation = \"okno {{width}} × {{height}}\")"
    );
    let (model, issues) = evaluate(&source);
    assert_eq!(errors(&issues), Vec::<String>::new());
    let bom = ketchup_program::bom(&model);
    let rows: Vec<_> = bom
        .hardware
        .iter()
        .map(|row| (row.item.as_str(), row.count, row.parts.len()))
        .collect();
    // Five pieces each, every opening counted once; equal sizes share a row.
    assert_eq!(
        rows,
        [
            ("door 900 x 2100", 1, 5),
            ("okno 400 × 800", 1, 5),
            ("window 1200 x 1250", 2, 10),
        ]
    );
    assert!(bom.hardware.iter().all(|row| row.attributes.is_empty()));
    // Bought, so none of their parts is made: only the wall is in the cut list.
    let made: Vec<_> = bom
        .cut_list
        .iter()
        .flat_map(|row| row.parts.iter())
        .collect();
    assert_eq!(made, ["wall"]);
}

/// PG-21 (review 2026-10-24): the mirrored copies of a window's five pieces kept
/// its `bought_item`, so two windows were ordered as one.
#[test]
fn a_copied_or_mirrored_window_is_one_more_bought_item() {
    let (model, _) = evaluate(WALL);
    let pieces: Vec<String> = model
        .parts
        .iter()
        .filter(|part| part.name.starts_with("win/"))
        .map(|part| format!("{:?}", part.name))
        .collect();
    assert_eq!(pieces.len(), 5, "{pieces:?}");
    let pieces = pieces.join(", ");
    let source = format!(
        "{WALL}
for piece in [{pieces}]:
    mirrored(piece, piece + \" mirrored\", (6000, 0, 0), (1, 0, 0))
    mirrored(piece, \"east/\" + piece, (8000, 0, 0), (1, 0, 0))
component(\"window\", [{pieces}])
instance(\"upper\", \"window\", at = (0, 0, 3000))
"
    );
    let (model, issues) = evaluate(&source);
    assert_eq!(errors(&issues), Vec::<String>::new());
    let windows = ketchup_program::bom(&model)
        .hardware
        .into_iter()
        .find(|row| row.item == "window 1200 x 1250")
        .expect("the window row");
    assert_eq!((windows.count, windows.parts.len()), (4, 20));
}

#[test]
fn a_door_keeps_the_space_its_leaf_swings_through_clear() {
    // The leaf is 740 wide (x 680..1420); inside the room is y > 200, outside y < 0.
    let things = "
box(\"cupboard\", (400, 300, 1800), at = (800, 400, 0))
box(\"bench\", (400, 300, 450), at = (800, -600, 0))
box(\"shelf\", (400, 300, 1800), at = (1500, 300, 0))
";
    let occupied = |swing: &str| {
        let source = format!(
            "wall = box(\"wall\", (4000, 200, 2600))
door(\"entry\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 600, 0, 900, 2100, cut = wall, swing = \"{swing}\")
{things}"
        );
        let (_, issues) = evaluate(&source);
        let found: Vec<_> = issues
            .iter()
            .filter(|issue| issue.kind == "free_space_occupied")
            .map(|issue| {
                assert_eq!(issue.severity, Severity::Error);
                assert!(issue.hint.contains("hang the door"), "{}", issue.hint);
                issue.message.clone()
            })
            .collect();
        assert!(
            issues
                .iter()
                .all(|issue| issue.kind != "free_space_unverified"),
            "{issues:?}"
        );
        found
    };
    assert_eq!(
        occupied("in"),
        ["cupboard reaches into the free space entry/swing space (400 x 300 x 1800 mm)"]
    );
    assert_eq!(
        occupied("out"),
        ["bench reaches into the free space entry/swing space (400 x 300 x 450 mm)"]
    );
}

#[test]
fn bad_windows_and_doors_name_the_problem() {
    let wall = "wall = box(\"wall\", (4000, 200, 2600))\n";
    for (call, message) in [
        (
            "window(\"w\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 900, 150, 1200)",
            "leaves no room inside its 80 mm frame",
        ),
        (
            "door(\"d\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 0, 900, 100)",
            "leaves no room inside its 80 mm frame and 20 mm threshold",
        ),
        (
            "door(\"d\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 300, 900, 2100)",
            "stands on its floor at v = 0",
        ),
        (
            "window(\"w\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 60, 100, 900, 900, 1200)",
            "does not fit in the 60 mm wall",
        ),
        (
            "window(\"w\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 3500, 900, 900, 1200, cut = wall)",
            "reaches outside the wall wall",
        ),
        (
            "window(\"w\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 2000, 900, 1200, cut = wall)",
            "reaches outside the wall wall",
        ),
        (
            "window(\"w\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 900, 900, 1200, glass = 90)",
            "the pane must be thinner than the 78 mm frame",
        ),
        (
            "door(\"d\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 0, 900, 2100, designation = \" \")",
            "designation must be a text",
        ),
        (
            "door(\"d\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 0, 900, 2100, swing = \"left\")",
            "swing must be \"in\" or \"out\"",
        ),
        (
            "buildup(\"floor\", (0, 0, 0), (1, 0, 0), (0, 1, 0), 4000, [{\"name\": \"a\", \"thickness\": 20}], height = 3000,
                     openings = [window(\"w\", (0, 200, 0), (1, 0, 0), (0, 0, 1), 200, 100, 900, 900, 1200)])",
            "window w does not lie in the plane of the panel",
        ),
    ] {
        let source = format!("{wall}{call}");
        let error = run("bad.star", &source, &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains(message), "{call}: {error}");
    }
}
