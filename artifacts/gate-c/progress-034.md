# Gate C Decision-Sentinel Repair 034

**Status: the pre-observation NO-GO substring false-success path is repaired and frozen; the physical-notebook blocker remains**

- Repair observation UTC: `2026-08-02T04:58:42.7085150Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired report validator: `scripts/windows/write-gate-c-report.ps1`
- Repaired report validator SHA-256: `6461e4c98531644bef94ab8236f556484ea70cc6753867ebbf6221a3ca232128`
- Superseded pre-observation report-validator SHA-256: `6f1dc6fe5919816ac85c8a351dbeafb94fd0d4ea92ebdea312688c8fddb4381e`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`

## Reproduced gap

The active G8 done-check uses the case-sensitive substring predicate `file_contains:artifacts/gate-c/report.md::GO`. The mission DSL implements this as `needle in text`, so a terminal report containing `NO-GO` at `artifacts/gate-c/report.md` would satisfy the GO content predicate even though the physical-notebook attempt failed. The existing report validator mapped a sealed `FAIL` decision to `NO-GO` and wrote every terminal outcome to that same path. This was therefore a genuine completion-sentinel false success, not a wording-only issue.

The gap was found before any HP-IGPU-01 fingerprint or formal observation existed. No threshold, corpus, hardware profile, query class, measurement source, runner, R0 lock, HP-DEV observation, or historical gate artifact was changed.

## Repair

The report validator now reserves `artifacts/gate-c/report.md` exclusively for a sealed `PASS` decision mapped to `GO`. A sealed measured failure is written immutably to `artifacts/gate-c/report-no-go.md`, and infrastructure-invalid evidence is written immutably to `artifacts/gate-c/report-infrastructure-invalid.md`. Both non-passing outcomes remain explicit auditable terminal reports while neither can satisfy G8's path-and-content GO sentinel.

The evidence validation, terminal decision mapping, report prose, and exclusive create-without-overwrite behavior are unchanged. Only the terminal filename is decision-specific.

## Validation

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Decision-to-filename regression | **PASS** - only `PASS` maps to `report.md`; `FAIL` and `INFRASTRUCTURE_INVALID` map to distinct paths |
| Real repository without HP-IGPU-01 evidence | **PASS** - failed closed before terminal decision handling and created no report path |
| Pre-existing terminal reports | **PASS** - `report.md`, `report-no-go.md`, and `report-infrastructure-invalid.md` are all absent |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| `git diff --check` for the repaired validator | **PASS** |

## Gate status

The done-check remains false. The real HP-IGPU-01 fingerprint, attempt claim, run manifest, six notebook metrics, and `artifacts/gate-c/report.md` are absent. A future measured failure or infrastructure-invalid attempt will produce an explicit immutable report without falsely advancing G8.

## Next action

Provide the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review the exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`; only a fully validated PASS can create the GO sentinel at `artifacts/gate-c/report.md`.
