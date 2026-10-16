//! A Viewer package of any bytes, its saved comments included, is read or
//! refused, within the fuzzer's time and memory limits.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = ketchup_view_format::Package::read_with_comments(bytes);
});
