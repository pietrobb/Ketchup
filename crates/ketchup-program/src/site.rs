//! The building site (`site()`, review 2026-11-08 §5.3): its country, municipality and
//! altitude, and the characteristic snow load on the ground s_k from that country's
//! national annex to EN 1991-1-3. A value the annex does not give for the inputs is
//! left unknown with what to read and where, never guessed.

use serde::{Deserialize, Serialize};

pub const COUNTRIES: [&str; 2] = ["SK", "CZ"];

/// ČSN EN 1991-1-3 national annex with change Z1:2006: s_k of the snow zones I-VII of
/// its map, kN/m². Zone VIII has no value in the annex; it is set for the place.
const CZ_SNOW_ZONES: [(&str, f64); 7] = [
    ("I", 0.7),
    ("II", 1.0),
    ("III", 1.5),
    ("IV", 2.0),
    ("V", 2.5),
    ("VI", 3.0),
    ("VII", 4.0),
];
const CZ_ANNEX: &str = "ČSN EN 1991-1-3 national annex with Z1:2006";
const SK_ANNEX: &str = "STN EN 1991-1-3/NA1:2012";
/// Above this altitude snow takes psi0 0.7 and psi2 0.2 (EN 1990 table A1.1).
const HIGH_SITE_M: f64 = 1000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub country: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub municipality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub altitude_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snow_zone: Option<String>,
    /// The characteristic snow load on the ground, kN/m²; none when not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sk_kn_m2: Option<f64>,
    /// Where s_k comes from, or what to read and where when it is not known.
    pub source: String,
}

/// The inputs of `site()` as the program gives them.
pub struct SiteInput<'a> {
    pub country: &'a str,
    pub municipality: Option<String>,
    pub altitude_m: Option<f64>,
    /// "I" to "VIII", or 1 to 8.
    pub snow_zone: Option<String>,
    pub sk: Option<f64>,
    pub source: Option<String>,
}

fn roman(zone: &str) -> Option<&'static str> {
    const ZONES: [&str; 8] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII"];
    let zone = zone.trim();
    ZONES
        .iter()
        .position(|z| z.eq_ignore_ascii_case(zone))
        .or_else(|| {
            zone.parse::<usize>()
                .ok()
                .filter(|n| (1..=8).contains(n))
                .map(|n| n - 1)
        })
        .map(|i| ZONES[i])
}

impl Site {
    pub fn resolve(input: SiteInput<'_>) -> anyhow::Result<Self> {
        let country = input.country.trim().to_ascii_uppercase();
        if !COUNTRIES.contains(&country.as_str()) {
            anyhow::bail!(
                "site(): country must be one of {COUNTRIES:?}, got {:?}",
                input.country
            );
        }
        if let Some(altitude) = input
            .altitude_m
            .filter(|a| !a.is_finite() || !(-500.0..=3000.0).contains(a))
        {
            anyhow::bail!(
                "site(): altitude is metres above sea level, -500 to 3000, got {altitude}"
            );
        }
        let source = input
            .source
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if let Some(sk) = input.sk {
            if !sk.is_finite() || sk <= 0.0 {
                anyhow::bail!("site(): sk must be a positive kN/m², got {sk}");
            }
            if source.is_none() {
                anyhow::bail!("site(): name the source of sk (the map and the place read off it)");
            }
        }
        let place = input
            .municipality
            .as_deref()
            .map_or_else(|| "the site".to_owned(), |m| format!("{m:?}"));
        let snow_zone = match input.snow_zone.as_deref() {
            None => None,
            Some(_) if country == "SK" => anyhow::bail!(
                "site(\"SK\"): {SK_ANNEX} gives s_k by region and altitude on its map, not by \
                 a zone; read s_k for {place} off the map and give sk= and source="
            ),
            Some(zone) => Some(roman(zone).ok_or_else(|| {
                anyhow::anyhow!("site(\"CZ\"): snow_zone is I to VIII (or 1 to 8), got {zone:?}")
            })?),
        };
        let zone_sk = snow_zone.and_then(|zone| {
            CZ_SNOW_ZONES
                .iter()
                .find(|(z, _)| *z == zone)
                .map(|(_, sk)| *sk)
        });
        let zone = snow_zone.unwrap_or("none");
        let (sk_kn_m2, source) = match (zone_sk, input.sk, source) {
            (Some(_), Some(_), _) => anyhow::bail!(
                "site(\"CZ\"): snow zone {zone} has s_k from the annex; give snow_zone or sk, not both"
            ),
            (Some(sk), None, _) => (Some(sk), format!("{CZ_ANNEX}, snow zone {zone}")),
            (None, Some(sk), Some(source)) => (Some(sk), source),
            (None, _, _) if snow_zone == Some("VIII") => (
                None,
                format!(
                    "s_k not given: snow zone VIII of the {CZ_ANNEX} map has no value; \
                     ask ČHMÚ for {place} and give sk= and source="
                ),
            ),
            (None, _, _) => (
                None,
                format!(
                    "s_k not given: read it for {place} off the {} map and give {}",
                    if country == "SK" { SK_ANNEX } else { CZ_ANNEX },
                    if country == "SK" {
                        "sk= and source="
                    } else {
                        "snow_zone= (or sk= and source=)"
                    }
                ),
            ),
        };
        Ok(Self {
            country,
            municipality: input.municipality,
            altitude_m: input.altitude_m,
            snow_zone: snow_zone.map(str::to_owned),
            sk_kn_m2,
            source,
        })
    }

    /// psi0 and psi2 of snow (EN 1990 table A1.1): 0.7 and 0.2 on a site above 1000 m,
    /// else (or when the altitude is not given) 0.5 and 0.
    #[must_use]
    pub fn snow_psi(site: Option<&Self>) -> (f64, f64) {
        match site.and_then(|s| s.altitude_m) {
            Some(altitude) if altitude > HIGH_SITE_M => (0.7, 0.2),
            _ => (0.5, 0.0),
        }
    }

    /// The line of the design basis that names the site and where its values come from.
    #[must_use]
    pub fn basis(&self) -> String {
        let mut place = self.country.clone();
        if let Some(municipality) = &self.municipality {
            place.push_str(&format!(", {municipality}"));
        }
        if let Some(altitude) = self.altitude_m {
            place.push_str(&format!(", {altitude} m above sea level"));
        }
        let sk = self.sk_kn_m2.map_or_else(
            || self.source.clone(),
            |sk| format!("s_k = {sk} kN/m² ({})", self.source),
        );
        let (psi0, psi2) = Self::snow_psi(Some(self));
        let altitude = match self.altitude_m {
            Some(a) if a > HIGH_SITE_M => "above 1000 m",
            Some(_) => "at most 1000 m",
            None => "altitude not given, taken as at most 1000 m",
        };
        format!(
            "site: {place}; {sk}; snow psi0/psi2 {psi0}/{psi2} (EN 1990 table A1.1, {altitude}); \
             the partial factors and the other psi are the EN recommended values above, \
             not read from the {} national annexes",
            self.country
        )
    }
}
