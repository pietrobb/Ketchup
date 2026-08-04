# Gate C Controlled-Wait Audit 015

**Status: the frozen Gate C software path passes, but the mandatory HP-IGPU-01 physical observation remains unavailable**

- Observation UTC: `2026-08-02`
- Active freeze: `r0-v11`
- Testable assumption: `A7`
- Runner SHA-256: `fb53cf88ae3d1f90b2fb9ae83b55ba9520c31e46f486bcdc7549ba588fd619d7`

## Fresh done-check evidence

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| R0 v11 preregistration validator | **PASS** |
| HP-IGPU-01 qualification on the available desktop | **EXPECTED REJECTION** before fingerprint or measurement evidence creation |
| `cargo test --workspace --all-targets` | **PASS** — all workspace targets passed |
| Runner SHA-256 | **PASS** — unchanged from progress-014 |
| `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v11.json` | **ABSENT** |
| `artifacts/gate-c/hp-igpu-01-r0-v11-run-manifest.json` | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

The available host again failed the frozen notebook, retail-year, Windows 11, mobile-CPU, integrated-GPU, 16-GiB memory, display-mode, AC/vendor-balanced, and clean-background qualification requirements. This is the required fail-closed outcome for this desktop. No HP-IGPU-01 fingerprint, measurement series, run manifest, or decision report was created.

## Done-check decision

The workspace-test criterion passes. The report existence criterion fails, and the required `GO` content cannot be asserted without the frozen physical evidence. L1 #23 therefore remains active.

No software mutation, threshold change, alternate hardware profile, synthetic result, or historical evidence rewrite can replace the missing observation. The next admissible action is to run `scripts/windows/run-gate-c-hp-igpu-01.ps1` on the first qualifying 2023–2026 Windows 11 notebook with one operational integrated Direct3D 12 GPU, freeze that machine fingerprint, complete three consecutive core and three consecutive NAV series, and issue `artifacts/gate-c/report.md` with `GO` or `NO-GO` from the resulting immutable evidence.
