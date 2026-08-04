# ADR 0002: Exact Backend Isolation

- Status: Accepted
- Date: 2026-08-01
- Decision owner: Project owner assisted by AI agents
- Gate evidence: `artifacts/gate-b/metrics.json`

## Context

Architecture V3 requires one logical exact-backend contract that can run in-process or in a worker, but it leaves the production isolation mode to Gate B evidence. The decision must preserve immutable canonical revisions, reject stale derived results, contain native exceptions, recover from a worker crash without damaging committed data, keep readers responsive, bound derived-cache memory, and satisfy the frozen `QC-B-TRANSPORT-01` overhead limits.

The in-process variant has the lowest conceptual complexity, but an unrecoverable native fault would share the application process. A persistent worker adds process and transport machinery while making cancellation and crash recovery enforceable without terminating a Rust thread or exposing mutable canonical state to the backend.

## Evidence

The formal release harness in `crates/ketchup-scheduler/tests/gate_b.rs` ran against the thresholds frozen in `thresholds/r0.yaml` and recorded every timing sample in `artifacts/gate-b/metrics.json`.

- 10,000 scheduling permutations produced zero stale-current inserts.
- 100 deliberate worker aborts produced zero changes to the last committed canonical revision.
- 100 native exception probes were returned as typed `backend_exception` results; no exception crossed the transport or FFI contract.
- A Proposal whose relevant read-set digest changed was rejected.
- Three complete 1,000-sample reader series had an aggregate p95 of 0.0001 ms and no blocking over 100 ms while a killable geometry job was active.
- Three complete 100-sample cancellation series had an aggregate p95 of 0.8146 ms, below 250 ms, with no committed-data loss.
- Three complete 1,000-sample transport series measured worker end-to-end p95 at 3.0134 ms, transport p95 at 0.0603 ms, and transport p95 at 2.430398% of end-to-end time. The matching in-process p95 was 3.0465 ms. Both frozen worker-overhead limits passed.
- The derived-cache accounting plateaued at the selected 64 MiB budget, with zero post-warm-up growth per 100 edits and 336 deterministic evictions. The measured harness private bytes were 3,080,192, below the frozen 2 GiB ceiling.

The measurements were made on the frozen `HP-DEV-01` Windows reference workstation. `HP-IGPU-01` remains mandatory for Gate C as specified by R0; it is not substituted or fabricated here.

## Decision

Ketchup selects a **persistent exact-backend worker process** as the default production isolation mode for exact geometry operations in the First Lovable Product.

The contract is:

1. The authoritative `DocumentStore`, committed revisions, canonical commands, and Proposal validation stay in the parent process. The worker never owns or mutates canonical document state.
2. Every scheduled result carries `revision_id`, per-node `generation`, and `input_digest`. The parent inserts it only when all three still match the current scheduled job; otherwise it is discarded as stale.
3. Worker requests return only typed geometry success or typed failure plus validity, bounds, volume, topology counts, and result fingerprint. Native pointers and OCCT types never cross the boundary.
4. A worker crash invalidates its disposable state and derived cache. The parent retains the last committed revision, restarts a clean worker, and reschedules from canonical inputs.
5. Cancellation of a killable exact job terminates the worker process and restarts it. No Rust or C++ thread is force-killed in-process.
6. Derived cache entries are revision-tagged and charged against an explicit 64 MiB LRU budget for the current narrow FLP. An entry larger than the budget is not cached. Changing this budget requires new measured evidence but not a canonical-format migration because cache data is disposable.
7. The in-process path remains available only as the comparison/reference path and for focused tests. Making it the product default or silently falling back to it after a worker failure requires a new ADR and crash evidence.
8. The Gate B line protocol is an internal proof transport, not a stable public API. Product evolution may replace its encoding, but it must preserve the typed logical contract and repeat Gate B transport, crash, and stale-result tests.

## Consequences

- A native process crash or cancellation cannot corrupt the parent-owned canonical document.
- Exact operations pay a measured transport cost, but the frozen latency and percentage limits pass with substantial margin.
- The scheduler can keep snapshot/query readers independent from long exact jobs.
- Worker-local caches are always disposable and cannot be the only carrier of document meaning.
- Packaging must include and supervise the worker executable together with the replaceable OCCT shared libraries.
- Gate C may begin using the persistent worker design; it may not bypass revision/generation/digest acceptance or canonical commands.
