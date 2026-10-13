use super::*;
use crate::drawing::OrthographicViewKind;

const CENTER: [f64; 2] = [10.0, 20.0];
const RADIUS: f64 = 5.0;
const STEPS: usize = 16;

fn point(angle_rad: f64) -> [f64; 2] {
    [
        CENTER[0] + RADIUS * angle_rad.cos(),
        CENTER[1] + RADIUS * angle_rad.sin(),
    ]
}

fn chord(id: String, from: usize, to: usize) -> ProjectedVisibleLine {
    let step = TAU / STEPS as f64;
    ProjectedVisibleLine {
        stable_line_id: id,
        start_mm: point(step * from as f64),
        end_mm: point(step * to as f64),
    }
}

fn circle(id: &str) -> ProjectedCircle {
    ProjectedCircle {
        stable_circle_id: id.to_owned(),
        center_mm: CENTER,
        radius_mm: RADIUS,
    }
}

fn view(
    visible: Vec<ProjectedVisibleLine>,
    hidden: Vec<ProjectedVisibleLine>,
    circles: Vec<ProjectedCircle>,
) -> OrthographicView {
    OrthographicView {
        kind: OrthographicViewKind::Top,
        stable_view_id: "sheet-1/view-top".into(),
        bounds_mm: [[0.0, 0.0], [30.0, 30.0]],
        visible_lines: visible,
        hidden_lines: hidden,
        circles,
        polyline_curves: Vec::new(),
    }
}

fn degrees(arc: &ModelArc) -> (f64, f64) {
    (arc.start_rad.to_degrees(), arc.span_rad.to_degrees())
}

#[test]
fn every_chord_of_a_circle_becomes_one_circle_with_the_exact_centre_and_radius() {
    let lines = (0..STEPS)
        .map(|step| chord(format!("e:{step}"), step, step + 1))
        .collect();
    let result = view_arcs(&view(lines, Vec::new(), vec![circle("c:1")]));
    let [arc] = result.arcs.as_slice() else {
        panic!("one circle");
    };
    assert_eq!(arc.id, "c:1/visible");
    assert_eq!((arc.center, arc.radius), (CENTER, RADIUS));
    assert_eq!((arc.start_rad, arc.span_rad), (0.0, TAU));
    assert_eq!(result.replaced_lines.len(), STEPS);
}

#[test]
fn a_circle_split_by_hidden_line_removal_gives_a_visible_and_a_hidden_arc() {
    // Edge 2 runs from 45 to 67.5 degrees and is split at its middle point on
    // the chord: the visible half ends exactly at the central projection there.
    let split = chord("e:2".into(), 2, 3);
    let middle = [
        (split.start_mm[0] + split.end_mm[0]) * 0.5,
        (split.start_mm[1] + split.end_mm[1]) * 0.5,
    ];
    let mut visible = vec![
        chord("e:0".into(), 0, 1),
        chord("e:1".into(), 1, 2),
        ProjectedVisibleLine {
            stable_line_id: "e:2/segment-000000".into(),
            start_mm: split.start_mm,
            end_mm: middle,
        },
    ];
    let mut hidden = vec![ProjectedVisibleLine {
        stable_line_id: "e:2/segment-000001".into(),
        start_mm: middle,
        end_mm: split.end_mm,
    }];
    hidden.extend((3..STEPS).map(|step| chord(format!("e:{step}"), step, step + 1)));
    visible.reverse();
    let result = view_arcs(&view(visible, hidden, vec![circle("c:1")]));
    let [visible, hidden] = result.arcs.as_slice() else {
        panic!("two arcs");
    };
    assert_eq!(
        (visible.id.as_str(), hidden.id.as_str()),
        ("c:1/visible-arc-0", "c:1/hidden-arc-0")
    );
    let (start, span) = degrees(visible);
    assert!(
        start.abs() < 1e-9 && (span - 56.25).abs() < 1e-9,
        "{start} {span}"
    );
    let (start, span) = degrees(hidden);
    assert!(
        (start - 56.25).abs() < 1e-9 && (span - 303.75).abs() < 1e-9,
        "{start} {span}"
    );
    assert_eq!(result.replaced_lines.len(), STEPS + 1);
}

#[test]
fn an_arc_across_zero_degrees_is_one_run() {
    let lines = [STEPS - 2, STEPS - 1, 0, 1]
        .into_iter()
        .map(|step| chord(format!("e:{step}"), step, step + 1))
        .collect();
    let result = view_arcs(&view(lines, Vec::new(), vec![circle("c:1")]));
    let [arc] = result.arcs.as_slice() else {
        panic!("one arc: {}", result.arcs.len());
    };
    let (start, span) = degrees(arc);
    assert!(
        (start - 315.0).abs() < 1e-9 && (span - 90.0).abs() < 1e-9,
        "{start} {span}"
    );
}

#[test]
fn coincident_circles_are_one_curve_named_by_the_first() {
    let lines = (0..STEPS)
        .map(|step| chord(format!("e:{step}"), step, step + 1))
        .collect();
    let result = view_arcs(&view(lines, Vec::new(), vec![circle("c:1"), circle("c:2")]));
    assert_eq!(
        result
            .arcs
            .iter()
            .map(|arc| arc.id.as_str())
            .collect::<Vec<_>>(),
        ["c:1/visible"]
    );
}

#[test]
fn lines_that_are_not_chords_of_an_exact_circle_stay_lines() {
    let quarter = TAU / 4.0;
    let lines = vec![
        // An inscribed square's side touches the circle at both ends.
        ProjectedVisibleLine {
            stable_line_id: "square".into(),
            start_mm: point(0.0),
            end_mm: point(quarter),
        },
        // One end on the circle, the other just inside it.
        ProjectedVisibleLine {
            stable_line_id: "inside".into(),
            start_mm: point(0.0),
            end_mm: [point(0.3)[0] - 1e-3, point(0.3)[1]],
        },
        // A chord whose pieces leave the chord.
        ProjectedVisibleLine {
            stable_line_id: "bent/segment-000000".into(),
            start_mm: point(1.0),
            end_mm: [CENTER[0], CENTER[1] + RADIUS + 1.0],
        },
        ProjectedVisibleLine {
            stable_line_id: "bent/segment-000001".into(),
            start_mm: [CENTER[0], CENTER[1] + RADIUS + 1.0],
            end_mm: point(1.3),
        },
    ];
    let result = view_arcs(&view(lines.clone(), Vec::new(), vec![circle("c:1")]));
    assert!(result.arcs.is_empty());
    assert!(result.replaced_lines.is_empty());
    // Without an exact circle no line is ever fitted to one.
    let chords = (0..STEPS)
        .map(|step| chord(format!("e:{step}"), step, step + 1))
        .collect();
    let result = view_arcs(&view(chords, Vec::new(), Vec::new()));
    assert!(result.arcs.is_empty() && result.replaced_lines.is_empty());
}
