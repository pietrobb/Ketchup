//! A DXF drawing of any bytes, read as an underlay, is written back with its objects
//! removed, moved, turned, mirrored, stretched and copied and a text drawn on it, as the
//! input's last byte chooses, or the edit is refused; what is written reads again
//! (review 2026-10-29 TST-61). All within the fuzzer's time and memory limits.
#![no_main]

use ketchup_model::import::{DxfImportOptions, ImportLengthUnit, read_dxf_underlay};
use ketchup_model::underlay::drawn::{AddedObject, DrawnShape};
use ketchup_model::underlay::edits::UnderlayEdits;
use libfuzzer_sys::fuzz_target;

/// A move, a mirror (as AutoCAD's Mirror leaves extrusion -Z), a turn and a halving.
const MAPS: [[[f64; 3]; 2]; 4] = [
    [[1.0, 0.0, 250.0], [0.0, 1.0, -40.0]],
    [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    [[0.0, -1.0, 10.0], [1.0, 0.0, 20.0]],
    [[0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
];

fuzz_target!(|input: &[u8]| {
    // The last byte, so a DXF seed ending in a line break is still a drawing without it.
    let Some((&choice, source)) = input.split_last() else {
        return;
    };
    let options = DxfImportOptions::new(Some(ImportLengthUnit::Millimetre));
    let Ok(underlay) = read_dxf_underlay(source, options) else {
        return;
    };
    // Only objects of the drawing itself are edited, not those an insert placed.
    let top_level = (0u32..)
        .zip(&underlay.lines.objects().objects)
        .filter(|(_, object)| object.parent.is_none())
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let Some(&stretched) = top_level.get(usize::from(choice >> 6) % top_level.len().max(1)) else {
        return;
    };
    let mut edits = UnderlayEdits::default();
    // One object made shorter and a longer copy of it, as Trim and Extend do to a line.
    edits.stretched_objects.insert(stretched, MAPS[3]);
    edits.added.push(AddedObject::Stretched {
        of: stretched,
        map: [[2.0, 0.0, 0.0], [0.0, 2.0, 500.0]],
    });
    // Two bits of `choice` for each of three others: left, taken out, moved or copied.
    let others = top_level.iter().filter(|&&object| object != stretched);
    for (index, &object) in others.take(3).enumerate() {
        let map = MAPS[(index + usize::from(choice)) % MAPS.len()];
        match choice >> (index * 2) & 3 {
            0 => {}
            1 => {
                edits.removed_objects.insert(object);
            }
            2 => {
                edits.moved_objects.insert(object, map);
            }
            _ => edits.added.push(AddedObject::Copy { of: object, map }),
        }
    }
    if let Some(layer) = underlay.layers.first() {
        edits.added.push(AddedObject::Drawn {
            layer: layer.clone(),
            shape: DrawnShape::Text {
                anchor_mm: [10.0, 20.0],
                height_mm: 250.0,
                rotation_deg: 30.0,
                text: "1.01 Kuchyňa".into(),
            },
        });
    }
    let written = ketchup_model::import::write_edited_dxf(source, options, &underlay.lines, &edits);
    if let Ok(written) = written {
        let _ = read_dxf_underlay(&written, options);
    }
});
