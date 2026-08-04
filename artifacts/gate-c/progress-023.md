# Gate C Qualification-Claim Repair 023

**Status: the pre-observation HP-IGPU-01 qualification race is repaired and the runner is repinned; physical notebook evidence remains unavailable**

- Repair UTC: `2026-08-02`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- HP-IGPU-01 runner SHA-256: `16ccaee667445f12b4d7ce6a47f66d094c1290ad3ec90ad41a6fb5fe147eb721`
- R0 v12 validator SHA-256: `a4e4b4225ca3eba95f1e6e91fad292fc511bffe6d7c272c228c930dfd3f3ac78`
- Testable assumption: `A7`
- Observation state: no HP-IGPU-01 fingerprint, attempt claim, stage log, metric series, run manifest, or Gate C report exists

## Pre-observation defect

The qualification path checked whether the immutable fingerprint existed and then wrote it with a replace-capable file operation. Two concurrent qualification invocations could both observe no fingerprint, qualify different machines, and overwrite the same path. That violated the frozen rule that the first available fully qualifying notebook becomes the exact HP-IGPU-01 reference machine.

This race existed before any HP-IGPU-01 fingerprint or measurement. No historical evidence was changed, and no threshold, corpus, hardware requirement, expected outcome, oracle, consequence, product source, measurement source, or observed binary changed.

## Focused repair

The runner now writes the qualification fingerprint with the same exclusive `CreateNew` helper used for immutable attempt evidence. Exactly one concurrent qualification can claim the fingerprint path; every later writer is rejected rather than replacing the selected machine. The existing fingerprint validation and machine-configuration match checks remain unchanged.

The R0 v12 validator pins runner `16ccaee667445f12b4d7ce6a47f66d094c1290ad3ec90ad41a6fb5fe147eb721` and requires the exact exclusive fingerprint-write contract. The active R0 v12 lock remains byte-identical because this is a pre-observation orchestration integrity repair, not a frozen measurement-input change.

## Fresh verification

| Check | Result |
|---|---|
| PowerShell parser: runner and validator | **PASS** |
| Exclusive qualification-fingerprint contract | **PASS** |
| Deterministic attempt-sealing self-test | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** |
| Available desktop HP-IGPU-01 qualification | **EXPECTED REJECTION** before fingerprint, claim, logs, metrics, or manifest |
| `git diff --check` | **PASS** |
| Frozen R0 lock hash | **MATCH** |
| HP-IGPU-01 fingerprint, attempt claim, stage logs, metrics, and run manifest | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

The workspace-test portion of the L1 done-criteria passes. Report existence and `GO` remain unmet, so L1 #23 stays active.

## Next action

Provide the first qualifying 2023–2026 Windows 11 notebook with exactly one operational integrated Direct3D 12 GPU. Run qualification-only to exclusively claim its fingerprint under runner `16ccaee667445f12b4d7ce6a47f66d094c1290ad3ec90ad41a6fb5fe147eb721`, review the immutable configuration, then run the three core and three navigation series once. Use the sealed terminal manifest and raw evidence to issue `artifacts/gate-c/report.md` as evidence-based `GO`, `NO-GO`, or infrastructure-invalid without rerunning or weakening the gate.
