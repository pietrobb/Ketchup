use ketchup_program::model::ProgramModel;
use ketchup_program::{Severity, run, validate};
use std::collections::BTreeMap;

const WALL: &str = "
LAYERS = [
    {\"name\": \"gypsum\", \"thickness\": 12.5, \"material\": \"gypsum board\", \"tags\": [\"gypsum\"]},
    {\"name\": \"frame\", \"thickness\": 140, \"material\": \"C24\", \"spacing\": 625, \"tags\": [\"frame\"],
     \"infill\": \"mineral wool\", \"infill_tags\": [\"insulation\"]},
    {\"name\": \"osb\", \"thickness\": 15, \"material\": \"OSB\", \"tags\": [\"osb\"]},
]
L = param(\"length\", 4000)
wall = buildup(\"wall\", (0, 167.5, 0), (1, 0, 0), (0, 0, 1), L, LAYERS, height = 2600,
               openings = [(600, 0, 900, 2100), (2000, 800, 1200, 1300)], tags = [\"construction\"])
";

fn eval(source: &str, overrides: &[(&str, f64)]) -> ProgramModel {
    let overrides = overrides
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect::<BTreeMap<_, _>>();
    run("buildup.star", source, &overrides)
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model
}

fn errors(model: &ProgramModel) -> Vec<String> {
    validate(model)
        .into_iter()
        .filter(|issue| issue.severity == Severity::Error)
        .map(|issue| issue.message)
        .collect()
}

fn named<'a>(model: &'a ProgramModel, prefix: &str) -> Vec<&'a str> {
    model
        .parts
        .iter()
        .map(|part| part.name.as_str())
        .filter(|name| name.starts_with(prefix))
        .collect()
}

#[test]
fn a_framed_wall_stacks_its_layers_around_the_openings_without_overlaps() {
    let model = eval(WALL, &[]);
    assert_eq!(errors(&model), Vec::<String>::new());
    let studs = named(&model, "wall/frame/stud");
    assert!(studs.len() >= 8, "{studs:?}");
    assert_eq!(named(&model, "wall/frame/header").len(), 2);
    assert_eq!(
        named(&model, "wall/frame/sill").len(),
        1,
        "a door has no sill"
    );
    assert_eq!(
        named(&model, "wall/frame/bottom plate").len(),
        2,
        "the sole plate stops at the door"
    );
    assert!(!named(&model, "wall/frame/infill").is_empty());
    let osb = model.part("wall/osb 1").unwrap();
    assert!(osb.tags.contains("construction") && osb.tags.contains("osb"));
    let wool = model.part("wall/frame/infill 1").unwrap();
    assert_eq!(wool.material.as_deref(), Some("mineral wool"));
    assert!(wool.tags.contains("insulation") && !wool.tags.contains("frame"));
    // The layers stack outward along along x up = -y from the inside face.
    let (min_y, max_y) = model
        .parts
        .iter()
        .flat_map(|part| {
            let (min, max) = part.world_bounds();
            [min[1], max[1]]
        })
        .fold((f64::MAX, f64::MIN), |(low, high), y| {
            (low.min(y), high.max(y))
        });
    assert!(
        min_y.abs() < 1e-6 && (max_y - 167.5).abs() < 1e-6,
        "{min_y} {max_y}"
    );
    assert!(model.groups.iter().any(|group| group.name == "wall"));
}

#[test]
fn the_build_up_is_rebuilt_from_its_numbers() {
    let short = eval(WALL, &[]);
    let long = eval(WALL, &[("length", 6000.0)]);
    assert!(named(&long, "wall/frame/stud").len() > named(&short, "wall/frame/stud").len());
    assert_eq!(errors(&long), Vec::<String>::new());
}

#[test]
fn a_gable_follows_its_top_line_and_stacked_openings_share_a_column() {
    let model = eval(
        "buildup(\"gable\", (0, 0, 0), (1, 0, 0), (0, 0, 1), 4000,
             [{\"name\": \"frame\", \"thickness\": 140, \"spacing\": 625, \"infill\": \"wool\"},
              {\"name\": \"board\", \"thickness\": 15}],
             top = [(0, 2600), (2000, 4280), (4000, 2600)],
             openings = [(1200, 750, 1000, 1350), (1500, 3200, 1000, 400)])
",
        &[],
    );
    assert_eq!(errors(&model), Vec::<String>::new());
    let highest = model
        .parts
        .iter()
        .map(|part| part.world_bounds().1[2])
        .fold(f64::MIN, f64::max);
    assert!((highest - 4280.0).abs() < 1e-6, "{highest}");
    assert_eq!(named(&model, "gable/frame/top plate").len(), 2);
    assert_eq!(named(&model, "gable/frame/header").len(), 2);
}

#[test]
fn alternative_representations_may_overlap_but_other_parts_may_not() {
    let concept = "box(\"concept wall\", (4000, 167.5, 2600), tags = [\"concept\"])\n";
    let tagged = WALL;
    let clashing = eval(&format!("{tagged}{concept}"), &[]);
    assert!(!errors(&clashing).is_empty());
    let alternatives = eval(
        &format!("{tagged}{concept}alternatives([\"concept\", \"construction\"])\n"),
        &[],
    );
    assert_eq!(errors(&alternatives), Vec::<String>::new());
    assert_eq!(alternatives.alternative_tags.len(), 1);
    let shared = eval(
        &format!(
            "{tagged}{concept}alternatives([\"concept\", \"construction\"])\n\
             box(\"pipe\", (100, 100, 100), at = (100, 20, 1000))\n"
        ),
        &[],
    );
    assert!(
        errors(&shared)
            .iter()
            .any(|message| message.contains("pipe")),
        "a part in neither representation still collides"
    );
}

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

fn tagged<'a>(model: &'a ProgramModel, tag: &str) -> Vec<&'a str> {
    model
        .parts
        .iter()
        .filter(|part| part.tags.contains(tag))
        .map(|part| part.name.as_str())
        .collect()
}

fn reach_x(model: &ProgramModel, names: &[&str]) -> f64 {
    names
        .iter()
        .map(|name| model.part(name).unwrap().world_bounds().1[0])
        .fold(f64::MIN, f64::max)
}

#[test]
fn the_house_concept_and_construction_come_from_the_same_numbers() {
    let model = eval(HOUSE, &[]);
    assert_eq!(errors(&model), Vec::<String>::new());
    let concept = tagged(&model, "koncept");
    let construction = tagged(&model, "konštrukcia");
    assert!(concept.len() >= 7, "{concept:?}");
    assert!(construction.len() > 100, "{}", construction.len());
    for layer in [
        "stĺpiky",
        "izolácia",
        "OSB",
        "sadrokartón",
        "krokvy",
        "stropnice",
        "fasáda",
    ] {
        assert!(!tagged(&model, layer).is_empty(), "no part carries {layer}");
    }
    assert!(
        concept.iter().all(|name| !construction.contains(name)),
        "a part is either concept or construction"
    );

    let longer = eval(HOUSE, &[("length", 7400.0)]);
    assert_eq!(errors(&longer), Vec::<String>::new());
    for (before, after) in [
        (
            reach_x(&model, &concept),
            reach_x(&longer, &tagged(&longer, "koncept")),
        ),
        (
            reach_x(&model, &construction),
            reach_x(&longer, &tagged(&longer, "konštrukcia")),
        ),
    ] {
        assert!(
            (after - before - 1200.0).abs() < 1e-6,
            "{before} -> {after}"
        );
    }

    let thicker = eval(HOUSE, &[("system", 1.0)]);
    assert_eq!(errors(&thicker), Vec::<String>::new());
    let wall = |model: &ProgramModel| {
        let (min, max) = model.part("steny/južná").unwrap().world_bounds();
        max[1] - min[1]
    };
    assert_eq!((wall(&model), wall(&thicker)), (200.0, 250.0));
}

#[test]
fn bad_build_ups_name_the_problem() {
    for (source, message) in [
        (
            "buildup(\"w\", (0,0,0), (1,0,0), (1,0,0), 1000, [{\"name\": \"a\", \"thickness\": 10}], height = 100)",
            "up must not run along",
        ),
        (
            "buildup(\"w\", (0,0,0), (1,0,0), (0,0,1), 1000, [{\"name\": \"a\", \"thickness\": 10}], height = 100, openings = [(100, 0, 300, 50), (300, 20, 300, 50)])",
            "overlap",
        ),
        (
            "buildup(\"w\", (0,0,0), (1,0,0), (0,0,1), 1000, [{\"name\": \"a\", \"thickness\": 10}])",
            "give height= or top=",
        ),
        ("alternatives([\"concept\"])", "at least two tags"),
    ] {
        let error = run("bad.star", source, &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains(message), "{source}: {error}");
    }
}
