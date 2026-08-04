# Gate A1 Decision Report

**Decision: GO**

## Scope

Gate A1 validates the canonical document vertical slice required by the frozen Architecture V3 and the thresholds preregistered in `thresholds/r0.yaml`. The evaluated implementation provides immutable revisions, node-level structural sharing, atomic `CommandBatch` commits, one-step batch undo/redo, dependency-scoped Proposal validation, versioned canonical commands, deterministic persistence, explicit legacy migration loss, and equivalent UI/RPC/CLI canonicalization.

The source baseline before the A1 working tree changes was commit `f0a0cf3afa9df45682fe8723dacc99cb8e153058`. No frozen threshold, corpus, hardware profile, or failure consequence was changed after observation.

## Frozen-threshold results

| Metric | Threshold | Observed | Result |
|---|---:|---:|---|
| Canonical parameter or ID changes after 100 save/load cycles | 0 | 0 | PASS |
| UI/RPC canonical digest and invariant equivalence | 100% | 100% across UI, RPC, and CLI | PASS |
| Atomic rollback without partial mutation | 100% | 100% | PASS |
| Independent-node recompute after local change | 0 | 0 | PASS |
| Authoritative precision values without drift | 100% | 100% | PASS |
| Old-schema meaning preserved or explicit loss reported | 100% | 100% | PASS |

## Additional contract evidence

- A successful multi-command batch is one visible undo/redo step.
- Unchanged canonical nodes retain shared `Arc` identity between immutable snapshots.
- A Proposal remains valid after an unrelated edit but is rejected after a relevant read-set change; no silent rebase occurs.
- The current document format stores IDs, source tokens, exact `f64` bit patterns, dependencies, names, schema, and revision in deterministic little-endian order.
- The legacy fixture preserves the authoritative binary dimension and reports the unrecoverable source token as structured migration loss.
- Exact and mesh geometry types are not conflated by this canonical core.

## Verification

- `cargo test -p ketchup-core --test gate_a1`: 7 passed, 0 failed.
- `cargo test --workspace --all-targets`: passed, including A1 and the existing A0 suite.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.

Machine-readable observations are recorded in `artifacts/gate-a1/metrics.json`.

## Consequence

All preregistered A1 thresholds passed, so Gate B may begin. Any later change to the canonical document, command schema, persistence format, precision contract, Proposal validity, or adapters must preserve these tests or produce a new explicit gate decision; this report does not create a stable long-term public file-format promise.
