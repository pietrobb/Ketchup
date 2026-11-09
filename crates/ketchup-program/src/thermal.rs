//! Thermal check of a build-up (`thermal_check()`, review 2026-11-08 §5.3) to
//! STN 73 0540-2 or ČSN 73 0540-2, by the site's country: the thermal
//! transmittance U (EN ISO 6946, framed layers by the upper and lower bounds),
//! the temperature factor of the inner surface against mould (EN ISO 13788) and
//! condensation inside the build-up in the design winter state (Glaser). A value
//! the program does not give leaves its criterion not verified, never guessed.

use crate::validate::{Issue, Severity};
use serde::{Deserialize, Serialize};

pub const THERMAL_FAILED: &str = "thermal_requirement_failed";
pub const THERMAL_NOT_VERIFIED: &str = "thermal_not_verified";

/// What the build-up separates, which sets its surface resistances and requirement.
pub const ELEMENTS: [&str; 6] = [
    "wall",
    "steep_roof",
    "roof",
    "floor_over_exterior",
    "ceiling_under_unheated",
    "floor_on_ground",
];

pub const BASIS: [&str; 5] = [
    "U = 1 / R_T, R_T = (R'_T + R''_T) / 2 (EN ISO 6946 6.7): R'_T over the sections through the framed layers' members and bays, R''_T with each framed layer's area-weighted conductivity; R_si/R_se 0.13/0.04 a wall, 0.10/0.04 a roof, 0.17/0.04 a floor over the outside, 0.10/0.10 a ceiling under an unheated space, 0.17/0 a floor on the ground; a ventilated layer and those outside it are left out, R_se = R_si",
    "requirement: STN 73 0540-2+Z1+Z2:2019 table 1 U_r2, normalized from 2021 (a wall and a roof over 45° 0.22, a roof up to 45° and a floor over the outside 0.15, a ceiling under an unheated space 0.20 W/(m²K); a floor on the ground R >= 2.50 m²K/W, table A.1); ČSN 73 0540-2:2011 table 3 U_N,20 for θ_im 18-22 °C (a wall, a roof over 45° and a ceiling under an unheated space 0.30, a roof up to 45° and a floor over the outside 0.24, a floor on the ground 0.45 W/(m²K))",
    "inner surface: f_Rsi = 1 - 0.25 / R_T of the section through the members with R_si 0.25 (EN ISO 13788), at least f_Rsi,cr at which the indoor air reaches 80 % on the surface (mould); the safety margins of the national standards on φ_i or f_Rsi are not added",
    "condensation: Glaser in the steady design winter state through the bays (θ_e, φ_e outside, θ_i, φ_i inside, s_d = μ d), p_sat of EN ISO 13788; where the vapour pressure would exceed saturation the build-up is not verified, as the annual balance (EN ISO 13788 monthly climate) is not computed",
    "not checked: thermal bridges, the average U of the envelope, the heat need of the building, the summer stability of the room and a floor's touch temperature",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// The share of the layer's area its members take (stud / spacing).
    pub fraction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infill_lambda: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infill_mu: Option<f64>,
}

/// One layer as the program gives it, inside first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub thickness_mm: f64,
    /// Design thermal conductivity, W/(m·K) (of the members of a framed layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lambda: Option<f64>,
    /// Thermal resistance given instead of lambda (an unventilated air layer), m²K/W.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    /// Water vapour resistance factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mu: Option<f64>,
    /// Equivalent air layer thickness given instead of mu (a membrane), m.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sd_m: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ventilated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Buildup {
    pub name: String,
    pub element: String,
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theta_e: Option<f64>,
    pub theta_i: f64,
    pub phi_i: f64,
    pub phi_e: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThermalCheck {
    pub name: String,
    pub element: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_w_m2k: Option<f64>,
    /// The layers' thermal resistance, without the surfaces, m²K/W.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r_m2k_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    /// "pass", "fail" or "not_verified" of U (or R) against the requirement.
    pub transmittance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f_rsi: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f_rsi_cr: Option<f64>,
    pub surface: String,
    pub condensation: String,
    /// The layers where vapour would condense in the design winter state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub condenses_in: Vec<String>,
    /// "pass" when every criterion passes, "fail" when one fails, else "not_verified".
    pub status: String,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThermalReport {
    pub basis: Vec<String>,
    pub buildups: Vec<ThermalCheck>,
}

impl ThermalReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buildups.is_empty()
    }

    /// An error for each failed build-up, a warning for each not verified one.
    #[must_use]
    pub fn issues(&self) -> Vec<Issue> {
        self.buildups
            .iter()
            .filter(|check| check.status != "pass")
            .map(|check| {
                let failed = check.status == "fail";
                let what = [
                    ("U", &check.transmittance),
                    ("inner surface", &check.surface),
                    ("condensation", &check.condensation),
                ]
                .iter()
                .filter(|(_, status)| status.as_str() == check.status)
                .map(|(what, _)| *what)
                .collect::<Vec<_>>()
                .join(", ");
                Issue {
                    source_lines: Vec::new(),
                    severity: if failed {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    kind: if failed {
                        THERMAL_FAILED
                    } else {
                        THERMAL_NOT_VERIFIED
                    },
                    parts: Vec::new(),
                    message: if failed {
                        format!(
                            "thermal_check({:?}): {what} fails (U {} W/(m²K), {}; f_Rsi {} < {}; condenses in {:?})",
                            check.name,
                            shown(check.u_w_m2k),
                            check.requirement.as_deref().unwrap_or("no requirement"),
                            shown(check.f_rsi),
                            shown(check.f_rsi_cr),
                            check.condenses_in
                        )
                    } else {
                        format!(
                            "thermal_check({:?}): {what} not verified: {}",
                            check.name,
                            check.missing.join("; ")
                        )
                    },
                    where_mm: None,
                    hint: if failed {
                        "Add insulation, use a material with a lower lambda, or move the vapour \
                         control layer to the warm side."
                            .to_owned()
                    } else {
                        "Give what is missing (lambda and mu of each layer, theta_e of the place, \
                         site(country))."
                            .to_owned()
                    },
                }
            })
            .collect()
    }
}

fn shown(value: Option<f64>) -> String {
    value.map_or_else(|| "?".to_owned(), |v| format!("{v:.3}"))
}

/// Surface resistances R_si, R_se of an element (EN ISO 6946 table 7).
fn surfaces(element: &str) -> (f64, f64) {
    match element {
        "wall" => (0.13, 0.04),
        "steep_roof" | "roof" => (0.10, 0.04),
        "floor_over_exterior" => (0.17, 0.04),
        "ceiling_under_unheated" => (0.10, 0.10),
        _ => (0.17, 0.0),
    }
}

enum Requirement {
    U(f64),
    R(f64),
}

fn requirement(country: &str, element: &str) -> (Requirement, &'static str) {
    if country == "SK" {
        const SK: &str = "STN 73 0540-2+Z1+Z2:2019 table 1, U_r2 normalized from 2021";
        match element {
            "wall" | "steep_roof" => (Requirement::U(0.22), SK),
            "roof" | "floor_over_exterior" => (Requirement::U(0.15), SK),
            "ceiling_under_unheated" => (Requirement::U(0.20), SK),
            _ => (
                Requirement::R(2.50),
                "STN 73 0540-2+Z1+Z2:2019 table A.1, R_r2 normalized from 2021",
            ),
        }
    } else {
        const CZ: &str = "ČSN 73 0540-2:2011 table 3, U_N,20";
        match element {
            "wall" | "steep_roof" | "ceiling_under_unheated" => (Requirement::U(0.30), CZ),
            "roof" | "floor_over_exterior" => (Requirement::U(0.24), CZ),
            _ => (Requirement::U(0.45), CZ),
        }
    }
}

/// Saturation pressure of water vapour, Pa (EN ISO 13788 (E.7), (E.8)).
fn p_sat(theta: f64) -> f64 {
    if theta >= 0.0 {
        610.5 * (17.269 * theta / (237.3 + theta)).exp()
    } else {
        610.5 * (21.875 * theta / (265.5 + theta)).exp()
    }
}

/// The temperature at which `p` Pa saturates the air.
fn dew_point(p: f64) -> f64 {
    let l = (p / 610.5).ln();
    if p >= 610.5 {
        237.3 * l / (17.269 - l)
    } else {
        265.5 * l / (21.875 - l)
    }
}

/// The thermal and vapour resistance of one material of a layer: (R, s_d).
type Material = (Option<f64>, Option<f64>);

fn members(layer: &Layer) -> Material {
    let d = layer.thickness_mm / 1000.0;
    (
        layer.r.or(layer.lambda.map(|lambda| d / lambda)),
        layer.sd_m.or(layer.mu.map(|mu| mu * d)),
    )
}

fn bays(layer: &Layer) -> Material {
    let d = layer.thickness_mm / 1000.0;
    let frame = layer.frame.as_ref();
    (
        frame.and_then(|f| f.infill_lambda).map(|lambda| d / lambda),
        frame.and_then(|f| f.infill_mu).map(|mu| mu * d),
    )
}

/// The layers counted: those inside the first ventilated one.
fn counted(buildup: &Buildup) -> (&[Layer], bool) {
    match buildup.layers.iter().position(|l| l.ventilated) {
        Some(at) => (&buildup.layers[..at], true),
        None => (&buildup.layers, false),
    }
}

struct Transmittance {
    /// Sum of the layers' R, m²K/W (R_T - R_si - R_se).
    r: f64,
    u: f64,
    /// R of the section through every framed layer's members.
    r_members: f64,
    /// R of the section through every bay, and its layers' (R, s_d) in order.
    bay_path: Vec<(String, f64, Option<f64>)>,
    /// Relative error of the bounds (EN ISO 6946 6.7.2.5).
    error: f64,
}

fn transmittance(
    layers: &[Layer],
    rsi: f64,
    rse: f64,
    missing: &mut Vec<String>,
) -> Option<Transmittance> {
    let mut sections = vec![(1.0, 0.0)];
    let mut lower = 0.0;
    let mut r_members = 0.0;
    let mut bay_path = Vec::new();
    let mut complete = true;
    for layer in layers {
        let (r_m, sd_m) = members(layer);
        let Some(r_m) = r_m else {
            missing.push(format!("layer {:?} has no lambda (or r)", layer.name));
            complete = false;
            continue;
        };
        r_members += r_m;
        match &layer.frame {
            None => {
                lower += r_m;
                sections.iter_mut().for_each(|s| s.1 += r_m);
                bay_path.push((layer.name.clone(), r_m, sd_m));
            }
            Some(frame) => {
                let (r_b, sd_b) = bays(layer);
                let Some(r_b) = r_b else {
                    missing.push(format!(
                        "framed layer {:?} has no infill_lambda",
                        layer.name
                    ));
                    complete = false;
                    continue;
                };
                let f = frame.fraction;
                lower += 1.0 / (f / r_m + (1.0 - f) / r_b);
                sections = sections
                    .iter()
                    .flat_map(|&(share, r)| [(share * f, r + r_m), (share * (1.0 - f), r + r_b)])
                    .collect();
                bay_path.push((layer.name.clone(), r_b, sd_b));
            }
        }
    }
    if !complete {
        return None;
    }
    let upper = 1.0
        / sections
            .iter()
            .map(|(share, r)| share / (rsi + r + rse))
            .sum::<f64>();
    let lower = rsi + lower + rse;
    let total = f64::midpoint(upper, lower);
    Some(Transmittance {
        r: total - rsi - rse,
        u: 1.0 / total,
        r_members,
        bay_path,
        error: (upper - lower) / (2.0 * total),
    })
}

/// The layers where the vapour pressure through the bays exceeds saturation,
/// or None when a layer has no mu (or s_d).
fn condensation(
    path: &[(String, f64, Option<f64>)],
    buildup: &Buildup,
    theta_e: f64,
    (rsi, rse): (f64, f64),
    missing: &mut Vec<String>,
) -> Option<Vec<String>> {
    let Some(path) = path
        .iter()
        .map(|(name, r, sd)| sd.map(|sd| (name, *r, sd)))
        .collect::<Option<Vec<_>>>()
    else {
        missing.extend(
            path.iter()
                .filter(|(_, _, sd)| sd.is_none())
                .map(|(name, _, _)| format!("layer {name:?} has no mu (or sd)")),
        );
        return None;
    };
    let r_total = rsi + path.iter().map(|(_, r, _)| r).sum::<f64>() + rse;
    let sd_total = path.iter().map(|(_, _, sd)| sd).sum::<f64>();
    let p_i = buildup.phi_i / 100.0 * p_sat(buildup.theta_i);
    let p_e = buildup.phi_e / 100.0 * p_sat(theta_e);
    let mut r_at = rsi;
    let mut sd_at = 0.0;
    let mut wet = Vec::new();
    const SLICES: u32 = 20;
    for (name, r, sd) in path {
        let condenses = (0..=SLICES).any(|k| {
            let t = f64::from(k) / f64::from(SLICES);
            let theta = buildup.theta_i - (buildup.theta_i - theta_e) * (r_at + t * r) / r_total;
            let p = if sd_total > 0.0 {
                p_i - (p_i - p_e) * (sd_at + t * sd) / sd_total
            } else {
                p_i
            };
            p > p_sat(theta)
        });
        if condenses {
            wet.push(name.clone());
        }
        r_at += r;
        sd_at += sd;
    }
    Some(wet)
}

fn thousandths(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn verdict(statuses: &[&str]) -> String {
    if statuses.contains(&"fail") {
        "fail"
    } else if statuses.iter().all(|s| *s == "pass") {
        "pass"
    } else {
        "not_verified"
    }
    .to_owned()
}

fn check(buildup: &Buildup, country: Option<&str>) -> ThermalCheck {
    let mut missing = Vec::new();
    let (layers, ventilated) = counted(buildup);
    let (rsi, mut rse) = surfaces(&buildup.element);
    if ventilated {
        rse = rsi;
    }
    let result = transmittance(layers, rsi, rse, &mut missing);
    if let Some(t) = result.as_ref().filter(|t| t.error > 0.2) {
        missing.push(format!(
            "the bounds of U differ by {:.0} % (over 20 %, EN ISO 6946 6.7.2.5): compute it numerically (EN ISO 10211)",
            100.0 * t.error
        ));
    }
    let bounded = result.as_ref().filter(|t| t.error <= 0.2);
    let (mut requirement_text, mut transmittance_status) = (None, "not_verified");
    match country {
        None => missing.push("no site(country): the requirement depends on it".to_owned()),
        Some("CZ") if !(18.0..=22.0).contains(&buildup.theta_i) => missing.push(format!(
            "ČSN 73 0540-2 U_N,20 holds for θ_im 18-22 °C, theta_i is {}",
            buildup.theta_i
        )),
        Some(country) => {
            let (required, standard) = requirement(country, &buildup.element);
            requirement_text = Some(match required {
                Requirement::U(u) => format!("U <= {u:.2} W/(m²K), {standard}"),
                Requirement::R(r) => format!("R >= {r:.2} m²K/W, {standard}"),
            });
            if let Some(t) = bounded {
                // Compared as the standards state them, to three decimals.
                transmittance_status = match required {
                    Requirement::U(u) if thousandths(t.u) > u => "fail",
                    Requirement::R(r) if thousandths(t.r) < r => "fail",
                    _ => "pass",
                };
            }
        }
    }
    let ground = buildup.element == "floor_on_ground";
    let theta_e = buildup.theta_e.filter(|_| !ground);
    if ground {
        missing.push(
            "the inner surface and condensation of a floor on the ground are not computed (the ground's temperature is not modelled)"
                .to_owned(),
        );
    } else if theta_e.is_none() {
        missing.push(
            "no theta_e: the design outdoor temperature of the place (STN 73 0540-3 / ČSN 73 0540-3)"
                .to_owned(),
        );
    }
    let (mut f_rsi, mut f_rsi_cr, mut surface) = (None, None, "not_verified");
    let (mut condensation_status, mut condenses_in) = ("not_verified", Vec::new());
    if let (Some(t), Some(theta_e)) = (bounded, theta_e) {
        let f = 1.0 - 0.25 / (0.25 + t.r_members + rse);
        let p_i = buildup.phi_i / 100.0 * p_sat(buildup.theta_i);
        let cr = (dew_point(p_i / 0.8) - theta_e) / (buildup.theta_i - theta_e);
        surface = if thousandths(f) >= thousandths(cr) {
            "pass"
        } else {
            "fail"
        };
        f_rsi = Some(f);
        f_rsi_cr = Some(cr);
        if let Some(wet) = condensation(&t.bay_path, buildup, theta_e, (rsi, rse), &mut missing) {
            if !wet.is_empty() {
                missing.push(format!(
                    "vapour condenses in {wet:?} in the design winter state; the annual balance is not computed"
                ));
            }
            condensation_status = if wet.is_empty() {
                "pass"
            } else {
                "not_verified"
            };
            condenses_in = wet;
        }
    }
    ThermalCheck {
        name: buildup.name.clone(),
        element: buildup.element.clone(),
        u_w_m2k: result.as_ref().map(|t| t.u),
        r_m2k_w: result.as_ref().map(|t| t.r),
        requirement: requirement_text,
        transmittance: transmittance_status.to_owned(),
        f_rsi,
        f_rsi_cr,
        surface: surface.to_owned(),
        condensation: condensation_status.to_owned(),
        condenses_in,
        status: verdict(&[transmittance_status, surface, condensation_status]),
        missing,
    }
}

/// The thermal check of every build-up the program declares.
#[must_use]
pub fn thermal_checks(model: &crate::ProgramModel) -> ThermalReport {
    if model.thermal.is_empty() {
        return ThermalReport::default();
    }
    let country = model.site.as_ref().map(|site| site.country.as_str());
    ThermalReport {
        basis: BASIS.map(str::to_owned).to_vec(),
        buildups: model.thermal.iter().map(|b| check(b, country)).collect(),
    }
}

/// One layer of `thermal_check(name)` from its dict: `read(key)` is the
/// number under key, if given.
pub(crate) fn layer(
    name: &str,
    label: String,
    ventilated: bool,
    read: impl Fn(&str) -> anyhow::Result<Option<f64>>,
) -> anyhow::Result<Layer> {
    let positive = |key: &str| -> anyhow::Result<Option<f64>> {
        read(key)?
            .map(|value| {
                if value <= 0.0 {
                    anyhow::bail!(
                        "thermal_check({name:?}): layer {label:?} {key} must be positive, got {value}"
                    );
                }
                Ok(value)
            })
            .transpose()
    };
    let thickness_mm = positive("thickness")?.ok_or_else(|| {
        anyhow::anyhow!("thermal_check({name:?}): layer {label:?} needs a thickness")
    })?;
    for (key, other) in [("lambda", "r"), ("mu", "sd")] {
        if read(key)?.is_some() && read(other)?.is_some() {
            anyhow::bail!(
                "thermal_check({name:?}): layer {label:?} takes {key} or {other}, not both"
            );
        }
    }
    for key in ["mu", "infill_mu"] {
        if positive(key)?.is_some_and(|mu| mu < 1.0) {
            anyhow::bail!("thermal_check({name:?}): layer {label:?} {key} is at least 1 (air)");
        }
    }
    // A framed layer of buildup(): "spacing" between its members, each "stud" wide.
    let frame = match positive("spacing")? {
        None => None,
        Some(spacing) => {
            let width = positive("stud")?.unwrap_or(60.0);
            if width >= spacing {
                anyhow::bail!(
                    "thermal_check({name:?}): layer {label:?} spacing must exceed its studs"
                );
            }
            Some(Frame {
                fraction: width / spacing,
                infill_lambda: positive("infill_lambda")?,
                infill_mu: positive("infill_mu")?,
            })
        }
    };
    Ok(Layer {
        lambda: positive("lambda")?,
        r: positive("r")?,
        mu: positive("mu")?,
        sd_m: positive("sd")?,
        ventilated,
        frame,
        thickness_mm,
        name: label,
    })
}

/// U and R of a build-up as `thermal_check()` returns them, when every layer has lambda.
#[must_use]
pub fn u_and_r(buildup: &Buildup) -> Option<(f64, f64)> {
    let (layers, ventilated) = counted(buildup);
    let (rsi, rse) = surfaces(&buildup.element);
    transmittance(
        layers,
        rsi,
        if ventilated { rsi } else { rse },
        &mut Vec::new(),
    )
    .map(|t| (t.u, t.r))
}
