//! Size, position and volume of the selection, shown above the program so
//! the user sees how big the picked part is without measuring it.

use super::*;
use ketchup_model::document::ScaledPlacement;

/// What the dock reports about the selected parts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SelectionMeasures {
    pub parts: usize,
    /// Size along the part's own axes; only for one part turned against the
    /// world axes, where it differs from the world box.
    pub own_size_mm: Option<[f64; 3]>,
    /// Lowest corner of the world box around the selection.
    pub world_min_mm: [f64; 3],
    pub world_size_mm: [f64; 3],
    /// Exact solid volume; `None` while any selected part has none yet, or
    /// when one is placed scaled.
    pub volume_mm3: Option<f64>,
    /// A selected part whose placement scales it: its exact solid has the
    /// unscaled volume, so no volume is shown.
    pub scaled_placement: Option<ScaledPlacement>,
}

/// Rows of the most mass lines: mass, centre and moments; or a partial or
/// unknown mass, the first missing part and how many more are missing.
const MASS_ROWS: usize = 3;

fn turned(transform: Transform) -> bool {
    let matrix = transform.matrix();
    let identity = Transform::identity();
    [0, 1, 2, 4, 5, 6, 8, 9, 10].into_iter().any(|index| {
        (matrix[index] - identity.matrix()[index]).abs() > ketchup_model::tolerance::ROUNDING
    })
}

/// A length of the card to a tenth of a millimetre, written with a point.
pub(crate) fn length(value_mm: f64) -> String {
    format_decimals(value_mm, 1)
}

fn size_text(size: [f64; 3]) -> String {
    format!(
        "{} × {} × {}",
        length(size[0]),
        length(size[1]),
        length(size[2])
    )
}

impl KetchupApp {
    /// Measures of the selected parts in the painted geometry, or `None`
    /// when nothing measurable is selected.
    pub(crate) fn selection_measures(&self) -> Option<SelectionMeasures> {
        self.measures_of(&self.selected_instance_paths())
    }

    /// Measures of the parts at `selected` in the painted geometry.
    pub(crate) fn measures_of(
        &self,
        selected: &BTreeSet<InstancePath>,
    ) -> Option<SelectionMeasures> {
        if selected.is_empty() {
            return None;
        }
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.hover.projection_cache.borrow();
        let projection = &cache
            .as_ref()
            .expect("interaction cache was built")
            .canonical;
        let exact = self.exact_results_for_snapshot(&snapshot);
        let mut parts = 0;
        let mut corners = Vec::new();
        let mut own_size = None;
        let mut volume = Some(0.0);
        let mut scaled_placement = None;
        for occurrence in projection.occurrences().iter().filter(|occurrence| {
            selected.iter().any(|path| {
                *path == occurrence.instance_path
                    || (path.steps().is_empty()
                        && path.root_occurrence() == occurrence.instance_path.root_occurrence())
            })
        }) {
            let definition_id = occurrence.body.definition_id;
            let Some([minimum, maximum]) =
                self.definition_local_bounds(&snapshot, definition_id, occurrence.local_box, true)
            else {
                continue;
            };
            let size = maximum - minimum;
            parts += 1;
            own_size =
                turned(occurrence.canonical_world_transform).then_some([size.x, size.y, size.z]);
            corners.extend(box_corners(size.x, size.y, size.z).map(|corner| {
                transform_model_point(occurrence.canonical_world_transform, corner + minimum)
            }));
            let part_volume = exact
                .and_then(|results| results.get_render(&snapshot, definition_id))
                .map(|package| match package.as_ref() {
                    ExactBodyPackage::Graph(graph) => graph.volume_mm3,
                    ExactBodyPackage::Imported(imported) => imported.volume_mm3,
                });
            volume = volume.zip(part_volume).map(|(sum, part)| sum + part);
            if let Err(scaled) = ScaledPlacement::check(
                occurrence.instance_path.root_occurrence(),
                occurrence.canonical_world_transform,
            ) {
                scaled_placement.get_or_insert(scaled);
            }
        }
        if scaled_placement.is_some() {
            volume = None;
        }
        let [minimum, maximum] = bounds_of(corners.into_iter())?;
        let size = maximum - minimum;
        Some(SelectionMeasures {
            parts,
            own_size_mm: if parts == 1 { own_size } else { None },
            world_min_mm: [minimum.x, minimum.y, minimum.z],
            world_size_mm: [size.x, size.y, size.z],
            volume_mm3: volume,
            scaled_placement,
        })
    }

    #[cfg(test)]
    pub(crate) fn selection_measure_lines(&self, measures: &SelectionMeasures) -> Vec<String> {
        self.measure_rows(measures)
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    }

    /// The lines under the dock's size tiles: what the tiles do not show
    /// (the world box around a turned part), the position and the volume,
    /// then the mass always in its three rows, so the volume and mass
    /// arriving with the exact solids move nothing below them.
    pub(crate) fn selection_detail_lines(&self, measures: &SelectionMeasures) -> Vec<String> {
        let turned = measures.own_size_mm.is_some();
        let mut lines = self
            .measure_rows(measures)
            .into_iter()
            .filter(|(key, _)| match *key {
                "selection-measures-world-box" => turned,
                "selection-measures-parts"
                | "selection-measures-own-size"
                | "selection-measures-size" => false,
                _ => true,
            })
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        let mut mass = self.selection_mass_lines(&self.selection_mass(measures));
        mass.resize(mass.len().max(MASS_ROWS), String::new());
        lines.extend(mass);
        lines
    }

    fn measure_rows(&self, measures: &SelectionMeasures) -> Vec<(&'static str, String)> {
        let format = |key: &'static str, arguments: &[(&'static str, String)]| {
            let line = self
                .catalog
                .format(key, &arguments.iter().cloned().collect::<BTreeMap<_, _>>());
            (key, line)
        };
        let size_text = |size| self.user_decimal(size_text(size));
        let length = |value| self.user_decimal(length(value));
        let mut rows = Vec::new();
        if measures.parts > 1 {
            rows.push(format(
                "selection-measures-parts",
                &[("count", measures.parts.to_string())],
            ));
        }
        if let Some(own) = measures.own_size_mm {
            rows.push(format(
                "selection-measures-own-size",
                &[("size", size_text(own))],
            ));
        }
        let world_key = if measures.parts > 1 || measures.own_size_mm.is_some() {
            "selection-measures-world-box"
        } else {
            "selection-measures-size"
        };
        rows.push(format(
            world_key,
            &[("size", size_text(measures.world_size_mm))],
        ));
        let [x, y, z] = measures.world_min_mm.map(length);
        rows.push(format(
            "selection-measures-position",
            &[("x", x), ("y", y), ("z", z)],
        ));
        rows.push(match (measures.volume_mm3, measures.scaled_placement) {
            (_, Some(scaled)) => format(
                "selection-measures-volume-refused",
                &[("occurrence", scaled.occurrence_id.0.to_string())],
            ),
            (Some(volume), None) => format(
                "selection-measures-volume",
                &[
                    (
                        "litres",
                        self.user_decimal(fixed_decimals(volume / 1.0e6, 3)),
                    ),
                    (
                        "cubic",
                        self.user_decimal(fixed_decimals(volume / 1.0e9, 4)),
                    ),
                ],
            ),
            (None, None) => format("selection-measures-volume-pending", &[]),
        });
        rows
    }
}
