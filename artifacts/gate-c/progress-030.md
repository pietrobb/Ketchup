# Gate C Clean-Build NAV Replacement Progress 030

**Status: the three-series clean-build HP-DEV-01 NAV replacement set passed and is sealed under the frozen portable-provenance contract; Gate C remains active**

- Observation UTC: `2026-08-02T04:05:09.9778607Z`
- Controller PID: `28820` (terminated normally)
- Controller terminal status: `PASS`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Clean-build NAV executable SHA-256: `b1495fd0442330279e2813a841e63c1c50bfe41d7e1e921cc0cbb3cd2d269a49`
- Controller state SHA-256: `d36998a5787f1c9900fb67548faa364a674bada8e40354e65112e1ca190c1547`
- NAV provenance SHA-256: `51de8f7bfdfb9697a66de1edec65d7bb0c447c42ed4846a9834ccabefae983da`
- Testable assumption: `A7`

## Progress artifact

The fail-closed controller completed series 3 after the frozen 1,200-second interval, wrote its metric, sealed its metric and logs, and terminated with terminal status `PASS`. All three completed metrics bind to schema version 1, query class `QC-C-NAV-01`, profile `HP-DEV-01`, their exact series number, the r0-v12 lock, 30 runs, 10-second warm-up and 30-second measurement intervals, 10,000 occurrences, 20,000 visible tessellated triangles, one shared authoritative geometry, the expected Direct3D 12 adapter, and 30 per-run sample-count entries for each distribution.

Every metric, stdout, and stderr SHA-256 matches the controller's terminal seal. The summed per-run sample counts equal the serialized frame and input-to-preview sample cardinalities in every series. The frozen executable, runner, validator, and R0 lock hashes also match. The completed set is sealed in `artifacts/gate-c/hp-dev-01-portable-nav-r0-v12-provenance.json`.

| Series | Completion UTC | Metric SHA-256 | Samples per distribution | Frame p95 ms | Frame p99 ms | Input-to-preview p95 ms | Frozen thresholds |
|---:|---|---|---:|---:|---:|---:|---|
| 1 | `2026-08-02T03:24:33.3500172Z` | `29905288c23a4486190594c259e048fd4a9f4a7657e21d0ed742ee4bec3c4cc3` | 227,678 | 5.0341 | 6.2485 | 0.4119 | **PASS** |
| 2 | `2026-08-02T03:44:34.4138952Z` | `d9f08c7fdc4aeae500bf4c503f32cca5ef14f5b49200613d498f5afdfc894b22` | 241,588 | 4.8206 | 6.0411 | 0.4121 | **PASS** |
| 3 | `2026-08-02T04:04:40.3978616Z` | `b42c8d79f6c86d293c058293d4836edd430840c44f196df40d6058b7cf125395` | 227,985 | 4.9443 | 5.9220 | 0.3999 | **PASS** |

Frozen maxima are 16.7 ms frame p95, 33.3 ms frame p99, and 50 ms input-to-preview p95. No Gate C benchmark process remained after the controller terminated.

## Gate status

This is a completed HP-DEV-01 reference observation, not HP-IGPU-01 Gate C evidence. The required physical 2023-2026 Windows 11 integrated-GPU notebook fingerprint, attempt claim, six formal metrics, and `artifacts/gate-c/report.md` remain absent. Gate C therefore remains active and no GO decision is claimed.

## Next action

Provide the first matching physical HP-IGPU-01 notebook. Run qualification-only, review and freeze its immutable fingerprint, then execute three core and three NAV formal series exactly once through the frozen runner. Create `artifacts/gate-c/report.md` only from that sealed physical-notebook evidence, with an evidence-based GO, NO-GO, or infrastructure-invalid decision.
