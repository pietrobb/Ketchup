# R0 v9 Preregistration Supersession Report

- Freeze: `r0-v9`
- Lock SHA-256: `da0dbcd3b3daf845a83f6a708a528c7cdcbf8e0155d1d93bfbb9637c539a7b25`
- Superseded observed lock: `r0-v8` (`ad2f6ff3c89043d1491b02de1e0af390a3211ae4844cb37f32bacb3956b7c456`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run and subsequent Gate C measurements under `r0-v9`

## Reason for supersession

The first explicitly non-gating Gate C core diagnostic used the existing `ketchup-interaction` package from the `ketchup-scheduler` measurement binary. This local package edge changed `Cargo.lock` after A0 run-008. The diagnostic remains immutable evidence but cannot authorize a formal Gate C result.

No dependency version or external source changed. A replacement freeze is still mandatory because the lock file is a preregistered input. A0 must pass again before any formal Gate C measurement.

## Frozen-contract comparison

The `r0-v9` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v8`; only `Cargo.lock` changes. The validator requires the exact local scheduler-to-interaction edge, the Direct3D 12-only application dependency, pinned eframe/egui/wgpu versions, and a passing cargo-deny license/source audit.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, license policy, toolchain evidence, OCCT input, or failure consequence changed. Historical A0 and Gate C diagnostic evidence is not rewritten.

## Consequence

A0 must run as immutable `run-009` under this exact lock before Gate C can claim a formal result. Gate C then requires three consecutive complete release series on both `HP-DEV-01` and the first qualifying, preregistered `HP-IGPU-01` machine.
