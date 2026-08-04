# Gate C Core Diagnostic Series 1

**Profile:** HP-DEV-01  
**Status:** diagnostic PASS; non-gating

The release runner completed the frozen core workloads with no outlier removal:

- `QC-C-EDIT-01`: 100 warmups plus 1,000 measured Smart Push/Pull exact commits; p95 2.2336 ms (threshold 100 ms); preview/commit canonical action digest match 100%.
- `QC-C-PICK-01`: 200 warmups plus 2,000 measured exact face, edge, endpoint, midpoint, and intersection queries over 10,000 occurrences sharing one authoritative geometry; p95 2.4757 ms (threshold 50 ms); wrong identities 0.
- `QC-C-LONG-01`: 100 killable two-second jobs; cancellation p95 1.1466 ms (threshold 250 ms); navigation block stayed below 100 ms; committed data loss 0.

Raw samples and the machine fingerprint are preserved in `diagnostic-core-series-1.json`. This series is deliberately non-gating because adding the local interaction dependency to the measurement runner changed `Cargo.lock` after R0 v8/A0 run-008. A new R0 v9 freeze and replacement A0 run are required before three formal core series. `QC-C-NAV-01` still requires the automated 30-run wgpu surface harness, and final Gate C remains externally blocked until a qualifying `HP-IGPU-01` notebook is fingerprinted and measured.
