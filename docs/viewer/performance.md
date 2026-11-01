# Kečup Viewer performance

Measured numbers for the three reference packages and the proposed budget.
The budget is a proposal until the owner accepts it; no speed-up work starts
before that.

## Reference packages

| Package | Size | Components | What it stands for |
|---|---|---|---|
| HPL kitchen (`viewer/samples/kitchen.ketchup-view`) | 0.6 MB | 20 | a typical single-room order |
| Nested furniture set (`examples/wardrobe-chair-table.ketchup`) | 1.0 MB | 82 | nested assemblies with joinery |
| Instanced house (`examples/house-project.ketchup`) | 5.4 MB | 1854 | a large instanced model |

The kitchen package is committed. The other two are exported the way
File > Export for Viewer does it, by
[viewer_reference_packages.rs](../../crates/ketchup-app/tests/viewer_reference_packages.rs),
which the nightly build runs so both stay exportable:

```
cargo build --release --locked -p ketchup-scheduler --bin ketchup-program-worker
set KETCHUP_TEST_REPORT_DIR=C:\some\dir
cargo test -p ketchup-app --release --test integration viewer_reference_packages -- --ignored --nocapture
```

## How it is measured

`ketchup-viewer --benchmark <package>` reads the package, uploads it to the
GPU, orbits it for a first frame and 300 more at 1600×900, picks components
and switches scenes. Frames are drawn offscreen: without present, vsync,
compositor or the user interface, so a window draws fewer frames per second
than the orbit rate below. [benchmark-viewer.ps1](../../scripts/windows/benchmark-viewer.ps1)
runs it on Windows and adds the peak working set.
[benchmark-android-viewer.ps1](../../scripts/windows/benchmark-android-viewer.ps1)
runs the installed Android Viewer on the connected device or emulator; the
Viewer adds its peak resident memory (`VmHWM`).

- **open**: reading the package and building the model, before the GPU.
- **GPU upload**: building the GPU buffers of all components.
- **first frame**: the first orbit frame, which also builds the pipeline
  and compiles the shaders; reported on its own.
- **orbit**: offscreen frames per second from the median frame time of the
  300 frames after the first; worst is the slowest of them.
- **pick**: median time to find the component under the pointer, over the
  positions of a 9 × 9 grid that hit a component; a miss returns at once
  and is not counted.
- **scene switch**: slowest switch between saved scenes.
- **peak memory**: on Windows the peak working set, which includes the
  graphics driver's allocations; on Android the resident memory of the app
  process. The two are not directly comparable.

## Measured on 2026-10-31

Viewer 0.3.0, release build. PC: CENTAURY, AMD Ryzen 9 5900X, 64 GB RAM,
AMD Radeon RX 6800 XT. Other Kečup instances were running during the
measurement, so worst frames and Windows orbit rates vary between runs.
These runs predate the measurement above: their worst frame includes the
first frame, and their pick median includes positions that hit nothing, so
the worst frames may be too high and the picks too fast. They are to be
measured again.

### Windows 10, Direct3D 12

| Package | open | GPU upload | orbit | worst frame | pick | scene switch | peak memory |
|---|---|---|---|---|---|---|---|
| Kitchen | 1.9 ms | 54.6 ms | 2578 fps | 9.6 ms | 0.001 ms | 0.4 ms | 213 MB |
| Furniture set | 4.8 ms | 48.1 ms | 1784 fps | 42.6 ms | 0.003 ms | 0.9 ms | 215 MB |
| House | 34–57 ms | 283–348 ms | 118–190 fps | 88–108 ms | 0.042 ms | 5–18 ms | 840 MB |

The house row is the range of three runs.

### Android emulator, OpenGL ES

Emulator `Ketchup_Viewer_Bench` (sdk_gphone64_x86_64, Android 16, host GPU
through the emulator's OpenGL ES translator) on the same PC. An emulator is
not a phone; numbers from a real arm64 phone are still missing.

| Package | open | GPU upload | orbit | worst frame | pick | scene switch | peak memory |
|---|---|---|---|---|---|---|---|
| Kitchen | 3.4 ms | 119.7 ms | 706 fps | 63.9 ms | 0.010 ms | 1.4 ms | 135 MB |
| Furniture set | 10.2 ms | 190.0 ms | 537 fps | 36.3 ms | 0.016 ms | 2.6 ms | 138 MB |
| House | 70.7 ms | 4782.5 ms | 60 fps | 500.3 ms | 0.043 ms | 21.7 ms | 172 MB |

## Proposed budget

Limits for the release build on a mid-range PC and on a phone. "House" means
a package of the house's class: up to 6 MB and 2000 components.

| Measure | Kitchen and furniture set | House | Today |
|---|---|---|---|
| open | ≤ 50 ms | ≤ 150 ms | met everywhere |
| open + GPU upload, until the first image | ≤ 500 ms | ≤ 1 s | met on Windows; the house on Android takes 4.9 s |
| orbit at 1600×900, median | ≥ 60 fps | ≥ 60 fps | met; the house on Android is exactly at the limit |
| worst orbit frame after the first image | ≤ 100 ms | ≤ 150 ms | the house misses it on Android (500 ms) |
| pick, median | ≤ 5 ms | ≤ 5 ms | met everywhere |
| scene switch, worst | ≤ 50 ms | ≤ 100 ms | met everywhere |
| peak memory, Windows | ≤ 400 MB | ≤ 1 GB | met |
| peak memory, Android | ≤ 300 MB | ≤ 500 MB | met |

If the owner accepts this budget, the work it implies is the Android GPU
upload and worst frame of the house. Their cause has not been investigated.
