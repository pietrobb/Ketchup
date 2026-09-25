//! Narrow, exception-safe exact geometry boundary used by the A0 gate.

use std::fmt;

const BACKEND_FINGERPRINT: &str = env!("KETCHUP_OCCT_BUILD_FINGERPRINT");
const TOLERANCE_PROFILE: &str = "r0-v1:bbox=1e-6mm:volume_abs=1e-6mm3:volume_rel=1e-10";
const MIN_LENGTH_MM: f64 = 0.01;
const MAX_LENGTH_MM: f64 = 100_000.0;
const MAX_COORDINATE_MM: f64 = 1_000_000.0;
const PLANAR_SEGMENT_STRIDE: usize = 10;
const SPATIAL_SEGMENT_STRIDE: usize = 14;
const MIN_SWEEP_PATH_SEGMENT_LENGTH_MM: f64 = 1.0e-7;
const MAX_SWEEP_PATH_SEGMENTS: usize = 64;
pub const MAX_PLANAR_LOOP_SEGMENTS: usize = 64;
pub const MAX_PLANAR_REGION_HOLES: usize = 64;
pub const MAX_PLANAR_REGION_SEGMENTS: usize = 4_096;

#[must_use]
pub const fn backend_fingerprint() -> &'static str {
    BACKEND_FINGERPRINT
}

#[must_use]
pub const fn tolerance_profile() -> &'static str {
    TOLERANCE_PROFILE
}

#[allow(dead_code, unsafe_code)]
#[cxx::bridge(namespace = "ketchup::exact")]
mod ffi {
    struct NativePairQuery {
        status: u8,
        diagnostic: String,
        common_volume_mm3: f64,
        common_contact_area_mm2: f64,
        distance_mm: f64,
    }

    struct NativeTopologySummary {
        vertex_count: u32,
        edge_count: u32,
        wire_count: u32,
        face_count: u32,
        shell_count: u32,
        solid_count: u32,
        volume_mm3: f64,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
    }

    struct NativeFaceEvidence {
        ordinal: u32,
        surface_kind: String,
        area_mm2: f64,
        centroid_x: f64,
        centroid_y: f64,
        centroid_z: f64,
        normal_x: f64,
        normal_y: f64,
        normal_z: f64,
        has_axis: bool,
        axis_origin_x: f64,
        axis_origin_y: f64,
        axis_origin_z: f64,
        axis_direction_x: f64,
        axis_direction_y: f64,
        axis_direction_z: f64,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
        edge_count: u32,
    }

    struct NativeFaceEdgeEvidence {
        face_ordinal: u32,
        edge_ordinal: u32,
    }

    struct NativeEdgeEvidence {
        ordinal: u32,
        curve_kind: String,
        length_mm: f64,
        centroid_x: f64,
        centroid_y: f64,
        centroid_z: f64,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
        closed: bool,
        has_circle: bool,
        has_axis: bool,
        circle_radius_mm: f64,
        axis_origin_x: f64,
        axis_origin_y: f64,
        axis_origin_z: f64,
        axis_direction_x: f64,
        axis_direction_y: f64,
        axis_direction_z: f64,
    }

    struct NativeEdgeFaceEvidence {
        edge_ordinal: u32,
        face_ordinal: u32,
    }

    struct NativeHistoryEvidence {
        semantic_role: String,
        relation: String,
        source_element_id: String,
        output_ordinal: u32,
        output_present: bool,
    }

    struct NativeEdgeHistoryEvidence {
        semantic_role: String,
        relation: String,
        source_element_id: String,
        output_ordinal: u32,
        output_present: bool,
    }

    struct NativeMeshVertex {
        x_mm: f64,
        y_mm: f64,
        z_mm: f64,
    }

    struct NativeMeshTriangle {
        first: u32,
        second: u32,
        third: u32,
        face_ordinal: u32,
    }

    struct NativeVolumeMeshTetrahedron {
        first: u32,
        second: u32,
        third: u32,
        fourth: u32,
    }

    unsafe extern "C++" {
        include!("ketchup_exact.hxx");

        type NativeOperationResult;
        type NativeMeshResult;
        type NativeVolumeMeshResult;

        fn make_box_native(
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn offset_rectangle_native(
            min_x: f64,
            min_y: f64,
            max_x: f64,
            max_y: f64,
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn offset_planar_profile_native(
            segments: &[f64],
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn planar_surface_profile_native(segments: &[f64]) -> UniquePtr<NativeOperationResult>;
        fn trim_surface_native(
            target: &NativeOperationResult,
            cutter: &NativeOperationResult,
        ) -> UniquePtr<NativeOperationResult>;
        fn extend_planar_surface_native(
            target: &NativeOperationResult,
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn combine_surfaces_native(
            base: &NativeOperationResult,
            added: &NativeOperationResult,
        ) -> UniquePtr<NativeOperationResult>;
        fn knit_surface_compound_native(
            surfaces: &NativeOperationResult,
            tolerance: f64,
            make_solid: bool,
        ) -> UniquePtr<NativeOperationResult>;
        fn thicken_surface_native(
            surface: &NativeOperationResult,
            thickness: f64,
            direction: u8,
        ) -> UniquePtr<NativeOperationResult>;
        fn offset_planar_region_native(
            segments: &[f64],
            loop_segment_counts: &[u32],
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn offset_planar_circle_native(
            center_x: f64,
            center_y: f64,
            radius: f64,
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn sweep_planar_profile_native(
            profile_segments: &[f64],
            path_segments: &[f64],
        ) -> UniquePtr<NativeOperationResult>;
        fn loft_framed_profiles_native(
            values: &[f64],
            guide_segments: &[f64],
            continuity: u8,
            make_solid: bool,
        ) -> UniquePtr<NativeOperationResult>;
        fn loft_spline_native(values: &[f64]) -> UniquePtr<NativeOperationResult>;
        fn loft_planar_profiles_native(
            segments: &[f64],
            section_segment_counts: &[u32],
            elevations: &[f64],
        ) -> UniquePtr<NativeOperationResult>;
        fn extrude_circle_native(
            center_x: f64,
            center_y: f64,
            radius: f64,
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn sweep_axial_tool_native(values: &[f64]) -> UniquePtr<NativeOperationResult>;
        fn extrude_mixed_profile_native(
            segments: &[f64],
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn extrude_planar_region_native(
            segments: &[f64],
            loop_segment_counts: &[u32],
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn revolve_general_profile_native(
            segments: &[f64],
            axis_start_x: f64,
            axis_start_y: f64,
            axis_end_x: f64,
            axis_end_y: f64,
            angle_degrees: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn revolve_planar_region_native(
            segments: &[f64],
            loop_segment_counts: &[u32],
            axis_start_x: f64,
            axis_start_y: f64,
            axis_end_x: f64,
            axis_end_y: f64,
            angle_degrees: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn shell_body_native(
            body: &NativeOperationResult,
            face_ordinals: &[u32],
            thickness: f64,
            direction: u8,
        ) -> UniquePtr<NativeOperationResult>;
        fn offset_body_face_native(
            body: &NativeOperationResult,
            face_ordinal: u32,
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        #[allow(clippy::too_many_arguments)]
        fn finish_body_native(
            body: &NativeOperationResult,
            edge_ordinals: &[u32],
            face_ordinals: &[u32],
            amount: f64,
            fillet: bool,
            fillet_radius_stations: &[f64],
            chamfer_mode: u8,
            chamfer_secondary: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn exception_probe_native() -> UniquePtr<NativeOperationResult>;
        fn import_step_native(path: &str) -> UniquePtr<NativeOperationResult>;
        fn import_step_solid_native(
            path: &str,
            solid_ordinal: u32,
        ) -> UniquePtr<NativeOperationResult>;
        fn import_step_xde_part_native(
            path: &str,
            part_index: u32,
        ) -> UniquePtr<NativeOperationResult>;
        fn step_xde_manifest_native(path: &str) -> String;
        fn export_step_xde_assembly_native(manifest: &str, path: &str) -> String;
        fn step_length_unit_native(path: &str) -> String;
        fn import_iges_native(path: &str) -> UniquePtr<NativeOperationResult>;
        fn import_iges_xde_part_native(
            path: &str,
            part_index: u32,
        ) -> UniquePtr<NativeOperationResult>;
        fn iges_xde_manifest_native(path: &str) -> String;
        fn export_iges_xde_assembly_native(manifest: &str, path: &str) -> String;
        fn iges_length_unit_native(path: &str) -> String;
        fn transform_body_native(
            body: &NativeOperationResult,
            matrix: &[f64],
        ) -> UniquePtr<NativeOperationResult>;
        fn combine_bodies_native(
            base: &NativeOperationResult,
            added: &NativeOperationResult,
        ) -> UniquePtr<NativeOperationResult>;
        #[allow(clippy::too_many_arguments)]
        fn trim_body_by_plane_native(
            body: &NativeOperationResult,
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            normal_x: f64,
            normal_y: f64,
            normal_z: f64,
            keep_x: f64,
            keep_y: f64,
            keep_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn query_body_pair_native(
            left: &NativeOperationResult,
            right: &NativeOperationResult,
        ) -> NativePairQuery;
        fn boolean_bodies_native(
            target: &NativeOperationResult,
            tool: &NativeOperationResult,
            operation: u8,
        ) -> UniquePtr<NativeOperationResult>;
        fn export_step_native(body: &NativeOperationResult, path: &str) -> String;
        fn export_iges_native(body: &NativeOperationResult, path: &str) -> String;
        fn tessellate_body_native(
            body: &NativeOperationResult,
            deflection: f64,
            angular_deflection: f64,
            max_triangles: u32,
        ) -> UniquePtr<NativeMeshResult>;

        fn mesh_status_code(self: &NativeMeshResult) -> u8;
        fn mesh_diagnostic(self: &NativeMeshResult) -> String;
        fn mesh_vertices(self: &NativeMeshResult) -> Vec<NativeMeshVertex>;
        fn mesh_triangles(self: &NativeMeshResult) -> Vec<NativeMeshTriangle>;

        fn volume_mesh_body_native(
            body: &NativeOperationResult,
            deflection: f64,
            angular_deflection: f64,
            max_tetrahedra: u32,
        ) -> UniquePtr<NativeVolumeMeshResult>;
        fn volume_mesh_status_code(self: &NativeVolumeMeshResult) -> u8;
        fn volume_mesh_diagnostic(self: &NativeVolumeMeshResult) -> String;
        fn volume_mesh_vertices(self: &NativeVolumeMeshResult) -> Vec<NativeMeshVertex>;
        fn volume_mesh_tetrahedra(
            self: &NativeVolumeMeshResult,
        ) -> Vec<NativeVolumeMeshTetrahedron>;
        fn volume_mesh_boundary_triangles(self: &NativeVolumeMeshResult)
        -> Vec<NativeMeshTriangle>;

        fn status_code(self: &NativeOperationResult) -> u8;
        fn diagnostic(self: &NativeOperationResult) -> String;
        fn valid(self: &NativeOperationResult) -> bool;
        fn topology_summary(self: &NativeOperationResult) -> NativeTopologySummary;
        fn face_evidence(self: &NativeOperationResult) -> Vec<NativeFaceEvidence>;
        fn edge_evidence(self: &NativeOperationResult) -> Vec<NativeEdgeEvidence>;
        fn face_edge_evidence(self: &NativeOperationResult) -> Vec<NativeFaceEdgeEvidence>;
        fn edge_face_evidence(self: &NativeOperationResult) -> Vec<NativeEdgeFaceEvidence>;
        fn history_evidence(self: &NativeOperationResult) -> Vec<NativeHistoryEvidence>;
        fn edge_history_evidence(self: &NativeOperationResult) -> Vec<NativeEdgeHistoryEvidence>;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const ORIGIN: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxSpec {
    pub origin_mm: Point3,
    pub size_mm: Size3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleExtrudeSpec {
    pub width_mm: f64,
    pub depth_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleOffsetSpec {
    pub min_mm: [f64; 2],
    pub max_mm: [f64; 2],
    pub distance_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleSweepSpec {
    pub profile_min_mm: [f64; 2],
    pub profile_max_mm: [f64; 2],
    pub path_start_mm: [f64; 2],
    pub path_end_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplineLoftSection {
    pub elevation_mm: f64,
    pub control_points_mm: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplineLoftSpec {
    pub sections: Vec<SplineLoftSection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanarLoftSection {
    pub elevation_mm: f64,
    pub profile: PlanarProfileLoop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanarLoftSpec {
    pub sections: Vec<PlanarLoftSection>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FramedLoftProfile {
    Planar(PlanarProfileLoop),
    Spline { control_points_mm: Vec<[f64; 2]> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FramedLoftSection {
    pub elevation_mm: f64,
    pub frame: [f64; 12],
    pub profile: FramedLoftProfile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FramedLoftSpec {
    pub sections: Vec<FramedLoftSection>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LoftSurfaceContinuity {
    #[default]
    Position,
    Tangent,
    Curvature,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleExtrudeSpec {
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxialToolMotion {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    Arc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        clockwise: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxialToolSweepSpec {
    pub motion: AxialToolMotion,
    pub radius_mm: f64,
    pub axial_length_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlanarProfileSegment {
    Line {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    CircularArc {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 2],
        control_1_mm: [f64; 2],
        control_2_mm: [f64; 2],
        end_mm: [f64; 2],
    },
}

/// One exact three-dimensional segment of an open spatial sweep path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpatialProfileSegment {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    CircularArc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        normal: [f64; 3],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 3],
        control_1_mm: [f64; 3],
        control_2_mm: [f64; 3],
        end_mm: [f64; 3],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanarProfileLoop {
    Segments(Vec<PlanarProfileSegment>),
    Circle { center_mm: [f64; 2], radius_mm: f64 },
}

fn planar_segment_endpoints(segment: &PlanarProfileSegment) -> ([f64; 2], [f64; 2]) {
    match segment {
        PlanarProfileSegment::Line { start_mm, end_mm }
        | PlanarProfileSegment::CircularArc {
            start_mm, end_mm, ..
        }
        | PlanarProfileSegment::CubicBezier {
            start_mm, end_mm, ..
        } => (*start_mm, *end_mm),
    }
}

const SWEEP_PATH_INTERSECTION_EPSILON_MM: f64 = 1.0e-9;

fn sweep_path_segment_bounds(segment: &PlanarProfileSegment) -> [[f64; 2]; 2] {
    let points = match segment {
        PlanarProfileSegment::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
        PlanarProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            ..
        } => {
            let start_radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
            let end_radius = (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]);
            let radius = start_radius.max(end_radius);
            return [
                [center_mm[0] - radius, center_mm[1] - radius],
                [center_mm[0] + radius, center_mm[1] + radius],
            ];
        }
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => vec![*start_mm, *control_1_mm, *control_2_mm, *end_mm],
    };
    [0, 1].map(|bound| {
        [0, 1].map(|axis| {
            points.iter().fold(
                if bound == 0 {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
                |value, point| {
                    if bound == 0 {
                        value.min(point[axis])
                    } else {
                        value.max(point[axis])
                    }
                },
            )
        })
    })
}

fn sweep_path_arc_angle(segment: &PlanarProfileSegment) -> Option<f64> {
    let PlanarProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        clockwise,
    } = segment
    else {
        return None;
    };
    let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
    let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
    Some(if *clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    })
}

fn sweep_path_join_is_separated(
    left: &PlanarProfileSegment,
    right: &PlanarProfileSegment,
    tangent: [f64; 2],
) -> bool {
    let join = planar_segment_endpoints(left).1;
    let projection =
        |point: [f64; 2]| (point[0] - join[0]) * tangent[0] + (point[1] - join[1]) * tangent[1];
    let left_is_behind = match left {
        PlanarProfileSegment::Line { start_mm, .. } => {
            projection(*start_mm) < -SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        PlanarProfileSegment::CircularArc { .. } => sweep_path_arc_angle(left)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            ..
        } => [*start_mm, *control_1_mm, *control_2_mm]
            .into_iter()
            .all(|point| projection(point) < -SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    let right_is_ahead = match right {
        PlanarProfileSegment::Line { end_mm, .. } => {
            projection(*end_mm) > SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        PlanarProfileSegment::CircularArc { .. } => sweep_path_arc_angle(right)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        PlanarProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => [*control_1_mm, *control_2_mm, *end_mm]
            .into_iter()
            .all(|point| projection(point) > SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    left_is_behind && right_is_ahead
}

fn sweep_path_self_intersects(
    segments: &[PlanarProfileSegment],
    metrics: &[(f64, [f64; 2], [f64; 2])],
) -> bool {
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            !sweep_path_join_is_separated(&segments[0], &segments[1], metrics[0].2)
        })
    {
        return true;
    }
    let bounds = segments
        .iter()
        .map(sweep_path_segment_bounds)
        .collect::<Vec<_>>();
    for left in 0..bounds.len() {
        for right in left + 2..bounds.len() {
            if [0, 1].into_iter().all(|axis| {
                bounds[left][0][axis] <= bounds[right][1][axis] + SWEEP_PATH_INTERSECTION_EPSILON_MM
                    && bounds[right][0][axis]
                        <= bounds[left][1][axis] + SWEEP_PATH_INTERSECTION_EPSILON_MM
            }) {
                return true;
            }
        }
    }
    false
}

fn flatten_planar_segments(segments: &[PlanarProfileSegment]) -> Vec<f64> {
    segments
        .iter()
        .flat_map(|segment| match segment {
            PlanarProfileSegment::Line { start_mm, end_mm } => [
                0.0,
                start_mm[0],
                start_mm[1],
                end_mm[0],
                end_mm[1],
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => [
                1.0,
                start_mm[0],
                start_mm[1],
                end_mm[0],
                end_mm[1],
                center_mm[0],
                center_mm[1],
                0.0,
                0.0,
                f64::from(*clockwise),
            ],
            PlanarProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => [
                2.0,
                start_mm[0],
                start_mm[1],
                end_mm[0],
                end_mm[1],
                control_1_mm[0],
                control_1_mm[1],
                control_2_mm[0],
                control_2_mm[1],
                0.0,
            ],
        })
        .collect()
}

fn flatten_planar_loop(profile: &PlanarProfileLoop) -> Vec<f64> {
    match profile {
        PlanarProfileLoop::Segments(segments) => flatten_planar_segments(segments),
        PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } => vec![
            3.0,
            center_mm[0],
            center_mm[1],
            *radius_mm,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
    }
}

fn spatial_segment_endpoints(segment: &SpatialProfileSegment) -> ([f64; 3], [f64; 3]) {
    match segment {
        SpatialProfileSegment::Line { start_mm, end_mm }
        | SpatialProfileSegment::CircularArc {
            start_mm, end_mm, ..
        }
        | SpatialProfileSegment::CubicBezier {
            start_mm, end_mm, ..
        } => (*start_mm, *end_mm),
    }
}

fn spatial_sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn spatial_dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn spatial_cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn spatial_length(vector: [f64; 3]) -> f64 {
    spatial_dot(vector, vector).sqrt()
}

fn spatial_unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = spatial_length(vector);
    (length.is_finite() && length > MIN_SWEEP_PATH_SEGMENT_LENGTH_MM)
        .then(|| vector.map(|coordinate| coordinate / length))
}

fn spatial_sweep_path_arc_angle(segment: &SpatialProfileSegment) -> Option<f64> {
    let SpatialProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        normal,
        clockwise,
    } = segment
    else {
        return None;
    };
    let normal = spatial_unit(*normal)?;
    let start_radius = spatial_sub(*start_mm, *center_mm);
    let end_radius = spatial_sub(*end_mm, *center_mm);
    let signed = spatial_dot(normal, spatial_cross(start_radius, end_radius))
        .atan2(spatial_dot(start_radius, end_radius));
    Some(if *clockwise {
        (-signed).rem_euclid(std::f64::consts::TAU)
    } else {
        signed.rem_euclid(std::f64::consts::TAU)
    })
}

fn spatial_sweep_path_join_is_separated(
    left: &SpatialProfileSegment,
    right: &SpatialProfileSegment,
    tangent: [f64; 3],
) -> bool {
    let join = spatial_segment_endpoints(left).1;
    let projection = |point: [f64; 3]| spatial_dot(spatial_sub(point, join), tangent);
    let left_is_behind = match left {
        SpatialProfileSegment::Line { start_mm, .. } => {
            projection(*start_mm) < -SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        SpatialProfileSegment::CircularArc { .. } => spatial_sweep_path_arc_angle(left)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        SpatialProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            ..
        } => [*start_mm, *control_1_mm, *control_2_mm]
            .into_iter()
            .all(|point| projection(point) < -SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    let right_is_ahead = match right {
        SpatialProfileSegment::Line { end_mm, .. } => {
            projection(*end_mm) > SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        SpatialProfileSegment::CircularArc { .. } => spatial_sweep_path_arc_angle(right)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        SpatialProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => [*control_1_mm, *control_2_mm, *end_mm]
            .into_iter()
            .all(|point| projection(point) > SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    left_is_behind && right_is_ahead
}

fn flatten_spatial_segments(segments: &[SpatialProfileSegment]) -> Vec<f64> {
    segments
        .iter()
        .flat_map(|segment| match segment {
            SpatialProfileSegment::Line { start_mm, end_mm } => [
                10.0,
                start_mm[0],
                start_mm[1],
                start_mm[2],
                end_mm[0],
                end_mm[1],
                end_mm[2],
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            SpatialProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                normal,
                clockwise,
            } => [
                11.0,
                start_mm[0],
                start_mm[1],
                start_mm[2],
                end_mm[0],
                end_mm[1],
                end_mm[2],
                center_mm[0],
                center_mm[1],
                center_mm[2],
                normal[0],
                normal[1],
                normal[2],
                f64::from(*clockwise),
            ],
            SpatialProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => [
                12.0,
                start_mm[0],
                start_mm[1],
                start_mm[2],
                end_mm[0],
                end_mm[1],
                end_mm[2],
                control_1_mm[0],
                control_1_mm[1],
                control_1_mm[2],
                control_2_mm[0],
                control_2_mm[1],
                control_2_mm[2],
                0.0,
            ],
        })
        .collect()
}

type SpatialSweepPathMetric = (f64, [f64; 3], [f64; 3]);

fn spatial_sweep_path_metrics(
    segment: &SpatialProfileSegment,
    operation: &'static str,
    input: &str,
) -> Result<SpatialSweepPathMetric, GeometryError> {
    let invalid = || {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            "Spatial Sweep path must contain only bounded non-degenerate lines, circular arcs, and cubic Bezier segments"
                .to_owned(),
        )
    };
    let (start, end) = spatial_segment_endpoints(segment);
    for (point_name, point) in [("start", start), ("end", end)] {
        for (axis, coordinate) in point.into_iter().enumerate() {
            validate_coordinate(
                coordinate,
                &format!("path_{point_name}_axis_{axis}"),
                operation,
                input,
            )?;
        }
    }
    match segment {
        SpatialProfileSegment::Line { .. } => {
            let direction = spatial_sub(end, start);
            let tangent = spatial_unit(direction).ok_or_else(invalid)?;
            Ok((spatial_length(direction), tangent, tangent))
        }
        SpatialProfileSegment::CircularArc {
            center_mm,
            normal,
            clockwise,
            ..
        } => {
            for (point_name, point) in [("center", *center_mm), ("normal", *normal)] {
                for (axis, coordinate) in point.into_iter().enumerate() {
                    validate_coordinate(
                        coordinate,
                        &format!("path_{point_name}_axis_{axis}"),
                        operation,
                        input,
                    )?;
                }
            }
            let normal_length = spatial_length(*normal);
            if (normal_length - 1.0).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM {
                return Err(invalid());
            }
            let normal = spatial_unit(*normal).ok_or_else(invalid)?;
            let start_radius = spatial_sub(start, *center_mm);
            let end_radius = spatial_sub(end, *center_mm);
            let radius = spatial_length(start_radius);
            let end_radius_length = spatial_length(end_radius);
            if radius <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                || (radius - end_radius_length).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM
                || spatial_dot(start_radius, normal).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM
                || spatial_dot(end_radius, normal).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM
                || start == end
            {
                return Err(invalid());
            }
            let signed_angle = spatial_dot(normal, spatial_cross(start_radius, end_radius))
                .atan2(spatial_dot(start_radius, end_radius));
            let angle = if *clockwise {
                (-signed_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                signed_angle.rem_euclid(std::f64::consts::TAU)
            };
            let length = radius * angle;
            if length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM {
                return Err(invalid());
            }
            let sign = if *clockwise { -1.0 } else { 1.0 };
            let start_tangent = spatial_unit(spatial_cross(normal, start_radius).map(|v| sign * v))
                .ok_or_else(invalid)?;
            let end_tangent = spatial_unit(spatial_cross(normal, end_radius).map(|v| sign * v))
                .ok_or_else(invalid)?;
            Ok((length, start_tangent, end_tangent))
        }
        SpatialProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } => {
            for (point_name, point) in [("control_1", *control_1_mm), ("control_2", *control_2_mm)]
            {
                for (axis, coordinate) in point.into_iter().enumerate() {
                    validate_coordinate(
                        coordinate,
                        &format!("path_{point_name}_axis_{axis}"),
                        operation,
                        input,
                    )?;
                }
            }
            let chord = spatial_sub(end, start);
            let first = spatial_sub(*control_1_mm, start);
            let middle = spatial_sub(*control_2_mm, *control_1_mm);
            let last = spatial_sub(end, *control_2_mm);
            let chord_squared = spatial_dot(chord, chord);
            let projection_1 = spatial_dot(first, chord);
            let projection_2 = spatial_dot(spatial_sub(*control_2_mm, start), chord);
            if start == end
                || projection_1 <= 0.0
                || projection_2 < projection_1
                || projection_2 >= chord_squared
            {
                return Err(invalid());
            }
            let start_tangent = spatial_unit(first).ok_or_else(invalid)?;
            let end_tangent = spatial_unit(last).ok_or_else(invalid)?;
            Ok((
                spatial_length(first) + spatial_length(middle) + spatial_length(last),
                start_tangent,
                end_tangent,
            ))
        }
    }
}

fn spatial_sweep_path_self_intersects(
    segments: &[SpatialProfileSegment],
    metrics: &[(f64, [f64; 3], [f64; 3])],
) -> bool {
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            !spatial_sweep_path_join_is_separated(&segments[0], &segments[1], metrics[0].2)
        })
    {
        return true;
    }
    let closed = spatial_segment_endpoints(segments.first().unwrap()).0
        == spatial_segment_endpoints(segments.last().unwrap()).1;
    if closed
        && !spatial_sweep_path_join_is_separated(
            segments.last().unwrap(),
            segments.first().unwrap(),
            metrics.last().unwrap().2,
        )
    {
        return true;
    }
    if segments.windows(2).zip(metrics.windows(2)).any(
        |(segments, metrics)| {
            matches!(
                (&segments[0], &segments[1]),
                (
                    SpatialProfileSegment::CircularArc {
                        start_mm,
                        center_mm: left_center,
                        ..
                    },
                    SpatialProfileSegment::CircularArc {
                        center_mm: right_center,
                        ..
                    }
                ) if left_center == right_center
                    && metrics[0].0 + metrics[1].0
                        >= std::f64::consts::TAU * spatial_length(spatial_sub(*start_mm, *left_center))
                            - SWEEP_PATH_INTERSECTION_EPSILON_MM
            )
        },
    ) {
        return true;
    }
    // Axis-aligned bounds are only a broad phase for spatial curves. The native
    // OCCT operation performs the authoritative edge-distance intersection test.
    false
}

fn validate_spatial_sweep_path(
    segments: &[SpatialProfileSegment],
    operation: &'static str,
    input: &str,
) -> Result<Vec<SpatialSweepPathMetric>, GeometryError> {
    let invalid = |diagnostic: &str| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            diagnostic.to_owned(),
        )
    };
    if !(1..=MAX_SWEEP_PATH_SEGMENTS).contains(&segments.len()) {
        return Err(invalid(
            "Spatial Sweep requires between one and 64 path segments",
        ));
    }
    let metrics = segments
        .iter()
        .map(|segment| spatial_sweep_path_metrics(segment, operation, input))
        .collect::<Result<Vec<_>, _>>()?;
    let closed = segments.first().map(spatial_segment_endpoints).unwrap().0
        == segments.last().map(spatial_segment_endpoints).unwrap().1;
    for (segments, metrics) in segments.windows(2).zip(metrics.windows(2)) {
        if spatial_segment_endpoints(&segments[0]).1 != spatial_segment_endpoints(&segments[1]).0 {
            return Err(invalid("Spatial Sweep path segments are disconnected"));
        }
        if spatial_dot(metrics[0].2, metrics[1].1) < 1.0 - 1.0e-9
            || spatial_length(spatial_cross(metrics[0].2, metrics[1].1)) > 1.0e-9
        {
            return Err(invalid(
                "Spatial Sweep path segments must be C1 tangent-continuous",
            ));
        }
    }
    if closed {
        let outgoing = metrics.last().unwrap().2;
        let incoming = metrics.first().unwrap().1;
        if spatial_dot(outgoing, incoming) < 1.0 - 1.0e-9
            || spatial_length(spatial_cross(outgoing, incoming)) > 1.0e-9
        {
            return Err(invalid(
                "Closed Spatial Sweep path seam must be C1 tangent-continuous",
            ));
        }
    }
    if spatial_sweep_path_self_intersects(segments, &metrics) {
        return Err(invalid("Spatial Sweep path must not self-intersect"));
    }
    validate_length(
        metrics.iter().map(|metric| metric.0).sum(),
        "path_length",
        operation,
        input,
    )?;
    Ok(metrics)
}

fn planar_segment_signed_area(segment: &PlanarProfileSegment, origin: [f64; 2]) -> f64 {
    let rebase = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
    match segment {
        PlanarProfileSegment::Line { start_mm, end_mm } => {
            let start = rebase(*start_mm);
            let end = rebase(*end_mm);
            0.5 * (start[0] * end[1] - end[0] * start[1])
        }
        PlanarProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => {
            let start = rebase(*start_mm);
            let end = rebase(*end_mm);
            let center = rebase(*center_mm);
            let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
            let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
            let mut sweep = end_angle - start_angle;
            if *clockwise {
                if sweep >= 0.0 {
                    sweep -= std::f64::consts::TAU;
                }
            } else if sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            0.5 * (radius * center[0] * (end_angle.sin() - start_angle.sin())
                - radius * center[1] * (end_angle.cos() - start_angle.cos())
                + radius * radius * sweep)
        }
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => {
            let start = rebase(*start_mm);
            let control_1 = rebase(*control_1_mm);
            let control_2 = rebase(*control_2_mm);
            let end = rebase(*end_mm);
            let coefficients = |axis: usize| {
                [
                    start[axis],
                    3.0 * (control_1[axis] - start[axis]),
                    3.0 * (start[axis] - 2.0 * control_1[axis] + control_2[axis]),
                    -start[axis] + 3.0 * control_1[axis] - 3.0 * control_2[axis] + end[axis],
                ]
            };
            let x = coefficients(0);
            let y = coefficients(1);
            let mut integral = 0.0;
            for left_degree in 0..=3 {
                for right_degree in 1..=3 {
                    let denominator = (left_degree + right_degree) as f64;
                    integral += (x[left_degree] * y[right_degree] * right_degree as f64
                        - y[left_degree] * x[right_degree] * right_degree as f64)
                        / denominator;
                }
            }
            0.5 * integral
        }
    }
}

fn reverse_planar_segments(segments: &[PlanarProfileSegment]) -> Vec<PlanarProfileSegment> {
    segments
        .iter()
        .rev()
        .map(|segment| match *segment {
            PlanarProfileSegment::Line { start_mm, end_mm } => PlanarProfileSegment::Line {
                start_mm: end_mm,
                end_mm: start_mm,
            },
            PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm: end_mm,
                end_mm: start_mm,
                center_mm,
                clockwise: !clockwise,
            },
            PlanarProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => PlanarProfileSegment::CubicBezier {
                start_mm: end_mm,
                control_1_mm: control_2_mm,
                control_2_mm: control_1_mm,
                end_mm: start_mm,
            },
        })
        .collect()
}

fn flatten_planar_region(
    outer: &PlanarProfileLoop,
    holes: &[PlanarProfileLoop],
    operation: &'static str,
    input: &str,
) -> Result<(Vec<f64>, Vec<u32>), GeometryError> {
    if holes.is_empty() || holes.len() > MAX_PLANAR_REGION_HOLES {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            "Planar region requires 1..=64 holes".to_owned(),
        ));
    }
    let mut flattened = Vec::new();
    let mut loop_segment_counts = Vec::with_capacity(holes.len() + 1);
    for (loop_index, planar_loop) in std::iter::once(outer).chain(holes).enumerate() {
        let before = flattened.len();
        match planar_loop {
            PlanarProfileLoop::Segments(segments) => {
                validate_mixed_profile(segments, operation, input)?;
                let origin = planar_segment_endpoints(&segments[0]).0;
                let mut signed_area = 0.0;
                let mut compensation = 0.0;
                for segment in segments {
                    let adjusted = planar_segment_signed_area(segment, origin) - compensation;
                    let next = signed_area + adjusted;
                    compensation = (next - signed_area) - adjusted;
                    signed_area = next;
                }
                if !signed_area.is_finite() || signed_area.abs() <= MIN_LENGTH_MM * MIN_LENGTH_MM {
                    return Err(parameter_error(
                        GeometryErrorCode::InvalidProfile,
                        operation,
                        input,
                        "Planar region loop has zero signed area".to_owned(),
                    ));
                }
                let should_reverse = (loop_index == 0) != (signed_area > 0.0);
                if should_reverse {
                    flattened.extend(flatten_planar_segments(&reverse_planar_segments(segments)));
                } else {
                    flattened.extend(flatten_planar_segments(segments));
                }
            }
            PlanarProfileLoop::Circle {
                center_mm,
                radius_mm,
            } => {
                validate_circle(*center_mm, *radius_mm, operation, input)?;
                flattened.extend([
                    3.0,
                    center_mm[0],
                    center_mm[1],
                    *radius_mm,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    f64::from(loop_index != 0),
                ]);
            }
        }
        loop_segment_counts.push(
            ((flattened.len() - before) / PLANAR_SEGMENT_STRIDE)
                .try_into()
                .map_err(|_| {
                    parameter_error(
                        GeometryErrorCode::InvalidProfile,
                        operation,
                        input,
                        "Planar region exceeds the segment limit".to_owned(),
                    )
                })?,
        );
    }
    if flattened.len() / PLANAR_SEGMENT_STRIDE > MAX_PLANAR_REGION_SEGMENTS {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            "Planar region exceeds the segment limit".to_owned(),
        ));
    }
    Ok((flattened, loop_segment_counts))
}

fn compare_planar_encoding(left: &[f64], right: &[f64]) -> std::cmp::Ordering {
    left.iter()
        .zip(right)
        .map(|(left, right)| left.total_cmp(right))
        .find(|ordering| !ordering.is_eq())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn canonicalize_planar_region_encoding(
    mut flattened: Vec<f64>,
    loop_segment_counts: Vec<u32>,
) -> (Vec<f64>, Vec<u32>) {
    for value in &mut flattened {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    let mut offset = 0;
    let mut loops = loop_segment_counts
        .into_iter()
        .map(|segment_count| {
            let value_count = segment_count as usize * PLANAR_SEGMENT_STRIDE;
            let mut values = flattened[offset..offset + value_count].to_vec();
            offset += value_count;
            let canonical_start = (0..segment_count as usize)
                .min_by(|left, right| {
                    let left = left * PLANAR_SEGMENT_STRIDE;
                    let right = right * PLANAR_SEGMENT_STRIDE;
                    compare_planar_encoding(
                        &values[left..]
                            .iter()
                            .chain(&values[..left])
                            .copied()
                            .collect::<Vec<_>>(),
                        &values[right..]
                            .iter()
                            .chain(&values[..right])
                            .copied()
                            .collect::<Vec<_>>(),
                    )
                })
                .expect("validated planar region loop is non-empty");
            values.rotate_left(canonical_start * PLANAR_SEGMENT_STRIDE);
            (segment_count, values)
        })
        .collect::<Vec<_>>();
    loops[1..].sort_by(|left, right| compare_planar_encoding(&left.1, &right.1));
    let counts = loops.iter().map(|(count, _)| *count).collect();
    let flattened = loops.into_iter().flat_map(|(_, values)| values).collect();
    (flattened, counts)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CylinderToolSpec {
    pub center_mm: [f64; 2],
    pub origin_z_mm: f64,
    pub radius_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeFinish {
    Fillet,
    Chamfer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdvancedChamferMode {
    TwoDistance { second_distance_mm: f64 },
    DistanceAngle { angle_degrees: f64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellDirection {
    Inward,
    Outward,
    Symmetric,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CutMode {
    ThroughAll,
    BlindPlanar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryErrorCode {
    InvalidParameter,
    InvalidProfile,
    NonFiniteParameter,
    NoGeometricChange,
    DegenerateOperation,
    InvalidShape,
    BackendException,
    NullResult,
}

impl GeometryErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParameter => "invalid_parameter",
            Self::InvalidProfile => "invalid_profile",
            Self::NonFiniteParameter => "non_finite_parameter",
            Self::NoGeometricChange => "no_geometric_change",
            Self::DegenerateOperation => "degenerate_operation",
            Self::InvalidShape => "invalid_shape",
            Self::BackendException => "backend_exception",
            Self::NullResult => "null_result",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryError {
    pub code: GeometryErrorCode,
    pub diagnostic: String,
    pub operation: &'static str,
    pub input_digest: String,
    pub backend_fingerprint: &'static str,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.diagnostic)
    }
}

impl std::error::Error for GeometryError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3 {
    pub min: Point3,
    pub max: Point3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FaceEvidence {
    pub ordinal: u32,
    pub surface_kind: String,
    pub area_mm2: f64,
    pub centroid_mm: Point3,
    pub normal: Point3,
    pub axis_origin_mm: Option<Point3>,
    pub axis_direction: Option<Point3>,
    pub bounds_mm: Bounds3,
    pub edge_count: u32,
    pub edge_ordinals: Vec<u32>,
    pub geometric_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeEvidence {
    pub ordinal: u32,
    pub curve_kind: String,
    pub length_mm: f64,
    pub centroid_mm: Point3,
    pub bounds_mm: Bounds3,
    pub closed: bool,
    pub circle_radius_mm: Option<f64>,
    pub axis_origin_mm: Option<Point3>,
    pub axis_direction: Option<Point3>,
    pub adjacent_face_ordinals: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyEvidence {
    pub vertex_count: u32,
    pub edge_count: u32,
    pub wire_count: u32,
    pub face_count: u32,
    pub shell_count: u32,
    pub solid_count: u32,
    pub volume_mm3: f64,
    pub bounds_mm: Bounds3,
    pub faces: Vec<FaceEvidence>,
    pub edges: Vec<EdgeEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEvidence {
    pub semantic_role: Option<String>,
    pub relation: String,
    pub source_element_id: String,
    pub output_face_ordinal: Option<u32>,
    pub output_edge_ordinal: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalfLapParticipant {
    A,
    B,
}

impl HalfLapParticipant {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalfLapFaceRole {
    Contact,
    WestWall,
    EastWall,
}

impl HalfLapFaceRole {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::WestWall => "wall.west",
            Self::EastWall => "wall.east",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HalfLapNotchSpec {
    pub joint_id: u64,
    pub participant: HalfLapParticipant,
    pub removed: BoxSpec,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HalfLapFaceEvidence {
    pub joint_id: u64,
    pub participant: HalfLapParticipant,
    pub role: HalfLapFaceRole,
    pub face_ordinal: u32,
    pub lineage_digest: String,
    pub geometric_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryConfidence {
    Complete,
    Partial,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StabilityClass {
    Guaranteed,
    HistoryTracked,
    Heuristic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubshapeRef {
    pub document_id: String,
    pub producer_feature_id: String,
    pub semantic_role: String,
    pub source_element_id: String,
    pub expected_type: String,
    pub stability_class: StabilityClass,
    pub backend_fingerprint: String,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceResolution {
    Resolved {
        face_ordinal: u32,
        migrated_backend: bool,
    },
    ResolvedEdge {
        edge_ordinal: u32,
        migrated_backend: bool,
    },
    Ambiguous {
        candidate_ordinals: Vec<u32>,
    },
    Lost,
    QuarantinedMigration {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryDiagnostic {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToleranceReport {
    pub profile: &'static str,
    pub shape_valid: bool,
    pub accepted_exact_solid: bool,
}

/// One triangle of a derived display mesh, tagged with the ordinal of the
/// exact face it was tessellated from so shading and picking can stay per-face.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactMeshTriangle {
    pub vertex_indices: [u32; 3],
    pub face_ordinal: u32,
}

/// A derived, non-canonical display mesh of an exact body.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactTessellation {
    pub vertices_mm: Vec<[f64; 3]>,
    pub triangles: Vec<ExactMeshTriangle>,
}

pub const EXACT_VOLUME_MESH_SCHEMA: &str = "ketchup.exact-volume-mesh.v1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactVolumeMeshOptions {
    pub surface_deflection_mm: f64,
    pub angular_deflection_rad: f64,
    pub max_tetrahedra: u32,
    pub max_relative_volume_error: f64,
    pub min_tetrahedron_quality: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactVolumeMeshTetrahedron {
    pub vertex_indices: [u32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactVolumeMesh {
    pub schema: &'static str,
    pub source_result_fingerprint: String,
    pub request_digest: String,
    pub mesh_fingerprint: String,
    pub vertices_mm: Vec<[f64; 3]>,
    pub tetrahedra: Vec<ExactVolumeMeshTetrahedron>,
    pub boundary_triangles: Vec<ExactMeshTriangle>,
    pub exact_volume_mm3: f64,
    pub tetrahedral_volume_mm3: f64,
    pub relative_volume_error: f64,
    pub minimum_signed_volume_mm3: f64,
    pub minimum_quality: f64,
    pub maximum_edge_ratio: f64,
}

/// Solid-set relation computed from OCCT BRep common volume and distance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactPairRelation {
    /// Strictly positive common solid volume, including full containment.
    Penetrating,
    /// No common volume and distance at most the requested contact tolerance.
    Touching,
    /// No common volume and distance greater than the contact tolerance.
    Separated,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactPairQueryResult {
    pub relation: ExactPairRelation,
    pub common_volume_mm3: f64,
    /// Area of zero-volume common faces; positive only for face contact.
    pub common_contact_area_mm2: f64,
    /// Minimum solid-set distance; zero for penetrating/contained bodies.
    pub distance_mm: f64,
}

pub struct ExactBody {
    native: cxx::UniquePtr<ffi::NativeOperationResult>,
    pub result_fingerprint: String,
    pub topology: TopologyEvidence,
}

impl fmt::Debug for ExactBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactBody")
            .field("result_fingerprint", &self.result_fingerprint)
            .field("topology", &self.topology)
            .finish_non_exhaustive()
    }
}

pub struct ExactOpOutput {
    pub body: ExactBody,
    pub topology_history: Vec<HistoryEvidence>,
    pub tolerance_report: ToleranceReport,
    pub diagnostics: Vec<GeometryDiagnostic>,
    pub input_digest: String,
    pub backend_fingerprint: &'static str,
    pub history_confidence: HistoryConfidence,
}

impl fmt::Debug for ExactOpOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactOpOutput")
            .field("body", &self.body)
            .field("topology_history", &self.topology_history)
            .field("tolerance_report", &self.tolerance_report)
            .field("diagnostics", &self.diagnostics)
            .field("input_digest", &self.input_digest)
            .field("backend_fingerprint", &self.backend_fingerprint)
            .field("history_confidence", &self.history_confidence)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactBodyBooleanOperation {
    Cut,
    Union,
    Intersect,
    Split,
}

pub const MAX_STEP_XDE_NODES: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepXdePart {
    pub index: u32,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepXdeNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub transform: [f64; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepXdeManifest {
    pub parts: Vec<StepXdePart>,
    pub nodes: Vec<StepXdeNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepXdeExportPart {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepXdeExportNode {
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub color: Option<[u8; 3]>,
    pub transform: [f64; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepXdeManifestError {
    InvalidPath,
    Reader(String),
    Malformed,
    OutOfEnvelope,
}

impl std::fmt::Display for StepXdeManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => formatter.write_str("STEP XDE path must not be empty"),
            Self::Reader(diagnostic) => formatter.write_str(diagnostic),
            Self::Malformed => formatter.write_str("STEP XDE manifest is malformed"),
            Self::OutOfEnvelope => {
                formatter.write_str("STEP XDE manifest exceeds the bounded envelope")
            }
        }
    }
}

impl std::error::Error for StepXdeManifestError {}

fn decode_manifest_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn parse_manifest_text(value: &str) -> Option<String> {
    let bytes = decode_manifest_hex(value)?;
    let text = String::from_utf8(bytes).ok()?;
    (!text.is_empty() && text.len() <= 1_024 && !text.chars().any(char::is_control)).then_some(text)
}

fn parse_manifest_color(value: &str) -> Option<Option<[u8; 3]>> {
    if value == "-" {
        return Some(None);
    }
    let bytes = decode_manifest_hex(value)?;
    (bytes.len() == 3).then(|| Some([bytes[0], bytes[1], bytes[2]]))
}

fn transform_is_rigid(matrix: &[f64; 16]) -> bool {
    if matrix.iter().any(|value| !value.is_finite())
        || matrix[12] != 0.0
        || matrix[13] != 0.0
        || matrix[14] != 0.0
        || matrix[15] != 1.0
    {
        return false;
    }
    let dot = |left: usize, right: usize| {
        matrix[left] * matrix[right]
            + matrix[4 + left] * matrix[4 + right]
            + matrix[8 + left] * matrix[8 + right]
    };
    (dot(0, 0) - 1.0).abs() <= 1.0e-10
        && (dot(1, 1) - 1.0).abs() <= 1.0e-10
        && (dot(2, 2) - 1.0).abs() <= 1.0e-10
        && dot(0, 1).abs() <= 1.0e-10
        && dot(0, 2).abs() <= 1.0e-10
        && dot(1, 2).abs() <= 1.0e-10
}

fn parse_step_xde_manifest(raw: &str) -> Result<StepXdeManifest, StepXdeManifestError> {
    if let Some(diagnostic) = raw.strip_prefix("ERR\t") {
        let text = String::from_utf8(
            decode_manifest_hex(diagnostic).ok_or(StepXdeManifestError::Malformed)?,
        )
        .map_err(|_| StepXdeManifestError::Malformed)?;
        return Err(StepXdeManifestError::Reader(text));
    }
    let mut lines = raw.lines();
    if lines.next() != Some("KETCHUP_STEP_XDE_V1") {
        return Err(StepXdeManifestError::Malformed);
    }
    let mut parts = Vec::new();
    let mut nodes = Vec::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.first().copied() {
            Some("P") if fields.len() == 5 => {
                if parts.len() >= MAX_STEP_XDE_NODES {
                    return Err(StepXdeManifestError::OutOfEnvelope);
                }
                let index = fields[1]
                    .parse::<u32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                if index as usize != parts.len() {
                    return Err(StepXdeManifestError::Malformed);
                }
                parts.push(StepXdePart {
                    index,
                    name: parse_manifest_text(fields[2]).ok_or(StepXdeManifestError::Malformed)?,
                    name_from_source: match fields[3] {
                        "0" => false,
                        "1" => true,
                        _ => return Err(StepXdeManifestError::Malformed),
                    },
                    color: parse_manifest_color(fields[4])
                        .ok_or(StepXdeManifestError::Malformed)?,
                });
            }
            Some("N") if fields.len() == 23 => {
                if nodes.len() >= MAX_STEP_XDE_NODES {
                    return Err(StepXdeManifestError::OutOfEnvelope);
                }
                let id = fields[1]
                    .parse::<u32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                if id as usize != nodes.len() {
                    return Err(StepXdeManifestError::Malformed);
                }
                let parent = fields[2]
                    .parse::<i32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                let parent_id = match parent {
                    -1 => None,
                    value if value >= 0 && (value as u32) < id => Some(value as u32),
                    _ => return Err(StepXdeManifestError::Malformed),
                };
                let part = fields[3]
                    .parse::<i32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                let part_index = match part {
                    -1 => None,
                    value if value >= 0 && (value as usize) < parts.len() => Some(value as u32),
                    _ => return Err(StepXdeManifestError::Malformed),
                };
                let mut transform = [0.0; 16];
                for (target, field) in transform.iter_mut().zip(&fields[7..]) {
                    *target = f64::from_bits(
                        u64::from_str_radix(field, 16)
                            .map_err(|_| StepXdeManifestError::Malformed)?,
                    );
                }
                if !transform_is_rigid(&transform) {
                    return Err(StepXdeManifestError::Malformed);
                }
                nodes.push(StepXdeNode {
                    id,
                    parent_id,
                    part_index,
                    name: parse_manifest_text(fields[4]).ok_or(StepXdeManifestError::Malformed)?,
                    name_from_source: match fields[5] {
                        "0" => false,
                        "1" => true,
                        _ => return Err(StepXdeManifestError::Malformed),
                    },
                    color: parse_manifest_color(fields[6])
                        .ok_or(StepXdeManifestError::Malformed)?,
                    transform,
                });
            }
            _ => return Err(StepXdeManifestError::Malformed),
        }
    }
    if parts.is_empty()
        || nodes.is_empty()
        || !nodes.iter().any(|node| node.parent_id.is_none())
        || parts
            .iter()
            .any(|part| !nodes.iter().any(|node| node.part_index == Some(part.index)))
    {
        return Err(StepXdeManifestError::Malformed);
    }
    Ok(StepXdeManifest { parts, nodes })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExactBackend;

/// Public exact-kernel name used by canonical geometry integrations.
pub type ExactKernel = ExactBackend;

impl ExactBackend {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn make_box(&self, spec: BoxSpec) -> Result<ExactOpOutput, GeometryError> {
        let input = box_input("box", spec);
        validate_box(spec, "box", &input)?;
        let native = ffi::make_box_native(
            spec.origin_mm.x,
            spec.origin_mm.y,
            spec.origin_mm.z,
            spec.size_mm.x,
            spec.size_mm.y,
            spec.size_mm.z,
        );
        collect_output(native, "box", &input, HistoryConfidence::Complete)
    }

    pub fn offset_rectangle(
        &self,
        spec: RectangleOffsetSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "offset_rectangle:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
            spec.min_mm[0].to_bits(),
            spec.min_mm[1].to_bits(),
            spec.max_mm[0].to_bits(),
            spec.max_mm[1].to_bits(),
            spec.distance_mm.to_bits()
        );
        for (name, coordinate) in [
            ("min_x", spec.min_mm[0]),
            ("min_y", spec.min_mm[1]),
            ("max_x", spec.max_mm[0]),
            ("max_y", spec.max_mm[1]),
        ] {
            validate_coordinate(coordinate, name, "offset_rectangle", &input)?;
        }
        let output_min = [
            spec.min_mm[0] - spec.distance_mm,
            spec.min_mm[1] - spec.distance_mm,
        ];
        let output_max = [
            spec.max_mm[0] + spec.distance_mm,
            spec.max_mm[1] + spec.distance_mm,
        ];
        if !spec.distance_mm.is_finite()
            || spec.distance_mm.abs() < MIN_LENGTH_MM
            || spec.max_mm[0] <= spec.min_mm[0]
            || spec.max_mm[1] <= spec.min_mm[1]
            || output_max[0] - output_min[0] < MIN_LENGTH_MM
            || output_max[1] - output_min[1] < MIN_LENGTH_MM
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "offset_rectangle",
                &input,
                "Rectangle offset is outside the bounded planar envelope".to_owned(),
            ));
        }
        for (name, coordinate) in [
            ("output_min_x", output_min[0]),
            ("output_min_y", output_min[1]),
            ("output_max_x", output_max[0]),
            ("output_max_y", output_max[1]),
        ] {
            validate_coordinate(coordinate, name, "offset_rectangle", &input)?;
        }
        let native = ffi::offset_rectangle_native(
            spec.min_mm[0],
            spec.min_mm[1],
            spec.max_mm[0],
            spec.max_mm[1],
            spec.distance_mm,
        );
        collect_output(
            native,
            "offset_rectangle",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn planar_surface_profile(
        &self,
        profile: &PlanarProfileLoop,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "planar_surface_profile";
        let input = format!("{operation}:{profile:?}");
        let PlanarProfileLoop::Segments(segments) = profile else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &input,
                "Planar surface currently requires a bounded segmented loop".to_owned(),
            ));
        };
        validate_mixed_profile(segments, operation, &input)?;
        let flattened = flatten_planar_segments(segments);
        collect_output(
            ffi::planar_surface_profile_native(&flattened),
            operation,
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn trim_surface(
        &self,
        target: &ExactBody,
        cutter: &ExactBody,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "trim_surface";
        let input = format!(
            "{operation}:{}:{}",
            target.result_fingerprint, cutter.result_fingerprint
        );
        if target.topology.solid_count != 0
            || cutter.topology.solid_count != 0
            || target.topology.face_count == 0
            || cutter.topology.face_count == 0
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface trim requires non-solid target and cutter surfaces".to_owned(),
            ));
        }
        let native_target = target.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface trim target lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native_cutter = cutter.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface trim cutter lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::trim_surface_native(native_target, native_cutter),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn extend_planar_surface(
        &self,
        target: &ExactBody,
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "extend_planar_surface";
        let input = format!(
            "{operation}:{}:{:016x}",
            target.result_fingerprint,
            distance_mm.to_bits()
        );
        validate_length(distance_mm, "distance_mm", operation, &input)?;
        if target.topology.solid_count != 0
            || target.topology.face_count != 1
            || target.topology.wire_count != 1
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface extend requires one simply bounded non-solid face".to_owned(),
            ));
        }
        let native_target = target.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface extend target lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::extend_planar_surface_native(native_target, distance_mm),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn knit_surfaces(
        &self,
        surfaces: &[&ExactBody],
        tolerance_mm: f64,
        make_solid: bool,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "knit_surfaces";
        let input = format!(
            "{operation}:{make_solid}:{:016x}:{}",
            tolerance_mm.to_bits(),
            surfaces
                .iter()
                .map(|surface| surface.result_fingerprint.as_str())
                .collect::<Vec<_>>()
                .join(":")
        );
        if !(2..=256).contains(&surfaces.len())
            || !tolerance_mm.is_finite()
            || !(1.0e-7..=10.0).contains(&tolerance_mm)
            || surfaces.iter().any(|surface| {
                surface.topology.solid_count != 0 || surface.topology.face_count == 0
            })
            || surfaces.iter().enumerate().any(|(index, surface)| {
                surfaces[..index]
                    .iter()
                    .any(|other| other.result_fingerprint == surface.result_fingerprint)
            })
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface knit requires 2..256 distinct non-solid surfaces and a tolerance from 1e-7 to 10 mm".to_owned(),
            ));
        }
        let missing_native = || GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface knit input lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        };
        let first = surfaces[0].native.as_ref().ok_or_else(missing_native)?;
        let second = surfaces[1].native.as_ref().ok_or_else(missing_native)?;
        let mut compound = collect_output(
            ffi::combine_surfaces_native(first, second),
            operation,
            &input,
            HistoryConfidence::None,
        )?;
        for surface in &surfaces[2..] {
            let added = surface.native.as_ref().ok_or_else(missing_native)?;
            let next = {
                let base = compound.body.native.as_ref().ok_or_else(missing_native)?;
                collect_output(
                    ffi::combine_surfaces_native(base, added),
                    operation,
                    &input,
                    HistoryConfidence::None,
                )?
            };
            compound = next;
        }
        let native_compound = compound.body.native.as_ref().ok_or_else(missing_native)?;
        collect_output(
            ffi::knit_surface_compound_native(native_compound, tolerance_mm, make_solid),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn thicken_surface(
        &self,
        surface: &ExactBody,
        thickness_mm: f64,
        direction: ShellDirection,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "thicken_surface";
        let input = format!(
            "{operation}:{}:{:016x}:{direction:?}",
            surface.result_fingerprint,
            thickness_mm.to_bits()
        );
        validate_length(thickness_mm, "thickness_mm", operation, &input)?;
        if surface.topology.solid_count != 0
            || surface.topology.face_count == 0
            || surface.topology.face_count > 256
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface thicken requires a valid non-solid face-bearing body".to_owned(),
            ));
        }
        let native_surface = surface.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface thicken target lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::thicken_surface_native(
                native_surface,
                thickness_mm,
                match direction {
                    ShellDirection::Inward => 0,
                    ShellDirection::Outward => 1,
                    ShellDirection::Symmetric => 2,
                },
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn offset_planar_profile(
        &self,
        profile: &PlanarProfileLoop,
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "offset_planar_profile:{profile:?}:{:016x}",
            distance_mm.to_bits()
        );
        if !distance_mm.is_finite() {
            return Err(parameter_error(
                GeometryErrorCode::NonFiniteParameter,
                "offset_planar_profile",
                &input,
                "Planar offset distance must be finite".to_owned(),
            ));
        }
        if !(MIN_LENGTH_MM..=MAX_LENGTH_MM).contains(&distance_mm.abs()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "offset_planar_profile",
                &input,
                "Planar offset distance is outside the bounded envelope".to_owned(),
            ));
        }
        if let PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } = profile
        {
            validate_circle(*center_mm, *radius_mm, "offset_planar_profile", &input)?;
            let output_radius_mm = *radius_mm + distance_mm;
            validate_circle(
                *center_mm,
                output_radius_mm,
                "offset_planar_profile",
                &input,
            )?;
            let output = collect_output(
                ffi::offset_planar_circle_native(
                    center_mm[0],
                    center_mm[1],
                    *radius_mm,
                    distance_mm,
                ),
                "offset_planar_profile",
                &input,
                HistoryConfidence::Complete,
            )?;
            let bounds = output.body.topology.bounds_mm;
            for (name, coordinate) in [
                ("output_min_x", bounds.min.x),
                ("output_min_y", bounds.min.y),
                ("output_min_z", bounds.min.z),
                ("output_max_x", bounds.max.x),
                ("output_max_y", bounds.max.y),
                ("output_max_z", bounds.max.z),
            ] {
                validate_coordinate(coordinate, name, "offset_planar_profile", &input)?;
            }
            return Ok(output);
        }
        let PlanarProfileLoop::Segments(segments) = profile else {
            unreachable!("all planar profile loop variants were handled")
        };
        validate_mixed_profile(segments, "offset_planar_profile", &input)?;
        for segment in segments {
            match segment {
                PlanarProfileSegment::Line { start_mm, end_mm } => validate_length(
                    (end_mm[0] - start_mm[0]).hypot(end_mm[1] - start_mm[1]),
                    "line_length",
                    "offset_planar_profile",
                    &input,
                )?,
                PlanarProfileSegment::CircularArc {
                    start_mm,
                    center_mm,
                    ..
                } => {
                    let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                    validate_length(radius, "arc_radius", "offset_planar_profile", &input)?;
                    for (name, coordinate) in [
                        ("arc_min_x", center_mm[0] - radius),
                        ("arc_min_y", center_mm[1] - radius),
                        ("arc_max_x", center_mm[0] + radius),
                        ("arc_max_y", center_mm[1] + radius),
                    ] {
                        validate_coordinate(coordinate, name, "offset_planar_profile", &input)?;
                    }
                }
                PlanarProfileSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => validate_length(
                    (control_1_mm[0] - start_mm[0]).hypot(control_1_mm[1] - start_mm[1])
                        + (control_2_mm[0] - control_1_mm[0])
                            .hypot(control_2_mm[1] - control_1_mm[1])
                        + (end_mm[0] - control_2_mm[0]).hypot(end_mm[1] - control_2_mm[1]),
                    "cubic_control_polygon_length",
                    "offset_planar_profile",
                    &input,
                )?,
            }
        }
        let origin = planar_segment_endpoints(&segments[0]).0;
        let mut signed_area = 0.0;
        let mut compensation = 0.0;
        for segment in segments {
            let adjusted = planar_segment_signed_area(segment, origin) - compensation;
            let next = signed_area + adjusted;
            compensation = (next - signed_area) - adjusted;
            signed_area = next;
        }
        if !signed_area.is_finite() || signed_area.abs() <= MIN_LENGTH_MM * MIN_LENGTH_MM {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                "offset_planar_profile",
                &input,
                "Planar offset loop has zero signed area".to_owned(),
            ));
        }
        let mut segments = if signed_area < 0.0 {
            reverse_planar_segments(segments)
        } else {
            segments.to_vec()
        };
        let encoded = flatten_planar_segments(&segments);
        let canonical_start = (0..segments.len())
            .min_by(|left, right| {
                for offset in 0..segments.len() {
                    for value in 0..PLANAR_SEGMENT_STRIDE {
                        let left_value = encoded
                            [((left + offset) % segments.len()) * PLANAR_SEGMENT_STRIDE + value];
                        let right_value = encoded
                            [((right + offset) % segments.len()) * PLANAR_SEGMENT_STRIDE + value];
                        let ordering = left_value.total_cmp(&right_value);
                        if !ordering.is_eq() {
                            return ordering;
                        }
                    }
                }
                std::cmp::Ordering::Equal
            })
            .expect("validated planar offset profile is non-empty");
        segments.rotate_left(canonical_start);
        let input = format!(
            "offset_planar_profile:{segments:?}:{:016x}",
            distance_mm.to_bits()
        );
        let flattened = flatten_planar_segments(&segments);
        let output = collect_output(
            ffi::offset_planar_profile_native(&flattened, distance_mm),
            "offset_planar_profile",
            &input,
            HistoryConfidence::Complete,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, "offset_planar_profile", &input)?;
        }
        Ok(output)
    }

    pub fn offset_planar_region(
        &self,
        outer: &PlanarProfileLoop,
        holes: &[PlanarProfileLoop],
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "offset_planar_region";
        let bounded_input = format!(
            "{operation}:loop_count={}:distance={:016x}",
            holes.len().saturating_add(1),
            distance_mm.to_bits()
        );
        if !distance_mm.is_finite() {
            return Err(parameter_error(
                GeometryErrorCode::NonFiniteParameter,
                operation,
                &bounded_input,
                "Planar region offset distance must be finite".to_owned(),
            ));
        }
        if !(MIN_LENGTH_MM..=MAX_LENGTH_MM).contains(&distance_mm.abs()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &bounded_input,
                "Planar region offset distance is outside the bounded envelope".to_owned(),
            ));
        }
        if holes.is_empty() || holes.len() > MAX_PLANAR_REGION_HOLES {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &bounded_input,
                "Planar region requires 1..=64 holes".to_owned(),
            ));
        }
        let mut segment_count = 0_usize;
        for planar_loop in std::iter::once(outer).chain(holes) {
            let loop_segment_count = match planar_loop {
                PlanarProfileLoop::Segments(segments) => segments.len(),
                PlanarProfileLoop::Circle { .. } => 1,
            };
            if loop_segment_count == 0 || loop_segment_count > MAX_PLANAR_LOOP_SEGMENTS {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &bounded_input,
                    "Planar region loop exceeds the segment limit".to_owned(),
                ));
            }
            segment_count = segment_count.saturating_add(loop_segment_count);
            if segment_count > MAX_PLANAR_REGION_SEGMENTS {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &bounded_input,
                    "Planar region exceeds the total segment limit".to_owned(),
                ));
            }
        }
        let (flattened, loop_segment_counts) =
            flatten_planar_region(outer, holes, operation, &bounded_input)?;
        let (flattened, loop_segment_counts) =
            canonicalize_planar_region_encoding(flattened, loop_segment_counts);
        let encoded_bits = flattened
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        let input = format!(
            "{operation}:{loop_segment_counts:?}:{encoded_bits:?}:{:016x}",
            distance_mm.to_bits()
        );
        let output = collect_output(
            ffi::offset_planar_region_native(&flattened, &loop_segment_counts, distance_mm),
            operation,
            &input,
            HistoryConfidence::Complete,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn sweep_planar_profile(
        &self,
        profile: &[PlanarProfileSegment],
        path: &[PlanarProfileSegment],
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "sweep_planar_profile";
        let profile_values = flatten_planar_segments(profile);
        let path_values = flatten_planar_segments(path);
        let input = format!(
            "{operation}:{:?}:{:?}",
            profile_values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            path_values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        validate_mixed_profile(profile, operation, &input)?;
        if !(2..=MAX_SWEEP_PATH_SEGMENTS).contains(&path.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &input,
                "Curved Sweep requires between two and 64 path segments".to_owned(),
            ));
        }
        let path_metrics = |segment: &PlanarProfileSegment| {
            let invalid = || {
                parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &input,
                    "Sweep path must contain only bounded non-degenerate lines, circular arcs, and cubic Bezier segments"
                        .to_owned(),
                )
            };
            let (start, end) = planar_segment_endpoints(segment);
            for (coordinate, name) in [
                (start[0], "path_start_x"),
                (start[1], "path_start_y"),
                (end[0], "path_end_x"),
                (end[1], "path_end_y"),
            ] {
                validate_coordinate(coordinate, name, operation, &input)?;
            }
            match segment {
                PlanarProfileSegment::Line { .. } => {
                    let direction = [end[0] - start[0], end[1] - start[1]];
                    let length = direction[0].hypot(direction[1]);
                    if length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM {
                        return Err(invalid());
                    }
                    let tangent = [direction[0] / length, direction[1] / length];
                    Ok((length, tangent, tangent))
                }
                PlanarProfileSegment::CircularArc {
                    center_mm,
                    clockwise,
                    ..
                } => {
                    validate_coordinate(center_mm[0], "path_center_x", operation, &input)?;
                    validate_coordinate(center_mm[1], "path_center_y", operation, &input)?;
                    let start_radius = [start[0] - center_mm[0], start[1] - center_mm[1]];
                    let end_radius = [end[0] - center_mm[0], end[1] - center_mm[1]];
                    let radius = start_radius[0].hypot(start_radius[1]);
                    let end_radius_length = end_radius[0].hypot(end_radius[1]);
                    if radius <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                        || (radius - end_radius_length).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM
                        || start == end
                    {
                        return Err(invalid());
                    }
                    let start_angle = start_radius[1].atan2(start_radius[0]);
                    let end_angle = end_radius[1].atan2(end_radius[0]);
                    let mut sweep = end_angle - start_angle;
                    if *clockwise {
                        if sweep >= 0.0 {
                            sweep -= std::f64::consts::TAU;
                        }
                    } else if sweep <= 0.0 {
                        sweep += std::f64::consts::TAU;
                    }
                    let length = radius * sweep.abs();
                    if length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM {
                        return Err(invalid());
                    }
                    let tangent = |radial: [f64; 2], radial_length: f64| {
                        let sign = if *clockwise { -1.0 } else { 1.0 };
                        [
                            sign * -radial[1] / radial_length,
                            sign * radial[0] / radial_length,
                        ]
                    };
                    Ok((
                        length,
                        tangent(start_radius, radius),
                        tangent(end_radius, end_radius_length),
                    ))
                }
                PlanarProfileSegment::CubicBezier {
                    control_1_mm,
                    control_2_mm,
                    ..
                } => {
                    for (coordinate, name) in [
                        (control_1_mm[0], "path_control_1_x"),
                        (control_1_mm[1], "path_control_1_y"),
                        (control_2_mm[0], "path_control_2_x"),
                        (control_2_mm[1], "path_control_2_y"),
                    ] {
                        validate_coordinate(coordinate, name, operation, &input)?;
                    }
                    let chord = [end[0] - start[0], end[1] - start[1]];
                    let start_handle = [control_1_mm[0] - start[0], control_1_mm[1] - start[1]];
                    let end_handle = [end[0] - control_2_mm[0], end[1] - control_2_mm[1]];
                    let middle = [
                        control_2_mm[0] - control_1_mm[0],
                        control_2_mm[1] - control_1_mm[1],
                    ];
                    let chord_squared = chord[0] * chord[0] + chord[1] * chord[1];
                    let start_length = start_handle[0].hypot(start_handle[1]);
                    let end_length = end_handle[0].hypot(end_handle[1]);
                    let length = start_length + middle[0].hypot(middle[1]) + end_length;
                    let projection_1 = start_handle[0] * chord[0] + start_handle[1] * chord[1];
                    let control_2_from_start =
                        [control_2_mm[0] - start[0], control_2_mm[1] - start[1]];
                    let projection_2 =
                        control_2_from_start[0] * chord[0] + control_2_from_start[1] * chord[1];
                    if start_length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                        || end_length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                        || projection_1 <= 0.0
                        || projection_2 < projection_1
                        || projection_2 >= chord_squared
                    {
                        return Err(invalid());
                    }
                    Ok((
                        length,
                        [
                            start_handle[0] / start_length,
                            start_handle[1] / start_length,
                        ],
                        [end_handle[0] / end_length, end_handle[1] / end_length],
                    ))
                }
            }
        };
        let metrics = path
            .iter()
            .map(path_metrics)
            .collect::<Result<Vec<_>, _>>()?;
        for (segments, metrics) in path.windows(2).zip(metrics.windows(2)) {
            if planar_segment_endpoints(&segments[0]).1 != planar_segment_endpoints(&segments[1]).0
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &input,
                    "Sweep path segments are disconnected".to_owned(),
                ));
            }
            let dot = metrics[0].2[0] * metrics[1].1[0] + metrics[0].2[1] * metrics[1].1[1];
            let cross = metrics[0].2[0] * metrics[1].1[1] - metrics[0].2[1] * metrics[1].1[0];
            if dot < 1.0 - 1.0e-9 || cross.abs() > 1.0e-9 {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &input,
                    "Sweep path segments must be C1 tangent-continuous".to_owned(),
                ));
            }
        }
        if sweep_path_self_intersects(path, &metrics) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &input,
                "Sweep path must not self-intersect".to_owned(),
            ));
        }
        validate_length(
            metrics.iter().map(|metrics| metrics.0).sum(),
            "path_length",
            operation,
            &input,
        )?;
        let output = collect_output(
            ffi::sweep_planar_profile_native(&profile_values, &path_values),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn sweep_spatial_profile(
        &self,
        profile: &[PlanarProfileSegment],
        path: &[SpatialProfileSegment],
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "sweep_spatial_profile";
        let profile_values = flatten_planar_segments(profile);
        let path_values = flatten_spatial_segments(path);
        debug_assert_eq!(path_values.len(), path.len() * SPATIAL_SEGMENT_STRIDE);
        let input = format!(
            "{operation}:{:?}:{:?}",
            profile_values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            path_values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        validate_mixed_profile(profile, operation, &input)?;
        validate_spatial_sweep_path(path, operation, &input)?;
        let output = collect_output(
            ffi::sweep_planar_profile_native(&profile_values, &path_values),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn loft_framed_profiles(
        &self,
        spec: &FramedLoftSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.loft_framed_profiles_with_controls(spec, None, LoftSurfaceContinuity::Position)
    }

    pub fn loft_framed_profiles_with_controls(
        &self,
        spec: &FramedLoftSpec,
        guide: Option<&[SpatialProfileSegment]>,
        continuity: LoftSurfaceContinuity,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.loft_framed_profiles_with_body_kind(spec, guide, continuity, true)
    }

    pub fn loft_framed_surface(
        &self,
        spec: &FramedLoftSpec,
        guide: Option<&[SpatialProfileSegment]>,
        continuity: LoftSurfaceContinuity,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.loft_framed_profiles_with_body_kind(spec, guide, continuity, false)
    }

    fn loft_framed_profiles_with_body_kind(
        &self,
        spec: &FramedLoftSpec,
        guide: Option<&[SpatialProfileSegment]>,
        continuity: LoftSurfaceContinuity,
        make_solid: bool,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = if make_solid {
            "loft_framed_profiles"
        } else {
            "loft_framed_surface"
        };
        if guide.is_some() && continuity == LoftSurfaceContinuity::Curvature {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                operation,
                "Guided Loft does not support curvature continuity".to_owned(),
            ));
        }
        let guide_values = if let Some(guide) = guide {
            validate_spatial_sweep_path(guide, operation, operation)?;
            flatten_spatial_segments(guide)
        } else {
            Vec::new()
        };
        let mut values = vec![spec.sections.len() as f64];
        if !(2..=16).contains(&spec.sections.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                operation,
                "Framed Loft requires 2 to 16 sections".to_owned(),
            ));
        }
        let mut previous_elevation = f64::NEG_INFINITY;
        for (section_index, section) in spec.sections.iter().enumerate() {
            validate_coordinate(
                section.elevation_mm,
                &format!("section_{section_index}_elevation"),
                operation,
                operation,
            )?;
            let x_axis = [section.frame[3], section.frame[4], section.frame[5]];
            let y_axis = [section.frame[6], section.frame[7], section.frame[8]];
            let normal = [section.frame[9], section.frame[10], section.frame[11]];
            let norm = |axis: [f64; 3]| {
                axis.into_iter()
                    .map(|value| value * value)
                    .sum::<f64>()
                    .sqrt()
            };
            let dot = |left: [f64; 3], right: [f64; 3]| {
                left.into_iter()
                    .zip(right)
                    .map(|(left, right)| left * right)
                    .sum::<f64>()
            };
            let cross = [
                x_axis[1] * y_axis[2] - x_axis[2] * y_axis[1],
                x_axis[2] * y_axis[0] - x_axis[0] * y_axis[2],
                x_axis[0] * y_axis[1] - x_axis[1] * y_axis[0],
            ];
            if section.elevation_mm <= previous_elevation
                || section
                    .frame
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > 1_000_000.0)
                || (norm(x_axis) - 1.0).abs() > 1.0e-8
                || (norm(y_axis) - 1.0).abs() > 1.0e-8
                || (norm(normal) - 1.0).abs() > 1.0e-8
                || dot(x_axis, y_axis).abs() > 1.0e-8
                || dot(x_axis, normal).abs() > 1.0e-8
                || dot(y_axis, normal).abs() > 1.0e-8
                || dot(cross, normal) < 1.0 - 1.0e-8
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidParameter,
                    operation,
                    operation,
                    "Framed Loft section frame or ordering is invalid".to_owned(),
                ));
            }
            previous_elevation = section.elevation_mm;
            let (kind, count, payload) = match &section.profile {
                FramedLoftProfile::Planar(PlanarProfileLoop::Segments(segments)) => {
                    validate_mixed_profile(segments, operation, operation)?;
                    (
                        0.0,
                        segments.len(),
                        flatten_planar_loop(&PlanarProfileLoop::Segments(segments.clone())),
                    )
                }
                FramedLoftProfile::Planar(PlanarProfileLoop::Circle {
                    center_mm,
                    radius_mm,
                }) => {
                    validate_circle(*center_mm, *radius_mm, operation, operation)?;
                    (1.0, 1, vec![center_mm[0], center_mm[1], *radius_mm])
                }
                FramedLoftProfile::Spline { control_points_mm } => {
                    if !(4..=64).contains(&control_points_mm.len()) {
                        return Err(parameter_error(
                            GeometryErrorCode::InvalidParameter,
                            operation,
                            operation,
                            "Framed Loft spline section is outside the bounded envelope".to_owned(),
                        ));
                    }
                    let mut payload = Vec::with_capacity(control_points_mm.len() * 2);
                    for (point_index, point) in control_points_mm.iter().enumerate() {
                        for (axis, coordinate) in point.iter().copied().enumerate() {
                            validate_coordinate(
                                coordinate,
                                &format!("section_{section_index}_point_{point_index}_axis_{axis}"),
                                operation,
                                operation,
                            )?;
                            payload.push(coordinate);
                        }
                    }
                    (2.0, control_points_mm.len(), payload)
                }
            };
            values.extend([kind, section.elevation_mm, count as f64]);
            values.extend(section.frame);
            values.extend(payload);
        }
        let continuity_code = match continuity {
            LoftSurfaceContinuity::Position => 0,
            LoftSurfaceContinuity::Tangent => 1,
            LoftSurfaceContinuity::Curvature => 2,
        };
        let input = format!(
            "{operation}:{:?}:{:?}:{continuity_code}:{make_solid}",
            values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            guide_values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        collect_output(
            ffi::loft_framed_profiles_native(&values, &guide_values, continuity_code, make_solid),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn loft_planar_profiles(
        &self,
        spec: &PlanarLoftSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "loft_planar_profiles";
        let mut input = format!("{operation}:{}", spec.sections.len());
        if !(2..=16).contains(&spec.sections.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Planar Loft requires 2 to 16 sections".to_owned(),
            ));
        }
        let mut segments = Vec::new();
        let mut section_segment_counts = Vec::with_capacity(spec.sections.len());
        let mut elevations = Vec::with_capacity(spec.sections.len());
        let mut previous_elevation = f64::NEG_INFINITY;
        for (section_index, section) in spec.sections.iter().enumerate() {
            validate_coordinate(
                section.elevation_mm,
                &format!("section_{section_index}_elevation"),
                operation,
                &input,
            )?;
            if section.elevation_mm <= previous_elevation {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidParameter,
                    operation,
                    &input,
                    "Planar Loft section elevations must be strictly increasing".to_owned(),
                ));
            }
            previous_elevation = section.elevation_mm;
            match &section.profile {
                PlanarProfileLoop::Segments(profile_segments) => {
                    validate_mixed_profile(profile_segments, operation, &input)?;
                    section_segment_counts.push(profile_segments.len() as u32);
                }
                PlanarProfileLoop::Circle {
                    center_mm,
                    radius_mm,
                } => {
                    validate_circle(*center_mm, *radius_mm, operation, &input)?;
                    section_segment_counts.push(1);
                }
            }
            let flattened = flatten_planar_loop(&section.profile);
            input.push_str(&format!(
                ":{}:{:016x}:{:?}",
                section_segment_counts.last().unwrap(),
                section.elevation_mm.to_bits(),
                flattened
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>()
            ));
            segments.extend(flattened);
            elevations.push(section.elevation_mm);
        }
        let output = collect_output(
            ffi::loft_planar_profiles_native(&segments, &section_segment_counts, &elevations),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn loft_spline(&self, spec: &SplineLoftSpec) -> Result<ExactOpOutput, GeometryError> {
        let mut input = format!("loft_spline:{}", spec.sections.len());
        let mut values = vec![spec.sections.len() as f64];
        if !(2..=16).contains(&spec.sections.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "loft_spline",
                &input,
                "Spline Loft requires 2 to 16 sections".to_owned(),
            ));
        }
        let mut previous_elevation = f64::NEG_INFINITY;
        for (section_index, section) in spec.sections.iter().enumerate() {
            validate_coordinate(
                section.elevation_mm,
                &format!("section_{section_index}_elevation"),
                "loft_spline",
                &input,
            )?;
            if !(4..=64).contains(&section.control_points_mm.len())
                || section.elevation_mm <= previous_elevation
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidParameter,
                    "loft_spline",
                    &input,
                    "Spline Loft sections are outside the bounded envelope".to_owned(),
                ));
            }
            previous_elevation = section.elevation_mm;
            input.push_str(&format!(
                ":{}:{:016x}",
                section.control_points_mm.len(),
                section.elevation_mm.to_bits()
            ));
            values.push(section.control_points_mm.len() as f64);
            values.push(section.elevation_mm);
            for (point_index, point) in section.control_points_mm.iter().enumerate() {
                for (axis, coordinate) in point.iter().copied().enumerate() {
                    validate_coordinate(
                        coordinate,
                        &format!("section_{section_index}_point_{point_index}_axis_{axis}"),
                        "loft_spline",
                        &input,
                    )?;
                    input.push_str(&format!(":{:016x}", coordinate.to_bits()));
                    values.push(coordinate);
                }
            }
        }
        let native = ffi::loft_spline_native(&values);
        collect_output(native, "loft_spline", &input, HistoryConfidence::Complete)
    }

    pub fn extrude_circle(&self, spec: CircleExtrudeSpec) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_circle:{:016x}:{:016x}:{:016x}:{:016x}",
            spec.center_mm[0].to_bits(),
            spec.center_mm[1].to_bits(),
            spec.radius_mm.to_bits(),
            spec.height_mm.to_bits()
        );
        validate_circle(spec.center_mm, spec.radius_mm, "extrude_circle", &input)?;
        validate_length(spec.height_mm, "height_mm", "extrude_circle", &input)?;
        let native = ffi::extrude_circle_native(
            spec.center_mm[0],
            spec.center_mm[1],
            spec.radius_mm,
            spec.height_mm,
        );
        collect_output(
            native,
            "extrude_circle",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn sweep_axial_tool(
        &self,
        spec: AxialToolSweepSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "sweep_axial_tool";
        let input = format!("{operation}:{spec:?}");
        validate_length(spec.radius_mm, "radius_mm", operation, &input)?;
        validate_length(spec.axial_length_mm, "axial_length_mm", operation, &input)?;
        let values = match spec.motion {
            AxialToolMotion::Line { start_mm, end_mm } => {
                for (index, value) in start_mm.into_iter().chain(end_mm).enumerate() {
                    validate_coordinate(value, &format!("point_{index}"), operation, &input)?;
                }
                vec![
                    0.0,
                    start_mm[0],
                    start_mm[1],
                    start_mm[2],
                    end_mm[0],
                    end_mm[1],
                    end_mm[2],
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    spec.radius_mm,
                    spec.axial_length_mm,
                ]
            }
            AxialToolMotion::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                for (index, value) in start_mm
                    .into_iter()
                    .chain(end_mm)
                    .chain(center_mm)
                    .enumerate()
                {
                    validate_coordinate(value, &format!("point_{index}"), operation, &input)?;
                }
                let start_radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                let end_radius = (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]);
                if (start_mm[2] - end_mm[2]).abs() > 1.0e-9
                    || (start_mm[2] - center_mm[2]).abs() > 1.0e-9
                    || start_radius <= 1.0e-9
                    || (start_radius - end_radius).abs() > 1.0e-7
                    || start_mm == end_mm
                {
                    return Err(parameter_error(
                        GeometryErrorCode::InvalidParameter,
                        operation,
                        &input,
                        "Axial tool arc must be planar, non-closed, concentric, and non-degenerate"
                            .to_owned(),
                    ));
                }
                vec![
                    1.0,
                    start_mm[0],
                    start_mm[1],
                    start_mm[2],
                    end_mm[0],
                    end_mm[1],
                    end_mm[2],
                    center_mm[0],
                    center_mm[1],
                    center_mm[2],
                    f64::from(clockwise),
                    spec.radius_mm,
                    spec.axial_length_mm,
                ]
            }
        };
        collect_output(
            ffi::sweep_axial_tool_native(&values),
            operation,
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn extrude_mixed_profile(
        &self,
        segments: &[PlanarProfileSegment],
        height_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_mixed_profile:{segments:?}:{:016x}",
            height_mm.to_bits()
        );
        validate_mixed_profile(segments, "extrude_mixed_profile", &input)?;
        validate_length(height_mm, "height_mm", "extrude_mixed_profile", &input)?;
        let flattened = flatten_planar_segments(segments);
        collect_output(
            ffi::extrude_mixed_profile_native(&flattened, height_mm),
            "extrude_mixed_profile",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn extrude_planar_region(
        &self,
        outer: &PlanarProfileLoop,
        holes: &[PlanarProfileLoop],
        height_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_planar_region:{outer:?}:{holes:?}:{:016x}",
            height_mm.to_bits()
        );
        validate_length(height_mm, "height_mm", "extrude_planar_region", &input)?;
        let (flattened, loop_segment_counts) =
            flatten_planar_region(outer, holes, "extrude_planar_region", &input)?;
        collect_output(
            ffi::extrude_planar_region_native(&flattened, &loop_segment_counts, height_mm),
            "extrude_planar_region",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn revolve_general_profile(
        &self,
        segments: &[PlanarProfileSegment],
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "revolve_general_profile:{segments:?}:{axis_start_mm:?}:{axis_end_mm:?}:{:016x}",
            angle_degrees.to_bits()
        );
        validate_general_revolve_profile(
            segments,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            &input,
        )?;
        let flattened = flatten_planar_segments(segments);
        collect_output(
            ffi::revolve_general_profile_native(
                &flattened,
                axis_start_mm[0],
                axis_start_mm[1],
                axis_end_mm[0],
                axis_end_mm[1],
                angle_degrees,
            ),
            "revolve_general_profile",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn revolve_planar_region(
        &self,
        outer: &PlanarProfileLoop,
        holes: &[PlanarProfileLoop],
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "revolve_planar_region:{outer:?}:{holes:?}:{axis_start_mm:?}:{axis_end_mm:?}:{:016x}",
            angle_degrees.to_bits()
        );
        validate_general_revolve_axis_angle(
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            "revolve_planar_region",
            &input,
        )?;
        let (flattened, loop_segment_counts) =
            flatten_planar_region(outer, holes, "revolve_planar_region", &input)?;
        collect_output(
            ffi::revolve_planar_region_native(
                &flattened,
                &loop_segment_counts,
                axis_start_mm[0],
                axis_start_mm[1],
                axis_end_mm[0],
                axis_end_mm[1],
                angle_degrees,
            ),
            "revolve_planar_region",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn shell_body(
        &self,
        body: &ExactBody,
        face_ordinals: &[u32],
        thickness_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.shell_body_with_direction(body, face_ordinals, thickness_mm, ShellDirection::Inward)
    }

    pub fn shell_body_with_direction(
        &self,
        body: &ExactBody,
        face_ordinals: &[u32],
        thickness_mm: f64,
        direction: ShellDirection,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "shell_body:{}:{face_ordinals:?}:{:016x}:{direction:?}",
            body.result_fingerprint,
            thickness_mm.to_bits()
        );
        validate_length(thickness_mm, "thickness_mm", "shell_body", &input)?;
        if face_ordinals.len() > 64
            || face_ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || face_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.face_count)
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "shell_body",
                &input,
                "Shell faces must be a canonical in-range selection".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "shell_body",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::shell_body_native(
                native,
                face_ordinals,
                thickness_mm,
                match direction {
                    ShellDirection::Inward => 0,
                    ShellDirection::Outward => 1,
                    ShellDirection::Symmetric => 2,
                },
            ),
            "shell_body",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn offset_body_face(
        &self,
        body: &ExactBody,
        face_ordinal: u32,
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "offset_body_face:{}:{face_ordinal}:{:016x}",
            body.result_fingerprint,
            distance_mm.to_bits()
        );
        validate_length(distance_mm.abs(), "distance_mm", "offset_body_face", &input)?;
        if face_ordinal >= body.topology.face_count {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "offset_body_face",
                &input,
                "Face offset requires one in-range face and a non-zero distance".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "offset_body_face",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::offset_body_face_native(native, face_ordinal, distance_mm),
            "offset_body_face",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn finish_body(
        &self,
        body: &ExactBody,
        edge_ordinals: &[u32],
        finish: EdgeFinish,
        amount_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.finish_body_with_radius_stations(body, edge_ordinals, finish, amount_mm, &[])
    }

    pub fn finish_body_with_radius_stations(
        &self,
        body: &ExactBody,
        edge_ordinals: &[u32],
        finish: EdgeFinish,
        amount_mm: f64,
        fillet_radius_stations: &[[f64; 2]],
    ) -> Result<ExactOpOutput, GeometryError> {
        let station_bits = fillet_radius_stations
            .iter()
            .map(|station| station.map(f64::to_bits))
            .collect::<Vec<_>>();
        let input = format!(
            "finish_body:{}:{edge_ordinals:?}:{finish:?}:{:016x}:{station_bits:?}",
            body.result_fingerprint,
            amount_mm.to_bits()
        );
        validate_length(amount_mm, "amount_mm", "finish_body", &input)?;
        let mut previous_position = 0.0;
        if fillet_radius_stations.len() > 32
            || (finish != EdgeFinish::Fillet && !fillet_radius_stations.is_empty())
            || (!fillet_radius_stations.is_empty()
                && fillet_radius_stations.last().map(|station| station[0]) != Some(1.0))
            || fillet_radius_stations.iter().any(|station| {
                let valid = station[0].is_finite()
                    && station[0] > previous_position
                    && station[0] <= 1.0
                    && validate_length(station[1], "radius", "finish_body", &input).is_ok();
                previous_position = station[0];
                !valid
            })
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "finish_body",
                &input,
                "Variable fillet stations are invalid or non-canonical".to_owned(),
            ));
        }
        if edge_ordinals.is_empty()
            || edge_ordinals.len() > 64
            || edge_ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || edge_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.edge_count)
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "finish_body",
                &input,
                "Finish edges must be a non-empty canonical in-range selection".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "finish_body",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let flattened_stations = fillet_radius_stations
            .iter()
            .flat_map(|station| *station)
            .collect::<Vec<_>>();
        collect_output(
            ffi::finish_body_native(
                native,
                edge_ordinals,
                &[],
                amount_mm,
                finish == EdgeFinish::Fillet,
                &flattened_stations,
                0,
                0.0,
            ),
            "finish_body",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn finish_body_advanced_chamfer(
        &self,
        body: &ExactBody,
        edge_ordinals: &[u32],
        face_ordinals: &[u32],
        amount_mm: f64,
        mode: AdvancedChamferMode,
    ) -> Result<ExactOpOutput, GeometryError> {
        let (mode_code, secondary) = match mode {
            AdvancedChamferMode::TwoDistance { second_distance_mm } => (1, second_distance_mm),
            AdvancedChamferMode::DistanceAngle { angle_degrees } => (2, angle_degrees),
        };
        let input = format!(
            "finish_body_advanced_chamfer:{}:{edge_ordinals:?}:{face_ordinals:?}:{:016x}:{mode_code}:{:016x}",
            body.result_fingerprint,
            amount_mm.to_bits(),
            secondary.to_bits()
        );
        validate_length(
            amount_mm,
            "amount_mm",
            "finish_body_advanced_chamfer",
            &input,
        )?;
        let valid_secondary = match mode {
            AdvancedChamferMode::TwoDistance { second_distance_mm } => validate_length(
                second_distance_mm,
                "second_distance_mm",
                "finish_body_advanced_chamfer",
                &input,
            )
            .is_ok(),
            AdvancedChamferMode::DistanceAngle { angle_degrees } => {
                angle_degrees.is_finite() && angle_degrees > 0.1 && angle_degrees < 89.9
            }
        };
        if edge_ordinals.is_empty()
            || edge_ordinals.len() > 64
            || edge_ordinals.len() != face_ordinals.len()
            || edge_ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || edge_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.edge_count)
            || face_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.face_count)
            || edge_ordinals
                .iter()
                .zip(face_ordinals)
                .any(|(edge_ordinal, face_ordinal)| {
                    !body.topology.edges.iter().any(|edge| {
                        edge.ordinal == *edge_ordinal
                            && edge.adjacent_face_ordinals.contains(face_ordinal)
                    })
                })
            || !valid_secondary
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "finish_body_advanced_chamfer",
                &input,
                "Advanced chamfer edge/face pairs or parameters are invalid".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "finish_body_advanced_chamfer",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::finish_body_native(
                native,
                edge_ordinals,
                face_ordinals,
                amount_mm,
                false,
                &[],
                mode_code,
                secondary,
            ),
            "finish_body_advanced_chamfer",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn exception_probe(&self) -> Result<ExactOpOutput, GeometryError> {
        collect_output(
            ffi::exception_probe_native(),
            "exception_probe",
            "intentional",
            HistoryConfidence::None,
        )
    }

    pub fn import_step(&self, path: &str) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step:{path}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_native(path),
            "import_step",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn import_step_solid(
        &self,
        path: &str,
        solid_ordinal: u32,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step_solid:{path}:{solid_ordinal}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step_solid",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_solid_native(path, solid_ordinal),
            "import_step_solid",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn import_step_xde_part(
        &self,
        path: &str,
        part_index: u32,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step_xde_part:{path}:{part_index}");
        if path.trim().is_empty() || part_index as usize >= MAX_STEP_XDE_NODES {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step_xde_part",
                &input,
                "STEP XDE path or part index is invalid".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_xde_part_native(path, part_index),
            "import_step_xde_part",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn step_xde_manifest(&self, path: &str) -> Result<StepXdeManifest, StepXdeManifestError> {
        if path.trim().is_empty() {
            return Err(StepXdeManifestError::InvalidPath);
        }
        parse_step_xde_manifest(&ffi::step_xde_manifest_native(path))
    }

    pub fn export_step_xde_assembly(
        &self,
        parts: &[StepXdeExportPart],
        nodes: &[StepXdeExportNode],
        path: &str,
    ) -> Result<(), GeometryError> {
        self.export_xde_assembly(parts, nodes, path, false)
    }

    pub fn export_iges_xde_assembly(
        &self,
        parts: &[StepXdeExportPart],
        nodes: &[StepXdeExportNode],
        path: &str,
    ) -> Result<(), GeometryError> {
        self.export_xde_assembly(parts, nodes, path, true)
    }

    fn export_xde_assembly(
        &self,
        parts: &[StepXdeExportPart],
        nodes: &[StepXdeExportNode],
        path: &str,
        iges: bool,
    ) -> Result<(), GeometryError> {
        let operation = if iges {
            "export_iges_xde_assembly"
        } else {
            "export_step_xde_assembly"
        };
        let input = format!("{operation}:{}:{}:{path}", parts.len(), nodes.len());
        let invalid = || {
            parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "XDE assembly manifest is malformed or outside the bounded envelope".to_owned(),
            )
        };
        if path.trim().is_empty()
            || parts.is_empty()
            || parts.len() > MAX_STEP_XDE_NODES
            || nodes.is_empty()
            || nodes.len() > MAX_STEP_XDE_NODES
            || parts.iter().any(|part| {
                part.path.trim().is_empty()
                    || part.name.is_empty()
                    || part.name.len() > 4_096
                    || part.name.chars().any(char::is_control)
            })
        {
            return Err(invalid());
        }
        let mut used_parts = vec![false; parts.len()];
        for (index, node) in nodes.iter().enumerate() {
            if node.name.is_empty()
                || node.name.len() > 4_096
                || node.name.chars().any(char::is_control)
                || node
                    .parent_id
                    .is_some_and(|parent| parent as usize >= index)
                || node
                    .part_index
                    .is_some_and(|part| part as usize >= parts.len())
                || node
                    .parent_id
                    .is_some_and(|parent| nodes[parent as usize].part_index.is_some())
                || !transform_is_rigid(&node.transform)
            {
                return Err(invalid());
            }
            if let Some(part) = node.part_index {
                used_parts[part as usize] = true;
            }
            let mut depth = 0usize;
            let mut parent = node.parent_id;
            while let Some(parent_id) = parent {
                depth += 1;
                if depth > 64 {
                    return Err(invalid());
                }
                parent = nodes[parent_id as usize].parent_id;
            }
        }
        if used_parts.iter().any(|used| !used) {
            return Err(invalid());
        }
        let encode = |value: &str| {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            let mut output = String::with_capacity(value.len() * 2);
            for byte in value.as_bytes() {
                output.push(DIGITS[(byte >> 4) as usize] as char);
                output.push(DIGITS[(byte & 0x0f) as usize] as char);
            }
            output
        };
        let mut manifest = String::from("KETCHUP_STEP_XDE_EXPORT_V1\n");
        for part in parts {
            use std::fmt::Write as _;
            writeln!(
                manifest,
                "P\t{}\t{}",
                encode(&part.path),
                encode(&part.name)
            )
            .expect("writing to String cannot fail");
        }
        for node in nodes {
            use std::fmt::Write as _;
            let parent = node.parent_id.map_or(-1_i64, i64::from);
            let part = node.part_index.map_or(-1_i64, i64::from);
            let color = node.color.map_or_else(
                || "-".to_owned(),
                |[red, green, blue]| format!("{red:02x}{green:02x}{blue:02x}"),
            );
            write!(
                manifest,
                "N\t{parent}\t{part}\t{}\t{color}",
                encode(&node.name)
            )
            .expect("writing to String cannot fail");
            for value in node.transform {
                write!(manifest, "\t{:016x}", value.to_bits())
                    .expect("writing to String cannot fail");
            }
            manifest.push('\n');
        }
        let diagnostic = if iges {
            ffi::export_iges_xde_assembly_native(&manifest, path)
        } else {
            ffi::export_step_xde_assembly_native(&manifest, path)
        };
        if diagnostic.is_empty() {
            Ok(())
        } else {
            Err(GeometryError {
                code: GeometryErrorCode::BackendException,
                diagnostic,
                operation,
                input_digest: stable_digest(&input),
                backend_fingerprint: BACKEND_FINGERPRINT,
            })
        }
    }

    #[must_use]
    pub fn step_length_unit_name(&self, path: &str) -> Option<String> {
        let unit = ffi::step_length_unit_native(path);
        (!unit.is_empty()).then_some(unit)
    }

    pub fn import_iges(&self, path: &str) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_iges:{path}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_iges",
                &input,
                "IGES path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_iges_native(path),
            "import_iges",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn import_iges_xde_part(
        &self,
        path: &str,
        part_index: u32,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_iges_xde_part:{path}:{part_index}");
        if path.trim().is_empty() || part_index as usize >= MAX_STEP_XDE_NODES {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_iges_xde_part",
                &input,
                "IGES XDE path or part index is invalid".to_owned(),
            ));
        }
        collect_output(
            ffi::import_iges_xde_part_native(path, part_index),
            "import_iges_xde_part",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn iges_xde_manifest(&self, path: &str) -> Result<StepXdeManifest, StepXdeManifestError> {
        if path.trim().is_empty() {
            return Err(StepXdeManifestError::InvalidPath);
        }
        parse_step_xde_manifest(&ffi::iges_xde_manifest_native(path))
    }

    #[must_use]
    pub fn iges_length_unit_name(&self, path: &str) -> Option<String> {
        let unit = ffi::iges_length_unit_native(path);
        (!unit.is_empty()).then_some(unit)
    }

    pub fn transform_body(
        &self,
        body: &ExactBody,
        matrix: &[f64; 16],
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("transform_body:{}:{matrix:?}", body.result_fingerprint);
        if !matrix.iter().all(|value| value.is_finite())
            || matrix[12] != 0.0
            || matrix[13] != 0.0
            || matrix[14] != 0.0
            || matrix[15] != 1.0
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "transform_body",
                &input,
                "body transform must be a finite affine 4x4 matrix".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "transform_body",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::transform_body_native(native, matrix),
            "transform_body",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn combine_bodies(
        &self,
        base: &ExactBody,
        added: &ExactBody,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "combine_bodies:{}:{}",
            base.result_fingerprint, added.result_fingerprint
        );
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Base exact body lost its owned native shape".to_owned(),
            operation: "combine_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native_added = added.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Added exact body lost its owned native shape".to_owned(),
            operation: "combine_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::combine_bodies_native(native_base, native_added),
            "combine_bodies",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn trim_body_by_plane(
        &self,
        body: &ExactBody,
        origin_mm: [f64; 3],
        normal: [f64; 3],
        keep_point_mm: [f64; 3],
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "trim_body_by_plane:{}:{origin_mm:?}:{normal:?}:{keep_point_mm:?}",
            body.result_fingerprint
        );
        if origin_mm
            .into_iter()
            .chain(normal)
            .chain(keep_point_mm)
            .any(|value| !value.is_finite())
            || normal.iter().map(|value| value * value).sum::<f64>() <= 1.0e-18
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "trim_body_by_plane",
                &input,
                "Plane trim requires finite points and a non-degenerate normal".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Plane trim body lost its owned native shape".to_owned(),
            operation: "trim_body_by_plane",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::trim_body_by_plane_native(
                native,
                origin_mm[0],
                origin_mm[1],
                origin_mm[2],
                normal[0],
                normal[1],
                normal[2],
                keep_point_mm[0],
                keep_point_mm[1],
                keep_point_mm[2],
            ),
            "trim_body_by_plane",
            &input,
            HistoryConfidence::Partial,
        )
    }

    /// Read-only narrow phase. Empty common results are valid; backend failures
    /// are always errors. Contact tolerance is in mm and never suppresses positive volume.
    pub fn query_body_pair(
        &self,
        left: &ExactBody,
        right: &ExactBody,
        contact_tolerance_mm: f64,
    ) -> Result<ExactPairQueryResult, GeometryError> {
        let input = format!(
            "pair:{}:{}:{contact_tolerance_mm:?}",
            left.result_fingerprint, right.result_fingerprint
        );
        let error = |code, diagnostic| parameter_error(code, "query_body_pair", &input, diagnostic);
        if !contact_tolerance_mm.is_finite() || contact_tolerance_mm < 0.0 {
            return Err(error(
                GeometryErrorCode::InvalidParameter,
                "Contact tolerance must be finite and nonnegative".to_owned(),
            ));
        }
        let left = left.native.as_ref().ok_or_else(|| {
            error(
                GeometryErrorCode::NullResult,
                "Left native body unavailable".to_owned(),
            )
        })?;
        let right = right.native.as_ref().ok_or_else(|| {
            error(
                GeometryErrorCode::NullResult,
                "Right native body unavailable".to_owned(),
            )
        })?;
        let result = ffi::query_body_pair_native(left, right);
        if result.status != 0 {
            return Err(error(
                if result.status == 6 {
                    GeometryErrorCode::BackendException
                } else {
                    GeometryErrorCode::InvalidShape
                },
                result.diagnostic,
            ));
        }
        if !result.common_volume_mm3.is_finite()
            || result.common_volume_mm3 < 0.0
            || !result.common_contact_area_mm2.is_finite()
            || result.common_contact_area_mm2 < 0.0
            || !result.distance_mm.is_finite()
            || result.distance_mm < 0.0
            || (result.common_volume_mm3 > 0.0
                && (result.common_contact_area_mm2 > 0.0 || result.distance_mm != 0.0))
            || (result.common_contact_area_mm2 > 0.0 && result.distance_mm != 0.0)
        {
            return Err(error(
                GeometryErrorCode::InvalidShape,
                "Invalid native pair measurements".to_owned(),
            ));
        }
        Ok(ExactPairQueryResult {
            relation: if result.common_volume_mm3 > 0.0 {
                ExactPairRelation::Penetrating
            } else if result.distance_mm <= contact_tolerance_mm {
                ExactPairRelation::Touching
            } else {
                ExactPairRelation::Separated
            },
            common_volume_mm3: result.common_volume_mm3,
            common_contact_area_mm2: result.common_contact_area_mm2,
            distance_mm: result.distance_mm,
        })
    }

    pub fn boolean_bodies(
        &self,
        target: &ExactBody,
        tool: &ExactBody,
        operation: ExactBodyBooleanOperation,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "boolean_bodies:{operation:?}:{}:{}",
            target.result_fingerprint, tool.result_fingerprint
        );
        let native_target = target.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Target exact body lost its owned native shape".to_owned(),
            operation: "boolean_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native_tool = tool.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Tool exact body lost its owned native shape".to_owned(),
            operation: "boolean_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let operation_code = match operation {
            ExactBodyBooleanOperation::Cut => 0,
            ExactBodyBooleanOperation::Union => 1,
            ExactBodyBooleanOperation::Intersect => 2,
            ExactBodyBooleanOperation::Split => 3,
        };
        collect_output(
            ffi::boolean_bodies_native(native_target, native_tool, operation_code),
            "boolean_bodies",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn export_step(&self, body: &ExactBody, path: &str) -> Result<(), GeometryError> {
        let input = format!("export_step:{}:{path}", body.result_fingerprint);
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "export_step",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "export_step",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let diagnostic = ffi::export_step_native(native, path);
        if diagnostic.is_empty() {
            Ok(())
        } else {
            Err(GeometryError {
                code: GeometryErrorCode::BackendException,
                diagnostic,
                operation: "export_step",
                input_digest: stable_digest(&input),
                backend_fingerprint: BACKEND_FINGERPRINT,
            })
        }
    }

    pub fn export_iges(&self, body: &ExactBody, path: &str) -> Result<(), GeometryError> {
        let input = format!("export_iges:{}:{path}", body.result_fingerprint);
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "export_iges",
                &input,
                "IGES path must not be empty".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "export_iges",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let diagnostic = ffi::export_iges_native(native, path);
        if diagnostic.is_empty() {
            Ok(())
        } else {
            Err(GeometryError {
                code: GeometryErrorCode::BackendException,
                diagnostic,
                operation: "export_iges",
                input_digest: stable_digest(&input),
                backend_fingerprint: BACKEND_FINGERPRINT,
            })
        }
    }

    /// Tessellate an exact body into a display mesh.
    ///
    /// The mesh is derived, never canonical: it exists so an exact body that
    /// carries no analytic render geometry — an imported STEP part — is still
    /// visible and pickable. Deflections are supplied by the caller so the
    /// same body always yields the same triangles.
    pub fn tessellate_body(
        &self,
        body: &ExactBody,
        deflection: f64,
        angular_deflection: f64,
        max_triangles: u32,
    ) -> Result<ExactTessellation, GeometryError> {
        let input = format!(
            "tessellate_body:{}:{}:{}:{max_triangles}",
            body.result_fingerprint,
            deflection.to_bits(),
            angular_deflection.to_bits()
        );
        let input_digest = stable_digest(&input);
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "tessellate_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let mesh =
            ffi::tessellate_body_native(native, deflection, angular_deflection, max_triangles);
        let mesh = mesh.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Native facade returned no tessellation object".to_owned(),
            operation: "tessellate_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        if mesh.mesh_status_code() != 0 {
            return Err(GeometryError {
                code: native_status(mesh.mesh_status_code()),
                diagnostic: mesh.mesh_diagnostic(),
                operation: "tessellate_body",
                input_digest,
                backend_fingerprint: BACKEND_FINGERPRINT,
            });
        }
        let vertices_mm = mesh
            .mesh_vertices()
            .into_iter()
            .map(|vertex| [vertex.x_mm, vertex.y_mm, vertex.z_mm])
            .collect::<Vec<_>>();
        let triangles = mesh
            .mesh_triangles()
            .into_iter()
            .map(|triangle| ExactMeshTriangle {
                vertex_indices: [triangle.first, triangle.second, triangle.third],
                face_ordinal: triangle.face_ordinal,
            })
            .collect::<Vec<_>>();
        let vertex_count = vertices_mm.len();
        if triangles.is_empty()
            || vertices_mm
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
            || triangles.iter().any(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= vertex_count)
            })
        {
            return Err(GeometryError {
                code: GeometryErrorCode::InvalidShape,
                diagnostic: "Native tessellation is not a well-formed indexed mesh".to_owned(),
                operation: "tessellate_body",
                input_digest,
                backend_fingerprint: BACKEND_FINGERPRINT,
            });
        }
        Ok(ExactTessellation {
            vertices_mm,
            triangles,
        })
    }

    /// Build a bounded tetrahedral mesh from one exact solid.
    ///
    /// The native mesher cones an OCCT face tessellation to a verified interior
    /// point. It therefore accepts only solids whose complete tessellated
    /// boundary is visible from that point and fails closed for unsupported
    /// non-star-shaped domains. The returned mesh is additionally checked for
    /// manifold connectivity, positive Jacobians, quality and volume error.
    pub fn volume_mesh_body(
        &self,
        body: &ExactBody,
        options: ExactVolumeMeshOptions,
    ) -> Result<ExactVolumeMesh, GeometryError> {
        let input = format!(
            "volume_mesh_body:{}:{:016x}:{:016x}:{}:{:016x}:{:016x}",
            body.result_fingerprint,
            options.surface_deflection_mm.to_bits(),
            options.angular_deflection_rad.to_bits(),
            options.max_tetrahedra,
            options.max_relative_volume_error.to_bits(),
            options.min_tetrahedron_quality.to_bits(),
        );
        let input_digest = stable_digest(&input);
        let invalid_options = !options.surface_deflection_mm.is_finite()
            || options.surface_deflection_mm <= 0.0
            || options.surface_deflection_mm > MAX_LENGTH_MM
            || !options.angular_deflection_rad.is_finite()
            || options.angular_deflection_rad <= 0.0
            || options.angular_deflection_rad > std::f64::consts::PI
            || !(4..=65_536).contains(&options.max_tetrahedra)
            || !options.max_relative_volume_error.is_finite()
            || !(1.0e-12..=0.25).contains(&options.max_relative_volume_error)
            || !options.min_tetrahedron_quality.is_finite()
            || !(0.0..=1.0).contains(&options.min_tetrahedron_quality);
        if invalid_options {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "volume_mesh_body",
                &input,
                "Volume-mesh options are non-finite or outside bounded ranges".to_owned(),
            ));
        }
        if body.topology.solid_count != 1 || body.topology.volume_mm3 <= 0.0 {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Volume meshing requires exactly one closed exact solid".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "volume_mesh_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let mesh = ffi::volume_mesh_body_native(
            native,
            options.surface_deflection_mm,
            options.angular_deflection_rad,
            options.max_tetrahedra,
        );
        let mesh = mesh.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Native facade returned no volume-mesh object".to_owned(),
            operation: "volume_mesh_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        if mesh.volume_mesh_status_code() != 0 {
            return Err(GeometryError {
                code: native_status(mesh.volume_mesh_status_code()),
                diagnostic: mesh.volume_mesh_diagnostic(),
                operation: "volume_mesh_body",
                input_digest,
                backend_fingerprint: BACKEND_FINGERPRINT,
            });
        }

        let vertices_mm = mesh
            .volume_mesh_vertices()
            .into_iter()
            .map(|vertex| [vertex.x_mm, vertex.y_mm, vertex.z_mm])
            .collect::<Vec<_>>();
        let tetrahedra = mesh
            .volume_mesh_tetrahedra()
            .into_iter()
            .map(|tetrahedron| ExactVolumeMeshTetrahedron {
                vertex_indices: [
                    tetrahedron.first,
                    tetrahedron.second,
                    tetrahedron.third,
                    tetrahedron.fourth,
                ],
            })
            .collect::<Vec<_>>();
        let boundary_triangles = mesh
            .volume_mesh_boundary_triangles()
            .into_iter()
            .map(|triangle| ExactMeshTriangle {
                vertex_indices: [triangle.first, triangle.second, triangle.third],
                face_ordinal: triangle.face_ordinal,
            })
            .collect::<Vec<_>>();
        let vertex_count = vertices_mm.len();
        if vertices_mm.len() < 4
            || tetrahedra.is_empty()
            || tetrahedra.len() > options.max_tetrahedra as usize
            || boundary_triangles.is_empty()
            || vertices_mm
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
            || tetrahedra.iter().any(|tetrahedron| {
                tetrahedron
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= vertex_count)
                    || {
                        let mut unique = tetrahedron.vertex_indices;
                        unique.sort_unstable();
                        unique.windows(2).any(|pair| pair[0] == pair[1])
                    }
            })
            || boundary_triangles.iter().any(|triangle| {
                triangle.face_ordinal >= body.topology.face_count
                    || triangle
                        .vertex_indices
                        .iter()
                        .any(|index| *index as usize >= vertex_count)
            })
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Native volume mesh is not a bounded well-formed indexed mesh".to_owned(),
            ));
        }

        let mut face_uses = std::collections::BTreeMap::<[u32; 3], u32>::new();
        let mut tetrahedral_volume_mm3 = 0.0;
        let mut minimum_signed_volume_mm3 = f64::INFINITY;
        let mut minimum_quality = f64::INFINITY;
        let mut maximum_edge_ratio = 0.0_f64;
        for tetrahedron in &tetrahedra {
            let indices = tetrahedron.vertex_indices;
            for face in [
                [indices[0], indices[1], indices[2]],
                [indices[0], indices[1], indices[3]],
                [indices[0], indices[2], indices[3]],
                [indices[1], indices[2], indices[3]],
            ] {
                let mut key = face;
                key.sort_unstable();
                *face_uses.entry(key).or_default() += 1;
            }
            let points = indices.map(|index| vertices_mm[index as usize]);
            let ab = subtract3(points[1], points[0]);
            let ac = subtract3(points[2], points[0]);
            let ad = subtract3(points[3], points[0]);
            let signed_volume_mm3 = dot3(ab, cross3(ac, ad)) / 6.0;
            if !signed_volume_mm3.is_finite() || signed_volume_mm3 <= 1.0e-15 {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "volume_mesh_body",
                    &input,
                    "Volume mesh contains an inverted or degenerate tetrahedron".to_owned(),
                ));
            }
            let edge_squared = [
                squared_distance3(points[0], points[1]),
                squared_distance3(points[0], points[2]),
                squared_distance3(points[0], points[3]),
                squared_distance3(points[1], points[2]),
                squared_distance3(points[1], points[3]),
                squared_distance3(points[2], points[3]),
            ];
            let minimum_edge_squared = edge_squared.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum_edge_squared = edge_squared.iter().copied().fold(0.0_f64, f64::max);
            let quality =
                12.0 * (3.0 * signed_volume_mm3).powf(2.0 / 3.0) / edge_squared.iter().sum::<f64>();
            if !quality.is_finite()
                || quality < options.min_tetrahedron_quality
                || minimum_edge_squared <= 0.0
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "volume_mesh_body",
                    &input,
                    "Volume mesh violates the requested tetrahedron quality bound".to_owned(),
                ));
            }
            tetrahedral_volume_mm3 += signed_volume_mm3;
            minimum_signed_volume_mm3 = minimum_signed_volume_mm3.min(signed_volume_mm3);
            minimum_quality = minimum_quality.min(quality);
            maximum_edge_ratio =
                maximum_edge_ratio.max((maximum_edge_squared / minimum_edge_squared).sqrt());
        }

        let expected_boundary = face_uses
            .iter()
            .filter_map(|(face, count)| (*count == 1).then_some(*face))
            .collect::<std::collections::BTreeSet<_>>();
        if face_uses.values().any(|count| *count > 2) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Volume mesh contains non-manifold tetrahedral faces".to_owned(),
            ));
        }
        let actual_boundary = boundary_triangles
            .iter()
            .map(|triangle| {
                let mut face = triangle.vertex_indices;
                face.sort_unstable();
                face
            })
            .collect::<std::collections::BTreeSet<_>>();
        if actual_boundary.len() != boundary_triangles.len() || actual_boundary != expected_boundary
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Boundary provenance does not match the tetrahedral boundary".to_owned(),
            ));
        }
        let mut boundary_edges = std::collections::BTreeMap::<[u32; 2], u32>::new();
        for triangle in &boundary_triangles {
            let indices = triangle.vertex_indices;
            for mut edge in [
                [indices[0], indices[1]],
                [indices[1], indices[2]],
                [indices[2], indices[0]],
            ] {
                edge.sort_unstable();
                *boundary_edges.entry(edge).or_default() += 1;
            }
        }
        if boundary_edges.values().any(|count| *count != 2) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Tetrahedral boundary is open or non-manifold".to_owned(),
            ));
        }

        let exact_volume_mm3 = body.topology.volume_mm3;
        let relative_volume_error =
            (tetrahedral_volume_mm3 - exact_volume_mm3).abs() / exact_volume_mm3;
        if !relative_volume_error.is_finite()
            || relative_volume_error > options.max_relative_volume_error
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                format!(
                    "Tetrahedral volume relative error {relative_volume_error:.6e} exceeds {:.6e}",
                    options.max_relative_volume_error
                ),
            ));
        }

        let mut fingerprint_input = format!(
            "{EXACT_VOLUME_MESH_SCHEMA}:{}:{input_digest}",
            body.result_fingerprint
        );
        for vertex in &vertices_mm {
            for coordinate in vertex {
                fingerprint_input.push_str(&format!(":{:016x}", coordinate.to_bits()));
            }
        }
        for tetrahedron in &tetrahedra {
            fingerprint_input.push_str(&format!(":{:?}", tetrahedron.vertex_indices));
        }
        for triangle in &boundary_triangles {
            fingerprint_input.push_str(&format!(
                ":{:?}:{}",
                triangle.vertex_indices, triangle.face_ordinal
            ));
        }
        let mesh_fingerprint = stable_digest(&fingerprint_input);
        Ok(ExactVolumeMesh {
            schema: EXACT_VOLUME_MESH_SCHEMA,
            source_result_fingerprint: body.result_fingerprint.clone(),
            request_digest: input_digest,
            mesh_fingerprint,
            vertices_mm,
            tetrahedra,
            boundary_triangles,
            exact_volume_mm3,
            tetrahedral_volume_mm3,
            relative_volume_error,
            minimum_signed_volume_mm3,
            minimum_quality,
            maximum_edge_ratio,
        })
    }
}

fn collect_output(
    native: cxx::UniquePtr<ffi::NativeOperationResult>,
    operation: &'static str,
    input: &str,
    history_confidence: HistoryConfidence,
) -> Result<ExactOpOutput, GeometryError> {
    let input_digest = stable_digest(input);
    let native_ref = native.as_ref().ok_or_else(|| GeometryError {
        code: GeometryErrorCode::NullResult,
        diagnostic: "Native facade returned no result object".to_owned(),
        operation,
        input_digest: input_digest.clone(),
        backend_fingerprint: BACKEND_FINGERPRINT,
    })?;
    let status = native_ref.status_code();
    let diagnostic = native_ref.diagnostic();
    if status != 0 || !native_ref.valid() {
        return Err(GeometryError {
            code: native_status(status),
            diagnostic,
            operation,
            input_digest,
            backend_fingerprint: BACKEND_FINGERPRINT,
        });
    }

    let summary = native_ref.topology_summary();
    let face_edges = native_ref.face_edge_evidence();
    let edge_faces = native_ref.edge_face_evidence();
    let faces = native_ref
        .face_evidence()
        .into_iter()
        .map(|face| {
            let axis_origin_mm = face.has_axis.then_some(Point3 {
                x: face.axis_origin_x,
                y: face.axis_origin_y,
                z: face.axis_origin_z,
            });
            let axis_direction = face.has_axis.then_some(Point3 {
                x: face.axis_direction_x,
                y: face.axis_direction_y,
                z: face.axis_direction_z,
            });
            let signature = if face.has_axis {
                format!(
                    "axis-v2:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                    face.ordinal,
                    face.surface_kind,
                    face.area_mm2.to_bits(),
                    face.centroid_x.to_bits(),
                    face.centroid_y.to_bits(),
                    face.centroid_z.to_bits(),
                    face.normal_x.to_bits(),
                    face.normal_y.to_bits(),
                    face.normal_z.to_bits(),
                    face.axis_origin_x.to_bits(),
                    face.axis_origin_y.to_bits(),
                    face.axis_origin_z.to_bits(),
                    face.axis_direction_x.to_bits(),
                    face.axis_direction_y.to_bits(),
                    face.axis_direction_z.to_bits(),
                    face.edge_count
                )
            } else {
                format!(
                    "{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                    face.ordinal,
                    face.surface_kind,
                    face.area_mm2.to_bits(),
                    face.centroid_x.to_bits(),
                    face.centroid_y.to_bits(),
                    face.centroid_z.to_bits(),
                    face.normal_x.to_bits(),
                    face.normal_y.to_bits(),
                    face.edge_count
                )
            };
            FaceEvidence {
                ordinal: face.ordinal,
                surface_kind: face.surface_kind,
                area_mm2: face.area_mm2,
                centroid_mm: Point3 {
                    x: face.centroid_x,
                    y: face.centroid_y,
                    z: face.centroid_z,
                },
                normal: Point3 {
                    x: face.normal_x,
                    y: face.normal_y,
                    z: face.normal_z,
                },
                axis_origin_mm,
                axis_direction,
                bounds_mm: Bounds3 {
                    min: Point3 {
                        x: face.min_x,
                        y: face.min_y,
                        z: face.min_z,
                    },
                    max: Point3 {
                        x: face.max_x,
                        y: face.max_y,
                        z: face.max_z,
                    },
                },
                edge_count: face.edge_count,
                edge_ordinals: {
                    let mut ordinals = face_edges
                        .iter()
                        .filter(|entry| entry.face_ordinal == face.ordinal)
                        .map(|entry| entry.edge_ordinal)
                        .collect::<Vec<_>>();
                    ordinals.sort_unstable();
                    ordinals
                },
                geometric_fingerprint: stable_digest(&signature),
            }
        })
        .collect::<Vec<_>>();
    let topology = TopologyEvidence {
        vertex_count: summary.vertex_count,
        edge_count: summary.edge_count,
        wire_count: summary.wire_count,
        face_count: summary.face_count,
        shell_count: summary.shell_count,
        solid_count: summary.solid_count,
        volume_mm3: summary.volume_mm3,
        bounds_mm: Bounds3 {
            min: Point3 {
                x: summary.min_x,
                y: summary.min_y,
                z: summary.min_z,
            },
            max: Point3 {
                x: summary.max_x,
                y: summary.max_y,
                z: summary.max_z,
            },
        },
        faces,
        edges: native_ref
            .edge_evidence()
            .into_iter()
            .map(|edge| {
                let mut adjacent_face_ordinals = edge_faces
                    .iter()
                    .filter(|entry| entry.edge_ordinal == edge.ordinal)
                    .map(|entry| entry.face_ordinal)
                    .collect::<Vec<_>>();
                adjacent_face_ordinals.sort_unstable();
                let axis_origin_mm = edge.has_axis.then_some(Point3 {
                    x: edge.axis_origin_x,
                    y: edge.axis_origin_y,
                    z: edge.axis_origin_z,
                });
                let axis_direction = edge.has_axis.then_some(Point3 {
                    x: edge.axis_direction_x,
                    y: edge.axis_direction_y,
                    z: edge.axis_direction_z,
                });
                EdgeEvidence {
                    ordinal: edge.ordinal,
                    curve_kind: edge.curve_kind,
                    length_mm: edge.length_mm,
                    centroid_mm: Point3 {
                        x: edge.centroid_x,
                        y: edge.centroid_y,
                        z: edge.centroid_z,
                    },
                    bounds_mm: Bounds3 {
                        min: Point3 {
                            x: edge.min_x,
                            y: edge.min_y,
                            z: edge.min_z,
                        },
                        max: Point3 {
                            x: edge.max_x,
                            y: edge.max_y,
                            z: edge.max_z,
                        },
                    },
                    closed: edge.closed,
                    circle_radius_mm: edge.has_circle.then_some(edge.circle_radius_mm),
                    axis_origin_mm,
                    axis_direction,
                    adjacent_face_ordinals,
                }
            })
            .collect(),
    };
    let edge_geometry_is_valid = topology.edges.iter().all(|edge| {
        let finite_point =
            |point: Point3| [point.x, point.y, point.z].into_iter().all(f64::is_finite);
        let circle_is_coherent = match edge.circle_radius_mm {
            None => edge.curve_kind != "circle",
            Some(radius) => edge.curve_kind == "circle" && radius.is_finite() && radius > 0.0,
        };
        let axis_is_coherent = match (edge.axis_origin_mm, edge.axis_direction) {
            (Some(origin), Some(direction)) => {
                matches!(edge.curve_kind.as_str(), "line" | "circle")
                    && finite_point(origin)
                    && finite_point(direction)
                    && (direction.x * direction.x
                        + direction.y * direction.y
                        + direction.z * direction.z
                        - 1.0)
                        .abs()
                        <= 1.0e-12
            }
            (None, None) => !matches!(edge.curve_kind.as_str(), "line" | "circle"),
            _ => false,
        };
        !edge.curve_kind.is_empty()
            && edge.length_mm.is_finite()
            && edge.length_mm > 0.0
            && finite_point(edge.centroid_mm)
            && finite_point(edge.bounds_mm.min)
            && finite_point(edge.bounds_mm.max)
            && edge.bounds_mm.min.x <= edge.bounds_mm.max.x
            && edge.bounds_mm.min.y <= edge.bounds_mm.max.y
            && edge.bounds_mm.min.z <= edge.bounds_mm.max.z
            && circle_is_coherent
            && axis_is_coherent
    });
    if topology.edges.len() != topology.edge_count as usize || !edge_geometry_is_valid {
        return Err(GeometryError {
            code: GeometryErrorCode::InvalidShape,
            diagnostic: "OCCT returned incomplete or invalid exact edge evidence".to_owned(),
            operation,
            input_digest,
            backend_fingerprint: BACKEND_FINGERPRINT,
        });
    }
    let result_signature = format!(
        "{}:{}:{}:{}:{}:{:016x}:{:?}",
        BACKEND_FINGERPRINT,
        operation,
        input_digest,
        topology.face_count,
        topology.solid_count,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm
    );
    let result_fingerprint = stable_digest(&result_signature);
    let mut history = native_ref
        .history_evidence()
        .into_iter()
        .map(|entry| HistoryEvidence {
            semantic_role: (!entry.semantic_role.is_empty()).then_some(entry.semantic_role),
            relation: entry.relation,
            source_element_id: entry.source_element_id,
            output_face_ordinal: entry.output_present.then_some(entry.output_ordinal),
            output_edge_ordinal: None,
        })
        .collect::<Vec<_>>();
    history.extend(
        native_ref
            .edge_history_evidence()
            .into_iter()
            .map(|entry| HistoryEvidence {
                semantic_role: (!entry.semantic_role.is_empty()).then_some(entry.semantic_role),
                relation: entry.relation,
                source_element_id: entry.source_element_id,
                output_face_ordinal: None,
                output_edge_ordinal: entry.output_present.then_some(entry.output_ordinal),
            }),
    );

    Ok(ExactOpOutput {
        body: ExactBody {
            native,
            result_fingerprint,
            topology,
        },
        topology_history: history,
        tolerance_report: ToleranceReport {
            profile: TOLERANCE_PROFILE,
            shape_valid: true,
            accepted_exact_solid: summary.solid_count > 0,
        },
        diagnostics: vec![GeometryDiagnostic {
            code: if summary.solid_count > 0 {
                "valid_exact_solid"
            } else {
                "valid_exact_planar_face"
            },
            message: diagnostic,
        }],
        input_digest,
        backend_fingerprint: BACKEND_FINGERPRINT,
        history_confidence,
    })
}

#[must_use]
pub fn has_complete_manifold_adjacency(topology: &TopologyEvidence) -> bool {
    if topology.faces.len() != topology.face_count as usize
        || topology.edges.len() != topology.edge_count as usize
        || topology
            .faces
            .iter()
            .enumerate()
            .any(|(ordinal, face)| face.ordinal != ordinal as u32)
        || topology
            .edges
            .iter()
            .enumerate()
            .any(|(ordinal, edge)| edge.ordinal != ordinal as u32)
    {
        return false;
    }

    topology.faces.iter().all(|face| {
        face.edge_count as usize == face.edge_ordinals.len()
            && !face.edge_ordinals.is_empty()
            && face.edge_ordinals.windows(2).all(|pair| pair[0] < pair[1])
            && face.edge_ordinals.iter().all(|edge_ordinal| {
                topology.edges.iter().any(|edge| {
                    edge.ordinal == *edge_ordinal
                        && edge.adjacent_face_ordinals.contains(&face.ordinal)
                })
            })
    }) && topology.edges.iter().all(|edge| {
        edge.adjacent_face_ordinals.len() == 2
            && edge.adjacent_face_ordinals[0] < edge.adjacent_face_ordinals[1]
            && edge.adjacent_face_ordinals.iter().all(|face_ordinal| {
                topology.faces.iter().any(|face| {
                    face.ordinal == *face_ordinal && face.edge_ordinals.contains(&edge.ordinal)
                })
            })
    })
}

pub fn validate_closed_planar_profile(points: &[Point3]) -> Result<(), GeometryError> {
    let input = format!("profile:{points:?}");
    if points.len() != 4
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite())
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "validate_profile",
            &input,
            "A0 supports exactly four finite planar profile vertices".to_owned(),
        ));
    }
    let z = points[0].z;
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left.x * right.y - right.x * left.y)
        .sum::<f64>();
    if points.iter().any(|point| (point.z - z).abs() > 1.0e-9)
        || twice_area.abs() <= 1.0e-12
        || segments_intersect(points[0], points[1], points[2], points[3])
        || segments_intersect(points[1], points[2], points[3], points[0])
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "validate_profile",
            &input,
            "Profile is non-planar, degenerate, or self-intersecting".to_owned(),
        ));
    }
    Ok(())
}

fn segments_intersect(a: Point3, b: Point3, c: Point3, d: Point3) -> bool {
    fn orientation(a: Point3, b: Point3, c: Point3) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    let first = orientation(a, b, c);
    let second = orientation(a, b, d);
    let third = orientation(c, d, a);
    let fourth = orientation(c, d, b);
    first * second < 0.0 && third * fourth < 0.0
}

fn native_status(status: u8) -> GeometryErrorCode {
    match status {
        1 => GeometryErrorCode::InvalidParameter,
        2 => GeometryErrorCode::NonFiniteParameter,
        3 => GeometryErrorCode::NoGeometricChange,
        4 => GeometryErrorCode::DegenerateOperation,
        5 => GeometryErrorCode::InvalidShape,
        6 => GeometryErrorCode::BackendException,
        _ => GeometryErrorCode::NullResult,
    }
}

fn validate_box(spec: BoxSpec, operation: &'static str, input: &str) -> Result<(), GeometryError> {
    for (value, name) in [
        (spec.origin_mm.x, "origin_x"),
        (spec.origin_mm.y, "origin_y"),
        (spec.origin_mm.z, "origin_z"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    for (value, name) in [
        (spec.size_mm.x, "size_x"),
        (spec.size_mm.y, "size_y"),
        (spec.size_mm.z, "size_z"),
    ] {
        validate_length(value, name, operation, input)?;
    }
    for (origin, size, name) in [
        (spec.origin_mm.x, spec.size_mm.x, "max_x"),
        (spec.origin_mm.y, spec.size_mm.y, "max_y"),
        (spec.origin_mm.z, spec.size_mm.z, "max_z"),
    ] {
        validate_coordinate(origin + size, name, operation, input)?;
    }
    Ok(())
}

fn validate_general_revolve_axis_angle(
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    for (value, name) in [
        (axis_start_mm[0], "axis_start_x"),
        (axis_start_mm[1], "axis_start_y"),
        (axis_end_mm[0], "axis_end_x"),
        (axis_end_mm[1], "axis_end_y"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    if axis_start_mm == axis_end_mm {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            "Revolve axis must have non-zero length".to_owned(),
        ));
    }
    if !angle_degrees.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            "Revolve angle must be finite".to_owned(),
        ));
    }
    if !(0.0 < angle_degrees && angle_degrees <= 360.0) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            "Revolve angle must be within (0, 360] degrees".to_owned(),
        ));
    }
    Ok(())
}

fn validate_general_revolve_profile(
    segments: &[PlanarProfileSegment],
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
    input: &str,
) -> Result<(), GeometryError> {
    let operation = "revolve_general_profile";
    let invalid = |diagnostic: String| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            diagnostic,
        )
    };
    if !(2..=MAX_PLANAR_LOOP_SEGMENTS).contains(&segments.len()) {
        return Err(invalid(
            "Revolve profile requires 2..=64 segments".to_owned(),
        ));
    }
    validate_general_revolve_axis_angle(
        axis_start_mm,
        axis_end_mm,
        angle_degrees,
        operation,
        input,
    )?;
    let endpoints = planar_segment_endpoints;
    for (index, segment) in segments.iter().enumerate() {
        let (start, end) = endpoints(segment);
        for (coordinate, name) in [
            (start[0], "start_x"),
            (start[1], "start_y"),
            (end[0], "end_x"),
            (end[1], "end_y"),
        ] {
            validate_coordinate(coordinate, name, operation, input)?;
        }
        if start == end {
            return Err(invalid(format!("Profile segment {index} is degenerate")));
        }
        if let PlanarProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } = segment
        {
            for (coordinate, name) in [
                (control_1_mm[0], "control_1_x"),
                (control_1_mm[1], "control_1_y"),
                (control_2_mm[0], "control_2_x"),
                (control_2_mm[1], "control_2_y"),
            ] {
                validate_coordinate(coordinate, name, operation, input)?;
            }
        }
        if let PlanarProfileSegment::CircularArc { center_mm, .. } = segment {
            validate_coordinate(center_mm[0], "center_x", operation, input)?;
            validate_coordinate(center_mm[1], "center_y", operation, input)?;
            let start_radius = (start[0] - center_mm[0]).hypot(start[1] - center_mm[1]);
            let end_radius = (end[0] - center_mm[0]).hypot(end[1] - center_mm[1]);
            if start_radius < MIN_LENGTH_MM
                || (start_radius - end_radius).abs()
                    > 1.0e-9 * start_radius.max(end_radius).max(1.0)
            {
                return Err(invalid(format!(
                    "Profile arc {index} has inconsistent radius"
                )));
            }
        }
        let (next_start, _) = endpoints(&segments[(index + 1) % segments.len()]);
        if end != next_start {
            return Err(invalid(format!("Profile is open after segment {index}")));
        }
    }
    Ok(())
}

fn validate_mixed_profile(
    segments: &[PlanarProfileSegment],
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    let invalid = |diagnostic: String| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            diagnostic,
        )
    };
    let line_only = segments
        .iter()
        .all(|segment| matches!(segment, PlanarProfileSegment::Line { .. }));
    if !(2..=MAX_PLANAR_LOOP_SEGMENTS).contains(&segments.len())
        || (line_only && segments.len() < 3)
    {
        return Err(invalid(
            "Segmented profile requires 2..=64 segments; line-only polygons require at least three lines".to_owned(),
        ));
    }
    let endpoints = planar_segment_endpoints;
    for (index, segment) in segments.iter().enumerate() {
        let (start, end) = endpoints(segment);
        for (coordinate, name) in [
            (start[0], "start_x"),
            (start[1], "start_y"),
            (end[0], "end_x"),
            (end[1], "end_y"),
        ] {
            validate_coordinate(coordinate, name, operation, input)?;
        }
        if start == end {
            return Err(invalid(format!("Profile segment {index} is degenerate")));
        }
        if let PlanarProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } = segment
        {
            for (coordinate, name) in [
                (control_1_mm[0], "control_1_x"),
                (control_1_mm[1], "control_1_y"),
                (control_2_mm[0], "control_2_x"),
                (control_2_mm[1], "control_2_y"),
            ] {
                validate_coordinate(coordinate, name, operation, input)?;
            }
        }
        if let PlanarProfileSegment::CircularArc { center_mm, .. } = segment {
            validate_coordinate(center_mm[0], "center_x", operation, input)?;
            validate_coordinate(center_mm[1], "center_y", operation, input)?;
            let start_radius = (start[0] - center_mm[0]).hypot(start[1] - center_mm[1]);
            let end_radius = (end[0] - center_mm[0]).hypot(end[1] - center_mm[1]);
            if start_radius < MIN_LENGTH_MM
                || (start_radius - end_radius).abs()
                    > 1.0e-9 * start_radius.max(end_radius).max(1.0)
            {
                return Err(invalid(format!(
                    "Profile arc {index} has inconsistent radius"
                )));
            }
        }
        let (next_start, _) = endpoints(&segments[(index + 1) % segments.len()]);
        if end != next_start {
            return Err(invalid(format!("Profile is open after segment {index}")));
        }
    }
    if line_only && !is_simple_linear_planar_profile(segments) {
        return Err(invalid(
            "Line-only profile must be a simple non-degenerate polygon".to_owned(),
        ));
    }
    Ok(())
}

fn is_simple_linear_planar_profile(segments: &[PlanarProfileSegment]) -> bool {
    let points = segments
        .iter()
        .filter_map(|segment| match segment {
            PlanarProfileSegment::Line { start_mm, .. } => Some(*start_mm),
            PlanarProfileSegment::CircularArc { .. } | PlanarProfileSegment::CubicBezier { .. } => {
                None
            }
        })
        .collect::<Vec<_>>();
    if points.len() != segments.len()
        || points.iter().enumerate().any(|(index, point)| {
            points[index + 1..].iter().any(|candidate| {
                (point[0] - candidate[0]).abs() <= 1.0e-9
                    && (point[1] - candidate[1]).abs() <= 1.0e-9
            })
        })
    {
        return false;
    }
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>();
    if !twice_area.is_finite() || twice_area.abs() <= 1.0e-9 {
        return false;
    }
    for left in 0..points.len() {
        let left_next = (left + 1) % points.len();
        for right in (left + 1)..points.len() {
            let right_next = (right + 1) % points.len();
            if left == right_next || left_next == right {
                continue;
            }
            if planar_segments_intersect(
                points[left],
                points[left_next],
                points[right],
                points[right_next],
            ) {
                return false;
            }
        }
    }
    true
}

fn planar_segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0])
    };
    let on_segment = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        point[0] >= start[0].min(end[0]) - 1.0e-9
            && point[0] <= start[0].max(end[0]) + 1.0e-9
            && point[1] >= start[1].min(end[1]) - 1.0e-9
            && point[1] <= start[1].max(end[1]) + 1.0e-9
    };
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ((ab_c > 1.0e-9 && ab_d < -1.0e-9) || (ab_c < -1.0e-9 && ab_d > 1.0e-9))
        && ((cd_a > 1.0e-9 && cd_b < -1.0e-9) || (cd_a < -1.0e-9 && cd_b > 1.0e-9))
    {
        return true;
    }
    (ab_c.abs() <= 1.0e-9 && on_segment(a, b, c))
        || (ab_d.abs() <= 1.0e-9 && on_segment(a, b, d))
        || (cd_a.abs() <= 1.0e-9 && on_segment(c, d, a))
        || (cd_b.abs() <= 1.0e-9 && on_segment(c, d, b))
}

fn validate_circle(
    center_mm: [f64; 2],
    radius_mm: f64,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    validate_coordinate(center_mm[0], "center_x", operation, input)?;
    validate_coordinate(center_mm[1], "center_y", operation, input)?;
    validate_length(radius_mm, "radius_mm", operation, input)?;
    for (value, name) in [
        (center_mm[0] - radius_mm, "min_x"),
        (center_mm[0] + radius_mm, "max_x"),
        (center_mm[1] - radius_mm, "min_y"),
        (center_mm[1] + radius_mm, "max_y"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    Ok(())
}

fn validate_length(
    value: f64,
    name: &str,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    if !value.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            format!("{name} must be finite"),
        ));
    }
    if !(MIN_LENGTH_MM..=MAX_LENGTH_MM).contains(&value) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            format!("{name} must be within {MIN_LENGTH_MM}..={MAX_LENGTH_MM} mm"),
        ));
    }
    Ok(())
}

fn validate_coordinate(
    value: f64,
    name: &str,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    if !value.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            format!("{name} must be finite"),
        ));
    }
    if value.abs() > MAX_COORDINATE_MM {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            format!("{name} exceeds the local coordinate envelope"),
        ));
    }
    Ok(())
}

fn parameter_error(
    code: GeometryErrorCode,
    operation: &'static str,
    input: &str,
    diagnostic: String,
) -> GeometryError {
    GeometryError {
        code,
        diagnostic,
        operation,
        input_digest: stable_digest(input),
        backend_fingerprint: BACKEND_FINGERPRINT,
    }
}

fn box_input(label: &str, spec: BoxSpec) -> String {
    format!(
        "{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
        label,
        spec.origin_mm.x.to_bits(),
        spec.origin_mm.y.to_bits(),
        spec.origin_mm.z.to_bits(),
        spec.size_mm.x.to_bits(),
        spec.size_mm.y.to_bits(),
        spec.size_mm.z.to_bits()
    )
}

fn subtract3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn dot3(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn squared_distance3(left: [f64; 3], right: [f64; 3]) -> f64 {
    let delta = subtract3(left, right);
    dot3(delta, delta)
}

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
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
                result.topology_history.iter().all(|entry| {
                    entry.semantic_role.as_deref() == Some("surface_thicken.face")
                })
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
        let frame = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
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
        assert!(
            (thickened.body.topology.volume_mm3 - 880.0 * std::f64::consts::PI).abs() <= 1.0e-5
        );
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
                    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 20.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                    1.0,
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
            Err(StepXdeManifestError::Malformed)
        );
        let scaled = valid.replacen("3ff0000000000000", "4000000000000000", 1);
        assert_eq!(
            parse_step_xde_manifest(&scaled),
            Err(StepXdeManifestError::Malformed)
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
        let error = validate_spatial_sweep_path(
            &adjacent_self_intersection,
            "sweep_spatial_profile",
            "unit",
        )
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
}
