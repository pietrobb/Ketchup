use super::*;

#[derive(Default)]
pub(super) struct SceneSnapGeometry {
    points: Vec<(SelectionId, SnapKind, Vec3)>,
    edges: Vec<(SelectionId, Vec<Vec3>)>,
    circles: Vec<(SelectionId, Vec3, Vec3, Vec3, f64)>,
}

fn boundary_paths(
    edges: &[[u32; 2]],
    positions: &[[f64; 3]],
    split_corners: bool,
) -> Vec<Vec<u32>> {
    let mut neighbors = BTreeMap::<u32, Vec<u32>>::new();
    let mut unused = BTreeSet::new();
    for &[a, b] in edges {
        neighbors.entry(a).or_default().push(b);
        neighbors.entry(b).or_default().push(a);
        unused.insert([a.min(b), a.max(b)]);
    }
    let corner = |v: u32| {
        let n = &neighbors[&v];
        if n.len() != 2 {
            return true;
        }
        if !split_corners {
            return false;
        }
        let p = positions[v as usize];
        let a = positions[n[0] as usize];
        let b = positions[n[1] as usize];
        let a = Vec3::new(a[0] - p[0], a[1] - p[1], a[2] - p[2]);
        let b = Vec3::new(b[0] - p[0], b[1] - p[1], b[2] - p[2]);
        dot(a, b) > -0.95 * a.length() * b.length()
    };
    let mut starts: Vec<_> = neighbors.keys().copied().filter(|v| corner(*v)).collect();
    starts.extend(neighbors.keys().copied());
    let mut paths = Vec::new();
    for start in starts {
        for &next in &neighbors[&start] {
            if !unused.remove(&[start.min(next), start.max(next)]) {
                continue;
            }
            let mut path = vec![start, next];
            let mut previous = start;
            let mut current = next;
            while current != start && !corner(current) {
                let Some(&next) = neighbors[&current].iter().find(|n| **n != previous) else {
                    break;
                };
                if !unused.remove(&[current.min(next), current.max(next)]) {
                    break;
                }
                path.push(next);
                previous = current;
                current = next;
            }
            paths.push(path);
        }
    }
    paths
}

impl SceneSnapGeometry {
    fn edge(&mut self, reference: &SelectionId, points: Vec<Vec3>, endpoints: bool) {
        if points.len() < 2 {
            return;
        }
        let mut reference = reference.clone();
        if let ElementId::Snap { index, .. } = &mut reference.element {
            *index = self.edges.len() as u32 * 4;
        }
        if endpoints {
            self.points
                .push((reference.clone(), SnapKind::Endpoint, points[0]));
            let mut end = reference.clone();
            if let ElementId::Snap { index, .. } = &mut end.element {
                *index += 1;
            }
            self.points
                .push((end, SnapKind::Endpoint, *points.last().unwrap()));
        }
        let length: f64 = points.windows(2).map(|s| s[0].distance(s[1])).sum();
        let mut remaining = length * 0.5;
        for segment in points.windows(2).filter(|_| endpoints) {
            let length = segment[0].distance(segment[1]);
            if remaining <= length && length > 1e-12 {
                let mut midpoint = reference.clone();
                if let ElementId::Snap { index, .. } = &mut midpoint.element {
                    *index += 2;
                }
                self.points.push((
                    midpoint,
                    SnapKind::Midpoint,
                    segment[0] + (segment[1] - segment[0]) * (remaining / length),
                ));
                break;
            }
            remaining -= length;
        }
        if let ElementId::Snap { index, .. } = &mut reference.element {
            *index += 3;
        }
        self.edges.push((reference, points));
    }

    fn surface(
        &mut self,
        reference: &SelectionId,
        transform: Transform,
        positions: &[[f64; 3]],
        triangles: &[[u32; 3]],
        groups: &[Option<u32>],
        evidence: &[ketchup_core::exact_product::ExactBRepGraphEdgeEvidence],
    ) {
        let positions_f32: Vec<_> = positions.iter().map(|p| p.map(|v| v as f32)).collect();
        let mut boundaries = BTreeMap::<Option<u32>, Vec<[u32; 2]>>::new();
        for (edge, uses) in renderer::feature_edge_triangles(&positions_f32, triangles, groups) {
            let faces: BTreeSet<_> = uses.iter().filter_map(|i| groups[*i as usize]).collect();
            let mut matches = evidence.iter().filter(|e| {
                !faces.is_empty()
                    && faces.is_subset(&e.adjacent_face_ordinals.iter().copied().collect())
                    && edge.iter().all(|i| {
                        (0..3).all(|axis| {
                            positions[*i as usize][axis] >= e.bounds_mm[0][axis] - 1e-6
                                && positions[*i as usize][axis] <= e.bounds_mm[1][axis] + 1e-6
                        })
                    })
            });
            let ordinal = matches
                .next()
                .map(|e| e.edge_ordinal)
                .filter(|_| matches.next().is_none());
            boundaries.entry(ordinal).or_default().push(edge);
        }
        for (ordinal, edges) in boundaries {
            if ordinal.is_some_and(|ordinal| {
                evidence.iter().any(|e| {
                    e.edge_ordinal == ordinal
                        && e.closed
                        && e.circle_radius_mm.is_some()
                        && e.axis_origin_mm.is_some()
                        && e.unit_axis_direction.is_some()
                })
            }) {
                continue;
            }
            for path in boundary_paths(&edges, positions, ordinal.is_none()) {
                let closed = path.first() == path.last();
                let mut edge_ref = reference.clone();
                if let (Some(ordinal), ElementId::Snap { feature_id, .. }) =
                    (ordinal, &reference.element)
                {
                    edge_ref.element = ElementId::TopologicalEdge {
                        feature_id: *feature_id,
                        ordinal,
                    };
                }
                let points: Vec<_> = path
                    .iter()
                    .map(|i| {
                        let p = positions[*i as usize];
                        transform_model_point(transform, Vec3::new(p[0], p[1], p[2]))
                    })
                    .collect();
                let linear = points
                    .windows(3)
                    .all(|p| cross(p[1] - p[0], p[2] - p[1]).length() <= 1e-8);
                self.edge(&edge_ref, points, !closed && linear);
            }
        }
        for e in evidence.iter().filter(|e| e.closed) {
            let (Some(radius), Some(c), Some(n)) =
                (e.circle_radius_mm, e.axis_origin_mm, e.unit_axis_direction)
            else {
                continue;
            };
            let center = Vec3::new(c[0], c[1], c[2]);
            let normal = Vec3::new(n[0], n[1], n[2]);
            let seed = if normal.x.abs() < 0.9 {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            let x = cross(normal, seed);
            let x = x * (1.0 / x.length());
            let y = cross(normal, x);
            let world_center = transform_model_point(transform, center);
            let x = transform_model_point(transform, center + x) - world_center;
            let y = transform_model_point(transform, center + y) - world_center;
            let mut circle_ref = reference.clone();
            if let ElementId::Snap { feature_id, .. } = reference.element {
                circle_ref.element = ElementId::TopologicalEdge {
                    feature_id,
                    ordinal: e.edge_ordinal,
                };
            }
            self.points
                .push((circle_ref.clone(), SnapKind::Center, world_center));
            self.circles.push((circle_ref, world_center, x, y, radius));
        }
    }

    fn profile(
        &mut self,
        snapshot: &Snapshot,
        id: FeatureId,
        reference: &SelectionId,
        transform: Transform,
        offset: Vec3,
    ) -> bool {
        let Some(feature) = snapshot.feature(id) else {
            return false;
        };
        let mut frame = WorkplaneFrame {
            origin_mm: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        };
        let segments = match feature.kind() {
            FeatureKind::Profile { points_mm } => (0..points_mm.len())
                .map(|i| ProfileSegment::Line {
                    start_mm: points_mm[i],
                    end_mm: points_mm[(i + 1) % points_mm.len()],
                })
                .collect::<Vec<_>>(),
            FeatureKind::SegmentProfile { segments, .. } => segments.clone(),
            FeatureKind::Sketch(sketch) => {
                let Some(workplane) = snapshot.feature(sketch.workplane) else {
                    return false;
                };
                let FeatureKind::Workplane(workplane) = workplane.kind() else {
                    return false;
                };
                frame = workplane.frame;
                let Ok(solved) = sketch.solve_geometry() else {
                    return false;
                };
                solved
                    .entities
                    .iter()
                    .flat_map(|entity| match entity {
                        SketchEntity::Line {
                            start_mm, end_mm, ..
                        } => vec![ProfileSegment::Line {
                            start_mm: *start_mm,
                            end_mm: *end_mm,
                        }],
                        SketchEntity::Arc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                            ..
                        } => vec![ProfileSegment::CircularArc {
                            start_mm: *start_mm,
                            end_mm: *end_mm,
                            center_mm: *center_mm,
                            clockwise: *clockwise,
                        }],
                        SketchEntity::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                            ..
                        } => vec![ProfileSegment::CubicBezier {
                            start_mm: *start_mm,
                            control_1_mm: *control_1_mm,
                            control_2_mm: *control_2_mm,
                            end_mm: *end_mm,
                        }],
                        SketchEntity::Circle {
                            center_mm: c,
                            radius_mm: r,
                            ..
                        } => vec![
                            ProfileSegment::CircularArc {
                                start_mm: [c[0] + r, c[1]],
                                end_mm: [c[0] - r, c[1]],
                                center_mm: *c,
                                clockwise: false,
                            },
                            ProfileSegment::CircularArc {
                                start_mm: [c[0] - r, c[1]],
                                end_mm: [c[0] + r, c[1]],
                                center_mm: *c,
                                clockwise: false,
                            },
                        ],
                    })
                    .collect()
            }
            _ => return false,
        };
        let world = |p: [f64; 2]| {
            transform_model_point(
                transform,
                Vec3::new(
                    frame.origin_mm[0] + frame.x_axis[0] * p[0] + frame.y_axis[0] * p[1],
                    frame.origin_mm[1] + frame.x_axis[1] * p[0] + frame.y_axis[1] * p[1],
                    frame.origin_mm[2] + frame.x_axis[2] * p[0] + frame.y_axis[2] * p[1],
                ) + offset,
            )
        };
        let circle = exact_circle_geometry(&segments, true);
        for segment in segments.iter().filter(|_| circle.is_none()) {
            let points = match segment {
                ProfileSegment::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => profile_arc_polyline(*start_mm, *end_mm, *center_mm, *clockwise, 64),
                ProfileSegment::CubicBezier {
                    start_mm: a,
                    control_1_mm: b,
                    control_2_mm: c,
                    end_mm: d,
                } => (0..=64)
                    .map(|i| {
                        let t = f64::from(i) / 64.0;
                        let u = 1.0 - t;
                        [
                            u.powi(3) * a[0]
                                + 3.0 * u * u * t * b[0]
                                + 3.0 * u * t * t * c[0]
                                + t.powi(3) * d[0],
                            u.powi(3) * a[1]
                                + 3.0 * u * u * t * b[1]
                                + 3.0 * u * t * t * c[1]
                                + t.powi(3) * d[1],
                        ]
                    })
                    .collect(),
            };
            self.edge(
                reference,
                points.into_iter().map(world).collect(),
                circle.is_none(),
            );
        }
        if let Some((center, radius)) = circle {
            let center_world = world(center);
            let x = (world([center[0] + radius, center[1]]) - center_world) * (1.0 / radius);
            let y = (world([center[0], center[1] + radius]) - center_world) * (1.0 / radius);
            let mut center_ref = reference.clone();
            if let ElementId::Snap { index, .. } = &mut center_ref.element {
                *index = 0x8000_0000 + self.circles.len() as u32;
            }
            self.points
                .push((center_ref.clone(), SnapKind::Center, center_world));
            self.circles.push((center_ref, center_world, x, y, radius));
        }
        true
    }
}

impl KetchupApp {
    fn build_scene_snap_geometry(&self, snapshot: &Snapshot) -> SceneSnapGeometry {
        let mut geometry = SceneSnapGeometry::default();
        let results = self.interaction_exact_registry(snapshot);
        for occurrence in snapshot.scene_query().into_iter().filter(|o| o.visible) {
            let Some(definition) = snapshot.definition(occurrence.definition_id) else {
                continue;
            };
            let Some(&id) = definition
                .feature_ids()
                .iter()
                .rev()
                .find(|id| !snapshot.feature_is_suppressed(**id))
            else {
                continue;
            };
            let reference = SelectionId {
                definition_id: occurrence.definition_id,
                instance_path: occurrence.instance_path.clone(),
                element: ElementId::Snap {
                    feature_id: id,
                    index: 0,
                },
            };
            let Some(feature) = snapshot.feature(id) else {
                continue;
            };
            let semantic = match feature.kind() {
                FeatureKind::Extrusion { profile, height } => {
                    let first_point = geometry.points.len();
                    let bottom = geometry.profile(
                        snapshot,
                        *profile,
                        &reference,
                        occurrence.transform,
                        Vec3::ZERO,
                    );
                    let normal = match snapshot.feature(*profile).map(|f| f.kind()) {
                        Some(FeatureKind::Sketch(sketch)) => match snapshot
                            .feature(sketch.workplane)
                            .map(|f| f.kind())
                        {
                            Some(FeatureKind::Workplane(w)) => {
                                Vec3::new(w.frame.normal[0], w.frame.normal[1], w.frame.normal[2])
                            }
                            _ => continue,
                        },
                        _ => Vec3::new(0.0, 0.0, 1.0),
                    };
                    let offset = normal * height.millimetres();
                    let corners: Vec<_> = geometry.points[first_point..]
                        .iter()
                        .filter(|(_, kind, _)| *kind == SnapKind::Endpoint)
                        .map(|(_, _, p)| *p)
                        .collect();
                    geometry.profile(snapshot, *profile, &reference, occurrence.transform, offset);
                    let world_offset = transform_model_point(occurrence.transform, offset)
                        - transform_model_point(occurrence.transform, Vec3::ZERO);
                    for corner in corners {
                        geometry.edge(&reference, vec![corner, corner + world_offset], true);
                    }
                    bottom
                }
                _ => geometry.profile(snapshot, id, &reference, occurrence.transform, Vec3::ZERO),
            };
            let packages: Vec<_> = results
                .render_values(snapshot)
                .filter(|p| p.definition_id() == occurrence.definition_id)
                .collect();
            if !packages.is_empty() {
                for package in packages {
                    let positions: Vec<_> =
                        package.vertices().iter().map(|v| v.position_mm).collect();
                    let triangles: Vec<_> = package
                        .triangles()
                        .iter()
                        .map(|t| t.vertex_indices)
                        .collect();
                    let groups: Vec<_> = (0..triangles.len())
                        .map(|i| match package.as_ref() {
                            ExactBodyPackage::Graph(p) => p.triangle_face_ordinals.get(i).copied(),
                            ExactBodyPackage::Imported(p) => {
                                p.triangle_face_ordinals.get(i).copied()
                            }
                            _ => None,
                        })
                        .collect();
                    let mut reference = reference.clone();
                    reference.element = ElementId::Snap {
                        feature_id: package.producer_feature_id(),
                        index: 0,
                    };
                    geometry.surface(
                        &reference,
                        occurrence.transform,
                        &positions,
                        &triangles,
                        &groups,
                        package.edge_evidence(),
                    );
                }
                continue;
            }
            if !semantic && let FeatureKind::MeshBody(mesh) = feature.kind() {
                geometry.surface(
                    &reference,
                    occurrence.transform,
                    &mesh.vertices_mm,
                    &mesh.triangles,
                    &vec![None; mesh.triangles.len()],
                    &[],
                );
            }
        }
        geometry
    }

    pub(super) fn scene_snap_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f32,
        frame: Option<WorkplaneFrame>,
    ) -> Option<SnapResult> {
        if !self.face_workflow.snaps_enabled() {
            return None;
        }
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.interaction_projection_cache.borrow();
        let geometry = cache
            .as_ref()?
            .snap_geometry
            .get_or_init(|| self.build_scene_snap_geometry(&snapshot));
        let retained = self.hover_snap.as_ref().filter(|s| {
            self.active_tool != ActiveTool::Rectangle
                && !matches!(s.kind, SnapKind::Edge | SnapKind::Face)
        });
        let pointer_ray = self.view_ray(pointer, rect)?;
        let frame = frame.or_else(|| {
            self.uses_drawing_plane()
                .then(|| self.sketch_start.map(|p| self.drawing_frame(Some(p))))
                .flatten()
        });
        let line_axis = (self.active_tool == ActiveTool::Line)
            .then(|| self.sketch_start.zip(self.line_axis_lock))
            .flatten();
        let move_drag = self.move_drag.as_ref().or(self.move_anchor.as_ref());
        if self.active_tool == ActiveTool::Move && move_drag.is_some_and(|d| d.axis.is_some()) {
            return None;
        }
        let mut candidates = Vec::new();
        let mut add = |reference: &SelectionId, kind: SnapKind, point: Vec3| {
            if line_axis.is_some_and(|(start, axis)| {
                cross(point - start, axis_direction(axis)).length() > 1e-7
            }) || move_drag
                .is_some_and(|drag| self.move_drag_applies_to_path(drag, &reference.instance_path))
            {
                return;
            }
            if dot(point - pointer_ray.origin, pointer_ray.direction) < 0.0
                || frame.is_some_and(|f| !drawing_plane::point_in_frame(point, f))
            {
                return;
            }
            let distance = self.project(point, rect).distance(pointer);
            let locked = retained.is_some_and(|s| s.reference == *reference && s.kind == kind);
            if distance <= tolerance_px * if locked { 1.5 } else { 1.0 } {
                candidates.push((
                    locked,
                    distance,
                    SnapResult {
                        kind,
                        reference: reference.clone(),
                        position_mm: point,
                        distance_mm: cross(point - pointer_ray.origin, pointer_ray.direction)
                            .length(),
                    },
                ));
            }
        };
        for (reference, kind, point) in &geometry.points {
            add(reference, *kind, *point);
        }
        for (reference, points) in &geometry.edges {
            for segment in points.windows(2) {
                let from = self.project(segment[0], rect);
                let to = self.project(segment[1], rect);
                let direction = to - from;
                if direction.length_sq() <= f32::EPSILON {
                    continue;
                }
                let factor =
                    ((pointer - from).dot(direction) / direction.length_sq()).clamp(0.0, 1.0);
                let screen = from.lerp(to, factor);
                // A screen interpolation is not affine in perspective. Resolve the world point from its ray.
                let Some(ray) = self.view_ray(screen, rect) else {
                    continue;
                };
                let edge = segment[1] - segment[0];
                let w = segment[0] - ray.origin;
                let b = dot(edge, ray.direction);
                let a = dot(edge, edge);
                let denominator = a - b * b;
                if denominator <= 1e-12 {
                    continue;
                }
                let t = if self.projection_mode == ProjectionMode::Perspective {
                    ((b * dot(w, ray.direction) - dot(edge, w)) / denominator).clamp(0.0, 1.0)
                } else {
                    f64::from(factor)
                };
                add(reference, SnapKind::Edge, segment[0] + edge * t);
            }
        }
        for (reference, center, x, y, radius) in &geometry.circles {
            let at =
                |angle: f64| *center + *x * (*radius * angle.cos()) + *y * (*radius * angle.sin());
            let distance = |angle: f64| self.project(at(angle), rect).distance_sq(pointer);
            let step = std::f64::consts::TAU / 128.0;
            let best = (0..128)
                .map(|i| f64::from(i) * step)
                .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
                .unwrap();
            let (mut low, mut high) = (best - step, best + step);
            for _ in 0..24 {
                let a = low + (high - low) / 3.0;
                let b = high - (high - low) / 3.0;
                if distance(a) < distance(b) {
                    high = b;
                } else {
                    low = a;
                }
            }
            add(reference, SnapKind::Edge, at((low + high) * 0.5));
        }
        if self.active_tool == ActiveTool::Arc
            && self.sketch_end.is_none()
            && let Some(anchor) = self.sketch_start
        {
            for (reference, center, x, y, radius) in &geometry.circles {
                let xx = dot(*x, *x);
                let yy = dot(*y, *y);
                if (xx - yy).abs() > 1e-8 || dot(*x, *y).abs() > 1e-8 || xx <= 1e-12 {
                    continue;
                }
                let delta = anchor - *center;
                if dot(delta, cross(*x, *y)).abs() > 1e-7 {
                    continue;
                }
                let local = Vec3::new(dot(delta, *x) / xx, dot(delta, *y) / yy, 0.0);
                for p in tangent_points(local, Vec3::ZERO, *radius) {
                    add(reference, SnapKind::Tangent, *center + *x * p.x + *y * p.y);
                }
            }
        }
        candidates
            .into_iter()
            .min_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| a.2.score().kind_rank.cmp(&b.2.score().kind_rank))
                    .then_with(|| a.1.total_cmp(&b.1))
                    .then_with(|| a.2.reference.cmp(&b.2.reference))
            })
            .map(|(_, _, s)| s)
    }

    pub(super) fn datum_snap_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        frame: Option<WorkplaneFrame>,
    ) -> Option<(Vec3, Option<Axis>)> {
        self.datum_snap_with_position(pointer, rect, frame, None)
    }

    pub(super) fn datum_snap_with_position(
        &self,
        pointer: Pos2,
        rect: Rect,
        frame: Option<WorkplaneFrame>,
        position: Option<Vec3>,
    ) -> Option<(Vec3, Option<Axis>)> {
        if !self.face_workflow.snaps_enabled() {
            return None;
        }
        let in_plane = |p| frame.is_none_or(|f| drawing_plane::point_in_frame(p, f));
        if in_plane(Vec3::ZERO) && self.project(Vec3::ZERO, rect).distance(pointer) <= 8.0 {
            return Some((Vec3::ZERO, None));
        }
        let ray = self.view_ray(pointer, rect)?;
        [
            (Axis::X, Vec3::new(1.0, 0.0, 0.0)),
            (Axis::Y, Vec3::new(0.0, 1.0, 0.0)),
            (Axis::Z, Vec3::new(0.0, 0.0, 1.0)),
        ]
        .into_iter()
        .filter_map(|(axis, d)| {
            let b = dot(d, ray.direction);
            let denominator = 1.0 - b * b;
            if denominator <= 1e-8 {
                return None;
            }
            let t = (dot(d, ray.origin) - b * dot(ray.direction, ray.origin)) / denominator;
            let point = d * position.map_or(t, |p| dot(p, d));
            let distance = self.project(point, rect).distance(pointer);
            (in_plane(point) && dot(point - ray.origin, ray.direction) >= 0.0 && distance <= 8.0)
                .then_some((distance, point, axis))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p, a)| (p, Some(a)))
    }
}
