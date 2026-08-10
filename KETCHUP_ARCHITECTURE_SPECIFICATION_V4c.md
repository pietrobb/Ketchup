# Ketchup Architecture Specification V4

## Current system, target architecture, and migration contract

- **Status:** Proposed for external architecture review
- **Snapshot date:** 2026-08-08 (as-built scope includes the immutable Appendix G baseline plus the explicitly bounded M11–M14 working-tree implementation described here; this is not a new certification freeze)
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
| **CURRENT EVIDENCE** | A test or inspection run against the immutable current-evidence commit named in Appendix G and its recorded date. All other sections refer to that single manifest entry rather than repeating a commit ID. |

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

As of the immutable current-evidence commit recorded in Appendix G, Ketchup is **not yet one integrated general CAD system**, but its bounded product paths share one canonical document, one `apply_batch` mutation authority, and one deterministic exact-result acceptance boundary:

1. **Canonical manual modeler and evaluator substrate — PARTIAL—PRODUCT PATH.** Immutable revisions, atomic command batches, definitions, occurrences, definition-local hierarchy, groups/components, Move/Copy, Group/Ungroup, Make Unique, edit context, Push/Pull, Undo/Redo, and atomic Save/Open are joined by a typed evaluator graph, bounded expressions, affected-only recomputation, hierarchical `SlotPath` identity, overrides, StateView projections, and schema-17 persistence. Rule outputs may bind to typed feature parameters; recompute is an explicit identity-bound canonical command, while Open remains observational and reports stale provenance without recomputing.
2. **Prismatic fabrication and validation — PARTIAL—PRODUCT PATH.** The beam workflow owns deterministic collision/containment, declared joints, the frozen `415 × 6`, `408 × 5`, `400` placement fixture, grouped pieces, full BOM, stable dimension sheets, a host-neutral built-in validator contract, resource limits, FurniGen regression evidence, W4A `Exact`/`Tolerant` reporting, worker-evaluated half-lap bodies, durable notch references, piece drawings, manufacturing operations, and fail-closed export. This is a bounded prismatic/beam slice, not a general fabrication service.
3. **Integrated exact product geometry — PARTIAL—PRODUCT PATH.** Rectangle extrusion, Boolean Cut/Union, the rotational bottle chain, and beam-piece exact bodies use shared feature-chain requests, `ExactBodyPackage` views, deterministic `ExactResultRegistry` keys, revision/digest/evaluator/backend/tolerance freshness checks, render/pick projection, and explicit-loss mesh export. The running application can accept multiple current exact bodies and execute the bounded groove/cut workflow without a special second authority.
4. **AI and extension surface — PARTIAL—PRODUCT PATH.** A bounded local Assistant, external Python plugin protocol, and signed no-import WASM validator host exist behind capability, Proposal, budget, trust, revocation, licensing, and fail-closed availability boundaries. The core additionally enforces signed, expiry-bounded, one-use human-only confirmation for explicitly scoped high-risk canonical Proposals and non-canonical side-effect Proposals; the desktop application applies the latter to existing-file overwrite Save, active-document or beam-workspace lossy OBJ exports, and Beam manufacturing release with payload-bound evidence and an authorization receipt before disk write, while host-mediated validator egress now requires an exact destination/provider/payload-bound external-disclosure receipt before connection. Natural-language/provider integration, production catalog/TLS egress, and credential brokerage remain incomplete.

Cuboid projection remains a derived fallback only for entities without a current exact package. Exact packages are likewise derived and revision/digest-bound; neither representation is canonical authority. The implementation still lacks broad profile/constraint/manual-operation coverage, general mesh import/procedural authoring and mesh-to-exact conversion, general topology/reference failure coverage, complete producer wiring and resource-plateau evidence, general fabrication projections, complete production AI/plugin hosting, and current release certification.

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

The highest-risk remaining gaps are:

1. evaluator outputs can bind deterministically to typed feature parameters, but coverage is limited to the current parameter slots and explicit recompute; the complete profile/sketch/constraint and universal feature-result vocabulary is not implemented;
2. exact rectangle extrusion, Boolean Cut/Union, the rotational bottle chain, and beam half-laps share one exact package/registry path, but broad operation families, arbitrary curved workflows, mesh import/procedural/render/query support, and mesh-to-exact conversion remain incomplete;
3. C1b covers bounded rectangle, boolean, bottle, and beam roles, and focused current-tree evidence distinguishes `Resolved`, `Ambiguous`, `Lost`, and `Quarantined`; broader transforms, mutations, arbitrary edge operations, and interaction-wide health propagation remain future work;
4. schema 17 provides checksums, limits, identity envelopes, graph/override/joint/reference/parameter-binding/persistent-dimension/tag/collection/mesh-body/space/clearance round-trip, freshness and reference-health projection, and review-only Open, but not the complete long-term container/blob/unknown-extension/migration/compatibility contract;
5. profile-only Rectangle and the canonical closed-polyline application path are implemented with atomic validation; axis-aligned rectangles have persisted deterministic width/height constraints; bounded persistent associative dimensions target stable feature parameters, complete `SlotPath` identities, or exact semantic references; canonical tags own persisted assignment and effective visibility; canonical collections provide non-owning persistent occurrence membership; and exact occurrence Align plus a bounded axis linear pattern have snapshot-bound preview, one-batch commit/Undo/Redo, and shared-definition preservation. The bounded viewport path now has explicit deterministic endpoint/intersection/midpoint/face scoring, acquire/release hysteresis, stable hover lock, Tab overlap cycling, exact-backed proxy inference, visible snap/readout feedback, and snapped Measure value-box evidence. Broader sketch constraints, arbitrary-point durable annotation authoring, and a complete Line/polyline viewport UX remain incomplete or absent;
6. validation/fabrication is strong for the bounded prismatic/beam path and selected current exact bodies, but collision, clearance, dimensions, drawings, BOM, and manufacturing are not yet general services over arbitrary documents;
7. the bounded Assistant, Python plugin, signed WASM validator, and core human-only high-risk confirmation paths exist, but broader intent/task coverage, trusted application confirmation UI, provider/egress policy, production package catalog, TLS, and credential brokerage remain open;
8. historical A0 v2 `FULL_GO` and other frozen evidence do not certify the current-evidence commit named in Appendix G; Gate C still lacks terminal integrated-GPU evidence;
9. architecture guards now enforce D-08/P07 write authority inside `ketchup-core`, but full release governance still depends on CI execution and current hashes/records;
10. V4 remains unratified under §14.3: ADR 0004 covers P15 and ADR 0006 records the immediate P07 implementation consequence, but the adoption ADR and broader dedicated P07/P08 ratification record remain outstanding.

## 1.5 Target-state statement

The target architecture has one revisioned canonical document and one validated mutation gateway. The document stores semantic inputs, rules, feature specifications, stable identity, explicit exact/mesh body specifications, reference state, declared joints, validation policy, and provenance. Evaluation produces replaceable derived results through a supervised exact worker and other deterministic services. Interaction, rendering, full state dumps, agent views, BOMs, drawings, and manufacturing exports are projections from the same snapshot; none is a second authority.

A user gesture or accepted AI Proposal commits exactly one validated `CanonicalCommandBatch`. Open audits and reports without recomputing, migrating, or rewriting canonical state. Evaluation or recomputation that preserves canonical semantics may register only a fully verified, revision/digest/envelope-bound derived-result event under §9.6; it creates no canonical revision or Undo step. Any migration or recomputation that changes canonical meaning produces a separate authoritative canonical diff and requires explicit confirmation and exactly one command-batch revision.

## 1.6 Can Ketchup model a ketchup bottle?

**Today: yes, for one bounded rotational workflow.** The desktop path creates a validated six-point half-profile, applies controlled body-radius scaling, body-height stretching, and shoulder flattening, then evaluates Revolve → Shell/thickness → Fillet or Chamfer through the supervised OCCT worker. Numeric edits and durable-role viewport drags commit one validated batch, remain Undoable, round-trip losslessly through schema 16, render and pick only a current accepted exact result, and export worker-produced ISO 10303 STEP plus deterministic OBJ with explicit editability/topology/tolerance loss. Stale or invalid results fail closed before export.

**Boundary:** this is not a general bottle or freeform CAD claim. The implemented case is rotational and intentionally does not require spline, Loft, or Sweep; asymmetric squeezed bottles and broader profile/edge-selection semantics remain planned. The acceptance principle remains workflow-led: candidate operation families are selected only when a frozen case requires them, not to complete a generic checklist.

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
| Rectangle profile | **IMPLEMENTED narrow profile path** | Core preserves an ordered finite single-loop profile and atomically rejects insufficient/duplicate closure, clockwise winding, self-intersection, tolerance-degenerate edges and coordinate-envelope violations. Rectangle creates only Profile + Occurrence; the same app path accepts exact closed-polylines, flat projection remains selectable, and first Z Push/Pull adds Extrusion in a separate canonical batch. Axis-aligned rectangles additionally support canonical evaluator-bound width/height constraints with deterministic anchored resize, dependent-only recompute, schema-16 persistence, Undo/Redo, and atomic rollback. Hole-bearing, multi-loop, non-rectangular constraint, and general solver vocabulary remain open. |
| Linear extrusion | **PARTIAL—PRODUCT PATH** | Rectangle/profile extrusion is evaluated as exact B-Rep through the supervised product worker, rendered/picked, and bound to durable reference evidence; broader profiles, directions, transforms, and result vocabulary remain incomplete. |
| Cut/union/opening | **PARTIAL—PRODUCT PATH** | Canonical bounded Boolean Cut/Union uses the shared exact feature/result path; rectangular ThroughCut additionally proves hole-safe render/pick, durable roles, and Save/Open. Arbitrary openings, broader operands, and general boolean contracts remain incomplete. |
| Revolve | **IMPLEMENTED narrow bottle product path** | A validated six-point radial half-profile produces a deterministic exact rotational body through the supervised worker; arbitrary axes, general profiles, and other turned products remain outside the envelope. |
| Sweep | **PLANNED** | Profiles along paths, trims, pipes, rails, and selected fabrication operations. |
| Loft | **PLANNED** | Transitions and asymmetric products, including non-rotational bottle bodies. |
| Shell/thickness | **IMPLEMENTED narrow bottle product path** | A bounded positive thickness creates one valid open-mouth bottle solid, preserves nine semantic outer/rim/inner roles, and fails closed outside the declared radius envelope. |
| Fillet/chamfer | **IMPLEMENTED narrow bottle product path** | Fillet or chamfer finishes the bounded shell shoulder under an amount envelope while preserving the nine shell-role lineages; arbitrary edge selection remains planned. |
| Spline/NURBS sketch and surface inputs | **PLANNED** | Required for controlled freeform exact geometry; not an organic sculpting system. |
| Mesh-authoritative bodies | **IMPLEMENTED narrow canonical path** | A validated closed, consistently oriented manifold `MeshBody` can be the sole geometry authority of a detached definition; imports, procedural authoring, rendering, and mesh queries remain planned. |
| Exact-to-mesh / mesh-to-exact conversion | **PARTIAL—PRODUCT PATH** | A current accepted exact package can create one detached canonical mesh definition through exactly one batch with source/destination/tolerance provenance, unsupported-semantics reporting and exact-reference `Lost` consequences; deterministic OBJ export remains available, while mesh-to-exact is planned. |
| 2D associative drawings | **PARTIAL—PRODUCT PATH** | Stable entity/`SlotPath` dimension sheets exist for the bounded beam workflow, and revision-bound exact parallel-face dimension projections carry durable body/face identities; persistent exact dimensions, general views, annotations, layouts, and professional drawing production remain later. |
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
| ADR 0002 | **RATIFIED, implementation PARTIAL—PRODUCT PATH** | Persistent exact worker is the production default. Parent owns canonical state; stale results require revision, generation, and digest match. The bounded product supervisor is integrated; broader job coverage and a fully conforming current Gate B rerun remain incomplete. |
| ADR 0003 | **RATIFIED** | Splash/version is presentation-only and dynamically sourced; it has no canonical effect. |
| ADR 0004 | **RATIFIED for V4-P15; amended after A0 v2** | Accepts the replacement sequence/gate charter. Its temporary L-01/L-02 loosens were withdrawn after A0 v2 `FULL_GO`; the accepted P15 ordering remains binding. |
| ADR 0005 | **RATIFIED governance control** | A gate NO-GO applies its safe halt immediately but cannot authorize redesign, fallback activation, or envelope/threshold loosening before cause localization and a separate accepted disposition. |
| ADR 0006 | **RATIFIED implementation consequence; broader V4-P07/P08 decision still OPEN** | Enforces exactly one canonical `apply_batch` path plus the envelope-checked non-canonical derived-result gateway. It explicitly is not the dedicated P07/P08 ratification record required below. |
| ADR 0007 | **RATIFIED for V4-O08** | Selects Windows x86-64 as the only first-release platform; other desktop platforms are deferred and must not delay end-to-end Windows product proof. |

## 3.3 Post-V3 decision disposition

Implementation evidence does not itself ratify a proposal. The register therefore preserves P01–P14 as proposals, records ADR 0006's deliberately narrow accepted consequences for P07/P08, and marks only P15 as ratified by its dedicated owner-approved ADR:

| ID | Status | Proposed decision |
|---|---|---|
| V4-P01 | **PROPOSED** | Unify the legacy `CanonicalNode` graph and product `Feature` graph into one canonical evaluator graph; no parallel rule/document truth. |
| V4-P02 | **PROPOSED** | Exact or mesh body specifications are canonical; interaction and render scenes are disposable snapshot-bound projections. |
| V4-P03 | **PROPOSED** | Freeze C1a now: every interaction occurrence originates from canonical projection and cannot become an independent model authority. |
| V4-P04 | **PROPOSED** | Introduce C1b only after exact bodies enter the app: exact topology resolution and interaction selection must produce equivalent stable references over a preregistered corpus. |
| V4-P05 | **PROPOSED, REVISED AFTER REVIEW** | Rules live in the canonical graph. Nested outputs carry stable provenance `(RootRuleNodeId, SlotPath)`, where every semantic path segment is minted by the producing rule level; resolution is segment-wise and never silently reindexes or retargets an override. |
| V4-P06 | **PROPOSED** | Every evaluable node has an input/Merkle digest including evaluator identity, backend identity where relevant, schema, tolerance profile, and dependent result fingerprints. |
| V4-P07 | **PROPOSED baseline; ADR 0006 ratifies only the implemented write-path consequence** | Open audits without recomputing or rewriting canonical state. Semantics-preserving evaluation registers a non-canonical derived-result event; only a migration/recompute that changes canonical meaning requires one explicit confirmed command-batch revision. Long-term retention, trust, and compatibility still require the dedicated P07/P08 owner decision. |
| V4-P08 | **PROPOSED baseline; ADR 0006 ratifies only the implemented authority boundary** | Canonical semantic specifications remain the sole model authority. Derived results may be persisted only as revision/digest/envelope-bound evidence or cache; they are excluded from the canonical model digest and never substitute for missing canonical meaning. Retention across evaluator/backend changes remains an explicit owner decision. |
| V4-P09 | **PROPOSED** | `StateView` has one shared deterministic encoder and two separately versioned projections: complete canonical dump and summarized agent view. |
| V4-P10 | **PROPOSED, REVISED AFTER REVIEW** | Collision validation uses broad-phase AABB, optional OBB/convex filtering, deterministic `f64` SAT over canonical convex coverage, and explicit convex-intersection containment for declared joints before curved-body conservative envelopes. SAT alone cannot prove that overlap lies inside an allowed joint volume. |
| V4-P11 | **PROPOSED** | A declared joint is a canonical entity with its own bounded allowed-overlap volume. Undeclared overlap, overlap outside that volume, and an empty declared joint are errors. |
| V4-P12 | **PROPOSED** | CI mechanically protects code quality, sole mutation authority, legacy-authority absence, gate suites, and R0 threshold direction (`tighten/loosen/neutral/unknown`). |
| V4-P13 | **PROPOSED** | The native schema advances only with explicit migration and resource-limit policy; a current file always declares document, evaluator, backend, and determinism envelopes. |
| V4-P14 | **PROPOSED, REVISED AFTER REVIEW** | Validators are first-class deterministic read-only services behind an open domain-neutral interface, diagnostic schema, policy model, and result taxonomy. M4a freezes this internal/host-neutral contract; third-party distribution, signatures, revocation, licensing, native isolation, and remote egress belong to M7 hosting. Structural/statics results are permitted best-effort decision support, never a Ketchup safety guarantee, regulatory certification, or substitute for approval by a qualified structural engineer. |
| V4-P15 | **RATIFIED by ADR 0004; amended after A0 v2** | Replace the old monolithic Gate C sequence with C1a before additional proxy-modeler stabilization, execute the early beam checkpoint M4a-E immediately after M2 and before OCCT product integration, complete the remaining M4a protocol/projection/evidence track without delaying that first run, then execute C1b after exact product integration. This is an accepted sequence change, not a clarification; withdrawal of temporary L-01/L-02 did not revoke this ordering. |
| **W4A** | **PROPOSED; ADDED AFTER REVIEW ROUND SIX** | Every validation diagnostic carries an evidence class — `Exact` or `Tolerant` — orthogonal to the §10.1 assertion classes and to the §7 reference stability classes. `Exact` requires exact inputs **and** an algebraically closed method; mixed sets take the weakest participant's class; aggregate reports separate the counts and never emit one combined “passed”. Introduced in M4a-E while every result is still `Exact`. See §10.1.1. |
| **W4B** | **PROPOSED; TARGET-STATE, NOT FLP-BLOCKING** | `Space` (with declared purpose and adjacency/access) and `ClearanceVolume` (bounded volume that must remain unoccupied, rule-derivable with `SlotPath`) become canonical entities so that clearance and advisory validators — including feng shui — consume declared inputs instead of inferring rooms, purpose, and access from geometry. `ClearanceVolume` is the sign-inverted twin of the §10.3 joint volume and reuses its containment machinery; the two contracts MUST be designed jointly. See §10.9. |

## 3.4 Open decisions

| ID | Status | Owner / decision point |
|---|---|---|
| V4-O01 | **OPEN; non-blocking for V4 adoption, blocks M14 format freeze** | Core/IO owner must choose exact blob persistence versus deterministic recomputation per body family before the M14 container/schema decision. |
| V4-O02 | **OPEN; non-blocking for V4 adoption, blocks a general expression/solver commitment** | Evaluator/sketch owner plus license/security reviewer must decide before expanding beyond the bounded implemented expression vocabulary. |
| V4-O03 | **OPEN; non-blocking unless an M12 frozen workflow requires it** | Evaluator/sketch owner must decide whether M12 needs a general constraint solver; otherwise retain it as an explicit post-FLP spike. |
| V4-O04 | **IMPLEMENTATION EVIDENCE COMPLETE; owner ratification still OPEN** | M14 selects ISO 10303 STEP for bounded exact exchange and Wavefront OBJ for derived mesh exchange; §5.3.2 records format, license, tolerance, loss, round-trip, and fail-closed evidence. This technical evidence does not impersonate the IO/domain owner. |
| V4-O05 | **OPEN; non-blocking for V4 adoption, blocks only BTLx scope in M17** | IO/domain owner must decide BTLx and manufacturing tolerances after general BOM/dimension projections pass; absence does not block non-BTLx fabrication work. |
| V4-O06 | **OPEN; non-blocking for V4 adoption, blocks M14 public compatibility promise** | Core/IO owner must not publish file/API compatibility until migration and backend-change suites pass and the promise is recorded. |
| V4-O07 | **OPEN; non-blocking for FLP and V4 adoption** | UI owner must decide long-term `egui` retention from measured UX, performance, and accessibility evidence; prototype familiarity is insufficient. |
| V4-O08 | **RESOLVED by ADR 0007 — Windows x86-64 first release** | The project owner selected Windows x86-64 as the only first-release platform on 2026-08-09. Parallel macOS/Linux support is deferred and must not delay end-to-end Windows product proof. |
| V4-O09 | **OPEN; non-blocking while local-first remains binding, blocks M18 cloud defaults** | Project owner must approve provider, privacy, retention, and explicit opt-in policy before any default cloud path; no decision means no cloud disclosure. |
| V4-O10 | **OPEN; BLOCKS V4 ratification and release readiness** | A named accountable project owner must accept or reject the V4 baseline and identify human owners, budget, and release quality bar. Role labels and agent implementation do not satisfy this decision. |

## 3.5 Decision governance

V4-P01–P14 remain **proposed MUSTs** until accepted by the accountable project owner; implementation and passing tests are evidence, not silent ratification. V4-P15 is already ratified by ADR 0004. Full V4 ratification therefore still requires the two owner records below, after which the adoption ADR incorporates the dedicated P07/P08 ADR by reference. Separate ADRs for the remaining individual P decisions are not required unless a later change alters their accepted architectural commitment.

| Required ratification record | Current disposition | Evidence already available | Blocking owner action |
|---|---|---|---|
| V4 adoption ADR for P01–P14 | **MISSING — BLOCKED ON EXPLICIT PROJECT-OWNER DECISION** | Current as-built source/tests, review dispositions, and the requirement/evidence/gap matrix prepared by M11C. | A named accountable project owner must review the final baseline and explicitly accept, reject, or amend P01–P14 in a dated record. Agents must not infer acceptance from implementation. |
| Dedicated P07/P08 retention/trust/compatibility ADR | **MISSING — BLOCKED ON EXPLICIT PROJECT-OWNER DECISION** | ADR 0006 proves and accepts the immediate two-write-path implementation consequence; observational Open, freshness audit, P07 envelope checks, and schema fixtures exist. | The owner must decide retained-result trust and display, evaluator/backend compatibility and quarantine, persistence/retention limits, and unavailable-producer behavior. ADR 0006 explicitly cannot substitute for this record. |
| Dedicated P15 sequence ADR | **SATISFIED by ADR 0004** | Accepted replacement gate charter, A0 dispositions, C1a/M4a-E/M3/C1b ordering, and amended A0 v2 outcome. | None unless the accepted order or evidence envelope changes. |
| Named accountable owner and release governance | **MISSING — BLOCKS RATIFICATION** | Role-level ownership exists in this section; V4-O10 records the unresolved gap. | Supply a named accountable project owner; later release readiness additionally needs named human owners, budget, and quality bar. |

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
| P15 | Architecture lead | Project owner | explicit old-C disposition and replacement gate charter, including M4a-E-before-M3 and full-M4a concurrency/exit rules | **Satisfied by ADR 0004; preserve as a regression constraint** | ADR 0004 accepted |
| O01, O06 | Core/IO lead | Project owner | storage/compatibility matrix and two-build migration suite | before public compatibility promise | Yes |
| O02, O03 | Evaluator/sketch lead + license reviewer | Project owner | license/security shortlist and focused spikes | before corresponding implementation | Yes |
| O04, O05 | IO/domain lead | Project owner | user workflow, format/license/tolerance evidence | before export scope freeze | Yes |
| O07 | UI lead | Project owner | measured UX/performance/accessibility evidence | before long-term UI commitment | Yes |
| O08–O10 | Project owner | Project owner | product/privacy/resourcing decision record | before release commitments | Yes where architecture/privacy changes |

Named people and calendar dates remain unavailable. This is not a documentation omission that an agent may fill: it is V4-O10 and the explicit ratification blocker above. Role names guide implementation ownership but do not satisfy accountable acceptance or release-readiness governance.

---

# 4. System-wide status matrix

| Subsystem / invariant | Status | Current boundary | Target |
|---|---|---|---|
| Immutable revisions and snapshot reads | **IMPLEMENTED** | Core snapshot history uses immutable `Arc` revisions; canonical writes append only after validation. | Structural-sharing optimization, explicit retention/checkpoints, and complete evaluator generation invalidation. |
| Sole canonical edit path | **IMPLEMENTED narrow** | `apply_batch` owns canonical semantic changes; ADR 0006 and the D-08 register define the P07 non-canonical path and reviewed lifecycle/construction operations. Architecture guards inspect internal `ketchup-core` writes and deliberate-red proves rejection. | Preserve while broadening command vocabulary and capability/audit policy. |
| Atomic multi-command commit | **IMPLEMENTED** | Candidate state is fully validated before one revision append. | Preserve while adding broader geometry/domain validation. |
| One batch = one Undo step | **IMPLEMENTED for canonical state** | Snapshot cursor navigation; P07 results create no revision or Undo step; history is not persisted. | Complete evaluator job invalidation across all clients. |
| Product identity/hierarchy | **PARTIAL—PRODUCT PATH** | Typed IDs, definitions, global and definition-local occurrences/groups, canonical tags/collections, document identity, direct nested query scope, joints, exact-reference evidence, and hierarchical `SlotPath`; allocation/remap policy remains bounded. | Non-reuse/import-remap policy, saved views, and future entity-family scope. |
| Parametric DAG | **PARTIAL—PRODUCT PATH** | Typed parameter/expression/rule nodes, ports, bounded parser, affected-only evaluation, per-node digests, outputs, `SlotPath`, override health, typed rule→feature parameter bindings, identity-bound explicit recompute, persisted provenance, and stale-input/backend audits exist. Binding coverage is still limited to current feature slots. | One complete typed feature/rule DAG for all product geometry and domains, including profile constraints and every supported feature parameter. |
| Definitions/components/Make Unique | **IMPLEMENTED narrow** | Sharing, clone/repoint, definition-local hierarchy, Group→Component, and Make Unique work for the current feature vocabulary. | Complete nested conversion and future-feature remapping. |
| Edit context | **IMPLEMENTED narrow** | Ephemeral app context state binds core-owned visible scene queries to document identity, revision and digest; direct nested scope, selection/picking, metadata counts and shared-definition Push/Pull fail closed outside that envelope. | Broaden the same contract with future local-occurrence transform commands and exact/mesh element families. |
| Persistence | **PARTIAL—PRODUCT PATH** | Deterministic container schema 1 separates required schema-16 `document.bin`, content-addressed blobs and bounded namespaced extension payloads with safe paths, per-entry SHA-256 and total/entry limits. Optional unknown namespaces survive app Open/edit/Save byte-for-byte; required unknown semantics become `ReviewOnly`; atomic replacement retains a last-verified recovery sibling. Schemas 0–15 remain observationally readable; supported lossy migration requires one confirmed batch into a preserved-source copy; the tested compatibility matrix is public in §5.3.1. | Complete broad product backend migration/quarantine, owner format freeze and public compatibility disposition. |
| Exact geometry | **PARTIAL—PRODUCT PATH** | Rectangle extrusion, Boolean Cut/Union, the rotational Revolve/Shell/Fillet-Chamfer bottle, and beam half-lap bodies use shared canonical feature chains, supervised OCCT evaluation, `ExactBodyPackage` views, and a deterministic multi-body `ExactResultRegistry`; the running app proves a bounded arbitrary groove/cut path and worker-mediated current-bottle STEP export. | Broader profile/transform/boolean/edge-operation families, arbitrary curved workflows, canonical mesh bodies, and complete conversions. |
| Mesh geometry | **PARTIAL—PRODUCT PATH canonical authority, query projection and derived export** | A schema-16 `MeshBody` is a validated closed, oriented manifold and the sole geometry feature of its detached definition. Current exact packages can produce it only through one explicit snapshot-bound conversion batch whose canonical provenance/loss envelope reports source and destination identities, tolerances, unsupported semantics, and exact-reference `Lost`; a read-only mesh interaction projection shares geometry by definition and performs BVH-accelerated physical triangle picking without becoming authority. Exact tessellation otherwise remains derived and OBJ export remains explicit-loss. | Mesh import/procedural authoring, render-path integration, mesh-to-exact conversion, and broader conversion envelopes. |
| Stable subshape references | **PARTIAL—PRODUCT PATH** | `BodySubshapeRef` and `AssemblySelectionTarget` are produced and consumed through the shared exact feature/result path, registered through P07, persisted in schema 16, and covered by bounded rectangle, Boolean Cut/Union, bottle, and beam-notch roles. The current exact registry now resolves durable identities explicitly as `Resolved`, `Ambiguous`, `Lost`, or `Quarantined`: occurrence transforms preserve lineage after current reevaluation, removed Cut wall roles become `Lost`, competing current candidates become `Ambiguous`, and invalid, foreign-document, or evaluator/backend/tolerance-incompatible evidence is quarantined without silent retargeting. | Broader profile/transform/boolean/edge-operation families, interaction-wide propagation of the explicit health result, and all supported feature families. |
| Scheduler/worker | **PARTIAL—PRODUCT PATH** | A general revision-safe coordinator now types exact, sketch, rule, mesh, and validator jobs; reports monotonic progress; separates cooperative cancel request/acknowledgment; enforces bounded restart; records lifecycle/cache telemetry; and publishes only current acceptance-bound results into the budgeted cache. The integrated exact path additionally has capability handshake, deadlines, process cancellation, crash restart, and one retry across rectangle/boolean/bottle/beam jobs. | Wire every product producer/executor through the general coordinator, add heartbeat/deadline telemetry, and prove production resource plateaus. |
| Interaction scene | **PARTIAL—PRODUCT PATH** | Canonical fallback, accepted exact packages and canonical mesh bodies now use one deterministic disposable BVH contract. Document/revision/digest binding rejects stale queries; stable candidate ordering preserves durable exact identity, snap/hysteresis and overlap behavior; focused 1,024-occurrence canonical/exact/mesh evidence bounds broad-phase candidates to four and bounds tests below one eighth of indexed items. | Broader topology, mesh snap/highlight identity, second-level triangle acceleration and product hardware evidence. |
| Renderer | **PARTIAL—PRODUCT PATH** | The app now emits revision-bound instanced render plans for canonical box fallback, canonical `MeshBody`, and current accepted exact packages; one deterministic derived geometry is shared per compatible definition/result, persistent wgpu vertex/index buffers survive instance-buffer refreshes, and the desktop viewport issues indexed instanced draws while CPU painting is limited to interaction overlays. A measured 10,000-occurrence DX12 product test on an AMD Radeon RX 6800 XT proved one shared scheduler-accepted mesh, one GPU geometry upload plus cache reuse, one draw, 10,000 instance transforms, and BVH picking with one candidate/28 bounds tests. | Richer GPU highlights/depth/ID surfaces, LOD/approximation metadata, general curved-body presentation, cross-hardware release certification, and sustained frame-time/resource-plateau evidence. |
| Manual shell | **PARTIAL—PRODUCT PATH** | Major shell, Outliner, exact numeric tools, profile-only Rectangle, first-extrusion Push/Pull, file workflows, hierarchy/component actions, and Undo/Redo exist. | Complete Line/polyline viewport UX, tags, persistent dimensions, remaining shortcuts/menu and physical-window evidence. |
| Collision validation | **PARTIAL—PRODUCT PATH** | The bounded prismatic path has AABB/OBB/SAT, convex intersection, certified joint containment, and FurniGen evidence; M4b validates exact-body clearance plus canonical joints with durable exact contacts; M17A resolves arbitrary visible root/nested occurrences through one current `ExactResultRegistry` or canonical `MeshBody`, binds shared exact keys or canonical mesh digests into deterministic collision/clearance input, propagates W4A, and rejects missing, ambiguous, stale, hidden, or invalid geometry. | Curved-body/mesh narrow phases, `ClearanceVolume`, broader domain coverage, and general standards/manufacturability validators. |
| Validator protocol and hosting | **PARTIAL—PRODUCT PATH** | Host-neutral read-only contract plus a bounded M7C host now verify Ed25519 publisher signatures, signed descriptor/artifact identities, monotonic updates, trust/revocation and external paid-license availability. Exact descriptor/current-invocation binding fails closed to `Unavailable`; signed no-import WASM executes under strict parser, fuel and memory/table/instance limits, while host-mediated TCP egress requires the signed publisher declaration, a separate exact-endpoint host grant, and a human-only external-disclosure receipt bound to the package/provider, `tcp://host:port`, and request bytes before connection; transport remains byte/timeout bounded and separately receipted. | Persistent production catalog, native OS sandbox launcher, public HTTPS/TLS egress deployment, credential brokerage, and broader validator result decoding remain later scope. |
| Rules and manufacturing projections | **PARTIAL—PRODUCT PATH** | Beam rules derive stable pieces/joints, grouped list, full BOM, and `SlotPath` dimensions; M17B additionally projects every validation-covered visible exact or sole-authority canonical-mesh body into deterministic definition/source-grouped BOM rows, stable local-bounds dimensions, named front/top/right SVG views, and snapshot-bound export envelopes. Supported rectangular exact stock and canonical Cut semantics produce definition-local manufacturing operations; stale/tampered projections, non-rigid occurrence transforms, canonical mesh, revolve, and Boolean Union manufacturing evidence fail closed. | Exact silhouette/section drawings, richer materials and manufacturing vocabulary, mesh/revolve/Union manufacturing semantics, professional layout, and later BTLx. |
| Proposal safety | **PARTIAL—PRODUCT PATH** | M7A exposes two workflow-derived dimension intents through a principal/capability grant and a revision/digest-bound Proposal with authoritative typed read/write sets, assumptions, before/after diff, Standard risk, ReviewRequired confirmation, bounded budget, stale/replay rejection, and transactional postcondition verification; accepted changes delegate exactly once to `apply_batch`, and the M7B plugin uses the same path. M18A adds a distinct signed human-only path for six high-risk classes, exact destination/provider/path scope, five-minute maximum lifetime, policy epochs, one-use replay protection, and fail-closed requester/approver/document/revision/command/before/result binding. M18B1–B5 separate non-canonical side-effect Proposals and receipts from canonical commit authority, wire exact-byte overwrite evidence into atomic Save, require a receipt over the framed OBJ plus loss-report bytes before active-document or beam-workspace lossy mesh export, block validator TCP disclosure unless a human-only receipt matches the exact provider, destination, and request bytes, and require a Beam-revision/path/payload-bound receipt before releasing the deterministic manufacturing export. M18C1–M18C46 add occurrence visibility, exact XYZ occurrence/group translation, definition/evaluator-node rename, evaluator expression and exact flat rule-output editing, bottle profile-control dimension, edge-finish-kind and exact profile-point editing, tag visibility, occurrence tag assignment/removal, occurrence definition repointing, occurrence group assignment/removal, group reparenting, exact collection membership, single-tag, single-collection, empty-group, uncollected-occurrence, empty-definition, and unused exact-profile-feature deletion, and named independent evaluator-input, evaluator-expression, flat evaluator-rule, resolved flat rule-override creation/deletion, resolved feature-parameter binding creation/deletion, single-bound feature-parameter recomputation, canonical-joint, canonical-space, canonical-clearance-volume, and persistent-dimension deletion, tag, empty-collection, empty-definition, exact profile-feature, root-group/root-occurrence creation, and bounded empty-group conversion as local typed intents with distinct capabilities, explicit boolean, Transform, text, BottleEdgeFinishKind, ordered 2D-point-list, exact RuleOutput-tree, EvaluatorInputState, EvaluatorExpressionState, EvaluatorRuleState, RuleOverrideState, FeatureParameterBindingState, JointState, SpaceState, ClearanceVolumeState, PersistentDimensionState, TagState, CollectionState, DefinitionState, GroupState, optional-Tag, optional-Group, Definition-ID, or canonical occurrence-ID-list before/after evidence, target-existence-or-absence assumptions, observational preparation, stale-target rejection, and one verified `apply_batch` commit. | Broader proven intent vocabulary and Gate D corpus, plus production provider/egress policy. |
| AI assistant | **PARTIAL—PRODUCT PATH** | The local Assistant panel prepares ephemeral typed dimension, bottle-edge-finish-kind, exact-profile-points, definition/evaluator-node-rename, evaluator-expression, flat-rule-output, occurrence-visibility, tag-visibility, occurrence-tag, occurrence-definition, occurrence-parent, group-parent, exact collection-membership, single-tag, single-collection, empty-group, uncollected-occurrence, empty-definition, and unused exact-profile-feature deletion, named independent-evaluator-input/evaluator-expression/flat-evaluator-rule creation, resolved-flat-rule-override creation/deletion, resolved feature-parameter binding creation/deletion, single-bound feature-parameter recomputation, canonical-joint, canonical-space, canonical-clearance-volume, and persistent-dimension deletion, and tag/empty-collection/empty-definition/exact-profile-feature/root-group/root-occurrence creation, and bounded empty-group conversion, or exact occurrence/group-translation Proposals, displays provenance/read-write/diff/risk/budget evidence including explicit text, boolean, BottleEdgeFinishKind, ordered 2D-point-list, exact RuleOutput-tree, EvaluatorInputState, EvaluatorExpressionState, EvaluatorRuleState, RuleOverrideState, FeatureParameterBindingState, JointState, SpaceState, ClearanceVolumeState, PersistentDimensionState, TagState, CollectionState, DefinitionState, GroupState, optional-Tag, optional-Group, Definition-ID, canonical occurrence-ID-list, and Transform changes, and offers explicit Confirm/Cancel; Confirm creates one verified canonical Undo step and Cancel creates none. Core M18A refuses high-risk commit without a separately authenticated human token, but the panel is not yet the trusted token-issuing surface. | Natural-language/model-provider integration, trusted high-risk confirmation UI, broader verification criteria, and full Gate D corpus remain incomplete. |
| Plugin system | **PARTIAL—PRODUCT PATH** | M7B provides a bounded external Python process protocol with StateView-only reads and Intent-only writes; M7C adds signed no-import WASM validators, trust/revocation/licensing checks, strict resource limits, and host-mediated TCP egress gated by both endpoint grant and exact human-only disclosure receipt. Neither path receives document, OCCT, renderer, or commit authority. | Persistent catalog/namespaces, native OS sandboxing, production HTTPS/TLS egress, credential brokerage, broader result decoding, and distribution governance. |
| CI governance | **PARTIAL—PRODUCT PATH** | Daily CI runs the unfiltered product workspace while sealed A0 v1/v2 require an explicit certification feature and runner; guards enforce that separation plus D-08/P07, legacy absence, StateView, optional supplied frozen inputs, and anti-loosening invariants. | Keep reviewed governance hashes current and complete required gate/hardware execution without merging historical certification freshness into the daily product signal. |
| Gate certification of current tree | **PARTIAL—PROOF ONLY** | Historical strengthened A0 v2 `FULL_GO`, Gate B, C1a, and narrow C1b evidence exist; the current-evidence baseline in Appendix G is not a new freeze and Gate C hardware evidence remains incomplete. | New freeze over stable current inputs and honest reruns on required hardware. |

# 5. As-built architecture

## 5.1 Repository and dependency reality

The intended architecture diagram remains valid as a target, but current crate connectivity is narrower:

```text
ketchup-app ───────> ketchup-core
     │               ketchup-interaction
     └─────────────> ketchup-scheduler ─> ketchup-exact ─> CXX façade ─> OCCT
                           │                    │
                           ├──────────────────> ketchup-core
                           └──────────────────> ketchup-interaction
```

`ketchup-app` now depends on `ketchup-scheduler` as well as core and interaction (`crates/ketchup-app/Cargo.toml`). The scheduler remains the Rust isolation boundary that owns the dependency on `ketchup-exact`; the app does not link the OCCT façade directly. Product rectangle-extrusion and ThroughCut requests therefore follow app → supervised worker → exact façade, while accepted packages return through revision/digest checks into derived render, interaction, and P07 evidence paths.

## 5.2 Canonical core — PARTIAL—PRODUCT PATH

### Data model

`ProductModel` owns:

- `DocumentId`, units, and deterministic ordered entity maps;
- definitions, global and definition-local occurrences/groups, and profile/extrusion/ThroughCut features;
- typed evaluator nodes, hierarchical derived outputs, canonical overrides, and declared joints;
- exact-reference evidence registered as non-canonical P07 data;
- occurrences pointing to definitions with transform, parent, tag reference, and visibility.

The evaluator vocabulary includes parameter, expression, and rule nodes with typed ports. Expressions are parsed under explicit depth/token limits; evaluation produces identity-bound outputs and SHA-256 input/result digests. A committed rule change records the actually affected/recomputed node set, while unchanged branches preserve their prior result identity. Hierarchical `(RootRuleNodeId, SlotPath)` resolution and explicit preserved/lost/ambiguous override health are implemented.

The product feature vocabulary is still bounded:

```text
Profile { ordered planar points }
Extrusion { profile_id, height }
ThroughCut { target_extrusion_id, profile_id }
```

The remaining unification boundary is that arbitrary geometric feature results are not yet first-class values of one universal evaluator DAG. The current graph is real product/core infrastructure, not the former dependency-closure-only proof, but it does not yet express every future body operation and domain output.

### Identity

Distinct typed numeric IDs exist for document, definition, occurrence, group, feature, tag, and legacy node (`document.rs:10-25`). Core validation rejects zero IDs, missing ownership, duplicates, invalid feature references, invalid group parents, and cycles (`document.rs:1403-1617`).

Limits:

- every empty model defaults to `DocumentId(1)`;
- clients allocate most IDs, and the app uses `max + 1` (`crates/ketchup-app/src/lib.rs:1459-1466`, `1596-1615`);
- non-reuse, import remapping, cross-document scope, overflow, and generation policy are unspecified;
- `TagId` exists as a field but there is no canonical tag table or tag command.

### Command and revision path

`CommandBatch` has schema `ketchup.command.v1`, ordered commands, and a stable command digest. `DocumentStore::apply_batch` rejects unsupported or empty batches, clones a private candidate, applies all commands, validates the complete graph/override/product candidate, and appends exactly one revision only after all checks pass. `commit_proposal`, `make_unique`, and `convert_group_to_component` delegate to it.

ADR 0006 records the second and only non-canonical path: `register_derived_result`. Evaluator registration and exact-reference evidence validate their payload-specific provenance, then delegate a current document/revision/canonical-digest envelope to this P07 gateway. The gateway checks the envelope at runtime, changes no canonical digest, appends no revision, and creates no Undo step. Exact-reference evidence may replace the active snapshot's derived evidence map while retaining identical canonical authority.

Entity fields and `DocumentStore` authority fields are private; persistence constructs a separate validated baseline through crate-private `from_product`. The D-08 register names construction and lifecycle operations: New/Open validated-store replacement, cursor-only Undo/Redo, and retention-only history discard. The architecture guard now inspects private writes inside `ketchup-core`; its deliberate-red suite proves that an injected in-core revision write is rejected. These reviewed operations are not a third semantic write path.

### Undo/Redo

One successful batch appends one snapshot revision. Failed batches append none. Undo and Redo move the cursor between immutable snapshots; committing after Undo truncates the redo tail (`document.rs:1179-1200`). Product tests prove digest restoration, sharing, and hierarchy behavior (`crates/ketchup-core/tests/product_document.rs:73-137`).

Missing target behavior is evaluator-aware cancellation/invalidation. No integrated geometry job is attached to core history today.

### Definitions, occurrences, groups, and Make Unique

Definitions own ordered feature IDs; many occurrences can reference one definition. World transforms derive from group ancestry (`document.rs:600-657`). Groups and occurrences can be created, deleted, transformed, and reparented through commands, with cycle and non-empty-delete checks.

`CloneDefinitionAndRepoint` validates a complete feature-ID mapping, clones profile/extrusion features, remaps internal profile references, creates a new definition, and repoints only one occurrence (`document.rs:1451-1541`). That provides real atomic Make Unique semantics for the current feature vocabulary.

The app’s “Make Component” currently renames an already reusable definition rather than converting an arbitrary group/hierarchy to a component (`crates/ketchup-app/src/lib.rs:1541-1579`). Group-to-component conversion and general feature cloning remain incomplete.

## 5.3 Persistence — PARTIAL—PRODUCT PATH

The native file now uses deterministic container schema 1 around a required `document.bin`, content-addressed `blobs/<sha256>`, and bounded `extensions/<namespace>/<relative-path>` entries. Every entry is path-normalized, size-bounded, uniquely named and SHA-256 verified before use; the complete container has an independent size/entry envelope. Unknown top-level data and unsafe paths fail closed. Safe optional unknown extension payloads remain non-canonical sidecars and survive app Open/edit/Save byte-for-byte, while an unknown required namespace is preserved in a `ReviewOnly` candidate and cannot become the active editable document. Current exact body families retain canonical semantic specifications and recompute derived exact packages; the generic blob store does not make accepted results canonical or let cache substitute for meaning.

The required document stream is schema 17 while retaining observational readers for schemas 0–16. Earlier schema evolution added its own manifest envelope and SHA-256 payload checksum; explicit graph/evaluator/tolerance identities; bounded file, payload, string, and collection sizes; typed evaluator nodes and ports; overrides and health audit; declared joints; definition-local hierarchy; ThroughCut/Boolean features; and exact-reference evidence. Schemas 6–8 add the bounded bottle vocabulary; schemas 9–11 add shared exact-feature forms, typed rule→feature parameter bindings, applied provenance, and freshness audit. Schema 12 adds persisted profile-width/profile-height binding tags. Schema 13 adds canonical persistent dimensions with stable feature-parameter, complete `SlotPath`, or exact semantic targets, declared display units and presentation precision. Schema 14 adds canonical tag entities and tag visibility. Schema 15 adds canonical collection entities with strictly ordered, non-owning root-occurrence membership. Schema 16 adds canonical mesh-body authority and exact-to-mesh source/destination/tolerance/loss provenance. Schema 17 appends canonical `Space` and `ClearanceVolume` identity, purpose, relations, owner, world-frame bounds, tolerance, severity, and complete optional rule-derived identity; Open remains observational and StateView health projection does not mutate the candidate.

`save_atomic_with_container` fully encodes and re-load-validates the new artifact, writes and synchronizes a sibling temporary file, preserves the previously valid destination as a stable recovery sibling, and then atomically replaces the destination. `load_file` prefers the primary but may activate only a fully verified recovery artifact after a missing or corrupt primary, marking that fact in `LoadAudit`. Load checks size before allocation, verifies both container entries and the inner document envelope, constructs a separate complete candidate through crate-private `from_product`, and audits compatibility and override resolution. A lossless candidate without required unknown semantics is editable; a lossy, incompatible, or required-unknown candidate is `ReviewOnly` and cannot replace the active editable document. Failed Open likewise leaves the active document unchanged. Open performs no hidden evaluation or canonical migration.

A supported lossy legacy candidate can become editable only through `confirm_semantic_migration`: the user-selected flow reproduces every reported source-token repair as exactly one canonical `CommandBatch`, revalidates the result, and writes it to a path distinct from the reviewed source before replacing the active document. The original bytes remain the pre-migration copy. Required unknown extensions, unresolved override identities, unreported losses, same-path targets, and failed target writes remain fail-closed; they do not consume the review candidate or alter the active document.

### 5.3.1 Current file, backend-change, and recovery compatibility matrix

This matrix is the public description of tested behavior, **not** the still-unratified V4-O06 compatibility promise.

| Input/change | Current disposition | Data/action guarantee | Evidence boundary |
|---|---|---|---|
| Container 1 + document schema 16, known semantics | Editable | Deterministic verified Open/Save; canonical digest preserved | Core/app container and file-workflow suites |
| Document schemas 1–15 with no reported semantic loss | Editable after observational audit | Read into current in-memory model; no Open-time revision | Golden/current schema readers; public promise still unratified |
| Document schema 0 reconstructed decimal source token | `ReviewOnly` on Open | Explicit confirmation creates one batch revision in a distinct current-schema copy; source bytes preserved | `persistence_m2` and app file-workflow migration tests |
| Safe optional unknown extension | Editable | Opaque bytes preserved across Open/edit/Save | Container and app round-trip tests |
| Required unknown extension or unresolved override identity | `ReviewOnly`; confirmation refused | No activation, recompute, export-as-healthy, or silent drop | Negative core/load tests |
| Unsupported schema/container entry, unsafe path, limit or checksum failure | Rejected | No candidate or active-document replacement | Persistence adversarial tests |
| Same backend/evaluator envelope with valid persisted exact evidence | Editable | Evidence is accepted only when it still matches the canonical request | Schema-15 exact-reference fixtures |
| Different exact backend build | Isolated resolver evidence only | Strengthened A0 proves three bounded references resolve and one removed reference quarantines; general product Open migration remains unavailable | `ketchup-exact/tests/gate_a0.rs`; no broad compatibility claim |
| Missing/corrupt primary + verified recovery sibling | Recovery artifact loaded; audit flag set | Only a fully parsed and checksum-verified sibling may recover | Atomic recovery test |
| Missing/corrupt primary + invalid/missing recovery | Rejected | Active document remains unchanged | `load_file` fail-closed behavior |
| Confirmed migration target write/revalidation failure | Migration rejected | Source, review candidate, and active document remain unchanged | App copy-bound workflow contract |

Canonical state uses deterministic ordered traversal and exact numeric bits. Canonical batch and result identity now use SHA-256 where the corresponding graph/evaluation contracts require it; the document's user-facing canonical digest remains an equality/dirty-state authority rather than a substitute for the file checksum. P07 exact-reference evidence round-trips but is excluded from canonical digest authority.

Current gaps are broad product backend migration/quarantine beyond the isolated A0 transfer evidence, a visible recovery/migration dialog, mesh import/procedural/render/query support and mesh-to-exact conversion, persisted history, and owner ratification of the bounded recompute-versus-blob policy before format freeze. The implemented container/schema-16 path, confirmed copy migration, compatibility matrix, and recovery sibling are product evidence, not yet a public compatibility promise.

### 5.3.2 M14 bounded export selection and loss evidence

M14's technical selection is **ISO 10303 STEP** for one standard exact exchange path and **Wavefront OBJ** for one derived mesh exchange path. This satisfies the bounded FLP implementation exit; it does not claim general importer/exporter coverage or substitute for the still-required human V4-O04 ratification.

| Evidence dimension | STEP exact exchange | OBJ mesh exchange |
|---|---|---|
| Product scope | Current accepted rotational bottle B-Rep | Any current accepted `ExactBodyView` exposed by the bounded product paths |
| Format rationale | Existing OCCT Data Exchange module and frozen three-file STEP import corpus; preserves exact B-Rep geometry/topology for downstream exact systems | Simple interoperable triangle format already produced deterministically from accepted render tessellation |
| License boundary | Uses the pinned unmodified OCCT 8.0.1 shared-library path under the R0 LGPL-2.1 + Open CASCADE Exception distribution checklist; user CAD output is not relicensed by OCCT | Ketchup writes the text format directly and introduces no additional runtime dependency |
| Freshness/failure boundary | Parent supplies the current snapshot, canonical request, and accepted package; the worker reconstructs the same operation, requires the expected result fingerprint, writes a sibling temporary artifact, and publishes only after an identity-echoed success | Export requires a current revision/digest-bound package; stale, missing, or quarantined exact evidence produces no healthy export |
| Verification | Focused suite writes `ISO-10303-21`, reimports it through OCCT, verifies one solid and matching bounds, and proves stale rejection leaves no target | Deterministic vertices/groups/faces are covered across rectangle, Boolean, bottle, transformed occurrence, and beam paths |
| Explicit loss report | `.step.loss.txt`: canonical feature/rule/dimension/Undo editability is lost; exact B-Rep topology remains but Ketchup durable subshape identity is lost; no tessellation loss is claimed, while receiving systems may apply another modeling tolerance | `.obj.loss.txt`: canonical editability, exact/analytic topology and durable face identity are lost; geometry is approximated under the named source tolerance profile |

The proprietary `.kbex` bottle recipe remains only internal legacy evidence and is no longer the product exact-export choice. STEP and OBJ exports do not mutate canonical state, create revisions, or add Undo steps.

## 5.4 Application and interaction — PARTIAL—PRODUCT PATH

### Shell and workflows

The desktop application now contains the major designed shell: top/menu/status bars, tool rail, viewport, Outliner/Tags dock, value box, localized hints/action digest, command registry, and file-dialog seam (`crates/ketchup-app/src/lib.rs:1205-1426`, `2578-2697`, `3247-3641`; `src/dialogs.rs:29-160`).

The following current narrow workflows are real and tested:

- object select, multiselect, clear, and shared viewport/Outliner selection;
- profile-only rectangle creation with exact values and an atomically validated closed-polyline application path;
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

The interaction crate owns typed selection/element identity, analytic cuboid services, canonical box projection, accepted-exact triangle projection, canonical-mesh triangle projection, and the shared `ketchup.spatial-bvh.v1` broad phase. The deterministic median-split BVH indexes world-space occurrence bounds, preserves stable path/triangle tie-breaking, accelerates ray and intersection-neighborhood candidates, and remains disposable derived state. Every projection binds document, revision and canonical digest; explicit current-query APIs reject stale projections rather than silently using old candidates. The app first binds current exact packages to the active snapshot; exact picking returns `AssemblySelectionTarget` with durable body/subshape identity. ThroughCut packages use a 16-vertex/32-triangle hole mesh and focused evidence confirms that a ray through the opening does not hit a fictitious cap. Entities without a current exact package retain the derived `SharedBoxGeometry` fallback; detached canonical `MeshBody` definitions share one read-only interaction geometry across occurrences.

The application builds `ketchup.render-plan.v1` from one immutable snapshot and current `ExactResultRegistry`: accepted exact triangles take precedence, canonical `MeshBody` geometry remains canonical input, and the cuboid projection is only a derived fallback. Compatible occurrences share one deterministic CPU render geometry and one persistent wgpu vertex/index allocation; per-frame placement is a compact instance buffer, and the desktop `egui_wgpu` callback submits one indexed instanced draw per batch. Plans bind `DocumentId`, revision and canonical digest and fail closed when no current supported geometry exists; neither plan nor GPU cache becomes geometry authority. CPU polygon generation remains only for selection, hover, preview, and edit-context overlays. The focused M16 product proof (`crates/ketchup-app/tests/renderer_m16.rs`) applies one canonical 10,003-command batch for one definition plus 10,000 occurrences, accepts its typed Mesh job through the general scheduler, resolves the final occurrence through the shared BVH, and performs a validation-clean DX12 submit/wait on a discrete AMD Radeon RX 6800 XT. The observed debug run used one geometry upload, one subsequent GPU cache hit, one draw for 10,000 instances, one BVH candidate/28 bounds tests, 195.538 ms render-plan construction, 35.880 ms GPU prepare/submit/wait, and 887.786 ms total test work. These figures are bounded evidence from that machine, not cross-hardware release certification or a sustained-frame plateau.

### Edit context

Selection and the edit-context stack remain ephemeral app state, while `BoundSceneQuery` binds the permitted canonical scene slice to `DocumentId`, revision and canonical digest. Core scope queries return only visible direct group/definition members, recompute sharing metadata inside that slice, reject stale/hidden anchors, and prevent traversal into a second nested component without explicit context entry. Viewport/outliner selection and exact/proxy picking consume the same permitted paths; definition-context Push/Pull resolves the selected `InstancePath`, previews without mutation, commits one canonical feature batch, and becomes stale if either snapshot or context changes. Unsupported local-occurrence transforms remain fail closed rather than mutating a root occurrence accidentally.

### Known shell gaps

- Line/polyline is disabled;
- Measure remains an ephemeral arbitrary-point reading because its viewport points are not durable references; bounded persistent dimensions are authored through canonical stable feature/`SlotPath`/exact-semantic targets, and a proxy-only Measure result is never promoted silently;
- canonical tags, assignment, query, persistence, and core-owned effective visibility exist; canonical collections add deterministic non-owning root-occurrence membership/query/persistence, but the shell has no complete tag/collection-management surface;
- component conversion is incomplete;
- a bounded exact axis linear-pattern workflow exists, but a complete interactive pattern surface is absent; union, export, Assistant, and plugin surfaces are absent; bounded ThroughCut exists but is not a general boolean/opening family;
- several menu commands and specified shortcuts are disabled or missing;
- status fields such as snap/grid/reference health are partly fixed labels rather than live services;
- the camera/projection and viewport decorations do not fully match the design specification;
- the bounded box/exact-backed viewport now consumes BVH-filtered scored snap and overlap candidates with hysteresis, cycling, a visible marker/readout, and snapped Measure/value-box evidence; broader arbitrary-curve inference, mesh snap/highlight semantics and triangle-level acceleration remain future work.

## 5.5 Exact backend — PARTIAL—PRODUCT PATH

`ketchup-exact` links the pinned OCCT build through CXX behind the process-isolated scheduler boundary. The native façade catches exceptions and validates successful shapes using B-Rep validity, solid count, finite bounds, positive volume, and deterministic result evidence.

The product path currently supports rectangular extrusion, bounded Boolean Cut/Union, the frozen rotational-bottle chain, and beam half-lap bodies. Shared feature-chain requests and `ExactBodyPackage` views carry exact topology evidence, semantic face roles, tessellated triangles, bounds/fingerprints, and durable body/subshape references into the deterministic `ExactResultRegistry`. Core owns `BodySubshapeRef` and `AssemblySelectionTarget`; accepted evidence is registered through P07, queried by the app, persisted by schema 16, and exercised by bounded C1b evidence. Cut covers outer roles plus four cut-wall roles and hole-safe interaction; the bottle covers five Revolve roles or nine Shell/finished-shell roles; beam pieces add durable notch contact/wall roles.

The current registry exposes one fail-closed durable-reference resolver over accepted current packages. It preserves a role only when document, definition, profile, producer, semantic source, type, cardinality, stability, lineage, evaluator, backend, and tolerance evidence permit it. Focused evidence proves stable remapping after an occurrence transform and reevaluation, `Lost` after Cut→Union removes a wall role, `Ambiguous` for competing current candidates, and quarantine for tampered lineage, another document, or an incompatible evaluation envelope. This resolver does not mutate canonical state or permit render/pick data to become model authority.

The façade also retains isolated box/STEP/probe operations that are not all product capabilities. Reference capture/resolution and backend identity remain narrower than the complete target: broad profile and curved-topology transformations, arbitrary feature mutations, general edge selection, propagation of explicit health through every interaction/validation consumer, and general exact operation families are not yet proven.

## 5.6 Scheduler and worker — PARTIAL—PRODUCT PATH

`ketchup-scheduler` retains revision/generation/input-digest stale rejection and bounded cache accounting, and now adds a product-neutral `GeneralJobScheduler` over the complete acceptance identity. Its typed `Exact`, `Sketch`, `Rule`, `Mesh`, and `Validator` jobs share one deterministic lifecycle: queue/start, monotonic bounded progress, cooperative cancellation request and executor acknowledgment, terminal failure, or a policy-bounded restart. A revision change marks every in-flight job stale before publication; current completion is the only path into the existing budgeted LRU cache. Same-revision cache hits carry the original acceptance token and result fingerprint, while cumulative telemetry reports requests, hits/misses, starts, progress, cancellation, restarts, completions, failures, and stale outcomes. Focused M16 evidence covers all five kinds, retry exhaustion, cancellation non-publication, stale revision rejection, and cache accounting (`crates/ketchup-scheduler/tests/general_scheduler_m16.rs`).

The application's `ExactWorkerSupervisor` remains the integrated process executor with capability handshake, request deadline, process cancellation, crash detection, restart, and one retry. Rectangle/ThroughCut, bottle, STEP-export, and beam requests accept packages only for their matching canonical source envelope. ADR 0002's parent-owned canonical state and process-isolation decision remains binding: neither coordinator nor worker owns document truth. Current limits are wiring every product producer through the general coordinator, heartbeat/deadline telemetry at that layer, richer resource classes, and production resource plateaus.

## 5.7 Validation and fabrication — PARTIAL—PRODUCT PATH

The M4a prismatic path implements a versioned tolerance policy; AABB/OBB/SAT broad and narrow phases; explicit convex intersection; certified containment in bounded declared joint volumes; and stable `NoContact`, `Touching`, `PenetratingExpected`, and `PenetratingIllegal` outcomes. Empty joints and undeclared overlap fail. The beam rule path preserves hierarchical slots and overrides while regenerating the frozen `415 × 6`, `408 × 5`, `400` fixture, grouped pieces, a deterministic full BOM, and stable dimension sheets. A FurniGen prismatic fixture guards required hard failures.

The host-neutral built-in validator contract has versioned descriptors, invocation/input digests, read scopes, policies, work/input/output limits, structured diagnostics, and distinct `Passed`, `Failed`, `NotEvaluated`, and `Unavailable` states. W4A is implemented: every diagnostic is `Exact` or carries `Tolerant` threshold/method/error-direction metadata; mixed participants take the weakest class; ValidationReport, BOM, complete/agent StateView, and app output report Exact and Tolerant counts separately. The current M4a-E beam reports `12 Exact / 0 Tolerant`.

The bounded M4b path accepts only resolved, current exact packages at supported occurrence transforms, binds both derived and exact-result identities into the invocation, and fails closed on stale snapshots or mismatched inputs. It classifies exact-body minimum gaps, touching, and intersection through conservative AABB envelopes; validates canonical joint volume plus durable opposing exact-face participants; and projects exact parallel-face dimensions with revision-bound body/reference identities. Rectangle boxes use `Exact` evidence; the real ThroughCut package exercises `Tolerant` propagation with an explicit threshold, method identity, and false-positive-only envelope.

M17A adds a family-neutral general-body collision/clearance input over arbitrary visible `InstancePath`s. A participant must resolve to either one current shared `ExactResultRegistry` key or the definition's sole canonical `MeshBody`; missing/ambiguous packages, hidden or invalid paths, stale exact results, invalid geometry, stale invocation envelopes, and tampered input digests fail closed. Deterministic transformed AABB evaluation remains `Exact` only for accepted axis-aligned translated rectangle boxes; curved exact packages, transformed envelopes, and canonical meshes are explicitly `Tolerant` with a false-positive-only method envelope. This closes the bounded G17-01 service contract, not curved/mesh narrow phases, target-state `ClearanceVolume`, structural certification, or third-party validator hosting.

M17B binds general fabrication regeneration to that same current validation input and accepted geometry set. Every visible geometry-bearing occurrence must be covered; container-only definitions are skipped, while unavailable evidence and non-rigid scale/shear fail closed. Stable exact/mesh source identity and local dimensions group BOM quantities; each group produces three named orthographic bounds views and associative x/y/z callouts, with deterministic result digests and W4A evidence. BOM and drawing export reject stale, tampered, failed-validation, or incompatible schema/evaluator envelopes. Manufacturing export is deliberately narrower: supported rectangular exact stock plus canonical ThroughCut/Boolean Cut semantics emit definition-local operations, while canonical mesh, revolve, Boolean Union, and unresolved semantics remain incomplete and unexportable. This is the bounded G17-02 service proof, not professional drawing layout, exact curved silhouettes/sections, a universal machining vocabulary, or BTLx.

M17C closes G17-03 with canonical `Space` and `ClearanceVolume` maps behind `apply_batch`. Spaces carry stable identity, declared purpose, bounded world volume, symmetric adjacency and directional access. Clearance volumes carry element/space owner, reason, world-frame bounds, tolerance, severity, and optional complete `DerivedIdentity`; creation requires a currently resolved `SlotPath`, while later `Lost`/`Ambiguous` paths remain canonical but validation fails closed without retargeting. Occupancy re-accepts current exact/mesh participants through `ExactResultRegistry` and uses the same deterministic `collide_axis_aligned_prisms` intersection path as joint overlap, with sign-inverted policy. Schema 17, canonical/Proposal digests and complete/agent StateView preserve the contract. V4-O05 remains owner-open, so G17-04 is closed by explicitly excluding BTLx rather than inventing scope or manufacturing tolerances.

M18A adds bounded G18-01 core evidence for human-only high-risk confirmation. Six explicit risk classes require a matching `HumanOnly` scope; disclosure binds destination and provider, while overwrite/lossy/manufacturing-warning paths bind the displayed path. A trusted Ed25519 confirmation surface signs the distinct authenticated human approver, requesting principal, document and revision, base canonical and dependency digests, exact command and intended result digests, complete risk scope, monotonic policy epoch, issue time, and expiry with a five-minute maximum. `DocumentStore` owns only the configured verification key, epoch, and consumed-signature set outside canonical state. Standard commit rejects high risk; the dedicated path rejects anonymous/machine/self approval, substitution, untrusted signatures, expiry, stale epochs, and replay before delegating once to the existing transactional verified `apply_batch` path.

M18B1–B3 extend that bounded G18-01 evidence to real non-canonical disk side effects without creating a second model authority. `SideEffectProposal` binds the current document/revision/canonical digest, identified requester, exact operation, risk scope, and SHA-256 of the bytes to be written. The trusted application surface uses OS-generated signing entropy, displays path/revision/payload/operation evidence, issues a five-minute domain-separated approval, and asks `DocumentStore` only for a one-use authorization receipt. Authorization changes no canonical digest, revision, Undo step, or derived registry and never calls `apply_batch`; existing-file Save performs its atomic write only after receipt issuance. Active-document bottle and occurrence OBJ export, plus Beam-workspace piece OBJ export, now frame both exact mesh bytes and the deterministic loss-report sidecar into one `LossyConversion` payload before receipt issuance, so refusal leaves both artifacts byte-for-byte unchanged. Beam approval is bound to the Beam source document and revision rather than the unrelated active shell document; consuming its receipt changes neither canonical state nor Undo. Payload substitution, stale snapshot, policy mismatch, expiry, and replay fail closed before disk write. M18B1–B3 did not yet authorize external disclosure or manufacturing export with warnings and did not create a provider/cloud default.

M18B4 extends the same non-canonical authority to the existing host-mediated validator TCP boundary. The scheduler consumes one `SideEffectAuthorizationReceipt` by value before DNS resolution or connection and matches its operation, `ExternalDisclosure` class, exact `tcp://host:port` destination, signed package identity as provider, request SHA-256, and absent file path. Publisher-declared hosts and a separate exact-endpoint host grant remain necessary but cannot substitute for disclosure approval. Missing authorization and payload, provider, destination, operation, or risk substitution fail closed before network I/O; successful egress still returns its separate request/response transport receipt. The authorization receipt was already verified against a current document/revision and human-only one-use approval by `DocumentStore`, so scheduler consumption creates no canonical mutation, revision, Undo step, or alternate `apply_batch` path. This bounded raw-TCP validator proof is not a production provider/cloud default, TLS deployment, credential broker, or retention/cancellation policy.

M18B5 wires `ReleaseManufacturingExportWithWarnings` to the existing Beam M5 release boundary. The application prepares the complete deterministic `.kfm` bytes first, then requests a Beam-document/revision, exact path, operation, and payload-SHA-256-bound human-only receipt before any write. Refusal preserves a pre-existing target byte-for-byte; approval changes neither the active document nor Beam canonical digest/revision or Undo state. Current M5 completeness, validation and lineage checks remain mandatory and cannot be replaced by confirmation. This closes only the trusted side-effect wiring for the existing bounded Beam export; it does not broaden manufacturing semantics or make incomplete/quarantined evidence exportable.

M18C1 broadens the canonical Assistant vocabulary from two dimension setters to one additional production command family: show or hide one root occurrence. The new intent has a distinct capability, exact occurrence target, target-existence assumption, explicit `true`/`false` before/after evidence, the existing bounded Standard-risk review contract, and no mutation during preparation. Missing targets and changes to the target occurrence after review fail closed; confirmation delegates once to `commit_verified_proposal` and therefore to the sole transactional `apply_batch` authority, producing exactly one visible Undo step. This bounded slice does not add natural-language inference, provider access, bulk visibility changes, plugin protocol expansion, or a second commit path.

M18C2 adds one exact occurrence-translation intent to that same path. The capability and Proposal goal are distinct from visibility, the typed value is three bounded finite millimetre inputs, and review exposes the complete canonical Transform before/after rather than reusing visibility evidence for the shared occurrence authority. Preparation remains observational; malformed, non-finite, missing-target, and stale-target requests fail closed. Confirmation executes the existing single `SetOccurrenceTransform` command through `commit_verified_proposal` and canonical `apply_batch`, creates exactly one revision and Undo step, and the running Assistant restores the prior placement through Undo. This is absolute XYZ translation only, not rotation, scale, bulk motion, path planning, or provider inference.

M18C3 adds one definition-rename intent through the already validated `RenameDefinition` canonical command. The intent has a distinct local capability and exact definition target; review exposes the complete old and requested names as typed text evidence rather than an opaque definition digest. Empty or missing targets fail during observational preparation, a concurrent rename makes the reviewed Proposal stale, and confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creating one revision and one Undo step. This bounded slice does not rename occurrences or features, infer names, expand plugin rights, or create a second mutation path.

M18C4 adds one tag-visibility intent through the existing `SetTagVisibility` canonical command. Its distinct local capability and exact tag target produce a target-existence assumption and explicit boolean before/after evidence; preparation changes no revision, digest, or Undo state. Capability denial, malformed UI booleans, missing tags, and changes to the reviewed tag fail closed, while confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch` and produces exactly one undoable revision. This bounded slice does not create, delete, rename, or bulk-edit tags, change occurrence membership, extend plugin capabilities, or add another mutation authority.

M18C5 adds one occurrence tag-assignment/removal intent through the existing `SetOccurrenceTag` canonical command. The separate local capability and exact occurrence target expose an explicit optional-Tag before/after value; assigning a tag adds that exact tag as an authoritative read dependency and target-existence assumption, while removal uses `none` without inventing a dependency. Preparation remains observational, and capability denial, malformed UI input, missing occurrence/tag, or a concurrent change to either reviewed dependency fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior membership. This bounded slice does not create tags, bulk-edit membership, infer classification, expand plugin rights, or add another mutation authority.

M18C6 adds one occurrence-definition intent through the existing `RepointOccurrence` canonical command. Its distinct local capability and exact occurrence write target expose the complete source and requested Definition IDs as typed before/after evidence; the requested Definition is also an authoritative read dependency and target-existence assumption. Preparation remains observational, and capability denial, malformed UI input, missing occurrence/definition, or a concurrent change to either reviewed dependency fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the original definition reference. This bounded slice does not create or clone definitions, repoint in bulk, infer component substitutions, expand plugin rights, or add another mutation authority.

M18C7 adds one occurrence-group assignment/removal intent through the existing `SetOccurrenceParent` canonical command. Its distinct local capability and exact occurrence write target expose an explicit optional Group ID before/after value; assignment binds the requested group ancestry as authoritative read evidence and the exact requested group as a target-existence assumption, while removal uses `none` without inventing a group dependency. Preparation remains observational, and capability denial, malformed UI input, missing occurrence/group, or a concurrent change to reviewed group ancestry fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the original parent. This bounded slice does not create groups, reparent groups, preserve world placement across parenting, bulk-edit membership, infer hierarchy, expand plugin rights, or add another mutation authority.

M18C8 adds one exact group-translation intent through the existing `SetGroupTransform` canonical command. Its distinct local capability and exact group target expose the complete canonical Transform before/after while accepting exactly three bounded finite millimetre inputs and preserving all non-translation matrix entries. Preparation remains observational; capability denial, malformed or non-finite input, missing groups, and concurrent target changes fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior group transform. This bounded slice is absolute XYZ translation only; it does not rotate, scale, reparent, create, delete, or bulk-edit groups, preserve child world placement, infer hierarchy, expand plugin rights, or add another mutation authority.

M18C9 adds one group-parent assignment/removal intent through the existing `SetGroupParent` canonical command. Its distinct local capability and exact group write target expose an explicit optional Group ID before/after value; assignment binds the requested parent and its complete ancestry as authoritative read evidence and the exact requested parent as a target-existence assumption, while removal uses `none` without inventing a parent dependency. Preparation remains observational, and capability denial, malformed UI input, missing target/parent, hierarchy cycles, or concurrent changes anywhere in the reviewed parent ancestry fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the original parent. This bounded slice does not create or delete groups, preserve child world placement across reparenting, bulk-edit hierarchy, infer grouping, expand plugin rights, or add another mutation authority.

M18C10 adds exact non-owning collection membership through the existing `SetCollectionOccurrences` canonical command. Its distinct local capability and exact Collection write target expose the complete strictly increasing occurrence-ID list before/after; every requested occurrence is an authoritative read dependency and target-existence assumption. Preparation remains observational, while capability denial, malformed UI input, missing collection/occurrence, duplicate or non-canonical membership, and concurrent changes to any reviewed occurrence fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the previous membership. This bounded slice does not create, delete, or rename collections, infer selection sets, preserve user ordering, expand plugin rights, or add another mutation authority.

M18C11 adds evaluator-node rename through the existing `RenameEvaluatorNode` canonical command. Its distinct local capability and exact EvaluatorNode target expose the complete old and requested names as typed text evidence while preserving node kind, expression/rule content, ports, outputs, dependencies, and parameter value. Preparation remains observational, and capability denial, empty names, missing nodes, or concurrent changes in the reviewed evaluator dependency closure fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior name. This bounded slice does not create nodes, alter dimensions or expressions, infer labels, expand plugin rights, or add another mutation authority.

M18C12 adds evaluator expression editing through the existing `SetNodeExpression` canonical command. Its distinct local capability and exact EvaluatorNode write target expose the complete old and requested expression source as typed text evidence. The Proposal read set binds the union of the target's existing dependency closure and every dependency closure referenced by the requested expression, so concurrent changes to either side fail closed before commit. Preparation remains observational, while capability denial, malformed expressions, parameter-node targets, missing targets or references, cycles, and stale dependencies fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior expression. This bounded slice does not create nodes, edit rule outputs or ports, infer expressions, expand plugin rights, or add another mutation authority.

M18C13 adds bottle edge-finish kind editing through the existing `SetBottleEdgeFinishKind` canonical command. Its distinct local capability and exact Feature write target expose the complete old and requested `Fillet` or `Chamfer` enum as typed evidence while preserving the target feature and finish amount. Preparation remains observational, while capability denial, malformed UI values, non-bottle-finish features, missing targets, invalid bottle chains, and concurrent changes in the reviewed feature dependency closure fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior finish kind. This bounded slice does not alter finish amount, create bottle features, infer manufacturing intent, expand plugin rights, or add another mutation authority.

M18C14 adds bottle profile-control dimension editing through the existing `SetBottleControlDimension` canonical command. Its distinct local capability and typed Proposal goal identify both the exact Feature write target and one explicit `BodyRadius`, `BodyHeight`, or `ShoulderRise` control, while review exposes the complete old and requested dimension source as typed before/after evidence. Preparation remains observational, and capability denial, malformed UI controls or dimensions, non-control features, missing targets, invalid bottle chains, and concurrent changes in the reviewed feature dependency closure fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior control dimension. This bounded slice changes exactly one existing bottle control; it does not edit multiple controls, create bottle features, infer proportions, expand plugin rights, or add another mutation authority.

M18C15 adds exact ordered profile-point editing through the existing `SetProfilePoints` canonical command. Its distinct local capability and exact Feature write target expose every old and requested 2D millimetre point as typed before/after evidence; the Assistant accepts only deterministic semicolon-separated `x,y` pairs. Preparation remains observational, while capability denial, malformed point text, non-finite, undersized, degenerate, clockwise or self-intersecting profiles, non-profile features, missing targets, and concurrent changes in the reviewed feature dependency closure fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior ordered points. This bounded slice edits one existing profile; it does not create profiles, infer topology, expose unrestricted sketches, expand plugin rights, or add another mutation authority.

M18C16 adds exact rule-output editing through the existing `SetRuleOutputs` canonical command. Its distinct local capability and exact EvaluatorNode write target expose the complete old and requested `Vec<RuleOutput>` trees as typed before/after evidence; the Assistant accepts only deterministic flat semicolon-separated `output_port:semantic_key` entries or `none`, and binds every segment to the target rule node. Preparation remains observational, while capability denial, malformed segments, undeclared ports, parameter or expression targets, missing targets, graph limits, and concurrent changes in the reviewed evaluator dependency closure fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior outputs. This bounded slice edits flat outputs on one existing rule; it does not create rules, expose nested-tree authoring, infer semantic keys, expand plugin rights, or add another mutation authority.

M18C17 adds named collection creation through the existing `CreateCollection` canonical command. Its distinct local capability and exact Collection write target expose typed `Missing` before evidence and the complete requested name as `Text` after evidence, with an explicit `TargetMissing` assumption for the requested ID. Preparation remains observational, while capability denial, zero or already-used IDs, invalid names, and concurrent creation at the reviewed ID fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created collection. This bounded slice creates one empty collection; it does not infer membership, allocate IDs, rename or delete collections, expand plugin rights, or add another mutation authority.

M18C18 adds named empty-definition creation through the existing `CreateDefinition` canonical command. Its distinct local capability and exact Definition write target expose typed `Missing` before evidence and the complete requested name as `Text` after evidence, with an explicit `TargetMissing` assumption for the requested ID. Preparation remains observational, while capability denial, zero or already-used IDs, invalid names, and concurrent creation at the reviewed ID fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created definition. This bounded slice creates one empty definition; it does not create features or occurrences, allocate IDs, rename or delete definitions, expand plugin rights, or add another mutation authority.

M18C19 adds named tag creation through the existing `CreateTag` canonical command. Its distinct local capability and exact Tag write target expose typed `Missing` before evidence and the complete requested name plus explicit visibility as `TagState` after evidence, with an explicit `TargetMissing` assumption for the requested ID. The Assistant accepts only deterministic `true:name` or `false:name` input. Preparation remains observational, while capability denial, malformed UI input, zero or already-used IDs, invalid names, and concurrent creation at the reviewed ID fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created tag. This bounded slice creates one unassigned tag; it does not infer names or visibility, allocate IDs, assign occurrences, rename or delete tags, expand plugin rights, or add another mutation authority.

M18C20 adds named root-group creation through the existing `CreateGroup` canonical command. Its distinct local capability and exact Group write target expose typed `Missing` before evidence and the complete requested `GroupState` after evidence: name, identity Transform, and no parent, with an explicit `TargetMissing` assumption for the requested ID. Preparation remains observational, while capability denial, zero or already-used IDs, invalid names, and concurrent creation at the reviewed ID fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created group. This bounded slice creates one empty root group; it does not infer hierarchy or placement, allocate IDs, create children, rename or delete groups, expand plugin rights, or add another mutation authority.

M18C21 adds named root-occurrence creation through the existing `CreateOccurrence` canonical command. Its distinct local capability and exact Occurrence write target expose typed `Missing` before evidence and the complete requested `OccurrenceState` after evidence: an explicit existing Definition ID, name, identity Transform, no parent, no tag, and visible state, with `TargetMissing` for the requested occurrence ID and `TargetExists` for the definition. The Assistant accepts only deterministic `definition-id:name` input. Preparation remains observational, while capability denial, malformed input, zero or already-used IDs, missing definitions, invalid names, and concurrent creation at the reviewed ID fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the occurrence. This bounded slice creates one visible root occurrence; it does not infer references, hierarchy, tags, visibility, or placement, allocate IDs, create definitions, rename or delete occurrences, expand plugin rights, or add another mutation authority.

M18C22 adds named exact profile-feature creation through the existing `CreateFeature` canonical command. Its distinct local capability binds an unused Feature ID and an existing Definition ID, while review exposes typed `Missing` to complete `ProfileFeatureState` evidence for the feature and exact before/after `DefinitionFeatures` membership for the second authoritative write. A dedicated two-write budget preserves every existing single-write M7A limit. The Assistant accepts only deterministic `definition-id:name:x,y;x,y;…` input; canonical profile validation rejects zero or reused IDs, missing definitions, invalid names, non-finite, undersized, degenerate, clockwise, or self-intersecting point loops. Preparation remains observational, concurrent target or definition changes fail stale, and confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creating one Undo step that removes both the feature and its definition membership. This bounded slice creates one explicit profile; it does not infer geometry, allocate IDs, create definitions or downstream solids, expand plugin rights, loosen other budgets, or add another mutation authority.

M18C23 adds deletion of one exact collection through the existing `DeleteCollection` canonical command. Its distinct local capability and exact Collection write target expose the complete reviewed `CollectionState` before evidence—name plus canonical ordered occurrence membership—and typed `Missing` after evidence, with an explicit `TargetExists` assumption. Preparation remains observational, while capability denial, missing targets, and concurrent deletion or membership changes fail closed. Confirmation remains a Standard-risk explicit review because this slice deletes one non-owning collection rather than performing destructive bulk change; it delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores both name and membership. This bounded slice does not delete occurrences, infer targets, perform bulk deletion, allocate IDs, expand plugin rights, or add another mutation authority.

M18C24 adds deletion of one exact empty group through the existing `DeleteGroup` canonical command. Its distinct local capability and exact Group write target expose the complete reviewed `GroupState` before evidence—name, Transform, and optional parent—and typed `Missing` after evidence, with an explicit `TargetExists` assumption. Preparation remains observational, while capability denial, missing targets, non-empty groups, and concurrent deletion or child-membership changes fail closed through the canonical group-child dependency closure. Confirmation remains a Standard-risk explicit review because this slice deletes one empty hierarchy container rather than children or a destructive bulk selection; it delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact group state. This bounded slice does not detach or delete occurrences/groups, infer targets, perform bulk deletion, allocate IDs, expand plugin rights, or add another mutation authority.

M18C25 adds deletion of one exact unassigned tag through the existing `DeleteTag` canonical command. Its distinct local capability and exact Tag write target expose the complete reviewed `TagState` before evidence—name and visibility—and typed `Missing` after evidence, with an explicit `TargetExists` assumption. Preparation remains observational, while capability denial, missing targets, tags assigned to either world or definition-local occurrences, reviewed-tag changes, and concurrent assignment fail closed without mutation; assignment is rechecked by the canonical command even when the Tag value itself is unchanged. Confirmation remains a Standard-risk explicit review because this slice deletes one unassigned display tag rather than occurrences or a destructive bulk selection; it delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact tag state. This bounded slice does not detach occurrences, infer targets, perform bulk deletion, allocate IDs, expand plugin rights, or add another mutation authority.

M18C26 adds deletion of one exact world occurrence through the existing `DeleteOccurrence` canonical command. Its distinct local capability and exact Occurrence write target expose the complete reviewed `OccurrenceState` before evidence—definition, name, Transform, optional parent, optional tag, and visibility—and typed `Missing` after evidence, with an explicit `TargetExists` assumption. Preparation remains observational, while capability denial, missing targets, occurrences retained by a collection, reviewed-occurrence changes, and collection assignment after review fail closed without mutation through the canonical occurrence-collection dependency closure. Confirmation remains a Standard-risk explicit review because this slice deletes one uncollected world occurrence rather than a bulk selection or owned definition; it delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact occurrence state. This bounded slice does not remove collection membership, delete definitions or groups, infer targets, perform bulk deletion, allocate IDs, expand plugin rights, or add another mutation authority.

M18C27 adds deletion of one exact unused profile feature through the existing `DeleteFeature` canonical command. Its distinct local capability binds the exact Feature write target and the owning Definition write target, while review exposes the complete `ProfileFeatureState` before evidence, typed `Missing` after evidence, and exact before/after `DefinitionFeatures` membership. The existing dedicated two-write feature budget preserves every single-write M7A limit. Preparation remains observational, while capability denial, missing or non-profile targets, downstream feature use, reviewed-feature changes, and concurrent Definition membership changes fail closed through canonical validation and the feature-user dependency closure. Confirmation remains a Standard-risk explicit review because this slice deletes one unused feature rather than a definition or destructive bulk selection; it delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores both the profile and its exact Definition membership. This bounded slice does not delete downstream features or definitions, infer targets, perform bulk deletion, allocate IDs, expand plugin rights, or add another mutation authority.

M18C28 adds deletion of one exact empty and unreferenced definition through the existing `DeleteDefinition` canonical command. Its distinct local capability and exact Definition write target expose the complete reviewed `DefinitionState` before evidence—name plus feature, definition-local occurrence, and definition-local group membership—and typed `Missing` after evidence, with an explicit `TargetExists` assumption. Preparation remains observational and accepts only an empty definition; capability denial, missing targets, any definition-local content, world-occurrence use, reviewed-state changes, and new users after review fail closed through the exact Definition and `DefinitionUsers` dependency evidence. Confirmation remains a Standard-risk explicit review because this slice deletes one empty unused container rather than owned geometry, instances, or a destructive bulk selection; it delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact empty definition. This bounded slice does not cascade-delete features or occurrences, infer targets, perform bulk deletion, allocate IDs, expand plugin rights, or add another mutation authority.

M18C29 adds creation of one named independent scalar evaluator input through the existing `CreateEvaluatorNode` canonical command. Its distinct local capability and exact EvaluatorNode write target expose typed `Missing` before evidence and the complete requested `EvaluatorInputState` after evidence—name, exact source-preserving Dimension, and an explicit empty dependency list—with a `TargetMissing` assumption for the requested ID. The Assistant accepts only deterministic `name:value` input. Preparation remains observational, while capability denial, malformed UI input, zero or already-used IDs, invalid names or dimensions, and concurrent creation at the reviewed ID fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created input. This bounded slice does not infer dependencies or units, create expression/rule nodes, allocate IDs, expand plugin rights, or add another mutation authority.

M18C30 adds creation of one named evaluator expression through the existing `CreateExpressionNode` canonical command. Its distinct local capability and exact EvaluatorNode write target expose typed `Missing` before evidence and the complete requested `EvaluatorExpressionState` after evidence—name, exact expression source, and the canonical dependency list—with a `TargetMissing` assumption for the requested ID and `TargetExists` assumptions for every referenced evaluator. The Proposal read set binds every referenced evaluator's complete dependency closure, so a concurrent source change after review fails stale. The Assistant accepts only deterministic `name:expression` input. Preparation remains observational, while capability denial, malformed UI input, zero or already-used IDs, empty names, malformed expressions, missing references, cycles, graph limits, concurrent creation, and stale dependencies fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created expression. This bounded slice does not infer names or formulas, create rule nodes, allocate IDs, expand plugin rights, or add another mutation authority.

M18C31 adds creation of one named flat evaluator rule through the existing `CreateRuleNode` canonical command. Its distinct local capability and exact EvaluatorNode write target expose typed `Missing` before evidence and the complete requested `EvaluatorRuleState` after evidence—name, exact expression source, canonical dependencies, no input ports, one numeric `result` output port, no output identities, and no override parameters—with a `TargetMissing` assumption for the requested ID and `TargetExists` assumptions for every referenced evaluator. The Proposal read set binds every referenced evaluator's complete dependency closure, so a concurrent source change after review fails stale. The Assistant accepts only deterministic `name:expression` input. Preparation remains observational, while capability denial, malformed UI input, zero or already-used IDs, empty names, malformed expressions, missing references, cycles, graph limits, concurrent creation, and stale dependencies fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created rule. This bounded slice does not infer names or formulas, create nested outputs or override parameters, allocate IDs, expand plugin rights, or add another mutation authority.

M18C32 adds creation of one resolved flat rule override through the existing `UpsertOverride` canonical command. Its distinct local capability and exact Override write target expose typed `Missing` before evidence and the complete requested `RuleOverrideState` after evidence—one-segment `DerivedIdentity`, declared parameter, finite value, and `Resolved` health—with `TargetMissing` for the override ID and `TargetExists` for the root rule. The Assistant accepts only deterministic `rule-id:output-port:semantic-key:parameter:value` input. Preparation remains observational and binds the root rule's evaluator dependency closure; capability denial, malformed or non-finite values, zero or occupied override IDs, missing or ambiguous output identities, undeclared parameters, and concurrent rule changes fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the override. This bounded slice does not replace existing overrides, author nested output paths, infer parameters or identities, allocate IDs, expand plugin rights, or add another mutation authority.

M18C33 adds deletion of one reviewed rule override through the existing `DeleteOverride` canonical command. Its distinct local capability and exact Override write target expose the complete current `RuleOverrideState` before evidence—derived identity, parameter, finite value, and current resolution health—and typed `Missing` after evidence, with `TargetExists` for the exact override ID. Preparation remains observational, while capability denial, missing IDs, and concurrent replacement or removal after review fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact override. This bounded slice does not delete evaluator rules, infer override IDs, expand plugin rights, or add another mutation authority.

M18C34 adds creation of one resolved feature-parameter binding through the existing `UpsertFeatureParameterBinding` canonical command. Its distinct local capability and exact composite FeatureParameterBinding write target expose typed `Missing` before evidence and the complete requested `FeatureParameterBindingState` after evidence—exact feature/slot target plus one-segment `DerivedIdentity`—with `TargetMissing` for the binding and `TargetExists` for both the feature and root rule. The Assistant accepts only deterministic `slot:rule-id:output-port:semantic-key` input for an exact feature ID. Preparation remains observational and binds the feature and evaluator dependency closures; capability denial, malformed input, unsupported feature slots, missing features or rules, unresolved output identities, occupied binding targets, and concurrent feature or evaluator changes fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the binding. This bounded slice does not replace existing bindings, recompute feature values, infer nested output paths, expand plugin rights, or add another mutation authority.

M18C35 adds deletion of one reviewed feature-parameter binding through the existing `DeleteFeatureParameterBinding` canonical command. Its distinct local capability and exact composite FeatureParameterBinding write target expose the complete current `FeatureParameterBindingState` before evidence—exact feature/slot target and `DerivedIdentity`—and typed `Missing` after evidence, with `TargetExists` for the exact binding. The Assistant accepts only one deterministic canonical slot name for an exact feature ID. Preparation remains observational, while capability denial, malformed slots, missing bindings, and concurrent replacement or removal after review fail closed without overwriting newer state. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact binding. This bounded slice does not delete evaluator rules or features, recompute parameter values, infer targets, expand plugin rights, or add another mutation authority.

M18C36 adds explicit recomputation of one exact bound feature parameter through the existing `RecomputeFeatureParameters` canonical command. Its distinct local capability and exact Feature write target expose the complete old and evaluated `Dimension` as typed before/after evidence, while the exact FeatureParameterBinding, feature dependency closure, and evaluator dependency closure are authoritative reads. The Assistant accepts one deterministic canonical slot name for an exact feature ID and uses the canonical default `EvaluationIdentity`. Preparation remains observational and is deliberately limited to a document containing exactly that one binding, so the global canonical command cannot hide additional writes behind a single-target goal; capability denial, malformed slots, missing or multiple bindings, failed evaluation, invalid dimensions, and concurrent feature, binding, or evaluator changes fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the prior parameter. This bounded slice does not batch-recompute multiple bindings, accept caller-selected evaluator identities, infer targets, expand plugin rights, or add another mutation authority.

M18C37 adds deletion of one reviewed canonical joint through the existing `DeleteJoint` command. Its distinct local capability and exact Joint write target expose the complete current `JointState` before evidence—both `DerivedIdentity` participants and the exact AABB minimum and maximum—and typed `Missing` after evidence, with `TargetExists` for the exact joint ID. Preparation remains observational, while capability denial, missing IDs, and concurrent joint replacement or removal after review fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact joint. This bounded slice does not create or infer joints, alter participants or joint geometry, expand plugin rights, or add another mutation authority.

M18C38 adds deletion of one reviewed canonical space through the existing `DeleteSpace` command. Its distinct local capability and exact Space write target expose the complete current `SpaceState` before evidence—purpose, exact AABB minimum and maximum, and canonical adjacency and accessibility lists—and typed `Missing` after evidence, with `TargetExists` for the exact space ID. Preparation remains observational, while capability denial, missing IDs, and concurrent space replacement or removal after review fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact space. This bounded slice does not create or infer spaces, rewrite relationships, expand plugin rights, or add another mutation authority.

M18C39 adds deletion of one reviewed canonical clearance volume through the existing `DeleteClearanceVolume` command. Its distinct local capability and exact ClearanceVolume write target expose the complete current `ClearanceVolumeState` before evidence—owner, semantic reason, exact AABB minimum and maximum, coordinate frame, tolerance, severity, and optional `DerivedIdentity`—and typed `Missing` after evidence, with `TargetExists` for the exact clearance-volume ID. Preparation remains observational, while capability denial, missing IDs, and concurrent clearance replacement or removal after review fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact clearance volume. This bounded slice does not create or infer clearance volumes, alter occupancy validation, expand plugin rights, or add another mutation authority.

M18C40 adds deletion of one reviewed persistent dimension through the existing `DeletePersistentDimension` command. Its distinct local capability and exact PersistentDimension write target expose the complete current `PersistentDimensionState` before evidence—name, exact target variant, display unit, and decimal precision—and typed `Missing` after evidence, with `TargetExists` for the exact persistent-dimension ID. Preparation remains observational, while capability denial, missing IDs, and concurrent dimension replacement or removal after review fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the exact persistent dimension. This bounded slice does not create or infer dimensions, alter referenced geometry or evaluator state, expand plugin rights, or add another mutation authority.

M18C41 adds creation of one named persistent dimension for an exact existing feature parameter through the existing `UpsertPersistentDimension` command. Its distinct local capability and exact PersistentDimension write target expose typed `Missing` before evidence and the complete requested `PersistentDimensionState` after evidence—name, exact `FeatureParameterTarget`, display unit, and decimal precision—with `TargetMissing` for the persistent-dimension ID and `TargetExists` for the referenced feature. The Assistant accepts only deterministic `name:feature-id:slot:mm|cm|in:precision` input. Preparation remains observational and binds the referenced feature dependency closure, while capability denial, malformed input, zero or occupied dimension IDs, invalid names, unsupported or missing feature parameters, invalid presentation precision, concurrent creation at the reviewed ID, and stale feature changes fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created persistent dimension. This bounded slice does not replace existing dimensions, infer geometry or targets, author derived-output or exact-reference targets, alter referenced geometry or evaluator state, expand plugin rights, or add another mutation authority.

M18C42 adds creation of one exact empty-relation canonical space through the existing `UpsertSpace` command. Its distinct local capability and exact Space write target expose typed `Missing` before evidence and the complete requested `SpaceState` after evidence—purpose, exact AABB minimum and maximum, and explicit empty adjacency and accessibility lists—with `TargetMissing` for the space ID. The Assistant accepts only deterministic `purpose:minx,miny,minz:maxx,maxy,maxz` input. Preparation remains observational, while capability denial, malformed or non-finite coordinates, zero or occupied space IDs, invalid purpose or AABB, concurrent creation at the reviewed ID, and stale replacement fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created space. This bounded slice does not replace existing spaces, author adjacency or accessibility relationships, infer semantic purpose or volume, create clearance volumes, expand plugin rights, or add another mutation authority.

M18C43 adds creation of one exact world-frame clearance volume for an existing exact space owner through the existing `UpsertClearanceVolume` command. Its distinct local capability and exact ClearanceVolume write target expose typed `Missing` before evidence and the complete requested `ClearanceVolumeState` after evidence—space owner, semantic reason, exact AABB minimum and maximum, world coordinate frame, tolerance, severity, and explicit absence of a derived identity—with `TargetMissing` for the clearance-volume ID and `TargetExists` for the owner Space. The Assistant accepts only deterministic `owner-space-id:reason:minx,miny,minz:maxx,maxy,maxz:tolerance:advisory|required` input. Preparation remains observational and binds the owner dependency, while capability denial, malformed or non-finite values, zero or occupied clearance-volume IDs, missing owners, invalid reason, AABB, or tolerance, concurrent creation at the reviewed ID, stale owner changes, and stale replacement fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created clearance volume. This bounded slice does not replace existing clearance volumes, author occurrence-owned or derived clearance, infer owners or geometry, alter occupancy validation, expand plugin rights, or add another mutation authority.

M18C44 adds creation of one canonical joint between two exact resolved one-segment rule outputs through the existing `UpsertJoint` command. Its distinct local capability and exact Joint write target expose typed `Missing` before evidence and the complete requested `JointState` after evidence—both `DerivedIdentity` participants and exact AABB minimum and maximum—with `TargetMissing` for the joint ID and `TargetExists` for each distinct participant root rule. The Assistant accepts only deterministic `rule-id,output-port,semantic-key:rule-id,output-port,semantic-key:minx,miny,minz:maxx,maxy,maxz` input. Preparation remains observational, verifies both output slots as resolved, and binds both evaluator dependency closures, while capability denial, malformed or non-finite values, zero or occupied joint IDs, duplicate or unresolved participants, invalid AABB, concurrent creation at the reviewed ID, and stale participant-rule changes fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo removes the created joint. This bounded slice does not replace existing joints, infer participants or geometry, create nested output paths, alter collision validation, expand plugin rights, or add another mutation authority.

M18C45 adds a bounded one-profile-feature `CloneDefinitionAndRepoint` Proposal. The Assistant requires deterministic `source-definition-id:source-feature-id:new-definition-id:new-feature-id:new-definition-name` input plus the exact occurrence target; no ID or mapping is inferred. The source occurrence must reference the exact source definition, whose canonical content must be exactly one Profile feature with no local groups, local occurrences, or feature-parameter bindings, so the complete mutation is the declared three-target write set: the existing occurrence, new definition, and new feature. Review exposes complete typed `OccurrenceState`, `DefinitionState`, and `ProfileFeatureState` before/after evidence, with `TargetExists` for the occurrence/source definition/source feature and `TargetMissing` for both new IDs. Preparation remains observational; malformed input, capability denial, unsupported source shape, missing or mismatched source identities, invalid names or IDs, occupied IDs, source changes, occurrence changes, and concurrent target claims fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the original occurrence and removes the cloned definition/profile. This slice does not clone arbitrary feature graphs, local entities, parameter bindings, infer mappings, expand plugin rights, or add another mutation authority.

M18C46 adds a bounded empty-group `ConvertGroupToComponent` Proposal. The Assistant requires deterministic `new-definition-id:new-occurrence-id:component-name` input plus the exact group target; no ID or mapping is inferred. The target must exist and have no child groups or occurrences, so the complete mutation is the declared three-target write set: the exact group subtree, new empty definition, and new root-or-sibling occurrence. Review exposes typed `GroupState -> Missing`, `Missing -> DefinitionState`, and `Missing -> OccurrenceState` evidence, with `TargetExists` for the exact group subtree and `TargetMissing` for both new IDs. Preparation remains observational; malformed input, capability denial, missing or non-empty groups, invalid names or IDs, occupied IDs, stale subtree changes, and concurrent target claims fail closed. Confirmation delegates once through `commit_verified_proposal` to canonical `apply_batch`, creates one Undo step, and Undo restores the group and removes the component definition and occurrence. This slice does not migrate child entities, collections, tags, or arbitrary subtrees, infer IDs, expand plugin rights, or add another mutation authority.

M19A1 adds a complete Slovak `sk-SK` resource beside the English source and a deterministic generated pseudo-locale. All three catalogs expose the same full key set; strict completeness validation rejects both missing and unexpected keys rather than silently certifying fallback text. Pseudo-localization expands and accents visible text while preserving argument placeholders exactly, and the shell accepts an injected complete catalog so the same command registry and AccessKit names can be exercised under real and pseudo translations. This slice establishes resource completeness only; layout, keyboard/focus, contrast, and screen-reader evidence remain required before G19-01 closes.

M19A2 closes the remaining G19-01 evidence against the real shell widget tree. English, Slovak, and pseudo-localized shells retain a positive viewport and keep every top-level localized menu inside the 1600 x 1000 acceptance viewport. Their icon-only production tools remain discoverable by localized AccessKit names; the same AccessKit focus action reaches Rectangle in every locale and keyboard Enter dispatches the localized command through the normal registry path. The production palette now names its panel, viewport, text, brand, and focus colors explicitly, uses dark text instead of failing white text on the orange brand mark, and executable WCAG calculations enforce at least 4.5:1 for normal text and 3:1 for the focus indicator. This is deterministic offscreen AccessKit/layout and palette evidence, not a physical assistive-technology certification on release hardware.

M19B1 adds a fail-closed Windows x86-64 technical release-candidate packager. It stages the optimized desktop application and co-located exact worker with the exact OCCT DLL set recorded in the pinned R0 manifest, verifies every source and packaged size/SHA-256, writes an allowlist manifest bound to `Cargo.lock` and the OCCT build fingerprint, and rejects missing, extra, duplicate, path-escaping, or modified runtime files. The adversarial packaging test constructs and verifies the bundle, executes the packaged worker through a real `PING`/`PONG` process boundary, and proves rejection of both an unrecorded DLL and a one-byte mutation. M19B2 additionally launches the packaged GUI from an empty working directory outside both the repository and package, requires a real product window, enumerates the live process modules, and rejects any pin-listed OCCT module resolved outside the package; foundational `TKernel.dll` must be observed as co-located live-process evidence. The manifest remains deliberately `release_eligible=false` with `blocking_decision=V4-O08`: this technical candidate does not close G19-02 until a named owner records the platform decision and physical release-hardware evidence exercises the native New/Open/Save/Save As dialogs and atomic failure continuity.

M19B3 adds an interactive physical-operator runner for that remaining workflow evidence. It first verifies the exact technical package, launches it from a foreign working directory, and requires named human observation of native Save As, Save, New, Open, malformed-Open continuity, and locked-destination Save failure. Objective file checks require byte-identical Open/Save As continuity, unchanged canonical bytes after both failures, an untouched locked destination, and a still-live product process after every step. A completed run is written once as an exact allowlisted SHA-256 bundle; its verifier rejects changed or extra artifacts, missing/duplicate steps, and any inferred V4-O08 or release-eligible claim. The runner and its contract test are implementation readiness only: no physical run has been captured here, and the owner decision remains open. M19B64 removes the failed-input dialog-observation path-probe race: malformed Open is held by one read handle that denies write/delete from its pre-dialog identity/size/SHA-256 snapshot through its post-dialog snapshot, while locked Save takes both snapshots from the same already-exclusive handle; exact evidence rejects an unguarded observation claim. M19B65 extends those same original handles through immutable-bundle creation, rechecks identity/size/SHA-256 immediately before copy, and copies both failed-operation inputs directly from their guarded streams; exact verification rejects a replaced object or changed bytes at the bundle boundary. M19B66 applies the same direct-stream boundary to all five successful Save outputs: each final filesystem object is identity/size/SHA-256 checked under a write/delete-blocking handle and copied from that exact stream into a create-new bundle artifact, while exact evidence rejects any path-copy claim. M19B67 makes the equivalent method claim explicit for both failed-operation inputs: malformed Open and locked Save must record `guarded-source-stream`, and exact verification rejects a path-copy declaration even when the remaining identity, size, SHA-256, and guard fields are plausible. M19B68 verifies every one of those seven stream copies before closing its exclusive read/write destination handle: the captured destination size and SHA-256 must exactly match the guarded source, exact-schema verification binds both values to the immutable artifact, and deliberate-red fixtures reject a forged successful-Save or failed-input destination digest. M19B69 hardens the offline evidence boundary itself: evidence and provenance JSON are size-checked before allocation, read under a write/delete-blocking handle, decoded as strict UTF-8, and only then deserialized; deliberate-red fixtures reject both an oversized evidence manifest and malformed UTF-8. M19B70 applies the same bounded-read discipline to native document evidence: each `.ketchup` container is size-checked before allocation and read completely from one write/delete-blocking handle before structural parsing; a deliberate-red sparse artifact over the 64 MiB envelope is rejected before allocation. M19B71 binds each trusted provenance digest to the exact guarded bytes that are parsed: the captured package manifest and pinned R0 OCCT manifest now compute SHA-256 from the same bounded write/delete-blocking read used for strict UTF-8 JSON deserialization, eliminating the path-hash/reopen gap; the source contract rejects its reintroduction and the deliberate-red forged package-manifest digest must fail at that boundary. M19B72 keeps the complete exact offline evidence bundle under read handles that deny write and delete for the entire verification interval, and requires the already-parsed evidence manifest hash to match its retained handle before trusting any claim; the focused contract pauses packaged-app reinspection and proves that a concurrent write to a guarded native artefact is denied. M19B73 makes those retained handles the sole byte authority for offline artifact fingerprints, captured package-manifest parsing, bounded native-document parsing, executable comparisons, exact failure sentinels, and terminal continuity checks; the source contract rejects reintroduced bundle path hashing or whole-file path reads after guard acquisition. M19B74 closes the equivalent time-of-check/time-of-use window in the external technical package supplied for offline reinspection: after the canonical package verifier passes, the evidence verifier acquires write/delete-blocking read handles for the exact complete 50-file registry, revalidates the manifest and every recorded size/SHA-256 under those retained handles, and keeps them through exact-worker PING/PONG plus both packaged-app semantic inspections. The focused contract pauses an inspection and proves that a concurrent write to a guarded OCCT runtime is denied; this secures verifier input stability only and does not replace the named physical dialog run. M19B75 removes the last redundant path-hash reopen after those package guards are acquired: comparisons of the verification package's manifest, desktop application, and exact worker now use the retained guarded streams as their sole byte authority before process probes. The source contract rejects reintroduced path hashing for any of those three files; executable launch remains necessarily path-based under the still-held complete package guard, and this hardening does not replace the named physical dialog run.

M19B4 closes a fail-closed provenance gap in the physical-dialog evidence verifier. Verification now recomputes the recorded package-manifest and runner SHA-256 values, binds the captured package manifest to the current `Cargo.lock` and pinned R0 OCCT source/build identity, and requires the exact desktop application, exact-worker, and complete pinned DLL role set with the frozen DLL sizes and hashes. The adversarial contract no longer relies on a minimal synthetic package declaration and proves rejection of a package detached from the current lockfile and of a forged package-manifest provenance hash. This strengthens evidence integrity only; it does not substitute for the named physical run or infer V4-O08.

M19B5 makes the physical-hardware claim itself fail closed. The immutable verifier now requires a bounded run identifier, parseable UTC capture and per-step timestamps, complete manufacturer/model/memory/OS identity, and at least one complete GPU name/driver/status record. Its adversarial contract proves rejection of an unsafe run identifier, incomplete machine identity, missing GPU inventory, malformed capture time, and non-UTC step time. These checks validate evidence structure only; no synthetic fixture is physical certification, and G19-02 remains open until the named release-hardware workflow and V4-O08 owner decision are real.

M19B6 binds the six physical file-dialog attestations to their required execution sequence and chronology. Verification now rejects a reordered step list, equal or decreasing confirmation times, and any step timestamp later than the bundle capture time; the adversarial contract exercises all three red paths. This prevents an unordered collection of independently plausible attestations from being presented as the prescribed continuity workflow, but it still does not replace the named physical run or infer V4-O08.

M19B7 binds the immutable physical-dialog bundle to the exact desktop application and exact-worker bytes named by the verified technical-package manifest. The runner captures both binaries alongside the workflow artifacts, and offline verification requires their measured sizes and SHA-256 values to match the package records; the adversarial contract rewrites the evidence hashes around a substituted executable and still proves rejection against package provenance. This removes a gap where a self-consistent evidence manifest could retain only an unattested declaration of the tested executables, but it does not substitute for the named physical run or infer V4-O08.

M19B8 removes a package time-of-check/time-of-use gap from the physical workflow. The runner now copies the exact allowlisted technical package into a fresh staging root, verifies that isolated snapshot before launch, runs the GUI only from that snapshot, records every observed pin-listed OCCT module after requiring foundational `TKernel.dll` from the snapshot root, and verifies the complete snapshot again after process exit. Offline evidence validation binds the workflow chronology between those two package checks and requires each live-module record to match a pinned package entry exactly. The adversarial contract rejects a non-isolated run, verification performed too late, a missing `TKernel.dll` observation, and an external load origin; this remains implementation-readiness evidence rather than the missing named physical run or V4-O08 owner decision.

M19B9 closes the remaining live-runtime observation window inside that physical workflow. The runner now enumerates the product process both before the first dialog step and after the final failure-continuity step, requires foundational `TKernel.dll` in both observations, merges any pin-listed modules loaded during the workflow, and accepts each only from its exact flat package-root path with the pinned size and SHA-256. Immutable evidence records both observation phases and timestamps inside the pre-launch/post-exit package-verification interval; the verifier rejects a missing final phase, a nested-path attribution, or a final scan not strictly after all six workflow confirmations. This strengthens runtime provenance only and still does not substitute for the named physical run or infer V4-O08.

M19B10 makes the immutable evidence and technical-package declarations exact-schema rather than merely checking known values. The packager and evidence verifier now reject every unknown property at the evidence, operator, machine/GPU, package-snapshot, workflow-step, artifact, loaded-module, package-provenance, OCCT-provenance, and package-file levels. Deliberate-red fixtures prove rejection of shadow `platform_decision` fields in both manifests and an `automated=true` field on a physically attested step. This prevents claim smuggling beside otherwise valid fields; it does not authenticate the human identity, replace the physical run, or infer V4-O08.

M19B11 makes live OCCT discovery exhaustive over the product process's `TK*.dll` modules instead of silently filtering observations through the known-name set. Both the release-candidate smoke test and physical-evidence runner now require every observed OCCT-shaped module name to resolve case-sensitively to exactly one `pinned-occt-runtime` package record before checking its exact root path, size, and SHA-256; any unpinned or ambiguously named module fails closed. The immutable-evidence deliberate-red contract also rejects a declared `TKShadow.dll` absent from the package registry. This closes an omission path in runtime provenance but still does not substitute for the named physical run or infer V4-O08.

M19B12 makes the technical package file registry an exact case-sensitive allowlist rather than accepting arbitrary self-declared non-DLL payload records. Verification derives the only permitted names and roles from the two canonical executables plus the pinned R0 OCCT runtime, requires exact lowercase SHA-256 and positive sizes, and compares the physical package contents against that same complete set. Deliberate-red coverage proves rejection even when an extra payload is accompanied by a self-consistent manifest record and when the desktop application role differs only by case. This closes package-registry claim expansion only; it does not replace the named physical run or infer V4-O08.

M19B13 binds both technical-package construction and offline physical-evidence verification to the immutable SHA-256 of the R0 OCCT build manifest recorded by preregistration. The technical package carries that exact manifest digest in its OCCT provenance record, while both verifier paths independently require the current R0 manifest bytes to retain the frozen digest before trusting its DLL registry. Deliberate-red coverage rejects a package whose otherwise plausible OCCT provenance declaration is detached from the frozen manifest. This prevents a self-consistent replacement of the OCCT manifest and runtime tree from silently redefining the release baseline; it is not an external publisher signature, a named physical run, or a V4-O08 owner decision.

M19B14 closes a native-document evidence gap: hash equality alone could prove continuity between arbitrary bytes without proving that successful Save/Open artifacts retained the current Ketchup persistence envelope. Offline verification now parses every successful and post-failure `.ketchup` artifact as a bounded schema-1 `KETCHUPCTR` container with one required schema-17 `KETCHUPDOC` entry, exact entry boundaries, and matching container plus canonical-payload SHA-256 values. Deliberate-red fixtures reject both a manifest rewritten around a self-consistent plain-text baseline and a changed canonical payload hidden behind repaired outer container/evidence hashes. This structural validation complements the physical product workflow; it does not replace full semantic Open, authenticate the operator, or infer V4-O08.

M19B15 binds those native-document artifacts to the exact current schema-17 semantic manifest rather than accepting any self-consistent envelope. Offline verification requires the exact bounded manifest length and the case-sensitive `ketchup.graph.schema.v1`, `ketchup.evaluator.numeric.v1`, and `ketchup.tolerance.r0-v1` identifiers with no trailing manifest bytes. A deliberate-red fixture substitutes the graph-schema identifier, repairs the enclosing container and evidence hashes, and remains rejected. This proves manifest identity only; it does not semantically deserialize the document, replace the named physical workflow, authenticate the operator, or infer V4-O08.

M19B16 records the project owner's explicit V4-O08 decision in ADR 0007: the first release targets Windows x86-64 only, while macOS and Linux are deferred. Technical-package and physical-evidence manifests now carry the exact decision and ADR path instead of claiming that V4-O08 remains open. They remain `release_eligible=false`: the technical package still requires the physical G19-02 dialog workflow, and a completed physical-dialog bundle still leaves G19-03 canonical tasks and G19-04 current-tree hardware certification. This decision removes parallel-platform work from the first-release critical path; it does not claim that Ketchup already works end to end.

M19B17 prevents an empty or unchanged document cycle from satisfying the physical New evidence. The named operator must first create a visible exact Rectangle→Push/Pull model before Save As, and the immutable verifier requires the subsequently saved New document to differ from that modeled baseline while preserving the existing byte-identical Open and failure-continuity requirements. A deliberate-red fixture copies the modeled baseline over the New artifact, repairs the evidence manifest, and remains rejected. This closes an objective workflow-distinctness gap only; it does not replace the named physical run or claim broad modeling coverage.

M19B18 strengthens that New distinction at the canonical payload boundary. The verifier now returns the validated schema-17 canonical payload digest for each successful document and requires the New digest to differ from the physically modeled baseline, rather than trusting only the outer container hash. A deliberate-red fixture copies the baseline canonical document and adds a valid optional sidecar entry so that the container hash changes while canonical state remains identical; the evidence still fails closed. This preserves supported optional extensions while preventing sidecar-only divergence from certifying New, and it still does not replace the named physical run.

M19B19 binds the physical workflow to read-only semantic inspection performed by the exact packaged desktop executable. After the interactive process exits, that same binary losslessly opens the captured schema-17 baseline and New files and reports each container SHA-256, canonical digest, document/revision identities, and exact counts of definitions, root occurrences, profiles, extrusions, and definitions containing a valid Profile→Extrusion chain. Immutable verification binds each reported container digest back to the captured artifact and requires the modeled baseline to contain at least two such chains and root occurrences, while New must restore the exact one-chain initial envelope with a distinct document identity and lower revision. A deliberate-red fixture preserves every document byte and evidence hash but claims only the initial model for the baseline, and remains rejected. This objectively binds the saved files to the requested additional Rectangle→Push/Pull model, but the named human must still perform and attest the native-dialog run.

M19B20 closes the remaining visibility ambiguity in that semantic evidence. Packaged-app inspection now intersects valid Profile→Extrusion definition identities with the effective visible root scene query, so hidden occurrences, hidden tags, and non-root nested instances cannot satisfy the physical instruction that the additional model remain visible before Save As. Immutable verification requires at least two such visible roots in the modeled baseline and exactly one in the fresh New document; a deliberate-red fixture keeps every other semantic count plausible but marks the additional modeled root hidden and remains rejected. This read-only check does not mutate Open state or replace the named physical interaction.

M19B21 makes the atomic failed-Save continuity guard load-bearing in the offline adversarial contract. A deliberate-red fixture replaces the post-failure artifact with a different structurally valid schema-17 document and repairs every recorded artifact fingerprint, so it reaches the terminal byte-continuity check rather than failing an earlier hash or container check; immutable verification still rejects it because the preserved document differs from the physically modeled baseline. This proves fail-closed regression coverage for the evidence verifier only and does not replace the required named physical locked-destination Save run.

M19B22 cryptographically binds each packaged-app canonical-digest declaration to the independently parsed schema-17 payload in its captured baseline or New artifact. The immutable verifier now recomputes the lowercase canonical SHA-256 from the native container and rejects a semantic inspection block whose otherwise well-formed digest is forged; the positive fixture carries real payload digests and a deliberate-red fixture proves the exact rejection path. This closes an offline-checkable inspection-provenance gap only and does not authenticate the human operator or replace the named physical workflow.

M19B23 makes every packaged-app semantic field re-executable instead of trusting plausible counts copied into the evidence manifest. Verification requires the supplied technical-package manifest and desktop executable to match the captured package byte-for-byte, then invokes that executable read-only against the captured baseline and New artifacts and compares all eleven exact semantic fields. A deliberate-red fixture inflates every baseline count consistently above the required threshold while retaining valid document hashes and relationships; only the independent reinspection rejects it. Reverification therefore requires the exact technical package (including its co-located runtime) as well as the immutable evidence directory, and still does not authenticate the named human or replace the physical workflow.

M19B24 binds the physical release-dialog run to a real exact-worker process boundary on the same isolated technical-package snapshot. Before the GUI workflow starts, the runner sends exact `PING`, requires exact `PONG` with exit code zero, and records its UTC observation strictly between pre-launch package verification and live GUI runtime-module observation. Offline verification requires the supplied worker to match the captured worker byte-for-byte and repeats that process probe; deliberate-red fixtures reject both a forged response and a probe timestamp outside the required execution interval. This proves worker executability and package identity for the evidence path, but the named physical dialog workflow itself remains outstanding.

M19B25 makes the package supplied for offline semantic reinspection pass the complete canonical technical-package verifier before any captured artifact is trusted. Verification now requires the exact manifest, desktop application, worker, complete pinned OCCT DLL registry and no extra payloads, rather than comparing only the three evidence-captured package files before launching the inspector. The adversarial contract uses a separate complete 50-file verification package and proves rejection of a changed `TKernel.dll` even while the immutable evidence bundle and package manifest remain unchanged. This closes a reinspection-environment provenance gap only; it does not replace the named physical dialog workflow.

M19B26 cryptographically binds both the technical-package manifest and physical-dialog evidence manifest to the exact accepted bytes of ADR 0007, rather than trusting only its stable path and decision label. Both verifier paths first require the current ADR to retain its frozen SHA-256 and then require the manifest declaration to match it; deliberate-red contracts independently forge the package and evidence digests and remain rejected. This prevents a changed platform decision from retaining stale Windows-first release evidence, but it does not provide an external signature, authenticate the project owner or physical operator, or replace the named physical dialog workflow.

M19B27 binds the immutable physical-dialog bundle to the packaged desktop process itself, not only to captured executable bytes and OCCT modules. Exact-schema evidence now records the canonical `ketchup-app.exe` package-relative identity, verified-package-root load origin, fresh working directory outside that package, observed main product window, and start/exit observations enclosing both live runtime scans and all six dialog steps. Deliberate-red fixtures reject an externally attributed desktop process and a process exit not strictly after the final runtime-module observation. This proves the current runner's GUI process boundary and chronology only; it still does not authenticate the named human or replace the required physical interaction.

M19B28 makes native file-dialog presence objectively observable during that physical workflow instead of relying only on the operator's retrospective attestation. At each of the eight required Open, Save, or Save As invocations the runner pauses while the dialog remains open, enumerates visible top-level windows owned by the live packaged GUI PID, and requires exactly one Windows native common-dialog class (`#32770`) before allowing the workflow to continue. Exact-schema evidence records the ordered observation, owning PID, visibility, and UTC time inside the corresponding physical-step interval; deliberate-red fixtures reject a missing baseline Save As observation and a dialog attributed to another process. This proves process-bound native dialog presence only; the named operator must still complete the real interaction on release hardware.

M19B29 prevents an arbitrary message box from satisfying that `#32770` observation. The live Windows probe now recursively enumerates the observed window's descendants and requires the case-sensitive `DirectUIHWND` marker of the Windows common item dialog before recording the event. Exact-schema evidence carries that positive observation for all eight dialogs, and a deliberate-red fixture retains the correct top-level class, PID, visibility, order, and chronology while clearing only the common-item marker; offline verification rejects it. This distinguishes the required native Open/Save surface from a generic process-owned dialog without relying on localized window titles, but it still does not replace the named physical interaction on release hardware.

M19B30 binds the packaged GUI observation to the exact stable `Ketchup` main-window title emitted by the accepted desktop entry point. The physical runner records that case-sensitive title before any runtime or dialog evidence, exact-schema verification requires it beside the verified executable identity and process origin, and a deliberate-red fixture substitutes an unrelated window title while preserving the PID and all later evidence; verification rejects it. This prevents a generic window in the exact process from satisfying the product-surface prerequisite, but it does not replace the named physical interaction on release hardware.

M19B31 requires every process-bound `#32770`/`DirectUIHWND` observation to be the active Windows foreground window at capture time. The live probe compares the sole visible common-item dialog handle with `GetForegroundWindow`, exact-schema evidence records the positive result for all eight ordered interactions, and a deliberate-red fixture preserves the correct process, class, visibility, common-item marker, and chronology while marking one dialog as background; offline verification rejects it. This prevents a valid but obscured or unattended process-owned dialog from satisfying the physical interaction checkpoint, but it still does not replace the named operator completing the release-hardware workflow.

M19B32 binds each of those foreground common-item dialogs to the exact observed Ketchup main window, rather than accepting any same-process dialog surface. The live probe records both window handles and requires `GetWindow(dialog, GW_OWNER)` to equal the packaged process's positive main-window handle; exact-schema offline verification enforces the same relationship and rejects self-ownership. A deliberate-red fixture keeps the dialog's PID, class, visibility, foreground status, common-item marker, and chronology valid while substituting a foreign owner handle, and remains rejected. This closes same-process dialog misattribution only; the named operator must still complete the physical release-hardware workflow.

M19B33 requires each captured common-item surface to be modal with respect to that exact Ketchup owner window. While the dialog remains open, the live probe requires `IsWindowEnabled(owner)` to be false; exact-schema evidence records the disabled state and a deliberate-red fixture preserves the correct handles, PID, class, foreground status, common-item marker, and chronology while claiming the owner remained enabled. This prevents a modeless same-process common-item surface from satisfying the file-dialog checkpoint, but it still does not replace the named operator completing the release-hardware workflow.

M19B34 proves that each of those eight observed modal surfaces is completed before the workflow advances. After the operator completes an Open, Save, or Save As interaction, the live probe requires the exact captured handle to be non-visible, requires zero remaining visible common dialogs in the packaged process, and requires the exact Ketchup owner window to be enabled again. Exact-schema evidence records a strictly later closure time before the target step confirmation and before any next dialog observation; a deliberate-red fixture that leaves an otherwise valid dialog marked visible after completion is rejected. This prevents one still-open modal surface from being replayed as multiple interactions, but the named physical release-hardware run remains outstanding.

M19B35 strengthens that completion proof from hidden to destroyed. The live Win32 probe now requires `IsWindow` to reject the exact captured dialog handle after completion, exact-schema evidence records `window_exists_after_close=false`, and a deliberate-red fixture keeps every closure, visibility, owner-reactivation, process, and chronology claim valid while marking the original HWND as still existing. This prevents a merely hidden common-item surface from satisfying closure or being retained for later replay; it still does not substitute for the named physical release-hardware run.

M19B36 makes the existing-document `Ctrl+S` rewrite independently checkable from the immutable bundle instead of retaining only a transient live assertion. Exact-schema evidence records the captured `baseline.ketchup` identity, matching before/after SHA-256 values, filesystem write times and enclosing runner observation times; offline verification recomputes the artifact fingerprint and requires the rewrite to occur strictly after the Save As checkpoint and before the operator confirms the existing-path Save, with the post-observation preceding the next workflow step. Deliberate-red fixtures reject both a non-increasing rewrite time and a forged pre-rewrite digest. This proves that the accepted bytes were rewritten at the known path without changing canonical content, but it still does not replace the named physical release-hardware run.

M19B37 strengthens that rewrite evidence from a timestamp change to a filesystem-object replacement. The live Windows probe reads the volume serial number and 64-bit file index through `GetFileInformationByHandle` immediately before and after the existing-path `Ctrl+S`; the exact-schema manifest requires canonical lower-case identities and requires them to differ while the content SHA-256 remains unchanged. A deliberate-red fixture preserves the valid hash, chronology, path, and operator step while replaying the pre-Save file identity as the post-Save identity, and offline verification rejects it. This distinguishes the implemented temporary-file replacement from an in-place rewrite or timestamp touch, but the named physical release-hardware run remains outstanding.

M19B38 binds all five successful Save or Save As results to exact targets that the live runner objectively observes as absent before their corresponding native dialog opens. Exact-schema evidence records each pre-check and post-creation observation, immutable size/SHA-256 fingerprint, dialog identity, and physical-step chronology; offline verification rejects both a target declared pre-existing and an absence check delayed until the dialog was already open. This prevents a pre-planted artifact from satisfying a successful file workflow checkpoint, but it does not replace the named physical release-hardware interaction.

M19B39 binds both expected failures to the exact immutable input object used by the corresponding native dialog. The live runner records pre/post size, SHA-256, Windows volume/file identity, dialog identity, and enclosing chronology for `corrupt.ketchup`; it records the same evidence for `locked-target.ketchup` through the already-open `FileShare.None` handle so the exclusive lock remains continuously held while hashing and identifying the file. Exact-schema verification rejects a forged pre-hash, replacement under unchanged bytes, a locked-Save claim without the lock, or a post-check delayed until the following continuity dialog. This prevents substituted failure fixtures or a merely asserted lock from satisfying malformed-Open and locked-Save continuity, but it still does not replace the named physical release-hardware interaction.

M19B40 binds the named physical-operator attestation to the Windows security context that actually runs the evidence harness. The runner captures the exact Windows account, SID, and nonzero interactive session, requires the packaged desktop process to start in that same session, and records both sides in the exact-schema immutable manifest. Offline verification rejects a malformed SID, noninteractive session, or packaged process attributed to another session; deliberate-red fixtures exercise SID forgery and session detachment. This strengthens attribution to a concrete local account and interactive desktop only; it is not cryptographic human identity proof and does not replace the named physical interaction.

M19B41 closes a runner time-of-check/time-of-use gap in the physical-dialog evidence path. The live harness fingerprints its own exact PowerShell bytes before the first technical-package verification and again after the packaged process exits and the isolated package passes its final verification; any byte change aborts capture. Exact-schema evidence records both hashes and UTC bounds, while offline verification requires both to match the manifest's runner digest and the current verifier and to enclose the complete package/workflow interval. Deliberate-red fixtures reject both changed post-workflow runner bytes and a pre-check delayed until package verification had already begun. This proves runner-byte continuity for the evidence path only; it does not authenticate the script publisher or replace the named physical release-hardware interaction.

M19B42 replaces the desktop process's declarative load-origin claim with a live process-image check. Immediately after launch, the physical runner resolves `MainModule.FileName`, requires that exact path to equal the isolated snapshot's `ketchup-app.exe`, and binds its measured size and SHA-256 to the unique desktop-application allowlist record. Exact-schema evidence retains the positive path-match result and process-image fingerprint; offline verification rejects both an external image attribution and a forged image digest against the captured technical package. This proves executable-image identity for the observed process boundary only; it does not authenticate the named human or replace the physical release-hardware interaction.

M19B43 binds the package's `windows-x86_64` claim to every executable image in its exact registry. Verification parses the bounded DOS and PE/COFF headers of the desktop application, exact worker, and all pinned OCCT DLLs, requiring the AMD64 machine code and PE32+ optional-header magic after the immutable size/SHA-256 check. Deliberate-red package fixtures retain self-consistent manifest fingerprints while substituting either an x86 machine code or PE32 magic, and remain rejected. This proves binary architecture identity for the technical package only; it does not replace the named physical release-hardware workflow.

M19B44 makes each physical workflow step's product-liveness claim an objective live observation. Immediately after every operator confirmation, the runner requires the original packaged process to remain alive and the exact previously captured `Ketchup` main-window handle to remain an existing visible window with the exact title; exact-schema step evidence binds those observations back to the desktop-process record. A deliberate-red fixture detaches an otherwise valid step from that main-window handle and remains rejected. This proves product-surface continuity through all six file operations, but it does not replace the named physical release-hardware interaction.

M19B45 closes the shorter continuity gap immediately after each native file dialog is dismissed. In addition to proving the common-item dialog HWND was destroyed and its owner re-enabled, the live runner now requires that exact owner HWND to remain existing and visible with the case-sensitive `Ketchup` title and to remain owned by the original packaged process. Exact-schema evidence records all four observations for every dialog closure, and a deliberate-red fixture reattributes the surviving owner to another process while preserving the rest of the dialog evidence; verification rejects it. This prevents a closed dialog followed by a replaced, hidden, or foreign owner surface from satisfying the checkpoint, but it still does not replace the named physical release-hardware interaction.

M19B46 closes the remaining restore-between-checks package TOCTOU window during physical evidence capture. After the isolated snapshot passes its pre-launch allowlist verification, the runner holds read handles that deny write and delete sharing for the package manifest and every one of the 50 exact allowlisted binaries through the worker probe, complete GUI workflow, process exit, final package verification, and evidence copy. Exact-schema evidence records the complete guarded-file register, guard mode, and enclosing UTC interval; deliberate-red fixtures omit one pinned runtime or release the guards before final verification and remain rejected. This proves continuous package-byte immutability during the evidence workflow only; it does not replace the named physical release-hardware interaction.

M19B47 closes the acquisition interval between the first isolated-package verification and completion of all 51 write/delete guards. Once every guard handle is held, the runner repeats the complete canonical technical-package verifier and records that successful check strictly before the exact-worker probe or GUI launch. Exact-schema chronology rejects a guarded-verification timestamp at or before guard acquisition, preventing a package state obtained through the previously unguarded interval from becoming executable evidence. This proves the worker and desktop start only from a fully verified continuously guarded snapshot; it does not replace the named physical release-hardware interaction.

M19B48 binds each physical workflow confirmation to one exact immutable operator attestation instead of retaining only a generic positive boolean. The runner displays the step-specific statement before accepting `PASS`, including explicit claims that the frozen malformed input was selected and rejected and that the exclusively locked target was selected and produced a Save failure while the baseline remained active. Exact-schema verification requires the statement assigned to each ordered step, and deliberate-red fixtures replace both failure statements with dialog-cancellation claims and remain rejected. This makes the named human attestation unambiguous but does not automate or replace the required physical release-hardware interaction.

M19B49 binds the physical workflow to native AMD64 execution of the exact packaged GUI process instead of proving only that its on-disk image is AMD64 PE32+. The live runner calls Windows `IsWow64Process2` against that process handle and requires `IMAGE_FILE_MACHINE_UNKNOWN` for the process machine plus `IMAGE_FILE_MACHINE_AMD64` for the native host, excluding x64-on-ARM64 emulation from Windows x86-64 release-hardware evidence. Exact-schema verification retains both machine codes and the positive native-execution claim; a deliberate-red fixture presents an otherwise valid AMD64 package as ARM64-emulated and remains rejected. This proves the execution architecture of the observed process boundary only and still does not replace the named physical dialog workflow.

M19B50 removes a focus-invalidating interaction from the physical runner. The harness no longer asks the operator to type `READY` into its console and then attempts to prove that the file dialog is still the active foreground window; instead it waits for up to two minutes and automatically captures the sole process-owned modal `#32770`/`DirectUIHWND` foreground dialog while the operator keeps focus in Ketchup. The focused contract rejects reintroduction of the console prompt or removal of the bounded automatic observation. This makes the physical workflow executable without weakening its foreground requirement, but it does not replace the named release-hardware interaction.

M19B51 prevents a transient or replaced common-item surface from satisfying that automatic capture. The runner now requires the same exact HWND to survive two live observations at least 100 ms apart and rechecks existence, visibility, owning PID, owner HWND, owner-disabled modality, foreground status, and the `DirectUIHWND` marker before recording the checkpoint. Exact-schema evidence binds both UTC observations and the positive same-window result before dialog closure; a deliberate-red fixture clears only that stability result and remains rejected. This closes the single-sample observation race but does not replace the named operator completing the physical release-hardware workflow.

M19B52 removes the remaining focus-invalidating wording from all eight native-dialog instructions. Each instruction now tells the operator to keep the requested dialog foreground until the console reports an explicit `OBSERVED` checkpoint emitted only after the stable two-sample capture; the focused source contract rejects any dialog instruction that still asks the operator to return to the console early and requires exactly eight corrected prompts. This aligns the executable operator procedure with the foreground-window evidence requirement but does not replace the named physical release-hardware interaction.

M19B53 makes the automatic native-dialog observation bound independent of adjustable wall-clock time. Each poll uses a monotonic `Stopwatch` with a strict two-minute ceiling while exact-schema evidence records its UTC start and measured wait in milliseconds; offline verification rejects negative or over-limit waits and a start later than the first sample. The focused source contract prevents reintroduction of the former UTC deadline, and a deliberate-red fixture exceeds the monotonic limit by one millisecond and remains rejected. This prevents a backward system-clock adjustment from extending an unattended poll indefinitely, but it does not replace the named physical release-hardware interaction.

M19B54 applies the same fail-closed timing boundary before the physical workflow starts. The packaged GUI main-window probe now uses a monotonic `Stopwatch` with a strict 30-second ceiling instead of an adjustable UTC deadline, refreshes the live process on every bounded sample, and records the UTC observation interval plus monotonic wait in the exact-schema desktop-process evidence. Offline verification rejects negative or over-limit waits and requires that interval to remain between the observed process start and the first runtime-module scan; a deliberate-red fixture exceeds the startup bound by one millisecond. This prevents a backward system-clock adjustment from hanging release capture before the dialog workflow, but it does not replace the named physical release-hardware interaction.

M19B55 makes native-dialog completion as race-resistant as opening observation. After the first live proof that the captured HWND is destroyed, no process-owned common-item dialog remains, and the exact Ketchup owner is re-enabled and live, the runner waits 100 ms and repeats every closure and owner-continuity check before recording success. Exact-schema evidence binds the two closure samples and requires the second sample before the physical step, output, failure-continuity, or next-dialog chronology can advance; a deliberate-red fixture clears the stable-closure result and remains rejected. This prevents a transient single-sample disappearance or owner reactivation from satisfying completion, but it does not replace the named physical release-hardware interaction.

M19B56 closes the corresponding owner-identity race during dialog opening. Both live samples now require the owner HWND to remain the exact observed Ketchup main window, exist, remain visible, retain the case-sensitive `Ketchup` title, belong to the packaged desktop PID, and remain disabled by the modal common-item dialog. Exact-schema evidence records that owner identity beside every native-dialog observation, and a deliberate-red fixture attributes the matching owner HWND to another live process and remains rejected. This prevents stale or reused HWND equality from impersonating the packaged product owner, but it does not replace the named physical release-hardware interaction.

M19B57 makes the initial packaged product-surface observation equally race-resistant. Before the first runtime-module scan or dialog step, the runner now requires the same nonzero main-window HWND to remain existing, visible, titled exactly `Ketchup`, and owned by the packaged desktop PID across two live samples at least 100 ms apart, with both samples inside the existing monotonic 30-second startup limit. Exact-schema evidence records the recheck time and stable live identity; a deliberate-red fixture clears only the same-handle stability result and remains rejected. This prevents a transient or replaced startup window from satisfying the physical product-window prerequisite, but it does not replace the named physical release-hardware interaction.

M19B58 extends that race-resistant product-surface identity through every physical workflow-step attestation. After each operator-confirmed step and closed dialog, the runner requires the same recorded main-window HWND to remain existing, visible, titled exactly `Ketchup`, and owned by the packaged desktop PID across two live samples at least 100 ms apart before recording PASS. Exact-schema step evidence binds both observation times, PID, and the positive same-handle result; deliberate-red fixtures reject a single-sample claim, a 50 ms recheck, and a matching HWND attributed to another process. This prevents transient or reused product windows from satisfying step continuity, but it does not replace the named physical release-hardware interaction.

M19B59 binds each of the five successful Save or Save As outputs to a newly created filesystem object, not only to an absent pre-check and later content fingerprint. The live runner records the Windows volume/file-index identity plus creation and last-write UTC times, and immutable verification requires unique identities whose creation and write occur after the corresponding stable native-dialog observation and no later than its first proven closure. Deliberate-red fixtures alias two outputs to one object and place an output creation before dialog observation; both remain rejected. This closes a pre-planted or hard-linked output substitution path, but it does not replace the named physical release-hardware interaction.

M19B60 closes the identity-chain gap around the existing-document atomic Save. Immutable verification now requires the pre-rewrite identity to equal the exact filesystem object created by baseline Save As, then requires the post-rewrite replacement identity to differ from every successful Save or Save As creation identity. Deliberate-red fixtures detach the rewrite from its baseline source and alias the replacement baseline to a later round-trip output; both remain rejected. This prevents a hard-linked continuity artifact from satisfying byte-identical Save As evidence while sharing the current baseline object, but it does not replace the named physical release-hardware interaction.

M19B61 closes the final source-to-bundle gap for successful Save evidence. Immediately before immutable artifact copy, the runner reopens all five successful output paths with read sharing only, requires the baseline path to retain its post-`Ctrl+S` replacement identity and every other path to retain its creation identity, rechecks each original size and SHA-256 through the guarded handle, and keeps all five handles open through copying. Exact-schema evidence binds the source identities, fingerprints, guarded interval, and capture chronology; deliberate-red fixtures replace one source object or change its pre-copy digest and remain rejected. This prevents a post-observation path swap or byte change from being silently copied into a self-consistent evidence bundle, but it does not replace the named physical release-hardware interaction.

M19B62 removes the corresponding path-based race from each immediate post-dialog Save observation. The runner opens the newly created target once with read sharing only, obtains its Windows volume/file-index identity plus creation and last-write times through `GetFileInformationByHandle`, reads its size and SHA-256 through that same handle, and records completion only after all measurements finish. The focused source contract rejects restoration of the former independent path identity/hash probes. This prevents one output path from being replaced between metadata, identity, and content observations, but it does not replace the named physical release-hardware interaction.

M19B63 removes the same race from both sides of the existing-document atomic `Ctrl+S` rewrite. Before interaction and again after operator-confirmed replacement, the runner opens the baseline once with read sharing only, obtains its volume/file-index identity and last-write time through `GetFileInformationByHandle`, and reads size plus SHA-256 through that same write/delete-blocking handle. Exact-schema verification binds both measured sizes and hashes to the immutable captured baseline, while the source contract rejects restoration of the former independent path probes and a deliberate-red fixture forges the post-rewrite size. This prevents a path swap between rewrite metadata and content measurements, but it does not replace the named physical release-hardware interaction.

The M5 beam path keeps each crossing as one canonical joint. Its ordered rule feature derives an upper beam notch, a complementary lower crossing-piece notch, and their common contact plane. The non-convex beam is represented by one lower rail plus thirteen upper fragments; component keys are sorted by explicit feature/fragment ordinal, checked for unique disjoint containment, and included in the validator input digest. Validation checks every non-empty component-pair intersection or declared contact against the joint volume, retains all four joint verdicts including declared-without-contact failure, and remains `12 Exact / 0 Tolerant` before and after the controlled spacing change. The supervised worker now evaluates all thirteen pieces as OCCT B-Reps, returns 48 unique contact/wall face identities with parameter-stable semantic lineage, and the running app accepts them only for the current revision before exposing a dimensioned SVG piece drawing and 24-operation deterministic manufacturing export. Stale packages and invalid/quarantined reference lineage block export.

## 5.8 Tests and evidence — CURRENT SNAPSHOT

Current runs against the immutable implementation baseline recorded once in Appendix G reported:

- the unfiltered product command `cargo test --locked --workspace --all-targets` passed across app, core, exact, interaction, scheduler, Gate B, C1a, narrow C1b, ThroughCut, capstone, P07, and W4A suites without discovering the feature-gated sealed A0 targets;
- `cargo test -p ketchup-app --test capstone_chain -- --nocapture` passed 2/2 with the exact worker present;
- focused P07/W4A core suites passed 16/16, the exact physical/durable interaction suite passed 4/4, and the M4b suite passed its accepted-body clearance, exact-face dimension, canonical-joint, stale-binding, and real ThroughCut `Tolerant` propagation assertions;
- the exact clearance unit proof classifies separated diagonal gap, touching, and positive-volume intersection distinctly; focused M17A evidence mixes current accepted exact and canonical mesh occurrences, reports `1 Exact / 1 Tolerant` for passing clearance, reports a `Tolerant` mesh collision failure, and rejects both unavailable exact evidence and stale snapshot/registry bindings; focused M17B evidence regenerates byte-identical grouped BOM, three-view SVG and exact-stock manufacturing projections, rejects row/evaluator tampering, stale snapshots and non-rigid transforms, and allows mixed exact/mesh BOM+drawing export while blocking mesh manufacturing export;
- the M5 beam proof derives both complementary notches from each canonical joint, validates a 14-component exact notched beam, preserves all four joint verdicts, and remains `12 Exact / 0 Tolerant`; physical worker/app acceptance additionally evaluates 13 OCCT B-Reps, accepts 48 durable notch-face references, emits a dimensioned SVG plus 24 manufacturing operations, rejects stale/quarantined export, and preserves reference lineage after the controlled spacing change;
- the M6A bottle proof passed its canonical/rollback/persistence, native OCCT, worker/current-reference, and app numeric/direct-edit/render/pick/export suites; its controlled batch changes radius `30 → 32`, height `110 → 120`, shoulder rise `20 → 16`, finish `Fillet 2 → Chamfer 1.5`, invalidates the old exact package, produces bounds `[-32,-32,0]..[32,32,161]`, and preserves nine current durable roles after lossless Save/Open;
- the M7 suites prove the bounded local Assistant, external Python protocol, and signed WASM validator paths under capability, Proposal, budget, trust, revocation, licensing, isolation, and fail-closed availability rules;
- the shared exact feature/result suites prove deterministic multi-body registry ordering, duplicate/stale rejection, Boolean Cut/Union and beam/bottle package views, canonical visibility authority, and bounded running-app groove/cut rendering, picking, and export; focused M15A evidence additionally distinguishes `Resolved`, `Ambiguous`, `Lost`, and `Quarantined` reference outcomes across an occurrence transform, Cut→Union role removal, competing current packages, tampered lineage, foreign-document evidence, and an incompatible evaluation envelope; focused M15B evidence converts one current accepted exact cuboid into a detached sole-authority canonical `MeshBody` through one batch, round-trips its schema-16 bits and structured loss provenance, preserves Undo/Redo, and rejects stale or open-mesh inputs atomically;
- the parameter-binding suites prove typed rule-output bindings, identity-bound explicit recompute through one `apply_batch`, atomic rollback, Undo/Redo, schema-16 persistence, observational Open, stale-input/backend diagnostics, and restoration of current exact render/pick/export only after recompute;
- the deliberate-red architecture self-test observed every intended rejection path, including A0 rediscovery, private in-core revision writes, registry freshness bypass, and Open-time recompute attempts;
- strict workspace Clippy passed for the current-evidence implementation baseline;
- the explicit `cargo test --locked -p ketchup-exact --features a0-certification --test gate_a0_v2 -- --exact gate_a0_v2 --nocapture` target failed closed before geometry on stale sealed inputs;
- the default architecture guard passed without consulting a historical A0 lock, while its explicit `-FrozenLockPath` mode and deliberate-red fixture still reject changed locked inputs.

Formal A0 tests remain runner-only: Cargo requires the explicit `a0-certification` feature, daily CI neither discovers nor name-skips them, and the dedicated A0 runners supply the sealed run ID, lock hash, backend identities, and evidence paths. Counts and durations are observations, not eternal requirements.

## 5.9 Evidence caveats

- Historical strengthened A0 v2 `FULL_GO` proves its frozen operation/reference subset and identities; it does not certify the current-evidence baseline in Appendix G, later working-tree changes, curved topology, or broad exact operations.
- The former A1 research graph has become product/core evaluator and schema-16 infrastructure with bounded rule→feature bindings, but the evidence still does not prove a universal geometry-feature DAG.
- Gate B primitives and the integrated supervisor support the narrow product worker path; they are not a sealed certification of every future job, restart, memory, or progress policy.
- C1a and bounded C1b product evidence exist for rectangle, Boolean Cut/Union, rotational bottle, and beam-body roles through the shared registry. Focused current-tree evidence adds explicit `Resolved`, `Ambiguous`, `Lost`, and `Quarantined` outcomes, but does not cover all transforms, mutations, feature families, arbitrary edge selection, or downstream consumers.
- The required terminal integrated-GPU Gate C evidence is incomplete.
- Appendix G remains the last immutable current-evidence commit. The explicitly bounded working-tree implementation described in this update is not a new certification freeze until it receives a reviewed commit identity and fresh required gate evidence.


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

This axis describes how a body is **represented**. It does not by itself determine how decidable a predicate over that body is: an exact body specification may still be curved, in which case predicates over it are threshold-dependent. The decidability axis is the evidence class defined in §10.1.1 (W4A), and the two MUST NOT be conflated. `CanonicalBodySpec::Exact` is a necessary but not sufficient condition for an `Exact` evidence class.

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

The container format MAY differ physically, but it MUST provide equivalent separation and integrity. The bounded M14A implementation uses a deterministic table rather than a ZIP directory: required `document.bin`, optional `blobs/<sha256>`, and `extensions/<namespace>/<safe-relative-path>` entries carry required/optional flags, lengths and SHA-256 digests under a 64 MiB/4,096-entry envelope. It deliberately does not persist disposable caches or current exact result packages; existing exact families recompute from canonical semantics. This is the implemented safe default and evidence for V4-O01, not silent owner ratification of the long-term format freeze.

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

### 10.1.1 Evidence class of a validation result (W4A)

Validation classes in §10.1 describe **what** a validator asserts. They do not describe **how decidable** that assertion is. These are independent axes and both MUST be carried.

The distinction is not implementation quality; it is a property of the geometry. Over planar/prismatic bodies, predicates such as “do these two pieces interpenetrate?” are algebraically closed and admit a determinate yes/no answer. Over curved bodies, the intersection of two general surfaces has no closed algebraic form, must be approximated numerically, and the same predicate becomes “they overlap by δ — is δ within threshold?”. A result produced under a threshold is a different kind of statement from a result produced by decision, and MUST NOT be reported as the same thing.

**W4A requirements:**

1. Every structured diagnostic MUST carry an **evidence class**: `Exact` or `Tolerant`.
2. `Exact` MAY be claimed only when **all** participating inputs are exact body specifications under §8.1 **and** the evaluation method is algebraically closed for those inputs (for example deterministic `f64` SAT and half-space clipping over convex polyhedra per §10.2). Any tessellation, conservative envelope, iterative surface–surface intersection, or numerically approximated predicate yields `Tolerant`.
3. A `Tolerant` diagnostic MUST additionally carry the applied threshold, the method identity, and the permitted error direction.
4. For a mixed pair or set, the result takes the class of the **weakest** participant. A single curved participant makes the whole result `Tolerant`.
5. Aggregate reporting — model summaries, gate reports, BOM/export preconditions, StateView projections, and agent-facing views — MUST report `Exact` and `Tolerant` counts separately. A single combined “passed” for a mixed model is prohibited.
6. `Exact`, `Tolerant`, `Unavailable`, and `NotEvaluated` are four distinct outcomes. As with `NotEvaluated`, a `Tolerant` result MUST NOT be silently promoted to an exact guarantee by any projection, report, or export.
7. Evidence class is orthogonal to the regulatory/structural/manufacturability/advisory classes in §10.1 and to the `Guaranteed`/`Best-effort` reference stability classes in §7; none substitutes for another.

**Introduction timing:** the field MUST be introduced in M4a-E, while the domain is entirely prismatic and every result is `Exact`. Introducing it then costs one field and no behavioural change. Introducing it after the first curved body would require revisiting every validator, every stored diagnostic, and every report projection, and would leave historical results unclassifiable.

## 10.2 Collision pipeline

For the initial prismatic domain, collision validation uses:

1. AABB broad phase over whole pieces;
2. optional OBB or convex bounds to reduce candidates;
3. deterministic `f64` SAT over convex polyhedra or canonical convex coverage as a candidate/contact test;
4. tolerance-profile classification of contact versus penetration;
5. for every SAT-positive convex component pair, deterministic construction of the convex intersection region `I_ij = A_i ∩ B_j`, for example by half-space clipping;
6. declared-joint containment evaluation for every non-empty penetrating intersection region at the whole-piece pair level.

A canonical convex **coverage** may overlap internally and is valid for collision detection if its union covers the solid without missing occupied space. It MUST NOT be reused for mass/volume calculations when overlapping components would double-count. Intersection containment is evaluated per non-empty convex component-pair region; regions MUST NOT be summed as physical overlap volume unless a non-overlapping representation proves that operation valid.

For curved bodies, the first fallback is a conservative envelope that may produce false positives but MUST NOT miss collisions. Tessellation-based validation is permitted only when tessellation parameters and permitted error direction are part of the determinism envelope. Every result produced by a conservative envelope or by tessellation is `Tolerant` under §10.1.1, as is every result for a pair containing at least one curved participant; only the fully prismatic path above — SAT and half-space clipping over convex polyhedra with exact inputs — yields `Exact`.

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

SAT is insufficient to classify a declared-joint overlap because it does not locate the complete penetration region. For allowed joint volume `J`, Euclidean tolerance ball `B_ε`, and tolerance threshold `ε`, every non-empty penetrating `I_ij` MUST be wholly contained in the certified acceptance region `K_ε = J ⊕ B_ε`; any portion outside is an error. In the current convex-polyhedral scope, the final certified test MUST evaluate `distance(v, J) ≤ ε` for every vertex `v` of `I_ij`: distance to a convex set is convex, so satisfaction at all vertices proves satisfaction over their convex hull. The closest-point calculation and all boundary comparisons use the frozen deterministic `f64` tolerance policy; no conservative inner approximation is required for this scope. Independently shifting every polyhedral face outward by `ε` generally creates a strict **superset** near edges and corners—up to `ε(√3−1)` extra radial reach at an orthogonal trihedral corner—and MUST NOT be used as the final acceptance region; it may only be a candidate stage followed by the certified vertex-distance test. Absence of the required intersection/contact is an empty-joint error. A joint MUST NOT exempt an entire pair of pieces from collision checking. The bounded, non-permissive containment test is what makes the exception safe. Every verdict in the table above carries an evidence class under §10.1.1: the certified vertex-distance containment test over convex polyhedra with exact participants is `Exact`; a joint whose participants or acceptance region are evaluated through an envelope, tessellation, or approximated intersection is `Tolerant` and MUST expose its threshold and method.

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
3. produce a minimal grouped list of piece identity, quantity, and length, and assert ordered generated piece positions against the frozen fixture whose expected remainder-distribution sequence is `415 × 6`, `408 × 5`, `400`;
4. declare bounded joints where expected intersections occur;
5. run collision and empty-joint validation, emitting the §10.1.1 evidence class on every diagnostic — in this slice every result MUST be `Exact`, and any `Tolerant` result is itself a finding;
6. change one governing value;
7. recompute only the dependent branch;
8. show preserved/lost/ambiguous slot and override identities;
9. regenerate the grouped piece/length list;
10. commit the accepted change as one user operation.

This minimal run is checkpoint M4a-E. It intentionally excludes the full BOM and stable-dimension projection contract, including rendering the reference sequence (`415 × 6`, `408 × 5`, `400`) as dimension-chain output, but it retains that sequence as the fixture assertion for generated piece positions and remainder-distribution semantics. It also excludes FurniGen import, third-party validator hosting, AI, and BTLx so that it exposes evaluator/slot/product failures as early as possible. The full BOM/dimension projections, built-in host-neutral validator contract, and available FurniGen evidence follow immediately in the M4a completion track without delaying this first observation.

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

A second FurniGen observation is recorded as a separate product finding. In a small-house generation experiment the geometric and topological output — walls, partitions, and layout — was usable, while furniture placement was not: the generator placed storage in front of a window. This was not a generation-quality failure. The constraint “a window keeps the volume in front of it clear” was never expressible, so it was never checked. The class of knowledge that succeeded was geometrically derivable; the class that failed required knowledge of what a space is *for*. §10.9 addresses the missing inputs.

## 10.9 Spatial semantics for clearance and advisory validators (W4B)

§10.1 already admits spacing/clearance and explicitly advisory validators, including feng shui, and M4a already freezes the host-neutral contract they need. The canonical document, however, models **pieces and their relationships** — definitions, occurrences, groups, features, parameters, rules, slots, and joints. It does not model **space**. A room is only a set of walls; a space has no identity, no declared purpose, no adjacency, and no access relation, and no element declares a volume that must remain clear.

Consequently the three motivating cases are not expressible today:

| Case | Missing input |
|---|---|
| Storage placed in front of a window | Declared clear volume belonging to the window |
| A door swing obstructed by a fixture | Declared clear volume belonging to the door’s motion |
| A WC opening directly off a bedroom | Space identity, declared purpose, and adjacency/access relation |

A validator forced to reconstruct rooms from wall geometry, infer purpose from size or furniture, and guess access from openings is inferring its own inputs. Such a validator cannot be deterministic, cannot bind a stable diagnostic location, and cannot be authored by a third party against a stable contract — which defeats the M4a/M7 package model.

**W4B requirements (target state; not an FLP obligation):**

1. `Space` becomes a canonical entity with stable identity, bounding representation, declared semantic purpose, and adjacency/access relations to other spaces. Purpose is declared, never inferred from geometry.
2. `ClearanceVolume` becomes a canonical entity: a bounded volume in a declared coordinate frame, owned by an element or space, that MUST remain free of declared occupancy. It carries owner, semantic reason, tolerance, and severity policy.
3. A clearance volume MAY be rule-derived (a door swing follows the door leaf; a drawer extension follows its travel) and then carries a `SlotPath` and participates in §10.4 result comparison exactly as any other derived output. `Lost` and `Ambiguous` handling is identical; silent retargeting is prohibited.
4. Anthropometric and access requirements — passing through an opening, sitting, standing, reaching, circulating — are modelled as clearance volumes, not as a separate mechanism, so that one contract serves furniture, sauna, bathroom, stair, and corridor domains.
5. Advisory validators, including feng shui, consume `Space`, purpose, adjacency, and `ClearanceVolume` as declared inputs. They remain advisory under §10.1 and `Tolerant` under §10.1.1, and MUST NOT be presented as regulatory or structural results.
6. Spatial and clearance entities are canonical model state under §6.3 and mutate only through `apply_batch`. They are not validator-local tables; a second authority over the model is prohibited.

**Mechanical note.** `ClearanceVolume` is the sign-inverted twin of the bounded joint volume in §10.3. A joint declares *“penetration here is expected”*; a clearance volume declares *“penetration here is an error.”* Both are bounded volumes in a declared frame, both are evaluated by the same convex intersection and certified containment machinery of §§10.2–10.3, and both need the same derived-identity and loss semantics. W4B is therefore a second application of mechanics proven in M4a-E, not a new subsystem — which is why it is recorded now, while joint semantics are still being completed, rather than after they harden.

**Scope and timing.** W4B is a target-state decision for the residential/spatial domain. It is explicitly **not** required by FLP, the beam workflows in §§10.5–10.6, or any fabrication output in §10.7; those need no spatial model. It becomes blocking for the first workflow that requires clearance or advisory validation, and its entities MUST be designed jointly with — not after — the joint volume contract to avoid two incompatible bounded-volume mechanisms.


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
| C1a | **CURRENT narrow product regression gate; not terminal Gate C certification.** Canonical visibility/context authority is enforced through snapshot-bound direct nested queries for fallback and shared exact-registry projections; deterministic snap scoring, hysteresis and overlap choice are covered. Broader topology and acceleration remain incomplete. |
| C1b | **CURRENT bounded product regression gate; not general topology certification.** Exact worker results, registry acceptance, interaction picking, persistence, and export agree for the preregistered rectangle/boolean/bottle/beam roles; broader transforms, mutations, edge operations, and `Ambiguous`/`Lost` paths remain incomplete. |
| D | **PARTIAL—PRODUCT PATH.** Bounded rule/model dimension intents pass through Proposal preparation, explicit review, one `apply_batch`, and transactional verification; full canonical-task corpus, natural-language/provider path, and human-only high-risk confirmation remain incomplete. |

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

The order `M0 → durable M1 prerequisites → M2 → M4a-E early beam checkpoint → M3`, with the remaining M4a protocol/projection/evidence track completing immediately after M4a-E and before its own full exit, followed by `M4b/M5 → workflow-led M6 → M7`, changes the frozen V3 sequence, which expected an exact Gate C before the narrow manual modeler. **ADR 0004 ratifies this V4-P15 replacement sequence** and preserves the applicable gate consequences; its temporary A0 loosenings were withdrawn after A0 v2 without revoking the accepted order.

The architectural reason for M4a-E-before-M3 is that the earliest beam workflow 6.3a depends on M2 rules, hierarchical slots, collision, bounded joints, and only a grouped piece/length list, but intentionally excludes the full BOM/dimension projection contract, FurniGen import, third-party validator hosting, exact notches, product OCCT integration, durable subshape references, and C1b. Historical exact Gate C evidence retains its original envelope and is not silently retired or promoted by the reordered implementation sequence.

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
5. run beam workflow 6.3a with a simple grouped list of piece identity, quantity, and length, and assert generated piece positions against the frozen `415 × 6`, `408 × 5`, `400` remainder-distribution fixture without requiring dimension-chain projection output.

**M4a-E exit:** one governing value changes the beam pieces; generated positions match the frozen remainder-distribution fixture; joints and overrides remain stable or become explicitly unresolved; illegal overlap outside the certified joint acceptance region is caught; empty declared joints fail; and the grouped piece/length list regenerates. If this reveals that evaluator, slot, override, collision, or joint semantics do not serve the product, M2 is corrected before M3 begins.

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

**Current bounded implementation:** accepted exact packages are revision/digest checked, translated AABBs classify minimum gap/touch/intersection conservatively, exact-face datums produce revision-bound dimensions, canonical joints require durable opposing face participants, and mixed ThroughCut cases propagate auditable `Tolerant` evidence. Curved-body narrow phases, domain standards/manufacturability validators, and target-state `ClearanceVolume` remain outside this slice.

## M5 — Exact fabrication features

**Goal:** add exact openings/notches and complete 6.3b.

1. canonical cut/union/opening feature;
2. expanded exact operation and TNP corpora;
3. joint-driven notch derivation;
4. exact collision representation for notched prisms;
5. piece-drawing/manufacturing operation projection;
6. beam workflow 6.3b.

**Exit:** semantic half-lap joint produces consistent geometry and dimensions on both pieces, survives controlled parameter changes, and exports no invalid/quarantined result. The fully prismatic beam acceptance case MUST remain `12 Exact / 0 Tolerant`; any `Tolerant` result is a finding that blocks M5 exit.

**Current implementation:** one canonical joint derives complementary plane-cut notches on both beam participants and a common contact plane. The exact-prismatic validator consumes a deterministic feature/fragment-ordered cuboid decomposition of the resulting non-convex beam, retains the four declared-joint verdicts, and passes the `12 Exact / 0 Tolerant` controlled-change proof. The supervised worker evaluates all thirteen pieces as OCCT B-Reps; the running application accepts only current revision/digest-bound results, preserves 48 semantic contact/wall references across the controlled spacing change, and emits a dimensioned SVG piece drawing plus a deterministic 24-operation manufacturing export. Stale results and invalid or quarantined lineage fail closed, so the full M5 exit is satisfied for beam workflow 6.3b; general document-wide drawings and fabrication export remain later scope.

## M6 — Broaden geometry deliberately

**Goal:** move beyond boxes through the smallest coherent operation sets proven by concrete editable workflows, while preserving authority, identity, and interaction contracts.

Workflow slices, not a universal feature checklist, determine priority:

1. **Bottle slice:** primitive or validated profile; controlled stretch/scale and flattening; bevel/fillet; shell/thickness; optional revolve for rotational variants; controlled spline/loft/sweep only when the asymmetric acceptance case requires them.
2. **Living-tree slice — non-blocking research/acceptance:** semantic trunk/branch `SlotPath` structure; variable-radius sweep/loft or explicitly mesh-authoritative procedural geometry; direct branch/path/radius edits with local recomputation and explicit reference health. Its failure or deferral MUST NOT block bottle, beam, M6 core, or FLP exit.
3. **Domain-led additions:** profiles, constraints, booleans, imports, conversions, and exports only when a frozen furniture, building, fabrication, bottle, tree, or other approved workflow demonstrates their value.

Every slice repeats the canonical-command-evaluator-reference-interaction-persistence-validation chain and proves direct manipulation, meaningful numeric control, Undo/Redo, Save/Open, and authority/loss reporting. M6 does not promise complete CAD, unrestricted sculpting, animation, or production-scale vegetation generation.

**Current M6A implementation:** the bounded rotational bottle slice is complete on the current-evidence implementation baseline in Appendix G. One canonical chain owns a validated six-point half-profile, controlled radius/height/shoulder dimensions, Revolve, open-mouth Shell thickness, and Fillet/Chamfer finish. The supervised worker returns a deterministic OCCT B-Rep, tessellation, and five or nine durable semantic face roles; the app accepts only a revision/digest-bound current package for rendering, picking, worker-mediated STEP export, and OBJ export with explicit editability/topology/tolerance loss. Numeric panel edits and body/shoulder/neck role drags each commit one atomic batch; Undo/Redo and schema-16 observational Save/Open preserve canonical identity. The controlled-change fixture records `30→32 mm` body radius, `110→120 mm` body height, `20→16 mm` shoulder rise, and `Fillet 2→Chamfer 1.5 mm`; it changes canonical/result identities, rejects the stale package, produces `[-32,-32,0]..[32,32,161]` bounds, and retains nine current durable roles after reopen. Invalid profiles, thicknesses, finish amounts, and combined edits roll back without a revision or Undo step.

**M6 status:** M6 closed on the blocking M6A bottle exit for this rotational envelope. The explicitly non-blocking M6B living-tree research slice and unapproved M6C additions were deferred without implementation claims; asymmetric spline/Loft/Sweep bottles remain planned.

## M7 — AI and extension surface

**Goal:** expose automation only after deterministic semantics exist.

1. intent vocabulary from proven workflows;
2. complete Proposal/read-write/risk/diff/budget path;
3. Assistant UI and verification;
4. gate D over canonical tasks;
5. Python SDK and constrained plugin pilot;
6. third-party validator hosting: discovery/install/update, authenticated publisher/signature provenance, trust and revocation, paid-license state, native/WASM process isolation, and confirmed remote egress;
7. later domain semantic packages, BIM primitives, and broader drawing/manufacturing integrations; the host-neutral validator protocol already exists from M4a.

**Current M7A implementation:** a bounded local Assistant accepts only `SetRuleDimension` and `SetFeatureDimension` typed intents under an explicit `LocalAssistant` capability grant. Core derives the command batch, provenance, dependency-scoped read set, typed write set, before/after dimension diff, intended write-result digest, Standard risk, ReviewRequired confirmation, and a host-bounded `1 command / 64 reads / 1 write` budget. Preparation dry-runs the same `apply_batch` path against an isolated snapshot; relevant changes, replay, cross-document use, invalid values/targets, missing capability, and budget violations fail closed. Unrelated changes may be revalidated. Confirm preflights again, applies exactly one canonical batch, verifies the write-result digest, and transactionally restores history and the derived-result registry on any mismatch; Cancel never mutates. The product panel displays this evidence and a verified receipt. Focused Gate D covers both rule and model tasks plus these adversarial boundaries, and the recorded focused suites pass on the Appendix G current-evidence implementation baseline.

**Current M7B implementation:** the constrained extension pilot runs an external Python SDK client behind `ketchup.plugin.v1`. An untrusted process declares a package/version, stable numeric principal, capabilities, and limits; the host separately grants a subset and enforces bounded request lines before full allocation, cumulative request/query bytes, the existing Proposal command/read/write budget, one review-only Proposal, timeout, cancellation, and process termination. The only read is Agent StateView v1 and the only writes are the two proven typed Intents from M7A; the process receives neither `DocumentStore` nor raw OCCT/renderer/commit access, and a trusted caller still performs the same stale-checked verified Proposal commit. Focused acceptance executes the Python process and covers denied capabilities, exhausted request/query budgets, direct-mutation vocabulary, oversized input, timeout/cancellation, stale commit, and one canonical Undo step. This pilot is process separation and protocol least privilege, not an OS filesystem/network sandbox or a production package host.

**Current M7C implementation:** the bounded third-party validator host discovers, installs, and monotonically updates packages whose complete descriptor digest, artifact digest, runtime, egress declaration, and license product identity are covered by an Ed25519 publisher signature. Host-owned trust, publisher/release revocation, and external `Missing`/`Active`/`Expired` paid-license state are rechecked on every resolution; no license secret enters the package or document. Resolution binds the authenticated descriptor to a current document invocation and returns structured `Unavailable` on stale, mismatched, revoked, untrusted, tampered, or unlicensed input without mutation. The executable product path is signed no-import WASM under strict parser, fuel, memory, table, and instance limits; modules receive no filesystem, process, or network imports. Declared egress is performed only by a host TCP broker when the signed publisher declaration intersects a separate host-owned grant for the exact host/port, with timeout, request/response byte limits, and digest-only receipts; focused acceptance confirms the transport against a loopback remote-endpoint fixture. Native packages without a configured OS sandbox remain unavailable, and the loopback proof does not claim a production public-network/TLS deployment.

**Current M18A/M18B1–M18B5/M18C1–M18C46 implementation:** `ketchup-core` defines six high-risk classes and a matching human-only scope, with destination/provider/path required where applicable. A trusted Ed25519 confirmation surface issues tokens only for a non-zero authenticated human distinct from the requester and for at most five minutes. Canonical high-risk approval covers requester, approver, document/revision, base/dependency/command/result digests, risk scope, policy epoch, issue time, and expiry; the ordinary Proposal commit refuses high risk, while the dedicated verified commit retains the unchanged transactional `apply_batch` path. A domain-separated non-canonical side-effect Proposal instead binds operation and exact payload SHA-256 to the same snapshot and scope, and yields a one-use authorization receipt without calling `apply_batch` or changing revision/Undo state. The desktop application configures this policy from an OS-entropy signing key and requires a receipt before overwriting an existing native document, exporting active-document or Beam-workspace lossy OBJ artifacts, or releasing the current Beam manufacturing payload with its explicit warning class. OBJ authorization frames the exact mesh and deterministic loss-report sidecar into one payload before either artifact is written; Beam manufacturing authorization binds the complete deterministic `.kfm` bytes and target path to the Beam source document/revision. Host-mediated validator TCP egress separately consumes an exact provider/destination/request-bound disclosure receipt before connection. Focused `gate_d`, `file_workflow`, app-lib, Beam app, and validator-hosting acceptance covers successful authorization plus refusal, machine/self/anonymous approval, payload/scope substitution, wrong key, expiry, stale snapshot/epoch, replay, unchanged bytes before approval, and unchanged canonical identity after Save/export/egress. The local Assistant additionally prepares occurrence-visibility, exact XYZ occurrence/group translation, definition/evaluator-node-rename, evaluator-expression, flat-rule-output, bottle-profile-control-dimension, bottle-edge-finish-kind, exact-profile-points, tag-visibility, occurrence-tag, occurrence-definition, occurrence-parent, group-parent, exact collection-membership, single-tag, single-collection, empty-group, uncollected-occurrence, empty-definition, and unused exact-profile-feature deletion, and named independent-evaluator-input/evaluator-expression/flat-evaluator-rule creation, resolved-flat-rule-override creation/deletion, resolved feature-parameter binding creation/deletion, single-bound feature-parameter recomputation, canonical-joint, canonical-space, canonical-clearance-volume, and persistent-dimension deletion, and tag/empty-collection/empty-definition/exact-profile-feature/root-group/root-occurrence creation, and bounded empty-group conversion Proposals with distinct capabilities, explicit boolean, Transform, text, BottleEdgeFinishKind, ordered 2D-point-list, exact RuleOutput-tree, EvaluatorInputState, EvaluatorExpressionState, EvaluatorRuleState, RuleOverrideState, FeatureParameterBindingState, JointState, SpaceState, ClearanceVolumeState, PersistentDimensionState, TagState, CollectionState, DefinitionState, GroupState, optional-Tag, optional-Group, Definition-ID, or canonical occurrence-ID-list diff evidence, no preview mutation, fail-closed invalid/missing/stale target handling, and exactly one verified canonical Undo step on confirmation.

**M7 status:** the bounded local Assistant with M18C1–M18C46 occurrence visibility, exact occurrence/group translation, definition and evaluator-node rename, evaluator expression and exact flat rule-output editing, bottle profile-control dimension, edge-finish-kind and exact profile-point editing, tag visibility, occurrence tag assignment/removal, occurrence definition repointing, occurrence group assignment/removal, group reparenting, exact collection membership, single-tag, single-collection, empty-group, uncollected-occurrence, empty-definition, and unused exact-profile-feature deletion, and named independent-evaluator-input/evaluator-expression/flat-evaluator-rule creation, resolved-flat-rule-override creation/deletion, resolved feature-parameter binding creation/deletion, single-bound feature-parameter recomputation, canonical-joint, canonical-space, canonical-clearance-volume, and persistent-dimension deletion, and tag/empty-collection/empty-definition/exact-profile-feature/root-group/root-occurrence creation, and bounded empty-group conversion, M7B constrained Python pilot, M7C signed no-import WASM validator path, M18A core high-risk confirmation gateway, and M18B1–M18B5 overwrite/lossy/disclosure/manufacturing-release receipts preserve one document and commit authority. Natural-language/model-provider integration, broader canonical task coverage, persistent public package catalog/namespaces, native OS sandboxing, production HTTPS/TLS egress and credential brokerage remain later scope.

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
- a named accountable project owner accepts the V4 target and all three dated decision records exist: the P01–P14 adoption ADR (**missing**), the dedicated P07/P08 Open/derived-authority ADR (**missing; ADR 0006 is only an implementation consequence**), and the dedicated P15 sequence ADR (**satisfied by ADR 0004**); every remaining `OPEN` decision is classified by blocking stage, and ratification cannot pass while V4-O10 lacks the named accountable owner;
- no planned exact, evaluator, validator, AI, or geometry capability is described as current;
- external review dispositions are recorded in an appendix or linked review artifact;
- contradictions with accepted ADRs and the execution contract are resolved explicitly;
- code owners accept the migration sequence and named stage exits;
- evidence caveats for A0/A1/B/C are retained;
- the project owner confirms the product emphasis on rules, validation, fabrication outputs, and broad-but-declared geometry.

Until then, V4C is the most complete review draft but remains unratified. Continued implementation may satisfy technical evidence gaps, but it cannot close the two missing owner decision records or V4-O10 by implication and does not retroactively rewrite frozen history.

## 14.4 Change control after acceptance

- Architectural changes require a dated ADR referencing the affected V4 decision IDs.
- Implementation status updates may change `PLANNED` → `PARTIAL—PROOF ONLY` or `PARTIAL—PRODUCT PATH` → `IMPLEMENTED` only with named evidence; proof-only promotion requires evidence on the intended product path.
- Gate evidence is appended/versioned, never rewritten.
- Public scope and compatibility promises require owner approval.
- A change that weakens a frozen operation envelope or threshold is labeled `loosen` and receives explicit review.
- The V4 document should be regenerated or revised when code ownership boundaries materially change, not for every local refactor.


# Appendix A — As-built ownership and evidence map

Line numbers are only a navigation aid for the current-evidence implementation baseline and may move. Tests and symbol names are the durable references.

| Contract | Owning implementation | Focused evidence | V4 status |
|---|---|---|---|
| Typed product entities and hierarchy | `crates/ketchup-core/src/document.rs` | `tests/product_document.rs`, hierarchy/component suites | **PARTIAL—PRODUCT PATH** |
| Typed evaluator graph and expressions | `crates/ketchup-core/src/graph.rs`; `document.rs` | `tests/graph_m2.rs`, `gate_a1.rs` | **IMPLEMENTED narrow; geometry vocabulary incomplete** |
| Hierarchical `SlotPath` and overrides | `graph.rs`; `document.rs` | `graph_m2.rs`, `beam_m4ae.rs` | **IMPLEMENTED narrow** |
| Command batch and atomic apply/rollback | `document.rs::apply_batch` | `gate_a1.rs`, `product_document.rs` | **IMPLEMENTED** |
| Canonical/P07 write authority | `document.rs::apply_batch`; `register_derived_result` | `graph_m2.rs`, `gate_c1b_product.rs`, deliberate-red guard suite; ADR 0006 | **IMPLEMENTED narrow** |
| Proposal preparation/commit/verification | `intent.rs`; `document.rs::commit_verified_proposal`; `document.rs::commit_high_risk_proposal`; app Assistant | `gate_d.rs`, `assistant_m7.rs`, `plugin_m7b.rs` | **PARTIAL—PRODUCT PATH; two bounded intents plus core human-only high-risk confirmation; trusted application issuer wiring remains open** |
| Undo/Redo | `document.rs::undo/redo` | core and app/capstone suites | **IMPLEMENTED canonical** |
| Definitions, local hierarchy, Group→Component, Make Unique | `document.rs` | product/hierarchy/component and headless-shell suites | **IMPLEMENTED current features** |
| Canonical/evaluation digests | `document.rs`; `graph.rs` | graph and persistence suites | **IMPLEMENTED for current contracts** |
| Schema-12 persistence and observational Open | `crates/ketchup-core/src/persistence.rs`; app Open path | persistence/schema-limit/file-workflow, exact-feature registry, M6 bottle, and parameter-binding freshness suites | **PARTIAL—PRODUCT PATH; schemas 0–11 remain readable under bounded compatibility rules** |
| Complete and agent StateView | `crates/ketchup-core/src/state_view.rs` | `tests/state_view_v1.rs` plus independent golden fixtures | **IMPLEMENTED current schema** |
| Designed shell and command registry | `crates/ketchup-app/src/lib.rs` | headless shell, registry, shortcut, and workflow suites | **PARTIAL—PRODUCT PATH** |
| Headless real-widget harness | `ketchup-app/tests/harness/mod.rs` | `capstone_chain.rs` and app integration tests | **IMPLEMENTED** |
| Selection/edit context | `ketchup-app/src/lib.rs` | capstone and focused context tests | **PARTIAL—PRODUCT PATH** |
| Canonical cuboid projection / C1a | `ketchup-interaction` projection; app consumption | C1a and Gate C interaction suites | **IMPLEMENTED narrow box path** |
| Exact triangle interaction/picking | `ketchup-interaction/src/exact_projection.rs`; app exact projection | `gate_c1b_product.rs`, ThroughCut product tests | **PARTIAL—PRODUCT PATH** |
| Exact façade and bounded operation families | `ketchup-exact`; worker binary | exact unit tests, A0 v2 evidence, rectangle/Boolean Cut/Union, fixed M6 bottle, and beam half-lap suites | **PARTIAL—PRODUCT PATH; Extrude, bounded Boolean Cut/Union, rotational Revolve/Shell/Fillet/Chamfer, and beam half-laps** |
| Shared exact packages, registry, and durable references / C1b | `ketchup-core/src/exact_product.rs`; `beam_m5.rs`; `bottle_m6.rs`; scheduler packages; schema 16 | deterministic registry, rectangle/boolean roles, bottle roles, beam-notch roles, Save/Open, C1a/C1b, and running-app groove/cut tests | **PARTIAL—PRODUCT PATH** |
| Editable rotational bottle workflow / M6A | `bottle_m6.rs`; `document.rs`; `ketchup-exact`; scheduler worker; app bottle panel/drag/export path | `product_document.rs` M6 tests, fixed native bottle tests, `gate_c1b_product.rs` bottle tests, app bottle tests | **IMPLEMENTED narrow product path; arbitrary profiles/axes/edges and asymmetric spline/Loft/Sweep remain planned** |
| Scheduler and exact-worker supervision | `ketchup-scheduler`; app integration | Gate B, supervisor, exact product, and capstone suites | **PARTIAL—PRODUCT PATH** |
| Prismatic collision and bounded joints | `ketchup-core/src/prismatic.rs` | `beam_m4ae.rs`, `validation_m4a.rs` | **IMPLEMENTED bounded slice** |
| Host-neutral validator and W4A | `ketchup-core/src/validation.rs` | `validation_m4a.rs`, StateView/BOM/app projections | **IMPLEMENTED bounded built-in slice** |
| Beam pieces, BOM, dimensions | `beam_m4ae.rs`; `fabrication.rs` | `beam_m4ae.rs` | **PARTIAL—PRODUCT PATH; beam scope** |
| Localization | `locales/en-US.ftl`; app locale service | app shell/resource tests; ADR 0001 | **PARTIAL—PRODUCT PATH; FLP incomplete** |
| Canonical mesh body | none | none | **PLANNED** |
| Persistent tags/collections/general dimensions | `document.rs`; `persistence.rs`; `state_view.rs` | `product_document.rs`, `state_view_v1.rs`, `gate_c1b_product.rs` | **IMPLEMENTED bounded canonical entities; complete shell management and arbitrary-point annotation authoring remain planned** |
| Assistant and extension hosts | `intent.rs`; app Assistant; `extension.rs`; `validator_hosting.rs`; scheduler plugin/WASM runtimes; Python SDK | `gate_d.rs`, `assistant_m7.rs`, `plugin_m7b.rs`, core/scheduler `validator_hosting_m7c.rs` | **PARTIAL—PRODUCT PATH; bounded intents/process/WASM paths** |

# Appendix B — V3 canonical task disposition

| # | Task | 2026-08-03 disposition |
|---:|---|---|
| 1 | Exact rectangular profile | **IMPLEMENTED narrow profile path:** exact-value Rectangle commits only canonical Profile + Occurrence, remains selectable as a flat projection, and first Z Push/Pull adds the Extrusion as a separate Undoable batch; the general Line/polyline viewport tool remains incomplete. |
| 2 | Exact profile extrusion | **IMPLEMENTED narrow product path:** canonical height drives supervised exact extrusion, rendering, picking, references, and Save/Open. |
| 3 | Change source extrusion height | **IMPLEMENTED narrow:** Push/Pull changes the canonical extrusion and regenerates the exact package. |
| 4 | Rectangular opening/cut | **IMPLEMENTED narrow product path:** canonical ThroughCut drives OCCT cut, seven semantic roles, hole mesh/picking, references, and Save/Open. |
| 5 | Unambiguous Push/Pull parameter change | **PARTIAL—PRODUCT PATH:** current unambiguous rectangle/extrusion path works and exact evidence regenerates; broad topology-driven Push/Pull is not complete. |
| 6 | Ambiguous Push/Pull choice | **PLANNED broader product behavior:** current narrow tools avoid the general ambiguous-topology choice problem. |
| 7 | Exact vector Move | **IMPLEMENTED for occurrences.** |
| 8 | Snapped Copy | **PARTIAL—PRODUCT PATH:** Ctrl-Copy/shared occurrence works; complete inference/snap UI evidence remains incomplete. |
| 9 | Shared definition and occurrences | **IMPLEMENTED narrow.** |
| 10 | Definition edit updates occurrences | **IMPLEMENTED narrow, including exact regeneration for current features.** |
| 11 | Make occurrence unique | **IMPLEMENTED current feature vocabulary.** |
| 12 | Group and edit context | **PARTIAL—PRODUCT PATH:** hierarchy, Group→Component, and context workflow work; complete arbitrary nested semantics remain bounded. |
| 13 | Tag assignment/visibility | **PARTIAL—CORE PRODUCT PATH:** canonical tags, assignment, deterministic query, persistence, Undo/Redo, and effective root/nested visibility are implemented; canonical collections add persistent deterministic root-occurrence membership without another ownership tree; the complete shell-management workflow remains planned. |
| 14 | Linear pattern | **IMPLEMENTED bounded:** exact axis spacing and count produce a snapshot-bound rendered preview, then one canonical occurrence-creation batch with Undo/Redo and one shared definition. |
| 15 | Parameter expression | **IMPLEMENTED narrow:** bounded typed expression nodes evaluate deterministically. |
| 16 | Dependent-only recompute | **IMPLEMENTED narrow:** affected-only evaluator recomputation and stable unrelated result identity are tested. |
| 17 | Save/Open dimensions/references | **PARTIAL—PRODUCT PATH:** schema 16 round-trips current identities, graph, overrides, joints, exact references, persistent dimensions, tags, collections, bottle/exact feature chains, and rule→feature binding provenance without semantic loss; long-term compatibility and arbitrary-point annotation authoring remain incomplete. |
| 18 | Batch Undo/Redo one step | **IMPLEMENTED canonical state; P07 results add no Undo step.** |
| 19 | Exact or mesh export with loss report | **IMPLEMENTED bounded product path:** the current accepted bottle exports ISO 10303 STEP and deterministic OBJ with explicit editability/topology/tolerance loss reports; STEP round-trip and stale rejection are tested. Arbitrary-body exact export and mesh-to-exact remain planned. |
| 20 | Risky AI Proposal workflow | **PARTIAL—PRODUCT PATH:** two bounded Standard-risk dimension intents expose assumptions, authoritative diff, explicit ReviewRequired confirmation, capability/budget checks, stale/replay rejection, one canonical Undo step, and postcondition verification in the product Assistant; human-only high-risk classes and broader task coverage remain incomplete. |

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

- current app is a narrow canonical/exact extrusion-and-ThroughCut modeler, not a general exact CAD app;
- interaction geometry is derived cuboid fallback or revision-bound exact triangles, never a second canonical B-Rep;
- the exact worker is connected to the product application through scheduler supervision for the bounded M3 slice;
- `apply_batch` is the canonical semantic path and P07 `register_derived_result` is the only non-canonical result path; construction/lifecycle operations are explicitly classified and guarded;
- evaluator nodes perform bounded typed expression/rule evaluation and affected-only recomputation, but universal geometric feature-result unification remains incomplete;
- schema 16 stores evaluator identity, graph/override/joint state, limits/checksum, exact-reference evidence, shared exact feature forms, parameter bindings, and freshness provenance with observational Open;
- M4a implements a bounded prismatic beam/validator/BOM/dimension slice with W4A evidence classes;
- historical gate evidence remains scoped to frozen source/harness and is not automatic current certification;
- C1b is real but narrow, and terminal integrated-GPU Gate C evidence remains incomplete.

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
| R32 | Same | Nine-item M4a delayed the early product signal that justified placing it before M3. | **Accept, final refinement** | P15, §§10.5 and 13 define M4a-E: tolerance → collision/containment → bounded joints → `SlotPath` override → beam with grouped piece/length list. The early fixture also asserts generated positions against `415 × 6`, `408 × 5`, `400`; only rendering that sequence as dimension-chain output waits for the completion track. |
| R33 | Same | Independent face offset by `ε` is a permissive superset of the Euclidean Minkowski inflation near edges/corners and can hide illegal overlap. | **Accept, final refinement** | §10.3 uses `K_ε = J ⊕ B_ε` and, for the convex-polyhedral scope, certifies containment by testing `distance(v, J) ≤ ε` at every vertex of `I_ij`; face-shift over-approximation cannot be the final acceptance test. |
| R34 | Same | The living-tree slice is harder than the bottle and unrelated to the first fabrication path, so it must not block product exits. | **Accept** | §§1.6, 2.3, 12.4, 13, and 14.2 mark it as a non-blocking research/acceptance slice. |
| R35 | Same plus project-owner clarification | Structural/statics packages create certification/liability ambiguity; the owner retains statics but rejects any claim of 100% certainty. | **Accept with clarification** | P14 and §§10.1 and 14.1 permit transparent best-effort structural/statics results but prohibit Ketchup safety/code guarantees; professional approval and any structural guarantee require independent review and signature by a qualified structural engineer; jurisdictional approval is supplementary. |

| R36 | External human architecture reviewer / 2026-08-04 | Validation results distinguished *what* is asserted (regulatory/structural/manufacturability/advisory) but not *how decidable* the assertion is; `Passed` therefore meant a decided predicate over prismatic bodies and a threshold-dependent assessment over curved bodies, with no way to tell them apart, no rule for mixed pairs, and no separation in aggregate reports. | **Accept** | New decision **W4A**; §10.1.1 defines the `Exact`/`Tolerant` evidence class with weakest-participant propagation and separated aggregate reporting; §§8.1, 10.2, 10.3, and 10.5 carry it; introduction is required in M4a-E while every result is still `Exact`, because retrofitting after the first curved body would leave historical results unclassifiable. |

| R37 | External human architecture reviewer / 2026-08-04, from project-owner product evidence | Clearance and advisory validation — including the explicitly admitted feng-shui class — has a complete host contract but no declared inputs: the canonical document models pieces, not space. Room identity, declared purpose, adjacency/access, and element clear volumes are absent, so a validator would have to infer its own inputs from geometry, which is neither deterministic nor third-party authorable. Evidence: a FurniGen small-house run produced usable walls/partitions but placed storage in front of a window, because “a window keeps the volume in front of it clear” was never expressible. | **Accept as target state** | New decision **W4B**; §10.9 defines `Space` and `ClearanceVolume` as canonical entities with declared purpose, adjacency/access, rule-derivable clear volumes under `SlotPath`, and anthropometric access modelled through the same mechanism; §10.8 records the small-house observation. Explicitly not FLP-blocking, but the contract MUST be designed jointly with §10.3 joint volumes, of which it is the sign-inverted twin sharing the same containment machinery. |

**Review verdict:** `NO-GO for ratification`, retained after six review rounds. Round six produced two further material findings — R36, incorporated as W4A, and R37, incorporated as W4B. The architecture review cycle is otherwise closed unless later evidence or a new product requirement invalidates an assumption. ADR 0004 satisfies P15 and ADR 0006 accepts only the immediate write-path consequence; the P01–P14 adoption ADR, the broader P07/P08 ADR, and a named accountable project owner remain missing. V4-O01–O09 are explicitly staged non-ratification decisions; V4-O10 blocks ratification. This document does not invent the person or record acceptance that has not been explicitly supplied.

# Appendix E — Document source set

Primary normative and historical sources used for this draft:

- `KETCHUP_ARCHITECTURE_PROPOSAL_V3.md`
- `docs/design/EXECUTION_CONTRACT.md`
- `docs/design/README.md`
- `docs/design/IMPLEMENTATION_PLAN.md`
- `docs/adr/0001-project-language-and-localization.md`
- `docs/adr/0002-exact-backend-isolation.md`
- `docs/adr/0003-splash-screen-and-version-display.md`
- `docs/adr/0004-v4-p15-sequence-and-a0-disposition.md`
- `docs/adr/0005-no-go-diagnostic-hold.md`
- `docs/adr/0006-canonical-and-derived-result-write-paths.md`
- `docs/adr/0007-windows-x86-64-first-release.md`
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
- complete and agent StateView v1 golden examples;
- current test/gate disposition with exact build fingerprint;
- one current `.ketchup` schema-16 fixture and decoded semantic inventory;
- one narrow manual capstone trace;
- one A0 reference fixture and one Gate B worker trace;
- explicit questions from section 14.2.

The **target architecture** is intended to be understandable without reading source. Independent verification of the **as-built** claims still requires source until the complete StateView, decoded fixture, dependency/API map, and execution traces above are attached. Ratification cannot rely on this narrative alone.

# Appendix G — Current evidence manifest

This appendix is the single authority for the current-evidence commit. Other sections refer here and MUST NOT repeat its hash. The M11 document-only working-tree observations below do not alter implementation behavior, manufacture a new A0 freeze, or replace historical sealed evidence.

| Field | Value |
|---|---|
| As-built update date | 2026-08-07 |
| Current-evidence commit | `55da1700f2bca3fc3ea467be34c1b675deb7e883` (`Complete exact-body integration and freshness gates`) |
| Implementation scope | Immutable committed M6/M7 paths plus shared exact feature/result integration, canonical rule→feature parameter bindings and explicit recompute, and running-app multi-body exact groove/cut acceptance through M10 |
| M11 observation scope | No tracked implementation change from the current-evidence commit; only this V4C documentation update is tracked. The pre-existing generated `sdk/python/ketchup_sdk/__pycache__/` remains untracked, untouched, and outside evidence scope. |
| OS | Windows 10 10.0.19045 x64 |
| Rust | `rustc 1.97.0 (2d8144b78 2026-07-07)` |
| Cargo | `cargo 1.97.0 (c980f4866 2026-06-30)` |
| Current Cargo.lock SHA-256 | `9e618823c78beb535f335dcd77c7ebd19168d7682a033e7159cd4dae6b5fb722` |
| Freeze status | No new freeze. Historical A0/A0 v2 artifacts retain their original source/input/backend scope. |
| Specification status | As-built statements report this named implementation scope. Target-decision ratification and the complete requirement→evidence→gap program are separate M11 records and do not follow from green implementation tests. |

Focused M11 revalidation against the unchanged implementation baseline plus this document-only diff:

| Command | Outcome |
|---|---|
| `cargo build --locked -p ketchup-scheduler --bin ketchup-exact-worker` | PASS; current exact worker built for product-path tests |
| core `product_document`: `explicit_feature_parameter_recompute_is_deterministic_undoable_and_identity_bound` and `feature_parameter_recompute_rolls_back_every_target_when_one_value_is_invalid` | PASS; 2/2, proving one explicit canonical recompute/Undo step, persisted provenance/freshness, and atomic fail-closed rollback |
| scheduler `gate_c1b_product`: `explicit_parameter_recompute_restores_exact_registry_render_pick_and_export` | PASS; 1/1, proving stale registry rejection and restoration of current exact render/pick/export only after recompute |
| interaction `gate_c1a_projection_authority` | PASS; 4/4, including multiple extrusions, exact-only Cut without filled proxy fallback, stale projection rejection, and canonical authority fields |
| app-lib `running_app_uses_one_exact_cut_body_for_render_pick_and_export` | PASS; 1/1, proving running-app worker→registry→render/pick/export and stale fail-closed behavior without canonical mutation by evaluation |
| `scripts/windows/test-architecture-guards.ps1` | PASS; sole mutation, legacy absence, projection authority, StateView fixtures, sealed-A0 separation, optional frozen inputs, and anti-loosening governance |

The broader M6/M7 and historical gate observations remain valid only for their recorded commits and envelopes; this M11 update does not silently promote them to current release certification. Cargo excludes sealed A0 tests from daily discovery unless `a0-certification` is explicitly enabled. The dedicated A0 runner remains the only conforming certification invocation because it supplies a sealed run ID, lock hash, backend identity matrix, runtime libraries, and unique evidence paths. Test counts and durations are observations, not permanent requirements; claim boundaries remain those in §5.9, §12.2, and Appendix A.

# Appendix H — Requirement-to-evidence-to-gap completion matrix

This appendix is the auditable completion ledger for the retained FLP and the cross-cutting target contracts needed to support it. A row is complete only when its evidence runs on the intended product path and every listed gap ID is closed; a proof-only harness or a narrower adjacent feature does not close it. Every autonomous implementation gap has exactly one delivery owner in M12–M19. Owner ratification records are listed separately because code and tests cannot manufacture human acceptance.

## H.1 FLP capability coverage

| Requirement | Current evidence | Current disposition | Remaining gap IDs | Delivery owner |
|---|---|---|---|---|
| FLP-01 — create a document with explicit units | Canonical document/unit persistence and app New/file workflow suites; Appendix A | **IMPLEMENTED narrow** | G19-02 physical packaging/file-dialog continuity | M19 |
| FLP-02 — draw and dimension a simple profile | Profile-only Rectangle/closed polyline, atomic validation, flat select/pick, separate first extrusion, numeric rectangle constraints, and canonical persistent feature/`SlotPath`/exact-semantic dimensions with declared units | **IMPLEMENTED narrow profile path** | G15-01 broader validated profiles and operation coverage | M15 |
| FLP-03 — extrude a profile into an exact solid | Worker-backed rectangle extrusion → registry → render/pick/reference → Save/Open | **IMPLEMENTED narrow rectangle path** | G15-01 broader validated profiles and transform-aware operation contract | M15 |
| FLP-04 — change dimensions without cumulative drift | Exact numeric edits, typed rule→feature binding, explicit dependent-only recompute, persistent associative dimension projection, Undo/Redo and schema-16 provenance | **IMPLEMENTED for current parameter vocabulary** | G15-01 broader feature-parameter coverage | M15 |
| FLP-05 — create a simple opening, cut, or union | Shared Boolean Cut/Union exact path; ThroughCut render/pick/reference/Save/Open evidence | **IMPLEMENTED bounded product path** | G15-01 general operands, profiles and boolean envelopes | M15 |
| FLP-06 — Smart Push/Pull with operation explanation | Unambiguous root and context-safe nested rectangle/extrusion Push/Pull, snapshot/context-bound preview, one canonical batch, exact recompute, Undo/Redo and stale rollback tests | **IMPLEMENTED narrow rectangle path** | G15-02 explicit ambiguous/lost/quarantine paths and broader operation families | M15 |
| FLP-07 — definitions, occurrences, groups, tags and collections | Definitions/occurrences/groups/components/Make Unique plus canonical tag assignment/query/persistence/effective-visibility and canonical non-owning collection membership/query/Undo/Open suites | **PARTIAL—PRODUCT PATH** | Complete shell organization workflow | M13 |
| FLP-08 — precise move, copy, snap, align and simple pattern | Canonical Move/Copy, exact occurrence Align, bounded axis linear pattern, snapshot-bound rendered preview, one canonical batch, Undo/Redo, shared-definition preservation, deterministic snap scoring/hysteresis/overlap choice and context-safe selection | **IMPLEMENTED bounded product path** | Broader transform/pattern families and spatial acceleration | M15–M16 |
| FLP-09 — Save/Open, Undo/Redo and last-commit survival | Atomic schema-16 Save/Open, failed-Open preservation, canonical Undo/Redo, supervised worker stale/crash boundaries | **PARTIAL—PRODUCT PATH** | G14-01 container/blob/unknown-extension policy; G14-02 migration/recovery/compatibility evidence; G16-01 general worker restart/reschedule proof | M14, then M16 |
| FLP-10 — one exact and one mesh export | Current accepted bottle ISO 10303 STEP plus deterministic OBJ, each with explicit loss evidence; STEP OCCT round-trip and stale rejection; beam manufacturing export | **IMPLEMENTED bounded product path** | Arbitrary accepted-body exact export and mesh-to-exact remain later coverage | M14 complete; broader conversion in M15 |
| FLP-11 — selected tasks through explainable Proposal/diff | Two typed dimension intents, product Assistant, plugin pilot and focused Gate D | **PARTIAL—PRODUCT PATH** | G18-01 broader intent/task corpus and human-only high-risk confirmation; G18-02 provider/egress policy | M18 |

## H.2 Cross-cutting target-contract coverage

| Requirement group | Current evidence | Current disposition | Remaining gap IDs | Delivery owner |
|---|---|---|---|---|
| SYS-01 — sole canonical mutation authority, atomic batch, one Undo step, observational Open | `apply_batch`, `register_derived_result`, ADR 0006 and architecture guards | **IMPLEMENTED for current paths** | Preserve as an invariant in every milestone; DG-02 still governs long-term retained-result policy | M12–M19 regression constraint; owner decision |
| SYS-02 — typed evaluator DAG, deterministic affected-only recompute and stable derivation identity | Graph/override/`SlotPath`/parameter-binding/freshness and rectangle-constraint suites | **PARTIAL—PRODUCT PATH** | G15-01 universal supported feature-result coverage | M15 |
| SYS-03 — canonical hierarchy, tags, dimensions and context-safe queries | Product hierarchy, schema-16 dimensions/tags/collections, exact-semantic health, core-owned visibility and document/revision/digest-bound direct nested scene queries with stale/hidden/out-of-context regressions | **IMPLEMENTED for current vocabulary** | Future entity families must preserve the same query envelope | M15–M19 regression constraint |
| SYS-04 — explicit exact/mesh authority and conversion loss | Exact package/registry, derived tessellation, bounded OBJ loss report, detached sole-authority canonical mesh conversion with schema-16 loss provenance, and read-only shared mesh query projection | **PARTIAL—PRODUCT PATH** | Mesh import/procedural/render support and mesh-to-exact conversion | M15 |
| SYS-05 — stable body/assembly references with no silent retargeting | Bounded rectangle/boolean/bottle/beam roles, persistence and C1b | **PARTIAL—PRODUCT PATH** | G15-02 complete `Ambiguous`/`Lost`/audit/quarantine behavior across supported operations | M15 |
| SYS-06 — general scheduler, interaction and renderer under production budgets | General typed scheduler lifecycle/cache telemetry plus one snapshot-bound deterministic BVH across canonical, accepted-exact and canonical-mesh interaction projections, with measured sublinear candidate evidence | **PARTIAL—PRODUCT PATH** | G16-01 integrated producer/resource plateau; G16-02 renderer instancing/derived GPU cache/highlights; G16-03 10,000-occurrence product proof | M16 |
| SYS-07 — deterministic validation and fabrication projections over arbitrary documents | Bounded prismatic/exact validation, joints, W4A, beam outputs, M17A general exact/mesh collision/clearance, M17B validation-bound fabrication projections, and M17C canonical schema-17 `Space`/`ClearanceVolume` with shared signed containment and fail-closed `SlotPath` health | **PARTIAL—PRODUCT PATH; M17 exits complete** | Richer curved/mesh drawing and manufacturing semantics remain outside bounded G17 scope; BTLx is explicitly excluded while V4-O05 remains owner-open | M17 |
| SYS-08 — capability-limited AI/plugin surface with fail-closed confirmation and egress | Proposal safety, local Assistant, Python process pilot, signed no-import WASM host | **PARTIAL—PRODUCT PATH** | G18-01; G18-02; G18-03 production catalog/sandbox/TLS/credential brokerage | M18 |
| SYS-09 — localization, accessibility, packaging and current certification | Complete English and Slovak resources, deterministic argument-preserving pseudo-locale, strict exact-key completeness checks, translated 1600 x 1000 shell layout, localized AccessKit names/focus/keyboard activation, WCAG palette checks, fail-closed Windows technical-candidate packaging with a pinned exact DLL set, co-located worker process proof, foreign-working-directory GUI launch with live OCCT module provenance, ADR 0007 Windows-first platform selection, historical gate artifacts and focused current tests | **PARTIAL—PRODUCT PATH / PARTIAL—PROOF ONLY; G19-01 complete, V4-O08 resolved, G19-02 technical candidate only** | G19-02 physical native-dialog continuity; G19-03 20 canonical tasks; G19-04 current R0/A0/A1/B/C hardware evidence | M19 |

## H.3 Complete implementation-gap ledger

The following IDs are exhaustive for the M11 as-built comparison. New scope requires change control under §14.4 rather than silently extending a milestone.

| Gap ID | Exit evidence required | Fixed milestone |
|---|---|---|
| G12-01 | Profile-only Rectangle and closed polyline preserve exact coordinates and reject closure, duplicate, self-intersection, winding, hole, tolerance and envelope violations atomically. | M12 |
| G12-02 | Numeric constraints drive supported profile parameters deterministically, persist losslessly, recompute only dependents and roll back one batch on invalid input. A general solver is required only if the frozen workflow and V4-O03 decision require it. | M12 |
| G12-03 | Persistent associative dimensions target stable entity/`SlotPath`/exact references, display declared units, survive Save/Open and become unresolved rather than silently retargeted. | M12 |
| G13-01 | Tags and collections are canonical, queryable, persistable and drive visibility without application-owned authority. | M13 |
| G13-02 | Align and one linear-pattern workflow use exact input, preview, one batch, Undo/Redo and shared-definition semantics. | M13 |
| G13-03 | Snap/inference scoring, hysteresis, overlap choice and value-box behavior have complete viewport product evidence. | M13 |
| G13-04 | Nested selection/query/edit commands are snapshot-bound, context-safe and cannot leak to hidden or out-of-context entities. | M13 |
| G14-01 | Native container records manifest/document/blob/cache/extension policy, checks safe paths, preserves safe unknown namespaced data and never relies on cache for meaning. V4-O01 is decided before format freeze. | M14 |
| G14-02 | Explicit confirmed migration operates on a copy, recovery preserves the last commit, old-schema and backend-change matrices pass, and any public promise obeys V4-O06. | M14 |
| G14-03 | Owner-selected standard exact and mesh formats pass license/tolerance fixtures, accepted-body export and explicit editability/topology/tolerance loss reporting under V4-O04. | M14 |
| G15-01 | Approved profile, transform, boolean and edge-operation families share canonical feature specs, worker requests, deterministic registry, direct/numeric edits, Undo/Redo and persistence. | M15 |
| G15-02 | Every supported mutation/reference corpus returns resolved, `Ambiguous`, `Lost` or quarantined explicitly; stale/backend-changed evidence never silently retargets or exports. | M15 |
| G15-03 | Canonical `MeshBody` and explicit exact→mesh/mesh→exact operations preserve provenance, validation and loss reports without creating a second authority. | M15 |
| G16-01 | General jobs expose status/progress/cancel/deadline/restart/reschedule, stale rejection, telemetry and bounded cache/eviction with a measured plateau. | M16 |
| G16-02 | Shared spatial queries, BVH/instancing and render caches support exact/mesh picking, snaps and highlights without renderer-owned identity. | M16 |
| G16-03 | The real product proves 10,000 occurrences with one shared authoritative geometry and declared frame/input/query thresholds on required hardware. | M16 |
| G17-01 | Collision and clearance consume arbitrary current accepted exact/mesh bodies, propagate W4A classes and fail closed on unavailable or stale evidence. | M17 |
| G17-02 | General associative dimensions, views/drawings, BOM and manufacturing projections regenerate deterministically from arbitrary supported documents. | M17 |
| G17-03 | Canonical `Space` and `ClearanceVolume` carry purpose/adjacency/access and rule-derived `SlotPath` identity, sharing certified containment machinery with joint volumes. | M17 |
| G17-04 | BTLx remains excluded unless V4-O05 selects scope and tolerances after the general projection evidence passes. | M17 |
| G18-01 | Frozen manual tasks map to broader typed intents and Gate D; high-risk actions require digest-bound, expiring, human-only approval and adversarial stale/replay/TOCTOU tests. | M18 |
| G18-02 | Local-first is enforced; provider upload, retention and egress require explicit per-operation/workspace opt-in under V4-O09 with auditable receipts. | M18 |
| G18-03 | Persistent package catalog/namespaces, update/revocation, native OS sandbox, production HTTPS/TLS and host-owned credential brokerage fail closed under quotas. | M18 |
| G19-01 | English, a second real locale and pseudo-locale pass resource completeness, layout, keyboard/focus, contrast and screen-reader accessibility evidence. | M19 |
| G19-02 | Release packaging proves pinned runtime DLL discovery, New/Open/Save/Save As file dialogs and atomic failure continuity under the Windows x86-64 first-release decision recorded by ADR 0007. | M19 |
| G19-03 | All 20 canonical tasks have frozen fixtures and pass through discoverable product paths with authoritative state/diff/loss evidence as applicable. | M19 |
| G19-04 | A new immutable current-tree certification bundle honestly executes R0/A0/A1/B/C on required hardware, including integrated GPU, without reusing historical freshness. | M19 |

## H.4 Owner-only decision ledger

| Decision gap | Why implementation cannot close it | Latest blocking stage |
|---|---|---|
| DG-01 — V4 adoption ADR for P01–P14 | Requires explicit dated acceptance, rejection or amendment by a named accountable project owner. | V4 ratification and M19 release readiness |
| DG-02 — dedicated P07/P08 retention/trust/compatibility ADR | ADR 0006 accepts only current write-path consequences; retained-result trust and compatibility policy require owner choice. | M14 format/compatibility freeze and V4 ratification |
| DG-03 — V4-O10 named owner, budget and release quality bar | Role labels, code and passing tests are not accountable human governance. | V4 ratification and M19 release readiness |

V4-O01/O04/O06 are decision inputs to M14, V4-O03 is conditional in M12, V4-O05 is conditional in M17, V4-O08 is resolved by ADR 0007 for a Windows x86-64 first release, and V4-O09 blocks cloud defaults in M18. V4-O02 remains a prerequisite only if work expands beyond the bounded expression vocabulary; V4-O07 does not block the FLP. Missing owner decisions stop the affected exit rather than authorizing an inferred default, except where the register explicitly defines a safe no-feature default such as no cloud path or no BTLx claim.

## H.5 Fixed completion sequence

| Order | Milestone | Entry dependency | Exit summary |
|---:|---|---|---|
| 1 | M12 — profiles, constraints and dimensions | M11 ledger frozen | Close every G12 gap through canonical `apply_batch` and observational Save/Open. |
| 2 | M13 — organization and precise manual operations | M12 stable profile/dimension identities | Close every G13 gap without app-owned model authority. |
| 3 | M14 — persistence and exchange | M12–M13 canonical vocabulary stable; required owner format decisions available | Close every G14 gap with explicit migration/recovery/loss evidence. |
| 4 | M15 — geometry/reference generalization | M14 persistence envelopes can represent the new contracts | Close every G15 gap through the deterministic exact registry and explicit mesh authority. |
| 5 | M16 — scheduler/interaction/renderer productionization | M15 accepted body/reference vocabulary stable | Close every G16 gap and produce the 10,000-occurrence product measurement. |
| 6 | M17 — general validation and fabrication | M15 body/reference and M16 query contracts stable | Close every G17 gap; omit BTLx if its owner decision is absent. |
| 7 | M18 — production AI and extensions | Proven M12–M17 manual/domain operations define safe intents | Close every G18 gap while retaining human confirmation and local-first defaults. |
| 8 | M19 — FLP release readiness | M12–M18 technical exits complete | Close every G19 gap, rerun the full matrix, and report DG-01–DG-03 honestly; release cannot be claimed while a blocking owner record is missing. |

The explicit post-FLP non-goals in §2.4 remain outside this queue: full BIM/IFC, professional multi-sheet drafting, mechanical simulation CAD, organic sculpting/animation, production-scale vegetation, a public marketplace, browser CAD and simultaneous collaboration. They are neither hidden gaps nor acceptable reasons to delay M12–M19.

---

# Final statement

Ketchup has crossed from disconnected experiments into a narrow coherent product substrate with a manual modeler, evaluator/schema-16 persistence, prismatic fabrication/validation, shared exact rectangle/Boolean/bottle/beam packages, deterministic multi-body registry, bounded exact-dependent validation, parameter-driven canonical recompute, and capability-limited Assistant/Python/WASM extension paths. It has not crossed into the complete FLP or a general exact, rule-driven 2D/3D product: the implemented slices remain bounded and the 27 technical gaps in Appendix H remain open.

The fixed next program is M12→M19. Each milestone must preserve canonical `apply_batch`, observational Open, deterministic exact-result acceptance, explicit exact/mesh authority and fail-closed reference/export/automation behavior; no green technical result may invent the owner decisions DG-01–DG-03 or silently expand the post-FLP non-goals.
