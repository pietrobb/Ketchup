#![forbid(unsafe_code)]

use crate::document::{
    BOTTLE_SHELL_OPENING_FACE_ROLE, BOTTLE_SHOULDER_EDGE_ROLE, DefinitionId, DocumentId,
    EdgeFinishKind, FeatureId, FeatureKind, ProfileSegment, Snapshot,
};
use crate::exact_product::{
    BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ExactFaceRole, ExactProductError,
    ExactProfileSegment, ExactTriangle, ExactVertex, canonical_reference_lineage_digest,
};
use sha2::{Digest, Sha256};

pub const EXACT_REVOLVE_SCHEMA_V1: &str = "ketchup.exact-revolve.v1";
pub const EXACT_REVOLVE_EVALUATOR_V1: &str = "ketchup.exact-revolve-evaluator.v1";
pub const EXACT_GENERAL_REVOLVE_SCHEMA_V1: &str = "ketchup.exact-general-revolve.v1";
pub const EXACT_GENERAL_REVOLVE_EVALUATOR_V1: &str = "ketchup.exact-general-revolve-evaluator.v1";
pub const EXACT_SHELL_SCHEMA_V1: &str = "ketchup.exact-shell.v1";
pub const EXACT_SHELL_EVALUATOR_V1: &str = "ketchup.exact-shell-evaluator.v1";
pub const EXACT_BOTTLE_FINISH_SCHEMA_V1: &str = "ketchup.exact-bottle-finish.v1";
pub const EXACT_BOTTLE_FINISH_EVALUATOR_V1: &str = "ketchup.exact-bottle-finish-evaluator.v1";
pub const REVOLVE_FACE_ROLES: [ExactFaceRole; 5] = [
    ExactFaceRole::RevolveBottom,
    ExactFaceRole::RevolveBody,
    ExactFaceRole::RevolveShoulder,
    ExactFaceRole::RevolveNeck,
    ExactFaceRole::RevolveMouth,
];
pub const GENERAL_REVOLVE_FULL_FACE_ROLES: [ExactFaceRole; 2] =
    [ExactFaceRole::RevolveSide0, ExactFaceRole::RevolveSide1];
pub const GENERAL_REVOLVE_PARTIAL_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::RevolveSide0,
    ExactFaceRole::RevolveSide1,
    ExactFaceRole::RevolveStart,
    ExactFaceRole::RevolveEnd,
];
pub const SHELL_FACE_ROLES: [ExactFaceRole; 9] = [
    ExactFaceRole::ShellOuterBottom,
    ExactFaceRole::ShellOuterBody,
    ExactFaceRole::ShellOuterShoulder,
    ExactFaceRole::ShellOuterNeck,
    ExactFaceRole::ShellRim,
    ExactFaceRole::ShellInnerBottom,
    ExactFaceRole::ShellInnerBody,
    ExactFaceRole::ShellInnerShoulder,
    ExactFaceRole::ShellInnerNeck,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactRevolveRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub control_feature_id: Option<FeatureId>,
    pub revolve_feature_id: FeatureId,
    pub shell_feature_id: Option<FeatureId>,
    pub thickness_bits: Option<u64>,
    pub edge_finish_feature_id: Option<FeatureId>,
    pub edge_finish_kind: Option<EdgeFinishKind>,
    pub edge_finish_amount_bits: Option<u64>,
    pub points_bits: Vec<[u64; 2]>,
    pub segments: Option<Vec<ExactProfileSegment>>,
    pub axis_start_bits: [u64; 2],
    pub axis_end_bits: [u64; 2],
    pub angle_degrees_bits: u64,
    pub general: bool,
    pub canonical_input_digest: String,
}

impl ExactRevolveRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        if let Some(request) = Self::try_from_general_snapshot(snapshot, definition_id)? {
            return Ok(request);
        }
        if !(2..=5).contains(&definition.feature_ids().len()) {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let mut profile = None;
        let mut control = None;
        let mut revolve = None;
        let mut shell = None;
        let mut finish = None;
        for feature_id in definition.feature_ids() {
            let feature = snapshot
                .feature(*feature_id)
                .ok_or(ExactProductError::UnsupportedDefinition)?;
            match feature.kind() {
                FeatureKind::Profile { points_mm } => {
                    if profile.replace((*feature_id, points_mm)).is_some() {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                }
                FeatureKind::BottleProfileControl {
                    profile,
                    body_radius,
                    body_height,
                    shoulder_rise,
                } => {
                    if control
                        .replace((
                            *feature_id,
                            *profile,
                            body_radius.millimetres(),
                            body_height.millimetres(),
                            shoulder_rise.millimetres(),
                        ))
                        .is_some()
                    {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                }
                FeatureKind::Revolve {
                    profile,
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                } => {
                    if *axis_start_mm != [0.0, 0.0]
                        || *axis_end_mm != [0.0, 1.0]
                        || *angle_degrees != 360.0
                        || revolve.replace((*feature_id, *profile)).is_some()
                    {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                }
                FeatureKind::Shell {
                    target,
                    removed_faces,
                    thickness,
                } => {
                    if removed_faces.len() != 1
                        || removed_faces[0].as_str() != BOTTLE_SHELL_OPENING_FACE_ROLE
                        || shell
                            .replace((*feature_id, *target, thickness.millimetres()))
                            .is_some()
                    {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                }
                FeatureKind::BottleEdgeFinish {
                    target,
                    edges,
                    kind,
                    amount,
                } => {
                    if edges.len() != 1
                        || edges[0].as_str() != BOTTLE_SHOULDER_EDGE_ROLE
                        || finish
                            .replace((*feature_id, *target, *kind, amount.millimetres()))
                            .is_some()
                    {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                }
                FeatureKind::Workplane(_)
                | FeatureKind::Sketch(_)
                | FeatureKind::SegmentProfile { .. }
                | FeatureKind::SplineProfile { .. }
                | FeatureKind::Extrusion { .. }
                | FeatureKind::Pad(_)
                | FeatureKind::SketchPocket(_)
                | FeatureKind::TopologyShell { .. }
                | FeatureKind::TopologyEdgeFinish { .. }
                | FeatureKind::TopologyFaceOffset { .. }
                | FeatureKind::ThroughCut { .. }
                | FeatureKind::Pocket { .. }
                | FeatureKind::Boolean { .. }
                | FeatureKind::PlanarOffset { .. }
                | FeatureKind::Sweep { .. }
                | FeatureKind::Loft { .. }
                | FeatureKind::ImportedExactBody(_)
                | FeatureKind::RigidTransform { .. }
                | FeatureKind::MeshBody(_) => {
                    return Err(ExactProductError::UnsupportedDefinition);
                }
            }
        }
        let Some((profile_feature_id, source_points)) = profile else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let (control_feature_id, points_mm, revolve_profile_id) = match control {
            None => (None, source_points.to_vec(), profile_feature_id),
            Some((control_id, source_id, body_radius, body_height, shoulder_rise))
                if source_id == profile_feature_id =>
            {
                (
                    Some(control_id),
                    controlled_bottle_profile(
                        source_points,
                        body_radius,
                        body_height,
                        shoulder_rise,
                    )?,
                    control_id,
                )
            }
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
        let Some((revolve_feature_id, actual_revolve_profile_id)) = revolve else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        if actual_revolve_profile_id != revolve_profile_id || !is_bounded_bottle_profile(&points_mm)
        {
            return Err(ExactProductError::UnsupportedProfile);
        }
        let (shell_feature_id, thickness_bits) = match shell {
            None => (None, None),
            Some((shell_feature_id, target, thickness_mm)) if target == revolve_feature_id => {
                inner_shell_profile(&points_mm, thickness_mm)?;
                (Some(shell_feature_id), Some(thickness_mm.to_bits()))
            }
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
        let (edge_finish_feature_id, edge_finish_kind, edge_finish_amount_bits) = match finish {
            None => (None, None, None),
            Some((feature_id, target, kind, amount_mm))
                if Some(target) == shell_feature_id
                    && finish_amount_is_conservative(&points_mm, amount_mm) =>
            {
                (Some(feature_id), Some(kind), Some(amount_mm.to_bits()))
            }
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
        let expected_feature_count = 2
            + usize::from(control_feature_id.is_some())
            + usize::from(shell_feature_id.is_some())
            + usize::from(edge_finish_feature_id.is_some());
        if definition.feature_ids().len() != expected_feature_count {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let points_bits = points_mm
            .iter()
            .map(|point| [point[0].to_bits(), point[1].to_bits()])
            .collect::<Vec<_>>();
        let source_digest = snapshot.canonical_digest();
        let schema = if edge_finish_feature_id.is_some() {
            EXACT_BOTTLE_FINISH_SCHEMA_V1
        } else if shell_feature_id.is_some() {
            EXACT_SHELL_SCHEMA_V1
        } else {
            EXACT_REVOLVE_SCHEMA_V1
        };
        let mut canonical = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            schema,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            profile_feature_id.0,
            control_feature_id.map_or(0, |id| id.0),
            revolve_feature_id.0,
            shell_feature_id.map_or(0, |id| id.0),
            thickness_bits.map_or(0, |bits| bits),
            edge_finish_feature_id.map_or(0, |id| id.0),
            edge_finish_kind.map_or(0, |kind| match kind {
                EdgeFinishKind::Fillet => 1,
                EdgeFinishKind::Chamfer => 2,
            }),
            edge_finish_amount_bits.map_or(0, |bits| bits),
            source_digest,
            points_bits.len()
        );
        for point in &points_bits {
            canonical.push_str(&format!(":{:016x}:{:016x}", point[0], point[1]));
        }
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id,
            control_feature_id,
            revolve_feature_id,
            shell_feature_id,
            thickness_bits,
            edge_finish_feature_id,
            edge_finish_kind,
            edge_finish_amount_bits,
            points_bits,
            segments: None,
            axis_start_bits: [0.0_f64.to_bits(), 0.0_f64.to_bits()],
            axis_end_bits: [0.0_f64.to_bits(), 1.0_f64.to_bits()],
            angle_degrees_bits: 360.0_f64.to_bits(),
            general: false,
            canonical_input_digest: sha256(&canonical),
        })
    }

    fn try_from_general_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Option<Self>, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        if definition.feature_ids().len() != 2 {
            return Ok(None);
        }
        let mut profile = None;
        let mut revolve = None;
        for feature_id in definition.feature_ids() {
            let feature = snapshot
                .feature(*feature_id)
                .ok_or(ExactProductError::UnsupportedDefinition)?;
            match feature.kind() {
                FeatureKind::Profile { points_mm } => {
                    profile = Some((*feature_id, points_mm.to_vec(), None));
                }
                FeatureKind::SegmentProfile { segments, closed } if *closed => {
                    if segments
                        .iter()
                        .any(|segment| matches!(segment, ProfileSegment::CubicBezier { .. }))
                    {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                    let exact_segments = segments
                        .iter()
                        .map(|segment| match segment {
                            ProfileSegment::Line { start_mm, end_mm } => {
                                ExactProfileSegment::Line {
                                    start_bits: start_mm.map(f64::to_bits),
                                    end_bits: end_mm.map(f64::to_bits),
                                }
                            }
                            ProfileSegment::CircularArc {
                                start_mm,
                                end_mm,
                                center_mm,
                                clockwise,
                            } => ExactProfileSegment::CircularArc {
                                start_bits: start_mm.map(f64::to_bits),
                                end_bits: end_mm.map(f64::to_bits),
                                center_bits: center_mm.map(f64::to_bits),
                                clockwise: *clockwise,
                            },
                            ProfileSegment::CubicBezier { .. } => unreachable!(),
                        })
                        .collect::<Vec<_>>();
                    let points = segments
                        .iter()
                        .map(ProfileSegment::start_mm)
                        .collect::<Vec<_>>();
                    profile = Some((*feature_id, points, Some(exact_segments)));
                }
                FeatureKind::Revolve {
                    profile,
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                } => {
                    revolve = Some((
                        *feature_id,
                        *profile,
                        *axis_start_mm,
                        *axis_end_mm,
                        *angle_degrees,
                    ));
                }
                _ => return Ok(None),
            }
        }
        let Some((profile_feature_id, points_mm, segments)) = profile else {
            return Ok(None);
        };
        let Some((revolve_feature_id, revolve_profile_id, axis_start_mm, axis_end_mm, angle)) =
            revolve
        else {
            return Ok(None);
        };
        if revolve_profile_id != profile_feature_id
            || !(2..=64).contains(&points_mm.len())
            || segments.is_none()
                && axis_start_mm == [0.0, 0.0]
                && axis_end_mm == [0.0, 1.0]
                && angle == 360.0
                && is_bounded_bottle_profile(&points_mm)
        {
            return Ok(None);
        }
        let points_bits = points_mm
            .iter()
            .map(|point| point.map(f64::to_bits))
            .collect::<Vec<_>>();
        let source_digest = snapshot.canonical_digest();
        let axis_start_bits = axis_start_mm.map(f64::to_bits);
        let axis_end_bits = axis_end_mm.map(f64::to_bits);
        let angle_degrees_bits = angle.to_bits();
        let mut canonical = format!(
            "{}:{}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
            EXACT_GENERAL_REVOLVE_SCHEMA_V1,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            profile_feature_id.0,
            revolve_feature_id.0,
            points_bits.len(),
            axis_start_bits[0],
            axis_start_bits[1],
            axis_end_bits[0],
            axis_end_bits[1],
            angle_degrees_bits,
            source_digest,
        );
        if let Some(segments) = &segments {
            for segment in segments {
                match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => canonical.push_str(&format!(
                        ":L:{:016x}:{:016x}:{:016x}:{:016x}",
                        start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                    )),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => canonical.push_str(&format!(
                        ":A:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                        start_bits[0],
                        start_bits[1],
                        end_bits[0],
                        end_bits[1],
                        center_bits[0],
                        center_bits[1],
                        clockwise,
                    )),
                }
            }
        } else {
            for point in &points_bits {
                canonical.push_str(&format!(":P:{:016x}:{:016x}", point[0], point[1]));
            }
        }
        Ok(Some(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id,
            control_feature_id: None,
            revolve_feature_id,
            shell_feature_id: None,
            thickness_bits: None,
            edge_finish_feature_id: None,
            edge_finish_kind: None,
            edge_finish_amount_bits: None,
            points_bits,
            segments,
            axis_start_bits,
            axis_end_bits,
            angle_degrees_bits,
            general: true,
            canonical_input_digest: sha256(&canonical),
        }))
    }

    #[must_use]
    pub fn points_mm(&self) -> Vec<[f64; 2]> {
        self.points_bits
            .iter()
            .map(|point| [f64::from_bits(point[0]), f64::from_bits(point[1])])
            .collect()
    }

    #[must_use]
    pub fn profile_segments(&self) -> Vec<ExactProfileSegment> {
        self.segments.clone().unwrap_or_else(|| {
            self.points_bits
                .iter()
                .enumerate()
                .map(|(index, start)| ExactProfileSegment::Line {
                    start_bits: *start,
                    end_bits: self.points_bits[(index + 1) % self.points_bits.len()],
                })
                .collect()
        })
    }

    #[must_use]
    pub fn axis_start_mm(&self) -> [f64; 2] {
        self.axis_start_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn axis_end_mm(&self) -> [f64; 2] {
        self.axis_end_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn angle_degrees(&self) -> f64 {
        f64::from_bits(self.angle_degrees_bits)
    }

    #[must_use]
    pub fn thickness_mm(&self) -> Option<f64> {
        self.thickness_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        self.edge_finish_feature_id
            .or(self.shell_feature_id)
            .unwrap_or(self.revolve_feature_id)
    }

    #[must_use]
    pub fn schema(&self) -> &'static str {
        if self.general {
            EXACT_GENERAL_REVOLVE_SCHEMA_V1
        } else if self.edge_finish_feature_id.is_some() {
            EXACT_BOTTLE_FINISH_SCHEMA_V1
        } else if self.shell_feature_id.is_some() {
            EXACT_SHELL_SCHEMA_V1
        } else {
            EXACT_REVOLVE_SCHEMA_V1
        }
    }

    #[must_use]
    pub fn evaluator(&self) -> &'static str {
        if self.general {
            EXACT_GENERAL_REVOLVE_EVALUATOR_V1
        } else if self.edge_finish_feature_id.is_some() {
            EXACT_BOTTLE_FINISH_EVALUATOR_V1
        } else if self.shell_feature_id.is_some() {
            EXACT_SHELL_EVALUATOR_V1
        } else {
            EXACT_REVOLVE_EVALUATOR_V1
        }
    }

    #[must_use]
    pub fn canonical_input_digest_for_envelope(
        &self,
        source_revision: u64,
        source_digest: &str,
    ) -> String {
        let mut canonical = if self.general {
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                self.schema(),
                self.document_id.0,
                source_revision,
                self.definition_id.0,
                self.profile_feature_id.0,
                self.revolve_feature_id.0,
                self.points_bits.len(),
                self.axis_start_bits[0],
                self.axis_start_bits[1],
                self.axis_end_bits[0],
                self.axis_end_bits[1],
                self.angle_degrees_bits,
                source_digest,
            )
        } else {
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                self.schema(),
                self.document_id.0,
                source_revision,
                self.definition_id.0,
                self.profile_feature_id.0,
                self.control_feature_id.map_or(0, |id| id.0),
                self.revolve_feature_id.0,
                self.shell_feature_id.map_or(0, |id| id.0),
                self.thickness_bits.map_or(0, |bits| bits),
                self.edge_finish_feature_id.map_or(0, |id| id.0),
                self.edge_finish_kind.map_or(0, |kind| match kind {
                    EdgeFinishKind::Fillet => 1,
                    EdgeFinishKind::Chamfer => 2,
                }),
                self.edge_finish_amount_bits.map_or(0, |bits| bits),
                source_digest,
                self.points_bits.len(),
            )
        };
        if self.general {
            if let Some(segments) = &self.segments {
                for segment in segments {
                    match segment {
                        ExactProfileSegment::Line {
                            start_bits,
                            end_bits,
                        } => canonical.push_str(&format!(
                            ":L:{:016x}:{:016x}:{:016x}:{:016x}",
                            start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                        )),
                        ExactProfileSegment::CircularArc {
                            start_bits,
                            end_bits,
                            center_bits,
                            clockwise,
                        } => canonical.push_str(&format!(
                            ":A:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                            start_bits[0],
                            start_bits[1],
                            end_bits[0],
                            end_bits[1],
                            center_bits[0],
                            center_bits[1],
                            clockwise,
                        )),
                    }
                }
            } else {
                for point in &self.points_bits {
                    canonical.push_str(&format!(":P:{:016x}:{:016x}", point[0], point[1]));
                }
            }
        } else {
            for point in &self.points_bits {
                canonical.push_str(&format!(":{:016x}:{:016x}", point[0], point[1]));
            }
        }
        sha256(&canonical)
    }

    #[must_use]
    pub fn face_roles(&self) -> &'static [ExactFaceRole] {
        if self.general && self.angle_degrees() < 360.0 {
            &GENERAL_REVOLVE_PARTIAL_FACE_ROLES
        } else if self.general {
            &GENERAL_REVOLVE_FULL_FACE_ROLES
        } else if self.shell_feature_id.is_some() {
            &SHELL_FACE_ROLES
        } else {
            &REVOLVE_FACE_ROLES
        }
    }
}

#[must_use]
pub fn reference_matches_revolve_request(
    reference: &BodySubshapeRef,
    request: &ExactRevolveRequest,
) -> bool {
    reference
        .role()
        .is_some_and(|role| request.face_roles().contains(&role))
        && reference.has_valid_lineage()
        && reference.document_id == request.document_id
        && reference.definition_id == request.definition_id
        && reference.profile_feature_id == request.profile_feature_id
        && reference.producer_feature_id == request.producer_feature_id()
        && reference.canonical_input_digest == request.canonical_input_digest
        && reference.evaluator == request.evaluator()
        && !reference.exact_input_digest.is_empty()
        && !reference.result_fingerprint.is_empty()
        && !reference.backend.is_empty()
        && !reference.tolerance.is_empty()
        && !reference.corroborating_geometry_fingerprint.is_empty()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevolveResultIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub control_feature_id: Option<FeatureId>,
    pub revolve_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub shell_feature_id: Option<FeatureId>,
    pub thickness_bits: Option<u64>,
    pub edge_finish_feature_id: Option<FeatureId>,
    pub edge_finish_kind: Option<EdgeFinishKind>,
    pub edge_finish_amount_bits: Option<u64>,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactRevolvePackage {
    pub identity: RevolveResultIdentity,
    pub bounds_mm: [[f64; 3]; 2],
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub references: Vec<BodySubshapeRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BottleAuthorityReport {
    pub canonical_authority: &'static str,
    pub evaluated_authority: &'static str,
    pub render_representation: &'static str,
    pub conversion_loss: &'static str,
    pub current: bool,
    pub validation_passed: bool,
    pub durable_reference_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BottleExportBundle {
    pub exact_recipe: Vec<u8>,
    pub mesh_obj: Vec<u8>,
    pub mesh_loss_report: String,
}

impl ExactRevolvePackage {
    #[must_use]
    pub fn authority_report(&self, snapshot: &Snapshot) -> BottleAuthorityReport {
        let request = ExactRevolveRequest::from_snapshot(snapshot, self.identity.definition_id);
        let validation_passed = request
            .as_ref()
            .is_ok_and(|request| self.validate_for_request(request).is_ok());
        BottleAuthorityReport {
            canonical_authority: "canonical bottle feature recipe",
            evaluated_authority: "accepted exact OCCT B-Rep",
            render_representation: "derived triangle mesh",
            conversion_loss: "render tessellation does not preserve exact topology or editability",
            current: self.is_current(snapshot),
            validation_passed,
            durable_reference_count: self.references.len(),
        }
    }

    pub fn export_bundle(
        &self,
        snapshot: &Snapshot,
    ) -> Result<BottleExportBundle, ExactProductError> {
        let request = ExactRevolveRequest::from_snapshot(snapshot, self.identity.definition_id)?;
        if !self.is_current(snapshot) {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        self.validate_for_request(&request)?;

        let mut exact_recipe = format!(
            "KETCHUP_EXACT_BOTTLE_RECIPE_V1\ndocument={}\nrevision={}\nsource_digest={}\ndefinition={}\nprofile_feature={}\ncontrol_feature={}\nrevolve_feature={}\nshell_feature={}\nthickness_bits={}\nfinish_feature={}\nfinish_kind={}\nfinish_amount_bits={}\ncanonical_input_digest={}\nexact_input_digest={}\nresult_fingerprint={}\nevaluator={}\nbackend={}\ntolerance={}\n",
            request.document_id.0,
            request.source_revision,
            request.source_digest,
            request.definition_id.0,
            request.profile_feature_id.0,
            request.control_feature_id.map_or(0, |id| id.0),
            request.revolve_feature_id.0,
            request.shell_feature_id.map_or(0, |id| id.0),
            request.thickness_bits.map_or(0, |bits| bits),
            request.edge_finish_feature_id.map_or(0, |id| id.0),
            request.edge_finish_kind.map_or("none", |kind| match kind {
                EdgeFinishKind::Fillet => "fillet",
                EdgeFinishKind::Chamfer => "chamfer",
            }),
            request.edge_finish_amount_bits.map_or(0, |bits| bits),
            request.canonical_input_digest,
            self.identity.exact_input_digest,
            self.identity.result_fingerprint,
            self.identity.evaluator,
            self.identity.backend,
            self.identity.tolerance,
        );
        for point in &request.points_bits {
            exact_recipe.push_str(&format!("point={:016x},{:016x}\n", point[0], point[1]));
        }

        let mesh_loss_report = format!(
            "authority=accepted exact OCCT B-Rep\nconversion=exact-to-mesh\nloss=exact topology, analytic surfaces, feature editability, and durable face identity are not preserved by OBJ\ntolerance={}\nsource_digest={}\nresult_fingerprint={}\n",
            self.identity.tolerance, self.identity.source_digest, self.identity.result_fingerprint
        );
        let mut mesh_obj = format!(
            "# Ketchup bottle OBJ\n# {}# canonical_authority=canonical bottle feature recipe\n",
            mesh_loss_report.replace('\n', "\n# ")
        );
        for vertex in &self.vertices {
            mesh_obj.push_str(&format!(
                "v {:.17} {:.17} {:.17}\n",
                vertex.position_mm[0], vertex.position_mm[1], vertex.position_mm[2]
            ));
        }
        let mut last_role = None;
        for triangle in &self.triangles {
            if triangle.face_role != last_role {
                let role = triangle
                    .face_role
                    .map_or("unclassified", ExactFaceRole::semantic_role);
                mesh_obj.push_str(&format!("g {role}\n"));
                last_role = triangle.face_role;
            }
            mesh_obj.push_str(&format!(
                "f {} {} {}\n",
                triangle.vertex_indices[0] + 1,
                triangle.vertex_indices[1] + 1,
                triangle.vertex_indices[2] + 1
            ));
        }

        Ok(BottleExportBundle {
            exact_recipe: exact_recipe.into_bytes(),
            mesh_obj: mesh_obj.into_bytes(),
            mesh_loss_report,
        })
    }

    #[must_use]
    pub fn reference(&self, role: ExactFaceRole) -> Option<&BodySubshapeRef> {
        self.references
            .iter()
            .find(|reference| reference.role() == Some(role))
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactRevolveRequest::from_snapshot(snapshot, self.identity.definition_id)
                .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactRevolveRequest,
    ) -> Result<(), ExactProductError> {
        let mut actual_roles = self
            .references
            .iter()
            .map(BodySubshapeRef::role)
            .collect::<Option<Vec<_>>>()
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        actual_roles.sort_unstable();
        let mut expected_roles = request.face_roles().to_vec();
        expected_roles.sort_unstable();
        let references_valid = self.references.iter().all(|reference| {
            let Some(role) = reference.role() else {
                return false;
            };
            request.face_roles().contains(&role)
                && reference_matches_revolve_request(reference, request)
                && reference.exact_input_digest == self.identity.exact_input_digest
                && reference.result_fingerprint == self.identity.result_fingerprint
                && reference.backend == self.identity.backend
                && reference.tolerance == self.identity.tolerance
                && reference.lineage_digest
                    == canonical_reference_lineage_digest(
                        request.document_id,
                        request.producer_feature_id(),
                        role.semantic_role(),
                        role.source_element_id(),
                        role.expected_type(),
                    )
        });
        let triangles_valid = !self.vertices.is_empty()
            && !self.triangles.is_empty()
            && self.triangles.iter().all(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .all(|index| (*index as usize) < self.vertices.len())
                    && triangle
                        .face_role
                        .is_some_and(|role| request.face_roles().contains(&role))
            })
            && request.face_roles().iter().all(|role| {
                self.triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(*role))
            });
        if self.identity.schema != request.schema()
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.definition_id != request.definition_id
            || self.identity.profile_feature_id != request.profile_feature_id
            || self.identity.control_feature_id != request.control_feature_id
            || self.identity.revolve_feature_id != request.revolve_feature_id
            || self.identity.producer_feature_id != request.producer_feature_id()
            || self.identity.shell_feature_id != request.shell_feature_id
            || self.identity.thickness_bits != request.thickness_bits
            || self.identity.edge_finish_feature_id != request.edge_finish_feature_id
            || self.identity.edge_finish_kind != request.edge_finish_kind
            || self.identity.edge_finish_amount_bits != request.edge_finish_amount_bits
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != request.evaluator()
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || self
                .bounds_mm
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            || self.references.len() != request.face_roles().len()
            || actual_roles != expected_roles
            || !references_valid
            || !triangles_valid
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

pub fn build_revolve_package(
    request: &ExactRevolveRequest,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    worker_bounds_mm: [[f64; 3]; 2],
    face_evidence: Vec<(ExactFaceRole, String, String)>,
) -> Result<ExactRevolvePackage, ExactProductError> {
    if request.general {
        return build_general_revolve_package(
            request,
            exact_input_digest,
            result_fingerprint,
            backend,
            tolerance,
            worker_bounds_mm,
            face_evidence,
        );
    }
    let points = request.points_mm();
    let max_radius = points.iter().map(|point| point[0]).fold(0.0_f64, f64::max);
    let expected_bounds = [
        [-max_radius, -max_radius, points[0][1]],
        [max_radius, max_radius, points[5][1]],
    ];
    let bounds_valid = worker_bounds_mm
        .iter()
        .flatten()
        .zip(expected_bounds.iter().flatten())
        .all(|(actual, expected)| actual.is_finite() && (actual - expected).abs() <= 1.0e-6);
    let mut roles = face_evidence
        .iter()
        .map(|(role, _, _)| *role)
        .collect::<Vec<_>>();
    roles.sort_unstable();
    let mut expected_roles = request.face_roles().to_vec();
    expected_roles.sort_unstable();
    let evidence_valid = roles == expected_roles
        && face_evidence.iter().all(|(role, lineage, geometry)| {
            *lineage
                == canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                )
                && !geometry.is_empty()
        });
    if !bounds_valid
        || !evidence_valid
        || exact_input_digest.is_empty()
        || result_fingerprint.is_empty()
        || backend.is_empty()
        || tolerance.is_empty()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }

    let (vertices, triangles) = if let Some(thickness_mm) = request.thickness_mm() {
        shell_render_mesh(&points, thickness_mm)?
    } else {
        revolve_render_mesh(&points)
    };
    let references = face_evidence
        .into_iter()
        .map(|(role, _, geometry)| {
            build_revolve_reference(
                request,
                role,
                exact_input_digest.clone(),
                result_fingerprint.clone(),
                backend.clone(),
                tolerance.clone(),
                geometry,
            )
        })
        .collect();
    let package = ExactRevolvePackage {
        identity: RevolveResultIdentity {
            schema: request.schema().to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            control_feature_id: request.control_feature_id,
            revolve_feature_id: request.revolve_feature_id,
            producer_feature_id: request.producer_feature_id(),
            shell_feature_id: request.shell_feature_id,
            thickness_bits: request.thickness_bits,
            edge_finish_feature_id: request.edge_finish_feature_id,
            edge_finish_kind: request.edge_finish_kind,
            edge_finish_amount_bits: request.edge_finish_amount_bits,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest,
            result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend,
            tolerance,
        },
        bounds_mm: worker_bounds_mm,
        vertices,
        triangles,
        references,
    };
    package.validate_for_request(request)?;
    Ok(package)
}

fn build_general_revolve_package(
    request: &ExactRevolveRequest,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    worker_bounds_mm: [[f64; 3]; 2],
    face_evidence: Vec<(ExactFaceRole, String, String)>,
) -> Result<ExactRevolvePackage, ExactProductError> {
    let bounds_valid = worker_bounds_mm
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        && (0..3).all(|axis| worker_bounds_mm[0][axis] < worker_bounds_mm[1][axis]);
    let mut roles = face_evidence
        .iter()
        .map(|(role, _, _)| *role)
        .collect::<Vec<_>>();
    roles.sort_unstable();
    let mut expected_roles = request.face_roles().to_vec();
    expected_roles.sort_unstable();
    let evidence_valid = roles == expected_roles
        && face_evidence.iter().all(|(role, lineage, geometry)| {
            *lineage
                == canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                )
                && !geometry.is_empty()
        });
    if !bounds_valid
        || !evidence_valid
        || exact_input_digest.is_empty()
        || result_fingerprint.is_empty()
        || backend.is_empty()
        || tolerance.is_empty()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let [min, max] = worker_bounds_mm;
    let vertices = vec![
        ExactVertex {
            position_mm: [min[0], min[1], min[2]],
        },
        ExactVertex {
            position_mm: [max[0], min[1], min[2]],
        },
        ExactVertex {
            position_mm: [max[0], max[1], min[2]],
        },
        ExactVertex {
            position_mm: [min[0], max[1], min[2]],
        },
        ExactVertex {
            position_mm: [min[0], min[1], max[2]],
        },
        ExactVertex {
            position_mm: [max[0], min[1], max[2]],
        },
        ExactVertex {
            position_mm: [max[0], max[1], max[2]],
        },
        ExactVertex {
            position_mm: [min[0], max[1], max[2]],
        },
    ];
    let triangle_indices = [[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]];
    let triangles = request
        .face_roles()
        .iter()
        .enumerate()
        .map(|(index, role)| ExactTriangle {
            vertex_indices: triangle_indices[index],
            face_role: Some(*role),
        })
        .collect::<Vec<_>>();
    let references = face_evidence
        .into_iter()
        .map(|(role, _, geometry)| {
            build_revolve_reference(
                request,
                role,
                exact_input_digest.clone(),
                result_fingerprint.clone(),
                backend.clone(),
                tolerance.clone(),
                geometry,
            )
        })
        .collect();
    let package = ExactRevolvePackage {
        identity: RevolveResultIdentity {
            schema: request.schema().to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            control_feature_id: None,
            revolve_feature_id: request.revolve_feature_id,
            producer_feature_id: request.producer_feature_id(),
            shell_feature_id: None,
            thickness_bits: None,
            edge_finish_feature_id: None,
            edge_finish_kind: None,
            edge_finish_amount_bits: None,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest,
            result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend,
            tolerance,
        },
        bounds_mm: worker_bounds_mm,
        vertices,
        triangles,
        references,
    };
    package.validate_for_request(request)?;
    Ok(package)
}

#[must_use]
pub fn build_revolve_reference(
    request: &ExactRevolveRequest,
    role: ExactFaceRole,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    corroborating_geometry_fingerprint: String,
) -> BodySubshapeRef {
    BodySubshapeRef {
        schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
        document_id: request.document_id,
        definition_id: request.definition_id,
        profile_feature_id: request.profile_feature_id,
        producer_feature_id: request.producer_feature_id(),
        semantic_role: role.semantic_role().to_owned(),
        source_element_id: role.source_element_id().to_owned(),
        expected_type: role.expected_type().to_owned(),
        expected_cardinality: 1,
        stability: crate::exact_product::ReferenceStability::Guaranteed,
        canonical_input_digest: request.canonical_input_digest.clone(),
        exact_input_digest,
        result_fingerprint,
        evaluator: request.evaluator().to_owned(),
        backend,
        tolerance,
        lineage_digest: canonical_reference_lineage_digest(
            request.document_id,
            request.producer_feature_id(),
            role.semantic_role(),
            role.source_element_id(),
            role.expected_type(),
        ),
        corroborating_geometry_fingerprint,
    }
}

#[must_use]
pub fn expected_volume_mm3(request: &ExactRevolveRequest) -> Option<f64> {
    if request.edge_finish_feature_id.is_some() {
        return None;
    }
    let points = request.points_mm();
    let outer = revolved_profile_volume(&points);
    request.thickness_mm().map_or(Some(outer), |thickness_mm| {
        let inner = inner_shell_profile(&points, thickness_mm).ok()?;
        Some(outer - revolved_profile_volume(&inner))
    })
}

pub fn controlled_bottle_profile(
    points: &[[f64; 2]],
    body_radius_mm: f64,
    body_height_mm: f64,
    shoulder_rise_mm: f64,
) -> Result<Vec<[f64; 2]>, ExactProductError> {
    if !is_bounded_bottle_profile(points)
        || !body_radius_mm.is_finite()
        || !body_height_mm.is_finite()
        || !shoulder_rise_mm.is_finite()
        || body_radius_mm <= points[3][0]
        || body_height_mm <= 0.0
        || shoulder_rise_mm <= 0.0
    {
        return Err(ExactProductError::UnsupportedProfile);
    }
    let base_z = points[0][1];
    let neck_height = points[4][1] - points[3][1];
    let body_top_z = base_z + body_height_mm;
    let shoulder_top_z = body_top_z + shoulder_rise_mm;
    let top_z = shoulder_top_z + neck_height;
    let controlled = vec![
        [0.0, base_z],
        [body_radius_mm, base_z],
        [body_radius_mm, body_top_z],
        [points[3][0], shoulder_top_z],
        [points[4][0], top_z],
        [0.0, top_z],
    ];
    if neck_height <= 0.0 || !is_bounded_bottle_profile(&controlled) {
        return Err(ExactProductError::UnsupportedProfile);
    }
    Ok(controlled)
}

#[must_use]
pub fn finish_amount_is_conservative(points: &[[f64; 2]], amount_mm: f64) -> bool {
    is_bounded_bottle_profile(points)
        && amount_mm.is_finite()
        && amount_mm > 0.0
        && amount_mm < (points[3][0] - points[2][0]).hypot(points[3][1] - points[2][1]) * 0.25
        && amount_mm < points[3][0] * 0.25
}

pub fn inner_shell_profile(
    points: &[[f64; 2]],
    thickness_mm: f64,
) -> Result<[[f64; 2]; 6], ExactProductError> {
    if !is_bounded_bottle_profile(points) || !thickness_mm.is_finite() || thickness_mm <= 0.0 {
        return Err(ExactProductError::UnsupportedShell);
    }
    let body_radius = points[1][0];
    let neck_radius = points[4][0];
    let [shoulder_start_radius, shoulder_start_z] = points[2];
    let [shoulder_end_radius, shoulder_end_z] = points[3];
    let dr = shoulder_end_radius - shoulder_start_radius;
    let dz = shoulder_end_z - shoulder_start_z;
    let length = dr.hypot(dz);
    if body_radius != shoulder_start_radius
        || neck_radius != shoulder_end_radius
        || thickness_mm >= body_radius * 0.5
        || thickness_mm >= neck_radius * 0.5
        || thickness_mm >= length * 0.5
        || dr.abs() <= 1.0e-9
        || dz <= 1.0e-9
    {
        return Err(ExactProductError::UnsupportedShell);
    }
    let shifted_radius = shoulder_start_radius - dz / length * thickness_mm;
    let shifted_z = shoulder_start_z + dr / length * thickness_mm;
    let inner_body_radius = body_radius - thickness_mm;
    let inner_neck_radius = neck_radius - thickness_mm;
    let intersect_z = |radius: f64| shifted_z + (radius - shifted_radius) / dr * dz;
    let inner = [
        [0.0, points[0][1] + thickness_mm],
        [inner_body_radius, points[0][1] + thickness_mm],
        [inner_body_radius, intersect_z(inner_body_radius)],
        [inner_neck_radius, intersect_z(inner_neck_radius)],
        [inner_neck_radius, points[4][1]],
        [0.0, points[4][1]],
    ];
    if inner.iter().flatten().any(|value| !value.is_finite())
        || inner[1][1] >= inner[2][1]
        || inner[2][1] >= inner[3][1]
        || inner[3][1] >= inner[4][1]
    {
        return Err(ExactProductError::UnsupportedShell);
    }
    Ok(inner)
}

fn revolve_render_mesh(points: &[[f64; 2]]) -> (Vec<ExactVertex>, Vec<ExactTriangle>) {
    const SEGMENTS: usize = 32;
    let mut vertices = Vec::with_capacity(2 + 4 * SEGMENTS);
    vertices.push(ExactVertex {
        position_mm: [0.0, 0.0, points[0][1]],
    });
    append_rings(&mut vertices, &points[1..5], SEGMENTS);
    let top_center = vertices.len() as u32;
    vertices.push(ExactVertex {
        position_mm: [0.0, 0.0, points[5][1]],
    });
    let ring = |point_index: usize, segment: usize| -> u32 {
        (1 + (point_index - 1) * SEGMENTS + segment % SEGMENTS) as u32
    };
    let mut triangles = Vec::with_capacity(8 * SEGMENTS);
    for segment in 0..SEGMENTS {
        let next = (segment + 1) % SEGMENTS;
        triangles.push(ExactTriangle {
            vertex_indices: [0, ring(1, next), ring(1, segment)],
            face_role: Some(ExactFaceRole::RevolveBottom),
        });
        for (profile_segment, role) in (1..4).zip(REVOLVE_FACE_ROLES[1..4].iter().copied()) {
            append_quad(
                &mut triangles,
                [
                    ring(profile_segment, segment),
                    ring(profile_segment, next),
                    ring(profile_segment + 1, next),
                    ring(profile_segment + 1, segment),
                ],
                role,
            );
        }
        triangles.push(ExactTriangle {
            vertex_indices: [ring(4, segment), ring(4, next), top_center],
            face_role: Some(ExactFaceRole::RevolveMouth),
        });
    }
    (vertices, triangles)
}

fn shell_render_mesh(
    points: &[[f64; 2]],
    thickness_mm: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    const SEGMENTS: usize = 32;
    let inner = inner_shell_profile(points, thickness_mm)?;
    let mut vertices = Vec::with_capacity(2 + 8 * SEGMENTS);
    vertices.push(ExactVertex {
        position_mm: [0.0, 0.0, points[0][1]],
    });
    append_rings(&mut vertices, &points[1..5], SEGMENTS);
    let inner_center = vertices.len() as u32;
    vertices.push(ExactVertex {
        position_mm: [0.0, 0.0, inner[0][1]],
    });
    let inner_start = vertices.len();
    append_rings(&mut vertices, &inner[1..5], SEGMENTS);
    let outer_ring = |point_index: usize, segment: usize| -> u32 {
        (1 + (point_index - 1) * SEGMENTS + segment % SEGMENTS) as u32
    };
    let inner_ring = |point_index: usize, segment: usize| -> u32 {
        (inner_start + (point_index - 1) * SEGMENTS + segment % SEGMENTS) as u32
    };
    let mut triangles = Vec::with_capacity(16 * SEGMENTS);
    for segment in 0..SEGMENTS {
        let next = (segment + 1) % SEGMENTS;
        triangles.push(ExactTriangle {
            vertex_indices: [0, outer_ring(1, next), outer_ring(1, segment)],
            face_role: Some(ExactFaceRole::ShellOuterBottom),
        });
        for (profile_segment, role) in (1..4).zip([
            ExactFaceRole::ShellOuterBody,
            ExactFaceRole::ShellOuterShoulder,
            ExactFaceRole::ShellOuterNeck,
        ]) {
            append_quad(
                &mut triangles,
                [
                    outer_ring(profile_segment, segment),
                    outer_ring(profile_segment, next),
                    outer_ring(profile_segment + 1, next),
                    outer_ring(profile_segment + 1, segment),
                ],
                role,
            );
        }
        append_quad(
            &mut triangles,
            [
                outer_ring(4, segment),
                outer_ring(4, next),
                inner_ring(4, next),
                inner_ring(4, segment),
            ],
            ExactFaceRole::ShellRim,
        );
        for (profile_segment, role) in (1..4).zip([
            ExactFaceRole::ShellInnerBody,
            ExactFaceRole::ShellInnerShoulder,
            ExactFaceRole::ShellInnerNeck,
        ]) {
            append_quad(
                &mut triangles,
                [
                    inner_ring(profile_segment, segment),
                    inner_ring(profile_segment + 1, segment),
                    inner_ring(profile_segment + 1, next),
                    inner_ring(profile_segment, next),
                ],
                role,
            );
        }
        triangles.push(ExactTriangle {
            vertex_indices: [inner_center, inner_ring(1, segment), inner_ring(1, next)],
            face_role: Some(ExactFaceRole::ShellInnerBottom),
        });
    }
    Ok((vertices, triangles))
}

fn append_rings(vertices: &mut Vec<ExactVertex>, points: &[[f64; 2]], segments: usize) {
    for point in points {
        for segment in 0..segments {
            let angle = std::f64::consts::TAU * segment as f64 / segments as f64;
            vertices.push(ExactVertex {
                position_mm: [point[0] * angle.cos(), point[0] * angle.sin(), point[1]],
            });
        }
    }
}

fn append_quad(triangles: &mut Vec<ExactTriangle>, indices: [u32; 4], role: ExactFaceRole) {
    triangles.extend([
        ExactTriangle {
            vertex_indices: [indices[0], indices[1], indices[2]],
            face_role: Some(role),
        },
        ExactTriangle {
            vertex_indices: [indices[0], indices[2], indices[3]],
            face_role: Some(role),
        },
    ]);
}

fn revolved_profile_volume(points: &[[f64; 2]]) -> f64 {
    points
        .windows(2)
        .map(|edge| {
            let [r0, z0] = edge[0];
            let [r1, z1] = edge[1];
            std::f64::consts::PI * (z1 - z0) * (r0 * r0 + r0 * r1 + r1 * r1) / 3.0
        })
        .sum()
}

fn is_bounded_bottle_profile(points: &[[f64; 2]]) -> bool {
    points.len() == 6
        && points.iter().flatten().all(|value| value.is_finite())
        && points.first().is_some_and(|point| point[0] == 0.0)
        && points.last().is_some_and(|point| point[0] == 0.0)
        && points[1..points.len() - 1]
            .iter()
            .all(|point| point[0] > 0.0)
        && points.windows(2).all(|edge| edge[0][1] <= edge[1][1])
        && points[0][1] < points[2][1]
        && points[2][1] < points[3][1]
        && points[3][1] < points[4][1]
        && points[1][0] == points[2][0]
        && points[3][0] == points[4][0]
        && points[1][0] > points[3][0]
}

fn sha256(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
