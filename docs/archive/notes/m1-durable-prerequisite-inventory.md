# M1 Durable-Prerequisite Inventory

- Status: Evidence inventory
- Date: 2026-08-04
- Authority: `KETCHUP_ARCHITECTURE_SPECIFICATION_V4.md` §13 M1
- Scope: Blocking prerequisites only; no PF0, M3 exact integration, or non-blocking product polish
- Disposition: **Separate bounded prerequisite phase before M2**

## Result

| Prerequisite | Classification | Current evidence | Blocking gap |
|---|---|---|---|
| Correct profile-versus-default-solid semantics | **SMALL GAP** | Core and persistence distinguish `Profile` and `Extrusion` | Rectangle still commits a hidden 20 mm extrusion; profile-only interaction and first Push/Pull semantics are absent |
| Eliminate remaining legacy model/history authority | **LARGE GAP** | App scene and Undo/Redo use canonical `DocumentStore`; C1a projection authority passes | `CanonicalNode` remains a parallel runtime, command, persistence, adapter, Gate A1, and StateView authority beside `ProductModel` |
| Minimum hierarchy and group-to-component semantics for M2 migration | **LARGE GAP** | Flat/nested group parents, transform composition, cycle checks, grouping, sharing, and Make Unique exist | Definition-local child graph, actual group→component conversion, world-placement preservation, and complete identity mapping do not exist |
| Representation-independent contracts preserving Save/Open identity | **SMALL GAP, dependent** | Typed canonical entities round-trip independently from proxy interaction geometry | Closure fixtures are missing for corrected profile-only semantics, group conversion/mappings, and one-way legacy import |

No prerequisite set-level PASS can be claimed. Two large schema/authority gaps make this more than a small M2 entry block.

## 1. Profile versus default solid

### Existing substrate

- `FeatureKind` represents profile and extrusion separately: `crates/ketchup-core/src/document.rs:93-102`.
- Persistence serializes profile points and extrusion profile/height as distinct kinds: `crates/ketchup-core/src/persistence.rs:79-99`.
- Interaction projection is explicitly a proxy derived from canonical snapshots: `crates/ketchup-interaction/src/projection.rs:9-16`, `crates/ketchup-interaction/src/projection.rs:53-90`.

### Gap

- The Rectangle tool initializes a hidden height of 20 mm: `crates/ketchup-app/src/lib.rs:598-602`.
- Completing a rectangle parses that height and calls the solid-creation path: `crates/ketchup-app/src/lib.rs:2601-2607`.
- `create_box_at` creates both profile and extrusion in one batch: `crates/ketchup-app/src/lib.rs:1749-1806`.
- Projection emits box geometry only when one extrusion exists; a profile-only definition has no selectable/projected profile representation: `crates/ketchup-interaction/src/projection.rs:197-243`.
- Existing capstone tests preserve the current default-solid behavior rather than the V4 prerequisite.

### Required closure

1. Rectangle commits a definition, profile, and occurrence without an extrusion.
2. The planar profile is visible/selectable through a representation-independent projection contract without becoming a fake solid.
3. First Push/Pull creates the extrusion; later Push/Pull edits its height.
4. Focused tests cover profile creation, first extrusion, Undo/Redo, and Save/Open identity.

## 2. Sole canonical authority

### Existing substrate

- `KetchupApp` owns a `DocumentStore` and disposable UI state, rebuilds visible interaction data from `CanonicalInteractionProjection`, and delegates Undo/Redo to the store.
- The old app-owned `SceneBox`, `SceneHistoryEntry`, `undo_stack`, and `redo_stack` authorities are absent.
- C1a tests prove snapshot/revision/digest provenance and stale-projection non-authority.

### Gap

- `CanonicalNode` still exists as a complete second model: `crates/ketchup-core/src/document.rs:311-365`.
- `CanonicalCommand` exposes `CreateNode`, `SetDimension`, and `RenameNode` beside product commands: `crates/ketchup-core/src/document.rs:367-483`.
- `Snapshot` holds both `nodes` and `product`: `crates/ketchup-core/src/document.rs:522-527`.
- Save chooses a research/product schema and serializes legacy nodes separately before product state: `crates/ketchup-core/src/persistence.rs:33-118`.
- StateView, adapters, proposal/read-set behavior, and Gate A1 fixtures still treat the node graph as live authority.

### Required closure

1. Remove legacy node commands and runtime mutation from the active canonical model.
2. Convert old schemas only at a one-way, explicit migration boundary into the single canonical graph.
3. Remove parallel node authority from adapters, StateView, proposal/read-set behavior, and current-format persistence.
4. Replace Gate A1 fixtures that preserve two authorities with migration and single-authority fixtures.
5. Preserve historical input compatibility without reintroducing a second writable model.

## 3. Hierarchy and group-to-component conversion

### Existing substrate

- Occurrences and groups carry parent IDs: `crates/ketchup-core/src/document.rs:158-234`.
- Product state currently stores definitions, features, occurrences, and groups as top-level maps: `crates/ketchup-core/src/document.rs:236-255`.
- Group/Ungroup uses one canonical batch for identity-transform groups: `crates/ketchup-app/src/lib.rs:1520-1624`.
- Make Unique clones/remaps a definition and repoints an occurrence.

### Gap

- A definition owns only feature IDs; it cannot own a child occurrence/group graph: `crates/ketchup-core/src/document.rs:134-156`.
- The command vocabulary has no group→component conversion command or conversion result map: `crates/ketchup-core/src/document.rs:383-462`.
- `make_component` only applies `RenameDefinition`: `crates/ketchup-app/src/lib.rs:1626-1663`.
- There is no complete old-path→new-path mapping, no unresolved mapping state, and no proof that converted descendants preserve world placement, sharing, Undo/Redo, or Save/Open identity.

### Required closure

1. Add the minimum definition-local child occurrence/group representation needed by M2 migration.
2. Define one atomic group→component command/result with complete old-ID/path→new-ID/path mapping.
3. Preserve world placement and sharing; never silently retarget an unresolved descendant.
4. Cover nested conversion, converted-copy sharing, Undo/Redo, and Save/Open continuity.
5. Do not expand this slice into tags, general assembly polish, or future evaluator `SlotPath` work beyond the migration boundary.

## 4. Representation independence and persistence continuity

### Existing substrate

- Core stores typed canonical definitions, features, occurrences, groups, transforms, and IDs independently of `SharedBoxGeometry`.
- Projection declares `PROXY_EVALUATOR_V1`, `PROXY_BACKEND_V1`, and `ProxyIncomplete`: `crates/ketchup-interaction/src/projection.rs:9-16`.
- Product persistence records canonical IDs, kinds, hierarchy, transforms, visibility, and units: `crates/ketchup-core/src/persistence.rs:62-118`.
- Existing product and shell tests preserve current document digest, revision, IDs, hierarchy, transforms, parameters, and sharing across Save/Open.

### Gap and closure

This prerequisite is substantially present for current entities, but cannot close independently. Add focused identity and migration fixtures after prerequisites 1–3 for:

- profile-only definition/occurrence round-trip;
- first extrusion after Open;
- group→component descendant and mapping round-trip;
- one-way legacy import with no live legacy authority;
- exact canonical digest and Undo/Redo continuity across each transition.

## Phase decision

Create a separate, tightly bounded durable-prerequisite phase before M2. Its four exit blocks are:

1. profile-only Rectangle → first Push/Pull semantics;
2. retirement of writable `CanonicalNode` authority behind a one-way import boundary;
3. minimum definition-local hierarchy and atomic group→component conversion/mapping;
4. focused Undo/Redo and Save/Open identity closure over the three changes.

Only after all four blocks pass should M2 begin. Tags/tag visibility, remaining menus and shortcuts, cuboid capstone polish, proxy rendering refinement, and dimensions not required by these contracts remain non-blocking product work under V4 §13 M1 and must not enlarge this phase.

## Validation evidence

The read-only inventory ran the focused current suites once:

- `ketchup-core --test product_document`: 5 passed;
- `ketchup-interaction --test gate_c1a_projection_authority`: 3 passed;
- `ketchup-app --test file_workflow --test headless_shell --test capstone --test capstone_chain`: 22 passed.

These tests establish the current substrate. Green capstone tests are not prerequisite-1 evidence because they currently assert the default 20 mm solid behavior that must change.
