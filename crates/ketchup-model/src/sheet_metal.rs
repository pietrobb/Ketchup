//! Sheet metal: a planar base face of one thickness with flanges bent off its edges, where
//! every flange can carry further bends, so the bends form a tree. The same bends fold the
//! sheet in space for the exact body and unroll it flat for the manufacturing blank.

use crate::document::{Dimension, DocumentId, FeatureId, FeatureKind, Snapshot, polygon_segments};
use crate::graph::sha256_hex;
use ketchup_geometry::linalg::{Frame, Vec3};
use ketchup_tolerance::MAX_COORDINATE_MM;
use ketchup_tolerance::limits;
use std::fmt;

pub const MIN_SHEET_METAL_BEND_ANGLE_DEGREES: f64 = 0.1;
pub const MAX_SHEET_METAL_BEND_ANGLE_DEGREES: f64 = 179.9;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SheetMetalSpec {
    /// Corners of the base face in the XY plane, counter-clockwise; the sheet fills z
    /// from 0 to its thickness.
    pub base_mm: Vec<[f64; 2]>,
    pub thickness: Dimension,
    pub k_factor: f64,
    /// Ordered by face and edge; a bend comes after the bend whose flange it leaves.
    pub bends: Vec<SheetMetalBend>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SheetMetalBend {
    /// The face the bend leaves: `None` for the base, `Some(i)` for the flange of bend `i`.
    pub parent: Option<usize>,
    /// The edge of that face from its corner `edge` to the next corner. A flange's corners
    /// start where its own bend begins, so its edge 0 hangs on that bend and edge 2 is the
    /// far edge.
    pub edge: usize,
    pub length: Dimension,
    /// Positive bends toward the face normal (+z on the base), negative away from it.
    pub angle_degrees: f64,
    pub inner_radius: Dimension,
}

/// A sheet in plain numbers, as the exact graph carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalShape {
    pub base_mm: Vec<[f64; 2]>,
    pub thickness_mm: f64,
    pub k_factor: f64,
    pub bends: Vec<BendShape>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BendShape {
    pub parent: Option<usize>,
    pub edge: usize,
    pub length_mm: f64,
    pub angle_degrees: f64,
    pub inner_radius_mm: f64,
}

/// The cross-section of a bend in its frame: the ring sector between the inner and outer
/// radius about `center_mm`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BendArc {
    pub center_mm: [f64; 2],
    pub outer_start_mm: [f64; 2],
    pub outer_end_mm: [f64; 2],
    pub inner_end_mm: [f64; 2],
    pub inner_start_mm: [f64; 2],
    /// Whether the outer arc runs clockwise from its start.
    pub clockwise: bool,
}

/// A bend of the folded sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldedBend {
    /// x leaves the parent face across the bent edge, y is the parent face normal and z
    /// runs back along the edge from its end, the origin; `arc` lies in the xy plane and
    /// reaches `span_mm` along z.
    pub frame: Frame,
    pub span_mm: f64,
    pub arc: BendArc,
    /// x runs away from the bend, y along the bent edge and z is the flange normal; the
    /// origin is the flange's corner 1.
    pub flange: Frame,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalFlatBend {
    pub parent: Option<usize>,
    pub edge: usize,
    pub bend_line_start_mm: [f64; 2],
    pub bend_line_end_mm: [f64; 2],
    pub bend_allowance_mm: f64,
    pub angle_degrees: f64,
    pub flange_corners_mm: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalFlatPattern {
    pub base_corners_mm: Vec<[f64; 2]>,
    /// The cut contour of the blank, counter-clockwise, without collinear corners.
    pub outline_mm: Vec<[f64; 2]>,
    pub blank_bounds_mm: [[f64; 2]; 2],
    pub bends: Vec<SheetMetalFlatBend>,
    pub material_area_mm2: f64,
}

/// A 2D placement of a face in the flat blank: origin, then the images of its x and y.
type Placement = [[f64; 2]; 3];

fn place(placement: &Placement, point: [f64; 2]) -> [f64; 2] {
    let [origin, x, y] = placement;
    [
        origin[0] + x[0] * point[0] + y[0] * point[1],
        origin[1] + x[1] * point[0] + y[1] * point[1],
    ]
}

fn in_envelope(value: f64) -> bool {
    value.is_finite() && (limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&value)
}

fn shoelace_area(corners: &[[f64; 2]]) -> f64 {
    corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        * 0.5
}

impl BendShape {
    #[must_use]
    pub fn allowance_mm(&self, thickness_mm: f64, k_factor: f64) -> f64 {
        self.angle_degrees.abs().to_radians() * (self.inner_radius_mm + k_factor * thickness_mm)
    }

    #[must_use]
    pub fn arc(&self, thickness_mm: f64) -> BendArc {
        let beta = self.angle_degrees.abs().to_radians();
        let sign = self.angle_degrees.signum();
        let inner = self.inner_radius_mm;
        let outer = inner + thickness_mm;
        let center_v = thickness_mm * 0.5 + sign * (inner + thickness_mm * 0.5);
        BendArc {
            center_mm: [0.0, center_v],
            outer_start_mm: [0.0, center_v - sign * outer],
            outer_end_mm: [outer * beta.sin(), center_v - sign * outer * beta.cos()],
            inner_end_mm: [inner * beta.sin(), center_v - sign * inner * beta.cos()],
            inner_start_mm: [0.0, center_v - sign * inner],
            clockwise: sign < 0.0,
        }
    }
}

impl SheetMetalShape {
    /// Corners of the base (`None`) or of the flange of bend `i`, given the lengths of the
    /// edges the earlier bends hang on.
    fn corners(&self, spans: &[f64], face: Option<usize>) -> Vec<[f64; 2]> {
        match face {
            None => self.base_mm.clone(),
            Some(index) => {
                let (span, length) = (spans[index], self.bends[index].length_mm);
                vec![[0.0, span], [0.0, 0.0], [length, 0.0], [length, span]]
            }
        }
    }

    fn edge(corners: &[[f64; 2]], edge: usize) -> Option<([f64; 2], [f64; 2])> {
        Some((*corners.get(edge)?, corners[(edge + 1) % corners.len()]))
    }

    /// The length of the edge each bend hangs on; the shape must be valid up to there.
    fn spans(&self) -> Vec<f64> {
        let mut spans = Vec::with_capacity(self.bends.len());
        for bend in &self.bends {
            let (a, b) = Self::edge(&self.corners(&spans, bend.parent), bend.edge)
                .expect("a validated sheet-metal edge");
            spans.push((b[0] - a[0]).hypot(b[1] - a[1]));
        }
        spans
    }

    fn validate_base(&self) -> Result<(), SheetMetalError> {
        let base = &self.base_mm;
        if base.len() < 3
            || base
                .iter()
                .flatten()
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            || shoelace_area(base) <= 0.0
            || !crate::exact_product::is_simple_linear_profile(
                &polygon_segments(base),
                limits::MIN_LENGTH_MM,
            )
        {
            return Err(SheetMetalError::InvalidBase);
        }
        let shortest = (0..base.len())
            .filter_map(|edge| Self::edge(base, edge))
            .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]))
            .fold(f64::INFINITY, f64::min);
        if self.thickness_mm >= shortest {
            return Err(SheetMetalError::ThicknessExceedsBase);
        }
        Ok(())
    }

    fn validate_bend(&self, index: usize, spans: &[f64]) -> Result<f64, SheetMetalError> {
        let bend = self.bends[index];
        if bend.parent.is_some_and(|parent| parent >= index) {
            return Err(SheetMetalError::InvalidBendParent);
        }
        let key = (bend.parent, bend.edge);
        let previous = index
            .checked_sub(1)
            .map(|before| (self.bends[before].parent, self.bends[before].edge));
        if previous == Some(key) || (bend.parent.is_some() && bend.edge == 0) {
            return Err(SheetMetalError::DuplicateEdge);
        }
        if previous.is_some_and(|previous| previous > key) {
            return Err(SheetMetalError::NonCanonicalBendOrder);
        }
        if !in_envelope(bend.length_mm)
            || !in_envelope(bend.inner_radius_mm)
            || bend.inner_radius_mm + self.thickness_mm > MAX_COORDINATE_MM
        {
            return Err(SheetMetalError::DimensionOutsideEnvelope);
        }
        if !bend.angle_degrees.is_finite()
            || !(MIN_SHEET_METAL_BEND_ANGLE_DEGREES..=MAX_SHEET_METAL_BEND_ANGLE_DEGREES)
                .contains(&bend.angle_degrees.abs())
        {
            return Err(SheetMetalError::InvalidBendAngle);
        }
        let (a, b) = Self::edge(&self.corners(spans, bend.parent), bend.edge)
            .ok_or(SheetMetalError::UnknownEdge)?;
        Ok((b[0] - a[0]).hypot(b[1] - a[1]))
    }

    /// Two bends on edges of one face that share a corner (a flange's edge 0 carries its
    /// own bend) would fold into each other without a corner relief.
    fn validate_corners(&self, spans: &[f64]) -> Result<(), SheetMetalError> {
        for (index, bend) in self.bends.iter().enumerate() {
            let count = self.corners(spans, bend.parent).len();
            let adjacent =
                |other: usize| (bend.edge + 1) % count == other || (other + 1) % count == bend.edge;
            let siblings = self.bends[index + 1..]
                .iter()
                .filter(|other| other.parent == bend.parent)
                .map(|other| other.edge);
            let own = bend.parent.map(|_| 0);
            if siblings.chain(own).any(adjacent) {
                return Err(SheetMetalError::AdjacentFlangesRequireCornerRelief);
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), SheetMetalError> {
        if !in_envelope(self.thickness_mm) {
            return Err(SheetMetalError::DimensionOutsideEnvelope);
        }
        if !self.k_factor.is_finite() || !(0.0..=1.0).contains(&self.k_factor) {
            return Err(SheetMetalError::InvalidKFactor);
        }
        self.validate_base()?;
        let mut spans = Vec::with_capacity(self.bends.len());
        for index in 0..self.bends.len() {
            let span = self.validate_bend(index, &spans)?;
            spans.push(span);
        }
        self.validate_corners(&spans)
    }

    pub fn bend_allowance_mm(&self, index: usize) -> Result<f64, SheetMetalError> {
        self.validate()?;
        let bend = self.bends.get(index).ok_or(SheetMetalError::UnknownBend)?;
        Ok(bend.allowance_mm(self.thickness_mm, self.k_factor))
    }

    /// Every bend of the folded sheet, in the order of `bends`.
    pub fn fold(&self) -> Result<Vec<FoldedBend>, SheetMetalError> {
        self.validate()?;
        let spans = self.spans();
        let mut folded: Vec<FoldedBend> = Vec::with_capacity(self.bends.len());
        for (index, bend) in self.bends.iter().enumerate() {
            let face = bend
                .parent
                .map_or(Frame::WORLD, |parent| folded[parent].flange);
            let (a, b) = Self::edge(&self.corners(&spans, bend.parent), bend.edge)
                .expect("a validated sheet-metal edge");
            let span = spans[index];
            let direction = [(b[0] - a[0]) / span, (b[1] - a[1]) / span];
            let along = face.vector(Vec3::new(direction[0], direction[1], 0.0));
            let outward = face.vector(Vec3::new(direction[1], -direction[0], 0.0));
            let normal = face.z;
            let arc = bend.arc(self.thickness_mm);
            let beta = bend.angle_degrees.abs().to_radians();
            let sign = bend.angle_degrees.signum();
            let tangent = outward * beta.cos() + normal * (sign * beta.sin());
            let radial = if sign > 0.0 {
                arc.outer_end_mm
            } else {
                arc.inner_end_mm
            };
            folded.push(FoldedBend {
                frame: Frame {
                    origin: face.point(Vec3::new(b[0], b[1], 0.0)),
                    x: outward,
                    y: normal,
                    z: -along,
                },
                span_mm: span,
                arc,
                flange: Frame {
                    origin: face.point(Vec3::new(a[0], a[1], 0.0))
                        + outward * radial[0]
                        + normal * radial[1],
                    x: tangent,
                    y: along,
                    z: tangent.cross(along),
                },
            });
        }
        Ok(folded)
    }

    /// The corners of the base or of the flange of bend `i` in its own plane.
    pub fn face_corners(&self, face: Option<usize>) -> Result<Vec<[f64; 2]>, SheetMetalError> {
        self.validate()?;
        if face.is_some_and(|index| index >= self.bends.len()) {
            return Err(SheetMetalError::UnknownBend);
        }
        Ok(self.corners(&self.spans(), face))
    }

    /// An axis-aligned box around the folded sheet.
    pub fn folded_bounds_mm(&self) -> Result<[[f64; 3]; 2], SheetMetalError> {
        let folded = self.fold()?;
        let spans = self.spans();
        let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        let mut include = |point: Vec3| {
            let [low, high] = &mut bounds;
            for (axis, (low, high)) in low.iter_mut().zip(high.iter_mut()).enumerate() {
                *low = low.min(point.component(axis));
                *high = high.max(point.component(axis));
            }
        };
        let faces = std::iter::once((Frame::WORLD, None)).chain(
            folded
                .iter()
                .enumerate()
                .map(|(i, bend)| (bend.flange, Some(i))),
        );
        for (frame, face) in faces {
            for corner in self.corners(&spans, face) {
                for depth in [0.0, self.thickness_mm] {
                    include(frame.point(Vec3::new(corner[0], corner[1], depth)));
                }
            }
        }
        for (bend, shape) in folded.iter().zip(&self.bends) {
            let reach = shape.inner_radius_mm + self.thickness_mm;
            for along in [0.0, bend.span_mm] {
                let center = bend.frame.point(Vec3::new(
                    bend.arc.center_mm[0],
                    bend.arc.center_mm[1],
                    along,
                ));
                include(center - Vec3::new(reach, reach, reach));
                include(center + Vec3::new(reach, reach, reach));
            }
        }
        Ok(bounds)
    }

    pub fn flat_pattern(&self) -> Result<SheetMetalFlatPattern, SheetMetalError> {
        self.validate()?;
        let spans = self.spans();
        let mut placements: Vec<Placement> = Vec::with_capacity(self.bends.len());
        let mut bends = Vec::with_capacity(self.bends.len());
        let mut material_area_mm2 = shoelace_area(&self.base_mm);
        for (index, bend) in self.bends.iter().enumerate() {
            let face = bend
                .parent
                .map_or([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], |parent| {
                    placements[parent]
                });
            let (a, b) = Self::edge(&self.corners(&spans, bend.parent), bend.edge)
                .expect("a validated sheet-metal edge");
            let span = spans[index];
            let direction = [(b[0] - a[0]) / span, (b[1] - a[1]) / span];
            let [_, x, y] = face;
            let along = [
                x[0] * direction[0] + y[0] * direction[1],
                x[1] * direction[0] + y[1] * direction[1],
            ];
            let outward = [along[1], -along[0]];
            let allowance = bend.allowance_mm(self.thickness_mm, self.k_factor);
            let shift =
                |point: [f64; 2], by: f64| [point[0] + outward[0] * by, point[1] + outward[1] * by];
            let (start, end) = (place(&face, a), place(&face, b));
            let flange = [shift(start, allowance), outward, along];
            placements.push(flange);
            material_area_mm2 += span * (allowance + bend.length_mm);
            bends.push(SheetMetalFlatBend {
                parent: bend.parent,
                edge: bend.edge,
                bend_line_start_mm: shift(start, allowance * 0.5),
                bend_line_end_mm: shift(end, allowance * 0.5),
                bend_allowance_mm: allowance,
                angle_degrees: bend.angle_degrees,
                flange_corners_mm: self
                    .corners(&spans, Some(index))
                    .into_iter()
                    .map(|corner| place(&flange, corner))
                    .collect(),
            });
        }
        let mut outline = Vec::new();
        self.outline(None, &spans, &placements, &mut outline);
        let outline_mm = without_collinear_corners(outline);
        let mut blank_bounds_mm = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
        for point in &outline_mm {
            for axis in 0..2 {
                blank_bounds_mm[0][axis] = blank_bounds_mm[0][axis].min(point[axis]);
                blank_bounds_mm[1][axis] = blank_bounds_mm[1][axis].max(point[axis]);
            }
        }
        Ok(SheetMetalFlatPattern {
            base_corners_mm: self.base_mm.clone(),
            outline_mm,
            blank_bounds_mm,
            bends,
            material_area_mm2,
        })
    }

    /// Walks the boundary of `face` in the blank: each bent edge detours around the
    /// flange it carries. A flange is entered at its corner 1 and left at its corner 0.
    fn outline(
        &self,
        face: Option<usize>,
        spans: &[f64],
        placements: &[Placement],
        outline: &mut Vec<[f64; 2]>,
    ) {
        let corners = self.corners(spans, face);
        let placement = face.map_or([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], |index| {
            placements[index]
        });
        let first = usize::from(face.is_some());
        for step in first..first + corners.len() {
            let corner = step % corners.len();
            outline.push(place(&placement, corners[corner]));
            if let Some(child) = self
                .bends
                .iter()
                .position(|bend| bend.parent == face && bend.edge == corner)
            {
                self.outline(Some(child), spans, placements, outline);
            }
        }
    }

    #[must_use]
    pub fn base_area_mm2(&self) -> f64 {
        shoelace_area(&self.base_mm)
    }
}

/// Drops repeated points and corners on a straight run of a closed contour.
fn without_collinear_corners(points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let close =
        |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) <= limits::MIN_LENGTH_MM;
    let mut points = points;
    points.dedup_by(|a, b| close(*a, *b));
    while points.len() > 1 && close(points[0], points[points.len() - 1]) {
        points.pop();
    }
    loop {
        let count = points.len();
        let straight = (0..count).find(|&index| {
            let previous = points[(index + count - 1) % count];
            let next = points[(index + 1) % count];
            let point = points[index];
            let run = [next[0] - previous[0], next[1] - previous[1]];
            let length = run[0].hypot(run[1]);
            let offset = (run[0] * (point[1] - previous[1]) - run[1] * (point[0] - previous[0]))
                / length.max(f64::MIN_POSITIVE);
            count > 3 && offset.abs() <= limits::MIN_LENGTH_MM
        });
        match straight {
            Some(index) => {
                points.remove(index);
            }
            None => return points,
        }
    }
}

impl SheetMetalBend {
    #[must_use]
    pub fn shape(&self) -> BendShape {
        BendShape {
            parent: self.parent,
            edge: self.edge,
            length_mm: self.length.millimetres(),
            angle_degrees: self.angle_degrees,
            inner_radius_mm: self.inner_radius.millimetres(),
        }
    }
}

impl SheetMetalSpec {
    #[must_use]
    pub fn shape(&self) -> SheetMetalShape {
        SheetMetalShape {
            base_mm: self.base_mm.clone(),
            thickness_mm: self.thickness.millimetres(),
            k_factor: self.k_factor,
            bends: self.bends.iter().map(SheetMetalBend::shape).collect(),
        }
    }

    pub fn validate(&self) -> Result<(), SheetMetalError> {
        let dimensions = std::iter::once(&self.thickness).chain(
            self.bends
                .iter()
                .flat_map(|bend| [&bend.length, &bend.inner_radius]),
        );
        for dimension in dimensions {
            if dimension.source_token().trim().is_empty() {
                return Err(SheetMetalError::DimensionOutsideEnvelope);
            }
        }
        self.shape().validate()
    }

    pub fn bend_allowance_mm(&self, index: usize) -> Result<f64, SheetMetalError> {
        self.validate()?;
        self.shape().bend_allowance_mm(index)
    }

    pub fn flat_pattern(&self) -> Result<SheetMetalFlatPattern, SheetMetalError> {
        self.validate()?;
        self.shape().flat_pattern()
    }

    pub fn folded_neutral_area_mm2(&self) -> Result<f64, SheetMetalError> {
        self.validate()?;
        let shape = self.shape();
        let spans = shape.spans();
        Ok(shape.base_area_mm2()
            + shape
                .bends
                .iter()
                .zip(spans)
                .map(|(bend, span)| {
                    span * (bend.allowance_mm(shape.thickness_mm, shape.k_factor) + bend.length_mm)
                })
                .sum::<f64>())
    }

    #[must_use]
    pub fn base_area_mm2(&self) -> f64 {
        shoelace_area(&self.base_mm)
    }
}

pub const SHEET_METAL_MANUFACTURING_EXPORT_V1: &str = "ketchup.sheet-metal-manufacturing-export.v1";
pub const SHEET_METAL_FLAT_PATTERN_DXF_V1: &str = "ketchup.sheet-metal-flat-pattern-dxf.v1";
pub const SHEET_METAL_BEND_TABLE_CSV_V1: &str = "ketchup.sheet-metal-bend-table-csv.v1";

#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalManufacturingProjection {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub feature_id: FeatureId,
    pub result_digest: String,
    pub flat_pattern: SheetMetalFlatPattern,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetMetalManufacturingArtifacts {
    pub flat_pattern_dxf: Vec<u8>,
    pub bend_table_csv: Vec<u8>,
}

impl SheetMetalManufacturingProjection {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }

    pub fn artifacts(
        &self,
        snapshot: &Snapshot,
    ) -> Result<SheetMetalManufacturingArtifacts, SheetMetalManufacturingExportError> {
        if !self.is_current(snapshot) {
            return Err(SheetMetalManufacturingExportError::StaleProjection);
        }
        let (spec, flat_pattern) = current_sheet_metal(snapshot, self.feature_id)?;
        let payload = sheet_metal_manufacturing_payload(self.feature_id, spec, &flat_pattern);
        if self.flat_pattern != flat_pattern || self.result_digest != sha256_hex(&payload) {
            return Err(SheetMetalManufacturingExportError::TamperedProjection);
        }
        Ok(SheetMetalManufacturingArtifacts {
            flat_pattern_dxf: sheet_metal_flat_pattern_dxf(self, spec),
            bend_table_csv: sheet_metal_bend_table_csv(self, spec),
        })
    }
}

pub fn project_sheet_metal_manufacturing(
    snapshot: &Snapshot,
    feature_id: FeatureId,
) -> Result<SheetMetalManufacturingProjection, SheetMetalManufacturingExportError> {
    let (spec, flat_pattern) = current_sheet_metal(snapshot, feature_id)?;
    let result_digest = sha256_hex(&sheet_metal_manufacturing_payload(
        feature_id,
        spec,
        &flat_pattern,
    ));
    Ok(SheetMetalManufacturingProjection {
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        feature_id,
        result_digest,
        flat_pattern,
    })
}

fn current_sheet_metal(
    snapshot: &Snapshot,
    feature_id: FeatureId,
) -> Result<(&SheetMetalSpec, SheetMetalFlatPattern), SheetMetalManufacturingExportError> {
    let feature = snapshot
        .feature(feature_id)
        .ok_or(SheetMetalManufacturingExportError::FeatureNotFound)?;
    let FeatureKind::SheetMetal(spec) = feature.kind() else {
        return Err(SheetMetalManufacturingExportError::NotSheetMetal);
    };
    if snapshot.feature_is_suppressed(feature_id) {
        return Err(SheetMetalManufacturingExportError::SuppressedFeature);
    }
    let flat_pattern = spec
        .flat_pattern()
        .map_err(SheetMetalManufacturingExportError::InvalidSheetMetal)?;
    Ok((spec, flat_pattern))
}

fn sheet_metal_manufacturing_payload(
    feature_id: FeatureId,
    spec: &SheetMetalSpec,
    pattern: &SheetMetalFlatPattern,
) -> Vec<u8> {
    let mut bytes = SHEET_METAL_MANUFACTURING_EXPORT_V1.as_bytes().to_vec();
    let number = |bytes: &mut Vec<u8>, value: f64| {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    };
    let points = |bytes: &mut Vec<u8>, points: &[[f64; 2]]| {
        bytes.extend_from_slice(&(points.len() as u64).to_le_bytes());
        for value in points.iter().flatten() {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    };
    bytes.extend_from_slice(&feature_id.0.to_le_bytes());
    for value in [
        spec.thickness.millimetres(),
        spec.k_factor,
        pattern.material_area_mm2,
    ] {
        number(&mut bytes, value);
    }
    points(&mut bytes, &pattern.base_corners_mm);
    points(&mut bytes, &pattern.outline_mm);
    points(&mut bytes, &pattern.blank_bounds_mm);
    bytes.extend_from_slice(&(pattern.bends.len() as u64).to_le_bytes());
    for bend in &pattern.bends {
        let parent = bend.parent.map_or(0, |parent| parent as u64 + 1);
        bytes.extend_from_slice(&parent.to_le_bytes());
        bytes.extend_from_slice(&(bend.edge as u64).to_le_bytes());
        points(
            &mut bytes,
            &[bend.bend_line_start_mm, bend.bend_line_end_mm],
        );
        number(&mut bytes, bend.bend_allowance_mm);
        number(&mut bytes, bend.angle_degrees);
        points(&mut bytes, &bend.flange_corners_mm);
    }
    bytes
}

fn sheet_metal_flat_pattern_dxf(
    projection: &SheetMetalManufacturingProjection,
    spec: &SheetMetalSpec,
) -> Vec<u8> {
    let mut dxf = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n9\n$KETCHUP_SCHEMA\n1\n{SHEET_METAL_FLAT_PATTERN_DXF_V1}\n9\n$KETCHUP_DOCUMENT_ID\n1\n{}\n9\n$KETCHUP_SOURCE_REVISION\n1\n{}\n9\n$KETCHUP_SOURCE_DIGEST\n1\n{}\n9\n$KETCHUP_FEATURE_ID\n1\n{}\n9\n$KETCHUP_RESULT_DIGEST\n1\n{}\n9\n$KETCHUP_THICKNESS_MM\n1\n{}\n9\n$KETCHUP_K_FACTOR\n1\n{}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n",
        projection.document_id.0,
        projection.source_revision,
        projection.source_digest,
        projection.feature_id.0,
        projection.result_digest,
        sheet_metal_number(spec.thickness.millimetres()),
        sheet_metal_number(spec.k_factor),
    );
    let outline = &projection.flat_pattern.outline_mm;
    for (start, end) in outline.iter().zip(outline.iter().cycle().skip(1)) {
        push_dxf_line(&mut dxf, "CUT", *start, *end);
    }
    for bend in &projection.flat_pattern.bends {
        let layer = if bend.angle_degrees.is_sign_positive() {
            "BEND_UP"
        } else {
            "BEND_DOWN"
        };
        push_dxf_line(
            &mut dxf,
            layer,
            bend.bend_line_start_mm,
            bend.bend_line_end_mm,
        );
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    dxf.into_bytes()
}

fn push_dxf_line(output: &mut String, layer: &str, start: [f64; 2], end: [f64; 2]) {
    output.push_str(&format!(
        "0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
        sheet_metal_number(start[0]),
        sheet_metal_number(start[1]),
        sheet_metal_number(end[0]),
        sheet_metal_number(end[1]),
    ));
}

fn sheet_metal_bend_table_csv(
    projection: &SheetMetalManufacturingProjection,
    spec: &SheetMetalSpec,
) -> Vec<u8> {
    let mut csv = format!(
        "{SHEET_METAL_BEND_TABLE_CSV_V1}\ndocument_id,source_revision,source_digest,feature_id,result_digest,thickness_mm,k_factor\n{},{},{},{},{},{},{}\nbend,face,edge,direction,angle_degrees,inner_radius_mm,bend_allowance_mm,start_x_mm,start_y_mm,end_x_mm,end_y_mm\n",
        projection.document_id.0,
        projection.source_revision,
        projection.source_digest,
        projection.feature_id.0,
        projection.result_digest,
        sheet_metal_number(spec.thickness.millimetres()),
        sheet_metal_number(spec.k_factor),
    );
    for (index, flat_bend) in projection.flat_pattern.bends.iter().enumerate() {
        let bend = &spec.bends[index];
        csv.push_str(&format!(
            "{index},{},{},{},{},{},{},{},{},{},{}\n",
            flat_bend
                .parent
                .map_or_else(|| "base".to_owned(), |parent| format!("bend-{parent}")),
            flat_bend.edge,
            if flat_bend.angle_degrees.is_sign_positive() {
                "up"
            } else {
                "down"
            },
            sheet_metal_number(flat_bend.angle_degrees),
            sheet_metal_number(bend.inner_radius.millimetres()),
            sheet_metal_number(flat_bend.bend_allowance_mm),
            sheet_metal_number(flat_bend.bend_line_start_mm[0]),
            sheet_metal_number(flat_bend.bend_line_start_mm[1]),
            sheet_metal_number(flat_bend.bend_line_end_mm[0]),
            sheet_metal_number(flat_bend.bend_line_end_mm[1]),
        ));
    }
    csv.into_bytes()
}

fn sheet_metal_number(value: f64) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    let mut formatted = format!("{value:.12}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetMetalManufacturingExportError {
    FeatureNotFound,
    NotSheetMetal,
    SuppressedFeature,
    InvalidSheetMetal(SheetMetalError),
    StaleProjection,
    TamperedProjection,
}

impl fmt::Display for SheetMetalManufacturingExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FeatureNotFound => {
                formatter.write_str("sheet-metal export feature was not found")
            }
            Self::NotSheetMetal => {
                formatter.write_str("sheet-metal export requires a sheet-metal feature")
            }
            Self::SuppressedFeature => {
                formatter.write_str("a suppressed sheet-metal feature cannot be exported")
            }
            Self::InvalidSheetMetal(error) => {
                write!(formatter, "invalid sheet-metal feature: {error}")
            }
            Self::StaleProjection => formatter.write_str(
                "sheet-metal export projection is stale for the current document revision",
            ),
            Self::TamperedProjection => formatter.write_str(
                "sheet-metal export projection does not match its canonical manufacturing result",
            ),
        }
    }
}

impl std::error::Error for SheetMetalManufacturingExportError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetMetalError {
    DimensionOutsideEnvelope,
    InvalidBase,
    ThicknessExceedsBase,
    InvalidKFactor,
    InvalidBendParent,
    UnknownEdge,
    DuplicateEdge,
    NonCanonicalBendOrder,
    InvalidBendAngle,
    AdjacentFlangesRequireCornerRelief,
    UnknownBend,
}

impl fmt::Display for SheetMetalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DimensionOutsideEnvelope => {
                "sheet-metal dimensions must be finite and inside the exact manufacturing envelope"
            }
            Self::InvalidBase => {
                "the sheet-metal base must be a simple counter-clockwise polygon of at least three corners inside the envelope"
            }
            Self::ThicknessExceedsBase => {
                "sheet-metal thickness must be smaller than every edge of the base"
            }
            Self::InvalidKFactor => "sheet-metal K-factor must be finite and between zero and one",
            Self::InvalidBendParent => {
                "a sheet-metal bend must leave the base or the flange of an earlier bend"
            }
            Self::UnknownEdge => "a sheet-metal bend names an edge its face does not have",
            Self::DuplicateEdge => "a sheet-metal edge carries at most one bend",
            Self::NonCanonicalBendOrder => {
                "sheet-metal bends must be strictly ordered by face and edge"
            }
            Self::InvalidBendAngle => {
                "sheet-metal bend angle must be non-zero and less than 180 degrees"
            }
            Self::AdjacentFlangesRequireCornerRelief => {
                "bends on edges that share a corner require an explicit corner relief to avoid self-intersection"
            }
            Self::UnknownBend => "the sheet-metal bend is not part of this body",
        })
    }
}

impl std::error::Error for SheetMetalError {}

#[cfg(test)]
mod tests;
