# Gate C Clean-Build NAV Replacement Progress 027

**Status: three sequential clean-build HP-DEV-01 NAV replacement series are running under the frozen portable-provenance contract; Gate C remains active and unobserved on HP-IGPU-01**

- Start UTC recorded by controller: `2026-08-02T03:04:32.3222489Z`
- Controller PID: `28820`
- Active series at verification: `1`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Clean-build NAV executable SHA-256: `b1495fd0442330279e2813a841e63c1c50bfe41d7e1e921cc0cbb3cd2d269a49`
- Expected physical adapter: `AMD Radeon RX 6800 XT`
- Portable build contract: `portable-build-provenance-v1`
- Testable assumption: `A7`

## Progress artifact

The bounded 600-second diagnostic from progress 026 was not reused and remains non-evidence. A new fail-closed controller preflighted that every replacement metric and log path was absent, verified the clean-build NAV executable against the hash sealed by `hp-dev-01-portable-core-r0-v12-provenance.json`, forced the frozen Direct3D 12 backend, and started series 1.

The controller invokes series 1, 2, and 3 strictly sequentially. Each invocation uses the preregistered 30 repetitions of 10 seconds warm-up plus 30 seconds measurement, so each series requires at least 1,200 seconds. It stops on the first nonzero exit or missing metric instead of starting a later series. Its live state is recorded in `artifacts/gate-c/hp-dev-01-nav-r0-v12-portable-replacement-state.json`; the three non-overlapping metric names are `hp-dev-01-nav-r0-v12-portable-replacement-series-{1,2,3}.json`, each with dedicated stdout and stderr logs.

A post-launch check found controller PID 28820 alive, state `RUNNING`, active series `1`, zero completed series, and no premature series-1 metric. This is the expected state before the 1,200-second harness completes. No competing build, test, or benchmark should be started while the controller is running.

## Gate status

This launch is progress, not a performance result. The replacement series must reach terminal `PASS` or `FAIL`, their metric schemas, lock binding, selected adapter, thresholds, and hashes must be audited, and a sealed provenance summary must be written before the clean-build NAV replacement is complete.

HP-IGPU-01 qualification and formal evidence remain absent. `artifacts/gate-c/report.md` therefore remains unavailable and L1 #23 stays active.

## Next action

Monitor only the controller state until all three sequential series finish; do not run workspace tests or other load concurrently. Then validate and seal the three replacement metrics. After that, provide the first qualifying 2023-2026 Windows 11 integrated-GPU notebook, capture and review its immutable fingerprint, execute the six formal series exactly once, and issue the evidence-based Gate C report.
