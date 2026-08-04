# ADR 0005: NO-GO Diagnostic Hold Before Architectural Disposition

- Status: Accepted — project owner, 2026-08-04
- Date: 2026-08-04
- Decision owner: Architecture lead
- Accountable approver: Project owner
- Evidence: `artifacts/gate-a0/diagnostics/a0d-run-002/`, `crates/ketchup-exact/src/native.cc`, ADR 0004

## Context

Strengthened A0 v1 failed after reaching native geometry, but its sealed report preserved neither process stderr nor the Rust panic. The initial NO-GO was therefore over-interpreted as a cross-build migration failure and prompted consideration of PF0, a replacement engine estimated at 73–160 person-weeks. A0-D later showed that both same-build producers failed before any consumer ran. Source-level inspection then localized the defect to the adapter: `BRepPrimAPI_MakePrism` received the final profile, while `Generated()` was queried with the stale pre-wire east-edge identity rather than the east edge owned by that final profile.

The preregistered safe halt was correct. The architectural inference was not supported until the cause was localized.

## Decision

1. A gate NO-GO immediately applies its preregistered safe halt. The halt does not wait for diagnosis.
2. A NO-GO alone cannot authorize architectural redesign, fallback activation, supported-envelope narrowing, threshold or consequence loosening, or failure reclassification.
3. Such a disposition requires a minimal reproducer and cause localization to either a concrete source line/path or a named external boundary with reproducible boundary evidence.
4. Until that evidence exists, the state is `diagnostic_hold`: preserve the halt, preserve complete stdout/stderr/panic/exit/build identity and skipped-stage evidence, draw no broader architectural conclusion, and execute the smallest discriminating diagnostic matrix.
5. If line-level localization is impossible because the defect is external or nondeterministic, the disposition must identify the exact boundary and the observations that exclude each nearer owned layer. “Backend failure” without that boundary evidence is not sufficient.
6. A post-observation loosen always requires a separately accepted disposition and a new preregistration before it can affect a later gate. It never converts the failed historical run into a pass.

## A0 v2 application

- The repaired adapter is localized to the profile-edge identity used for `BRepPrimAPI_MakePrism::Generated()` in `crates/ketchup-exact/src/native.cc`.
- A0 v2 retains the original top, bottom, and `side(profile_edge=east)` `Guaranteed` envelope and all inherited thresholds.
- Every same-build and cross-build consumer includes one intentionally invalid reference negative control. It must return `Lost` under identical identity or compatibility quarantine under changed identity; `Resolved` or `Ambiguous` fails the combination.
- A four-of-four pass withdraws L-01/L-02 from ADR 0004 because their evidentiary basis was the defective A0 observation, leaves L-03/L-04 unadopted, and releases the M3 halt.
- If both same-build paths pass but either cross-build path fails without wrong or ambiguous identity, L-01/L-02 and the current version remain, changed identities stay quarantined, L-03/L-04 remain unadopted, and M3 is released on the unchanged passing identity. The project owner fixed both success dispositions before A0 v2 observation.

## Classification

This is a governance tightening. It changes neither a measured threshold nor a historical result and authorizes no loosen.

## Consequences

- Gate evidence must remain diagnostic enough to distinguish failure stages even when a process panics or a downstream stage is not run.
- Expensive fallback proposals cannot be justified by an unresolved gate symptom.
- Historical strengthened A0 v1 and A0-D artifacts remain immutable.
- PF0 remains inactive.
