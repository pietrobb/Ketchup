# Gate C Closure-Report Contract 032

**Status: the post-observation decision path is now fail-closed and frozen before HP-IGPU-01 observation; the physical-notebook blocker remains**

- Contract freeze UTC: `2026-08-02T04:34:11.4133175Z`
- Active freeze: `r0-v12`
- Testable assumption: `A7`
- Report validator: `scripts/windows/write-gate-c-report.ps1`
- Report validator SHA-256: `6cea6c56d9cfbe0fa913b876ea54bbcb5b1a0ec1c125ad6829699e337f9c7d4c`
- Runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 validator SHA-256: `2efd7ab90ff199c2cd9669fbb603af6ba1db58b1ef264e4d126baed5564c0c56`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- HP-DEV-01 core provenance SHA-256: `d24a34e50cfe910aa30f702344ecb951ad96dcdf578917ed3b58e83b3a50d090`
- HP-DEV-01 NAV provenance SHA-256: `51de8f7bfdfb9697a66de1edec65d7bb0c447c42ed4846a9834ccabefae983da`

## Progress artifact

A separate post-observation validator now prevents the final Gate C report from depending on manual evidence interpretation. It pins and rehashes the r0-v12 lock, frozen runner, R0 validator, and both sealed HP-DEV-01 reference sets. It then requires one consistent identity chain across the immutable HP-IGPU-01 fingerprint, exclusive attempt claim, terminal run manifest, build-input tree, OCCT tree, runner, every stage, every recorded evidence hash, and all six formal metric files.

A `PASS` manifest can become `**Decision: GO**` only when it contains exactly the ordered release-build, three core, and three navigation stages with successful exits. Every core metric must match the frozen scene, warm-up, sample-count, exact-identity, action-digest, cancellation, navigation-block, and data-loss contracts. Every NAV metric must bind to the fingerprinted integrated Direct3D 12 adapter and match the 30-run timing, geometry-sharing, triangle, and sample-cardinality contracts. The validator independently recomputes nearest-rank p95/p99 values and the navigation maximum from raw samples before applying the frozen thresholds.

A sealed measured `FAIL` maps to `NO-GO`; `INFRASTRUCTURE_INVALID` remains neither a pass nor a product failure. The generated report uses exclusive creation and refuses to overwrite historical evidence.

## Contract tests

A temporary synthetic evidence directory was built from copies of the sealed HP-DEV-01 raw metrics with only the profile and adapter identity changed for test purposes. These files were never placed at the real HP-IGPU-01 evidence paths, were not treated as observations, and were deleted after each test.

| Test | Result |
|---|---|
| Complete internally sealed seven-stage PASS set | **PASS** — validator accepted the evidence and reached the GO report path |
| Immutable report creation | **PASS** — a temporary report containing `**Decision: GO**` was created |
| Second write to the same report | **PASS** — overwrite was rejected |
| Metric file changed after manifest seal | **PASS** — SHA-256 mismatch was rejected |
| Summary changed and manifest record resealed while raw samples stayed unchanged | **PASS** — independent percentile recomputation rejected recorded edit p95 `0` versus raw nearest-rank p95 `2.4312` |
| Real repository without HP-IGPU-01 evidence | **PASS** — failed closed before report creation |
| Temporary self-test directories after validation | **PASS** — none remain |
| PowerShell parser | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** — all 32 tests |
| `git diff --check` before this artifact | **PASS** |

## Gate status

The done-check remains false. The real `hp-igpu-01-fingerprint-r0-v12.json`, attempt claim, run manifest, six notebook metrics, and `artifacts/gate-c/report.md` are absent. No GO or notebook observation was fabricated.

## Next action

Provide the first qualifying physical 2023–2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review its exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`; it will issue GO, NO-GO, or infrastructure-invalid only from the sealed physical-notebook evidence.
