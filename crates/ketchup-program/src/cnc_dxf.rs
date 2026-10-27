//! CNC DXF of the made flat parts: one AutoCAD R12 file per part, seen on its
//! face, with the outline and the machining on the layers a CNC program maps
//! to its tools.
//!
//! A part is drawn when it is in the production scope (`production(tags=…)`),
//! made in the shop (fabrication role manufactured item) and an extrusion: a
//! box is seen along its thinnest side, any other profile along its
//! extrusion. The drawing lies in the first quadrant from (0, 0), the face the
//! part is seen from up. Layers, the numbers in millimetres with a decimal
//! point written `_` (R12 takes no `.` in a layer name):
//!
//! - `CONTOUR` the outline, `CUTOUT` a cut through the whole part;
//! - `DRILL_D8_Z12` a hole 8 across and 12 deep drilled from the face seen,
//!   a through hole as deep as the part; `DRILL_D8_Z12_BOTTOM` from the face
//!   below;
//! - `HDRILL_D8_Z30_H9` a hole drilled into an edge, a line from where it
//!   enters to its bottom, its axis 9 above the face below (the drawing is
//!   flat: a reader drops a line's height);
//! - `POCKET_Z5` a pocket or a cut 5 deep from the face seen,
//!   `POCKET_Z5_BOTTOM` from the face below;
//! - `PART_NAME` the part's name as text, its accents as `\U+XXXX`.
//!
//! The file has no blocks, no handles and only ASCII. A part whose shape a
//! drawing on its face cannot carry (a boolean, a mirror, a slanted hole, a
//! pocket in an edge) is listed with the reason instead of drawn wrong.

use crate::model::{
    Hole, Part, Pocket, ProgramModel, ProgramOperation, ProgramPartBody, ProgramProfileSegment,
    profile_bounds,
};
use crate::table::{Cell, DecimalSeparator, Table, stored_zip};
use ketchup_model::drawing_export::layers::R12Layers;
use ketchup_model::r12_text;
use ketchup_tolerance::{APPROXIMATION, CNC_CURVE_CHORD_MM};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

const ROLE: &str = "classification:ketchup.fabrication-role.v1";
const MADE_IN_SHOP: &str = "fabrication.manufactured-item.v1";
/// The most lines one Bezier curve is drawn with.
const MAX_CURVE_LINES: u32 = 1000;
/// Names Windows keeps for devices; a file named so cannot be written there.
const DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// One drawn entity, in mm in the part's drawing.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "entity", rename_all = "snake_case")]
pub enum Entity {
    /// A closed outline: each vertex with the bulge of the segment that starts
    /// there (0 straight, tan(sweep / 4) an arc, negative clockwise).
    Outline {
        layer: String,
        vertices: Vec<([f64; 2], f64)>,
    },
    Circle {
        layer: String,
        center_mm: [f64; 2],
        radius_mm: f64,
    },
    /// A hole into an edge, from where it enters to its bottom.
    Line {
        layer: String,
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
}

impl Entity {
    #[must_use]
    pub fn layer(&self) -> &str {
        match self {
            Self::Outline { layer, .. } | Self::Circle { layer, .. } | Self::Line { layer, .. } => {
                layer
            }
        }
    }
}

/// The drawing of one part.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CncPart {
    pub part: String,
    /// `<name>.dxf`: the name in ASCII, at most 31 characters, unique in the
    /// export without regard to case.
    pub file: String,
    pub thickness_mm: f64,
    /// The extent of the drawing along x and y.
    pub size_mm: [f64; 2],
    pub entities: Vec<Entity>,
}

/// Why a part in scope is not drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undrawable {
    NoRole,
    NotAnExtrusion,
    /// A cut goes down local z, which is not the part's thickness.
    CutAcross {
        cut: String,
    },
    SlantedHole {
        hole: String,
    },
    PocketNotInFace {
        pocket: String,
    },
    /// An operation that changes the shape beyond what the face shows.
    Reshaped {
        kind: &'static str,
        name: String,
    },
}

impl fmt::Display for Undrawable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRole => formatter.write_str("it has no fabrication role"),
            Self::NotAnExtrusion => {
                formatter.write_str("its body is not an extrusion, so it has no flat face to draw")
            }
            Self::CutAcross { cut } => write!(
                formatter,
                "cut {cut:?} goes down its local z, which is not its thickness"
            ),
            Self::SlantedHole { hole } => {
                write!(
                    formatter,
                    "hole {hole:?} is drilled at a slant to its faces"
                )
            }
            Self::PocketNotInFace { pocket } => write!(
                formatter,
                "pocket {pocket:?} is milled into an edge or at a slant, not into a face"
            ),
            Self::Reshaped { kind, name } => write!(
                formatter,
                "its {kind} {name:?} changes its shape in a way a drawing on its face does not show"
            ),
        }
    }
}

impl Serialize for Undrawable {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A part in scope that is not drawn, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotDrawn {
    pub part: String,
    pub reason: Undrawable,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CncExport {
    pub parts: Vec<CncPart>,
    pub not_drawn: Vec<NotDrawn>,
}

/// The CNC drawings of the model's made flat parts in its production scope.
#[must_use]
pub fn cnc_parts(model: &ProgramModel) -> CncExport {
    let mut files = R12Layers::reserving(&DEVICE_NAMES);
    let mut export = CncExport::default();
    for part in model
        .parts
        .iter()
        .filter(|part| model.in_production(part) && part.made())
    {
        match part.attributes.get(ROLE).map(String::as_str) {
            Some(MADE_IN_SHOP) => {}
            Some(_) => continue,
            None => {
                export.not_drawn.push(NotDrawn {
                    part: part.name.clone(),
                    reason: Undrawable::NoRole,
                });
                continue;
            }
        }
        match drawing(part) {
            Ok((thickness_mm, size_mm, entities)) => export.parts.push(CncPart {
                part: part.name.clone(),
                file: format!("{}.dxf", files.layer(&part.name)),
                thickness_mm,
                size_mm,
                entities,
            }),
            Err(reason) => export.not_drawn.push(NotDrawn {
                part: part.name.clone(),
                reason,
            }),
        }
    }
    export
}

/// How a part is seen: along local axis `normal`, with local axes `across`
/// as the drawing's x and y, from the corner `min` of its blank.
struct View {
    normal: usize,
    across: [usize; 2],
    min: [f64; 3],
    thickness: f64,
}

impl View {
    fn along(normal: usize, min: [f64; 3], max: [f64; 3]) -> Self {
        Self {
            normal,
            across: [(normal + 1) % 3, (normal + 2) % 3],
            min,
            thickness: max[normal] - min[normal],
        }
    }

    fn point(&self, local: [f64; 3]) -> [f64; 2] {
        self.across.map(|axis| local[axis] - self.min[axis])
    }

    fn height(&self, local: [f64; 3]) -> f64 {
        local[self.normal] - self.min[self.normal]
    }

    /// Which face a tool going `inward` enters.
    fn entered(&self, inward: [f64; 3]) -> Entered {
        let along = inward[self.normal];
        if along <= -1.0 + APPROXIMATION {
            Entered::Seen
        } else if along >= 1.0 - APPROXIMATION {
            Entered::Below
        } else if along.abs() <= APPROXIMATION {
            Entered::Edge
        } else {
            Entered::Slanted
        }
    }
}

/// The face a hole or pocket enters, as the drawing sees it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Entered {
    Seen,
    Below,
    Edge,
    Slanted,
}

type Drawing = (f64, [f64; 2], Vec<Entity>);

fn drawing(part: &Part) -> Result<Drawing, Undrawable> {
    let ProgramPartBody::Extrusion {
        segments,
        distance_mm,
        ..
    } = &part.body
    else {
        return Err(Undrawable::NotAnExtrusion);
    };
    let ([x0, y0], [x1, y1]) = profile_bounds(segments);
    let (min, max) = (
        [x0, y0, distance_mm.min(0.0)],
        [x1, y1, distance_mm.max(0.0)],
    );
    let rectangle = axis_aligned_rectangle(segments);
    // A box is seen on its largest face; a profile only along its extrusion.
    let normal = if rectangle {
        [2, 0, 1]
            .into_iter()
            .reduce(|best, axis| {
                if max[axis] - min[axis] < max[best] - min[best] {
                    axis
                } else {
                    best
                }
            })
            .expect("three axes")
    } else {
        2
    };
    let view = View::along(normal, min, max);
    let size = view.across.map(|axis| max[axis] - min[axis]);
    let mut entities = vec![Entity::Outline {
        layer: "CONTOUR".to_owned(),
        vertices: if rectangle {
            rectangle_vertices([[0.0, 0.0], size])
        } else {
            outline_vertices(segments, [x0, y0])
        },
    }];
    for operation in &part.operations {
        match operation {
            ProgramOperation::Hole(hole) => entities.push(hole_entity(&view, hole)?),
            ProgramOperation::Pocket(pocket) => entities.push(pocket_entity(&view, pocket)?),
            ProgramOperation::Cut(cut) => {
                if normal != 2 {
                    return Err(Undrawable::CutAcross {
                        cut: cut.name.clone(),
                    });
                }
                let layer = match cut.depth_mm {
                    None => "CUTOUT".to_owned(),
                    Some(depth) => format!("POCKET_Z{}", token(depth)),
                };
                entities.push(Entity::Outline {
                    layer,
                    vertices: outline_vertices(&cut.segments, [x0, y0]),
                });
            }
            other => {
                return Err(Undrawable::Reshaped {
                    kind: operation_kind(other),
                    name: other.name().to_owned(),
                });
            }
        }
    }
    Ok((view.thickness, size, entities))
}

fn operation_kind(operation: &ProgramOperation) -> &'static str {
    match operation {
        ProgramOperation::Cut(_) => "cut",
        ProgramOperation::Finish(_) => "edge finish",
        ProgramOperation::FaceOffset(_) => "face offset",
        ProgramOperation::Boolean(_) => "boolean",
        ProgramOperation::Mirror(_) => "mirror",
        ProgramOperation::Shell(_) => "shell",
        ProgramOperation::Hole(_) => "hole",
        ProgramOperation::Pocket(_) => "pocket",
    }
}

fn hole_entity(view: &View, hole: &Hole) -> Result<Entity, Undrawable> {
    let depth = if hole.through {
        view.thickness
    } else {
        hole.depth_mm
    };
    let diameter = token(hole.diameter_mm);
    match view.entered(hole.inward) {
        face @ (Entered::Seen | Entered::Below) => Ok(Entity::Circle {
            layer: format!(
                "DRILL_D{diameter}_Z{}{}",
                token(depth),
                if face == Entered::Below && !hole.through {
                    "_BOTTOM"
                } else {
                    ""
                }
            ),
            center_mm: view.point(hole.entry_mm),
            radius_mm: hole.diameter_mm / 2.0,
        }),
        Entered::Edge => {
            let end: [f64; 3] = std::array::from_fn(|i| hole.entry_mm[i] + hole.inward[i] * depth);
            Ok(Entity::Line {
                layer: format!(
                    "HDRILL_D{diameter}_Z{}_H{}",
                    token(depth),
                    token(view.height(hole.entry_mm))
                ),
                start_mm: view.point(hole.entry_mm),
                end_mm: view.point(end),
            })
        }
        Entered::Slanted => Err(Undrawable::SlantedHole {
            hole: hole.id.clone(),
        }),
    }
}

fn pocket_entity(view: &View, pocket: &Pocket) -> Result<Entity, Undrawable> {
    let below = match view.entered(pocket.inward) {
        Entered::Seen => false,
        Entered::Below => true,
        Entered::Edge | Entered::Slanted => {
            return Err(Undrawable::PocketNotInFace {
                pocket: pocket.id.clone(),
            });
        }
    };
    let [du, dv] = pocket.extent_mm();
    let corner = |a: f64, b: f64| {
        view.point(std::array::from_fn(|i| {
            pocket.corner_mm[i] + pocket.u[i] * a + pocket.v[i] * b
        }))
    };
    let mut points = vec![
        corner(0.0, 0.0),
        corner(du, 0.0),
        corner(du, dv),
        corner(0.0, dv),
    ];
    let doubled_area: f64 = (0..4)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % 4]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if doubled_area < 0.0 {
        points.reverse();
    }
    Ok(Entity::Outline {
        layer: format!(
            "POCKET_Z{}{}",
            token(pocket.depth_mm),
            if below { "_BOTTOM" } else { "" }
        ),
        vertices: points.into_iter().map(|point| (point, 0.0)).collect(),
    })
}

/// Whether a closed profile is a rectangle with sides along x and y.
fn axis_aligned_rectangle(segments: &[ProgramProfileSegment]) -> bool {
    segments.len() == 4
        && segments.iter().all(|segment| {
            segment.is_line()
                && (segment.start_mm[0] == segment.end_mm[0]
                    || segment.start_mm[1] == segment.end_mm[1])
        })
}

fn rectangle_vertices([[x0, y0], [x1, y1]]: [[f64; 2]; 2]) -> Vec<([f64; 2], f64)> {
    vec![
        ([x0, y0], 0.0),
        ([x1, y0], 0.0),
        ([x1, y1], 0.0),
        ([x0, y1], 0.0),
    ]
}

/// A closed profile as polyline vertices, moved by `-origin`: arcs as
/// bulges, a full circle as two halves, Bezier curves as lines.
fn outline_vertices(segments: &[ProgramProfileSegment], origin: [f64; 2]) -> Vec<([f64; 2], f64)> {
    let moved = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
    let mut vertices = Vec::new();
    for segment in segments {
        if let Some(arc) = segment.arc {
            let c = arc.center_mm;
            let angle = |p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
            let counter =
                (angle(segment.end_mm) - angle(segment.start_mm)).rem_euclid(std::f64::consts::TAU);
            let sweep = if arc.clockwise {
                (std::f64::consts::TAU - counter).rem_euclid(std::f64::consts::TAU)
            } else {
                counter
            };
            let sign = if arc.clockwise { -1.0 } else { 1.0 };
            if sweep <= f64::EPSILON {
                // A whole circle: two half arcs, a bulge of ±1 each.
                let opposite = [
                    2.0 * c[0] - segment.start_mm[0],
                    2.0 * c[1] - segment.start_mm[1],
                ];
                vertices.push((moved(segment.start_mm), sign));
                vertices.push((moved(opposite), sign));
            } else {
                vertices.push((moved(segment.start_mm), sign * (sweep / 4.0).tan()));
            }
        } else if let Some([c1, c2]) = segment.bezier {
            let second = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
                (a[0] - 2.0 * b[0] + c[0]).hypot(a[1] - 2.0 * b[1] + c[1])
            };
            let bend = second(segment.start_mm, c1, c2).max(second(c1, c2, segment.end_mm));
            let lines = (0.75 * bend / CNC_CURVE_CHORD_MM)
                .sqrt()
                .ceil()
                .clamp(1.0, f64::from(MAX_CURVE_LINES));
            for step in (0..MAX_CURVE_LINES)
                .map(f64::from)
                .take_while(|step| *step < lines)
            {
                let t = step / lines;
                vertices.push((moved(segment.bezier_point(t)), 0.0));
            }
        } else {
            vertices.push((moved(segment.start_mm), 0.0));
        }
    }
    vertices
}

/// A length in a layer name: to a hundredth of a mm, `.` written `_`.
fn token(mm: f64) -> String {
    number((mm * 100.0).round() / 100.0).replace('.', "_")
}

fn number(value: f64) -> String {
    let rounded = (value * 1.0e6).round() / 1.0e6;
    let mut text = format!("{:.6}", if rounded == 0.0 { 0.0 } else { rounded });
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

const fn layer_color(layer: &str) -> u8 {
    match layer.as_bytes() {
        [b'C', b'O', ..] => 7,
        [b'C', b'U', ..] => 6,
        [b'D', ..] => 1,
        [b'H', ..] => 5,
        [b'P', b'O', ..] => 3,
        _ => 8,
    }
}

impl CncPart {
    /// The layers the drawing uses, the name's included.
    #[must_use]
    pub fn layers(&self) -> BTreeSet<&str> {
        self.entities
            .iter()
            .map(Entity::layer)
            .chain(["PART_NAME"])
            .collect()
    }

    /// The drawing as an ASCII DXF R12 (AC1009) file.
    #[must_use]
    pub fn dxf(&self) -> Vec<u8> {
        let layers = self.layers();
        let [width, height] = self.size_mm;
        let mut out = format!(
            "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$INSUNITS\n70\n4\n\
             9\n$EXTMIN\n10\n0\n20\n0\n30\n0\n9\n$EXTMAX\n10\n{}\n20\n{}\n30\n{}\n0\nENDSEC\n\
             0\nSECTION\n2\nTABLES\n\
             0\nTABLE\n2\nLTYPE\n70\n1\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0\n0\nENDTAB\n\
             0\nTABLE\n2\nLAYER\n70\n{}\n",
            number(width),
            number(height),
            number(self.thickness_mm),
            layers.len()
        );
        for layer in &layers {
            let _ = write!(
                out,
                "0\nLAYER\n2\n{layer}\n70\n0\n62\n{}\n6\nCONTINUOUS\n",
                layer_color(layer)
            );
        }
        out.push_str(
            "0\nENDTAB\n0\nTABLE\n2\nSTYLE\n70\n1\n0\nSTYLE\n2\nSTANDARD\n70\n0\n40\n0\n41\n1\n50\n0\n71\n0\n42\n2.5\n3\ntxt\n4\n\n0\nENDTAB\n0\nENDSEC\n\
             0\nSECTION\n2\nENTITIES\n",
        );
        for entity in &self.entities {
            match entity {
                Entity::Outline { layer, vertices } => {
                    let _ = write!(
                        out,
                        "0\nPOLYLINE\n8\n{layer}\n66\n1\n10\n0\n20\n0\n30\n0\n70\n1\n"
                    );
                    for ([x, y], bulge) in vertices {
                        let _ = write!(
                            out,
                            "0\nVERTEX\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n",
                            number(*x),
                            number(*y)
                        );
                        if *bulge != 0.0 {
                            let _ = write!(out, "42\n{}\n", number(*bulge));
                        }
                    }
                    let _ = write!(out, "0\nSEQEND\n8\n{layer}\n");
                }
                Entity::Circle {
                    layer,
                    center_mm: [x, y],
                    radius_mm,
                } => {
                    let _ = write!(
                        out,
                        "0\nCIRCLE\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n40\n{}\n",
                        number(*x),
                        number(*y),
                        number(*radius_mm)
                    );
                }
                Entity::Line {
                    layer,
                    start_mm,
                    end_mm,
                } => {
                    let _ = write!(
                        out,
                        "0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
                        number(start_mm[0]),
                        number(start_mm[1]),
                        number(end_mm[0]),
                        number(end_mm[1])
                    );
                }
            }
        }
        // The name inside the outline's box, small enough to stay in it.
        let text_height = (width.min(height) / 10.0).clamp(0.5, 20.0);
        for (line, text) in (0_u32..)
            .map(f64::from)
            .zip(r12_text::lines(&self.part, f64::INFINITY).iter())
        {
            let y = height - text_height * (1.5 * line + 2.0);
            let _ = write!(
                out,
                "0\nTEXT\n8\nPART_NAME\n10\n{}\n20\n{}\n30\n0\n40\n{}\n1\n{}\n",
                number(text_height),
                number(y),
                number(text_height),
                r12_text::lossy(text)
            );
        }
        out.push_str("0\nENDSEC\n0\nEOF\n");
        out.into_bytes()
    }
}

impl CncExport {
    /// The files and what is not drawn, one row per part: a CSV beside the
    /// drawings tells the shop what it got.
    #[must_use]
    pub fn table(&self) -> Table {
        let mut rows = self
            .parts
            .iter()
            .map(|part| {
                vec![
                    Cell::Text(part.part.clone()),
                    Cell::Text(part.file.clone()),
                    Cell::Number(part.thickness_mm),
                    Cell::Number(part.size_mm[0]),
                    Cell::Number(part.size_mm[1]),
                    Cell::Text(part.layers().into_iter().collect::<Vec<_>>().join(" ")),
                    Cell::Text(String::new()),
                ]
            })
            .collect::<Vec<_>>();
        rows.extend(self.not_drawn.iter().map(|part| {
            vec![
                Cell::Text(part.part.clone()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(part.reason.to_string()),
            ]
        }));
        Table {
            header: [
                "part",
                "file",
                "thickness_mm",
                "x_mm",
                "y_mm",
                "layers",
                "not_drawn",
            ]
            .map(str::to_owned)
            .to_vec(),
            rows,
        }
    }

    /// Every drawing as `<name>.dxf` and the table as `parts.csv`, in one
    /// ZIP archive.
    #[must_use]
    pub fn zip(&self) -> Vec<u8> {
        let drawings = self
            .parts
            .iter()
            .map(|part| (part.file.as_str(), part.dxf()))
            .collect::<Vec<_>>();
        let table = self.table().csv(DecimalSeparator::Point).into_bytes();
        let files = drawings
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .chain([("parts.csv", table.as_slice())])
            .collect::<Vec<_>>();
        stored_zip(&files)
    }
}
