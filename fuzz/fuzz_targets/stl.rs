//! A binary or ASCII STL mesh of any bytes is read or refused, within the
//! fuzzer's time and memory limits.
#![no_main]

use ketchup_model::import::{ImportLengthUnit, ImportUnitAuthority, ImportUnitDecision};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let units = ImportUnitDecision::new(
        ImportLengthUnit::Millimetre,
        ImportUnitAuthority::UserDeclared,
    );
    let _ = ketchup_model::import::parse_stl(bytes, units);
});
