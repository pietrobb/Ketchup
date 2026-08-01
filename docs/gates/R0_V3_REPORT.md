# R0 v3 Preregistration Supersession Report

- Freeze: `r0-v3`
- Lock SHA-256: `f824876fc5b98279212f1f3e926a64edc17f7439a48df1ccb3239be5f6fa4afb`
- Superseded historical lock: `r0-v2` (`72cd515b0ca87a7dd9a685fbcda1a8521b467ed56fc4a0b717e50bc305eb195e`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run under `r0-v3`

## Reason for supersession

The pre-commit Git index audit found that the generic JSON line-ending rule would normalize the two frozen CRLF toolchain manifests to LF in the repository. Their staged bytes would therefore differ from the hashes validated by `r0-v2`, making a clean checkout non-reproducible. The checkout policy now preserves those two evidence files byte-for-byte and pins `Cargo.lock` to LF.

## Frozen-contract comparison

The `r0-v3` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v2`; only `.gitattributes` changes from `d56ae54f673414478c855870dcc03dfb79becbc0e3d5bf642588b8fce00850a6` to `61027be410ee223a1d44cf2baa010affee439512feb98a94f0725e6087fbc9d9`.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, dependency, toolchain manifest, or failure consequence changed. Historical `r0-v2` A0 evidence remains unchanged. A0 must be rerun under `r0-v3`, and any listed byte change after that observation fails the replacement run.
