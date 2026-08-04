# Gate C Terminal-Manifest Repair 036

**Status: the pre-observation forged terminal-manifest acceptance path is repaired and frozen; the physical-notebook blocker remains**

- Repair observation UTC: `2026-08-02T05:21:00.5849340Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired report validator: `scripts/windows/write-gate-c-report.ps1`
- Repaired report validator SHA-256: `0f0a0199c5088bd671fe0e1fc4494a10e08a3525ed44a3927091ae8de48b71c7`
- Superseded pre-observation report-validator SHA-256: `22d4662581ec3245e8de400f5307f36353aae5db0f6223e855348b565ed3fba2`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`

## Reproduced gap

The closure validator derived non-PASS evidence membership from whatever stages a terminal run manifest listed, but it did not prove that those stages formed a runner-realizable canonical prefix. A modified and resealed manifest could therefore omit all stages, skip a stage, disagree with its claimed failing stage, or label a recorded infrastructure failure as a measured product failure and still reach terminal report generation if its listed hashes were internally consistent.

The gap was found before any HP-IGPU-01 fingerprint or formal observation existed. No threshold, corpus, hardware profile, query class, measurement source, runner, R0 lock, HP-DEV observation, or historical gate artifact was changed.

## Repair

The report validator now requires every non-PASS manifest to contain a non-empty prefix of the exact seven-stage runner sequence. Every stage before the terminal stage must be `PASS` with exit code zero. A measured `FAIL` must identify the final recorded stage and cannot carry a launch error; a recorded `INFRASTRUCTURE_INVALID` stage must identify the final stage and carry its launch error. The runner's only unrecorded transition, `executable-verification`, is accepted exclusively after one passing release-build stage and exclusively as infrastructure-invalid. Every terminal outcome must include a non-empty failure message.

This semantic validation runs before exact evidence-membership derivation, so arbitrary stage arrays can no longer define what the terminal report considers complete evidence.

## Validation

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Valid measured-failure sequence | **PASS** - release build followed by a failing core series was accepted |
| Valid recorded infrastructure sequence | **PASS** - release-build launch failure was accepted as infrastructure-invalid |
| Valid executable-verification transition | **PASS** - one passing release build followed by the runner's unrecorded verification failure was accepted |
| Empty-stage forged terminal manifest | **PASS** - rejected |
| Skipped-stage forged terminal manifest | **PASS** - rejected |
| Terminal-decision/stage-decision mismatch | **PASS** - rejected |
| Executable-verification mislabeled as measured failure | **PASS** - rejected |
| Real repository without HP-IGPU-01 evidence | **PASS** - failed closed and created no terminal report |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| Repaired validator SHA-256 | **PASS** - `0f0a0199c5088bd671fe0e1fc4494a10e08a3525ed44a3927091ae8de48b71c7` |
| Whitespace validation | **PASS** |

## Gate status

The G8 done-check remains false. `artifacts/gate-c/report.md` is absent, as are the real HP-IGPU-01 fingerprint, attempt claim, run manifest, and six notebook metrics. No GO, NO-GO, or infrastructure-invalid report was fabricated from synthetic or development-host evidence.

## Next action

Provide the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review the exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`; only a complete, canonically bound, semantically valid PASS set can create `artifacts/gate-c/report.md`.
