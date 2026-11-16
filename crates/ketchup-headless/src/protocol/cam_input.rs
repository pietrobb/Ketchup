//! The CAM operations and fixtures a `cam_preview` or `cam_export` request names.
use super::*;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CamOperationInput {
    Face {
        id: u64,
        minimum_mm: [f64; 2],
        maximum_mm: [f64; 2],
        target_z_mm: f64,
    },
    Pocket {
        id: u64,
        minimum_mm: [f64; 2],
        maximum_mm: [f64; 2],
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
    Contour {
        id: u64,
        center_path: CamPathInput,
        top_z_mm: f64,
        bottom_z_mm: f64,
        applied_radial_allowance_mm: f64,
    },
    Drill {
        id: u64,
        points_mm: Vec<[f64; 2]>,
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
    FacePocket {
        id: u64,
        face_ordinal: u32,
    },
    FaceContour {
        id: u64,
        face_ordinal: u32,
        side: CamContourSideInput,
    },
    FaceDrill {
        id: u64,
        face_ordinal: u32,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CamContourSideInput {
    Outside,
    Inside,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CamPathInput {
    start_mm: [f64; 2],
    segments: Vec<CamPathSegmentInput>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CamPathSegmentInput {
    Line {
        to_mm: [f64; 2],
    },
    Arc {
        to_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CamFixtureInput {
    id: u64,
    minimum_mm: [f64; 3],
    maximum_mm: [f64; 3],
}

impl From<CamOperationInput> for CamOperation {
    fn from(operation: CamOperationInput) -> Self {
        match operation {
            CamOperationInput::Face {
                id,
                minimum_mm,
                maximum_mm,
                target_z_mm,
            } => Self::Face {
                id,
                minimum_mm,
                maximum_mm,
                target_z_mm,
            },
            CamOperationInput::Pocket {
                id,
                minimum_mm,
                maximum_mm,
                top_z_mm,
                bottom_z_mm,
            } => Self::Pocket {
                id,
                minimum_mm,
                maximum_mm,
                top_z_mm,
                bottom_z_mm,
            },
            CamOperationInput::Contour {
                id,
                center_path,
                top_z_mm,
                bottom_z_mm,
                applied_radial_allowance_mm,
            } => Self::Contour {
                id,
                center_path: CamPath2d {
                    start_mm: center_path.start_mm,
                    segments: center_path
                        .segments
                        .into_iter()
                        .map(|segment| match segment {
                            CamPathSegmentInput::Line { to_mm } => CamPathSegment2d::Line { to_mm },
                            CamPathSegmentInput::Arc {
                                to_mm,
                                center_mm,
                                clockwise,
                            } => CamPathSegment2d::Arc {
                                to_mm,
                                center_mm,
                                clockwise,
                            },
                        })
                        .collect(),
                },
                top_z_mm,
                bottom_z_mm,
                applied_radial_allowance_mm,
            },
            CamOperationInput::Drill {
                id,
                points_mm,
                top_z_mm,
                bottom_z_mm,
            } => Self::Drill {
                id,
                points_mm,
                top_z_mm,
                bottom_z_mm,
            },
            CamOperationInput::FacePocket { id, face_ordinal } => {
                Self::FacePocket { id, face_ordinal }
            }
            CamOperationInput::FaceContour {
                id,
                face_ordinal,
                side,
            } => Self::FaceContour {
                id,
                face_ordinal,
                side: match side {
                    CamContourSideInput::Outside => CamContourSide::Outside,
                    CamContourSideInput::Inside => CamContourSide::Inside,
                },
            },
            CamOperationInput::FaceDrill { id, face_ordinal } => {
                Self::FaceDrill { id, face_ordinal }
            }
        }
    }
}

pub(super) fn cam_review_request(p: &Map<String, Value>) -> Result<CamReviewRequest> {
    let operations: Vec<CamOperationInput> = serde_json::from_value(
        p.get("operations")
            .cloned()
            .ok_or_else(|| Error::invalid("missing operations"))?,
    )
    .map_err(|error| Error::invalid_because(&error))?;
    let fixtures: Vec<CamFixtureInput> = serde_json::from_value(
        p.get("fixtures")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| Error::invalid_because(&error))?;
    let dialect = CamPostprocessorDialect::library(string(p, "dialect")?)
        .map_err(|error| Error::invalid_because(&error))?;
    Ok(CamReviewRequest {
        plan_id: CamPlanId(uint(p, "plan_id")?),
        operations: operations.into_iter().map(Into::into).collect(),
        fixtures: fixtures
            .into_iter()
            .map(|fixture| CamFixture {
                id: fixture.id,
                minimum_mm: fixture.minimum_mm,
                maximum_mm: fixture.maximum_mm,
            })
            .collect(),
        dialect,
    })
}
