# Gate C GPU Qualification Repair 013

**Status: the HP-IGPU-01 runner now objectively rejects any candidate with more than one operational GPU**

- Observation UTC: `2026-08-02`
- Active freeze: `r0-v11`
- Testable assumption: `A7`
- Repaired runner SHA-256: `fb53cf88ae3d1f90b2fb9ae83b55ba9520c31e46f486bcdc7549ba588fd619d7`

## Diagnostic finding

The runner required an operator attestation that every discrete GPU was disabled, but it did not corroborate that statement against the captured `Win32_VideoController` state. This left an admissibility gap: a hybrid notebook could claim that its discrete GPU was disabled while both integrated and discrete adapters remained operational. Because the frozen egui/wgpu configuration uses its normal adapter selection path, such a candidate could measure the NAV series on the wrong GPU and still be labeled `HP-IGPU-01`.

The display check also accepted a qualifying 1920x1080 mode reported by any controller instead of requiring that mode to belong to the selected integrated GPU.

## Repair

`scripts/windows/run-gate-c-hp-igpu-01.ps1` now requires all of the following before it can freeze the first-machine fingerprint:

1. exactly one captured GPU has `Status = OK`;
2. that sole operational GPU is the exact `IntegratedGpuName` selected by the operator;
3. the selected integrated GPU itself has `Status = OK`; and
4. the required 1920x1080 at 60 Hz display mode is reported by that selected integrated GPU.

The existing Direct3D 12, production-driver, and disabled-discrete-GPU attestations remain required. The new objective checks supplement rather than replace them. No benchmark source, threshold, corpus, hardware profile, historical result, or done criterion changed.

## Verification

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Synthetic notebook with operational integrated and discrete GPUs | **EXPECTED REJECTION** despite a positive disabled-dGPU attestation |
| Same synthetic notebook after the discrete GPU status changes to `Error` | **PASS** |
| R0 v11 preregistration validation | **PASS** |
| Qualification on the available desktop | **EXPECTED REJECTION** before fingerprint or measurement evidence creation |
| `cargo test --workspace --all-targets` | **PASS** |
| `git diff --check` | **PASS** |
| `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v11.json` | **ABSENT** |
| `artifacts/gate-c/hp-igpu-01-r0-v11-run-manifest.json` | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

## Done-check

Gate C remains open. The command criterion passes, but the report existence and GO criteria remain unmet because no qualifying physical HP-IGPU-01 notebook has been provided. L1 #23 must remain active; the next admissible action is to run the repaired qualification on the first available 2023–2026 Windows 11 notebook, freeze its fingerprint, and then execute the three core plus three NAV series.
