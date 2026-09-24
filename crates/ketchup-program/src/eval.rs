//! Evaluates a Starlark rule program into a [`ProgramModel`].
//!
//! Rust exposes only generic geometry builtins (`param`, `box`, `hole`,
//! `pocket`, `contact`, `joint`). Everything with a domain name (boards,
//! dowels, grooves, ...) lives in `library/prelude.star`.

// Starlark builtins mirror their keyword arguments, so some take many
// parameters; the macro-generated wrappers inherit that.
#![allow(clippy::too_many_arguments)]

use crate::model::{Face, Hole, Joint, Param, Part, Pocket, ProgramModel};
use serde::Serialize;
use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::float::UnpackFloat;
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;
use starlark::values::{Heap, UnpackValue, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Contact and fit tolerance in millimetres.
pub const TOLERANCE_MM: f64 = 0.01;
/// Upper bound on generated parts, so a runaway loop fails fast.
pub const MAX_PARTS: usize = 20_000;
const MAX_ABS_MM: f64 = 1_000_000.0;
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
    overrides: BTreeMap<String, f64>,
    model: RefCell<ProgramModel>,
    log: RefCell<Vec<String>>,
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
    if !number.is_finite() || number.abs() > MAX_ABS_MM {
        anyhow::bail!("{what} must be a finite number within ±{MAX_ABS_MM} mm, got {number}");
    }
    Ok(number)
}

fn numbers<'v, const N: usize>(
    value: Value<'v>,
    heap: Heap<'v>,
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

fn part_name<'v>(value: Value<'v>, heap: Heap<'v>) -> anyhow::Result<String> {
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

fn face(value: &str) -> anyhow::Result<Face> {
    Face::parse(value)
        .ok_or_else(|| anyhow::anyhow!("face must be one of x-, x+, y-, y+, z-, z+; got {value:?}"))
}

fn part_value<'v>(part: &Part, heap: Heap<'v>) -> Value<'v> {
    heap.alloc(AllocStruct([
        ("name", heap.alloc(part.name.as_str())),
        (
            "size",
            heap.alloc((part.size_mm[0], part.size_mm[1], part.size_mm[2])),
        ),
        (
            "at",
            heap.alloc((part.at_mm[0], part.at_mm[1], part.at_mm[2])),
        ),
        (
            "max",
            heap.alloc((part.max_mm()[0], part.max_mm()[1], part.max_mm()[2])),
        ),
    ]))
}

fn with_part<R>(
    state: &State,
    name: &str,
    f: impl FnOnce(&mut Part) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    let mut model = state.model.borrow_mut();
    let part = model
        .parts
        .iter_mut()
        .find(|part| part.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown part {name:?}; create it with box() first"))?;
    f(part)
}

/// Contact between two axis-aligned parts: a shared face patch of positive area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub axis: usize,
    /// Face of the first part that touches the second.
    pub face_a: Face,
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
}

/// Finds the face patch where `a` touches `b`, if any.
#[must_use]
pub fn contact(a: &Part, b: &Part) -> Option<Contact> {
    let (a_min, a_max, b_min, b_max) = (a.at_mm, a.max_mm(), b.at_mm, b.max_mm());
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
                return Some(Contact {
                    axis,
                    face_a: Face::from_axis(axis, max_side),
                    min_mm: min,
                    max_mm: max,
                });
            }
        }
    }
    None
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

    /// An axis-aligned cuboid part. `size` is (x, y, z) and `at` its minimum corner.
    fn r#box<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] size: Value<'v>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named)] material: Option<Value<'v>>,
        #[starlark(require = named)] grain: Option<Value<'v>>,
        #[starlark(require = named)] color: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            anyhow::bail!("part name must be 1-128 printable bytes, got {name:?}");
        }
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
        let part = Part {
            name: name.to_owned(),
            size_mm: size,
            at_mm: at,
            material,
            grain_axis,
            color,
            holes: Vec::new(),
            pockets: Vec::new(),
        };
        {
            let mut model = state.model.borrow_mut();
            if model.parts.iter().any(|existing| existing.name == name) {
                anyhow::bail!(
                    "part {name:?} already exists; part names are identities and must be unique"
                );
            }
            if model.parts.len() >= MAX_PARTS {
                anyhow::bail!("more than {MAX_PARTS} parts; check the program for a runaway loop");
            }
            model.parts.push(part.clone());
        }
        Ok(part_value(&part, heap))
    }

    /// Current name, size, at and max of a part.
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
            .ok_or_else(|| anyhow::anyhow!("unknown part {name:?}"))?;
        Ok(part_value(part, heap))
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
        let face = self::face(face)?;
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
        let face = self::face(face)?;
        let [u_min, v_min, u_max, v_max] = numbers::<4>(rect, heap, "rect")?;
        let depth = number(depth, "depth")?;
        let id = text(id, "id")?;
        if u_max - u_min <= TOLERANCE_MM || v_max - v_min <= TOLERANCE_MM || depth <= 0.0 {
            anyhow::bail!(
                "pocket in {name:?}: rect must have positive width and height and depth must be positive"
            );
        }
        let state = state(eval)?;
        with_part(&state, &name, |part| {
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
            Ok(NoneType)
        })
    }

    /// Where part `a` touches part `b`: a struct with `axis` ("x"/"y"/"z"),
    /// `face_a`, `face_b`, and the world rectangle `min`/`max`; None if they
    /// do not touch.
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
        Ok(match self::contact(part_a, part_b) {
            None => Value::new_none(),
            Some(contact) => heap.alloc(AllocStruct([
                ("axis", heap.alloc(["x", "y", "z"][contact.axis])),
                ("face_a", heap.alloc(contact.face_a.name())),
                ("face_b", heap.alloc(contact.face_a.opposite().name())),
                (
                    "min",
                    heap.alloc((contact.min_mm[0], contact.min_mm[1], contact.min_mm[2])),
                ),
                (
                    "max",
                    heap.alloc((contact.max_mm[0], contact.max_mm[1], contact.max_mm[2])),
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
    Module::with_temp_heap(|module| {
        {
            let mut eval = Evaluator::new(&module);
            eval.eval_module(ast, globals)
                .map_err(|error| evaluation_error("prelude_invalid", error))?;
        }
        module
            .freeze()
            .map_err(|error| evaluation_error("prelude_invalid", format!("{error:?}")))
    })
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
        .map_err(|error| evaluation_error("syntax_error", error))?;
    let state = std::rc::Rc::new(State {
        overrides: overrides.clone(),
        ..State::default()
    });
    STATE.with(|slot| *slot.borrow_mut() = Some(state.clone()));
    let result = Module::with_temp_heap(|module| {
        module.import_public_symbols(&prelude);
        let mut eval = Evaluator::new(&module);
        eval.set_print_handler(state.as_ref());
        eval.eval_module(ast, &globals)
            .map_err(|error| evaluation_error("evaluation_error", error))?;
        Ok::<(), ProgramError>(())
    });
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
    Ok(Evaluated {
        model,
        log: state.log.into_inner(),
        unused_overrides,
    })
}
