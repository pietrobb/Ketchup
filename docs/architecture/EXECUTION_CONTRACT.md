# Ketchup Architecture V3 Execution Contract

- Status: Frozen execution baseline
- Date: 2026-08-01
- Scope: R0 through the First Lovable Product
- License: Apache-2.0
- Initial platform: Windows

This English contract preserves the binding product scope, architecture decisions, sequencing, and fail-able gate metrics of the reviewed Architecture V3. It is the working contract for implementation. Architecture changes require a dated ADR and must not rewrite historical gate evidence.

## 1. Product contract

Ketchup is a fast desktop parametric modeler for architecture, interiors, and furniture. It combines direct SketchUp-like interaction, precise canonical dimensions, an exact CAD backend, and an optional safe AI assistant.

The product must remain usable without AI. UI, CLI, plugins, and AI use the same validated canonical command path. AI may propose explainable changes, but it cannot bypass validation, budgets, preview, confirmation, transactionality, or undo.

### First Lovable Product capabilities

A user can:

1. create a model with explicit units;
2. draw and dimension a simple profile;
3. extrude it to an exact solid;
4. change dimensions without cumulative drift;
5. create a simple opening, cut, or union;
6. use Smart Push/Pull with an explanation of the resulting operation;
7. organize definitions, occurrences, groups, tags, and collections;
8. move, copy, snap, align, and create simple patterns precisely;
9. save, load, undo/redo, and recover the last committed revision after a worker crash;
10. export at least one exact format and one mesh format, subject to gate evidence;
11. execute selected tasks through an AI Proposal workflow that shows assumptions and a diff before commit.

The FLP is not full BIM, professional drawing production, mechanical assembly CAD, organic sculpting, production-scale nature generation, a plugin marketplace, browser CAD, or simultaneous multi-user editing.

## 2. Binding decisions

- Rust owns the canonical document, protocol, scheduler, renderer, and application.
- Open CASCADE Technology is the initial exact B-Rep backend behind a narrow versioned C++ facade.
- OCCT types, raw pointers, ownership rules, exceptions, and thread-safety details do not cross the facade.
- `wgpu` is the renderer baseline. The UI framework remains a measured PoC choice.
- The document uses immutable revisions with one writer and multiple snapshot readers.
- Only a validated `CanonicalCommandBatch` mutates the document.
- Dragging and preview are ephemeral interaction state, not document transactions.
- The canonical document—not the command log—is the source of truth.
- `ExactBody` and `MeshBody` are distinct canonical types. Mesh is never an implicit exact fallback.
- Topological references use explicit stability classes: `Guaranteed`, `BestEffort`, `Ephemeral`, `Ambiguous`, and `Lost`.
- AI uses an `Intent -> Proposal -> CanonicalCommandBatch -> Verification` workflow.
- One committed user `CommandBatch` is one visible undo step.
- Deferred domains do not enter the PoC.

## 3. Dependency boundaries

```text
UI / CLI / trusted tools / AI
              |
          Intent layer
              |
         Proposal layer
 assumptions / read-write set / risk / preview / digest
              |
     Canonical command gateway
 schema / capability / precondition / budget / transaction
              |
  Revisioned canonical document
 entities / parameters / feature specs / stable references
              |
       Evaluation scheduler
 dirty DAG / generation / cancellation / stale rejection
        |                         |
 exact geometry             sketch / mesh /
 facade or worker           procedural services
        |                         |
 interaction and spatial query service
 exact hit / snaps / inference / reference resolution
              |
       derived render data
              |
         wgpu renderer
```

The document core knows neither OCCT nor widgets nor GPU buffers. Geometry services know neither natural language nor user permissions. The renderer does not decide exact CAD identity. Importers, plugins, and AI are untrusted capability- and budget-limited clients.

## 4. Canonical document and revisions

Canonical state contains stable IDs, document/asset scope, schemas, parameters, units, expressions, feature specifications, input references, semantic entities, explicit exact or mesh bodies, reference diagnostics, content-addressed blob references, determinism envelope, and backend provenance.

B-Rep caches, feature results, tessellation, BVHs, thumbnails, and GPU buffers are derived and replaceable.

Each worker job carries revision, generation token, and input digest. A result is inserted only if all still match; stale results are discarded. A renderer may display the last good result only when visibly marked stale.

Undo navigates canonical revisions, invalidates or cancels jobs, and never restores stale derived data as current. Revisions use structural sharing. Derived caches have revision tags, explicit memory budgets, and eviction. Repeated edits must reach a measured memory plateau after warm-up.

## 5. Geometry and precision

```rust
enum CanonicalBody {
    Exact(ExactBodySpec),
    Mesh(MeshBodySpec),
}
```

Exact-to-mesh or mesh-to-exact conversion is an explicit operation with provenance and a loss report.

Every exact operation returns a shape handle, topology history, tolerance report, structured diagnostics, result fingerprint, history confidence, and automatic shape-validity result. The facade catches every C++ exception and exposes only owned Ketchup types.

Backend `Generated/Modified/Deleted` history is evidence, not assumed complete truth. A `Guaranteed` reference cannot rely on undocumented history. Post-operation topology walkers and diffs supplement backend evidence where required.

Public dimensions have explicit units. Canonical values survive save/load unchanged. Measured or rendered approximations never overwrite authoritative parameters. The central versioned tolerance profile belongs to the determinism envelope. Georeferencing is a transform over a local model.

Ketchup guarantees deterministic canonical data and geometric equivalence within declared tolerances, not bit-identical B-Rep or mesh blobs on every backend and platform.

## 6. Topological references

A `SubshapeRef` may include document scope, schema version, producer feature, output port, semantic role, source element, lineage path, expected geometry type, adjacency and geometric signatures, expected cardinality, stability class, and backend provenance.

Resolution uses semantic role, evidenced backend history, lineage, topology/adjacency signatures, and geometric fingerprints in that order. A fingerprint proves similarity, not identity. The only valid outcomes are `Resolved`, `Ambiguous`, or `Lost`; silently selecting a different subshape is always an error.

The A0 `Guaranteed` subset is frozen before measurements. It initially targets the top and bottom of a simple extrusion and a side face derived from a specified profile edge. Evidence may narrow this subset only through the preregistered failure consequence; it cannot be expanded without proof.

Opening a document under a different backend fingerprint invalidates caches and runs a non-destructive reference audit. The original remains untouched. Affected branches enter compatibility quarantine when required. Migration becomes explicit only after review and commit.

## 7. Interaction and Smart Push/Pull

The interaction service owns exact hit testing, overlapping candidates, snaps, inference, scoring, hysteresis, hover locking, selection filters, and conversion to `SubshapeRef`. The renderer supplies only coarse GPU candidates and visual highlighting.

```text
gesture
-> ephemeral interaction state
-> inexpensive transform or mesh preview
-> numeric HUD + snap + localized action digest
-> confirmation
-> one CanonicalCommandBatch
-> exact recompute
```

Preview is not a promise of final exact geometry. Any mismatch or failure after commit is explicit.

Smart Push/Pull may modify a source extrusion parameter only with unambiguous provenance, or may add an offset/extrude feature, cut/opening, or body. Ambiguity presents choices. The safe default is a new feature, never an unexplained distant parameter change.

All action digests are localized resource messages. Widgets contain no hard-coded user-facing prose; ADR 0001 is binding.

## 8. Proposal and commit safety

A Proposal records provenance revision, assumptions, risk, diff, intended command digest, and validity contract. The core—not the client—computes authoritative read/write sets, transitive input fingerprints, relevant query and selection state, policy epochs, global invariants, tolerance/schema versions, and planned command digest.

Before commit, the dependency digest is recomputed. A relevant change makes the Proposal stale; it is never silently rebased. An unrelated revision may be revalidated without a new language-model plan.

The commit pipeline is:

1. schema and capability validation;
2. read/write/dependency calculation;
3. preconditions and resource budgets;
4. isolated dry-run;
5. geometry and domain validation;
6. authoritative diff and digest;
7. confirmation for risky changes;
8. dependency-digest revalidation;
9. atomic commit or rollback;
10. new revision, audit, and scheduling.

Dry-run cannot mutate the document or poison shared caches.

## 9. AI, privacy, and security

Model, document, workspace, prompt, and telemetry data remain local unless the user explicitly opts in for a specific operation or workspace. Cloud upload and other high-risk operations require confirmation.

Document metadata is untrusted data, never system instruction. Threats include prompt/tool-output injection, geometry denial of service, TOCTOU, preview mismatch, capability escalation, malicious importers, cloud exfiltration, cache poisoning, path traversal, archive bombs, and sensitive audit logs.

Every batch has limits for command count, entity count, topology growth, wall time, CPU, RAM, I/O, and concurrent jobs. Risky importers run behind process or sandbox boundaries with quotas.

## 10. Native file contract

```text
model.ketchup
|- manifest.json
|- document.bin
|- audit/commands.log       # optional
|- blobs/<content-hash>
|- cache/                   # disposable
|- previews/
`- extensions/<namespace>/
```

Saving is atomic and crash-recoverable. Paths are safe and checksummed. Schemas are versioned and migrations explicit. Unknown namespaced data is preserved when safe. No migration operates on the only copy. Canonical round-trip has no drift. Cache is never the sole carrier of meaning.

No public compatibility promise is made before old-schema migration tests and at least one backend-build migration test pass.

## 11. Mandatory sequence

The order is fixed by risk:

1. R0: licensing inputs, pinned backend/toolchain, corpora, hardware, thresholds, owners;
2. A0: exception-safe facade, extrude/cut, topology evidence, reference survival;
3. A1: canonical document, revisions, save/load, precision, UI/protocol equivalence;
4. B: scheduler, memory, cancellation, crash recovery, worker/in-process decision;
5. C: viewport, exact picking, snapping, Smart Push/Pull;
6. FLP evaluation against the 20 tasks frozen in R0;
7. narrow manually usable modeler;
8. AI Proposal workflow over the working modeler;
9. only then consider solver, drawing, BIM primitives, and plugin pilots.

If A0 cannot prove the minimum exact/reference contract, product renderer work does not start. If C cannot prove simple responsive interaction, AI tool-surface expansion does not start.

## 12. Preregistration rule

Before each gate, version and freeze:

- fixture corpus and difficulty classes;
- expected result for each fixture;
- metric, threshold, and measurement method;
- reference hardware/software envelope;
- exact success and failure consequence.

Changing any of these after observing results fails the original gate run and creates a new test version.

A NO-GO immediately applies its preregistered safe halt, but it does not by itself authorize an architectural redesign, fallback activation, supported-envelope narrowing, or threshold/consequence loosen. Before any such disposition, diagnostics must reduce the cause to a minimal reproducer and either a concrete source line/path or a named external boundary with reproducible boundary evidence. Until then the gate remains in `diagnostic_hold`: preserve the halt and complete process evidence, draw no broader architecture inference, and run the smallest discriminating diagnostic matrix. Any later loosen requires a separately accepted disposition and a new preregistration; it never rewrites the failed historical run.

## 13. R0 entry gate

Before A0, R0 must contain:

- pinned OCCT release, source commit, build fingerprint, compiler, and shared-library model;
- primary license sources and distribution checklist;
- reproducible toolchain;
- fixed, generative, mutation, adversarial, and provenance-safe external corpora;
- named interactive operation classes;
- B/C hardware profiles;
- the 20 canonical tasks;
- owners and deadlines for open decisions;
- immutable threshold and consequence files.

A0 does not start without a pinned backend, corpus, and threshold file.

## 14. Gate thresholds

### A0: exact kill-risk

Scope: narrow facade, simple profile, extrusion, planar cut, parameter mutation, automatic shape validation, topology evidence, and reference resolver. No final UI, native save format, or AI.

| Metric | Threshold |
|---|---|
| C++ exception crosses FFI | 0 across the corpus and at least 10,000 fuzz calls |
| Baseline valid extrude/cut fixtures | 100% expected valid results |
| Supported adversarial corpus | At least 90% expected valid results; every failure structurally diagnosed |
| Silent geometrically/topologically invalid shape | 0 |
| Frozen `Guaranteed` TNP mutation tests | 100% correct identity |
| Silent wrong identity in any stability class | 0 |
| History evidence for frozen `Guaranteed` subset | 100%; missing evidence fails the run |

Failure consequences are preregistered: failure of even simple extrusion top/bottom/side identity stops A1 and reopens the backend/reference model; sub-threshold adversarial behavior narrows the supported operation envelope and triggers a targeted exact-versus-mesh benchmark; any leaked exception fails the run.

### A1: canonical exact vertical slice

| Metric | Threshold |
|---|---|
| Canonical parameter/ID changes after 100 save/load cycles | 0 |
| Equivalent UI and RPC CommandBatch | 100% identical canonical digest and invariants |
| Atomic rollback after error | 100% of fixtures without partial mutation |
| Independent node recomputed after local change | 0 in frozen DAG corpus |
| Precision corpus | 100% authoritative values without drift; geometry within declared tolerance |
| Old-schema migration fixtures | 100% preserved declared meaning or explicit loss report |

Failure of save/load, rollback, or protocol equivalence blocks B.

### B: concurrency, isolation, and memory

| Metric | Threshold |
|---|---|
| Stale result inserted as current | 0 in at least 10,000 schedule permutations |
| Worker crash damages last committed revision | 0 in 100 crash-recovery runs |
| C++ exception crosses transport/FFI contract | 0 |
| Proposal with changed relevant read-set digest accepted | 0 |
| Long geometry job blocks navigation/query reader | 0 blocks over 100 ms; query p95 <= 16.7 ms in reference scenario |
| Worker cancellation | p95 <= 250 ms for killable test job; no committed data loss |
| Repeated edits and eviction | Post-warm-up plateau inside frozen memory budget |
| Worker transport overhead for named interactive class | p95 <= 15 ms and <= 20% of end-to-end time |

A stale insert, revision loss, or unbounded memory blocks C. Failure of worker overhead gets one bounded transport optimization and rerun; continued failure requires an evidence-based isolation ADR.

### C: interaction and performance

At least one frozen hardware profile is a mainstream integrated-GPU notebook.

| Metric | Threshold |
|---|---|
| Navigation and ephemeral preview after warm-up | p95 frame <= 16.7 ms; p99 <= 33.3 ms |
| Input to preview | p95 <= 50 ms |
| Exact parameter edit in named interactive class | p95 result <= 100 ms |
| Exact pick/snap | p95 <= 50 ms on reference scene |
| Long operation | Progress/cancel without blocking navigation over 100 ms |
| 10,000 occurrences of one definition | One shared authoritative geometry; per occurrence only transform/override/index data |
| Preview/commit meaning | 100% matching action digest or explicit mismatch/error |

A slow exact operation leaves the interactive class and gets asynchronous progress. If that operation is the baseline Push/Pull scenario, C fails. Slow picking is fixed in spatial queries, never hidden by renderer approximation.

## 15. Canonical task set

R0 freezes fixtures, natural-language requests, expected Intent, CommandBatch shape, and deterministic invariants for:

1. exact rectangular profile;
2. exact profile extrusion;
3. source extrusion height change;
4. rectangular opening/cut;
5. unambiguous Push/Pull source-parameter change;
6. ambiguous Push/Pull requiring a choice;
7. exact vector move;
8. snapped copy;
9. shared definition with multiple occurrences;
10. definition edit updating all occurrences;
11. make one occurrence unique;
12. group and edit context;
13. tag visibility without changing geometry ownership;
14. simple linear pattern;
15. parameter expression such as `width / 2`;
16. dependent-only DAG recompute;
17. save/reopen with dimensions and references intact;
18. multi-command batch undo/redo as one step;
19. exact or mesh export with a loss report;
20. risky AI edit through Proposal, preview, confirmation, and verification.

A task passes only when canonical document and geometry invariants pass, never merely because an AI produced a plausible text response.

## 16. Operating baseline

- Capacity: project owner plus AI agents; no guaranteed additional human FTE.
- Privacy: local-first; cloud AI only by explicit operation/workspace opt-in.
- Documentation and code language: English.
- UI: English fallback and localization resources from the first widget, per ADR 0001.
- Public API and file-format stability: not promised before their dedicated evidence gates.
- Commits may be created only at validated milestones. Pushing, releases, PRs, and shared-state changes require explicit operator approval.