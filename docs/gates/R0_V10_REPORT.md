# R0 v10 Preregistration Supersession Report

- Freeze: `r0-v10`
- Lock SHA-256: `ad3113150d52805efb2659c2e4a4b51dc9e8c695207beef52309e437e2509e99`
- Superseded failed lock: `r0-v9` (`da0dbcd3b3daf845a83f6a708a528c7cdcbf8e0155d1d93bfbb9637c539a7b25`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run and subsequent Gate C measurements under `r0-v10`

## Observed r0-v9 failure

The first complete formal `HP-DEV-01` `QC-C-NAV-01` series remains immutable at `artifacts/gate-c/hp-dev-01-nav-series-1.json`. All 30 runs completed and retained 53,834 frame and input-to-preview samples. Frame p95 was 17.3552 ms against the frozen 16.7 ms maximum, so the r0-v9 Gate C run is failed. Frame p99 was 17.9553 ms against 33.3 ms and input-to-preview p95 was 0.4247 ms against 50 ms.

The result is not discarded, averaged away, relabeled diagnostic, or rewritten. No remaining r0-v9 series is run because three consecutive passing series are impossible after this failure.

## Bounded remediation

The failure distribution is the 60 Hz presentation cadence, not CPU interaction work: eframe's native default enables vertical synchronization, while the frozen threshold leaves only 0.0333 ms above a nominal 60 Hz interval. The smallest product-path correction explicitly sets `NativeOptions.vsync` to false while preserving the Direct3D 12 wgpu renderer, scene, operation, sample protocol, and every threshold. A regression test requires both wgpu and non-vsync native options.

The r0-v10 lock adds hashes for the product renderer configuration and unchanged NAV harness. All 16 inherited paths remain byte-identical to r0-v9. No threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, license policy, toolchain evidence, OCCT input, or failure consequence changed.

## Consequence

A0 must run as immutable `run-010` under this exact lock before any replacement Gate C observation. Gate C then requires three new consecutive complete release series on both `HP-DEV-01` and the first qualifying, preregistered `HP-IGPU-01` machine. Historical r0-v9 evidence remains part of the audit record and can never count as a pass.
