#![forbid(unsafe_code)]

use crate::document::Snapshot;
use crate::drawing::{
    DRAWING_SHEET_LAYOUT_SCHEMA_V2, DrawingAngularDimensionLayout, DrawingBomBalloonLayout,
    DrawingCircularDimensionLayout, DrawingDatumSymbolLayout, DrawingFeatureControlFrameLayout,
    DrawingLinearDimensionLayout, DrawingSheetLayout, DrawingViewPlacement,
    ORTHOGRAPHIC_LINEWORK_SCHEMA_V2, OrthographicDrawing, OrthographicView, ProjectedPolylineCurve,
    drawing_layout_digest, drawing_result_digest,
};
use crate::r12_text;
use crate::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
use std::collections::BTreeSet;
use std::fmt;

mod annotations;
mod arcs;
pub mod layers;

pub use annotations::SheetAnnotations;

pub const DRAWING_EXPORT_SCHEMA_V1: &str = "ketchup.drawing-export.v1";
const MAX_EXPORT_LINES: usize = 100_000;
const MAX_EXPORT_TEXTS: usize = 1_024;
const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
/// Baseline-to-baseline distance of a text broken over lines, in text heights.
const LINE_SPACING: f64 = 1.5;
/// A generous glyph advance of the sheet's sans-serif font, in text heights,
/// so a line measured with it is no shorter than in any reader.
const GLYPH_WIDTH: f64 = 0.7;
/// The title-block texts shrink in these steps until they fit its frame; at
/// the last one a title is 2.4 mm and the other fields 1.8 mm high.
const TITLE_BLOCK_SCALES: [f64; 5] = [1.0, 0.9, 0.8, 0.7, 0.6];
/// Text height of a parts-list row.
const BOM_ROW_HEIGHT_MM: f64 = 3.0;
/// Width of a parts-list column; its rows keep 3 mm clear of the next one.
pub(crate) const BOM_COLUMN_WIDTH_MM: f64 = 70.0;
const COORDINATE_EPSILON_MM: f64 = DEFAULT_LINEAR_TOLERANCE_MM;
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
    dxf_curves: DxfCurves,
    annotations: SheetAnnotations,
}

/// How the DXF carries the curves of the drawing: exact circles of the views
/// as CIRCLE and ARC entities in place of their tessellated lines, and the
/// exact curves it keeps as lines, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DxfCurves {
    pub circles: usize,
    pub arcs: usize,
    pub replaced_lines: usize,
    pub polyline_curves: Vec<ProjectedPolylineCurve>,
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

    #[must_use]
    pub fn dxf_curves(&self) -> &DxfCurves {
        &self.dxf_curves
    }

    /// The plan annotations the top views drew, and those left out.
    #[must_use]
    pub fn annotations(&self) -> &SheetAnnotations {
        &self.annotations
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingExportError {
    StaleDrawing,
    InvalidDrawing,
    UnsupportedPdfText,
    UnsupportedDxfText,
    DxfTextTooLong,
    TitleBlockTooLong,
    WrappedNoteOverlaps,
    ResourceLimit,
}

impl fmt::Display for DrawingExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StaleDrawing => "drawing export source is stale",
            Self::InvalidDrawing => "drawing export contract is inconsistent",
            Self::UnsupportedPdfText => "drawing contains text unsupported by the PDF font",
            Self::UnsupportedDxfText => {
                "drawing contains a character outside the Basic Multilingual Plane, which DXF \\U+ escapes cannot carry"
            }
            Self::DxfTextTooLong => {
                "drawing contains text longer than the 255 characters a DXF R12 string carries once non-ASCII characters are escaped"
            }
            Self::TitleBlockTooLong => {
                "the title block's title, drawing number, revision and author do not fit its frame even at the smallest legible text size"
            }
            Self::WrappedNoteOverlaps => {
                "a note longer than a DXF R12 string continues on lines below its position that would print over other text or the title block; move the note or shorten it"
            }
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

#[derive(Clone)]
struct PageLine {
    id: String,
    start: [f64; 2],
    end: [f64; 2],
    style: LineStyle,
}

/// A filled triangle of the sheet: a leader's arrowhead.
#[derive(Clone)]
struct PageFill {
    id: String,
    corners: [[f64; 2]; 3],
}

struct PageArc {
    id: String,
    center: [f64; 2],
    radius: f64,
    start_degrees: f64,
    end_degrees: f64,
    full: bool,
    style: LineStyle,
}

/// A text of the sheet: its first line's baseline starts at `position` and
/// each further line sits [`LINE_SPACING`] text heights below the previous one.
#[derive(Clone)]
struct PageText {
    id: String,
    position: [f64; 2],
    lines: Vec<String>,
    height_mm: f64,
    /// Counter-clockwise turn of the text about `position`, in degrees.
    angle_deg: f64,
    /// Whether `position` is the middle of the line, not its start.
    middle: bool,
}

impl PageText {
    fn turned(&self) -> bool {
        self.angle_deg.abs() > f64::EPSILON
    }
}

pub fn export_drawing(
    snapshot: &Snapshot,
    drawing: &OrthographicDrawing,
) -> Result<DrawingExportBundle, DrawingExportError> {
    let sheet = validate_contract(snapshot, drawing)?;
    let (mut lines, mut texts) = collect_page_content(drawing)?;
    let (annotations, marks) = annotations::draw_sheet_annotations(snapshot, sheet, drawing);
    // The sheet's own lines and texts come first; the DXF writes the
    // annotations' apart, on the layers of their tags.
    let (sheet_lines, sheet_texts) = (lines.len(), texts.len());
    let mut fills = Vec::new();
    for mark in &marks {
        lines.extend(mark.lines.iter().cloned());
        fills.extend(mark.fills.iter().cloned());
        texts.extend(mark.texts.iter().cloned());
    }
    if lines.len() + fills.len() > MAX_EXPORT_LINES || texts.len() > MAX_EXPORT_LINES {
        return Err(DrawingExportError::ResourceLimit);
    }
    let (page_arcs, replaced_lines) = collect_page_arcs(drawing)?;
    let svg = export_svg(drawing, &lines, &fills, &texts);
    let pdf = export_pdf(drawing, &lines, &fills, &texts)?;
    let dxf = export_dxf(
        drawing,
        SheetContent {
            lines: &lines[..sheet_lines],
            texts: &texts[..sheet_texts],
            arcs: &page_arcs,
            replaced_lines: &replaced_lines,
            marks: &marks,
        },
    )?;
    if [svg.len(), dxf.len(), pdf.len()]
        .into_iter()
        .any(|length| length > MAX_EXPORT_BYTES)
    {
        return Err(DrawingExportError::ResourceLimit);
    }
    debug_assert_eq!(sheet.id(), drawing.sheet_id);
    let dxf_curves = DxfCurves {
        circles: page_arcs.iter().filter(|arc| arc.full).count(),
        arcs: page_arcs.iter().filter(|arc| !arc.full).count(),
        replaced_lines: replaced_lines.len(),
        polyline_curves: drawing
            .views
            .iter()
            .flat_map(|view| view.polyline_curves.iter().cloned())
            .collect(),
    };
    Ok(DrawingExportBundle {
        svg,
        dxf,
        pdf,
        dxf_curves,
        annotations,
    })
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
        let scale = view_scale(view, placement);
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
            lines: bom_row_lines(&row.label),
            height_mm: BOM_ROW_HEIGHT_MM,
            angle_deg: 0.0,
            middle: false,
        });
    }
    let notes = texts.len()..texts.len() + drawing.layout.notes.len();
    for note in &drawing.layout.notes {
        texts.push(PageText {
            id: note.stable_note_id.clone(),
            position: note.position_mm,
            lines: r12_text::lines(&note.text, f64::INFINITY),
            height_mm: 3.5,
            angle_deg: 0.0,
            middle: false,
        });
    }
    push_title_block_text(&mut texts, &drawing.layout)?;
    check_wrapped_notes(&texts, notes, drawing.layout.title_block_bounds_mm[1][1])?;
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

fn view_scale(view: &OrthographicView, placement: &DrawingViewPlacement) -> f64 {
    let model_width = view.bounds_mm[1][0] - view.bounds_mm[0][0];
    let page_width = placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0];
    page_width / model_width
}

/// The exact circles and arcs of the views on the page, and the ids of the
/// lines they replace in the DXF.
fn collect_page_arcs(
    drawing: &OrthographicDrawing,
) -> Result<(Vec<PageArc>, BTreeSet<String>), DrawingExportError> {
    let mut page_arcs = Vec::new();
    let mut replaced = BTreeSet::new();
    for (view, placement) in drawing.views.iter().zip(&drawing.layout.view_placements) {
        let scale = view_scale(view, placement);
        let view_arcs = arcs::view_arcs(view);
        for arc in view_arcs.arcs {
            let center = page_point(arc.center, placement.origin_mm, scale);
            let radius = arc.radius * scale;
            if !valid_point(center) || !radius.is_finite() || radius <= 0.0 {
                return Err(DrawingExportError::InvalidDrawing);
            }
            page_arcs.push(PageArc {
                id: arc.id,
                center,
                radius,
                start_degrees: arc.start_rad.to_degrees(),
                end_degrees: (arc.start_rad + arc.span_rad).to_degrees() % 360.0,
                full: arc.span_rad >= std::f64::consts::TAU,
                style: arc.style,
            });
        }
        replaced.extend(view_arcs.replaced_lines);
    }
    Ok((page_arcs, replaced))
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
        lines: vec![dimension.label.clone()],
        height_mm: 3.5,
        angle_deg: 0.0,
        middle: false,
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
        lines: vec![dimension.label.clone()],
        height_mm: 3.5,
        angle_deg: 0.0,
        middle: false,
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
        lines: vec![dimension.label.clone()],
        height_mm: 3.5,
        angle_deg: 0.0,
        middle: false,
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
        lines: vec![datum.label.clone()],
        height_mm: 3.5,
        angle_deg: 0.0,
        middle: false,
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
        lines: vec![frame.label.clone()],
        height_mm: 3.0,
        angle_deg: 0.0,
        middle: false,
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
        lines: vec![balloon.label.clone()],
        height_mm: 3.5,
        angle_deg: 0.0,
        middle: false,
    });
}

/// The title, drawing number, revision and author one under the other in the
/// title-block frame, each broken into lines that fit its width (and an R12
/// string), every field starting below the last line of the one above. When
/// they do not fit the frame's height the texts shrink; when they do not fit
/// even at the smallest legible size the export is refused.
fn push_title_block_text(
    texts: &mut Vec<PageText>,
    layout: &DrawingSheetLayout,
) -> Result<(), DrawingExportError> {
    let [[left, bottom], [right, top]] = layout.title_block_bounds_mm;
    let fields = [
        ("title", layout.title_block.title(), 4.0),
        ("drawing-number", layout.title_block.drawing_number(), 3.0),
        ("revision", layout.title_block.revision(), 3.0),
        ("author", layout.title_block.author(), 3.0),
    ];
    for scale in TITLE_BLOCK_SCALES {
        let mut placed = Vec::new();
        let mut y = top - 5.0 * scale;
        for (id, value, height) in fields {
            if value.is_empty() {
                continue;
            }
            let height_mm = height * scale;
            let characters = (right - left - 4.0) / (GLYPH_WIDTH * height_mm);
            let lines = r12_text::lines(value, characters.max(1.0));
            let first = y;
            for _ in 1..lines.len() {
                y -= LINE_SPACING * height_mm;
            }
            placed.push(PageText {
                id: format!("title-block/{id}"),
                position: [left + 2.0, first],
                lines,
                height_mm,
                angle_deg: 0.0,
                middle: false,
            });
            y -= 7.0 * scale;
        }
        if y + 7.0 * scale - 1.0 >= bottom {
            texts.extend(placed);
            return Ok(());
        }
    }
    Err(DrawingExportError::TitleBlockTooLong)
}

/// A note longer than an R12 string continues on lines below the position
/// its author gave it. Those lines must stay above the title block and clear
/// of every other text of the sheet; otherwise the export says so rather than
/// printing them over what is there.
fn check_wrapped_notes(
    texts: &[PageText],
    notes: std::ops::Range<usize>,
    title_block_top: f64,
) -> Result<(), DrawingExportError> {
    for index in notes {
        for wrapped in line_boxes(&texts[index]).skip(1) {
            if wrapped[0][1] < title_block_top
                || texts.iter().enumerate().any(|(other, text)| {
                    other != index
                        && line_boxes(text).any(|line| {
                            wrapped[0][0] < line[1][0]
                                && line[0][0] < wrapped[1][0]
                                && wrapped[0][1] < line[1][1]
                                && line[0][1] < wrapped[1][1]
                        })
                })
            {
                return Err(DrawingExportError::WrappedNoteOverlaps);
            }
        }
    }
    Ok(())
}

/// The page area each line of `text` covers, from its descenders to its cap
/// and as wide as [`GLYPH_WIDTH`] makes it.
fn line_boxes(text: &PageText) -> impl Iterator<Item = [[f64; 2]; 2]> + '_ {
    let advance = GLYPH_WIDTH * text.height_mm;
    text.lines
        .iter()
        .scan(text.position[1], move |baseline, line| {
            let width = line.chars().fold(0.0, |width, _| width + advance);
            let bounds = [
                [text.position[0], *baseline - 0.25 * text.height_mm],
                [text.position[0] + width, *baseline + text.height_mm],
            ];
            *baseline -= LINE_SPACING * text.height_mm;
            Some(bounds)
        })
}

fn export_svg(
    drawing: &OrthographicDrawing,
    lines: &[PageLine],
    fills: &[PageFill],
    texts: &[PageText],
) -> Vec<u8> {
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
    for fill in fills {
        let points = fill
            .corners
            .map(|[x, y]| format!("{},{}", number(x), number(height - y)))
            .join(" ");
        svg.push_str(&format!(
            "<polygon id=\"{}\" points=\"{points}\" style=\"fill:#000;stroke:none\"/>\n",
            xml_escape(&fill.id),
        ));
    }
    for text in texts {
        let value = match text.lines.as_slice() {
            [line] => xml_escape(line),
            lines => lines
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    format!(
                        "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                        number(text.position[0]),
                        number(if index == 0 {
                            0.0
                        } else {
                            LINE_SPACING * text.height_mm
                        }),
                        xml_escape(line)
                    )
                })
                .collect(),
        };
        let (x, y) = (number(text.position[0]), number(height - text.position[1]));
        let anchor = if text.middle {
            " text-anchor=\"middle\""
        } else {
            ""
        };
        // The SVG runs y down, so a counter-clockwise turn is negative there.
        let turn = if !text.turned() {
            String::new()
        } else {
            format!(" transform=\"rotate({} {x} {y})\"", number(-text.angle_deg))
        };
        svg.push_str(&format!(
            "<text id=\"{}\" x=\"{x}\" y=\"{y}\" font-size=\"{}\"{anchor}{turn}>{value}</text>\n",
            xml_escape(&text.id),
            number(text.height_mm),
        ));
    }
    svg.push_str("</svg>\n");
    svg.into_bytes()
}

/// What the sheet DXF writes: the sheet's own lines, texts and exact curves,
/// and the annotations apart, each on the layer of its tag.
struct SheetContent<'a> {
    lines: &'a [PageLine],
    texts: &'a [PageText],
    arcs: &'a [PageArc],
    replaced_lines: &'a BTreeSet<String>,
    marks: &'a [annotations::SheetMark],
}

/// The sheet as AutoCAD R12 (AC1009) DXF, like the profile export: R12 needs no
/// handles, BLOCK_RECORD table or OBJECTS section, and each entity ends with
/// its KETCHUP id as extended data, after all of its own groups. A sheet
/// without annotations writes no tag layer and no BLOCKS section.
fn export_dxf(
    drawing: &OrthographicDrawing,
    content: SheetContent<'_>,
) -> Result<Vec<u8>, DrawingExportError> {
    let tag_layers: BTreeSet<&str> = content
        .marks
        .iter()
        .map(|mark| mark.layer.as_str())
        .collect();
    let mut annotation_layers = String::new();
    if !content.marks.is_empty() {
        annotation_layers.push_str("0\nLAYER\n2\n0\n70\n0\n62\n7\n6\nCONTINUOUS\n");
    }
    for layer in &tag_layers {
        annotation_layers.push_str(&format!(
            "0\nLAYER\n2\n{layer}\n70\n0\n62\n7\n6\nCONTINUOUS\n"
        ));
    }
    let layer_count = 5 + usize::from(!content.marks.is_empty()) + tag_layers.len();
    let mut dxf = format!(
        "999\n{DRAWING_EXPORT_SCHEMA_V1}\n999\nresult={} layout={}\n0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$DWGCODEPAGE\n3\nANSI_1252\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLTYPE\n70\n2\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0\n0\nLTYPE\n2\nHIDDEN\n70\n0\n3\nHidden line\n72\n65\n73\n2\n40\n3\n49\n2\n49\n-1\n0\nENDTAB\n0\nTABLE\n2\nLAYER\n70\n",
        drawing.result_digest, drawing.layout.digest
    );
    dxf.push_str(&format!("{layer_count}\n"));
    dxf.push_str("0\nLAYER\n2\nBORDER\n70\n0\n62\n7\n6\nCONTINUOUS\n0\nLAYER\n2\nVISIBLE\n70\n0\n62\n7\n6\nCONTINUOUS\n0\nLAYER\n2\nHIDDEN\n70\n0\n62\n8\n6\nHIDDEN\n0\nLAYER\n2\nDIMENSION\n70\n0\n62\n3\n6\nCONTINUOUS\n0\nLAYER\n2\nANNOTATION\n70\n0\n62\n7\n6\nCONTINUOUS\n");
    dxf.push_str(&annotation_layers);
    dxf.push_str(
        "0\nENDTAB\n0\nTABLE\n2\nAPPID\n70\n1\n0\nAPPID\n2\nKETCHUP\n70\n0\n0\nENDTAB\n0\nENDSEC\n",
    );
    let (blocks, annotation_entities) = annotation_dxf(content.marks)?;
    if !blocks.is_empty() {
        dxf.push_str("0\nSECTION\n2\nBLOCKS\n");
        dxf.push_str(&blocks);
        dxf.push_str("0\nENDSEC\n");
    }
    dxf.push_str("0\nSECTION\n2\nENTITIES\n");
    for line in content
        .lines
        .iter()
        .filter(|line| !content.replaced_lines.contains(&line.id))
    {
        push_dxf_line(&mut dxf, line, line.style.name(), line.style.dxf_linetype())?;
    }
    for arc in content.arcs {
        let angles = if arc.full {
            String::new()
        } else {
            format!(
                "50\n{}\n51\n{}\n",
                number(arc.start_degrees),
                number(arc.end_degrees)
            )
        };
        dxf.push_str(&format!(
            "0\n{}\n8\n{}\n6\n{}\n10\n{}\n20\n{}\n30\n0\n40\n{}\n{angles}1001\nKETCHUP\n1000\n{}\n",
            if arc.full { "CIRCLE" } else { "ARC" },
            arc.style.name(),
            arc.style.dxf_linetype(),
            number(arc.center[0]),
            number(arc.center[1]),
            number(arc.radius),
            dxf_text(&arc.id)?,
        ));
    }
    for text in content.texts {
        push_dxf_text(&mut dxf, text, "ANNOTATION")?;
    }
    dxf.push_str(&annotation_entities);
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    Ok(dxf.into_bytes())
}

/// The annotations of the sheet as DXF: the blocks that draw its dimensions,
/// and its entities. A dimension is an aligned DIMENSION through the measured
/// points on the page; as the view is drawn to scale, it carries the value
/// Kečup measured in the model as its text.
fn annotation_dxf(
    marks: &[annotations::SheetMark],
) -> Result<(String, String), DrawingExportError> {
    let (mut blocks, mut entities) = (String::new(), String::new());
    let mut block_count = 0_usize;
    for mark in marks {
        let Some(dimension) = &mark.dimension else {
            for line in &mark.lines {
                push_dxf_line(&mut entities, line, &mark.layer, "CONTINUOUS")?;
            }
            for fill in &mark.fills {
                // R12 takes a triangle as a SOLID whose last two corners meet.
                let [a, b, c] = fill.corners;
                entities.push_str(&format!(
                    "0\nSOLID\n8\n{}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n12\n{}\n22\n{}\n32\n0\n13\n{}\n23\n{}\n33\n0\n1001\nKETCHUP\n1000\n{}\n",
                    mark.layer,
                    number(a[0]),
                    number(a[1]),
                    number(b[0]),
                    number(b[1]),
                    number(c[0]),
                    number(c[1]),
                    number(c[0]),
                    number(c[1]),
                    dxf_text(&fill.id)?,
                ));
            }
            for text in &mark.texts {
                push_dxf_text(&mut entities, text, &mark.layer)?;
            }
            continue;
        };
        block_count += 1;
        let block = format!("*D{block_count}");
        blocks.push_str(&format!(
            "0\nBLOCK\n8\n0\n2\n{block}\n70\n1\n10\n0\n20\n0\n30\n0\n3\n{block}\n"
        ));
        for line in &mark.lines {
            push_dxf_line(&mut blocks, line, "0", "CONTINUOUS")?;
        }
        for text in &mark.texts {
            push_dxf_text(&mut blocks, text, "0")?;
        }
        blocks.push_str("0\nENDBLK\n8\n0\n");
        let id = mark
            .texts
            .first()
            .map_or_else(String::new, |text| text.id.clone());
        entities.push_str(&format!(
            "0\nDIMENSION\n8\n{}\n2\n{block}\n3\nSTANDARD\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n70\n1\n1\n{}\n13\n{}\n23\n{}\n33\n0\n14\n{}\n24\n{}\n34\n0\n1001\nKETCHUP\n1000\n{}\n",
            mark.layer,
            number(dimension.line[1][0]),
            number(dimension.line[1][1]),
            number(dimension.value_at[0]),
            number(dimension.value_at[1]),
            dxf_text(&dimension.value)?,
            number(dimension.measured[0][0]),
            number(dimension.measured[0][1]),
            number(dimension.measured[1][0]),
            number(dimension.measured[1][1]),
            dxf_text(&id)?,
        ));
    }
    Ok((blocks, entities))
}

fn push_dxf_line(
    dxf: &mut String,
    line: &PageLine,
    layer: &str,
    linetype: &str,
) -> Result<(), DrawingExportError> {
    dxf.push_str(&format!(
        "0\nLINE\n8\n{layer}\n6\n{linetype}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n1001\nKETCHUP\n1000\n{}\n",
        number(line.start[0]),
        number(line.start[1]),
        number(line.end[0]),
        number(line.end[1]),
        dxf_text(&line.id)?,
    ));
    Ok(())
}

/// A text as one TEXT per line, each turned with it and the next one a line
/// pitch further down its own up direction.
fn push_dxf_text(dxf: &mut String, text: &PageText, layer: &str) -> Result<(), DrawingExportError> {
    let id = dxf_text(&text.id)?;
    let (sin, cos) = text.angle_deg.to_radians().sin_cos();
    let mut at = text.position;
    for piece in text
        .lines
        .iter()
        .flat_map(|line| r12_text::lines(line, f64::INFINITY))
    {
        let value = dxf_text(&piece)?;
        let (x, y) = (number(at[0]), number(at[1]));
        let turn = if !text.turned() {
            String::new()
        } else {
            format!("50\n{}\n", number(text.angle_deg))
        };
        // Centred on the alignment point (11/21), which R12 reads instead of
        // the insertion point.
        let middle = if text.middle {
            format!("72\n1\n11\n{x}\n21\n{y}\n31\n0\n")
        } else {
            String::new()
        };
        dxf.push_str(&format!(
            "0\nTEXT\n8\n{layer}\n10\n{x}\n20\n{y}\n30\n0\n40\n{}\n1\n{value}\n{turn}{middle}1001\nKETCHUP\n1000\n{id}\n",
            number(text.height_mm),
        ));
        let step = LINE_SPACING * text.height_mm;
        at = [at[0] + sin * step, at[1] - cos * step];
    }
    Ok(())
}

fn export_pdf(
    drawing: &OrthographicDrawing,
    lines: &[PageLine],
    fills: &[PageFill],
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
    for fill in fills {
        let [a, b, c] = fill
            .corners
            .map(|[x, y]| format!("{} {}", number(x), number(y)));
        stream.push_str(&format!(
            "% fill {}\n{a} m {b} l {c} l h f\n",
            pdf_comment(&fill.id)
        ));
    }
    for text in texts {
        if text.middle || text.turned() {
            push_pdf_turned_text(&mut stream, text)?;
            continue;
        }
        let mut baseline = text.position[1];
        for line in &text.lines {
            stream.push_str(&format!(
                "% text {}\nBT /F1 {} Tf {} {} Td <{}> Tj ET\n",
                pdf_comment(&text.id),
                number(text.height_mm),
                number(text.position[0]),
                number(baseline),
                pdf_text_hex(line)?,
            ));
            baseline -= LINE_SPACING * text.height_mm;
        }
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

/// A text turned about its position, or centred on it: each line placed by a
/// text matrix, centred with the Helvetica widths of its characters.
fn push_pdf_turned_text(stream: &mut String, text: &PageText) -> Result<(), DrawingExportError> {
    let (sin, cos) = text.angle_deg.to_radians().sin_cos();
    let mut at = text.position;
    for line in &text.lines {
        let half = if text.middle {
            line.chars().map(helvetica_width_em).sum::<f64>() * text.height_mm / 2.0
        } else {
            0.0
        };
        stream.push_str(&format!(
            "% text {}\nBT /F1 {} Tf {} {} {} {} {} {} Tm <{}> Tj ET\n",
            pdf_comment(&text.id),
            number(text.height_mm),
            number(cos),
            number(sin),
            number(-sin),
            number(cos),
            number(at[0] - cos * half),
            number(at[1] - sin * half),
            pdf_text_hex(line)?,
        ));
        let step = LINE_SPACING * text.height_mm;
        at = [at[0] + sin * step, at[1] - cos * step];
    }
    Ok(())
}

/// The advance of a Helvetica character in em (Adobe's AFM): exact for the
/// digits, separators and signs a measured value writes, a digit's width for
/// any other.
fn helvetica_width_em(character: char) -> f64 {
    match character {
        '.' | ',' | ' ' => 0.278,
        '-' => 0.333,
        _ => 0.556,
    }
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

/// A string of the R12 file as [`crate::r12_text`] writes it. A character
/// beyond the Basic Multilingual Plane (an emoji) is refused rather than
/// written as a surrogate pair no reader decodes, and a string longer than
/// the 255 characters an R12 string carries once written.
fn dxf_text(value: &str) -> Result<String, DrawingExportError> {
    let output = value
        .chars()
        .map(|character| {
            r12_text::character(character).ok_or(DrawingExportError::UnsupportedDxfText)
        })
        .collect::<Result<String, _>>()?;
    if output.len() > r12_text::MAX_STRING_CHARACTERS {
        return Err(DrawingExportError::DxfTextTooLong);
    }
    Ok(output)
}

/// A parts-list row's label broken into the lines that fit its column.
pub(crate) fn bom_row_lines(label: &str) -> Vec<String> {
    let characters = (BOM_COLUMN_WIDTH_MM - 3.0) / (GLYPH_WIDTH * BOM_ROW_HEIGHT_MM);
    r12_text::lines(label, characters.floor())
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

#[cfg(test)]
mod dxf_text_tests {
    use super::{DrawingExportError, dxf_text, r12_text};

    #[test]
    fn a_long_text_breaks_after_the_last_space_that_fits_or_between_characters() {
        let words = format!("{} {}", "a".repeat(250), "b".repeat(10));
        assert_eq!(
            r12_text::lines(&words, f64::INFINITY),
            [format!("{} ", "a".repeat(250)), "b".repeat(10)]
        );
        let word = "č".repeat(40);
        assert_eq!(
            r12_text::lines(&word, f64::INFINITY),
            ["č".repeat(36), "č".repeat(4)]
        );
        assert_eq!(
            r12_text::lines("ab cd efgh ij", 6.0),
            ["ab cd ".to_owned(), "efgh ".to_owned(), "ij".to_owned()]
        );
        assert_eq!(r12_text::lines("", 6.0), [String::new()]);
    }

    #[test]
    fn a_character_beyond_the_basic_plane_is_refused_not_written_as_surrogates() {
        assert_eq!(
            dxf_text("Drawing \u{1F6AB}"),
            Err(DrawingExportError::UnsupportedDxfText)
        );
        assert_eq!(
            dxf_text("Kečup\u{FFFD}").as_deref(),
            Ok("Ke\\U+010Dup\\U+FFFD")
        );
    }

    /// Review 2026-10-24 MF-28: the sheet wrote a percent sign as it is, so
    /// "%%d" in a text came out as a degree sign.
    #[test]
    fn a_percent_sign_is_written_as_the_r12_code_of_one() {
        assert_eq!(dxf_text("50 %%d").as_deref(), Ok("50 %%%%%%d"));
    }
}
