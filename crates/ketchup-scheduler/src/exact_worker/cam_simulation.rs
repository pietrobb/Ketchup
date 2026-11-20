//! CAM simulation: cuts the tool's sweeps out of the stock with the exact kernel and
//! reports gouges and tool, holder and fixture collisions.
use super::{
    AxialToolMotion, AxialToolSweepSpec, AxialToolTip, BTreeSet, BoxSpec, CAM_SIMULATION_SCHEMA_V1,
    CamSimulationWireCollision, CamSimulationWireEvidence, CamSimulationWireRequest,
    ExactBRepGraph, ExactBackend, ExactOpOutput, ExactPairRelation, MAX_COORDINATE_MM, Point3,
    Size3, WorkerFailure, WorkerReply, WorkerResult, evaluate_exact_brep_graph, geometry_failure,
    is_sha256_hex, validated_graph,
};

pub(super) fn cam_simulation_response(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    request: &CamSimulationWireRequest,
) -> WorkerResult {
    validated_graph(graph)?;
    let bounded_values = request
        .stock_bounds_mm
        .iter()
        .flatten()
        .chain(request.setup_to_world.iter())
        .copied()
        .chain([
            request.cutter_radius_mm,
            request.cutter_length_mm,
            request.tool_length_mm,
            request.holder_radius_mm,
            request.holder_offset_mm,
            request.holder_length_mm,
        ])
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM);
    let bounded_pairs = request
        .motions
        .len()
        .checked_mul(request.fixtures.len().saturating_mul(2).saturating_add(2))
        .is_some_and(|checks| checks <= crate::MAX_CAM_SIMULATION_PAIR_CHECKS);
    if request.schema != CAM_SIMULATION_SCHEMA_V1
        || request.target_exact_graph_digest != graph.graph_digest
        || !is_sha256_hex(&request.plan_digest)
        || !is_sha256_hex(&request.toolpath_digest)
        || request.motions.is_empty()
        || request.motions.len() > crate::MAX_CAM_SIMULATION_MOTIONS
        || request.fixtures.len() > crate::MAX_CAM_SIMULATION_FIXTURES
        || !bounded_pairs
        || !bounded_values
        || request.cutter_radius_mm <= 0.0
        || request.cutter_length_mm <= 0.0
        || request.tool_length_mm < request.cutter_length_mm
        || request.holder_radius_mm <= 0.0
        || request.holder_offset_mm < request.tool_length_mm
        || request.holder_length_mm <= 0.0
        || (0..3).any(|axis| request.stock_bounds_mm[0][axis] >= request.stock_bounds_mm[1][axis])
        || request.motions.iter().any(|motion| {
            motion.kind > 3
                || motion.path_kind > 1
                || motion
                    .start_mm
                    .into_iter()
                    .chain(motion.end_mm)
                    .chain(motion.center_mm)
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        })
        || request.fixtures.iter().any(|fixture| {
            fixture.id == 0
                || (0..3).any(|axis| fixture.bounds_mm[0][axis] >= fixture.bounds_mm[1][axis])
                || fixture
                    .bounds_mm
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        })
        || request
            .fixtures
            .iter()
            .map(|fixture| fixture.id)
            .collect::<BTreeSet<_>>()
            .len()
            != request.fixtures.len()
    {
        return Err(WorkerFailure::invalid_request());
    }
    let tip = cutter_tip(request).ok_or_else(WorkerFailure::invalid_request)?;
    simulate_cam_geometry(backend, graph, request, tip)
        .map(WorkerReply::Cam)
        .map_err(|error| geometry_failure(&error))
}

/// The cutting end `request` names, if it is one the cutter can have: a point or a
/// ball shorter than the flutes, a ball as long as the radius.
fn cutter_tip(request: &CamSimulationWireRequest) -> Option<AxialToolTip> {
    let length = request.cutter_tip_length_mm;
    let fits = length.is_finite() && length > 0.0 && length < request.cutter_length_mm;
    match request.cutter_tip {
        0 if length == 0.0 => Some(AxialToolTip::Flat),
        1 if fits => Some(AxialToolTip::Cone { length_mm: length }),
        2 if fits
            && (length - request.cutter_radius_mm).abs()
                <= ketchup_model::tolerance::ROUNDING * request.cutter_radius_mm.max(1.0) =>
        {
            Some(AxialToolTip::Ball)
        }
        _ => None,
    }
}

/// `motion` in the setup frame, raised by `axial_offset_mm`, as an axial tool path.
fn cam_axial_motion(
    motion: &crate::CamSimulationWireMotion,
    axial_offset_mm: f64,
) -> AxialToolMotion {
    let offset = |mut point: [f64; 3]| {
        point[2] += axial_offset_mm;
        point
    };
    if motion.path_kind == 0 {
        AxialToolMotion::Line {
            start_mm: offset(motion.start_mm),
            end_mm: offset(motion.end_mm),
        }
    } else {
        AxialToolMotion::Arc {
            start_mm: offset(motion.start_mm),
            end_mm: offset(motion.end_mm),
            center_mm: offset(motion.center_mm),
            clockwise: motion.clockwise,
        }
    }
}

fn cam_axial_sweep(
    backend: &ExactBackend,
    motion: &crate::CamSimulationWireMotion,
    radius_mm: f64,
    axial_length_mm: f64,
    axial_offset_mm: f64,
    tip: AxialToolTip,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    backend.sweep_axial_tool(AxialToolSweepSpec {
        motion: cam_axial_motion(motion, axial_offset_mm),
        radius_mm,
        axial_length_mm,
        tip,
    })
}

/// A world box enclosing the sweep [`cam_world_sweep`] builds: the motion's box (an
/// arc's whole circle) widened by the radius and raised by the offset and length, with
/// its corners placed in the world.
fn cam_world_sweep_bounds(
    request: &CamSimulationWireRequest,
    motion: &crate::CamSimulationWireMotion,
    radius_mm: f64,
    axial_length_mm: f64,
    axial_offset_mm: f64,
) -> [[f64; 3]; 2] {
    let mut low = [0.0_f64; 3];
    let mut high = [0.0_f64; 3];
    for axis in 0..3 {
        low[axis] = motion.start_mm[axis].min(motion.end_mm[axis]);
        high[axis] = motion.start_mm[axis].max(motion.end_mm[axis]);
    }
    if motion.path_kind != 0 {
        let circle = (motion.start_mm[0] - motion.center_mm[0])
            .hypot(motion.start_mm[1] - motion.center_mm[1]);
        for axis in 0..2 {
            low[axis] = low[axis].min(motion.center_mm[axis] - circle);
            high[axis] = high[axis].max(motion.center_mm[axis] + circle);
        }
    }
    for axis in 0..2 {
        low[axis] -= radius_mm;
        high[axis] += radius_mm;
    }
    low[2] += axial_offset_mm;
    high[2] += axial_offset_mm + axial_length_mm;
    let setup_to_world = ketchup_geometry::linalg::Affine3::from_row_major(request.setup_to_world);
    let mut world = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for corner in 0..8 {
        let local = ketchup_geometry::linalg::Vec3::new(
            if corner & 1 == 0 { low[0] } else { high[0] },
            if corner & 2 == 0 { low[1] } else { high[1] },
            if corner & 4 == 0 { low[2] } else { high[2] },
        );
        let point = setup_to_world.transform_point(local).to_array();
        for (axis, &value) in point.iter().enumerate() {
            world[0][axis] = world[0][axis].min(value);
            world[1][axis] = world[1][axis].max(value);
        }
    }
    world
}

fn cam_world_sweep(
    backend: &ExactBackend,
    request: &CamSimulationWireRequest,
    motion: &crate::CamSimulationWireMotion,
    radius_mm: f64,
    axial_length_mm: f64,
    axial_offset_mm: f64,
    tip: AxialToolTip,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let local = cam_axial_sweep(
        backend,
        motion,
        radius_mm,
        axial_length_mm,
        axial_offset_mm,
        tip,
    )?;
    backend.transform_body(&local.body, &request.setup_to_world)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_cam_collision(
    collisions: &mut Vec<CamSimulationWireCollision>,
    backend: &ExactBackend,
    motion_index: usize,
    motion_kind: u8,
    participant: u8,
    target: u8,
    fixture_id: Option<u64>,
    moving: &ketchup_exact::ExactBody,
    obstacle: &ketchup_exact::ExactBody,
    tolerance_mm: f64,
) -> Result<(), ketchup_exact::GeometryError> {
    // Bounds enclose their solids, so boxes apart by more than the tolerance
    // prove the solids apart without asking the kernel.
    let (left, right) = (&moving.topology.bounds_mm, &obstacle.topology.bounds_mm);
    if left.min.x - right.max.x > tolerance_mm
        || right.min.x - left.max.x > tolerance_mm
        || left.min.y - right.max.y > tolerance_mm
        || right.min.y - left.max.y > tolerance_mm
        || left.min.z - right.max.z > tolerance_mm
        || right.min.z - left.max.z > tolerance_mm
    {
        return Ok(());
    }
    let relation = backend.query_body_pair(moving, obstacle, tolerance_mm)?;
    if relation.relation != ExactPairRelation::Separated {
        collisions.push(CamSimulationWireCollision {
            motion_index,
            motion_kind,
            participant,
            target,
            fixture_id,
            common_volume_mm3: relation.common_volume_mm3,
            contact_area_mm2: relation.common_contact_area_mm2,
            distance_mm: relation.distance_mm,
        });
    }
    Ok(())
}

fn simulate_cam_geometry(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    request: &CamSimulationWireRequest,
    tip: AxialToolTip,
) -> Result<CamSimulationWireEvidence, ketchup_exact::GeometryError> {
    let target = evaluate_exact_brep_graph(backend, graph, &[])?;
    let tolerance_mm = graph.tolerance.linear_mm();
    let box_from_bounds = |bounds: [[f64; 3]; 2]| BoxSpec {
        origin_mm: Point3 {
            x: bounds[0][0],
            y: bounds[0][1],
            z: bounds[0][2],
        },
        size_mm: Size3 {
            x: bounds[1][0] - bounds[0][0],
            y: bounds[1][1] - bounds[0][1],
            z: bounds[1][2] - bounds[0][2],
        },
    };
    let mut stock = backend.make_box(box_from_bounds(request.stock_bounds_mm))?;
    let stock_before_mm3 = stock.body.topology.volume_mm3;
    let fixture_bodies = request
        .fixtures
        .iter()
        .map(|fixture| {
            backend
                .make_box(box_from_bounds(fixture.bounds_mm))
                .map(|body| (fixture.id, body))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut collisions = Vec::new();
    let mut cutting_motion_count = 0;
    // Cuts wait until a check needs the stock they leave, so consecutive cuts run as one
    // kernel call and the stock is inspected once for them all.
    let mut pending_cuts = Vec::new();
    let flush = |stock: &mut ExactOpOutput,
                 pending: &mut Vec<AxialToolMotion>|
     -> Result<(), ketchup_exact::GeometryError> {
        if !pending.is_empty() {
            *stock = backend.cut_axial_sweeps(
                &stock.body,
                pending,
                request.cutter_radius_mm,
                request.cutter_length_mm,
                tip,
                &request.setup_to_world,
            )?;
            pending.clear();
        }
        Ok(())
    };
    let near = |sweep: [[f64; 3]; 2], obstacle: [[f64; 3]; 2]| {
        (0..3).all(|axis| {
            sweep[0][axis] - obstacle[1][axis] <= tolerance_mm
                && obstacle[0][axis] - sweep[1][axis] <= tolerance_mm
        })
    };
    for (motion_index, motion) in request.motions.iter().enumerate() {
        // Stock only shrinks, so its first bounds enclose it throughout; a sweep whose
        // enclosing box keeps clear of an obstacle's is never built for it.
        let tool_box = cam_world_sweep_bounds(
            request,
            motion,
            request.cutter_radius_mm,
            request.tool_length_mm,
            0.0,
        );
        let holder_box = cam_world_sweep_bounds(
            request,
            motion,
            request.holder_radius_mm,
            request.holder_length_mm,
            request.holder_offset_mm,
        );
        let tool_meets_stock = motion.kind == 0 && near(tool_box, request.stock_bounds_mm);
        let holder_meets_stock = near(holder_box, request.stock_bounds_mm);
        let tool = if tool_meets_stock
            || request
                .fixtures
                .iter()
                .any(|fixture| near(tool_box, fixture.bounds_mm))
        {
            Some(cam_world_sweep(
                backend,
                request,
                motion,
                request.cutter_radius_mm,
                request.tool_length_mm,
                0.0,
                tip,
            )?)
        } else {
            None
        };
        let holder = if holder_meets_stock
            || request
                .fixtures
                .iter()
                .any(|fixture| near(holder_box, fixture.bounds_mm))
        {
            Some(cam_world_sweep(
                backend,
                request,
                motion,
                request.holder_radius_mm,
                request.holder_length_mm,
                request.holder_offset_mm,
                AxialToolTip::Flat,
            )?)
        } else {
            None
        };
        if tool_meets_stock || holder_meets_stock {
            flush(&mut stock, &mut pending_cuts)?;
        }
        if let (true, Some(tool)) = (tool_meets_stock, &tool) {
            append_cam_collision(
                &mut collisions,
                backend,
                motion_index,
                motion.kind,
                0,
                0,
                None,
                &tool.body,
                &stock.body,
                tolerance_mm,
            )?;
        }
        if let (true, Some(holder)) = (holder_meets_stock, &holder) {
            append_cam_collision(
                &mut collisions,
                backend,
                motion_index,
                motion.kind,
                1,
                0,
                None,
                &holder.body,
                &stock.body,
                tolerance_mm,
            )?;
        }
        for (fixture_id, fixture) in &fixture_bodies {
            for (participant, sweep) in [(0, &tool), (1, &holder)] {
                if let Some(sweep) = sweep {
                    append_cam_collision(
                        &mut collisions,
                        backend,
                        motion_index,
                        motion.kind,
                        participant,
                        1,
                        Some(*fixture_id),
                        &sweep.body,
                        &fixture.body,
                        tolerance_mm,
                    )?;
                }
            }
        }
        if matches!(motion.kind, 1 | 2) {
            cutting_motion_count += 1;
            pending_cuts.push(cam_axial_motion(motion, 0.0));
        }
    }
    flush(&mut stock, &mut pending_cuts)?;
    let stock_after_mm3 = stock.body.topology.volume_mm3;
    let common = backend.query_body_pair(&stock.body, &target.body, tolerance_mm)?;
    let residual_stock_mm3 = (stock_after_mm3 - common.common_volume_mm3).max(0.0);
    let gouge_mm3 = (target.body.topology.volume_mm3 - common.common_volume_mm3).max(0.0);
    let evidence = CamSimulationWireEvidence {
        schema: CAM_SIMULATION_SCHEMA_V1.to_owned(),
        plan_digest: request.plan_digest.clone(),
        toolpath_digest: request.toolpath_digest.clone(),
        target_exact_graph_digest: request.target_exact_graph_digest.clone(),
        motion_count: request.motions.len(),
        cutting_motion_count,
        fixture_count: request.fixtures.len(),
        stock_before_mm3,
        stock_after_mm3,
        removed_stock_mm3: (stock_before_mm3 - stock_after_mm3).max(0.0),
        residual_stock_mm3,
        gouge_mm3,
        collisions,
        backend: ketchup_exact::backend_fingerprint().to_owned(),
        tolerance: ketchup_exact::tolerance_profile().to_owned(),
        result_fingerprint: String::new(),
    };
    Ok(evidence)
}
