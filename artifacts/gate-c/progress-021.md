# Gate C Formal-Failure Evidence Diagnosis 021

**Status: a pre-observation failure-path gap must be repaired before the first HP-IGPU-01 formal invocation**

- Diagnosis UTC: `2026-08-02`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- HP-IGPU-01 runner SHA-256: `83a9c9a8d37b615afa7ca5a6209a164a97afe81dc92c2029b64cd0cce47f0d7d`
- Testable assumption: `A7`
- Observation state: no HP-IGPU-01 fingerprint, formal series, run manifest, or Gate C report exists

## Diagnostic finding

The formal runner seals `hp-igpu-01-r0-v12-run-manifest.json` only after all three core and all three NAV processes return success. Each nonzero child exit instead throws immediately. The runner does not persist the child exit code, stdout, stderr, failing stage, or an immutable failed-attempt manifest.

This creates two failure modes on the first irreplaceable notebook run:

1. The core and NAV binaries write their metric JSON before asserting thresholds. A threshold miss therefore leaves one or more immutable series files, but the runner exits without a manifest and refuses a later invocation because formal evidence already exists.
2. A process failure before metric JSON creation leaves no durable runner-owned failure packet at all. The console error is insufficient for the required auditable, evidence-based `NO-GO` report unless the operator independently captured it.

Failing closed and refusing overwrite are correct. The defect is that a legitimate formal failure is not deterministically sealed as immutable evidence. The current passing-only manifest path can prove `GO`, but the operator handoff does not reliably support the promised `NO-GO` path.

## Required focused repair

Before qualification or measurement on HP-IGPU-01, update the runner so every started formal attempt ends with exactly one immutable attempt manifest. The manifest must record the fingerprint hash, runner and executable hashes, ordered stage results and exit codes, hashes of every produced raw artifact, the failing stage when applicable, and a terminal `PASS`, `FAIL`, or infrastructure-invalid decision. Child stdout and stderr must be captured to immutable per-stage files. Existing raw evidence must still never be overwritten or silently resumed.

Because this changes formal measurement orchestration before the first HP-IGPU-01 observation, the repair must be reviewed and frozen before use. No threshold, corpus, hardware profile, source measurement logic, or historical evidence may change. A deterministic simulated child-failure test should prove that a failed attempt is sealed and that a second invocation cannot overwrite it.

## Fresh verification

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | **PASS** |
| PowerShell parser for the current HP-IGPU-01 runner | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| Frozen runner hash | **MATCH** |
| Frozen R0 lock hash | **MATCH** |
| HP-IGPU-01 fingerprint | **ABSENT** |
| HP-IGPU-01 formal run manifest | **ABSENT** |
| Gate C report | **ABSENT** |

No formal HP-IGPU-01 observation was started and no historical evidence changed. The workspace-test portion of the L1 done-criteria passes, while report existence and `GO` remain unmet. L1 #23 stays active.
