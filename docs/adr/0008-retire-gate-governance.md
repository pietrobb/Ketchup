# ADR 0008: Retire gate governance; keep only checks that protect data

- Status: Accepted
- Date: 2026-09-24
- Decision owner: Project owner
- Supersedes: ADR 0004, ADR 0005, and sections 11, 12 and 16 of the archived execution contract (`docs/archive/EXECUTION_CONTRACT.md`)

## Context

The project grew a process layer:
- preregistered gates (R0, A0, A1, B, C, D) with freeze IDs and frozen SHA-256 input locks;
- a register of contract changes;
- CI jobs that verified historical gate results and hashed their own tools;
- release scripts that hash-checked ADR text.

R0 alone was re-frozen 13 times, often for toolchain or formatting changes.

This process did not make the modeler more capable or more correct. It did make every change slower. Coding agents also spent their effort satisfying the process instead of the product. Roughly 40 % of test assertions checked process artifacts (digests, revisions, rejection codes) rather than geometric or manufacturing outcomes.

## Decision

1. The gate sequence, preregistration, freeze IDs, frozen input locks, the contract-change register and the NO-GO diagnostic-hold procedure are retired. Their scripts, locks, reports and corpora manifests are removed from the working tree; git history keeps them.
2. Only these checks remain mandatory:
   - one mutation path (`apply_batch`), atomic batches and one undo step per batch;
   - asynchronous exact results carry a revision number, and stale results are dropped;
   - OCCT runs isolated in a worker process with timeouts;
   - exact geometry is never silently replaced by a mesh;
   - production export is blocked by validation issues;
   - saved files carry a checksum.
3. CI runs formatting, Clippy, the workspace tests, the Python tests and the named-product ratchet (`scripts/check_no_named_products.py`). Nothing else.
4. New digests, stamps, epochs, receipts, signatures, preregistrations or evidence documents need a concrete, reproduced failure that they catch, or explicit approval by the owner.
5. Coding agents follow `AGENTS.md`.

## Consequences

- Changes are judged by their tests and by the product: geometry, bill of materials, drilling plans and exports.
- Historical gate evidence is only available through git history.
- The OCCT build manifest (`artifacts/r0/occt-build-manifest.json`) stays because the packaging scripts use it to collect runtime libraries.
