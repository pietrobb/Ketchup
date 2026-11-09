//! The table of storeys in the drawings window (review 2026-11-08 MF-241): the
//! designer gives each storey's finished floor and structure, and the altitude
//! of ±0,000, instead of the plans and level marks being found from the faces.

use eframe::egui;
use ketchup_interaction::LocaleCatalog;
use ketchup_manufacturing::title_block::{SheetSettings, StoreyLevels};
use std::collections::BTreeMap;

/// A storey added over the highest one stands this much higher.
const ADDED_STOREY_MM: i32 = 3000;

/// Shows the rows of `settings.storeys`, lowest first as entered, with a
/// remove button each, an add button, and the altitude of ±0,000.
pub(crate) fn show_storey_table(
    ui: &mut egui::Ui,
    catalog: &LocaleCatalog,
    settings: &mut SheetSettings,
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
            settings.storeys.push(added_storey(&settings.storeys));
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
}
