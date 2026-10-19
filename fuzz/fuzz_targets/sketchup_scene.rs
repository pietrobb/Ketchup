//! A SketchUp scene export of any bytes is read or refused, within the
//! fuzzer's time and memory limits.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = ketchup_model::import::inspect_sketchup_scene(bytes);
});
