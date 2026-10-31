//! `drawing action=view` and `action=solid_from_views`: drawings set up in space as a plan
//! and views on vertical planes, and the solid they show together. Each view's area is
//! written into the window's program as `program action=from_underlay` writes it, with
//! the frame its view stands in, and pulled along that view's line of sight through the
//! box of all of them; the pulls are intersected. The sections follow the drawings: an
//! edit, a new placement or a new view of one rewrites its data in the same Undo step.

use super::program_access::invalid;
use super::underlay_program::{BEGIN, END, is_identifier, with_data};
use super::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::view::{UnderlayView, ViewPlane, plan_underlay_view};

/// The most views one solid is made of.
const MAX_VIEWS: usize = 8;

/// The area of one drawing a view shows: around a point, holding a text, or one closed
/// object; in that drawing's document millimetres, as `inspect action=underlay_regions`
/// answers.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewArea {
    pub underlay_id: u64,
    #[serde(default)]
    pub at_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub object_id: Option<u32>,
    #[serde(default)]
    pub layers: Option<Vec<String>>,
    #[serde(default)]
    pub closing_segments_mm: Vec<[[f64; 2]; 2]>,
    /// With `at_mm`, the whole figure drawn around the point, its outer edge, the edges
    /// seen within the view ignored; `false`, the innermost region around it.
    #[serde(default = "seen_whole")]
    pub whole: bool,
}

const fn seen_whole() -> bool {
    true
}

impl LiveBridge {
    /// `drawing_view`: sets the drawing up on a plane, one Undo step.
    pub(super) fn drawing_view(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingView {
            expected,
            underlay_id,
            plane,
            plane_at_mm,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let plane = ViewPlane::from_name(plane).ok_or_else(|| {
            invalid(
                "plane",
                format!("{plane:?} is no view plane."),
                "Use top, front, back, left or right.",
            )
        })?;
        let snapshot = app.document.current();
        let id = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?.id;
        let view = UnderlayView {
            plane,
            at_mm: *plane_at_mm,
        };
        let batch = plan_underlay_view(&snapshot, id, view)
            .map_err(|error| failed_because("invalid_params", error))?;
        Self::require_request_authority(cancelled)?;
        app.apply_batch_with_work_recovery(&batch)
            .map_err(|error| failed_because("commit_rejected", error))?;
        self.query.invalidate();
        let snapshot = app.document.current();
        let underlay = snapshot.underlay(id).ok_or("entity_not_found")?;
        let (x, y, _) = plane.axes();
        Ok(json!({
            "underlay_id": id.0,
            "plane": plane.name(),
            "plane_at_mm": underlay.view.at_mm,
            // Where a document point [u, v] of the drawing lies:
            // origin + u * drawing_x_world + v * drawing_y_world.
            "origin_world_mm": underlay.view.transform().transform_point([0.0; 3]),
            "drawing_x_world": x,
            "drawing_y_world": y,
            "undo_steps": app.undo_step_count(),
            "dirty": app.is_dirty(),
        }))
    }

    /// `drawing_solid_from_views`: the views' areas pulled through each other and
    /// intersected, written into the program, one Undo step.
    pub(super) fn drawing_solid_from_views(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingSolidFromViews {
            expected,
            name,
            views,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let name = match name {
            Some(name) if is_identifier(name) => name.clone(),
            Some(name) => {
                return Err(invalid(
                    "name",
                    format!("{name:?} is not a program name."),
                    "Use letters, digits and _, starting with a letter, e.g. BRACKET.",
                ));
            }
            None => super::underlay_program::free_name(app, "SOLID"),
        };
        let snapshot = app.document.current();
        let planes = views
            .iter()
            .map(|view| {
                snapshot
                    .underlay(UnderlayId(view.underlay_id))
                    .map(|underlay| underlay.view.plane)
                    .ok_or("entity_not_found")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let axes = planes
            .iter()
            .map(|plane| plane.axes().2)
            .collect::<BTreeSet<_>>();
        if views.len() > MAX_VIEWS || axes.len() < 2 {
            return Err(invalid(
                "views",
                format!(
                    "{} views on {} different lines of sight; a solid needs 2 to {MAX_VIEWS} views seen along at least two different axes.",
                    views.len(),
                    axes.len()
                ),
                "Set the drawings up first with drawing action=view (a plan stays top; plane=front, right, … for the others), then name the area of each.",
            ));
        }
        if app.document.current_rule_program().is_none() && snapshot.definitions().next().is_some()
        {
            return Err(invalid(
                "views",
                "No program owns this document, so the solid could not follow the drawings.",
                "Start from a document a program owns, or an empty one, or pull a single area with action=extrude.",
            ));
        }
        let names = (1..=views.len())
            .map(|i| format!("{name}_V{i}"))
            .collect::<Vec<_>>();
        let current = app.document.current_rule_program();
        let mut source = current.map_or_else(String::new, |program| program.source.clone());
        source = with_data(&source, &name, &intersection(&name, &names, &planes));
        for (view, data) in views.iter().zip(&names).rev() {
            let request = Request::ProgramFromUnderlay {
                expected: app.live_bridge_stamp(),
                name: data.clone(),
                underlay_id: Some(view.underlay_id),
                point_mm: view.at_mm,
                label: view.label.clone(),
                layers: view.layers.clone(),
                tolerance_mm: None,
                closing_segments_mm: view.closing_segments_mm.clone(),
                within_mm: None,
                object_id: view.object_id,
                block: None,
                thickness_mm: None,
            };
            let underlay =
                super::underlay_regions::one_underlay(&snapshot, Some(view.underlay_id))?;
            let mut binding =
                super::underlay_binding::Binding::new(app, &snapshot, underlay, request)?;
            binding.frame = true;
            binding.whole = view.whole;
            source = with_data(&source, data, &binding.written(app, &snapshot)?);
        }
        Self::require_request_authority(cancelled)?;
        let file_name = current.map_or_else(
            || "drawing.star".to_owned(),
            |program| program.file_name.clone(),
        );
        let program = ketchup_model::document::RuleProgramSource {
            variants: current
                .map(|program| program.variants.clone())
                .unwrap_or_default(),
            overrides: current
                .map(|program| program.overrides.clone())
                .unwrap_or_default(),
            file_name,
            source,
        };
        app.apply_program_source_with(program, false, Vec::new())
            .map_err(super::program_failure)?;
        self.query.invalidate();
        Ok(json!({
            "name": name,
            "views": names.iter().zip(&planes).map(|(data, plane)| json!({"data": data, "plane": plane.name()})).collect::<Vec<_>>(),
            "bound_to_drawing": true,
            "undo_steps": app.undo_step_count(),
            "dirty": app.is_dirty(),
        }))
    }
}

/// The section that makes the part `name`: each view's outline (`NAME_Vi`, its holes and
/// frame) pulled along the axis it is seen along through the box of every view, then
/// intersected, the first view the part and the rest its tools. Views all seen along one
/// axis, as when one is laid back into the plan, fail: the box they span has no depth.
fn intersection(name: &str, views: &[String], planes: &[ViewPlane]) -> String {
    let seen = planes
        .iter()
        .map(|plane| plane.name())
        .collect::<Vec<_>>()
        .join(", ");
    let listed = views
        .iter()
        .map(|view| format!("({view}, {view}_HOLES, {view}_FRAME)"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        "{BEGIN} {name}: the solid the views {} ({seen}) show, each pulled through the others and intersected. Follows the views.\n\
def {name}_through(name, outline, holes, frame, tool):
    o, x, y = frame
    z = vec_cross(x, y)
    a = vec_dot({name}_LOW, z)
    b = vec_dot({name}_HIGH, z)
    start = min(a, b) - vec_dot(o, z)
    part = extrude(name, profile = outline, distance = max(a, b) - min(a, b), tool = tool)
    for i, hole in enumerate(holes):
        cut(part, profile = hole, through = True, name = name + \"/hole%d\" % (i + 1))
    place(part, origin = (o[0] + start * z[0], o[1] + start * z[1], o[2] + start * z[2]), z = z, x = x)
    return part

def {name}_seen(views):
    z = [vec_cross(f[1], f[2]) for v, h, f in views]
    if max([max([abs(c) for c in vec_cross(z[0], w)]) for w in z]) < {rounding:e}:
        fail(\"{name}: its views are all seen along one axis, so they show no thickness; stand one of them on another plane\")
    return z

{name}_VIEWS = [{listed}]
{name}_SEEN = {name}_seen({name}_VIEWS)
{name}_WORLD = [[f[0][k] + p[0] * f[1][k] + p[1] * f[2][k] for k in range(3)] for v, h, f in {name}_VIEWS for p in [q[1] if type(q[0]) == \"string\" else q for q in v]]
{name}_LOW = [min([q[k] for q in {name}_WORLD]) - 1 for k in range(3)]
{name}_HIGH = [max([q[k] for q in {name}_WORLD]) + 1 for k in range(3)]
{name}_PART = {name}_through({}, {views0}, {views0}_HOLES, {views0}_FRAME, False)
",
        views.join(", "),
        quoted(name),
        views0 = views[0],
        rounding = ketchup_model::tolerance::ROUNDING,
    );
    for (index, view) in views.iter().enumerate().skip(1) {
        let tool = quoted(&format!("{name}/view{}", index + 1));
        text.push_str(&format!(
            "intersect({name}_PART, {name}_through({tool}, {view}, {view}_HOLES, {view}_FRAME, True), name = {tool})\n"
        ));
    }
    text.push_str(&format!("{END} {name}\n"));
    text
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}
