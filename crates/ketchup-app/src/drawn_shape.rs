//! Push/Pull of a closed shape drawn on a face of a part. Pushed into the part
//! it mills a pocket of that shape. It works on any planar face of any part,
//! rotated or not, and the shape may run off the face. On a program part the
//! edit is written into the program as `pocket_shape()`, and pulling out writes
//! a `boss()`, so the program keeps owning the part; on any other part the
//! pocket is added to the part's definition as a tool prism and a cut, so every
//! copy of a component changes, while pulling out leaves a part of its own.
//! The drawn shape is used up in the same Undo step.
use super::*;
use ketchup_core::document::{Occurrence, RuleProgramSource};
use ketchup_core::tolerance::{APPROXIMATION, ROUNDING};

/// Distance within which the drawn shape counts as lying on a face.
const ON_FACE_MM: f64 = APPROXIMATION;

/// How a drawn-shape Push/Pull changes the document.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum DrawnShapeChange {
    /// A line appended to the program that owns the part, and the commands
    /// that remove the drawn shape once it is used.
    Program {
        source: RuleProgramSource,
        consumed: Vec<CanonicalCommand>,
    },
    /// Features added to the part's definition, removing the drawn shape.
    Definition(CommandBatch),
}

/// A Push/Pull made from a drawn shape.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnShapeEdit {
    pub change: DrawnShapeChange,
    pub part: String,
    pub pocket: bool,
    pub amount_mm: f64,
}

/// A drawn-shape Push/Pull waiting for Enter. It stays valid only while the
/// document, the selection and the typed distance are what it was made from.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnShapePreview {
    canonical_digest: String,
    selection: SelectionId,
    distance_mm_bits: u64,
    edit: DrawnShapeEdit,
}

/// One edge of the drawn loop in world coordinates; `arc` holds the centre
/// and the direction as drawn.
#[derive(Clone, Copy)]
struct WorldSegment {
    start: Vec3,
    end: Vec3,
    arc: Option<(Vec3, bool)>,
}

/// A program part face the drawn shape lies on.
#[derive(Clone)]
struct Candidate {
    part: String,
    face: &'static str,
    /// The Push/Pull distance along the face's outward normal.
    along: f64,
    mirrored: bool,
    at: Vec3,
    u: Vec3,
    v: Vec3,
    min: [f64; 2],
}

impl Candidate {
    /// Face coordinates of a world point on the face.
    fn uv(&self, p: Vec3) -> [f64; 2] {
        [
            dot(p - self.at, self.u) - self.min[0],
            dot(p - self.at, self.v) - self.min[1],
        ]
    }
}

fn point(value: [f64; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

/// `value` rounded to a micrometre, without trailing zeros.
fn number(value: f64) -> String {
    let rounded = (value * 1.0e6).round() / 1.0e6;
    let text = format!("{:.6}", if rounded == 0.0 { 0.0 } else { rounded });
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

/// The drawn loop of a definition holding a single closed profile, in the
/// definition's own coordinates.
fn drawn_loop(snapshot: &Snapshot, definition_id: DefinitionId) -> Option<Vec<ProfileSegment>> {
    let definition = snapshot.definition(definition_id)?;
    let [feature_id] = definition.feature_ids() else {
        return None;
    };
    match snapshot.feature(*feature_id)?.kind() {
        FeatureKind::Profile { segments, closed } if is_closed_profile(segments, *closed) => {
            Some(segments.clone())
        }
        _ => None,
    }
}

/// Commands that remove the drawn shape once it is used.
fn consumed(snapshot: &Snapshot, drawn: &Occurrence) -> Vec<CanonicalCommand> {
    let mut commands = vec![CanonicalCommand::DeleteOccurrence { id: drawn.id() }];
    if snapshot
        .occurrences()
        .all(|other| other.id() == drawn.id() || other.definition_id() != drawn.definition_id())
    {
        commands.push(CanonicalCommand::DeleteDefinition {
            id: drawn.definition_id(),
        });
    }
    commands
}

/// The bounds of a drawn loop in its own plane; an arc counts with its
/// whole circle.
fn loop_bounds(segments: &[ProfileSegment]) -> ([f64; 2], [f64; 2]) {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    let mut include = |p: [f64; 2], reach: f64| {
        for i in 0..2 {
            low[i] = low[i].min(p[i] - reach);
            high[i] = high[i].max(p[i] + reach);
        }
    };
    for segment in segments {
        match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                include(*start_mm, 0.0);
                include(*end_mm, 0.0);
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                include(*start_mm, 0.0);
                include(*end_mm, 0.0);
                let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                include(*center_mm, radius);
            }
            ProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => {
                for p in [start_mm, control_1_mm, control_2_mm, end_mm] {
                    include(*p, 0.0);
                }
            }
            ProfileSegment::Spline { points_mm } => {
                for p in points_mm {
                    include(*p, 0.0);
                }
            }
        }
    }
    (low, high)
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl KetchupApp {
    /// Commits a batch that only adds a drawn shape. A program that owns the
    /// document keeps owning it: the shape is construction, not a part, and
    /// is used up by the Push/Pull that writes it into the program.
    pub(super) fn apply_drawing_batch(&mut self, batch: &CommandBatch) -> bool {
        let program = self.document.current_rule_program().cloned();
        self.complete_mutation_with_work_recovery(|document| {
            document.apply_batch(batch)?;
            if let Some(program) = program {
                document.bind_rule_program(program);
            }
            Ok::<(), CanonicalError>(())
        })
        .is_ok()
    }

    /// The edit a Push/Pull of `distance_mm` on the drawn shape `selection`
    /// makes, or `None` when `selection` is not a closed shape drawn on a
    /// planar face of a part. Errors name what cannot be done.
    pub(super) fn drawn_shape_edit(
        &self,
        selection: &SelectionId,
        distance_mm: f64,
    ) -> Option<Result<DrawnShapeEdit, String>> {
        if !selection.instance_path.is_root() || distance_mm.abs() < 0.01 {
            return None;
        }
        // A typed correction of the last Push/Pull replans from the document
        // as it was before that step.
        let snapshot = self.push_pull_planning_snapshot();
        let correcting = matches!(
            self.smart_push_pull_planning,
            Some(SmartPushPullPlanning::TipReplacement(_))
        );
        let occurrence = snapshot.occurrence(selection.instance_path.root_occurrence())?;
        let segments = drawn_loop(&snapshot, occurrence.definition_id())?;
        if let Some(program) = self.document.current_rule_program().filter(|_| !correcting) {
            let owned = ketchup_application::rule_program_part_sources(program).ok()?;
            if owned.contains_key(occurrence.name()) {
                return None;
            }
            if let Some(edit) = self.program_shape_edit(
                program.clone(),
                &snapshot,
                occurrence,
                &segments,
                distance_mm,
            ) {
                return Some(edit);
            }
        }
        self.definition_shape_edit(&snapshot, occurrence, &segments, distance_mm)
    }

    /// The program line a Push/Pull on a face of a program part writes, or
    /// `None` when the shape lies on no program part.
    fn program_shape_edit(
        &self,
        program: RuleProgramSource,
        snapshot: &Snapshot,
        occurrence: &Occurrence,
        segments: &[ProfileSegment],
        distance_mm: f64,
    ) -> Option<Result<DrawnShapeEdit, String>> {
        let owned = ketchup_application::rule_program_part_sources(&program).ok()?;
        let transform = occurrence.transform();
        let to_world =
            |value: [f64; 2]| transform_model_point(transform, Vec3::new(value[0], value[1], 0.0));
        let mut world = Vec::with_capacity(segments.len());
        for segment in segments {
            world.push(match segment {
                ProfileSegment::Line { start_mm, end_mm } => WorldSegment {
                    start: to_world(*start_mm),
                    end: to_world(*end_mm),
                    arc: None,
                },
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => WorldSegment {
                    start: to_world(*start_mm),
                    end: to_world(*end_mm),
                    arc: Some((to_world(*center_mm), *clockwise)),
                },
                ProfileSegment::CubicBezier { .. } | ProfileSegment::Spline { .. } => {
                    return Some(Err(
                        "a drawn curve cannot be pushed into a program part yet; draw it with lines and arcs"
                            .to_owned(),
                    ));
                }
            });
        }
        let origin = to_world([0.0, 0.0]);
        let drawn_normal = transform_model_point(transform, Vec3::new(0.0, 0.0, 1.0)) - origin;
        let drawn_normal = drawn_normal * (1.0 / vector_length(drawn_normal));
        let model = ketchup_application::plan_rule_program(&self.document, &program)
            .ok()?
            .evaluated
            .model;

        // Every program part face the shape lies on, with the distance
        // turned into a move along that face's outward normal.
        let mut candidates: Vec<Candidate> = Vec::new();
        for part in &model.parts {
            if !owned.contains_key(&part.name) {
                continue;
            }
            let at = point(part.at_mm);
            let axes =
                [0, 1, 2].map(|axis| point(ketchup_program::frame::axis(&part.rotation, axis)));
            let (min, max) = part.local_bounds();
            for (face, normal_axis, u_axis, v_axis) in [
                (["x+", "x-"], 0, 1, 2),
                (["y+", "y-"], 1, 0, 2),
                (["z+", "z-"], 2, 0, 1),
            ] {
                let cosine = dot(drawn_normal, axes[normal_axis]);
                if cosine.abs() < 1.0 - ROUNDING {
                    continue;
                }
                for (face, outward, level) in [
                    (face[0], 1.0, max[normal_axis]),
                    (face[1], -1.0, min[normal_axis]),
                ] {
                    if (dot(origin - at, axes[normal_axis]) - level).abs() > ON_FACE_MM {
                        continue;
                    }
                    let candidate = Candidate {
                        part: part.name.clone(),
                        face,
                        along: distance_mm * cosine * outward,
                        // The drawn loop turns the other way in face
                        // coordinates (u, v) when u x v points against the
                        // drawing's normal; u x v is -y on y faces.
                        mirrored: cosine * if normal_axis == 1 { -1.0 } else { 1.0 } < 0.0,
                        at,
                        u: axes[u_axis],
                        v: axes[v_axis],
                        min: [min[u_axis], min[v_axis]],
                    };
                    let (low, high) = world
                        .iter()
                        .flat_map(|s| [s.start, s.end])
                        .map(|p| candidate.uv(p))
                        .fold(
                            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                            |(low, high), [u, v]| {
                                (
                                    [low[0].min(u), low[1].min(v)],
                                    [high[0].max(u), high[1].max(v)],
                                )
                            },
                        );
                    let size = [max[u_axis] - min[u_axis], max[v_axis] - min[v_axis]];
                    if (0..2).all(|i| high[i] > ON_FACE_MM && low[i] < size[i] - ON_FACE_MM) {
                        candidates.push(candidate);
                    }
                }
            }
        }
        // Pushing into a part mills it; otherwise the shape stands out of the
        // part it lies on.
        let chosen = candidates
            .iter()
            .find(|candidate| candidate.along < 0.0)
            .or_else(|| candidates.first())?
            .clone();
        let (part, face, along, mirrored) = (
            chosen.part.clone(),
            chosen.face,
            chosen.along,
            chosen.mirrored,
        );
        let uv = |p: Vec3| chosen.uv(p);
        let pair = |p: Vec3| {
            let [u, v] = uv(p);
            format!("[{}, {}]", number(u), number(v))
        };
        let profile = if world.iter().all(|segment| segment.arc.is_none()) {
            format!(
                "[{}]",
                world
                    .iter()
                    .map(|segment| pair(segment.start))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            let items = world
                .iter()
                .enumerate()
                .map(|(index, segment)| {
                    let name = quote(&format!("edge{}", index + 1));
                    match segment.arc {
                        None => format!("[{name}, {}, {}]", pair(segment.start), pair(segment.end)),
                        Some((center, clockwise)) => {
                            let [cu, cv] = uv(center);
                            let clockwise = if clockwise != mirrored { "True" } else { "False" };
                            format!(
                                "[{name}, {}, {}, {{\"center\": ({}, {}), \"clockwise\": {clockwise}}}]",
                                pair(segment.start),
                                pair(segment.end),
                                number(cu),
                                number(cv)
                            )
                        }
                    }
                })
                .collect::<Vec<_>>();
            format!("[{}]", items.join(", "))
        };
        let pocket = along < 0.0;
        let (call, prefix) = if pocket {
            ("pocket_shape", "pocket")
        } else {
            ("boss", "boss")
        };
        let taken = |name: &str| program.source.contains(&format!("name={}", quote(name)));
        let name = (1..)
            .map(|index| format!("{prefix} {index}"))
            .find(|name| !taken(name))
            .expect("some name is free");
        let amount_mm = along.abs();
        let mut rewritten = program;
        if !rewritten.source.ends_with('\n') {
            rewritten.source.push('\n');
        }
        rewritten.source.push_str(&format!(
            "{call}({}, {}, {profile}, {}, name={})\n",
            quote(&part),
            quote(face),
            number(amount_mm),
            quote(&name)
        ));
        Some(Ok(DrawnShapeEdit {
            change: DrawnShapeChange::Program {
                source: rewritten,
                consumed: consumed(snapshot, occurrence),
            },
            part,
            pocket,
            amount_mm,
        }))
    }

    /// A tool prism of the shape and a boolean added to the definition of the
    /// part whose planar face the shape lies on, or `None` when it lies on no
    /// evaluated part. A component's definition is shared, so all its copies
    /// change.
    fn definition_shape_edit(
        &self,
        snapshot: &Snapshot,
        drawn: &Occurrence,
        segments: &[ProfileSegment],
        distance_mm: f64,
    ) -> Option<Result<DrawnShapeEdit, String>> {
        let (low, high) = loop_bounds(segments);
        // A tessellated face lies in the drawing's plane within the model tolerance.
        let plane_mm = snapshot.tolerance().linear_mm();
        // Hosts with a face in the drawing plane that the shape overlaps; the
        // sign says whether that face looks along the drawing's normal.
        let mut hosts = Vec::new();
        let current = self.document.current();
        for host in snapshot.occurrences() {
            if host.id() == drawn.id() || host.definition_id() == drawn.definition_id() {
                continue;
            }
            let definition_id = host.definition_id();
            // A correction plans against an earlier revision; a host the last
            // step left alone keeps the solid evaluated for the current one.
            let Some(package) = self
                .topology_results
                .get_render(snapshot, definition_id)
                .or_else(|| {
                    let package = self.topology_results.get_render(&current, definition_id)?;
                    let producer = package.producer_feature_id();
                    let input = |snapshot: &Snapshot| {
                        snapshot
                            .exact_brep_graph(definition_id, producer)
                            .map(|graph| graph.canonical_input_digest.clone())
                    };
                    (input(snapshot)? == input(&current)?).then_some(package)
                })
            else {
                continue;
            };
            let Some(to_host) = host
                .transform()
                .rigid_inverse()
                .map(|inverse| inverse.compose(drawn.transform()))
            else {
                continue;
            };
            let m = *to_host.matrix();
            let origin = [m[3], m[7], m[11]];
            let axes = [[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]];
            let vertices = package.vertices();
            let facing = package.triangles().iter().find_map(|triangle| {
                let [a, b, c] = triangle.vertex_indices.map(|index| {
                    vertices
                        .get(index as usize)
                        .map(|vertex| vertex.position_mm)
                });
                let [a, b, c] = [a?, b?, c?];
                let normal = cross3(sub3(b, a), sub3(c, a));
                let length = dot3(normal, normal).sqrt();
                if length < ROUNDING {
                    return None;
                }
                let cosine = dot3(normal, axes[2]) / length;
                if cosine.abs() < 1.0 - APPROXIMATION
                    || [a, b, c]
                        .iter()
                        .any(|p| dot3(sub3(*p, origin), axes[2]).abs() > plane_mm)
                {
                    return None;
                }
                let uv = [a, b, c].map(|p| {
                    [
                        dot3(sub3(p, origin), axes[0]),
                        dot3(sub3(p, origin), axes[1]),
                    ]
                });
                let overlaps = (0..2).all(|i| {
                    uv.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min) < high[i] - plane_mm
                        && uv.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max)
                            > low[i] + plane_mm
                });
                overlaps.then_some(cosine.signum())
            });
            if let Some(facing) = facing {
                hosts.push((host, to_host, facing, package.producer_feature_id()));
            }
        }
        // Pushing into a part mills it. Pulled out of a plain part the shape
        // stays a part of its own (the other Push/Pull paths make it), as
        // assembled parts are separate bodies, not bosses on each other.
        let (host, to_host, facing, target) = *hosts
            .iter()
            .find(|(_, _, facing, _)| distance_mm * facing < 0.0)?;
        let amount_mm = distance_mm.abs();
        // The tool covers [start, start + length] along the face's outward
        // normal: into the part and a millimetre out of it.
        let (start, length) = (-amount_mm, amount_mm + 1.0);
        let offset = if facing > 0.0 {
            start
        } else {
            -(start + length)
        };
        let definition_id = host.definition_id();
        let definition = snapshot.definition(definition_id)?;
        let tool_body = BodyId(
            definition
                .bodies()
                .map(|body| body.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let first = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let [profile, prism, placed, boolean] = [0, 1, 2, 3].map(|index| FeatureId(first + index));
        let Ok(height) = Dimension::new(format_height(length), length) else {
            return Some(Err(format!("cannot make a {length} mm tool")));
        };
        let placement = Transform::from_translation(0.0, 0.0, offset)
            .ok()
            .map(|lift| to_host.compose(lift))?;
        let name = self.catalog.text("feature-pocket");
        let tool_name = self.catalog.text("model-drawn-shape-tool");
        let feature = |id, name: String, kind| CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name,
            kind,
        };
        let mut commands = vec![
            CanonicalCommand::CreateBody {
                definition_id,
                id: tool_body,
                name: tool_name.clone(),
                visible: false,
            },
            CanonicalCommand::SetActiveBody {
                definition_id,
                id: tool_body,
            },
            feature(
                profile,
                tool_name.clone(),
                FeatureKind::Profile {
                    segments: segments.to_vec(),
                    closed: true,
                },
            ),
            feature(
                prism,
                tool_name.clone(),
                FeatureKind::extrusion(profile, height),
            ),
            feature(
                placed,
                tool_name,
                FeatureKind::RigidTransform {
                    target: prism,
                    transform: placement,
                },
            ),
            CanonicalCommand::SetActiveBody {
                definition_id,
                id: definition.active_body_id(),
            },
            feature(
                boolean,
                name,
                FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target,
                    tool: placed,
                },
            ),
            CanonicalCommand::ConsumeBody {
                definition_id,
                id: tool_body,
                by_feature_id: boolean,
            },
        ];
        commands.extend(consumed(snapshot, drawn));
        let batch = CommandBatch::new(commands);
        // The result must be one the exact evaluator can build.
        let buildable = self
            .prepare_manual_push_pull_proposal(batch.clone())
            .and_then(|proposal| proposal.preview(&self.document))
            .is_some_and(|preview| {
                ExactBRepGraph::from_snapshot(&preview, definition_id, boolean).is_ok()
            });
        if !buildable {
            return Some(Err(format!(
                "the drawn shape cannot be cut into part {} yet",
                host.name()
            )));
        }
        Some(Ok(DrawnShapeEdit {
            change: DrawnShapeChange::Definition(batch),
            part: host.name().to_owned(),
            pocket: true,
            amount_mm,
        }))
    }

    /// Starts the Push/Pull of a drawn shape into or out of a part. `None`
    /// when the selection is not such a shape, so the other Push/Pull paths
    /// run.
    pub(super) fn prepare_drawn_shape_preview(
        &mut self,
        selection: &SelectionId,
        distance_mm: f64,
    ) -> Option<bool> {
        let edit = match self.drawn_shape_edit(selection, distance_mm)? {
            Ok(edit) => edit,
            Err(error) => {
                self.clear_push_pull_preview();
                self.status_key = "error-preview-stale";
                self.digest = error;
                return Some(false);
            }
        };
        let planning = self.smart_push_pull_planning.take();
        self.clear_push_pull_preview();
        self.smart_push_pull_planning = planning;
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            if edit.pocket {
                "digest-drawn-shape-pocket-live"
            } else {
                "digest-drawn-shape-boss-live"
            },
            &BTreeMap::from([
                ("amount", format_height(edit.amount_mm)),
                ("part", edit.part.clone()),
            ]),
        );
        self.tool_preview.open(DrawnShapePreview {
            canonical_digest: self.canonical_digest(),
            selection: selection.clone(),
            distance_mm_bits: distance_mm.to_bits(),
            edit,
        });
        Some(true)
    }

    #[must_use]
    pub fn has_drawn_shape_preview(&self) -> bool {
        self.tool_preview.get::<DrawnShapePreview>().is_some()
    }

    /// Commits the previewed drawn-shape Push/Pull and removes the drawn shape,
    /// as one Undo step. `None` when no such preview is open.
    pub(super) fn confirm_drawn_shape_preview(&mut self) -> Option<bool> {
        let preview = self.tool_preview.remove::<DrawnShapePreview>()?;
        let current = self.canonical_digest() == preview.canonical_digest
            && self.selection.primary.as_ref() == Some(&preview.selection)
            && parse_distance_mm(&self.push_pull_distance_input).map(f64::to_bits)
                == Some(preview.distance_mm_bits);
        let fresh = self
            .drawn_shape_edit(&preview.selection, f64::from_bits(preview.distance_mm_bits))
            .and_then(Result::ok);
        if !current || fresh.as_ref() != Some(&preview.edit) {
            self.status_key = "error-preview-stale";
            self.digest = self.catalog.text("error-preview-stale");
            return Some(false);
        }
        let edit = preview.edit;
        let applied = match &edit.change {
            DrawnShapeChange::Program { source, consumed } => self
                .apply_program_source_with(source.clone(), false, consumed.clone())
                .map(|_| ())
                .map_err(|error| error.to_string()),
            DrawnShapeChange::Definition(batch) => {
                match self.prepare_manual_push_pull_proposal(batch.clone()) {
                    Some(proposal) => self
                        .complete_mutation_with_work_recovery(|document| proposal.commit(document))
                        .map_err(|_| self.catalog.text("error-preview-stale")),
                    None => Err(self.catalog.text("error-preview-stale")),
                }
            }
        };
        if let Err(error) = applied {
            self.status_key = "error-preview-stale";
            self.digest = error;
            return Some(false);
        }
        self.smart_push_pull_planning = None;
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            if edit.pocket {
                "digest-drawn-shape-pocket-committed"
            } else {
                "digest-drawn-shape-boss-committed"
            },
            &BTreeMap::from([
                ("amount", format_height(edit.amount_mm)),
                ("part", edit.part),
            ]),
        );
        Some(true)
    }
}

#[cfg(test)]
#[path = "drawn_shape_tests.rs"]
mod tests;
