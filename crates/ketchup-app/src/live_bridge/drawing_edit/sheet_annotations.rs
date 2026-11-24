//! Sheet geometry discovery and typed annotation placement for MCP drawing actions.

use super::super::*;
use ketchup_model::drawing::DrawingSheetId;
use ketchup_model::drawing::annotation::DrawingAnnotation;

pub(super) fn record(cause: ketchup_rejection::Rejection) -> &'static str {
    let code = ketchup_application::rejections::host_rejection(cause.code())
        .map_or("invalid_params", |entry| entry.code);
    failed_because(code, cause)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SheetRequest {
    #[serde(default)]
    expected: Option<Stamp>,
    sheet_id: u64,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_size")]
    limit: usize,
}

const fn page_size() -> usize {
    100
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotateRequest {
    #[serde(default)]
    expected: Option<Stamp>,
    sheet_id: u64,
    #[serde(default)]
    annotations: Vec<DrawingAnnotation>,
    #[serde(default)]
    hole_callouts: bool,
}

impl LiveBridge {
    pub(in crate::live_bridge) fn drawing_sheet(
        app: &KetchupApp,
        request: &SheetRequest,
        ui_busy: bool,
    ) -> Result<Value, ketchup_rejection::Rejection> {
        Self::guard(app, &request.expected).map_err(ketchup_application::rejections::rejected)?;
        Self::available(app, ui_busy).map_err(ketchup_application::rejections::rejected)?;
        if !(1..=200).contains(&request.limit) {
            return Err(ketchup_application::rejections::rejected("invalid_params")
                .target("limit")
                .reason("Use a drawing geometry page limit from 1 to 200."));
        }
        let drawing = app.drawing_sheet_geometry(DrawingSheetId(request.sheet_id))?;
        let view = drawing
            .views
            .iter()
            .find(|view| {
                request
                    .view
                    .as_ref()
                    .is_none_or(|name| view.kind.stable_name() == *name)
            })
            .ok_or_else(|| {
                ketchup_application::rejections::rejected("invalid_params")
                    .target("view")
                    .reason("Choose a view of this sheet from the views returned by drawing action=sheet.")
            })?;
        let line_count = view.visible_lines.len() + view.hidden_lines.len();
        let total = line_count + view.circles.len();
        let end = request.offset.saturating_add(request.limit).min(total);
        let lines: Vec<_> = view.visible_lines.iter().map(|line| (line, false))
            .chain(view.hidden_lines.iter().map(|line| (line, true)))
            .enumerate().skip(request.offset).take(request.limit)
            .map(|(_, (line, hidden))| json!({"id": line.stable_line_id, "start_mm": line.start_mm,
                "end_mm": line.end_mm, "hidden": hidden, "foreshortened": line.foreshortened, "clipped": line.clipped})).collect();
        let circles: Vec<_> = view.circles.iter().enumerate()
            .filter(|(index, _)| (request.offset..end).contains(&(line_count + index)))
            .map(|(_, circle)| json!({"id": circle.stable_circle_id, "center_mm": circle.center_mm,
                "radius_mm": circle.radius_mm, "start_rad": circle.start_rad, "span_rad": circle.span_rad})).collect();
        Ok(
            json!({"sheet_id": request.sheet_id, "view": view.kind.stable_name(),
            "views": drawing.views.iter().map(|view| view.kind.stable_name()).collect::<Vec<_>>(),
            "bounds_mm": view.bounds_mm, "lines": lines, "circles": circles,
            "offset": request.offset, "total": total, "next_offset": (end < total).then_some(end),
            "annotation_counts": {"angular": drawing.layout.angular_dimensions.len(),
                "circular": drawing.layout.circular_dimensions.len(), "datum": drawing.layout.datum_symbols.len(),
                "feature_control_frame": drawing.layout.feature_control_frames.len()}}),
        )
    }

    pub(in crate::live_bridge) fn drawing_annotate(
        &mut self,
        app: &mut KetchupApp,
        request: &AnnotateRequest,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, ketchup_rejection::Rejection> {
        Self::guard(app, &request.expected).map_err(ketchup_application::rejections::rejected)?;
        Self::available(app, ui_busy).map_err(ketchup_application::rejections::rejected)?;
        Self::require_request_authority(cancelled)
            .map_err(ketchup_application::rejections::rejected)?;
        let removed_sources = app.annotate_drawing_sheet(
            DrawingSheetId(request.sheet_id),
            &request.annotations,
            request.hole_callouts,
        )?;
        self.query.invalidate();
        Ok(
            json!({"sheet_id": request.sheet_id, "added": request.annotations.len(), "removed_callout_sources": removed_sources, "dirty": app.is_dirty()}),
        )
    }
}
