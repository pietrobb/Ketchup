# Gate C Adapter-Binding Repair 019

**Status: the adapter-selection repair is frozen as r0-v12 and all six HP-DEV-01 replacement reference series pass**

- Repair UTC: `2026-08-01`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Testable assumption: `A7`

## Repair

The product wgpu path now uses `WgpuSetupCreateNew.native_adapter_selector` with a Direct3D 12-only instance. Gate C navigation requires exactly one surface-compatible physical adapter whose exact name and `DeviceType` match the frozen hardware profile; CPU, virtual, ambiguous, wrong-backend, wrong-name, and wrong-device-type candidates fail before the application and measurement loop start.

The selector captures the actual `wgpu::AdapterInfo`. Every new NAV evidence file records its adapter name, vendor ID, device ID, device type, driver, driver information, and backend. The HP-IGPU-01 runner passes the integrated GPU name from the immutable machine attestation and requires `IntegratedGpu`; HP-DEV-01 reference invocation requires the frozen discrete adapter.

R0 v12 was frozen before any replacement observation. It inherits all thresholds, corpora, expected outcomes, hardware profiles, oracles, consequences, dependencies, and non-NAV measurement sources from r0-v11. Only `crates/ketchup-app/src/lib.rs` and `crates/ketchup-app/src/bin/ketchup-gate-c-nav.rs` changed in the 18-file lock. Historical r0-v9 through r0-v11 evidence remains unchanged.

## Verification and measurement state

| Check | Result |
|---|---|
| `cargo test -p ketchup-app --all-targets` | **PASS** |
| `cargo clippy -p ketchup-app --all-targets -- -D warnings` | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| PowerShell parser for validator and HP-IGPU runner | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** |
| `git diff --check` | **PASS** |
| HP-IGPU-01 runner SHA-256 | `83a9c9a8d37b615afa7ca5a6209a164a97afe81dc92c2029b64cd0cce47f0d7d` |
| HP-IGPU-01 qualification on available desktop | **EXPECTED REJECTION** before fingerprint or evidence creation |
| HP-DEV-01 r0-v12 core series 1–3 | **PASS** |
| HP-DEV-01 r0-v12 NAV series 1–3 | **PASS** — identical DX12 AdapterInfo and all thresholds met |
| HP-DEV-01 r0-v12 provenance manifests | **PASS** — every listed file hash reproduced |
| HP-IGPU-01 r0-v12 fingerprint and run manifest | **ABSENT** |
| Gate C report | **ABSENT** |

Two shell-launch diagnostics failed before renderer initialization because Windows PowerShell split the space-containing adapter name into multiple arguments. They produced only immutable stdout/stderr diagnostics and no result JSON. The argument-vector launch preserved the adapter name, and all three sequential NAV series completed successfully without overlap.

## Next action

Provide the qualifying physical Windows 11 integrated-GPU notebook, freeze its first-machine fingerprint under the repaired runner, run three core and three NAV series, and issue the evidence-based Gate C GO or NO-GO report. No software-only substitute can satisfy the frozen HP-IGPU-01 physical profile.
