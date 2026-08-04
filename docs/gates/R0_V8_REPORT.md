# R0 v8 Preregistration Supersession Report

- Freeze: `r0-v8`
- Lock SHA-256: `ad2f6ff3c89043d1491b02de1e0af390a3211ae4844cb37f32bacb3956b7c456`
- Superseded observed lock: `r0-v7` (`50db67abb3696890006d714394897942c6367991fdfb91b0ae175abff7b361a4`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run and subsequent Gate C measurements under `r0-v8`

## Reason for supersession

The first non-gating release application startup smoke after A0 run-007 correctly exposed a configuration failure before any Gate C observation: eframe's wgpu integration had no backend feature compiled for Windows and exited with code 101. The application now depends directly on the already-pinned wgpu 25.0.2 with default features disabled and only `dx12` enabled. A repeated five-second release smoke created a Direct3D 12 window and remained responsive.

This feature closure changes `Cargo.lock`, so run-007 remains immutable historical evidence but cannot authorize later observations. The corrected executable is frozen before its replacement A0 run and before any formal Gate C measurement.

## Frozen-contract comparison

The `r0-v8` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v7`; only `Cargo.lock` changes. The validator requires the exact Direct3D 12-only dependency declaration, the pinned eframe/egui/wgpu versions, the Windows allocator closure, and a passing cargo-deny license/source audit.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, license policy, toolchain evidence, OCCT input, or failure consequence changed. Historical A0 runs and Gate B evidence are not rewritten.

## Consequence

A0 must run as immutable `run-008` under this exact lock before Gate C can claim a formal result. Gate C then requires three consecutive complete release series on both `HP-DEV-01` and an exact machine satisfying `HP-IGPU-01`; the successful development-workstation smoke does not substitute for the mandatory notebook.
