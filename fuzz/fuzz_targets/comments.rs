//! A comments file of any bytes, read without its package as the author reads
//! it, is read or refused, within the fuzzer's time and memory limits.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = ketchup_view_format::Comments::read_unbound(bytes);
});
