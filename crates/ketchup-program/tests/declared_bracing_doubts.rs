//! Bracing a program declares is held against the geometry (review 2026-11-09
//! PG-300 to PG-307): a declaration the model contradicts holds nothing and
//! says why, and a check standing on a joint of an unknown kind is not verified.

use ketchup_program::loads::MemberLoad;
use ketchup_program::member_check::MemberCheck;
use ketchup_program::{Report, Statics, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("bracing.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn load<'a>(report: &'a Report, part: &str) -> &'a MemberLoad {
    report
        .loads
        .members
        .iter()
        .find(|load| load.part == part)
        .unwrap_or_else(|| panic!("no loads of {part}: {:?}", report.loads.members))
}

fn checked<'a>(report: &'a Report, part: &str) -> &'a MemberCheck {
    report
        .design
        .members
        .iter()
        .find(|member| member.part == part)
        .unwrap_or_else(|| panic!("{part} is not checked: {:?}", report.design.members))
}

fn ignored(report: &Report) -> Vec<&str> {
    report
        .issues
        .iter()
        .filter(|issue| issue.kind == "declared_bracing_ignored")
        .map(|issue| issue.message.as_str())
        .collect()
}

/// A GL24h 80 x 400 beam (z 2000 - 2400) on two grounded posts over 5.8 m under
/// about 13 kN at its middle and a grounded wall 1000 mm beside it.
const BEAM: &str = "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
";

/// [`BEAM`] with 50 x 50 laths under it, screwed to its underside and to the wall.
fn laths_under(declared: &str) -> String {
    let mut source = BEAM.to_owned();
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"lath {x}\", (50, 1000, 50), at = ({x}, 0, 1950), material = \"GL24h\", tags = [\"lath\"])
joint(\"lath {x}\", \"beam\", kind = \"screw\")
joint(\"lath {x}\", \"wall\", kind = \"screw\")
"
        ));
    }
    source.push_str(declared);
    source
}

/// PG-300: laths under the beam declared to hold its top held both its edges
/// and the beam passed (rc 0), the compressed top free over 5.8 m. The edge
/// the geometry reads wins: the declaration is reported and the top is free.
#[test]
fn a_brace_declared_on_the_top_but_lying_under_the_beam_holds_only_the_bottom() {
    let declared = report(&laths_under(
        "brace([\"beam\"], by = [\"lath\"], edge = \"top\", spacing = 300)\n",
    ));
    let beam = load(&declared, "beam");
    assert!(
        beam.braced_by.iter().all(|brace| brace.edge == "bottom"),
        "{:?}",
        beam.braced_by
    );
    let messages = ignored(&declared);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("lath 300") && message.contains("bottom")),
        "{messages:?}"
    );
    assert_ne!(declared.statics(), Statics::Verified);
    assert_ne!(checked(&declared, "beam").status, "pass");
}

/// A brace touching the beam's side at mid-depth, where the geometry cannot
/// tell the edge, still holds the edge it declares (review 2026-11-08 §5.1).
#[test]
fn a_brace_on_the_side_at_mid_depth_holds_the_edge_it_declares() {
    let mut source = BEAM.to_owned();
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"lath {x}\", (50, 920, 50), at = ({x}, 80, 2188), material = \"GL24h\", tags = [\"lath\"])
joint(\"lath {x}\", \"beam\", kind = \"screw\")
joint(\"lath {x}\", \"wall\", kind = \"screw\")
"
        ));
    }
    source.push_str("brace([\"beam\"], by = [\"lath\"], edge = \"top\")\n");
    let declared = report(&source);
    assert_eq!(ignored(&declared), Vec::<&str>::new());
    assert!(
        load(&declared, "beam")
            .braced_by
            .iter()
            .any(|brace| brace.edge == "top")
    );
    assert_eq!(declared.statics(), Statics::Verified);
}

/// PG-307: a spacing of 0.0001 mm took 55 s and 3.7 GB before the process
/// died; a program from an AI could bring it down. Below 10 mm it is refused.
#[test]
fn a_brace_spacing_below_ten_millimetres_is_refused() {
    let Err(error) = run(
        "bracing.star",
        &laths_under("brace([\"beam\"], by = [\"lath\"], edge = \"bottom\", spacing = 0.0001)\n"),
        &BTreeMap::new(),
    ) else {
        panic!("a spacing of 0.0001 mm is taken");
    };
    assert!(error.to_string().contains("spacing"), "{error}");
}

/// [`BEAM`] beside a second beam on its own posts, a deck in two boards on
/// both, either side of the load, screwed to the beams every 300 mm, and
/// grounded walls past both ends of the beams.
fn deck(extra: &str) -> String {
    let screws = |from: i32, to: i32, y: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, {y}, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({{\"GL24h\": \"GL24h\"}})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"post c\", (100, 80, 2000), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"post d\", (100, 80, 2000), at = (5800, 1000, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"beam 2\", (5900, 80, 400), at = (0, 1000, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"end wall\", (100, 1080, 2600), at = (6000, 0, 0), material = \"GL24h\", grounded = True)
box(\"start wall\", (100, 1080, 2600), at = (-100, 0, 0), material = \"GL24h\", grounded = True)
box(\"deck a\", (2750, 1080, 22), at = (0, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
box(\"deck b\", (2850, 1080, 22), at = (3150, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
joint(\"deck a\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"deck b\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"deck a\", \"beam 2\", kind = \"screw\")
joint(\"deck b\", \"beam 2\", kind = \"screw\")
{extra}",
        screws(150, 2700, 40),
        screws(3300, 5700, 40)
    )
}

/// The deck screwed to both walls and declared a diaphragm anchored to them.
fn anchored_deck(end_wall_kind: &str) -> String {
    deck(&format!(
        "joint(\"deck b\", \"end wall\", kind = \"{end_wall_kind}\")
joint(\"deck a\", \"start wall\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"end wall\", \"start wall\"])
"
    ))
}

/// The deck anchored at both ends braces the beam along its length.
#[test]
fn a_deck_whose_every_board_reaches_an_anchor_braces_the_beam() {
    let anchored = report(&anchored_deck("screw"));
    let held = load(&anchored, "beam")
        .braced_by
        .iter()
        .filter(|brace| brace.part.starts_with("deck") && brace.edge == "top")
        .count();
    assert!(held >= 18, "{:?}", load(&anchored, "beam").braced_by);
    assert_eq!(checked(&anchored, "beam").status, "pass");
}

/// PG-302: the deck's anchor joined to a strip of it by a joint of an unknown
/// kind ("spoj") was reported not verified, yet the beam the deck braces
/// passed: only joints of the beam and of the board bracing it counted. Every
/// joint of the chain from the member to the ground counts.
#[test]
fn an_unknown_kind_in_the_diaphragm_chain_leaves_the_braced_member_not_verified() {
    let unknown = report(&deck(
        "box(\"strip\", (200, 1080, 22), at = (5800, 0, 2422), material = \"GL24h\", tags = [\"deck\"])
joint(\"strip\", \"end wall\", kind = \"spoj\")
joint(\"deck a\", \"start wall\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"end wall\", \"start wall\"])
",
    ));
    assert!(
        load(&unknown, "beam")
            .braced_by
            .iter()
            .any(|brace| brace.part == "deck b"),
        "{:?}",
        load(&unknown, "beam").braced_by
    );
    let beam = checked(&unknown, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("\"spoj\"")),
        "{:?}",
        beam.missing
    );
}

/// PG-301: a diaphragm held when one of its boards was fastened to its anchor,
/// though the others touched neither it nor the anchor. A board holds only
/// when it reaches an anchor through boards fastened to or touching each other.
#[test]
fn a_board_that_reaches_no_anchor_braces_nothing() {
    let one_end = report(&deck(
        "joint(\"deck b\", \"end wall\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"end wall\"])
",
    ));
    let beam = load(&one_end, "beam");
    assert!(
        beam.braced_by
            .iter()
            .filter(|brace| brace.part == "deck a")
            .count()
            == 0,
        "{:?}",
        beam.braced_by
    );
    assert!(
        beam.braced_by.iter().any(|brace| brace.part == "deck b"),
        "{:?}",
        beam.braced_by
    );
    let messages = ignored(&one_end);
    assert!(
        messages.iter().any(|message| message.contains("deck a")),
        "{messages:?}"
    );
}

/// [`BEAM`] with laths over its top, declared to hold it, each anchored through
/// `anchor` (a grounded part's source) and screwed to it as `joint`.
fn laths_over(anchor: &str, joint: &str) -> String {
    let mut source = BEAM.to_owned();
    source.push_str(anchor);
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"lath {x}\", (50, 1000, 50), at = ({x}, 0, 2400), material = \"GL24h\", tags = [\"lath\"])
joint(\"lath {x}\", \"beam\", kind = \"screw\")
{}
",
            joint.replace("{x}", &x.to_string())
        ));
    }
    source.push_str("brace([\"beam\"], by = [\"lath\"], edge = \"top\")\n");
    source
}

/// PG-303: a declared brace was anchored by any grounded part it was joined
/// to: one whose screws are away from where they touch, or a block in line
/// with the beam's end, which sways with it. The anchorage follows the rules
/// of a brace the geometry finds: fastened where they touch, across the member.
#[test]
fn a_declared_brace_is_anchored_by_the_rules_of_a_found_one() {
    let wall_top = "box(\"wall top\", (5900, 100, 50), at = (0, 1000, 2400), material = \"GL24h\", grounded = True)\n";
    let good = report(&laths_over(
        wall_top,
        "joint(\"lath {x}\", \"wall top\", kind = \"screw\")",
    ));
    assert_eq!(ignored(&good), Vec::<&str>::new());
    assert_eq!(checked(&good, "beam").status, "pass");

    let off_contact = report(&laths_over(
        wall_top,
        "joint(\"lath {x}\", \"wall top\", kind = \"screw\", fasteners = [({x}, 500, 2425)])",
    ));
    assert_ne!(checked(&off_contact, "beam").status, "pass");
    assert!(
        ignored(&off_contact)
            .iter()
            .any(|message| message.contains("anchored")),
        "{:?}",
        ignored(&off_contact)
    );

    // A block on the beam's line past its end, the laths running along it.
    let mut in_line = BEAM.to_owned();
    in_line.push_str(
        "box(\"block\", (100, 80, 50), at = (5900, 0, 2400), material = \"GL24h\", grounded = True)
box(\"lath\", (5900, 80, 50), at = (100, 0, 2400), material = \"GL24h\", tags = [\"lath\"])
joint(\"lath\", \"beam\", kind = \"screw\")
joint(\"lath\", \"block\", kind = \"screw\")
brace([\"beam\"], by = [\"lath\"], edge = \"top\", spacing = 300)
",
    );
    let in_line = report(&in_line);
    assert_ne!(checked(&in_line, "beam").status, "pass");
}

/// A GL24h 80 x 400 rafter at `degrees` extruded from its side, 3 000 mm in
/// plan, its `length_axis` named or not.
fn rafter(degrees: f64, length_axis: Option<&str>) -> String {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let run = 3000.0;
    let rise = run * sin / cos;
    let plumb = 400.0 / cos;
    let attributes = length_axis
        .map(|axis| format!(", attributes = {{\"length_axis\": \"{axis}\"}}"))
        .unwrap_or_default();
    format!(
        "load_path(only = [\"frame\"])
self_weight([\"frame\"])
timber_design({{\"GL24h\": \"GL24h\"}})
part = extrude(\"rafter\", profile = [(0, 0), ({run}, {rise}), ({run}, {}), (0, {plumb})],
               distance = 80, material = \"GL24h\", tags = [\"frame\"]{attributes})
place(part, origin = (0, 80, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"foot\", (100, 80, 2000), at = (-100, 0, 0), material = \"GL24h\", grounded = True)
box(\"head\", (100, 80, 2000), at = ({run}, 0, {}), material = \"GL24h\", grounded = True)
",
        rise + plumb,
        rise
    )
}

/// PG-304, a regression of f0f733b3: a sloping member drawn from its side whose
/// `length_axis` names the level axis took its vertical cut for its depth: a
/// rafter 80 x 400 at 30 degrees was checked as 80 x 461.9. The grain runs
/// along its true length, the named axis choosing only which way that is.
#[test]
fn a_sloping_member_naming_its_level_axis_keeps_its_true_depth() {
    for degrees in [30.0, 50.0] {
        let named = report(&rafter(degrees, Some("x")));
        let section = checked(&named, "rafter").section_mm.expect("a section");
        assert!(
            (section[0] - 80.0).abs() < 1.0 && (section[1] - 400.0).abs() < 1.0,
            "{degrees}: {section:?}"
        );
        let plain = report(&rafter(degrees, None));
        assert_eq!(
            checked(&plain, "rafter").section_mm,
            Some(section),
            "{degrees}"
        );
    }
}

/// PG-305: a 5.9 m beam whose `length_axis` named its 400 mm height became a
/// column and passed; an unknown name was dropped silently. A named axis far
/// shorter than the part's longest leaves it not verified, and an unknown one
/// is refused.
#[test]
fn a_length_axis_far_shorter_than_the_part_or_unknown_is_not_trusted() {
    let named = |axis: &str| {
        BEAM.replace(
            "tags = [\"frame\"])",
            &format!("tags = [\"frame\"], attributes = {{\"length_axis\": \"{axis}\"}})"),
        )
    };
    let column = report(&named("z"));
    let beam = checked(&column, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("length_axis")),
        "{:?}",
        beam.missing
    );
    let Err(error) = run("bracing.star", &named("lengthwise"), &BTreeMap::new()) else {
        panic!("an unknown length_axis is taken");
    };
    assert!(error.to_string().contains("length_axis"), "{error}");
}

/// R1109-PG-310: every declaration that held nothing gave the same hint, to
/// fasten the brace and anchor the diaphragm, whatever the reason was. Each
/// reason now says what to do about it, and a member lists a reason once.
#[test]
fn each_reason_a_declaration_holds_nothing_has_its_own_hint() {
    let hints = |source: &str| -> Vec<(String, String)> {
        let report = report(source);
        for member in &report.design.members {
            let mut once = member.missing.clone();
            once.sort();
            once.dedup();
            assert_eq!(once.len(), member.missing.len(), "{:?}", member.missing);
        }
        report
            .issues
            .iter()
            .filter(|issue| issue.kind == "declared_bracing_ignored")
            .map(|issue| (issue.message.clone(), issue.hint.clone()))
            .collect()
    };
    let cases = [
        (
            laths_under("brace([\"beam\"], by = [\"lath\"], edge = \"top\", spacing = 300)\n"),
            "edge=\"bottom\"",
        ),
        (
            format!("{BEAM}brace([\"beam\"], by = [\"ghost\"], edge = \"top\")\n"),
            "instead of ghost",
        ),
        (
            deck(
                "joint(\"deck b\", \"end wall\", kind = \"screw\")\ndiaphragm([\"deck\"], anchored_to = [\"end wall\"])\n",
            ),
            "Lay deck a against",
        ),
        (
            deck("diaphragm([\"deck\"], anchored_to = [\"post a\", \"end wall\"])\n"),
            "Fasten a sheet to",
        ),
    ];
    let mut seen = Vec::new();
    for (source, hint) in &cases {
        let found = hints(source);
        assert!(
            found.iter().any(|(_, given)| given.contains(hint)),
            "{hint}: {found:#?}"
        );
        seen.extend(found.into_iter().map(|(_, given)| given));
    }
    seen.sort();
    seen.dedup();
    assert!(seen.len() >= cases.len(), "{seen:#?}");
}
