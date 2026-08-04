# ADR 0004: V4-P15 Sequence and A0 Disposition

- Status: Amended after A0 v2 FULL_GO — project owner, 2026-08-04; L-01/L-02 withdrawn, M3 halt released
- Date: 2026-08-04
- Decision owner: Architecture lead
- Accountable approver: Project owner
- Evidence: `artifacts/gate-a0/runs/strengthened-run-001/`, `artifacts/gate-a0/diagnostics/a0d-run-002/`, `KETCHUP_ARCHITECTURE_SPECIFICATION_V4.md`

## Context

V4-P15 proposes replacing the frozen V3 sequence with `M0 → durable M1 prerequisites → M2 → M4a-E → M3`, while the remaining M4a completion track starts after M4a-E, may overlap M3, and must finish before M4b or a full-M4a claim. V4 explicitly calls this a sequence change rather than a clarification.

Strengthened A0 v1 returned a substantive NO-GO before its consumer ran. Its report did not preserve process stderr or the Rust panic, so it could not distinguish same-build construction/capture from cross-build transfer. A0-D repaired that evidence defect without changing a threshold or consequence and executed the four planned producer/consumer combinations.

The sealed A0-D result is:

- both frozen OCCT builds compiled and staged successfully under their validated identities;
- prior→prior and current→current producers both exited 101;
- both panics report `Guaranteed role extrusion.side(profile_edge=east) has 0 candidates` after complete reciprocal adjacency had passed;
- prior→current and current→prior producers fail identically;
- all four consumers are explicitly `not_run`, so no cross-build transfer claim can be made;
- the demonstrated defect is shared semantic topology-history/reference capture before transfer, not a demonstrated backend migration failure.

## Decision

The project owner accepts the following disposition. Acceptance is limited to L-01/L-02 and does not weaken the original same-build or cross-build A0 expectations.

1. Adopt the V4-P15 order `durable M1 prerequisites → M2 → M4a-E → M3` after A0-D and the M1-prerequisite inventory are closed.
2. Permit only representation-independent durable M1 prerequisite work, M2 canonical graph/persistence work, and the exact-independent M4a-E beam checkpoint while A0 remains NO-GO.
3. Keep M3, exact product integration, durable `SubshapeRef`, C1b, exact-dependent dimensions, exact notches, and every claim of exact Gate C halted.
4. Do not reinterpret A0-D as migration evidence. Repair the common topology-history/capture defect, preregister A0 v2 before observing it, and retain the original same-build and cross-build expectations unless a separately approved loosen changes them.
5. Bind any eventual `Guaranteed` claim to an explicit backend/evaluator/tolerance identity and evidence envelope. A changed identity always triggers the non-destructive audit in V4 §8.5; it never silently inherits a guarantee.
6. Keep PF0 inactive. It remains an estimated alternative, not an implementation milestone.
7. Retire the old monolithic exact Gate C as a sequence gate only when this ADR is accepted. Its safety intent is split, not discarded: C1a owns canonical projection authority, strengthened A0/A0 v2 owns the narrow exact/reference proof, M4a-E owns exact-independent product semantics, and C1b owns integrated exact picking/reference equivalence.

## Anti-loosening classification

V4 requires every post-observation threshold, envelope, and consequence change to be classified. Narrowing a promised envelope after failure is a `loosen`, even if the replacement test is stricter inside that envelope.

| ID | Proposed or conditional change | Classification | Disposition |
|---|---|---|---|
| L-01 | Allow durable M1 prerequisites, M2, and M4a-E while the recorded A0 consequence halted M1/M2/M3 | **loosen** | **Withdrawn after A0 v2 FULL_GO.** Its basis was the defective A0 observation; it is not retained as an unsupported precaution. |
| L-02 | Replace the old monolithic exact Gate C sequence with C1a + M4a-E-before-M3 + later C1b | **loosen** of the old sequence constraint, with replacement controls | **Withdrawn after A0 v2 FULL_GO.** The substantive A0 NO-GO no longer applies; M3 is released subject to its normal entry criteria. |
| L-03 | If current→current passes but prior→prior fails, redefine A0 as “Guaranteed on the current build” | **loosen** | Not adopted by current evidence; would require a new accepted ADR/preregistration. |
| L-04 | If same-build paths pass but a cross-build path fails, limit `Guaranteed` to identical backend/evaluator/tolerance identity | **loosen** of the strengthened A0 v1 cross-build envelope | Not adopted by current evidence; would require a new accepted ADR/preregistration and V4 §8.5 audit for identity changes. |
| L-05 | Convert durable references permanently from `Guaranteed` to `BestEffort` | **loosen** | Rejected. Scope a guarantee by identity/evidence instead of silently weakening the reference class. |
| N-01 | Repair stderr/panic/exit/build-identity preservation and explicit consumer `not_run` evidence | `neutral` harness correction | Completed by A0-D; no threshold or consequence changed. |
| N-02 | Repair topology history and rerun a preregistered A0 v2 with the original envelope and thresholds | `neutral` defect repair | Required before M3; historical v1 and A0-D evidence remain immutable. |

## Complete A0-D decision tree

1. **Both same-build producers fail before consumers, as observed.** Treat this as a common construction/history/capture defect. Draw no migration conclusion. Keep M3 halted, repair the source defect, and preregister A0 v2 with the original envelope. No threshold narrowing is justified.
2. **Current→current passes and prior→prior fails.** The current implementation may support a current-identity guarantee, while older identities require V4 §8.5 audit/quarantine. Any current-only A0 redefinition is `loosen` L-03 and needs approval before preregistration.
3. **Prior→prior passes and current→current fails.** Treat this as a current-build regression. Do not narrow the guarantee to the prior build; repair or replace the current backend/source path before A0 v2.
4. **Both same-build paths pass and both cross-build paths pass.** The earlier NO-GO is not reproduced. Preserve the original cross-build envelope, repair the harness, preregister A0 v2, and require a conforming GO before M3.
5. **Both same-build paths pass and either cross-build direction fails.** Local capture is viable but transfer is not proven for the failing direction. Any same-identity-only guarantee is `loosen` L-04; changed identities use V4 §8.5 audit and affected branches fail closed.
6. **A build, provenance, or harness preflight fails.** Classify as hash/provenance-only, make no geometry claim, repair the instrument, and issue a new preregistration version before formal observation.
7. **A consumer starts and fails with wrong, ambiguous, or silent identity.** Classify as substantive reference failure. Zero silent misbinding remains non-negotiable; keep M3 halted and choose repair, scoped loosen, or PF0 only through an accepted disposition.

## Replacement gate charter

| Gate / checkpoint | Responsible owner | Entry criteria | Threshold / exit | Deadline and concurrency |
|---|---|---|---|---|
| C1a canonical projection authority | Core + interaction lead | Canonical Snapshot→Projection path and named revision identity exist | Zero authority divergence; no durable interaction-only meaning or history | Already completed in M0; remains a regression gate throughout all later work. |
| Durable M1 prerequisites | Core/IO lead | A0-D closed; this ADR accepted | Representation-independent profile semantics, sole authority, required hierarchy/component semantics, Save/Open identity | Immediately after A0-D. Inventory decides whether the residual work is a small M2 entry block or a separate prerequisite phase. |
| M2 unified graph/persistence | Core/evaluator + IO lead | Durable M1 prerequisites proven | V4 M2 exit: affected-only recompute, stable derivation identity, observational Open without hidden mutation | Before M4a-E; may run while A0 source repair proceeds, but may not add exact-product dependencies. |
| M4a-E early beam checkpoint | Validation/domain lead | M2 exit, frozen collision/contact tolerance, hierarchical slots and overrides | Frozen `415 × 6`, `408 × 5`, `400` positions; stable or explicitly unresolved joints/overrides; undeclared overlap and empty joints fail; grouped piece/length list regenerates | Must complete and be disposed before M3 starts. No integrated OCCT, durable `SubshapeRef`, C1b, exact notches, or full BOM/dimension-chain dependency. |
| Strengthened A0 v2 | Exact/reference lead | Common topology-history/capture defect repaired; immutable preregistration and complete evidence harness | Original inherited thresholds unless an accepted `loosen`; 100% correct identity, zero silent misbinding, complete adjacency, required same/cross-build outcomes | May be repaired in parallel with M2/M4a-E, but a conforming GO is mandatory before M3 entry. |
| M3 exact product integration | Exact + scheduler + application lead | M4a-E disposed, A0 v2 GO, ADR 0002 retained, backend/evaluator/tolerance identity complete | Integrated rectangle/profile → exact extrusion → render/pick → stable reference → Save/Open slice | Starts only after both M4a-E and A0 v2 GO. |
| C1b resolver equivalence | Exact + interaction lead | Accepted exact results enter the product path; preregistered integrated corpus | Zero silent identity mismatches; exact resolver and interaction picking agree or interaction is conservatively downgraded | Runs within M3 and must pass before M3 exit. |
| Full M4a completion | Validation/domain + projection lead | M4a-E observed | M4a-E stays green; protocol availability cannot masquerade as pass; deterministic full BOM/dimension projections | Starts immediately after M4a-E, may overlap M3, and must finish before M4b or any full-M4a claim. |

## PF0 numeric estimate — inactive alternative

The estimate covers a bounded planar/prismatic exact kernel through booleans and joints, not a general CAD kernel. Units are senior-engineer person-weeks and include implementation plus focused tests, but not broad import/export compatibility, curved surfaces, fillets, shelling, constraints, or certification.

| Work package | Optimistic | Plausible high | Required result |
|---|---:|---:|---|
| Frozen scope, tolerance model, predicates, corpora | 3 | 6 | Exact operating envelope and adversarial oracle |
| Canonical planar profiles and robust 2D arrangements | 5 | 10 | Deterministic profile splitting/classification |
| Prismatic B-Rep, manifold adjacency, lineage/history | 6 | 12 | Valid solids and construction-derived references |
| Prism intersections and boolean split/classify/stitch/heal | 14 | 28 | Union/cut/intersection without silent invalid solids |
| Stable-reference mutation and identity-change audit | 6 | 12 | Zero silent misbinding in the bounded envelope |
| Bounded joints, half-laps/notches, collision integration | 8 | 16 | Joint-driven geometry and diagnostics |
| Worker/product/persistence/render-pick/C1b integration | 8 | 14 | One exact product vertical slice |
| Adversarial hardening, fuzzing, packaging, release evidence | 8 | 16 | Repeatable gate-ready evidence |
| **Subtotal** | **58** | **114** | |
| Integration/risk contingency (25–40%) | **15** | **46** | Numerical robustness and topology rework |
| **PF0 chain total** | **73 person-weeks** | **160 person-weeks** | Prism → booleans → joints → integrated proof |

For one senior engineer this is approximately 18–40 person-months. Two engineers do not halve the critical path; a realistic calendar envelope is roughly 12–24 months after scope freeze. The confidence is low-to-medium because boolean robustness dominates the variance. PF0 is a separate engine project, not a practical fallback within Kečup: activating it would replace the product roadmap rather than protect it. It remains inactive unless OCCT/reference repair is exhausted and the project owner explicitly accepts the 73–160 person-week scope, opportunity cost, and new gates.

## Consequences

- A0-D no longer supports the statement “migration failed”; it supports “same-build reference capture fails on both frozen builds before transfer.”
- Acceptance deliberately loosens the original halt for exact-independent substrate work, while preserving a hard stop on M3 and exact claims.
- M2/M4a-E cannot be used as evidence that exact geometry or durable subshape identity works.
- A0 v2 must preserve full process evidence even on panic or early exit.
- Historical strengthened-run-001 and A0-D artifacts remain immutable and separately named.
- PF0 is visible as a costly contingency without becoming an active milestone.

## Acceptance

Accepted by the project owner on 2026-08-04 after verifying that L-01/L-02 change only sequence and consequence. Acceptance does not remove `side(profile_edge=…)` from `Guaranteed`, weaken same-build evidence, pre-authorize L-03/L-04, activate PF0, or change any A0 threshold.
