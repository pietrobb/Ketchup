# R0 v7 Preregistration Supersession Report

- Freeze: `r0-v7`
- Lock SHA-256: `50db67abb3696890006d714394897942c6367991fdfb91b0ae175abff7b361a4`
- Superseded observed lock: `r0-v6` (`4a07111ad9cadb40d9c57f2a8c827b317e2edbe34243143f54cee133c2c74256`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run and subsequent Gate C measurements under `r0-v7`

## Reason for supersession

Gate C introduces the planned localization-ready Windows desktop shell, exact interaction service, and wgpu viewport. `Cargo.lock` now pins the local `ketchup-interaction` and `ketchup-app` packages, eframe/egui 0.32.3, wgpu 25.0.2, and their transitive dependencies. The failed workspace attempt before this freeze stopped inside the preregistration validator before any A0 geometry observation.

The dependency audit identified four licenses not present in the earlier allowlist: BSL-1.0 for the Windows clipboard path, CC0-1.0 for the wgpu parser path, and OFL-1.1 plus Ubuntu-font-1.0 for embedded UI fonts. All are permissive/free inputs. They are now explicitly allowed in `deny.toml`, and `cargo deny check licenses sources` passes. No GPL-family dependency or unknown source was accepted.

## Frozen-contract comparison

The `r0-v7` lock inherits the same 16 paths. Fourteen hashes are byte-identical to `r0-v6`; only `Cargo.lock` and `deny.toml` change. The validator requires the exact eframe, egui, and wgpu versions, the two local Gate C packages, all four newly audited licenses, and a passing cargo-deny license/source audit.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, toolchain evidence, OCCT input, or failure consequence changed. Historical A0 run-006 and Gate B evidence remain immutable and are not rewritten.

## Consequence

A0 must run as immutable `run-007` under this exact lock before Gate C can claim any formal result. Any later byte change to one of the 16 listed inputs invalidates run-007 and requires a new explicit freeze. Gate C still requires three consecutive complete release series on both `HP-DEV-01` and an exact machine satisfying `HP-IGPU-01`; this report does not substitute the development workstation for the mandatory notebook.
