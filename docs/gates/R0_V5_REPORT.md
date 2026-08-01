# R0 v5 Preregistration Supersession Report

- Freeze: `r0-v5`
- Lock SHA-256: `0a47c3d0b6d6a24201f64d221b8850892926f7786a3c23ba117c13df881c3d58`
- Superseded unobserved lock: `r0-v4` (`a439287c4beac41c5cb844556a77e90f858fcbe28bde2ba4a3bb3cad8171b5ce`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run under `r0-v5`

## Reason for supersession

The pre-observation staged-byte audit found that `-text` alone did not clear the `eol=lf` attribute inherited from the generic JSON rule. The two specific rules now use `-text !eol`, preserving the already-frozen CRLF toolchain manifests byte-for-byte in Git and after checkout. `Cargo.lock` remains explicitly pinned to LF. No A0 observation was made under `r0-v4`.

## Frozen-contract comparison

The `r0-v5` lock inherits the same 16 paths. Fifteen hashes are byte-identical to `r0-v4`; only `.gitattributes` changes from `87214907a8d7012df8fd8b9ea495e9a0d3f27aac29bab1a337985ecf5e2f07f7` to `fd18fd3c583d7c48c260660e985f5998b6db7eac60abafd364fcd043cc6b56e5`.

No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, dependency, toolchain manifest, or failure consequence changed. A0 must be rerun under `r0-v5`, and any listed byte change after that observation fails the replacement run.
