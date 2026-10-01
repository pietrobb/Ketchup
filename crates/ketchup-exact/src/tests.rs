use super::*;

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "{actual} != {expected}"
    );
}

#[test]
fn axial_tool_sweeps_are_continuous_exact_solids_for_line_arc_and_plunge() {
    let backend = ExactBackend::new();
    let line = backend
        .sweep_axial_tool(AxialToolSweepSpec {
            motion: AxialToolMotion::Line {
                start_mm: [0.0, 0.0, 1.0],
                end_mm: [10.0, 0.0, 1.0],
            },
            radius_mm: 2.0,
            axial_length_mm: 5.0,
        })
        .unwrap();
    assert_eq!(line.body.topology.solid_count, 1);
    assert_close(
        line.body.topology.volume_mm3,
        (40.0 + 4.0 * std::f64::consts::PI) * 5.0,
    );
    assert_close(line.body.topology.bounds_mm.min.x, -2.0);
    assert_close(line.body.topology.bounds_mm.max.x, 12.0);

    let plunge = backend
        .sweep_axial_tool(AxialToolSweepSpec {
            motion: AxialToolMotion::Line {
                start_mm: [3.0, 4.0, -5.0],
                end_mm: [3.0, 4.0, 0.0],
            },
            radius_mm: 2.0,
            axial_length_mm: 10.0,
        })
        .unwrap();
    assert_close(
        plunge.body.topology.volume_mm3,
        4.0 * std::f64::consts::PI * 15.0,
    );
    assert_close(plunge.body.topology.bounds_mm.min.z, -5.0);
    assert_close(plunge.body.topology.bounds_mm.max.z, 10.0);

    let arc = backend
        .sweep_axial_tool(AxialToolSweepSpec {
            motion: AxialToolMotion::Arc {
                start_mm: [10.0, 0.0, 2.0],
                end_mm: [0.0, 10.0, 2.0],
                center_mm: [0.0, 0.0, 2.0],
                clockwise: false,
            },
            radius_mm: 2.0,
            axial_length_mm: 5.0,
        })
        .unwrap();
    assert_eq!(arc.body.topology.solid_count, 1);
    assert_close(
        arc.body.topology.volume_mm3,
        24.0 * std::f64::consts::PI * 5.0,
    );
    assert_eq!(
        backend
            .sweep_axial_tool(AxialToolSweepSpec {
                motion: AxialToolMotion::Line {
                    start_mm: [0.0, 0.0, 0.0],
                    end_mm: [1.0, 0.0, 1.0],
                },
                radius_mm: 1.0,
                axial_length_mm: 5.0,
            })
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

fn planar_rectangle(min: [f64; 2], max: [f64; 2]) -> PlanarProfileLoop {
    PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: min,
            end_mm: [max[0], min[1]],
        },
        PlanarProfileSegment::Line {
            start_mm: [max[0], min[1]],
            end_mm: max,
        },
        PlanarProfileSegment::Line {
            start_mm: max,
            end_mm: [min[0], max[1]],
        },
        PlanarProfileSegment::Line {
            start_mm: [min[0], max[1]],
            end_mm: min,
        },
    ])
}

#[test]
fn native_code_reads_the_tolerances_of_ketchup_tolerance() {
    let native = native_tolerances();
    assert_eq!(
        [
            native.linear_mm,
            native.rounding,
            native.accumulated_rounding,
            native.approximation,
            native.negligible,
        ],
        [
            DEFAULT_LINEAR_TOLERANCE_MM,
            ROUNDING,
            ACCUMULATED_ROUNDING,
            APPROXIMATION,
            NEGLIGIBLE,
        ]
    );
    // The native knit check reads the same linear tolerance as the Rust one: a value
    // just below it is refused, the tolerance itself is accepted.
    let backend = ExactBackend::new();
    let left = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [10.0, 10.0]))
        .unwrap();
    let right = backend
        .planar_surface_profile(&planar_rectangle([10.0, 0.0], [20.0, 10.0]))
        .unwrap();
    let native_knit = |tolerance_mm: f64| {
        let compound = ffi::combine_surfaces_native(
            left.body.native.as_ref().unwrap(),
            right.body.native.as_ref().unwrap(),
        );
        ffi::knit_surface_compound_native(compound.as_ref().unwrap(), tolerance_mm, false)
            .as_ref()
            .unwrap()
            .status_code()
    };
    assert_eq!(native_knit(DEFAULT_LINEAR_TOLERANCE_MM * 0.5), 1);
    assert_eq!(native_knit(DEFAULT_LINEAR_TOLERANCE_MM), 0);
}

#[test]
fn surface_knit_joins_connected_faces_with_explicit_tolerance() {
    let backend = ExactBackend::new();
    let left = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [10.0, 10.0]))
        .unwrap();
    let right = backend
        .planar_surface_profile(&planar_rectangle([10.0, 0.0], [20.0, 10.0]))
        .unwrap();
    let near = backend
        .planar_surface_profile(&planar_rectangle([10.0005, 0.0], [20.0005, 10.0]))
        .unwrap();

    let knitted = backend
        .knit_surfaces(&[&left.body, &right.body], 1.0e-7, false)
        .unwrap();
    assert_eq!(knitted.body.topology.solid_count, 0);
    assert_eq!(knitted.body.topology.shell_count, 1);
    assert_eq!(knitted.body.topology.face_count, 2);
    assert_close(
        knitted
            .body
            .topology
            .faces
            .iter()
            .map(|face| face.area_mm2)
            .sum(),
        200.0,
    );
    assert!(
        knitted
            .topology_history
            .iter()
            .all(|entry| { entry.semantic_role.as_deref() == Some("surface_knit.face") })
    );

    let tolerance_join = backend
        .knit_surfaces(&[&left.body, &near.body], 0.001, false)
        .unwrap();
    assert_eq!(tolerance_join.body.topology.shell_count, 1);
    assert!(
        backend
            .knit_surfaces(&[&left.body, &near.body], 0.0001, false)
            .is_err()
    );
    assert!(
        backend
            .knit_surfaces(&[&left.body, &right.body], 1.0e-7, true)
            .is_err()
    );
    assert_eq!(
        backend
            .knit_surfaces(&[&left.body, &left.body], 1.0e-7, false)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn surface_thicken_creates_one_exact_solid_with_explicit_side_policy() {
    let backend = ExactBackend::new();
    let surface = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [10.0, 20.0]))
        .unwrap();

    let inward = backend
        .thicken_surface(&surface.body, 2.0, ShellDirection::Inward)
        .unwrap();
    let outward = backend
        .thicken_surface(&surface.body, 2.0, ShellDirection::Outward)
        .unwrap();
    let symmetric = backend
        .thicken_surface(&surface.body, 2.0, ShellDirection::Symmetric)
        .unwrap();
    for result in [&inward, &outward, &symmetric] {
        assert_eq!(result.body.topology.solid_count, 1);
        assert_close(result.body.topology.volume_mm3, 400.0);
        assert!(
            result
                .topology_history
                .iter()
                .all(|entry| { entry.semantic_role.as_deref() == Some("surface_thicken.face") })
        );
    }
    assert_close(inward.body.topology.bounds_mm.min.z, -2.0);
    assert_close(inward.body.topology.bounds_mm.max.z, 0.0);
    assert_close(outward.body.topology.bounds_mm.min.z, 0.0);
    assert_close(outward.body.topology.bounds_mm.max.z, 2.0);
    assert_close(symmetric.body.topology.bounds_mm.min.z, -1.0);
    assert_close(symmetric.body.topology.bounds_mm.max.z, 1.0);
}

#[test]
fn surface_thicken_handles_curved_loft_and_rejects_collapsed_offset() {
    let backend = ExactBackend::new();
    let frame = ketchup_geometry::linalg::Frame::WORLD;
    let circle = || {
        FramedLoftProfile::Planar(PlanarProfileLoop::Circle {
            center_mm: [0.0, 0.0],
            radius_mm: 10.0,
        })
    };
    let surface = backend
        .loft_framed_surface(
            &FramedLoftSpec {
                sections: vec![
                    FramedLoftSection {
                        elevation_mm: 0.0,
                        frame,
                        profile: circle(),
                    },
                    FramedLoftSection {
                        elevation_mm: 20.0,
                        frame,
                        profile: circle(),
                    },
                ],
            },
            None,
            LoftSurfaceContinuity::Position,
        )
        .unwrap();
    assert_eq!(surface.body.topology.solid_count, 0);

    let thickened = backend
        .thicken_surface(&surface.body, 2.0, ShellDirection::Outward)
        .unwrap();
    assert_eq!(thickened.body.topology.solid_count, 1);
    assert!((thickened.body.topology.volume_mm3 - 880.0 * std::f64::consts::PI).abs() <= 1.0e-5);
    assert!(
        backend
            .thicken_surface(&surface.body, 20.0, ShellDirection::Inward)
            .is_err()
    );
}

#[test]
fn surface_thicken_rejects_zero_thickness_and_solid_inputs() {
    let backend = ExactBackend::new();
    let surface = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [10.0, 20.0]))
        .unwrap();
    let solid = backend
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 10.0,
                y: 20.0,
                z: 5.0,
            },
        })
        .unwrap();

    assert_eq!(
        backend
            .thicken_surface(&surface.body, 0.0, ShellDirection::Outward)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    assert_eq!(
        backend
            .thicken_surface(&solid.body, 2.0, ShellDirection::Outward)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn surface_knit_creates_a_solid_only_from_a_closed_watertight_shell() {
    let backend = ExactBackend::new();
    let bottom = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [10.0, 20.0]))
        .unwrap();
    let top = backend
        .transform_body(
            &bottom.body,
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 30.0, 0.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
    let xz = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [10.0, 30.0]))
        .unwrap();
    let front = backend
        .transform_body(
            &xz.body,
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
    let back = backend
        .transform_body(
            &xz.body,
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 20.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
    let yz = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [20.0, 30.0]))
        .unwrap();
    let left = backend
        .transform_body(
            &yz.body,
            &[
                0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
    let right = backend
        .transform_body(
            &yz.body,
            &[
                0.0, 0.0, 1.0, 10.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
    let surfaces = [
        &bottom.body,
        &top.body,
        &front.body,
        &back.body,
        &left.body,
        &right.body,
    ];

    let shell = backend.knit_surfaces(&surfaces, 1.0e-7, false).unwrap();
    assert_eq!(shell.body.topology.solid_count, 0);
    assert_eq!(shell.body.topology.shell_count, 1);
    assert_eq!(shell.body.topology.face_count, 6);
    assert_close(shell.body.topology.volume_mm3, 0.0);
    assert_close(
        shell
            .body
            .topology
            .faces
            .iter()
            .map(|face| face.area_mm2)
            .sum(),
        2_200.0,
    );

    let solid = backend.knit_surfaces(&surfaces, 1.0e-7, true).unwrap();
    assert_eq!(solid.body.topology.solid_count, 1);
    assert_eq!(solid.body.topology.shell_count, 1);
    assert_eq!(solid.body.topology.face_count, 6);
    assert_close(solid.body.topology.volume_mm3, 6_000.0);

    assert!(backend.knit_surfaces(&surfaces[..5], 1.0e-7, true).is_err());
}

#[test]
fn planar_surface_trim_and_extend_are_exact_non_solid_changes() {
    let backend = ExactBackend::new();
    let target = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [40.0, 25.0]))
        .unwrap();
    let cutter = backend
        .planar_surface_profile(&planar_rectangle([10.0, 5.0], [30.0, 20.0]))
        .unwrap();

    let trimmed = backend.trim_surface(&target.body, &cutter.body).unwrap();
    assert_eq!(trimmed.body.topology.face_count, 1);
    assert_eq!(trimmed.body.topology.wire_count, 1);
    assert_eq!(trimmed.body.topology.solid_count, 0);
    assert_close(trimmed.body.topology.volume_mm3, 0.0);
    assert_close(trimmed.body.topology.faces[0].area_mm2, 300.0);
    assert!(
        trimmed
            .topology_history
            .iter()
            .any(|entry| entry.semantic_role.as_deref() == Some("surface_trim.face"))
    );

    let extended = backend.extend_planar_surface(&target.body, 5.0).unwrap();
    assert_eq!(extended.body.topology.face_count, 1);
    assert_eq!(extended.body.topology.wire_count, 1);
    assert_eq!(extended.body.topology.solid_count, 0);
    assert_close(extended.body.topology.volume_mm3, 0.0);
    assert_close(extended.body.topology.faces[0].area_mm2, 1_750.0);
    assert!(
        extended
            .topology_history
            .iter()
            .any(|entry| entry.semantic_role.as_deref() == Some("surface_extend.face"))
    );
}

#[test]
fn surface_trim_and_extend_fail_closed_on_invalid_or_unchanged_inputs() {
    let backend = ExactBackend::new();
    let target = backend
        .planar_surface_profile(&planar_rectangle([0.0, 0.0], [40.0, 25.0]))
        .unwrap();
    let disjoint = backend
        .planar_surface_profile(&planar_rectangle([50.0, 50.0], [60.0, 60.0]))
        .unwrap();
    let containing = backend
        .planar_surface_profile(&planar_rectangle([-5.0, -5.0], [45.0, 30.0]))
        .unwrap();
    let ambiguous = backend
        .planar_surface_profile(&PlanarProfileLoop::Segments(vec![
            PlanarProfileSegment::Line {
                start_mm: [0.0, -10.0],
                end_mm: [40.0, -10.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [40.0, -10.0],
                end_mm: [40.0, 5.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [40.0, 5.0],
                end_mm: [30.0, 5.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [30.0, 5.0],
                end_mm: [30.0, 0.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [30.0, 0.0],
                end_mm: [10.0, 0.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [10.0, 0.0],
                end_mm: [10.0, 5.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [10.0, 5.0],
                end_mm: [0.0, 5.0],
            },
            PlanarProfileSegment::Line {
                start_mm: [0.0, 5.0],
                end_mm: [0.0, -10.0],
            },
        ]))
        .unwrap();
    let solid = backend
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 10.0,
                y: 10.0,
                z: 10.0,
            },
        })
        .unwrap();

    assert!(backend.trim_surface(&target.body, &disjoint.body).is_err());
    assert!(
        backend
            .trim_surface(&target.body, &containing.body)
            .is_err()
    );
    assert!(backend.trim_surface(&target.body, &ambiguous.body).is_err());
    assert_eq!(
        backend
            .trim_surface(&solid.body, &target.body)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    assert_eq!(
        backend
            .extend_planar_surface(&target.body, 0.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    assert_eq!(
        backend
            .extend_planar_surface(&solid.body, 5.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn planar_surface_profile_is_one_exact_face_and_not_a_solid() {
    let profile = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [40.0, 0.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [40.0, 0.0],
            end_mm: [40.0, 25.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [40.0, 25.0],
            end_mm: [0.0, 25.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [0.0, 25.0],
            end_mm: [0.0, 0.0],
        },
    ]);
    let output = ExactBackend::new()
        .planar_surface_profile(&profile)
        .unwrap();

    assert_eq!(output.body.topology.face_count, 1);
    assert_eq!(output.body.topology.solid_count, 0);
    assert_close(output.body.topology.volume_mm3, 0.0);
    assert_close(output.body.topology.faces[0].area_mm2, 1_000.0);
    assert_eq!(output.topology_history.len(), 1);
    assert_eq!(
        output.topology_history[0].semantic_role.as_deref(),
        Some("planar_surface.face")
    );
}

#[test]
fn step_xde_manifest_reads_a_real_independent_step_part_and_refuses_bad_indices() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let path = repository.join("corpora/r0/step/self-authored-box.step");
    let backend = ExactBackend::new();
    let manifest = backend.step_xde_manifest(path.to_str().unwrap()).unwrap();

    assert_eq!(manifest.parts.len(), 1);
    assert_eq!(manifest.nodes.len(), 1);
    assert_eq!(manifest.nodes[0].part_index, Some(0));
    assert_eq!(manifest.nodes[0].parent_id, None);
    assert!(transform_is_rigid(&manifest.nodes[0].transform));
    let part = backend
        .import_step_xde_part(path.to_str().unwrap(), 0)
        .unwrap();
    let whole = backend.import_step(path.to_str().unwrap()).unwrap();
    assert_close(
        part.body.topology.volume_mm3,
        whole.body.topology.volume_mm3,
    );
    assert_eq!(
        backend
            .import_step_xde_part(path.to_str().unwrap(), 1)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn step_xde_manifest_preserves_real_nested_repeated_assembly_metadata_and_parts() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let path = repository.join("corpora/r0/step/independent-xde-assembly.step");
    let backend = ExactBackend::new();
    let manifest = backend.step_xde_manifest(path.to_str().unwrap()).unwrap();

    assert_eq!(manifest.parts.len(), 2);
    assert_eq!(manifest.nodes.len(), 5);
    assert!(manifest.parts.iter().all(|part| part.name_from_source));
    assert!(manifest.nodes.iter().all(|node| node.name_from_source));
    let root = manifest
        .nodes
        .iter()
        .find(|node| node.name == "Fixture root assembly")
        .unwrap();
    let carriage = manifest
        .nodes
        .iter()
        .find(|node| node.name == "Carriage nested instance")
        .unwrap();
    let left = manifest
        .nodes
        .iter()
        .find(|node| node.name == "Bracket left instance")
        .unwrap();
    let right = manifest
        .nodes
        .iter()
        .find(|node| node.name == "Bracket right instance")
        .unwrap();
    let pin = manifest
        .nodes
        .iter()
        .find(|node| node.name == "Pin root instance")
        .unwrap();
    assert_eq!(root.parent_id, None);
    assert_eq!(root.part_index, None);
    assert_eq!(carriage.parent_id, Some(root.id));
    assert_eq!(carriage.part_index, None);
    assert_eq!(left.parent_id, Some(carriage.id));
    assert_eq!(right.parent_id, Some(carriage.id));
    assert_eq!(left.part_index, right.part_index);
    assert_eq!(pin.parent_id, Some(root.id));
    assert_ne!(pin.part_index, left.part_index);
    assert_eq!(left.color, Some([255, 0, 0]));
    assert_eq!(right.color, Some([0, 255, 0]));
    assert_eq!(pin.color, Some([0, 0, 255]));
    assert_close(carriage.transform[7], 50.0);
    assert_close(right.transform[3], 40.0);
    assert_close(pin.transform[3], 20.0);
    assert_close(pin.transform[7], 10.0);
    assert_close(pin.transform[11], 5.0);
    assert!(
        manifest
            .nodes
            .iter()
            .all(|node| transform_is_rigid(&node.transform))
    );

    let bracket = backend
        .import_step_xde_part(path.to_str().unwrap(), left.part_index.unwrap())
        .unwrap();
    let pin_body = backend
        .import_step_xde_part(path.to_str().unwrap(), pin.part_index.unwrap())
        .unwrap();
    assert_close(bracket.body.topology.volume_mm3, 6_000.0);
    assert_close(
        pin_body.body.topology.volume_mm3,
        std::f64::consts::PI * 375.0,
    );
}

#[test]
fn iges_xde_roundtrip_reports_actual_assembly_names_colors_and_transforms() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let box_part = repository.join("corpora/r0/step/self-authored-box.step");
    let output = std::env::temp_dir().join(format!(
        "ketchup-iges-xde-roundtrip-{}.iges",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&output);
    let backend = ExactBackend::new();
    let parts = vec![StepXdeExportPart {
        path: box_part.to_string_lossy().into_owned(),
        name: "Shared box".to_owned(),
    }];
    let transform = |x: f64, y: f64, z: f64| {
        [
            1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0,
        ]
    };
    let nodes = vec![
        StepXdeExportNode {
            parent_id: None,
            part_index: None,
            name: "Root assembly".to_owned(),
            color: None,
            transform: transform(0.0, 0.0, 0.0),
        },
        StepXdeExportNode {
            parent_id: Some(0),
            part_index: None,
            name: "Nested assembly".to_owned(),
            color: None,
            transform: transform(0.0, 50.0, 0.0),
        },
        StepXdeExportNode {
            parent_id: Some(1),
            part_index: Some(0),
            name: "Box left".to_owned(),
            color: Some([255, 0, 0]),
            transform: transform(0.0, 0.0, 0.0),
        },
        StepXdeExportNode {
            parent_id: Some(1),
            part_index: Some(0),
            name: "Box right".to_owned(),
            color: Some([0, 255, 0]),
            transform: transform(40.0, 0.0, 0.0),
        },
        StepXdeExportNode {
            parent_id: Some(0),
            part_index: Some(0),
            name: "Box third".to_owned(),
            color: Some([0, 0, 255]),
            transform: transform(20.0, 10.0, 5.0),
        },
    ];

    backend
        .export_iges_xde_assembly(&parts, &nodes, output.to_str().unwrap())
        .unwrap();
    let manifest = backend.iges_xde_manifest(output.to_str().unwrap()).unwrap();
    assert_eq!(manifest.parts.len(), 3, "{manifest:#?}");
    assert_eq!(manifest.nodes.len(), 3, "{manifest:#?}");
    assert!(manifest.nodes.iter().all(|node| node.parent_id.is_none()));
    assert_eq!(
        manifest
            .nodes
            .iter()
            .filter_map(|node| node.color)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([[255, 0, 0], [0, 255, 0], [0, 0, 255]])
    );
    assert!(
        manifest
            .nodes
            .iter()
            .all(|node| node.transform == transform(0.0, 0.0, 0.0))
    );
    assert_eq!(
        manifest
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Box left", "Box right", "Box third"]
    );
    let bodies = (0..manifest.parts.len())
        .map(|index| {
            backend
                .import_iges_xde_part(output.to_str().unwrap(), index as u32)
                .unwrap()
                .body
        })
        .collect::<Vec<_>>();
    assert!(bodies.iter().all(|body| body.topology.volume_mm3 > 0.0));
    assert_close(
        bodies[1].topology.bounds_mm.min.x - bodies[0].topology.bounds_mm.min.x,
        40.0,
    );
    assert_close(
        bodies[1].topology.bounds_mm.min.y - bodies[0].topology.bounds_mm.min.y,
        0.0,
    );
    assert_close(
        bodies[2].topology.bounds_mm.min.x - bodies[0].topology.bounds_mm.min.x,
        20.0,
    );
    assert_close(
        bodies[2].topology.bounds_mm.min.y - bodies[0].topology.bounds_mm.min.y,
        -40.0,
    );
    assert_close(
        bodies[2].topology.bounds_mm.min.z - bodies[0].topology.bounds_mm.min.z,
        5.0,
    );
    std::fs::remove_file(output).unwrap();
}

#[test]
fn step_xde_manifest_parser_rejects_non_rigid_and_forward_parent_payloads() {
    let identity = [
        1.0_f64, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let fields = identity
        .iter()
        .map(|value| format!("{:016x}", value.to_bits()))
        .collect::<Vec<_>>()
        .join("\t");
    let valid = format!(
        "KETCHUP_STEP_XDE_V1\nP\t0\t50617274\t1\t-\nN\t0\t-1\t0\t496e7374616e6365\t1\tff0000\t{fields}\n"
    );
    assert!(parse_step_xde_manifest(&valid).is_ok());
    assert_eq!(
        parse_step_xde_manifest(&valid.replacen("N\t0\t-1", "N\t0\t0", 1)),
        Err(StepXdeManifestError::Malformed(format!(
            "node parent (must be -1 or an earlier node) \"0\" in line {:?} is invalid",
            valid
                .lines()
                .nth(2)
                .unwrap()
                .replacen("N\t0\t-1", "N\t0\t0", 1)
        )))
    );
    let scaled = valid.replacen("3ff0000000000000", "4000000000000000", 1);
    assert_eq!(
        parse_step_xde_manifest(&scaled),
        Err(StepXdeManifestError::Malformed(
            "node 0 transform is not rigid".to_owned()
        ))
    );
    let unreadable = valid.replacen("3ff0000000000000", "3ff000000000000x", 1);
    let error = parse_step_xde_manifest(&unreadable)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("transform entry (invalid digit found in string) \"3ff000000000000x\""),
        "{error}"
    );
}

#[test]
fn step_xde_export_rejects_invalid_hierarchy_transform_and_unused_parts_before_writing() {
    let backend = ExactBackend::new();
    let output = std::env::temp_dir().join(format!(
        "ketchup-xde-invalid-{}-must-not-exist.step",
        std::process::id()
    ));
    assert!(!output.exists());
    let parts = vec![StepXdeExportPart {
        path: "part.step".to_owned(),
        name: "Part".to_owned(),
    }];
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let node = StepXdeExportNode {
        parent_id: None,
        part_index: Some(0),
        name: "Instance".to_owned(),
        color: None,
        transform: identity,
    };
    let mut forward_parent = node.clone();
    forward_parent.parent_id = Some(0);
    assert_eq!(
        backend
            .export_step_xde_assembly(&parts, &[forward_parent], output.to_str().unwrap(),)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    let mut scaled = node.clone();
    scaled.transform[0] = 2.0;
    assert_eq!(
        backend
            .export_step_xde_assembly(&parts, &[scaled], output.to_str().unwrap())
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    let mut unused_parts = parts.clone();
    unused_parts.push(StepXdeExportPart {
        path: "unused.step".to_owned(),
        name: "Unused".to_owned(),
    });
    assert_eq!(
        backend
            .export_step_xde_assembly(&unused_parts, &[node], output.to_str().unwrap())
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    assert!(!output.exists());
}

#[test]
fn spatial_sweep_validation_accepts_canonical_segment_count_bounds() {
    let path = |count: usize| {
        (0..count)
            .map(|index| SpatialProfileSegment::Line {
                start_mm: [index as f64, 0.0, 0.0],
                end_mm: [index as f64 + 1.0, 0.0, 0.0],
            })
            .collect::<Vec<_>>()
    };

    assert!(validate_spatial_sweep_path(&path(1), "sweep_spatial_profile", "unit").is_ok());
    assert!(validate_spatial_sweep_path(&path(64), "sweep_spatial_profile", "unit").is_ok());
    assert_eq!(
        validate_spatial_sweep_path(&[], "sweep_spatial_profile", "unit")
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    assert_eq!(
        validate_spatial_sweep_path(&path(65), "sweep_spatial_profile", "unit")
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
}

#[test]
fn spatial_sweep_validation_separates_adjacent_segments_at_the_join_tangent() {
    let straight = [
        SpatialProfileSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [10.0, 0.0, 0.0],
        },
        SpatialProfileSegment::Line {
            start_mm: [10.0, 0.0, 0.0],
            end_mm: [20.0, 0.0, 0.0],
        },
    ];
    let circular = [
        SpatialProfileSegment::CircularArc {
            start_mm: [10.0, 0.0, 0.0],
            end_mm: [0.0, 10.0, 0.0],
            center_mm: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
        SpatialProfileSegment::CircularArc {
            start_mm: [0.0, 10.0, 0.0],
            end_mm: [-10.0, 0.0, 0.0],
            center_mm: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
    ];
    let non_coplanar_mixed = [
        SpatialProfileSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [10.0, 0.0, 0.0],
        },
        SpatialProfileSegment::CircularArc {
            start_mm: [10.0, 0.0, 0.0],
            end_mm: [20.0, 10.0, 0.0],
            center_mm: [10.0, 10.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: [20.0, 10.0, 0.0],
            control_1_mm: [20.0, 15.0, 0.0],
            control_2_mm: [20.0, 20.0, 5.0],
            end_mm: [20.0, 20.0, 10.0],
        },
    ];
    for path in [&straight[..], &circular[..], &non_coplanar_mixed[..]] {
        validate_spatial_sweep_path(path, "sweep_spatial_profile", "unit")
            .expect("strictly separated C1 joins must remain valid");
    }

    let adjacent_self_intersection = [
        SpatialProfileSegment::CubicBezier {
            start_mm: [0.0, 0.0, 0.0],
            control_1_mm: [10.0, -5.0, 0.0],
            control_2_mm: [9.0, 10.0, 0.0],
            end_mm: [10.0, 10.0, 0.0],
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: [10.0, 10.0, 0.0],
            control_1_mm: [11.0, 10.0, 0.0],
            control_2_mm: [-10.0, -20.0, 0.0],
            end_mm: [20.0, 0.0, 0.0],
        },
    ];
    let error =
        validate_spatial_sweep_path(&adjacent_self_intersection, "sweep_spatial_profile", "unit")
            .expect_err("C1 cubics that cross again must fail exact preflight");
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
    assert!(error.diagnostic.contains("self-intersect"));
}

#[test]
fn spatial_sweep_validation_accepts_distinct_near_full_circle_endpoints() {
    let angle = -5.0e-9_f64;
    let path = [SpatialProfileSegment::CircularArc {
        start_mm: [10.0, 0.0, 0.0],
        end_mm: [10.0 * angle.cos(), 10.0 * angle.sin(), 0.0],
        center_mm: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        clockwise: false,
    }];

    let metrics = validate_spatial_sweep_path(&path, "sweep_spatial_profile", "unit")
        .expect("distinct endpoints and a bounded near-full arc length must be accepted");
    assert!(metrics[0].0 > 60.0);
}

#[test]
fn spatial_sweep_validation_accepts_nonintersecting_aabb_overlap() {
    let path = [
        SpatialProfileSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [30.0, 0.0, 0.0],
        },
        SpatialProfileSegment::CircularArc {
            start_mm: [30.0, 0.0, 0.0],
            end_mm: [40.0, 10.0, 0.0],
            center_mm: [30.0, 10.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
        SpatialProfileSegment::CircularArc {
            start_mm: [40.0, 10.0, 0.0],
            end_mm: [30.0, 20.0, 0.0],
            center_mm: [30.0, 10.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
    ];
    validate_spatial_sweep_path(&path, "sweep_spatial_profile", "unit")
        .expect("overlapping world-axis bounds alone do not prove a spatial intersection");
}

#[test]
fn segment_request_digest_encoding_is_frozen() {
    // Stored exact provenance holds digests of this encoding, so it stays as it
    // was when segments still crossed the native boundary as float arrays.
    assert_eq!(
        planar_segment_digest_values(&PlanarProfileSegment::Line {
            start_mm: [1.0, 2.0],
            end_mm: [3.0, 4.0],
        }),
        [0.0, 1.0, 2.0, 3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        planar_segment_digest_values(&PlanarProfileSegment::CircularArc {
            start_mm: [1.0, 2.0],
            end_mm: [3.0, 4.0],
            center_mm: [5.0, 6.0],
            clockwise: true,
        }),
        [1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        planar_segment_digest_values(&PlanarProfileSegment::CubicBezier {
            start_mm: [1.0, 2.0],
            control_1_mm: [5.0, 6.0],
            control_2_mm: [7.0, 8.0],
            end_mm: [3.0, 4.0],
        }),
        [2.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 0.0]
    );
    assert_eq!(
        circle_digest_values([7.0, 8.0], 9.0, true),
        [3.0, 7.0, 8.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        spatial_segment_digest_values(&SpatialProfileSegment::CircularArc {
            start_mm: [1.0, 2.0, 3.0],
            end_mm: [4.0, 5.0, 6.0],
            center_mm: [7.0, 8.0, 9.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        }),
        [
            11.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 0.0, 0.0, 1.0, 0.0
        ]
    );
}

#[test]
fn native_segments_carry_their_kind_as_a_tag_and_name_every_value() {
    let point = |x, y, z| ffi::NativePoint { x, y, z };
    let arc = native_planar_segment(&PlanarProfileSegment::CircularArc {
        start_mm: [1.0, 2.0],
        end_mm: [3.0, 4.0],
        center_mm: [5.0, 6.0],
        clockwise: true,
    });
    assert!(arc.kind == ffi::NativeSegmentKind::CircularArc);
    assert_eq!(arc.start, point(1.0, 2.0, 0.0));
    assert_eq!(arc.end, point(3.0, 4.0, 0.0));
    assert_eq!(arc.center, point(5.0, 6.0, 0.0));
    assert_eq!(arc.normal, point(0.0, 0.0, 1.0));
    assert!(arc.clockwise);

    let bezier = native_planar_segment(&PlanarProfileSegment::CubicBezier {
        start_mm: [1.0, 2.0],
        control_1_mm: [5.0, 6.0],
        control_2_mm: [7.0, 8.0],
        end_mm: [3.0, 4.0],
    });
    assert!(bezier.kind == ffi::NativeSegmentKind::CubicBezier);
    assert_eq!(bezier.control_1, point(5.0, 6.0, 0.0));
    assert_eq!(bezier.control_2, point(7.0, 8.0, 0.0));
    assert_eq!(bezier.center, point(0.0, 0.0, 0.0));

    let hole = native_circle([7.0, 8.0], 9.0, true);
    assert!(hole.kind == ffi::NativeSegmentKind::Circle);
    assert_eq!(
        (hole.center, hole.radius, hole.clockwise),
        (point(7.0, 8.0, 0.0), 9.0, true)
    );

    let spatial = native_spatial_segment(&SpatialProfileSegment::CubicBezier {
        start_mm: [1.0, 2.0, 3.0],
        control_1_mm: [4.0, 5.0, 6.0],
        control_2_mm: [7.0, 8.0, 9.0],
        end_mm: [10.0, 11.0, 12.0],
    });
    assert!(spatial.kind == ffi::NativeSegmentKind::CubicBezier);
    assert_eq!(spatial.control_2, point(7.0, 8.0, 9.0));
    assert_eq!(spatial.end, point(10.0, 11.0, 12.0));
}

#[test]
fn canonical_planar_region_keeps_native_segments_aligned_with_the_request_digest() {
    let square = |start: usize| {
        let corners = [[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]];
        PlanarProfileLoop::Segments(
            (0..4)
                .map(|index| PlanarProfileSegment::Line {
                    start_mm: corners[(start + index) % 4],
                    end_mm: corners[(start + index + 1) % 4],
                })
                .collect(),
        )
    };
    let circle = PlanarProfileLoop::Circle {
        center_mm: [25.0, 40.0],
        radius_mm: 10.0,
    };
    let slot = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [60.0, 30.0],
            end_mm: [80.0, 30.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [80.0, 30.0],
            end_mm: [80.0, 50.0],
            center_mm: [80.0, 40.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [80.0, 50.0],
            end_mm: [60.0, 50.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [60.0, 50.0],
            end_mm: [60.0, 30.0],
            center_mm: [60.0, 40.0],
            clockwise: false,
        },
    ]);
    let region = |outer: &PlanarProfileLoop, holes: &[PlanarProfileLoop]| {
        flatten_planar_region(outer, holes, "unit", "unit")
            .unwrap()
            .canonicalize()
    };
    let canonical = region(&square(2), &[slot.clone(), circle.clone()]);
    let digest = canonical.digest_values();
    let native = canonical.native_segments();
    assert_eq!(canonical.loop_segment_counts(), [4, 4, 1]);
    assert_eq!(digest.len(), native.len() * 10);
    for (values, segment) in digest.chunks(10).zip(&native) {
        if segment.kind == ffi::NativeSegmentKind::Circle {
            assert_eq!(
                [values[1], values[2], values[3]],
                [segment.center.x, segment.center.y, segment.radius]
            );
        } else {
            assert_eq!(
                [values[1], values[2], values[3], values[4]],
                [
                    segment.start.x,
                    segment.start.y,
                    segment.end.x,
                    segment.end.y
                ]
            );
        }
        assert_eq!(values[9] != 0.0, segment.clockwise);
    }

    let reordered = region(&square(1), &[circle, slot]);
    assert_eq!(reordered.digest_values(), digest);
    assert_eq!(reordered.native_segments(), native);
}

#[test]
fn native_sources_receive_segments_as_tagged_structs() {
    let source_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![source_dir.join("include").join("ketchup_exact.hxx")];
    for entry in std::fs::read_dir(source_dir.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("cc" | "hxx")
        ) {
            sources.push(path);
        }
    }
    let float_kind_comparison = |line: &str| {
        line.match_indices("kind").any(|(index, _)| {
            let rest = line[index + 4..].trim_start();
            let Some(rest) = rest.strip_prefix("==").or_else(|| rest.strip_prefix("!=")) else {
                return false;
            };
            let literal = rest
                .trim_start()
                .split(|character: char| !(character.is_ascii_digit() || character == '.'))
                .next()
                .unwrap_or_default();
            literal.contains('.')
                && literal.starts_with(|character: char| character.is_ascii_digit())
        })
    };
    let mut violations = Vec::new();
    for entry in std::fs::read_dir(source_dir.join("src")).unwrap() {
        let path = entry.unwrap().path();
        // Review 2026-09-29 §10: no exact-kernel source grows back into a monolith.
        if std::fs::metadata(&path).unwrap().len() > 60 * 1024 {
            violations.push(format!("{} is over 60 KB", path.display()));
        }
    }
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            if float_kind_comparison(line)
                || line.to_ascii_lowercase().contains("stride")
                || (line.contains("rust::Slice<const double>") && line.contains("segments"))
            {
                violations.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn native_occt_exception_is_contained_by_the_facade() {
    let error = collect_output(
        ffi::exception_probe_native(),
        "exception_probe",
        "intentional",
        HistoryConfidence::None,
    )
    .expect_err("probe must become a typed boundary error");
    assert_eq!(error.code, GeometryErrorCode::BackendException);
    assert!(
        error
            .diagnostic
            .contains("intentional A0 exception-boundary probe")
    );
}
