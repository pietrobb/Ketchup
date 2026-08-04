# Gate C Implementation Progress 002

**Status: R0 v9 and A0 replacement PASS; HP-DEV-01 core PASS; NAV and HP-IGPU-01 measurements pending**

## R0 v9 and replacement A0

- R0 v9 freezes the existing Gate C core-runner dependency edge before formal observation. Fifteen of sixteen inherited lock entries are byte-identical to v8; only `Cargo.lock` changed.
- R0 v9 lock SHA-256: `da0dbcd3b3daf845a83f6a708a528c7cdcbf8e0155d1d93bfbb9637c539a7b25`.
- The v9 validator passed without changing thresholds, corpora, expected outcomes, hardware profiles, or consequences.
- Immutable A0 `run-009` passed: 10,000/10,000 structure-aware fuzz calls, 24/24 Guaranteed identity/history outcomes, 3/3 STEP fixtures, zero silent invalid shapes, and zero silent wrong identities.

## HP-DEV-01 formal core series

All three consecutive release series passed under R0 v9. Every series preserved 1,000 edit samples, 2,000 pick/snap samples, 100 long-job cancellation samples, the environment fingerprint, and one shared authoritative geometry for 10,000 occurrences.

| Series | Edit p95 ms | Pick/snap p95 ms | Navigation block max ms | Cancel p95 ms | Digest match | Wrong identity | Data loss |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 2.3300 | 2.4201 | 2.8486 | 1.0041 | 100% | 0 | 0 |
| 2 | 2.2629 | 2.4179 | 3.4094 | 1.0544 | 100% | 0 | 0 |
| 3 | 2.2659 | 2.4167 | 3.1318 | 1.0594 | 100% | 0 | 0 |

The separate immutable provenance packet records Git HEAD, dirty state, source hashes, release executable hashes, and the hashes of all three raw series files.

## Navigation harness

- `ketchup-gate-c-nav` automates the exact frozen 30-run series: 10 seconds of excluded warmup plus 30 seconds of measurement per run.
- The release Direct3D 12 surface renders 10,000 occurrences as 20,000 visible tessellated triangles from one shared authoritative geometry representation.
- Orbit, pan, zoom, and ephemeral transform preview are exercised continuously.
- Every frame interval and input-to-preview sample is retained. Nearest-rank p95/p99 is computed over all individual samples without per-run averaging or outlier removal.
- The output records environment, Git revision/dirty state, per-run sample cardinalities, raw samples, and frozen lock hash; existing output paths are never overwritten.
- App tests and Clippy pass, the release runner builds, and an eight-second Direct3D 12 startup smoke remained responsive.
- The complete workspace test suite, workspace Clippy, formatting check, R0 v9 validator, and diff check pass after the Gate C changes.

## Remaining Gate C work

1. Execute three complete `QC-C-NAV-01` release series on `HP-DEV-01` (approximately 20 minutes each).
2. Record the first qualifying `HP-IGPU-01` fingerprint before any observation.
3. Execute three complete core and NAV series on that exact notebook.
4. Publish `artifacts/gate-c/report.md` with GO only if every required profile and threshold passes; otherwise publish the preregistered NO-GO evidence.
