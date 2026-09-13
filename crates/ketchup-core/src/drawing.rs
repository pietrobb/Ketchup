#![forbid(unsafe_code)]

use crate::assembly::{
    AssemblyReferenceHealth, AssemblySolveStatus, AssemblySolverPolicy, solve_rigid_assembly,
};
use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentId, DocumentStore, InstancePath,
    InstancePathStep, OccurrenceId, Proposal, ProposalPrepareError, Snapshot, Transform,
};
use crate::exact_product::{ExactBRepGraphEdgeEvidence, ExactBodyPackage, ExactResultRegistry};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub const ORTHOGRAPHIC_DRAWING_SCHEMA_V1: &str = "ketchup.orthographic-drawing.v1";
pub const ORTHOGRAPHIC_DRAWING_SCHEMA_V2: &str = "ketchup.orthographic-drawing.v2";
pub const ORTHOGRAPHIC_LINEWORK_SCHEMA_V2: &str = "ketchup.orthographic-linework.v2";
pub const DRAWING_SHEET_LAYOUT_SCHEMA_V1: &str = "ketchup.drawing-sheet-layout.v1";
pub const DRAWING_SHEET_LAYOUT_SCHEMA_V2: &str = "ketchup.drawing-sheet-layout.v2";
const VISIBILITY_EPSILON: f64 = 1.0e-12;
const INTERSECTION_EPSILON: f64 = 1.0e-10;
const MAX_DRAWING_ABS_COORDINATE_MM: f64 = 1.0e12;
const MAX_DRAWING_INSTANCES: usize = 8_000;
const MAX_DRAWING_TRIANGLES: usize = 100_000;
const MAX_DRAWING_EDGES: usize = 300_000;
const MAX_DRAWING_OCCLUSION_TESTS: usize = 4_000_000;
const MAX_DRAWING_SPLITS: usize = 1_000_000;
const MAX_DRAWING_OUTPUT_LINES: usize = 1_000_000;
pub(crate) const MAX_DRAWING_VIEWS: usize = 32;
pub(crate) const MAX_DRAWING_DIMENSIONS: usize = 256;
pub(crate) const MAX_DRAWING_NOTES: usize = 256;
pub(crate) const MAX_DRAWING_BOM_BALLOONS: usize = 256;
const MAX_DRAWING_TEXT_BYTES: usize = 256;
const MAX_DRAWING_SCALE_TERM: u32 = 1_000_000;
const DRAWING_TITLE_BLOCK_HEIGHT_MM: f64 = 36.0;
const DRAWING_LAYOUT_GAP_MM: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingSheetId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingDimensionId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingNoteId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingDatumId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingFeatureControlFrameId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingBomBalloonId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingViewFrame {
    horizontal_bits: [u64; 3],
    vertical_bits: [u64; 3],
    depth_bits: [u64; 3],
}

impl DrawingViewFrame {
    pub fn new(direction: [f64; 3], up: [f64; 3]) -> Result<Self, DrawingError> {
        let depth = normalized(direction).ok_or(DrawingError::InvalidView)?;
        let horizontal = normalized(cross(up, depth)).ok_or(DrawingError::InvalidView)?;
        let vertical = normalized(cross(depth, horizontal)).ok_or(DrawingError::InvalidView)?;
        Ok(Self::from_axes(horizontal, vertical, depth))
    }

    fn from_axes(horizontal: [f64; 3], vertical: [f64; 3], depth: [f64; 3]) -> Self {
        Self {
            horizontal_bits: horizontal.map(canonical_view_component).map(f64::to_bits),
            vertical_bits: vertical.map(canonical_view_component).map(f64::to_bits),
            depth_bits: depth.map(canonical_view_component).map(f64::to_bits),
        }
    }

    pub(crate) fn from_persisted_axes(
        horizontal: [f64; 3],
        vertical: [f64; 3],
        depth: [f64; 3],
    ) -> Result<Self, DrawingError> {
        let axes = [horizontal, vertical, depth];
        if axes
            .iter()
            .flatten()
            .any(|component| !component.is_finite())
            || axes
                .iter()
                .any(|axis| (length_squared(*axis) - 1.0).abs() > 1.0e-12)
            || dot(horizontal, vertical).abs() > 1.0e-12
            || dot(horizontal, depth).abs() > 1.0e-12
            || dot(vertical, depth).abs() > 1.0e-12
            || subtract(cross(vertical, depth), horizontal)
                .into_iter()
                .any(|component| component.abs() > 1.0e-12)
        {
            return Err(DrawingError::InvalidView);
        }
        Ok(Self::from_axes(horizontal, vertical, depth))
    }

    #[must_use]
    pub fn horizontal(self) -> [f64; 3] {
        self.horizontal_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn vertical(self) -> [f64; 3] {
        self.vertical_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn direction(self) -> [f64; 3] {
        self.depth_bits.map(f64::from_bits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingSectionPlane {
    frame: DrawingViewFrame,
    depth_bits: u64,
}

impl DrawingSectionPlane {
    pub fn new(frame: DrawingViewFrame, depth_mm: f64) -> Result<Self, DrawingError> {
        if !depth_mm.is_finite() || depth_mm.abs() > MAX_DRAWING_ABS_COORDINATE_MM {
            return Err(DrawingError::InvalidView);
        }
        Ok(Self {
            frame,
            depth_bits: canonical_view_component(depth_mm).to_bits(),
        })
    }

    #[must_use]
    pub const fn frame(self) -> DrawingViewFrame {
        self.frame
    }

    #[must_use]
    pub fn depth_mm(self) -> f64 {
        f64::from_bits(self.depth_bits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingDetailRegion {
    frame: DrawingViewFrame,
    center_bits: [u64; 2],
    radius_bits: u64,
    magnification: DrawingScale,
}

impl DrawingDetailRegion {
    pub fn new(
        frame: DrawingViewFrame,
        center_mm: [f64; 2],
        radius_mm: f64,
        magnification: DrawingScale,
    ) -> Result<Self, DrawingError> {
        if center_mm
            .into_iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_DRAWING_ABS_COORDINATE_MM)
            || !radius_mm.is_finite()
            || radius_mm <= INTERSECTION_EPSILON
            || radius_mm > MAX_DRAWING_ABS_COORDINATE_MM
        {
            return Err(DrawingError::InvalidView);
        }
        Ok(Self {
            frame,
            center_bits: center_mm.map(canonical_view_component).map(f64::to_bits),
            radius_bits: radius_mm.to_bits(),
            magnification,
        })
    }

    #[must_use]
    pub const fn frame(self) -> DrawingViewFrame {
        self.frame
    }

    #[must_use]
    pub fn center_mm(self) -> [f64; 2] {
        self.center_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn radius_mm(self) -> f64 {
        f64::from_bits(self.radius_bits)
    }

    #[must_use]
    pub const fn magnification(self) -> DrawingScale {
        self.magnification
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrthographicViewKind {
    Front,
    Top,
    Right,
    Isometric,
    Auxiliary(DrawingViewFrame),
    Section(DrawingSectionPlane),
    Detail(DrawingDetailRegion),
}

impl OrthographicViewKind {
    const ALL: [Self; 3] = [Self::Front, Self::Top, Self::Right];

    pub fn auxiliary(direction: [f64; 3], up: [f64; 3]) -> Result<Self, DrawingError> {
        DrawingViewFrame::new(direction, up).map(Self::Auxiliary)
    }

    pub fn section(direction: [f64; 3], up: [f64; 3], depth_mm: f64) -> Result<Self, DrawingError> {
        let frame = DrawingViewFrame::new(direction, up)?;
        DrawingSectionPlane::new(frame, depth_mm).map(Self::Section)
    }

    pub fn detail(
        direction: [f64; 3],
        up: [f64; 3],
        center_mm: [f64; 2],
        radius_mm: f64,
        magnification: DrawingScale,
    ) -> Result<Self, DrawingError> {
        let frame = DrawingViewFrame::new(direction, up)?;
        DrawingDetailRegion::new(frame, center_mm, radius_mm, magnification).map(Self::Detail)
    }

    #[must_use]
    pub fn stable_name(self) -> String {
        match self {
            Self::Front => "front".to_owned(),
            Self::Top => "top".to_owned(),
            Self::Right => "right".to_owned(),
            Self::Isometric => "isometric".to_owned(),
            Self::Auxiliary(frame) => format!(
                "auxiliary-{:016x}{:016x}{:016x}-{:016x}{:016x}{:016x}",
                frame.depth_bits[0],
                frame.depth_bits[1],
                frame.depth_bits[2],
                frame.vertical_bits[0],
                frame.vertical_bits[1],
                frame.vertical_bits[2]
            ),
            Self::Section(section) => {
                let frame = section.frame;
                format!(
                    "section-{:016x}-{:016x}{:016x}{:016x}-{:016x}{:016x}{:016x}",
                    section.depth_bits,
                    frame.depth_bits[0],
                    frame.depth_bits[1],
                    frame.depth_bits[2],
                    frame.vertical_bits[0],
                    frame.vertical_bits[1],
                    frame.vertical_bits[2]
                )
            }
            Self::Detail(detail) => {
                let frame = detail.frame;
                format!(
                    "detail-{:016x}{:016x}-{:016x}-{}-{}-{:016x}{:016x}{:016x}-{:016x}{:016x}{:016x}",
                    detail.center_bits[0],
                    detail.center_bits[1],
                    detail.radius_bits,
                    detail.magnification.numerator(),
                    detail.magnification.denominator(),
                    frame.depth_bits[0],
                    frame.depth_bits[1],
                    frame.depth_bits[2],
                    frame.vertical_bits[0],
                    frame.vertical_bits[1],
                    frame.vertical_bits[2]
                )
            }
        }
    }

    fn frame(self) -> DrawingViewFrame {
        match self {
            Self::Front => {
                DrawingViewFrame::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0])
            }
            Self::Top => {
                DrawingViewFrame::from_axes([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
            }
            Self::Right => {
                DrawingViewFrame::from_axes([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0])
            }
            Self::Isometric => DrawingViewFrame::new([1.0, -1.0, 1.0], [-1.0, 1.0, 2.0])
                .expect("fixed isometric frame is valid"),
            Self::Auxiliary(frame) => frame,
            Self::Section(section) => section.frame(),
            Self::Detail(detail) => detail.frame(),
        }
    }

    fn section_depth_mm(self) -> Option<f64> {
        match self {
            Self::Section(section) => Some(section.depth_mm()),
            _ => None,
        }
    }

    fn detail_region(self) -> Option<DrawingDetailRegion> {
        match self {
            Self::Detail(detail) => Some(detail),
            _ => None,
        }
    }

    fn layout_scale_multiplier(self) -> f64 {
        self.detail_region()
            .map_or(1.0, |detail| detail.magnification().factor())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawingDimensionTolerance {
    #[default]
    None,
    Symmetric {
        deviation_bits: u64,
    },
    Bilateral {
        upper_bits: u64,
        lower_bits: u64,
    },
}

impl DrawingDimensionTolerance {
    pub fn symmetric(deviation_mm: f64) -> Result<Self, DrawingError> {
        validate_tolerance_value(deviation_mm, false)?;
        Ok(Self::Symmetric {
            deviation_bits: canonical_view_component(deviation_mm).to_bits(),
        })
    }

    pub fn bilateral(upper_mm: f64, lower_mm: f64) -> Result<Self, DrawingError> {
        validate_tolerance_value(upper_mm, true)?;
        validate_tolerance_value(lower_mm, true)?;
        if upper_mm <= INTERSECTION_EPSILON && lower_mm <= INTERSECTION_EPSILON {
            return Err(DrawingError::InvalidDimension);
        }
        Ok(Self::Bilateral {
            upper_bits: canonical_view_component(upper_mm).to_bits(),
            lower_bits: canonical_view_component(lower_mm).to_bits(),
        })
    }

    #[must_use]
    pub fn symmetric_deviation_mm(self) -> Option<f64> {
        match self {
            Self::Symmetric { deviation_bits } => Some(f64::from_bits(deviation_bits)),
            _ => None,
        }
    }

    #[must_use]
    pub fn bilateral_deviations_mm(self) -> Option<[f64; 2]> {
        match self {
            Self::Bilateral {
                upper_bits,
                lower_bits,
            } => Some([f64::from_bits(upper_bits), f64::from_bits(lower_bits)]),
            _ => None,
        }
    }

    fn validate(self) -> Result<(), DrawingError> {
        match self {
            Self::None => Ok(()),
            Self::Symmetric { deviation_bits } => {
                validate_tolerance_value(f64::from_bits(deviation_bits), false)
            }
            Self::Bilateral {
                upper_bits,
                lower_bits,
            } => {
                let upper = f64::from_bits(upper_bits);
                let lower = f64::from_bits(lower_bits);
                validate_tolerance_value(upper, true)?;
                validate_tolerance_value(lower, true)?;
                if upper <= INTERSECTION_EPSILON && lower <= INTERSECTION_EPSILON {
                    return Err(DrawingError::InvalidDimension);
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingLinearDimension {
    id: DrawingDimensionId,
    view_stable_name: String,
    source_line_id: String,
    offset_bits: u64,
    tolerance: DrawingDimensionTolerance,
}

impl DrawingLinearDimension {
    pub fn new(
        id: DrawingDimensionId,
        view: OrthographicViewKind,
        source_line_id: impl Into<String>,
        offset_page_mm: f64,
    ) -> Result<Self, DrawingError> {
        Self::with_tolerance(
            id,
            view,
            source_line_id,
            offset_page_mm,
            DrawingDimensionTolerance::None,
        )
    }

    pub fn with_tolerance(
        id: DrawingDimensionId,
        view: OrthographicViewKind,
        source_line_id: impl Into<String>,
        offset_page_mm: f64,
        tolerance: DrawingDimensionTolerance,
    ) -> Result<Self, DrawingError> {
        Self::from_persisted(
            id,
            view.stable_name(),
            source_line_id.into(),
            offset_page_mm,
            tolerance,
        )
    }

    pub(crate) fn from_persisted(
        id: DrawingDimensionId,
        view_stable_name: String,
        source_line_id: String,
        offset_page_mm: f64,
        tolerance: DrawingDimensionTolerance,
    ) -> Result<Self, DrawingError> {
        if id.0 == 0
            || view_stable_name.trim().is_empty()
            || !valid_drawing_text(&view_stable_name)
            || source_line_id.trim().is_empty()
            || !valid_drawing_text(&source_line_id)
            || !offset_page_mm.is_finite()
            || offset_page_mm.abs() <= INTERSECTION_EPSILON
            || offset_page_mm.abs() > MAX_DRAWING_ABS_COORDINATE_MM
        {
            return Err(DrawingError::InvalidDimension);
        }
        tolerance.validate()?;
        Ok(Self {
            id,
            view_stable_name,
            source_line_id,
            offset_bits: canonical_view_component(offset_page_mm).to_bits(),
            tolerance,
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingDimensionId {
        self.id
    }

    #[must_use]
    pub fn view_stable_name(&self) -> &str {
        &self.view_stable_name
    }

    #[must_use]
    pub fn source_line_id(&self) -> &str {
        &self.source_line_id
    }

    #[must_use]
    pub fn offset_page_mm(&self) -> f64 {
        f64::from_bits(self.offset_bits)
    }

    #[must_use]
    pub const fn tolerance(&self) -> DrawingDimensionTolerance {
        self.tolerance
    }

    #[must_use]
    pub const fn unit(&self) -> DrawingDimensionUnit {
        DrawingDimensionUnit::Millimetres
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingDimensionUnit {
    Millimetres,
    Degrees,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingAngularDimension {
    id: DrawingDimensionId,
    view_stable_name: String,
    source_line_ids: [String; 2],
    arc_radius_bits: u64,
    tolerance: DrawingDimensionTolerance,
}

impl DrawingAngularDimension {
    pub fn new(
        id: DrawingDimensionId,
        view: OrthographicViewKind,
        source_line_ids: [String; 2],
        arc_radius_page_mm: f64,
        tolerance_degrees: DrawingDimensionTolerance,
    ) -> Result<Self, DrawingError> {
        Self::from_persisted(
            id,
            view.stable_name(),
            source_line_ids,
            arc_radius_page_mm,
            tolerance_degrees,
        )
    }

    pub(crate) fn from_persisted(
        id: DrawingDimensionId,
        view_stable_name: String,
        source_line_ids: [String; 2],
        arc_radius_page_mm: f64,
        tolerance: DrawingDimensionTolerance,
    ) -> Result<Self, DrawingError> {
        if id.0 == 0
            || view_stable_name.trim().is_empty()
            || !valid_drawing_text(&view_stable_name)
            || source_line_ids[0] == source_line_ids[1]
            || source_line_ids
                .iter()
                .any(|source| source.trim().is_empty() || !valid_drawing_text(source))
            || !arc_radius_page_mm.is_finite()
            || arc_radius_page_mm <= INTERSECTION_EPSILON
            || arc_radius_page_mm > MAX_DRAWING_ABS_COORDINATE_MM
        {
            return Err(DrawingError::InvalidDimension);
        }
        tolerance.validate()?;
        let excessive_tolerance = match tolerance {
            DrawingDimensionTolerance::None => false,
            DrawingDimensionTolerance::Symmetric { deviation_bits } => {
                f64::from_bits(deviation_bits) >= 180.0
            }
            DrawingDimensionTolerance::Bilateral {
                upper_bits,
                lower_bits,
            } => f64::from_bits(upper_bits) >= 180.0 || f64::from_bits(lower_bits) >= 180.0,
        };
        if excessive_tolerance {
            return Err(DrawingError::InvalidDimension);
        }
        Ok(Self {
            id,
            view_stable_name,
            source_line_ids,
            arc_radius_bits: canonical_view_component(arc_radius_page_mm).to_bits(),
            tolerance,
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingDimensionId {
        self.id
    }

    #[must_use]
    pub fn view_stable_name(&self) -> &str {
        &self.view_stable_name
    }

    #[must_use]
    pub fn source_line_ids(&self) -> [&str; 2] {
        [&self.source_line_ids[0], &self.source_line_ids[1]]
    }

    #[must_use]
    pub fn arc_radius_page_mm(&self) -> f64 {
        f64::from_bits(self.arc_radius_bits)
    }

    #[must_use]
    pub const fn tolerance(&self) -> DrawingDimensionTolerance {
        self.tolerance
    }

    #[must_use]
    pub const fn unit(&self) -> DrawingDimensionUnit {
        DrawingDimensionUnit::Degrees
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingCircularDimensionKind {
    Radius,
    Diameter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingCircularDimension {
    id: DrawingDimensionId,
    view_stable_name: String,
    source_circle_id: String,
    leader_angle_bits: u64,
    offset_bits: u64,
    kind: DrawingCircularDimensionKind,
    tolerance: DrawingDimensionTolerance,
}

impl DrawingCircularDimension {
    pub fn new(
        id: DrawingDimensionId,
        view: OrthographicViewKind,
        source_circle_id: impl Into<String>,
        kind: DrawingCircularDimensionKind,
        leader_angle_degrees: f64,
        offset_page_mm: f64,
        tolerance: DrawingDimensionTolerance,
    ) -> Result<Self, DrawingError> {
        Self::from_persisted(
            id,
            view.stable_name(),
            source_circle_id.into(),
            kind,
            leader_angle_degrees,
            offset_page_mm,
            tolerance,
        )
    }

    pub(crate) fn from_persisted(
        id: DrawingDimensionId,
        view_stable_name: String,
        source_circle_id: String,
        kind: DrawingCircularDimensionKind,
        leader_angle_degrees: f64,
        offset_page_mm: f64,
        tolerance: DrawingDimensionTolerance,
    ) -> Result<Self, DrawingError> {
        if id.0 == 0
            || view_stable_name.trim().is_empty()
            || !valid_drawing_text(&view_stable_name)
            || source_circle_id.trim().is_empty()
            || !valid_drawing_text(&source_circle_id)
            || !leader_angle_degrees.is_finite()
            || !(0.0..360.0).contains(&leader_angle_degrees)
            || !offset_page_mm.is_finite()
            || offset_page_mm <= INTERSECTION_EPSILON
            || offset_page_mm > MAX_DRAWING_ABS_COORDINATE_MM
        {
            return Err(DrawingError::InvalidDimension);
        }
        tolerance.validate()?;
        Ok(Self {
            id,
            view_stable_name,
            source_circle_id,
            leader_angle_bits: canonical_view_component(leader_angle_degrees).to_bits(),
            offset_bits: canonical_view_component(offset_page_mm).to_bits(),
            kind,
            tolerance,
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingDimensionId {
        self.id
    }

    #[must_use]
    pub fn view_stable_name(&self) -> &str {
        &self.view_stable_name
    }

    #[must_use]
    pub fn source_circle_id(&self) -> &str {
        &self.source_circle_id
    }

    #[must_use]
    pub const fn kind(&self) -> DrawingCircularDimensionKind {
        self.kind
    }

    #[must_use]
    pub fn leader_angle_degrees(&self) -> f64 {
        f64::from_bits(self.leader_angle_bits)
    }

    #[must_use]
    pub fn offset_page_mm(&self) -> f64 {
        f64::from_bits(self.offset_bits)
    }

    #[must_use]
    pub const fn tolerance(&self) -> DrawingDimensionTolerance {
        self.tolerance
    }

    #[must_use]
    pub const fn unit(&self) -> DrawingDimensionUnit {
        DrawingDimensionUnit::Millimetres
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingMaterialCondition {
    None,
    MaximumMaterial,
    LeastMaterial,
    RegardlessOfFeatureSize,
}

impl DrawingMaterialCondition {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MaximumMaterial => "maximum-material",
            Self::LeastMaterial => "least-material",
            Self::RegardlessOfFeatureSize => "regardless-of-feature-size",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingGeometricCharacteristic {
    Straightness,
    Flatness,
    Circularity,
    Cylindricity,
    ProfileOfLine,
    ProfileOfSurface,
    Angularity,
    Perpendicularity,
    Parallelism,
    Position,
    Concentricity,
    Symmetry,
    CircularRunout,
    TotalRunout,
}

impl DrawingGeometricCharacteristic {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Straightness => "straightness",
            Self::Flatness => "flatness",
            Self::Circularity => "circularity",
            Self::Cylindricity => "cylindricity",
            Self::ProfileOfLine => "profile-of-line",
            Self::ProfileOfSurface => "profile-of-surface",
            Self::Angularity => "angularity",
            Self::Perpendicularity => "perpendicularity",
            Self::Parallelism => "parallelism",
            Self::Position => "position",
            Self::Concentricity => "concentricity",
            Self::Symmetry => "symmetry",
            Self::CircularRunout => "circular-runout",
            Self::TotalRunout => "total-runout",
        }
    }

    const fn permits_datum_references(self) -> bool {
        !matches!(
            self,
            Self::Straightness | Self::Flatness | Self::Circularity | Self::Cylindricity
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingDatumReference {
    label: String,
    material_condition: DrawingMaterialCondition,
}

impl DrawingDatumReference {
    pub fn new(
        label: impl Into<String>,
        material_condition: DrawingMaterialCondition,
    ) -> Result<Self, DrawingError> {
        let label = label.into();
        if !valid_datum_label(&label) {
            return Err(DrawingError::InvalidAnnotation);
        }
        Ok(Self {
            label,
            material_condition,
        })
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn material_condition(&self) -> DrawingMaterialCondition {
        self.material_condition
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingDatumSymbol {
    id: DrawingDatumId,
    view_stable_name: String,
    source_line_id: String,
    label: String,
    offset_bits: [u64; 2],
}

impl DrawingDatumSymbol {
    pub fn new(
        id: DrawingDatumId,
        view: OrthographicViewKind,
        source_line_id: impl Into<String>,
        label: impl Into<String>,
        offset_page_mm: [f64; 2],
    ) -> Result<Self, DrawingError> {
        Self::from_persisted(
            id,
            view.stable_name(),
            source_line_id.into(),
            label.into(),
            offset_page_mm,
        )
    }

    pub(crate) fn from_persisted(
        id: DrawingDatumId,
        view_stable_name: String,
        source_line_id: String,
        label: String,
        offset_page_mm: [f64; 2],
    ) -> Result<Self, DrawingError> {
        if id.0 == 0
            || view_stable_name.trim().is_empty()
            || !valid_drawing_text(&view_stable_name)
            || source_line_id.trim().is_empty()
            || !valid_drawing_text(&source_line_id)
            || !valid_datum_label(&label)
            || offset_page_mm
                .into_iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_DRAWING_ABS_COORDINATE_MM)
            || offset_page_mm
                .into_iter()
                .all(|value| value.abs() <= INTERSECTION_EPSILON)
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        Ok(Self {
            id,
            view_stable_name,
            source_line_id,
            label,
            offset_bits: offset_page_mm
                .map(canonical_view_component)
                .map(f64::to_bits),
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingDatumId {
        self.id
    }

    #[must_use]
    pub fn view_stable_name(&self) -> &str {
        &self.view_stable_name
    }

    #[must_use]
    pub fn source_line_id(&self) -> &str {
        &self.source_line_id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn offset_page_mm(&self) -> [f64; 2] {
        self.offset_bits.map(f64::from_bits)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingFeatureControlFrame {
    id: DrawingFeatureControlFrameId,
    view_stable_name: String,
    source_line_id: String,
    characteristic: DrawingGeometricCharacteristic,
    tolerance_bits: u64,
    diameter_zone: bool,
    material_condition: DrawingMaterialCondition,
    datum_references: Vec<DrawingDatumReference>,
    offset_bits: [u64; 2],
}

impl DrawingFeatureControlFrame {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: DrawingFeatureControlFrameId,
        view: OrthographicViewKind,
        source_line_id: impl Into<String>,
        characteristic: DrawingGeometricCharacteristic,
        tolerance_mm: f64,
        diameter_zone: bool,
        material_condition: DrawingMaterialCondition,
        datum_references: Vec<DrawingDatumReference>,
        offset_page_mm: [f64; 2],
    ) -> Result<Self, DrawingError> {
        Self::from_persisted(
            id,
            view.stable_name(),
            source_line_id.into(),
            characteristic,
            tolerance_mm,
            diameter_zone,
            material_condition,
            datum_references,
            offset_page_mm,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted(
        id: DrawingFeatureControlFrameId,
        view_stable_name: String,
        source_line_id: String,
        characteristic: DrawingGeometricCharacteristic,
        tolerance_mm: f64,
        diameter_zone: bool,
        material_condition: DrawingMaterialCondition,
        datum_references: Vec<DrawingDatumReference>,
        offset_page_mm: [f64; 2],
    ) -> Result<Self, DrawingError> {
        if id.0 == 0
            || view_stable_name.trim().is_empty()
            || !valid_drawing_text(&view_stable_name)
            || source_line_id.trim().is_empty()
            || !valid_drawing_text(&source_line_id)
            || !tolerance_mm.is_finite()
            || tolerance_mm <= INTERSECTION_EPSILON
            || tolerance_mm > MAX_DRAWING_ABS_COORDINATE_MM
            || datum_references.len() > 3
            || (!characteristic.permits_datum_references() && !datum_references.is_empty())
            || datum_references
                .iter()
                .map(DrawingDatumReference::label)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != datum_references.len()
            || offset_page_mm
                .into_iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_DRAWING_ABS_COORDINATE_MM)
            || offset_page_mm
                .into_iter()
                .all(|value| value.abs() <= INTERSECTION_EPSILON)
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        Ok(Self {
            id,
            view_stable_name,
            source_line_id,
            characteristic,
            tolerance_bits: canonical_view_component(tolerance_mm).to_bits(),
            diameter_zone,
            material_condition,
            datum_references,
            offset_bits: offset_page_mm
                .map(canonical_view_component)
                .map(f64::to_bits),
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingFeatureControlFrameId {
        self.id
    }

    #[must_use]
    pub fn view_stable_name(&self) -> &str {
        &self.view_stable_name
    }

    #[must_use]
    pub fn source_line_id(&self) -> &str {
        &self.source_line_id
    }

    #[must_use]
    pub const fn characteristic(&self) -> DrawingGeometricCharacteristic {
        self.characteristic
    }

    #[must_use]
    pub fn tolerance_mm(&self) -> f64 {
        f64::from_bits(self.tolerance_bits)
    }

    #[must_use]
    pub const fn diameter_zone(&self) -> bool {
        self.diameter_zone
    }

    #[must_use]
    pub const fn material_condition(&self) -> DrawingMaterialCondition {
        self.material_condition
    }

    #[must_use]
    pub fn datum_references(&self) -> &[DrawingDatumReference] {
        &self.datum_references
    }

    #[must_use]
    pub fn offset_page_mm(&self) -> [f64; 2] {
        self.offset_bits.map(f64::from_bits)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingBomBalloon {
    id: DrawingBomBalloonId,
    view_stable_name: String,
    instance_path: InstancePath,
    position: u32,
    offset_bits: [u64; 2],
}

impl DrawingBomBalloon {
    pub fn new(
        id: DrawingBomBalloonId,
        view: OrthographicViewKind,
        instance_path: InstancePath,
        position: u32,
        offset_page_mm: [f64; 2],
    ) -> Result<Self, DrawingError> {
        Self::from_persisted(
            id,
            view.stable_name(),
            instance_path,
            position,
            offset_page_mm,
        )
    }

    pub(crate) fn from_persisted(
        id: DrawingBomBalloonId,
        view_stable_name: String,
        instance_path: InstancePath,
        position: u32,
        offset_page_mm: [f64; 2],
    ) -> Result<Self, DrawingError> {
        if id.0 == 0
            || view_stable_name.trim().is_empty()
            || !valid_drawing_text(&view_stable_name)
            || instance_path.root_occurrence().0 == 0
            || position == 0
            || offset_page_mm
                .into_iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_DRAWING_ABS_COORDINATE_MM)
            || offset_page_mm
                .into_iter()
                .all(|value| value.abs() <= INTERSECTION_EPSILON)
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        Ok(Self {
            id,
            view_stable_name,
            instance_path,
            position,
            offset_bits: offset_page_mm
                .map(canonical_view_component)
                .map(f64::to_bits),
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingBomBalloonId {
        self.id
    }

    #[must_use]
    pub fn view_stable_name(&self) -> &str {
        &self.view_stable_name
    }

    #[must_use]
    pub const fn instance_path(&self) -> &InstancePath {
        &self.instance_path
    }

    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position
    }

    #[must_use]
    pub fn offset_page_mm(&self) -> [f64; 2] {
        self.offset_bits.map(f64::from_bits)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingNote {
    id: DrawingNoteId,
    position_bits: [u64; 2],
    text_template: String,
}

impl DrawingNote {
    pub fn new(
        id: DrawingNoteId,
        position_page_mm: [f64; 2],
        text_template: impl Into<String>,
    ) -> Result<Self, DrawingError> {
        let text_template = text_template.into();
        if id.0 == 0
            || position_page_mm.into_iter().any(|coordinate| {
                !coordinate.is_finite()
                    || !(0.0..=MAX_DRAWING_ABS_COORDINATE_MM).contains(&coordinate)
            })
            || text_template.trim().is_empty()
            || !valid_drawing_template(&text_template)
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        Ok(Self {
            id,
            position_bits: position_page_mm
                .map(canonical_view_component)
                .map(f64::to_bits),
            text_template,
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingNoteId {
        self.id
    }

    #[must_use]
    pub fn position_page_mm(&self) -> [f64; 2] {
        self.position_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn text_template(&self) -> &str {
        &self.text_template
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrawingAnnotations {
    linear_dimensions: Vec<DrawingLinearDimension>,
    angular_dimensions: Vec<DrawingAngularDimension>,
    circular_dimensions: Vec<DrawingCircularDimension>,
    datum_symbols: Vec<DrawingDatumSymbol>,
    feature_control_frames: Vec<DrawingFeatureControlFrame>,
    bom_balloons: Vec<DrawingBomBalloon>,
    notes: Vec<DrawingNote>,
}

impl DrawingAnnotations {
    #[must_use]
    pub fn new(linear_dimensions: Vec<DrawingLinearDimension>, notes: Vec<DrawingNote>) -> Self {
        Self::with_typed_dimensions(linear_dimensions, Vec::new(), Vec::new(), notes)
    }

    #[must_use]
    pub fn with_typed_dimensions(
        linear_dimensions: Vec<DrawingLinearDimension>,
        angular_dimensions: Vec<DrawingAngularDimension>,
        circular_dimensions: Vec<DrawingCircularDimension>,
        notes: Vec<DrawingNote>,
    ) -> Self {
        Self::with_manufacturing_annotations(
            linear_dimensions,
            angular_dimensions,
            circular_dimensions,
            Vec::new(),
            Vec::new(),
            notes,
        )
    }

    #[must_use]
    pub fn with_manufacturing_annotations(
        linear_dimensions: Vec<DrawingLinearDimension>,
        angular_dimensions: Vec<DrawingAngularDimension>,
        circular_dimensions: Vec<DrawingCircularDimension>,
        datum_symbols: Vec<DrawingDatumSymbol>,
        feature_control_frames: Vec<DrawingFeatureControlFrame>,
        notes: Vec<DrawingNote>,
    ) -> Self {
        Self::with_bom_annotations(
            linear_dimensions,
            angular_dimensions,
            circular_dimensions,
            datum_symbols,
            feature_control_frames,
            Vec::new(),
            notes,
        )
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn with_bom_annotations(
        linear_dimensions: Vec<DrawingLinearDimension>,
        angular_dimensions: Vec<DrawingAngularDimension>,
        circular_dimensions: Vec<DrawingCircularDimension>,
        datum_symbols: Vec<DrawingDatumSymbol>,
        feature_control_frames: Vec<DrawingFeatureControlFrame>,
        bom_balloons: Vec<DrawingBomBalloon>,
        notes: Vec<DrawingNote>,
    ) -> Self {
        Self {
            linear_dimensions,
            angular_dimensions,
            circular_dimensions,
            datum_symbols,
            feature_control_frames,
            bom_balloons,
            notes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingSource {
    Definition(DefinitionId),
    RigidAssembly { occurrence_ids: Vec<OccurrenceId> },
    RigidAssemblyInstances { instance_paths: Vec<InstancePath> },
}

impl DrawingSource {
    #[must_use]
    pub fn references_root_occurrence(&self, occurrence_id: OccurrenceId) -> bool {
        match self {
            Self::Definition(_) => false,
            Self::RigidAssembly { occurrence_ids } => occurrence_ids.contains(&occurrence_id),
            Self::RigidAssemblyInstances { instance_paths } => instance_paths
                .iter()
                .any(|path| path.root_occurrence() == occurrence_id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingPageSize {
    A0,
    A1,
    A2,
    A3,
    A4,
}

impl DrawingPageSize {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::A0 => "a0",
            Self::A1 => "a1",
            Self::A2 => "a2",
            Self::A3 => "a3",
            Self::A4 => "a4",
        }
    }

    #[must_use]
    pub const fn portrait_dimensions_mm(self) -> [u16; 2] {
        match self {
            Self::A0 => [841, 1189],
            Self::A1 => [594, 841],
            Self::A2 => [420, 594],
            Self::A3 => [297, 420],
            Self::A4 => [210, 297],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingPageOrientation {
    Portrait,
    Landscape,
}

impl DrawingPageOrientation {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingScale {
    numerator: u32,
    denominator: u32,
}

impl DrawingScale {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, DrawingError> {
        if numerator == 0
            || denominator == 0
            || numerator > MAX_DRAWING_SCALE_TERM
            || denominator > MAX_DRAWING_SCALE_TERM
        {
            return Err(DrawingError::InvalidScale);
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    fn factor(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }
}

impl Default for DrawingScale {
    fn default() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawingMargins {
    left_mm: u16,
    right_mm: u16,
    top_mm: u16,
    bottom_mm: u16,
}

impl DrawingMargins {
    #[must_use]
    pub const fn new(left_mm: u16, right_mm: u16, top_mm: u16, bottom_mm: u16) -> Self {
        Self {
            left_mm,
            right_mm,
            top_mm,
            bottom_mm,
        }
    }

    #[must_use]
    pub const fn values_mm(self) -> [u16; 4] {
        [self.left_mm, self.right_mm, self.top_mm, self.bottom_mm]
    }
}

impl Default for DrawingMargins {
    fn default() -> Self {
        Self::new(20, 10, 10, 10)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawingPageTemplate {
    size: DrawingPageSize,
    orientation: DrawingPageOrientation,
    scale: DrawingScale,
    margins: DrawingMargins,
}

impl DrawingPageTemplate {
    pub fn new(
        size: DrawingPageSize,
        orientation: DrawingPageOrientation,
        scale: DrawingScale,
        margins: DrawingMargins,
    ) -> Result<Self, DrawingError> {
        let template = Self {
            size,
            orientation,
            scale,
            margins,
        };
        template.validate()?;
        Ok(template)
    }

    #[must_use]
    pub const fn size(self) -> DrawingPageSize {
        self.size
    }

    #[must_use]
    pub const fn orientation(self) -> DrawingPageOrientation {
        self.orientation
    }

    #[must_use]
    pub const fn scale(self) -> DrawingScale {
        self.scale
    }

    #[must_use]
    pub const fn margins(self) -> DrawingMargins {
        self.margins
    }

    #[must_use]
    pub fn dimensions_mm(self) -> [u16; 2] {
        let [width, height] = self.size.portrait_dimensions_mm();
        match self.orientation {
            DrawingPageOrientation::Portrait => [width, height],
            DrawingPageOrientation::Landscape => [height, width],
        }
    }

    fn validate(self) -> Result<(), DrawingError> {
        let [width, height] = self.dimensions_mm();
        let [left, right, top, bottom] = self.margins.values_mm();
        if left == 0
            || right == 0
            || top == 0
            || bottom == 0
            || u32::from(left) + u32::from(right) + DRAWING_LAYOUT_GAP_MM as u32 >= u32::from(width)
            || u32::from(top)
                + u32::from(bottom)
                + DRAWING_TITLE_BLOCK_HEIGHT_MM as u32
                + 2 * DRAWING_LAYOUT_GAP_MM as u32
                >= u32::from(height)
        {
            return Err(DrawingError::InvalidPageTemplate);
        }
        Ok(())
    }
}

impl Default for DrawingPageTemplate {
    fn default() -> Self {
        Self {
            size: DrawingPageSize::A3,
            orientation: DrawingPageOrientation::Landscape,
            scale: DrawingScale::default(),
            margins: DrawingMargins::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingTitleBlock {
    title: String,
    drawing_number: String,
    revision: String,
    author: String,
    parametric: bool,
}

impl DrawingTitleBlock {
    pub fn new(
        title: impl Into<String>,
        drawing_number: impl Into<String>,
        revision: impl Into<String>,
        author: impl Into<String>,
    ) -> Result<Self, DrawingError> {
        let value = Self {
            title: title.into(),
            drawing_number: drawing_number.into(),
            revision: revision.into(),
            author: author.into(),
            parametric: false,
        };
        if value.title.trim().is_empty()
            || [
                &value.title,
                &value.drawing_number,
                &value.revision,
                &value.author,
            ]
            .into_iter()
            .any(|field| !valid_drawing_text(field))
        {
            return Err(DrawingError::InvalidTitleBlock);
        }
        Ok(value)
    }

    pub fn parametric(
        title: impl Into<String>,
        drawing_number: impl Into<String>,
        revision: impl Into<String>,
        author: impl Into<String>,
    ) -> Result<Self, DrawingError> {
        let mut value = Self::new(title, drawing_number, revision, author)?;
        if [
            &value.title,
            &value.drawing_number,
            &value.revision,
            &value.author,
        ]
        .into_iter()
        .any(|field| !valid_drawing_template(field))
        {
            return Err(DrawingError::InvalidTitleBlock);
        }
        value.parametric = true;
        Ok(value)
    }

    pub(crate) fn from_persisted(
        title: String,
        drawing_number: String,
        revision: String,
        author: String,
        parametric: bool,
    ) -> Result<Self, DrawingError> {
        if parametric {
            Self::parametric(title, drawing_number, revision, author)
        } else {
            Self::new(title, drawing_number, revision, author)
        }
    }

    #[must_use]
    pub const fn is_parametric(&self) -> bool {
        self.parametric
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn drawing_number(&self) -> &str {
        &self.drawing_number
    }

    #[must_use]
    pub fn revision(&self) -> &str {
        &self.revision
    }

    #[must_use]
    pub fn author(&self) -> &str {
        &self.author
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingSheet {
    schema: &'static str,
    id: DrawingSheetId,
    name: String,
    source: DrawingSource,
    page: DrawingPageTemplate,
    title_block: DrawingTitleBlock,
    views: Vec<OrthographicViewKind>,
    linear_dimensions: Vec<DrawingLinearDimension>,
    angular_dimensions: Vec<DrawingAngularDimension>,
    circular_dimensions: Vec<DrawingCircularDimension>,
    datum_symbols: Vec<DrawingDatumSymbol>,
    feature_control_frames: Vec<DrawingFeatureControlFrame>,
    bom_balloons: Vec<DrawingBomBalloon>,
    notes: Vec<DrawingNote>,
}

impl DrawingSheet {
    pub fn new(
        id: DrawingSheetId,
        name: impl Into<String>,
        source: DrawingSource,
    ) -> Result<Self, DrawingError> {
        let name = name.into();
        let default_title = if valid_drawing_text(&name) {
            name.clone()
        } else {
            "Untitled drawing".to_owned()
        };
        let title_block = DrawingTitleBlock::new(default_title, "", "", "")?;
        Self::with_contract(
            id,
            name,
            source,
            DrawingPageTemplate::default(),
            title_block,
        )
    }

    pub fn with_contract(
        id: DrawingSheetId,
        name: impl Into<String>,
        source: DrawingSource,
        page: DrawingPageTemplate,
        title_block: DrawingTitleBlock,
    ) -> Result<Self, DrawingError> {
        Self::with_contract_and_views(
            id,
            name,
            source,
            page,
            title_block,
            OrthographicViewKind::ALL.to_vec(),
        )
    }

    pub fn with_contract_and_views(
        id: DrawingSheetId,
        name: impl Into<String>,
        source: DrawingSource,
        page: DrawingPageTemplate,
        title_block: DrawingTitleBlock,
        views: Vec<OrthographicViewKind>,
    ) -> Result<Self, DrawingError> {
        Self::with_contract_views_and_dimensions(
            id,
            name,
            source,
            page,
            title_block,
            views,
            Vec::new(),
        )
    }

    pub fn with_contract_views_and_dimensions(
        id: DrawingSheetId,
        name: impl Into<String>,
        source: DrawingSource,
        page: DrawingPageTemplate,
        title_block: DrawingTitleBlock,
        views: Vec<OrthographicViewKind>,
        linear_dimensions: Vec<DrawingLinearDimension>,
    ) -> Result<Self, DrawingError> {
        Self::with_contract_views_and_annotations(
            id,
            name,
            source,
            page,
            title_block,
            views,
            DrawingAnnotations::new(linear_dimensions, Vec::new()),
        )
    }

    pub fn with_contract_views_and_annotations(
        id: DrawingSheetId,
        name: impl Into<String>,
        source: DrawingSource,
        page: DrawingPageTemplate,
        title_block: DrawingTitleBlock,
        views: Vec<OrthographicViewKind>,
        annotations: DrawingAnnotations,
    ) -> Result<Self, DrawingError> {
        let DrawingAnnotations {
            linear_dimensions,
            angular_dimensions,
            circular_dimensions,
            datum_symbols,
            feature_control_frames,
            bom_balloons,
            notes,
        } = annotations;
        let name = name.into();
        if id.0 == 0 || name.trim().is_empty() {
            return Err(DrawingError::InvalidSheet);
        }
        let invalid_assembly_source = match &source {
            DrawingSource::RigidAssembly { occurrence_ids } => {
                occurrence_ids.is_empty()
                    || occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1])
            }
            DrawingSource::RigidAssemblyInstances { instance_paths } => {
                instance_paths.is_empty()
                    || instance_paths.windows(2).any(|pair| pair[0] >= pair[1])
            }
            DrawingSource::Definition(_) => false,
        };
        if invalid_assembly_source {
            return Err(DrawingError::InvalidSheet);
        }
        if views.is_empty()
            || views.len() > MAX_DRAWING_VIEWS
            || views
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != views.len()
        {
            return Err(DrawingError::InvalidView);
        }
        let dimension_count =
            linear_dimensions.len() + angular_dimensions.len() + circular_dimensions.len();
        let dimension_ids = linear_dimensions
            .iter()
            .map(DrawingLinearDimension::id)
            .chain(angular_dimensions.iter().map(DrawingAngularDimension::id))
            .chain(circular_dimensions.iter().map(DrawingCircularDimension::id))
            .collect::<std::collections::BTreeSet<_>>();
        if dimension_count > MAX_DRAWING_DIMENSIONS
            || dimension_ids.len() != dimension_count
            || linear_dimensions
                .windows(2)
                .any(|pair| pair[0].id() >= pair[1].id())
            || angular_dimensions
                .windows(2)
                .any(|pair| pair[0].id() >= pair[1].id())
            || circular_dimensions
                .windows(2)
                .any(|pair| pair[0].id() >= pair[1].id())
            || linear_dimensions.iter().any(|dimension| {
                !views
                    .iter()
                    .any(|view| view.stable_name() == dimension.view_stable_name())
                    || !dimension.source_line_id().starts_with(&format!(
                        "sheet-{}/view-{}/",
                        id.0,
                        dimension.view_stable_name()
                    ))
            })
            || angular_dimensions.iter().any(|dimension| {
                !views
                    .iter()
                    .any(|view| view.stable_name() == dimension.view_stable_name())
                    || dimension.source_line_ids().iter().any(|source| {
                        !source.starts_with(&format!(
                            "sheet-{}/view-{}/",
                            id.0,
                            dimension.view_stable_name()
                        ))
                    })
            })
            || circular_dimensions.iter().any(|dimension| {
                !views
                    .iter()
                    .any(|view| view.stable_name() == dimension.view_stable_name())
                    || !dimension.source_circle_id().starts_with(&format!(
                        "sheet-{}/view-{}/",
                        id.0,
                        dimension.view_stable_name()
                    ))
            })
        {
            return Err(DrawingError::InvalidDimension);
        }
        let source_paths = match &source {
            DrawingSource::Definition(_) => None,
            DrawingSource::RigidAssembly { occurrence_ids } => Some(
                occurrence_ids
                    .iter()
                    .copied()
                    .map(InstancePath::root)
                    .collect::<Vec<_>>(),
            ),
            DrawingSource::RigidAssemblyInstances { instance_paths } => {
                Some(instance_paths.clone())
            }
        };
        let balloon_paths = bom_balloons
            .iter()
            .map(DrawingBomBalloon::instance_path)
            .collect::<std::collections::BTreeSet<_>>();
        if bom_balloons.len() > MAX_DRAWING_BOM_BALLOONS
            || bom_balloons
                .windows(2)
                .any(|pair| pair[0].id() >= pair[1].id())
            || balloon_paths.len() != bom_balloons.len()
            || bom_balloons.iter().any(|balloon| {
                balloon.position() as usize > MAX_DRAWING_BOM_BALLOONS
                    || !views
                        .iter()
                        .any(|view| view.stable_name() == balloon.view_stable_name())
                    || source_paths
                        .as_ref()
                        .is_none_or(|paths| paths.binary_search(balloon.instance_path()).is_err())
            })
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        let datum_labels = datum_symbols
            .iter()
            .map(DrawingDatumSymbol::label)
            .collect::<std::collections::BTreeSet<_>>();
        if datum_symbols.len() > MAX_DRAWING_NOTES
            || feature_control_frames.len() > MAX_DRAWING_NOTES
            || datum_symbols
                .windows(2)
                .any(|pair| pair[0].id() >= pair[1].id())
            || feature_control_frames
                .windows(2)
                .any(|pair| pair[0].id() >= pair[1].id())
            || datum_labels.len() != datum_symbols.len()
            || datum_symbols.iter().any(|datum| {
                !views
                    .iter()
                    .any(|view| view.stable_name() == datum.view_stable_name())
                    || !datum.source_line_id().starts_with(&format!(
                        "sheet-{}/view-{}/",
                        id.0,
                        datum.view_stable_name()
                    ))
            })
            || feature_control_frames.iter().any(|frame| {
                !views
                    .iter()
                    .any(|view| view.stable_name() == frame.view_stable_name())
                    || !frame.source_line_id().starts_with(&format!(
                        "sheet-{}/view-{}/",
                        id.0,
                        frame.view_stable_name()
                    ))
                    || frame
                        .datum_references()
                        .iter()
                        .any(|reference| !datum_labels.contains(reference.label()))
            })
            || notes.len() > MAX_DRAWING_NOTES
            || notes.windows(2).any(|pair| pair[0].id() >= pair[1].id())
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        page.validate()?;
        Ok(Self {
            schema: ORTHOGRAPHIC_DRAWING_SCHEMA_V2,
            id,
            name,
            source,
            page,
            title_block,
            views,
            linear_dimensions,
            angular_dimensions,
            circular_dimensions,
            datum_symbols,
            feature_control_frames,
            bom_balloons,
            notes,
        })
    }

    pub fn with_bom_balloons(
        &self,
        bom_balloons: Vec<DrawingBomBalloon>,
    ) -> Result<Self, DrawingError> {
        Self::with_contract_views_and_annotations(
            self.id,
            self.name.clone(),
            self.source.clone(),
            self.page,
            self.title_block.clone(),
            self.views.clone(),
            DrawingAnnotations::with_bom_annotations(
                self.linear_dimensions.clone(),
                self.angular_dimensions.clone(),
                self.circular_dimensions.clone(),
                self.datum_symbols.clone(),
                self.feature_control_frames.clone(),
                bom_balloons,
                self.notes.clone(),
            ),
        )
    }

    pub fn with_source(&self, source: DrawingSource) -> Result<Self, DrawingError> {
        Self::with_contract_views_and_annotations(
            self.id,
            self.name.clone(),
            source,
            self.page,
            self.title_block.clone(),
            self.views.clone(),
            DrawingAnnotations::with_bom_annotations(
                self.linear_dimensions.clone(),
                self.angular_dimensions.clone(),
                self.circular_dimensions.clone(),
                self.datum_symbols.clone(),
                self.feature_control_frames.clone(),
                self.bom_balloons.clone(),
                self.notes.clone(),
            ),
        )
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    #[must_use]
    pub const fn id(&self) -> DrawingSheetId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn source(&self) -> &DrawingSource {
        &self.source
    }

    #[must_use]
    pub const fn page(&self) -> DrawingPageTemplate {
        self.page
    }

    #[must_use]
    pub const fn title_block(&self) -> &DrawingTitleBlock {
        &self.title_block
    }

    #[must_use]
    pub fn views(&self) -> &[OrthographicViewKind] {
        &self.views
    }

    #[must_use]
    pub fn linear_dimensions(&self) -> &[DrawingLinearDimension] {
        &self.linear_dimensions
    }

    #[must_use]
    pub fn angular_dimensions(&self) -> &[DrawingAngularDimension] {
        &self.angular_dimensions
    }

    #[must_use]
    pub fn circular_dimensions(&self) -> &[DrawingCircularDimension] {
        &self.circular_dimensions
    }

    #[must_use]
    pub fn datum_symbols(&self) -> &[DrawingDatumSymbol] {
        &self.datum_symbols
    }

    #[must_use]
    pub fn feature_control_frames(&self) -> &[DrawingFeatureControlFrame] {
        &self.feature_control_frames
    }

    #[must_use]
    pub fn bom_balloons(&self) -> &[DrawingBomBalloon] {
        &self.bom_balloons
    }

    #[must_use]
    pub fn notes(&self) -> &[DrawingNote] {
        &self.notes
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedVisibleLine {
    pub stable_line_id: String,
    pub start_mm: [f64; 2],
    pub end_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedCircle {
    pub stable_circle_id: String,
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrthographicView {
    pub kind: OrthographicViewKind,
    pub stable_view_id: String,
    pub bounds_mm: [[f64; 2]; 2],
    pub visible_lines: Vec<ProjectedVisibleLine>,
    pub hidden_lines: Vec<ProjectedVisibleLine>,
    pub circles: Vec<ProjectedCircle>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingViewPlacement {
    pub kind: OrthographicViewKind,
    pub stable_view_id: String,
    pub origin_mm: [f64; 2],
    pub page_bounds_mm: [[f64; 2]; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingLinearDimensionLayout {
    pub stable_dimension_id: String,
    pub source_line_id: String,
    pub value_mm: f64,
    pub tolerance: DrawingDimensionTolerance,
    pub label: String,
    pub extension_lines_mm: [[[f64; 2]; 2]; 2],
    pub dimension_line_mm: [[f64; 2]; 2],
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingAngularDimensionLayout {
    pub stable_dimension_id: String,
    pub source_line_ids: [String; 2],
    pub value_degrees: f64,
    pub tolerance: DrawingDimensionTolerance,
    pub label: String,
    pub extension_lines_mm: [[[f64; 2]; 2]; 2],
    pub arc_points_mm: Vec<[f64; 2]>,
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingCircularDimensionLayout {
    pub stable_dimension_id: String,
    pub source_circle_id: String,
    pub kind: DrawingCircularDimensionKind,
    pub value_mm: f64,
    pub tolerance: DrawingDimensionTolerance,
    pub label: String,
    pub leader_line_mm: [[f64; 2]; 2],
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingDatumSymbolLayout {
    pub stable_datum_id: String,
    pub source_line_id: String,
    pub label: String,
    pub leader_line_mm: [[f64; 2]; 2],
    pub triangle_mm: [[f64; 2]; 3],
    pub frame_bounds_mm: [[f64; 2]; 2],
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingFeatureControlFrameLayout {
    pub stable_frame_id: String,
    pub source_line_id: String,
    pub characteristic: DrawingGeometricCharacteristic,
    pub tolerance_mm: f64,
    pub diameter_zone: bool,
    pub material_condition: DrawingMaterialCondition,
    pub datum_references: Vec<DrawingDatumReference>,
    pub label: String,
    pub leader_line_mm: [[f64; 2]; 2],
    pub frame_bounds_mm: [[f64; 2]; 2],
    pub separator_x_mm: Vec<f64>,
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingBomBalloonLayout {
    pub stable_balloon_id: String,
    pub instance_path: InstancePath,
    pub position: u32,
    pub label: String,
    pub leader_line_mm: [[f64; 2]; 2],
    pub circle_center_mm: [f64; 2],
    pub circle_radius_mm: f64,
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingBomRowLayout {
    pub position: u32,
    pub definition_id: DefinitionId,
    pub part_name: String,
    pub quantity: u32,
    pub label: String,
    pub text_position_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingNoteLayout {
    pub stable_note_id: String,
    pub position_mm: [f64; 2],
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingSheetLayout {
    pub schema: &'static str,
    pub page_size_mm: [f64; 2],
    pub border_bounds_mm: [[f64; 2]; 2],
    pub title_block_bounds_mm: [[f64; 2]; 2],
    pub view_placements: Vec<DrawingViewPlacement>,
    pub linear_dimensions: Vec<DrawingLinearDimensionLayout>,
    pub angular_dimensions: Vec<DrawingAngularDimensionLayout>,
    pub circular_dimensions: Vec<DrawingCircularDimensionLayout>,
    pub datum_symbols: Vec<DrawingDatumSymbolLayout>,
    pub feature_control_frames: Vec<DrawingFeatureControlFrameLayout>,
    pub bom_balloons: Vec<DrawingBomBalloonLayout>,
    pub bom_rows: Vec<DrawingBomRowLayout>,
    pub notes: Vec<DrawingNoteLayout>,
    pub title_block: DrawingTitleBlock,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrthographicDrawing {
    pub schema: &'static str,
    pub sheet_id: DrawingSheetId,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub stable_source_identity: String,
    pub result_digest: String,
    pub views: Vec<OrthographicView>,
    pub layout: DrawingSheetLayout,
}

impl OrthographicDrawing {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingError {
    InvalidSheet,
    InvalidPageTemplate,
    InvalidScale,
    InvalidTitleBlock,
    InvalidView,
    InvalidDimension,
    InvalidAnnotation,
    DimensionSourceLost,
    AnnotationSourceLost,
    LayoutOverflow,
    SourceLost,
    SourceStale,
    SourceFailed,
    SourceAmbiguous,
    SourceNotRigid,
    InvalidGeometry,
    ResourceLimit,
}

impl fmt::Display for DrawingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSheet => "drawing sheet identity or source is invalid",
            Self::InvalidPageTemplate => "drawing page template or border is invalid",
            Self::InvalidScale => "drawing scale is invalid",
            Self::InvalidTitleBlock => "drawing title block is invalid",
            Self::InvalidView => "drawing view direction or up vector is invalid",
            Self::InvalidDimension => "drawing dimension contract is invalid",
            Self::InvalidAnnotation => "drawing annotation contract is invalid",
            Self::DimensionSourceLost => "drawing dimension source line is lost",
            Self::AnnotationSourceLost => "drawing annotation source line is lost",
            Self::LayoutOverflow => {
                "drawing views or dimensions do not fit inside the selected sheet layout"
            }
            Self::SourceLost => "drawing source identity is lost",
            Self::SourceStale => "drawing source geometry is stale",
            Self::SourceFailed => "drawing source geometry failed or is unavailable",
            Self::SourceAmbiguous => "drawing source geometry is ambiguous",
            Self::SourceNotRigid => "drawing assembly source is not a resolved rigid component",
            Self::InvalidGeometry => "drawing source geometry is invalid",
            Self::ResourceLimit => "drawing projection exceeded its resource limit",
        })
    }
}

impl std::error::Error for DrawingError {}

#[derive(Debug)]
pub enum DrawingAuthoringError {
    Drawing(DrawingError),
    Proposal(ProposalPrepareError),
}

impl fmt::Display for DrawingAuthoringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drawing(error) => error.fmt(formatter),
            Self::Proposal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DrawingAuthoringError {}

impl From<DrawingError> for DrawingAuthoringError {
    fn from(error: DrawingError) -> Self {
        Self::Drawing(error)
    }
}

impl From<ProposalPrepareError> for DrawingAuthoringError {
    fn from(error: ProposalPrepareError) -> Self {
        Self::Proposal(error)
    }
}

pub fn prepare_create_drawing_sheet(
    document: &DocumentStore,
    results: &ExactResultRegistry,
    sheet: DrawingSheet,
) -> Result<(Proposal, OrthographicDrawing), DrawingAuthoringError> {
    project_orthographic_drawing(&document.current(), results, &sheet)?;
    let batch = CommandBatch::new(vec![CanonicalCommand::CreateDrawingSheet(sheet.clone())]);
    let candidate = document
        .preview_batch(&batch)
        .map_err(ProposalPrepareError::Canonical)?;
    let candidate_results = ExactResultRegistry::carried_forward(&candidate, results);
    let drawing = project_orthographic_drawing(&candidate, &candidate_results, &sheet)?;
    let proposal = document.prepare_proposal(batch)?;
    Ok((proposal, drawing))
}

pub fn prepare_edit_drawing_sheet(
    document: &DocumentStore,
    results: &ExactResultRegistry,
    sheet: DrawingSheet,
) -> Result<(Proposal, OrthographicDrawing), DrawingAuthoringError> {
    project_orthographic_drawing(&document.current(), results, &sheet)?;
    let batch = CommandBatch::new(vec![CanonicalCommand::UpdateDrawingSheet(sheet.clone())]);
    let candidate = document
        .preview_batch(&batch)
        .map_err(ProposalPrepareError::Canonical)?;
    let candidate_results = ExactResultRegistry::carried_forward(&candidate, results);
    let drawing = project_orthographic_drawing(&candidate, &candidate_results, &sheet)?;
    let proposal = document.prepare_proposal(batch)?;
    Ok((proposal, drawing))
}

pub fn prepare_delete_drawing_sheet(
    document: &DocumentStore,
    id: DrawingSheetId,
) -> Result<Proposal, ProposalPrepareError> {
    document.prepare_proposal(CommandBatch::new(vec![
        CanonicalCommand::DeleteDrawingSheet { id },
    ]))
}

pub fn project_orthographic_drawing(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    sheet: &DrawingSheet,
) -> Result<OrthographicDrawing, DrawingError> {
    validate_source(snapshot, sheet.source())?;
    let instances = source_instances(snapshot, results, sheet.source())?;
    let stable_source_identity = stable_source_identity(sheet.source(), &instances);
    let mut views = Vec::with_capacity(sheet.views().len());
    for &kind in sheet.views() {
        views.push(project_view(sheet.id(), kind, &instances)?);
    }
    let result_digest = drawing_result_digest(&stable_source_identity, &views);
    let layout = layout_drawing_sheet(snapshot, sheet, &views)?;
    Ok(OrthographicDrawing {
        schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
        sheet_id: sheet.id(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        stable_source_identity,
        result_digest,
        views,
        layout,
    })
}

pub(crate) fn validate_source(
    snapshot: &Snapshot,
    source: &DrawingSource,
) -> Result<(), DrawingError> {
    match source {
        DrawingSource::Definition(id) => {
            if snapshot.definition(*id).is_none() {
                return Err(DrawingError::SourceLost);
            }
        }
        DrawingSource::RigidAssembly { occurrence_ids } => {
            let instance_paths = occurrence_ids
                .iter()
                .copied()
                .map(InstancePath::root)
                .collect::<Vec<_>>();
            validate_rigid_source(snapshot, &instance_paths)?;
        }
        DrawingSource::RigidAssemblyInstances { instance_paths } => {
            validate_rigid_source(snapshot, instance_paths)?;
        }
    }
    Ok(())
}

fn validate_rigid_source(
    snapshot: &Snapshot,
    instance_paths: &[InstancePath],
) -> Result<(), DrawingError> {
    if instance_paths.len() > MAX_DRAWING_INSTANCES {
        return Err(DrawingError::ResourceLimit);
    }
    if instance_paths.is_empty()
        || instance_paths.windows(2).any(|pair| pair[0] >= pair[1])
        || instance_paths
            .iter()
            .any(|path| snapshot.resolve_instance_path(path).is_err())
    {
        return Err(DrawingError::SourceLost);
    }
    if !instance_paths
        .iter()
        .any(|path| snapshot.occurrence_is_grounded(path.root_occurrence()))
    {
        return Err(DrawingError::SourceNotRigid);
    }
    let mut has_internal_mate = false;
    for mate in snapshot.assembly_mates() {
        let a_in = instance_paths
            .binary_search(mate.endpoint_a().instance_path())
            .is_ok();
        let b_in = instance_paths
            .binary_search(mate.endpoint_b().instance_path())
            .is_ok();
        if a_in != b_in
            || ((a_in || b_in)
                && (mate.endpoint_a().health() != AssemblyReferenceHealth::Resolved
                    || mate.endpoint_b().health() != AssemblyReferenceHealth::Resolved))
        {
            return Err(DrawingError::SourceNotRigid);
        }
        has_internal_mate |= a_in && b_in;
    }
    if instance_paths
        .iter()
        .all(|path| snapshot.occurrence_is_grounded(path.root_occurrence()))
    {
        return Ok(());
    }
    if !has_internal_mate {
        return Err(DrawingError::SourceNotRigid);
    }
    let solved = solve_rigid_assembly(snapshot, AssemblySolverPolicy::default())
        .map_err(|_| DrawingError::SourceNotRigid)?;
    if solved.status() != AssemblySolveStatus::FullyConstrained
        || !solved.conflicting_mate_ids().is_empty()
        || !solved.maximum_residual().is_finite()
        || instance_paths.iter().any(|path| {
            solved
                .occurrence_at_path(path)
                .is_none_or(|occurrence| occurrence.remaining_dof() != 0)
        })
    {
        return Err(DrawingError::SourceNotRigid);
    }
    Ok(())
}

#[derive(Clone)]
struct DrawingInstance {
    token: String,
    transform: Transform,
    package: Arc<ExactBodyPackage>,
}

fn source_instances(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    source: &DrawingSource,
) -> Result<Vec<DrawingInstance>, DrawingError> {
    match source {
        DrawingSource::Definition(id) => Ok(vec![DrawingInstance {
            token: format!("definition-{}", id.0),
            transform: Transform::identity(),
            package: unique_current_package(snapshot, results, *id)?,
        }]),
        DrawingSource::RigidAssembly { occurrence_ids } => {
            let instance_paths = occurrence_ids
                .iter()
                .copied()
                .map(InstancePath::root)
                .collect::<Vec<_>>();
            drawing_instances_for_paths(snapshot, results, &instance_paths)
        }
        DrawingSource::RigidAssemblyInstances { instance_paths } => {
            drawing_instances_for_paths(snapshot, results, instance_paths)
        }
    }
}

fn drawing_instances_for_paths(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    instance_paths: &[InstancePath],
) -> Result<Vec<DrawingInstance>, DrawingError> {
    instance_paths
        .iter()
        .map(|path| {
            let resolved = snapshot
                .resolve_instance_path(path)
                .map_err(|_| DrawingError::SourceLost)?;
            Ok(DrawingInstance {
                token: format!("instance-{}", stable_instance_path(path)),
                transform: resolved.world_transform,
                package: unique_current_package(snapshot, results, resolved.definition_id)?,
            })
        })
        .collect()
}

fn unique_current_package(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    definition_id: DefinitionId,
) -> Result<Arc<ExactBodyPackage>, DrawingError> {
    let mut current = results
        .render_values(snapshot)
        .filter(|package| package.definition_id() == definition_id);
    let package = current.next();
    if current.next().is_some() {
        return Err(DrawingError::SourceAmbiguous);
    }
    if let Some(package) = package {
        return Ok(Arc::clone(package));
    }
    if results
        .values()
        .any(|package| package.definition_id() == definition_id)
    {
        Err(DrawingError::SourceStale)
    } else {
        Err(DrawingError::SourceFailed)
    }
}

fn stable_source_identity(source: &DrawingSource, instances: &[DrawingInstance]) -> String {
    let source_token = match source {
        DrawingSource::Definition(id) => format!("definition:{}", id.0),
        DrawingSource::RigidAssembly { occurrence_ids } => format!(
            "assembly:{}",
            occurrence_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
        DrawingSource::RigidAssemblyInstances { instance_paths } => format!(
            "assembly-paths:{}",
            instance_paths
                .iter()
                .map(stable_instance_path)
                .collect::<Vec<_>>()
                .join(",")
        ),
    };
    let geometry = instances
        .iter()
        .map(|instance| {
            let key = instance.package.result_key();
            format!(
                "{}:{}:{}",
                instance.token, key.definition_id.0, key.producer_feature_id.0
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    format!("{source_token}/{geometry}")
}

fn stable_instance_path(path: &InstancePath) -> String {
    let mut token = path.root_occurrence().0.to_string();
    for step in path.steps() {
        match step {
            InstancePathStep::Group(id) => token.push_str(&format!("/g{}", id.0)),
            InstancePathStep::Occurrence(id) => token.push_str(&format!("/o{}", id.0)),
        }
    }
    token
}

#[derive(Clone)]
struct EdgeEvidence {
    instance_index: usize,
    vertex_indices: [u32; 2],
    start: [f64; 3],
    end: [f64; 3],
    normals: Vec<[f64; 3]>,
}

#[derive(Clone, Copy)]
struct ProjectedTriangle {
    instance_index: usize,
    vertex_indices: [u32; 3],
    depths: [f64; 3],
    projected: [[f64; 2]; 3],
    bounds: [[f64; 2]; 2],
}

fn project_instance_circles(
    sheet_id: DrawingSheetId,
    kind: OrthographicViewKind,
    instance_token: &str,
    transform: Transform,
    edges: &[ExactBRepGraphEdgeEvidence],
) -> Result<Vec<ProjectedCircle>, DrawingError> {
    let frame = kind.frame();
    let horizontal = frame.horizontal();
    let vertical = frame.vertical();
    let depth = frame.direction();
    let circles = edges
        .iter()
        .filter_map(|edge| {
            let (Some(radius_mm), Some(center), Some(axis)) = (
                edge.circle_radius_mm,
                edge.axis_origin_mm,
                edge.unit_axis_direction,
            ) else {
                return None;
            };
            Some((edge, radius_mm, center, axis))
        })
        .filter_map(|(edge, radius_mm, center, axis)| {
            let world_axis = normalized(transform_vector(transform, axis))?;
            if (dot(world_axis, depth).abs() - 1.0).abs() > 1.0e-9 {
                return None;
            }
            let world_center = transform_point(transform, center);
            Some(ProjectedCircle {
                stable_circle_id: format!(
                    "sheet-{}/view-{}/{}:circle:{}",
                    sheet_id.0,
                    kind.stable_name(),
                    instance_token,
                    edge.edge_ordinal
                ),
                center_mm: [dot(world_center, horizontal), dot(world_center, vertical)],
                radius_mm,
            })
        })
        .collect();
    Ok(circles)
}

fn project_view(
    sheet_id: DrawingSheetId,
    kind: OrthographicViewKind,
    instances: &[DrawingInstance],
) -> Result<OrthographicView, DrawingError> {
    let frame = kind.frame();
    let horizontal = frame.horizontal();
    let vertical = frame.vertical();
    let depth = frame.direction();
    let section_depth_mm = kind.section_depth_mm();
    let mut circles = Vec::new();
    if section_depth_mm.is_none() {
        for instance in instances {
            circles.extend(project_instance_circles(
                sheet_id,
                kind,
                instance.token.as_str(),
                instance.transform,
                instance.package.edge_evidence(),
            )?);
        }
        circles.sort_by(|left, right| left.stable_circle_id.cmp(&right.stable_circle_id));
    }
    let mut edges = BTreeMap::<String, EdgeEvidence>::new();
    let mut projected_triangles = Vec::new();
    for (instance_index, instance) in instances.iter().enumerate() {
        for (triangle_index, triangle) in instance.package.triangles().iter().enumerate() {
            let indices = triangle.vertex_indices;
            let mut points = [[0.0; 3]; 3];
            for (offset, index) in indices.into_iter().enumerate() {
                let vertex = instance
                    .package
                    .vertices()
                    .get(index as usize)
                    .ok_or(DrawingError::InvalidGeometry)?;
                points[offset] = transform_point(instance.transform, vertex.position_mm);
            }
            if points
                .iter()
                .flatten()
                .any(|value| !value.is_finite() || value.abs() > MAX_DRAWING_ABS_COORDINATE_MM)
            {
                return Err(DrawingError::InvalidGeometry);
            }
            let normal = cross(
                subtract(points[1], points[0]),
                subtract(points[2], points[0]),
            );
            if length_squared(normal) <= VISIBILITY_EPSILON {
                continue;
            }
            let clipped_points = section_depth_mm.map_or_else(
                || points.to_vec(),
                |section_depth_mm| clip_triangle_to_depth(points, depth, section_depth_mm),
            );
            if clipped_points.len() >= 3 {
                let triangle_count = clipped_points.len() - 2;
                if projected_triangles
                    .len()
                    .checked_add(triangle_count)
                    .is_none_or(|count| count > MAX_DRAWING_TRIANGLES)
                {
                    return Err(DrawingError::ResourceLimit);
                }
                for offset in 1..clipped_points.len() - 1 {
                    let clipped = [
                        clipped_points[0],
                        clipped_points[offset],
                        clipped_points[offset + 1],
                    ];
                    let projected =
                        clipped.map(|point| [dot(point, horizontal), dot(point, vertical)]);
                    projected_triangles.push(ProjectedTriangle {
                        instance_index,
                        vertex_indices: indices,
                        depths: clipped.map(|point| dot(point, depth)),
                        projected,
                        bounds: projected_bounds(projected),
                    });
                }
            }
            for (left, right) in [(0, 1), (1, 2), (2, 0)] {
                let (first, second, start, end) = if indices[left] <= indices[right] {
                    (indices[left], indices[right], points[left], points[right])
                } else {
                    (indices[right], indices[left], points[right], points[left])
                };
                let Some([start, end]) = section_depth_mm.map_or(Some([start, end]), |cut| {
                    clip_segment_to_depth(start, end, depth, cut)
                }) else {
                    continue;
                };
                let key = format!("{}:{first}:{second}", instance.token);
                edges
                    .entry(key)
                    .and_modify(|evidence| evidence.normals.push(normal))
                    .or_insert(EdgeEvidence {
                        instance_index,
                        vertex_indices: [first, second],
                        start,
                        end,
                        normals: vec![normal],
                    });
                if edges.len() > MAX_DRAWING_EDGES {
                    return Err(DrawingError::ResourceLimit);
                }
            }
            if let Some(section_depth_mm) = section_depth_mm
                && let Some([start, end]) =
                    triangle_section_segment(points, depth, section_depth_mm)
            {
                let key = format!("{}/section:{triangle_index}", instance.token);
                edges.insert(
                    key,
                    EdgeEvidence {
                        instance_index,
                        vertex_indices: [indices[0], indices[1]],
                        start,
                        end,
                        normals: vec![normal],
                    },
                );
                if edges.len() > MAX_DRAWING_EDGES {
                    return Err(DrawingError::ResourceLimit);
                }
            }
        }
    }
    let candidate_edges = edges
        .into_iter()
        .filter(|(_, evidence)| {
            evidence.normals.len() == 1
                || evidence
                    .normals
                    .windows(2)
                    .any(|pair| length_squared(cross(pair[0], pair[1])) > VISIBILITY_EPSILON)
        })
        .collect::<Vec<_>>();
    if candidate_edges
        .len()
        .checked_mul(projected_triangles.len())
        .is_none_or(|count| count > MAX_DRAWING_OCCLUSION_TESTS)
    {
        return Err(DrawingError::ResourceLimit);
    }

    let mut visible_lines = Vec::new();
    let mut hidden_lines = Vec::new();
    let mut occlusion_tests = 0usize;
    let mut generated_splits = 0usize;
    let mut generated_output_lines = 0usize;
    for (edge, evidence) in candidate_edges {
        let base_id = format!("sheet-{}/view-{}/{}", sheet_id.0, kind.stable_name(), edge);
        let projected_edge = [
            [
                dot(evidence.start, horizontal),
                dot(evidence.start, vertical),
            ],
            [dot(evidence.end, horizontal), dot(evidence.end, vertical)],
        ];
        let projected_delta = subtract_2d(projected_edge[1], projected_edge[0]);
        let projected_length_squared =
            projected_delta[0] * projected_delta[0] + projected_delta[1] * projected_delta[1];
        if !projected_length_squared.is_finite() {
            return Err(DrawingError::InvalidGeometry);
        }
        if projected_length_squared <= VISIBILITY_EPSILON {
            continue;
        }
        generated_splits = generated_splits
            .checked_add(2)
            .ok_or(DrawingError::ResourceLimit)?;
        if generated_splits > MAX_DRAWING_SPLITS {
            return Err(DrawingError::ResourceLimit);
        }
        let mut splits = vec![0.0, 1.0];
        let edge_bounds = projected_bounds(projected_edge);
        for triangle in &projected_triangles {
            if triangle_is_incident(&evidence, *triangle) {
                continue;
            }
            occlusion_tests = occlusion_tests
                .checked_add(1)
                .ok_or(DrawingError::ResourceLimit)?;
            if occlusion_tests > MAX_DRAWING_OCCLUSION_TESTS {
                return Err(DrawingError::ResourceLimit);
            }
            if !bounds_overlap(edge_bounds, triangle.bounds) {
                continue;
            }
            for (left, right) in [(0, 1), (1, 2), (2, 0)] {
                if let Some(parameter) = segment_intersection_parameter(
                    projected_edge[0],
                    projected_edge[1],
                    triangle.projected[left],
                    triangle.projected[right],
                ) {
                    generated_splits = generated_splits
                        .checked_add(1)
                        .ok_or(DrawingError::ResourceLimit)?;
                    if generated_splits > MAX_DRAWING_SPLITS {
                        return Err(DrawingError::ResourceLimit);
                    }
                    splits.push(parameter);
                }
            }
        }
        splits.sort_by(f64::total_cmp);
        splits.dedup_by(|left, right| (*left - *right).abs() <= INTERSECTION_EPSILON);
        let boundary_splits = splits.clone();
        for triangle in &projected_triangles {
            if triangle_is_incident(&evidence, *triangle)
                || !bounds_overlap(edge_bounds, triangle.bounds)
            {
                continue;
            }
            for interval in boundary_splits.windows(2) {
                occlusion_tests = occlusion_tests
                    .checked_add(1)
                    .ok_or(DrawingError::ResourceLimit)?;
                if occlusion_tests > MAX_DRAWING_OCCLUSION_TESTS {
                    return Err(DrawingError::ResourceLimit);
                }
                let [start_parameter, end_parameter] = [interval[0], interval[1]];
                let midpoint_parameter = (start_parameter + end_parameter) * 0.5;
                let midpoint =
                    interpolate(projected_edge[0], projected_edge[1], midpoint_parameter);
                if triangle_depth_at(*triangle, midpoint).is_none() {
                    continue;
                }
                let start_point =
                    interpolate(projected_edge[0], projected_edge[1], start_parameter);
                let end_point = interpolate(projected_edge[0], projected_edge[1], end_parameter);
                let (Some(start_surface_depth), Some(end_surface_depth)) = (
                    triangle_depth_at(*triangle, start_point),
                    triangle_depth_at(*triangle, end_point),
                ) else {
                    continue;
                };
                let start_edge_depth = dot(evidence.start, depth) * (1.0 - start_parameter)
                    + dot(evidence.end, depth) * start_parameter;
                let end_edge_depth = dot(evidence.start, depth) * (1.0 - end_parameter)
                    + dot(evidence.end, depth) * end_parameter;
                let start_difference = start_edge_depth - start_surface_depth;
                let end_difference = end_edge_depth - end_surface_depth;
                if [start_difference, end_difference]
                    .into_iter()
                    .any(|difference| !difference.is_finite())
                {
                    return Err(DrawingError::InvalidGeometry);
                }
                let epsilon = depth_epsilon(start_edge_depth, end_edge_depth);
                if !((start_difference < -epsilon && end_difference > epsilon)
                    || (start_difference > epsilon && end_difference < -epsilon))
                {
                    continue;
                }
                let fraction = start_difference / (start_difference - end_difference);
                let parameter = start_parameter + (end_parameter - start_parameter) * fraction;
                if !parameter.is_finite()
                    || parameter <= start_parameter + INTERSECTION_EPSILON
                    || parameter >= end_parameter - INTERSECTION_EPSILON
                {
                    continue;
                }
                generated_splits = generated_splits
                    .checked_add(1)
                    .ok_or(DrawingError::ResourceLimit)?;
                if generated_splits > MAX_DRAWING_SPLITS {
                    return Err(DrawingError::ResourceLimit);
                }
                splits.push(parameter);
            }
        }
        splits.sort_by(f64::total_cmp);
        splits.dedup_by(|left, right| (*left - *right).abs() <= INTERSECTION_EPSILON);
        let mut segments = Vec::<(bool, f64, f64)>::new();
        for interval in splits.windows(2) {
            let [start_parameter, end_parameter] = [interval[0], interval[1]];
            if end_parameter - start_parameter <= INTERSECTION_EPSILON {
                continue;
            }
            let midpoint_parameter = (start_parameter + end_parameter) * 0.5;
            let midpoint = interpolate(projected_edge[0], projected_edge[1], midpoint_parameter);
            let edge_depth = dot(evidence.start, depth) * (1.0 - midpoint_parameter)
                + dot(evidence.end, depth) * midpoint_parameter;
            if !edge_depth.is_finite() {
                return Err(DrawingError::InvalidGeometry);
            }
            let mut hidden = false;
            for triangle in &projected_triangles {
                if triangle_is_incident(&evidence, *triangle) {
                    continue;
                }
                occlusion_tests = occlusion_tests
                    .checked_add(1)
                    .ok_or(DrawingError::ResourceLimit)?;
                if occlusion_tests > MAX_DRAWING_OCCLUSION_TESTS {
                    return Err(DrawingError::ResourceLimit);
                }
                let Some(surface_depth) = triangle_depth_at(*triangle, midpoint) else {
                    continue;
                };
                if !surface_depth.is_finite() {
                    return Err(DrawingError::InvalidGeometry);
                }
                let epsilon = depth_epsilon(surface_depth, edge_depth);
                hidden = surface_depth - epsilon > edge_depth;
                if hidden {
                    break;
                }
            }
            if let Some(previous) = segments.last_mut()
                && previous.0 == hidden
                && (previous.2 - start_parameter).abs() <= INTERSECTION_EPSILON
            {
                previous.2 = end_parameter;
            } else {
                segments.push((hidden, start_parameter, end_parameter));
            }
        }
        let segment_count = segments.len();
        for (ordinal, (hidden, start_parameter, end_parameter)) in segments.into_iter().enumerate()
        {
            let start_mm = interpolate(projected_edge[0], projected_edge[1], start_parameter);
            let end_mm = interpolate(projected_edge[0], projected_edge[1], end_parameter);
            if start_mm
                .into_iter()
                .chain(end_mm)
                .any(|coordinate| !coordinate.is_finite())
            {
                return Err(DrawingError::InvalidGeometry);
            }
            let delta = subtract_2d(end_mm, start_mm);
            let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
            if !length_squared.is_finite() {
                return Err(DrawingError::InvalidGeometry);
            }
            if length_squared <= VISIBILITY_EPSILON {
                continue;
            }
            generated_output_lines = generated_output_lines
                .checked_add(1)
                .ok_or(DrawingError::ResourceLimit)?;
            if generated_output_lines > MAX_DRAWING_OUTPUT_LINES {
                return Err(DrawingError::ResourceLimit);
            }
            let stable_line_id = if segment_count == 1 {
                base_id.clone()
            } else {
                format!("{base_id}/segment-{ordinal:06}")
            };
            let line = ProjectedVisibleLine {
                stable_line_id,
                start_mm,
                end_mm,
            };
            if hidden {
                hidden_lines.push(line);
            } else {
                visible_lines.push(line);
            }
        }
    }
    if let Some(detail) = kind.detail_region() {
        visible_lines = clip_lines_to_detail(visible_lines, detail)?;
        hidden_lines = clip_lines_to_detail(hidden_lines, detail)?;
    }
    visible_lines.sort_by(|left, right| left.stable_line_id.cmp(&right.stable_line_id));
    hidden_lines.sort_by(|left, right| left.stable_line_id.cmp(&right.stable_line_id));
    if visible_lines.is_empty() {
        return Err(DrawingError::SourceFailed);
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for line in visible_lines.iter().chain(&hidden_lines) {
        for point in [line.start_mm, line.end_mm] {
            min[0] = min[0].min(point[0]);
            min[1] = min[1].min(point[1]);
            max[0] = max[0].max(point[0]);
            max[1] = max[1].max(point[1]);
        }
    }
    Ok(OrthographicView {
        kind,
        stable_view_id: format!("sheet-{}/view-{}", sheet_id.0, kind.stable_name()),
        bounds_mm: [min, max],
        visible_lines,
        hidden_lines,
        circles,
    })
}

fn clip_lines_to_detail(
    lines: Vec<ProjectedVisibleLine>,
    detail: DrawingDetailRegion,
) -> Result<Vec<ProjectedVisibleLine>, DrawingError> {
    let center = detail.center_mm();
    let radius_squared = detail.radius_mm() * detail.radius_mm();
    let mut clipped = Vec::with_capacity(lines.len());
    for mut line in lines {
        let direction = subtract_2d(line.end_mm, line.start_mm);
        let offset = subtract_2d(line.start_mm, center);
        let a = dot_2d(direction, direction);
        let b = 2.0 * dot_2d(offset, direction);
        let c = dot_2d(offset, offset) - radius_squared;
        let discriminant = b * b - 4.0 * a * c;
        if [a, b, c, discriminant]
            .into_iter()
            .any(|value| !value.is_finite())
        {
            return Err(DrawingError::InvalidGeometry);
        }
        if a <= VISIBILITY_EPSILON || discriminant < 0.0 {
            continue;
        }
        let root = discriminant.max(0.0).sqrt();
        let first = (-b - root) / (2.0 * a);
        let second = (-b + root) / (2.0 * a);
        let start_parameter = first.max(0.0);
        let end_parameter = second.min(1.0);
        if end_parameter - start_parameter <= INTERSECTION_EPSILON {
            continue;
        }
        let [start_mm, end_mm] = [line.start_mm, line.end_mm];
        line.start_mm = interpolate(start_mm, end_mm, start_parameter);
        line.end_mm = interpolate(start_mm, end_mm, end_parameter);
        clipped.push(line);
    }
    Ok(clipped)
}

fn clip_triangle_to_depth(
    points: [[f64; 3]; 3],
    depth_axis: [f64; 3],
    maximum_depth: f64,
) -> Vec<[f64; 3]> {
    let mut clipped = Vec::with_capacity(4);
    let mut previous = points[2];
    let mut previous_depth = dot(previous, depth_axis);
    let mut previous_inside =
        previous_depth <= maximum_depth + depth_epsilon(previous_depth, maximum_depth);
    for current in points {
        let current_depth = dot(current, depth_axis);
        let current_inside =
            current_depth <= maximum_depth + depth_epsilon(current_depth, maximum_depth);
        if previous_inside != current_inside {
            let parameter = (maximum_depth - previous_depth) / (current_depth - previous_depth);
            clipped.push(interpolate_3d(previous, current, parameter.clamp(0.0, 1.0)));
        }
        if current_inside {
            clipped.push(current);
        }
        previous = current;
        previous_depth = current_depth;
        previous_inside = current_inside;
    }
    clipped.dedup_by(|left, right| same_point_3d(*left, *right));
    if clipped.len() > 1 && same_point_3d(clipped[0], clipped[clipped.len() - 1]) {
        clipped.pop();
    }
    clipped
}

fn clip_segment_to_depth(
    start: [f64; 3],
    end: [f64; 3],
    depth_axis: [f64; 3],
    maximum_depth: f64,
) -> Option<[[f64; 3]; 2]> {
    let start_depth = dot(start, depth_axis);
    let end_depth = dot(end, depth_axis);
    let start_inside = start_depth <= maximum_depth + depth_epsilon(start_depth, maximum_depth);
    let end_inside = end_depth <= maximum_depth + depth_epsilon(end_depth, maximum_depth);
    match (start_inside, end_inside) {
        (true, true) => Some([start, end]),
        (false, false) => None,
        (true, false) => {
            let parameter = (maximum_depth - start_depth) / (end_depth - start_depth);
            Some([start, interpolate_3d(start, end, parameter.clamp(0.0, 1.0))])
        }
        (false, true) => {
            let parameter = (maximum_depth - start_depth) / (end_depth - start_depth);
            Some([interpolate_3d(start, end, parameter.clamp(0.0, 1.0)), end])
        }
    }
}

fn triangle_section_segment(
    points: [[f64; 3]; 3],
    depth_axis: [f64; 3],
    section_depth: f64,
) -> Option<[[f64; 3]; 2]> {
    let mut intersections = Vec::with_capacity(3);
    for (left, right) in [(0, 1), (1, 2), (2, 0)] {
        let start = points[left];
        let end = points[right];
        let start_depth = dot(start, depth_axis);
        let end_depth = dot(end, depth_axis);
        let start_difference = start_depth - section_depth;
        let end_difference = end_depth - section_depth;
        let epsilon = depth_epsilon(start_depth, end_depth).max(depth_epsilon(section_depth, 0.0));
        if start_difference.abs() <= epsilon {
            push_unique_point(&mut intersections, start);
        }
        if (start_difference < -epsilon && end_difference > epsilon)
            || (start_difference > epsilon && end_difference < -epsilon)
        {
            let parameter = (section_depth - start_depth) / (end_depth - start_depth);
            push_unique_point(
                &mut intersections,
                interpolate_3d(start, end, parameter.clamp(0.0, 1.0)),
            );
        }
    }
    (intersections.len() == 2 && !same_point_3d(intersections[0], intersections[1]))
        .then(|| [intersections[0], intersections[1]])
}

fn push_unique_point(points: &mut Vec<[f64; 3]>, point: [f64; 3]) {
    if !points
        .iter()
        .any(|candidate| same_point_3d(*candidate, point))
    {
        points.push(point);
    }
}

fn same_point_3d(left: [f64; 3], right: [f64; 3]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| (left - right).abs() <= INTERSECTION_EPSILON)
}

fn interpolate_3d(start: [f64; 3], end: [f64; 3], parameter: f64) -> [f64; 3] {
    [
        start[0] * (1.0 - parameter) + end[0] * parameter,
        start[1] * (1.0 - parameter) + end[1] * parameter,
        start[2] * (1.0 - parameter) + end[2] * parameter,
    ]
}

fn triangle_is_incident(edge: &EdgeEvidence, triangle: ProjectedTriangle) -> bool {
    edge.instance_index == triangle.instance_index
        && edge
            .vertex_indices
            .iter()
            .all(|index| triangle.vertex_indices.contains(index))
}

fn projected_bounds<const N: usize>(points: [[f64; 2]; N]) -> [[f64; 2]; 2] {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for point in points {
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    [min, max]
}

fn bounds_overlap(left: [[f64; 2]; 2], right: [[f64; 2]; 2]) -> bool {
    (0..2).all(|axis| {
        left[0][axis] <= right[1][axis] + INTERSECTION_EPSILON
            && right[0][axis] <= left[1][axis] + INTERSECTION_EPSILON
    })
}

fn segment_intersection_parameter(
    start: [f64; 2],
    end: [f64; 2],
    other_start: [f64; 2],
    other_end: [f64; 2],
) -> Option<f64> {
    let direction = subtract_2d(end, start);
    let other_direction = subtract_2d(other_end, other_start);
    let denominator = cross_2d(direction, other_direction);
    if denominator.abs() <= INTERSECTION_EPSILON {
        return None;
    }
    let offset = subtract_2d(other_start, start);
    let parameter = cross_2d(offset, other_direction) / denominator;
    let other_parameter = cross_2d(offset, direction) / denominator;
    (parameter > INTERSECTION_EPSILON
        && parameter < 1.0 - INTERSECTION_EPSILON
        && (-INTERSECTION_EPSILON..=1.0 + INTERSECTION_EPSILON).contains(&other_parameter))
    .then_some(parameter)
}

fn triangle_depth_at(triangle: ProjectedTriangle, point: [f64; 2]) -> Option<f64> {
    if !bounds_overlap(
        [[point[0], point[1]], [point[0], point[1]]],
        triangle.bounds,
    ) {
        return None;
    }
    let [a, b, c] = triangle.projected;
    let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if denominator.abs() <= VISIBILITY_EPSILON {
        return None;
    }
    let first =
        ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / denominator;
    let second =
        ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / denominator;
    let third = 1.0 - first - second;
    if [first, second, third]
        .into_iter()
        .any(|weight| !(-INTERSECTION_EPSILON..=1.0 + INTERSECTION_EPSILON).contains(&weight))
    {
        return None;
    }
    Some(first * triangle.depths[0] + second * triangle.depths[1] + third * triangle.depths[2])
}

fn depth_epsilon(left: f64, right: f64) -> f64 {
    64.0 * f64::EPSILON * left.abs().max(right.abs()).max(1.0)
}

fn interpolate(start: [f64; 2], end: [f64; 2], parameter: f64) -> [f64; 2] {
    [
        start[0] * (1.0 - parameter) + end[0] * parameter,
        start[1] * (1.0 - parameter) + end[1] * parameter,
    ]
}

fn subtract_2d(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn cross_2d(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn dot_2d(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[0] + left[1] * right[1]
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn validate_tolerance_value(value_mm: f64, allow_zero: bool) -> Result<(), DrawingError> {
    if !value_mm.is_finite()
        || value_mm < 0.0
        || (!allow_zero && value_mm <= INTERSECTION_EPSILON)
        || value_mm > MAX_DRAWING_ABS_COORDINATE_MM
    {
        return Err(DrawingError::InvalidDimension);
    }
    Ok(())
}

fn valid_drawing_text(value: &str) -> bool {
    value.len() <= MAX_DRAWING_TEXT_BYTES && !value.chars().any(char::is_control)
}

fn valid_datum_label(value: &str) -> bool {
    (1..=3).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn valid_drawing_template(value: &str) -> bool {
    if !valid_drawing_text(value) {
        return false;
    }
    let literal = value
        .replace("{sheet_name}", "")
        .replace("{source_name}", "");
    !literal.contains(['{', '}'])
}

fn resolve_drawing_template(
    template: &str,
    sheet_name: &str,
    source_name: &str,
) -> Result<String, DrawingError> {
    if !valid_drawing_template(template) {
        return Err(DrawingError::InvalidAnnotation);
    }
    let resolved = template
        .replace("{sheet_name}", sheet_name)
        .replace("{source_name}", source_name);
    if !valid_drawing_text(&resolved) {
        return Err(DrawingError::InvalidAnnotation);
    }
    Ok(resolved)
}

fn drawing_source_name(
    snapshot: &Snapshot,
    source: &DrawingSource,
) -> Result<String, DrawingError> {
    let name = match source {
        DrawingSource::Definition(id) => snapshot
            .definition(*id)
            .ok_or(DrawingError::SourceLost)?
            .name()
            .to_owned(),
        DrawingSource::RigidAssembly { occurrence_ids } => occurrence_ids
            .iter()
            .map(|id| {
                snapshot
                    .occurrence(*id)
                    .map(|occurrence| occurrence.name().to_owned())
                    .ok_or(DrawingError::SourceLost)
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(", "),
        DrawingSource::RigidAssemblyInstances { instance_paths } => {
            let scene = snapshot.scene_query();
            instance_paths
                .iter()
                .map(|path| {
                    scene
                        .iter()
                        .find(|occurrence| occurrence.instance_path == *path)
                        .map(|occurrence| occurrence.occurrence_name.clone())
                        .ok_or(DrawingError::SourceLost)
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        }
    };
    if !valid_drawing_text(&name) {
        return Err(DrawingError::InvalidAnnotation);
    }
    Ok(name)
}

fn layout_drawing_sheet(
    snapshot: &Snapshot,
    sheet: &DrawingSheet,
    views: &[OrthographicView],
) -> Result<DrawingSheetLayout, DrawingError> {
    let page = sheet.page();
    let [page_width, page_height] = page.dimensions_mm().map(f64::from);
    let [left, right, top, bottom] = page.margins().values_mm().map(f64::from);
    let border_bounds_mm = [[left, bottom], [page_width - right, page_height - top]];
    let title_block_bounds_mm = [
        border_bounds_mm[0],
        [
            border_bounds_mm[1][0],
            border_bounds_mm[0][1] + DRAWING_TITLE_BLOCK_HEIGHT_MM,
        ],
    ];
    let drawing_min_y = title_block_bounds_mm[1][1] + DRAWING_LAYOUT_GAP_MM;
    let drawing_width = border_bounds_mm[1][0] - border_bounds_mm[0][0];
    let drawing_height = border_bounds_mm[1][1] - drawing_min_y;
    let (column_count, row_count) = if views.len() <= 4 {
        (2, 2)
    } else {
        let page_aspect = drawing_width / drawing_height;
        let columns =
            ((views.len() as f64 * page_aspect).sqrt().ceil() as usize).clamp(1, views.len());
        (columns, views.len().div_ceil(columns))
    };
    let cell_width = (drawing_width
        - DRAWING_LAYOUT_GAP_MM * column_count.saturating_sub(1) as f64)
        / column_count as f64;
    let cell_height = (drawing_height - DRAWING_LAYOUT_GAP_MM * row_count.saturating_sub(1) as f64)
        / row_count as f64;
    if cell_width <= 0.0 || cell_height <= 0.0 {
        return Err(DrawingError::InvalidPageTemplate);
    }
    let mut view_placements = Vec::with_capacity(views.len());
    let mut view_cell_bounds = Vec::with_capacity(views.len());
    for (index, view) in views.iter().enumerate() {
        let scale = page.scale().factor() * view.kind.layout_scale_multiplier();
        let (column, row) = ((index / row_count) as f64, (index % row_count) as f64);
        let cell_min = [
            border_bounds_mm[0][0] + column * (cell_width + DRAWING_LAYOUT_GAP_MM),
            drawing_min_y + row * (cell_height + DRAWING_LAYOUT_GAP_MM),
        ];
        let cell_max = [cell_min[0] + cell_width, cell_min[1] + cell_height];
        let scaled_size = [
            (view.bounds_mm[1][0] - view.bounds_mm[0][0]) * scale,
            (view.bounds_mm[1][1] - view.bounds_mm[0][1]) * scale,
        ];
        if scaled_size
            .into_iter()
            .any(|value| !value.is_finite() || value <= 0.0)
            || scaled_size[0] > cell_width
            || scaled_size[1] > cell_height
        {
            return Err(DrawingError::LayoutOverflow);
        }
        let origin_mm = [
            (cell_min[0] + cell_max[0] - scaled_size[0]) * 0.5 - view.bounds_mm[0][0] * scale,
            (cell_min[1] + cell_max[1] - scaled_size[1]) * 0.5 - view.bounds_mm[0][1] * scale,
        ];
        let page_bounds_mm = [
            [
                origin_mm[0] + view.bounds_mm[0][0] * scale,
                origin_mm[1] + view.bounds_mm[0][1] * scale,
            ],
            [
                origin_mm[0] + view.bounds_mm[1][0] * scale,
                origin_mm[1] + view.bounds_mm[1][1] * scale,
            ],
        ];
        view_cell_bounds.push([cell_min, cell_max]);
        view_placements.push(DrawingViewPlacement {
            kind: view.kind,
            stable_view_id: view.stable_view_id.clone(),
            origin_mm,
            page_bounds_mm,
        });
    }
    let linear_dimensions = layout_linear_dimensions(
        sheet,
        views,
        &view_placements,
        border_bounds_mm,
        drawing_min_y,
    )?;
    let angular_dimensions = layout_angular_dimensions(
        sheet,
        views,
        &view_placements,
        border_bounds_mm,
        drawing_min_y,
    )?;
    let circular_dimensions = layout_circular_dimensions(
        sheet,
        views,
        &view_placements,
        border_bounds_mm,
        drawing_min_y,
    )?;
    let datum_symbols = layout_datum_symbols(
        sheet,
        views,
        &view_placements,
        border_bounds_mm,
        drawing_min_y,
    )?;
    let feature_control_frames = layout_feature_control_frames(
        sheet,
        views,
        &view_placements,
        border_bounds_mm,
        drawing_min_y,
    )?;
    let (bom_balloons, bom_rows) = layout_bom_annotations(
        snapshot,
        sheet,
        views,
        &view_placements,
        &view_cell_bounds,
        border_bounds_mm,
        drawing_min_y,
    )?;
    let source_name = if sheet.title_block().is_parametric() || !sheet.notes().is_empty() {
        drawing_source_name(snapshot, sheet.source())?
    } else {
        String::new()
    };
    let title_block = if sheet.title_block().is_parametric() {
        DrawingTitleBlock::new(
            resolve_drawing_template(sheet.title_block().title(), sheet.name(), &source_name)?,
            resolve_drawing_template(
                sheet.title_block().drawing_number(),
                sheet.name(),
                &source_name,
            )?,
            resolve_drawing_template(sheet.title_block().revision(), sheet.name(), &source_name)?,
            resolve_drawing_template(sheet.title_block().author(), sheet.name(), &source_name)?,
        )?
    } else {
        sheet.title_block().clone()
    };
    let notes = sheet
        .notes()
        .iter()
        .map(|note| {
            let position_mm = note.position_page_mm();
            if position_mm[0] < border_bounds_mm[0][0] - INTERSECTION_EPSILON
                || position_mm[0] > border_bounds_mm[1][0] + INTERSECTION_EPSILON
                || position_mm[1] < drawing_min_y - INTERSECTION_EPSILON
                || position_mm[1] > border_bounds_mm[1][1] + INTERSECTION_EPSILON
            {
                return Err(DrawingError::LayoutOverflow);
            }
            Ok(DrawingNoteLayout {
                stable_note_id: format!("sheet-{}/note-{}", sheet.id().0, note.id().0),
                position_mm,
                text: resolve_drawing_template(note.text_template(), sheet.name(), &source_name)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let digest = drawing_layout_digest(
        page,
        &title_block,
        border_bounds_mm,
        title_block_bounds_mm,
        &view_placements,
        &linear_dimensions,
        &angular_dimensions,
        &circular_dimensions,
        &datum_symbols,
        &feature_control_frames,
        &bom_balloons,
        &bom_rows,
        &notes,
    );
    Ok(DrawingSheetLayout {
        schema: DRAWING_SHEET_LAYOUT_SCHEMA_V2,
        page_size_mm: [page_width, page_height],
        border_bounds_mm,
        title_block_bounds_mm,
        view_placements,
        linear_dimensions,
        angular_dimensions,
        circular_dimensions,
        datum_symbols,
        feature_control_frames,
        bom_balloons,
        bom_rows,
        notes,
        title_block,
        digest,
    })
}

fn layout_linear_dimensions(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<Vec<DrawingLinearDimensionLayout>, DrawingError> {
    let mut output = Vec::with_capacity(sheet.linear_dimensions().len());
    for dimension in sheet.linear_dimensions() {
        let view_index = views
            .iter()
            .position(|view| view.kind.stable_name() == dimension.view_stable_name())
            .ok_or(DrawingError::DimensionSourceLost)?;
        let view = &views[view_index];
        let placement = &placements[view_index];
        let mut matching_lines = view
            .visible_lines
            .iter()
            .chain(&view.hidden_lines)
            .filter(|line| line.stable_line_id == dimension.source_line_id());
        let source_line = matching_lines
            .next()
            .ok_or(DrawingError::DimensionSourceLost)?;
        if matching_lines.next().is_some() {
            return Err(DrawingError::DimensionSourceLost);
        }
        let source_delta = subtract_2d(source_line.end_mm, source_line.start_mm);
        let value_mm = dot_2d(source_delta, source_delta).sqrt();
        if !value_mm.is_finite() || value_mm <= INTERSECTION_EPSILON {
            return Err(DrawingError::InvalidGeometry);
        }
        let scale = sheet.page().scale().factor() * view.kind.layout_scale_multiplier();
        let page_points = [source_line.start_mm, source_line.end_mm].map(|point| {
            [
                placement.origin_mm[0] + point[0] * scale,
                placement.origin_mm[1] + point[1] * scale,
            ]
        });
        let page_delta = subtract_2d(page_points[1], page_points[0]);
        let page_length = dot_2d(page_delta, page_delta).sqrt();
        if !page_length.is_finite() || page_length <= INTERSECTION_EPSILON {
            return Err(DrawingError::InvalidGeometry);
        }
        let normal = [-page_delta[1] / page_length, page_delta[0] / page_length];
        let offset = dimension.offset_page_mm();
        let dimension_line_mm =
            page_points.map(|point| [point[0] + normal[0] * offset, point[1] + normal[1] * offset]);
        let text_position_mm = [
            (dimension_line_mm[0][0] + dimension_line_mm[1][0]) * 0.5,
            (dimension_line_mm[0][1] + dimension_line_mm[1][1]) * 0.5,
        ];
        let extension_lines_mm = [
            [page_points[0], dimension_line_mm[0]],
            [page_points[1], dimension_line_mm[1]],
        ];
        if extension_lines_mm
            .into_iter()
            .flatten()
            .chain(dimension_line_mm)
            .chain([text_position_mm])
            .flatten()
            .any(|coordinate| !coordinate.is_finite())
            || extension_lines_mm
                .into_iter()
                .flatten()
                .chain(dimension_line_mm)
                .chain([text_position_mm])
                .any(|point| {
                    point[0] < border_bounds_mm[0][0] - INTERSECTION_EPSILON
                        || point[0] > border_bounds_mm[1][0] + INTERSECTION_EPSILON
                        || point[1] < drawing_min_y - INTERSECTION_EPSILON
                        || point[1] > border_bounds_mm[1][1] + INTERSECTION_EPSILON
                })
        {
            return Err(DrawingError::LayoutOverflow);
        }
        output.push(DrawingLinearDimensionLayout {
            stable_dimension_id: format!("sheet-{}/dimension-{}", sheet.id().0, dimension.id().0),
            source_line_id: dimension.source_line_id().to_owned(),
            value_mm,
            tolerance: dimension.tolerance(),
            label: format_dimension_mm(value_mm, dimension.tolerance()),
            extension_lines_mm,
            dimension_line_mm,
            text_position_mm,
        });
    }
    Ok(output)
}

fn layout_angular_dimensions(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<Vec<DrawingAngularDimensionLayout>, DrawingError> {
    let mut output = Vec::with_capacity(sheet.angular_dimensions().len());
    for dimension in sheet.angular_dimensions() {
        let view_index = views
            .iter()
            .position(|view| view.kind.stable_name() == dimension.view_stable_name())
            .ok_or(DrawingError::DimensionSourceLost)?;
        let view = &views[view_index];
        let placement = &placements[view_index];
        let mut source_lines = Vec::with_capacity(2);
        for source_id in dimension.source_line_ids() {
            let mut matching = view
                .visible_lines
                .iter()
                .chain(&view.hidden_lines)
                .filter(|line| line.stable_line_id == source_id);
            let line = matching.next().ok_or(DrawingError::DimensionSourceLost)?;
            if matching.next().is_some() {
                return Err(DrawingError::DimensionSourceLost);
            }
            source_lines.push(line);
        }
        let directions = source_lines
            .iter()
            .map(|line| subtract_2d(line.end_mm, line.start_mm))
            .map(|direction| {
                let length = dot_2d(direction, direction).sqrt();
                if !length.is_finite() || length <= INTERSECTION_EPSILON {
                    return Err(DrawingError::InvalidGeometry);
                }
                Ok([direction[0] / length, direction[1] / length])
            })
            .collect::<Result<Vec<_>, _>>()?;
        let cross = directions[0][0] * directions[1][1] - directions[0][1] * directions[1][0];
        if cross.abs() <= INTERSECTION_EPSILON {
            return Err(DrawingError::InvalidGeometry);
        }
        let delta = subtract_2d(source_lines[1].start_mm, source_lines[0].start_mm);
        let parameter = (delta[0] * directions[1][1] - delta[1] * directions[1][0]) / cross;
        let center_model = [
            source_lines[0].start_mm[0] + directions[0][0] * parameter,
            source_lines[0].start_mm[1] + directions[0][1] * parameter,
        ];
        if center_model
            .into_iter()
            .any(|coordinate| !coordinate.is_finite())
        {
            return Err(DrawingError::InvalidGeometry);
        }
        let dot = dot_2d(directions[0], directions[1]).clamp(-1.0, 1.0);
        let signed_sweep = cross.atan2(dot);
        let (start_direction, sweep) = if signed_sweep >= 0.0 {
            (directions[0], signed_sweep)
        } else {
            (directions[1], -signed_sweep)
        };
        if sweep <= INTERSECTION_EPSILON || std::f64::consts::PI - sweep <= INTERSECTION_EPSILON {
            return Err(DrawingError::InvalidGeometry);
        }
        let scale = sheet.page().scale().factor() * view.kind.layout_scale_multiplier();
        let center = [
            placement.origin_mm[0] + center_model[0] * scale,
            placement.origin_mm[1] + center_model[1] * scale,
        ];
        let radius = dimension.arc_radius_page_mm();
        let start_angle = start_direction[1].atan2(start_direction[0]);
        let arc_points_mm = (0..=16)
            .map(|index| {
                let angle = start_angle + sweep * f64::from(index) / 16.0;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            })
            .collect::<Vec<_>>();
        let extension_lines_mm = [
            [center, arc_points_mm[0]],
            [
                center,
                *arc_points_mm.last().expect("angular arc has endpoints"),
            ],
        ];
        let text_angle = start_angle + sweep * 0.5;
        let text_position_mm = [
            center[0] + (radius + 3.0) * text_angle.cos(),
            center[1] + (radius + 3.0) * text_angle.sin(),
        ];
        if extension_lines_mm
            .iter()
            .flatten()
            .chain(&arc_points_mm)
            .chain([&text_position_mm])
            .any(|point| !drawing_page_contains(*point, border_bounds_mm, drawing_min_y))
        {
            return Err(DrawingError::LayoutOverflow);
        }
        let value_degrees = sweep.to_degrees();
        output.push(DrawingAngularDimensionLayout {
            stable_dimension_id: format!("sheet-{}/dimension-{}", sheet.id().0, dimension.id().0),
            source_line_ids: dimension.source_line_ids().map(str::to_owned),
            value_degrees,
            tolerance: dimension.tolerance(),
            label: format_dimension(
                value_degrees,
                dimension.tolerance(),
                DrawingDimensionUnit::Degrees,
            ),
            extension_lines_mm,
            arc_points_mm,
            text_position_mm,
        });
    }
    Ok(output)
}

fn layout_circular_dimensions(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<Vec<DrawingCircularDimensionLayout>, DrawingError> {
    let mut output = Vec::with_capacity(sheet.circular_dimensions().len());
    for dimension in sheet.circular_dimensions() {
        let view_index = views
            .iter()
            .position(|view| view.kind.stable_name() == dimension.view_stable_name())
            .ok_or(DrawingError::DimensionSourceLost)?;
        let view = &views[view_index];
        let placement = &placements[view_index];
        let mut matching = view
            .circles
            .iter()
            .filter(|circle| circle.stable_circle_id == dimension.source_circle_id());
        let circle = matching.next().ok_or(DrawingError::DimensionSourceLost)?;
        if matching.next().is_some() {
            return Err(DrawingError::DimensionSourceLost);
        }
        let scale = sheet.page().scale().factor() * view.kind.layout_scale_multiplier();
        let center = [
            placement.origin_mm[0] + circle.center_mm[0] * scale,
            placement.origin_mm[1] + circle.center_mm[1] * scale,
        ];
        let radius_page = circle.radius_mm * scale;
        let angle = dimension.leader_angle_degrees().to_radians();
        let direction = [angle.cos(), angle.sin()];
        let start_radius = match dimension.kind() {
            DrawingCircularDimensionKind::Radius => 0.0,
            DrawingCircularDimensionKind::Diameter => -radius_page,
        };
        let leader_line_mm = [
            [
                center[0] + direction[0] * start_radius,
                center[1] + direction[1] * start_radius,
            ],
            [
                center[0] + direction[0] * (radius_page + dimension.offset_page_mm()),
                center[1] + direction[1] * (radius_page + dimension.offset_page_mm()),
            ],
        ];
        let text_position_mm = [
            leader_line_mm[1][0] + direction[0] * 2.0,
            leader_line_mm[1][1] + direction[1] * 2.0,
        ];
        if leader_line_mm
            .iter()
            .chain([&text_position_mm])
            .any(|point| !drawing_page_contains(*point, border_bounds_mm, drawing_min_y))
        {
            return Err(DrawingError::LayoutOverflow);
        }
        let value_mm = match dimension.kind() {
            DrawingCircularDimensionKind::Radius => circle.radius_mm,
            DrawingCircularDimensionKind::Diameter => circle.radius_mm * 2.0,
        };
        let prefix = match dimension.kind() {
            DrawingCircularDimensionKind::Radius => "R",
            DrawingCircularDimensionKind::Diameter => "⌀",
        };
        output.push(DrawingCircularDimensionLayout {
            stable_dimension_id: format!("sheet-{}/dimension-{}", sheet.id().0, dimension.id().0),
            source_circle_id: dimension.source_circle_id().to_owned(),
            kind: dimension.kind(),
            value_mm,
            tolerance: dimension.tolerance(),
            label: format!(
                "{prefix}{}",
                format_dimension(
                    value_mm,
                    dimension.tolerance(),
                    DrawingDimensionUnit::Millimetres
                )
            ),
            leader_line_mm,
            text_position_mm,
        });
    }
    Ok(output)
}

fn layout_datum_symbols(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<Vec<DrawingDatumSymbolLayout>, DrawingError> {
    sheet
        .datum_symbols()
        .iter()
        .map(|datum| {
            let anchor = annotation_source_anchor(
                sheet,
                views,
                placements,
                datum.view_stable_name(),
                datum.source_line_id(),
            )?;
            let offset = datum.offset_page_mm();
            let length = dot_2d(offset, offset).sqrt();
            let direction = [offset[0] / length, offset[1] / length];
            let perpendicular = [-direction[1], direction[0]];
            let frame_center = [anchor[0] + offset[0], anchor[1] + offset[1]];
            let triangle_base = [
                anchor[0] + direction[0] * 3.0,
                anchor[1] + direction[1] * 3.0,
            ];
            let triangle_mm = [
                anchor,
                [
                    triangle_base[0] + perpendicular[0] * 2.0,
                    triangle_base[1] + perpendicular[1] * 2.0,
                ],
                [
                    triangle_base[0] - perpendicular[0] * 2.0,
                    triangle_base[1] - perpendicular[1] * 2.0,
                ],
            ];
            let frame_bounds_mm = [
                [frame_center[0] - 3.0, frame_center[1] - 3.0],
                [frame_center[0] + 3.0, frame_center[1] + 3.0],
            ];
            let leader_line_mm = [triangle_base, frame_center];
            let text_position_mm = [frame_center[0] - 1.2, frame_center[1] - 1.2];
            if triangle_mm
                .iter()
                .chain(leader_line_mm.iter())
                .chain(frame_bounds_mm.iter())
                .chain([&text_position_mm])
                .any(|point| !drawing_page_contains(*point, border_bounds_mm, drawing_min_y))
            {
                return Err(DrawingError::LayoutOverflow);
            }
            Ok(DrawingDatumSymbolLayout {
                stable_datum_id: format!("sheet-{}/datum-{}", sheet.id().0, datum.id().0),
                source_line_id: datum.source_line_id().to_owned(),
                label: datum.label().to_owned(),
                leader_line_mm,
                triangle_mm,
                frame_bounds_mm,
                text_position_mm,
            })
        })
        .collect()
}

fn layout_feature_control_frames(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<Vec<DrawingFeatureControlFrameLayout>, DrawingError> {
    sheet
        .feature_control_frames()
        .iter()
        .map(|frame| {
            let anchor = annotation_source_anchor(
                sheet,
                views,
                placements,
                frame.view_stable_name(),
                frame.source_line_id(),
            )?;
            let offset = frame.offset_page_mm();
            let origin = [anchor[0] + offset[0], anchor[1] + offset[1]];
            let tolerance = format_decimal_mm(frame.tolerance_mm());
            let zone = if frame.diameter_zone() { "DIA " } else { "" };
            let condition = match frame.material_condition() {
                DrawingMaterialCondition::None => "",
                DrawingMaterialCondition::MaximumMaterial => " MMC",
                DrawingMaterialCondition::LeastMaterial => " LMC",
                DrawingMaterialCondition::RegardlessOfFeatureSize => " RFS",
            };
            let mut cells = vec![
                frame.characteristic().stable_name().to_ascii_uppercase(),
                format!("{zone}{tolerance}{condition}"),
            ];
            cells.extend(frame.datum_references().iter().map(|reference| {
                let suffix = match reference.material_condition() {
                    DrawingMaterialCondition::None => "",
                    DrawingMaterialCondition::MaximumMaterial => " MMC",
                    DrawingMaterialCondition::LeastMaterial => " LMC",
                    DrawingMaterialCondition::RegardlessOfFeatureSize => " RFS",
                };
                format!("{}{suffix}", reference.label())
            }));
            let cell_widths = cells
                .iter()
                .map(|cell| (cell.chars().count() as f64 * 2.2 + 3.0).max(8.0))
                .collect::<Vec<_>>();
            let width = cell_widths.iter().sum::<f64>();
            let frame_bounds_mm = [origin, [origin[0] + width, origin[1] + 7.0]];
            let mut running_x = origin[0];
            let separator_x_mm = cell_widths
                .iter()
                .take(cell_widths.len().saturating_sub(1))
                .map(|width| {
                    running_x += width;
                    running_x
                })
                .collect::<Vec<_>>();
            let leader_line_mm = [anchor, [origin[0], origin[1] + 3.5]];
            let text_position_mm = [origin[0] + 1.5, origin[1] + 2.0];
            if leader_line_mm
                .iter()
                .chain(frame_bounds_mm.iter())
                .chain([&text_position_mm])
                .any(|point| !drawing_page_contains(*point, border_bounds_mm, drawing_min_y))
                || separator_x_mm.iter().any(|x| !x.is_finite())
            {
                return Err(DrawingError::LayoutOverflow);
            }
            Ok(DrawingFeatureControlFrameLayout {
                stable_frame_id: format!(
                    "sheet-{}/feature-control-frame-{}",
                    sheet.id().0,
                    frame.id().0
                ),
                source_line_id: frame.source_line_id().to_owned(),
                characteristic: frame.characteristic(),
                tolerance_mm: frame.tolerance_mm(),
                diameter_zone: frame.diameter_zone(),
                material_condition: frame.material_condition(),
                datum_references: frame.datum_references().to_vec(),
                label: cells.join(" | "),
                leader_line_mm,
                frame_bounds_mm,
                separator_x_mm,
                text_position_mm,
            })
        })
        .collect()
}

fn layout_bom_annotations(
    snapshot: &Snapshot,
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    view_cell_bounds: &[[[f64; 2]; 2]],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<(Vec<DrawingBomBalloonLayout>, Vec<DrawingBomRowLayout>), DrawingError> {
    if sheet.bom_balloons().is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let source_path_count = match sheet.source() {
        DrawingSource::Definition(_) => return Err(DrawingError::InvalidAnnotation),
        DrawingSource::RigidAssembly { occurrence_ids } => occurrence_ids.len(),
        DrawingSource::RigidAssemblyInstances { instance_paths } => instance_paths.len(),
    };
    if sheet.bom_balloons().len() != source_path_count {
        return Err(DrawingError::InvalidAnnotation);
    }

    let mut definition_positions = BTreeMap::<DefinitionId, u32>::new();
    let mut positions = BTreeMap::<u32, (DefinitionId, String, u32)>::new();
    let mut balloons = Vec::with_capacity(sheet.bom_balloons().len());
    for balloon in sheet.bom_balloons() {
        let resolved = snapshot
            .resolve_instance_path(balloon.instance_path())
            .map_err(|_| DrawingError::AnnotationSourceLost)?;
        if definition_positions
            .insert(resolved.definition_id, balloon.position())
            .is_some_and(|position| position != balloon.position())
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        let definition = snapshot
            .definition(resolved.definition_id)
            .ok_or(DrawingError::AnnotationSourceLost)?;
        match positions.entry(balloon.position()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((resolved.definition_id, definition.name().to_owned(), 1));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().0 != resolved.definition_id {
                    return Err(DrawingError::InvalidAnnotation);
                }
                entry.get_mut().2 = entry
                    .get()
                    .2
                    .checked_add(1)
                    .ok_or(DrawingError::ResourceLimit)?;
            }
        }

        let view_index = views
            .iter()
            .position(|view| view.kind.stable_name() == balloon.view_stable_name())
            .ok_or(DrawingError::AnnotationSourceLost)?;
        let view = &views[view_index];
        let prefix = format!(
            "sheet-{}/view-{}/instance-{}:",
            sheet.id().0,
            balloon.view_stable_name(),
            stable_instance_path(balloon.instance_path())
        );
        let source = view
            .visible_lines
            .iter()
            .chain(&view.hidden_lines)
            .find(|line| line.stable_line_id.starts_with(&prefix))
            .ok_or(DrawingError::AnnotationSourceLost)?;
        let scale = sheet.page().scale().factor() * view.kind.layout_scale_multiplier();
        let anchor_model = [
            (source.start_mm[0] + source.end_mm[0]) * 0.5,
            (source.start_mm[1] + source.end_mm[1]) * 0.5,
        ];
        let anchor = [
            placements[view_index].origin_mm[0] + anchor_model[0] * scale,
            placements[view_index].origin_mm[1] + anchor_model[1] * scale,
        ];
        let preferred_offset = balloon.offset_page_mm();
        let view_center = [
            (placements[view_index].page_bounds_mm[0][0]
                + placements[view_index].page_bounds_mm[1][0])
                * 0.5,
            (placements[view_index].page_bounds_mm[0][1]
                + placements[view_index].page_bounds_mm[1][1])
                * 0.5,
        ];
        let inward_offset = [
            preferred_offset[0]
                .abs()
                .copysign(view_center[0] - anchor[0]),
            preferred_offset[1]
                .abs()
                .copysign(view_center[1] - anchor[1]),
        ];
        let circle_radius_mm = 4.0;
        let cell_bounds = view_cell_bounds[view_index];
        let offset = [preferred_offset, inward_offset]
            .into_iter()
            .find(|candidate| {
                let center = [anchor[0] + candidate[0], anchor[1] + candidate[1]];
                [
                    [center[0] - circle_radius_mm, center[1] - circle_radius_mm],
                    [center[0] + circle_radius_mm, center[1] + circle_radius_mm],
                ]
                .into_iter()
                .all(|point| {
                    drawing_page_contains(point, border_bounds_mm, drawing_min_y)
                        && point[0] >= cell_bounds[0][0] - INTERSECTION_EPSILON
                        && point[0] <= cell_bounds[1][0] + INTERSECTION_EPSILON
                        && point[1] >= cell_bounds[0][1] - INTERSECTION_EPSILON
                        && point[1] <= cell_bounds[1][1] + INTERSECTION_EPSILON
                })
            })
            .ok_or(DrawingError::LayoutOverflow)?;
        let offset_length = dot_2d(offset, offset).sqrt();
        let direction = [offset[0] / offset_length, offset[1] / offset_length];
        let circle_center_mm = [anchor[0] + offset[0], anchor[1] + offset[1]];
        let leader_line_mm = [
            anchor,
            [
                circle_center_mm[0] - direction[0] * circle_radius_mm,
                circle_center_mm[1] - direction[1] * circle_radius_mm,
            ],
        ];
        let text_position_mm = [circle_center_mm[0] - 1.2, circle_center_mm[1] - 1.2];
        balloons.push(DrawingBomBalloonLayout {
            stable_balloon_id: format!("sheet-{}/balloon-{}", sheet.id().0, balloon.id().0),
            instance_path: balloon.instance_path().clone(),
            position: balloon.position(),
            label: balloon.position().to_string(),
            leader_line_mm,
            circle_center_mm,
            circle_radius_mm,
            text_position_mm,
        });
    }
    if positions.keys().copied().ne(1..=positions.len() as u32) {
        return Err(DrawingError::InvalidAnnotation);
    }

    let row_height = 4.5;
    let rows_per_column = ((border_bounds_mm[1][1] - drawing_min_y - 5.0) / row_height)
        .floor()
        .max(1.0) as usize;
    let column_width = 70.0;
    let mut rows = Vec::with_capacity(positions.len());
    for (index, (position, (definition_id, part_name, quantity))) in
        positions.into_iter().enumerate()
    {
        let column = index / rows_per_column;
        let row = index % rows_per_column;
        let text_position_mm = [
            border_bounds_mm[0][0] + 3.0 + column as f64 * column_width,
            border_bounds_mm[1][1] - 5.0 - row as f64 * row_height,
        ];
        if text_position_mm[0] + column_width > border_bounds_mm[1][0]
            || !drawing_page_contains(text_position_mm, border_bounds_mm, drawing_min_y)
        {
            return Err(DrawingError::LayoutOverflow);
        }
        rows.push(DrawingBomRowLayout {
            position,
            definition_id,
            part_name: part_name.clone(),
            quantity,
            label: format!("{position} | {part_name} | QTY {quantity}"),
            text_position_mm,
        });
    }
    Ok((balloons, rows))
}

fn annotation_source_anchor(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    view_stable_name: &str,
    source_line_id: &str,
) -> Result<[f64; 2], DrawingError> {
    let view_index = views
        .iter()
        .position(|view| view.kind.stable_name() == view_stable_name)
        .ok_or(DrawingError::AnnotationSourceLost)?;
    let view = &views[view_index];
    let mut matching = view
        .visible_lines
        .iter()
        .chain(&view.hidden_lines)
        .filter(|line| line.stable_line_id == source_line_id);
    let source = matching.next().ok_or(DrawingError::AnnotationSourceLost)?;
    if matching.next().is_some() {
        return Err(DrawingError::AnnotationSourceLost);
    }
    let scale = sheet.page().scale().factor() * view.kind.layout_scale_multiplier();
    let midpoint = [
        (source.start_mm[0] + source.end_mm[0]) * 0.5,
        (source.start_mm[1] + source.end_mm[1]) * 0.5,
    ];
    Ok([
        placements[view_index].origin_mm[0] + midpoint[0] * scale,
        placements[view_index].origin_mm[1] + midpoint[1] * scale,
    ])
}

fn drawing_page_contains(
    point: [f64; 2],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> bool {
    point.into_iter().all(f64::is_finite)
        && point[0] >= border_bounds_mm[0][0] - INTERSECTION_EPSILON
        && point[0] <= border_bounds_mm[1][0] + INTERSECTION_EPSILON
        && point[1] >= drawing_min_y - INTERSECTION_EPSILON
        && point[1] <= border_bounds_mm[1][1] + INTERSECTION_EPSILON
}

fn format_dimension_mm(value_mm: f64, tolerance: DrawingDimensionTolerance) -> String {
    format_dimension(value_mm, tolerance, DrawingDimensionUnit::Millimetres)
}

fn format_dimension(
    value: f64,
    tolerance: DrawingDimensionTolerance,
    unit: DrawingDimensionUnit,
) -> String {
    let nominal = format_decimal_mm(value);
    let suffix = match unit {
        DrawingDimensionUnit::Millimetres => " mm",
        DrawingDimensionUnit::Degrees => "°",
    };
    match tolerance {
        DrawingDimensionTolerance::None => format!("{nominal}{suffix}"),
        DrawingDimensionTolerance::Symmetric { deviation_bits } => format!(
            "{nominal} ±{}{suffix}",
            format_decimal_mm(f64::from_bits(deviation_bits))
        ),
        DrawingDimensionTolerance::Bilateral {
            upper_bits,
            lower_bits,
        } => format!(
            "{nominal} +{}/-{}{suffix}",
            format_decimal_mm(f64::from_bits(upper_bits)),
            format_decimal_mm(f64::from_bits(lower_bits))
        ),
    }
}

fn format_decimal_mm(value_mm: f64) -> String {
    let mut value = format!("{value_mm:.3}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value
}

pub(crate) fn drawing_layout_digest(
    page: DrawingPageTemplate,
    title_block: &DrawingTitleBlock,
    border_bounds_mm: [[f64; 2]; 2],
    title_block_bounds_mm: [[f64; 2]; 2],
    placements: &[DrawingViewPlacement],
    dimensions: &[DrawingLinearDimensionLayout],
    angular_dimensions: &[DrawingAngularDimensionLayout],
    circular_dimensions: &[DrawingCircularDimensionLayout],
    datum_symbols: &[DrawingDatumSymbolLayout],
    feature_control_frames: &[DrawingFeatureControlFrameLayout],
    bom_balloons: &[DrawingBomBalloonLayout],
    bom_rows: &[DrawingBomRowLayout],
    notes: &[DrawingNoteLayout],
) -> String {
    let mut digest = Sha256::new();
    push_digest(&mut digest, DRAWING_SHEET_LAYOUT_SCHEMA_V2.as_bytes());
    push_digest(&mut digest, page.size().stable_name().as_bytes());
    push_digest(&mut digest, page.orientation().stable_name().as_bytes());
    digest.update(page.scale().numerator().to_le_bytes());
    digest.update(page.scale().denominator().to_le_bytes());
    for margin in page.margins().values_mm() {
        digest.update(margin.to_le_bytes());
    }
    for field in [
        title_block.title(),
        title_block.drawing_number(),
        title_block.revision(),
        title_block.author(),
    ] {
        push_digest(&mut digest, field.as_bytes());
    }
    for coordinate in border_bounds_mm
        .into_iter()
        .flatten()
        .chain(title_block_bounds_mm.into_iter().flatten())
    {
        digest.update(coordinate.to_bits().to_le_bytes());
    }
    for placement in placements {
        push_digest(&mut digest, placement.kind.stable_name().as_bytes());
        push_digest(&mut digest, placement.stable_view_id.as_bytes());
        for coordinate in placement
            .origin_mm
            .into_iter()
            .chain(placement.page_bounds_mm.into_iter().flatten())
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for dimension in dimensions {
        push_digest(&mut digest, dimension.stable_dimension_id.as_bytes());
        push_digest(&mut digest, dimension.source_line_id.as_bytes());
        digest.update(dimension.value_mm.to_bits().to_le_bytes());
        push_digest(&mut digest, dimension.label.as_bytes());
        for coordinate in dimension
            .extension_lines_mm
            .into_iter()
            .flatten()
            .flatten()
            .chain(dimension.dimension_line_mm.into_iter().flatten())
            .chain(dimension.text_position_mm)
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for dimension in angular_dimensions {
        push_digest(&mut digest, dimension.stable_dimension_id.as_bytes());
        for source in &dimension.source_line_ids {
            push_digest(&mut digest, source.as_bytes());
        }
        digest.update(dimension.value_degrees.to_bits().to_le_bytes());
        push_digest(&mut digest, dimension.label.as_bytes());
        for coordinate in dimension
            .extension_lines_mm
            .iter()
            .flatten()
            .chain(&dimension.arc_points_mm)
            .chain([&dimension.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for dimension in circular_dimensions {
        push_digest(&mut digest, dimension.stable_dimension_id.as_bytes());
        push_digest(&mut digest, dimension.source_circle_id.as_bytes());
        digest.update(match dimension.kind {
            DrawingCircularDimensionKind::Radius => [1],
            DrawingCircularDimensionKind::Diameter => [2],
        });
        digest.update(dimension.value_mm.to_bits().to_le_bytes());
        push_digest(&mut digest, dimension.label.as_bytes());
        for coordinate in dimension
            .leader_line_mm
            .iter()
            .chain([&dimension.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for datum in datum_symbols {
        push_digest(&mut digest, datum.stable_datum_id.as_bytes());
        push_digest(&mut digest, datum.source_line_id.as_bytes());
        push_digest(&mut digest, datum.label.as_bytes());
        for coordinate in datum
            .leader_line_mm
            .iter()
            .chain(&datum.triangle_mm)
            .chain(datum.frame_bounds_mm.iter())
            .chain([&datum.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for frame in feature_control_frames {
        push_digest(&mut digest, frame.stable_frame_id.as_bytes());
        push_digest(&mut digest, frame.source_line_id.as_bytes());
        push_digest(&mut digest, frame.characteristic.stable_name().as_bytes());
        digest.update(frame.tolerance_mm.to_bits().to_le_bytes());
        digest.update([u8::from(frame.diameter_zone)]);
        push_digest(
            &mut digest,
            frame.material_condition.stable_name().as_bytes(),
        );
        for reference in &frame.datum_references {
            push_digest(&mut digest, reference.label().as_bytes());
            push_digest(
                &mut digest,
                reference.material_condition().stable_name().as_bytes(),
            );
        }
        push_digest(&mut digest, frame.label.as_bytes());
        for coordinate in frame
            .leader_line_mm
            .iter()
            .chain(frame.frame_bounds_mm.iter())
            .chain([&frame.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
        for separator in &frame.separator_x_mm {
            digest.update(separator.to_bits().to_le_bytes());
        }
    }
    for balloon in bom_balloons {
        push_digest(&mut digest, balloon.stable_balloon_id.as_bytes());
        push_digest(
            &mut digest,
            stable_instance_path(&balloon.instance_path).as_bytes(),
        );
        digest.update(balloon.position.to_le_bytes());
        push_digest(&mut digest, balloon.label.as_bytes());
        digest.update(balloon.circle_radius_mm.to_bits().to_le_bytes());
        for coordinate in balloon
            .leader_line_mm
            .iter()
            .chain([&balloon.circle_center_mm, &balloon.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for row in bom_rows {
        digest.update(row.position.to_le_bytes());
        digest.update(row.definition_id.0.to_le_bytes());
        push_digest(&mut digest, row.part_name.as_bytes());
        digest.update(row.quantity.to_le_bytes());
        push_digest(&mut digest, row.label.as_bytes());
        for coordinate in row.text_position_mm {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for note in notes {
        push_digest(&mut digest, note.stable_note_id.as_bytes());
        for coordinate in note.position_mm {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
        push_digest(&mut digest, note.text.as_bytes());
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn drawing_result_digest(
    stable_source_identity: &str,
    views: &[OrthographicView],
) -> String {
    let mut digest = Sha256::new();
    push_digest(&mut digest, ORTHOGRAPHIC_LINEWORK_SCHEMA_V2.as_bytes());
    push_digest(&mut digest, stable_source_identity.as_bytes());
    for view in views {
        push_digest(&mut digest, view.kind.stable_name().as_bytes());
        for bound in view.bounds_mm.iter().flatten() {
            digest.update(bound.to_bits().to_le_bytes());
        }
        for (classification, lines) in [
            (b"visible".as_slice(), view.visible_lines.as_slice()),
            (b"hidden".as_slice(), view.hidden_lines.as_slice()),
        ] {
            push_digest(&mut digest, classification);
            for line in lines {
                push_digest(&mut digest, line.stable_line_id.as_bytes());
                for coordinate in line.start_mm.into_iter().chain(line.end_mm) {
                    digest.update(coordinate.to_bits().to_le_bytes());
                }
            }
        }
        for circle in &view.circles {
            push_digest(&mut digest, circle.stable_circle_id.as_bytes());
            for coordinate in circle.center_mm {
                digest.update(coordinate.to_bits().to_le_bytes());
            }
            digest.update(circle.radius_mm.to_bits().to_le_bytes());
        }
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn push_digest(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

fn transform_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

fn transform_vector(transform: Transform, vector: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * vector[0] + matrix[1] * vector[1] + matrix[2] * vector[2],
        matrix[4] * vector[0] + matrix[5] * vector[1] + matrix[6] * vector[2],
        matrix[8] * vector[0] + matrix[9] * vector[1] + matrix[10] * vector[2],
    ]
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalized(value: [f64; 3]) -> Option<[f64; 3]> {
    if value.into_iter().any(|component| !component.is_finite()) {
        return None;
    }
    let squared = length_squared(value);
    if !squared.is_finite() || squared <= VISIBILITY_EPSILON {
        return None;
    }
    let length = squared.sqrt();
    Some(value.map(|component| component / length))
}

fn canonical_view_component(value: f64) -> f64 {
    if value.abs() <= VISIBILITY_EPSILON {
        0.0
    } else {
        value
    }
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn length_squared(value: [f64; 3]) -> f64 {
    value[0] * value[0] + value[1] * value[1] + value[2] * value[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_frames_are_orthonormal_and_invalid_auxiliary_frames_fail_closed() {
        for kind in [
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
            OrthographicViewKind::Isometric,
        ] {
            let frame = kind.frame();
            assert!((length_squared(frame.horizontal()) - 1.0).abs() <= 1.0e-12);
            assert!((length_squared(frame.vertical()) - 1.0).abs() <= 1.0e-12);
            assert!((length_squared(frame.direction()) - 1.0).abs() <= 1.0e-12);
            assert!(dot(frame.horizontal(), frame.vertical()).abs() <= 1.0e-12);
            assert!(dot(frame.horizontal(), frame.direction()).abs() <= 1.0e-12);
            assert!(dot(frame.vertical(), frame.direction()).abs() <= 1.0e-12);
        }
        assert_eq!(
            OrthographicViewKind::auxiliary([0.0; 3], [0.0, 1.0, 0.0]),
            Err(DrawingError::InvalidView)
        );
        assert_eq!(
            OrthographicViewKind::auxiliary([1.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            Err(DrawingError::InvalidView)
        );
        assert_eq!(
            OrthographicViewKind::auxiliary([f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Err(DrawingError::InvalidView)
        );
    }

    #[test]
    fn typed_angular_radius_and_diameter_dimensions_layout_export_and_tamper_fail_closed() {
        let view_kind = OrthographicViewKind::Front;
        let horizontal_id = "sheet-9/view-front/definition-1:edge:1".to_owned();
        let vertical_id = "sheet-9/view-front/definition-1:edge:2".to_owned();
        let circle_id = "sheet-9/view-front/definition-1:circle:7".to_owned();
        assert_eq!(
            DrawingCircularDimension::new(
                DrawingDimensionId(1),
                view_kind,
                circle_id.clone(),
                DrawingCircularDimensionKind::Radius,
                360.0,
                8.0,
                DrawingDimensionTolerance::None,
            ),
            Err(DrawingError::InvalidDimension)
        );
        assert_eq!(
            DrawingAngularDimension::new(
                DrawingDimensionId(1),
                view_kind,
                [horizontal_id.clone(), vertical_id.clone()],
                12.0,
                DrawingDimensionTolerance::symmetric(180.0).unwrap(),
            ),
            Err(DrawingError::InvalidDimension)
        );
        let sheet = DrawingSheet::with_contract_views_and_annotations(
            DrawingSheetId(9),
            "Typed dimensions",
            DrawingSource::Definition(DefinitionId(1)),
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("Typed dimensions", "TD-9", "A", "Kečup").unwrap(),
            vec![view_kind],
            DrawingAnnotations::with_typed_dimensions(
                Vec::new(),
                vec![
                    DrawingAngularDimension::new(
                        DrawingDimensionId(1),
                        view_kind,
                        [horizontal_id.clone(), vertical_id.clone()],
                        12.0,
                        DrawingDimensionTolerance::symmetric(0.5).unwrap(),
                    )
                    .unwrap(),
                ],
                vec![
                    DrawingCircularDimension::new(
                        DrawingDimensionId(2),
                        view_kind,
                        circle_id.clone(),
                        DrawingCircularDimensionKind::Radius,
                        45.0,
                        8.0,
                        DrawingDimensionTolerance::None,
                    )
                    .unwrap(),
                    DrawingCircularDimension::new(
                        DrawingDimensionId(3),
                        view_kind,
                        circle_id.clone(),
                        DrawingCircularDimensionKind::Diameter,
                        135.0,
                        8.0,
                        DrawingDimensionTolerance::bilateral(0.2, 0.1).unwrap(),
                    )
                    .unwrap(),
                ],
                Vec::new(),
            ),
        )
        .unwrap();
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Part".into(),
                },
                CanonicalCommand::CreateDrawingSheet(sheet),
            ]))
            .unwrap();
        let snapshot = document.current();
        let sheet = snapshot.drawing_sheet(DrawingSheetId(9)).unwrap();
        let views = vec![OrthographicView {
            kind: view_kind,
            stable_view_id: "sheet-9/view-front".into(),
            bounds_mm: [[-15.0, -15.0], [15.0, 15.0]],
            visible_lines: vec![
                ProjectedVisibleLine {
                    stable_line_id: horizontal_id,
                    start_mm: [-10.0, 0.0],
                    end_mm: [10.0, 0.0],
                },
                ProjectedVisibleLine {
                    stable_line_id: vertical_id,
                    start_mm: [0.0, -10.0],
                    end_mm: [0.0, 10.0],
                },
            ],
            hidden_lines: Vec::new(),
            circles: vec![ProjectedCircle {
                stable_circle_id: circle_id,
                center_mm: [0.0, 0.0],
                radius_mm: 5.0,
            }],
        }];
        let mut parallel_views = views.clone();
        parallel_views[0].visible_lines[1].end_mm = [10.0, -10.0];
        assert_eq!(
            layout_drawing_sheet(&snapshot, sheet, &parallel_views),
            Err(DrawingError::InvalidGeometry)
        );
        let mut missing_circle_views = views.clone();
        missing_circle_views[0].circles.clear();
        assert_eq!(
            layout_drawing_sheet(&snapshot, sheet, &missing_circle_views),
            Err(DrawingError::DimensionSourceLost)
        );
        let layout = layout_drawing_sheet(&snapshot, sheet, &views).unwrap();
        assert_eq!(layout.angular_dimensions[0].value_degrees, 90.0);
        assert_eq!(layout.angular_dimensions[0].label, "90 ±0.5°");
        assert_eq!(layout.circular_dimensions[0].label, "R5 mm");
        assert_eq!(layout.circular_dimensions[1].label, "⌀10 +0.2/-0.1 mm");
        assert_eq!(
            sheet.angular_dimensions()[0].unit(),
            DrawingDimensionUnit::Degrees
        );
        assert_eq!(
            sheet.circular_dimensions()[0].unit(),
            DrawingDimensionUnit::Millimetres
        );
        let stable_source_identity = "definition-1".to_owned();
        let drawing = OrthographicDrawing {
            schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
            sheet_id: sheet.id(),
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            result_digest: drawing_result_digest(&stable_source_identity, &views),
            stable_source_identity,
            views,
            layout,
        };
        let exported = crate::drawing_export::export_drawing(&snapshot, &drawing).unwrap();
        let svg = std::str::from_utf8(exported.svg()).unwrap();
        assert!(svg.contains("90 ±0.5°"));
        assert!(svg.contains("R5 mm"));
        assert!(svg.contains("⌀10 +0.2/-0.1 mm"));
        let mut tampered = drawing;
        tampered.layout.angular_dimensions[0].value_degrees = 91.0;
        assert_eq!(
            crate::drawing_export::export_drawing(&snapshot, &tampered),
            Err(crate::drawing_export::DrawingExportError::InvalidDrawing)
        );
    }

    #[test]
    fn typed_gdt_datums_layout_export_and_tamper_fail_closed() {
        let view = OrthographicViewKind::Front;
        let horizontal_id = "sheet-12/view-front/definition-1:edge:1".to_owned();
        let vertical_id = "sheet-12/view-front/definition-1:edge:2".to_owned();
        let datum_a = DrawingDatumReference::new("A", DrawingMaterialCondition::None).unwrap();
        assert_eq!(
            DrawingDatumReference::new("a", DrawingMaterialCondition::None),
            Err(DrawingError::InvalidAnnotation)
        );
        assert_eq!(
            DrawingFeatureControlFrame::new(
                DrawingFeatureControlFrameId(1),
                view,
                horizontal_id.clone(),
                DrawingGeometricCharacteristic::Flatness,
                0.1,
                false,
                DrawingMaterialCondition::None,
                vec![datum_a.clone()],
                [20.0, 20.0],
            ),
            Err(DrawingError::InvalidAnnotation)
        );
        let sheet = DrawingSheet::with_contract_views_and_annotations(
            DrawingSheetId(12),
            "GD&T sheet",
            DrawingSource::Definition(DefinitionId(1)),
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("GD&T sheet", "GDT-12", "A", "Kečup").unwrap(),
            vec![view],
            DrawingAnnotations::with_manufacturing_annotations(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![
                    DrawingDatumSymbol::new(
                        DrawingDatumId(1),
                        view,
                        horizontal_id.clone(),
                        "A",
                        [25.0, 20.0],
                    )
                    .unwrap(),
                    DrawingDatumSymbol::new(
                        DrawingDatumId(2),
                        view,
                        vertical_id.clone(),
                        "B",
                        [-25.0, 20.0],
                    )
                    .unwrap(),
                ],
                vec![
                    DrawingFeatureControlFrame::new(
                        DrawingFeatureControlFrameId(1),
                        view,
                        horizontal_id.clone(),
                        DrawingGeometricCharacteristic::Position,
                        0.1,
                        true,
                        DrawingMaterialCondition::MaximumMaterial,
                        vec![
                            datum_a,
                            DrawingDatumReference::new(
                                "B",
                                DrawingMaterialCondition::MaximumMaterial,
                            )
                            .unwrap(),
                        ],
                        [35.0, 35.0],
                    )
                    .unwrap(),
                ],
                Vec::new(),
            ),
        )
        .unwrap();
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Part".into(),
                },
                CanonicalCommand::CreateDrawingSheet(sheet),
            ]))
            .unwrap();
        let snapshot = document.current();
        let sheet = snapshot.drawing_sheet(DrawingSheetId(12)).unwrap();
        let views = vec![OrthographicView {
            kind: view,
            stable_view_id: "sheet-12/view-front".into(),
            bounds_mm: [[-15.0, -15.0], [15.0, 15.0]],
            visible_lines: vec![
                ProjectedVisibleLine {
                    stable_line_id: horizontal_id,
                    start_mm: [-10.0, 0.0],
                    end_mm: [10.0, 0.0],
                },
                ProjectedVisibleLine {
                    stable_line_id: vertical_id,
                    start_mm: [0.0, -10.0],
                    end_mm: [0.0, 10.0],
                },
            ],
            hidden_lines: Vec::new(),
            circles: Vec::new(),
        }];
        let layout = layout_drawing_sheet(&snapshot, sheet, &views).unwrap();
        assert_eq!(layout.datum_symbols[0].label, "A");
        assert_eq!(
            layout.feature_control_frames[0].label,
            "POSITION | DIA 0.1 MMC | A | B MMC"
        );
        let stable_source_identity = "definition-1".to_owned();
        let drawing = OrthographicDrawing {
            schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
            sheet_id: sheet.id(),
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            result_digest: drawing_result_digest(&stable_source_identity, &views),
            stable_source_identity,
            views: views.clone(),
            layout,
        };
        let exported = crate::drawing_export::export_drawing(&snapshot, &drawing).unwrap();
        let svg = std::str::from_utf8(exported.svg()).unwrap();
        assert!(svg.contains("sheet-12/datum-1/triangle-0"));
        assert!(svg.contains("POSITION | DIA 0.1 MMC | A | B MMC"));

        let mut missing_source_views = views;
        missing_source_views[0].visible_lines.clear();
        assert_eq!(
            layout_drawing_sheet(&snapshot, sheet, &missing_source_views),
            Err(DrawingError::AnnotationSourceLost)
        );
        let mut tampered = drawing;
        tampered.layout.feature_control_frames[0].tolerance_mm = 0.2;
        assert_eq!(
            crate::drawing_export::export_drawing(&snapshot, &tampered),
            Err(crate::drawing_export::DrawingExportError::InvalidDrawing)
        );
    }

    #[test]
    fn bom_balloon_flips_preferred_offset_inward_at_view_cell_boundary() {
        let path = InstancePath::root(OccurrenceId(1));
        let balloon = DrawingBomBalloon::new(
            DrawingBomBalloonId(1),
            OrthographicViewKind::Front,
            path.clone(),
            1,
            [8.0, 8.0],
        )
        .unwrap();
        let sheet = DrawingSheet::with_contract_views_and_annotations(
            DrawingSheetId(13),
            "Boundary BOM",
            DrawingSource::RigidAssemblyInstances {
                instance_paths: vec![path.clone()],
            },
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("Boundary BOM", "BOM-13", "A", "Kečup").unwrap(),
            vec![OrthographicViewKind::Front],
            DrawingAnnotations::with_bom_annotations(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![balloon],
                Vec::new(),
            ),
        )
        .unwrap();
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Boundary part".into(),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "Boundary instance".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let source_line_id = "sheet-13/view-front/instance-1:edge:1".to_owned();
        let views = vec![OrthographicView {
            kind: OrthographicViewKind::Front,
            stable_view_id: "sheet-13/view-front".into(),
            bounds_mm: [[0.0, 0.0], [185.0, 10.0]],
            visible_lines: vec![ProjectedVisibleLine {
                stable_line_id: source_line_id,
                start_mm: [185.0, 0.0],
                end_mm: [185.0, 10.0],
            }],
            hidden_lines: Vec::new(),
            circles: Vec::new(),
        }];

        let layout = layout_drawing_sheet(&snapshot, &sheet, &views).unwrap();
        let balloon = &layout.bom_balloons[0];
        assert!(balloon.circle_center_mm[0] < balloon.leader_line_mm[0][0]);
        assert_eq!(sheet.bom_balloons()[0].offset_page_mm(), [8.0, 8.0]);
    }

    #[test]
    fn exact_circle_evidence_projects_only_in_axis_normal_views() {
        let edge = ExactBRepGraphEdgeEvidence {
            edge_ordinal: 7,
            curve_kind: "circle".into(),
            length_mm: 31.415_926_535_897_93,
            centroid_mm: [2.0, 3.0, 4.0],
            bounds_mm: [[-3.0, 3.0, -1.0], [7.0, 3.0, 9.0]],
            closed: true,
            circle_radius_mm: Some(5.0),
            axis_origin_mm: Some([2.0, 3.0, 4.0]),
            unit_axis_direction: Some([0.0, -1.0, 0.0]),
            adjacent_face_ordinals: vec![1],
        };
        let transform = Transform::from_translation(10.0, 20.0, 30.0).unwrap();

        assert_eq!(
            project_instance_circles(
                DrawingSheetId(9),
                OrthographicViewKind::Front,
                "instance-4/g5/o6",
                transform,
                std::slice::from_ref(&edge),
            )
            .unwrap(),
            vec![ProjectedCircle {
                stable_circle_id: "sheet-9/view-front/instance-4/g5/o6:circle:7".into(),
                center_mm: [12.0, 34.0],
                radius_mm: 5.0,
            }]
        );
        assert!(
            project_instance_circles(
                DrawingSheetId(9),
                OrthographicViewKind::Top,
                "instance-4/g5/o6",
                transform,
                &[edge],
            )
            .unwrap()
            .is_empty()
        );
    }
}
