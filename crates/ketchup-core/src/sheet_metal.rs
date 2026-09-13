use crate::document::{Dimension, DocumentId, FeatureId, FeatureKind, Snapshot};
use crate::graph::sha256_hex;
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_SHEET_METAL_FLANGES: usize = 4;
pub const MIN_SHEET_METAL_LENGTH_MM: f64 = 1.0e-4;
pub const MAX_SHEET_METAL_LENGTH_MM: f64 = 100_000.0;
pub const MIN_SHEET_METAL_BEND_ANGLE_DEGREES: f64 = 0.1;
pub const MAX_SHEET_METAL_BEND_ANGLE_DEGREES: f64 = 179.9;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SheetMetalEdge {
    MinX,
    MaxX,
    MinY,
    MaxY,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalFlange {
    pub edge: SheetMetalEdge,
    pub length: Dimension,
    pub angle_degrees: f64,
    pub inner_radius: Dimension,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalSpec {
    pub width: Dimension,
    pub depth: Dimension,
    pub thickness: Dimension,
    pub k_factor: f64,
    pub flanges: Vec<SheetMetalFlange>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetMetalFlatFlange {
    pub edge: SheetMetalEdge,
    pub bend_line_start_mm: [f64; 2],
    pub bend_line_end_mm: [f64; 2],
    pub bend_allowance_mm: f64,
    pub angle_degrees: f64,
    pub panel_bounds_mm: [[f64; 2]; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalFlatPattern {
    pub base_bounds_mm: [[f64; 2]; 2],
    pub blank_bounds_mm: [[f64; 2]; 2],
    pub flanges: Vec<SheetMetalFlatFlange>,
    pub material_area_mm2: f64,
}

impl SheetMetalSpec {
    fn validate_parameters(&self) -> Result<(), SheetMetalError> {
        for dimension in [&self.width, &self.depth, &self.thickness] {
            validate_positive_dimension(dimension)?;
        }
        if self.thickness.millimetres() >= self.width.millimetres().min(self.depth.millimetres()) {
            return Err(SheetMetalError::ThicknessExceedsBase);
        }
        if !self.k_factor.is_finite() || !(0.0..=1.0).contains(&self.k_factor) {
            return Err(SheetMetalError::InvalidKFactor);
        }
        if self.flanges.len() > MAX_SHEET_METAL_FLANGES {
            return Err(SheetMetalError::TooManyFlanges);
        }
        let mut edges = BTreeSet::new();
        let mut previous = None;
        for flange in &self.flanges {
            if !edges.insert(flange.edge) {
                return Err(SheetMetalError::DuplicateEdge);
            }
            if previous.is_some_and(|edge| edge >= flange.edge) {
                return Err(SheetMetalError::NonCanonicalFlangeOrder);
            }
            previous = Some(flange.edge);
            validate_positive_dimension(&flange.length)?;
            validate_positive_dimension(&flange.inner_radius)?;
            if !flange.angle_degrees.is_finite()
                || !(MIN_SHEET_METAL_BEND_ANGLE_DEGREES..=MAX_SHEET_METAL_BEND_ANGLE_DEGREES)
                    .contains(&flange.angle_degrees.abs())
            {
                return Err(SheetMetalError::InvalidBendAngle);
            }
            if flange.inner_radius.millimetres() + self.thickness.millimetres()
                > MAX_SHEET_METAL_LENGTH_MM
            {
                return Err(SheetMetalError::DimensionOutsideEnvelope);
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), SheetMetalError> {
        self.validate_parameters()?;
        for (index, flange) in self.flanges.iter().enumerate() {
            if self.flanges[index + 1..]
                .iter()
                .any(|other| edges_are_adjacent(flange.edge, other.edge))
            {
                return Err(SheetMetalError::AdjacentFlangesRequireCornerRelief);
            }
        }
        Ok(())
    }

    pub fn bend_allowance_mm(&self, flange: &SheetMetalFlange) -> Result<f64, SheetMetalError> {
        self.validate_parameters()?;
        if !self.flanges.iter().any(|candidate| candidate == flange) {
            return Err(SheetMetalError::UnknownFlange);
        }
        Ok(flange.angle_degrees.abs().to_radians()
            * (flange.inner_radius.millimetres() + self.k_factor * self.thickness.millimetres()))
    }

    pub fn flat_pattern(&self) -> Result<SheetMetalFlatPattern, SheetMetalError> {
        self.validate()?;
        let width = self.width.millimetres();
        let depth = self.depth.millimetres();
        let mut blank_bounds_mm = [[0.0, 0.0], [width, depth]];
        let mut material_area_mm2 = width * depth;
        let mut flanges = Vec::with_capacity(self.flanges.len());
        for flange in &self.flanges {
            let allowance = self.bend_allowance_mm(flange)?;
            let length = flange.length.millimetres();
            let (bend_line_start_mm, bend_line_end_mm, panel_bounds_mm) = match flange.edge {
                SheetMetalEdge::MinX => (
                    [-allowance * 0.5, 0.0],
                    [-allowance * 0.5, depth],
                    [[-allowance - length, 0.0], [-allowance, depth]],
                ),
                SheetMetalEdge::MaxX => (
                    [width + allowance * 0.5, 0.0],
                    [width + allowance * 0.5, depth],
                    [
                        [width + allowance, 0.0],
                        [width + allowance + length, depth],
                    ],
                ),
                SheetMetalEdge::MinY => (
                    [0.0, -allowance * 0.5],
                    [width, -allowance * 0.5],
                    [[0.0, -allowance - length], [width, -allowance]],
                ),
                SheetMetalEdge::MaxY => (
                    [0.0, depth + allowance * 0.5],
                    [width, depth + allowance * 0.5],
                    [
                        [0.0, depth + allowance],
                        [width, depth + allowance + length],
                    ],
                ),
            };
            for axis in 0..2 {
                blank_bounds_mm[0][axis] = blank_bounds_mm[0][axis].min(panel_bounds_mm[0][axis]);
                blank_bounds_mm[1][axis] = blank_bounds_mm[1][axis].max(panel_bounds_mm[1][axis]);
            }
            let span = if matches!(flange.edge, SheetMetalEdge::MinX | SheetMetalEdge::MaxX) {
                depth
            } else {
                width
            };
            material_area_mm2 += span * (allowance + length);
            flanges.push(SheetMetalFlatFlange {
                edge: flange.edge,
                bend_line_start_mm,
                bend_line_end_mm,
                bend_allowance_mm: allowance,
                angle_degrees: flange.angle_degrees,
                panel_bounds_mm,
            });
        }
        Ok(SheetMetalFlatPattern {
            base_bounds_mm: [[0.0, 0.0], [width, depth]],
            blank_bounds_mm,
            flanges,
            material_area_mm2,
        })
    }

    pub fn folded_neutral_area_mm2(&self) -> Result<f64, SheetMetalError> {
        self.validate()?;
        let mut area = self.base_area_mm2();
        for flange in &self.flanges {
            let span = if matches!(flange.edge, SheetMetalEdge::MinX | SheetMetalEdge::MaxX) {
                self.depth.millimetres()
            } else {
                self.width.millimetres()
            };
            area += span * (self.bend_allowance_mm(flange)? + flange.length.millimetres());
        }
        Ok(area)
    }

    #[must_use]
    pub fn base_area_mm2(&self) -> f64 {
        self.width.millimetres() * self.depth.millimetres()
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
    bytes.extend_from_slice(&feature_id.0.to_le_bytes());
    for value in [
        spec.width.millimetres(),
        spec.depth.millimetres(),
        spec.thickness.millimetres(),
        spec.k_factor,
        pattern.material_area_mm2,
    ] {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    for point in [pattern.base_bounds_mm[0], pattern.base_bounds_mm[1]] {
        for value in point {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    for point in [pattern.blank_bounds_mm[0], pattern.blank_bounds_mm[1]] {
        for value in point {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    bytes.extend_from_slice(&(pattern.flanges.len() as u64).to_le_bytes());
    for flange in &pattern.flanges {
        bytes.push(sheet_metal_edge_tag(flange.edge));
        for point in [flange.bend_line_start_mm, flange.bend_line_end_mm] {
            for value in point {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        for value in [flange.bend_allowance_mm, flange.angle_degrees] {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        for point in flange.panel_bounds_mm {
            for value in point {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    bytes
}

fn sheet_metal_flat_pattern_dxf(
    projection: &SheetMetalManufacturingProjection,
    spec: &SheetMetalSpec,
) -> Vec<u8> {
    let minimum = projection.flat_pattern.blank_bounds_mm[0];
    let maximum = projection.flat_pattern.blank_bounds_mm[1];
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
    for (start, end) in [
        (minimum, [maximum[0], minimum[1]]),
        ([maximum[0], minimum[1]], maximum),
        (maximum, [minimum[0], maximum[1]]),
        ([minimum[0], maximum[1]], minimum),
    ] {
        push_dxf_line(&mut dxf, "CUT", start, end);
    }
    for flange in &projection.flat_pattern.flanges {
        let layer = if flange.angle_degrees.is_sign_positive() {
            "BEND_UP"
        } else {
            "BEND_DOWN"
        };
        push_dxf_line(
            &mut dxf,
            layer,
            flange.bend_line_start_mm,
            flange.bend_line_end_mm,
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
        "{SHEET_METAL_BEND_TABLE_CSV_V1}\ndocument_id,source_revision,source_digest,feature_id,result_digest,thickness_mm,k_factor\n{},{},{},{},{},{},{}\nedge,direction,angle_degrees,inner_radius_mm,bend_allowance_mm,start_x_mm,start_y_mm,end_x_mm,end_y_mm\n",
        projection.document_id.0,
        projection.source_revision,
        projection.source_digest,
        projection.feature_id.0,
        projection.result_digest,
        sheet_metal_number(spec.thickness.millimetres()),
        sheet_metal_number(spec.k_factor),
    );
    for (index, flat_flange) in projection.flat_pattern.flanges.iter().enumerate() {
        let flange = &spec.flanges[index];
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            sheet_metal_edge_token(flat_flange.edge),
            if flat_flange.angle_degrees.is_sign_positive() {
                "up"
            } else {
                "down"
            },
            sheet_metal_number(flat_flange.angle_degrees),
            sheet_metal_number(flange.inner_radius.millimetres()),
            sheet_metal_number(flat_flange.bend_allowance_mm),
            sheet_metal_number(flat_flange.bend_line_start_mm[0]),
            sheet_metal_number(flat_flange.bend_line_start_mm[1]),
            sheet_metal_number(flat_flange.bend_line_end_mm[0]),
            sheet_metal_number(flat_flange.bend_line_end_mm[1]),
        ));
    }
    csv.into_bytes()
}

const fn sheet_metal_edge_tag(edge: SheetMetalEdge) -> u8 {
    match edge {
        SheetMetalEdge::MinX => 0,
        SheetMetalEdge::MaxX => 1,
        SheetMetalEdge::MinY => 2,
        SheetMetalEdge::MaxY => 3,
    }
}

const fn sheet_metal_edge_token(edge: SheetMetalEdge) -> &'static str {
    match edge {
        SheetMetalEdge::MinX => "min-x",
        SheetMetalEdge::MaxX => "max-x",
        SheetMetalEdge::MinY => "min-y",
        SheetMetalEdge::MaxY => "max-y",
    }
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

fn edges_are_adjacent(left: SheetMetalEdge, right: SheetMetalEdge) -> bool {
    !matches!(
        (left, right),
        (SheetMetalEdge::MinX, SheetMetalEdge::MaxX)
            | (SheetMetalEdge::MaxX, SheetMetalEdge::MinX)
            | (SheetMetalEdge::MinY, SheetMetalEdge::MaxY)
            | (SheetMetalEdge::MaxY, SheetMetalEdge::MinY)
    )
}

fn validate_positive_dimension(dimension: &Dimension) -> Result<(), SheetMetalError> {
    let value = dimension.millimetres();
    if dimension.source_token().trim().is_empty()
        || !value.is_finite()
        || !(MIN_SHEET_METAL_LENGTH_MM..=MAX_SHEET_METAL_LENGTH_MM).contains(&value)
    {
        return Err(SheetMetalError::DimensionOutsideEnvelope);
    }
    Ok(())
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
    ThicknessExceedsBase,
    InvalidKFactor,
    TooManyFlanges,
    DuplicateEdge,
    NonCanonicalFlangeOrder,
    InvalidBendAngle,
    AdjacentFlangesRequireCornerRelief,
    UnknownFlange,
}

impl fmt::Display for SheetMetalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DimensionOutsideEnvelope => {
                "sheet-metal dimensions must be finite and inside the exact manufacturing envelope"
            }
            Self::ThicknessExceedsBase => {
                "sheet-metal thickness must be smaller than both base dimensions"
            }
            Self::InvalidKFactor => "sheet-metal K-factor must be finite and between zero and one",
            Self::TooManyFlanges => {
                "sheet-metal base supports at most one flange on each boundary edge"
            }
            Self::DuplicateEdge => "sheet-metal flange edges must be unique",
            Self::NonCanonicalFlangeOrder => "sheet-metal flanges must be strictly ordered by edge",
            Self::InvalidBendAngle => {
                "sheet-metal bend angle must be non-zero and less than 180 degrees"
            }
            Self::AdjacentFlangesRequireCornerRelief => {
                "adjacent sheet-metal flanges require an explicit corner relief to avoid self-intersection"
            }
            Self::UnknownFlange => "sheet-metal flange is not part of this body",
        })
    }
}

impl std::error::Error for SheetMetalError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimension(value: f64) -> Dimension {
        Dimension::new(value.to_string(), value).unwrap()
    }

    fn flange(edge: SheetMetalEdge, length: f64, angle_degrees: f64) -> SheetMetalFlange {
        SheetMetalFlange {
            edge,
            length: dimension(length),
            angle_degrees,
            inner_radius: dimension(3.0),
        }
    }

    #[test]
    fn flat_pattern_preserves_neutral_area_and_signed_opposite_bends() {
        let spec = SheetMetalSpec {
            width: dimension(100.0),
            depth: dimension(50.0),
            thickness: dimension(2.0),
            k_factor: 0.4,
            flanges: vec![
                flange(SheetMetalEdge::MinX, 20.0, 90.0),
                flange(SheetMetalEdge::MaxX, 30.0, -45.0),
            ],
        };
        let pattern = spec.flat_pattern().unwrap();
        let min_allowance = spec.bend_allowance_mm(&spec.flanges[0]).unwrap();
        let max_allowance = spec.bend_allowance_mm(&spec.flanges[1]).unwrap();

        assert_eq!(pattern.base_bounds_mm, [[0.0, 0.0], [100.0, 50.0]]);
        assert_eq!(
            pattern.blank_bounds_mm,
            [
                [-min_allowance - 20.0, 0.0],
                [100.0 + max_allowance + 30.0, 50.0]
            ]
        );
        assert_eq!(pattern.flanges[0].angle_degrees, 90.0);
        assert_eq!(pattern.flanges[1].angle_degrees, -45.0);
        assert_eq!(
            pattern.material_area_mm2,
            spec.folded_neutral_area_mm2().unwrap()
        );
    }

    #[test]
    fn adjacent_flanges_without_corner_relief_are_rejected_as_self_intersecting() {
        let spec = SheetMetalSpec {
            width: dimension(100.0),
            depth: dimension(50.0),
            thickness: dimension(2.0),
            k_factor: 0.4,
            flanges: vec![
                flange(SheetMetalEdge::MinX, 20.0, 90.0),
                flange(SheetMetalEdge::MinY, 20.0, 90.0),
            ],
        };
        assert_eq!(
            spec.flat_pattern(),
            Err(SheetMetalError::AdjacentFlangesRequireCornerRelief)
        );
    }

    #[test]
    fn manufacturing_export_is_associative_tamper_evident_and_history_stable() {
        use crate::document::{
            CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureParameterTarget,
            ParameterValueType,
        };
        use crate::persistence::{ContainerData, load, save_document_store};

        let feature_id = FeatureId(41);
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(40),
                    name: "Manufactured bracket".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: feature_id,
                    definition_id: DefinitionId(40),
                    name: "Opposite flanges".into(),
                    kind: FeatureKind::SheetMetal(SheetMetalSpec {
                        width: dimension(100.0),
                        depth: dimension(50.0),
                        thickness: dimension(2.0),
                        k_factor: 0.4,
                        flanges: vec![
                            flange(SheetMetalEdge::MinX, 20.0, 90.0),
                            flange(SheetMetalEdge::MaxX, 30.0, -45.0),
                        ],
                    }),
                },
            ]))
            .unwrap();
        let original_projection =
            project_sheet_metal_manufacturing(&document.current(), feature_id).unwrap();
        let original = original_projection.artifacts(&document.current()).unwrap();
        let dxf = String::from_utf8(original.flat_pattern_dxf.clone()).unwrap();
        let bends = String::from_utf8(original.bend_table_csv.clone()).unwrap();
        assert!(dxf.contains(SHEET_METAL_FLAT_PATTERN_DXF_V1));
        assert_eq!(dxf.matches("0\nLINE\n8\nCUT\n").count(), 4);
        assert_eq!(dxf.matches("0\nLINE\n8\nBEND_UP\n").count(), 1);
        assert_eq!(dxf.matches("0\nLINE\n8\nBEND_DOWN\n").count(), 1);
        assert!(bends.contains("min-x,up,90,3,"));
        assert!(bends.contains("max-x,down,-45,3,"));

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureParameter {
                    target: FeatureParameterTarget::new(
                        feature_id,
                        "flanges.1.length",
                        ParameterValueType::Length,
                    )
                    .unwrap(),
                    dimension: dimension(40.0),
                },
            ]))
            .unwrap();
        assert_eq!(
            original_projection.artifacts(&document.current()),
            Err(SheetMetalManufacturingExportError::StaleProjection)
        );
        let edited_projection =
            project_sheet_metal_manufacturing(&document.current(), feature_id).unwrap();
        let edited = edited_projection.artifacts(&document.current()).unwrap();
        assert_ne!(edited, original);

        let mut tampered = edited_projection.clone();
        tampered.flat_pattern.flanges[0].bend_allowance_mm += 1.0;
        assert_eq!(
            tampered.artifacts(&document.current()),
            Err(SheetMetalManufacturingExportError::TamperedProjection)
        );
        let mut tampered_digest = edited_projection.clone();
        tampered_digest.result_digest.replace_range(..1, "0");
        assert_eq!(
            tampered_digest.artifacts(&document.current()),
            Err(SheetMetalManufacturingExportError::TamperedProjection)
        );

        document.undo().unwrap();
        assert_eq!(
            original_projection.artifacts(&document.current()).unwrap(),
            original
        );
        document.redo().unwrap();
        assert_eq!(
            edited_projection.artifacts(&document.current()).unwrap(),
            edited
        );

        let saved = save_document_store(&document, &ContainerData::default()).unwrap();
        let (mut reopened, _) = load(&saved)
            .unwrap()
            .into_editable_with_container()
            .ok()
            .expect("schema-81 sheet metal must reopen editable");
        assert_eq!(
            project_sheet_metal_manufacturing(&reopened.current(), feature_id)
                .unwrap()
                .artifacts(&reopened.current())
                .unwrap(),
            edited
        );
        reopened.undo().unwrap();
        assert_eq!(
            project_sheet_metal_manufacturing(&reopened.current(), feature_id)
                .unwrap()
                .artifacts(&reopened.current())
                .unwrap(),
            original
        );
    }
}
