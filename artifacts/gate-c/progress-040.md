# Gate C Physical-Notebook Execution Handoff 040

**Status: the frozen Gate C closure path is execution-ready; the only missing input is the first qualifying physical HP-IGPU-01 notebook**

- Diagnostic UTC: `2026-08-02T05:58:38.225768Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- Frozen report-validator SHA-256: `21d8c7e3dd820925ff7f0d264e511890c5fb31f2317aeb88b5b8a914f034a4b3`

## Transfer boundary

The qualifying notebook must receive this exact workspace state, including the frozen OCCT install tree and uncommitted Gate C inputs. A fresh checkout from the remote repository is not equivalent because the current gate implementation and evidence are not all committed. Copying the workspace is permitted, but no build input, runner, lock, reference provenance, historical evidence, or terminal report may be edited during transfer. The runner rehashes the frozen build-input tree, source files, toolchain, OCCT tree, and R0 lock before it can create the fingerprint.

## Qualification-only invocation

From the repository root on the candidate notebook, use Windows PowerShell and supply evidence-backed values for every placeholder:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows\run-gate-c-hp-igpu-01.ps1 `
  -ReleaseYear <2023-through-2026> `
  -NominalCpuPowerW <15-through-30> `
  -SharedGpuBudgetGiB <greater-than-0-through-4> `
  -IntegratedGpuName "<exact operational integrated-GPU name>" `
  -RetailModelEvidence "<manufacturer/model evidence>" `
  -Direct3D12Confirmed `
  -FullyPatchedConfirmed `
  -DiscreteGpuDisabledConfirmed `
  -VendorBalancedProfileConfirmed `
  -ProductionDriverConfirmed `
  -BackgroundStateConfirmed
```

Do not add `-RunFormalMeasurements` during qualification. A PASS creates `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v12.json` immutably and exits before observing a Gate C metric. Review that fingerprint before proceeding. It must objectively identify a 2023-2026 physical notebook, Windows 11 build 22631 or later, an x86-64 mobile CPU with at least four physical cores, exactly one operational integrated GPU, 16 GiB RAM, 1920x1080 at 60 Hz and 96 DPI, AC power, the vendor balanced profile, and the required clean background state.

## One-shot formal invocation

Only after the immutable fingerprint is reviewed and the notebook remains unchanged, reconfirm the clean background state and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows\run-gate-c-hp-igpu-01.ps1 `
  -RunFormalMeasurements `
  -BackgroundStateConfirmed
```

The runner revalidates the fingerprint and configuration before sealing the attempt claim. It then performs one clean release build, three core series, and three NAV series. Each NAV series includes 30 runs with a 10-second warm-up and 30-second measurement interval, so reserve at least 60 uninterrupted minutes for NAV measurement alone. Do not restart, delete, overwrite, or hand-edit any evidence after the attempt claim exists; the terminal manifest records PASS, measured FAIL, or infrastructure-invalid exactly once.

## Report invocation

After the runner writes its immutable terminal manifest, validate first and then create the corresponding immutable report:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows\write-gate-c-report.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows\write-gate-c-report.ps1 -WriteReport
```

Only a sealed PASS can create `artifacts/gate-c/report.md`. A measured FAIL creates `report-no-go.md`; infrastructure-invalid creates `report-infrastructure-invalid.md`. Neither non-PASS outcome satisfies the Gate C done-check.

## Fresh non-observational validation

| Check | Result |
|---|---|
| R0 v12 preregistration validator | **PASS** |
| Runner and report-validator PowerShell parsers | **PASS** |
| Runner attempt-sealing self-test | **PASS** |
| Portable build-provenance self-test | **PASS** - tree `6dc2be8e1cfe992247d2946853c77977915ba249930437b6797f0b053d65b3b6` |
| Frozen lock, runner, and report-validator hashes | **MATCH** |
| Incomplete real evidence | **PASS** - rejected before report generation |
| Notebook fingerprint, attempt claim, and run manifest | **ABSENT** |
| All three terminal report paths | **ABSENT** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |

The first shell attempt used unavailable `pwsh`; the same non-observational checks passed with the repository-supported `powershell.exe`. No frozen input, physical-notebook evidence, metric, or terminal report was created or changed.

## Done-check

The workspace-test condition passes. `artifacts/gate-c/report.md` remains absent, so both report existence and `GO` conditions remain false. Gate C stays active under testable assumption A7 until the above protocol runs on the first qualifying physical notebook.
