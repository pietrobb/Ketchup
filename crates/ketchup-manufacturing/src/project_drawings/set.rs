//! The project drawings as a set of sheets: a floor plan of each storey, a
//! sheet of the longitudinal and the cross section and a sheet of the four
//! elevations, all at one scale, each sheet on the smallest ISO A format that
//! holds it (or the format the caller fixes) with its frame and title block.
//! The title block of every sheet says what the sheet shows and its number in
//! the set, "n / N". After the elevations comes a shop drawing of each group
//! of parts the caller names, at its own scale (see [`super::shop`]). When the
//! model has windows or doors, each plan marks those of its storey and a last
//! sheet lists them (see [`super::openings`]).
//!
//! Each sheet is drawn twice: painted far to near for the PDF, and as its
//! visible lines alone for a DXF, which cannot paint faces over lines.

use super::openings::{OpeningSchedule, draw_schedule, opening_points, schedule_format};
use super::shop::{PlannedShop, ShopDrawings, ShopSummary, plan_shop};
use super::visible::{VisibleLine, visible_lines};
use super::{Annotation, AnnotationCounts};
use super::{
    DrawingSolid, DrawnView, ProjectDrawingError, ProjectView, SheetViews, Storey, VIEW_PAD_MM,
    ViewSummary, annotation_bounds, bounds, choose_layout_scale, draw_view, draw_view_on_page,
    extent, format_at_scale, solid_edges, view_frames,
};
use crate::sheet_dxf::page_dxf;
use crate::sheet_pdf::{Page, PdfInfo, pages_pdf};
use crate::title_block::{
    DEFAULT_COARSEST_SCALE, SheetFormat, TitleBlock, TitleField, draw_frame_and_title_block,
    drawing_area,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The plan of one storey and the texts it is titled with.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanSheet {
    pub storey: Storey,
    /// The title under the plan view.
    pub view_title: String,
    /// What the sheet shows, in its title block.
    pub sheet_title: String,
}

/// What the sheets of sections and of elevations show, in their title blocks.
#[derive(Clone, Debug, Default)]
pub struct SheetTitles {
    pub sections: String,
    pub elevations: String,
}

/// What the caller decides: the plans and titles, the format, the scale and
/// the title block.
#[derive(Clone, Debug, Default)]
pub struct DrawingSetOptions {
    /// One plan sheet per storey, lowest first ([`super::storeys`] finds them).
    pub plans: Vec<PlanSheet>,
    pub sheet_titles: SheetTitles,
    /// The titles under the sections and elevations.
    pub view_titles: BTreeMap<ProjectView, String>,
    /// The format of every sheet; when absent each takes the smallest that
    /// holds it at the set's scale.
    pub format: Option<SheetFormat>,
    /// The coarsest scale 1:N the automatic formats may use;
    /// [`DEFAULT_COARSEST_SCALE`] when absent.
    pub coarsest_scale: Option<u32>,
    /// Captions of the title block cells.
    pub title_labels: BTreeMap<TitleField, String>,
    /// The title block values. A drawing title, when given, names the set
    /// before each sheet's own; scale, format and the sheet number are filled
    /// in by each sheet.
    pub title_block: TitleBlock,
    /// Only the plans, x to the right and y up.
    pub plan_only: bool,
    /// The scale 1:N every view is drawn at; when absent the set picks it.
    pub scale: Option<u32>,
    /// The annotations; each plan draws those of its storey (from its floor
    /// up to the next storey's floor; the lowest plan also those below it).
    pub annotations: Vec<Annotation>,
    /// The windows and doors: each plan marks those of its storey (parted
    /// like the annotations, by their lowest point), and their schedule is
    /// the last sheet.
    pub openings: OpeningSchedule,
    /// A sheet per shop drawing, after the elevations, each at its own scale.
    pub shop: ShopDrawings,
}

/// What a sheet of the set shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SheetKind {
    /// The plan of storey `storey` (0 the lowest).
    Plan {
        storey: usize,
    },
    Sections,
    Elevations,
    /// Shop drawing `drawing` (an index into [`ShopDrawings::sheets`]).
    Shop {
        drawing: usize,
    },
    /// The schedule of windows and doors.
    Openings,
}

/// One sheet of the set.
#[derive(Clone, Debug, PartialEq)]
pub struct SetSheet {
    pub kind: SheetKind,
    /// Its number in the set, from 1.
    pub number: usize,
    /// What it shows, as its title block says.
    pub title: String,
    pub format: SheetFormat,
    /// The painted sheet, as the PDF shows it.
    pub page: Page,
    /// The sheet with its views as visible lines alone, as its DXF carries it.
    pub lines: Page,
    pub views: Vec<ViewSummary>,
    /// The annotations its plan draws (none on the other sheets).
    pub annotations: AnnotationCounts,
    /// The marks of the windows and doors its plan draws, one per opening.
    pub openings: Vec<String>,
    /// What a shop drawing shows.
    pub shop: Option<ShopSummary>,
}

/// The sheets, numbered in order, and the scale 1:`scale` of every view.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingSet {
    pub sheets: Vec<SetSheet>,
    pub scale: u32,
}

impl DrawingSet {
    /// The set as one vector PDF, a page per sheet, the typeface embedded once.
    #[must_use]
    pub fn pdf(&self, info: &PdfInfo) -> Vec<u8> {
        let pages = self
            .sheets
            .iter()
            .map(|sheet| &sheet.page)
            .collect::<Vec<_>>();
        pages_pdf(&pages, info)
    }

    /// Each sheet as an R12 DXF of its visible lines, in paper millimetres.
    #[must_use]
    pub fn dxf(&self) -> Vec<Vec<u8>> {
        self.sheets
            .iter()
            .map(|sheet| page_dxf(&sheet.lines))
            .collect()
    }

    /// The annotations all plans draw.
    #[must_use]
    pub fn annotations(&self) -> AnnotationCounts {
        let mut total = AnnotationCounts::default();
        for counts in self.sheets.iter().map(|sheet| &sheet.annotations) {
            total.texts += counts.texts;
            total.dimensions += counts.dimensions;
            total.leaders += counts.leaders;
            total.lines += counts.lines;
            total.outside_plan += counts.outside_plan;
        }
        total
    }

    /// The characters of every sheet's texts the typeface lacks, each once.
    #[must_use]
    pub fn printed_as_question_mark(&self) -> Vec<char> {
        self.sheets
            .iter()
            .flat_map(|sheet| sheet.page.printed_as_question_mark())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

/// A sheet before it is laid out: its views with their titles.
struct Planned {
    kind: SheetKind,
    title: String,
    views: SheetViews,
    titles: Vec<String>,
}

/// The views of a sheet that draw anything, in rows of `per_row`.
fn sheet_views(drawn: Vec<(DrawnView, String)>, per_row: usize) -> (SheetViews, Vec<String>) {
    let (views, titles): (Vec<_>, Vec<_>) = drawn
        .into_iter()
        .filter(|(view, _)| view.min[0].is_finite())
        .unzip();
    let rows = (0..views.len())
        .collect::<Vec<_>>()
        .chunks(per_row)
        .map(<[usize]>::to_vec)
        .collect();
    (SheetViews { views, rows }, titles)
}

/// The lowest point of `annotation`, along z.
fn lowest(annotation: &Annotation) -> f64 {
    annotation
        .points()
        .into_iter()
        .map(|point| point[2])
        .fold(f64::INFINITY, f64::min)
}

/// Lays the views of `sheet` out on `page` at 1:`scale`, row under row, each
/// left to right; with `visible`, as visible lines.
fn draw_views(
    page: &mut Page,
    sheet: &Planned,
    format: SheetFormat,
    scale: u32,
    visible: Option<&[Vec<VisibleLine>]>,
) {
    let scale = f64::from(scale);
    let views = &sheet.views.views;
    let [[area_left, mut top], _] = drawing_area(format);
    for row in &sheet.views.rows {
        let row_height = row
            .iter()
            .map(|index| extent(&views[*index])[1] / scale + 2.0 * VIEW_PAD_MM)
            .fold(0.0, f64::max);
        let mut left = area_left;
        for &index in row {
            let view = &views[index];
            let [width, height] = extent(view);
            let origin = [left + VIEW_PAD_MM, top + VIEW_PAD_MM / 2.0 + height / scale];
            draw_view_on_page(
                page,
                view,
                &sheet.titles[index],
                origin,
                scale,
                visible.map(|lines| lines[index].as_slice()),
            );
            left += width / scale + 2.0 * VIEW_PAD_MM;
        }
        top += row_height;
    }
}

/// Draws the project drawings of `solids` as a set of sheets.
///
/// # Errors
/// [`ProjectDrawingError::Empty`] when there is no triangle and no annotation
/// to draw, or no plan is asked for; [`ProjectDrawingError::DoesNotFit`] when
/// a sheet does not fit its format at the scale the options fix;
/// [`ProjectDrawingError::TitleValueDoesNotFit`] when a title block value
/// does not fit its cell.
pub fn drawing_set(
    solids: &[DrawingSolid],
    options: &DrawingSetOptions,
) -> Result<DrawingSet, ProjectDrawingError> {
    let annotated = annotation_bounds(&options.annotations);
    let (min, max) = match (bounds(solids), annotated) {
        (Some((min, max)), Some((low, high))) => (
            std::array::from_fn(|axis| min[axis].min(low[axis])),
            std::array::from_fn(|axis| max[axis].max(high[axis])),
        ),
        (Some(bounds), None) | (None, Some(bounds)) => bounds,
        (None, None) => return Err(ProjectDrawingError::Empty),
    };
    let first_cut = options
        .plans
        .first()
        .ok_or(ProjectDrawingError::Empty)?
        .storey
        .cut_mm;
    let edges = solids.iter().map(solid_edges).collect::<Vec<_>>();
    let title_of = |view: ProjectView| {
        options
            .view_titles
            .get(&view)
            .cloned()
            .unwrap_or_else(|| format!("{view:?}"))
    };

    let opening_points = opening_points(&options.openings.openings, solids);
    let mut planned = Vec::new();
    for (index, plan) in options.plans.iter().enumerate() {
        let (view, frame) = view_frames(min, max, plan.storey.cut_mm, options.plan_only)[0];
        let mut drawn = draw_view(view, frame, solids, &edges);
        // A plan draws the annotations from its floor up to the next plan's;
        // without a floor, two plans part at the lower one's cut (review
        // 2026-10-24 MF-29: they parted nowhere, so each drew them all).
        let parting = |lower: &PlanSheet, upper: &PlanSheet| {
            upper.storey.floor_mm.unwrap_or(lower.storey.cut_mm)
        };
        let below = index.checked_sub(1).map_or(f64::NEG_INFINITY, |lower| {
            parting(&options.plans[lower], plan)
        });
        let above = options
            .plans
            .get(index + 1)
            .map_or(f64::INFINITY, |upper| parting(plan, upper));
        let annotations = options
            .annotations
            .iter()
            .filter(|annotation| (below..above).contains(&lowest(annotation)))
            .cloned()
            .collect::<Vec<_>>();
        if !annotations.is_empty() {
            drawn.annotate(&frame, &annotations);
        }
        let marks = opening_points
            .iter()
            .filter(|(_, point)| (below..above).contains(&point[2]))
            .cloned()
            .collect::<Vec<_>>();
        drawn.mark_openings(&frame, &marks);
        let (views, titles) = sheet_views(vec![(drawn, plan.view_title.clone())], 1);
        if !views.views.is_empty() {
            planned.push(Planned {
                kind: SheetKind::Plan { storey: index },
                title: plan.sheet_title.clone(),
                views,
                titles,
            });
        }
    }
    if !options.plan_only {
        let frames = view_frames(min, max, first_cut, false);
        let drawn = |wanted: &[ProjectView]| {
            frames
                .iter()
                .filter(|(view, _)| wanted.contains(view))
                .map(|(view, frame)| (draw_view(*view, *frame, solids, &edges), title_of(*view)))
                .collect::<Vec<_>>()
        };
        let named = |title: &str, fallback: &str| {
            if title.trim().is_empty() {
                fallback.to_owned()
            } else {
                title.to_owned()
            }
        };
        for (kind, views, per_row, title) in [
            (
                SheetKind::Sections,
                drawn(&[ProjectView::LongitudinalSection, ProjectView::CrossSection]),
                2,
                named(&options.sheet_titles.sections, "Sections"),
            ),
            (
                SheetKind::Elevations,
                drawn(&[
                    ProjectView::Front,
                    ProjectView::Back,
                    ProjectView::Left,
                    ProjectView::Right,
                ]),
                2,
                named(&options.sheet_titles.elevations, "Elevations"),
            ),
        ] {
            let (views, titles) = sheet_views(views, per_row);
            if !views.views.is_empty() {
                planned.push(Planned {
                    kind,
                    title,
                    views,
                    titles,
                });
            }
        }
    }
    if planned.is_empty() {
        return Err(ProjectDrawingError::Empty);
    }

    let scale = match options.scale {
        Some(scale) => scale,
        None => {
            let preferred = options.coarsest_scale.unwrap_or(DEFAULT_COARSEST_SCALE);
            planned
                .iter()
                .map(|sheet| choose_layout_scale(&sheet.views, options.format, preferred))
                .max()
                .unwrap_or(preferred)
        }
    };
    let formats = planned
        .iter()
        .map(|sheet| format_at_scale(&sheet.views, options.format, scale))
        .collect::<Result<Vec<_>, _>>()?;

    // The schedule of windows and doors goes with the whole set, not the plans alone.
    let schedule = options.openings.rows();
    let schedule_format = if options.plan_only || schedule.is_empty() {
        None
    } else {
        Some(
            schedule_format(&options.openings.columns, &schedule, options.format).ok_or(
                ProjectDrawingError::DoesNotFit {
                    scale,
                    format: options.format,
                },
            )?,
        )
    };

    let shops = if options.plan_only {
        Vec::new()
    } else {
        options
            .shop
            .sheets
            .iter()
            .enumerate()
            .filter_map(|(drawing, sheet)| {
                plan_shop(sheet, solids, &options.shop.columns, options.format)
                    .transpose()
                    .map(|planned| planned.map(|planned| (drawing, planned)))
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    let count = planned.len() + shops.len() + usize::from(schedule_format.is_some());
    let set_title = options
        .title_block
        .get(&TitleField::Drawing)
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty());
    let title_values = |title: &str, scale: String, format: SheetFormat, number: usize| {
        let mut values = options.title_block.clone();
        values.insert(
            TitleField::Drawing,
            set_title
                .as_ref()
                .map_or_else(|| title.to_owned(), |set| format!("{set} – {title}")),
        );
        values.insert(TitleField::Scale, scale);
        values.insert(TitleField::Format, format.name().to_owned());
        values.insert(TitleField::Sheet, format!("{number} / {count}"));
        values
    };
    let mut sheets = Vec::with_capacity(count);
    for (index, (sheet, format)) in planned.into_iter().zip(formats).enumerate() {
        let number = index + 1;
        let values = title_values(&sheet.title, format!("1:{scale}"), format, number);
        let blank = || Page {
            size_mm: format.size_mm(),
            marks: Vec::new(),
        };
        let mut page = blank();
        draw_views(&mut page, &sheet, format, scale, None);
        draw_frame_and_title_block(&mut page, format, &options.title_labels, &values)?;
        let visible = sheet
            .views
            .views
            .iter()
            .map(visible_lines)
            .collect::<Vec<_>>();
        let mut lines = blank();
        draw_views(&mut lines, &sheet, format, scale, Some(&visible));
        draw_frame_and_title_block(&mut lines, format, &options.title_labels, &values)?;
        let views = sheet
            .views
            .views
            .iter()
            .zip(&sheet.titles)
            .map(|(view, title)| ViewSummary {
                view: view.view,
                title: title.clone(),
                cut_solids: view.cuts.len(),
                cut_at_mm: view.cut_at_mm,
                extent_mm: extent(view).map(|value| (value * 10.0).round() / 10.0),
            })
            .collect();
        let annotations = sheet
            .views
            .views
            .iter()
            .find_map(|view| view.annotations.as_ref())
            .map(|annotations| annotations.counts.clone())
            .unwrap_or_default();
        let openings = sheet
            .views
            .views
            .iter()
            .flat_map(|view| view.marks.iter().map(|(_, mark)| mark.clone()))
            .collect();
        sheets.push(SetSheet {
            kind: sheet.kind,
            number,
            title: sheet.title,
            format,
            page,
            lines,
            views,
            annotations,
            openings,
            shop: None,
        });
    }
    for (drawing, shop) in shops {
        let number = sheets.len() + 1;
        let values = title_values(
            &shop.title,
            format!("1:{}", shop.scale),
            shop.format,
            number,
        );
        sheets.push(shop_sheet(
            drawing,
            shop,
            number,
            &options.shop.columns,
            &options.title_labels,
            &values,
        )?);
    }
    if let Some(format) = schedule_format {
        let title = if options.openings.title.trim().is_empty() {
            "Windows and doors".to_owned()
        } else {
            options.openings.title.clone()
        };
        // A table has no scale.
        let values = title_values(&title, "–".to_owned(), format, count);
        let mut page = Page {
            size_mm: format.size_mm(),
            marks: Vec::new(),
        };
        draw_schedule(&mut page, format, &options.openings.columns, &schedule);
        draw_frame_and_title_block(&mut page, format, &options.title_labels, &values)?;
        sheets.push(SetSheet {
            kind: SheetKind::Openings,
            number: count,
            title,
            format,
            lines: page.clone(),
            page,
            views: Vec::new(),
            annotations: AnnotationCounts::default(),
            openings: Vec::new(),
            shop: None,
        });
    }
    Ok(DrawingSet { sheets, scale })
}

/// Shop drawing `drawing` as sheet `number`, its title block filled with `values`.
fn shop_sheet(
    drawing: usize,
    shop: PlannedShop,
    number: usize,
    columns: &[String; 4],
    labels: &BTreeMap<TitleField, String>,
    values: &TitleBlock,
) -> Result<SetSheet, ProjectDrawingError> {
    let blank = || Page {
        size_mm: shop.format.size_mm(),
        marks: Vec::new(),
    };
    let mut page = blank();
    shop.draw(&mut page, columns, None);
    draw_frame_and_title_block(&mut page, shop.format, labels, values)?;
    let visible = visible_lines(&shop.view);
    let mut lines = blank();
    shop.draw(&mut lines, columns, Some(&visible));
    draw_frame_and_title_block(&mut lines, shop.format, labels, values)?;
    Ok(SetSheet {
        kind: SheetKind::Shop { drawing },
        number,
        views: vec![ViewSummary {
            view: ProjectView::Shop,
            title: shop.title.clone(),
            cut_solids: 0,
            cut_at_mm: None,
            extent_mm: extent(&shop.view).map(|value| (value * 10.0).round() / 10.0),
        }],
        title: shop.title,
        format: shop.format,
        page,
        lines,
        annotations: AnnotationCounts::default(),
        openings: Vec::new(),
        shop: Some(shop.summary),
    })
}
