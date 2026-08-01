# R0 v2 Preregistration Supersession Report

- Freeze: `r0-v2`
- Lock SHA-256: `72cd515b0ca87a7dd9a685fbcda1a8521b467ed56fc4a0b717e50bc305eb195e`
- Superseded historical lock: `r0-v1` (`213b56e5bb50cd6c82afdbdd4067a002a92ac2f56714f7dd3272f3f8f1e1e6be`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one formal A0 run under `r0-v2`

## Reason for supersession

The G4 exact-backend facade added the already selected and pinned `cxx 1.0.198` implementation dependency after the `r0-v1` freeze. Cargo therefore updated `Cargo.lock` before any formal A0 observation. The `r0-v1` lock is retained unchanged and is not represented as a passing A0 run.

## Frozen-contract comparison

The `r0-v2` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v1`; only `Cargo.lock` changes from `2e127583a6eebb3aba1ae4d817c0fa702ab732f88153f15e3d712abf6a3a7029` to `a2f103264259b56e39e6b9a60243869f7ff4612faa311b2b7d28fb6a1cb66351`.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, or failure consequence changed. Any listed byte change after the first formal A0 observation fails the `r0-v2` run.
