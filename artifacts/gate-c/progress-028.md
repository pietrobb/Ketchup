# Gate C Clean-Build NAV Replacement Progress 028

**Status: two of three sequential clean-build HP-DEV-01 NAV replacement series passed under the frozen portable-provenance contract; series 3 is running and Gate C remains active**

- Observation UTC: `2026-08-02T03:45:30.9121558Z`
- Controller PID: `28820`
- Controller status: `RUNNING`
- Active series: `3`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Clean-build NAV executable SHA-256: `b1495fd0442330279e2813a841e63c1c50bfe41d7e1e921cc0cbb3cd2d269a49`
- Expected and selected adapter: `AMD Radeon RX 6800 XT`
- Portable build contract: `portable-build-provenance-v1`
- Testable assumption: `A7`

## Progress artifact

The fail-closed controller from progress 027 remained alive across both completed 1,200-second series and started series 3 only after series 2 exited successfully and wrote its metric. At observation, exactly one direct child existed: the series-3 `ketchup-gate-c-nav.exe` process. No competing build, test, or benchmark was started.

The completed metric files were parsed and audited against the frozen protocol. Both bind to schema version 1, query class `QC-C-NAV-01`, profile `HP-DEV-01`, their exact series number, the r0-v12 lock, 30 runs, 10-second warm-up and 30-second measurement intervals, 10,000 occurrences, 20,000 visible tessellated triangles, one shared authoritative geometry, the expected Direct3D 12 adapter, and 30 per-run sample counts for both reported distributions. The executable still matches the hash pinned at launch.

| Series | Completion UTC | Metric SHA-256 | Frame p95 ms | Frame p99 ms | Input-to-preview p95 ms | Frozen thresholds |
|---:|---|---|---:|---:|---:|---|
| 1 | `2026-08-02T03:24:33.3500172Z` | `29905288c23a4486190594c259e048fd4a9f4a7657e21d0ed742ee4bec3c4cc3` | 5.0341 | 6.2485 | 0.4119 | **PASS** |
| 2 | `2026-08-02T03:44:34.4138952Z` | `d9f08c7fdc4aeae500bf4c503f32cca5ef14f5b49200613d498f5afdfc894b22` | 4.8206 | 6.0411 | 0.4121 | **PASS** |

Frozen maxima are 16.7 ms frame p95, 33.3 ms frame p99, and 50 ms input-to-preview p95. These are valid HP-DEV-01 reference observations, not HP-IGPU-01 Gate C evidence.

## Gate status

The clean-build NAV replacement remains incomplete until series 3 reaches terminal `PASS` or `FAIL` and all three artifacts are sealed in a provenance summary. HP-IGPU-01 qualification and formal evidence remain absent, so `artifacts/gate-c/report.md` cannot yet be created and L1 #23 remains active.

## Next action

Monitor only the existing controller until series 3 finishes. Then audit series 3, verify the terminal controller state and all recorded hashes, and seal the three clean-build NAV reference metrics. After that, provide the first qualifying 2023-2026 Windows 11 integrated-GPU notebook, capture and review its immutable fingerprint, execute the six formal series exactly once, and issue the evidence-based Gate C report.
