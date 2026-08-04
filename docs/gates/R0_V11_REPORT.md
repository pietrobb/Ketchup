# R0 v11 Preregistration Supersession Report

- Freeze: `r0-v11`
- Lock SHA-256: `d6c9edacd884a1b24a8fc6d42a14ad4bc25c248883faf7ba5c0d846977ae8de7`
- Superseded failed lock: `r0-v10` (`ad3113150d52805efb2659c2e4a4b51dc9e8c695207beef52309e437e2509e99`)
- Measurement state at freeze: `not_started`
- **Decision: GO** for one replacement formal A0 run and subsequent Gate C measurements under `r0-v11`

## Observed r0-v10 failure

The complete r0-v10 replacement series remains immutable at `artifacts/gate-c/hp-dev-01-nav-r0-v10-series-1.json`. All 30 runs completed and retained 53,937 frame and input-to-preview samples. Frame p95 was 17.2952 ms against the frozen 16.7 ms maximum, so the r0-v10 Gate C run failed. Frame p99 was 17.6263 ms against 33.3 ms and input-to-preview p95 was 0.4279 ms against 50 ms.

## Root cause and bounded remediation

Source inspection established that eframe's `NativeOptions.vsync` configures the OpenGL integration but does not configure a wgpu surface. The r0-v10 attempt therefore left `WgpuConfiguration.present_mode` at its `AutoVsync` default. The product now configures the actual wgpu surface as `AutoNoVsync`; the regression test asserts that exact present mode.

The r0-v11 lock preserves the same 18 paths as r0-v10. Seventeen hashes are byte-identical; only `crates/ketchup-app/src/lib.rs` changes. The NAV harness, Direct3D 12 dependency, scene, operation, sample protocol, threshold, corpus, expected outcome, validity oracle, Guaranteed subset, hardware profile, query class, license policy, toolchain evidence, OCCT inputs, and failure consequences remain unchanged.

## Consequence

A0 must run as immutable `run-011` under this exact lock before any r0-v11 Gate C observation. Gate C then requires three new consecutive complete release series on both required hardware profiles. Historical r0-v9 and r0-v10 failures remain in the audit record and never count as passing evidence.
