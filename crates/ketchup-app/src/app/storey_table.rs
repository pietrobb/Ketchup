//! The table of storeys in the drawings window (review 2026-11-08 MF-241): the
//! designer gives each storey's finished floor and structure, and the altitude
//! of ±0,000, instead of the plans and level marks being found from the faces.

use eframe::egui;
use ketchup_interaction::LocaleCatalog;
use ketchup_manufacturing::project_drawings::{
    NEAR_FLOOR_MM, StoreyTableError, StoreyTableProblem,
};
use ketchup_manufacturing::title_block::{SheetSettings, StoreyLevels};
use std::collections::BTreeMap;

/// A storey added over the highest one stands this much higher.
const ADDED_STOREY_MM: i32 = 3000;

/// Shows the rows of `settings.storeys`, lowest first as entered, with a
/// remove button each, an add button, which fills an empty table with the
/// storeys `found` in the model, and the altitude of ±0,000.
pub(crate) fn show_storey_table(
    ui: &mut egui::Ui,
    catalog: &LocaleCatalog,
    settings: &mut SheetSettings,
    found: &[StoreyLevels],
) {
    ui.vertical(|ui| {
        if settings.storeys.is_empty() {
            ui.weak(catalog.text("drawings-storeys-found"));
        }
        let mut removed = None;
        for (index, storey) in settings.storeys.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(catalog.format(
                    "drawings-storey-row",
                    &BTreeMap::from([("storey", (index + 1).to_string())]),
                ));
                ui.add(egui::DragValue::new(&mut storey.finished_floor_mm).suffix(" mm"));
                ui.label(catalog.text("drawings-storey-structural"));
                ui.add(egui::DragValue::new(&mut storey.structural_mm).suffix(" mm"));
                if ui.button(catalog.text("drawings-storey-remove")).clicked() {
                    removed = Some(index);
                }
            });
        }
        if let Some(index) = removed {
            settings.storeys.remove(index);
        }
        if ui.button(catalog.text("drawings-storey-add")).clicked() {
            let added = added_storeys(&settings.storeys, found);
            settings.storeys.extend(added);
        }
        ui.horizontal(|ui| {
            ui.label(catalog.text("drawings-zero-altitude"));
            let mut none = settings.zero_altitude_mm.is_none();
            if ui
                .checkbox(&mut none, catalog.text("drawings-zero-altitude-none"))
                .changed()
            {
                settings.zero_altitude_mm = (!none).then_some(0);
            }
            if let Some(altitude) = settings.zero_altitude_mm.as_mut() {
                let mut metres = f64::from(*altitude) / 1000.0;
                if ui
                    .add(
                        egui::DragValue::new(&mut metres)
                            .fixed_decimals(3)
                            .speed(0.01)
                            .suffix(" m"),
                    )
                    .changed()
                {
                    // Whole millimetres; none past the range of i32.
                    if let Ok(mm) = format!("{:.0}", metres * 1000.0).parse::<i32>() {
                        *altitude = mm;
                    }
                }
            }
        });
        show_north(ui, catalog, settings);
    });
}

/// The direction of north the plans draw their arrow by and the elevations
/// are named by (review 2026-11-08 §5.2), or none.
fn show_north(ui: &mut egui::Ui, catalog: &LocaleCatalog, settings: &mut SheetSettings) {
    ui.horizontal(|ui| {
        ui.label(catalog.text("drawings-north"));
        let mut none = settings.north_tenths_deg.is_none();
        if ui
            .checkbox(&mut none, catalog.text("drawings-north-none"))
            .changed()
        {
            settings.north_tenths_deg = (!none).then_some(0);
        }
        if let Some(tenths) = settings.north_tenths_deg.as_mut() {
            let mut degrees = f64::from(*tenths) / 10.0;
            if ui
                .add(
                    egui::DragValue::new(&mut degrees)
                        .range(-360.0..=360.0)
                        .fixed_decimals(1)
                        .speed(0.5)
                        .suffix(" °"),
                )
                .changed()
            {
                // Tenths of a degree, inside the range the field keeps.
                if let Ok(value) = format!("{:.0}", degrees * 10.0).parse::<i32>() {
                    *tenths = value;
                }
            }
        }
    });
}

/// Why the drawings were refused for a row of the table of storeys (review
/// 2026-11-09 MF-300), as the window says it.
pub(crate) fn storey_table_error_text(catalog: &LocaleCatalog, error: &StoreyTableError) -> String {
    let mut arguments = BTreeMap::from([
        ("storey", error.storey.to_string()),
        ("floor", error.levels.finished_floor_mm.to_string()),
        ("structural", error.levels.structural_mm.to_string()),
        ("near", format!("{NEAR_FLOOR_MM:.0}")),
    ]);
    let key = match error.problem {
        StoreyTableProblem::StructureAboveFloor => "drawings-storey-structure-above-floor",
        StoreyTableProblem::SameFloorAs(other) => {
            arguments.insert("other", other.to_string());
            "drawings-storey-same-floor"
        }
        StoreyTableProblem::OutsideModel => "drawings-storey-outside-model",
        StoreyTableProblem::NoFloorInModel => "drawings-storey-no-floor-in-model",
    };
    catalog.format(key, &arguments)
}

/// The storeys "Add a storey" puts in an empty table: those found in the
/// model (review 2026-11-09 MF-300: one row of 0/0 threw away the other
/// plans and moved ±0,000), else one at 0; in a table, one storey over the
/// highest.
fn added_storeys(storeys: &[StoreyLevels], found: &[StoreyLevels]) -> Vec<StoreyLevels> {
    if storeys.is_empty() && !found.is_empty() {
        found.to_vec()
    } else {
        vec![added_storey(storeys)]
    }
}

/// The storey the add button makes: the ground floor at 0 in an empty
/// table, else one storey over the highest.
fn added_storey(storeys: &[StoreyLevels]) -> StoreyLevels {
    let top = storeys
        .iter()
        .map(|storey| storey.finished_floor_mm.max(storey.structural_mm))
        .max();
    let floor = top.map_or(0, |top| top.saturating_add(ADDED_STOREY_MM));
    StoreyLevels {
        finished_floor_mm: floor,
        structural_mm: floor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_storey_is_added_at_zero_then_a_storey_higher() {
        assert_eq!(added_storey(&[]).finished_floor_mm, 0);
        let ground = StoreyLevels {
            finished_floor_mm: 50,
            structural_mm: 0,
        };
        assert_eq!(added_storey(&[ground]).finished_floor_mm, 3050);
    }

    /// Review 2026-11-09 MF-300: one row of 0/0 in an empty table.
    #[test]
    fn an_empty_table_is_filled_with_the_storeys_found() {
        let at = |floor| StoreyLevels {
            finished_floor_mm: floor,
            structural_mm: floor,
        };
        let found = [at(200), at(2954)];
        assert_eq!(added_storeys(&[], &found), found);
        assert_eq!(added_storeys(&[], &[]), [at(0)]);
        assert_eq!(added_storeys(&[at(200)], &found), [at(3200)]);
    }
}
