# ADR 0009: Correctly Rounded Math for Exact Results

- Status: Accepted — project owner, 2026-10-11
- Date: 2026-10-11
- Decision owner: Project owner
- Accountable approver: Project owner
- Evidence: `third_party/crmath/`, `scripts/macos/build-occt.sh`, `scripts/windows/build-occt.ps1`, `crates/ketchup-exact/build.rs`, `crates/ketchup-exact/tests/crmath.rs`

## Context

Documents pin exact results bit for bit: an imported STEP body stores its volume, area, bounds and `result_fingerprint`, and evaluation refuses a body whose recomputed evidence differs in any bit. On macOS two of the 17 parts of `examples/hettich-quadro-v6-drawer.ketchup` were refused because a volume differed in the last bit (`…39206` against `…39207`).

The cause is the platform math library. OCCT and the Rust exact path call `sin`, `cos`, `pow`, `atan2`, … from the system libm; Apple's and Microsoft's implementations are accurate to about one unit in the last place but round differently for some arguments (about 3 % of the `sin`/`cos` calls while importing that drawer). Neither is correctly rounded, so neither platform's results can be reproduced on the other, and Microsoft's x64 CRT switches to FMA3 code paths on CPUs that have them, which can change results between machines (`_set_FMA3_enable`).

## Decision

1. Every binary that computes exact results uses correctly rounded math: CORE-MATH (MIT, pinned commit in `third_party/crmath/CMakeLists.txt`) behind a small shim that defines the standard names (`sin`, `pow`, …, and Apple's `__sincos_stret`/`__exp10`).
2. The OCCT build scripts build that static library first, link it whole into every OCCT library and compile OCCT with `-fno-builtin-*` for those functions (macOS). A gate fails the build if any OCCT library still imports one of them from the system. On Windows the library is built with the ClangCL toolset of the pinned Visual Studio and linked with `/WHOLEARCHIVE`.
3. `ketchup-exact/build.rs` links the same library whole into every Rust binary that uses the exact backend, so Rust's `f64::sin` etc. resolve to it too.
4. `crates/ketchup-exact/tests/crmath.rs` checks 20 000 arguments per function against digests computed independently with MPFR. The same digests must hold on every platform.
5. Functions IEEE 754 already defines exactly (`sqrt`, `fma`, `fmod`, `ldexp`, `modf`, `nextafter`, `remainder`) are not replaced. The tolerance profile and backend fingerprint strings are unchanged; evidence that depended on a platform libm is recomputed once.

## Consequences

- Exact results no longer depend on the platform libm, so a document evaluated on Windows evaluates on macOS and back.
- Stored evidence that was produced with Microsoft's libm and differs from the correctly rounded value had to be recomputed (see the commit that introduced this ADR).
- The Windows OCCT runtime must be rebuilt with `scripts/windows/build-occt.ps1`, which now also needs the Visual Studio component `Microsoft.VisualStudio.Component.VC.Llvm.Clang`. Until then `ketchup-exact` does not build on Windows (`missing …ketchup_crmath.lib`). The clang-cl hash printed by the script still has to be frozen like the other native tools.
- Without hardware FMA (built for baseline x86-64) CORE-MATH falls back to the CRT's `fma`, which is slower; whether that is measurable in OCCT on Windows is not yet known.
