# Gate C Replacement Pre-Observation Freeze Progress 044

**Status: R0 v13 freezes the repaired viewport and unrestricted orbit before any replacement observation; preregistration and all workspace tests pass, while Gate C remains open without HP-IGPU-01 evidence**

- Freeze UTC: `2026-08-02T06:47:28.2309770Z`
- Active freeze: `r0-v13`
- Superseded historical freeze: `r0-v12`
- R0 v13 lock SHA-256: `b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01`
- Repaired application source SHA-256: `69ee2729fbc371924eaa94aa45544f6257a0197438e732a04fbcb1a3b5f9d977`
- Gate C build-input tree SHA-256: `de8592b10b5ed88d2ae7cf8394c127d3d7ca1ea8b22830911cc28a8fbdca84bb` across 29 files
- HP-IGPU-01 runner SHA-256: `8bade6a87253ebdac41f4cf9f92acc11a84ddec7e907cce9e2ad16af7cdcf564`
- R0 v13 validator SHA-256: `fe7c7f2006cae767f938f26c7e97327ddc995ce40bcb21b3473751b0f0cc62a3`
- Testable assumption: `A7`

## Authorized repair

Operator usability review occurred before any HP-IGPU-01 qualification or formal observation. It found incorrect cuboid occlusion and selected-face presentation, disagreement between camera and exact picking, undiscoverable Push/Pull interaction, and an artificial orbit-pitch limit. The repaired product now renders an opaque cuboid, aligns picking with the camera, exposes localized Push/Pull guidance, and allows orbit through both poles without a pitch clamp.

R0 v13 supersedes r0-v12 only for future observations. It preserves all 18 inherited R0 lock paths except `crates/ketchup-app/src/lib.rs`; every threshold, corpus, expected outcome, hardware profile, oracle, consequence, license policy, toolchain input, OCCT input, and NAV harness hash is unchanged. The complete product, test, and locale build-input tree is separately bound by the runner and validator. Historical r0-v9 through r0-v12 evidence was not edited and cannot certify the repaired executable.

## Frozen provenance

The active Gate A0 wrapper now requires the exact r0-v13 lock. The HP-IGPU-01 runner requires the exact lock, repaired application source, unchanged NAV/core measurement sources, 29-file build-input tree, pinned Rust/MSVC tools, and complete OCCT install-tree fingerprint. It reserves fresh r0-v13 fingerprint, attempt-claim, stage-log, metric, run-manifest, and clean-build namespaces and retains immutable attempt sealing.

The R0 v13 validator confirms the r0-v12 to r0-v13 lineage, the single changed inherited lock path, runner hash, portable build provenance, Direct3D 12 physical-adapter binding, unchanged Gate C thresholds, dependency policy, R0 v13 GO report, and the immutable passing r0-v12 portable NAV summary.

## Verification

| Check | Result |
|---|---|
| R0 v13 lock hash and `not_started` state | **PASS** |
| Runner and validator PowerShell parsing | **PASS** |
| Portable build-provenance and attempt-sealing self-test | **PASS** |
| R0 v13 preregistration validator | **PASS** |
| Dependency license/source audit | **PASS** |
| `cargo fmt --all -- --check` | **PASS** |
| `cargo test --workspace --all-targets` in an isolated target | **PASS** - 32 tests |
| Recomputed 29-file build-input tree | **PASS** - exact frozen hash |
| Isolated test target cleanup | **PASS** |
| HP-IGPU-01 r0-v13 fingerprint, claim, and run manifest | **ABSENT** |
| Gate C terminal reports | **ABSENT** |

## Done-check

| Criterion | Result |
|---|---|
| `file_exists:artifacts/gate-c/report.md` | **FAIL** - correctly absent |
| `file_contains:artifacts/gate-c/report.md::GO` | **NOT EVALUABLE** - no admissible HP-IGPU-01 run exists |
| `cargo test --workspace --all-targets` | **PASS** |

## Next action

Generate fresh HP-DEV-01 core and NAV references from one clean r0-v13 build, seal their provenance with the new lock, build-input, runner, and validator hashes, then update and freeze the Gate C report writer and transfer preflight. Keep final Gate C closure waiting for the first genuinely qualifying HP-IGPU-01 notebook; the available Windows 11 desktop remains valid only for HP-DEV-01 reference and usability work.
