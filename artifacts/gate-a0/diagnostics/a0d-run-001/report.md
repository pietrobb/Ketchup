# A0 Diagnostic Matrix v1

- Run: `a0d-run-001`
- Strengthened A0 v1 lock: `5ae34bdd0eb7cad4719c11154e57e5ec8d955d51313e7ffb14ff5f96809a7ff0`
- Gate effect: diagnostic only
- Threshold/consequence change: none; no loosen and no tightening
- Combinations observed: 4/4
- Combinations passed: 0/4

| Combination | Producer state / exit | Consumer state / exit | Pass |
|---|---:|---:|---:|
| prior-to-prior | ran / 101 | not_run /  | False |
| current-to-current | ran / 101 | not_run /  | False |
| prior-to-current | ran / 101 | not_run /  | False |
| current-to-prior | ran / 101 | not_run /  | False |

## Diagnosis

At least one same-build path failed. The evidence localizes the problem before cross-build transfer (build-specific construction, adjacency, capture, or consumer behavior); inspect the sealed process stderr.

Every launched process has immutable stdout/stderr files, exit code, command, build identity, and panic detection in `processes.json`. Every skipped consumer is explicit as `not_run` with a reason.
