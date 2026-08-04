# Gate C HP-IGPU-01 Handoff and Blocker Proof 020

**Status: repository-side Gate C work is ready; formal closure is blocked only by the unavailable frozen hardware profile**

- Check UTC: `2026-08-02`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- HP-IGPU-01 runner SHA-256: `83a9c9a8d37b615afa7ca5a6209a164a97afe81dc92c2029b64cd0cce47f0d7d`
- Testable assumption: `A7`

## Fresh bounded verification

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | **PASS** |
| PowerShell parser for the HP-IGPU-01 runner | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| Frozen runner hash | **MATCH** |
| Frozen R0 lock hash | **MATCH** |
| HP-IGPU-01 fingerprint | **ABSENT** |
| HP-IGPU-01 formal run manifest | **ABSENT** |
| Gate C report | **ABSENT** |

No formal HP-IGPU-01 observation was started. The r0-v12 freeze and all historical evidence remain unchanged.

## Required physical profile

The first admitted machine must satisfy every runner-enforced condition: a 2023–2026 notebook; Windows 11 build 22631 or later and fully patched; an x86-64 mobile CPU with at least four physical cores in the documented 15–30 W nominal class; exactly 16 GiB system memory; an integrated-GPU shared budget no greater than 4 GiB; exactly one operational GPU, which is the named integrated Direct3D 12 adapter with its production driver; 1920×1080 at 60 Hz and 96 DPI; AC power; the vendor balanced profile; and no pending update, build, debugger, profiler, or overlapping formal measurement.

## Two-phase operator procedure

Run from the repository root on the qualifying notebook. Replace each angle-bracket value with verified machine data. The first command is qualification-only: it irreversibly selects and fingerprints the first passing machine but does not start measurement.

```powershell
& .\scripts\windows\run-gate-c-hp-igpu-01.ps1 `
  -ReleaseYear <2023-through-2026> `
  -NominalCpuPowerW <15-through-30> `
  -SharedGpuBudgetGiB <value-no-greater-than-4> `
  -IntegratedGpuName '<exact-Win32_VideoController-name>' `
  -RetailModelEvidence '<verified-model-evidence>' `
  -Direct3D12Confirmed `
  -FullyPatchedConfirmed `
  -DiscreteGpuDisabledConfirmed `
  -VendorBalancedProfileConfirmed `
  -ProductionDriverConfirmed `
  -BackgroundStateConfirmed
```

After reviewing the newly written `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v12.json`, run the formal series on the same unchanged machine and configuration:

```powershell
& .\scripts\windows\run-gate-c-hp-igpu-01.ps1 `
  -BackgroundStateConfirmed `
  -RunFormalMeasurements
```

A passing formal invocation writes three core series, three NAV series, and `artifacts/gate-c/hp-igpu-01-r0-v12-run-manifest.json`. Only those immutable outputs may support `artifacts/gate-c/report.md` with an evidence-based `GO` or `NO-GO` decision.

## Done-check

The workspace-test criterion passes. The report existence and `GO` criteria fail because the mandatory physical observation does not exist. L1 #23 therefore remains active; no software-only substitute, hardware-profile relaxation, synthetic evidence, or historical rewrite is admissible.
