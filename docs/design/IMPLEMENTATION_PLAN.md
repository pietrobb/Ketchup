# Ketchup Modeler Implementation Plan

**Status:** implementation baseline  
**Date:** 2026-08-02  
**Scope:** manual desktop modeler foundations and discoverable interface  
**Authority:** `EXECUTION_CONTRACT.md` wins over this plan; `README.md` is the interaction and visual specification.

## 1. Why this plan exists

Ketchup already has a detailed interface reference in `docs/design/README.md` and `Ketchup Modeler.dc.html`. The current Rust application does not implement that design. It exposes model operations as unrelated controls in a 230 px sidebar and keeps a second scene/history model inside `ketchup-app`. This makes implemented operations difficult to discover and bypasses the intended canonical document architecture.

The correction is not to finish an isolated engine before building the interface, and not to keep attaching widgets to the prototype. Ketchup will be built as workflow-complete vertical slices. Each slice starts with the canonical document contract required by the workflow and ends with a discoverable viewport interaction in the designed shell.

## 2. Sources of truth

Use this precedence when requirements disagree:

1. `docs/design/EXECUTION_CONTRACT.md` — product and architecture invariants.
2. `docs/design/README.md` — interaction behavior, layout, states, copy, and visual tokens.
3. This document — implementation order, boundaries, and acceptance workflows.
4. `Ketchup Modeler.dc.html` — visual and behavioral reference, not production geometry code.
5. Existing Rust prototype behavior — reusable only when it agrees with the documents above.

The interface is English-first and every visible string remains localization-backed.

## 3. Current-state diagnosis

### Foundations that are worth keeping

- `ketchup-core` has immutable revisions, atomic `CommandBatch` commits, structural sharing, Undo/Redo, and a binary persistence proof.
- `ketchup-interaction` has typed selection identities, exact box picking, snap candidates, shared geometry, preview sessions, and Smart Push/Pull planning.
- `ketchup-exact` and `ketchup-scheduler` preserve the exact-backend and worker foundations.
- `ketchup-app` has a working Windows viewport, free orbit, finite edge rendering, face highlighting, multi-box picking, and focused regressions for reported rendering bugs.
- The design reference already specifies the main window, tools, Outliner, value box, hints, action digest, gestures, shortcuts, and component terminology.

### Prototype structures that must not become product architecture

- `SceneBox` and `Vec<SceneBox>` in `ketchup-app` are a temporary second document model.
- `SceneHistoryEntry` is a second Undo/Redo system alongside `DocumentStore`.
- A `CanonicalNode` currently stores only one dimension and dependencies; it cannot represent definitions, occurrences, transforms, groups, profiles, or tags.
- Move, Rotate, Delete, box creation, and sketch creation currently mutate application-owned scene state instead of emitting canonical commands.
- Persistence serializes the research `CanonicalNode` graph, not a user model, and is not wired to File/Open/Save.
- The current left control panel is inconsistent with the designed tool rail, direct manipulation, value box, and Outliner.

These prototype paths may remain temporarily behind compatibility adapters while a slice is migrated, but no new modeling behavior may be added to them.

## 4. Product interaction model

### Main window

Implement the shell already specified in `docs/design/README.md`:

- top bar: document identity, Undo/Redo, view presets, units;
- menu bar: complete discoverable command surface;
- 56 px tool rail: Select, Line, Rectangle, Push/Pull, Move, Measure, Orbit, Pan, Delete;
- central viewport: renderer plus action digest, hover/camera readout, per-tool hint, and value control box;
- right dock: Outliner first, then Tags; Assistant remains deferred until manual workflows work;
- status bar: active tool, selection count, snap state, grid step, and reference health.

A tool is not considered implemented because a method or sidebar button exists. It is implemented only when it is selectable from the menu/tool rail, has a localized hint, supports its viewport gesture, accepts an exact value where applicable, commits one command batch, and can be undone/redone.

### Tool-state contract

The application view model owns only ephemeral and presentation state:

- active tool;
- hover and selection view;
- edit-context stack;
- pending gesture and preview;
- value-box input and last applicable operation;
- camera and viewport display settings;
- open menu/panel state and localized digest.

It must not own authoritative geometry, object transforms, model hierarchy, or a separate document history.

### Selection contract

Selection is explicit and context-aware:

- object selection targets an occurrence or group;
- sub-element selection targets a stable face/edge/point reference inside the active edit context;
- Shift adds/removes from a selection set;
- empty click clears selection;
- Esc cancels a pending gesture first, then clears selection, then exits one edit context;
- Outliner and viewport use the same selection model;
- hidden or out-of-context entities cannot be selected accidentally.

## 5. Minimum canonical model

The next document schema must represent product concepts rather than generic research nodes.

### Identity and hierarchy

- `DocumentId`, `DefinitionId`, `OccurrenceId`, `GroupId`, `FeatureId`, `TagId`, and stable subshape references are distinct typed IDs.
- A `Definition` owns reusable local geometry/features.
- An `Occurrence` references one definition and owns a transform, visibility, tag assignment, name, and parent edit context.
- A `Group` is an ownership/edit-context boundary. It is unique by default and can be converted to a reusable component definition without changing world placement.
- A component is represented by a reusable `Definition` plus one or more `Occurrence`s.
- `Make Unique` clones a definition and repoints only the selected occurrence.
- Editing a shared definition updates every occurrence; editing a group affects only that unique group.
- Parent/child context and transforms compose deterministically. Geometry remains in definition-local coordinates.

### Initial feature vocabulary

The first usable modeler needs only:

- planar profile from rectangle or closed polyline;
- exact extrusion height;
- definition-local profile/height edits for unambiguous Push/Pull;
- occurrence transform;
- persistent dimension;
- tag and visibility metadata.

General arbitrary topology editing, booleans, constraints, and broad CAD feature coverage remain later slices.

### Canonical command vocabulary

UI actions must canonicalize into atomic commands equivalent to:

- create/delete/rename definition;
- create/delete occurrence;
- set occurrence transform and visibility;
- create/delete group and change group membership;
- clone definition and repoint occurrence (`Make Unique`);
- create profile and extrusion feature;
- set a feature parameter for unambiguous Push/Pull;
- create/delete dimension;
- create/update tag assignment.

Exact Rust enum names are decided in the core implementation, but the semantics above are required. One completed user gesture emits one `CommandBatch`; previews never mutate the document.

### Persistence boundary

Save/Load serializes the canonical model, schema version, units, IDs, hierarchy, parameters, transforms, and required provenance. Camera and optional UI layout may be saved separately but are not geometry authority. File writes must be atomic; failed loads leave the current document unchanged.

## 6. Implementation sequence

Each stage ends in a runnable Windows build and one user workflow. Do not start a later stage while its dependency is still represented only in `SceneBox`.

### Stage 0 — Freeze the product path

- Keep Gate C certification and AI work paused.
- Treat the existing design files as the UI specification.
- Stop adding behavior to the sidebar prototype.
- Preserve current camera, picking, and rendering regressions as compatibility tests.

**Exit:** this plan is accepted as the implementation baseline and the old functionality-first mission is retired.

### Stage 1 — Canonical scene and file-safe model

- Replace generic model-only `CanonicalNode` state with the minimum Definition/Occurrence/Group/Feature/Transform schema.
- Add canonical commands for creation, transforms, deletion, grouping, component instancing, and Make Unique.
- Make `DocumentStore` the only Undo/Redo authority.
- Extend versioned persistence and migration tests for the new schema.
- Add a read-only view model/query that the app can render without owning scene geometry.

**Acceptance workflow:** construct a definition with two occurrences, transform one, group another, make one unique, Undo/Redo every batch, serialize, reload, and obtain the same IDs, hierarchy, values, and digest.

### Stage 2 — Designed shell, Select, and Outliner

- Replace the 230 px controls panel with the top/menu/tool/status/right-dock shell.
- Implement the command registry used by menus, tool rail, shortcuts, and later AI/CLI adapters.
- Bind viewport and Outliner to one multi-selection/edit-context model.
- Show hover, selection, active tool, localized hint, and action digest consistently.
- Keep modeling commands unavailable rather than exposing incomplete form controls.

**Acceptance workflow:** open the app, identify all visible objects in the Outliner, select the same occurrence from viewport or Outliner, Shift-select multiple occurrences, clear selection, and navigate without changing the document.

### Stage 3 — Rectangle to exact solid and Smart Push/Pull

- Implement Rectangle as a viewport tool on the ground or supported face plane.
- Use the value box for exact width/depth without requiring a separate form.
- Commit one definition, profile/extrusion feature, and occurrence.
- Implement direct hover-drag Push/Pull with preview, exact value, action digest, and one canonical commit.
- Shared-definition impact must be explicit before commit.

**Acceptance workflow:** press R, draw or type an exact rectangle, Push/Pull it to an exact height, Undo/Redo both operations, and verify dimensions in the canonical document.

### Stage 4 — Direct Move and Copy

- Implement M as a direct viewport gesture with snap/inference and exact value entry.
- Ctrl-drag creates another occurrence of the same definition.
- Move changes occurrence transform only; it never edits shared geometry.
- Add only the minimum rotate behavior needed by a real workflow after Move/Copy is usable.

**Acceptance workflow:** select a solid, press M, move it by gesture and exact distance, Ctrl-copy it, verify Outliner shared-definition count, then Undo/Redo both actions.

### Stage 5 — Groups, Components, and edit context

- Implement Group/Ungroup for multi-selection.
- Implement Make Component and Make Unique.
- Enter/exit edit context from viewport and Outliner with clear visual isolation.
- Editing a component definition updates all its occurrences; Make Unique isolates one.
- Prevent selection leakage across contexts.

**Acceptance workflow:** create two solids, group them, copy the group, convert it to a component if needed, edit shared geometry and observe every occurrence update, make one occurrence unique, edit it independently, then Undo/Redo and verify hierarchy.

### Stage 6 — Open, Save, and recovery-safe continuity

- Wire New/Open/Save/Save As to versioned canonical persistence.
- Use atomic replace and explicit error reporting.
- Track dirty state and document name.
- Preserve the current model on failed open/save.

**Acceptance workflow:** create and organize a small model, save it, close/reopen or load it into a fresh app instance, and recover identical definitions, occurrences, groups, transforms, dimensions, and IDs.

### Stage 7 — Complete the narrow manual-modeler loop

- Add persistent Measure/dimensions, tags/visibility, Zoom Fit, and the remaining specified shortcuts.
- Remove all legacy sidebar and duplicate scene/history state.
- Run the complete capstone workflow below.
- Only after this stage may AI Proposal UI or broader geometry features resume.

**Capstone workflow:** create a cabinet-like solid from a rectangle, Push/Pull it, copy and place occurrences precisely, group related objects, create a reusable component, edit shared instances, make one unique, measure it, hide/show by tag, save, reopen, and Undo/Redo meaningful edits through the designed interface without using hidden developer controls.

## 7. Stage gates and anti-ad-hoc rules

A stage is complete only when all are true:

1. The workflow starts from the designed menu/tool rail or a documented shortcut.
2. The viewport gesture and exact value path both work where specified.
3. A localized hint explains the active tool before the first action.
4. Preview is ephemeral and the digest explains the intended canonical change.
5. The commit is one validated `CommandBatch` and one Undo step.
6. Outliner, viewport, persistence, and document queries agree on identity.
7. Focused tests cover the canonical transition and the interaction state.
8. A runnable build demonstrates the entire stage workflow.

Forbidden implementation shortcuts:

- adding a new authoritative `Vec<...>` scene inside `ketchup-app`;
- adding a second Undo/Redo stack for model state;
- implementing a modeling operation only as numeric sidebar fields;
- mutating document state continuously during drag;
- duplicating a definition when Copy should create an occurrence;
- treating a selected face as the object identity;
- hard-coding visible text in widgets;
- declaring a feature done when the user cannot discover it from the main window.

## 8. Code-boundary plan

- `ketchup-core`: canonical entities, commands, revisions, queries, persistence, and migration.
- `ketchup-interaction`: tool-neutral picking, snapping, inference, selection targets, gesture previews, and stable-reference resolution.
- `ketchup-exact`: exact profile/extrusion evaluation and topology evidence; no widget or user-session state.
- `ketchup-scheduler`: derived evaluation and cancellation; no model semantics.
- `ketchup-app`: localized shell, view model, tool state machines, renderer integration, file dialogs, and command dispatch; no duplicate scene authority.

Extract focused modules from `ketchup-app/src/lib.rs` as real boundaries appear:

- `app_shell` for chrome and panels;
- `view_model` for document queries and UI state;
- `tools` for interaction state machines;
- `viewport` for projection/render integration.

Do not create all modules upfront. Extract each when its first workflow would otherwise mix canonical, interaction, and widget responsibilities.

## 9. Verification policy

Prefer fast tests tied to the active stage:

- core tests for command atomicity, identity, hierarchy, sharing, Undo/Redo, persistence, and migration;
- interaction tests for selection context, snapping, gesture preview, and cancel/commit boundaries;
- app tests for command dispatch, tool transitions, localized hints/digests, and document/view consistency;
- one scoped debug build and manual acceptance workflow per completed stage.

Long Gate C benchmarks, broad certification reruns, AI surface expansion, and unrelated architecture reviews remain paused until the capstone manual workflow passes.

## 10. Immediate next action

Do not continue generalizing Push/Pull inside `SceneBox`. Implement Stage 1 first: the canonical Definition/Occurrence/Group/Transform model, command vocabulary, persistence migration, and read-only scene query. The first UI work after that is Stage 2, reproducing the existing designed shell rather than inventing another interface.
