# Ketchup Architecture Specification V4

## Current system, target architecture, and migration contract

- **Status:** Proposed for external architecture review
- **Snapshot date:** 2026-08-03
- **Language:** English, per ADR 0001
- **Supersedes for current architectural review:** `KETCHUP_ARCHITECTURE_PROPOSAL_V3.md`
- **Does not erase:** V3, frozen gate evidence, accepted ADRs, or historical reports
- **Normative baseline:** `docs/design/EXECUTION_CONTRACT.md`
- **Initial platform:** Windows
- **Project license:** Apache-2.0

> This document is deliberately both an as-built description and a target-state specification. It does not claim that planned components already exist. Every material subsystem is classified as **IMPLEMENTED**, **PARTIAL—PRODUCT PATH**, **PARTIAL—PROOF ONLY**, or **PLANNED**, and every new post-V3 architectural choice is marked **PROPOSED** until ratified under §§3.5 and 14.3; an accepted review disposition alone is not ratification.

---

# 0. How to read this specification

## 0.1 Status vocabulary

| Label | Meaning |
|---|---|
| **IMPLEMENTED** | Present on the relevant production or user path and supported by focused executable evidence. The implementation may still have explicitly listed limits. |
| **PARTIAL—PRODUCT PATH** | The intended product/user path exists, but a bounded part of its stated contract or focused evidence is missing. |
| **PARTIAL—PROOF ONLY** | Executable implementation or evidence exists only in an isolated crate, legacy research path, proxy, or synthetic harness, or is disconnected from a required subsystem. It MUST NOT be claimed as product capability. |
| **PLANNED** | Required or intended target behavior without an implementation satisfying the intended product path. |
| **RATIFIED** | A V3 or accepted-ADR decision retained without semantic change. |
| **PROPOSED** | A post-V3 decision recommended by this document but not yet accepted as a binding ADR. |
| **OPEN** | A decision that this document intentionally does not invent. It needs an owner, evidence, and a decision deadline. |
| **HISTORICAL EVIDENCE** | A valid result for the frozen source, corpus, thresholds, and hardware of that run; not automatically proof for the current working tree. |
| **CURRENT EVIDENCE** | A test or inspection run against the 2026-08-03 working tree. |

“Implemented” never means “the final product is complete.” It means the specifically stated contract exists. Conversely, “planned” is not a criticism of a prototype; it is a guard against treating intended architecture as present architecture. `PARTIAL` MUST NOT appear unqualified: every partial claim names whether a product path or proof-only island exists, the missing boundary, and the evidence required for promotion. Implementation maturity, decision status, and evidence status are orthogonal.

## 0.2 Normative language

The words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are normative. Descriptive statements under an **As built** heading report observed code and are not permission to preserve accidental behavior.

## 0.3 Precedence

When sources disagree, use this order:

1. accepted ADRs for the decision they own;
2. `docs/design/EXECUTION_CONTRACT.md` for frozen product and architectural invariants;
3. this V4 specification only after ratification under §§3.5 and 14.3;
4. `docs/design/README.md` for interaction, layout, states, and copy;
5. `docs/design/IMPLEMENTATION_PLAN.md` for workflow order;
6. frozen gate preregistration for the exact run it governs;
7. implementation and tests as evidence of current behavior, never as silent authority to rewrite the contract.

The Slovak post-V3 discussion files under `docs/` are design evidence. Their recommendations are not accepted ADRs merely because code or later discussion refers to them.

## 0.4 Evidence rule

A capability claim MUST name:

- the canonical state or service that owns the behavior;
- the public path by which it is exercised;
- the focused executable evidence;
- the boundary beyond which the claim no longer holds.

A green test that exercises a proxy, synthetic harness, or isolated crate MUST NOT be described as an end-to-end product proof.

---

# 1. Executive summary

## 1.1 Product thesis

Ketchup is intended to become a fast, open-source, desktop 2D/3D parametric modeler for architecture, interiors, furniture, and fabrication-oriented rule systems. It combines:

- direct, discoverable SketchUp-like manual interaction;
- exact, unit-safe, revisioned canonical meaning;
- a replaceable exact B-Rep backend;
- deterministic rules and validators;
- safe optional AI that proposes changes through the same command boundary as UI, CLI, and plugins.

AI is not the product’s source of truth and is never required for ordinary modeling. The long-term differentiator is not “CAD with a chat box.” It is a model whose rules, derived pieces, joints, dimensions, and validations remain understandable and recomputable when a governing parameter changes.

## 1.2 Honest current-state statement

As of 2026-08-03, Ketchup is **not yet one integrated general CAD system**. It contains three substantial but incompletely connected islands:

1. **A narrow canonical manual modeler — PARTIAL—PRODUCT PATH.** It has immutable document revisions, atomic command batches, definitions, occurrences, groups, profile/extrusion features, Move/Copy, Group/Ungroup, shared definitions, Make Unique, edit context, Undo/Redo, and atomic binary Save/Open. Its visible geometry is effectively cuboid-only; promotion requires completion of its declared narrow shell contract and physical-window evidence.
2. **An exact OCCT proof — PARTIAL—PROOF ONLY.** It has an exception-contained C++ façade, simple rectangular extrusion, box cut, STEP import, shape validation, fingerprints, and a narrow topological-reference proof. Exact results do not yet enter the canonical product model or desktop viewport; promotion requires the M3 product slice and C1b.
3. **A scheduler/worker proof — PARTIAL—PROOF ONLY.** It has revision/generation/input-digest stale rejection, a bounded LRU cache, a persistent line-protocol worker proof, cancellation/crash experiments, and accepted ADR 0002. The product application does not yet use this worker path; promotion requires integrated supervision and product-path evidence.

The interaction and rendering path currently projects canonical product entities into `SharedBoxGeometry` and painted cuboid faces. It is a derived proxy, not an OCCT-backed exact scene. Gate C evidence over that path MUST NOT be interpreted as proof of exact-body picking or general curved-body interaction.

## 1.3 Current strengths

The following foundations are real and should be retained:

- private canonical entity fields and immutable snapshots;
- one candidate-state `apply_batch` path for ordinary canonical edits;
- one successful user batch per visible Undo step;
- rollback on invalid multi-command batches;
- typed product IDs and deterministic ordered maps;
- definition/occurrence sharing and atomic Make Unique;
- application-owned ephemeral previews rather than continuous document mutation;
- designed shell, command registry, localized resources, Outliner, value box, and headless UI harness;
- atomic Save and failure-safe Open;
- OCCT exception containment and narrow exact validity checks;
- stale-result rejection primitives and process-isolation decision;
- frozen R0 methodology and historical gate artifacts.

## 1.4 Critical architectural gaps

The highest-risk gaps are:

1. the legacy `CanonicalNode` dependency graph and the product `Feature` graph coexist as parallel canonical structures rather than one evaluator-owned graph;
2. no expression evaluator computes dependent values or product outputs;
3. exact bodies, mesh bodies, feature results, evaluator identity, backend provenance, and `SubshapeRef` are not part of the product document or native file;
4. interaction identity is box-specific and disconnected from exact topology identity;
5. the accepted exact worker is not integrated into the application;
6. the native file lacks the V3 container, checksums, resource limits, per-node digests, compatibility audit, derived-result registry identity, and explicit semantic-migration transaction;
7. persistent dimensions, real tags, collections, simple patterning, simple cut/union, exact/mesh export, and AI Proposal UI remain incomplete or absent;
8. collision validation, declared joints, rule-derived stable slots, manufacturing output, and the beam end-to-end case are planned only;
9. historical gates do not automatically certify the current dirty working tree; current A0 fails closed on a frozen build-input hash mismatch and Gate C lacks terminal integrated evidence;
10. CI does not yet mechanically enforce the sole-mutation, threshold-governance, formatting, lint, test, build, and gate invariants.

## 1.5 Target-state statement

The target architecture has one revisioned canonical document and one validated mutation gateway. The document stores semantic inputs, rules, feature specifications, stable identity, explicit exact/mesh body specifications, reference state, declared joints, validation policy, and provenance. Evaluation produces replaceable derived results through a supervised exact worker and other deterministic services. Interaction, rendering, full state dumps, agent views, BOMs, drawings, and manufacturing exports are projections from the same snapshot; none is a second authority.

A user gesture or accepted AI Proposal commits exactly one validated `CanonicalCommandBatch`. Open audits and reports without recomputing, migrating, or rewriting canonical state. Evaluation or recomputation that preserves canonical semantics may register only a fully verified, revision/digest/envelope-bound derived-result event under §9.6; it creates no canonical revision or Undo step. Any migration or recomputation that changes canonical meaning produces a separate authoritative canonical diff and requires explicit confirmation and exactly one command-batch revision.

## 1.6 Can Ketchup model a ketchup bottle?

**Today: no, not faithfully.** The desktop path can create and manipulate extruded box-like solids. Its render and picking representations reduce definitions to cuboid bounds.

**Target: yes, through a workflow-proven editable modeling slice.** The acceptance case is not merely that a backend can construct a bottle. A user must be able to start from a primitive or profile, stretch or scale selected regions, flatten them, apply bevel/fillet and shell/thickness, and keep every step editable, Undoable, persistable, and explicit about exact/mesh authority and reference loss. A rotational bottle may use **Revolve**, **Shell**, and **Fillet/Chamfer**; a squeezed asymmetric bottle may require controlled spline, **Loft**, or **Sweep** operations. These are candidate means selected by the workflow, not a mandatory quest to complete every CAD feature family.

A second bounded organic reference workflow is one editable living tree: trunk and branch structure has semantic paths and variable-radius sweep/loft or explicitly mesh-authoritative procedural geometry. This is a non-blocking research/acceptance slice after the fabrication path, not a release gate or prerequisite for the bottle, beam, exact integration, or FLP. Ketchup’s target is not “every mathematically imaginable shape,” unrestricted sculpting, animation, or production-scale vegetation generation. It is selected exact and mesh workflows that make furniture, buildings, bottles, trees, and similarly understandable shapes quick to create and revise, with honest fallback and loss reporting.

---

# 2. Product scope and capability envelope

## 2.1 First Lovable Product retained from V3

The V3 FLP remains the binding near-term product envelope. A user must be able to:

1. create a document with explicit units;
2. draw and dimension a simple profile;
3. extrude it into an exact solid;
4. change dimensions without cumulative drift;
5. create a simple opening, cut, or union;
6. use Smart Push/Pull with an explanation of the operation;
7. organize definitions, occurrences, groups, tags, and collections;
8. move, copy, snap, align, and create simple patterns precisely;
9. save, open, Undo/Redo, and survive exact-worker failure without losing the last commit;
10. export at least one exact format and one mesh format, subject to gate evidence;
11. execute selected tasks through a Proposal workflow with assumptions and an authoritative diff before commit.

The current narrow manual-modeler milestone is a subset of this FLP, not a replacement for it.

## 2.2 Domain emphasis added after V3

The first domain remains architecture, interiors, and furniture, but V4 makes fabrication-oriented rule modeling explicit. The reference use case is a timber building containing roughly 1,400 fabricated members whose lengths, spacing, notches, dimensions, and joints derive from a smaller set of rules.

The desired operation is not merely “move this wall.” It is:

> Change a governing parameter, deterministically recompute only the dependent branches, preserve or explicitly invalidate stable overrides, validate the result, and regenerate the BOM and manufacturing outputs.

Therefore validators, stable derivation identity, dimensions, BOMs, and bounded joint semantics are product features rather than optional internal infrastructure. M4a MUST freeze a host-neutral validator interface, diagnostic schema, policy model, and result classes so built-in checks—and later open-source or proprietary/paid implementations—cannot create a second document authority. Loading, distributing, licensing, signing, revoking, isolating, or remotely invoking third-party validator code is extension hosting and is deferred to M7.

## 2.3 Geometry roadmap

| Geometry family | Current status | Target role |
|---|---|---|
| Rectangle profile | **PARTIAL—PRODUCT PATH** | Core stores an ordered finite point loop and checks only point count/nonzero signed area; simple-polygon, closure, duplicate, self-intersection, hole, winding, tolerance, and coordinate-envelope rules are not complete. UI creates rectangles; target adds validated closed polylines and constraints. |
| Linear extrusion | **PARTIAL—PRODUCT PATH** | Canonical parameter exists and the app presents cuboid semantics, but exact product evaluation is absent; target evaluates exact B-Rep and preserves source provenance. |
| Cut/union/opening | **PLANNED on product path** | Narrow exact box cut exists only in the exact proof; FLP requires user-visible canonical features. |
| Revolve | **PLANNED** | Rotational objects such as bottles, legs, knobs, and turned profiles. |
| Sweep | **PLANNED** | Profiles along paths, trims, pipes, rails, and selected fabrication operations. |
| Loft | **PLANNED** | Transitions and asymmetric products, including non-rotational bottle bodies. |
| Shell/thickness | **PLANNED** | Hollow containers and wall-like bodies with explicit failure diagnostics. |
| Fillet/chamfer | **PLANNED** | Edge finishing with stable-reference limits stated per feature contract. |
| Spline/NURBS sketch and surface inputs | **PLANNED** | Required for controlled freeform exact geometry; not an organic sculpting system. |
| Mesh-authoritative bodies | **PLANNED** | Imports and procedural polygonal workflows where mesh is explicitly authoritative. |
| Exact-to-mesh / mesh-to-exact conversion | **PLANNED** | Named operations with provenance and loss reports; never an invisible fallback. |
| 2D associative drawings | **PLANNED** | Deterministic views and dimensions from the canonical model; full professional drawing production remains later. |
| BIM semantics and IFC | **PLANNED after FLP** | Domain packages over the same core, not a parallel document architecture. |

Operation-family priority is determined by frozen user-workflow evidence, not by completing this table. The bottle workflow in §1.6 is a blocking reference test for its selected M6 slice; the living-tree workflow is an explicitly non-blocking research/acceptance slice. In either case, each implemented operation must remain directly editable, numerically controllable where meaningful, Undoable, persistable, and explicit about exact versus mesh authority. A bounded individual-tree workflow does not imply a vegetation/world generator.

## 2.4 Explicit non-goals for the FLP

The FLP is not:

- a full BIM authoring and IFC round-trip system;
- a professional multi-sheet drafting suite;
- mechanical assembly/simulation CAD;
- an organic sculpting or animation package;
- a production-scale vegetation/world generator;
- a public plugin marketplace;
- browser CAD or simultaneous multi-user editing;
- a promise of bit-identical B-Rep across all backend and platform versions;
- a promise that every imported or generated topology can retain a stable semantic reference.

## 2.5 Product invariants

- Manual modeling MUST remain useful without AI.
- AI MUST operate on semantic intents and validated proposals, never raw document memory.
- Canonical values MUST retain units and source meaning; measured/rendered approximations MUST NOT overwrite them.
- Definitions own reusable local geometry; occurrences own placement and occurrence metadata.
- Copy SHOULD create another occurrence when sharing is intended; Make Unique MUST clone and repoint only the selected occurrence.
- Preview MUST remain ephemeral until confirmation.
- Any operation that cannot preserve identity MUST return `Ambiguous`, `Lost`, or an explicit loss report rather than silently choosing a replacement.
- A user-visible feature is not complete until it is discoverable from the designed shell, accepts exact input where relevant, creates one batch, supports Undo/Redo, and has executable workflow evidence.

---

# 3. Decision register

## 3.1 Ratified V3 decisions

The following remain binding and retain their V3 IDs:

| ID | Status | Decision |
|---|---|---|
| D-01 | **RATIFIED** | First segment: architecture, interiors, and furniture. |
| D-02 | **RATIFIED** | Long-term core remains domain-neutral. |
| D-03 | **RATIFIED** | Rust owns the canonical core and primary application services. |
| D-04 | **RATIFIED** | OCCT is the initial exact B-Rep backend. |
| D-05 | **RATIFIED** | OCCT stays behind a narrow owned, versioned C++ façade. |
| D-06 | **RATIFIED** | `wgpu` is the renderer baseline. |
| D-07 | **RATIFIED** | The document is revisioned, one-writer/many-readers. |
| D-08 | **RATIFIED** | Only a validated canonical command batch mutates canonical model state. |
| D-09 | **RATIFIED** | Dragging and preview are ephemeral, not document transactions. |
| D-10 | **RATIFIED** | The canonical document, not command history, is the source of truth. |
| D-11 | **RATIFIED** | Exact and mesh bodies are distinct explicit canonical types. |
| D-12 | **RATIFIED** | Stable topology has a narrow guaranteed subset and explicit failure. |
| D-13 | **RATIFIED** | AI uses Intent → Proposal → CommandBatch → Verification. |
| D-14 | **RATIFIED** | One committed user batch is one visible Undo step. |
| D-15 | **RATIFIED** | Deferred domains do not enter the PoC/FLP by stealth. |

## 3.2 Accepted ADRs

| ADR | Status | Consequence in V4 |
|---|---|---|
| ADR 0001 | **RATIFIED** | English project artifacts; localized UI resources; locale-independent canonical state. |
| ADR 0002 | **RATIFIED, implementation PARTIAL—PROOF ONLY** | Persistent exact worker is the production default. Parent owns canonical state; stale results require revision, generation, and digest match. Product integration is still missing. |
| ADR 0003 | **RATIFIED** | Splash/version is presentation-only and dynamically sourced; it has no canonical effect. |

## 3.3 Proposed post-V3 decisions

These decisions are recommendations for review, not retroactively accepted facts:

| ID | Status | Proposed decision |
|---|---|---|
| V4-P01 | **PROPOSED** | Unify the legacy `CanonicalNode` graph and product `Feature` graph into one canonical evaluator graph; no parallel rule/document truth. |
| V4-P02 | **PROPOSED** | Exact or mesh body specifications are canonical; interaction and render scenes are disposable snapshot-bound projections. |
| V4-P03 | **PROPOSED** | Freeze C1a now: every interaction occurrence originates from canonical projection and cannot become an independent model authority. |
| V4-P04 | **PROPOSED** | Introduce C1b only after exact bodies enter the app: exact topology resolution and interaction selection must produce equivalent stable references over a preregistered corpus. |
| V4-P05 | **PROPOSED, REVISED AFTER REVIEW** | Rules live in the canonical graph. Nested outputs carry stable provenance `(RootRuleNodeId, SlotPath)`, where every semantic path segment is minted by the producing rule level; resolution is segment-wise and never silently reindexes or retargets an override. |
| V4-P06 | **PROPOSED** | Every evaluable node has an input/Merkle digest including evaluator identity, backend identity where relevant, schema, tolerance profile, and dependent result fingerprints. |
| V4-P07 | **PROPOSED, REVISED AFTER REVIEW** | Open audits without recomputing or rewriting canonical state. Semantics-preserving evaluation registers a non-canonical derived-result event; only a migration/recompute that changes canonical meaning requires one explicit confirmed command-batch revision. |
| V4-P08 | **PROPOSED, REVISED AFTER REVIEW** | Canonical semantic specifications remain the sole model authority. Derived results may be persisted only as revision/digest/envelope-bound evidence or cache; they are excluded from the canonical model digest and never substitute for missing canonical meaning. |
| V4-P09 | **PROPOSED** | `StateView` has one shared deterministic encoder and two separately versioned projections: complete canonical dump and summarized agent view. |
| V4-P10 | **PROPOSED, REVISED AFTER REVIEW** | Collision validation uses broad-phase AABB, optional OBB/convex filtering, deterministic `f64` SAT over canonical convex coverage, and explicit convex-intersection containment for declared joints before curved-body conservative envelopes. SAT alone cannot prove that overlap lies inside an allowed joint volume. |
| V4-P11 | **PROPOSED** | A declared joint is a canonical entity with its own bounded allowed-overlap volume. Undeclared overlap, overlap outside that volume, and an empty declared joint are errors. |
| V4-P12 | **PROPOSED** | CI mechanically protects code quality, sole mutation authority, legacy-authority absence, gate suites, and R0 threshold direction (`tighten/loosen/neutral/unknown`). |
| V4-P13 | **PROPOSED** | The native schema advances only with explicit migration and resource-limit policy; a current file always declares document, evaluator, backend, and determinism envelopes. |
| V4-P14 | **PROPOSED, REVISED AFTER REVIEW** | Validators are first-class deterministic read-only services behind an open domain-neutral interface, diagnostic schema, policy model, and result taxonomy. M4a freezes this internal/host-neutral contract; third-party distribution, signatures, revocation, licensing, native isolation, and remote egress belong to M7 hosting. Structural/statics results are permitted best-effort decision support, never a Ketchup safety guarantee, regulatory certification, or substitute for approval by a qualified structural engineer. |
| V4-P15 | **PROPOSED; DEDICATED ADR REQUIRED; REVISED AFTER REVIEW** | Replace the old monolithic Gate C sequence with C1a before additional proxy-modeler stabilization, execute the early beam checkpoint M4a-E immediately after M2 and before OCCT product integration, complete the remaining M4a protocol/projection/evidence track without delaying that first run, then execute C1b after exact product integration. This is a sequence change, not a clarification. |

## 3.4 Open decisions

| ID | Status | Owner / decision point |
|---|---|---|
| V4-O01 | **OPEN** | Exact shape/blob persistence strategy versus deterministic recomputation for each body family; decide before schema 3 is frozen. |
| V4-O02 | **OPEN** | Exact expression language and solver; decide after license/security review and before rule-node implementation. |
| V4-O03 | **OPEN** | General sketch constraint solver; post-FLP spike unless required by a frozen workflow. |
| V4-O04 | **OPEN** | Supported exact export and mesh export formats for FLP; decide before export slice. |
| V4-O05 | **OPEN** | BTLx scope and manufacturing tolerance contract; decide after beam workflows and BOM/dimension projections pass. |
| V4-O06 | **OPEN** | Public file/API compatibility promise; prohibited before migration and backend-change suites pass. |
| V4-O07 | **OPEN** | Whether `egui` remains the long-term UI framework; decide from product UX/performance evidence, not prototype familiarity. |
| V4-O08 | **OPEN** | Windows-only first release versus parallel desktop support; owner decision before packaging commitments. |
| V4-O09 | **OPEN** | Cloud AI providers and default privacy mode; local-first remains binding until explicit opt-in design is approved. |
| V4-O10 | **OPEN** | Team, budget, release quality bar, and named human owners for safety-critical decisions. |

## 3.5 Decision governance

All `V4-P*` requirements are **proposed MUSTs** until accepted by the accountable project owner. Ratification requires exactly three decision records: (1) one V4 adoption ADR accepting P01–P14 as a coherent baseline; (2) one dedicated P07/P08 ADR governing observational Open, derived-result registration, and canonical-digest authority; and (3) one dedicated P15 ADR governing every deviation from the frozen V3 gate/milestone sequence, including C1a/C1b and M4a-E-before-M3. The adoption ADR incorporates the dedicated P07/P08 ADR by reference. Separate ADRs for the remaining individual P decisions are not required unless a later change alters their accepted architectural commitment.

| Decision set | Responsible owner role | Accountable approver | Required evidence | Deadline / stage | ADR required |
|---|---|---|---|---|---|
| P01, P05, P06 | Core/evaluator lead | Project owner | unified graph prototype, migration and dirty-evaluation tests | before M2 implementation | V4 adoption ADR |
| P02, P03 | Core + interaction lead | Project owner | C1a failing/passing authority tests | before further interaction expansion | V4 adoption ADR |
| P04 | Exact + interaction lead | Project owner | preregistered C1b corpus and integrated exact body path | before C1b observation | V4 adoption ADR |
| P07, P08 | Core/IO lead | Project owner | two-phase Open/recompute design, derived-result trust/retention policy, and schema fixtures | before schema 3 freeze | Dedicated P07/P08 ADR |
| P09 | Core/protocol lead | Project owner | complete/agent StateView golden fixtures | in M0 | V4 adoption ADR |
| P10, P11, P14 | Validation/domain lead | Project owner | collision/intersection/joint corpus, host-neutral validator contract, best-effort structural-result boundary, and beam 6.3a | M4a-E evidence before M3; remaining contract evidence before full M4a exit | V4 adoption ADR |
| P12 | Build/test lead | Project owner | deliberate-red CI guards | in M0 | V4 adoption ADR |
| P13 | Core/IO lead | Project owner | schema, migration, limits, and compatibility fixtures | before schema 3 freeze | V4 adoption ADR |
| P15 | Architecture lead | Project owner | explicit old-C disposition and replacement gate charter, including M4a-E-before-M3 and full-M4a concurrency/exit rules | before relying on reordered sequence, no later than M0 exit | Dedicated P15 ADR |
| O01, O06 | Core/IO lead | Project owner | storage/compatibility matrix and two-build migration suite | before public compatibility promise | Yes |
| O02, O03 | Evaluator/sketch lead + license reviewer | Project owner | license/security shortlist and focused spikes | before corresponding implementation | Yes |
| O04, O05 | IO/domain lead | Project owner | user workflow, format/license/tolerance evidence | before export scope freeze | Yes |
| O07 | UI lead | Project owner | measured UX/performance/accessibility evidence | before long-term UI commitment | Yes |
| O08–O10 | Project owner | Project owner | product/privacy/resourcing decision record | before release commitments | Yes where architecture/privacy changes |

Named people and calendar dates remain unavailable. That is itself an unresolved governance gap; role names do not satisfy a release-readiness owner requirement.

---

# 4. System-wide status matrix

| Subsystem / invariant | Status | Current boundary | Target |
|---|---|---|---|
| Immutable revisions and snapshot reads | **IMPLEMENTED** | Core snapshot history; product and legacy maps copied through `Arc`. | Structural sharing, explicit retention/checkpoints, evaluator generation invalidation. |
| Sole canonical edit path | **PARTIAL—PRODUCT PATH** | `DocumentStore::apply_batch` is the ordinary product mutation path; lifecycle methods Undo/Redo/load are separate and the invariant lacks CI enforcement. | Explicit gateway, typed capabilities, budgets, dry-run, diff, audit, and documented lifecycle exceptions. |
| Atomic multi-command commit | **IMPLEMENTED** | Candidate maps are validated before revision append. | Preserve while adding geometry/domain validation and resource budgets. |
| One batch = one Undo step | **IMPLEMENTED for canonical state** | Snapshot cursor navigation; history is not persisted. | Invalidate evaluator jobs/results and preserve one semantic operation across all clients. |
| Product identity/hierarchy | **PARTIAL—PRODUCT PATH** | Typed IDs, definitions, occurrences, groups, features; constant default DocumentId and app-side ID allocation. | Durable document IDs, non-reuse/remap policy, tags/collections/views, hierarchical stable derived `SlotPath` identity. |
| Parametric DAG | **PARTIAL—PRODUCT PATH** | Dependency closure over legacy nodes; no expression evaluation; product features are separate. | One typed feature/rule DAG with ports, expressions, dirty/evaluated state, diagnostics, and outputs. |
| Definitions/components/Make Unique | **PARTIAL—PRODUCT PATH** | Sharing and clone/repoint work; “Make Component” mostly renames an existing definition. | Explicit group-to-component conversion, nested-context semantics, complete tests. |
| Edit context | **PARTIAL—PRODUCT PATH** | Ephemeral app stack and filtering; not a core query contract. | Snapshot-bound context-safe selection/query semantics without making UI state canonical. |
| Persistence | **PARTIAL—PRODUCT PATH** | Atomic binary schema 0/1/2; deterministic round trips for implemented fields. | Container, manifest, checksums, limits, schema 3+, provenance, digests, references, migration transaction. |
| Exact geometry | **PARTIAL—PROOF ONLY** | Isolated box/extrude/cut/import façade and tests; promotion requires the M3 product slice. | Canonical exact feature evaluation through supervised worker, broad operation vocabulary. |
| Mesh geometry | **PLANNED** | No canonical product `MeshBody`. | Explicit mesh-authoritative specs, validation, rendering, conversion/loss contract. |
| Stable subshape references | **PARTIAL—PROOF ONLY** | Narrow exact-crate resolver only; not canonical/persisted/app-visible. | Typed core reference, complete evidence fields, audit/quarantine/migration, C1b. |
| Scheduler/worker | **PARTIAL—PROOF ONLY** | Stale rejection, cache, proof client/worker; no app integration, deadline, restart supervisor. | Production supervision, timeout, restart/reschedule, backend identity, progress, cancellation. |
| Interaction scene | **PARTIAL—PRODUCT PATH** | Derived cuboid proxies and box IDs; app rebuilds scene per pick. | Snapshot/revision-bound exact/mesh query service producing stable references. |
| Renderer | **PARTIAL—PRODUCT PATH** | `egui` cuboid painting on a wgpu-backed app. | Derived mesh cache, instancing, BVH, highlights, stale markers, general exact/mesh bodies. |
| Manual shell | **PARTIAL—PRODUCT PATH** | Major shell, tools, menus, Outliner, hints, value box, file workflows. | Finish line/profile, tags, dimensions, shortcuts, menu contract, visual/accessibility evidence. |
| Collision validation | **PLANNED** | No canonical collision service. | Versioned AABB/OBB/SAT, convex-intersection containment, conservative-envelope validator, and FurniGen corpus. |
| Validator protocol and hosting | **PLANNED** | No protocol, policy registry, result taxonomy, or package host. | M4a: host-neutral read-only interface, diagnostics, policy, and `NotEvaluated`/`Unavailable` semantics for built-in validators. M7: optional third-party host with provenance, privacy, budgets, trust, licensing, isolation, egress, and revocation. |
| Rules and manufacturing projections | **PLANNED** | Dependency skeleton only. | Stable derived pieces/joints/dimensions, BOM, piece drawings, later BTLx. |
| Proposal safety | **PARTIAL—PROOF ONLY** | Narrow proposal/digest primitive, mostly legacy-node scope; promotion requires an intended product client through the authoritative gateway. | Authoritative typed read/write sets, assumptions, risks, budgets, dry-run, diff, verification. |
| AI assistant | **PLANNED** | No product Assistant surface. | Optional local-first intent/proposal UI after manual modeler and validators. |
| Plugin system | **PLANNED** | No host/capability sandbox. | Capability- and budget-limited host after core protocol stabilizes. |
| CI governance | **PLANNED** | Tests run manually; invariants depend on agent discipline. | Required automated checks, gate provenance, mutation guard, threshold-direction guard. |
| Gate certification of current tree | **PARTIAL—PROOF ONLY** | Historical GO artifacts exist; current A0 hash mismatch and incomplete Gate C do not certify the product tree. | New freeze over current inputs and honest reruns on required hardware. |

# 5. As-built architecture

## 5.1 Repository and dependency reality

The intended architecture diagram remains valid as a target, but current crate connectivity is narrower:

```text
ketchup-app ───────> ketchup-core
     │
     └─────────────> ketchup-interaction

ketchup-scheduler ─> ketchup-core
       │             ketchup-exact
       └────────────> ketchup-interaction

ketchup-exact ─────> CXX façade ─> OCCT shared libraries
```

`ketchup-app` does not currently depend on `ketchup-exact` or `ketchup-scheduler` (`crates/ketchup-app/Cargo.toml:11-16`). Consequently, no product action schedules an OCCT feature evaluation. The scheduler crate is the only Rust integration point that already references core, exact, and interaction (`crates/ketchup-scheduler/Cargo.toml:11-14`), but its proof runner is not the desktop application.

## 5.2 Canonical core — PARTIAL—PRODUCT PATH

### Data model

`ProductModel` owns:

- `DocumentId` and units;
- ordered maps of definitions, features, occurrences, and groups;
- definition-local features;
- occurrences pointing to definitions with transform, parent, tag reference, and visibility;
- groups with transforms and optional parents.

Relevant implementation: `crates/ketchup-core/src/document.rs:93-255`.

The feature vocabulary contains only:

```text
Profile { ordered planar points }
Extrusion { profile_id, height }
```

A second, older canonical structure remains: `CanonicalNode { id, name, Dimension, dependencies }` (`document.rs:311-365`). It was created for the A1 dependency experiment. Both maps are stored in the same snapshot (`document.rs:522-527`), but they are not one graph:

- product-feature commands do not populate the legacy affected-node closure;
- legacy dependencies do not express feature input/output ports;
- a `Dimension.source_token` may contain text such as `width / 2`, but no evaluator executes it;
- product entities do not carry `derived_from` provenance.

The property currently named `recomputed_nodes` is therefore an affected dependency-ID closure, not proof that values were recomputed.

### Identity

Distinct typed numeric IDs exist for document, definition, occurrence, group, feature, tag, and legacy node (`document.rs:10-25`). Core validation rejects zero IDs, missing ownership, duplicates, invalid feature references, invalid group parents, and cycles (`document.rs:1403-1617`).

Limits:

- every empty model defaults to `DocumentId(1)`;
- clients allocate most IDs, and the app uses `max + 1` (`crates/ketchup-app/src/lib.rs:1459-1466`, `1596-1615`);
- non-reuse, import remapping, cross-document scope, overflow, and generation policy are unspecified;
- `TagId` exists as a field but there is no canonical tag table or tag command.

### Command and revision path

`CommandBatch` has schema `ketchup.command.v1`, ordered commands, and a stable command digest (`document.rs:485-519`). `DocumentStore::apply_batch`:

1. rejects unsupported or empty batches;
2. clones the current candidate maps;
3. applies every command to the private candidate;
4. validates complete legacy and product structures;
5. appends exactly one revision only after all checks pass.

Implementation: `document.rs:784-1183`.

Entity fields are private, snapshots expose read-only accessors, and persistence constructs a validated baseline through crate-private `from_parts` (`document.rs:529-755`). `commit_proposal` delegates to `apply_batch` (`document.rs:1233-1243`). This is a strong narrow implementation of D-08.

The documented exceptions are lifecycle operations, not ordinary model edits:

- Undo/Redo move the active revision cursor;
- loading constructs and swaps an already validated store;
- history-discard operations alter retention rather than entity content.

V4 requires these exceptions to be named and CI-enforced so that “only `apply_batch` mutates” does not become an inaccurate slogan.

### Undo/Redo

One successful batch appends one snapshot revision. Failed batches append none. Undo and Redo move the cursor between immutable snapshots; committing after Undo truncates the redo tail (`document.rs:1179-1200`). Product tests prove digest restoration, sharing, and hierarchy behavior (`crates/ketchup-core/tests/product_document.rs:73-137`).

Missing target behavior is evaluator-aware cancellation/invalidation. No integrated geometry job is attached to core history today.

### Definitions, occurrences, groups, and Make Unique

Definitions own ordered feature IDs; many occurrences can reference one definition. World transforms derive from group ancestry (`document.rs:600-657`). Groups and occurrences can be created, deleted, transformed, and reparented through commands, with cycle and non-empty-delete checks.

`CloneDefinitionAndRepoint` validates a complete feature-ID mapping, clones profile/extrusion features, remaps internal profile references, creates a new definition, and repoints only one occurrence (`document.rs:1451-1541`). That provides real atomic Make Unique semantics for the current feature vocabulary.

The app’s “Make Component” currently renames an already reusable definition rather than converting an arbitrary group/hierarchy to a component (`crates/ketchup-app/src/lib.rs:1541-1579`). Group-to-component conversion and general feature cloning remain incomplete.

## 5.3 Persistence — PARTIAL—PRODUCT PATH

The native proof format is a custom binary stream with magic and schemas 0, 1, and 2 (`crates/ketchup-core/src/persistence.rs:15-18`). Schema 2 serializes the legacy node graph followed by the current product model: units, IDs, definitions, features, occurrences, groups, transforms, visibility, and source tokens (`persistence.rs:33-119`).

`save_atomic` writes to a sibling temporary path, flushes/synchronizes, then persists the replacement (`persistence.rs:122-138`). Load builds and validates a complete replacement before returning it and rejects invalid magic/schema, malformed values, duplicates, invalid graph/product state, and trailing bytes (`persistence.rs:144-203`, `327-381`). The app swaps documents only on successful load and preserves the active model on failure (`crates/ketchup-app/src/lib.rs:630-741`).

The canonical digest uses deterministic ordered traversal and exact numeric bits (`document.rs:1729-1847`). It is a 64-bit FNV-style equality/dirty-state digest, not a cryptographic file checksum.

Current strengths:

- canonical numeric values and source tokens round-trip;
- current IDs and hierarchy round-trip;
- Save is atomic;
- failed Open does not replace the active document;
- schema 0 migration reports reconstructed decimal-token loss.

Current gaps:

- no V3 container/manifest;
- no cryptographic checksum or content-addressed blobs;
- no entity/file/depth/resource limits;
- no unknown-extension preservation;
- no evaluator/backend/tolerance envelope;
- no per-node input/result digest;
- no persisted exact/mesh body spec or stable reference;
- no reference audit or compatibility quarantine;
- no transactional backup/migration UI;
- history is intentionally discarded on load;
- empty product models may be written using the older schema selection rule.

## 5.4 Application and interaction — PARTIAL—PRODUCT PATH

### Shell and workflows

The desktop application now contains the major designed shell: top/menu/status bars, tool rail, viewport, Outliner/Tags dock, value box, localized hints/action digest, command registry, and file-dialog seam (`crates/ketchup-app/src/lib.rs:1205-1426`, `2578-2697`, `3247-3641`; `src/dialogs.rs:29-160`).

The following current narrow workflows are real and tested:

- object select, multiselect, clear, and shared viewport/Outliner selection;
- rectangle creation with exact values;
- Push/Pull-like profile/height edits with ephemeral preview;
- Move and Ctrl-Copy as occurrence operations;
- Group/Ungroup for supported parent/transform cases;
- shared definition edits and Make Unique;
- component/group edit-context entry, isolation, and Escape exit;
- Measure as a non-persistent readout;
- per-occurrence visibility;
- Zoom Fit and common navigation;
- New/Open/Save/Save As with dirty-state handling;
- canonical Undo/Redo.

The continuous capstone path is exercised in `crates/ketchup-app/tests/capstone_chain.rs:21-202`. The `egui_kittest` harness runs the actual widget tree through AccessKit identities and pointer/keyboard input (`tests/harness/mod.rs:29-278`). It is a genuine headless UI harness, not a physical-mouse recorder.

### Interaction and rendering representation

The interaction crate owns typed `SelectionId`, `ElementId`, exact analytic box hits, snap candidates, and `SharedBoxGeometry` (`crates/ketchup-interaction/src/lib.rs:121-363`). `SharedBoxGeometry` is an axis-aligned cuboid with eight endpoints and twelve edge midpoints. It cannot represent curved faces, profile holes, arbitrary topology, or a cut body.

The app queries the canonical model, computes profile XY bounds plus extrusion height, and creates cuboid render proxies (`crates/ketchup-app/src/lib.rs:3705-3775`). It rebuilds an interaction scene for picking (`lib.rs:3073-3091`) and paints six projected cuboid faces (`lib.rs:2921-3019`, `3885-3936`). The window uses a wgpu-backed `eframe` configuration, but the model renderer is not yet a general wgpu CAD renderer.

This is permissible only as an explicitly derived narrow proxy. It is not an alternative geometric authority.

### Edit context

Selection and the edit-context stack are ephemeral app state (`lib.rs:383-443`). Context filtering and entry/exit behavior exist (`lib.rs:906-993`) and block out-of-context picking. That is appropriate presentation/session ownership, but core lacks a snapshot-bound context-aware query contract for nested definitions, stable sub-elements, and command scope.

### Known shell gaps

- Line/polyline is disabled;
- Rectangle currently creates a default-height solid rather than a zero-height profile awaiting extrusion;
- Measure does not create a persistent dimension;
- tags are not canonical entities and tag-level visibility is absent;
- component conversion is incomplete;
- simple pattern, cut/union, export, Assistant, and plugin surfaces are absent;
- several menu commands and specified shortcuts are disabled or missing;
- status fields such as snap/grid/reference health are partly fixed labels rather than live services;
- the camera/projection and viewport decorations do not fully match the design specification;
- snap primitives exist in interaction tests, but the app mainly consumes the primary hit and does not complete inference visuals.

## 5.5 Exact backend — PARTIAL—PROOF ONLY

`ketchup-exact` links a pinned OCCT 8.0.1 shared build through CXX and a narrow native façade (`crates/ketchup-exact/build.rs:21-74`). The native boundary catches exceptions and validates successful shapes using B-Rep validity, solid count, finite bounds, and positive volume (`src/native.cc:209-279`).

Implemented proof operations include:

- box and rectangular extrusion;
- box cut;
- selected STEP import;
- structured errors;
- validity evidence, bounds, volume, topology counts, and result fingerprints;
- a narrow capture/resolve proof for extrusion top, bottom, and one side.

Rust types for `SubshapeRef` and `Resolved/Ambiguous/Lost/QuarantinedMigration` exist in `crates/ketchup-exact/src/lib.rs:220-260`. Capture and resolution are narrow and partly reconstruct semantic faces by extrema/history (`lib.rs:561-665`; `native.cc:405-441`). They do not yet validate every field envisioned by V3 and are not stored in core or persistence.

The backend fingerprint is a hard-coded narrow identifier (`lib.rs:6-10`) rather than the complete normalized build/install-tree identity already recorded in R0 artifacts.

## 5.6 Scheduler and worker — PARTIAL—PROOF ONLY

`ketchup-scheduler` implements:

- job keys containing node, revision, generation, and input digest;
- result acceptance only when revision, generation, and digest still match;
- a bounded 64 MiB LRU accounting proof;
- a persistent line-protocol worker client;
- explicit process cancellation and crash detection.

See `crates/ketchup-scheduler/src/lib.rs:41-199`, `222-360`.

The worker protocol supports only proof operations such as ping, fixed rectangular extrusion, exception probe, sleep, and deliberate abort (`src/bin/ketchup-exact-worker.rs:27-77`). Current production gaps include request deadlines, heartbeat, automatic restart, retry/reschedule policy, progress, complete backend identity, arbitrary feature requests, and application wiring.

ADR 0002’s parent-owned state and process-isolation decision is binding. Its restart/reschedule contract remains a target behavior rather than a complete current implementation.

## 5.7 Tests and evidence — CURRENT SNAPSHOT

Focused current runs performed during the V4 audit reported:

- `cargo test -p ketchup-core`: 13 tests passed;
- `cargo test -p ketchup-app -p ketchup-interaction`: 63 tests passed;
- `cargo test -p ketchup-exact --lib`: 6 tests passed;
- `cargo test -p ketchup-core --test gate_a1`: 7 tests passed;
- `cargo test -p ketchup-interaction --test gate_c_interaction`: 9 tests passed;
- `cargo test -p ketchup-scheduler --test gate_b --no-run`: compiled;
- current formal A0 test: failed closed before geometry observation because the r0-v13 frozen build-input hash does not match the current working tree.

These counts are observations, not eternal requirements. CI SHOULD target named invariants and suites rather than hard-code a total test count.

## 5.8 Evidence caveats

- Historical A0 GO proves its frozen operation/reference subset for its frozen inputs; it does not prove current source or broader topology.
- Historical A1 proves the research-node schema and then-focused protocol/round-trip invariants; it does not prove an integrated product evaluator, exact bodies, or schema 3.
- Historical Gate B supports the worker decision, but some reader/memory scenarios are weaker than their full preregistered product interpretation.
- Gate C interaction tests prove analytic cuboid services. The navigation executable is a synthetic harness, not the complete Ketchup application and not an OCCT-backed model scene.
- The required integrated-GPU terminal Gate C evidence is incomplete.
- The repository is currently heavily modified/untracked relative to the last commit; this V4 describes the working tree, not only `HEAD`.


# 6. Target architecture and ownership boundaries

## 6.1 Logical architecture

```text
Manual low-level UI / CLI adapters ───────────────────────┐
                                                         │
High-level tools / plugin / AI                            │
             │                                           │
       Intent adapters                                    │
             │                                           │
       Proposal service                                   │
 assumptions • risk • intended result • provenance        │
             └───────────────────────────────────────────┤
                                                         │
              Canonical command gateway <────────────────┘
 schema • capability • budgets • preconditions • dry-run • diff
                         │
           Revisioned canonical document
 entities • rules • features • body specs • refs • joints • policy
                         │
                  Change analysis
       typed read/write set • affected DAG • generations
                         │
                Evaluation scheduler
       deadlines • cancellation • stale rejection • cache budget
              ┌──────────┼──────────┐
              │          │          │
       Exact worker   Sketch/rule   Mesh/procedural
          (OCCT)       evaluator       services
              └──────────┼──────────┘
                         │
              Derived result registry
 exact results • mesh results • diagnostics • fingerprints
                         │
           Spatial query / interaction service
 picks • snaps • inference • stable-reference resolution • collision
                         │
       Derived render scene / BVH / GPU cache / projections
                         │
                    wgpu renderer
```

## 6.2 Dependency rules

- Core MUST NOT depend on OCCT, UI widgets, localization prose, GPU types, natural language, or cloud providers.
- A geometry backend MUST NOT own canonical document state, user permissions, or command authorization.
- The scheduler MUST know job identity and resource policy but MUST NOT reinterpret product semantics.
- Interaction MUST consume canonical identity plus accepted derived geometry; it MUST NOT invent persistent identity.
- Renderer output MUST NOT decide exact CAD identity.
- UI presentation state MAY contain selection, camera, open panels, pending gestures, and edit-context navigation, but MUST NOT contain authoritative geometry, hierarchy, transforms, or model Undo history.
- Importers, plugins, scripts, and AI MUST be treated as untrusted clients even when running locally.
- Domain validators MUST consume the common document through the open host-neutral protocol and MUST NOT establish another model database. M4a defines that semantic boundary only; loading open-source or proprietary/paid third-party implementations is an M7 host responsibility.

## 6.3 One authority, multiple projections

The phrase “one source of truth” means one semantic authority, not one in-memory data structure.

### Canonical

Canonical state answers questions whose answers must survive Save/Open and remain valid without caches:

- what entities, rules, features, parameters, units, relationships, and explicit overrides exist;
- what exact or mesh body specification is intended;
- what stable reference a downstream feature means;
- what joint and allowed-overlap volume was declared;
- which evaluator/backend/tolerance envelopes are permitted by document policy;
- which validation policies govern commit/export.

### Derived result registry

Costly outputs MAY be retained or persisted for fast Open, compatibility review, or manufacturing evidence only when keyed by document revision, root producer node and complete `SlotPath`, complete input digest, evaluator/backend identity, tolerance/schema envelope, and result digest. The registry is **not canonical model state**, is excluded from the canonical model digest, and is never directly editable. Registering a result for an unchanged canonical revision is a derived-result event, not a model revision and not an Undo step.

A retained result is current only after its complete key and validity policy are verified. Otherwise it is `Stale`, `Unverified`, or `Unavailable`. If the producing backend is unavailable, the semantic document still opens; retained geometry may be displayed as visibly unverified evidence but MUST NOT support exact edit, reference migration, manufacturing export, or validation claims. Absence of retained material causes unavailable derived geometry, not loss of canonical meaning.

### Disposable

B-Rep handles, tessellations, BVHs, GPU buffers, hover state, thumbnails, transient previews, and worker-local caches are disposable. They must be reproducible from canonical meaning or explicitly reported unavailable.

### Projections

Outliner rows, interaction occurrences, complete StateView, agent StateView, BOM, drawing views, manufacturing exports, and render scenes are projections. Each projection MUST declare:

- source document revision;
- projection schema/version;
- evaluator/backend identity where geometry-dependent;
- stale or incomplete state;
- stable IDs linking output back to canonical entities.

## 6.4 Canonical command gateway

The target gateway is the only ordinary route to a new model revision. Its pipeline is:

1. decode and validate command schema;
2. authenticate caller and capabilities;
3. calculate authoritative typed read/write sets and affected graph;
4. enforce command/entity/topology/time/CPU/RAM/I/O/concurrency budgets;
5. validate preconditions and current dependency digest;
6. perform isolated candidate-state application;
7. evaluate mandatory cheap semantic/domain validators;
8. produce authoritative canonical diff, risks, and digest;
9. require confirmation when policy classifies the change as risky;
10. revalidate dependency and policy epochs to prevent TOCTOU;
11. append exactly one immutable revision or no revision;
12. invalidate/increment affected generations and schedule derived work;
13. append structured audit evidence according to retention policy.

A dry-run MUST NOT mutate the document, consume persistent IDs irreversibly, publish shared cache entries, or alter worker-global state.

Undo, Redo, Open, and document replacement are explicitly modeled lifecycle operations. Undo/Redo select an existing canonical revision and invalidate incompatible jobs. Open builds and audits a separate candidate store before replacing the current store. Neither operation is permission for an arbitrary second mutation API.

## 6.5 Queries

Queries are read-only, snapshot-bound, and deterministic for a declared projection schema. A query MUST NOT trigger an implicit canonical commit. Expensive derived-data requests MAY schedule work but return `Pending`, `Stale`, `Unavailable`, or structured diagnostics until results are accepted.

The minimum query families are:

- canonical entity/relationship query;
- hierarchy and edit-context projection;
- feature/rule DAG and evaluation status;
- exact/mesh body and reference-health query;
- spatial/picking/snap query;
- validation diagnostics;
- complete StateView;
- agent StateView;
- BOM/dimension/manufacturing projections.

## 6.6 Revision and concurrency contract

- Exactly one writer appends model revisions.
- Readers hold immutable snapshots and do not block on exact evaluation.
- Each derived job carries document scope, revision, root producer plus complete `SlotPath`, generation, input digest, evaluator identity, backend identity where relevant, and tolerance/schema envelope.
- A result is accepted only if every acceptance field still matches.
- Stale results are discarded or retained only as explicitly labeled diagnostics; they are never current.
- Cancellation is cooperative for safe services and process termination for killable exact-worker operations.
- Worker crash cannot damage the last committed document revision.
- The supervisor restarts a clean worker and reschedules from canonical inputs according to bounded retry policy.
- Repeated edits must reach a measured cache/memory plateau after warm-up.

## 6.7 Failure model

Failures are typed and localized only at the presentation edge. Required high-level classes include:

- schema/capability/budget/precondition failure;
- stale proposal or stale evaluation result;
- invalid canonical invariant;
- evaluator unavailable or version mismatch;
- exact backend exception/crash/timeout;
- invalid geometry or tolerance degradation;
- reference ambiguous/lost/quarantined;
- validation error, incomplete validation, or required validator unavailable/unlicensed/stale/incompatible;
- persistence corruption/limit/checksum/migration failure;
- export loss or unsupported semantics.

A failure MUST identify whether canonical state changed. Unless a successful commit digest is returned, callers assume no canonical mutation.


# 7. Canonical document and evaluator model

## 7.1 Target conceptual schema

Exact Rust layouts remain an implementation detail, but the semantic ownership is normative:

```rust
Document {
    id: DocumentId,
    schema: DocumentSchemaVersion,
    units: UnitSystem,
    determinism: DeterminismEnvelope,
    evaluation: EvaluationEnvelope,
    entities: EntityTables,
    graph: FeatureRuleGraph,
    references: ReferenceTable,
    joints: JointTable,
    validation_policy: ValidationPolicyRef,
    extension_namespaces: ExtensionMap,
}

EntityTables {
    definitions,
    occurrences,
    groups,
    tags,
    collections,
    saved_views,
    persistent_dimensions,
}

FeatureRuleGraph {
    nodes: NodeId -> NodeSpec,
    edges: typed input/output references,
}

NodeSpec = Parameter | Expression | Rule | Profile | Extrude | Cut | Union |
           Revolve | Sweep | Loft | Shell | Fillet | MeshOperation |
           DomainFeature | ...

BodySpec = Exact(ExactBodySpec) | Mesh(MeshBodySpec)
```

`ValidationPolicyRef` resolves a versioned canonical policy containing required/optional validator contract identities and versions, declared input/result envelopes, governed standards/jurisdictions/editions where applicable, and severity/blocking rules. It grants no executable capability. M7 hosting may bind that contract identity to publisher, trust, licensing, isolation, or egress policy, but those host concerns are not prerequisites for the M4a protocol or its first built-in validators and license secrets never enter canonical state.

The document stores specifications and semantic relationships. Native OCCT handles, GPU objects, validator binaries/license credentials, and widget state never appear in canonical state.

## 7.2 ID policy

Every durable entity ID MUST:

- be non-zero and unique within its declared document/asset scope;
- never be silently reused for semantically different content;
- survive Save/Open unchanged unless an explicit import/remap transaction reports the mapping;
- have a defined allocator owned by core, not ad hoc `max + 1` clients;
- have overflow and malformed-input behavior;
- appear in complete StateView and relevant audit records.

`DocumentId` MUST be generated per document rather than defaulting every document to `1`.

Derived output identity uses `(root_rule_node_id, SlotPath)`, not creation order or a flat current array index. `SlotPath` is a non-empty ordered sequence of semantic `SlotSegment { producer_rule_id, output_port, semantic_key }` values; each rule invocation mints the segment for the nesting level it owns. A segment MUST remain stable under unrelated sibling insertion, deletion, or reordering and MUST NOT be derived solely from a current index. For example, `A/bay:named-end-7/B/post:named-end-3` identifies a nested post without flattening its ancestry.

Resolution traverses the path segment by segment and validates the expected producer and output schema at every level. A missing continuation is `Lost`; multiple valid continuations are `Ambiguous`. Resolution MUST NOT reindex, choose a nearest sibling, or silently retarget an override. Jobs, caches, persistence, StateView, references, joints, diagnostics, and overrides carry the complete `SlotPath`. `SlotPath` identifies nested derivation and is distinct from `InstancePath`, which identifies one placement of reusable definition content.

## 7.3 Definitions, occurrences, and groups

- A `Definition` owns reusable local feature/rule outputs and local semantic metadata.
- An `Occurrence` references exactly one definition and owns transform, occurrence-level name/visibility/tag/override data, and parent context.
- A `Group` is a unique ownership/edit boundary and can contain groups and occurrences.
- Geometry remains in definition-local coordinates; world placement derives from deterministic transform composition.
- A component is a reusable definition plus one or more occurrences, not a separate geometry authority.
- `MakeUnique` clones all owned canonical definition-local structures through a complete ID mapping and repoints only selected occurrences.
- Group-to-component conversion preserves world placement and reports all resulting identity mappings.
- Deleting a non-empty ownership boundary requires an explicit semantic operation such as recursive delete or reparent; it MUST NOT silently orphan content.

### Reusable nested assemblies

A reusable assembly cannot be modeled only by global occurrences and groups. Each `Definition` therefore owns a **definition-local content graph** containing local features, local groups, and child-occurrence prototypes. A root document assembly is a distinguished root definition/scope. Instantiating a definition creates an occurrence of its entire local graph without cloning semantic definition content.

An `InstancePath` contains one globally scoped root `OccurrenceId` followed by ordered `DefinitionLocalOccurrenceKey` segments. Each segment records its expected owning `DefinitionId`/scope and a stable, non-reused child-prototype key. The pair `(owning_definition_id, local_key)` identifies a reusable prototype; the complete path identifies one placement. Path resolution validates the expected definition at every hop and never resolves by an unscoped numeric occurrence ID. The path is distinct from definition-local body/feature identity shared by all instances.

Normative rules:

- definition-local child occurrence prototypes reference definitions and have stable local IDs;
- definition ownership edges are acyclic, even when ordinary group nesting is considered separately;
- world transforms compose root occurrence, nested child occurrence, and local group transforms in a fixed order;
- edit context names a definition scope plus an optional instance path used for presentation/override context;
- an edit to shared definition-local content affects every instance path of that definition;
- an occurrence-specific override is keyed by the complete instance path and permitted override slot;
- group-to-component conversion moves selected owned content into a new definition-local graph, emits a complete old-path-to-new-path mapping for every affected descendant, and preserves world placement;
- Make Unique clones the entire owned definition-local graph, including local groups, child prototypes, rules, features, references, and allowed overrides, then remaps all internal IDs and descendant paths atomically; any path without a proved mapping becomes `Lost` or `Ambiguous`, never silently retargeted;
- deletion reports orphaned overrides/references and never silently retargets another instance;
- persistence and StateView encode local scope and instance paths explicitly.

The current global `Group` plus top-level `Occurrence` model does not yet satisfy this contract.

## 7.4 Parameters, expressions, and rules

V4-P01/V4-P05 replace the current parallel graphs with one evaluator graph.

A parameter or expression node has:

- stable ID and name;
- explicit type and unit dimension;
- source expression/token;
- parsed versioned expression representation;
- typed dependencies;
- evaluator identity;
- declared output type/schema and a snapshot-bound evaluation-status reference; evaluated values, geometry results, and evaluation diagnostics reside in the non-canonical derived-result registry;
- input/Merkle digest;
- optional domain meaning.

A rule node additionally declares:

- input ports and output slots;
- deterministic semantic `SlotPath` segment generation at the rule level that owns each nesting segment;
- output schema;
- allowed overrides and merge policy;
- validators that MUST pass before outputs are accepted;
- resource budget class.

Rules do not directly mutate live document maps while evaluating. They produce a deterministic candidate result or a command/diff representation that the gateway validates and accepts atomically.

## 7.5 Dirty propagation and evaluation

For each node:

```text
input_digest = H(
  node_spec,
  ordered dependency result fingerprints,
  evaluator identity,
  relevant backend identity,
  schema versions,
  tolerance profile,
  platform envelope when relevant
)
```

The document-level digest is derived from canonical semantic state. It is not a substitute for per-node input identity.

On commit:

1. calculate the exact affected graph from typed writes;
2. increment affected generation counters;
3. retain last accepted results as explicitly stale where useful;
4. schedule ready nodes in DAG order;
5. accept results only when complete job identity still matches;
6. attach diagnostics/fingerprint/reference changes;
7. expose status to queries and UI.

A node can be `Clean`, `Dirty`, `Evaluating`, `Succeeded`, `Failed`, `Stale`, `Blocked`, or `Quarantined`. Downstream evaluation MUST NOT silently proceed through an ambiguous/lost required reference or invalid upstream result.

## 7.6 Overrides

Manual edits to rule-derived objects require explicit semantics:

- **parameter override:** replaces a named allowed parameter while retaining derivation;
- **transform override:** changes allowed placement output;
- **feature augmentation:** attaches a downstream feature to a stable derived slot;
- **detach:** explicitly converts derived content to independent canonical content with provenance/loss report;
- **forbidden edit:** rejected when no safe merge policy exists.

Every override carries the root producer and complete `SlotPath` it targets. Re-evaluation resolves every segment using the same explicit `Resolved/Ambiguous/Lost` discipline as subshape references; failure at any nesting level is reported at that segment and never retargeted to a sibling.

## 7.7 Persistent dimensions, tags, collections, and views

Persistent dimensions are canonical semantic annotations referencing stable canonical entities, complete derivation `SlotPath` values, or durable subshapes and storing intended measurement mode, presentation preferences, and reference-health state. Their displayed numeric result is derived. Entity- and `SlotPath`-anchored dimensions MAY precede M3; a dimension requiring exact subshape identity MUST wait for integrated exact results and C1b. A proxy-only pick/face identifier MUST NOT become a durable annotation reference.

Tags are canonical metadata entities. Assignment does not change geometry ownership. Tag visibility is a view policy over occurrences/entities and is distinct from occurrence deletion or group membership.

Collections organize semantic membership without creating another parent/ownership tree. Saved views store camera/presentation choices and optional visibility/filter state but never become geometry authority.

## 7.8 Canonical digest and StateView

The canonical digest MUST:

- use a versioned domain separator;
- traverse canonical data in deterministic order;
- include exact numeric encodings and semantic schema IDs;
- exclude disposable caches and localized prose;
- have a cryptographically suitable algorithm for file integrity/content addressing, distinct from a cheap in-memory dirty-check hash if both are retained.

The complete StateView is a deterministic, lossless textual or structured projection used by golden tests, diffs, diagnostics, and external review. The agent StateView summarizes names, relationships, dimensions, rules, validation/reference health, and intended actions while omitting irrelevant matrices/blobs. They share one semantic encoder but have separate version IDs and compatibility policies.


# 8. Geometry, references, interaction, and rendering

## 8.1 Exact and mesh authority

```rust
enum CanonicalBodySpec {
    Exact(ExactBodySpec),
    Mesh(MeshBodySpec),
}
```

An exact body specification is authoritative for exact dimensions, B-Rep operations, semantic topology references, and exact export. A mesh body specification is authoritative only for explicitly mesh-native/imported/procedural workflows. Tessellating an exact body creates derived render/query data, not a second canonical mesh body.

Exact-to-mesh and mesh-to-exact conversions are named operations. They record source, destination, tolerances, unsupported semantics, stable-reference consequences, and loss report. A failed exact operation MUST NOT silently retry as a mesh operation and present it as equivalent.

## 8.2 Exact worker contract

The persistent worker selected by ADR 0002 exposes a versioned logical protocol independent of transport encoding. A request contains:

- document/revision/node/output identity;
- generation and full input digest;
- evaluator and backend build identity;
- operation schema and exact typed inputs;
- tolerance and coordinate envelope;
- resource/deadline/cancellation class.

A successful result contains:

- owned result handle or transferable persisted representation;
- automatic validity result;
- backend-produced conservative bounds with declared gap/tolerance, algorithm/version, and error direction; analytical or tolerance-checked volume/area where applicable; and topology summary;
- complete available `Generated/Modified/Deleted` evidence;
- supplemental topology walk/diff evidence;
- tolerance/degradation report;
- structured diagnostics;
- result fingerprint and history confidence;
- backend identity actually used.

Every C++ exception is contained. Raw pointers and OCCT types never cross the boundary. Worker death yields a typed infrastructure failure, never a partially accepted model commit.

The parent supervisor provides request deadlines, heartbeat/health detection, bounded retries, restart, and deterministic rescheduling. Repeated deterministic geometry failure is not retried indefinitely.

## 8.3 Operation families and staged support

Each operation family owns an explicit support envelope, reference contract, corpus, and fallback consequence. Candidate families include rectangular/general profiles and extrusion, booleans, revolve, shell, fillet/chamfer, spline inputs, sweep, loft, import/export/repair, mesh-authoritative procedural operations, and domain-specific exact operations such as timber notches.

Their implementation order is not fixed by mathematical generality. It is chosen by the smallest operation set needed for a frozen product workflow: first the fabrication slices in §10, then the blocking editable bottle slice in §1.6/M6, while the living-tree slice remains non-blocking research, and thereafter measured domain demand. Adding an operation is not only adding a façade function. It requires canonical schema, command/proposal semantics, evaluator wiring, reference behavior, direct manipulation, rendering, persistence/migration, validation, and end-to-end workflow evidence.

Before a point loop is accepted as a profile, its schema must define closure, orientation/winding, duplicate vertex/edge handling, self-intersection, collinearity and minimum edge rules, hole containment/orientation, tolerance quantization, coordinate envelope, and deterministic normalization. Invalid loops are rejected before exact evaluation/export; a cuboid bounding proxy must never mask malformed profile semantics. Adversarial profile tests run through the product command path as well as the isolated exact validator.

## 8.4 Body-local and assembly selection references

V4 distinguishes reusable body topology from a selected occurrence path:

```text
BodySubshapeRef {
  document_or_asset_scope
  reference_schema_version
  producer_node_id
  complete_slot_path
  result_and_input_digest
  semantic_role
  source_element_id
  genesis_or_lineage_path
  expected_geometry_type
  topology_orientation_and_location_schema
  adjacency_signature
  geometric_signature
  expected_cardinality
  stability_class
  backend/evaluator/tolerance provenance
}

AssemblySelectionTarget {
  root_document_scope
  instance_path
  body_result_ref
  subshape: BodySubshapeRef
}
```

`BodySubshapeRef` is definition/body-result local and may be shared by many occurrences. `AssemblySelectionTarget` identifies one placed occurrence path and is used by picking, occurrence-specific dimensions, joints, overrides, and assembly relationships. A definition-wide feature uses the body-local reference without pretending one occurrence is special.

Every durable reference requires document scope, schema, producer and output identity, complete input/result digest, expected geometry and cardinality, stability class, and evaluator/backend/tolerance provenance. `Guaranteed` additionally requires a named semantic role/source contract and the complete operation-specific topology evidence schema. Optional geometric signatures may corroborate identity but cannot replace missing required provenance.

Resolution order is:

1. exact semantic role in a supported feature contract;
2. evidenced backend history;
3. lineage/genesis path;
4. topology and adjacency signatures;
5. geometric signature as corroboration, never sole proof of identity.

Valid results are `Resolved`, `Ambiguous`, and `Lost`. Resolution canonicalizes topology orientation/location, adjacency ordering, tolerance quantization, and candidate ordering according to the reference schema before comparison. If any `InstancePath` segment is missing or semantically changed, the assembly target is `Lost` or `Ambiguous` even when the same body-local face exists elsewhere. `Quarantined` is document/evaluation state caused by an unresolved compatibility audit, not permission to choose a candidate.

Stability classes are:

- `Guaranteed`: only a named, frozen, fully evidenced feature subset;
- `BestEffort`: deterministic safe recovery may succeed but is not promised;
- `Ephemeral`: valid only for one preview/result generation;
- `Ambiguous`: multiple candidates remain;
- `Lost`: no safe candidate exists.

Every expansion of `Guaranteed` requires a new preregistered corpus and 100% correct identity with zero silent misbinding.

## 8.5 Backend-change audit

When document and active backend/evaluator identities differ:

1. keep the source file untouched;
2. invalidate disposable caches;
3. load stored canonical state and last accepted outputs in review mode;
4. run a non-destructive reference and derivation audit;
5. report each `Resolved/Ambiguous/Lost` change and causal identity mismatch;
6. quarantine only affected branches where possible;
7. prevent invalid branches from recomputing/exporting as healthy;
8. generate an authoritative migration/recompute diff;
9. commit only after explicit confirmation through one command batch;
10. preserve the pre-migration copy and audit disposition.

## 8.6 C1a — authority gate

C1a is required before broadening the interaction model. Its threshold is zero authority divergence.

The test suite MUST prove that:

- every interaction occurrence is projected from a named canonical occurrence in a named revision;
- transform, dimensions/body reference, visibility, parent/edit scope, and sharing identity agree with that snapshot;
- interaction scene mutation APIs cannot create durable model meaning;
- ephemeral previews are tagged and disappear on cancel/revision change;
- a document edit appears only after a successful canonical batch;
- no independent interaction Undo/history exists.

Public APIs such as `InteractionScene::add_occurrence` may remain as builders, but production construction must be centralized behind a canonical projection service and tested as such.

## 8.7 C1b — exact resolver equivalence gate

C1b begins only when accepted exact results enter the product application. For a preregistered body and mutation corpus:

- interaction picking resolves to the same canonical `SubshapeRef` as direct exact-topology resolution;
- zero silent identity mismatches are permitted;
- ambiguous/lost outcomes agree or are explicitly conservatively downgraded;
- every selected result carries revision, generation, result fingerprint, and backend identity;
- coarse GPU/mesh candidates are verified by the authoritative spatial service before semantic selection.

Failure produces an architecture disposition: a single resolver authority, conservative accelerator contract, or explicit body-family split. It MUST NOT be patched by choosing whichever candidate looks visually plausible.

## 8.8 Spatial query and snapping

The target interaction service owns:

- broad spatial indices and overlap candidate sets;
- exact or conservatively verified hit testing;
- endpoint, midpoint, intersection, axis, plane, alignment, and inference candidates;
- deterministic candidate scoring, hysteresis, hover lock, and cycling;
- selection filters and edit-context constraints;
- stable-reference creation and resolution;
- diagnostics when geometry is stale, unavailable, or ambiguous.

The renderer may supply a coarse ID buffer or depth candidate. It does not decide semantic identity.

## 8.9 Rendering

The target renderer consumes revision-tagged derived render packages:

- shared mesh per accepted definition/body result;
- per-occurrence transform, visibility, style override, and pick index;
- BVH/spatial acceleration references;
- edges, faces, highlights, snaps, grid/axes/gizmo, and stale/failure overlays;
- explicit approximation and LOD metadata.

Ten thousand occurrences of one definition must retain one shared authoritative geometry/result and shared render geometry where compatible. Per-occurrence data is limited to placement, overrides, and indices.

Rendering MAY remain responsive using the last good mesh while evaluation runs, but stale presentation must be visible and cannot be selected/exported as if current unless the interaction contract explicitly resolves against that accepted revision.


# 9. Persistence, Open, migration, and compatibility

## 9.1 Native container target

```text
model.ketchup
├─ manifest.json
├─ document.bin
├─ accepted-results/         # optional, derivation-tagged
├─ audit/commands.log        # optional, bounded/retention-controlled
├─ blobs/<content-hash>
├─ cache/                    # disposable
├─ previews/
└─ extensions/<namespace>/
```

The container format MAY differ physically, but it MUST provide equivalent separation and integrity.

## 9.2 Manifest

The manifest declares at minimum:

- container and document schema versions;
- document ID and canonical digest;
- writer core version/build;
- evaluator identities;
- exact/mesh backend identities and build fingerprints;
- unit/tolerance/coordinate/platform determinism envelope;
- required and optional extension namespaces;
- required/optional validator contract and policy identities, versions, declared result class/envelope, governing standards/jurisdictions/editions where applicable, and last accepted result provenance; optional M7 host metadata may add trust/licensing state but never license secrets;
- checksums, sizes, and content hashes;
- migration lineage and last accepted audit status;
- whether accepted derived results are present and under which input digests.

## 9.3 Safety limits

Before allocating unbounded structures, Open enforces versioned limits for:

- total file/container size;
- decompressed size and expansion ratio;
- entry count and path safety;
- entity/node/edge/reference counts;
- hierarchy and expression depth;
- string/blob sizes;
- topology/result sizes;
- extension namespace budgets.

Unknown safe namespaced data is preserved. Unknown required semantics block full edit/export rather than being discarded. Paths are normalized and constrained within the container.

## 9.4 Save contract

Save:

1. serializes one immutable canonical snapshot;
2. deterministically emits canonical content and manifest;
3. calculates checksums/hashes;
4. writes to a sibling temporary target;
5. flushes and synchronizes required data;
6. validates the completed temporary artifact where practical;
7. atomically replaces the destination;
8. never destroys the only good copy on migration failure.

The document becomes clean only when the saved canonical digest matches the active snapshot. Saving does not imply all disposable caches were serialized.

## 9.5 Open contract

Open distinguishes **representation decoding** from **semantic migration**.

Representation decoding may losslessly map an older physical encoding into an equivalent in-memory schema. It is not a model edit. Any transformation that changes, reconstructs, drops, or guesses semantic meaning is a semantic migration and cannot be silently completed by Open.

Open is a staged, failure-safe observation:

1. inspect container structure and resource limits;
2. verify checksums and required namespaces;
3. losslessly decode into a separate read-only candidate where possible;
4. collect every required semantic migration and explicit loss instead of applying it invisibly;
5. validate candidate canonical invariants and reference syntax;
6. compare stored and active evaluation/determinism envelopes;
7. classify retained derived results as `Current`, `Stale`, `Unverified`, or `Unavailable` from their complete keys;
8. run a non-destructive compatibility/reference audit;
9. present losses, warnings, quarantine state, and available actions;
10. activate an editable document only when decoding is lossless and no semantic migration or unresolved loss is required; otherwise expose a separate read-only review candidate while leaving the previously active editable document unchanged;
11. retain the previous active document unchanged on any rejected Open.

A lossless old-schema candidate may become the active document without creating a model revision. A lossy or meaning-changing migration first opens as a review candidate, preserves the source artifact, and produces an authoritative migration diff. A review candidate is not the active canonical store, cannot receive ordinary model edits, and cannot be saved over the source artifact. Only the confirmed migration transaction may create and activate the new document lineage; it is never marked clean against the old file until explicitly saved.

The current schema-0 `migration_losses` behavior is therefore a known implementation gap: the app must surface and require disposition of those losses rather than silently treating the replacement as an ordinary clean Open.

## 9.6 Two-phase evaluation and recompute policy

Ordinary evaluation of unchanged canonical semantics is a derived-data operation, not a model mutation:

1. bind base document ID/revision, root producer and complete `SlotPath`, dependency and policy epochs, complete input digest, evaluator/backend identity, and resource budget;
2. evaluate into isolated result storage;
3. validate result geometry, references, diagnostics, and required domain policy;
4. produce a result digest and projected-result comparison;
5. revalidate every bound identity using compare-and-swap semantics;
6. register the result for that exact canonical revision or reject it as stale.

This registration creates a derived-result event. It is excluded from model Undo/Redo and the canonical model digest. Undo simply selects another canonical revision and may reuse only a registry entry whose complete key matches it. Save may persist registry evidence/cache independently of whether the canonical document is dirty. Worker failure before registration changes no current result.

A recompute caused by evaluator/backend change uses the same isolated protocol and reports:

- affected nodes and downstream branches;
- whether inputs, evaluator, backend, tolerance, schema, file integrity, or extension availability caused the mismatch;
- reference-resolution changes;
- validation changes;
- result/projection differences;
- whether exact edit, validation, or export is safe.

If recomputation changes only disposable derived representation while preserving all canonical references and semantics, accepting the verified result is a derived-result event, not a command batch. If it changes a canonical `SubshapeRef`, stability class, body specification, rule output identity, override, migration lineage, or other semantic state, the system produces a separate candidate canonical diff; human confirmation and dependency revalidation precede exactly one command-batch revision. This semantic transaction is Undoable.

Concurrent Save binds either the pre-existing or newly registered derived-result manifest atomically; it never writes a half-registered result. A stale confirmation, base revision change, policy epoch change, or worker result mismatch fails closed.

## 9.7 Schema evolution

Each schema transition has:

- source and destination versions;
- classification of each step as lossless representation decode or semantic migration;
- deterministic candidate migration implementation;
- invariant and resource-limit validation;
- declared preserved, transformed, dropped, reconstructed, guessed, and unknown semantics;
- explicit machine-readable and user-visible loss report;
- authoritative candidate diff and confirmation class;
- round-trip/golden fixtures;
- original-file preservation and new file lineage;
- migration audit record;
- failure/rollback behavior before and after Save.

Schema 3 is the earliest plausible target for expressions, per-node digests, derivation provenance, and evaluation envelopes. The number is not binding until an ADR and migration fixture set are accepted.

## 9.8 Compatibility promise

No public backward-compatibility promise is made until the suite includes:

- every released historical schema fixture;
- repeated Save/Open cycles with zero canonical drift;
- corruption and resource-limit cases;
- unknown optional/required namespace cases;
- at least two pinned backend/evaluator builds;
- `Resolved/Ambiguous/Lost` reference migration evidence;
- exact and mesh body cases;
- retained-result mismatch, semantics-preserving derived registration, and semantic-diff transaction behavior;
- rollback/recovery after interrupted Save and migration.


# 10. Validation, joints, rules, and fabrication outputs

## 10.1 Validation as a product service

Validators are deterministic read-only consumers of a named snapshot and accepted derived-result set. Core owns an open, domain-neutral validator interface, policy model, diagnostic schema, result taxonomy, and capability boundary; it need not contain every domain implementation. M4a freezes this host-neutral protocol and proves it with built-in validators. Discovery, installation, update, signatures, revocation, payment/licensing, native-code isolation, and remote execution/egress for third-party implementations are M7 hosting concerns, not part of the M4a exit.

Every validator invocation declares stable contract/implementation identity and version, required read scopes, determinism and input envelope, resource limits, policy severity, diagnostic schema, and—where applicable—the governing standard, jurisdiction, and edition. Results bind the document revision, accepted derived-result identities, contract/policy identity, and complete input digest. When M7 introduces external packages, the host additionally binds publisher/signature/trust/revocation state and, for remote execution, destination, data classes, retention assumptions, and cancellation behavior before any egress.

Validation classes include:

- canonical structural invariants;
- unit/expression/rule correctness;
- exact/mesh body validity;
- collision, spacing, and clearance;
- declared-joint consistency;
- support/connectivity/completeness;
- structural/statics checks;
- standards/code and jurisdiction-specific checks;
- manufacturability and export preconditions;
- explicitly advisory systems, including feng shui where a package defines reproducible inputs and rules.

Regulatory, structural, manufacturability, and advisory results are distinct classes. Structural/statics validation is supported as transparent **best-effort decision support**: the result MUST expose its assumptions, model, envelope, version, evidence, and unresolved conditions, and MUST NOT be presented as a Ketchup guarantee that a structure is safe or code-compliant. Any structural/statics safety guarantee, certification, or approval carrying professional responsibility MUST be independently reviewed and signed by a qualified structural engineer; any additional jurisdictional-authority approval is supplementary, never a substitute. A Ketchup result does not replace that review regardless of whether it says `Passed`. Distribution terms and user-facing release/export copy MUST preserve this boundary. A subjective/advisory validator likewise MUST NOT present itself as regulatory certification or structural proof. A required validator that is unavailable, unlicensed under an M7 host, stale, incompatible, outside its declared envelope, or missing trusted inputs returns `Unavailable` or `NotEvaluated`, never `Passed`, and may block commit/export/release according to document policy.

A command pipeline may require selected cheap validators to pass before commit. More expensive validators may run asynchronously and block export/release rather than interactive commit, according to explicit policy. Validators return structured diagnostics with stable codes, entity/reference locations, severity, policy version, and evidence; they do not repair or mutate the document implicitly. “Not evaluated” is distinct from “passed.”

## 10.2 Collision pipeline

For the initial prismatic domain, collision validation uses:

1. AABB broad phase over whole pieces;
2. optional OBB or convex bounds to reduce candidates;
3. deterministic `f64` SAT over convex polyhedra or canonical convex coverage as a candidate/contact test;
4. tolerance-profile classification of contact versus penetration;
5. for every SAT-positive convex component pair, deterministic construction of the convex intersection region `I_ij = A_i ∩ B_j`, for example by half-space clipping;
6. declared-joint containment evaluation for every non-empty penetrating intersection region at the whole-piece pair level.

A canonical convex **coverage** may overlap internally and is valid for collision detection if its union covers the solid without missing occupied space. It MUST NOT be reused for mass/volume calculations when overlapping components would double-count. Intersection containment is evaluated per non-empty convex component-pair region; regions MUST NOT be summed as physical overlap volume unless a non-overlapping representation proves that operation valid.

For curved bodies, the first fallback is a conservative envelope that may produce false positives but MUST NOT miss collisions. Tessellation-based validation is permitted only when tessellation parameters and permitted error direction are part of the determinism envelope.

## 10.3 Declared joints

A declared joint is a canonical entity containing:

- stable joint ID and semantic joint type;
- participating entity, complete `InstancePath`, and/or complete derivation `SlotPath` references;
- bounded joint volume in a declared coordinate frame;
- expected contact/overlap relationship and tolerances;
- source rule or manual provenance;
- optional manufacturing semantics.

Validation outcomes:

| Condition | Result |
|---|---|
| Penetration without a declared joint | Error |
| Expected penetration wholly inside the joint volume | Valid joint overlap |
| Penetration outside the joint volume | Error |
| Declared joint with no required intersection/contact | Error |
| Ambiguous/lost participant reference | Error or quarantine; never valid |

SAT is insufficient to classify a declared-joint overlap because it does not locate the complete penetration region. For allowed joint volume `J`, Euclidean tolerance ball `B_ε`, and tolerance threshold `ε`, every non-empty penetrating `I_ij` MUST be wholly contained in an acceptance region `K_ε` that is proven to satisfy `K_ε ⊆ J ⊕ B_ε`; any portion outside is an error. An exact/certified distance-to-`J` test or a conservative inner approximation is permitted. Independently shifting every polyhedral face outward by `ε` generally creates a strict **superset** near edges and corners—up to `ε(√3−1)` extra radial reach at an orthogonal trihedral corner—and MUST NOT be used as the final acceptance region; it may only be a candidate stage followed by the certified containment test. Absence of the required intersection/contact is an empty-joint error. A joint MUST NOT exempt an entire pair of pieces from collision checking. The bounded, non-permissive containment test is what makes the exception safe.

## 10.4 Stable rule outputs

Rule-generated pieces, dimensions, and joints use hierarchical semantic `SlotPath` values. A spacing rule might mint a segment from a named end and preserve explicit remainder-distribution semantics; a nested rule appends its own segment without flattening or renumbering the parent path. Output order is deterministic and derives from rule semantics, not map iteration.

A changed rule produces a result comparison:

- preserved paths with unchanged semantic identity;
- modified paths;
- newly created paths;
- removed/lost paths;
- ambiguous correspondence;
- overrides that still apply, require review, or are orphaned.

This comparison is visible before accepting a risky recompute.

## 10.5 Beam workflow 6.3a

The first product-level rule test is a timber beam/member workflow without exact notches:

1. define span, field counts, cross-section, named origin, and remainder-distribution rule;
2. generate canonical derived pieces/slots;
3. produce a minimal grouped list of piece identity, quantity, and length;
4. declare bounded joints where expected intersections occur;
5. run collision and empty-joint validation;
6. change one governing value;
7. recompute only the dependent branch;
8. show preserved/lost/ambiguous slot and override identities;
9. regenerate the grouped piece/length list;
10. commit the accepted change as one user operation.

This minimal run is checkpoint M4a-E. It intentionally excludes the full BOM and stable-dimension projection contract, the reference dimension chain (`415 × 6`, `408 × 5`, `400`), FurniGen import, third-party validator hosting, AI, and BTLx so that it exposes evaluator/slot/product failures as early as possible. The full BOM/dimension projections, built-in host-neutral validator contract, and available FurniGen evidence follow immediately in the M4a completion track without delaying this first observation.

## 10.6 Beam workflow 6.3b

The second slice adds exact notch/groove geometry and depends on:

- product exact-worker integration;
- exact canonical body specifications;
- C1b reference/picking equivalence;
- exact cut feature and expanded topology corpus;
- joint-to-feature provenance;
- collision coverage for the notched body;
- manufacturing dimension and operation projection.

The half-lap relationship is one semantic joint whose two cuts and dimensions are derived consequences, not unrelated manually drawn voids.

## 10.7 BOM, dimensions, drawings, and manufacturing

These outputs are deterministic projections:

- **BOM:** stable piece/definition/slot identity, material, quantity, dimensions, and validation state;
- **piece dimension sheet:** named views and dimension chains generated from stable features/references;
- **manufacturing operation list:** semantic cuts, drillings, notches, and coordinate frames;
- **BTLx or other domain export:** a later encoder over validated manufacturing semantics, not over arbitrary rendered triangles.

Full professional drawing layout remains later scope, but a deterministic piece drawing and dimension chain is part of the fabrication product path.

## 10.8 FurniGen evidence

The owner’s prior FurniGen experience establishes an important product hypothesis: generation without validators was unusable; the same generation constrained by validators produced buildable furniture. Ketchup therefore treats validator coverage and false-negative control as acceptance metrics, not optional polish.

The original FurniGen failure corpus, exemptions, and tolerance rationale should be imported as provenance-preserving fixtures when available. If unavailable, the project records that evidence gap rather than fabricating equivalence.


# 11. UI, AI, plugin, privacy, and security contracts

## 11.1 Manual application contract

The designed desktop shell remains the product baseline. A workflow is complete only when:

1. the command is discoverable through menu, tool rail, Outliner, or documented shortcut;
2. the viewport gesture and exact numeric path both work where applicable;
3. localized hint, status, and action digest explain the operation;
4. preview is ephemeral and cancellable;
5. confirmation emits one canonical batch;
6. viewport, Outliner, StateView, persistence, and queries agree on identity;
7. Undo/Redo restores canonical meaning;
8. focused tests exercise canonical transition and interaction state;
9. a runnable build demonstrates the workflow without hidden developer controls.

## 11.2 Presentation state

The app owns only:

- active tool and pending gesture;
- hover and selection view;
- edit-context navigation stack;
- value-box text and last applicable operation;
- camera, viewport, open panel, and menu state;
- localized action/diagnostic presentation;
- file-dialog and confirmation session state.

Model geometry, entity transforms, hierarchy, feature parameters, tags, persistent dimensions, validation policy, and model history belong to core.

## 11.3 Proposal model

An AI or high-level automation produces an `Intent` and then a `Proposal`, not direct privileged mutation. A Proposal records:

- base snapshot provenance;
- user goal and structured assumptions;
- proposed semantic operations;
- intended command digest;
- authoritative diff placeholder;
- risk and required confirmation class;
- validity dependencies;
- requested resource budget;
- expected verification criteria.

Core calculates authoritative read/write sets, dependency fingerprints, query/selection inputs, policy epochs, schema/tolerance identity, and final diff. Client-supplied sets are hints only.

A relevant change makes the Proposal stale. It is never silently rebased. An unrelated revision may be revalidated without asking the language model to regenerate the plan.

## 11.4 AI boundaries

- AI receives a small contextual intent/tool vocabulary rather than unrestricted internal commands.
- AI output is untrusted data and must pass schema, capability, budget, semantic, geometry, and domain validation.
- AI never receives a mutable document handle or exact-backend pointer.
- A plausible natural-language response is not acceptance evidence; canonical and geometry invariants decide success.
- Bulk deletion, lossy conversion, overwrite, cloud upload, manufacturing export with warnings, and other high-risk operations require explicit confirmation.
- Verification compares the committed result with Proposal expectations; a safety- or manufacturing-relevant mismatch fails closed and blocks export until explicitly resolved.

Every operation records a requesting principal with an explicit capability grant. A high-risk confirmation additionally records a distinct authenticated approving-human principal and the approval capability exercised. AI, plugin, importer, and other machine principals cannot mint or satisfy human-only approval. Human-only classes include destructive bulk changes, overwrite, lossy conversion, external/cloud disclosure, release/manufacturing export with unresolved warnings, and capability expansion.

Only a trusted confirmation surface may issue the short-lived token. It is bound to both requester and approving human, base document/revision, exact authoritative command digest, authoritative diff/result digest, displayed risk class, destination/path/provider where applicable, policy epoch, and expiry. Any bound-value change invalidates it. Confirmation cannot be replayed on a different requester, approver, document, provider, path, or revised diff.

Queries use least-privilege read scopes. Data returned to a model is minimized, labeled by origin/trust, and treated as tainted content rather than system instruction. Provider and egress policy identifies destination, data classes, retention assumptions, and revocation/cancellation behavior. Secrets never enter prompts or persisted audit prose. Audit records use structured redaction and bounded retention.

Adversarial acceptance tests cover prompt/tool-output injection, confused-deputy calls, stale/replayed confirmation, capability escalation, unintended data exfiltration, cancellation/revocation, and verification mismatch.

The Assistant UI remains deferred until the narrow manual modeler, StateView, validators, and Proposal gateway are usable.

## 11.5 Plugins and importers

Plugins and importers are capability- and budget-limited clients. The target host provides:

- explicit command/query capabilities;
- filesystem/network/cloud permissions denied by default;
- CPU, memory, time, I/O, entity, and topology limits;
- namespaced persisted data with version and preservation policy;
- process or WASM isolation selected by measured pilot evidence;
- structured errors and audit events;
- no raw OCCT, renderer, or internal database access.

WASM Component Model remains a hypothesis, not a binding implementation choice. M4a defines only the host-neutral validator interface, policy/result semantics, declared snapshot/result reads, and structured diagnostics; it does not load or distribute third-party code. M7 may implement a validator/plugin host. At that point an open or paid package uses the same least-privilege boundary, and untrusted native or remote validators require process isolation, authenticated provenance, revocation handling, and the same egress confirmation rules as other external services.

## 11.6 Threat model

Required threats include:

- prompt and tool-output injection;
- malicious document metadata treated as instructions;
- geometry and expression denial of service;
- decompression/archive bombs and path traversal;
- capability escalation and confused-deputy actions;
- TOCTOU between preview and commit;
- preview/final geometry mismatch;
- native backend memory faults and hangs;
- cache poisoning and stale-result acceptance;
- malicious, substituted, revoked, or over-privileged validator/plugin/importer packages and malformed files;
- cloud exfiltration and telemetry leakage;
- sensitive audit logs;
- manufacturing export from invalid/quarantined state.

Security controls live at boundaries and are versioned/tested. Internal impossible states rely on type/invariant design rather than redundant defensive branches.

## 11.7 Privacy

Model, document, workspace, prompt, and telemetry data remain local unless the user explicitly opts in for a named operation or workspace. A locale, file type, or Assistant panel choice is not consent to upload. Cloud operations show the data scope and destination before confirmation.

## 11.8 Localization

ADR 0001 remains binding:

- English is the project, schema, code, test, and default UI language;
- every user-facing string uses stable resource keys;
- canonical state stores semantic values/error codes, never localized labels or formatted numbers;
- locale-aware parsing occurs at the presentation boundary and produces locale-independent canonical values;
- pseudo-locale and missing-key tests are required;
- a second test locale is required by FLP acceptance.


# 12. Verification, gates, and CI governance

## 12.1 General gate rule

Before observation, each gate freezes:

- exact source/build identity;
- corpus and difficulty classes;
- expected result and oracle;
- metric, method, threshold, and sample count;
- hardware/software envelope;
- success and failure consequences.

After observation, any change creates a new version. Every transition is classified `tighten`, `loosen`, `neutral`, or `unknown`, with whether the change occurred before or after observing the affected result. Narrowing a promised operating envelope after failure is a **loosen**, even if the new test is technically stricter inside the smaller envelope.

Historical results remain immutable and named. A latest report MUST NOT overwrite the provenance of an earlier run.

## 12.2 Current gate disposition

| Gate | V4 disposition |
|---|---|
| R0 | **HISTORICAL GO; current governance PARTIAL—PROOF ONLY.** Toolchain, OCCT, corpora, thresholds, hardware classes, and lock methodology exist. Transition-direction metadata and upper-envelope coverage need completion. |
| A0 | **HISTORICAL REPORT RECORDED GO; INDEPENDENT AUDIT FOUND ORACLE/EVIDENCE GAPS; current tree NOT CERTIFIED.** The run supports a synthetic rectangular semantic-role resolver but does not prove complete adjacency evidence or migration between two real backend builds. Current formal invocation fails closed on r0-v13 build-input hash mismatch. Preserve the artifact, add an audit addendum, and preregister a stronger A0 before another claim. |
| A1 | **PARTIAL—PROOF ONLY for the legacy research graph.** Atomicity/round-trip evidence is useful but does not prove unified evaluator, exact bodies, references, or new persistence. |
| B | **HISTORICAL DECISION-SUPPORTING PROOF; formal preregistered scenario conformity INCOMPLETE; ADR 0002 accepted.** The worker direction stands, but the reader test was not exact hover over 10,000 product occurrences, crash tests did not exercise supervisor restart/reschedule, and cache plateau was synthetic accounting rather than integrated derived-memory behavior. Preserve the report and rerun conforming scenarios on the product supervisor before claiming Gate B GO. |
| C | **PARTIAL—PROOF ONLY.** Box interaction primitives and historical performance artifacts exist; terminal current report, integrated-GPU evidence, actual product viewport, and OCCT-backed path are incomplete. |
| C1a | **PLANNED, immediate.** Canonical-to-interaction authority/equivalence. |
| C1b | **PLANNED after exact app integration.** Exact topology versus interaction-reference equivalence. |
| D | **PLANNED after StateView/rules/validators.** Intent/Proposal task success; must not precede deterministic product substrate. |

## 12.3 Required CI baseline

CI MUST run on supported changes:

- formatting check;
- lint with warnings denied for project code;
- workspace tests and product binary build;
- localization key/fallback checks and pseudo-locale coverage;
- canonical core, persistence, interaction, app workflow, exact façade, scheduler, and relevant gate suites;
- no second model Undo authority or legacy application-owned model scene;
- sole public canonical mutation-entry guard, allowing only documented lifecycle exceptions;
- threshold/operation-envelope diff classification and anti-loosening review gate;
- frozen-input hash/provenance validation before formal gate execution;
- dependency/license policy and locked dependency audit;
- schema/golden StateView compatibility checks.

Each architectural guard must have a deliberate red test proving that CI can fail for the prohibited change.

## 12.4 Focused acceptance suites

### Core

- atomic candidate rollback;
- one-batch Undo/Redo and redo-tail truncation;
- typed ID allocation/non-reuse/remap;
- hierarchy/transform cycles and nested composition;
- sharing, group/component conversion, Make Unique mapping;
- rule DAG dirty propagation and actual evaluation;
- hierarchical `SlotPath` segment stability under nested sibling insertion/deletion/reordering and override resolution without silent retargeting;
- proposal read/write/dependency scope;
- M4a validator interface/diagnostic/policy/result behavior, including required `Unavailable`/`NotEvaluated` fail-closed cases and structural best-effort labeling; M7 separately tests package identity, trust, licensing, isolation, revocation, and egress;
- SAT-positive convex-intersection construction and complete containment in a certified `K_ε ⊆ J ⊕ B_ε`, rejecting permissive face-shift-only acceptance and avoiding double-counted coverage volume.

### Persistence

- all schemas and explicit loss reports;
- product/reference/rule/joint/body round trips;
- checksums, truncation, malicious sizes/depths, unknown namespaces;
- failed Open leaves current document unchanged;
- interrupted Save/migration recovery;
- backend/evaluator mismatch audit, semantics-preserving derived-result registration, and explicit transaction for any canonical semantic diff;
- zero canonical drift over repeated cycles.

### Exact and scheduler

- façade exception containment;
- operation validity and adversarial envelope, including upper coordinates;
- stable-reference corpora per operation family;
- deadline/cancel/crash/restart/reschedule;
- stale race permutations and cache plateau;
- full backend identity in request/result/cache acceptance.

### Interaction and renderer

- C1a projection authority;
- C1b resolver equivalence;
- shared geometry over 10,000 occurrences in the actual product path;
- overlapping candidates, snaps, inference, context filters, ambiguity;
- preview/cancel/commit and stale-display behavior;
- visual/accessibility tests against designed shell tokens.

### Product workflows

- current narrow manual capstone without hidden controls;
- persistent dimension and tag continuity;
- simple cut/union and exact/mesh export;
- Save/New/Open with identity/reference/validation continuity;
- minimal beam 6.3a at M4a-E immediately after M2, full 6.3a projections by full M4a exit, and beam 6.3b only after M3 exact integration;
- directly editable bottle slice with stretch/scale, flatten, bevel/fillet, shell/thickness, and only the additional curve operations its frozen case requires;
- one **non-blocking** directly editable living-tree research slice with stable trunk/branch paths and explicit exact/mesh authority; failure or deferral does not block the bottle, beam, exact-integration, or FLP exits;
- M4a built-in validator fixtures for the host-neutral interface and result taxonomy; representative open and proprietary/paid package fixtures move to M7 and must then have identical capability, provenance, privacy, and fail-closed rules;
- AI Proposal workflow only after deterministic acceptance criteria exist.

## 12.5 Manual evidence

Automated tests are necessary but not sufficient for:

- real GPU/display/input behavior;
- native file dialogs and packaging;
- worker/DLL deployment and restart;
- accessibility and localization expansion;
- user comprehension of action digests and ambiguity choices;
- manufacturing-domain review.

Manual runs are recorded with build fingerprint, hardware, steps, expected result, artifacts, and disposition. “Looked good” without provenance is not gate evidence.


# 13. Migration plan from the 2026-08-03 system

The sequence is dependency-driven. A later slice MUST NOT be used to conceal an unmet earlier invariant.

The order `M0 → durable M1 prerequisites → M2 → M4a-E early beam checkpoint → M3`, with the remaining M4a protocol/projection/evidence track completing immediately after M4a-E and before its own full exit, followed by `M4b/M5 → workflow-led M6 → M7`, changes the frozen V3 sequence, which expected an exact Gate C before the narrow manual modeler. It remains **proposed** until the dedicated V4-P15 ADR explicitly retires or narrows the old Gate C claim, preserves applicable failure consequences, classifies the change under anti-loosening rules, and gives C1a/C1b/M4a-E/full-M4a owners, entry criteria, thresholds, concurrency rules, and deadlines.

Until that ADR is accepted, the execution contract’s original sequence remains binding. Completed prototype work may be recorded as evidence but cannot be used to claim sequence compliance or retire exact Gate C. The architectural reason for M4a-E-before-M3 is informational: the earliest beam workflow 6.3a depends on M2 rules, hierarchical slots, collision, bounded joints, and only a grouped piece/length list, but intentionally excludes the full BOM/dimension projection contract, FurniGen import, third-party validator hosting, exact notches, product OCCT integration, durable subshape references, and C1b.

## M0 — Preserve and govern the current substrate

**Goal:** make current truth mechanically visible before expanding it.

1. create the minimal current source/provenance freeze required for a valid strengthened A0 and execute A0 immediately, before other M0 implementation work;
2. record whether failure is hash/provenance-only or a substantive topology/reference failure; a substantive failure halts M1/M2/M3 investment for an explicit planar-fallback or redesign disposition;
3. establish CI and deliberate-red guard tests;
4. freeze C1a canonical projection authority;
5. add complete and agent StateView v1 with golden fixtures;
6. document lifecycle exceptions to D-08;
7. classify R0 V1→V13 transitions and add upper-envelope adversarial cases.

**Exit:** A0 has a recorded evidence-scoped disposition and consequence, and a second authority, threshold loosen, stale frozen input, or StateView drift fails automatically. A hash-only A0 failure is not geometry evidence; a substantive NO-GO cannot be deferred behind later milestones.

## M1 — Complete the narrow manual modeler

**Goal:** preserve the useful narrow product while separating durable M2 prerequisites from polish that would harden a temporary cuboid proxy.

**Blocking durable-prerequisite track:** 

1. correct profile-versus-default-solid semantics;
2. eliminate every remaining legacy model/history authority;
3. complete only hierarchy and group-to-component semantics required by M2 migration;
4. ensure these contracts are representation-independent and preserve current Save/Open identity.

**Non-blocking product track:** canonical tags/tag visibility, remaining shortcuts/menu behavior, cuboid-capstone polish, proxy rendering refinements, and dimensions that target a stable canonical entity or `SlotPath`. A dimension requiring exact subshape identity waits for M3/C1b. No proxy-only anchor or interaction implementation may establish the durable reference contract or delay M2/M4a-E.

**Blocking exit:** representation-independent profile, authority, hierarchy, and migration prerequisites pass focused tests. **Separately reported product evidence:** the cuboid-proxy capstone passes through visible shell controls with identity and persistence continuity. Neither exit claims exact Gate C, exact-body product geometry, or FLP completion.

## M2 — Unify the canonical graph and persistence

**Goal:** replace parallel legacy/product semantics with one evaluator-ready model.

1. durable document/core ID allocation policy;
2. typed graph nodes, ports, parameters, expressions, and rule outputs;
3. actual evaluator and affected-only recomputation;
4. `(RootRuleNodeId, SlotPath)` hierarchical derived identity, segment-wise resolution, and override state;
5. per-node input/result digests and evaluator identity;
6. complete/agent StateView updates;
7. schema 3 candidate, manifest/checksums/limits, migration fixtures;
8. non-mutating Open audit; non-canonical derived-result registration for semantics-preserving recompute; and an explicit command batch only for semantic migration or recompute-generated canonical diffs.

**Exit:** changing one rule recomputes only dependent nodes, persists derivation identity, and survives Open without hidden mutation.

## M4a — Prismatic validators and first rule product

**Goal:** obtain the earliest product signal that the M2 evaluator and identity substrate serves real fabrication, then complete the reusable validation/projection contract without turning that first signal into a nine-item program.

**M4a-E early checkpoint — blocks the start of M3:**

1. freeze the versioned collision/contact tolerance policy;
2. run AABB/OBB/SAT over canonical prismatic convex coverage and construct every relevant `I_ij` with the non-permissive containment rule of §10.3;
3. create canonical bounded joints and detect undeclared overlap and empty declared joints;
4. resolve hierarchical `SlotPath` overrides and expose preserved/lost/ambiguous diagnostics;
5. run beam workflow 6.3a with a simple grouped list of piece identity, quantity, and length.

**M4a-E exit:** one governing value changes the beam pieces; joints and overrides remain stable or become explicitly unresolved; illegal overlap outside the certified joint acceptance region is caught; empty declared joints fail; and the grouped piece/length list regenerates. If this reveals that evaluator, slot, override, collision, or joint semantics do not serve the product, M2 is corrected before M3 begins.

**M4a completion track — starts immediately after M4a-E and does not delay its first observation:**

6. freeze the open host-neutral validator interface, diagnostic schema, policy model, result classes, and built-in `Unavailable`/`NotEvaluated` behavior; third-party hosting remains M7;
7. add the full BOM and stable entity/`SlotPath` dimension projection contract, including the reference dimension chain;
8. import the FurniGen regression corpus where available, or record the evidence gap explicitly;
9. rerun the beam over the completed built-in protocol and projections.

M4a MUST NOT depend on integrated OCCT results, durable `SubshapeRef`, C1b, or a third-party package host. Under the P15 ADR, M3 may begin only after the M4a-E result is disposed; the M4a completion track then proceeds immediately and may overlap M3, but must finish before M4b or any claim of full M4a exit.

**Full M4a exit:** M4a-E remains green; expected dimensions and joints remain stable or explicitly unresolved; built-in required validators cannot masquerade as passing when `Unavailable` or `NotEvaluated`; and the full BOM/dimension projections regenerate deterministically.

## M3 — Integrate exact evaluation

**Goal:** connect accepted ADR 0002 to the product path after the prismatic product substrate is proven.

1. complete backend/evaluator identity;
2. general feature request/result protocol;
3. worker deadline, health, restart, bounded retry, and reschedule;
4. canonical exact body specifications and accepted result registry;
5. product evaluation status/diagnostics;
6. derived meshes and render packages;
7. `SubshapeRef` in core and persistence;
8. C1b preregistration and execution.

**Exit:** rectangle/profile → exact extrusion → render/pick → stable reference → Save/Open is one integrated vertical slice, not separate crate proofs.

## M4b — Exact-dependent validators

**Goal:** extend the proven M4a validator/joint protocol only where accepted exact bodies or exact references are genuinely required.

1. exact-body collision and clearance representations with conservative failure behavior;
2. exact-subshape-aware dimensions and joint participants;
3. backend/tolerance-aware validator evidence and stale-result policy;
4. exact-dependent built-in standards, structural/statics, and manufacturability validators selected by frozen domain workflows; M7 may later host third-party implementations of the same contracts.

**Exit:** exact-dependent validation results bind accepted body/reference identities, fail closed when exact evidence is unavailable or stale, and remain projections rather than model mutation.

## M5 — Exact fabrication features

**Goal:** add exact openings/notches and complete 6.3b.

1. canonical cut/union/opening feature;
2. expanded exact operation and TNP corpora;
3. joint-driven notch derivation;
4. exact collision representation for notched prisms;
5. piece-drawing/manufacturing operation projection;
6. beam workflow 6.3b.

**Exit:** semantic half-lap joint produces consistent geometry and dimensions on both pieces, survives controlled parameter changes, and exports no invalid/quarantined result.

## M6 — Broaden geometry deliberately

**Goal:** move beyond boxes through the smallest coherent operation sets proven by concrete editable workflows, while preserving authority, identity, and interaction contracts.

Workflow slices, not a universal feature checklist, determine priority:

1. **Bottle slice:** primitive or validated profile; controlled stretch/scale and flattening; bevel/fillet; shell/thickness; optional revolve for rotational variants; controlled spline/loft/sweep only when the asymmetric acceptance case requires them.
2. **Living-tree slice — non-blocking research/acceptance:** semantic trunk/branch `SlotPath` structure; variable-radius sweep/loft or explicitly mesh-authoritative procedural geometry; direct branch/path/radius edits with local recomputation and explicit reference health. Its failure or deferral MUST NOT block bottle, beam, M6 core, or FLP exit.
3. **Domain-led additions:** profiles, constraints, booleans, imports, conversions, and exports only when a frozen furniture, building, fabrication, bottle, tree, or other approved workflow demonstrates their value.

Every slice repeats the canonical-command-evaluator-reference-interaction-persistence-validation chain and proves direct manipulation, meaningful numeric control, Undo/Redo, Save/Open, and authority/loss reporting. M6 does not promise complete CAD, unrestricted sculpting, animation, or production-scale vegetation generation.

## M7 — AI and extension surface

**Goal:** expose automation only after deterministic semantics exist.

1. intent vocabulary from proven workflows;
2. complete Proposal/read-write/risk/diff/budget path;
3. Assistant UI and verification;
4. gate D over canonical tasks;
5. Python SDK and constrained plugin pilot;
6. third-party validator hosting: discovery/install/update, authenticated publisher/signature provenance, trust and revocation, paid-license state, native/WASM process isolation, and confirmed remote egress;
7. later domain semantic packages, BIM primitives, and broader drawing/manufacturing integrations; the host-neutral validator protocol already exists from M4a.

**Exit:** AI can change rules and models only through the same safe, observable, undoable path as manual actions.

## Sequencing prohibitions

- Do not expand AI tool surface before manual and validator workflows pass.
- Do not claim exact product geometry before exact results enter the app.
- Do not begin product OCCT integration before M4a-E has tested M2 against the minimal beam workflow 6.3a and its result is disposed, unless the P15 ADR records a new dependency and consequence.
- Do not broaden `Guaranteed` references without preregistered evidence.
- Do not add a second scene/history/evaluator database for expedience.
- Do not make Open an implicit migration/recompute command.
- Do not use render tessellation as silent exact authority.
- Do not claim current-gate certification from historical artifacts after source inputs change.


# 14. Risks, review questions, and acceptance of V4

## 14.1 Principal risks

| Risk | Current exposure | Required control |
|---|---|---|
| Parallel semantic models | Legacy dependency nodes and product features are disconnected. | M2 unified graph; migration fixtures; delete superseded authority after transition. |
| False confidence from proxy geometry | Box interaction/render tests can appear to certify exact CAD. | Explicit evidence labels, C1a, then exact integration and C1b. |
| Topological misbinding | Narrow resolver proof not canonical or persisted. | Typed durable references, conservative outcomes, operation-specific corpora, audit/quarantine. |
| Native crash/hang | Worker proof lacks complete supervision in product. | ADR 0002 integration, deadlines, restart/reschedule, packaging tests. |
| Silent Open mutation | Future migration or semantic recompute could bypass D-08. | V4-P07 observational Open, non-canonical semantics-preserving result registration, and explicit command only for canonical semantic diffs. |
| Unstable derived identity | Rule outputs currently have no hierarchical semantic identity. | `(RootRuleNodeId, SlotPath)`, segment-wise override resolution, Lost/Ambiguous handling, and sibling-reordering fixtures. |
| Validator false negatives | Manufacturing, structural, regulatory, or advisory output may look plausible but be invalid or misclassified. | Versioned contract/implementation identity, convex-intersection/joint/domain corpora, class labels, `NotEvaluated`/`Unavailable`, fail-closed export policy, and explicit best-effort structural labeling with qualified-engineer approval outside Ketchup. |
| Tolerance/envelope drift | Historical versions lack explicit change direction. | CI classification and new upper-envelope evidence. |
| Unbounded input/work | File, expression, geometry, and AI can cause resource exhaustion. | Limits at every external boundary and worker isolation. |
| Scope explosion | General CAD, BIM, manufacturing, AI, and plugins compete. | Stage exits, non-goals, workflow-first operation additions. |
| File compatibility debt | Current schema lacks provenance and limits. | Schema 3 design before public promise; migration suite. |
| Validator supply-chain or certification overclaim | A built-in or later paid/open result could be mistaken for a safety guarantee, professional approval, or legal certification. | M4a result-class/envelope/best-effort semantics; qualified-engineer approval for structural guarantees; M7 signed/versioned package identity, least privilege, revocation, no license secrets in documents, and fail-closed availability semantics. |
| Human ownership gap | Project owner plus agents, limited independent review. | Explicit owners, external review, conservative claims, no silent decisions. |

## 14.2 Questions for external reviewers

1. Is the proposed unification of legacy nodes and product features sufficient to avoid a second source of truth while preserving rule-derived outputs?
2. Which derived-result families should be persisted as non-canonical, fully keyed evidence/cache, and what retention, display, and trust policy applies when the producing evaluator/backend is unavailable?
3. Does the hierarchical `(RootRuleNodeId, SlotPath)` contract preserve every nested output and override under sibling insertion/deletion/reordering, with precise segment-level `Lost/Ambiguous` diagnostics?
4. Does the proposed `SubshapeRef` evidence order avoid unsafe geometric-fingerprint rebinding?
5. Are C1a and C1b correctly separated and independently fail-able?
6. Are worker supervision, result acceptance, and cache identity complete enough for exact product integration?
7. Does observational Open plus keyed derived-result registration and explicit semantic migration preserve user intent without making retained old results misleading?
8. Does the collision corpus prove construction of every SAT-positive convex intersection and complete containment in a certified non-permissive `K_ε ⊆ J ⊕ B_ε`, without face-shift over-approximation, whole-pair exemption, or overlap-volume double counting?
9. Do the blocking bottle/fabrication workflows and explicitly non-blocking living-tree research slice select the smallest valuable geometry operations without turning M6 into a generic feature-completeness program?
10. Are security capabilities, budgets, and confirmation boundaries sufficient before an AI/plugin surface is introduced?
11. Which V3 gate claims should be retired, rerun, or narrowed in light of actual harness scope?
12. What minimum migration matrix is required before the first public `.ketchup` compatibility promise?

## 14.3 V4 acceptance criteria

This document is ready to become the authoritative successor to V3 only when:

- all as-built claims have been checked against the current source and tests;
- a named accountable project owner accepts the V4 target and the three merged dated decision records exist: the P01–P14 adoption ADR, the dedicated P07/P08 Open/derived-authority ADR, and the dedicated P15 sequence ADR; any remaining `OPEN` decision is explicitly non-blocking for ratification and has a named owner and stage or calendar deadline;
- no planned exact, evaluator, validator, AI, or geometry capability is described as current;
- external review dispositions are recorded in an appendix or linked review artifact;
- contradictions with accepted ADRs and the execution contract are resolved explicitly;
- code owners accept the migration sequence and named stage exits;
- evidence caveats for A0/A1/B/C are retained;
- the project owner confirms the product emphasis on rules, validation, fabrication outputs, and broad-but-declared geometry.

Until then, V4 is the most complete review draft but not a retroactive rewrite of frozen history.

## 14.4 Change control after acceptance

- Architectural changes require a dated ADR referencing the affected V4 decision IDs.
- Implementation status updates may change `PLANNED` → `PARTIAL—PROOF ONLY` or `PARTIAL—PRODUCT PATH` → `IMPLEMENTED` only with named evidence; proof-only promotion requires evidence on the intended product path.
- Gate evidence is appended/versioned, never rewritten.
- Public scope and compatibility promises require owner approval.
- A change that weakens a frozen operation envelope or threshold is labeled `loosen` and receives explicit review.
- The V4 document should be regenerated or revised when code ownership boundaries materially change, not for every local refactor.


# Appendix A — As-built ownership and evidence map

Line numbers are a snapshot aid for the 2026-08-03 working tree and may move. Tests and symbol names are the more durable references.

| Contract | Owning implementation | Focused evidence | V4 status |
|---|---|---|---|
| Typed product entities | `crates/ketchup-core/src/document.rs:10-255` | `tests/product_document.rs:19-203` | **PARTIAL—PRODUCT PATH** |
| Legacy dependency nodes | `document.rs:271-365`, `1668-1727` | `tests/gate_a1.rs:17-61` | **PARTIAL—PRODUCT PATH; to unify** |
| Command batch/digest | `document.rs:485-519` | `gate_a1.rs`, `product_document.rs` | **IMPLEMENTED narrow** |
| Atomic apply/rollback | `document.rs:784-1183` | `gate_a1.rs:64-106`; `product_document.rs:175-203` | **IMPLEMENTED** |
| Proposal commit | `document.rs:1202-1287` | `gate_a1.rs:109-155` | **PARTIAL—PROOF ONLY; legacy scope** |
| Undo/Redo | `document.rs:706-710`, `1186-1200` | `gate_a1.rs:64-106`; app headless tests | **IMPLEMENTED canonical** |
| Definition/occurrence sharing | `document.rs:104-204`, `600-627` | `product_document.rs:73-137` | **IMPLEMENTED narrow** |
| Group hierarchy/world transforms | `document.rs:158-234`, `629-657`, `1585-1617` | `product_document.rs:73-101`, `175-203` | **PARTIAL—PRODUCT PATH** |
| Make Unique | `document.rs:1451-1541` | `product_document.rs:103-137`; `headless_shell.rs:168-194` | **IMPLEMENTED current features** |
| Canonical digest | `document.rs:1729-1847` | round-trip/product tests | **IMPLEMENTED non-cryptographic** |
| Binary persistence | `crates/ketchup-core/src/persistence.rs:15-381` | `gate_a1.rs:157-295`; `product_document.rs:140-172` | **PARTIAL—PRODUCT PATH** |
| Atomic Save/Open | `persistence.rs:122-203`; `ketchup-app/src/lib.rs:630-741` | `ketchup-app/tests/file_workflow.rs:34-189` | **IMPLEMENTED narrow** |
| Designed shell | `ketchup-app/src/lib.rs:3247-3641` | `tests/headless_shell.rs:15-52` | **PARTIAL—PRODUCT PATH** |
| Command registry | `ketchup-app/src/lib.rs:126-381`, `1205-1305` | app command/shortcut tests | **PARTIAL—PRODUCT PATH** |
| Headless real-widget harness | `ketchup-app/tests/harness/mod.rs:29-278` | `capstone_chain.rs` and app integration tests | **IMPLEMENTED** |
| Selection/edit context | `ketchup-app/src/lib.rs:383-443`, `906-1101` | `lib.rs:4790-4834`; `capstone_chain.rs:118-130` | **PARTIAL—PRODUCT PATH** |
| Box interaction geometry | `ketchup-interaction/src/lib.rs:121-607` | `tests/gate_c_interaction.rs:22-240` | **IMPLEMENTED box-only** |
| App canonical-to-box projection | `ketchup-app/src/lib.rs:3705-3775`, `3073-3091` | capstone and selection tests | **PARTIAL—PRODUCT PATH; needs C1a** |
| Box renderer | `ketchup-app/src/lib.rs:2921-3019`, `3885-3936` | viewport tests | **PARTIAL—PRODUCT PATH** |
| Exact façade | `ketchup-exact/src/lib.rs:325-419`; `src/native.cc:209-279` | exact crate unit tests; historical A0 | **PARTIAL—PROOF ONLY; narrow operation set** |
| Narrow SubshapeRef proof | `ketchup-exact/src/lib.rs:220-260`, `561-665` | `tests/gate_a0.rs:165-255` | **PARTIAL—PROOF ONLY; isolated** |
| Scheduler stale rejection/cache | `ketchup-scheduler/src/lib.rs:41-199` | `tests/gate_b.rs` | **IMPLEMENTED proof** |
| Worker client/process proof | `ketchup-scheduler/src/lib.rs:222-360`; worker bin | `gate_b.rs`; ADR 0002 | **PARTIAL—PROOF ONLY; product wiring absent** |
| Localization | `locales/en-US.ftl`; app locale service | app shell/resource tests; ADR 0001 | **PARTIAL—PRODUCT PATH; FLP incomplete** |
| Exact/mesh canonical body | none in product core | none | **PLANNED** |
| Expressions/evaluator | none beyond source token/dependency closure | none calculating outputs | **PLANNED** |
| Persistent dimensions/tags | no complete canonical entities | none | **PLANNED** |
| Collision/joints | none | none | **PLANNED** |
| Assistant/plugin host | none | none | **PLANNED** |

# Appendix B — V3 canonical task disposition

| # | Task | 2026-08-03 disposition |
|---:|---|---|
| 1 | Exact rectangular profile | **PARTIAL—PRODUCT PATH:** canonical rectangle/profile and exact numeric UI; app immediately creates default-height solid and exact backend is disconnected. |
| 2 | Exact profile extrusion | **PARTIAL—PRODUCT PATH:** canonical height and isolated exact extrusion proof; no integrated exact product result. |
| 3 | Change source extrusion height | **IMPLEMENTED narrow:** current profile/extrusion model and Push/Pull path. |
| 4 | Rectangular opening/cut | **PLANNED product path:** isolated exact box cut only. |
| 5 | Unambiguous Push/Pull parameter change | **PARTIAL—PRODUCT PATH:** app edits first matching profile/extrusion with box provenance, not durable `SubshapeRef`. |
| 6 | Ambiguous Push/Pull choice | **PLANNED product behavior:** primitive ambiguity proof is not wired to app exact topology. |
| 7 | Exact vector Move | **IMPLEMENTED for occurrences.** |
| 8 | Snapped Copy | **PARTIAL—PRODUCT PATH:** Ctrl-Copy/shared occurrence works; complete inference/snap UI evidence is incomplete. |
| 9 | Shared definition and occurrences | **IMPLEMENTED narrow.** |
| 10 | Definition edit updates occurrences | **IMPLEMENTED narrow.** |
| 11 | Make occurrence unique | **IMPLEMENTED current feature vocabulary.** |
| 12 | Group and edit context | **PARTIAL—PRODUCT PATH:** supported hierarchy/context workflow, incomplete conversion/nested semantics. |
| 13 | Tag assignment/visibility | **PLANNED canonical tags; occurrence visibility only is implemented.** |
| 14 | Linear pattern | **PLANNED.** |
| 15 | Parameter expression | **PLANNED:** source token may contain expression text but no evaluator exists. |
| 16 | Dependent-only recompute | **PARTIAL—PROOF ONLY; bookkeeping:** dependency closure exists; values/product outputs are not evaluated. |
| 17 | Save/Open dimensions/references | **PARTIAL—PRODUCT PATH:** current product IDs/parameters/hierarchy round-trip; persistent dimensions/SubshapeRef/provenance absent. |
| 18 | Batch Undo/Redo one step | **IMPLEMENTED canonical state.** |
| 19 | Exact or mesh export with loss report | **PLANNED product path.** |
| 20 | Risky AI Proposal workflow | **PLANNED:** narrow proposal primitive only, no Assistant/product pipeline. |

# Appendix C — Change log from V3

## Retained without semantic change

- product segment and manual-first principle;
- Rust core, OCCT façade, wgpu baseline;
- immutable revisions and one-writer/many-readers;
- one canonical command path and one batch per Undo step;
- exact/mesh distinction;
- explicit topological-reference failure;
- Intent/Proposal/Command/Verification separation;
- worker result identity and stale rejection;
- native container direction and no early compatibility promise;
- R0/A0/A1/B/C preregistration methodology;
- deferred full BIM/drawing/mechanical/nature/plugin marketplace/browser/collaboration scope.

## Clarified from implementation evidence

- current app is a canonical cuboid modeler, not a general exact CAD app;
- interaction geometry is a box proxy rather than a second full analytic B-Rep;
- exact and scheduler proofs are disconnected from product application;
- `apply_batch` is the ordinary sole edit path, with lifecycle exceptions named;
- current “recomputed nodes” is affected-set bookkeeping, not an evaluator;
- schema 2 stores both legacy nodes and product entities but lacks derivation identity;
- historical gate evidence is scoped to frozen source/harness, not automatically current certification;
- Gate C proxy/harness scope is narrower than the product claim previously implied.

## Added in V4

- explicit **IMPLEMENTED / PARTIAL—PRODUCT PATH / PARTIAL—PROOF ONLY / PLANNED** classification;
- unified rule/feature DAG proposal;
- stable hierarchical derived identity `(RootRuleNodeId, SlotPath)` and segment-wise override semantics;
- per-node digest including evaluator/backend identity;
- observational Open, non-canonical semantics-preserving result registration, and explicit command only for semantic migration/canonical diffs;
- two StateView projections from one encoder;
- C1a authority gate and later C1b resolver-equivalence gate;
- deterministic collision/SAT/convex-coverage architecture plus explicit convex-intersection containment inside bounded joint volumes;
- bounded canonical declared joints and empty-joint error;
- beam 6.3a/6.3b workflows, BOM/dimension/manufacturing projections;
- workflow-led editable bottle and living-tree geometry slices rather than generic operation-family completeness;
- open host-neutral validator protocol in M4a, with open-source/proprietary/paid third-party hosting deferred to M7 and structural/statics results explicitly best effort;
- exact current gate caveats and CI anti-loosening governance;
- staged migration with strengthened A0 first, M2 → M4a-E → M3 sequencing, and a non-delaying full-M4a completion track before M4b.

# Appendix D — Review disposition log

This maintained log records every material external-review finding and its disposition. A review is not “incorporated” merely by producing comments; accepted findings must change every affected normative section, while rejected or deferred findings require a stated reason. `Accept` in this log is an editorial review disposition only: it does not record accountable-owner acceptance, merge an ADR, ratify a proposed decision, or change the document’s NO-GO status.

| Review ID | Reviewer/date | Finding | Disposition (`accept`, `accept with change`, `reject`, `defer`) | Resulting decision/section/ADR |
|---|---|---|---|---|
| R1 | Independent architecture code-review agent / 2026-08-03 | Persisted accepted output risked becoming a second authority. | **Accept with change** | P08 revised; §§6.3 and 9.6 define non-canonical derived-result registry. Storage details remain O01/ADR. |
| R2 | Same | Open, semantic migration, and recompute lacked a safe transaction protocol. | **Accept** | §§9.5–9.7 rewritten: decode vs migration, two-phase evaluation, CAS acceptance, explicit semantic command. |
| R3 | Same | M0–M3 silently changed frozen Gate C sequence. | **Accept** | P15 and §13 require dated sequence ADR; M1 exit explicitly remains cuboid proxy, not exact Gate C. |
| R4 | Same | Gate B “GO” overclaimed reader/crash/cache harness fidelity. | **Accept** | §12.2 narrowed to decision-supporting proof; integrated conforming rerun required. Historical artifact unchanged. |
| R5 | Same | A0 overclaimed complete topology evidence and real backend migration. | **Accept** | §12.2 narrowed to reported GO with oracle/evidence gaps; stronger preregistration required. Historical artifact unchanged. |
| R6 | Same | Target schema lacked reusable nested assemblies. | **Accept** | §7.3 adds definition-local content, child prototypes, instance paths, scope, cycles, clone/remap, and persistence rules. |
| R7 | Same | `SubshapeRef` lacked occurrence/instance-path identity and mandatory evidence. | **Accept** | §8.4 splits `BodySubshapeRef` from `AssemblySelectionTarget` and specifies required evidence/instance-loss behavior. |
| R8 | Same | “Canonical polygon” overstated current profile validation. | **Accept** | §§2.3 and 8.3 now state current limits and target closure/winding/self-intersection/hole/envelope rules. |
| R9 | Same | Exact-result bounds were incorrectly described as exact; TNP normalization incomplete. | **Accept** | §§8.2 and 8.4 specify conservative bounds, gap/error direction, orientation/location, quantization, deterministic ordering. |
| R10 | Same | Proposed MUSTs lacked owners, approver, evidence, deadlines, and ADR requirements. | **Accept with limitation** | §3.5 adds role governance; named people/calendar dates remain an explicit unresolved release-governance gap. |
| R11 | Same | AI confirmation/principal/egress boundaries were too abstract. | **Accept** | §11.4 adds principals, human-only classes, digest-bound expiring confirmation, taint/read/egress/audit policy, fail-closed tests. |
| R12 | Same | Architecture diagram incorrectly forced every manual command through Proposal. | **Accept** | §6.1 now shows direct manual adapters and Proposal-based high-level path converging on one gateway. |
| R13 | Same | Current evidence was not bound to immutable working-tree provenance. | **Accept** | Appendix G records the audit-scope hash, HEAD, dirty state, toolchain, commands, outcomes, and document hash. |
| R14 | Same | Target was source-independent, but as-built verification package was incomplete. | **Accept** | Appendix F/final review claim narrowed; ratification still requires StateView/fixture/traces package. |
| R15 | Second independent review / 2026-08-03 | Residual wording still made every recompute canonical. | **Accept** | §§1.5, 3.3, 7.4, 9.6, 13, 14 and Appendix C aligned: semantics-preserving evaluation is non-canonical; only semantic diffs use a batch. |
| R16 | Same | Lossy Open could still activate an editable candidate. | **Accept** | §9.5 now requires separate read-only review candidate and leaves active editable document unchanged. |
| R17 | Same | Nested instance-path segment identity was unscoped. | **Accept** | §7.3 uses global root occurrence plus `(owning DefinitionId, DefinitionLocalOccurrenceKey)` segments and complete path mappings. |
| R18 | Same | Sequence ADR deadline was too late. | **Accept** | §3.5/§13 require ADR before any sequence-dependent M1 approval/claim and no later than M0 exit. |
| R19 | Same | Ratification could retain unaccepted proposed MUSTs. | **Accept** | §14.3 now requires named human acceptance and merged ADR for every target/M0–M3 P decision; blocking OPENs cannot remain. |
| R20 | Same | Confirmation token did not distinguish requester from human approver. | **Accept** | §11.4 binds both principals and approval capability through a trusted confirmation surface. |
| R21 | Same | Evidence hash omitted C++/locales/scripts/corpora/gate artifacts. | **Accept with limitation** | Appendix G expanded from 41 to 271 files across all cited source/evidence classes. Full ratification package and named evidence artifact remain required. |
| R22 | External human architecture reviewer / 2026-08-03 | Beam 6.3a does not depend on M3, so first product measurement was needlessly behind OCCT integration. | **Accept, refined by R32** | P15 and §13 split M4a from exact-dependent M4b; R32 further narrows the pre-M3 requirement to M2 → M4a-E → M3. |
| R23 | Same | M1 mixed durable prerequisites with polish over a proxy that M2/M3 will replace. | **Accept** | §13 splits blocking representation-independent M1 prerequisites from non-blocking shell/proxy work; §7.7 prevents proxy anchors from defining durable references. |
| R24 | Same | Per-decision ADR and named-role ratification would leave V4 permanently proposed. | **Accept** | §§3.5 and 14.3 require one adoption ADR plus dedicated P07/P08 and P15 ADRs. |
| R25 | Same | Current A0 failure must be resolved before later investment because a substantive failure may force planar fallback. | **Accept** | M0 now begins with a current freeze and strengthened A0; substantive NO-GO requires explicit fallback/redesign disposition. |
| R26 | Same | Flat `(NodeId, SlotKey)` cannot preserve nested-rule identity. | **Accept** | P05 and §§6–10/12–14 use hierarchical `(RootRuleNodeId, SlotPath)` with segment-wise resolution and no silent reindexing. |
| R27 | Same | SAT cannot determine whether the penetration region lies inside a bounded joint volume. | **Accept, refined by R33** | P10 and §10 require convex intersection construction for every penetrating component pair; R33 constrains final acceptance to certified `K_ε ⊆ J ⊕ B_ε`. |
| R28 | Same | M6 was a general-CAD feature program rather than a sequence selected by valuable workflows. | **Accept, refined by R34** | §§1.6, 2.3, 8.3, 12.4, and M6 use workflow-led operation priority; R34 makes the living-tree research slice explicitly non-blocking. |
| R29 | Same | `PARTIAL` conflated a narrow product path with disconnected proofs. | **Accept** | §§0.1, 1.2, 4, 5, 12, and Appendices A–C distinguish `PARTIAL—PRODUCT PATH` from `PARTIAL—PROOF ONLY`. |
| R30 | Project-owner product requirement / 2026-08-03 | Core must support extensible domain validators, including open and paid standards, spacing, structural/statics, manufacturability, and feng-shui packages. | **Accept with staged hosting** | P14 and §§2.2, 6.2, 10.1, 11.5, 12.4, and M4a define the open host-neutral interface/policy/result contract; M7 owns third-party package distribution, trust, licensing, isolation, revocation, and egress. |
| R31 | External human architecture reviewer / 2026-08-03 | R30 accidentally pulled costly plugin hosting, signatures, revocation, licensing, native isolation, and remote egress into M4a. | **Accept** | P14 and §§2.2, 4, 6.2, 7.1, 9.2, 10.1, 11.5, 12.4, and M4a/M7 separate the M4a host-neutral contract from M7 third-party hosting. |
| R32 | Same | Nine-item M4a delayed the early product signal that justified placing it before M3. | **Accept** | P15, §§10.5 and 13 define M4a-E: tolerance → collision/containment → bounded joints → `SlotPath` override → beam with grouped piece/length list. Full projections, protocol evidence, and FurniGen follow without blocking the first observation. |
| R33 | Same | Independent face offset by `ε` is a permissive superset of the Euclidean Minkowski inflation near edges/corners and can hide illegal overlap. | **Accept** | §10.3 requires a certified acceptance region `K_ε ⊆ J ⊕ B_ε`; face-shift over-approximation cannot be the final acceptance test. |
| R34 | Same | The living-tree slice is harder than the bottle and unrelated to the first fabrication path, so it must not block product exits. | **Accept** | §§1.6, 2.3, 12.4, 13, and 14.2 mark it as a non-blocking research/acceptance slice. |
| R35 | Same plus project-owner clarification | Structural/statics packages create certification/liability ambiguity; the owner retains statics but rejects any claim of 100% certainty. | **Accept with clarification** | P14 and §§10.1 and 14.1 permit transparent best-effort structural/statics results but prohibit Ketchup safety/code guarantees; professional approval and any structural guarantee require independent review and signature by a qualified structural engineer; jurisdictional approval is supplementary. |

**Review verdict:** `NO-GO for ratification`, retained after four review rounds. The latest material architectural findings have accepted dispositions in this draft. Ratification now requires the three decision records defined by §3.5, a named accountable project owner, disposition of blocking OPEN decisions, and the StateView/fixture/trace evidence package; it no longer requires one ADR per P decision. Naming the owner is a single signature/acceptance action, but this document does not invent the person or record acceptance that has not been explicitly supplied.

# Appendix E — Document source set

Primary normative and historical sources used for this draft:

- `KETCHUP_ARCHITECTURE_PROPOSAL_V3.md`
- `docs/design/EXECUTION_CONTRACT.md`
- `docs/design/README.md`
- `docs/design/IMPLEMENTATION_PLAN.md`
- `docs/adr/0001-project-language-and-localization.md`
- `docs/adr/0002-exact-backend-isolation.md`
- `docs/adr/0003-splash-screen-and-version-display.md`
- `R0_LICENSE_AND_TOOLCHAIN_BASELINE.md`
- `thresholds/r0.yaml` and R0/gate artifacts
- `docs/ketchup-zadanie-2026-08-03.md`
- `docs/ketchup-odpoved-2026-08-03-b.md`
- `docs/ketchup-odpoved-2026-08-03-c.md`
- current source and tests in `ketchup-core`, `ketchup-app`, `ketchup-interaction`, `ketchup-exact`, and `ketchup-scheduler`.

# Appendix F — External-review package checklist

Before sending this draft for review, include:

- this V4 document;
- V3 and the frozen execution contract;
- accepted ADRs;
- current crate dependency graph;
- complete StateView example when M0 provides it;
- current test/gate disposition with exact build fingerprint;
- one current `.ketchup` schema-2 fixture and decoded semantic inventory;
- one narrow manual capstone trace;
- one A0 reference fixture and one Gate B worker trace;
- explicit questions from section 14.2.

The **target architecture** is intended to be understandable without reading source. Independent verification of the **as-built** claims still requires source until the complete StateView, decoded fixture, dependency/API map, and execution traces above are attached. Ratification cannot rely on this narrative alone.

# Appendix G — Current evidence manifest

This manifest binds the as-built audit to a reproducible **scope**, not to the repository’s stale clean `HEAD` alone.

| Field | Value |
|---|---|
| Audit timestamp | 2026-08-03T17:31:35Z |
| Git HEAD | `f0a0cf3afa9df45682fe8723dacc99cb8e153058` |
| Working tree | `dirty=true`; V4 explicitly audits the current working tree |
| OS | Windows 10 10.0.19045 x64 |
| Rust | `rustc 1.97.0 (2d8144b78 2026-07-07)` |
| Cargo | `cargo 1.97.0 (c980f4866 2026-06-30)` |
| Cargo.lock SHA-256 | `eceb482fd1aa9e9f6bcdc644431e03d1e516f2efa787355de7f0767744ffe852` |
| Audit scope | 271 files across Cargo/toolchain/threshold inputs, corpora, Rust/C++ exact source, crate manifests, locales, design/ADR/R0 reports, Windows gate scripts, and cited R0/A0/A1/B/C artifacts/traces |
| Audit-scope SHA-256 | `6b9910271c899bff8b849722fb68b9bbf1c376748db67bf00e7eb7dee96f7e86` |
| V4 body + Appendices A–F prefix bytes | `151835` bytes, ending immediately before Appendix G |
| V4 body + Appendices A–F prefix SHA-256 | `7c4c02987e9067c8be572795bbc1325e2ecfa4cb0ad390d350a9d6cec7013ede` |

The audit-scope hash is computed by sorting normalized repository-relative paths, then hashing `path + NUL + file bytes + NUL` for each included file. The included patterns are: root Cargo/toolchain/threshold files; `thresholds/**/*`; `corpora/**/*`; `crates/**/*.rs|cc|h|hpp` plus crate manifests; `locales/**/*`; design/ADR/R0 reports; `scripts/windows/*.ps1`; all files under `artifacts/r0`, `artifacts/gate-a0`, `gate-a1`, `gate-b`, and `gate-c`; V3; and the R0 baseline. It excludes `.git`, `target`, and bundled third-party OCCT source. The deterministic pattern list plus count/hash makes scope drift visible, but this dirty-tree manifest is still not a clean release identity.

Current validation observations used by this draft:

| Command | Outcome |
|---|---|
| `cargo test -p ketchup-core` | PASS; 13 tests across current core suites |
| `cargo test -p ketchup-app -p ketchup-interaction` | PASS; 63 tests across current app/interaction suites |
| `cargo test -p ketchup-exact --lib` | PASS; 6 tests |
| `cargo test -p ketchup-core --test gate_a1` | PASS; 7 tests |
| `cargo test -p ketchup-interaction --test gate_c_interaction` | PASS; 9 tests |
| `cargo test -p ketchup-scheduler --test gate_b --no-run` | PASS compile-only |
| `cargo test -p ketchup-exact --test gate_a0 -- --nocapture` | FAIL CLOSED before geometry observation because r0-v13 frozen build-input hash differs from current tree |

Test counts are observations, not permanent architecture requirements. The precise claim boundaries remain those in §5.8, §12.2, and Appendix A.

---

# Final statement

Ketchup has crossed the line from a collection of pure experiments into a narrow, coherent manual-modeler substrate, but it has not yet crossed the line into an integrated exact, rule-driven, general 2D/3D product. V4 preserves the successful core decisions, names the disconnected proofs, and makes the integration contract explicit.

The next architectural success is not another broad feature demo. It is proving that one canonical rule/feature model can drive exact evaluation, interaction, rendering, persistence, validation, and fabrication projections without creating a second authority or silently changing meaning during Open, recompute, or AI automation.

