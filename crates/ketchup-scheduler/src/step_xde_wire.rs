//! The parts and assembly nodes a worker reads from a STEP file with XDE.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepXdeRejectedPart {
    pub index: u32,
    pub name: String,
    pub reason: String,
}

/// A shape a read part left out: see `ketchup_exact::RejectedShape`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepXdeRejectedShape {
    pub ordinal: u32,
    pub kind: String,
    pub source_entity: Option<u32>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepXdeWorkerPart {
    pub index: u32,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub result_fingerprint: String,
    pub body_kind: String,
    pub solid_count: u32,
    pub topology_counts: [u32; 5],
    pub area_mm2: f64,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
    pub rejected_shapes: Vec<StepXdeRejectedShape>,
    /// See `ImportedExactBodySpec::enclosing_bounds_mm`; absent from a worker
    /// that does not find it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosing_bounds_mm: Option<[[f64; 3]; 2]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepXdeWorkerNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub transform: [f64; 16],
}
