# ADR 0009: Rule programs and a generic core

- Status: Accepted
- Date: 2026-09-24 (decision); 2026-10-16 (recorded in English)
- Decision owner: Project owner
- Sources: the Slovak analysis and cleanup plan of 2026-09-24, kept as history in
  `docs/archive/notes/analyza-fundamentov-2026-09-24.md` and
  `docs/archive/notes/plan-upratovania-2026-09-24.md`. This ADR carries their decisions; the
  archived documents are not binding (ADR 0001: normative decisions are English ADRs).

## Context

On 2026-09-24 an analysis of the code base found that the model was built from absolute,
per-piece geometry; design rules were a calculator the AI could not see. Errors were narrowed
before they reached the agent, the whole model was validated as a gate on every edit, eight
interfaces did the same thing, and a closed command vocabulary pushed named shapes (bottle,
teapot, balloon letters, roofs, stairs, capsules) into the core as special cases. The process
around the code (gates, preregistrations, digests) cost more than it protected; ADR 0008
retired it.

## Decision

1. **The core holds only generic operations.** Rust crates contain sketch, profile, extrude,
   revolve, sweep, loft, boolean, shell, fillet, chamfer, offset, transform, pattern, a generic
   joint (parts, volume, fasteners) and collision, contact and support over any solids. The test
   for anything new: *would someone modeling something completely different use it?* Format
   exporters (STEP, BTLx, HOMAG, DXF) and validators against a design standard are allowed; a
   format or a standard is not a product. Named-product types, commands, features or face roles,
   evaluators for one shape, hand-written meshes for shape combinations, features with a fixed
   point count and modules for one test are forbidden. `scripts/check_no_named_products.py`
   enforces it.
2. **A rule program describes the model.** The program (Starlark, crate `ketchup-program`) is
   deterministic, has no I/O and a step limit. Board, dowel, groove, shelf, cabinet and hardware
   catalogs are functions of its library (`crates/ketchup-program/library/`), the only place for
   domain names. A program evaluates to a command batch through the one mutation path, as one
   Undo step. A derived part's identity is its path in the program, and re-evaluation pairs parts
   by that path. The program is stored in the document. A manual edit of a derived part becomes a
   parameter change or an explicit override keyed by that path; it is never lost silently.
3. **Validators report; only production export blocks.** Evaluation succeeds whenever the program
   runs and returns issues with severity, kind, parts, place, message and hint. An edit that
   creates a collision is written and reported. A joint that carries a volume accepts overlap
   inside that volume, reports overlap outside it, and reports a declared joint without overlap.
   Core consistency invariants (unknown IDs, cycles) stay in the mutation path, local to the
   command.
4. **One interface for AI.** Every AI client uses the same protocol (today the MCP server
   `ketchup-app --mcp`). No stamp is mandatory; an optional one
   guards concurrent edits. Every rejection carries a code, the path of the offending field or
   program line, a plain-language message and a concrete fix hint, and no layer narrows it.
5. **Geometry uses the cheapest exact means.** Prismatic parts (a box minus convex cuts) are
   computed analytically for volume, bounds, contact and collision. OCCT is called for fillets,
   lofts, sweeps and STEP, through one IR whose version is the application build.

## Direction

These follow from the decision and are the target where the code does not yet match:

- the file holds the program, parameters, overrides and hand-modeled parts; derived and exact
  results are a cache keyed by program, evaluator and backend build that can always be dropped;
- duplicate concepts merge (one extrude, one cut with an extent, one shell, one sketch profile,
  one generic joint beside assembly kinematics);
- domain validators (shelf deflection, tipping) move from the core into the library with explicit
  parameters;
- premature features (FEA, motion studies, gearing, PDM) stay frozen until used.

## Consequences

- A new capability is a library function of tens of lines, not match arms across the core.
- The AI changes rules, not coordinates: "move the dowels" and "make the cabinet wider" are one
  number each.
- `AGENTS.md` states these rules for coding agents; a step is done when the workspace tests,
  Clippy and the named-product ratchet pass.
