# Gate C Formal-Attempt Sealing Repair 022

**Status: the pre-observation formal-failure evidence gap is repaired and the runner is frozen for HP-IGPU-01 qualification**

- Repair UTC: `2026-08-02`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- HP-IGPU-01 runner SHA-256: `cc434d807dfe429956035052874e260c58c6b21f970376b8ebd86531cb56faeb`
- R0 v12 validator SHA-256: `339e66e2cddc5db658f0c2948fa925c7fb27a6798fafb43fb5665e72902a467e`
- Testable assumption: `A7`
- Observation state: no HP-IGPU-01 fingerprint, attempt claim, stage log, metric series, run manifest, or Gate C report exists

## Focused repair

The Windows runner now creates an immutable attempt claim atomically before the first formal child process. This closes the check-then-start concurrency gap: two invocations cannot both begin measurement or share evidence paths. Every normally terminated formal attempt writes exactly one exclusive terminal run manifest, and any existing claim, log, metric file, or manifest makes a later invocation fail before measurement.

The release build and each of the three core and three navigation series capture separate immutable stdout and stderr files. Ordered stage records preserve start and completion times, exit codes, log hashes, and metric-artifact hashes. A nonzero child exit with a structurally valid matching HP-IGPU-01 metric artifact is classified as `FAIL`; a launch error, missing or invalid metric artifact, invalid executable, or runner error is classified as `INFRASTRUCTURE_INVALID`. All successful stages produce `PASS`. The terminal manifest records the fingerprint hash, frozen runner hash, actual executable hashes, failing stage, failure message, all stage results, and hashes of every produced evidence file.

The repair changes no threshold, corpus, expected outcome, hardware profile, oracle, consequence, product source, measurement source, or observed binary. The existing r0-v12 lock remains byte-identical. Because no HP-IGPU-01 fingerprint or observation existed, the repaired orchestration was pinned before qualification by its SHA-256 in the R0 v12 validator; the future fingerprint also records this exact runner hash.

## Deterministic failure verification

`-VerifyAttemptSealing` runs entirely in a unique temporary directory and exercises the same production stage and manifest helpers. It proves that:

1. a simulated child that writes matching metric JSON and exits `7` is sealed as `FAIL` with stdout, stderr, metric, and exit-code evidence;
2. a simulated child that exits `9` without a metric artifact is classified as `INFRASTRUCTURE_INVALID`; and
3. a second attempt to write the same terminal manifest is rejected by exclusive file creation.

The validator pins the repaired runner hash, verifies the required sealed-attempt contract, and executes this self-test on every preregistration validation. The runner's native-process wrapper temporarily uses nonterminating stderr handling only around the redirected child invocation so Windows PowerShell cannot discard the real exit code; the global fail-closed error policy is restored immediately afterward.

## Fresh verification

| Check | Result |
|---|---|
| PowerShell parser: repaired runner and validator | **PASS** |
| Deterministic threshold-failure classification and sealing | **PASS** |
| Deterministic infrastructure-invalid classification | **PASS** |
| Immutable manifest overwrite rejection | **PASS** |
| Atomic attempt-claim contract | **PASS** |
| R0 v12 preregistration validator, including pinned runner self-test | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** |
| Available desktop HP-IGPU-01 qualification | **EXPECTED REJECTION** before fingerprint, claim, logs, or metrics |
| `git diff --check` | **PASS** |
| Frozen R0 lock hash | **MATCH** |
| HP-IGPU-01 fingerprint, attempt claim, stage logs, metrics, and run manifest | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

The workspace-test portion of the L1 done-criteria passes. Report existence and `GO` remain unmet, so L1 #23 stays active.

## Next action

Provide the first qualifying 2023–2026 Windows 11 notebook with exactly one operational integrated Direct3D 12 GPU. Run qualification-only to freeze its fingerprint under runner `cc434d807dfe429956035052874e260c58c6b21f970376b8ebd86531cb56faeb`, review the immutable configuration, then run the six formal series once. Use the sealed terminal manifest and raw evidence to issue `artifacts/gate-c/report.md` as evidence-based `GO`, `NO-GO`, or infrastructure-invalid rather than rerunning or weakening the gate.
