# Gate C Adapter-Binding Diagnosis 018

**Status: the HP-IGPU-01 runner forces Direct3D 12 but does not yet prove that the measured surface is rendered by the frozen integrated adapter**

- Observation UTC: `2026-08-02`
- Active freeze: `r0-v11`
- Testable assumption: `A7`
- Affected evidence state: no `HP-IGPU-01` fingerprint, formal run manifest, or Gate C report exists

## Diagnostic finding

The runner objectively requires exactly one operational WMI video controller, ties the required display mode to the attested integrated GPU, and sets `WGPU_BACKEND=dx12` for every navigation series. These checks establish the host configuration and backend family, but they do not bind or expose the adapter selected by the wgpu surface.

The pinned `egui-wgpu 0.32.3` default setup reads `WGPU_BACKEND` and `WGPU_POWER_PREF`, enumerates matching adapters, and then calls `Instance::request_adapter`. It does not use wgpu's `WGPU_ADAPTER_NAME` helper. The dependency provides `WgpuSetupCreateNew.native_adapter_selector` as the explicit native mechanism for selecting and validating the adapter used by the surface.

Consequently, the current evidence path cannot independently prove the full claim "QC-C-NAV-01 ran on the frozen HP-IGPU-01 integrated adapter." A successful Direct3D 12 run is strong evidence on a single-operational-GPU notebook, but it is not an immutable record of the actual wgpu `AdapterInfo`, and software or otherwise unexpected adapter selection is not rejected explicitly.

## Evidence

| Source | Relevant behavior |
|---|---|
| `scripts/windows/run-gate-c-hp-igpu-01.ps1` | Sets `WGPU_BACKEND=dx12` around the NAV executable but supplies no native adapter selector or selected-adapter record |
| `egui-wgpu 0.32.3/src/setup.rs` | Documents `WGPU_BACKEND` and `WGPU_POWER_PREF`; `WgpuSetupCreateNew` defaults `native_adapter_selector` to `None` |
| `egui-wgpu 0.32.3/src/lib.rs` | With no native selector, calls `request_adapter`; selected `AdapterInfo` is logged but not included in Gate C JSON |
| `wgpu 25.0.2/src/util/init.rs` | Implements `WGPU_ADAPTER_NAME`, but this helper is not called by the pinned egui-wgpu default setup |

## Required repair direction

Before the first HP-IGPU-01 observation, introduce a new preregistration freeze rather than weakening the hardware profile or rewriting r0-v11 history:

1. configure `WgpuSetupCreateNew.native_adapter_selector` in the frozen product path;
2. require a Direct3D 12, non-software adapter whose identity matches the frozen HP-IGPU-01 integrated GPU;
3. write the selected wgpu `AdapterInfo` into every NAV evidence file and validate it against the machine fingerprint;
4. freeze the changed source and executable hashes under a new freeze ID;
5. rerun the required reference series under that new freeze before evaluating HP-IGPU-01.

This is a pre-observation integrity diagnosis, not a threshold result. No threshold, corpus, hardware profile, expected outcome, or historical evidence has been changed.

## Verification

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | **PASS** |
| PowerShell runner parser | **PASS** |
| R0 v11 preregistration validator | **PASS** |
| Frozen runner SHA-256 | `c191529ab8f50b6ad2852b9e9e0e0f23e4a1bd755892b0ec71ea390229193c9b` — unchanged |
| `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v11.json` | **ABSENT** |
| `artifacts/gate-c/hp-igpu-01-r0-v11-run-manifest.json` | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |
| `git diff --check` | **PASS** |

## Done-check

The Gate C report existence and `GO` criteria remain unmet. L1 #23 stays active. The next admissible repository action is a focused new-freeze repair for objective wgpu adapter binding; the mandatory physical HP-IGPU-01 notebook is still required afterward for the lower-reference measurements.
