//! A drawing sheet as an AutoCAD drawing a CAD user keeps working on: the views in model
//! space at 1:1 in millimetres, and the sheet itself as a paper-space layout of its paper
//! size, its frame and title block drawn on paper and a viewport at the sheet's scale on each
//! view. Written as DXF 2013 ([`SHEET_DXF_VERSION`], UTF-8, so every Central European letter
//! stays a letter) or as DWG of [`crate::WRITTEN_VERSION`], both from the same drawing.
//!
//! The caller describes the drawing in plain data ([`Sheet`]); no acadrust type crosses this
//! API. Coordinates are millimetres with y up, as AutoCAD reads them.

use acadrust::entities::{
    BoundaryEdge, BoundaryPath, BoundaryPathFlags, Hatch, PolylineEdge, TextHorizontalAlignment,
    TextVerticalAlignment, Viewport, ViewportStatusFlags,
};
use acadrust::objects::ObjectType;
use acadrust::tables::LineTypeElement;
use acadrust::{
    CadDocument, Color, DwgWriter, DxfVersion, DxfWriter, EntityType, Layer, LineType, LineWeight,
    LwPolyline, Text, Vector2, Vector3,
};

use crate::{DwgError, Stage, guarded};

/// The DXF version a sheet is written as: AutoCAD 2013.
pub const SHEET_DXF_VERSION: &str = "AC1027";

/// How a layer's lines are drawn.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LinePattern {
    Continuous,
    /// Hidden edges and what lies above the cut.
    Dashed,
    /// Axes and cutting planes.
    DashDot,
}

impl LinePattern {
    /// The linetype's name in the drawing, as AutoCAD's own `acadiso.lin` names it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Continuous => "Continuous",
            Self::Dashed => "DASHED",
            Self::DashDot => "CENTER",
        }
    }
}

/// A layer: what it holds, how it is drawn and printed.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetLayer {
    pub name: String,
    /// The AutoCAD colour index, 1 to 255 (7 draws black on paper, white on a dark screen).
    pub color: u8,
    /// The printed line width in hundredths of a millimetre, as AutoCAD's lineweights.
    pub lineweight: i16,
    pub pattern: LinePattern,
}

/// Where a text sits on its insertion point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAlign {
    /// The start of its baseline.
    Start,
    /// The middle of its baseline.
    Middle,
    /// The end of its baseline.
    End,
}

/// What a sheet draws, in model space or on paper.
#[derive(Clone, Debug, PartialEq)]
pub enum SheetEntity {
    /// A polyline of straight sides; `pen` overrides its layer's.
    Polyline {
        layer: String,
        points: Vec<[f64; 2]>,
        closed: bool,
        pen: Pen,
    },
    /// One line of text; `height` is the height of its capital letters.
    Text {
        layer: String,
        at: [f64; 2],
        value: String,
        height: f64,
        angle_deg: f64,
        align: TextAlign,
    },
    /// A solid fill of closed loops, a loop inside another a hole; `color` overrides its
    /// layer's.
    Fill {
        layer: String,
        loops: Vec<Vec<[f64; 2]>>,
        color: Option<[u8; 3]>,
    },
}

/// What an entity draws with in place of its layer's colour, lineweight and linetype;
/// `None` takes the layer's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pen {
    pub color: Option<[u8; 3]>,
    /// Hundredths of a millimetre, one of AutoCAD's lineweights.
    pub lineweight: Option<i16>,
    pub pattern: Option<LinePattern>,
}

/// AutoCAD's lineweights in hundredths of a millimetre, the only ones a drawing may hold.
pub const LINEWEIGHTS: [i16; 24] = [
    0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200,
    211,
];

/// The lineweight of [`LINEWEIGHTS`] nearest `width_mm`.
#[must_use]
pub fn lineweight(width_mm: f64) -> i16 {
    let hundredths = width_mm * 100.0;
    LINEWEIGHTS
        .into_iter()
        .min_by(|a, b| {
            (f64::from(*a) - hundredths)
                .abs()
                .total_cmp(&(f64::from(*b) - hundredths).abs())
        })
        .unwrap_or(0)
}

/// A viewport of the layout: the window `paper_min`..`paper_max` on paper shows model space
/// at 1:`scale`, turned by `twist_deg` counter-clockwise, `view_center` in the middle;
/// `view_center` is in model space turned so (AutoCAD's display coordinates), which are model
/// space itself when `twist_deg` is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetViewport {
    pub paper_min: [f64; 2],
    pub paper_max: [f64; 2],
    pub view_center: [f64; 2],
    pub twist_deg: f64,
    pub scale: f64,
}

/// One drawing sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    /// The layout's name, as its tab shows it.
    pub layout: String,
    /// The paper's width and height in millimetres.
    pub paper_mm: [f64; 2],
    /// Every layer an entity names, each once.
    pub layers: Vec<SheetLayer>,
    /// The views at 1:1.
    pub model: Vec<SheetEntity>,
    /// What lies on paper: frame, title block, view titles.
    pub paper: Vec<SheetEntity>,
    pub viewports: Vec<SheetViewport>,
}

/// `sheet` as a DXF 2013 drawing.
///
/// # Errors
/// [`DwgError::Library`] or [`DwgError::Panicked`] when acadrust refuses or panics while the
/// drawing is built ([`Stage::BuildSheet`]) or written ([`Stage::WriteSheetDxf`]).
pub fn sheet_dxf(sheet: &Sheet) -> Result<Vec<u8>, DwgError> {
    let mut document = sheet_document(sheet)?;
    document.version = DxfVersion::AC1027;
    guarded(Stage::WriteSheetDxf, || {
        let mut out = Vec::new();
        DxfWriter::new(&document).write_to_writer(&mut out)?;
        Ok(out)
    })
}

/// `sheet` as a DWG drawing of [`crate::WRITTEN_VERSION`].
///
/// # Errors
/// As [`sheet_dxf`], [`Stage::WriteDwg`] while the DWG is written.
pub fn sheet_dwg(sheet: &Sheet) -> Result<Vec<u8>, DwgError> {
    let mut document = sheet_document(sheet)?;
    document.version = DxfVersion::AC1032;
    guarded(Stage::WriteDwg, || DwgWriter::write_to_vec(&document))
}

fn sheet_document(sheet: &Sheet) -> Result<CadDocument, DwgError> {
    guarded(Stage::BuildSheet, || {
        let mut document = CadDocument::new();
        document.header.insertion_units = 4;
        add_line_types(&mut document);
        for layer in &sheet.layers {
            let mut entry = Layer::new(layer.name.clone());
            entry.color = Color::from_index(i16::from(layer.color));
            entry.line_weight = LineWeight::Value(layer.lineweight);
            entry.line_type = layer.pattern.name().to_owned();
            document.layers.add_or_replace(entry);
        }
        // Entries added here have no handle yet; a record of handle 0 is not a record.
        document.assign_table_entry_handles();
        let layout = name_the_paper_layout(&mut document, sheet)?;
        for entity in &sheet.model {
            document.add_entity(entity_type(entity))?;
        }
        for entity in &sheet.paper {
            document.add_entity_to_layout(entity_type(entity), &layout)?;
        }
        let whole = paper_viewport(sheet.paper_mm);
        let whole = document.add_entity_to_layout(whole, &layout)?;
        for object in document.objects.values_mut() {
            if let ObjectType::Layout(entry) = object
                && entry.name == layout
            {
                entry.viewport = whole;
            }
        }
        for (index, view) in sheet.viewports.iter().enumerate() {
            document.add_entity_to_layout(viewport(view, index), &layout)?;
        }
        Ok(document)
    })
}

/// The linetypes of [`LinePattern`] with dashes in paper millimetres, as `acadiso.lin` has
/// them; `$PSLTSCALE` keeps them so in every viewport whatever its scale.
fn add_line_types(document: &mut CadDocument) {
    let mut dashed = LineType::new(LinePattern::Dashed.name());
    dashed.description = "Dashed __ __ __".to_owned();
    for element in [LineTypeElement::dash(12.7), LineTypeElement::space(6.35)] {
        dashed.add_element(element);
    }
    let mut center = LineType::new(LinePattern::DashDot.name());
    center.description = "Center ____ _ ____ _".to_owned();
    for element in [
        LineTypeElement::dash(31.75),
        LineTypeElement::space(6.35),
        LineTypeElement::dash(6.35),
        LineTypeElement::space(6.35),
    ] {
        center.add_element(element);
    }
    document.line_types.add_or_replace(dashed);
    document.line_types.add_or_replace(center);
    document.header.linetype_scale = 1.0;
    document.header.paper_space_linetype_scaling = true;
}

/// Names the document's one paper layout after the sheet and sizes it to the paper; returns
/// the name.
fn name_the_paper_layout(
    document: &mut CadDocument,
    sheet: &Sheet,
) -> Result<String, acadrust::DxfError> {
    let [width, height] = sheet.paper_mm;
    let mut renamed = None;
    for object in document.objects.values_mut() {
        if let ObjectType::Layout(layout) = object
            && layout.name == "Layout1"
        {
            layout.name.clone_from(&sheet.layout);
            layout.paper_width = width;
            layout.paper_height = height;
            layout.plot_paper_units = 1;
            layout.min_limits = (0.0, 0.0);
            layout.max_limits = (width, height);
            layout.min_extents = (0.0, 0.0, 0.0);
            layout.max_extents = (width, height, 0.0);
            renamed = Some(layout.handle);
        }
    }
    let handle = renamed.ok_or_else(|| {
        acadrust::DxfError::Custom("a new drawing has no paper layout Layout1".to_owned())
    })?;
    if let Some(ObjectType::Dictionary(layouts)) = document
        .objects
        .get_mut(&document.header.acad_layout_dict_handle)
    {
        for (name, entry) in &mut layouts.entries {
            if *entry == handle {
                name.clone_from(&sheet.layout);
            }
        }
    }
    Ok(sheet.layout.clone())
}

fn point(&[x, y]: &[f64; 2]) -> Vector2 {
    Vector2::new(x, y)
}

fn entity_type(entity: &SheetEntity) -> EntityType {
    match entity {
        SheetEntity::Polyline {
            layer,
            points,
            closed,
            pen,
        } => {
            let mut polyline = LwPolyline::from_points(points.iter().map(point).collect());
            if *closed {
                polyline.close();
            }
            polyline.common.layer.clone_from(layer);
            if let Some([r, g, b]) = pen.color {
                polyline.common.color = Color::Rgb { r, g, b };
            }
            if let Some(lineweight) = pen.lineweight {
                polyline.common.line_weight = LineWeight::Value(lineweight);
            }
            if let Some(pattern) = pen.pattern {
                pattern.name().clone_into(&mut polyline.common.linetype);
            }
            EntityType::LwPolyline(polyline)
        }
        SheetEntity::Text {
            layer,
            at,
            value,
            height,
            angle_deg,
            align,
        } => {
            let at = Vector3::new(at[0], at[1], 0.0);
            // "%%" starts a control code in a TEXT ("%%d" a degree sign); "%%%" is one "%".
            let mut text = Text::with_value(value.replace('%', "%%%"), at)
                .with_height(*height)
                .with_rotation(angle_deg.to_radians());
            text.horizontal_alignment = match align {
                TextAlign::Start => TextHorizontalAlignment::Left,
                TextAlign::Middle => TextHorizontalAlignment::Center,
                TextAlign::End => TextHorizontalAlignment::Right,
            };
            text.vertical_alignment = TextVerticalAlignment::Baseline;
            if *align != TextAlign::Start {
                text.alignment_point = Some(at);
            }
            text.common.layer.clone_from(layer);
            EntityType::Text(text)
        }
        SheetEntity::Fill {
            layer,
            loops,
            color,
        } => {
            let mut hatch = Hatch::solid();
            for points in loops {
                let mut flags = BoundaryPathFlags::new();
                flags.set_external(true);
                flags.set_polyline(true);
                let mut path = BoundaryPath::with_flags(flags);
                path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
                    points.iter().map(point).collect(),
                    true,
                )));
                hatch.add_path(path);
            }
            hatch.common.layer.clone_from(layer);
            if let Some([r, g, b]) = *color {
                hatch.common.color = Color::Rgb { r, g, b };
            }
            EntityType::Hatch(Box::new(hatch))
        }
    }
}

/// The layout's own viewport (id 1), which AutoCAD shows the paper through.
fn paper_viewport([width, height]: [f64; 2]) -> EntityType {
    let centre = Vector3::new(width / 2.0, height / 2.0, 0.0);
    let mut viewport = Viewport::with_size(centre, width, height);
    viewport.id = 1;
    viewport.status = ViewportStatusFlags::default_on();
    viewport.view_center = centre;
    viewport.set_view_height(height);
    EntityType::Viewport(Box::new(viewport))
}

fn viewport(view: &SheetViewport, index: usize) -> EntityType {
    let [left, bottom] = view.paper_min;
    let [right, top] = view.paper_max;
    let mut viewport = Viewport::with_size(
        Vector3::new(f64::midpoint(left, right), f64::midpoint(bottom, top), 0.0),
        right - left,
        top - bottom,
    );
    // The layout's own viewport is 1; the views' follow it.
    viewport.id = i16::try_from(index + 2).unwrap_or(i16::MAX);
    viewport.view_center = Vector3::new(view.view_center[0], view.view_center[1], 0.0);
    viewport.twist_angle = view.twist_deg.to_radians();
    viewport.set_scale(1.0 / view.scale);
    viewport.lock();
    EntityType::Viewport(Box::new(viewport))
}

#[cfg(test)]
mod tests;
