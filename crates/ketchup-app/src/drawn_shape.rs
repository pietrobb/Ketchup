//! Push/Pull of a closed shape drawn on a face of a part. Pushed into the part
//! it mills a pocket of that shape. It works on any planar face of any part,
//! rotated or not, and the shape may run off the face. On a program part the
//! edit is written into the program as `pocket_shape()`, and pulling out writes
//! a `boss()`, so the program keeps owning the part; on any other part the
//! pocket is added to the part's definition as a tool prism and a cut, so every
//! copy of a component changes, while pulling out leaves a part of its own.
//! The drawn shape is used up in the same Undo step.
use super::*;
use ketchup_geometry::linalg::{cross, dot, sub};
use ketchup_model::document::{Occurrence, RuleProgramSource};
use ketchup_model::tolerance::{APPROXIMATION, ROUNDING};

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
    pub kind: DrawnShapeKind,
    pub amount_mm: f64,
}

/// What a drawn-shape Push/Pull makes of the shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DrawnShapeKind {
    /// Milled into the part it lies on.
    Pocket,
    /// Standing out of the part it lies on, joined to it.
    Boss,
    /// A part of its own, standing on no part.
    Part,
}

impl DrawnShapeKind {
    const fn digest_key(self, committed: bool) -> &'static str {
        match (self, committed) {
            (Self::Pocket, false) => "digest-drawn-shape-pocket-live",
            (Self::Boss, false) => "digest-drawn-shape-boss-live",
            (Self::Part, false) => "digest-drawn-shape-part-live",
            (Self::Pocket, true) => "digest-drawn-shape-pocket-committed",
            (Self::Boss, true) => "digest-drawn-shape-boss-committed",
            (Self::Part, true) => "digest-drawn-shape-part-committed",
        }
    }
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
/// and the direction as drawn, `controls` the inner points of a cubic.
#[derive(Clone, Copy)]
struct WorldSegment {
    start: Vec3,
    end: Vec3,
    arc: Option<(Vec3, bool)>,
    controls: Option<[Vec3; 2]>,
}

/// The drawn loop placed by `transform`, or `None` when it holds a spline,
/// which a program profile cannot express.
fn world_segments(transform: Transform, segments: &[ProfileSegment]) -> Option<Vec<WorldSegment>> {
    let to_world =
        |value: [f64; 2]| transform_model_point(transform, Vec3::new(value[0], value[1], 0.0));
    segments
        .iter()
        .map(|segment| {
            let (start, end, arc, controls) = match segment {
                ProfileSegment::Line { start_mm, end_mm } => (start_mm, end_mm, None, None),
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => (
                    start_mm,
                    end_mm,
                    Some((to_world(*center_mm), *clockwise)),
                    None,
                ),
                ProfileSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => (
                    start_mm,
                    end_mm,
                    None,
                    Some([to_world(*control_1_mm), to_world(*control_2_mm)]),
                ),
                ProfileSegment::Spline { .. } => return None,
            };
            Some(WorldSegment {
                start: to_world(*start),
                end: to_world(*end),
                arc,
                controls,
            })
        })
        .collect()
}

/// The program profile of `world` in the face coordinates `uv`: a point
/// list for a polygon, named segments when it holds arcs or cubics. A
/// `mirrored` frame turns arcs the other way.
fn program_profile(
    world: &[WorldSegment],
    uv: impl Fn(Vec3) -> [f64; 2],
    mirrored: bool,
) -> String {
    let pair = |p: Vec3| {
        let [u, v] = uv(p);
        format!("[{}, {}]", number(u), number(v))
    };
    if world
        .iter()
        .all(|segment| segment.arc.is_none() && segment.controls.is_none())
    {
        let points = world.iter().map(|segment| pair(segment.start));
        return format!("[{}]", points.collect::<Vec<_>>().join(", "));
    }
    let items = world.iter().enumerate().map(|(index, segment)| {
        let name = quote(&format!("edge{}", index + 1));
        let (start, end) = (pair(segment.start), pair(segment.end));
        match (segment.arc, segment.controls) {
            (Some((center, clockwise)), _) => {
                let [cu, cv] = uv(center);
                let clockwise = if clockwise != mirrored { "True" } else { "False" };
                format!(
                    "[{name}, {start}, {end}, {{\"center\": ({}, {}), \"clockwise\": {clockwise}}}]",
                    number(cu),
                    number(cv)
                )
            }
            (None, Some([first, second])) => format!(
                "[{name}, {start}, {end}, {{\"controls\": [{}, {}]}}]",
                pair(first),
                pair(second)
            ),
            (None, None) => format!("[{name}, {start}, {end}]"),
        }
    });
    format!("[{}]", items.collect::<Vec<_>>().join(", "))
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

/// The program line that extrudes a drawn shape lying on no part into a part
/// of its own, `distance_mm` along the drawing's normal.
fn free_shape_program_edit(
    mut program: RuleProgramSource,
    evaluated: &ketchup_program::Evaluated,
    snapshot: &Snapshot,
    drawn: &Occurrence,
    segments: &[ProfileSegment],
    distance_mm: f64,
) -> Result<DrawnShapeEdit, DrawnShapeRefusal> {
    let transform = drawn.transform();
    let world = world_segments(transform, segments).ok_or(DrawnShapeRefusal::CurveOnProgramPart)?;
    let at = |x, y, z| transform_model_point(transform, Vec3::new(x, y, z));
    let origin = at(0.0, 0.0, 0.0);
    let unit = |v: Vec3| v * (1.0 / length(v));
    let (x, y, z) = (
        at(1.0, 0.0, 0.0) - origin,
        at(0.0, 1.0, 0.0) - origin,
        at(0.0, 0.0, 1.0) - origin,
    );
    let mirrored = dot(cross(x, y), z) < 0.0;
    let (x, z) = (unit(x), unit(z));
    let y = cross(z, x);
    let profile = program_profile(
        &world,
        |p| [dot(p - origin, x), dot(p - origin, y)],
        mirrored,
    );
    // The part rises from the drawing along its normal, or sinks below it.
    let amount_mm = distance_mm.abs();
    let base = origin + z * distance_mm.min(0.0);
    let name = (1..)
        .map(|index| format!("shape {index}"))
        .find(|name| {
            !evaluated.part_sources.contains_key(name) && !program.source.contains(&quote(name))
        })
        .expect("some name is free");
    let triple = |v: Vec3| format!("({}, {}, {})", number(v.x), number(v.y), number(v.z));
    if !program.source.ends_with('\n') {
        program.source.push('\n');
    }
    program.source.push_str(&format!(
        "place(extrude({}, profile = {profile}, distance = {}), origin = {}, z = {}, x = {})\n",
        quote(&name),
        number(amount_mm),
        triple(base),
        triple(z),
        triple(x),
    ));
    Ok(DrawnShapeEdit {
        change: DrawnShapeChange::Program {
            source: program,
            consumed: consumed(snapshot, drawn),
        },
        part: name,
        kind: DrawnShapeKind::Part,
        amount_mm,
    })
}

/// Why a drawn shape cannot be pushed or pulled into the part it lies on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DrawnShapeRefusal {
    /// The shape holds a curve other than lines and arcs and lies on a program part.
    CurveOnProgramPart,
    /// The tool the distance asks for has no valid height.
    ToolLength { length_mm: f64 },
    /// The exact evaluator cannot build the cut into this part.
    NotBuildable { part: String },
}

impl std::fmt::Display for DrawnShapeRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurveOnProgramPart => formatter.write_str(
                "a drawn curve cannot be pushed into a program part yet; draw it with lines and arcs",
            ),
            Self::ToolLength { length_mm } => write!(formatter, "cannot make a {length_mm} mm tool"),
            Self::NotBuildable { part } => {
                write!(formatter, "the drawn shape cannot be cut into part {part} yet")
            }
        }
    }
}

impl std::error::Error for DrawnShapeRefusal {}

impl KetchupApp {
    /// Commits a batch that only adds a drawn shape. A program that owns the
    /// document keeps owning it: the shape is construction, not a part, and
    /// is used up by the Push/Pull that writes it into the program.
    pub(super) fn apply_drawing_batch(&mut self, batch: &CommandBatch) -> bool {
        let program = self.document.current_rule_program().cloned();
        self.complete_mutation_with_work_recovery(|document| {
            document.apply_batch(batch)?;
            if let Some(program) = program {
                document.bind_rule_program(program)?;
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
    ) -> Option<Result<DrawnShapeEdit, DrawnShapeRefusal>> {
        if !selection.instance_path.is_root() || distance_mm.abs() < 0.01 {
            return None;
        }
        // A typed correction of the last Push/Pull replans from the document
        // as it was before that step.
        let snapshot = self.push_pull_planning_snapshot();
        let correcting = matches!(
            self.push_pull.smart_planning,
            Some(SmartPushPullPlanning::TipReplacement(_))
        );
        let occurrence = snapshot.occurrence(selection.instance_path.root_occurrence())?;
        let segments = drawn_loop(&snapshot, occurrence.definition_id())?;
        if let Some(program) = self.document.current_rule_program().filter(|_| !correcting) {
            let evaluated = self.evaluated_program(program)?;
            if evaluated.part_sources.contains_key(occurrence.name()) {
                return None;
            }
            if let Some(edit) = self.program_shape_edit(
                program.clone(),
                &evaluated,
                &snapshot,
                occurrence,
                &segments,
                distance_mm,
            ) {
                return Some(edit);
            }
            // A shape on no part becomes a part of the program, which keeps
            // owning the document.
            return self
                .definition_shape_edit(&snapshot, occurrence, &segments, distance_mm)
                .or_else(|| {
                    Some(free_shape_program_edit(
                        program.clone(),
                        &evaluated,
                        &snapshot,
                        occurrence,
                        &segments,
                        distance_mm,
                    ))
                });
        }
        self.definition_shape_edit(&snapshot, occurrence, &segments, distance_mm)
    }

    /// `program` evaluated once per source and overrides: a Push/Pull drag
    /// asks on every pointer move, and a house program takes a second.
    fn evaluated_program(
        &self,
        program: &RuleProgramSource,
    ) -> Option<std::rc::Rc<ketchup_program::Evaluated>> {
        if let Some((cached, evaluated)) = self.push_pull.program_evaluation.borrow().as_ref()
            && cached == program
        {
            return evaluated.clone();
        }
        let evaluated =
            ketchup_program::evaluate(&program.file_name, &program.source, &program.overrides)
                .ok()
                .map(std::rc::Rc::new);
        *self.push_pull.program_evaluation.borrow_mut() =
            Some((program.clone(), evaluated.clone()));
        evaluated
    }

    /// The program line a Push/Pull on a face of a program part writes, or
    /// `None` when the shape lies on no program part.
    fn program_shape_edit(
        &self,
        program: RuleProgramSource,
        evaluated: &ketchup_program::Evaluated,
        snapshot: &Snapshot,
        occurrence: &Occurrence,
        segments: &[ProfileSegment],
        distance_mm: f64,
    ) -> Option<Result<DrawnShapeEdit, DrawnShapeRefusal>> {
        let owned = &evaluated.part_sources;
        let transform = occurrence.transform();
        let Some(world) = world_segments(transform, segments) else {
            return Some(Err(DrawnShapeRefusal::CurveOnProgramPart));
        };
        let origin = transform_model_point(transform, Vec3::new(0.0, 0.0, 0.0));
        let drawn_normal = transform_model_point(transform, Vec3::new(0.0, 0.0, 1.0)) - origin;
        let drawn_normal = drawn_normal * (1.0 / length(drawn_normal));
        let model = &evaluated.model;

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
        let profile = program_profile(&world, |p| chosen.uv(p), mirrored);
        let kind = if along < 0.0 {
            DrawnShapeKind::Pocket
        } else {
            DrawnShapeKind::Boss
        };
        let (call, prefix) = if kind == DrawnShapeKind::Pocket {
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
            kind,
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
    ) -> Option<Result<DrawnShapeEdit, DrawnShapeRefusal>> {
        let (low, high) = loop_bounds(segments);
        // A tessellated face lies in the drawing's plane within the model tolerance.
        let plane_mm = snapshot.tolerance().linear_mm();
        // Hosts with a face in the drawing plane that the shape overlaps; the
        // sign says whether that face looks along the drawing's normal.
        let mut hosts = Vec::new();
        let current = self.document.current();
        // Looked up per host, so indexed once rather than scanned per host.
        let rendered = self.exact.topology_results.render_by_definition(snapshot);
        let rendered_current = self.exact.topology_results.render_by_definition(&current);
        for host in snapshot.occurrences() {
            if host.id() == drawn.id() || host.definition_id() == drawn.definition_id() {
                continue;
            }
            let definition_id = host.definition_id();
            // A correction plans against an earlier revision; a host the last
            // step left alone keeps the solid evaluated for the current one.
            let Some(package) = rendered.get(&definition_id).copied().or_else(|| {
                let package = *rendered_current.get(&definition_id)?;
                let producer = package.producer_feature_id();
                let input = |snapshot: &Snapshot| {
                    snapshot
                        .exact_brep_graph(definition_id, producer)
                        .map(|graph| graph.canonical_input_digest.clone())
                };
                (input(snapshot)? == input(&current)?).then_some(package)
            }) else {
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
            // Every face the triangle test below accepts lies in the host's
            // bounds, so a host whose bounds miss the drawing's plane or the
            // shape within it is skipped without walking its triangles.
            let [min, max] = package.bounds_mm();
            let corners = (0..8).map(|index| {
                let pick = |axis: usize| {
                    if index >> axis & 1 == 0 {
                        min[axis]
                    } else {
                        max[axis]
                    }
                };
                sub([pick(0), pick(1), pick(2)], origin)
            });
            let ranges = corners.fold([[f64::INFINITY, f64::NEG_INFINITY]; 3], |mut ranges, p| {
                for (range, axis) in ranges.iter_mut().zip(axes) {
                    let value = dot(p, axis);
                    range[0] = range[0].min(value);
                    range[1] = range[1].max(value);
                }
                ranges
            });
            if ranges[2][0] > plane_mm
                || ranges[2][1] < -plane_mm
                || (0..2).any(|i| ranges[i][0] >= high[i] || ranges[i][1] <= low[i])
            {
                continue;
            }
            let vertices = package.vertices();
            let facing = package.triangles().iter().find_map(|triangle| {
                let [a, b, c] = triangle.vertex_indices.map(|index| {
                    vertices
                        .get(index as usize)
                        .map(|vertex| vertex.position_mm)
                });
                let [a, b, c] = [a?, b?, c?];
                let normal = cross(sub(b, a), sub(c, a));
                let length = dot(normal, normal).sqrt();
                if length < ROUNDING {
                    return None;
                }
                let cosine = dot(normal, axes[2]) / length;
                if cosine.abs() < 1.0 - APPROXIMATION
                    || [a, b, c]
                        .iter()
                        .any(|p| dot(sub(*p, origin), axes[2]).abs() > plane_mm)
                {
                    return None;
                }
                let uv =
                    [a, b, c].map(|p| [dot(sub(p, origin), axes[0]), dot(sub(p, origin), axes[1])]);
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
            return Some(Err(DrawnShapeRefusal::ToolLength { length_mm: length }));
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
            return Some(Err(DrawnShapeRefusal::NotBuildable {
                part: host.name().to_owned(),
            }));
        }
        Some(Ok(DrawnShapeEdit {
            change: DrawnShapeChange::Definition(batch),
            part: host.name().to_owned(),
            kind: DrawnShapeKind::Pocket,
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
                self.digest = error.to_string();
                return Some(false);
            }
        };
        let planning = self.push_pull.smart_planning.take();
        self.clear_push_pull_preview();
        self.push_pull.smart_planning = planning;
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            edit.kind.digest_key(false),
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
            && parse_distance_mm(&self.push_pull.distance_input).map(f64::to_bits)
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
                .map_err(|error| failed("program.apply", error)),
            DrawnShapeChange::Definition(batch) => {
                match self.prepare_manual_push_pull_proposal(batch.clone()) {
                    Some(proposal) => self
                        .complete_mutation_with_work_recovery(|document| proposal.commit(document))
                        .map_err(|error| {
                            self.catalog.refusal_because("error-preview-stale", error)
                        }),
                    None => Err(self.catalog.refusal("error-preview-stale")),
                }
            }
        };
        if let Err(error) = applied {
            self.status_key = "error-preview-stale";
            self.digest = error.reason_text().to_owned();
            return Some(false);
        }
        self.push_pull.smart_planning = None;
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            edit.kind.digest_key(true),
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
pub(crate) mod tests;
