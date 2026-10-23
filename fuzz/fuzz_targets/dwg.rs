//! A DWG file of any bytes is read as DXF or refused, within the fuzzer's time
//! and memory limits, as the program worker reads it. `dwg_to_dxf` catches
//! acadrust's panics and returns them as errors, so this target replaces
//! libfuzzer-sys's aborting panic hook with a quiet one: a panic that escapes
//! `dwg_to_dxf` still ends the run as a crash, through libfuzzer-sys's own
//! `catch_unwind`.
//!
//! acadrust allocates the sizes a file's header declares (`dwg_to_dxf` says
//! so); in the worker such an allocation fails at the worker's memory limit and
//! the read is refused as `WorkerError::MemoryLimit`. Here an allocation of more than the
//! fuzzer's 2 GiB malloc limit fails the same way instead of ending the run:
//! the allocation error hook turns the failure into a panic `dwg_to_dxf`
//! catches, so the fuzzer goes on past the header into the rest of the reader.
#![no_main]
#![feature(alloc_error_hook)]

use libfuzzer_sys::fuzz_target;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Once;

/// The fuzz job's `-malloc_limit_mb=2048`.
const MALLOC_LIMIT: usize = 2048 << 20;

struct Limited;

// SAFETY: every call goes to `System` unchanged, or fails with null as
// `GlobalAlloc` allows.
unsafe impl GlobalAlloc for Limited {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > MALLOC_LIMIT {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > MALLOC_LIMIT {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > MALLOC_LIMIT {
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Limited = Limited;

static QUIET_CAUGHT_PANICS: Once = Once::new();

fuzz_target!(|bytes: &[u8]| {
    QUIET_CAUGHT_PANICS.call_once(|| {
        std::panic::set_hook(Box::new(|_| {}));
        std::alloc::set_alloc_error_hook(|layout| {
            panic!("an allocation of {} bytes is past the malloc limit", layout.size())
        });
    });
    let _ = ketchup_dwg::dwg_to_dxf(bytes, usize::MAX);
});
