//! Exact-evidence model tests; no mesh geometry is used to infer a hole.
use super::*;
use ketchup_geometry::sketch::{
    CutStart, FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec,
};
use ketchup_model::document::{HoleThread, ProfileSegment, Snapshot};
use ketchup_model::drawing::OrthographicDrawing;
use ketchup_model::drawing::hole_callouts::{HoleCalloutDiagnosticKind, generate_hole_callouts};
use ketchup_model::exact_product::{ExactBRepGraphFaceEvidence, ExactConeFaceEvidence};

const CUT: FeatureId = FeatureId(5);

fn document() -> DocumentStore {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DEFINITION,
                name: "Bore profile".into(),
                kind: FeatureKind::Profile {
                    closed: true,
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [13.4, 5.0],
                            end_mm: [6.6, 5.0],
                            center_mm: [10.0, 5.0],
                            clockwise: false,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [6.6, 5.0],
                            end_mm: [13.4, 5.0],
                            center_mm: [10.0, 5.0],
                            clockwise: false,
                        },
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT,
                definition_id: DEFINITION,
                name: "Circular cut".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(FeatureId(4)),
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("20").unwrap()),
                    operation: PadOperation::Cut {
                        target: EXTRUSION,
                        start: CutStart::ProfilePlane,
                    },
                }),
            },
        ]))
        .unwrap();
    document
}

fn thread(depth: f64) -> CanonicalCommand {
    CanonicalCommand::SetHoleThread {
        id: CUT,
        thread: Some(HoleThread {
            size: "M8".into(),
            pitch_mm: 1.25,
            depth_mm: depth,
            modeled: Some(false),
        }),
    }
}

fn exact(snapshot: &Snapshot, depth: f64, complete: bool) -> ExactResultRegistry {
    let producer = snapshot
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
        .map(|feature| feature.id())
        .max()
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(snapshot, DEFINITION, producer).unwrap();
    let mesh = box_mesh();
    let edges = [0., depth]
        .into_iter()
        .enumerate()
        .map(|(i, z)| ExactBRepGraphEdgeEvidence {
            edge_ordinal: i as u32,
            curve_kind: "circle".into(),
            length_mm: std::f64::consts::TAU * 3.4,
            centroid_mm: [10., 5., z],
            bounds_mm: [[6.6, 1.6, z], [13.4, 8.4, z]],
            closed: complete,
            circle_radius_mm: Some(3.4),
            axis_origin_mm: Some([10., 5., z]),
            unit_axis_direction: Some([0., 0., 1.]),
            adjacent_face_ordinals: vec![6],
        })
        .collect();
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            mass: None,
            exact_input_digest: "hole-callouts-input".into(),
            result_fingerprint: format!("hole-{depth}-{complete}"),
            volume_mm3: 5000.,
            area_mm2: 0.,
            topology_counts: [8, 14, 7, 1, 1],
            wire_count: None,
            bounds_mm: [[0., 0., 0.], [20., 10., 30.]],
            backend: "exact-evidence-test-double".into(),
            tolerance: "1e-7-mm".into(),
            faces: vec![ExactBRepGraphFaceEvidence {
                semantic_role: "bore".into(),
                source_element_id: "bore".into(),
                face_ordinal: 6,
                surface_kind: "cylinder".into(),
                corroborating_geometry_fingerprint: "bore-cylinder".into(),
                centroid_mm: [10., 5., depth / 2.],
                unit_normal: [1., 0., 0.],
                axis_origin_mm: Some([10., 5., 0.]),
                unit_axis_direction: Some([0., 0., 1.]),
                cone: None,
            }],
            edges,
        },
        &mesh,
    )
    .unwrap();
    ExactResultRegistry::accept(snapshot, [Arc::new(ExactBodyPackage::from(package))]).unwrap()
}

fn box_mesh() -> StepImportMesh {
    StepImportMesh {
        vertices_mm: vec![
            [0., 0., 0.],
            [20., 0., 0.],
            [20., 10., 0.],
            [0., 10., 0.],
            [0., 0., 30.],
            [20., 0., 30.],
            [20., 10., 30.],
            [0., 10., 30.],
        ],
        triangles: [
            ([0, 2, 1], 0),
            ([0, 3, 2], 0),
            ([4, 5, 6], 1),
            ([4, 6, 7], 1),
            ([0, 1, 5], 2),
            ([0, 5, 4], 2),
            ([1, 2, 6], 3),
            ([1, 6, 5], 3),
            ([2, 3, 7], 4),
            ([2, 7, 6], 4),
            ([3, 0, 4], 5),
            ([3, 4, 7], 5),
        ]
        .into_iter()
        .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
            vertex_indices,
            face_ordinal,
        })
        .collect(),
    }
}

fn annotated_sheet() -> DrawingSheet {
    sheet_with(
        vec![OrthographicViewKind::Top],
        vec![DrawingNote::new(DrawingNoteId(7), [20., 80.], "Manual inspection").unwrap()],
    )
}

fn sheet_with(views: Vec<OrthographicViewKind>, notes: Vec<DrawingNote>) -> DrawingSheet {
    DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "Holes",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Holes", "H-1", "A", "Test").unwrap(),
        views,
        DrawingAnnotations::new(vec![], notes),
    )
    .unwrap()
}

fn circle_profile(center: [f64; 2], radius: f64) -> FeatureKind {
    let [x, y] = center;
    FeatureKind::Profile {
        closed: true,
        segments: vec![
            ProfileSegment::CircularArc {
                start_mm: [x + radius, y],
                end_mm: [x - radius, y],
                center_mm: center,
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [x - radius, y],
                end_mm: [x + radius, y],
                center_mm: center,
                clockwise: false,
            },
        ],
    }
}

/// The Ø6.8 bore of `document()` plus a Ø3 bore cut into it.
fn two_bore_document() -> DocumentStore {
    let mut document = document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DEFINITION,
                name: "Small bore profile".into(),
                kind: circle_profile([4.0, 5.0], 1.5),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(7),
                definition_id: DEFINITION,
                name: "Small circular cut".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(FeatureId(6)),
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("20").unwrap()),
                    operation: PadOperation::Cut {
                        target: CUT,
                        start: CutStart::ProfilePlane,
                    },
                }),
            },
        ]))
        .unwrap();
    document
}

/// Exact evidence for blind bores `(center, radius)` of `depth`, each with two
/// closed rims (edges 2k, 2k+1) around cylinder face 6+k.
fn exact_bores(snapshot: &Snapshot, bores: &[([f64; 2], f64)], depth: f64) -> ExactResultRegistry {
    let producer = snapshot
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
        .map(|feature| feature.id())
        .max()
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(snapshot, DEFINITION, producer).unwrap();
    let mesh = box_mesh();
    let mut edges = Vec::new();
    let mut faces = Vec::new();
    for (k, &([x, y], radius)) in bores.iter().enumerate() {
        let face_ordinal = 6 + k as u32;
        for (i, z) in [0., depth].into_iter().enumerate() {
            edges.push(ExactBRepGraphEdgeEvidence {
                edge_ordinal: (2 * k + i) as u32,
                curve_kind: "circle".into(),
                length_mm: std::f64::consts::TAU * radius,
                centroid_mm: [x, y, z],
                bounds_mm: [[x - radius, y - radius, z], [x + radius, y + radius, z]],
                closed: true,
                circle_radius_mm: Some(radius),
                axis_origin_mm: Some([x, y, z]),
                unit_axis_direction: Some([0., 0., 1.]),
                adjacent_face_ordinals: vec![face_ordinal],
            });
        }
        faces.push(ExactBRepGraphFaceEvidence {
            semantic_role: "bore".into(),
            source_element_id: format!("bore-{k}"),
            face_ordinal,
            surface_kind: "cylinder".into(),
            corroborating_geometry_fingerprint: format!("bore-cylinder-{k}"),
            centroid_mm: [x, y, depth / 2.],
            unit_normal: [1., 0., 0.],
            axis_origin_mm: Some([x, y, 0.]),
            unit_axis_direction: Some([0., 0., 1.]),
            cone: None,
        });
    }
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            mass: None,
            exact_input_digest: "hole-callouts-input".into(),
            result_fingerprint: format!("bores-{}-{depth}", bores.len()),
            volume_mm3: 5000.,
            area_mm2: 0.,
            topology_counts: [8, 14, 6 + bores.len() as u32, 1, 1],
            wire_count: None,
            bounds_mm: [[0., 0., 0.], [20., 10., 30.]],
            backend: "exact-evidence-test-double".into(),
            tolerance: "1e-7-mm".into(),
            faces,
            edges,
        },
        &mesh,
    )
    .unwrap();
    ExactResultRegistry::accept(snapshot, [Arc::new(ExactBodyPackage::from(package))]).unwrap()
}

/// Exact evidence of one hole about (10, 5) entering the box's z = 0 face
/// (face 0) along +z, as an OCCT body reports it: its axial section `profile`
/// as (depth, radius) corners from the entry, joined by cylinders, cones and
/// flat rings (faces 6, 7, …). A last corner of radius 0 ends it in a flat
/// bottom (after a ring) or a drill point (after a cone, an apex edge); one at
/// depth 30 opens on the box's far face (face 1).
fn exact_hole(snapshot: &Snapshot, profile: &[(f64, f64)]) -> ExactResultRegistry {
    let producer = snapshot
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
        .map(|feature| feature.id())
        .max()
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(snapshot, DEFINITION, producer).unwrap();
    let [x, y] = [10., 5.];
    let axis = [0., 0., 1.];
    let mut edges = Vec::new();
    let mut faces = Vec::new();
    let last = profile.len() - 1;
    for (k, &(z, radius)) in profile.iter().enumerate() {
        let before = if k == 0 { 0 } else { 5 + k as u32 };
        if radius == 0.0 {
            if profile[k - 1].0 != z {
                edges.push(ExactBRepGraphEdgeEvidence {
                    edge_ordinal: k as u32,
                    curve_kind: "degenerate".into(),
                    length_mm: 0.,
                    centroid_mm: [x, y, z],
                    bounds_mm: [[x, y, z], [x, y, z]],
                    closed: false,
                    circle_radius_mm: None,
                    axis_origin_mm: None,
                    unit_axis_direction: None,
                    adjacent_face_ordinals: vec![before],
                });
            }
            continue;
        }
        let after = if k < last {
            6 + k as u32
        } else {
            assert_eq!(z, 30., "a hole ends at the far face or in radius 0");
            1
        };
        edges.push(ExactBRepGraphEdgeEvidence {
            edge_ordinal: k as u32,
            curve_kind: "circle".into(),
            length_mm: std::f64::consts::TAU * radius,
            centroid_mm: [x, y, z],
            bounds_mm: [[x - radius, y - radius, z], [x + radius, y + radius, z]],
            closed: true,
            circle_radius_mm: Some(radius),
            axis_origin_mm: Some([x, y, z]),
            unit_axis_direction: Some(axis),
            adjacent_face_ordinals: vec![before, after],
        });
    }
    for (k, pair) in profile.windows(2).enumerate() {
        let [(z0, r0), (z1, r1)] = [pair[0], pair[1]];
        let face_ordinal = 6 + k as u32;
        let (surface_kind, unit_normal, axis_origin_mm, unit_axis_direction, cone) = if r0 == r1 {
            (
                "cylinder",
                [-1., 0., 0.],
                Some([x, y, 0.]),
                Some(axis),
                None,
            )
        } else if z0 == z1 {
            let facing = if r1 < r0 { -1. } else { 1. };
            ("plane", [0., 0., facing], None, None, None)
        } else {
            // Out of the material, into the hole: towards the axis and the entry.
            let length = (z1 - z0).hypot(r1 - r0);
            let normal = [-(z1 - z0) / length, 0., (r1 - r0) / length];
            let cone = ExactConeFaceEvidence {
                unit_axis_direction: axis,
                unit_normal: normal,
            };
            ("cone", normal, None, None, Some(cone))
        };
        faces.push(ExactBRepGraphFaceEvidence {
            semantic_role: "hole".into(),
            source_element_id: format!("hole-{k}"),
            face_ordinal,
            surface_kind: surface_kind.into(),
            corroborating_geometry_fingerprint: format!("hole-{surface_kind}-{k}"),
            centroid_mm: [x + (r0 + r1) / 2., y, (z0 + z1) / 2.],
            unit_normal,
            axis_origin_mm,
            unit_axis_direction,
            cone,
        });
    }
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            mass: None,
            exact_input_digest: "hole-callouts-input".into(),
            result_fingerprint: format!("hole-{profile:?}"),
            volume_mm3: 5000.,
            area_mm2: 0.,
            topology_counts: [8, 14, 6 + faces.len() as u32, 1, 1],
            wire_count: None,
            bounds_mm: [[0., 0., 0.], [20., 10., 30.]],
            backend: "exact-evidence-test-double".into(),
            tolerance: "1e-7-mm".into(),
            faces,
            edges,
        },
        &box_mesh(),
    )
    .unwrap();
    ExactResultRegistry::accept(snapshot, [Arc::new(ExactBodyPackage::from(package))]).unwrap()
}

/// `document()` with a Ø9 cut of `depth` on the axis of its Ø6.8 bore.
fn counterbored_document(depth: &str) -> DocumentStore {
    let mut document = document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DEFINITION,
                name: "Counterbore profile".into(),
                kind: circle_profile([10.0, 5.0], 4.5),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(7),
                definition_id: DEFINITION,
                name: "Counterbore".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(FeatureId(6)),
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal(depth).unwrap()),
                    operation: PadOperation::Cut {
                        target: CUT,
                        start: CutStart::ProfilePlane,
                    },
                }),
            },
        ]))
        .unwrap();
    document
}

/// The callout texts `generate_hole_callouts` writes, after no diagnostic.
fn callouts(snapshot: &Snapshot, results: &ExactResultRegistry) -> Vec<(String, usize)> {
    let report = generate_hole_callouts(snapshot, results, &annotated_sheet()).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let drawing = project_orthographic_drawing(snapshot, results, &report.sheet).unwrap();
    report
        .sheet
        .notes()
        .iter()
        .filter_map(|note| {
            let sources = note.hole_callout_sources()?;
            let layout = drawing
                .layout
                .notes
                .iter()
                .find(|layout| {
                    layout
                        .stable_note_id
                        .ends_with(&format!("/note-{}", note.id().0))
                })
                .unwrap();
            assert!(layout.leader_line_mm.is_some() && layout.target_circle_id.is_some());
            Some((note.text_template().to_owned(), sources.len()))
        })
        .collect()
}

/// Holes are read from the finished exact faces about the cut's axis: a
/// counterbore, a countersink, a stepped bore and how each ends.
#[test]
fn hole_callouts_read_counterbores_countersinks_steps_and_ends_from_exact_faces() {
    let snapshot = document().current();
    let through = [(0., 3.4), (30., 3.4)];
    assert_eq!(
        callouts(&snapshot, &exact_hole(&snapshot, &through)),
        [("BORE Ø6.8 mm THRU".to_owned(), 1)]
    );
    assert_eq!(
        callouts(
            &snapshot,
            &exact_hole(&snapshot, &[(0., 3.4), (20., 3.4), (20., 0.)])
        ),
        [("BORE Ø6.8 mm DEPTH 20 mm, FLAT BOTTOM".to_owned(), 1)]
    );
    // A bore deepened by a second cut can stay two faces; it is one bore.
    assert_eq!(
        callouts(
            &snapshot,
            &exact_hole(&snapshot, &[(0., 3.4), (8., 3.4), (20., 3.4), (20., 0.)])
        ),
        [("BORE Ø6.8 mm DEPTH 20 mm, FLAT BOTTOM".to_owned(), 1)]
    );
    assert_eq!(
        callouts(
            &snapshot,
            &exact_hole(&snapshot, &[(0., 3.4), (18., 3.4), (21.4, 0.)])
        ),
        [("BORE Ø6.8 mm DEPTH 18 mm, 90° POINT".to_owned(), 1)]
    );
    // A chamfered entry: the cone is read from the body, whatever cut it.
    assert_eq!(
        callouts(
            &snapshot,
            &exact_hole(&snapshot, &[(0., 4.4), (1., 3.4), (30., 3.4)])
        ),
        [("BORE Ø6.8 mm THRU; COUNTERSINK Ø8.8 mm × 90°".to_owned(), 1)]
    );
    // Two coaxial cuts make one counterbored hole: one callout, both sources.
    let counterbored = counterbored_document("5").current();
    assert_eq!(
        callouts(
            &counterbored,
            &exact_hole(
                &counterbored,
                &[(0., 4.5), (5., 4.5), (5., 3.4), (30., 3.4)]
            )
        ),
        [(
            "BORE Ø6.8 mm THRU; COUNTERBORE Ø9 mm DEPTH 5 mm".to_owned(),
            2
        )]
    );
    assert_eq!(
        callouts(
            &counterbored,
            &exact_hole(
                &counterbored,
                &[(0., 4.5), (5., 4.5), (5., 3.4), (18., 3.4), (21.4, 0.)]
            )
        ),
        [(
            "BORE Ø6.8 mm DEPTH 18 mm, 90° POINT; COUNTERBORE Ø9 mm DEPTH 5 mm".to_owned(),
            2
        )]
    );
    // A step whose shoulder is a cone, as a step drill leaves it.
    assert_eq!(
        callouts(
            &counterbored,
            &exact_hole(
                &counterbored,
                &[(0., 4.5), (5., 4.5), (6.1, 3.4), (30., 3.4)]
            )
        ),
        [(
            "BORE Ø6.8 mm THRU; COUNTERBORE Ø9 mm DEPTH 5 mm, 90° STEP".to_owned(),
            2
        )]
    );
    // A tapped through hole names where the bore ends.
    let mut tapped = document();
    tapped
        .apply_batch(&CommandBatch::new(vec![thread(12.)]))
        .unwrap();
    let tapped = tapped.current();
    assert_eq!(
        callouts(&tapped, &exact_hole(&tapped, &through)),
        [(
            "M8 x 1.25 THREAD DEPTH 12 mm; BORE Ø6.8 mm THRU".to_owned(),
            1
        )]
    );
}

/// A bore that widens again below its entry is reported, never described as
/// a simpler hole.
#[test]
fn hole_callouts_report_bores_that_widen_below_their_entry() {
    let snapshot = document().current();
    let undercut = exact_hole(
        &snapshot,
        &[
            (0., 3.4),
            (5., 3.4),
            (5., 4.5),
            (10., 4.5),
            (10., 3.4),
            (30., 3.4),
        ],
    );
    let report = generate_hole_callouts(&snapshot, &undercut, &annotated_sheet()).unwrap();
    assert_eq!(report.sheet.notes(), annotated_sheet().notes());
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(
        diagnostic.kind,
        HoleCalloutDiagnosticKind::UnsupportedProvenance
    );
    assert!(
        diagnostic.reason.contains("widens"),
        "{}",
        diagnostic.reason
    );
}

/// Every generated callout's leader must end on the projected rim of a bore
/// whose diameter its text states, in the view that shows it; returns the ends.
fn assert_leaders_on_own_bores(
    drawing: &OrthographicDrawing,
    sheet: &DrawingSheet,
) -> Vec<[f64; 2]> {
    let mut ends = Vec::new();
    for note in sheet.notes() {
        let layout = drawing
            .layout
            .notes
            .iter()
            .find(|layout| {
                layout.stable_note_id == format!("sheet-{}/note-{}", SHEET.0, note.id().0)
            })
            .unwrap();
        if note.hole_callout_sources().is_none() {
            assert_eq!(layout.leader_line_mm, None);
            continue;
        }
        let leader = layout
            .leader_line_mm
            .expect("generated callout has a leader");
        let target = layout.target_circle_id.as_deref().unwrap();
        let (view, placement) = drawing
            .views
            .iter()
            .zip(&drawing.layout.view_placements)
            .find(|(view, _)| view.circles.iter().any(|c| c.stable_circle_id == target))
            .expect("target circle is in a view");
        let circle = view
            .circles
            .iter()
            .find(|c| c.stable_circle_id == target)
            .unwrap();
        let scale = (placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0])
            / (view.bounds_mm[1][0] - view.bounds_mm[0][0]);
        let center = [
            placement.origin_mm[0] + circle.center_mm[0] * scale,
            placement.origin_mm[1] + circle.center_mm[1] * scale,
        ];
        let end = leader[1];
        let distance = (end[0] - center[0]).hypot(end[1] - center[1]);
        assert!(
            (distance - circle.radius_mm * scale).abs() < 1e-6,
            "{distance} {circle:?}"
        );
        assert_eq!(leader[0], layout.position_mm);
        let stated: f64 = layout
            .text
            .split('Ø')
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            (stated - 2.0 * circle.radius_mm).abs() < 1e-9,
            "{} → {target}",
            layout.text
        );
        ends.push(end);
    }
    ends
}

#[test]
fn hole_callout_leaders_point_at_their_own_bores_and_follow_the_view() {
    let bores = [([10., 5.], 3.4), ([4., 5.], 1.5)];
    let mut document = two_bore_document();
    let snapshot = document.current();
    let report = generate_hole_callouts(
        &snapshot,
        &exact_bores(&snapshot, &bores, 20.),
        &annotated_sheet(),
    )
    .unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(
        report
            .sheet
            .notes()
            .iter()
            .filter(|note| note.hole_callout_sources().is_some())
            .count(),
        2
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDrawingSheet(report.sheet.clone()),
        ]))
        .unwrap();
    let snapshot = document.current();
    let results = exact_bores(&snapshot, &bores, 20.);
    let drawing = project_orthographic_drawing(&snapshot, &results, &report.sheet).unwrap();
    let ends = assert_leaders_on_own_bores(&drawing, &report.sheet);
    assert_eq!(ends.len(), 2);
    assert_ne!(ends[0], ends[1]);
    let svg =
        String::from_utf8_lossy(export_drawing(&snapshot, &drawing).unwrap().svg()).into_owned();
    for note in report
        .sheet
        .notes()
        .iter()
        .filter(|n| n.hole_callout_sources().is_some())
    {
        assert!(svg.contains(&format!("sheet-{}/note-{}/leader", SHEET.0, note.id().0)));
    }
    // A Front view ahead of Top moves Top up the page; the leaders follow it.
    let moved = sheet_with(
        vec![OrthographicViewKind::Front, OrthographicViewKind::Top],
        report.sheet.notes().to_vec(),
    );
    let moved_drawing = project_orthographic_drawing(&snapshot, &results, &moved).unwrap();
    let moved_ends = assert_leaders_on_own_bores(&moved_drawing, &moved);
    assert_ne!(moved_ends, ends);
}

#[test]
fn hole_callouts_refuse_holes_no_view_shows_as_a_circle() {
    let document = document();
    let snapshot = document.current();
    let front_only = sheet_with(vec![OrthographicViewKind::Front], vec![]);
    let report =
        generate_hole_callouts(&snapshot, &exact(&snapshot, 20., true), &front_only).unwrap();
    assert!(report.sheet.notes().is_empty());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.kind == HoleCalloutDiagnosticKind::NotShownAsCircle)
    );
}

#[test]
fn hole_callouts_count_the_finished_cylinder_not_the_cut_features() {
    let mut document = document();
    let FeatureKind::Pad(mut pad) = document.current().feature(CUT).unwrap().kind().clone() else {
        panic!("cut")
    };
    pad.operation = PadOperation::Cut {
        target: CUT,
        start: CutStart::ProfilePlane,
    };
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(6),
            definition_id: DEFINITION,
            name: "Deepen bore".into(),
            kind: FeatureKind::Pad(pad),
        }]))
        .unwrap();
    let snapshot = document.current();
    let results = exact(&snapshot, 20., true);
    let report = generate_hole_callouts(&snapshot, &results, &annotated_sheet()).unwrap();
    assert!(report.diagnostics.is_empty());
    assert_eq!(
        report.sheet.notes()[1].text_template(),
        "BORE Ø6.8 mm; CYLINDRICAL DEPTH 20 mm"
    );
    assert_eq!(
        report.sheet.notes()[1]
            .hole_callout_sources()
            .unwrap()
            .len(),
        2
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDrawingSheet(report.sheet.clone()),
        ]))
        .unwrap();
    let snapshot = document.current();
    let results = exact(&snapshot, 20., true);
    let drawing = project_orthographic_drawing(&snapshot, &results, &report.sheet).unwrap();
    let bundle = export_drawing(&snapshot, &drawing).unwrap();
    assert!(!String::from_utf8_lossy(bundle.svg()).contains("2×"));
}

#[test]
fn hole_callouts_plain_and_tapped_export_regenerate_and_persist() {
    let mut document = document();
    let plain = generate_hole_callouts(
        &document.current(),
        &exact(&document.current(), 20., true),
        &annotated_sheet(),
    )
    .unwrap();
    assert!(plain.diagnostics.is_empty());
    assert_eq!(
        plain.sheet.notes()[1].text_template(),
        "BORE Ø6.8 mm; CYLINDRICAL DEPTH 20 mm"
    );
    assert_eq!(plain.sheet.notes()[0], annotated_sheet().notes()[0]);
    let again = generate_hole_callouts(
        &document.current(),
        &exact(&document.current(), 20., true),
        &plain.sheet,
    )
    .unwrap();
    assert_eq!(again.sheet, plain.sheet);
    document
        .apply_batch(&CommandBatch::new(vec![thread(12.)]))
        .unwrap();
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &exact(&document.current(), 20., true),
            &plain.sheet
        )
        .unwrap_err(),
        DrawingError::AnnotationSourceLost
    );
    let tapped = generate_hole_callouts(
        &document.current(),
        &exact(&document.current(), 20., true),
        &plain.sheet,
    )
    .unwrap();
    assert_eq!(
        tapped.sheet.notes()[1].text_template(),
        "M8 x 1.25 THREAD DEPTH 12 mm; BORE Ø6.8 mm"
    );
    assert_eq!(tapped.sheet.notes()[1].id(), plain.sheet.notes()[1].id());
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDrawingSheet(tapped.sheet.clone()),
        ]))
        .unwrap();
    let bytes = persistence::save(&document.current());
    let golden = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/persistence/format113-hole-callouts.bin"
    );
    if ketchup_test_env::update_golden() {
        std::fs::write(golden, &bytes).unwrap();
    }
    let fixture = std::fs::read(golden).unwrap();
    let fixture_document = persistence::load(&fixture)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(
        fixture_document.current().drawing_sheet(SHEET),
        Some(&tapped.sheet)
    );
    assert_eq!(persistence::save(&fixture_document.current()), fixture);
    let reopened = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&tapped.sheet));
    assert_eq!(persistence::save(&reopened.current()), bytes);
    let drawing = project_orthographic_drawing(
        &reopened.current(),
        &exact(&reopened.current(), 20., true),
        &tapped.sheet,
    )
    .unwrap();
    let bundle = export_drawing(&reopened.current(), &drawing).unwrap();
    assert!(
        String::from_utf8_lossy(bundle.svg())
            .contains("M8 x 1.25 THREAD DEPTH 12 mm; BORE Ø6.8 mm")
    );
    assert!(String::from_utf8_lossy(bundle.dxf()).contains("THREAD DEPTH 12"));
    assert!(bundle.pdf().is_ok());
}

#[test]
fn hole_callouts_stale_incomplete_and_removed_threads_never_export_old_text() {
    let mut document = document();
    document
        .apply_batch(&CommandBatch::new(vec![thread(12.)]))
        .unwrap();
    let old_exact = exact(&document.current(), 20., true);
    let old = generate_hole_callouts(&document.current(), &old_exact, &annotated_sheet())
        .unwrap()
        .sheet;
    document
        .apply_batch(&CommandBatch::new(vec![thread(10.)]))
        .unwrap();
    let changed = exact(&document.current(), 20., true);
    assert_eq!(
        project_orthographic_drawing(&document.current(), &changed, &old).unwrap_err(),
        DrawingError::AnnotationSourceLost
    );
    let regenerated = generate_hole_callouts(&document.current(), &changed, &old).unwrap();
    assert!(
        regenerated.sheet.notes()[1]
            .text_template()
            .contains("THREAD DEPTH 10")
    );
    let incomplete = exact(&document.current(), 20., false);
    assert_eq!(
        project_orthographic_drawing(&document.current(), &incomplete, &old).unwrap_err(),
        DrawingError::AnnotationSourceLost
    );
    let removed = generate_hole_callouts(&document.current(), &incomplete, &old).unwrap();
    assert_eq!(removed.sheet.notes(), annotated_sheet().notes());
    assert!(
        removed
            .diagnostics
            .iter()
            .any(|d| d.kind == HoleCalloutDiagnosticKind::RemovedOrChangedSource)
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetHoleThread {
            id: CUT,
            thread: None,
        }]))
        .unwrap();
    let plain = generate_hole_callouts(
        &document.current(),
        &exact(&document.current(), 20., true),
        &old,
    )
    .unwrap();
    assert!(
        plain.sheet.notes()[1]
            .text_template()
            .starts_with("BORE Ø6.8")
    );
    // Cylinder span, not the cutter's 20 mm travel, determines the plain depth.
    let shorter = generate_hole_callouts(
        &document.current(),
        &exact(&document.current(), 16., true),
        &plain.sheet,
    )
    .unwrap();
    assert!(
        shorter.sheet.notes()[1]
            .text_template()
            .contains("DEPTH 16 mm")
    );
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &exact(&document.current(), 16., true),
            &plain.sheet
        )
        .unwrap_err(),
        DrawingError::AnnotationSourceLost
    );
}
