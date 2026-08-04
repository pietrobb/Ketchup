# Gate C Runner/Reporter Transition Repair 037

**Status: the pre-observation runner/report-validator incompatibility is repaired and frozen; the physical-notebook blocker remains**

- Repair observation UTC: `2026-08-02T05:29:36.4465232Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired report validator: `scripts/windows/write-gate-c-report.ps1`
- Repaired report validator SHA-256: `5ab7d75a5ae49f3ae3d9e6dcf6a1b374d82132b0080cb52fa46e2aa6497e2638`
- Superseded pre-observation report-validator SHA-256: `0f0a0199c5088bd671fe0e1fc4494a10e08a3525ed44a3927091ae8de48b71c7`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`

## Reproduced incompatibility

The frozen runner classifies a stage as `INFRASTRUCTURE_INVALID` in three distinct cases: a process launch exception, an ordinary process result without a valid required metric, and a nonzero release-build process result. Only the first case carries `launch_error`; the other two carry `exit_code` and no launch error. The progress-036 report validator incorrectly required every recorded infrastructure-invalid stage to contain a non-empty launch error, so it rejected runner-realizable terminal evidence such as a release build exiting with code 101.

The mismatch was reproduced by extracting the actual validator functions and presenting the exact frozen-runner transition: canonical `release-build`, decision `INFRASTRUCTURE_INVALID`, exit code 101, null launch error, matching failing-stage identity, and a non-empty failure message. The pre-repair validator rejected it with `An infrastructure-invalid recorded stage lacks its launch error.`

The gap was found before any HP-IGPU-01 fingerprint or formal observation existed. No threshold, corpus, hardware profile, query class, measurement source, runner, R0 lock, HP-DEV observation, or historical gate artifact was changed.

## Repair

Terminal-stage validation now distinguishes the runner's process-launch and process-result paths:

- a measured `FAIL` must have no launch error and must carry a nonzero process exit code;
- an infrastructure launch exception must carry a launch error and no process exit code;
- an infrastructure-invalid process result must carry an exit code even though it has no launch error;
- a release build with exit code zero cannot be labeled infrastructure-invalid;
- the previously frozen canonical stage-prefix, failing-stage identity, terminal-decision, failure-message, and executable-verification constraints remain unchanged.

This is a reporter-only pre-observation repair. It makes the closure validator accept every relevant frozen-runner terminal shape without allowing impossible missing-process evidence or a zero-exit measured failure.

## Validation

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Reproduced ordinary nonzero release-build infrastructure result | **PASS** - accepted after repair |
| Process launch exception with no exit code | **PASS** - accepted |
| Zero-exit metric stage with missing/invalid metric semantics | **PASS** - accepted |
| Measured nonzero metric failure | **PASS** - accepted |
| Infrastructure stage lacking both launch error and exit code | **PASS** - rejected |
| Zero-exit release build labeled infrastructure-invalid | **PASS** - rejected |
| Zero-exit metric stage labeled measured `FAIL` | **PASS** - rejected |
| Real repository without HP-IGPU-01 evidence | **PASS** - failed closed and created no terminal report |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| Repaired validator SHA-256 | **PASS** - `5ab7d75a5ae49f3ae3d9e6dcf6a1b374d82132b0080cb52fa46e2aa6497e2638` |

## Gate status

The G8 done-check remains false. `artifacts/gate-c/report.md` is absent, as are the real HP-IGPU-01 fingerprint, attempt claim, run manifest, and six notebook metrics. No GO, NO-GO, or infrastructure-invalid report was fabricated from synthetic or development-host evidence.

## Next action

Provide the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review the exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`; only complete sealed physical-notebook evidence can create the terminal report.
