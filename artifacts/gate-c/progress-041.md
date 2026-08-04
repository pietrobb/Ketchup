# Gate C Transfer-Integrity Preflight 041

**Status: the physical-notebook handoff now has one fail-closed pre-qualification integrity command; HP-IGPU-01 observation remains blocked on hardware availability**

- Diagnostic UTC: `2026-08-02T06:08:34.8885576Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- New preflight: `scripts/windows/verify-gate-c-transfer.ps1`
- Preflight SHA-256: `1e3ebd2f5024a11a8570ff869c661e5c52ee371017a6932c576814fba45bdcf1`

## Closed transfer gap

The execution handoff in progress 040 required the exact uncommitted workspace to be copied to the first qualifying notebook, but it provided no single fail-closed command to establish that the transferred closure inputs still matched before qualification. The new read-only preflight verifies the frozen R0 v12 lock, HP-IGPU-01 runner, R0 validator, report writer, and both portable reference-provenance records by SHA-256. It also parses the report writer, invokes the complete R0 v12 validator and portable build-provenance self-test, and rejects any workspace where a notebook fingerprint, attempt claim, terminal manifest, or terminal report already exists.

This script is an operational preflight, not a measurement input. It does not change any threshold, corpus, hardware profile, expected outcome, consequence, R0 lock, runner, report validator, source file, reference evidence, or historical artifact.

## Notebook invocation

Run this immediately after copying the exact workspace and before the qualification-only command from progress 040:

```powershell
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File .\scripts\windows\verify-gate-c-transfer.ps1
```

A PASS proves that the copied closure inputs match the source-side frozen hashes, the build-input and OCCT provenance checks pass on that notebook, and the workspace is still pre-observation. Any mismatch or prior Gate C evidence fails closed before qualification.

## Fresh validation

| Check | Result |
|---|---|
| Transfer preflight on current exact workspace | **PASS** |
| R0 v12 preregistration validator invoked by preflight | **PASS** |
| Runner attempt-sealing and portable build-provenance self-test | **PASS** - tree `6dc2be8e1cfe992247d2946853c77977915ba249930437b6797f0b053d65b3b6` |
| Frozen lock, runner, R0 validator, report writer, and two references | **PASS** - all SHA-256 values match |
| Report-writer PowerShell parse | **PASS** |
| Notebook fingerprint, attempt claim, and terminal manifest | **ABSENT** |
| `report.md`, `report-no-go.md`, and `report-infrastructure-invalid.md` | **ABSENT** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |

## Done-check

The workspace-test condition passes. `artifacts/gate-c/report.md` remains absent, so the report-existence and `GO` conditions remain false. Gate C stays active under testable assumption A7.

## Next action

Copy the exact workspace and frozen OCCT tree to the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run the new transfer preflight, run qualification-only and review the immutable fingerprint, then execute the three core and three NAV formal series exactly once and invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`.
