//! Size, position and volume of the selection, shown above the program so
//! the user sees how big the picked part is without measuring it.

use super::*;

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
    /// Exact solid volume; `None` while any selected part has none yet.
    pub volume_mm3: Option<f64>,
}

fn turned(transform: Transform) -> bool {
    let matrix = transform.matrix();
    let identity = Transform::identity();
    [0, 1, 2, 4, 5, 6, 8, 9, 10].into_iter().any(|index| {
        (matrix[index] - identity.matrix()[index]).abs() > ketchup_model::tolerance::ROUNDING
    })
}

fn length(value_mm: f64) -> String {
    let text = format!("{value_mm:.1}");
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
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
        let selected = self.selected_instance_paths();
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
        }
        let [minimum, maximum] = bounds_of(corners.into_iter())?;
        let size = maximum - minimum;
        Some(SelectionMeasures {
            parts,
            own_size_mm: if parts == 1 { own_size } else { None },
            world_min_mm: [minimum.x, minimum.y, minimum.z],
            world_size_mm: [size.x, size.y, size.z],
            volume_mm3: volume,
        })
    }

    pub(crate) fn selection_measure_lines(&self, measures: &SelectionMeasures) -> Vec<String> {
        let format = |key: &str, arguments: &[(&'static str, String)]| {
            self.catalog
                .format(key, &arguments.iter().cloned().collect::<BTreeMap<_, _>>())
        };
        let mut lines = Vec::new();
        if measures.parts > 1 {
            lines.push(format(
                "selection-measures-parts",
                &[("count", measures.parts.to_string())],
            ));
        }
        if let Some(own) = measures.own_size_mm {
            lines.push(format(
                "selection-measures-own-size",
                &[("size", size_text(own))],
            ));
        }
        let world_key = if measures.parts > 1 || measures.own_size_mm.is_some() {
            "selection-measures-world-box"
        } else {
            "selection-measures-size"
        };
        lines.push(format(
            world_key,
            &[("size", size_text(measures.world_size_mm))],
        ));
        let [x, y, z] = measures.world_min_mm.map(length);
        lines.push(format(
            "selection-measures-position",
            &[("x", x), ("y", y), ("z", z)],
        ));
        lines.push(match measures.volume_mm3 {
            Some(volume) => format(
                "selection-measures-volume",
                &[
                    ("litres", format!("{:.3}", volume / 1.0e6)),
                    ("cubic", format!("{:.4}", volume / 1.0e9)),
                ],
            ),
            None => self.catalog.text("selection-measures-volume-pending"),
        });
        lines
    }

    /// Shows the size, position and volume of the selection; nothing when
    /// nothing is selected, so the dock does not grow.
    pub(super) fn show_selection_measures(&mut self, ui: &mut egui::Ui) {
        let Some(measures) = self.selection_measures() else {
            return;
        };
        egui::CollapsingHeader::new(self.catalog.text("selection-measures-title"))
            .id_salt("selection-measures")
            .default_open(true)
            .show(ui, |ui| {
                for line in self.selection_measure_lines(&measures) {
                    ui.label(line);
                }
            });
        ui.separator();
    }
}
