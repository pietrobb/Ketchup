//! The wire form of a CAM simulation the exact worker runs: the request with its
//! cutter, motions and fixtures, and the evidence it answers.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireRequest {
    pub schema: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub target_exact_graph_digest: String,
    pub stock_bounds_mm: [[f64; 3]; 2],
    pub setup_to_world: [f64; 16],
    pub cutter_radius_mm: f64,
    pub cutter_length_mm: f64,
    /// The cutting end: 0 flat, 1 a drill's point `cutter_tip_length_mm` long, 2 a ball.
    pub cutter_tip: u8,
    pub cutter_tip_length_mm: f64,
    pub tool_length_mm: f64,
    pub holder_radius_mm: f64,
    pub holder_offset_mm: f64,
    pub holder_length_mm: f64,
    pub motions: Vec<CamSimulationWireMotion>,
    pub fixtures: Vec<CamSimulationWireFixture>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireMotion {
    pub kind: u8,
    pub path_kind: u8,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub center_mm: [f64; 3],
    pub clockwise: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireFixture {
    pub id: u64,
    pub bounds_mm: [[f64; 3]; 2],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireEvidence {
    pub schema: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub target_exact_graph_digest: String,
    pub motion_count: usize,
    pub cutting_motion_count: usize,
    pub fixture_count: usize,
    pub stock_before_mm3: f64,
    pub stock_after_mm3: f64,
    pub removed_stock_mm3: f64,
    pub residual_stock_mm3: f64,
    pub gouge_mm3: f64,
    pub collisions: Vec<CamSimulationWireCollision>,
    pub backend: String,
    pub tolerance: String,
    pub result_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireCollision {
    pub motion_index: usize,
    pub motion_kind: u8,
    pub participant: u8,
    pub target: u8,
    pub fixture_id: Option<u64>,
    pub common_volume_mm3: f64,
    pub contact_area_mm2: f64,
    pub distance_mm: f64,
}
