# Gate C Controlled-Wait Audit 014

**Status: the formal HP-IGPU-01 path remains fail-closed and no further runner mutation is justified without the qualifying notebook**

- Observation UTC: `2026-08-02`
- Active freeze: `r0-v11`
- Testable assumption: `A7`
- Runner SHA-256: `fb53cf88ae3d1f90b2fb9ae83b55ba9520c31e46f486bcdc7549ba588fd619d7`

## Bounded integrity audit

A fresh review of the formal runner, frozen core and NAV harnesses, and R0 v11 lock found no defensible runner-only repair that would improve the current gate without adding speculative machinery or changing the preregistered observation path. The runner already validates the active lock, frozen measurement sources, every frozen OCCT runtime library, and the exact three HP-DEV-01 release executable hashes before formal sampling. It also requires one objectively operational selected iGPU, binds the required display mode to that adapter, forces the NAV process to the Direct3D 12 backend, refuses existing series outputs, and writes the run manifest only after all three core and all three NAV processes return success.

The remaining missing proof is physical rather than software-only: no available host satisfies HP-IGPU-01. Synthetic output, a virtual machine, the current desktop, a remote discrete GPU, or a weakened hardware profile would not be an admissible substitute. The runner was therefore left unchanged.

## Fresh verification

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| R0 v11 preregistration validator | **PASS** |
| HP-IGPU-01 qualification on the available desktop | **EXPECTED REJECTION** before fingerprint or measurement evidence creation |
| `cargo test --workspace --all-targets` | **PASS** — all workspace targets passed, including A0, A1, Gate B, and Gate C interaction coverage |
| Runner SHA-256 | **PASS** — matches progress-013 |
| `git diff --check` | **PASS** |
| `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v11.json` | **ABSENT** |
| `artifacts/gate-c/hp-igpu-01-r0-v11-run-manifest.json` | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

The desktop rejection was caused by its non-notebook form factor, insufficient Windows 11 build qualification, non-16-GiB memory configuration, nonconforming active display mode, and missing notebook AC/vendor-balanced state. No HP-IGPU evidence was created.

## Done-check and next admissible action

The workspace-test criterion passes. The report existence criterion fails and the required `GO` content is not evaluable, so L1 #23 remains active.

Provide the first qualifying 2023–2026 Windows 11 notebook with one operational integrated Direct3D 12 GPU. Freeze its machine fingerprint before observation, then run three consecutive core series and three consecutive NAV series with `scripts/windows/run-gate-c-hp-igpu-01.ps1`; only the resulting immutable evidence can support the Gate C `GO` or `NO-GO` report. No threshold, corpus, hardware profile, source hash, historical result, mission queue, or done criterion changed in this tick.
