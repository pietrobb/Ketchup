# Gate C Implementation Progress 004

**Status: HP-DEV-01 complete PASS under R0 v11; mandatory HP-IGPU-01 evidence blocked on unavailable hardware**

## Preserved failures and remediation

- R0 v9 NAV series 1 failed frame p95 at 17.3552 ms versus 16.7 ms. The raw result remains immutable.
- R0 v10 explicitly changed `NativeOptions.vsync`, but source inspection later proved that field does not configure eframe's wgpu surface. Its replacement series also remained refresh-limited and failed at 17.2952 ms. That raw result also remains immutable.
- R0 v11 configures `WgpuConfiguration.present_mode` directly as `AutoNoVsync`. It changes only the frozen product renderer source, without changing thresholds, scene, protocol, corpus, hardware rules, or consequences.
- The replacement A0 `run-011` passed before any R0 v11 Gate C observation.

## HP-DEV-01 R0 v11 results

All three consecutive 30-run NAV series passed with complete raw samples and provenance:

| Series | Frame p95 ms | Frame p99 ms | Input-to-preview p95 ms | Frame samples |
|---:|---:|---:|---:|---:|
| 1 | 5.2199 | 6.4700 | 0.4408 | 224,755 |
| 2 | 5.3903 | 7.2572 | 0.4053 | 216,154 |
| 3 | 5.6148 | 6.4348 | 0.4254 | 216,540 |

All three consecutive core series also passed. Edit p95 was 2.2523–2.3888 ms, pick/snap p95 was 2.4464–2.4526 ms, navigation block maximum was 2.8555–2.9298 ms, and cancellation p95 was 1.1074–1.1428 ms. Action-digest match was 100%; wrong identities and committed-data loss were zero.

## Remaining blocker

Gate C requires the first qualifying `HP-IGPU-01` notebook in addition to `HP-DEV-01`. No qualifying notebook is available in the current environment, so its exact fingerprint cannot be frozen and its three core plus three NAV series cannot be executed. `artifacts/gate-c/report.md` remains absent and Gate C remains open; issuing GO without that hardware would violate the preregistered contract.
