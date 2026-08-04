# Gate B Decision Report

**Decision: GO**

## Scope

Gate B validates the snapshot scheduler, stale-result rejection, exact-backend isolation, crash recovery, cancellation, concurrent readers, Proposal digest enforcement, worker transport, bounded derived cache, and process-memory contract required by frozen Architecture V3 and `thresholds/r0.yaml`.

The implementation adds a dedicated `ketchup-scheduler` crate. Its parent-owned scheduler accepts derived results only when revision, generation, and input digest match; its LRU cache has an explicit 64 MiB budget; and its persistent exact worker returns typed geometry evidence without receiving mutable canonical document state. ADR `docs/adr/0002-exact-backend-isolation.md` selects the worker as the FLP default. The previous splash ADR was renumbered from 0002 to 0003 to keep ADR identifiers unique; its decision text was not otherwise changed.

No frozen threshold, corpus, hardware profile, query class, or failure consequence was changed after observation. The source baseline before the uncommitted A1 and Gate B working-tree changes was commit `f0a0cf3afa9df45682fe8723dacc99cb8e153058`.

## Frozen-threshold results

| Metric | Threshold | Observed | Result |
|---|---:|---:|---|
| Schedule permutations | at least 10,000 | 10,000 | PASS |
| Stale result inserted as current | 0 | 0 | PASS |
| Crash-recovery runs | at least 100 | 100 | PASS |
| Worker crash damages committed revision | 0 | 0 | PASS |
| C++ exception crosses transport/FFI contract | 0 | 0 in 100 probes | PASS |
| Changed relevant Proposal read digest accepted | 0 | 0 | PASS |
| Reader blocking above 100 ms | 0 | 0 in 3,000 samples | PASS |
| Concurrent reader query p95 | at most 16.7 ms | 0.0001 ms | PASS |
| Worker cancellation p95 | at most 250 ms | 0.8146 ms across 300 samples | PASS |
| Worker transport p95 | at most 15 ms | 0.0603 ms | PASS |
| Worker transport share p95 | at most 20% end-to-end | 2.430398% | PASS |
| Derived cache budget | at most 512 MiB | 64 MiB | PASS |
| Post-warm-up cache growth per 100 edits | at most 1 MiB | 0 bytes | PASS |
| Process private bytes | at most 2 GiB | 3,080,192 bytes | PASS |

All latency criteria passed in three consecutive complete release-mode series. The aggregate worker end-to-end p95 was 3.0134 ms; the comparison in-process p95 was 3.0465 ms. Raw cancellation, reader, worker, transport, transport-percentage, and in-process samples are preserved in `artifacts/gate-b/metrics.json`; no outliers were removed.

## Recovery and ownership evidence

- The worker was deliberately terminated with an abnormal process abort in each crash-recovery run.
- The canonical document remained in the parent and retained the same canonical digest after every crash and cancellation.
- Restarting the worker regenerated exact output from canonical inputs; worker state and cache were treated as disposable.
- Native exception probes crossed the worker boundary only as the stable typed code `backend_exception`.
- Long geometry work executed outside the scheduler lock, so snapshot/query readers did not wait on the exact operation.
- Stale results were tested in both old-first and current-first completion orders across all 10,000 permutations.
- Cache use plateaued exactly at the selected budget and deterministic LRU eviction continued without growth.

## Hardware and measurement envelope

The authoritative run used `cargo test --release -p ketchup-scheduler --test gate_b -- --nocapture` on frozen profile `HP-DEV-01`, with Rust 1.97.0 and the frozen OCCT 8.0.1 backend. Timing used Rust's monotonic `Instant`, equivalent to the preregistered Windows high-resolution monotonic clock. Warm-up samples were excluded, nearest-rank p95 was used, and no outlier was removed.

`HP-IGPU-01` is still required for Gate C. Gate B does not claim an unobserved notebook fingerprint or substitute this workstation for the later mandatory Gate C machine.

## Verification

- `cargo test --release -p ketchup-scheduler --test gate_b -- --nocapture`: 1 passed, 0 failed; formal harness duration 167.10 seconds.
- `cargo clippy -p ketchup-scheduler --all-targets -- -D warnings`: passed.
- `scripts/windows/validate-r0-v6-preregistration.ps1`: passed; only the local scheduler package changed the dependency lock.
- Immutable A0 `run-006` under `r0-v6`: GO with 10,000/10,000 fuzz calls, 24/24 Guaranteed identities, zero silent invalid/wrong outcomes, and 3/3 STEP fixtures.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, and `git diff --check`: passed after A0, A1, and Gate B were combined.
- The formal harness itself verifies the 10,000 schedule permutations, 100 crash recoveries, three independent performance series, Proposal race, cache plateau, and private-memory ceiling.

## Consequence

All frozen Gate B correctness, recovery, concurrency, cancellation, transport, cache, and memory criteria passed, so Gate C may begin. The persistent exact worker is the selected product direction; Gate C must preserve canonical-command mutation, parent-owned revisions, typed geometry results, and strict revision/generation/digest stale-result rejection.
