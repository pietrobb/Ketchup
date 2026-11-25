//! The parts holding a member sideways and how their paths to the ground hold it.

use serde::{Deserialize, Serialize};

/// A part holding a member sideways at one edge (review 2026-11-02 PG-120,
/// 2026-11-04 PG-140 to PG-142, r19 PG-160 to PG-163): it bears on the member
/// and on a part declared grounded across the member, and is fastened to both
/// by a joint() of a kind that fastens. It holds the member at its fasteners
/// where the two touch, or at the middle of each place they touch when the
/// joint names none; `from_mm` equals `to_mm` then.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Brace {
    pub part: String,
    /// `top` (compressed under a sagging moment) or `bottom` (under hogging).
    pub edge: String,
    pub from_mm: f64,
    pub to_mm: f64,
    /// How its path to the ground holds the member sideways, from the member
    /// through the brace (a diaphragm's sheet through all its anchors), the
    /// weakest of the stretch; none where no path reaches the ground.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchorage: Option<BraceAnchorage>,
}

/// How stiffly and up to what force a brace's path to the ground holds the
/// member sideways (review 2026-11-13 r95 structural point 1), which the
/// member's check holds against what EN 1995-1-1 9.2.5.3 asks of a brace.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct BraceAnchorage {
    /// The part of the path, with the joint it leaves by, giving the most.
    pub through: String,
    /// The force across the member per millimetre it moves there, N/mm.
    pub stiffness_n_mm: f64,
    /// The force across the member a part of the path buckles under, N: k_c f_c,0,k A here,
    /// k_c f_c,0,d A in a member check's `held_by`; none where no part carries it along its length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_n: Option<f64>,
    /// The part of the path that buckles first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buckling_in: Option<String>,
    /// In a member check's `held_by`, the ultimate combination closest to
    /// buckling the path (review 2026-11-25 R1125og-PG-1001: each combination
    /// asks its own F_d of a path buckling at its own k_mod)...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combination: Option<String>,
    /// ...its k_mod for the part buckling...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_mod: Option<f64>,
    /// ...and the force F_d it asks of the brace, N.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_n: Option<f64>,
}
