# Strengthened Gate A0 v2 Report

- Run: `strengthened-v2-r3-run-001`
- Native observation reached: True
- Inherited backend suites: 2/2
- Matrix combinations: 4/4
- Failure class: `none`
- **Decision: FULL_GO**

| Combination | Producer state / exit | Consumer state / exit | Active resolved | Negative control | Pass |
|---|---:|---:|---:|---|---:|
| prior-to-prior | ran / 0 | ran / 0 | 3/3 | lost | True |
| current-to-current | ran / 0 | ran / 0 | 3/3 | lost | True |
| prior-to-current | ran / 0 | ran / 0 | 3/3 | quarantined | True |
| current-to-prior | ran / 0 | ran / 0 | 3/3 | quarantined | True |

## Disposition

A0 v2 passes both frozen backends and all four same/cross-build directions. Release M3, withdraw L-01/L-02 from ADR 0004, leave L-03/L-04 unadopted, and keep PF0 inactive.

The negative control is a real north-face fingerprint under an intentionally absent semantic role. `Resolved` or `Ambiguous` is forbidden. Historical v1 and A0-D artifacts were not modified.
