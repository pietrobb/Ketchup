# R0 v12 Preregistration Report

**Decision: GO**

- Freeze: `r0-v12`
- Lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Measurement state at freeze: `not_started`

## Authorized repair

A pre-observation Gate C audit found that the r0-v11 renderer forced Direct3D 12 but did not bind or preserve the identity of the adapter selected by egui-wgpu. R0 v12 changes only the product adapter-selection source and the NAV evidence source: `native_adapter_selector` now requires one surface-compatible physical Direct3D 12 adapter matching the frozen profile name and device type, and every NAV result records the selected `wgpu::AdapterInfo`.

All thresholds, corpora, expected outcomes, hardware profiles, oracles, consequences, dependencies, and non-NAV measurement sources remain unchanged. Historical r0-v9, r0-v10, and r0-v11 observations remain immutable. New HP-DEV-01 reference series and HP-IGPU-01 formal series must use this exact lock; no r0-v12 measurement existed when the lock was frozen.
