//! Parts whose world bounds lie beyond the gap limit are dropped without
//! measuring their boxes, and the relations stay exactly those of measuring
//! every candidate pair.
use super::*;
use crate::exact::ExactPair;
use std::collections::BTreeMap;

const TINY_HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");
const HOUSE_PROJECT: &str = include_str!("../../../examples/programs/house-project.star");

fn model(source: &str) -> ProgramModel {
    crate::evaluate("test.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model
}

fn assert_same(model: &ProgramModel, issues: &[Issue], exact: &ExactShapes) -> Vec<Relation> {
    let skipped = relations_by_bounds(model, issues, exact, true);
    let measured = relations_by_bounds(model, issues, exact, false);
    if let Some((index, (left, right))) = skipped
        .iter()
        .zip(&measured)
        .enumerate()
        .find(|(_, (left, right))| left != right)
    {
        panic!("relation {index} differs:\n{left:#?}\n{right:#?}");
    }
    assert_eq!(skipped.len(), measured.len());
    skipped
}

/// Exact answers near and far: every part apart from every other unless
/// stated, and a collision or a clearance for pairs at every distance, the
/// far ones outside their boxes.
fn exact_answers(model: &ProgramModel) -> ExactShapes {
    let names: Vec<String> = model.parts.iter().map(|part| part.name.clone()).collect();
    let mut exact = ExactShapes::default();
    exact.checked_against(names.clone(), names.clone());
    let collision = ExactPair {
        common_volume_mm3: 1000.0,
        common_thickness_mm: 5.0,
        ..ExactPair::default()
    };
    let clearance = ExactPair {
        distance_mm: Some(3.0),
        ..ExactPair::default()
    };
    for (index, name) in names.iter().enumerate() {
        for (step, answer) in [(1, collision), (7, clearance), (names.len() / 2, collision)] {
            if let Some(other) = names.get(index + step.max(1)) {
                exact.insert(name, other, answer);
            }
        }
    }
    exact
}

fn assert_house_unchanged(model: &ProgramModel) {
    for exact in [ExactShapes::default(), exact_answers(model)] {
        let issues = validate::validate_with(model, &exact);
        let relations = assert_same(model, &issues, &exact);
        assert!(relations.len() > 1000, "{}", relations.len());
        let gaps = || relations.iter().filter_map(|relation| relation.gap_mm);
        assert!(gaps().count() > 100);
        assert!(
            relations
                .iter()
                .filter(|relation| relation.joint.is_none())
                .all(|relation| relation.gap_mm.is_none_or(|gap| gap <= GAP_LIMIT_MM)),
            "only a jointed pair is listed beyond the gap limit"
        );
    }
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_bed_lists_the_relations_of_measuring_every_pair() {
    assert_house_unchanged(&model(&TINY_HOUSE.replace("BED_W = 1400", "BED_W = 1300")));
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_project_lists_the_relations_of_measuring_every_pair() {
    assert_house_unchanged(&model(HOUSE_PROJECT));
}

fn two_boxes(gap: f64) -> ProgramModel {
    model(&format!(
        "box(\"a\", (100, 100, 100), at = (0, 0, 0))\n\
         box(\"b\", (100, 100, 100), at = ({}, 0, 0))\n",
        100.0 + gap
    ))
}

#[test]
fn a_pair_at_the_gap_limit_is_measured_and_kept_and_one_beyond_dropped() {
    for (gap, listed, skipped) in [
        (19.95, true, false),
        (20.0, true, false),
        (20.05, false, false),
        (20.5, false, false),
        (21.0, false, false),
        (21.05, false, true),
        (25.0, false, true),
    ] {
        let model = two_boxes(gap);
        let bounds = model
            .parts
            .iter()
            .map(Part::world_bounds)
            .collect::<Vec<_>>();
        assert_eq!(
            bounds_beyond_gap_limit(&bounds[0], &bounds[1]),
            skipped,
            "{gap}"
        );
        let relations = assert_same(&model, &[], &ExactShapes::default());
        assert_eq!(
            relations.len(),
            usize::from(listed),
            "{gap}: {relations:#?}"
        );
        if listed {
            assert_eq!(relations[0].kind, RelationKind::Gap);
            // gap_mm is rounded to 0.1 mm; 19.95 may be built as 19.9499…
            let measured = relations[0].gap_mm.expect("a gap is measured");
            assert!((measured - gap).abs() <= 0.05 + 1e-9, "{gap}: {measured}");
        }
    }
}

/// A plank turned 45° about z, whose world bounds are far larger than its
/// box, and a block turned with it, centred at `centre`.
fn plank_and_block(centre: impl Fn(&Obb) -> [f64; 3]) -> ProgramModel {
    let mut model = model(
        "plank = box(\"plank\", (2000, 100, 100), at = (0, 0, 0))\n\
         rotate(plank, axis = (0, 0, 1), angle = 45)\n\
         box(\"block\", (50, 50, 50), at = (0, 0, 0))\n",
    );
    let (plank, block) = model.parts.split_at_mut(1);
    let (plank, block) = (&plank[0], &mut block[0]);
    let target = centre(&plank.obb());
    block.rotation = plank.rotation;
    let now = block.obb().centre;
    block.at_mm = std::array::from_fn(|i| block.at_mm[i] + target[i] - now[i]);
    model
}

fn beside(obb: &Obb, clearance: f64) -> [f64; 3] {
    std::array::from_fn(|i| obb.centre[i] + obb.axes[1][i] * (obb.half[1] + clearance + 25.0))
}

#[test]
fn a_turned_part_is_measured_on_its_box_where_its_bounds_reach() {
    let distances = |model: &ProgramModel| {
        let (a, b) = (&model.parts[0], &model.parts[1]);
        let near = !bounds_beyond_gap_limit(&a.world_bounds(), &b.world_bounds());
        (near, a.obb().distance(&b.obb()))
    };

    // Beside the long face, 10 mm off: listed with that clearance.
    let model = plank_and_block(|obb| beside(obb, 10.0));
    let (near, distance) = distances(&model);
    assert!(near && (distance - 10.0).abs() < 1.0e-6, "{distance}");
    let relations = assert_same(&model, &[], &ExactShapes::default());
    assert_eq!(relations.len(), 1, "{relations:#?}");
    assert_eq!(relations[0].gap_mm, Some(10.0));

    // Just beyond the limit beside the face, well inside the plank's bounds:
    // measured on the boxes and dropped.
    let model = plank_and_block(|obb| beside(obb, 20.5));
    let (near, distance) = distances(&model);
    assert!(near && distance > GAP_LIMIT_MM, "{distance}");
    assert!(assert_same(&model, &[], &ExactShapes::default()).is_empty());

    // In the empty corner of the plank's bounds, hundreds of millimetres off
    // the plank although the bounds overlap.
    let model = plank_and_block(|obb| {
        let (low, high) = obb.world_bounds();
        [
            [low[0], low[1]],
            [low[0], high[1]],
            [high[0], low[1]],
            [high[0], high[1]],
        ]
        .into_iter()
        .map(|[x, y]| [x, y, obb.centre[2]])
        .max_by(|left, right| {
            let off = |point: &[f64; 3]| {
                obb.distance(&Obb {
                    centre: *point,
                    axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                    half: [0.0; 3],
                })
            };
            off(left).total_cmp(&off(right))
        })
        .unwrap()
    });
    let (near, distance) = distances(&model);
    assert!(near && distance > 200.0, "{distance}");
    assert!(assert_same(&model, &[], &ExactShapes::default()).is_empty());

    // Above the plank's bounds: dropped without measuring the boxes.
    let model =
        plank_and_block(|obb| std::array::from_fn(|i| obb.centre[i] + [0.0, 0.0, 200.0][i]));
    let (near, distance) = distances(&model);
    assert!(!near && distance > GAP_LIMIT_MM, "{distance}");
    assert!(assert_same(&model, &[], &ExactShapes::default()).is_empty());
}
