# Gate C Evidence-Membership Repair 035

**Status: the pre-observation incomplete-manifest acceptance path is repaired and frozen; the physical-notebook blocker remains**

- Repair observation UTC: `2026-08-02T05:11:29.1149565Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired report validator: `scripts/windows/write-gate-c-report.ps1`
- Repaired report validator SHA-256: `22d4662581ec3245e8de400f5307f36353aae5db0f6223e855348b565ed3fba2`
- Superseded pre-observation report-validator SHA-256: `6461e4c98531644bef94ab8236f556484ea70cc6753867ebbf6221a3ca232128`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`

## Reproduced gap

The closure validator rehashed every file record that a run manifest happened to contain, but it did not require the manifest to contain the complete evidence set. A modified PASS manifest could therefore omit the attempt claim or any formal stdout/stderr log and still reach the later metric checks. The attempt claim was parsed separately, but without an exact evidence-membership requirement a post-seal claim mutation that preserved the checked identity fields was not detected by the manifest seal. Stage artifact records were also checked only by metric hash, not by canonical path and not for stdout/stderr.

The gap was found before any HP-IGPU-01 fingerprint or formal observation existed. No threshold, corpus, hardware profile, query class, measurement source, runner, R0 lock, HP-DEV observation, or historical gate artifact was changed.

## Repair

For a PASS decision, the report validator now requires exactly the 21 canonical runner artifacts: the attempt claim, two build logs, six metrics, and twelve series logs. Missing, duplicate, substituted, or additional file records fail closed. Every release-build, core, and NAV stage record is independently bound to its canonical stdout, stderr, and metric path and current SHA-256; the build stage must not claim a metric. The run manifest must also bind the canonical fingerprint path and the same attempt start timestamp as the attempt claim.

For terminal FAIL or infrastructure-invalid attempts, exact membership is derived from the attempt claim plus every artifact record of the stages that actually ran, preserving valid early-stop manifests while preventing unsealed stage evidence.

## Validation

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Synthetic omitted-record regression | **PASS** - a one-of-two record set was rejected by exact cardinality |
| Synthetic complete-set regression | **PASS** - the complete two-record isolated set was accepted and rehashed |
| Synthetic stage-misbinding regression | **PASS** - a valid hash bound to the wrong canonical path was rejected |
| Temporary synthetic evidence cleanup | **PASS** - no `.evidence-membership-test-*` directory remains |
| Real repository without HP-IGPU-01 evidence | **PASS** - failed closed and created no terminal report |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| Repaired validator SHA-256 | **PASS** - `22d4662581ec3245e8de400f5307f36353aae5db0f6223e855348b565ed3fba2` |
| Whitespace validation | **PASS** |

## Gate status

The G8 done-check remains false. `artifacts/gate-c/report.md` is absent, as are the real HP-IGPU-01 fingerprint, attempt claim, run manifest, and six notebook metrics. No GO report was fabricated from synthetic or development-host evidence.

## Next action

Provide the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review the exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`; only a complete, canonically bound, fully validated PASS set can create `artifacts/gate-c/report.md`.
