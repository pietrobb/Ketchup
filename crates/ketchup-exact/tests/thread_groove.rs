use ketchup_exact::{
    BoxSpec, CircleExtrudeSpec, ExactBackend, ExactBodyBooleanOperation, ExactOpOutput,
    ExactPairRelation, GeometryErrorCode, Point3, Size3, ThreadGrooveSpec,
};
use ketchup_geometry::linalg::{Affine3, Mat3, Vec3};
use ketchup_geometry::thread_profile::{MetricThread, ThreadPhase, groove_area_per_length};
use std::collections::BTreeSet;
use std::f64::consts::PI;
use std::time::{Duration, Instant};

const M5: MetricThread = MetricThread {
    nominal_diameter_mm: 5.0,
    pitch_mm: 0.8,
};
const LENGTH_MM: f64 = 20.0;

/// A right-handed groove along +z through the origin, phase on +x, running
/// one pitch past both ends of `0..LENGTH_MM`.
fn groove(profile: Vec<[f64; 2]>) -> ThreadGrooveSpec {
    ThreadGrooveSpec {
        origin_mm: [0.0; 3],
        axis: [0.0, 0.0, 1.0],
        reference: [1.0, 0.0, 0.0],
        pitch_mm: M5.pitch_mm,
        start_mm: -M5.pitch_mm,
        end_mm: LENGTH_MM + M5.pitch_mm,
        left_handed: false,
        profile_mm: profile,
        trim_mm: None,
        crest_mm: None,
    }
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = work();
    (value, start.elapsed())
}

fn assert_one_valid_solid(output: &ExactOpOutput) {
    assert!(output.tolerance_report.shape_valid);
    assert!(output.tolerance_report.accepted_exact_solid);
    assert_eq!(output.body.topology.solid_count, 1);
}

fn swept_faces(output: &ExactOpOutput) -> usize {
    output
        .body
        .topology
        .faces
        .iter()
        .filter(|face| !matches!(face.surface_kind.as_str(), "plane" | "cylinder"))
        .count()
}

/// The roles `thread.side.*` the output's faces carry.
fn thread_roles(output: &ExactOpOutput) -> BTreeSet<String> {
    output
        .topology_history
        .iter()
        .filter(|record| record.output_face_ordinal.is_some())
        .filter_map(|record| record.semantic_role.clone())
        .filter(|role| role.starts_with("thread.side."))
        .collect()
}

/// The size of the STEP file of `output`, and how many B-spline surfaces
/// (the swept flanks) it writes.
fn step_export(backend: &ExactBackend, output: &ExactOpOutput, name: &str) -> (u64, usize) {
    let path =
        std::env::temp_dir().join(format!("ketchup-thread-{name}-{}.step", std::process::id()));
    backend
        .export_step(&output.body, path.to_str().unwrap())
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    std::fs::remove_file(&path).unwrap();
    (size, text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count())
}

fn assert_volume(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= expected * 1.0e-4,
        "volume {actual} mm³, profile gives {expected} mm³"
    );
}

#[test]
fn a_thread_groove_has_the_volume_of_its_profile() {
    let profile = M5
        .external_groove(M5.external_6g().unwrap().major_mm)
        .unwrap();
    let output = ExactBackend::new()
        .thread_groove(&groove(profile.clone()))
        .unwrap();
    assert_one_valid_solid(&output);
    let length = LENGTH_MM + 2.0 * M5.pitch_mm;
    assert_volume(
        output.body.topology.volume_mm3,
        length * groove_area_per_length(&profile, M5.pitch_mm, 0.0, 10.0),
    );
    assert_eq!(thread_roles(&output).len(), profile.len());
}

#[test]
fn a_modelled_m5_bolt_thread_is_one_solid_with_the_volume_of_its_profile() {
    let backend = ExactBackend::new();
    let crest = M5.external_6g().unwrap().major_mm / 2.0;
    let profile = M5.external_groove(2.0 * crest).unwrap();
    let shank = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [0.0, 0.0],
            radius_mm: crest,
            height_mm: LENGTH_MM,
        })
        .unwrap();
    let (bolt, time) = timed(|| backend.cut_thread(&shank.body, &groove(profile.clone())));
    let bolt = bolt.unwrap();
    assert_one_valid_solid(&bolt);
    let removed = groove_area_per_length(&profile, M5.pitch_mm, 0.0, crest);
    assert_volume(
        bolt.body.topology.volume_mm3,
        LENGTH_MM * (PI * crest * crest - removed),
    );
    // Root and both flanks; the groove's outer face lies outside the shank.
    assert_eq!(thread_roles(&bolt).len(), 3);
    let (step_bytes, step_surfaces) = step_export(&backend, &bolt, "bolt");
    assert!(step_surfaces >= 2, "STEP writes the helical flanks");
    println!(
        "M5x20 bolt: thread {time:?}, {} faces ({} swept), STEP {step_bytes} B, {step_surfaces} B-spline surfaces",
        bolt.body.topology.face_count,
        swept_faces(&bolt)
    );
}

#[test]
fn a_bolt_thread_turns_its_full_shank_down_and_ends_flat_at_the_head() {
    let backend = ExactBackend::new();
    // The library's bolt: a shank of the nominal diameter up to a head.
    let (head_mm, nominal) = (3.5, M5.nominal_diameter_mm / 2.0);
    let shank = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [0.0, 0.0],
            radius_mm: nominal,
            height_mm: LENGTH_MM,
        })
        .unwrap();
    let head = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -4.0,
                y: -4.0,
                z: LENGTH_MM,
            },
            size_mm: Size3 {
                x: 8.0,
                y: 8.0,
                z: head_mm,
            },
        })
        .unwrap();
    let bolt = backend
        .boolean_bodies(&shank.body, &head.body, ExactBodyBooleanOperation::Union)
        .unwrap();
    let crest = M5.external_6g().unwrap().major_mm / 2.0;
    let profile = M5.external_groove(2.0 * crest).unwrap();
    let pitch = M5.pitch_mm;
    let spec = ThreadGrooveSpec {
        start_mm: -2.0 * pitch,
        end_mm: LENGTH_MM + pitch,
        // Past the tip, flat at the head.
        trim_mm: Some([-pitch, LENGTH_MM]),
        crest_mm: Some([crest, nominal + pitch / 16.0]),
        ..groove(profile.clone())
    };
    let (threaded, time) = timed(|| backend.cut_thread(&bolt.body, &spec));
    let threaded = threaded.unwrap();
    assert_one_valid_solid(&threaded);
    let removed = groove_area_per_length(&profile, pitch, 0.0, crest);
    assert_volume(
        threaded.body.topology.volume_mm3,
        64.0 * head_mm + LENGTH_MM * (PI * crest * crest - removed),
    );
    let roles = threaded
        .topology_history
        .iter()
        .filter(|record| record.output_face_ordinal.is_some())
        .filter_map(|record| record.semantic_role.clone())
        .collect::<BTreeSet<_>>();
    for role in ["thread.crest", "thread.end"] {
        assert!(roles.contains(role), "{roles:?}");
    }
    println!(
        "M5x20 bolt with head, turned down and trimmed: {time:?}, {} faces",
        threaded.body.topology.face_count
    );

    // Trim and crest stay inside the helix, and crest needs a trim.
    let outside = ThreadGrooveSpec {
        trim_mm: Some([-3.0 * pitch, LENGTH_MM]),
        ..spec.clone()
    };
    let crest_alone = ThreadGrooveSpec {
        trim_mm: None,
        ..spec
    };
    for invalid in [outside, crest_alone] {
        assert_eq!(
            backend.cut_thread(&bolt.body, &invalid).unwrap_err().code,
            GeometryErrorCode::InvalidParameter
        );
    }
}

#[test]
fn a_modelled_m5_tapped_hole_is_one_solid_with_the_volume_of_its_profile() {
    let backend = ExactBackend::new();
    let bore = 4.2;
    let profile = M5.internal_groove(bore).unwrap();
    let block = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -6.0,
                y: -6.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 12.0,
                y: 12.0,
                z: LENGTH_MM,
            },
        })
        .unwrap();
    let drill = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [0.0, 0.0],
            radius_mm: bore / 2.0,
            height_mm: LENGTH_MM,
        })
        .unwrap();
    let drilled = backend
        .boolean_bodies(&block.body, &drill.body, ExactBodyBooleanOperation::Cut)
        .unwrap();
    let (tapped, time) = timed(|| backend.cut_thread(&drilled.body, &groove(profile.clone())));
    let tapped = tapped.unwrap();
    assert_one_valid_solid(&tapped);
    let removed = groove_area_per_length(&profile, M5.pitch_mm, bore / 2.0, 10.0);
    assert_volume(
        tapped.body.topology.volume_mm3,
        LENGTH_MM * (144.0 - PI * bore * bore / 4.0 - removed),
    );
    assert_eq!(thread_roles(&tapped).len(), 3);
    let (step_bytes, step_surfaces) = step_export(&backend, &tapped, "hole");
    assert!(step_surfaces >= 2, "STEP writes the helical flanks");
    println!(
        "M5 hole 20 deep: thread {time:?}, {} faces ({} swept), STEP {step_bytes} B, {step_surfaces} B-spline surfaces",
        tapped.body.topology.face_count,
        swept_faces(&tapped)
    );
}

/// The groove of `phase` running one pitch past both ends of the part's
/// stretch of axis from `from` to `to`.
fn phased(phase: &ThreadPhase, from: Vec3, to: Vec3, profile: Vec<[f64; 2]>) -> ThreadGrooveSpec {
    let [a, b] = [phase.offset_of(from), phase.offset_of(to)];
    ThreadGrooveSpec {
        origin_mm: phase.origin.to_array(),
        axis: phase.axis.to_array(),
        reference: phase.reference.to_array(),
        pitch_mm: M5.pitch_mm,
        start_mm: a.min(b) - M5.pitch_mm,
        end_mm: a.max(b) + M5.pitch_mm,
        left_handed: false,
        profile_mm: profile,
        trim_mm: None,
        crest_mm: None,
    }
}

#[test]
fn a_modelled_m5_bolt_meshes_with_its_tapped_hole_at_any_depth_and_turn() {
    let backend = ExactBackend::new();
    let (hole_x, hole_y, plate_mm, bore) = (3.0, -2.0, 10.0, 4.2);
    // The agreed phase of a tapped hole: its entry point, the axis into the
    // material and the part's x direction.
    let hole = ThreadPhase {
        origin: Vec3::new(hole_x, hole_y, plate_mm),
        axis: Vec3::new(0.0, 0.0, -1.0),
        reference: Vec3::X,
    };
    let block = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: hole_x - 6.0,
                y: hole_y - 6.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 12.0,
                y: 12.0,
                z: plate_mm,
            },
        })
        .unwrap();
    let drill = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [hole_x, hole_y],
            radius_mm: bore / 2.0,
            height_mm: plate_mm,
        })
        .unwrap();
    let drilled = backend
        .boolean_bodies(&block.body, &drill.body, ExactBodyBooleanOperation::Cut)
        .unwrap();
    let hole_spec = phased(
        &hole,
        hole.origin,
        Vec3::new(hole_x, hole_y, 0.0),
        M5.internal_groove(bore).unwrap(),
    );
    let tapped = backend.cut_thread(&drilled.body, &hole_spec).unwrap();
    assert_one_valid_solid(&tapped);

    // An M5x20 bolt along its own +z from its tip, screwed 7.3 mm deep (not
    // a whole number of pitches) and turned 37° about its axis.
    let crest = M5.external_6g().unwrap().major_mm / 2.0;
    let bolt_profile = M5.external_groove(2.0 * crest).unwrap();
    let depth = 7.3;
    let (sin, cos) = 37.0_f64.to_radians().sin_cos();
    let placement = Affine3 {
        linear: Mat3::from_rows([[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]]),
        translation: Vec3::new(hole_x, hole_y, plate_mm - depth),
    };
    let shank = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [0.0, 0.0],
            radius_mm: crest,
            height_mm: LENGTH_MM,
        })
        .unwrap();
    let (tip, head) = (Vec3::ZERO, Vec3::new(0.0, 0.0, LENGTH_MM));
    let screwed_bolt = |phase: &ThreadPhase| {
        let bolt = backend
            .cut_thread(&shank.body, &phased(phase, tip, head, bolt_profile.clone()))
            .unwrap();
        assert_one_valid_solid(&bolt);
        let placed = backend
            .transform_body(&bolt.body, &placement.to_row_major())
            .unwrap();
        (bolt, placed)
    };

    let in_phase = hole.in_part(placement).unwrap();
    let ((bolt, placed), time) = timed(|| screwed_bolt(&in_phase));
    let removed = groove_area_per_length(&bolt_profile, M5.pitch_mm, 0.0, crest);
    let expected = LENGTH_MM * (PI * crest * crest - removed);
    assert_volume(bolt.body.topology.volume_mm3, expected);
    let (pair, query_time) = timed(|| {
        backend
            .query_body_pair(&tapped.body, &placed.body, 0.0)
            .unwrap()
    });
    assert_eq!(pair.common_volume_mm3, 0.0, "{pair:?}");
    assert_eq!(pair.relation, ExactPairRelation::Separated, "{pair:?}");
    println!(
        "M5x20 bolt 7.3 deep, turned 37°: thread {time:?}, volume {:.4} mm³ (profile {expected:.4}), pair query {query_time:?}, {:?}, clearance {:.4} mm",
        bolt.body.topology.volume_mm3, pair.relation, pair.distance_mm
    );

    // The same bolt cut with its own phase instead of the hole's interferes,
    // and the same pair query reports it, so it does see a phase error.
    let own = ThreadPhase {
        origin: tip,
        axis: Vec3::Z,
        reference: Vec3::X,
    };
    let (_, wrong) = screwed_bolt(&own);
    let clash = backend
        .query_body_pair(&tapped.body, &wrong.body, 0.0)
        .unwrap();
    assert_eq!(clash.relation, ExactPairRelation::Penetrating, "{clash:?}");
    assert!(clash.common_volume_mm3 > 1.0, "{clash:?}");
    println!(
        "own phase instead: {:.3} mm³ overlap",
        clash.common_volume_mm3
    );
}

#[test]
fn thread_grooves_turn_either_way_and_refuse_overlapping_turns() {
    let backend = ExactBackend::new();
    let profile = M5
        .external_groove(M5.external_6g().unwrap().major_mm)
        .unwrap();
    let mut left = groove(profile.clone());
    left.left_handed = true;
    left.end_mm = 4.0;
    assert_one_valid_solid(&backend.thread_groove(&left).unwrap());

    let mut wide = groove(vec![[2.0, -0.4], [2.6, -0.4], [2.6, 0.4], [2.0, 0.4]]);
    wide.end_mm = 4.0;
    let error = backend.thread_groove(&wide).unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter);

    let mut tilted = groove(profile);
    tilted.reference = [0.6, 0.0, 0.8];
    let error = backend.thread_groove(&tilted).unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter);
}
