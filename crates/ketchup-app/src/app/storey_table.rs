//! The table of storeys in the drawings window (review 2026-11-08 MF-241): the
//! designer gives each storey's finished floor and structure, and the altitude
//! of ±0,000, instead of the plans and level marks being found from the faces.

use eframe::egui;
use ketchup_interaction::LocaleCatalog;
use ketchup_manufacturing::project_drawings::table_ground_floor;
use ketchup_manufacturing::title_block::{SheetSettings, StoreyLevels};
use std::collections::BTreeMap;

/// A storey added over the highest one stands this much higher.
const ADDED_STOREY_MM: i32 = 3000;

/// Shows the rows of `settings.storeys` as entered, each numbered as its plan
/// sheet is, by height, with the choice of the storey that is ±0,000 and a
/// remove button; an add button, and the altitude of ±0,000.
pub(crate) fn show_storey_table(
    ui: &mut egui::Ui,
    catalog: &LocaleCatalog,
    settings: &mut SheetSettings,
) {
    ui.vertical(|ui| {
        if settings.storeys.is_empty() {
            ui.weak(catalog.text("drawings-storeys-found"));
        }
        let numbers = sheet_numbers(&settings.storeys);
        let zero = zero_storey(settings);
        let mut removed = None;
        let mut chosen_zero = None;
        for (index, storey) in settings.storeys.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(catalog.format(
                    "drawings-storey-row",
                    &BTreeMap::from([("storey", numbers[index].to_string())]),
                ));
                ui.add(egui::DragValue::new(&mut storey.finished_floor_mm).suffix(" mm"));
                ui.label(catalog.text("drawings-storey-structural"));
                ui.add(egui::DragValue::new(&mut storey.structural_mm).suffix(" mm"));
                if ui
                    .radio(zero == Some(index), catalog.text("drawings-storey-zero"))
                    .clicked()
                {
                    chosen_zero = Some(storey.finished_floor_mm);
                }
                if ui.button(catalog.text("drawings-storey-remove")).clicked() {
                    removed = Some(index);
                }
            });
        }
        if chosen_zero.is_some() {
            settings.zero_level_mm = chosen_zero;
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
    });
}

/// The number of each storey's plan sheet, by row: the plans go up from the
/// lowest finished floor, whatever order the rows were entered in (review
/// 2026-11-09 MF-301).
fn sheet_numbers(storeys: &[StoreyLevels]) -> Vec<usize> {
    let mut order = (0..storeys.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| storeys[index].finished_floor_mm);
    let mut numbers = vec![0; storeys.len()];
    for (number, index) in order.into_iter().enumerate() {
        numbers[index] = number + 1;
    }
    numbers
}

/// The row whose finished floor is ±0,000: the height set, else the ground
/// floor the drawings choose (review 2026-11-09 MF-301: a basement at the
/// model's origin is ±0,000 unless the ground floor is chosen).
fn zero_storey(settings: &SheetSettings) -> Option<usize> {
    let zero = settings
        .zero_level_mm
        .map_or_else(|| table_ground_floor(&settings.storeys), f64::from);
    settings
        .storeys
        .iter()
        .position(|storey| (f64::from(storey.finished_floor_mm) - zero).abs() < 0.5)
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

    /// Review 2026-11-09 MF-301: with the basement at the model's origin the
    /// table did not say which storey was ±0,000, and numbered its rows as
    /// entered while the plan sheets go by height.
    #[test]
    fn rows_are_numbered_as_their_plans_and_show_the_storey_at_zero() {
        let storey = |floor| StoreyLevels {
            finished_floor_mm: floor,
            structural_mm: floor - 50,
        };
        let mut settings = SheetSettings {
            storeys: vec![storey(3000), storey(0), storey(6000)],
            ..SheetSettings::default()
        };
        assert_eq!(sheet_numbers(&settings.storeys), [2, 1, 3]);
        assert_eq!(
            zero_storey(&settings),
            Some(1),
            "the basement at the origin"
        );
        settings.zero_level_mm = Some(3000);
        assert_eq!(zero_storey(&settings), Some(0), "the ground floor chosen");
        settings.zero_level_mm = Some(1500);
        assert_eq!(zero_storey(&settings), None, "no storey is at ±0,000");
    }
}
