//! The drawing annotations `text`, `dim`, `leader` and `polyline`. They are no
//! parts: no body, no bill of materials, no collision, no mass, no takeoff.
use super::*;
use crate::model::ProgramAnnotation;
use ketchup_model::annotation::{Annotation, AnnotationMark, first_problem};

/// Text height on paper when a call gives none, in millimetres.
pub(crate) const DEFAULT_TEXT_HEIGHT_MM: f64 = 2.5;

/// A point as (x, y) on the floor plane z = 0 or as (x, y, z).
fn point<'v>(value: Value<'v>, heap: &'v Heap, what: &str) -> anyhow::Result<[f64; 3]> {
    let items = iterate(
        value,
        heap,
        format_args!("{what} must be a list or tuple of 2 or 3 numbers"),
    )?
    .collect::<Vec<_>>();
    if !(2..=3).contains(&items.len()) {
        anyhow::bail!("{what} must have 2 or 3 numbers, got {}", items.len());
    }
    let mut out = [0.0; 3];
    for (index, item) in items.into_iter().enumerate() {
        out[index] = number(item, &format!("{what}[{index}]"))?;
    }
    Ok(out)
}

fn height(value: Option<Value>) -> anyhow::Result<f64> {
    given(value).map_or(Ok(DEFAULT_TEXT_HEIGHT_MM), |value| {
        number(value, "height_mm")
    })
}

fn add<'v>(
    eval: &Evaluator<'v, '_, '_>,
    function: &str,
    name: &str,
    tags: Option<Value<'v>>,
    mark: AnnotationMark,
) -> anyhow::Result<NoneType> {
    let tags = given(tags)
        .map(|value| tag_names(value, eval.heap()))
        .transpose()?
        .unwrap_or_default();
    let checked = Annotation {
        name: name.to_owned(),
        tags: BTreeSet::new(),
        mark,
    };
    if let Some((_, problem)) = first_problem(std::slice::from_ref(&checked)) {
        anyhow::bail!("{function}({name:?}): the annotation {problem}");
    }
    let state = state(eval)?;
    let mut model = state.model.borrow_mut();
    if model.annotations.iter().any(|other| other.name == name) {
        anyhow::bail!(
            "{function}({name:?}): another annotation already has this name; give each text, \
             dimension, leader and polyline its own name"
        );
    }
    if model.annotations.len() >= ketchup_model::annotation::MAX_ANNOTATIONS {
        anyhow::bail!(
            "{function}({name:?}): a program declares at most {} annotations",
            ketchup_model::annotation::MAX_ANNOTATIONS
        );
    }
    model.annotations.push(ProgramAnnotation {
        name: checked.name,
        tags,
        mark: checked.mark,
    });
    Ok(NoneType)
}

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Writes `value` with its insertion point at `at`.
    fn text<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] value: &str,
        #[starlark(require = named)] at: Value<'v>,
        #[starlark(require = named)] height_mm: Option<Value<'v>>,
        #[starlark(require = named)] tags: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mark = AnnotationMark::Text {
            at_mm: point(at, eval.heap(), "text at")?,
            text: value.to_owned(),
            height_mm: height(height_mm)?,
        };
        add(eval, "text", name, tags, mark)
    }

    /// Dimensions the distance from `a` to `b`, measured by Kečup.
    fn dim<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        #[starlark(require = named)] offset_mm: Option<Value<'v>>,
        #[starlark(require = named)] height_mm: Option<Value<'v>>,
        #[starlark(require = named)] tags: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let mark = AnnotationMark::Dimension {
            from_mm: point(a, heap, "dim a")?,
            to_mm: point(b, heap, "dim b")?,
            offset_mm: given(offset_mm).map_or(Ok(0.0), |value| number(value, "offset_mm"))?,
            height_mm: height(height_mm)?,
        };
        add(eval, "dim", name, tags, mark)
    }

    /// Points an arrow at `point` with a line to `value`, written at `at`.
    fn leader<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] value: &str,
        #[starlark(require = named)] point: Value<'v>,
        #[starlark(require = named)] at: Value<'v>,
        #[starlark(require = named)] height_mm: Option<Value<'v>>,
        #[starlark(require = named)] tags: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let mark = AnnotationMark::Leader {
            point_mm: self::point(point, heap, "leader point")?,
            at_mm: self::point(at, heap, "leader at")?,
            text: value.to_owned(),
            height_mm: height(height_mm)?,
        };
        add(eval, "leader", name, tags, mark)
    }

    /// Draws straight lines through `points`, back to the first when `closed`.
    fn polyline<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] points: Value<'v>,
        #[starlark(require = named, default = false)] closed: bool,
        #[starlark(require = named)] tags: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let points_mm = iterate(
            points,
            heap,
            format_args!("polyline points must be a list of points"),
        )?
        .enumerate()
        .map(|(index, value)| point(value, heap, &format!("polyline points[{index}]")))
        .collect::<anyhow::Result<Vec<_>>>()?;
        add(
            eval,
            "polyline",
            name,
            tags,
            AnnotationMark::Line { points_mm, closed },
        )
    }
}
