//! The machining builtins `hole` and `pocket`, which drill and mill a part's
//! faces, and the thread a hole is tapped with.
use super::*;
use ketchup_geometry::tapping::{TappedHole, TappedHoleMismatch};
use ketchup_model::document::HoleThread;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Drills a hole into `face` (a face name or a face() value) along its
    /// inward normal. Give either face coordinates `at=(u, v)` (on a curved
    /// face `at=(angle, v)`, see face()) or a world point `world=(x, y, z)`
    /// on the face. `thread=("M5", 10)` taps an ISO metric coarse thread that
    /// deep; the diameter is then the tap drill D − P and may be omitted.
    /// `modeled_thread=True` models that thread as helical geometry, `False`
    /// keeps it cosmetic, whatever the document's thread mode.
    fn hole<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] face: Value<'v>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named)] world: Option<Value<'v>>,
        #[starlark(require = named)] diameter: Option<Value<'v>>,
        #[starlark(require = named)] depth: Value<'v>,
        #[starlark(require = named, default = false)] through: bool,
        #[starlark(require = named)] thread: Option<Value<'v>>,
        #[starlark(require = named)] modeled_thread: Option<bool>,
        #[starlark(require = named)] id: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        if modeled_thread.is_some() && given(thread).is_none() {
            anyhow::bail!("hole in {name:?}: modeled_thread= needs thread=");
        }
        let face = face_name(face, heap, Some(&name))?;
        let depth = number(depth, "depth")?;
        let diameter = given(diameter)
            .map(|diameter| number(diameter, "diameter"))
            .transpose()?;
        let (diameter, thread) = match given(thread) {
            Some(thread) => {
                let (diameter, thread) = tapped(thread, diameter, depth, through, heap)
                    .map_err(|error| anyhow::anyhow!("hole in {name:?}: {error}"))?;
                (
                    diameter,
                    Some(HoleThread {
                        modeled: modeled_thread,
                        ..thread
                    }),
                )
            }
            None => (
                diameter.ok_or_else(|| {
                    anyhow::anyhow!(
                        "hole in {name:?}: give diameter=, or thread= for a tapped hole"
                    )
                })?,
                None,
            ),
        };
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
            let frame = part
                .machining_frame(&face)
                .map_err(|error| anyhow::anyhow!("hole in {name:?}: {error}"))?;
            let at = match (at, world) {
                (Some(at), None) => at,
                (None, Some(world)) => frame.coordinates(part.to_local(world)),
                _ => {
                    anyhow::bail!(
                        "hole in {name:?}: give exactly one of at=(u, v) or world=(x, y, z)"
                    )
                }
            };
            let id = id.unwrap_or_else(|| format!("h{}", part.holes().count() + 1));
            add_operation(
                part,
                ProgramOperation::Hole(Hole {
                    id: id.clone(),
                    face,
                    at_mm: at,
                    entry_mm: frame.point(at),
                    inward: frame.normal_at(at).map(|value| -value),
                    diameter_mm: diameter,
                    depth_mm: depth,
                    through,
                    thread,
                }),
            )?;
            Ok(heap.alloc(AllocStruct([
                ("part", heap.alloc(name.as_str())),
                ("id", heap.alloc(id)),
            ])))
        })
    }

    /// Mills a rectangular pocket into the flat `face` (a name or a face()
    /// value). `rect=(u_min, v_min, u_max, v_max)` in face coordinates; it may
    /// extend past the face edges (grooves, rabbets).
    fn pocket<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = pos)] face: Value<'v>,
        #[starlark(require = named)] rect: Value<'v>,
        #[starlark(require = named)] depth: Value<'v>,
        #[starlark(require = named)] id: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        let face = face_name(face, heap, Some(&name))?;
        let rect_mm = numbers::<4>(rect, heap, "rect")?;
        let [u_min, v_min, u_max, v_max] = rect_mm;
        let depth = number(depth, "depth")?;
        let id = text(id, "id")?;
        if u_max - u_min <= CONTACT_FIT_MM || v_max - v_min <= CONTACT_FIT_MM || depth <= 0.0 {
            anyhow::bail!(
                "pocket in {name:?}: rect must have positive width and height and depth must be positive"
            );
        }
        let state = state(eval)?;
        record_source(eval, &state, &[&name]);
        with_part(&state, &name, |part| {
            let frame = part
                .machining_frame(&face)
                .map_err(|error| anyhow::anyhow!("pocket in {name:?}: {error}"))?;
            if frame.kind != FaceKind::Planar {
                anyhow::bail!(
                    "pocket in {name:?}: face {face:?} is curved; a pocket is milled into a flat face"
                );
            }
            let id = id.unwrap_or_else(|| format!("p{}", part.pockets().count() + 1));
            add_operation(
                part,
                ProgramOperation::Pocket(Pocket {
                    id,
                    face,
                    rect_mm,
                    corner_mm: frame.point([u_min, v_min]),
                    u: frame.u,
                    v: frame.v,
                    inward: frame.normal.map(|value| -value),
                    depth_mm: depth,
                }),
            )?;
            Ok(NoneType)
        })
    }
}

/// `thread=(size, depth)` of a hole `depth_mm` deep: the library thread and
/// the hole's diameter, its tap drill when `diameter_mm` is not given. The
/// same rule as the `drill_holes` operation: the size is in
/// `METRIC_COARSE_PITCH`, the hole is its tap drill (`METRIC_TAP_DRILL`) within
/// half a drill step, and the thread is at most as deep as a `through` hole,
/// or as a blind one less its runout (`METRIC_BLIND_HOLE_RUNOUT`).
fn tapped<'v>(
    thread: Value<'v>,
    diameter_mm: Option<f64>,
    depth_mm: f64,
    through: bool,
    heap: &'v Heap,
) -> anyhow::Result<(f64, HoleThread)> {
    let shape = "thread must be (size, depth), e.g. (\"M5\", 10)";
    let [size, thread_depth] = items(thread, heap, "thread")
        .ok()
        .and_then(|items| <[Value; 2]>::try_from(items).ok())
        .ok_or_else(|| anyhow::anyhow!("{shape}"))?;
    let size = size
        .unpack_str()
        .ok_or_else(|| anyhow::anyhow!("{shape}"))?
        .to_owned();
    let thread_depth = number(thread_depth, "thread depth")?;
    if thread_depth <= 0.0 {
        anyhow::bail!("the thread depth must be positive");
    }
    let library = library_metric_threads().map_err(|error| anyhow::anyhow!("{error}"))?;
    let Some(metric) = library.get(&size) else {
        let sizes: Vec<&str> = library.keys().map(String::as_str).collect();
        anyhow::bail!(
            "the fastener library has no thread {size:?}; its threads are {}",
            sizes.join(", ")
        );
    };
    let diameter_mm = diameter_mm.unwrap_or(metric.tap_drill_mm);
    // The rule every export checks the hole by again, at the default model
    // tolerance: a hole made here is one the machine files take.
    let hole = TappedHole {
        size: &size,
        pitch_mm: metric.pitch_mm,
        thread_depth_mm: thread_depth,
        hole_diameter_mm: diameter_mm,
        hole_depth_mm: depth_mm,
        through,
    };
    match hole.check(ketchup_tolerance::DEFAULT_LINEAR_TOLERANCE_MM) {
        Ok(()) => {}
        Err(TappedHoleMismatch::Drill { .. }) => anyhow::bail!(
            "an {size} thread (pitch {}) is tapped into its {} mm tap drill hole; this hole is \
             {diameter_mm} mm (omit diameter to drill the tap drill)",
            metric.pitch_mm,
            metric.tap_drill_mm
        ),
        Err(TappedHoleMismatch::TooDeep { .. }) if through => anyhow::bail!(
            "the thread is {thread_depth} mm deep, deeper than the {depth_mm} mm hole"
        ),
        Err(TappedHoleMismatch::TooDeep { .. }) => anyhow::bail!(
            "a {thread_depth} mm deep {size} thread needs a blind hole at least {} mm deep: the \
             tap needs {} mm below the thread (DIN 76-1 runout e1, METRIC_BLIND_HOLE_RUNOUT); \
             this hole is {depth_mm} mm",
            metric.blind_hole_depth_mm(thread_depth),
            metric.blind_hole_runout_mm
        ),
        Err(mismatch) => anyhow::bail!(
            "the fastener library's {size} thread is not the one machine files are checked by: \
             {mismatch}"
        ),
    }
    Ok((
        diameter_mm,
        HoleThread {
            size,
            pitch_mm: metric.pitch_mm,
            depth_mm: thread_depth,
            modeled: None,
        },
    ))
}
