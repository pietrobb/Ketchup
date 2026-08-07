# ADR 0006: Canonical and Derived-Result Write Paths

- Status: Accepted implementation consequence — project owner, 2026-08-04
- Date: 2026-08-04
- Decision owner: Core/IO lead
- Accountable approver: Project owner
- Evidence: `crates/ketchup-core/src/document.rs`, `governance/d08-lifecycle-exceptions.json`, `scripts/windows/test-architecture-guards.ps1`, `scripts/windows/test-ci-guard-red-paths.ps1`
- Scope: Implements the immediate D-08 / V4-P07 / V4-P08 consequence requested after the audit of commit `10a7bea`; it is not the V4 adoption ADR or the broader dedicated P07/P08 ratification record required by V4C §3.5.

## Context

`register_exact_reference_evidence` previously replaced the active revision snapshot directly. The canonical digest and revision identity stayed unchanged, but the write did not pass through a named derived-result gateway. This created an ungoverned third implementation path beside `DocumentStore::apply_batch` and lifecycle construction/loading. The existing architecture guard also concentrated on public mutators and did not reject every private write to `DocumentStore` authority from inside `ketchup-core`.

V4-P07 distinguishes a semantic canonical change from registration of a semantics-preserving evaluated result. V4-P08 requires derived results to stay bound to the canonical document/revision/digest envelope and outside canonical digest authority.

## Decision

There are exactly two semantic write paths into an active document:

1. **Canonical path — `DocumentStore::apply_batch`.** Any change to canonical meaning is a validated `CommandBatch`, appends exactly one revision, and creates exactly one visible Undo step. Existing canonical delegates are `commit_verified_proposal`, `make_unique`, and `convert_group_to_component`; each must call `apply_batch`. The compatibility wrapper `commit_proposal` must delegate only to `commit_verified_proposal`.
2. **Non-canonical P07 path — `DocumentStore::register_derived_result`.** A semantics-preserving result must carry the current `document_id`, `revision_id`, and canonical digest. The gateway checks this envelope at runtime and fails closed on mismatch. It may update only derived-result storage, must not advance or append a revision, must not create an Undo step, and must not change the canonical digest.

The currently admitted P07 payloads are:

- evaluator result registration through `register_evaluation`;
- exact-reference provenance evidence through `register_exact_reference_evidence`.

Both public methods validate payload-specific evidence before delegating to `register_derived_result`. Adding another derived payload requires updating this ADR's evidence register and the deliberate-red guard coverage; it must not add another document write mechanism.

## Lifecycle and construction classification

The following operations are not a third semantic write path:

- `DocumentStore::new` and crate-private `from_product` construct a separate fully validated store;
- successful Open decodes and validates a separate candidate, and swaps it into the application only if it is editable and lossless;
- failed or review-only Open leaves the active editable document unchanged;
- Undo/Redo select an immutable revision by moving only the cursor;
- `discard_history_before_current` changes retention only.

Open remains observational: it does not evaluate, recompute, migrate canonical meaning, or manufacture missing derived evidence. Exact-reference evidence may round-trip in schema 5, but it remains excluded from canonical digest authority. Evaluator registry entries are replaceable derived runtime results and are not canonical history.

## Mechanical enforcement

`governance/d08-lifecycle-exceptions.json` enumerates the canonical gateway, canonical delegates, P07 gateway, P07 delegates, construction paths, and lifecycle exceptions. `test-architecture-guards.ps1` rejects:

- public or private `DocumentStore` authority mutation outside the reviewed blocks;
- a canonical delegate that stops calling `apply_batch`;
- a P07 delegate that bypasses `register_derived_result`;
- a P07 gateway without runtime document/revision/digest checks;
- revision, cursor, or Undo effects inside the P07 gateway;
- public persistence construction or mutation after candidate validation.

`test-ci-guard-red-paths.ps1` deliberately injects a private in-core revision write and requires the sole-mutation guard to reject it, in addition to the existing public-mutator, post-validation, Undo, persistence-constructor, and lifecycle-register red paths.

## Consequences

- Exact-reference registration is explicitly non-canonical while remaining available to Save/Open and product queries.
- Canonical digest, revision count, and visible Undo/Redo counts are invariant under accepted P07 events.
- Release builds enforce the derived-result envelope; correctness does not depend on `debug_assert`.
- The two-path invariant is machine-enforced inside `ketchup-core`, not merely at crate boundaries.
- Broader P07/P08 decisions such as long-term derived-result retention, compatibility across changed backend identities, and V4 ratification remain owned by the dedicated record required by V4C §3.5.
