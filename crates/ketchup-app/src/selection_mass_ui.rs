//! Mass, centre of mass and principal moments of the selection, shown under its
//! size. The density of a program material comes from the program evaluation,
//! asked without waiting: while it or the exact solids are being computed the
//! dock says so, never a zero and never a frame spent waiting.

use super::*;
use crate::app_state::ExactCacheKey;
use crate::program_evaluation::Lookup;
use crate::selection_measures_ui::SelectionMeasures;
use ketchup_model::exact_product::ExactResultRegistry;
use serde_json::Value;

/// The assembly answer for (document, revision, exact contents, selection).
pub(crate) type SelectionMassCache = Option<((ExactCacheKey, BTreeSet<InstancePath>), Value)>;

/// The mass of the selection as the dock shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SelectionMass {
    /// The program or the exact solids are still being computed.
    Pending,
    /// The `assembly` of the mass report of the selected parts.
    Ready(Value),
}

/// `value` with four significant digits, without an exponent.
fn significant(value: f64) -> String {
    let mut decimals = 0;
    let mut scaled = value.abs();
    while scaled != 0.0 && scaled < 1000.0 && decimals < 9 {
        scaled *= 10.0;
        decimals += 1;
    }
    format!("{value:.decimals$}")
}

impl KetchupApp {
    /// The mass of the parts `measures` describe; `Pending` while their exact
    /// solids or the program giving their densities are being computed.
    pub(crate) fn selection_mass(&self, measures: &SelectionMeasures) -> SelectionMass {
        if measures.volume_mm3.is_none() {
            return SelectionMass::Pending;
        }
        let evaluation = match self.document.current_rule_program() {
            None => None,
            Some(program) => match self.program_evaluations.try_get(program) {
                Lookup::Pending => return SelectionMass::Pending,
                Lookup::Ready(evaluation) => Some(evaluation),
            },
        };
        let snapshot = self.document.current();
        let exact = self.exact_results_for_snapshot(&snapshot);
        let selected = self.selected_instance_paths();
        let key = (
            (
                snapshot.document_id(),
                snapshot.revision_id(),
                exact.map_or(0, ExactResultRegistry::contents_stamp),
            ),
            selected,
        );
        let mut cache = self.selection_mass.borrow_mut();
        if let Some((cached, assembly)) = cache.as_ref()
            && *cached == key
        {
            return SelectionMass::Ready(assembly.clone());
        }
        let report = ketchup_application::mass_properties::mass_report(
            &snapshot,
            exact,
            evaluation
                .as_ref()
                .and_then(|evaluation| evaluation.as_ref().ok())
                .map(|evaluated| &evaluated.model),
            ketchup_application::validation_rules::ValidationRules::library(),
            Some(&key.1),
        );
        let assembly = report["assembly"].clone();
        *cache = Some((key, assembly.clone()));
        SelectionMass::Ready(assembly)
    }

    pub(crate) fn selection_mass_lines(&self, mass: &SelectionMass) -> Vec<String> {
        let format = |key: &str, arguments: &[(&'static str, String)]| {
            self.catalog
                .format(key, &arguments.iter().cloned().collect::<BTreeMap<_, _>>())
        };
        let SelectionMass::Ready(assembly) = mass else {
            return vec![self.catalog.text("selection-mass-pending")];
        };
        let counted = assembly["parts_counted"].as_u64().unwrap_or(0);
        let missing = assembly["missing"].as_array().cloned().unwrap_or_default();
        let mut lines = Vec::new();
        match (assembly["status"].as_str(), assembly["mass_kg"].as_f64()) {
            (Some("complete"), Some(kg)) => {
                lines.push(format("selection-mass", &[("kg", significant(kg))]));
                let centre = &assembly["centroid_mm"];
                let [x, y, z] = [0, 1, 2]
                    .map(|axis| format!("{:.1}", centre[axis].as_f64().unwrap_or(f64::NAN)));
                lines.push(format(
                    "selection-mass-centre",
                    &[("x", x), ("y", y), ("z", z)],
                ));
                let moments = assembly["principal_kg_mm2"]
                    .as_array()
                    .map(|moments| {
                        moments
                            .iter()
                            .filter_map(Value::as_f64)
                            .map(|moment| significant(moment / 1.0e6))
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_default();
                lines.push(format("selection-mass-inertia", &[("moments", moments)]));
            }
            // Centre and moments of the counted parts alone would describe
            // another body than the selection; only their mass is shown.
            (Some("partial"), Some(kg)) => lines.push(format(
                "selection-mass-partial",
                &[
                    ("kg", significant(kg)),
                    ("counted", counted.to_string()),
                    (
                        "total",
                        (counted + assembly["parts_missing"].as_u64().unwrap_or(0)).to_string(),
                    ),
                ],
            )),
            _ => lines.push(self.catalog.text("selection-mass-unknown")),
        }
        if let Some(first) = missing.first() {
            lines.push(format(
                "selection-mass-missing",
                &[
                    (
                        "name",
                        first["name"].as_str().unwrap_or_default().to_owned(),
                    ),
                    (
                        "reason",
                        first["reason"].as_str().unwrap_or_default().to_owned(),
                    ),
                ],
            ));
        }
        if missing.len() > 1 {
            lines.push(format(
                "selection-mass-more-missing",
                &[("count", (missing.len() - 1).to_string())],
            ));
        }
        lines
    }
}
