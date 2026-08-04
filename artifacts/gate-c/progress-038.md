# Gate C Terminal Evidence Path-Binding Repair 038

**Status: a pre-observation non-PASS evidence-substitution flaw is repaired and frozen; the physical-notebook blocker remains**

- Repair observation UTC: `2026-08-02T05:40:59.9758083Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired report validator: `scripts/windows/write-gate-c-report.ps1`
- Repaired report validator SHA-256: `21d8c7e3dd820925ff7f0d264e511890c5fb31f2317aeb88b5b8a914f034a4b3`
- Superseded pre-observation report-validator SHA-256: `5ab7d75a5ae49f3ae3d9e6dcf6a1b374d82132b0080cb52fa46e2aa6497e2638`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`

## Reproduced flaw

The progress-037 validator enforced canonical stage order and terminal semantics, but its non-PASS evidence-path check derived each expected stdout, stderr, and optional metric path directly from the untrusted run-manifest record. The subsequent exact-membership comparison was therefore tautological: a forged canonical `release-build` stage could bind its stdout and stderr records to unrelated repository files, include those same records in the manifest evidence list, and pass every path/hash check.

The gap was reproduced before repair with the actual validator path and hash helpers. A synthetic terminal `release-build` stage bound stdout to `artifacts/gate-c/progress-037.md` and stderr to `artifacts/gate-c/progress-036.md`; both hashes were genuine, and the pre-repair non-PASS evidence-membership logic accepted the substituted files. The regression fixture was isolated and removed after the test.

No HP-IGPU-01 fingerprint, formal observation, threshold, corpus, hardware profile, query class, frozen runner, R0 lock, HP-DEV reference, or historical evidence was changed.

## Repair

The report validator now maps every runner stage ID to its canonical frozen evidence paths independently of the manifest:

- `release-build` must bind the canonical build stdout and stderr and must not contain a metric artifact;
- each `core-series-N` stage must bind the canonical core stdout, stderr, and, when recorded, metric path for series `N`;
- each `navigation-series-N` stage must bind the canonical NAV stdout, stderr, and, when recorded, metric path for series `N`;
- a prior PASS or terminal measured FAIL stage cannot omit its canonical metric artifact;
- only an infrastructure-invalid metric stage may terminate without a metric artifact;
- exact manifest evidence membership is computed from these independent canonical paths.

All records remain content-addressed with SHA-256. This is a report-validator-only pre-observation repair and does not modify the frozen measurement runner or any gate threshold.

## Validation

| Check | Result |
|---|---|
| Pre-repair arbitrary stage-path substitution | **PASS** - reproduced as accepted |
| Canonical runner-realizable release-build infrastructure outcome | **PASS** - accepted after repair |
| Same sealed outcome with stdout rebound to `progress-037.md` | **PASS** - rejected with a canonical path mismatch |
| Synthetic regression evidence and scripts cleanup | **PASS** |
| PowerShell parser | **PASS** |
| Real repository without HP-IGPU-01 evidence | **PASS** - failed closed and created no terminal report |
| `report.md`, `report-no-go.md`, and `report-infrastructure-invalid.md` absence | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| `git diff --check` for the repaired validator | **PASS** |
| Repaired validator SHA-256 | **PASS** - `21d8c7e3dd820925ff7f0d264e511890c5fb31f2317aeb88b5b8a914f034a4b3` |

## Gate status

The G8 done-check remains false. `artifacts/gate-c/report.md` is absent, as are the real HP-IGPU-01 fingerprint, attempt claim, run manifest, and six notebook metrics. The repaired validator cannot turn development-host or substituted repository files into a terminal Gate C decision.

## Next action

Provide the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review the exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`; only complete sealed physical-notebook evidence with canonical stage bindings can create the terminal report.
