#![forbid(unsafe_code)]

use crate::document::Snapshot;
use crate::drawing::{
    DRAWING_SHEET_LAYOUT_SCHEMA_V2, DrawingAngularDimensionLayout, DrawingBomBalloonLayout,
    DrawingCircularDimensionLayout, DrawingDatumSymbolLayout, DrawingFeatureControlFrameLayout,
    DrawingLinearDimensionLayout, DrawingSheetLayout, DrawingViewPlacement,
    ORTHOGRAPHIC_LINEWORK_SCHEMA_V2, OrthographicDrawing, OrthographicView, drawing_layout_digest,
    drawing_result_digest,
};
use std::fmt;

pub const DRAWING_EXPORT_SCHEMA_V1: &str = "ketchup.drawing-export.v1";
const MAX_EXPORT_LINES: usize = 100_000;
const MAX_EXPORT_TEXTS: usize = 1_024;
const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
const COORDINATE_EPSILON_MM: f64 = 1.0e-7;
const POINTS_PER_MM: f64 = 72.0 / 25.4;
const PDF_UNICODE_GLYPHS: &[(char, u8, &str)] = &[
    ('Á', 128, "Aacute"),
    ('Ä', 129, "Adieresis"),
    ('Č', 130, "Ccaron"),
    ('Ď', 131, "Dcaron"),
    ('É', 132, "Eacute"),
    ('Í', 133, "Iacute"),
    ('Ĺ', 134, "Lacute"),
    ('Ľ', 135, "Lcaron"),
    ('Ň', 136, "Ncaron"),
    ('Ó', 137, "Oacute"),
    ('Ô', 138, "Ocircumflex"),
    ('Ŕ', 139, "Racute"),
    ('Š', 140, "Scaron"),
    ('Ť', 141, "Tcaron"),
    ('Ú', 142, "Uacute"),
    ('Ý', 143, "Yacute"),
    ('Ž', 144, "Zcaron"),
    ('á', 145, "aacute"),
    ('ä', 146, "adieresis"),
    ('č', 147, "ccaron"),
    ('ď', 148, "dcaron"),
    ('é', 149, "eacute"),
    ('í', 150, "iacute"),
    ('ĺ', 151, "lacute"),
    ('ľ', 152, "lcaron"),
    ('ň', 153, "ncaron"),
    ('ó', 154, "oacute"),
    ('ô', 155, "ocircumflex"),
    ('ŕ', 156, "racute"),
    ('š', 157, "scaron"),
    ('ť', 158, "tcaron"),
    ('ú', 159, "uacute"),
    ('ý', 160, "yacute"),
    ('ž', 161, "zcaron"),
    ('±', 162, "plusminus"),
    ('⌀', 163, "Oslash"),
    ('°', 164, "degree"),
    ('×', 165, "multiply"),
    ('µ', 166, "mu"),
    ('≤', 167, "lessequal"),
    ('≥', 168, "greaterequal"),
    ('≠', 169, "notequal"),
    ('–', 170, "endash"),
    ('—', 171, "emdash"),
    ('„', 172, "quotedblbase"),
    ('“', 173, "quotedblleft"),
    ('”', 174, "quotedblright"),
    ('’', 175, "quoteright"),
    ('\u{a0}', 176, "space"),
    ('€', 177, "Euro"),
    ('Ø', 178, "Oslash"),
    ('ø', 179, "oslash"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingExportBundle {
    svg: Vec<u8>,
    dxf: Vec<u8>,
    pdf: Vec<u8>,
}

impl DrawingExportBundle {
    #[must_use]
    pub fn svg(&self) -> &[u8] {
        &self.svg
    }

    #[must_use]
    pub fn dxf(&self) -> &[u8] {
        &self.dxf
    }

    #[must_use]
    pub fn pdf(&self) -> &[u8] {
        &self.pdf
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingExportError {
    StaleDrawing,
    InvalidDrawing,
    UnsupportedPdfText,
    ResourceLimit,
}

impl fmt::Display for DrawingExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StaleDrawing => "drawing export source is stale",
            Self::InvalidDrawing => "drawing export contract is inconsistent",
            Self::UnsupportedPdfText => "drawing contains text unsupported by the PDF font",
            Self::ResourceLimit => "drawing export exceeded its resource limit",
        })
    }
}

impl std::error::Error for DrawingExportError {}

#[derive(Clone, Copy)]
enum LineStyle {
    Border,
    Visible,
    Hidden,
    Dimension,
}

impl LineStyle {
    const fn name(self) -> &'static str {
        match self {
            Self::Border => "BORDER",
            Self::Visible => "VISIBLE",
            Self::Hidden => "HIDDEN",
            Self::Dimension => "DIMENSION",
        }
    }

    const fn dxf_linetype(self) -> &'static str {
        match self {
            Self::Hidden => "HIDDEN",
            Self::Border | Self::Visible | Self::Dimension => "CONTINUOUS",
        }
    }
}

struct PageLine {
    id: String,
    start: [f64; 2],
    end: [f64; 2],
    style: LineStyle,
}

struct PageText {
    id: String,
    position: [f64; 2],
    value: String,
    height_mm: f64,
}

pub fn export_drawing(
    snapshot: &Snapshot,
    drawing: &OrthographicDrawing,
) -> Result<DrawingExportBundle, DrawingExportError> {
    let sheet = validate_contract(snapshot, drawing)?;
    let (lines, texts) = collect_page_content(drawing)?;
    let svg = export_svg(drawing, &lines, &texts);
    let dxf = export_dxf(drawing, &lines, &texts);
    let pdf = export_pdf(drawing, &lines, &texts)?;
    if [svg.len(), dxf.len(), pdf.len()]
        .into_iter()
        .any(|length| length > MAX_EXPORT_BYTES)
    {
        return Err(DrawingExportError::ResourceLimit);
    }
    debug_assert_eq!(sheet.id(), drawing.sheet_id);
    Ok(DrawingExportBundle { svg, dxf, pdf })
}

fn validate_contract<'a>(
    snapshot: &'a Snapshot,
    drawing: &OrthographicDrawing,
) -> Result<&'a crate::drawing::DrawingSheet, DrawingExportError> {
    if !drawing.is_current(snapshot) {
        return Err(DrawingExportError::StaleDrawing);
    }
    let sheet = snapshot
        .drawing_sheet(drawing.sheet_id)
        .ok_or(DrawingExportError::InvalidDrawing)?;
    let source_line_count = drawing.views.iter().try_fold(0_usize, |count, view| {
        count.checked_add(view.visible_lines.len() + view.hidden_lines.len())
    });
    if source_line_count.is_none_or(|count| count > MAX_EXPORT_LINES) {
        return Err(DrawingExportError::ResourceLimit);
    }
    if drawing.schema != ORTHOGRAPHIC_LINEWORK_SCHEMA_V2
        || drawing.layout.schema != DRAWING_SHEET_LAYOUT_SCHEMA_V2
        || drawing.stable_source_identity.is_empty()
        || drawing.views.is_empty()
        || drawing.views.len() != drawing.layout.view_placements.len()
        || drawing.views.len() != sheet.views().len()
        || drawing.result_digest
            != drawing_result_digest(&drawing.stable_source_identity, &drawing.views)
        || drawing.layout.digest
            != drawing_layout_digest(
                sheet.page(),
                &drawing.layout.title_block,
                drawing.layout.border_bounds_mm,
                drawing.layout.title_block_bounds_mm,
                &drawing.layout.view_placements,
                &drawing.layout.linear_dimensions,
                &drawing.layout.angular_dimensions,
                &drawing.layout.circular_dimensions,
                &drawing.layout.datum_symbols,
                &drawing.layout.feature_control_frames,
                &drawing.layout.bom_balloons,
                &drawing.layout.bom_rows,
                &drawing.layout.notes,
            )
    {
        return Err(DrawingExportError::InvalidDrawing);
    }
    let expected_page_size = sheet.page().dimensions_mm().map(f64::from);
    if drawing.layout.page_size_mm != expected_page_size
        || !valid_bounds(drawing.layout.border_bounds_mm, expected_page_size)
        || !valid_bounds(drawing.layout.title_block_bounds_mm, expected_page_size)
        || !bounds_contains(
            drawing.layout.border_bounds_mm,
            drawing.layout.title_block_bounds_mm[0],
        )
        || !bounds_contains(
            drawing.layout.border_bounds_mm,
            drawing.layout.title_block_bounds_mm[1],
        )
    {
        return Err(DrawingExportError::InvalidDrawing);
    }
    for ((view, placement), expected_kind) in drawing
        .views
        .iter()
        .zip(&drawing.layout.view_placements)
        .zip(sheet.views())
    {
        validate_view(view, placement, drawing.layout.page_size_mm)?;
        if &view.kind != expected_kind {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    validate_annotations(&drawing.layout, drawing.layout.page_size_mm)?;
    Ok(sheet)
}

fn validate_view(
    view: &OrthographicView,
    placement: &DrawingViewPlacement,
    page_size: [f64; 2],
) -> Result<(), DrawingExportError> {
    if view.kind != placement.kind
        || view.stable_view_id != placement.stable_view_id
        || view.stable_view_id.is_empty()
        || !valid_model_bounds(view.bounds_mm)
        || !valid_bounds(placement.page_bounds_mm, page_size)
    {
        return Err(DrawingExportError::InvalidDrawing);
    }
    let model_size = [
        view.bounds_mm[1][0] - view.bounds_mm[0][0],
        view.bounds_mm[1][1] - view.bounds_mm[0][1],
    ];
    let page_size = [
        placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0],
        placement.page_bounds_mm[1][1] - placement.page_bounds_mm[0][1],
    ];
    let scales = [page_size[0] / model_size[0], page_size[1] / model_size[1]];
    if scales
        .into_iter()
        .any(|scale| !scale.is_finite() || scale <= 0.0)
        || (scales[0] - scales[1]).abs() > COORDINATE_EPSILON_MM * scales[0].max(scales[1])
    {
        return Err(DrawingExportError::InvalidDrawing);
    }
    for line in view.visible_lines.iter().chain(&view.hidden_lines) {
        if line.stable_line_id.is_empty()
            || !valid_point(line.start_mm)
            || !valid_point(line.end_mm)
            || !model_bounds_contains(view.bounds_mm, line.start_mm)
            || !model_bounds_contains(view.bounds_mm, line.end_mm)
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    Ok(())
}

fn validate_annotations(
    layout: &DrawingSheetLayout,
    page_size: [f64; 2],
) -> Result<(), DrawingExportError> {
    if layout.linear_dimensions.len()
        + layout.angular_dimensions.len()
        + layout.circular_dimensions.len()
        + layout.datum_symbols.len()
        + layout.feature_control_frames.len()
        + layout.bom_balloons.len()
        + layout.bom_rows.len()
        + layout.notes.len()
        > MAX_EXPORT_TEXTS
    {
        return Err(DrawingExportError::ResourceLimit);
    }
    for dimension in &layout.linear_dimensions {
        if dimension.stable_dimension_id.is_empty()
            || dimension.source_line_id.is_empty()
            || !dimension.value_mm.is_finite()
            || dimension.value_mm <= 0.0
            || !valid_text(&dimension.label)
            || !valid_point(dimension.text_position_mm)
            || !page_contains(page_size, dimension.text_position_mm)
            || dimension
                .extension_lines_mm
                .iter()
                .flatten()
                .chain(dimension.dimension_line_mm.iter())
                .any(|point| !valid_point(*point) || !page_contains(page_size, *point))
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for dimension in &layout.angular_dimensions {
        if dimension.stable_dimension_id.is_empty()
            || dimension.source_line_ids.iter().any(String::is_empty)
            || !dimension.value_degrees.is_finite()
            || !(0.0..180.0).contains(&dimension.value_degrees)
            || !valid_text(&dimension.label)
            || !valid_point(dimension.text_position_mm)
            || !page_contains(page_size, dimension.text_position_mm)
            || dimension.arc_points_mm.len() < 2
            || dimension
                .extension_lines_mm
                .iter()
                .flatten()
                .chain(&dimension.arc_points_mm)
                .any(|point| !valid_point(*point) || !page_contains(page_size, *point))
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for dimension in &layout.circular_dimensions {
        if dimension.stable_dimension_id.is_empty()
            || dimension.source_circle_id.is_empty()
            || !dimension.value_mm.is_finite()
            || dimension.value_mm <= 0.0
            || !valid_text(&dimension.label)
            || !valid_point(dimension.text_position_mm)
            || !page_contains(page_size, dimension.text_position_mm)
            || dimension
                .leader_line_mm
                .iter()
                .any(|point| !valid_point(*point) || !page_contains(page_size, *point))
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for datum in &layout.datum_symbols {
        if datum.stable_datum_id.is_empty()
            || datum.source_line_id.is_empty()
            || datum.label.is_empty()
            || !valid_bounds(datum.frame_bounds_mm, page_size)
            || !valid_point(datum.text_position_mm)
            || datum
                .leader_line_mm
                .iter()
                .chain(&datum.triangle_mm)
                .any(|point| !valid_point(*point) || !page_contains(page_size, *point))
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for frame in &layout.feature_control_frames {
        if frame.stable_frame_id.is_empty()
            || frame.source_line_id.is_empty()
            || !frame.tolerance_mm.is_finite()
            || frame.tolerance_mm <= 0.0
            || !valid_text(&frame.label)
            || !valid_bounds(frame.frame_bounds_mm, page_size)
            || !valid_point(frame.text_position_mm)
            || frame
                .leader_line_mm
                .iter()
                .any(|point| !valid_point(*point) || !page_contains(page_size, *point))
            || frame.separator_x_mm.iter().any(|x| {
                !x.is_finite()
                    || *x <= frame.frame_bounds_mm[0][0]
                    || *x >= frame.frame_bounds_mm[1][0]
            })
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for balloon in &layout.bom_balloons {
        let radius = balloon.circle_radius_mm;
        let circle_bounds = [
            [
                balloon.circle_center_mm[0] - radius,
                balloon.circle_center_mm[1] - radius,
            ],
            [
                balloon.circle_center_mm[0] + radius,
                balloon.circle_center_mm[1] + radius,
            ],
        ];
        if balloon.stable_balloon_id.is_empty()
            || balloon.instance_path.root_occurrence().0 == 0
            || balloon.position == 0
            || !valid_text(&balloon.label)
            || !radius.is_finite()
            || radius <= 0.0
            || !valid_bounds(circle_bounds, page_size)
            || !valid_point(balloon.text_position_mm)
            || balloon
                .leader_line_mm
                .iter()
                .any(|point| !valid_point(*point) || !page_contains(page_size, *point))
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for row in &layout.bom_rows {
        if row.position == 0
            || row.definition_id.0 == 0
            || row.quantity == 0
            || !valid_text(&row.part_name)
            || !valid_text(&row.label)
            || !valid_point(row.text_position_mm)
            || !page_contains(page_size, row.text_position_mm)
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    for note in &layout.notes {
        if note.stable_note_id.is_empty()
            || !valid_text(&note.text)
            || !valid_point(note.position_mm)
            || !page_contains(page_size, note.position_mm)
        {
            return Err(DrawingExportError::InvalidDrawing);
        }
    }
    let title = &layout.title_block;
    if [
        title.title(),
        title.drawing_number(),
        title.revision(),
        title.author(),
    ]
    .into_iter()
    .any(|value| !valid_text(value))
    {
        return Err(DrawingExportError::InvalidDrawing);
    }
    Ok(())
}

fn collect_page_content(
    drawing: &OrthographicDrawing,
) -> Result<(Vec<PageLine>, Vec<PageText>), DrawingExportError> {
    let line_count = drawing
        .views
        .iter()
        .try_fold(0_usize, |count, view| {
            count.checked_add(view.visible_lines.len() + view.hidden_lines.len())
        })
        .and_then(|count| count.checked_add(drawing.layout.linear_dimensions.len() * 3 + 8))
        .and_then(|count| count.checked_add(drawing.layout.angular_dimensions.len() * 18))
        .and_then(|count| count.checked_add(drawing.layout.circular_dimensions.len()))
        .and_then(|count| count.checked_add(drawing.layout.datum_symbols.len() * 8))
        .and_then(|count| count.checked_add(drawing.layout.feature_control_frames.len() * 9))
        .and_then(|count| count.checked_add(drawing.layout.bom_balloons.len() * 17))
        .ok_or(DrawingExportError::ResourceLimit)?;
    if line_count > MAX_EXPORT_LINES {
        return Err(DrawingExportError::ResourceLimit);
    }
    let mut lines = Vec::with_capacity(line_count);
    push_rectangle(
        &mut lines,
        "sheet-border",
        drawing.layout.border_bounds_mm,
        LineStyle::Border,
    );
    push_rectangle(
        &mut lines,
        "title-block",
        drawing.layout.title_block_bounds_mm,
        LineStyle::Border,
    );
    for (view, placement) in drawing.views.iter().zip(&drawing.layout.view_placements) {
        let model_width = view.bounds_mm[1][0] - view.bounds_mm[0][0];
        let page_width = placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0];
        let scale = page_width / model_width;
        for (style, source_lines) in [
            (LineStyle::Visible, view.visible_lines.as_slice()),
            (LineStyle::Hidden, view.hidden_lines.as_slice()),
        ] {
            for line in source_lines {
                lines.push(PageLine {
                    id: line.stable_line_id.clone(),
                    start: page_point(line.start_mm, placement.origin_mm, scale),
                    end: page_point(line.end_mm, placement.origin_mm, scale),
                    style,
                });
            }
        }
    }
    let mut texts = Vec::with_capacity(
        drawing.layout.linear_dimensions.len()
            + drawing.layout.angular_dimensions.len()
            + drawing.layout.circular_dimensions.len()
            + drawing.layout.datum_symbols.len()
            + drawing.layout.feature_control_frames.len()
            + drawing.layout.bom_balloons.len()
            + drawing.layout.bom_rows.len()
            + drawing.layout.notes.len()
            + 4,
    );
    for dimension in &drawing.layout.linear_dimensions {
        push_dimension(&mut lines, &mut texts, dimension);
    }
    for dimension in &drawing.layout.angular_dimensions {
        push_angular_dimension(&mut lines, &mut texts, dimension);
    }
    for dimension in &drawing.layout.circular_dimensions {
        push_circular_dimension(&mut lines, &mut texts, dimension);
    }
    for datum in &drawing.layout.datum_symbols {
        push_datum_symbol(&mut lines, &mut texts, datum);
    }
    for frame in &drawing.layout.feature_control_frames {
        push_feature_control_frame(&mut lines, &mut texts, frame);
    }
    for balloon in &drawing.layout.bom_balloons {
        push_bom_balloon(&mut lines, &mut texts, balloon);
    }
    for row in &drawing.layout.bom_rows {
        texts.push(PageText {
            id: format!("bom-row-{}", row.position),
            position: row.text_position_mm,
            value: row.label.clone(),
            height_mm: 3.0,
        });
    }
    for note in &drawing.layout.notes {
        texts.push(PageText {
            id: note.stable_note_id.clone(),
            position: note.position_mm,
            value: note.text.clone(),
            height_mm: 3.5,
        });
    }
    push_title_block_text(&mut texts, &drawing.layout);
    if lines.iter().any(|line| {
        !valid_point(line.start)
            || !valid_point(line.end)
            || !page_contains(drawing.layout.page_size_mm, line.start)
            || !page_contains(drawing.layout.page_size_mm, line.end)
    }) {
        return Err(DrawingExportError::InvalidDrawing);
    }
    Ok((lines, texts))
}

fn push_rectangle(lines: &mut Vec<PageLine>, id: &str, bounds: [[f64; 2]; 2], style: LineStyle) {
    let [min, max] = bounds;
    for (index, [start, end]) in [
        [[min[0], min[1]], [max[0], min[1]]],
        [[max[0], min[1]], [max[0], max[1]]],
        [[max[0], max[1]], [min[0], max[1]]],
        [[min[0], max[1]], [min[0], min[1]]],
    ]
    .into_iter()
    .enumerate()
    {
        lines.push(PageLine {
            id: format!("{id}-{index}"),
            start,
            end,
            style,
        });
    }
}

fn push_dimension(
    lines: &mut Vec<PageLine>,
    texts: &mut Vec<PageText>,
    dimension: &DrawingLinearDimensionLayout,
) {
    for (index, segment) in dimension.extension_lines_mm.iter().enumerate() {
        lines.push(PageLine {
            id: format!("{}/extension-{index}", dimension.stable_dimension_id),
            start: segment[0],
            end: segment[1],
            style: LineStyle::Dimension,
        });
    }
    lines.push(PageLine {
        id: format!("{}/line", dimension.stable_dimension_id),
        start: dimension.dimension_line_mm[0],
        end: dimension.dimension_line_mm[1],
        style: LineStyle::Dimension,
    });
    texts.push(PageText {
        id: dimension.stable_dimension_id.clone(),
        position: dimension.text_position_mm,
        value: dimension.label.clone(),
        height_mm: 3.5,
    });
}

fn push_angular_dimension(
    lines: &mut Vec<PageLine>,
    texts: &mut Vec<PageText>,
    dimension: &DrawingAngularDimensionLayout,
) {
    for (index, segment) in dimension.extension_lines_mm.iter().enumerate() {
        lines.push(PageLine {
            id: format!("{}/extension-{index}", dimension.stable_dimension_id),
            start: segment[0],
            end: segment[1],
            style: LineStyle::Dimension,
        });
    }
    for (index, points) in dimension.arc_points_mm.windows(2).enumerate() {
        lines.push(PageLine {
            id: format!("{}/arc-{index}", dimension.stable_dimension_id),
            start: points[0],
            end: points[1],
            style: LineStyle::Dimension,
        });
    }
    texts.push(PageText {
        id: dimension.stable_dimension_id.clone(),
        position: dimension.text_position_mm,
        value: dimension.label.clone(),
        height_mm: 3.5,
    });
}

fn push_circular_dimension(
    lines: &mut Vec<PageLine>,
    texts: &mut Vec<PageText>,
    dimension: &DrawingCircularDimensionLayout,
) {
    lines.push(PageLine {
        id: format!("{}/leader", dimension.stable_dimension_id),
        start: dimension.leader_line_mm[0],
        end: dimension.leader_line_mm[1],
        style: LineStyle::Dimension,
    });
    texts.push(PageText {
        id: dimension.stable_dimension_id.clone(),
        position: dimension.text_position_mm,
        value: dimension.label.clone(),
        height_mm: 3.5,
    });
}

fn push_datum_symbol(
    lines: &mut Vec<PageLine>,
    texts: &mut Vec<PageText>,
    datum: &DrawingDatumSymbolLayout,
) {
    lines.push(PageLine {
        id: format!("{}/leader", datum.stable_datum_id),
        start: datum.leader_line_mm[0],
        end: datum.leader_line_mm[1],
        style: LineStyle::Dimension,
    });
    for (index, edge) in [[0, 1], [1, 2], [2, 0]].into_iter().enumerate() {
        lines.push(PageLine {
            id: format!("{}/triangle-{index}", datum.stable_datum_id),
            start: datum.triangle_mm[edge[0]],
            end: datum.triangle_mm[edge[1]],
            style: LineStyle::Dimension,
        });
    }
    push_rectangle(
        lines,
        &format!("{}/frame", datum.stable_datum_id),
        datum.frame_bounds_mm,
        LineStyle::Dimension,
    );
    texts.push(PageText {
        id: datum.stable_datum_id.clone(),
        position: datum.text_position_mm,
        value: datum.label.clone(),
        height_mm: 3.5,
    });
}

fn push_feature_control_frame(
    lines: &mut Vec<PageLine>,
    texts: &mut Vec<PageText>,
    frame: &DrawingFeatureControlFrameLayout,
) {
    lines.push(PageLine {
        id: format!("{}/leader", frame.stable_frame_id),
        start: frame.leader_line_mm[0],
        end: frame.leader_line_mm[1],
        style: LineStyle::Dimension,
    });
    push_rectangle(
        lines,
        &format!("{}/frame", frame.stable_frame_id),
        frame.frame_bounds_mm,
        LineStyle::Dimension,
    );
    for (index, x) in frame.separator_x_mm.iter().enumerate() {
        lines.push(PageLine {
            id: format!("{}/separator-{index}", frame.stable_frame_id),
            start: [*x, frame.frame_bounds_mm[0][1]],
            end: [*x, frame.frame_bounds_mm[1][1]],
            style: LineStyle::Dimension,
        });
    }
    texts.push(PageText {
        id: frame.stable_frame_id.clone(),
        position: frame.text_position_mm,
        value: frame.label.clone(),
        height_mm: 3.0,
    });
}

fn push_bom_balloon(
    lines: &mut Vec<PageLine>,
    texts: &mut Vec<PageText>,
    balloon: &DrawingBomBalloonLayout,
) {
    lines.push(PageLine {
        id: format!("{}/leader", balloon.stable_balloon_id),
        start: balloon.leader_line_mm[0],
        end: balloon.leader_line_mm[1],
        style: LineStyle::Dimension,
    });
    let points = (0..16)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / 16.0;
            [
                balloon.circle_center_mm[0] + balloon.circle_radius_mm * angle.cos(),
                balloon.circle_center_mm[1] + balloon.circle_radius_mm * angle.sin(),
            ]
        })
        .collect::<Vec<_>>();
    for index in 0..points.len() {
        lines.push(PageLine {
            id: format!("{}/circle-{index}", balloon.stable_balloon_id),
            start: points[index],
            end: points[(index + 1) % points.len()],
            style: LineStyle::Dimension,
        });
    }
    texts.push(PageText {
        id: balloon.stable_balloon_id.clone(),
        position: balloon.text_position_mm,
        value: balloon.label.clone(),
        height_mm: 3.5,
    });
}

fn push_title_block_text(texts: &mut Vec<PageText>, layout: &DrawingSheetLayout) {
    let bounds = layout.title_block_bounds_mm;
    let x = bounds[0][0] + 2.0;
    let mut y = bounds[1][1] - 5.0;
    for (id, value) in [
        ("title", layout.title_block.title()),
        ("drawing-number", layout.title_block.drawing_number()),
        ("revision", layout.title_block.revision()),
        ("author", layout.title_block.author()),
    ] {
        if !value.is_empty() {
            texts.push(PageText {
                id: format!("title-block/{id}"),
                position: [x, y],
                value: value.to_owned(),
                height_mm: if id == "title" { 4.0 } else { 3.0 },
            });
            y -= 7.0;
        }
    }
}

fn export_svg(drawing: &OrthographicDrawing, lines: &[PageLine], texts: &[PageText]) -> Vec<u8> {
    let [width, height] = drawing.layout.page_size_mm;
    let mut svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- {DRAWING_EXPORT_SCHEMA_V1} result={} layout={} -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"0 0 {} {}\">\n<style>.BORDER{{stroke:#000;stroke-width:.5;fill:none}}.VISIBLE{{stroke:#000;stroke-width:.35;fill:none}}.HIDDEN{{stroke:#000;stroke-width:.25;stroke-dasharray:2 1;fill:none}}.DIMENSION{{stroke:#000;stroke-width:.18;fill:none}}text{{font-family:sans-serif;fill:#000}}</style>\n",
        drawing.result_digest,
        drawing.layout.digest,
        number(width),
        number(height),
        number(width),
        number(height),
    );
    for line in lines {
        svg.push_str(&format!(
            "<line id=\"{}\" class=\"{}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>\n",
            xml_escape(&line.id),
            line.style.name(),
            number(line.start[0]),
            number(height - line.start[1]),
            number(line.end[0]),
            number(height - line.end[1]),
        ));
    }
    for text in texts {
        svg.push_str(&format!(
            "<text id=\"{}\" x=\"{}\" y=\"{}\" font-size=\"{}\">{}</text>\n",
            xml_escape(&text.id),
            number(text.position[0]),
            number(height - text.position[1]),
            number(text.height_mm),
            xml_escape(&text.value),
        ));
    }
    svg.push_str("</svg>\n");
    svg.into_bytes()
}

fn export_dxf(drawing: &OrthographicDrawing, lines: &[PageLine], texts: &[PageText]) -> Vec<u8> {
    let mut dxf = format!(
        "999\n{DRAWING_EXPORT_SCHEMA_V1}\n999\nresult={} layout={}\n0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1027\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLTYPE\n70\n2\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0\n0\nLTYPE\n2\nHIDDEN\n70\n0\n3\nHidden line\n72\n65\n73\n2\n40\n3\n49\n2\n74\n0\n49\n-1\n74\n0\n0\nENDTAB\n0\nTABLE\n2\nLAYER\n70\n5\n0\nLAYER\n2\nBORDER\n70\n0\n62\n7\n6\nCONTINUOUS\n0\nLAYER\n2\nVISIBLE\n70\n0\n62\n7\n6\nCONTINUOUS\n0\nLAYER\n2\nHIDDEN\n70\n0\n62\n8\n6\nHIDDEN\n0\nLAYER\n2\nDIMENSION\n70\n0\n62\n3\n6\nCONTINUOUS\n0\nLAYER\n2\nANNOTATION\n70\n0\n62\n7\n6\nCONTINUOUS\n0\nENDTAB\n0\nTABLE\n2\nAPPID\n70\n1\n0\nAPPID\n2\nKETCHUP\n70\n0\n0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n",
        drawing.result_digest, drawing.layout.digest
    );
    for line in lines {
        dxf.push_str(&format!(
            "0\nLINE\n8\n{}\n6\n{}\n1001\nKETCHUP\n1000\n{}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
            line.style.name(),
            line.style.dxf_linetype(),
            dxf_text(&line.id),
            number(line.start[0]),
            number(line.start[1]),
            number(line.end[0]),
            number(line.end[1]),
        ));
    }
    for text in texts {
        dxf.push_str(&format!(
            "0\nTEXT\n8\nANNOTATION\n1001\nKETCHUP\n1000\n{}\n10\n{}\n20\n{}\n30\n0\n40\n{}\n1\n{}\n",
            dxf_text(&text.id),
            number(text.position[0]),
            number(text.position[1]),
            number(text.height_mm),
            dxf_text(&text.value),
        ));
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    dxf.into_bytes()
}

fn export_pdf(
    drawing: &OrthographicDrawing,
    lines: &[PageLine],
    texts: &[PageText],
) -> Result<Vec<u8>, DrawingExportError> {
    let mut stream = format!(
        "% {DRAWING_EXPORT_SCHEMA_V1} result={} layout={}\nq\n{} 0 0 {} 0 0 cm\n",
        drawing.result_digest,
        drawing.layout.digest,
        number(POINTS_PER_MM),
        number(POINTS_PER_MM),
    );
    for line in lines {
        let (width, dash) = match line.style {
            LineStyle::Border => (0.5, "[] 0 d"),
            LineStyle::Visible => (0.35, "[] 0 d"),
            LineStyle::Hidden => (0.25, "[2 1] 0 d"),
            LineStyle::Dimension => (0.18, "[] 0 d"),
        };
        stream.push_str(&format!(
            "% {} {}\n{} w {dash} {} {} m {} {} l S\n",
            line.style.name(),
            pdf_comment(&line.id),
            number(width),
            number(line.start[0]),
            number(line.start[1]),
            number(line.end[0]),
            number(line.end[1]),
        ));
    }
    for text in texts {
        stream.push_str(&format!(
            "% text {}\nBT /F1 {} Tf {} {} Td <{}> Tj ET\n",
            pdf_comment(&text.id),
            number(text.height_mm),
            number(text.position[0]),
            number(text.position[1]),
            pdf_text_hex(&text.value)?,
        ));
    }
    stream.push_str("Q\n");
    let [width, height] = drawing.layout.page_size_mm;
    let encoding_differences = PDF_UNICODE_GLYPHS
        .iter()
        .map(|(_, code, glyph)| format!("{code} /{glyph}"))
        .collect::<Vec<_>>()
        .join(" ");
    let to_unicode = pdf_to_unicode_cmap();
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            number(width * POINTS_PER_MM),
            number(height * POINTS_PER_MM),
        ),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding 6 0 R /ToUnicode 7 0 R >>"
            .to_owned(),
        format!(
            "<< /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [{encoding_differences}] >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{to_unicode}endstream",
            to_unicode.len()
        ),
    ];
    let mut pdf = b"%PDF-1.7\n% Ketchup deterministic vector drawing\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    Ok(pdf)
}

fn page_point(point: [f64; 2], origin: [f64; 2], scale: f64) -> [f64; 2] {
    [origin[0] + point[0] * scale, origin[1] + point[1] * scale]
}

fn valid_model_bounds(bounds: [[f64; 2]; 2]) -> bool {
    bounds.into_iter().flatten().all(f64::is_finite)
        && bounds[0][0] < bounds[1][0]
        && bounds[0][1] < bounds[1][1]
}

fn valid_bounds(bounds: [[f64; 2]; 2], page_size: [f64; 2]) -> bool {
    valid_model_bounds(bounds)
        && bounds_contains([[0.0, 0.0], page_size], bounds[0])
        && bounds_contains([[0.0, 0.0], page_size], bounds[1])
}

fn valid_point(point: [f64; 2]) -> bool {
    point.into_iter().all(f64::is_finite)
}

fn page_contains(page_size: [f64; 2], point: [f64; 2]) -> bool {
    bounds_contains([[0.0, 0.0], page_size], point)
}

fn model_bounds_contains(bounds: [[f64; 2]; 2], point: [f64; 2]) -> bool {
    bounds_contains(bounds, point)
}

fn bounds_contains(bounds: [[f64; 2]; 2], point: [f64; 2]) -> bool {
    point[0] >= bounds[0][0] - COORDINATE_EPSILON_MM
        && point[0] <= bounds[1][0] + COORDINATE_EPSILON_MM
        && point[1] >= bounds[0][1] - COORDINATE_EPSILON_MM
        && point[1] <= bounds[1][1] + COORDINATE_EPSILON_MM
}

fn valid_text(value: &str) -> bool {
    value.len() <= 256 && !value.chars().any(char::is_control)
}

fn number(value: f64) -> String {
    let canonical = if value.abs() < 0.000_000_5 {
        0.0
    } else {
        value
    };
    let mut output = format!("{canonical:.6}");
    while output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    output
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn dxf_text(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn pdf_text_hex(value: &str) -> Result<String, DrawingExportError> {
    let mut encoded = String::with_capacity(value.len() * 2);
    for character in value.chars() {
        let code = if character.is_ascii() && !character.is_ascii_control() {
            character as u8
        } else {
            PDF_UNICODE_GLYPHS
                .iter()
                .find_map(|(candidate, code, _)| (*candidate == character).then_some(*code))
                .ok_or(DrawingExportError::UnsupportedPdfText)?
        };
        encoded.push_str(&format!("{code:02X}"));
    }
    Ok(encoded)
}

fn pdf_to_unicode_cmap() -> String {
    let mappings = (32_u8..=126)
        .map(|code| (code, char::from(code)))
        .chain(
            PDF_UNICODE_GLYPHS
                .iter()
                .map(|(character, code, _)| (*code, *character)),
        )
        .collect::<Vec<_>>();
    let mut cmap = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /KetchupUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n".to_owned();
    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (code, character) in chunk {
            cmap.push_str(&format!("<{code:02X}> <{:04X}>\n", *character as u32));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

fn pdf_comment(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\r' | '\n' => " ".to_owned(),
            character if character.is_ascii() => character.to_string(),
            character => format!("U+{:04X}", character as u32),
        })
        .collect()
}
