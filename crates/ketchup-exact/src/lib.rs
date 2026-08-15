//! Narrow, exception-safe exact geometry boundary used by the A0 gate.

use std::fmt;

const BACKEND_FINGERPRINT: &str = env!("KETCHUP_OCCT_BUILD_FINGERPRINT");
const TOLERANCE_PROFILE: &str = "r0-v1:bbox=1e-6mm:volume_abs=1e-6mm3:volume_rel=1e-10";
const MIN_LENGTH_MM: f64 = 0.01;
const MAX_LENGTH_MM: f64 = 100_000.0;
const MAX_COORDINATE_MM: f64 = 1_000_000.0;

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
    struct NativeTopologySummary {
        vertex_count: u32,
        edge_count: u32,
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

    unsafe extern "C++" {
        include!("ketchup_exact.hxx");

        type NativeOperationResult;

        fn make_box_native(
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn extrude_rectangle_native(
            width: f64,
            depth: f64,
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn offset_rectangle_native(
            min_x: f64,
            min_y: f64,
            max_x: f64,
            max_y: f64,
            distance: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn sweep_rectangle_native(values: &[f64]) -> UniquePtr<NativeOperationResult>;
        fn loft_spline_native(values: &[f64]) -> UniquePtr<NativeOperationResult>;
        fn extrude_circle_native(
            center_x: f64,
            center_y: f64,
            radius: f64,
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn extrude_mixed_profile_native(
            segments: &[f64],
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn revolve_profile_native(points: &[f64]) -> UniquePtr<NativeOperationResult>;
        fn revolve_general_profile_native(
            segments: &[f64],
            axis_start_x: f64,
            axis_start_y: f64,
            axis_end_x: f64,
            axis_end_y: f64,
            angle_degrees: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn shell_box_native(
            width: f64,
            depth: f64,
            height: f64,
            thickness: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn finish_shell_box_native(
            width: f64,
            depth: f64,
            height: f64,
            thickness: f64,
            amount: f64,
            fillet: bool,
        ) -> UniquePtr<NativeOperationResult>;
        fn shell_revolve_profile_native(
            points: &[f64],
            thickness: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn finish_shell_revolve_profile_native(
            points: &[f64],
            thickness: f64,
            amount: f64,
            fillet: bool,
        ) -> UniquePtr<NativeOperationResult>;
        fn cut_box_native(
            base: &NativeOperationResult,
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn cut_cylinder_native(
            base: &NativeOperationResult,
            center_x: f64,
            center_y: f64,
            origin_z: f64,
            radius: f64,
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn fuse_box_native(
            base: &NativeOperationResult,
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn common_box_native(
            base: &NativeOperationResult,
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn split_box_native(
            base: &NativeOperationResult,
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn exception_probe_native() -> UniquePtr<NativeOperationResult>;
        fn import_step_native(path: &str) -> UniquePtr<NativeOperationResult>;
        fn step_length_unit_native(path: &str) -> String;
        fn transform_body_native(
            body: &NativeOperationResult,
            matrix: &[f64],
        ) -> UniquePtr<NativeOperationResult>;
        fn combine_bodies_native(
            base: &NativeOperationResult,
            added: &NativeOperationResult,
        ) -> UniquePtr<NativeOperationResult>;
        fn export_step_native(body: &NativeOperationResult, path: &str) -> String;

        fn status_code(self: &NativeOperationResult) -> u8;
        fn diagnostic(self: &NativeOperationResult) -> String;
        fn valid(self: &NativeOperationResult) -> bool;
        fn topology_summary(self: &NativeOperationResult) -> NativeTopologySummary;
        fn face_evidence(self: &NativeOperationResult) -> Vec<NativeFaceEvidence>;
        fn face_edge_evidence(self: &NativeOperationResult) -> Vec<NativeFaceEdgeEvidence>;
        fn edge_face_evidence(self: &NativeOperationResult) -> Vec<NativeEdgeFaceEvidence>;
        fn history_evidence(self: &NativeOperationResult) -> Vec<NativeHistoryEvidence>;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleExtrudeSpec {
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
    pub height_mm: f64,
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CylinderToolSpec {
    pub center_mm: [f64; 2],
    pub origin_z_mm: f64,
    pub radius_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BottleEdgeFinish {
    Fillet,
    Chamfer,
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
    pub bounds_mm: Bounds3,
    pub edge_count: u32,
    pub edge_ordinals: Vec<u32>,
    pub geometric_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeEvidence {
    pub ordinal: u32,
    pub adjacent_face_ordinals: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyEvidence {
    pub vertex_count: u32,
    pub edge_count: u32,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct ExactBackend;

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
        collect_output(native, "box", &input, HistoryConfidence::None)
    }

    pub fn extrude_rectangle(
        &self,
        spec: RectangleExtrudeSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_rectangle:{:016x}:{:016x}:{:016x}",
            spec.width_mm.to_bits(),
            spec.depth_mm.to_bits(),
            spec.height_mm.to_bits()
        );
        validate_length(spec.width_mm, "width_mm", "extrude_rectangle", &input)?;
        validate_length(spec.depth_mm, "depth_mm", "extrude_rectangle", &input)?;
        validate_length(spec.height_mm, "height_mm", "extrude_rectangle", &input)?;
        let native = ffi::extrude_rectangle_native(spec.width_mm, spec.depth_mm, spec.height_mm);
        collect_output(
            native,
            "extrude_rectangle",
            &input,
            HistoryConfidence::Complete,
        )
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

    pub fn sweep_rectangle(
        &self,
        spec: RectangleSweepSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "sweep_rectangle:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
            spec.profile_min_mm[0].to_bits(),
            spec.profile_min_mm[1].to_bits(),
            spec.profile_max_mm[0].to_bits(),
            spec.profile_max_mm[1].to_bits(),
            spec.path_start_mm[0].to_bits(),
            spec.path_start_mm[1].to_bits(),
            spec.path_end_mm[0].to_bits(),
            spec.path_end_mm[1].to_bits(),
        );
        for (name, coordinate) in [
            ("profile_min_u", spec.profile_min_mm[0]),
            ("profile_min_v", spec.profile_min_mm[1]),
            ("profile_max_u", spec.profile_max_mm[0]),
            ("profile_max_v", spec.profile_max_mm[1]),
            ("path_start_x", spec.path_start_mm[0]),
            ("path_start_y", spec.path_start_mm[1]),
            ("path_end_x", spec.path_end_mm[0]),
            ("path_end_y", spec.path_end_mm[1]),
        ] {
            validate_coordinate(coordinate, name, "sweep_rectangle", &input)?;
        }
        validate_length(
            spec.profile_max_mm[0] - spec.profile_min_mm[0],
            "profile_width",
            "sweep_rectangle",
            &input,
        )?;
        validate_length(
            spec.profile_max_mm[1] - spec.profile_min_mm[1],
            "profile_height",
            "sweep_rectangle",
            &input,
        )?;
        let path_x = spec.path_end_mm[0] - spec.path_start_mm[0];
        let path_y = spec.path_end_mm[1] - spec.path_start_mm[1];
        let path_length = path_x.hypot(path_y);
        validate_length(path_length, "path_length", "sweep_rectangle", &input)?;
        let section = [path_y / path_length, -path_x / path_length];
        for (index, (u, v, at_end)) in [
            (spec.profile_min_mm[0], spec.profile_min_mm[1], false),
            (spec.profile_max_mm[0], spec.profile_min_mm[1], false),
            (spec.profile_max_mm[0], spec.profile_max_mm[1], false),
            (spec.profile_min_mm[0], spec.profile_max_mm[1], false),
            (spec.profile_min_mm[0], spec.profile_min_mm[1], true),
            (spec.profile_max_mm[0], spec.profile_min_mm[1], true),
            (spec.profile_max_mm[0], spec.profile_max_mm[1], true),
            (spec.profile_min_mm[0], spec.profile_max_mm[1], true),
        ]
        .into_iter()
        .enumerate()
        {
            let along = if at_end { [path_x, path_y] } else { [0.0, 0.0] };
            for (axis, coordinate) in [
                spec.path_start_mm[0] + section[0] * u + along[0],
                spec.path_start_mm[1] + section[1] * u + along[1],
                v,
            ]
            .into_iter()
            .enumerate()
            {
                validate_coordinate(
                    coordinate,
                    &format!("output_corner_{index}_axis_{axis}"),
                    "sweep_rectangle",
                    &input,
                )?;
            }
        }
        let native = ffi::sweep_rectangle_native(&[
            spec.profile_min_mm[0],
            spec.profile_min_mm[1],
            spec.profile_max_mm[0],
            spec.profile_max_mm[1],
            spec.path_start_mm[0],
            spec.path_start_mm[1],
            spec.path_end_mm[0],
            spec.path_end_mm[1],
        ]);
        collect_output(
            native,
            "sweep_rectangle",
            &input,
            HistoryConfidence::Complete,
        )
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
            if !(4..=64).contains(&section.control_points_mm.len())
                || !section.elevation_mm.is_finite()
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

    pub fn extrude_mixed_profile(
        &self,
        segments: &[PlanarProfileSegment],
        height_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_mixed_profile:{segments:?}:{:016x}",
            height_mm.to_bits()
        );
        validate_mixed_profile(segments, &input)?;
        validate_length(height_mm, "height_mm", "extrude_mixed_profile", &input)?;
        let flattened = segments
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
                    f64::from(*clockwise),
                ],
            })
            .collect::<Vec<_>>();
        collect_output(
            ffi::extrude_mixed_profile_native(&flattened, height_mm),
            "extrude_mixed_profile",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn revolve_profile(&self, points_mm: &[[f64; 2]]) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("revolve_profile:{points_mm:?}");
        validate_bottle_revolve_profile(points_mm, &input)?;
        let flattened = points_mm
            .iter()
            .flat_map(|point| point.iter().copied())
            .collect::<Vec<_>>();
        let native = ffi::revolve_profile_native(&flattened);
        collect_output(
            native,
            "revolve_profile",
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
        let flattened = segments
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
                    f64::from(*clockwise),
                ],
            })
            .collect::<Vec<_>>();
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

    pub fn shell_box(
        &self,
        spec: RectangleExtrudeSpec,
        thickness_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "shell_box:{:016x}:{:016x}:{:016x}:{:016x}",
            spec.width_mm.to_bits(),
            spec.depth_mm.to_bits(),
            spec.height_mm.to_bits(),
            thickness_mm.to_bits(),
        );
        validate_length(spec.width_mm, "width_mm", "shell_box", &input)?;
        validate_length(spec.depth_mm, "depth_mm", "shell_box", &input)?;
        validate_length(spec.height_mm, "height_mm", "shell_box", &input)?;
        validate_length(thickness_mm, "thickness_mm", "shell_box", &input)?;
        collect_output(
            ffi::shell_box_native(spec.width_mm, spec.depth_mm, spec.height_mm, thickness_mm),
            "shell_box",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn finish_shell_box(
        &self,
        spec: RectangleExtrudeSpec,
        thickness_mm: f64,
        finish: BottleEdgeFinish,
        amount_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "finish_shell_box:{:016x}:{:016x}:{:016x}:{:016x}:{}:{:016x}",
            spec.width_mm.to_bits(),
            spec.depth_mm.to_bits(),
            spec.height_mm.to_bits(),
            thickness_mm.to_bits(),
            match finish {
                BottleEdgeFinish::Fillet => "fillet",
                BottleEdgeFinish::Chamfer => "chamfer",
            },
            amount_mm.to_bits(),
        );
        validate_length(spec.width_mm, "width_mm", "finish_shell_box", &input)?;
        validate_length(spec.depth_mm, "depth_mm", "finish_shell_box", &input)?;
        validate_length(spec.height_mm, "height_mm", "finish_shell_box", &input)?;
        validate_length(thickness_mm, "thickness_mm", "finish_shell_box", &input)?;
        validate_length(amount_mm, "amount_mm", "finish_shell_box", &input)?;
        collect_output(
            ffi::finish_shell_box_native(
                spec.width_mm,
                spec.depth_mm,
                spec.height_mm,
                thickness_mm,
                amount_mm,
                finish == BottleEdgeFinish::Fillet,
            ),
            "finish_shell_box",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn shell_revolve_profile(
        &self,
        points_mm: &[[f64; 2]],
        thickness_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "shell_revolve_profile:{points_mm:?}:{:016x}",
            thickness_mm.to_bits()
        );
        validate_bottle_revolve_profile(points_mm, &input)?;
        validate_bottle_shell_thickness(points_mm, thickness_mm, &input)?;
        let flattened = points_mm
            .iter()
            .flat_map(|point| point.iter().copied())
            .collect::<Vec<_>>();
        let native = ffi::shell_revolve_profile_native(&flattened, thickness_mm);
        collect_output(
            native,
            "shell_revolve_profile",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn finish_shell_revolve_profile(
        &self,
        points_mm: &[[f64; 2]],
        thickness_mm: f64,
        finish: BottleEdgeFinish,
        amount_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "finish_shell_revolve_profile:{points_mm:?}:{:016x}:{}:{:016x}",
            thickness_mm.to_bits(),
            match finish {
                BottleEdgeFinish::Fillet => "fillet",
                BottleEdgeFinish::Chamfer => "chamfer",
            },
            amount_mm.to_bits()
        );
        validate_bottle_revolve_profile(points_mm, &input)?;
        validate_bottle_shell_thickness(points_mm, thickness_mm, &input)?;
        validate_bottle_finish_amount(points_mm, amount_mm, &input)?;
        let flattened = points_mm
            .iter()
            .flat_map(|point| point.iter().copied())
            .collect::<Vec<_>>();
        let native = ffi::finish_shell_revolve_profile_native(
            &flattened,
            thickness_mm,
            amount_mm,
            finish == BottleEdgeFinish::Fillet,
        );
        collect_output(
            native,
            "finish_shell_revolve_profile",
            &input,
            HistoryConfidence::Complete,
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

    #[must_use]
    pub fn step_length_unit_name(&self, path: &str) -> Option<String> {
        let unit = ffi::step_length_unit_native(path);
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

    pub fn cut_box(
        &self,
        base: &ExactBody,
        tool: BoxSpec,
        mode: CutMode,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "cut_box:{}:{}:{:?}",
            base.result_fingerprint,
            box_input("tool", tool),
            mode
        );
        validate_box(tool, "cut_box", &input)?;
        classify_box_intersection(base.topology.bounds_mm, tool, "cut_box", "Cut", &input)?;
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "cut_box",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native = ffi::cut_box_native(
            native_base,
            tool.origin_mm.x,
            tool.origin_mm.y,
            tool.origin_mm.z,
            tool.size_mm.x,
            tool.size_mm.y,
            tool.size_mm.z,
        );
        collect_output(native, "cut_box", &input, HistoryConfidence::Partial)
    }

    pub fn cut_cylinder(
        &self,
        base: &ExactBody,
        tool: CylinderToolSpec,
        mode: CutMode,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "cut_cylinder:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{mode:?}",
            base.result_fingerprint,
            tool.center_mm[0].to_bits(),
            tool.center_mm[1].to_bits(),
            tool.origin_z_mm.to_bits(),
            tool.radius_mm.to_bits(),
            tool.height_mm.to_bits()
        );
        validate_circle(tool.center_mm, tool.radius_mm, "cut_cylinder", &input)?;
        validate_coordinate(tool.origin_z_mm, "origin_z_mm", "cut_cylinder", &input)?;
        validate_length(tool.height_mm, "height_mm", "cut_cylinder", &input)?;
        validate_coordinate(
            tool.origin_z_mm + tool.height_mm,
            "max_z",
            "cut_cylinder",
            &input,
        )?;
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "cut_cylinder",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native = ffi::cut_cylinder_native(
            native_base,
            tool.center_mm[0],
            tool.center_mm[1],
            tool.origin_z_mm,
            tool.radius_mm,
            tool.height_mm,
        );
        collect_output(native, "cut_cylinder", &input, HistoryConfidence::Partial)
    }

    pub fn common_box(
        &self,
        base: &ExactBody,
        tool: BoxSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "common_box:{}:{}",
            base.result_fingerprint,
            box_input("tool", tool)
        );
        validate_box(tool, "common_box", &input)?;
        classify_box_intersection(
            base.topology.bounds_mm,
            tool,
            "common_box",
            "Common",
            &input,
        )?;
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "common_box",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native = ffi::common_box_native(
            native_base,
            tool.origin_mm.x,
            tool.origin_mm.y,
            tool.origin_mm.z,
            tool.size_mm.x,
            tool.size_mm.y,
            tool.size_mm.z,
        );
        collect_output(native, "common_box", &input, HistoryConfidence::Partial)
    }

    pub fn split_box(
        &self,
        base: &ExactBody,
        tool: BoxSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "split_box:{}:{}",
            base.result_fingerprint,
            box_input("tool", tool)
        );
        validate_box(tool, "split_box", &input)?;
        classify_box_intersection(base.topology.bounds_mm, tool, "split_box", "Split", &input)?;
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "split_box",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native = ffi::split_box_native(
            native_base,
            tool.origin_mm.x,
            tool.origin_mm.y,
            tool.origin_mm.z,
            tool.size_mm.x,
            tool.size_mm.y,
            tool.size_mm.z,
        );
        collect_output(native, "split_box", &input, HistoryConfidence::Partial)
    }

    pub fn fuse_box(
        &self,
        base: &ExactBody,
        tool: BoxSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "fuse_box:{}:{}",
            base.result_fingerprint,
            box_input("tool", tool)
        );
        validate_box(tool, "fuse_box", &input)?;
        classify_box_intersection(base.topology.bounds_mm, tool, "fuse_box", "Fuse", &input)?;
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "fuse_box",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native = ffi::fuse_box_native(
            native_base,
            tool.origin_mm.x,
            tool.origin_mm.y,
            tool.origin_mm.z,
            tool.size_mm.x,
            tool.size_mm.y,
            tool.size_mm.z,
        );
        collect_output(native, "fuse_box", &input, HistoryConfidence::Partial)
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
            let signature = format!(
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
            );
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
        edges: (0..summary.edge_count)
            .map(|ordinal| {
                let mut adjacent_face_ordinals = edge_faces
                    .iter()
                    .filter(|entry| entry.edge_ordinal == ordinal)
                    .map(|entry| entry.face_ordinal)
                    .collect::<Vec<_>>();
                adjacent_face_ordinals.sort_unstable();
                EdgeEvidence {
                    ordinal,
                    adjacent_face_ordinals,
                }
            })
            .collect(),
    };
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
    let history = native_ref
        .history_evidence()
        .into_iter()
        .map(|entry| HistoryEvidence {
            semantic_role: (!entry.semantic_role.is_empty()).then_some(entry.semantic_role),
            relation: entry.relation,
            source_element_id: entry.source_element_id,
            output_face_ordinal: entry.output_present.then_some(entry.output_ordinal),
        })
        .collect::<Vec<_>>();

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

fn has_complete_partition_adjacency(topology: &TopologyEvidence) -> bool {
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
        let mut unique_faces = edge.adjacent_face_ordinals.clone();
        unique_faces.dedup();
        edge.adjacent_face_ordinals
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
            && (2..=3).contains(&unique_faces.len())
            && unique_faces.iter().all(|face_ordinal| {
                topology.faces.iter().any(|face| {
                    face.ordinal == *face_ordinal && face.edge_ordinals.contains(&edge.ordinal)
                })
            })
    })
}

pub fn capture_guaranteed_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_guaranteed_references",
            &output.input_digest,
            "Guaranteed output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("extrusion.top", "profile.face"),
        ("extrusion.bottom", "profile.face"),
        ("extrusion.side(profile_edge=east)", "profile.edge.east"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_guaranteed_references",
                &output.input_digest,
                format!(
                    "Guaranteed role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        }
        let ordinal = candidates[0];
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == ordinal && face.surface_kind == "plane")
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_guaranteed_references",
                    &output.input_digest,
                    format!("Guaranteed role {semantic_role} is not a planar output face"),
                )
            })?;
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

fn has_closed_revolve_adjacency(topology: &TopologyEvidence) -> bool {
    topology.solid_count == 1
        && topology.shell_count == 1
        && topology.faces.len() == topology.face_count as usize
        && topology.edges.len() == topology.edge_count as usize
        && topology.faces.iter().all(|face| {
            face.edge_count as usize == face.edge_ordinals.len()
                && !face.edge_ordinals.is_empty()
                && face.edge_ordinals.iter().all(|edge_ordinal| {
                    topology.edges.iter().any(|edge| {
                        edge.ordinal == *edge_ordinal
                            && edge.adjacent_face_ordinals.contains(&face.ordinal)
                    })
                })
        })
        && topology.edges.iter().all(|edge| {
            (1..=2).contains(&edge.adjacent_face_ordinals.len())
                && edge.adjacent_face_ordinals.iter().all(|face_ordinal| {
                    topology.faces.iter().any(|face| {
                        face.ordinal == *face_ordinal && face.edge_ordinals.contains(&edge.ordinal)
                    })
                })
        })
}

pub fn capture_circle_extrusion_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_circle_extrusion_references",
            &output.input_digest,
            "Circle extrusion lacks complete face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("extrusion.top", "profile.face", "plane", "planar_face"),
        ("extrusion.bottom", "profile.face", "plane", "planar_face"),
        (
            "extrusion.side(profile_edge=circle)",
            "profile.edge.circle",
            "cylinder",
            "cylindrical_face",
        ),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id, surface_kind, expected_type) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        let [ordinal] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_circle_extrusion_references",
                &output.input_digest,
                format!("Circle extrusion role {semantic_role} is ambiguous"),
            ));
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == *ordinal && face.surface_kind == surface_kind)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_circle_extrusion_references",
                    &output.input_digest,
                    format!("Circle extrusion role {semantic_role} has the wrong surface type"),
                )
            })?;
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_mixed_profile_extrusion_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_mixed_profile_extrusion_references",
            &output.input_digest,
            "Mixed profile extrusion lacks complete face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("extrusion.top", "profile.face", "plane", "planar_face"),
        ("extrusion.bottom", "profile.face", "plane", "planar_face"),
        (
            "extrusion.side(profile_edge=arc.0)",
            "profile.edge.arc.0",
            "other",
            "face",
        ),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id, surface_kind, expected_type) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        let [ordinal] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_mixed_profile_extrusion_references",
                &output.input_digest,
                format!("Mixed profile role {semantic_role} is ambiguous"),
            ));
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == *ordinal && face.surface_kind == surface_kind)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_mixed_profile_extrusion_references",
                    &output.input_digest,
                    format!(
                        "Mixed profile role {semantic_role} has the wrong surface type; available={:?}",
                        output
                            .body
                            .topology
                            .faces
                            .iter()
                            .map(|face| face.surface_kind.as_str())
                            .collect::<Vec<_>>()
                    ),
                )
            })?;
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&format!(
                "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
            )),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_circular_through_cut_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
    base: RectangleExtrudeSpec,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_circular_through_cut_references",
            &output.input_digest,
            "Circular cut lacks complete face/edge adjacency".to_owned(),
        ));
    }
    let tolerance = 1.0e-6;
    let roles = [
        ("extrusion.top", "profile.face", "planar_face"),
        ("extrusion.bottom", "profile.face", "planar_face"),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            "planar_face",
        ),
        (
            "through_cut.wall.circle",
            "cut_profile.edge.circle",
            "cylindrical_face",
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, expected_type) in roles {
        let candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| match semantic_role {
                "extrusion.top" => {
                    face.surface_kind == "plane"
                        && (face.bounds_mm.min.z - base.height_mm).abs() <= tolerance
                        && (face.bounds_mm.max.z - base.height_mm).abs() <= tolerance
                }
                "extrusion.bottom" => {
                    face.surface_kind == "plane"
                        && face.bounds_mm.min.z.abs() <= tolerance
                        && face.bounds_mm.max.z.abs() <= tolerance
                }
                "extrusion.side(profile_edge=east)" => {
                    face.surface_kind == "plane"
                        && (face.bounds_mm.min.x - base.width_mm).abs() <= tolerance
                        && (face.bounds_mm.max.x - base.width_mm).abs() <= tolerance
                }
                "through_cut.wall.circle" => face.surface_kind == "cylinder",
                _ => false,
            })
            .collect::<Vec<_>>();
        let [face] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_circular_through_cut_references",
                &output.input_digest,
                format!(
                    "Circular cut role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        output.topology_history.push(HistoryEvidence {
            semantic_role: Some(semantic_role.to_owned()),
            relation: "geometric_identity".to_owned(),
            source_element_id: source_element_id.to_owned(),
            output_face_ordinal: Some(face.ordinal),
        });
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_revolve_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_revolve_references",
            &output.input_digest,
            "Revolve output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("revolve.bottom", "profile.edge.0"),
        ("revolve.body", "profile.edge.1"),
        ("revolve.shoulder", "profile.edge.2"),
        ("revolve.neck", "profile.edge.3"),
        ("revolve.mouth", "profile.edge.4"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        let [ordinal] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_revolve_references",
                &output.input_digest,
                format!(
                    "Revolve role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == *ordinal)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_revolve_references",
                    &output.input_digest,
                    format!("Revolve role {semantic_role} has no output face"),
                )
            })?;
        let expected_type = "face";
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_general_revolve_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
    partial: bool,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_general_revolve_references",
            &output.input_digest,
            "General revolve output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let mut required = vec![
        ("revolve.side.0", "profile.edge.0"),
        ("revolve.side.1", "profile.edge.1"),
    ];
    if partial {
        required.extend([
            ("revolve.start", "profile.face"),
            ("revolve.end", "profile.face"),
        ]);
    }
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        let [ordinal] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_general_revolve_references",
                &output.input_digest,
                format!(
                    "General revolve role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == *ordinal)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_general_revolve_references",
                    &output.input_digest,
                    format!("General revolve role {semantic_role} has no output face"),
                )
            })?;
        let expected_type = "face";
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_box_shell_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_box_shell_references",
            &output.input_digest,
            "Box shell output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("shell.box.outer.bottom", "extrusion.bottom"),
        ("shell.box.outer.east", "extrusion.side(profile_edge=east)"),
        ("shell.box.rim", "extrusion.top"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        let [ordinal] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_box_shell_references",
                &output.input_digest,
                format!(
                    "Box shell role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == *ordinal)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_box_shell_references",
                    &output.input_digest,
                    format!("Box shell role {semantic_role} has no output face"),
                )
            })?;
        let expected_type = "planar_face";
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_shell_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_closed_revolve_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_shell_references",
            &output.input_digest,
            "Shell output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("shell.outer.bottom", "revolve.face.bottom"),
        ("shell.outer.body", "revolve.face.body"),
        ("shell.outer.shoulder", "revolve.face.shoulder"),
        ("shell.outer.neck", "revolve.face.neck"),
        ("shell.rim", "revolve.face.mouth"),
        ("shell.inner.bottom", "shell.offset.bottom"),
        ("shell.inner.body", "shell.offset.body"),
        ("shell.inner.shoulder", "shell.offset.shoulder"),
        ("shell.inner.neck", "shell.offset.neck"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        let [ordinal] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_shell_references",
                &output.input_digest,
                format!(
                    "Shell role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == *ordinal)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_shell_references",
                    &output.input_digest,
                    format!("Shell role {semantic_role} has no output face"),
                )
            })?;
        let expected_type = "face";
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_bounded_through_cut_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
    base: RectangleExtrudeSpec,
    cut: BoxSpec,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_bounded_through_cut_references",
            &output.input_digest,
            "Through-cut output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let cut_max_x = cut.origin_mm.x + cut.size_mm.x;
    let cut_max_y = cut.origin_mm.y + cut.size_mm.y;
    let roles = [
        ("extrusion.top", "profile.face", 2_usize, base.height_mm),
        ("extrusion.bottom", "profile.face", 2_usize, 0.0),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            0_usize,
            base.width_mm,
        ),
        (
            "through_cut.wall.west",
            "cut_profile.edge.west",
            0_usize,
            cut.origin_mm.x,
        ),
        (
            "through_cut.wall.east",
            "cut_profile.edge.east",
            0_usize,
            cut_max_x,
        ),
        (
            "through_cut.wall.south",
            "cut_profile.edge.south",
            1_usize,
            cut.origin_mm.y,
        ),
        (
            "through_cut.wall.north",
            "cut_profile.edge.north",
            1_usize,
            cut_max_y,
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, axis, coordinate) in roles {
        let candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| face.surface_kind == "plane")
            .filter(|face| face_matches_cut_role(face, semantic_role, axis, coordinate, base, cut))
            .collect::<Vec<_>>();
        let [face] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_bounded_through_cut_references",
                &output.input_digest,
                format!(
                    "Through-cut role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face_ordinal = face.ordinal;
        let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
        if !output.topology_history.iter().any(|entry| {
            entry.semantic_role.as_deref() == Some(semantic_role)
                && entry.source_element_id == source_element_id
                && entry.output_face_ordinal == Some(face_ordinal)
        }) {
            output.topology_history.push(HistoryEvidence {
                semantic_role: Some(semantic_role.to_owned()),
                relation: "bounded_cut_classification".to_owned(),
                source_element_id: source_element_id.to_owned(),
                output_face_ordinal: Some(face_ordinal),
            });
        }
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint,
        });
    }
    Ok(references)
}

pub fn capture_bounded_pocket_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
    base: RectangleExtrudeSpec,
    pocket: BoxSpec,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_bounded_pocket_references",
            &output.input_digest,
            "Pocket output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let max_x = pocket.origin_mm.x + pocket.size_mm.x;
    let max_y = pocket.origin_mm.y + pocket.size_mm.y;
    let roles = [
        ("extrusion.top", "profile.face", 2_usize, base.height_mm),
        ("extrusion.bottom", "profile.face", 2_usize, 0.0),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            0_usize,
            base.width_mm,
        ),
        (
            "pocket.floor",
            "pocket_profile.face",
            2_usize,
            pocket.origin_mm.z,
        ),
        (
            "pocket.wall.west",
            "pocket_profile.edge.west",
            0_usize,
            pocket.origin_mm.x,
        ),
        (
            "pocket.wall.east",
            "pocket_profile.edge.east",
            0_usize,
            max_x,
        ),
        (
            "pocket.wall.south",
            "pocket_profile.edge.south",
            1_usize,
            pocket.origin_mm.y,
        ),
        (
            "pocket.wall.north",
            "pocket_profile.edge.north",
            1_usize,
            max_y,
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, axis, coordinate) in roles {
        let candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| face.surface_kind == "plane")
            .filter(|face| {
                face_matches_pocket_role(face, semantic_role, axis, coordinate, base, pocket)
            })
            .collect::<Vec<_>>();
        let [face] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_bounded_pocket_references",
                &output.input_digest,
                format!(
                    "Pocket role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face_ordinal = face.ordinal;
        let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
        if !output.topology_history.iter().any(|entry| {
            entry.semantic_role.as_deref() == Some(semantic_role)
                && entry.source_element_id == source_element_id
                && entry.output_face_ordinal == Some(face_ordinal)
        }) {
            output.topology_history.push(HistoryEvidence {
                semantic_role: Some(semantic_role.to_owned()),
                relation: "bounded_pocket_classification".to_owned(),
                source_element_id: source_element_id.to_owned(),
                output_face_ordinal: Some(face_ordinal),
            });
        }
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint,
        });
    }
    Ok(references)
}

pub fn capture_rectangular_union_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology)
        || output.body.topology.solid_count != 1
        || output.body.topology.face_count != 6
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_rectangular_union_references",
            &output.input_digest,
            "Union result is not one closed rectangular solid".to_owned(),
        ));
    }
    let bounds = output.body.topology.bounds_mm;
    let roles = [
        ("extrusion.top", "profile.face", 2_usize, bounds.max.z),
        ("extrusion.bottom", "profile.face", 2_usize, bounds.min.z),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            0_usize,
            bounds.max.x,
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, axis, coordinate) in roles {
        let candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| face.surface_kind == "plane")
            .filter(|face| {
                let min = [
                    face.bounds_mm.min.x,
                    face.bounds_mm.min.y,
                    face.bounds_mm.min.z,
                ];
                let max = [
                    face.bounds_mm.max.x,
                    face.bounds_mm.max.y,
                    face.bounds_mm.max.z,
                ];
                (min[axis] - coordinate).abs() <= 1.0e-6 && (max[axis] - coordinate).abs() <= 1.0e-6
            })
            .collect::<Vec<_>>();
        let [face] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_rectangular_union_references",
                &output.input_digest,
                format!(
                    "Union role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face_ordinal = face.ordinal;
        let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
        output.topology_history.push(HistoryEvidence {
            semantic_role: Some(semantic_role.to_owned()),
            relation: "rectangular_union_classification".to_owned(),
            source_element_id: source_element_id.to_owned(),
            output_face_ordinal: Some(face_ordinal),
        });
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint,
        });
    }
    Ok(references)
}

pub fn capture_rectangular_sweep_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology)
        || output.body.topology.vertex_count != 8
        || output.body.topology.edge_count != 12
        || output.body.topology.face_count != 6
        || output.body.topology.shell_count != 1
        || output.body.topology.solid_count != 1
        || output.body.topology.volume_mm3 <= 0.0
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_rectangular_sweep_references",
            &output.input_digest,
            "Sweep result is not one closed rectangular solid".to_owned(),
        ));
    }
    let required = [
        ("sweep.start", "profile.face"),
        ("sweep.end", "profile.face"),
        ("sweep.side.0", "profile.edge.0"),
        ("sweep.side.1", "profile.edge.1"),
        ("sweep.side.2", "profile.edge.2"),
        ("sweep.side.3", "profile.edge.3"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .collect::<Vec<_>>();
        let [history] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_rectangular_sweep_references",
                &output.input_digest,
                format!(
                    "Sweep role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face_ordinal = history.output_face_ordinal.ok_or_else(|| {
            parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_rectangular_sweep_references",
                &output.input_digest,
                format!("Sweep role {semantic_role} has no output face"),
            )
        })?;
        let face = output
            .body
            .topology
            .faces
            .get(face_ordinal as usize)
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_rectangular_sweep_references",
                    &output.input_digest,
                    format!("Sweep role {semantic_role} has invalid face ordinal"),
                )
            })?;
        if face.surface_kind != "plane" {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_rectangular_sweep_references",
                &output.input_digest,
                format!("Sweep role {semantic_role} is not planar"),
            ));
        }
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_spline_loft_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    let topology = &output.body.topology;
    if topology.faces.len() != topology.face_count as usize
        || topology.edges.len() != topology.edge_count as usize
        || topology
            .edges
            .iter()
            .any(|edge| edge.adjacent_face_ordinals.len() != 2)
        || topology.face_count != 3
        || topology.shell_count != 1
        || topology.solid_count != 1
        || topology.volume_mm3 <= 0.0
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_spline_loft_references",
            &output.input_digest,
            "Spline Loft result is not one closed three-face solid".to_owned(),
        ));
    }
    let required = [
        ("loft.start", "profile.face", "planar_face"),
        ("loft.end", "profile.face", "planar_face"),
        ("loft.side", "profile.edge.spline", "face"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id, expected_type) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .collect::<Vec<_>>();
        let [history] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_spline_loft_references",
                &output.input_digest,
                format!("Loft role {semantic_role} is not unique"),
            ));
        };
        let face_ordinal = history.output_face_ordinal.ok_or_else(|| {
            parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_spline_loft_references",
                &output.input_digest,
                format!("Loft role {semantic_role} has no output face"),
            )
        })?;
        let face = topology.faces.get(face_ordinal as usize).ok_or_else(|| {
            parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_spline_loft_references",
                &output.input_digest,
                format!("Loft role {semantic_role} has an invalid face ordinal"),
            )
        })?;
        if expected_type == "planar_face" && face.surface_kind != "plane" {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_spline_loft_references",
                &output.input_digest,
                format!("Loft cap {semantic_role} is not planar"),
            ));
        }
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:{expected_type}"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: expected_type.to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_planar_offset_reference(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<SubshapeRef, GeometryError> {
    let topology = &output.body.topology;
    let [face] = topology.faces.as_slice() else {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_planar_offset_reference",
            &output.input_digest,
            "Planar offset must produce exactly one face".to_owned(),
        ));
    };
    if topology.vertex_count != 4
        || topology.edge_count != 4
        || topology.face_count != 1
        || topology.shell_count != 0
        || topology.solid_count != 0
        || topology.volume_mm3.abs() > 1.0e-12
        || face.surface_kind != "plane"
        || face.edge_count != 4
        || topology
            .edges
            .iter()
            .any(|edge| edge.adjacent_face_ordinals != [0])
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_planar_offset_reference",
            &output.input_digest,
            "Planar offset topology is not one bounded rectangular face".to_owned(),
        ));
    }
    let face_ordinal = face.ordinal;
    let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
    output.topology_history.push(HistoryEvidence {
        semantic_role: Some("planar_offset.face".to_owned()),
        relation: "rectangular_offset_classification".to_owned(),
        source_element_id: "profile.face".to_owned(),
        output_face_ordinal: Some(face_ordinal),
    });
    let lineage =
        format!("{document_id}:{producer_feature_id}:planar_offset.face:profile.face:planar_face");
    Ok(SubshapeRef {
        document_id: document_id.to_owned(),
        producer_feature_id: producer_feature_id.to_owned(),
        semantic_role: "planar_offset.face".to_owned(),
        source_element_id: "profile.face".to_owned(),
        expected_type: "planar_face".to_owned(),
        stability_class: StabilityClass::Guaranteed,
        backend_fingerprint: output.backend_fingerprint.to_owned(),
        lineage_digest: stable_digest(&lineage),
        corroborating_geometry_fingerprint,
    })
}

pub fn capture_rectangular_intersection_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology)
        || output.body.topology.solid_count != 1
        || output.body.topology.face_count != 6
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_rectangular_intersection_references",
            &output.input_digest,
            "Intersection result is not one closed rectangular solid".to_owned(),
        ));
    }
    let bounds = output.body.topology.bounds_mm;
    let roles = [
        ("extrusion.top", "profile.face", 2_usize, bounds.max.z),
        ("extrusion.bottom", "profile.face", 2_usize, bounds.min.z),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            0_usize,
            bounds.max.x,
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, axis, coordinate) in roles {
        let candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| face.surface_kind == "plane")
            .filter(|face| {
                let min = [
                    face.bounds_mm.min.x,
                    face.bounds_mm.min.y,
                    face.bounds_mm.min.z,
                ];
                let max = [
                    face.bounds_mm.max.x,
                    face.bounds_mm.max.y,
                    face.bounds_mm.max.z,
                ];
                (min[axis] - coordinate).abs() <= 1.0e-6 && (max[axis] - coordinate).abs() <= 1.0e-6
            })
            .collect::<Vec<_>>();
        let [face] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_rectangular_intersection_references",
                &output.input_digest,
                format!(
                    "Intersection role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face_ordinal = face.ordinal;
        let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
        output.topology_history.push(HistoryEvidence {
            semantic_role: Some(semantic_role.to_owned()),
            relation: "rectangular_intersection_classification".to_owned(),
            source_element_id: source_element_id.to_owned(),
            output_face_ordinal: Some(face_ordinal),
        });
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint,
        });
    }
    Ok(references)
}

pub fn capture_rectangular_split_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_partition_adjacency(&output.body.topology)
        || output.body.topology.solid_count < 2
        || output.body.topology.shell_count != output.body.topology.solid_count
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_rectangular_split_references",
            &output.input_digest,
            "Split result is not a closed multi-solid target partition".to_owned(),
        ));
    }
    let bounds = output.body.topology.bounds_mm;
    let roles = [
        ("extrusion.top", "profile.face", 2_usize, bounds.max.z),
        ("extrusion.bottom", "profile.face", 2_usize, bounds.min.z),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            0_usize,
            bounds.max.x,
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, axis, coordinate) in roles {
        let mut candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| face.surface_kind == "plane")
            .filter(|face| {
                let min = [
                    face.bounds_mm.min.x,
                    face.bounds_mm.min.y,
                    face.bounds_mm.min.z,
                ];
                let max = [
                    face.bounds_mm.max.x,
                    face.bounds_mm.max.y,
                    face.bounds_mm.max.z,
                ];
                (min[axis] - coordinate).abs() <= 1.0e-6 && (max[axis] - coordinate).abs() <= 1.0e-6
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .area_mm2
                .total_cmp(&left.area_mm2)
                .then_with(|| left.geometric_fingerprint.cmp(&right.geometric_fingerprint))
        });
        let Some(face) = candidates.first() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_rectangular_split_references",
                &output.input_digest,
                format!("Split role {semantic_role} has no candidate"),
            ));
        };
        let face_ordinal = face.ordinal;
        let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
        output.topology_history.push(HistoryEvidence {
            semantic_role: Some(semantic_role.to_owned()),
            relation: "rectangular_split_classification".to_owned(),
            source_element_id: source_element_id.to_owned(),
            output_face_ordinal: Some(face_ordinal),
        });
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint,
        });
    }
    Ok(references)
}

pub fn capture_half_lap_notch_references(
    output: &ExactOpOutput,
    document_id: u64,
    piece_key: &str,
    notches: &[HalfLapNotchSpec],
) -> Result<Vec<HalfLapFaceEvidence>, GeometryError> {
    if piece_key.is_empty()
        || notches.is_empty()
        || !has_complete_manifold_adjacency(&output.body.topology)
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_half_lap_notch_references",
            &output.input_digest,
            "Half-lap output lacks identity, notch inputs, or reciprocal manifold adjacency"
                .to_owned(),
        ));
    }
    let mut references = Vec::new();
    for notch in notches {
        let roles = if notch.participant == HalfLapParticipant::A {
            vec![
                (HalfLapFaceRole::Contact, 2_usize, notch.removed.origin_mm.z),
                (
                    HalfLapFaceRole::WestWall,
                    0_usize,
                    notch.removed.origin_mm.x,
                ),
                (
                    HalfLapFaceRole::EastWall,
                    0_usize,
                    notch.removed.origin_mm.x + notch.removed.size_mm.x,
                ),
            ]
        } else {
            vec![(
                HalfLapFaceRole::Contact,
                2_usize,
                notch.removed.origin_mm.z + notch.removed.size_mm.z,
            )]
        };
        for (role, axis, coordinate) in roles {
            let candidates = output
                .body
                .topology
                .faces
                .iter()
                .filter(|face| face.surface_kind == "plane")
                .filter(|face| half_lap_face_matches(face, *notch, role, axis, coordinate))
                .collect::<Vec<_>>();
            let [face] = candidates.as_slice() else {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_half_lap_notch_references",
                    &output.input_digest,
                    format!(
                        "Half-lap joint {} participant {} role {} has {} candidates",
                        notch.joint_id,
                        notch.participant.token(),
                        role.token(),
                        candidates.len()
                    ),
                ));
            };
            references.push(HalfLapFaceEvidence {
                joint_id: notch.joint_id,
                participant: notch.participant,
                role,
                face_ordinal: face.ordinal,
                lineage_digest: half_lap_lineage_digest(
                    document_id,
                    piece_key,
                    notch.joint_id,
                    notch.participant,
                    role,
                ),
                geometric_fingerprint: face.geometric_fingerprint.clone(),
            });
        }
    }
    references.sort_by_key(|reference| {
        (
            reference.joint_id,
            reference.participant.token(),
            reference.role.token(),
        )
    });
    if references.windows(2).any(|pair| {
        pair[0].joint_id == pair[1].joint_id
            && pair[0].participant == pair[1].participant
            && pair[0].role == pair[1].role
    }) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_half_lap_notch_references",
            &output.input_digest,
            "Half-lap reference roles are not unique".to_owned(),
        ));
    }
    Ok(references)
}

fn half_lap_face_matches(
    face: &FaceEvidence,
    notch: HalfLapNotchSpec,
    role: HalfLapFaceRole,
    axis: usize,
    coordinate: f64,
) -> bool {
    let min = [
        face.bounds_mm.min.x,
        face.bounds_mm.min.y,
        face.bounds_mm.min.z,
    ];
    let max = [
        face.bounds_mm.max.x,
        face.bounds_mm.max.y,
        face.bounds_mm.max.z,
    ];
    if (min[axis] - coordinate).abs() > 1.0e-6 || (max[axis] - coordinate).abs() > 1.0e-6 {
        return false;
    }
    let removed_min = [
        notch.removed.origin_mm.x,
        notch.removed.origin_mm.y,
        notch.removed.origin_mm.z,
    ];
    let removed_max = [
        notch.removed.origin_mm.x + notch.removed.size_mm.x,
        notch.removed.origin_mm.y + notch.removed.size_mm.y,
        notch.removed.origin_mm.z + notch.removed.size_mm.z,
    ];
    match role {
        HalfLapFaceRole::Contact => {
            (min[0] - removed_min[0]).abs() <= 1.0e-6
                && (max[0] - removed_max[0]).abs() <= 1.0e-6
                && (min[1] - removed_min[1]).abs() <= 1.0e-6
                && (max[1] - removed_max[1]).abs() <= 1.0e-6
        }
        HalfLapFaceRole::WestWall | HalfLapFaceRole::EastWall => {
            (min[1] - removed_min[1]).abs() <= 1.0e-6
                && (max[1] - removed_max[1]).abs() <= 1.0e-6
                && (min[2] - removed_min[2]).abs() <= 1.0e-6
                && (max[2] - removed_max[2]).abs() <= 1.0e-6
        }
    }
}

fn half_lap_lineage_digest(
    document_id: u64,
    piece_key: &str,
    joint_id: u64,
    participant: HalfLapParticipant,
    role: HalfLapFaceRole,
) -> String {
    stable_digest(&format!(
        "{document_id}:{piece_key}:{joint_id}:{}:{}:planar_face",
        participant.token(),
        role.token()
    ))
}

fn face_matches_cut_role(
    face: &FaceEvidence,
    semantic_role: &str,
    axis: usize,
    coordinate: f64,
    base: RectangleExtrudeSpec,
    cut: BoxSpec,
) -> bool {
    let mins = [
        face.bounds_mm.min.x,
        face.bounds_mm.min.y,
        face.bounds_mm.min.z,
    ];
    let maxs = [
        face.bounds_mm.max.x,
        face.bounds_mm.max.y,
        face.bounds_mm.max.z,
    ];
    if (mins[axis] - coordinate).abs() > 1.0e-6 || (maxs[axis] - coordinate).abs() > 1.0e-6 {
        return false;
    }
    let cut_max_x = cut.origin_mm.x + cut.size_mm.x;
    let cut_max_y = cut.origin_mm.y + cut.size_mm.y;
    match semantic_role {
        "extrusion.top" | "extrusion.bottom" => {
            (mins[0]).abs() <= 1.0e-6
                && (mins[1]).abs() <= 1.0e-6
                && (maxs[0] - base.width_mm).abs() <= 1.0e-6
                && (maxs[1] - base.depth_mm).abs() <= 1.0e-6
        }
        "extrusion.side(profile_edge=east)" => {
            mins[1].abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[1] - base.depth_mm).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        "through_cut.wall.west" | "through_cut.wall.east" => {
            (mins[1] - cut.origin_mm.y).abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[1] - cut_max_y).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        "through_cut.wall.south" | "through_cut.wall.north" => {
            (mins[0] - cut.origin_mm.x).abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[0] - cut_max_x).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        _ => false,
    }
}

fn face_matches_pocket_role(
    face: &FaceEvidence,
    semantic_role: &str,
    axis: usize,
    coordinate: f64,
    base: RectangleExtrudeSpec,
    pocket: BoxSpec,
) -> bool {
    let mins = [
        face.bounds_mm.min.x,
        face.bounds_mm.min.y,
        face.bounds_mm.min.z,
    ];
    let maxs = [
        face.bounds_mm.max.x,
        face.bounds_mm.max.y,
        face.bounds_mm.max.z,
    ];
    if (mins[axis] - coordinate).abs() > 1.0e-6 || (maxs[axis] - coordinate).abs() > 1.0e-6 {
        return false;
    }
    let max_x = pocket.origin_mm.x + pocket.size_mm.x;
    let max_y = pocket.origin_mm.y + pocket.size_mm.y;
    match semantic_role {
        "extrusion.top" | "extrusion.bottom" => {
            mins[0].abs() <= 1.0e-6
                && mins[1].abs() <= 1.0e-6
                && (maxs[0] - base.width_mm).abs() <= 1.0e-6
                && (maxs[1] - base.depth_mm).abs() <= 1.0e-6
        }
        "extrusion.side(profile_edge=east)" => {
            mins[1].abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[1] - base.depth_mm).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        "pocket.floor" => {
            (mins[0] - pocket.origin_mm.x).abs() <= 1.0e-6
                && (mins[1] - pocket.origin_mm.y).abs() <= 1.0e-6
                && (maxs[0] - max_x).abs() <= 1.0e-6
                && (maxs[1] - max_y).abs() <= 1.0e-6
        }
        "pocket.wall.west" | "pocket.wall.east" => {
            (mins[1] - pocket.origin_mm.y).abs() <= 1.0e-6
                && (mins[2] - pocket.origin_mm.z).abs() <= 1.0e-6
                && (maxs[1] - max_y).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        "pocket.wall.south" | "pocket.wall.north" => {
            (mins[0] - pocket.origin_mm.x).abs() <= 1.0e-6
                && (mins[2] - pocket.origin_mm.z).abs() <= 1.0e-6
                && (maxs[0] - max_x).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        _ => false,
    }
}

#[must_use]
pub fn resolve_subshape_reference(
    reference: &SubshapeRef,
    output: &ExactOpOutput,
) -> ReferenceResolution {
    let migrated_backend = reference.backend_fingerprint != output.backend_fingerprint;
    let expected_lineage = stable_digest(&format!(
        "{}:{}:{}:{}:{}",
        reference.document_id,
        reference.producer_feature_id,
        reference.semantic_role,
        reference.source_element_id,
        reference.expected_type
    ));
    if reference.document_id.is_empty()
        || reference.producer_feature_id.is_empty()
        || reference.lineage_digest != expected_lineage
    {
        return if migrated_backend {
            ReferenceResolution::QuarantinedMigration {
                reason: "Reference provenance or lineage digest is invalid".to_owned(),
            }
        } else {
            ReferenceResolution::Lost
        };
    }
    let mut candidates = output
        .topology_history
        .iter()
        .filter(|entry| {
            entry.semantic_role.as_deref() == Some(reference.semantic_role.as_str())
                && entry.source_element_id == reference.source_element_id
        })
        .filter_map(|entry| entry.output_face_ordinal)
        .filter(|ordinal| {
            output.body.topology.faces.iter().any(|face| {
                face.ordinal == *ordinal
                    && (reference.expected_type != "planar_face" || face.surface_kind == "plane")
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    match candidates.as_slice() {
        [face_ordinal] => ReferenceResolution::Resolved {
            face_ordinal: *face_ordinal,
            migrated_backend,
        },
        [] if migrated_backend => ReferenceResolution::QuarantinedMigration {
            reason: format!(
                "No lineage match for {} after backend migration",
                reference.semantic_role
            ),
        },
        [] => ReferenceResolution::Lost,
        _ => ReferenceResolution::Ambiguous {
            candidate_ordinals: candidates,
        },
    }
}

fn validate_bottle_shell_thickness(
    points_mm: &[[f64; 2]],
    thickness_mm: f64,
    input: &str,
) -> Result<(), GeometryError> {
    let shoulder = [
        points_mm[3][0] - points_mm[2][0],
        points_mm[3][1] - points_mm[2][1],
    ];
    let shoulder_length = shoulder[0].hypot(shoulder[1]);
    let minimum_radius = points_mm[1..5]
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    if !thickness_mm.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            "shell_revolve_profile",
            input,
            "Shell thickness must be finite".to_owned(),
        ));
    }
    if thickness_mm < MIN_LENGTH_MM
        || thickness_mm >= minimum_radius * 0.5
        || thickness_mm >= shoulder_length * 0.5
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            "shell_revolve_profile",
            input,
            "Shell thickness is outside the conservative bottle offset envelope".to_owned(),
        ));
    }
    Ok(())
}

fn validate_bottle_finish_amount(
    points_mm: &[[f64; 2]],
    amount_mm: f64,
    input: &str,
) -> Result<(), GeometryError> {
    let shoulder_length =
        (points_mm[3][0] - points_mm[2][0]).hypot(points_mm[3][1] - points_mm[2][1]);
    if !amount_mm.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            "finish_shell_revolve_profile",
            input,
            "Bottle edge finish amount must be finite".to_owned(),
        ));
    }
    if amount_mm < MIN_LENGTH_MM
        || amount_mm >= shoulder_length * 0.25
        || amount_mm >= points_mm[3][0] * 0.25
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            "finish_shell_revolve_profile",
            input,
            "Bottle edge finish amount is outside the conservative shoulder envelope".to_owned(),
        ));
    }
    Ok(())
}

fn validate_bottle_revolve_profile(
    points_mm: &[[f64; 2]],
    input: &str,
) -> Result<(), GeometryError> {
    let valid_coordinates = points_mm
        .iter()
        .flatten()
        .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM);
    let valid_axis = points_mm.len() == 6
        && points_mm.first().is_some_and(|point| point[0] == 0.0)
        && points_mm.last().is_some_and(|point| point[0] == 0.0)
        && points_mm[1..points_mm.len() - 1]
            .iter()
            .all(|point| point[0] >= MIN_LENGTH_MM);
    let distinct = points_mm.iter().enumerate().all(|(index, point)| {
        points_mm[index + 1..]
            .iter()
            .all(|candidate| point != candidate)
    });
    if !valid_coordinates || !valid_axis || !distinct {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "revolve_profile",
            input,
            "Bottle revolve requires six finite distinct [radius, z] points with only its endpoints on the Z axis".to_owned(),
        ));
    }
    Ok(())
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
    if !(2..=64).contains(&segments.len()) {
        return Err(invalid(
            "Revolve profile requires 2..=64 segments".to_owned(),
        ));
    }
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
    let endpoints = |segment: &PlanarProfileSegment| match segment {
        PlanarProfileSegment::Line { start_mm, end_mm }
        | PlanarProfileSegment::CircularArc {
            start_mm, end_mm, ..
        } => (*start_mm, *end_mm),
    };
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
    input: &str,
) -> Result<(), GeometryError> {
    let invalid = |diagnostic: String| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            "extrude_mixed_profile",
            input,
            diagnostic,
        )
    };
    if !(2..=64).contains(&segments.len())
        || !segments
            .iter()
            .any(|segment| matches!(segment, PlanarProfileSegment::Line { .. }))
        || !segments
            .iter()
            .any(|segment| matches!(segment, PlanarProfileSegment::CircularArc { .. }))
    {
        return Err(invalid(
            "Mixed profile requires 2..=64 segments including a line and a circular arc".to_owned(),
        ));
    }
    let endpoints = |segment: &PlanarProfileSegment| match segment {
        PlanarProfileSegment::Line { start_mm, end_mm }
        | PlanarProfileSegment::CircularArc {
            start_mm, end_mm, ..
        } => (*start_mm, *end_mm),
    };
    for (index, segment) in segments.iter().enumerate() {
        let (start, end) = endpoints(segment);
        for (coordinate, name) in [
            (start[0], "start_x"),
            (start[1], "start_y"),
            (end[0], "end_x"),
            (end[1], "end_y"),
        ] {
            validate_coordinate(coordinate, name, "extrude_mixed_profile", input)?;
        }
        if start == end {
            return Err(invalid(format!("Profile segment {index} is degenerate")));
        }
        if let PlanarProfileSegment::CircularArc { center_mm, .. } = segment {
            validate_coordinate(center_mm[0], "center_x", "extrude_mixed_profile", input)?;
            validate_coordinate(center_mm[1], "center_y", "extrude_mixed_profile", input)?;
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

fn classify_box_intersection(
    base: Bounds3,
    tool: BoxSpec,
    operation: &'static str,
    operation_label: &str,
    input: &str,
) -> Result<(), GeometryError> {
    let tool_max = Point3 {
        x: tool.origin_mm.x + tool.size_mm.x,
        y: tool.origin_mm.y + tool.size_mm.y,
        z: tool.origin_mm.z + tool.size_mm.z,
    };
    let overlaps = [
        base.max.x.min(tool_max.x) - base.min.x.max(tool.origin_mm.x),
        base.max.y.min(tool_max.y) - base.min.y.max(tool.origin_mm.y),
        base.max.z.min(tool_max.z) - base.min.z.max(tool.origin_mm.z),
    ];
    const INTERSECTION_TOLERANCE_MM: f64 = 1.0e-6;
    if overlaps
        .iter()
        .any(|overlap| *overlap < -INTERSECTION_TOLERANCE_MM)
    {
        return Err(parameter_error(
            GeometryErrorCode::NoGeometricChange,
            operation,
            input,
            format!("{operation_label} tool does not intersect the exact body's bounds"),
        ));
    }
    if overlaps
        .iter()
        .any(|overlap| overlap.abs() <= INTERSECTION_TOLERANCE_MM)
    {
        return Err(parameter_error(
            GeometryErrorCode::DegenerateOperation,
            operation,
            input,
            format!("{operation_label} tool only touches the exact body's bounds"),
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

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayToken {
    pub seed: u64,
    pub case_index: u32,
}

impl fmt::Display for ReplayToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "r0-v1:{}:{}", self.seed, self.case_index)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GeneratedOperation {
    Extrude(RectangleExtrudeSpec),
    Cut {
        base: BoxSpec,
        tool: BoxSpec,
        mode: CutMode,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedCase {
    pub replay: ReplayToken,
    pub operation: GeneratedOperation,
}

#[derive(Clone, Copy, Debug)]
pub struct StructuredGenerator {
    seed: u64,
}

impl StructuredGenerator {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    #[must_use]
    pub fn case(&self, case_index: u32) -> GeneratedCase {
        let mut random = SplitMix64::new(self.seed);
        for _ in 0..case_index {
            let _ = random.next();
        }
        let selector = random.next() % 100;
        let operation = if selector < 50 {
            GeneratedOperation::Extrude(RectangleExtrudeSpec {
                width_mm: generated_length(&mut random, 10_000, 100_000_000),
                depth_mm: generated_length(&mut random, 10_000, 100_000_000),
                height_mm: generated_length(&mut random, 10_000, 100_000_000),
            })
        } else {
            let base_size = Size3 {
                x: generated_length(&mut random, 30_000, 100_000_000),
                y: generated_length(&mut random, 30_000, 100_000_000),
                z: generated_length(&mut random, 30_000, 99_998_000),
            };
            let tool_x = bounded_tool_length(&mut random, base_size.x);
            let tool_y = bounded_tool_length(&mut random, base_size.y);
            let margin_x = (base_size.x - tool_x) / 2.0;
            let margin_y = (base_size.y - tool_y) / 2.0;
            let mode = if selector < 80 {
                CutMode::ThroughAll
            } else {
                CutMode::BlindPlanar
            };
            let (origin_z, size_z) = match mode {
                CutMode::ThroughAll => (-1.0, base_size.z + 2.0),
                CutMode::BlindPlanar => (base_size.z / 2.0, base_size.z / 2.0 + 1.0),
            };
            GeneratedOperation::Cut {
                base: BoxSpec {
                    origin_mm: Point3::ORIGIN,
                    size_mm: base_size,
                },
                tool: BoxSpec {
                    origin_mm: Point3 {
                        x: margin_x,
                        y: margin_y,
                        z: origin_z,
                    },
                    size_mm: Size3 {
                        x: tool_x,
                        y: tool_y,
                        z: size_z,
                    },
                },
                mode,
            }
        };
        GeneratedCase {
            replay: ReplayToken {
                seed: self.seed,
                case_index,
            },
            operation,
        }
    }
}

impl ExactBackend {
    pub fn execute_generated(&self, case: &GeneratedCase) -> Result<ExactOpOutput, GeometryError> {
        match case.operation {
            GeneratedOperation::Extrude(spec) => self.extrude_rectangle(spec),
            GeneratedOperation::Cut { base, tool, mode } => {
                let base = self.make_box(base)?;
                self.cut_box(&base.body, tool, mode)
            }
        }
    }
}

#[must_use]
pub fn minimize_replayable_failure<F>(case: &GeneratedCase, mut still_fails: F) -> GeneratedCase
where
    F: FnMut(&GeneratedCase) -> bool,
{
    if !still_fails(case) {
        return case.clone();
    }
    let mut current = case.clone();
    loop {
        let candidate = shrink_case(&current);
        if candidate == current || !still_fails(&candidate) {
            return current;
        }
        current = candidate;
    }
}

fn shrink_case(case: &GeneratedCase) -> GeneratedCase {
    let operation = match case.operation {
        GeneratedOperation::Extrude(spec) => GeneratedOperation::Extrude(RectangleExtrudeSpec {
            width_mm: shrink_length(spec.width_mm),
            depth_mm: shrink_length(spec.depth_mm),
            height_mm: shrink_length(spec.height_mm),
        }),
        GeneratedOperation::Cut {
            base,
            tool: _,
            mode,
        } => {
            let size = Size3 {
                x: shrink_length(base.size_mm.x),
                y: shrink_length(base.size_mm.y),
                z: shrink_length(base.size_mm.z),
            };
            let tool_x = (size.x / 2.0).max(MIN_LENGTH_MM);
            let tool_y = (size.y / 2.0).max(MIN_LENGTH_MM);
            let (origin_z, size_z) = match mode {
                CutMode::ThroughAll => (-MIN_LENGTH_MM, size.z + 2.0 * MIN_LENGTH_MM),
                CutMode::BlindPlanar => (size.z / 2.0, size.z / 2.0 + MIN_LENGTH_MM),
            };
            GeneratedOperation::Cut {
                base: BoxSpec {
                    origin_mm: Point3::ORIGIN,
                    size_mm: size,
                },
                tool: BoxSpec {
                    origin_mm: Point3 {
                        x: (size.x - tool_x) / 2.0,
                        y: (size.y - tool_y) / 2.0,
                        z: origin_z,
                    },
                    size_mm: Size3 {
                        x: tool_x,
                        y: tool_y,
                        z: size_z,
                    },
                },
                mode,
            }
        }
    };
    GeneratedCase {
        replay: case.replay,
        operation,
    }
}

fn shrink_length(value: f64) -> f64 {
    ((value + MIN_LENGTH_MM) / 2.0).max(MIN_LENGTH_MM)
}

fn generated_length(random: &mut SplitMix64, minimum_um: u64, maximum_um: u64) -> f64 {
    let span = maximum_um - minimum_um + 1;
    (minimum_um + random.next() % span) as f64 / 1000.0
}

fn bounded_tool_length(random: &mut SplitMix64, base_mm: f64) -> f64 {
    let base_um = (base_mm * 1000.0).round() as u64;
    let maximum_um = base_um.saturating_sub(2_000).max(10_000);
    generated_length(random, 10_000, maximum_um)
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
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
    fn fixed_extrusion_is_valid_and_carries_guaranteed_history() {
        let output = ExactBackend::new()
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: 60.0,
                height_mm: 20.0,
            })
            .expect("fixed extrusion must succeed");

        assert_eq!(output.body.topology.solid_count, 1);
        assert_eq!(output.body.topology.face_count, 6);
        assert_close(output.body.topology.volume_mm3, 120_000.0);
        assert_eq!(output.history_confidence, HistoryConfidence::Complete);
        for role in [
            "extrusion.top",
            "extrusion.bottom",
            "extrusion.side(profile_edge=east)",
        ] {
            assert!(output.topology_history.iter().any(|entry| {
                entry.semantic_role.as_deref() == Some(role) && entry.output_face_ordinal.is_some()
            }));
        }
        let references = capture_guaranteed_references(&output, "unit-document", "extrusion-001")
            .expect("Guaranteed references must be captured from backend history");
        for (role, expected_axis_coordinate, axis) in [
            ("extrusion.top", 20.0, 'z'),
            ("extrusion.bottom", 0.0, 'z'),
            ("extrusion.side(profile_edge=east)", 100.0, 'x'),
        ] {
            let reference = references
                .iter()
                .find(|reference| reference.semantic_role == role)
                .expect("Guaranteed reference must exist");
            let ReferenceResolution::Resolved { face_ordinal, .. } =
                resolve_subshape_reference(reference, &output)
            else {
                panic!("Guaranteed reference must resolve uniquely");
            };
            let face = output
                .body
                .topology
                .faces
                .iter()
                .find(|face| face.ordinal == face_ordinal)
                .expect("Guaranteed face ordinal must exist");
            assert_eq!(face.surface_kind, "plane");
            let coordinate = if axis == 'x' {
                face.centroid_mm.x
            } else {
                face.centroid_mm.z
            };
            assert_close(coordinate, expected_axis_coordinate);
        }
        assert!(output.input_digest.starts_with("fnv1a64:"));
        assert!(output.body.result_fingerprint.starts_with("fnv1a64:"));
    }

    #[test]
    fn fixed_bottle_revolve_is_valid_and_carries_five_durable_faces() {
        let profile = [
            [0.0, 0.0],
            [30.0, 0.0],
            [30.0, 110.0],
            [12.0, 130.0],
            [12.0, 155.0],
            [0.0, 155.0],
        ];
        let output = ExactBackend::new()
            .revolve_profile(&profile)
            .expect("fixed bottle revolve must succeed");
        assert_eq!(output.body.topology.solid_count, 1);
        assert_eq!(output.body.topology.shell_count, 1);
        assert_close(output.body.topology.bounds_mm.min.x, -30.0);
        assert_close(output.body.topology.bounds_mm.min.y, -30.0);
        assert_close(output.body.topology.bounds_mm.min.z, 0.0);
        assert_close(output.body.topology.bounds_mm.max.x, 30.0);
        assert_close(output.body.topology.bounds_mm.max.y, 30.0);
        assert_close(output.body.topology.bounds_mm.max.z, 155.0);
        assert_close(
            output.body.topology.volume_mm3,
            111_960.0 * std::f64::consts::PI,
        );
        assert_eq!(output.history_confidence, HistoryConfidence::Complete);

        let references = capture_revolve_references(&output, "unit-document", "revolve-001")
            .expect("revolve references must come from OCCT history");
        assert_eq!(references.len(), 5);
        for role in [
            "revolve.bottom",
            "revolve.body",
            "revolve.shoulder",
            "revolve.neck",
            "revolve.mouth",
        ] {
            let reference = references
                .iter()
                .find(|reference| reference.semantic_role == role)
                .expect("every bounded revolve role must exist");
            assert!(matches!(
                resolve_subshape_reference(reference, &output),
                ReferenceResolution::Resolved {
                    migrated_backend: false,
                    ..
                }
            ));
        }
    }

    #[test]
    fn fixed_bottle_shell_is_valid_open_and_carries_nine_durable_faces() {
        let profile = [
            [0.0, 0.0],
            [30.0, 0.0],
            [30.0, 110.0],
            [12.0, 130.0],
            [12.0, 155.0],
            [0.0, 155.0],
        ];
        let backend = ExactBackend::new();
        let output = backend
            .shell_revolve_profile(&profile, 2.0)
            .expect("bounded bottle shell must succeed");
        assert_eq!(output.body.topology.solid_count, 1);
        assert_eq!(output.body.topology.shell_count, 1);
        assert_eq!(output.body.topology.face_count, 9);
        assert!(output.body.topology.volume_mm3 > 0.0);
        assert!(output.body.topology.volume_mm3 < 111_960.0 * std::f64::consts::PI);
        let references = capture_shell_references(&output, "unit-document", "shell-001")
            .expect("shell roles must resolve uniquely");
        assert_eq!(references.len(), 9);
        assert!(references.iter().all(|reference| matches!(
            resolve_subshape_reference(reference, &output),
            ReferenceResolution::Resolved {
                migrated_backend: false,
                ..
            }
        )));

        let error = backend
            .shell_revolve_profile(&profile, 6.0)
            .expect_err("half-neck-radius thickness must fail conservatively");
        assert_eq!(error.code, GeometryErrorCode::InvalidParameter);
    }

    #[test]
    fn fixed_bottle_fillet_and_chamfer_are_exact_and_preserve_shell_roles() {
        let profile = [
            [0.0, 0.0],
            [30.0, 0.0],
            [30.0, 110.0],
            [12.0, 130.0],
            [12.0, 155.0],
            [0.0, 155.0],
        ];
        let backend = ExactBackend::new();
        let shell = backend.shell_revolve_profile(&profile, 2.0).unwrap();
        for finish in [BottleEdgeFinish::Fillet, BottleEdgeFinish::Chamfer] {
            let output = backend
                .finish_shell_revolve_profile(&profile, 2.0, finish, 2.0)
                .expect("bounded shoulder finish must succeed");
            assert_eq!(output.body.topology.solid_count, 1);
            assert_eq!(output.body.topology.shell_count, 1);
            assert!(output.body.topology.face_count > shell.body.topology.face_count);
            assert_ne!(
                output.body.result_fingerprint,
                shell.body.result_fingerprint
            );
            let references = capture_shell_references(&output, "unit-document", "finish-001")
                .expect("all nine shell roles must survive the finish");
            assert_eq!(references.len(), 9);
            assert!(references.iter().all(|reference| matches!(
                resolve_subshape_reference(reference, &output),
                ReferenceResolution::Resolved {
                    migrated_backend: false,
                    ..
                }
            )));
        }
        assert_eq!(
            backend
                .finish_shell_revolve_profile(&profile, 2.0, BottleEdgeFinish::Fillet, 8.0)
                .unwrap_err()
                .code,
            GeometryErrorCode::InvalidParameter
        );
    }

    #[test]
    fn fixed_through_cut_is_valid_and_captures_history() {
        let backend = ExactBackend::new();
        let base = backend
            .make_box(BoxSpec {
                origin_mm: Point3::ORIGIN,
                size_mm: Size3 {
                    x: 40.0,
                    y: 30.0,
                    z: 10.0,
                },
            })
            .expect("base must succeed");
        let output = backend
            .cut_box(
                &base.body,
                BoxSpec {
                    origin_mm: Point3 {
                        x: 10.0,
                        y: 10.0,
                        z: -5.0,
                    },
                    size_mm: Size3 {
                        x: 20.0,
                        y: 10.0,
                        z: 20.0,
                    },
                },
                CutMode::ThroughAll,
            )
            .expect("fixed cut must succeed");

        assert_eq!(output.body.topology.solid_count, 1);
        assert_close(output.body.topology.volume_mm3, 10_000.0);
        assert_eq!(output.history_confidence, HistoryConfidence::Partial);
        assert!(!output.topology_history.is_empty());
    }

    #[test]
    fn invalid_and_degenerate_inputs_have_stable_typed_errors() {
        let backend = ExactBackend::new();
        let error = backend
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: 0.0,
                depth_mm: 10.0,
                height_mm: 10.0,
            })
            .expect_err("zero width must be rejected");
        assert_eq!(error.code, GeometryErrorCode::InvalidParameter);

        let error = backend
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: f64::NAN,
                depth_mm: 10.0,
                height_mm: 10.0,
            })
            .expect_err("non-finite width must be rejected");
        assert_eq!(error.code, GeometryErrorCode::NonFiniteParameter);

        let base = backend
            .make_box(BoxSpec {
                origin_mm: Point3::ORIGIN,
                size_mm: Size3 {
                    x: 10.0,
                    y: 10.0,
                    z: 10.0,
                },
            })
            .expect("base must succeed");
        let error = backend
            .cut_box(
                &base.body,
                BoxSpec {
                    origin_mm: Point3 {
                        x: 10.0,
                        y: 2.0,
                        z: 2.0,
                    },
                    size_mm: Size3 {
                        x: 2.0,
                        y: 2.0,
                        z: 2.0,
                    },
                },
                CutMode::BlindPlanar,
            )
            .expect_err("touch-only cut must be rejected");
        assert_eq!(error.code, GeometryErrorCode::DegenerateOperation);
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

    #[test]
    fn structured_cases_replay_and_execute() {
        let backend = ExactBackend::new();
        for seed in [1, 7, 19] {
            let generator = StructuredGenerator::new(seed);
            for index in 0..5 {
                let first = generator.case(index);
                let replayed =
                    StructuredGenerator::new(first.replay.seed).case(first.replay.case_index);
                assert_eq!(first, replayed);
                let output = backend
                    .execute_generated(&first)
                    .unwrap_or_else(|error| panic!("{} failed: {error}", first.replay));
                assert!(output.tolerance_report.accepted_exact_solid);
            }
        }
    }

    #[test]
    fn minimizer_preserves_replay_token_and_structure() {
        let case = GeneratedCase {
            replay: ReplayToken {
                seed: 43,
                case_index: 9,
            },
            operation: GeneratedOperation::Extrude(RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: 60.0,
                height_mm: 20.0,
            }),
        };
        let minimized = minimize_replayable_failure(&case, |candidate| match candidate.operation {
            GeneratedOperation::Extrude(spec) => spec.width_mm > 20.0,
            GeneratedOperation::Cut { .. } => false,
        });
        assert_eq!(minimized.replay, case.replay);
        let GeneratedOperation::Extrude(spec) = minimized.operation else {
            panic!("minimizer changed operation structure");
        };
        assert!(spec.width_mm > 20.0);
        assert!(spec.width_mm < 100.0);
    }
}
