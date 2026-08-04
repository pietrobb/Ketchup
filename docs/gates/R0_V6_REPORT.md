# R0 v6 Preregistration Supersession Report

- Freeze: `r0-v6`
- Lock SHA-256: `4a07111ad9cadb40d9c57f2a8c827b317e2edbe34243143f54cee133c2c74256`
- Superseded observed lock: `r0-v5` (`0a47c3d0b6d6a24201f64d221b8850892926f7786a3c23ba117c13df881c3d58`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run under `r0-v6`

## Reason for supersession

Gate B introduces the local `ketchup-scheduler` workspace package. Cargo therefore adds one local package record to `Cargo.lock`, referencing the already-present `ketchup-core` and `ketchup-exact` packages. No external dependency package, version, source, checksum, feature selection, or license changes.

The prior `r0-v5` lock and immutable A0 `run-005` remain valid historical evidence for commit `f0a0cf3afa9df45682fe8723dacc99cb8e153058`; they are not rewritten. The current working tree cannot claim A0 integrity under that old dependency lock, so `r0-v6` is frozen before a replacement observation and A0 must pass again before Gate B closes.

## Frozen-contract comparison

The `r0-v6` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v5`; only `Cargo.lock` changes from `a2f103264259b56e39e6b9a60243869f7ff4612faa311b2b7d28fb6a1cb66351` to `fab3ca695f64cc4c89a8fc87bf57e87e1542cb19b0cf32dc43a22ee4d623636e`.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, toolchain evidence, external dependency, or failure consequence changed. The validator requires the new lock to contain the local scheduler package with only `ketchup-core` and `ketchup-exact` dependencies.

## Consequence

A0 must run as immutable `run-006` under this exact lock before Gate B receives a final GO. Any later byte change to one of the 16 listed inputs invalidates `run-006` and requires a new explicit freeze; it may not be hidden by bypassing the validator or rewriting earlier evidence.
