//! Review 2026-10-29 CD-63: writing a changed drawing back took ~16 times the size of its
//! DXF, and a drawing that declares no unit was read up to ten times before it was
//! refused. The memory the write takes is counted here by the allocator of this test, a
//! binary of its own, as the counts are of the whole process.
// The allocator only counts what it passes on to the system's.
#![allow(unsafe_code)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use ketchup_model::import::{
    DxfImportOptions, ImportLengthUnit, read_dxf_underlay, write_edited_dxf,
};
use ketchup_model::underlay::edits::UnderlayEdits;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        TOTAL.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's layout is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` was allocated by `System` with this layout, in `alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The memory `run` takes above what is held before it, at most and in all.
fn counted<T>(run: impl FnOnce() -> T) -> (T, usize, usize) {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let total = TOTAL.load(Ordering::Relaxed);
    let value = run();
    let peak = PEAK.load(Ordering::Relaxed) - before;
    (value, peak, TOTAL.load(Ordering::Relaxed) - total)
}

/// A drawing of `count` lines with handles, in millimetres or with no unit.
fn drawing(count: usize, unit: bool) -> Vec<u8> {
    let mut text = String::from("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n");
    if unit {
        text.push_str("9\n$INSUNITS\n70\n4\n");
    }
    text.push_str("9\n$HANDSEED\n5\nFFFFFF\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for index in 0..count {
        let x = (index % 100) as f64 * 150.0;
        let y = (index / 100) as f64 * 90.0;
        text.push_str(&format!(
            "0\nLINE\n5\n{:X}\n330\n1F\n100\nAcDbEntity\n8\nwalls\n100\nAcDbLine\n\
             10\n{x}\n20\n{y}\n30\n0.0\n11\n{}\n21\n{}\n31\n0.0\n",
            index + 0x100,
            x + 120.0,
            y + 45.5,
        ));
    }
    text.push_str("0\nENDSEC\n0\nEOF\n");
    text.into_bytes()
}

/// One test, as the counts are of the whole process: tests run side by side.
#[test]
fn a_changed_drawing_takes_no_more_than_its_import_and_is_read_at_most_twice() {
    a_changed_drawing_is_written_in_what_its_import_takes();
    a_drawing_without_a_unit_is_not_read_in_every_unit_before_it_is_refused();
}

fn a_changed_drawing_is_written_in_what_its_import_takes() {
    let source = drawing(60_000, true);
    let options = DxfImportOptions::new(None);
    let (read, read_peak, _) = counted(|| read_dxf_underlay(&source, options).unwrap());
    let lines = read.lines;
    let mut edits = UnderlayEdits::default();
    edits.removed_objects.insert(7);
    edits
        .moved_objects
        .insert(11, [[1.0, 0.0, 250.0], [0.0, 1.0, 0.0]]);
    let (written, peak, _) = counted(|| write_edited_dxf(&source, options, &lines, &edits));
    let written = written.unwrap();
    assert!(written.len() > source.len() / 2);
    // What the import of a drawing takes, its export does not exceed: a drawing that
    // imports, exports.
    let times = peak as f64 / read_peak as f64;
    assert!(
        times < 1.1,
        "the write took {times:.1} times what the import of the {} bytes took",
        source.len()
    );
}

fn a_drawing_without_a_unit_is_not_read_in_every_unit_before_it_is_refused() {
    let source = drawing(5_000, false);
    let options = DxfImportOptions::new(Some(ImportLengthUnit::Millimetre));
    let (read, _, one_read) = counted(|| read_dxf_underlay(&source, options).unwrap());
    // Lines read from another drawing are found in none of the units.
    let other = read_dxf_underlay(&drawing(4_999, false), options)
        .unwrap()
        .lines;
    drop(read);
    let (refused, _, total) =
        counted(|| write_edited_dxf(&source, options, &other, &UnderlayEdits::default()));
    assert!(refused.is_err());
    let reads = total as f64 / one_read as f64;
    assert!(reads < 4.0, "the write took what {reads:.1} reads take");
}
