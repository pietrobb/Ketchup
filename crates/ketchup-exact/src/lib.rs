//! Narrow, exception-safe exact geometry boundary used by the A0 gate.

use ketchup_tolerance::{
    ACCUMULATED_ROUNDING, APPROXIMATION, DEFAULT_LINEAR_TOLERANCE_MM, MAX_COORDINATE_MM,
    NEGLIGIBLE, ROUNDING,
};
use std::fmt;

pub mod naming;

mod booleans;
mod evidence;
mod exchange;
mod mesh;
mod output;
mod profiles;
mod segments;
mod solids;
mod spec;
mod sweep_loft;
mod validation;
pub use evidence::*;
pub use exchange::*;
use output::*;
use segments::*;
pub use spec::*;
pub use validation::*;

const BACKEND_FINGERPRINT: &str = env!("KETCHUP_OCCT_BUILD_FINGERPRINT");
// not a tolerance: the evidence identity naming the kernel checks.
const TOLERANCE_PROFILE: &str = "r0-v1:bbox=1e-6mm:volume_abs=1e-6mm3:volume_rel=1e-10";
const MIN_LENGTH_MM: f64 = 0.01;
const MAX_LENGTH_MM: f64 = 100_000.0;
const PLANAR_SEGMENT_STRIDE: usize = 10;
const SPATIAL_SEGMENT_STRIDE: usize = 14;
const MIN_SWEEP_PATH_SEGMENT_LENGTH_MM: f64 = DEFAULT_LINEAR_TOLERANCE_MM;
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

const fn native_tolerances() -> ffi::NativeTolerances {
    ffi::NativeTolerances {
        linear_mm: DEFAULT_LINEAR_TOLERANCE_MM,
        rounding: ROUNDING,
        accumulated_rounding: ACCUMULATED_ROUNDING,
        approximation: APPROXIMATION,
        negligible: NEGLIGIBLE,
    }
}

#[allow(dead_code, unsafe_code)]
#[cxx::bridge(namespace = "ketchup::exact")]
mod ffi {
    /// The tolerances and numeric guards of `ketchup-tolerance`, the only values native
    /// code compares geometry against.
    struct NativeTolerances {
        linear_mm: f64,
        rounding: f64,
        accumulated_rounding: f64,
        approximation: f64,
        negligible: f64,
    }

    extern "Rust" {
        fn native_tolerances() -> NativeTolerances;
    }

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
        fn named_prism_native(
            segments: &[f64],
            base_z: f64,
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn named_revol_native(
            segments: &[f64],
            axis_start_x: f64,
            axis_start_y: f64,
            axis_end_x: f64,
            axis_end_y: f64,
            angle_degrees: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn named_finish_native(
            body: &NativeOperationResult,
            labels: &[String],
            edge_ordinals: &[u32],
            amount: f64,
            fillet: bool,
        ) -> UniquePtr<NativeOperationResult>;
        fn named_boolean_native(
            target: &NativeOperationResult,
            target_labels: &[String],
            tool: &NativeOperationResult,
            tool_labels: &[String],
            operation: u8,
        ) -> UniquePtr<NativeOperationResult>;
        fn named_offset_face_native(
            body: &NativeOperationResult,
            labels: &[String],
            face_ordinal: u32,
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn named_shell_native(
            body: &NativeOperationResult,
            labels: &[String],
            open_ordinals: &[u32],
            thickness: f64,
            prefix: &str,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct ExactBackend;

/// Public exact-kernel name used by canonical geometry integrations.
pub type ExactKernel = ExactBackend;

impl ExactBackend {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests;
