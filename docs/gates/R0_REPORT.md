# R0 Entry Gate Report

- Gate: R0 entry gate for A0
- Freeze: `r0-v1`
- Freeze time: 2026-08-01T13:05:07Z
- Measurement state at freeze: no A0 observations
- Preregistration lock SHA-256: `213b56e5bb50cd6c82afdbdd4067a002a92ac2f56714f7dd3272f3f8f1e1e6be`
- Architecture authority: `docs/architecture/EXECUTION_CONTRACT.md`

## Decision

**Decision: GO**

A0 implementation and subsequent measurement may begin only against the locked `r0-v1` inputs. This decision does not claim that A0 passes, does not authorize work beyond the current mission sequence, and does not authorize a commit, push, release, or binary distribution.

## Entry audit

| R0 requirement | Evidence | Result |
|---|---|---|
| Pinned exact backend | OCCT 8.0.1 tag `V8.0.1`, commit `b8f597c677811d1f9f4d8a97f5ae2825c0353a42` | PASS |
| Build and shared-library model | Clean Release/shared C++17 build; normalized CMake contract; 48 DLL fingerprints; 7,400-file install-tree fingerprint | PASS |
| Toolchain | Rust 1.97.0, Cargo lock, cargo-deny policy, VS 2022 17.5/MSVC 14.35, SDK 10.0.22000.0, CMake 4.2.1 | PASS |
| License inputs | Primary OCCT sources and mandatory internal dynamic-link distribution checklist in `R0_LICENSE_AND_TOOLCHAIN_BASELINE.md` | PASS |
| Fixed corpus | Four baseline extrude/cut fixture specifications in `corpora/manifest.yaml` | PASS |
| Generative corpus | Structure-aware SplitMix64 contract, 20 frozen seeds, 50 cases per seed, 1,000 cases total | PASS |
| Mutation corpus | Eight frozen parameter mutations over the exact Guaranteed-subset base fixture | PASS |
| Adversarial corpus | Ten expected-valid and six expected-rejected cases with typed outcomes | PASS |
| External STEP corpus | Three original Apache-2.0 fixtures generated from declared geometry, normalized, checked in, and SHA-256 pinned | PASS |
| Canonical task set | Exactly 20 tasks with fixture, English request, expected Intent, canonical command-batch shape, and deterministic invariants | PASS |
| Operation and validity envelope | Narrow rectangular profile, +Z extrusion, rectangular planar cut, explicit rejection boundary, and multi-part validity oracle | PASS |
| Guaranteed subset | Exactly top, bottom, and specified-profile-edge side faces of one simple rectangular extrusion across M01-M08 | PASS |
| Hardware profiles | Exact high-performance workstation `HP-DEV-01` and frozen mainstream integrated-GPU notebook class `HP-IGPU-01` | PASS |
| Query classes | Six named B/C scenes and operations with samples, warm-up rules, and latency/correctness thresholds | PASS |
| Gate thresholds and consequences | R0, A0, A1, B, C, and FLP thresholds plus non-negotiable failure consequences in `thresholds/r0.yaml` | PASS |
| Owners and deadlines | Five open decisions have an owner, deadline condition, and blocking gate | PASS |
| Immutability | Sixteen preregistration/supporting files are bound by `artifacts/r0/preregistration-lock.json` | PASS |

## Guaranteed-subset loophole closure

`Guaranteed` in `r0-v1` is not a label that implementation may assign after seeing results. It means only:

1. `extrusion.top`;
2. `extrusion.bottom`;
3. `extrusion.side(profile_edge=east)`;

for one supported four-edge rectangular profile extruded linearly along +Z, with no cut or downstream boolean, across all eight frozen mutation cases. All 24 outcomes must resolve to the correct identity and carry the required producer, role, lineage, backend-history-where-emitted, topology, and adjacency evidence. Geometry fingerprints are corroboration only. Any silent wrong match, ambiguity, loss, or missing required evidence fails Guaranteed coverage.

## Corpus provenance boundary

The three STEP files are original Ketchup project fixtures licensed under Apache-2.0. Their source geometry, generator, OCCT build, normalized timestamp, expected volume and bounds, and byte hashes are recorded. They exercise the external file-format/import boundary but do not claim independent CAD-kernel diversity. Random internet CAD, undocumented OCCT test data, and files without an explicit redistribution grant are excluded.

Adding independently authored STEP files later is allowed only under a new corpus version. It cannot retroactively change or rescue an observed `r0-v1` result.

## Hardware qualification boundary

The exact `HP-IGPU-01` notebook is not yet selected. Its admissible hardware class and selection rule are frozen now, and the exact first qualifying machine must be fingerprinted before any Gate C observation. That is a Gate C prerequisite, not an A0 blocker. Selecting or substituting a machine after observing Gate C results fails that run.

## Failure consequences

- A post-observation change to a locked input fails the original run and requires a new freeze ID.
- Any C++ exception crossing the A0 boundary fails A0.
- Any silent invalid shape or silent wrong identity fails A0 and blocks A1.
- Failure of top, bottom, or specified-edge side identity reopens the backend/reference model and blocks A1.
- Sub-threshold adversarial behavior produces a failed `r0-v1` run, a newly narrowed preregistration, and a targeted exact-versus-mesh benchmark; mesh never becomes an implicit fallback.
- Later gate failures block the dependent expansion exactly as stated in `thresholds/r0.yaml`.

## Verification

Run from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/windows/validate-r0-preregistration.ps1
```

The validator checks every locked hash, all corpus categories and STEP hashes, all 20 task contracts, the exact Guaranteed subset, critical immutable thresholds, both hardware profiles, all named query classes, the OCCT evidence, and this report's decision/lock identity.

## Remaining non-blocking work

- Implement the A0 facade and corpus runner without changing `r0-v1`.
- Acquire and fingerprint the first notebook matching `HP-IGPU-01` before Gate C measurement.
- Perform the public-binary OCCT compliance packet only before public binary distribution.

No A0 result is asserted in this report.
