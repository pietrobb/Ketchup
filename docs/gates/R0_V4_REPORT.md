# R0 v4 Preregistration Supersession Report

- Freeze: `r0-v4`
- Lock SHA-256: `a439287c4beac41c5cb844556a77e90f858fcbe28bde2ba4a3bb3cad8171b5ce`
- Superseded historical lock: `r0-v3` (`f824876fc5b98279212f1f3e926a64edc17f7439a48df1ccb3239be5f6fa4afb`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run under `r0-v4`

## Reason for supersession

The staged-byte audit after historical `run-004` found that the two specific `-text` rules preceded and were overridden by the generic JSON rule. The rules now follow the generic rule, so the two frozen CRLF toolchain manifests remain byte-identical in Git and after checkout. `Cargo.lock` remains explicitly pinned to LF.

## Frozen-contract comparison

The `r0-v4` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v3`; only `.gitattributes` changes from `61027be410ee223a1d44cf2baa010affee439512feb98a94f0725e6087fbc9d9` to `87214907a8d7012df8fd8b9ea495e9a0d3f27aac29bab1a337985ecf5e2f07f7`.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, dependency, toolchain manifest, or failure consequence changed. Historical `r0-v3` evidence remains unchanged and is not the active result. A0 must be rerun under `r0-v4`, and any listed byte change after that observation fails the replacement run.
