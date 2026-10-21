//! Line styles of a drawing read as an underlay: the color (group 62, or 420 for a true
//! color), linetype (6) with its scale (48) and lineweight (370) an entity names, or takes
//! from its layer (ByLayer) or from the insert that places it (ByBlock), with the
//! drawing's `$LTSCALE` and `$LWDEFAULT`. What cannot be drawn as asked is counted and
//! named in the report: a linetype the drawing does not define is drawn continuous, the
//! shapes and texts of a complex one are left out of its dashes, and a color, lineweight
//! or linetype scale that is none is drawn by the layer's (or at scale 1).

use super::*;
use crate::underlay::style::LineStyle;

/// The groups of an entity an underlay reads as its style, not reported ignored; 430 names
/// the color book of a 420 true color.
pub(super) const STYLE_GROUPS: [i16; 6] = [6, 48, 62, 370, 420, 430];
/// The header variables an underlay reads for its styles.
pub(super) const STYLE_VARIABLES: [&str; 2] = ["$LTSCALE", "$LWDEFAULT"];

/// What an entity says of one part of its style.
#[derive(Clone, Debug, Default, PartialEq)]
enum Named<T> {
    #[default]
    ByLayer,
    ByBlock,
    Own(T),
    /// A value that names none, as the drawing gives it: drawn ByLayer and reported.
    Unknown(i64),
}

/// `value` as the entity names it, or [`Named::Unknown`] if it names none.
fn own<T>(known: Option<T>, value: i64) -> Named<T> {
    known.map_or(Named::Unknown(value), Named::Own)
}

/// An entity's style as its record names it; ByLayer is resolved where it is drawn, on
/// its final layer, ByBlock where an insert places it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct EntityStyle {
    /// `None` for the foreground color, ACI 7.
    color: Named<Option<[u8; 3]>>,
    /// The upper-case name.
    linetype: Named<String>,
    /// Hundredths of a millimetre, or -3 for the drawing's default.
    weight: Named<i16>,
    /// Its linetype scale, times the scale of the inserts that place it.
    scale: f64,
    /// A scale it gives that is none, drawn at 1 and reported.
    invalid_scale: Option<f64>,
}

impl Default for EntityStyle {
    fn default() -> Self {
        Self {
            color: Named::ByLayer,
            linetype: Named::ByLayer,
            weight: Named::ByLayer,
            scale: 1.0,
            invalid_scale: None,
        }
    }
}

impl EntityStyle {
    pub(super) fn read(record: &[DxfPair<'_>]) -> Result<Self, DxfImportError> {
        let color = match parse_optional_i64(record, 420)? {
            Some(rgb) => Named::Own(Some(true_color(rgb))),
            None => match parse_optional_i64(record, 62)? {
                None | Some(256) => Named::ByLayer,
                Some(0) => Named::ByBlock,
                Some(aci) => own(aci_color(aci), aci),
            },
        };
        let linetype = match parse_optional_text(record, 6)? {
            None => Named::ByLayer,
            Some(name) if name.eq_ignore_ascii_case("BYLAYER") => Named::ByLayer,
            Some(name) if name.eq_ignore_ascii_case("BYBLOCK") => Named::ByBlock,
            Some(name) => Named::Own(name.to_ascii_uppercase()),
        };
        let weight = match parse_optional_i64(record, 370)? {
            None | Some(-1) => Named::ByLayer,
            Some(-2) => Named::ByBlock,
            Some(weight) => own(lineweight(weight), weight),
        };
        let scale = parse_optional_number(record, 48)?.unwrap_or(1.0);
        let valid = scale.is_finite() && scale > 0.0;
        Ok(Self {
            color,
            linetype,
            weight,
            scale: if valid { scale } else { 1.0 },
            invalid_scale: (!valid).then_some(scale),
        })
    }

    /// The style of an entity of a block an insert of `style` places at `scale`: what it
    /// takes ByBlock it takes from the insert, and its pattern scales with it.
    pub(super) fn within(&self, style: &Self, scale: f64) -> Self {
        fn of<T: Clone>(own: &Named<T>, insert: &Named<T>) -> Named<T> {
            match own {
                Named::ByBlock => insert.clone(),
                _ => own.clone(),
            }
        }
        Self {
            color: of(&self.color, &style.color),
            linetype: of(&self.linetype, &style.linetype),
            weight: of(&self.weight, &style.weight),
            scale: self.scale * scale,
            invalid_scale: self.invalid_scale,
        }
    }
}

/// A layer's own style.
struct LayerStyle {
    color: Option<[u8; 3]>,
    linetype: String,
    weight: i16,
}

/// A linetype's pattern in drawing units, and whether shapes or texts belong to it.
struct Linetype {
    pattern: Vec<f64>,
    complex: bool,
}

/// The layers, linetypes and style variables of a drawing.
pub(super) struct DrawingStyles {
    /// By upper-case name, as a drawing names them in any case.
    layers: BTreeMap<String, LayerStyle>,
    linetypes: BTreeMap<String, Linetype>,
    /// Millimetres of pattern per drawing unit of a linetype at scale 1.
    pattern_scale: f64,
    default_weight: i16,
}

impl DrawingStyles {
    /// The styles of the TABLES section `tables` and the HEADER section `header`; a color or
    /// lineweight that is none is counted in `diagnostics`.
    pub(super) fn read(
        tables: &[DxfPair<'_>],
        header: &[DxfPair<'_>],
        units: ImportUnitDecision,
        diagnostics: &mut DiagnosticCounts,
    ) -> Result<Self, DxfImportError> {
        let variable = |name: &str, code: i16| {
            let at = header
                .iter()
                .position(|pair| pair.code == 9 && pair.value == name)?;
            header[at + 1..]
                .iter()
                .take_while(|pair| pair.code != 9)
                .find(|pair| pair.code == code)
                .map(|pair| pair.value)
        };
        let ltscale = variable("$LTSCALE", 40)
            .map(parse_number)
            .transpose()?
            .filter(|scale| *scale > 0.0)
            .unwrap_or(1.0);
        let default_weight = match variable("$LWDEFAULT", 370).map(parse_integer::<i64>) {
            None => None,
            Some(weight) => known(weight?, lineweight, LINEWEIGHT_UNKNOWN, diagnostics)?,
        }
        .filter(|weight| *weight >= 0)
        .unwrap_or(25);
        let mut styles = Self {
            layers: BTreeMap::new(),
            linetypes: BTreeMap::new(),
            pattern_scale: ltscale * units.millimetres_per_unit(),
            default_weight,
        };
        let mut table = String::new();
        let mut index = 0;
        while index < tables.len() {
            let kind = tables[index].value;
            let start = index + 1;
            index = start;
            while index < tables.len() && tables[index].code != 0 {
                index += 1;
            }
            let record = &tables[start..index];
            let name = || {
                record
                    .iter()
                    .find(|pair| pair.code == 2)
                    .map(|pair| pair.value.to_ascii_uppercase())
            };
            if kind.eq_ignore_ascii_case("TABLE") {
                table = name().unwrap_or_default();
            } else if table == "LAYER" && kind.eq_ignore_ascii_case("LAYER") {
                let Some(name) = name() else { continue };
                let color = match parse_optional_i64(record, 420)? {
                    Some(rgb) => Some(true_color(rgb)),
                    // A negative color is a layer turned off; its lines keep the color.
                    None => {
                        let aci = parse_optional_i64(record, 62)?.map_or(7, i64::abs);
                        known(aci, aci_color, COLOR_UNKNOWN, diagnostics)?.flatten()
                    }
                };
                let weight = match parse_optional_i64(record, 370)? {
                    Some(weight) => known(weight, lineweight, LINEWEIGHT_UNKNOWN, diagnostics)?,
                    None => None,
                }
                .unwrap_or(-3);
                let linetype = parse_optional_text(record, 6)?
                    .unwrap_or("CONTINUOUS")
                    .to_ascii_uppercase();
                styles.layers.insert(
                    name,
                    LayerStyle {
                        color,
                        linetype,
                        weight,
                    },
                );
            } else if table == "LTYPE" && kind.eq_ignore_ascii_case("LTYPE") {
                let Some(name) = name() else { continue };
                let pattern = record
                    .iter()
                    .filter(|pair| pair.code == 49)
                    .map(|pair| parse_number(pair.value))
                    .collect::<Result<Vec<_>, _>>()?;
                let complex = record
                    .iter()
                    .filter(|pair| pair.code == 74)
                    .any(|pair| pair.value.trim() != "0");
                styles.linetypes.insert(name, Linetype { pattern, complex });
            }
        }
        Ok(styles)
    }

    /// How an entity of `style` on `layer` is drawn; a linetype it cannot draw as asked is
    /// counted in `diagnostics`.
    pub(super) fn line_style(
        &self,
        style: &EntityStyle,
        layer: &str,
        diagnostics: &mut DiagnosticCounts,
    ) -> Result<LineStyle, DxfImportError> {
        let own = self.layers.get(&layer.to_ascii_uppercase());
        let color = match &style.color {
            Named::Own(color) => *color,
            Named::ByLayer => own.and_then(|layer| layer.color),
            Named::ByBlock => None,
            Named::Unknown(aci) => {
                report(COLOR_UNKNOWN, *aci, diagnostics)?;
                own.and_then(|layer| layer.color)
            }
        };
        let weight = match style.weight {
            Named::Own(weight) => weight,
            Named::ByLayer => own.map_or(-3, |layer| layer.weight),
            Named::ByBlock => -3,
            Named::Unknown(weight) => {
                report(LINEWEIGHT_UNKNOWN, weight, diagnostics)?;
                own.map_or(-3, |layer| layer.weight)
            }
        };
        let linetype = match &style.linetype {
            Named::Own(name) => Some(name.as_str()),
            Named::ByLayer => own.map(|layer| layer.linetype.as_str()),
            Named::ByBlock | Named::Unknown(_) => None,
        };
        if let Some(scale) = style.invalid_scale {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.linetype-scale-invalid",
                Some(scale.to_string()),
                1,
            )?;
        }
        let mut pattern_mm = Vec::new();
        if let Some(name) = linetype.filter(|name| *name != "CONTINUOUS") {
            match self.linetypes.get(name) {
                None => diagnostics.add(
                    ImportDiagnosticSeverity::Warning,
                    "dxf.linetype-undefined",
                    Some(name.to_owned()),
                    1,
                )?,
                Some(linetype) => {
                    if linetype.complex {
                        diagnostics.add(
                            ImportDiagnosticSeverity::Warning,
                            "dxf.linetype-shapes-not-drawn",
                            Some(name.to_owned()),
                            1,
                        )?;
                    }
                    // A pattern of dots only has nothing to repeat: it is drawn continuous.
                    if linetype.pattern.iter().any(|length| *length != 0.0) {
                        let scale = self.pattern_scale * style.scale;
                        pattern_mm = linetype.pattern.iter().map(|l| l * scale).collect();
                    }
                }
            }
        }
        Ok(LineStyle {
            color,
            weight_hmm: u16::try_from(if weight < 0 {
                self.default_weight
            } else {
                weight
            })
            .unwrap_or(0),
            pattern_mm,
        })
    }
}

const COLOR_UNKNOWN: &str = "dxf.color-unknown";
const LINEWEIGHT_UNKNOWN: &str = "dxf.lineweight-unknown";

/// Counts in `diagnostics`, under `code`, a value that names no color or lineweight.
fn report(
    code: &str,
    value: i64,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        code,
        Some(value.to_string()),
        1,
    )
}

/// What `value` names by `name`, or `None` counted under `code` if it names nothing.
fn known<T>(
    value: i64,
    name: fn(i64) -> Option<T>,
    code: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<T>, DxfImportError> {
    let named = name(value);
    if named.is_none() {
        report(code, value, diagnostics)?;
    }
    Ok(named)
}

/// A lineweight value of group 370: hundredths of a millimetre up to 2.11 mm, or -3 for
/// the drawing's default (-1 and -2 are ByLayer and ByBlock).
fn lineweight(value: i64) -> Option<i16> {
    i16::try_from(value)
        .ok()
        .filter(|weight| (-3..=211).contains(weight))
}

/// A 420 true color, `0x00RRGGBB`.
fn true_color(value: i64) -> [u8; 3] {
    let [.., red, green, blue] = value.to_be_bytes();
    [red, green, blue]
}

/// The color of AutoCAD Color Index `aci` (1 to 255), `Some(None)` for 7, the foreground,
/// and `None` for no index. The hues 10 to 249 are computed as AutoCAD lays them out, 24
/// hues of five shades, each full and pale; they match its palette to within one level of
/// 255 per channel.
fn aci_color(aci: i64) -> Option<Option<[u8; 3]>> {
    let aci = u8::try_from(aci).ok().filter(|aci| *aci >= 1)?;
    let fixed: [[u8; 3]; 9] = [
        [255, 0, 0],
        [255, 255, 0],
        [0, 255, 0],
        [0, 255, 255],
        [0, 0, 255],
        [255, 0, 255],
        [255, 255, 255],
        [128, 128, 128],
        [192, 192, 192],
    ];
    Some(match aci {
        7 => None,
        1..=9 => Some(fixed[usize::from(aci - 1)]),
        250..=255 => {
            let level = (51.0 + f64::from(aci - 250) * 40.8).floor();
            Some([level; 3].map(channel))
        }
        _ => {
            let hue = f64::from(aci / 10 - 1) * 15.0;
            let value = [255.0, 165.0, 127.0, 76.0, 38.0][usize::from(aci % 10 / 2)];
            let pale = aci % 2 == 1;
            Some([0.0, 120.0, 240.0].map(|offset| {
                // How far the channel's own hue is from this one, in sixths of a turn.
                let away = ((hue - offset + 540.0) % 360.0 - 180.0).abs() / 60.0;
                let full = value * (2.0 - away).clamp(0.0, 1.0);
                channel(
                    if pale {
                        full + (value - full) / 2.0
                    } else {
                        full
                    }
                    .floor(),
                )
            }))
        }
    })
}

/// A channel level 0 to 255 as a byte.
fn channel(level: f64) -> u8 {
    (0..=255u8)
        .find(|byte| f64::from(*byte) >= level)
        .unwrap_or(255)
}
