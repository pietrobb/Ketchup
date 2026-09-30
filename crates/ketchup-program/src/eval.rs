//! Evaluates a Starlark rule program into a [`ProgramModel`].
//!
//! Rust exposes only generic geometry builtins (`param`, `box`, `hole`,
//! `pocket`, `contact`, `joint`). Everything with a domain name (boards,
//! dowels, grooves, ...) lives in `library/prelude.star`.

// Starlark builtins mirror their keyword arguments, so some take many
// parameters; the macro-generated wrappers inherit that.
#![allow(clippy::too_many_arguments)]

use crate::expect::{Comparison, Direction, Expectation, Measure};
use crate::faces::{FaceFrame, FaceKind};
use crate::frame::{self, Mat3};
use crate::model::{
    Face, Hole, Joint, Param, Part, Pocket, ProgramArc, ProgramBoolean, ProgramBooleanKind,
    ProgramCut, ProgramEdgeFillet, ProgramEdgeFinishKind, ProgramFaceOffset, ProgramLoftSection,
    ProgramMirror, ProgramModel, ProgramOperation, ProgramPartBody, ProgramPathSegment,
    ProgramProfileSegment, ProgramShell, profile_bounds,
};
use ketchup_core::tolerance::{APPROXIMATION, MAX_COORDINATE_MM};
use serde::Serialize;
use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::dict::DictRef;
use starlark::values::float::UnpackFloat;
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;
use starlark::values::{Heap, UnpackValue, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// Contact and fit tolerance in millimetres.
pub const TOLERANCE_MM: f64 = 0.01;
/// Upper bound on generated parts, so a runaway loop fails fast.
pub const MAX_PARTS: usize = 20_000;
const PRELUDE: &str = include_str!("../library/prelude.star");

/// Why a program could not be evaluated. `message` already contains the file,
/// line, column and a source excerpt produced by the interpreter.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for ProgramError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProgramError {}

#[derive(Debug, Default)]
struct State {
    /// Name of the user's program file; frames from the prelude are not source lines.
    file_name: String,
    overrides: BTreeMap<String, f64>,
    model: RefCell<ProgramModel>,
    log: RefCell<Vec<String>>,
    part_sources: RefCell<BTreeMap<String, BTreeSet<SourceLines>>>,
}

/// An inclusive, 1-based line range of the user's program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SourceLines {
    pub first: usize,
    pub last: usize,
}

/// Records every call site in the user's program that led to this builtin
/// call, so a part can show the lines that define it, including calls made
/// through the user's own helper functions and the prelude.
fn record_source(eval: &Evaluator, state: &State, parts: &[&str]) {
    let mut spans = eval
        .call_stack()
        .frames
        .into_iter()
        .filter_map(|frame| frame.location)
        .chain(eval.call_stack_top_location())
        .filter(|location| location.filename() == state.file_name)
        .map(|location| {
            let span = location.resolve_span();
            SourceLines {
                first: span.begin.line + 1,
                last: span.end.line + 1,
            }
        })
        .peekable();
    if spans.peek().is_none() {
        return;
    }
    let spans = spans.collect::<Vec<_>>();
    let mut sources = state.part_sources.borrow_mut();
    for part in parts {
        sources
            .entry((*part).to_owned())
            .or_default()
            .extend(spans.iter().copied());
    }
}

impl starlark::PrintHandler for State {
    fn println(&self, text: &str) -> starlark::Result<()> {
        self.log.borrow_mut().push(text.to_owned());
        Ok(())
    }
}

/// Result of a successful evaluation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Evaluated {
    pub model: ProgramModel,
    /// Lines printed by the program with `print()`.
    pub log: Vec<String>,
    /// Overrides that did not match any `param()`.
    pub unused_overrides: Vec<String>,
    /// Program lines that created or changed each part, keyed by part name.
    pub part_sources: BTreeMap<String, Vec<SourceLines>>,
}

thread_local! {
    /// State of the evaluation running on this thread. Starlark evaluation is
    /// single-threaded; `evaluate` installs and removes it.
    static STATE: RefCell<Option<std::rc::Rc<State>>> = const { RefCell::new(None) };
}

fn state(_eval: &Evaluator) -> anyhow::Result<std::rc::Rc<State>> {
    STATE
        .with(|state| state.borrow().clone())
        .ok_or_else(|| anyhow::anyhow!("internal error: program state is unavailable"))
}

/// Named argument that was given and is not `None`.
fn given(value: Option<Value>) -> Option<Value> {
    value.filter(|value| !value.is_none())
}

fn text(value: Option<Value>, what: &str) -> anyhow::Result<Option<String>> {
    given(value)
        .map(|value| {
            value
                .unpack_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow::anyhow!("{what} must be a string, got {}", value.get_type()))
        })
        .transpose()
}

fn number(value: Value, what: &str) -> anyhow::Result<f64> {
    let number = UnpackFloat::unpack_value(value)
        .ok()
        .flatten()
        .ok_or_else(|| anyhow::anyhow!("{what} must be a number, got {}", value.get_type()))?
        .0;
    if !number.is_finite() || number.abs() > MAX_COORDINATE_MM {
        anyhow::bail!(
            "{what} must be a finite number within ±{MAX_COORDINATE_MM} mm, got {number}"
        );
    }
    Ok(number)
}

fn numbers<'v, const N: usize>(
    value: Value<'v>,
    heap: &'v Heap,
    what: &str,
) -> anyhow::Result<[f64; N]> {
    let items = value
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("{what} must be a list or tuple of {N} numbers"))?
        .collect::<Vec<_>>();
    if items.len() != N {
        anyhow::bail!("{what} must have exactly {N} numbers, got {}", items.len());
    }
    let mut out = [0.0; N];
    for (index, item) in items.into_iter().enumerate() {
        out[index] = number(item, &format!("{what}[{index}]"))?;
    }
    Ok(out)
}

fn profile_segments<'v>(
    value: Value<'v>,
    heap: &'v Heap,
    what: &str,
) -> anyhow::Result<Vec<ProgramProfileSegment>> {
    let items = value
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("{what} must be a list of 2D points or named segments"))?
        .collect::<Vec<_>>();
    let named = items.first().is_some_and(|item| {
        item.iterate(heap)
            .ok()
            .and_then(|mut fields| fields.next())
            .is_some_and(|field| field.unpack_str().is_some())
    });
    if items.len() < if named { 2 } else { 3 } {
        anyhow::bail!("{what} must contain at least three points or two named segments");
    }
    if named {
        let mut names = BTreeSet::new();
        return items
            .into_iter()
            .map(|segment| {
                let fields = segment
                    .iterate(heap)
                    .map_err(|_| {
                        anyhow::anyhow!("{what} named segment must be [name, start, end]")
                    })?
                    .collect::<Vec<_>>();
                let (name, start, end, arc) = match fields.as_slice() {
                    [name, start, end] => (name, start, end, None),
                    [name, start, end, arc] => (name, start, end, Some(*arc)),
                    _ => anyhow::bail!(
                        "{what} named segment must be [name, start, end] or [name, start, end, arc]"
                    ),
                };
                let name = name
                    .unpack_str()
                    .ok_or_else(|| anyhow::anyhow!("{what} segment name must be a string"))?;
                if name.is_empty()
                    || name.len() > 128
                    || name
                        .chars()
                        .any(|character| character.is_control() || "#,().:".contains(character))
                    || !names.insert(name.to_owned())
                {
                    anyhow::bail!(
                        "{what} segment names must be unique printable names without # , ( ) . :"
                    );
                }
                let start_mm = numbers::<2>(*start, heap, what)?;
                let end_mm = numbers::<2>(*end, heap, what)?;
                let bezier = arc
                    .and_then(|curve| bezier_controls(curve, heap).transpose())
                    .transpose()
                    .map_err(|error| anyhow::anyhow!("{what} segment {name:?}: {error}"))?;
                let arc = arc
                    .filter(|_| bezier.is_none())
                    .map(|arc| profile_arc(arc, start_mm, end_mm, heap))
                    .transpose()
                    .map_err(|error| anyhow::anyhow!("{what} segment {name:?}: {error}"))?;
                Ok(ProgramProfileSegment {
                    name: name.to_owned(),
                    start_mm,
                    end_mm,
                    arc,
                    bezier,
                })
            })
            .collect();
    }
    let points = items
        .into_iter()
        .map(|point| numbers::<2>(point, heap, what))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok((0..points.len())
        .map(|index| {
            ProgramProfileSegment::line(
                format!("segment{}", index + 1),
                points[index],
                points[(index + 1) % points.len()],
            )
        })
        .collect())
}

/// The two inner control points of a cubic Bezier segment given as
/// `{"controls": [(x1, y1), (x2, y2)]}`; `None` for any other dict.
fn bezier_controls<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<Option<[[f64; 2]; 2]>> {
    let Some(dict) = DictRef::from_value(value) else {
        return Ok(None);
    };
    let Some(controls) = dict.get_str("controls") else {
        return Ok(None);
    };
    if dict.len() != 1 {
        anyhow::bail!("a curve is {{\"controls\": [(x1, y1), (x2, y2)]}} with no other keys");
    }
    let points = controls
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("curve controls must be two points"))?
        .map(|point| numbers::<2>(point, heap, "curve control"))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let [first, second] = points.as_slice() else {
        anyhow::bail!("curve controls must be two points, got {}", points.len());
    };
    Ok(Some([*first, *second]))
}

/// The arc of a named segment from a dict: `{"center": (x, y),
/// "clockwise": False}`, `{"through": (x, y)}` or `{"radius": r,
/// "clockwise": False, "large": False}`.
fn profile_arc<'v>(
    value: Value<'v>,
    start: [f64; 2],
    end: [f64; 2],
    heap: &'v Heap,
) -> anyhow::Result<ProgramArc> {
    const FORMS: &str = "the arc must be {\"center\": (x, y), \"clockwise\": False}, \
                         {\"through\": (x, y)} or {\"radius\": r, \"clockwise\": False, \"large\": False}";
    let dict = DictRef::from_value(value).ok_or_else(|| anyhow::anyhow!("{FORMS}"))?;
    for key in dict.keys() {
        if !matches!(
            key.unpack_str(),
            Some("center" | "through" | "radius" | "clockwise" | "large")
        ) {
            anyhow::bail!("unknown arc key {key}; {FORMS}");
        }
    }
    let flag = |key: &str| {
        dict.get_str(key).map_or(Ok(false), |value| {
            value
                .unpack_bool()
                .ok_or_else(|| anyhow::anyhow!("arc {key} must be True or False"))
        })
    };
    let (clockwise, large) = (flag("clockwise")?, flag("large")?);
    let chord = [end[0] - start[0], end[1] - start[1]];
    let half = chord[0].hypot(chord[1]) / 2.0;
    if half <= TOLERANCE_MM {
        anyhow::bail!("an arc needs distinct start and end points (make a circle from two arcs)");
    }
    let middle = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
    let left = [-chord[1] / (2.0 * half), chord[0] / (2.0 * half)];
    let arc = match (
        dict.get_str("center"),
        dict.get_str("through"),
        dict.get_str("radius"),
    ) {
        (Some(center), None, None) => ProgramArc {
            center_mm: numbers::<2>(center, heap, "arc center")?,
            clockwise,
        },
        (None, Some(through), None) => {
            if dict.get_str("clockwise").is_some() || dict.get_str("large").is_some() {
                anyhow::bail!("an arc through a point takes no clockwise or large");
            }
            let p = numbers::<2>(through, heap, "arc through")?;
            let turn = (p[0] - start[0]) * (end[1] - p[1]) - (p[1] - start[1]) * (end[0] - p[0]);
            if turn.abs() <= TOLERANCE_MM * half {
                anyhow::bail!("the through point lies on the line from start to end; use a line");
            }
            // Centre on the chord's bisector, equally far from start and p.
            let along = ((p[0] - middle[0]).powi(2) + (p[1] - middle[1]).powi(2) - half * half)
                / (2.0 * ((p[0] - middle[0]) * left[0] + (p[1] - middle[1]) * left[1]));
            ProgramArc {
                center_mm: [middle[0] + left[0] * along, middle[1] + left[1] * along],
                clockwise: turn < 0.0,
            }
        }
        (None, None, Some(radius)) => {
            let radius = number(radius, "arc radius")?;
            if radius < half - TOLERANCE_MM {
                anyhow::bail!(
                    "arc radius {radius} is smaller than half the {} mm chord",
                    2.0 * half
                );
            }
            let offset = (radius * radius - half * half).max(0.0).sqrt();
            let side = if clockwise == large { 1.0 } else { -1.0 };
            ProgramArc {
                center_mm: [
                    middle[0] + left[0] * offset * side,
                    middle[1] + left[1] * offset * side,
                ],
                clockwise,
            }
        }
        _ => anyhow::bail!("{FORMS}"),
    };
    let c = arc.center_mm;
    let (r_start, r_end) = (
        (start[0] - c[0]).hypot(start[1] - c[1]),
        (end[0] - c[0]).hypot(end[1] - c[1]),
    );
    if (r_start - r_end).abs() > TOLERANCE_MM.max(APPROXIMATION * r_start) {
        anyhow::bail!(
            "start and end must lie equally far from the arc center ({r_start} mm vs {r_end} mm)"
        );
    }
    Ok(arc)
}

fn named_edges<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<Vec<[String; 2]>> {
    value
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("edges must be a list of [face_a, face_b] pairs"))?
        .map(|edge| {
            let faces = edge
                .iterate(heap)
                .map_err(|_| anyhow::anyhow!("each edge must be [face_a, face_b]"))?
                .collect::<Vec<_>>();
            let [first, second] = faces.as_slice() else {
                anyhow::bail!("each edge must contain exactly two face names");
            };
            let face = |value: Value<'v>| {
                value
                    .unpack_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("edge face names must be strings"))
            };
            Ok([face(*first)?, face(*second)?])
        })
        .collect()
}

fn part_name<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<String> {
    if let Some(name) = value.unpack_str() {
        return Ok(name.to_owned());
    }
    value
        .get_attr("name", heap)
        .ok()
        .flatten()
        .and_then(|name| name.unpack_str().map(ToOwned::to_owned))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "expected a part (the value returned by box()/board()) or a part name, got {}",
                value.get_type()
            )
        })
}

fn box_face(value: &str) -> anyhow::Result<Face> {
    Face::parse(value)
        .ok_or_else(|| anyhow::anyhow!("face must be one of x-, x+, y-, y+, z-, z+; got {value:?}"))
}

fn items<'v>(value: Value<'v>, heap: &'v Heap, what: &str) -> anyhow::Result<Vec<Value<'v>>> {
    Ok(value
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("{what} must be a list or tuple, got {}", value.get_type()))?
        .collect())
}

/// `(x, y, z)` in world, or `(part, "z+")`: that face's outward normal
/// wherever the part ends up.
fn expect_direction<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<Direction> {
    let values = items(value, heap, "direction")?;
    if let [part, face_name] = values.as_slice()
        && let Some(face_name) = face_name.unpack_str()
    {
        return Ok(Direction::Face {
            part: part_name(*part, heap)?,
            face: box_face(face_name)?,
        });
    }
    let vector = numbers::<3>(value, heap, "direction")?;
    frame::normalized(vector)
        .map(Direction::World)
        .ok_or_else(|| anyhow::anyhow!("direction must be non-zero"))
}

/// `("reach", part, direction)`, `("distance", a, b)` or
/// `("contact_area", a, b[, face_of_a])`.
fn expect_measure<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<Measure> {
    let values = items(value, heap, "measure")?;
    let kind = values
        .first()
        .and_then(|kind| kind.unpack_str())
        .unwrap_or("");
    let part = |index: usize| part_name(values[index], heap);
    Ok(match (kind, values.len()) {
        ("reach", 3) => Measure::Reach {
            part: part(1)?,
            direction: expect_direction(values[2], heap)?,
        },
        ("distance", 3) => Measure::Distance {
            a: part(1)?,
            b: part(2)?,
        },
        ("contact_area", 3 | 4) => Measure::ContactArea {
            a: part(1)?,
            b: part(2)?,
            face: values
                .get(3)
                .map(|face_name| {
                    box_face(face_name.unpack_str().ok_or_else(|| {
                        anyhow::anyhow!("contact_area face must be a string like \"z+\"")
                    })?)
                })
                .transpose()?,
        },
        _ => anyhow::bail!(
            "measure must be (\"reach\", part, direction), (\"distance\", a, b) or \
             (\"contact_area\", a, b[, face]); got {value}"
        ),
    })
}

fn measure_parts(measure: &Measure) -> Vec<&str> {
    match measure {
        Measure::Reach { part, direction } => match direction {
            Direction::Face { part: of, .. } => vec![part, of],
            Direction::World(_) => vec![part],
        },
        Measure::Distance { a, b } | Measure::ContactArea { a, b, .. } => vec![a, b],
    }
}

fn part_value<'v>(part: &Part, heap: &'v Heap) -> Value<'v> {
    let triple = |value: [f64; 3]| heap.alloc((value[0], value[1], value[2]));
    let (min, max) = part.world_bounds();
    let (local_min, local_max) = part.local_bounds();
    heap.alloc(AllocStruct([
        ("name", heap.alloc(part.name.as_str())),
        ("size", triple(part.size_mm)),
        ("at", triple(part.at_mm)),
        ("min", triple(min)),
        ("max", triple(max)),
        ("local_min", triple(local_min)),
        ("local_max", triple(local_max)),
        ("x", triple(frame::axis(&part.rotation, 0))),
        ("y", triple(frame::axis(&part.rotation, 1))),
        ("z", triple(frame::axis(&part.rotation, 2))),
    ]))
}

/// A face as a program value: struct(part, name, kind, origin, normal, u, v,
/// min, max, radius, center) in world.
fn face_value<'v>(part: &Part, face: &FaceFrame, heap: &'v Heap) -> Value<'v> {
    let world = part.world_face(face);
    let triple = |value: [f64; 3]| heap.alloc((value[0], value[1], value[2]));
    let pair = |value: [f64; 2]| heap.alloc((value[0], value[1]));
    let (kind, radius) = match world.kind {
        FaceKind::Planar => ("planar", Value::new_none()),
        FaceKind::Cylindrical { radius_mm } => ("cylindrical", heap.alloc(radius_mm)),
    };
    heap.alloc(AllocStruct([
        ("part", heap.alloc(part.name.as_str())),
        ("name", heap.alloc(world.name.as_str())),
        ("kind", heap.alloc(kind)),
        ("origin", triple(world.origin_mm)),
        ("normal", triple(world.normal)),
        ("u", triple(world.u)),
        ("v", triple(world.v)),
        ("min", pair(world.min)),
        ("max", pair(world.max)),
        ("radius", radius),
        ("center", triple(world.center())),
    ]))
}

fn known_part<'m>(model: &'m ProgramModel, name: &str) -> anyhow::Result<&'m Part> {
    model
        .part(name)
        .or_else(|| model.tool(name))
        .ok_or_else(|| anyhow::anyhow!("unknown part {name:?}"))
}

/// Distance or overlap between two parts' boxes in their own frames.
struct Relation {
    distance: f64,
    overlap: f64,
    touching: bool,
    direction: [f64; 3],
}

impl Relation {
    fn between(a: &Part, b: &Part) -> Self {
        let (a, b) = (a.obb(), b.obb());
        let (direction, separation) = a.separating_axis(&b);
        Self {
            distance: if separation > TOLERANCE_MM {
                a.distance(&b)
            } else {
                0.0
            },
            overlap: if separation < -TOLERANCE_MM {
                -separation
            } else {
                0.0
            },
            touching: separation.abs() <= TOLERANCE_MM,
            direction,
        }
    }

    fn fields<'v>(&self, heap: &'v Heap) -> [(&'static str, Value<'v>); 4] {
        let d = self.direction;
        [
            ("distance", heap.alloc(self.distance)),
            ("overlap", heap.alloc(self.overlap)),
            ("touching", Value::new_bool(self.touching)),
            ("direction", heap.alloc((d[0], d[1], d[2]))),
        ]
    }

    fn value<'v>(&self, heap: &'v Heap) -> Value<'v> {
        heap.alloc(AllocStruct(self.fields(heap)))
    }

    fn value_named<'v>(&self, heap: &'v Heap, name: &str) -> Value<'v> {
        let [distance, overlap, touching, direction] = self.fields(heap);
        heap.alloc(AllocStruct([
            ("name", heap.alloc(name)),
            distance,
            overlap,
            touching,
            direction,
        ]))
    }
}

/// Applies a world rigid motion `p -> pivot + rotation·(p - pivot)` to a part.
fn rotate_part(part: &mut Part, rotation: &Mat3, pivot: [f64; 3]) {
    move_part(part, rotation, pivot);
    part.refresh_feature_tree();
}

/// Moves a part and the tools of its booleans rigidly, so cuts already made
/// stay where they are on the part, like its holes and pockets.
fn move_part(part: &mut Part, rotation: &Mat3, pivot: [f64; 3]) {
    let offset = frame::apply(
        rotation,
        std::array::from_fn(|axis| part.at_mm[axis] - pivot[axis]),
    );
    part.at_mm = std::array::from_fn(|axis| pivot[axis] + offset[axis]);
    part.rotation = frame::multiply(rotation, &part.rotation);
    for boolean in part.booleans_mut() {
        move_part(&mut boolean.tool, rotation, pivot);
    }
}

/// Sets a part's frame to `origin`/`rotation`, carrying its booleans along.
fn set_frame(part: &mut Part, origin: [f64; 3], rotation: Mat3) {
    // The motion old frame -> new frame is p -> new_R·old_Rᵀ·(p - old_at) + origin,
    // i.e. a rotation about old_at followed by a shift of origin - old_at.
    let motion = frame::multiply(&rotation, &frame::transposed(&part.rotation));
    let old_at = part.at_mm;
    move_part(part, &motion, old_at);
    let shift: [f64; 3] = std::array::from_fn(|axis| origin[axis] - old_at[axis]);
    translate_part(part, shift);
    part.at_mm = origin;
    part.rotation = rotation;
    part.refresh_feature_tree();
}

fn translate_part(part: &mut Part, shift: [f64; 3]) {
    part.at_mm = std::array::from_fn(|axis| part.at_mm[axis] + shift[axis]);
    for boolean in part.booleans_mut() {
        translate_part(&mut boolean.tool, shift);
    }
}

/// Runs `f` on a part or tool body by name.
fn with_part<R>(
    state: &State,
    name: &str,
    f: impl FnOnce(&mut Part) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    let mut model = state.model.borrow_mut();
    let model = &mut *model;
    let part = model
        .parts
        .iter_mut()
        .chain(model.tools.iter_mut())
        .find(|part| part.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown part {name:?}; create it with box() first"))?;
    f(part)
}

fn check_part_name(name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        anyhow::bail!("part name must be 1-128 printable bytes, got {name:?}");
    }
    Ok(())
}

/// Adds a part, or a tool body when `tool` is true. Names are shared by both.
fn insert_part(state: &State, mut part: Part, tool: bool) -> anyhow::Result<Part> {
    part.refresh_feature_tree();
    let mut model = state.model.borrow_mut();
    let name = &part.name;
    if model.part(name).is_some() || model.tool(name).is_some() {
        anyhow::bail!("part {name:?} already exists; part names are identities and must be unique");
    }
    if model.parts.len() + model.tools.len() >= MAX_PARTS {
        anyhow::bail!("more than {MAX_PARTS} parts; check the program for a runaway loop");
    }
    if tool {
        model.tools.push(part.clone());
    } else {
        model.parts.push(part.clone());
    }
    Ok(part)
}

fn new_part(name: &str, size_mm: [f64; 3], at_mm: [f64; 3], body: ProgramPartBody) -> Part {
    Part {
        name: name.to_owned(),
        size_mm,
        at_mm,
        rotation: frame::IDENTITY,
        material: None,
        grain_axis: None,
        color: None,
        body,
        operations: Vec::new(),
        features: Vec::new(),
        holes: Vec::new(),
        pockets: Vec::new(),
    }
}

fn insert_profile_part(
    state: &State,
    name: &str,
    at_mm: [f64; 3],
    body: ProgramPartBody,
    tool: bool,
) -> anyhow::Result<Part> {
    check_part_name(name)?;
    let size_mm = match &body {
        ProgramPartBody::Extrusion {
            segments,
            distance_mm,
        } => {
            let (min, max) = profile_bounds(segments);
            [max[0] - min[0], max[1] - min[1], *distance_mm]
        }
        ProgramPartBody::Revolve { segments, .. } => {
            let (min, max) = profile_bounds(segments);
            let radius = max[0].abs().max(min[0].abs());
            [radius * 2.0, max[1] - min[1], radius * 2.0]
        }
        ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. } => {
            let (min, max) = new_part(name, [0.0; 3], at_mm, body.clone()).local_bounds();
            std::array::from_fn(|axis| max[axis] - min[axis])
        }
        ProgramPartBody::Panel => unreachable!(),
    };
    if size_mm.iter().any(|value| *value <= TOLERANCE_MM) {
        anyhow::bail!("part {name:?}: profile and body dimensions must be positive");
    }
    insert_part(state, new_part(name, size_mm, at_mm, body), tool)
}

fn reject_swept(part: &Part, operation: &str) -> anyhow::Result<()> {
    if matches!(
        part.body,
        ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. }
    ) {
        anyhow::bail!(
            "{operation} on {:?}: a swept or lofted part has no named faces; shape it with subtract() or intersect() and a tool body",
            part.name
        );
    }
    Ok(())
}

/// A sweep path: 3D points (a polyline whose corners `bend` rounds), or
/// segments `[start, end]` and `[start, end, arc]` with the arc
/// `{"through": p}` or `{"center": c, "normal": n}` (counter-clockwise
/// about `n`).
fn sweep_path<'v>(
    value: Value<'v>,
    bend: Option<f64>,
    heap: &'v Heap,
) -> anyhow::Result<Vec<ProgramPathSegment>> {
    const FORMS: &str = "path must be a list of 3D points, or of segments [start, end] / [start, end, {\"through\": p}] / [start, end, {\"center\": c, \"normal\": n}]";
    let entries = items(value, heap, "path")?;
    let is_point = |entry: &Value<'v>| {
        entry
            .iterate(heap)
            .ok()
            .and_then(|mut fields| fields.next())
            .is_some_and(|first| UnpackFloat::unpack_value(first).ok().flatten().is_some())
    };
    let path = if entries.iter().all(is_point) {
        let points = entries
            .into_iter()
            .map(|point| numbers::<3>(point, heap, "path point"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        crate::path::polyline(&points, bend.unwrap_or(0.0)).map_err(anyhow::Error::msg)?
    } else {
        if bend.is_some() {
            anyhow::bail!("bend rounds the corners of a point path; segments give their arcs");
        }
        let mut previous_end: Option<[f64; 3]> = None;
        entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let what = format!("path segment {}", index + 1);
                let fields = items(entry, heap, &what)?;
                let (start, end, arc) = match fields.as_slice() {
                    [start, end] => (*start, *end, None),
                    [start, end, arc] => (*start, *end, Some(*arc)),
                    _ => anyhow::bail!("{FORMS}"),
                };
                let mut start_mm = numbers::<3>(start, heap, &what)?;
                // Endpoints typed twice may differ in the last digits.
                if let Some(previous) = previous_end
                    && (0..3).all(|axis| (previous[axis] - start_mm[axis]).abs() <= TOLERANCE_MM)
                {
                    start_mm = previous;
                }
                let end_mm = numbers::<3>(end, heap, &what)?;
                previous_end = Some(end_mm);
                let arc = arc
                    .map(|arc| {
                        let dict =
                            DictRef::from_value(arc).ok_or_else(|| anyhow::anyhow!("{FORMS}"))?;
                        let point = |key: &str| {
                            dict.get_str(key)
                                .map(|value| numbers::<3>(value, heap, &format!("arc {key}")))
                                .transpose()
                        };
                        if dict.keys().any(|key| {
                            !matches!(key.unpack_str(), Some("through" | "center" | "normal"))
                        }) {
                            anyhow::bail!("{FORMS}");
                        }
                        match (point("through")?, point("center")?, point("normal")?) {
                            (Some(through), None, None) => {
                                crate::path::arc_through(start_mm, end_mm, through)
                            }
                            (None, Some(center), Some(normal)) => {
                                crate::path::arc_about(start_mm, end_mm, center, normal)
                            }
                            _ => return Err(anyhow::anyhow!("{FORMS}")),
                        }
                        .map_err(anyhow::Error::msg)
                    })
                    .transpose()
                    .map_err(|error| anyhow::anyhow!("{what}: {error}"))?;
                Ok(ProgramPathSegment {
                    start_mm,
                    end_mm,
                    arc,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    crate::path::validate(&path).map_err(anyhow::Error::msg)?;
    Ok(path)
}

/// Loft sections `[(profile, z), ...]` at strictly increasing heights.
fn loft_sections<'v>(value: Value<'v>, heap: &'v Heap) -> anyhow::Result<Vec<ProgramLoftSection>> {
    let entries = items(value, heap, "sections")?;
    if !(2..=16).contains(&entries.len()) {
        anyhow::bail!("a loft needs 2 to 16 sections, got {}", entries.len());
    }
    let mut sections: Vec<ProgramLoftSection> = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let what = format!("section {}", index + 1);
        let [profile, z] = items(entry, heap, &what)?[..] else {
            anyhow::bail!("{what} must be (profile, z)");
        };
        let elevation_mm = number(z, &format!("{what} z"))?;
        if let Some(previous) = sections.last()
            && elevation_mm <= previous.elevation_mm + TOLERANCE_MM
        {
            anyhow::bail!(
                "{what} z = {elevation_mm} must be above the previous section's {}",
                previous.elevation_mm
            );
        }
        sections.push(ProgramLoftSection {
            segments: profile_segments(profile, heap, &format!("{what} profile"))?,
            elevation_mm,
        });
    }
    Ok(sections)
}

/// Records `target <kind> tool` on the target part.
fn apply_boolean<'v>(
    part: Value<'v>,
    tool: Value<'v>,
    name: Option<Value<'v>>,
    kind: ProgramBooleanKind,
    eval: &mut Evaluator<'v, '_, '_>,
) -> anyhow::Result<Value<'v>> {
    let heap = eval.heap();
    let target = part_name(part, heap)?;
    let tool = part_name(tool, heap)?;
    let operation = match kind {
        ProgramBooleanKind::Subtract => "subtract",
        ProgramBooleanKind::Intersect => "intersect",
        ProgramBooleanKind::Union => "union",
    };
    let state = state(eval)?;
    let tool_part = {
        let model = state.model.borrow();
        model
            .part(&tool)
            .or_else(|| model.tool(&tool))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{operation}(): unknown tool {tool:?}"))?
    };
    if tool == target {
        anyhow::bail!("{operation}({target:?}, {tool:?}): a part cannot be its own tool");
    }
    let requested = text(name, "name")?;
    record_source(eval, &state, &[&target]);
    with_part(&state, &target, |part| {
        let name = requested.unwrap_or_else(|| format!("{operation} {tool}"));
        check_part_name(&name)?;
        add_operation(
            part,
            ProgramOperation::Boolean(Box::new(ProgramBoolean {
                name,
                kind,
                tool: tool_part,
            })),
        )?;
        Ok(part_value(part, heap))
    })
}

/// A shared face patch of positive area between two parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    /// Local axis of the first part the touching faces are perpendicular to.
    pub axis: usize,
    /// Face of the first part that touches the second, in the first part's frame.
    pub face_a: Face,
    /// Face of the second part that touches the first, in its own frame.
    pub face_b: Face,
    /// World bounds of the patch.
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
    /// World unit normal pointing out of the first part.
    pub normal: [f64; 3],
    /// World directions of `face_a`'s (u, v) axes; the patch's bounding
    /// rectangle along them starts at `origin_mm` and measures `size_mm`.
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub origin_mm: [f64; 3],
    pub size_mm: [f64; 2],
    /// World corners of the convex patch.
    pub points_mm: Vec<[f64; 3]>,
}

/// Finds the face patch where `a` touches `b`, if any. Faces are the uncut
/// box faces of each part in its own frame.
#[must_use]
pub fn contact(a: &Part, b: &Part) -> Option<Contact> {
    if a.is_rotated() || b.is_rotated() {
        return rotated_contact(a, b);
    }
    let ((a_min, a_max), (b_min, b_max)) = (a.world_bounds(), b.world_bounds());
    for axis in 0..3 {
        for max_side in [true, false] {
            let plane = if max_side { a_max[axis] } else { a_min[axis] };
            let other = if max_side { b_min[axis] } else { b_max[axis] };
            if (plane - other).abs() > TOLERANCE_MM {
                continue;
            }
            let mut min = [0.0; 3];
            let mut max = [0.0; 3];
            let mut area_ok = true;
            for other_axis in (0..3).filter(|candidate| *candidate != axis) {
                min[other_axis] = a_min[other_axis].max(b_min[other_axis]);
                max[other_axis] = a_max[other_axis].min(b_max[other_axis]);
                area_ok &= max[other_axis] - min[other_axis] > TOLERANCE_MM;
            }
            if area_ok {
                min[axis] = plane;
                max[axis] = plane;
                let face_a = Face::from_axis(axis, max_side);
                let (u_axis, v_axis) = face_a.uv_axes();
                let unit = |index: usize| -> [f64; 3] {
                    std::array::from_fn(|i| if i == index { 1.0 } else { 0.0 })
                };
                let corner = |u: f64, v: f64| {
                    let mut point = min;
                    point[u_axis] = u;
                    point[v_axis] = v;
                    point
                };
                return Some(Contact {
                    axis,
                    face_a,
                    face_b: face_a.opposite(),
                    min_mm: min,
                    max_mm: max,
                    normal: unit(axis).map(|value| if max_side { value } else { -value }),
                    u: unit(u_axis),
                    v: unit(v_axis),
                    origin_mm: min,
                    size_mm: [max[u_axis] - min[u_axis], max[v_axis] - min[v_axis]],
                    points_mm: vec![
                        corner(min[u_axis], min[v_axis]),
                        corner(max[u_axis], min[v_axis]),
                        corner(max[u_axis], max[v_axis]),
                        corner(min[u_axis], max[v_axis]),
                    ],
                });
            }
        }
    }
    None
}

fn rotated_contact(a: &Part, b: &Part) -> Option<Contact> {
    let patch = a.obb().face_contact(&b.obb(), TOLERANCE_MM)?;
    let (min_mm, max_mm) = patch.points.iter().fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(min, max), point| {
            (
                std::array::from_fn(|i| f64::min(min[i], point[i])),
                std::array::from_fn(|i| f64::max(max[i], point[i])),
            )
        },
    );
    Some(Contact {
        axis: patch.face.0,
        face_a: Face::from_axis(patch.face.0, patch.face.1),
        face_b: Face::from_axis(patch.other_face.0, patch.other_face.1),
        min_mm,
        max_mm,
        normal: patch.normal,
        u: patch.u,
        v: patch.v,
        origin_mm: patch.origin,
        size_mm: patch.size,
        points_mm: patch.points,
    })
}

/// A mirror is a part's last step: its faces keep their names on the other
/// side, so later steps would address them in the wrong place.
fn reject_mirrored(part: &Part, step: &str) -> anyhow::Result<()> {
    if let Some(mirror) = part
        .operations
        .iter()
        .find(|operation| matches!(operation, ProgramOperation::Mirror(_)))
    {
        anyhow::bail!(
            "{step:?} on {:?} comes after its mirror {:?}; shape the part first and mirror it last",
            part.name,
            mirror.name()
        );
    }
    Ok(())
}

/// Appends one operation to a part, keeping operation names unique.
fn add_operation(part: &mut Part, operation: ProgramOperation) -> anyhow::Result<()> {
    reject_mirrored(part, operation.name())?;
    let name = operation.name();
    if part
        .operations
        .iter()
        .any(|existing| existing.name() == name)
    {
        anyhow::bail!(
            "feature name {name:?} is already used on {:?}; pass a unique name=",
            part.name
        );
    }
    part.operations.push(operation);
    part.refresh_feature_tree();
    Ok(())
}

fn apply_named_edge_finish(
    state: &State,
    part_name: &str,
    edges: Vec<[String; 2]>,
    amount_mm: f64,
    requested_name: Option<String>,
    kind: ProgramEdgeFinishKind,
) -> anyhow::Result<NoneType> {
    let operation = match kind {
        ProgramEdgeFinishKind::Fillet => "fillet",
        ProgramEdgeFinishKind::Chamfer => "chamfer",
    };
    if edges.is_empty() {
        anyhow::bail!("{operation} on {part_name:?}: edges must not be empty");
    }
    if amount_mm <= TOLERANCE_MM {
        anyhow::bail!("{operation} on {part_name:?}: amount must be positive");
    }
    with_part(state, part_name, |part| {
        let mut unique = BTreeSet::new();
        for [first, second] in &edges {
            if first == second {
                anyhow::bail!("{operation} edge faces must be different, got {first:?} twice");
            }
            let mut pair = [
                part.exact_face_label(first)
                    .map_err(|error| anyhow::anyhow!("{operation} on {part_name:?}: {error}"))?,
                part.exact_face_label(second)
                    .map_err(|error| anyhow::anyhow!("{operation} on {part_name:?}: {error}"))?,
            ];
            pair.sort_unstable();
            if !unique.insert(pair) {
                anyhow::bail!("{operation} edge ({first:?}, {second:?}) is listed more than once");
            }
        }
        let count = part.finishes().count();
        let name = requested_name
            .clone()
            .unwrap_or_else(|| format!("{part_name} {operation} {}", count + 1));
        if name.trim().is_empty() {
            anyhow::bail!("{operation} name must be non-empty");
        }
        add_operation(
            part,
            ProgramOperation::Finish(ProgramEdgeFillet {
                name,
                kind,
                edges,
                radius_mm: amount_mm,
            }),
        )?;
        Ok(NoneType)
    })
}

#[starlark_module]
fn builtins(builder: &mut GlobalsBuilder) {
    /// A named number the caller can override. Returns an int when the
    /// default is an int and the value is whole.
    fn param<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] default: Value<'v>,
        #[starlark(require = named)] min: Option<Value<'v>>,
        #[starlark(require = named)] max: Option<Value<'v>>,
        #[starlark(require = named, default = "")] doc: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let integer_default = default.unpack_i32().is_some();
        let default = number(default, &format!("param({name:?}) default"))?;
        let min = given(min).map(|value| number(value, "min")).transpose()?;
        let max = given(max).map(|value| number(value, "max")).transpose()?;
        let state = state(eval)?;
        let value = state.overrides.get(name).copied().unwrap_or(default);
        {
            let mut model = state.model.borrow_mut();
            if model.params.iter().any(|param| param.name == name) {
                anyhow::bail!("param {name:?} is defined twice");
            }
            model.params.push(Param {
                name: name.to_owned(),
                value,
                default,
                min,
                max,
                doc: doc.to_owned(),
            });
        }
        let heap = eval.heap();
        if integer_default && value.fract() == 0.0 && value.abs() < 1.0e9 {
            #[allow(clippy::cast_possible_truncation)]
            return Ok(heap.alloc(value as i32));
        }
        Ok(heap.alloc(value))
    }

    /// A cuboid part occupying `0..size` in its own frame, with its origin at
    /// `at`. `tool=True` makes a helper body for subtract()/intersect() that is
    /// never built, listed or validated.
    fn r#box<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] size: Value<'v>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named)] material: Option<Value<'v>>,
        #[starlark(require = named)] grain: Option<Value<'v>>,
        #[starlark(require = named)] color: Option<Value<'v>>,
        #[starlark(require = named, default = false)] tool: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        check_part_name(name)?;
        let size = numbers::<3>(size, heap, "size")?;
        if let Some(axis) = size.iter().position(|value| *value <= TOLERANCE_MM) {
            anyhow::bail!(
                "part {name:?}: size[{axis}] must be positive, got {}",
                size[axis]
            );
        }
        let at = given(at).map_or(Ok([0.0; 3]), |at| numbers::<3>(at, heap, "at"))?;
        let material = text(material, "material")?;
        let grain_axis = text(grain, "grain")?
            .map(|grain| match grain.as_str() {
                "x" => Ok(0),
                "y" => Ok(1),
                "z" => Ok(2),
                other => Err(anyhow::anyhow!(
                    "grain must be \"x\", \"y\" or \"z\", got {other:?}"
                )),
            })
            .transpose()?;
        let color = given(color)
            .map(|color| {
                let rgb = numbers::<3>(color, heap, "color")?;
                if rgb.iter().any(|channel| !(0.0..=255.0).contains(channel)) {
                    anyhow::bail!("color channels must be 0-255");
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(rgb.map(|channel| channel.round() as u8))
            })
            .transpose()?;
        let state = state(eval)?;
        let part = insert_part(
            &state,
            Part {
                material,
                grain_axis,
                color,
                ..new_part(name, size, at, ProgramPartBody::Panel)
            },
            tool,
        )?;
        record_source(eval, &state, &[name]);
        Ok(part_value(&part, heap))
    }

    /// Extrudes one closed point loop or list of named segments along +Z.
    fn extrude<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] profile: Value<'v>,
        #[starlark(require = named)] distance: Value<'v>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named, default = false)] tool: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let segments = profile_segments(profile, heap, "profile")?;
        let distance_mm = number(distance, "distance")?;
        if distance_mm <= TOLERANCE_MM {
            anyhow::bail!("distance must be positive");
        }
        let at_mm = given(at).map_or(Ok([0.0; 3]), |at| numbers::<3>(at, heap, "at"))?;
        let state = state(eval)?;
        let part = insert_profile_part(
            &state,
            name,
            at_mm,
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
            },
            tool,
        )?;
        record_source(eval, &state, &[name]);
        Ok(part_value(&part, heap))
    }

    /// Revolves one closed point loop or list of named segments around an in-plane axis.
    fn revolve<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] profile: Value<'v>,
        #[starlark(require = named)] axis: Value<'v>,
        #[starlark(require = named)] angle: Option<Value<'v>>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named, default = false)] tool: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let segments = profile_segments(profile, heap, "profile")?;
        let axis_points = axis
            .iterate(heap)
            .map_err(|_| anyhow::anyhow!("axis must contain two 2D points"))?
            .map(|point| numbers::<2>(point, heap, "axis"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let [axis_start_mm, axis_end_mm] = axis_points.as_slice() else {
            anyhow::bail!("axis must contain exactly two 2D points");
        };
        let angle = given(angle)
            .map(|angle| number(angle, "angle"))
            .transpose()?
            .unwrap_or(360.0);
        if !angle.is_finite() || !(0.0..=360.0).contains(&angle) || angle == 0.0 {
            anyhow::bail!("angle must be within (0, 360] degrees");
        }
        let at_mm = given(at).map_or(Ok([0.0; 3]), |at| numbers::<3>(at, heap, "at"))?;
        let state = state(eval)?;
        let part = insert_profile_part(
            &state,
            name,
            at_mm,
            ProgramPartBody::Revolve {
                segments,
                axis_start_mm: *axis_start_mm,
                axis_end_mm: *axis_end_mm,
                angle_degrees: angle,
            },
            tool,
        )?;
        record_source(eval, &state, &[name]);
        Ok(part_value(&part, heap))
    }

    /// Carries a closed profile along a smooth path of lines and arcs. The
    /// profile's (u, v) plane stands square to the path at its start; going
    /// horizontally, u points right of the direction of travel and v up.
    fn sweep<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] profile: Value<'v>,
        #[starlark(require = named)] path: Value<'v>,
        #[starlark(require = named)] bend: Option<Value<'v>>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named, default = false)] tool: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let segments = profile_segments(profile, heap, "profile")?;
        let bend = given(bend).map(|bend| number(bend, "bend")).transpose()?;
        if bend.is_some_and(|bend| bend <= TOLERANCE_MM) {
            anyhow::bail!("bend must be a positive radius");
        }
        let path = sweep_path(path, bend, heap)?;
        let at_mm = given(at).map_or(Ok([0.0; 3]), |at| numbers::<3>(at, heap, "at"))?;
        let state = state(eval)?;
        let part = insert_profile_part(
            &state,
            name,
            at_mm,
            ProgramPartBody::Sweep { segments, path },
            tool,
        )?;
        record_source(eval, &state, &[name]);
        Ok(part_value(&part, heap))
    }

    /// A solid through closed profiles `sections=[(profile, z), ...]`, each
    /// in the part's XY plane at height z (strictly increasing).
    fn loft<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] sections: Value<'v>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named, default = false)] tool: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let sections = loft_sections(sections, heap)?;
        let at_mm = given(at).map_or(Ok([0.0; 3]), |at| numbers::<3>(at, heap, "at"))?;
        let state = state(eval)?;
        let part = insert_profile_part(
            &state,
            name,
            at_mm,
            ProgramPartBody::Loft { sections },
            tool,
        )?;
        record_source(eval, &state, &[name]);
        Ok(part_value(&part, heap))
    }

    /// Rounds edges identified by the two named faces that meet there.
    fn fillet<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] edges: Value<'v>,
        #[starlark(require = named)] radius: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let edges = named_edges(edges, heap)?;
        let radius_mm = number(radius, "radius")?;
        let requested_name = text(name, "name")?;
        let state = state(eval)?;
        apply_named_edge_finish(
            &state,
            &part_name,
            edges,
            radius_mm,
            requested_name,
            ProgramEdgeFinishKind::Fillet,
        )?;
        record_source(eval, &state, &[&part_name]);
        Ok(NoneType)
    }

    /// Bevels edges identified by the two named faces that meet there.
    fn chamfer<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] edges: Value<'v>,
        #[starlark(require = named)] distance: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let edges = named_edges(edges, heap)?;
        let distance_mm = number(distance, "distance")?;
        let requested_name = text(name, "name")?;
        let state = state(eval)?;
        apply_named_edge_finish(
            &state,
            &part_name,
            edges,
            distance_mm,
            requested_name,
            ProgramEdgeFinishKind::Chamfer,
        )?;
        record_source(eval, &state, &[&part_name]);
        Ok(NoneType)
    }

    /// Cuts a named closed profile down from the body's current top face.
    fn cut<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] profile: Value<'v>,
        #[starlark(require = named)] depth: Value<'v>,
        #[starlark(require = named)] name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let segments = profile_segments(profile, heap, "profile")?;
        let depth_mm = number(depth, "depth")?;
        if depth_mm <= TOLERANCE_MM {
            anyhow::bail!("cut on {part_name:?}: depth must be positive");
        }
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            anyhow::bail!("cut name must be 1-128 printable bytes, got {name:?}");
        }
        let state = state(eval)?;
        record_source(eval, &state, &[&part_name]);
        with_part(&state, &part_name, |part| {
            reject_swept(part, "cut")?;
            add_operation(
                part,
                ProgramOperation::Cut(ProgramCut {
                    name: name.to_owned(),
                    segments,
                    depth_mm,
                }),
            )?;
            Ok(NoneType)
        })
    }

    /// Moves one program-named planar face along its outward normal.
    fn push_pull<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] face: &str,
        #[starlark(require = named)] distance: Value<'v>,
        #[starlark(require = named)] name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let distance_mm = number(distance, "distance")?;
        if distance_mm.abs() <= TOLERANCE_MM {
            anyhow::bail!("push_pull on {part_name:?}: distance must be non-zero");
        }
        if face.trim().is_empty() || face.len() > 128 || face.chars().any(char::is_control) {
            anyhow::bail!("push_pull face must be 1-128 printable bytes, got {face:?}");
        }
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            anyhow::bail!("push_pull name must be 1-128 printable bytes, got {name:?}");
        }
        let state = state(eval)?;
        record_source(eval, &state, &[&part_name]);
        with_part(&state, &part_name, |part| {
            reject_swept(part, "push_pull")?;
            part.exact_face_label(face)
                .map_err(|error| anyhow::anyhow!("push_pull on {part_name:?}: {error}"))?;
            add_operation(
                part,
                ProgramOperation::FaceOffset(ProgramFaceOffset {
                    name: name.to_owned(),
                    face: face.to_owned(),
                    distance_mm,
                }),
            )?;
            Ok(NoneType)
        })
    }

    /// Reflects the part's solid, as shaped so far, across its own middle
    /// plane across local `axis` ("x", "y" or "z"); its bounds stay. Faces keep
    /// the names of the faces they mirror. It is the part's last shaping step.
    fn mirror<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named, default = "x")] axis: &str,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let axis_index = match axis {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => anyhow::bail!(
                "mirror({part_name:?}): axis must be \"x\", \"y\" or \"z\", got {axis:?}"
            ),
        };
        let name = text(name, "name")?.unwrap_or_else(|| format!("{part_name} mirror {axis}"));
        let state = state(eval)?;
        record_source(eval, &state, &[&part_name]);
        with_part(&state, &part_name, |part| {
            if !(part.holes.is_empty() && part.pockets.is_empty()) {
                anyhow::bail!(
                    "mirror({part_name:?}): the part has holes or pockets; mirror it first and drill the mirrored part"
                );
            }
            let (min, max) = part.local_bounds();
            add_operation(
                part,
                ProgramOperation::Mirror(ProgramMirror {
                    name,
                    axis: axis_index,
                    center_mm: (min[axis_index] + max[axis_index]) / 2.0,
                }),
            )?;
            Ok(part_value(part, heap))
        })
    }

    /// Hollows the part to walls `thickness` mm thick, measured inwards, open
    /// at the faces `open` (at least one). The inner wall following face F is
    /// "<name>.F".
    fn shell<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] thickness: Value<'v>,
        #[starlark(require = named)] open: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let thickness_mm = number(thickness, "thickness")?;
        let open = items(open, heap, "open")?
            .into_iter()
            .map(|face| {
                face.unpack_str().map(str::to_owned).ok_or_else(|| {
                    anyhow::anyhow!("shell({part_name:?}): open must be a list of face names")
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let name = text(name, "name")?.unwrap_or_else(|| format!("{part_name} shell"));
        if name.trim().is_empty() || name.contains(['#', ',', '(', ')', '.', ':']) {
            anyhow::bail!(
                "shell({part_name:?}): name {name:?} must be non-empty without # , ( ) . or :, it prefixes the inner walls"
            );
        }
        let state = state(eval)?;
        record_source(eval, &state, &[&part_name]);
        with_part(&state, &part_name, |part| {
            reject_swept(part, "shell")?;
            if open.is_empty() {
                anyhow::bail!(
                    "shell({part_name:?}): open at least one face, e.g. open=[\"z+\"]; a closed hollow is not supported"
                );
            }
            let (min, max) = part.local_bounds();
            let thinnest = (0..3)
                .map(|axis| max[axis] - min[axis])
                .fold(f64::INFINITY, f64::min);
            if thickness_mm <= TOLERANCE_MM || 2.0 * thickness_mm >= thinnest {
                anyhow::bail!(
                    "shell({part_name:?}): thickness must be positive and under half the part's thinnest size {thinnest} mm, got {thickness_mm}"
                );
            }
            let mut unique = BTreeSet::new();
            for face in &open {
                part.exact_face_label(face)
                    .map_err(|error| anyhow::anyhow!("shell on {part_name:?}: {error}"))?;
                if !unique.insert(face.as_str()) {
                    anyhow::bail!("shell({part_name:?}): face {face:?} is listed twice");
                }
            }
            add_operation(
                part,
                ProgramOperation::Shell(ProgramShell {
                    name,
                    thickness_mm,
                    open,
                }),
            )?;
            Ok(part_value(part, heap))
        })
    }

    /// Rotates a part by `angle` degrees (right hand) about the world direction
    /// `axis` through `pivot` (default: the part's origin `at`). Rotations compose.
    fn rotate<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] axis: Value<'v>,
        #[starlark(require = named)] angle: Value<'v>,
        #[starlark(require = named)] pivot: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let axis = numbers::<3>(axis, heap, "axis")?;
        let angle = number(angle, "angle")?;
        let rotation = frame::axis_angle(axis, angle).ok_or_else(|| {
            anyhow::anyhow!("rotate({name:?}): axis must be a non-zero direction, got {axis:?}")
        })?;
        let pivot = given(pivot)
            .map(|pivot| numbers::<3>(pivot, heap, "pivot"))
            .transpose()?;
        let state = state(eval)?;
        record_source(eval, &state, &[&name]);
        with_part(&state, &name, |part| {
            rotate_part(part, &rotation, pivot.unwrap_or(part.at_mm));
            Ok(part_value(part, heap))
        })
    }

    /// Sets a part's frame absolutely: local origin at `origin`, local z along
    /// `z` and local x along `x` (made perpendicular to `z`); y completes a
    /// right-handed frame.
    fn place<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] origin: Value<'v>,
        #[starlark(require = named)] z: Value<'v>,
        #[starlark(require = named)] x: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let origin = numbers::<3>(origin, heap, "origin")?;
        let (z, x) = (numbers::<3>(z, heap, "z")?, numbers::<3>(x, heap, "x")?);
        let rotation = frame::from_axes(x, z).ok_or_else(|| {
            anyhow::anyhow!(
                "place({name:?}): z={z:?} and x={x:?} must be non-zero and not parallel; \
                 give x any direction across the part's length"
            )
        })?;
        let state = state(eval)?;
        record_source(eval, &state, &[&name]);
        with_part(&state, &name, |part| {
            set_frame(part, origin, rotation);
            Ok(part_value(part, heap))
        })
    }

    /// Current name, size, origin `at`, world bounds `min`/`max`, bounds in
    /// its own frame `local_min`/`local_max` and local axes `x`, `y`, `z`
    /// (world directions) of a part.
    fn part_info<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let state = state(eval)?;
        let model = state.model.borrow();
        let part = model
            .part(&name)
            .or_else(|| model.tool(&name))
            .ok_or_else(|| anyhow::anyhow!("unknown part {name:?}"))?;
        Ok(part_value(part, heap))
    }

    /// How far a part reaches along a world `direction` (the largest
    /// `direction · p` over its body); works for rotated parts.
    fn reach<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] direction: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<f64> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let direction = numbers::<3>(direction, heap, "direction")?;
        let direction = frame::normalized(direction).ok_or_else(|| {
            anyhow::anyhow!("reach({name:?}): direction must be non-zero, got {direction:?}")
        })?;
        let state = state(eval)?;
        let model = state.model.borrow();
        Ok(known_part(&model, &name)?.reach(direction))
    }

    /// Face `name` of a part as it is now, in world: struct(part, name, kind,
    /// origin, normal, u, v, min, max, radius, center). kind "planar": points
    /// origin + a*u + b*v for face coordinates (a, b) from min to max, normal
    /// outward. kind "cylindrical": points origin + b*v + radius*(cos a*u +
    /// sin a*(v x u)), a in degrees; normal is the outward normal at a = 0.
    fn face<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let state = state(eval)?;
        let model = state.model.borrow();
        let part = known_part(&model, &part_name)?;
        let face = part
            .face_frame(name)
            .map_err(|error| anyhow::anyhow!("face({part_name:?}, {name:?}): {error}"))?;
        Ok(face_value(part, &face, heap))
    }

    /// Every flat or cylindrical face of a part as it is now, like face().
    fn faces<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let state = state(eval)?;
        let model = state.model.borrow();
        let part = known_part(&model, &part_name)?;
        Ok(heap.alloc(
            part.face_frames()
                .iter()
                .map(|face| face_value(part, face, heap))
                .collect::<Vec<_>>(),
        ))
    }

    /// The face of a part the world `point` lies on, like face(), or None.
    fn face_at<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] point: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let part_name = part_name(part, heap)?;
        let point = numbers::<3>(point, heap, "point")?;
        let state = state(eval)?;
        let model = state.model.borrow();
        let part = known_part(&model, &part_name)?;
        Ok(part
            .face_frame_at(part.to_local(point), TOLERANCE_MM)
            .map_or_else(Value::new_none, |face| face_value(part, &face, heap)))
    }

    /// Moves a part (and what it carries: holes, cuts, tools) by the world
    /// vector `by`.
    fn r#move<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] by: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let by = numbers::<3>(by, heap, "by")?;
        let state = state(eval)?;
        record_source(eval, &state, &[&name]);
        with_part(&state, &name, |part| {
            translate_part(part, by);
            part.refresh_feature_tree();
            Ok(part_value(part, heap))
        })
    }

    /// How two parts sit relative to each other, measured on their boxes in
    /// their own frames (exact for box parts, rotated or not): `distance`
    /// (0 when touching or overlapping), `overlap` (how deep they overlap,
    /// 0 otherwise), `touching`, and `direction`, the unit vector from `a`
    /// towards `b` along which that distance or overlap is measured.
    fn distance<'v>(
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let (a, b) = (part_name(a, heap)?, part_name(b, heap)?);
        let state = state(eval)?;
        let model = state.model.borrow();
        let relation = Relation::between(known_part(&model, &a)?, known_part(&model, &b)?);
        Ok(relation.value(heap))
    }

    /// The part closest to `part` (among `among`, default every other part):
    /// struct(name, distance, overlap, touching, direction) like distance(),
    /// or None when there is no other part.
    fn nearest<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] among: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let among = given(among)
            .map(|among| {
                among
                    .iterate(heap)
                    .map_err(|_| anyhow::anyhow!("among must be a list of parts"))?
                    .map(|other| part_name(other, heap))
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .transpose()?;
        let state = state(eval)?;
        let model = state.model.borrow();
        let this = known_part(&model, &name)?;
        let candidates = match &among {
            Some(names) => names
                .iter()
                .map(|other| known_part(&model, other))
                .collect::<anyhow::Result<Vec<_>>>()?,
            None => model.parts.iter().collect(),
        };
        let best = candidates
            .into_iter()
            .filter(|other| other.name != name)
            .map(|other| (other, Relation::between(this, other)))
            .min_by(|(_, x), (_, y)| (x.distance - x.overlap).total_cmp(&(y.distance - y.overlap)));
        Ok(match best {
            None => Value::new_none(),
            Some((other, relation)) => relation.value_named(heap, &other.name),
        })
    }

    /// Removes the volume of `tool` (a tool body or another part, as it is
    /// now) from `part`. Applied after the part's other features.
    fn subtract<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] tool: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        apply_boolean(part, tool, name, ProgramBooleanKind::Subtract, eval)
    }

    /// Keeps only the volume `part` shares with `tool` (as it is now).
    fn intersect<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] tool: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        apply_boolean(part, tool, name, ProgramBooleanKind::Intersect, eval)
    }

    /// A new part `name` identical to `part` as it is now: same body, frame,
    /// holes and operations. Joints and contacts are not copied.
    fn copy<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let source = part_name(part, heap)?;
        check_part_name(name)?;
        let state = state(eval)?;
        let (copy, tool) = {
            let model = state.model.borrow();
            match (model.part(&source), model.tool(&source)) {
                (Some(part), _) => (part.clone(), false),
                (None, Some(part)) => (part.clone(), true),
                (None, None) => anyhow::bail!("copy(): unknown part {source:?}"),
            }
        };
        let copy = insert_part(
            &state,
            Part {
                name: name.to_owned(),
                ..copy
            },
            tool,
        )?;
        {
            let mut sources = state.part_sources.borrow_mut();
            let lines = sources.get(&source).cloned().unwrap_or_default();
            sources.entry(name.to_owned()).or_default().extend(lines);
        }
        record_source(eval, &state, &[name]);
        Ok(part_value(&copy, heap))
    }

    /// Joins `other` (touching or overlapping `part`) into `part` as one
    /// solid; `other` stops being a separate part.
    fn union<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] other: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let (target, joined) = (part_name(part, heap)?, part_name(other, heap)?);
        {
            let state = state(eval)?;
            let model = state.model.borrow();
            if let Some(other) = model.part(&joined).or_else(|| model.tool(&joined))
                && !(other.holes.is_empty() && other.pockets.is_empty())
            {
                anyhow::bail!(
                    "union({target:?}, {joined:?}): {joined:?} has holes or pockets; drill and mill after the union, on {target:?}"
                );
            }
            if let (Some(a), Some(b)) = (model.part(&target), model.part(&joined)) {
                let apart = Relation::between(a, b).distance;
                if apart > TOLERANCE_MM {
                    anyhow::bail!(
                        "union({target:?}, {joined:?}): the parts are {apart:.3} mm apart; they must touch or overlap to become one solid"
                    );
                }
            }
        }
        let result = apply_boolean(part, other, name, ProgramBooleanKind::Union, eval)?;
        let state = state(eval)?;
        state
            .model
            .borrow_mut()
            .parts
            .retain(|part| part.name != joined);
        let mut sources = state.part_sources.borrow_mut();
        if let Some(lines) = sources.remove(&joined) {
            sources.entry(target).or_default().extend(lines);
        }
        Ok(result)
    }

    /// Drills a hole perpendicular to `face`. Give either face coordinates
    /// `at=(u, v)` or a world point `world=(x, y, z)` on the face.
    fn hole<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] face: &str,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named)] world: Option<Value<'v>>,
        #[starlark(require = named)] diameter: Value<'v>,
        #[starlark(require = named)] depth: Value<'v>,
        #[starlark(require = named)] id: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let face = box_face(face)?;
        let diameter = number(diameter, "diameter")?;
        let depth = number(depth, "depth")?;
        if diameter <= 0.0 || depth <= 0.0 {
            anyhow::bail!("hole in {name:?}: diameter and depth must be positive");
        }
        let at = given(at)
            .map(|at| numbers::<2>(at, heap, "at"))
            .transpose()?;
        let world = given(world)
            .map(|world| numbers::<3>(world, heap, "world"))
            .transpose()?;
        let id = text(id, "id")?;
        let state = state(eval)?;
        record_source(eval, &state, &[&name]);
        with_part(&state, &name, |part| {
            let (u, v) = match (at, world) {
                (Some([u, v]), None) => (u, v),
                (None, Some(world)) => {
                    let local = part.to_local(world);
                    let (u_axis, v_axis) = face.uv_axes();
                    (local[u_axis], local[v_axis])
                }
                _ => anyhow::bail!(
                    "hole in {name:?}: give exactly one of at=(u, v) or world=(x, y, z)"
                ),
            };
            reject_mirrored(part, "hole")?;
            let id = id.unwrap_or_else(|| format!("h{}", part.holes.len() + 1));
            if part.holes.iter().any(|hole| hole.id == id) {
                anyhow::bail!("hole id {id:?} is used twice on part {name:?}");
            }
            part.holes.push(Hole {
                id,
                face,
                u_mm: u,
                v_mm: v,
                diameter_mm: diameter,
                depth_mm: depth,
            });
            part.refresh_feature_tree();
            Ok(NoneType)
        })
    }

    /// Mills a rectangular pocket into `face`. `rect=(u_min, v_min, u_max, v_max)`
    /// in face coordinates; it may extend past the face edges (grooves, rabbets).
    fn pocket<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] face: &str,
        #[starlark(require = named)] rect: Value<'v>,
        #[starlark(require = named)] depth: Value<'v>,
        #[starlark(require = named)] id: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let face = box_face(face)?;
        let [u_min, v_min, u_max, v_max] = numbers::<4>(rect, heap, "rect")?;
        let depth = number(depth, "depth")?;
        let id = text(id, "id")?;
        if u_max - u_min <= TOLERANCE_MM || v_max - v_min <= TOLERANCE_MM || depth <= 0.0 {
            anyhow::bail!(
                "pocket in {name:?}: rect must have positive width and height and depth must be positive"
            );
        }
        let state = state(eval)?;
        record_source(eval, &state, &[&name]);
        with_part(&state, &name, |part| {
            reject_mirrored(part, "pocket")?;
            let id = id.unwrap_or_else(|| format!("p{}", part.pockets.len() + 1));
            if part.pockets.iter().any(|pocket| pocket.id == id) {
                anyhow::bail!("pocket id {id:?} is used twice on part {name:?}");
            }
            part.pockets.push(Pocket {
                id,
                face,
                u_min_mm: u_min,
                v_min_mm: v_min,
                u_max_mm: u_max,
                v_max_mm: v_max,
                depth_mm: depth,
            });
            part.refresh_feature_tree();
            Ok(NoneType)
        })
    }

    /// Where part `a` touches part `b` face to face (rotated parts too): a
    /// struct with `axis` (a's local "x"/"y"/"z"), `face_a`/`face_b` in each
    /// part's own frame, world bounds `min`/`max`, world `normal` out of `a`,
    /// the patch rectangle `origin` + `size=(du, dv)` along world directions
    /// `u`/`v` (face_a's face axes) and its world corner `points`; None if
    /// they do not touch.
    fn contact<'v>(
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let (a, b) = (part_name(a, heap)?, part_name(b, heap)?);
        let state = state(eval)?;
        let model = state.model.borrow();
        let part_a = model
            .part(&a)
            .ok_or_else(|| anyhow::anyhow!("unknown part {a:?}"))?;
        let part_b = model
            .part(&b)
            .ok_or_else(|| anyhow::anyhow!("unknown part {b:?}"))?;
        let point = |value: [f64; 3]| heap.alloc((value[0], value[1], value[2]));
        Ok(match self::contact(part_a, part_b) {
            None => Value::new_none(),
            Some(contact) => heap.alloc(AllocStruct([
                ("axis", heap.alloc(["x", "y", "z"][contact.axis])),
                ("face_a", heap.alloc(contact.face_a.name())),
                ("face_b", heap.alloc(contact.face_b.name())),
                ("min", point(contact.min_mm)),
                ("max", point(contact.max_mm)),
                ("normal", point(contact.normal)),
                ("u", point(contact.u)),
                ("v", point(contact.v)),
                ("origin", point(contact.origin_mm)),
                ("size", heap.alloc((contact.size_mm[0], contact.size_mm[1]))),
                (
                    "points",
                    heap.alloc(
                        contact
                            .points_mm
                            .iter()
                            .map(|p| point(*p))
                            .collect::<Vec<_>>(),
                    ),
                ),
            ])),
        })
    }

    /// Declares a connection between two parts. `fasteners` are world points
    /// (dowel or screw centres); `volume=(min, max)` is where overlap is
    /// expected; `max_gap` allows a clearance between the parts.
    fn joint<'v>(
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        #[starlark(require = named)] kind: &str,
        #[starlark(require = named)] fasteners: Option<Value<'v>>,
        #[starlark(require = named)] fastener: Option<Value<'v>>,
        #[starlark(require = named)] volume: Option<Value<'v>>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        #[starlark(require = named)] max_gap: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let (a, b) = (part_name(a, heap)?, part_name(b, heap)?);
        let max_gap = given(max_gap).map_or(Ok(0.0), |gap| number(gap, "max_gap"))?;
        if max_gap < 0.0 {
            anyhow::bail!("max_gap must not be negative");
        }
        let fasteners = match given(fasteners) {
            None => Vec::new(),
            Some(list) => list
                .iterate(heap)
                .map_err(|_| anyhow::anyhow!("fasteners must be a list of (x, y, z) points"))?
                .enumerate()
                .map(|(index, point)| numbers::<3>(point, heap, &format!("fasteners[{index}]")))
                .collect::<anyhow::Result<Vec<_>>>()?,
        };
        let volume = given(volume)
            .map(|volume| {
                let corners = volume
                    .iterate(heap)
                    .map_err(|_| anyhow::anyhow!("volume must be (min, max)"))?
                    .collect::<Vec<_>>();
                let [min, max] = corners.as_slice() else {
                    anyhow::bail!("volume must be (min, max)");
                };
                Ok((
                    numbers::<3>(*min, heap, "volume min")?,
                    numbers::<3>(*max, heap, "volume max")?,
                ))
            })
            .transpose()?;
        let state = state(eval)?;
        record_source(eval, &state, &[&a, &b]);
        let mut model = state.model.borrow_mut();
        for part in [&a, &b] {
            if model.part(part).is_none() {
                anyhow::bail!("joint refers to unknown part {part:?}");
            }
        }
        let name = text(name, "name")?.unwrap_or_else(|| format!("{kind}:{a}+{b}"));
        model.joints.push(Joint {
            name,
            kind: kind.to_owned(),
            parts: [a, b],
            volume_mm: volume,
            fasteners_mm: fasteners,
            fastener: text(fastener, "fastener")?,
            max_gap_mm: max_gap,
        });
        Ok(NoneType)
    }

    /// States a condition on the final model: `Σ coefficient · measure`
    /// compared by `op` ("==", "<=", ">=", ">") with `value` within
    /// `tolerance`. `terms` is a list of `(coefficient, measure)`; a measure
    /// is `("reach", part, direction)`, `("distance", a, b)` (negative when
    /// they overlap) or `("contact_area", a, b[, face_of_a])`; a direction is
    /// `(x, y, z)` or `(part, "z+")`. Measured after the whole program ran; a
    /// condition that does not hold is an `expectation_failed` error.
    fn expect<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] terms: Value<'v>,
        #[starlark(require = named)] op: Option<&str>,
        #[starlark(require = named)] value: Option<Value<'v>>,
        #[starlark(require = named)] tolerance: Option<Value<'v>>,
        #[starlark(require = named)] unit: Option<&str>,
        #[starlark(require = named)] hint: Option<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let comparison = Comparison::parse(op.unwrap_or("=="))
            .ok_or_else(|| anyhow::anyhow!("expect({name:?}): op must be ==, <=, >= or >"))?;
        let terms = items(terms, heap, "terms")?
            .into_iter()
            .map(|term| {
                let pair = items(term, heap, "each term")?;
                let [coefficient, measure] = pair.as_slice() else {
                    anyhow::bail!("each term must be (coefficient, measure)");
                };
                Ok((
                    number(*coefficient, "coefficient")?,
                    expect_measure(*measure, heap)?,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        if terms.is_empty() {
            anyhow::bail!("expect({name:?}): terms must not be empty");
        }
        let tolerance = given(tolerance).map_or(Ok(0.1), |value| number(value, "tolerance"))?;
        if tolerance < 0.0 {
            anyhow::bail!("expect({name:?}): tolerance must not be negative");
        }
        let mut parts: Vec<String> = Vec::new();
        for part in terms.iter().flat_map(|(_, measure)| measure_parts(measure)) {
            if !parts.iter().any(|known| known == part) {
                parts.push(part.to_owned());
            }
        }
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        for part in &parts {
            known_part(&model, part)?;
        }
        model.expectations.push(Expectation {
            name: name.to_owned(),
            terms,
            comparison,
            value: given(value).map_or(Ok(0.0), |value| number(value, "value"))?,
            tolerance,
            unit: unit.unwrap_or("mm").to_owned(),
            parts,
            hint: hint
                .unwrap_or("Move or resize the parts, or correct the condition.")
                .to_owned(),
        });
        Ok(NoneType)
    }
}

/// `math.*`: the usual functions in radians plus `radians`/`degrees`.
#[allow(non_upper_case_globals)]
#[starlark_module]
fn math_functions(builder: &mut GlobalsBuilder) {
    fn sqrt(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        if x.0 < 0.0 {
            anyhow::bail!("math.sqrt of a negative number {}", x.0);
        }
        Ok(x.0.sqrt())
    }
    fn sin(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        Ok(x.0.sin())
    }
    fn cos(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        Ok(x.0.cos())
    }
    fn tan(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        Ok(x.0.tan())
    }
    fn asin(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        if !(-1.0..=1.0).contains(&x.0) {
            anyhow::bail!("math.asin needs a value in [-1, 1], got {}", x.0);
        }
        Ok(x.0.asin())
    }
    fn acos(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        if !(-1.0..=1.0).contains(&x.0) {
            anyhow::bail!("math.acos needs a value in [-1, 1], got {}", x.0);
        }
        Ok(x.0.acos())
    }
    fn atan(#[starlark(require = pos)] x: UnpackFloat) -> anyhow::Result<f64> {
        Ok(x.0.atan())
    }
    fn atan2(
        #[starlark(require = pos)] y: UnpackFloat,
        #[starlark(require = pos)] x: UnpackFloat,
    ) -> anyhow::Result<f64> {
        Ok(y.0.atan2(x.0))
    }
    fn hypot(
        #[starlark(require = pos)] x: UnpackFloat,
        #[starlark(require = pos)] y: UnpackFloat,
    ) -> anyhow::Result<f64> {
        Ok(x.0.hypot(y.0))
    }
    fn radians(#[starlark(require = pos)] degrees: UnpackFloat) -> anyhow::Result<f64> {
        Ok(degrees.0.to_radians())
    }
    fn degrees(#[starlark(require = pos)] radians: UnpackFloat) -> anyhow::Result<f64> {
        Ok(radians.0.to_degrees())
    }
    const pi: f64 = std::f64::consts::PI;
}

fn dialect() -> Dialect {
    Dialect {
        enable_f_strings: true,
        ..Dialect::Extended
    }
}

fn globals() -> Globals {
    GlobalsBuilder::extended_by(&[
        LibraryExtension::StructType,
        LibraryExtension::Print,
        LibraryExtension::Json,
        LibraryExtension::Map,
        LibraryExtension::Filter,
        LibraryExtension::Partial,
    ])
    .with(builtins)
    .with_namespace("math", math_functions)
    .build()
}

fn evaluation_error(code: &'static str, error: impl std::fmt::Display) -> ProgramError {
    ProgramError {
        code,
        message: error.to_string(),
    }
}

fn prelude(globals: &Globals) -> Result<FrozenModule, ProgramError> {
    let ast = AstModule::parse("prelude.star", PRELUDE.to_owned(), &dialect())
        .map_err(|error| evaluation_error("prelude_invalid", error))?;
    let module = Module::new();
    {
        let mut eval = Evaluator::new(&module);
        eval.eval_module(ast, globals)
            .map_err(|error| evaluation_error("prelude_invalid", error))?;
    }
    module
        .freeze()
        .map_err(|error| evaluation_error("prelude_invalid", format!("{error:?}")))
}

/// Evaluates `source` (a Starlark program) with parameter `overrides`.
///
/// # Errors
/// Returns a [`ProgramError`] with the interpreter's message, which names the
/// file, line and column and quotes the offending source.
pub fn evaluate(
    file_name: &str,
    source: &str,
    overrides: &BTreeMap<String, f64>,
) -> Result<Evaluated, ProgramError> {
    let globals = globals();
    let prelude = prelude(&globals)?;
    let ast = AstModule::parse(file_name, source.to_owned(), &dialect())
        .map_err(|error| {
            // Before a trailing newline the parser reports an unclosed bracket at 1:1;
            // without it the same error points at the program's real end.
            let trimmed = source.trim_end();
            match AstModule::parse(file_name, trimmed.to_owned(), &dialect()) {
                Err(located) if trimmed.len() < source.len() => located,
                _ => error,
            }
        })
        .map_err(|error| evaluation_error("syntax_error", error))?;
    let state = std::rc::Rc::new(State {
        file_name: file_name.to_owned(),
        overrides: overrides.clone(),
        ..State::default()
    });
    STATE.with(|slot| *slot.borrow_mut() = Some(state.clone()));
    let module = Module::new();
    module.import_public_symbols(&prelude);
    let result = {
        let mut eval = Evaluator::new(&module);
        eval.set_print_handler(state.as_ref());
        eval.eval_module(ast, &globals)
            .map(|_| ())
            .map_err(|error| evaluation_error("evaluation_error", error))
    };
    drop(module);
    STATE.with(|slot| *slot.borrow_mut() = None);
    result?;
    let state = std::rc::Rc::try_unwrap(state)
        .map_err(|_| evaluation_error("internal_error", "program state is still shared"))?;
    let model = state.model.into_inner();
    let unused_overrides = overrides
        .keys()
        .filter(|name| !model.params.iter().any(|param| &param.name == *name))
        .cloned()
        .collect();
    let part_sources = state
        .part_sources
        .into_inner()
        .into_iter()
        .map(|(part, lines)| (part, lines.into_iter().collect()))
        .collect();
    Ok(Evaluated {
        model,
        log: state.log.into_inner(),
        unused_overrides,
        part_sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The library's comments are what `KetchupDiscover section=program` hands
    /// an AI (topic by topic), so a builtin missing there is a tool the AI
    /// cannot know about.
    #[test]
    fn every_builtin_is_documented_in_the_library_comments() {
        let header = PRELUDE
            .lines()
            .filter(|line| line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let math_line = header
            .lines()
            .find(|line| line.trim_start_matches(['#', ' ']).starts_with("math."))
            .expect("library header documents math.*");
        let mut missing = Vec::new();
        for name in GlobalsBuilder::new().with(builtins).build().names() {
            let name = name.as_str();
            if !header.contains(&format!("{name}(")) {
                missing.push(name.to_owned());
            }
        }
        for name in GlobalsBuilder::new().with(math_functions).build().names() {
            let name = name.as_str();
            let listed = math_line
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|word| word == name);
            if !listed {
                missing.push(format!("math.{name}"));
            }
        }
        assert!(missing.is_empty(), "undocumented builtins: {missing:?}");
    }
}
