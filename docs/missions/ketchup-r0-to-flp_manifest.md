# Mission manifest: Ketchup R0 to First Lovable Product

**Domain:** code
**Mode:** strict
**Created:** 2026-08-01
**Peer:** C:\Sources8\Ketchup
**Slug:** ketchup-r0-to-flp

## 1. Goal (Definition of Done)

Deliver a Windows-first, Apache-2.0 Ketchup First Lovable Product that passes the preregistered R0, A0, A1, B, and C gates defined by the frozen Architecture V3, with an auditable English technical record for every gate.

Measurable output: a reproducible repository revision containing the narrow usable modeler, all gate artifacts and test reports, the 20-task FLP evaluation, localization-ready English UI resources, and passing workspace tests; any failed kill gate must halt expansion and produce an explicit no-go report rather than silently weakening a threshold.

## 2. Starting point

- Current state: Architecture V3 is frozen and judged executable; the public GitHub repository exists and local `main` tracks `origin/main`, but only the upstream README and Apache-2.0 license are committed.
- Existing assets: Architecture V3, an R0 license/toolchain audit, verified OCCT 8.0.1 and Rust ecosystem candidates, Windows reference workstation, Apache-2.0 decision, and the canonical 20-task product definition.
- Missing assets: English-first project baseline commit, localization contract, pinned/reproducible build, clean OCCT fingerprint, provenance-safe corpora, immutable thresholds, exact façade, TNP evidence, canonical document and command pipeline, concurrency decision, viewport interaction, and FLP evidence.

## 3. Key assumptions (tiered)

```yaml
assumptions:
  hard:
    - id: A1
      text: OCCT can legally and technically serve as a replaceable shared-library exact backend behind Ketchup's own facade.
    - id: A2
      text: The preregistered A0 exact and topological naming minimum can be achieved without silently returning invalid geometry or wrong identity.
  soft:
    - id: A3
      text: One project owner plus AI agents, with no guaranteed additional human FTE, can execute the risk-ordered plan by keeping scope narrow.
    - id: A4
      text: egui and wgpu remain suitable through the proof stages; either may be replaced by ADR if measured evidence rejects it.
  testable:
    - id: A5
      text: The pinned Windows toolchain and OCCT build are reproducible.
      check: command_passes:cargo test --workspace --all-targets::timeout=600
    - id: A6
      text: The canonical command/document model is deterministic inside the declared determinism envelope.
      check: command_passes:cargo test --workspace --all-targets::timeout=600
    - id: A7
      text: The reference hardware can meet the preregistered interaction and query classes.
      check: file_exists:artifacts/gate-c/report.md
  preference:
    - id: A8
      text: Windows is the first supported platform; portability is preserved where cheap but other platform delivery is deferred.
    - id: A9
      text: Default UI copy is English while all user-facing strings are resolved through locale resources from the first widget.
```

## 4. Out-of-scope and invariants

**Out-of-scope:**
- Broad BIM, collaboration, mobile, cloud rendering, plugin marketplace, stable public API, and stable long-term file-format promises before the FLP gate.
- Rewriting Architecture V3; subsequent architectural changes require focused ADRs.
- Renderer or AI tool-surface expansion before their prerequisite kill gates pass.
- Formal legal review; the internal OCCT distribution compliance checklist remains mandatory.
- Publishing releases, pushing commits, creating PRs, or changing remote/shared infrastructure without explicit operator approval.

**Invariants:**
- All technical documentation, code, identifiers, schemas, tests, commit messages, and default UI copy are English.
- UI widgets never hard-code user-facing prose; they use stable localization keys and locale resources, with English as the complete fallback locale.
- Model/document/workspace data is local-first and is sent to cloud AI only after explicit opt-in for the operation or workspace.
- Thresholds, corpora, hardware profiles, and consequences are frozen before observing a gate result; changing one after observation fails that gate run.
- Exact and mesh bodies remain distinct; mesh is never an implicit exact fallback.
- UI, CLI, plugins, and AI use the same versioned canonical commands; stale proposals are never silently rebased.
- No destructive Git commands, no force pushes, no hook bypasses, no commits containing secrets, and no remote push without explicit operator approval.
- Architecture V3 changes only by a new ADR; historical evidence is not rewritten to make a gate pass.

**Allowed paths:**
- `README.md`
- `.gitignore`
- `rust-toolchain.toml`
- `Cargo.toml`
- `Cargo.lock`
- `deny.toml`
- `KETCHUP_ARCHITECTURE_PROPOSAL_V3.md`
- `R0_LICENSE_AND_TOOLCHAIN_BASELINE.md`
- `docs/**`
- `crates/**`
- `locales/**`
- `tests/**`
- `corpora/**`
- `thresholds/**`
- `scripts/**`
- `artifacts/**`
- `third_party/**`

## 5. Budget envelope and drift detector

```yaml
budget:
  max_ticks: 400
  max_tool_calls: 2400
  max_wall_clock: 120h
  max_repairs: 3

drift:
  max_ticks_without_progress: 4
```

## 6. Sequential decomposition

```yaml
steps:
  - id: G1
    text: Establish and commit the English-first governance, documentation, localization, capacity, privacy, and mission baseline.
    deferable: false
    substeps:
      - Publish an English execution contract that preserves the binding decisions, scope, invariants, gate order, and metrics of the frozen V3; keep uncommitted Slovak drafts as legacy source material only.
      - Translate the R0 baseline to English, record the language/localization decision as an ADR, and expose both from the README.
      - Record owner-plus-AI capacity and local-first explicit-opt-in privacy.
      - Commit only reviewed baseline files locally; do not push.
  - id: G2
    text: Build and freeze the reproducible Windows Rust and OCCT toolchain baseline.
    deferable: false
    substeps:
      - Pin Rust 1.97.0 and initialize the minimal workspace and validation scripts.
      - Produce a clean OCCT 8.0.1 shared-library build with minimal modules.
      - Record compiler, SDK, CMake cache, DLL list, source commit, and SHA-256 fingerprints.
      - Add dependency license policy and run the available build, test, and license checks.
  - id: G3
    text: Complete R0 preregistration and pass the R0 entry gate for A0.
    deferable: false
    substeps:
      - Add provenance-safe fixed, generative, mutation, and external STEP corpus manifests.
      - Freeze the 20 canonical tasks, operation envelope, validity oracle, hardware profiles, query classes, thresholds, and negative-result consequences.
      - Correct the Guaranteed-subset loophole by freezing the tested subset before A0.
      - Audit all R0 artifacts and issue an explicit R0 go/no-go report.
  - id: G4
    text: Implement the A0 exception-safe exact-backend facade and structure-aware geometry harness.
    deferable: false
    substeps:
      - Implement the narrow OCCT facade for preregistered primitives, extrude, and cut.
      - Convert all backend exceptions and failures into typed boundary results.
      - Implement automatic shape validity checks and topology/history evidence capture.
      - Add structure-aware generation and replayable failure minimization.
  - id: G5
    text: Execute A0 topological naming, mutation, adversarial, and migration tests and make the kill-gate decision.
    deferable: false
    substeps:
      - Implement SubshapeRef lineage, fingerprints, stability classes, and the frozen Guaranteed subset.
      - Run fixed, generative, mutation, and external corpus tests with zero silent invalid or wrong-identity outcomes.
      - Test backend build migration and quarantine unresolved references.
      - Publish an immutable A0 report; halt or narrow exactly as preregistered on failure.
  - id: G6
    text: Implement and pass A1 for the canonical document, revisions, canonical commands, precision, persistence, and adapter equivalence.
    deferable: false
    substeps:
      - Implement immutable revisions, structural sharing, CommandBatch undo/redo, and dependency-aware Proposal validation.
      - Implement versioned canonical commands used identically by UI and RPC/CLI adapters.
      - Implement deterministic save/load and precision contracts.
      - Prove equivalent canonical results for equivalent UI and protocol actions.
  - id: G7
    text: Implement and pass Gate B for scheduling, isolation, recovery, concurrency, cache bounds, and memory.
    deferable: false
    substeps:
      - Measure in-process and stateful worker variants with revision/cache/recovery contracts.
      - Select worker versus in-process by preregistered evidence and record the decision in an ADR.
      - Pass crash recovery, cancellation, concurrency, soak, RAM, and cache-eviction criteria.
  - id: G8
    text: Implement and pass Gate C for the localization-ready viewport, exact picking, snapping, preview, and Smart Push/Pull.
    deferable: false
    substeps:
      - Build the narrow English UI using locale keys/resources from the first widget.
      - Implement wgpu viewport, exact picking, snapping, preview cancellation, and action digests.
      - Measure interaction and query latency by preregistered classes and reference hardware.
      - Confirm that no renderer or widget path bypasses canonical commands.
  - id: G9
    text: Validate the First Lovable Product on all 20 canonical tasks and prepare an auditable narrow alpha.
    deferable: false
    substeps:
      - Run the complete canonical task suite and document successes, failures, and supported boundaries.
      - Verify privacy defaults, English fallback completeness, a second test locale, packaging notices, and OCCT replaceability.
      - Produce the FLP decision report, user-facing English documentation, and reproducible local alpha package.
      - Stop at the narrow FLP; defer broader domains and ecosystem work to a separately approved mission.
```

## 7. Dependencies and risks

- Principal risks: OCCT operation robustness, incomplete topology history, topological identity, build reproducibility, provenance-safe external CAD data, scheduler isolation cost, memory growth, exact picking correctness, and scope inflation under single-owner capacity.
- External blockers: acquiring a legally usable external STEP corpus and access to a representative integrated-GPU notebook before final Gate C closure.
- Local prerequisites: supported VS 2022 C++ workload, Windows SDK, disk capacity for OCCT builds, and network access only when fetching pinned public dependencies.
- Any failed hard assumption requires evidence and at least three genuinely attempted alternatives before mission halt.

## 8. Verification per step

- G1: `file_exists:docs/architecture/EXECUTION_CONTRACT.md && file_contains:R0_LICENSE_AND_TOOLCHAIN_BASELINE.md::local-first && file_contains:docs/adr/0001-project-language-and-localization.md::Localization && file_exists:docs/missions/ketchup-r0-to-flp_manifest.md && git_log_contains:Establish English-first project baseline`
- G2: `file_exists:rust-toolchain.toml && file_exists:artifacts/r0/occt-build-manifest.json && command_passes:rustc --version::timeout=120 && command_passes:cargo test --workspace --all-targets::timeout=600`
- G3: `file_exists:thresholds/r0.yaml && file_exists:corpora/manifest.yaml && file_exists:docs/gates/R0_REPORT.md && file_contains:docs/gates/R0_REPORT.md::GO`
- G4: `file_exists:crates/ketchup-exact/src/lib.rs && command_passes:cargo test -p ketchup-exact::timeout=900`
- G5: `file_exists:artifacts/gate-a0/report.md && file_contains:artifacts/gate-a0/report.md::GO && command_passes:cargo test -p ketchup-exact --test gate_a0::timeout=1800`
- G6: `file_exists:artifacts/gate-a1/report.md && file_contains:artifacts/gate-a1/report.md::GO && command_passes:cargo test --workspace --all-targets::timeout=1800`
- G7: `file_exists:artifacts/gate-b/report.md && file_contains:artifacts/gate-b/report.md::GO && file_exists:docs/adr/0002-exact-backend-isolation.md`
- G8: `file_exists:artifacts/gate-c/report.md && file_contains:artifacts/gate-c/report.md::GO && command_passes:cargo test --workspace --all-targets::timeout=1800`
- G9: `file_exists:artifacts/flp/report.md && file_contains:artifacts/flp/report.md::GO && file_exists:locales/en-US.ftl && command_passes:cargo test --workspace --all-targets::timeout=1800`

## 9. Proposed cron interval

`@every 30m` using `ask_llm_and_send`, anchored to this mission and instructed to work only on the current focus goal, verify evidence, save a tick summary, and stop on a preregistered kill condition.

## A. Code domain addendum

**Affected architecture:** a Rust workspace split along Architecture V3 boundaries: canonical model/commands, exact façade, scheduler, persistence, renderer, UI adapters, protocol adapters, and tests. OCCT stays behind a narrow C++ façade and replaceable shared-library boundary.

**Relevant files:** currently `KETCHUP_ARCHITECTURE_PROPOSAL_V3.md`, `R0_LICENSE_AND_TOOLCHAIN_BASELINE.md`, `.gitignore`, and `README.md`; later work is restricted to the allowed paths above. Local Supervisor runtime files under `.claude`, `skills`, executables, model configuration, and logs are never project inputs.

**API and contracts:** versioned canonical commands are the only mutation API; Proposal carries read/dependency digests; exact operations return typed results with validity and topology evidence; SubshapeRef is document-scoped and stability-classed; UI resolves localization keys before rendering; cloud AI boundaries require explicit consent.

**Test strategy:** Cargo test workspace plus narrow per-gate integration binaries and replayable corpus runners. Every gate writes machine-readable raw results and an English decision report. Benchmarks use frozen hardware classes and thresholds and never substitute for correctness tests.

**Refactor versus new code:** preserve the frozen architecture document, add only the smallest new workspace/modules needed by each current gate, and avoid speculative abstractions or post-FLP features.

## Critique

The mission is intentionally ambitious for one owner and may exceed the nominal wall-clock envelope; the control mechanism is not optimism but strict sequencing and kill gates. The weakest dependencies are a provenance-safe external STEP corpus and representative integrated-GPU hardware, neither of which should block early R0/A0 implementation but both can block final closure of their respective measurements.

There is also a risk that English translation accidentally changes a frozen architectural statement. The first baseline review must compare section/table/code-block counts and spot-check all quantitative gates; later changes go through ADRs rather than edits disguised as translation. Localization readiness must remain a concrete code invariant—resource lookup and fallback tests—not merely a documentation promise.