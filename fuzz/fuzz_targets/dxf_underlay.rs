//! A DXF drawing of any bytes is read as an underlay or refused, within the
//! fuzzer's time and memory limits.
#![no_main]

use ketchup_model::import::{DxfImportOptions, ImportLengthUnit};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let options = DxfImportOptions::new(Some(ImportLengthUnit::Millimetre));
    let _ = ketchup_model::import::read_dxf_underlay(bytes, options);
});
