# Handoff: Ketchup — Modeler Main Window (R0/C interaction shell)

## Overview
The design is the main desktop window of **Ketchup**, a fast parametric modeler for architecture, interiors and furniture (see `EXECUTION_CONTRACT.md`, the frozen Architecture V3 execution contract that this UI must serve). It covers the *First Lovable Product* interaction surface: draw an exact profile, extrude it, edit dimensions, organise definitions/occurrences/tags, move/copy/pattern, measure, undo/redo, and drive a **Proposal-only** AI assistant.

Target platform: Windows desktop, ultrawide/large monitor. UI language: English (ADR 0001 — all user-facing strings must come from localization resources, never hard-coded in widgets).

## About the Design Files
`Ketchup Modeler.dc.html` is a **design reference created in HTML/Canvas 2D** — a working prototype that shows the intended look, layout, wording, and interaction model. It is **not production code and must not be ported literally**.

The real product is Rust + `wgpu` + OCCT behind a narrow C++ facade. The task is to **recreate this design in the target environment**: the panel chrome in whatever UI framework the measured PoC selects, and the viewport as the real `wgpu` renderer driven by the interaction/spatial-query service. Every prototype behaviour that mutates the model maps to a validated `CanonicalCommandBatch` — the prototype's direct mutation of a JS object is a stand-in, nothing more.

The prototype's 3D engine (hand-written perspective projection, painter's-algorithm face sort, screen-space snap search) exists only to make the interaction feel real. Do not port its geometry code.

## Fidelity
**High-fidelity** for the UI chrome: final colors, typography, spacing, wording, states. Recreate pixel-accurately.
**Medium-fidelity** for the viewport: the layout, overlay positions, HUD wording, snap/inference visuals and gesture model are final; the rendering itself (shading, anti-aliasing, edge weights, hidden-line quality) will be far better from the real renderer and should not imitate the canvas approximation.

## Screens / Views
One screen: **Main modeler window**. Root is a CSS-grid-equivalent layout, fixed to the viewport:

- Rows: `46px` top bar / `28px` menu bar / `1fr` body / `26px` status bar
- Columns: `56px` tool rail / `1fr` viewport / `320px` right dock
- Top bar and status bar span all columns.

### 1. Top bar (height 46px, bg `--panel`, bottom border 1px `--line`, padding `0 12px 0 10px`, flex, gap 14px)
- **Brand block** — 22×22 rounded-6px square filled `--accent` with white "K" (600, 13px); wordmark "Ketchup" (600, 14px, letter-spacing −0.01em). Right padding 12px, right border 1px `--line`, full height.
- **Document name** — "kitchen-run.ketchup", 12.5px, `--dim`.
- **Undo / Redo buttons** — height 28px, padding `0 10px`, radius 7px, border 1px `--line`, bg `--panel2`, 12px. `opacity: .4` when the respective stack is empty. Titles: "Undo (Ctrl+Z)", "Redo (Ctrl+Y)".
- Spacer.
- **View segmented control** — container radius 9px, padding 3px, bg `--panel2`, border 1px `--line`. Items: `Iso`, `Top`, `Front`, `Zoom fit` — height 24px, padding `0 10px`, radius 6px, 11.5px, `--dim`; the active camera preset gets `background: var(--panel); color: var(--text)`.
- **Unit chip** — "mm", IBM Plex Mono 11px, `--dim`.
- **Theme button** — same style as Undo; label is the *other* theme ("Light" while dark).

### 2. Menu bar (height 28px, bg `--panel`, bottom border 1px `--line`, padding `0 6px`, flex, gap 1px, `position: relative`, z-index 40)
Classic application menu — the complete command surface; the tool rail is only the fast path.
- **Titles**: height 22px, padding `0 10px`, radius 6px, 12px `--text`, transparent; hover `--panel2`; open title keeps `--panel2`. Click toggles; once any menu is open, hovering another title switches to it. A capture-phase pointerdown outside `[data-menu]` closes.
- **Dropdown**: absolute, `top: 25px; left: 0`, min-width 262px, bg `--panel2`, border 1px `--line`, radius 10px, shadow `0 18px 44px rgba(0,0,0,.45)`, padding 5px, z-index 60.
- **Item**: height 26px, padding `0 10px`, radius 6px, 12px, gap 10px — a 10px check column (`✓` in `--accent` for toggles), label, right-aligned shortcut in mono 10.5px `--dim`. Hover `rgba(240,78,35,.14)`.
- **Separator**: 1px `--line`, margin `5px 8px`, no pointer events.

Menu contents (✓ = checkable toggle, ● = wired in the prototype):

| Menu | Items |
|---|---|
| File | ● New model (Ctrl+N) · Open… (Ctrl+O) · Save (Ctrl+S) · Save As… (Ctrl+Shift+S) — Import… · Export exact (STEP)… · Export mesh (OBJ)… — ● Document info… · Exit (Alt+F4) |
| Edit | ● Undo (Ctrl+Z) · ● Redo (Ctrl+Y) — ● Delete (Del) · ● Make unique — ● Select all (Ctrl+A) · ● Deselect (Esc) — Preferences… |
| View | ● Isometric · ● Top · ● Front · ● Zoom fit (Shift+Z) — ✓● Ground grid · ✓● Axes · ✓● Dimensions · ✓● Snapping — ✓● Dark interface |
| Draw | ● Line (L) · ● Rectangle (R) |
| Tools | ● Select (Space) · ● Push / Pull (P) · ● Move (M) · ● Measure (T) — ● Orbit (O) · ● Pan (H) |
| Model | ● Make unique · ● Purge unused definitions — ● Statistics… · Reference audit… |
| Window | ✓● Outliner · ✓● Tags · ✓● Assistant |
| Help | Keyboard shortcuts (F1) · Execution contract… · About Ketchup |

Non-wired items still emit a localized digest describing what they would do (e.g. Export mesh → "Mesh export — explicit exact→mesh conversion with loss report"), so the intended contract semantics are documented in the UI itself. In production every menu item is a command-registry entry: same id, same validation path, same capability check as the toolbar, CLI, plugin and AI callers (contract §1) — the menu must never own behaviour of its own.

### 3. Tool rail (width 56px, bg `--panel`, right border 1px `--line`, column flex, gap 4px, padding `8px 0`)
Buttons 40×40, radius 9px, transparent border, icon 18px stroke 1.6 `currentColor`, color `--dim`.
Active state: `background: rgba(240,78,35,.14); color: var(--accent); border-color: rgba(240,78,35,.35)`.

Order (tooltip / shortcut):
1. Select — Space (cursor arrow glyph)
2. Line — L
3. Rectangle — R
4. Push / Pull — P
5. Move — M (Ctrl = copy)
6. Measure — T
7. — 24×1px `--line` divider, margin `6px 0` —
8. Orbit — O (or middle-drag)
9. Pan — H (or Shift+middle-drag)
10. Spacer, then **Delete selection** (trash icon) pinned to the bottom.

### 4. Viewport (center cell, bg `--bg`, `position: relative`, overflow hidden)
Full-bleed render surface plus four non-interactive overlays (all `pointer-events: none` except the value box):

- **Action digest chip** — top-left 12px. Height 28px, padding `0 12px`, radius 8px, `rgba(0,0,0,.42)` + `backdrop-filter: blur(10px)`, border 1px `--line`, 12px white text, preceded by a 6px `--accent` dot. Shows the localized digest of the current/last operation (see *Digest strings*).
- **Camera readout + hover readout** — top-right 12px, stacked, gap 6px, right-aligned. Padding `5px 10px`, radius 7px, same translucent bg, IBM Plex Mono 11px. Camera: `dist 5200 · az -129° · el 30°`. Hover: hovered entity + face kind, or snap label + rounded world coordinates.
- **Hint bar** — bottom-left 14px, max-width 46%, padding `8px 12px`, radius 9px, same translucent bg, 12px, line-height 1.45. Per-tool one-liner (full strings in *Copy*).
- **Value box (VCB)** — bottom-right 14px, `pointer-events: auto`. Row, radius 10px, border 1px `--line`, bg `rgba(0,0,0,.55)` + blur(10px), shadow `0 10px 30px rgba(0,0,0,.35)`. Left: label, padding `0 12px`, 11px, uppercase, letter-spacing .06em, `#9aa2ab` — text is `Dimensions` / `Width, Depth` (rect) / `Distance` (push, move) / `Length` (line). Right: input 190×38, transparent, left border 1px `--line`, IBM Plex Mono 15px white, placeholder "type exact value".

Viewport render content (all in mm, Z-up, right-handed):
- Ground grid on Z=0: minor lines every `gridStep`, major every 5th; minor `--gridMinor`, major `--gridMajor`. `gridStep` is adaptive: 500 mm above 12 m camera distance, 100 mm above 4 m, else 10 mm.
- Axis lines through origin: X `#c0453a`, Y `#3f8f57`, Z `#3f6fbf`, 1.4px.
- Solids: flat-shaded faces, light direction `normalize(0.38, −0.55, 0.74)`, `k = 0.62 + 0.38·max(0, n·L)`, base face color `rgb(206,211,218)` dark theme / `rgb(252,252,253)` light. Edges 1px `#14171a` (dark) / `#4a525c` (light).
- Hovered face: `rgba(76,141,255,.20)` overlay. Selected occurrence: faces tinted toward accent, edges `#F04E23` at 1.8px.
- Snap marker: 4.5px filled dot + 1.2px white ring, color by snap class — Endpoint `#39d98a`, Midpoint `#4C8DFF`, Origin `#F04E23`, Grid/plane `#8A93A0`; label chip 14px up-right, `rgba(0,0,0,.6)`, mono 11px white.
- Dimensions: 1.6px `#4C8DFF` line with a centered mono label chip.
- Rubber-band preview: rect fill `rgba(240,78,35,.16)`, dashed 5/4 `#F04E23` outline 1.6px; line segments solid 1.8px accent with a dashed leader to the cursor.
- Axis gizmo: bottom-left, origin at (46, H−46), 26px arms, labels X/Y/Z in `#d4553f` / `#5fa66b` / `#5f86d0`, mono 600 10px.

### 5. Right dock (width 320px, bg `--panel`, left border 1px `--line`, three stacked sections)
Section headers: height 34px, padding `0 12px`, label 11px uppercase letter-spacing .08em `--dim`, with a right-side meta item.

**a) Outliner** (`flex: 1.15`, scrolls). Header meta: `4 occ · 2 defs`.
Per definition, a card: margin-bottom 6px, border 1px `--line`, radius 9px, bg `--panel2`, overflow hidden.
- Card head (padding `8px 10px`, gap 8px, clickable = select all occurrences): 13px accent "definition" glyph; name (12.5px, 500, ellipsis); spec line `600 × 580 × 720` (mono 10.5px `--dim`) = profile bbox × height; count pill `3×` (10px, border 1px `--line`, radius 20px, padding `1px 7px`).
- Occurrence rows (padding `6px 10px 6px 28px`, top border 1px `--line`, 12px, gap 8px): 7px rounded tag swatch; name `Base Cabinet 600 #1`; mono 10.5px position `620,0`; `shown`/`hidden` toggle (10px `--dim`, stops propagation). Selected row: `background: rgba(240,78,35,.14)`.

**b) Tags** (auto height). Header action on the right: **"Make unique"**, 11px `--accent`, clickable. Rows: padding `6px 8px`, radius 7px, gap 9px, 12px — 9px color swatch, name, mono count, `shown`/`hidden`. Hidden tag row: `opacity: .45`.
Seed tags: Furniture `#F04E23`, Architecture `#4C8DFF`, Reference `#8A93A0`.

**c) Assistant** (`flex: 1`, scrolls). Header meta pill: "local · proposal only" (10px, border 1px `--line`, radius 20px, padding `1px 8px`).
- Message list: padding `0 12px 8px`, gap 8px, bubbles 12px/1.5, padding `8px 10px`, radius 9px. System: bg `--panel2`, border 1px `--line`, `--dim`. User: `rgba(240,78,35,.12)`, border `rgba(240,78,35,.28)`, `align-self: flex-end`.
- **Proposal card** (only when a proposal exists): border 1px `--accent`, radius 10px, bg `--panel2`. Title row 12px/600, padding `9px 11px`, bottom border. Body: assumptions as `· text` lines (11.5px `--dim`), then diff rows in mono 11px on `rgba(240,78,35,.10)`, radius 5px, padding `4px 7px`; then the digest line in mono 10px `--dim`. Footer: **Commit** (flex 1, height 30px, radius 7px, bg `--accent`, white 12px/500) and **Discard** (height 30px, padding `0 12px`, border 1px `--line`, transparent, `--dim`).
- Composer: top border 1px `--line`, padding `8px 10px 10px`, gap 6px. Input height 32px, radius 8px, border 1px `--line`, bg `--panel2`, 12px, placeholder `e.g. height 900 · array 4 x 700`. Send button 32×32, radius 8px, bg `--accent`, white arrow icon.

### 6. Status bar (height 26px, bg `--panel`, top border 1px `--line`, mono 11px `--dim`, gap 16px, padding `0 12px`)
Left → right: `Tool: push` (in `--text`), `1 selected`, spacer, `snap on|off` (clickable toggle), `grid 100 mm`, `A0 refs: guaranteed` (reference-health indicator — in production it must reflect the real resolver state: `guaranteed` / `best-effort` / `ambiguous` / `lost`).

## Interactions & Behavior

### Navigation (always available)
- Middle-drag or right-drag = orbit; Shift + middle-drag = pan; Orbit/Pan tools do the same with left-drag.
- Orbit sensitivity: `az -= dx·0.006`, `el += dy·0.005`, elevation clamped to ±1.45 rad.
- Wheel = zoom **to cursor**: `dist *= exp(±0.12)`, clamped 200–60 000 mm, then the camera target is corrected so the ground point under the cursor stays put.
- View presets: Iso `az −2.25, el 0.52`; Top `az −1.5708, el 1.44`; Front `az −1.5708, el 0.02`. Zoom fit frames the model bbox at `dist = diagonal × 1.7`, min 1200.
- Perspective FOV 0.82 rad. Near plane 30 mm; polygons are clipped in view space before projection.

### Select
Click a face → select its occurrence; Shift+click adds. Empty click clears. Digest names the definition **and how many occurrences share it** — this shared-definition awareness is the core teaching moment of the UI.

### Rectangle
Click corner 1 → live preview with live `W,D` mirrored into the value box → click corner 2 commits. Drawing plane = the hovered top face's plane if the first click is on one, otherwise Z=0. Typing `3000,2000` + Enter at any time creates the exact rectangle from corner 1, keeping the direction signs of the current drag. A rectangle creates a **new definition with h = 0** plus one occurrence positioned at the profile's min corner.

### Line
Click points; clicking within `1.2 × gridStep` of the first point after ≥3 points closes the profile and creates a definition (polygon is normalised to CCW; occurrence position = bbox min). Typing a length + Enter extends along the current cursor direction exactly. Esc cancels.

### Push / Pull (the flagship gesture)
Hover a face, press and drag. Screen-space delta is projected onto the screen direction of the face normal, then rounded to `gridStep` when snapping is on.
- **Top / bottom face** → edits `definition.h`. Digest: `Push/Pull — definition height 760 mm · 3 occurrences follow`.
- **Side face** → edits the two profile vertices of that edge along the outward normal. Digest: `Smart Push/Pull — unambiguous profile edge, editing the source parameter`.
- Live value streams into the value box; on release the digest becomes `Committed — height 760 mm · 1 undo step`, and typing a new number + Enter **re-applies exactly** on the last operation (SketchUp VCB semantics).
- Production rule from the contract: the safe default for an *ambiguous* provenance is **a new feature, never a silent distant parameter change** — the prototype only implements the unambiguous branch; the ambiguous branch must present a choice.

### Move / Copy
Drag a solid on its own base plane (ray ∩ plane at `pos.z`), snapped to `gridStep`. Shift constrains to the dominant axis. **Ctrl/Cmd at press = copy**, which pushes a new *occurrence of the same definition* (not a duplicated definition) and selects it. Value box accepts an exact distance along the last direction after release.

### Measure
Two snapped clicks place a persistent dimension with a mm label.

### Make unique
Clones the selected occurrence's definition and repoints only that occurrence. Digest: `Made unique — new definition, other occurrences untouched`.

### Undo / Redo
Whole-document JSON snapshot before every committed batch; stack capped at 60; redo cleared on new edits. **One committed user batch = one visible undo step** (contract §2). Ctrl+Z / Ctrl+Y.

### Value box (VCB)
Typing any of `0-9 . , -` anywhere (outside another input) appends to the value box and focuses it — the user never has to click it. Enter parses `,`, `;`, `x`, `*` as separators and applies to, in priority order: an active rectangle, an active line segment, the last push/pull, the last move. Esc clears the pending gesture.

### Assistant (Proposal → preview → commit)
Requires an explicit selected occurrence; without one it returns a "no target" proposal that changes nothing. Recognised intents:
- `height <mm>` / `make it <mm> tall` → change definition height (assumption states how many occurrences follow)
- `array|pattern|copy <n> x <mm>` (`y` in the string switches axis) → linear pattern of occurrences
- `move x|y|z <mm>` → exact vector move
- `delete` / `remove` → delete occurrence, definition retained

Every proposal renders title, assumptions, an authoritative diff (`def(cab).h  720 → 900`), and a digest line (`batch digest 1 command · risk low`). Nothing mutates until **Commit**; Discard leaves the document untouched and logs it in the message list. In production the diff, read/write set and digest are computed by the **core**, not the client, and the dependency digest is revalidated immediately before commit (contract §8).

### Keyboard
`Space` select · `L` line · `R` rect · `P` push/pull · `M` move · `T` measure · `O` orbit · `H` pan · `Ctrl+A` select all · `Esc` cancel/deselect · `Delete`/`Backspace` delete selection · `Ctrl+Z` / `Ctrl+Y`.

### Performance budget (contract §14, gate C — these are fail-able)
Navigation and ephemeral preview p95 ≤ 16.7 ms, p99 ≤ 33.3 ms; input→preview p95 ≤ 50 ms; exact parameter edit p95 ≤ 100 ms; exact pick/snap p95 ≤ 50 ms; long operations must show progress/cancel without blocking navigation >100 ms. 10 000 occurrences of one definition must share one authoritative geometry.

## Copy — every user-facing string
All of these are placeholders for localization resource keys (ADR 0001: no hard-coded prose in widgets). Placeholders like `{n}`, `{mm}`, `{name}` mark interpolated values.

### Chrome labels
| Where | String |
|---|---|
| Wordmark | `Ketchup` |
| Document name | `kitchen-run.ketchup` |
| Top bar buttons | `Undo`, `Redo`, `Light` / `Dark` (shows the theme you would switch *to*) |
| Tooltips | `Undo (Ctrl+Z)`, `Redo (Ctrl+Y)`, `Hide / show`, `Delete selection` |
| View segment | `Iso`, `Top`, `Front`, `Zoom fit` |
| Unit chip | `mm` |
| Dock headers | `OUTLINER`, `TAGS`, `ASSISTANT` (uppercase, letter-spaced) |
| Outliner meta | `{occ} occ · {defs} defs` |
| Definition spec line | `{w} × {d} × {h}` (profile bbox × height, mm, no unit suffix) |
| Occurrence name | `{definitionName} #{index}` |
| Occurrence position | `{x},{y}` |
| Visibility toggle | `shown` / `hidden` |
| Definition count pill | `{n}×` |
| Tags header action | `Make unique` |
| Seed tags | `Furniture`, `Architecture`, `Reference` |
| Assistant header pill | `local · proposal only` |
| Assistant placeholder | `e.g. height 900 · array 4 x 700` |
| Value box labels | `Dimensions` (default), `Width, Depth` (rectangle), `Distance` (push/pull, move), `Length` (line) |
| Value box placeholder | `type exact value` |
| Status bar | `Tool: {tool}` · `{n} selected` · `snap on` / `snap off` · `grid {step} mm` · `A0 refs: guaranteed` |
| Camera readout | `dist {d} · az {a}° · el {e}°` |
| Hover readout | `{definitionName} · {top\|bottom\|side} face` — or `{snapLabel}  {x}, {y}, {z}` — or `—` |
| Snap labels | `Endpoint`, `Midpoint`, `Origin`, `Grid {step}`, `On plane` |
| Tool tooltips | `Select — Space`, `Line — L`, `Rectangle — R`, `Push / Pull — P`, `Move — M (Ctrl = copy)`, `Measure — T`, `Orbit — O (or middle-drag)`, `Pan — H (or Shift+middle-drag)` |

### Hint bar (bottom-left, one per active tool)
- **Select** — "Click a face to select its occurrence. Middle-drag or right-drag to orbit, wheel to zoom."
- **Line** — "Click points on the ground or on a face. Click the first point to close the profile. Type a length + Enter for exact segments."
- **Rectangle** — "Click the first corner, then the opposite one. Or type “3000,2000” + Enter for an exact rectangle."
- **Push / Pull** — "Hover a face and drag. Top face edits the definition height — every occurrence follows. Side face edits the profile edge. Type an exact distance + Enter."
- **Move** — "Drag a solid on its base plane. Hold Ctrl to make a copy — copies stay occurrences of the same definition."
- **Measure** — "Click two points to place a dimension. Snaps to endpoints and midpoints."
- **Orbit** — "Drag to orbit. Wheel to zoom, Shift+drag to pan."
- **Pan** — "Drag to pan the view."

### Action digests (the chip top-left — the contract's localized action digest)
Idle / navigation:
- `Ready`
- `Cancelled` (Esc)
- `Selection cleared`

Selection:
- `Selected occurrence of “{definitionName}” · {n} occurrence(s) share this definition`
- `Selected {definitionName} #{i}` (from the outliner)
- `Selected all {n} occurrences of “{definitionName}”` (definition card click)
- `Selected {n} occurrences` (Ctrl+A)

Drawing:
- `Exact rectangle {w} × {d} mm — Push/Pull it next`
- `Closed profile — {n} edges`
- `Segment {mm}` (exact typed line length)
- `Dimension {mm}` (measure committed)

Push/Pull — live during the drag:
- `Push/Pull — definition height {mm}` · appended when shared: ` · {n} occurrences follow`
- `Smart Push/Pull — unambiguous profile edge, editing the source parameter`

Push/Pull and move — on release:
- `Committed — height {mm} · 1 undo step`
- `Committed — profile edge · 1 undo step`
- `Move Δ{mm}` / `Copy Δ{mm}` (live), then `Committed — move · 1 undo step` / `Committed — copy · 1 undo step`
- `Copy — new occurrence of the same definition` (at the moment Ctrl-copy starts)
- `Exact value applied — {mm}` / `Exact move — {mm}` (typed into the value box)
- `Nothing to apply the value to` (value typed with no live gesture)

Document / model:
- `Undo — one command batch reverted`, `Redo`
- `Deleted — 1 undo step`
- `Made unique — new definition, other occurrences untouched`
- `Select an occurrence first` (Make unique with empty selection)
- `Purged {n} unused definition(s)` / `No unused definitions`
- `{occ} occurrences · {defs} definitions · {dims} dimensions · ~{tris} preview triangles` (Statistics / Document info)
- `Proposal committed — verified, 1 undo step`

Menu items not yet wired still state their intended contract semantics:
- New model → `New model — units mm`
- Open… → `Open .ketchup — atomic, checksummed read`
- Save → `Saved — atomic write, crash-recoverable revision`
- Import… → `Importers run sandboxed with quotas`
- Export exact (STEP)… → `Exact export queued — provenance + loss report attached`
- Export mesh (OBJ)… → `Mesh export — explicit exact→mesh conversion with loss report`
- Preferences… → `Preferences — units, tolerance profile, snapping`
- Reference audit… → `Reference audit — all frozen Guaranteed refs resolved`
- Keyboard shortcuts → `Space select · L line · R rect · P push/pull · M move · T measure · O orbit · H pan`

### Assistant messages
- Opening system message: "Local assistant. I propose a command batch; nothing changes until you commit."
- Unparsed request: "I can plan: height <mm>, move x/y <mm>, array <n> x <mm>, delete. Nothing was changed."
- After commit: "Committed: {proposalTitle}. Verification passed; one undo step created."
- After discard: "Proposal discarded. Document unchanged."

### Proposal cards
| Intent | Title | Assumptions | Diff | Digest line |
|---|---|---|---|---|
| no selection | `No target selected` | "A proposal needs an explicit target occurrence." | `select 1 occurrence, then re-ask` | `digest —` |
| height | `Change definition height` | "Target: definition “{name}”" · "{n} occurrence(s) will follow — this is not a per-instance override" | `def({id}).h  {old} → {new}` | `batch digest 1 command · risk low` |
| array | `Linear pattern` | "Direction: {red (X)\|green (Y)} axis" · "Copies are occurrences of the same definition" | `+{n} occurrence(s) of {defId}` · `spacing {mm} mm` | `batch digest {n} commands · risk low` |
| move | `Exact move` | "Vector move on the {X\|Y\|Z} axis" · "Only this occurrence transform changes" | `occ({id}).pos.{axis}  {old} → {new}` | `batch digest 1 command · risk low` |
| delete | `Delete occurrence` | "Definition stays in the document" · "{n} occurrence(s) remain" | `- occ({id})` | `batch digest 1 command · risk medium` |

Buttons: `Commit` (accent, primary) and `Discard` (ghost). The card never appears without at least one assumption and one diff row — an empty proposal is a bug.

### Tone rules for new copy
1. State what the system did or will do, in the model's own vocabulary: *definition*, *occurrence*, *profile*, *command batch*, *undo step*.
2. Always say how far a change reaches when it reaches beyond the clicked thing ("{n} occurrences follow").
3. Never promise exactness the preview cannot guarantee — the preview digest describes the *intended* commit, and a post-commit mismatch is reported explicitly.
4. Numbers carry their unit on first mention (`760 mm`), never inside dense mono readouts where the unit is in the status bar.
5. No exclamation marks, no encouragement, no personality in the assistant — it reports assumptions and diffs.

## State Management
Prototype state (the shape the real client should mirror, minus the transport):

**Document (canonical, undoable)**
```
defs: { [id]: { id, name, poly: [[x,y], …] (CCW, local), h } }
occ:  [ { id, defId, pos: [x,y,z], tag, visible } ]
tags: [ { id, name, color, visible } ]
dims: [ { a: vec3, b: vec3, label } ]
seq:  number   // id counter
```
Occurrence world geometry = `def.poly` translated by `pos.xy`, extruded from `pos.z` to `pos.z + def.h`. In production these are canonical entities with stable IDs, units, expressions and `SubshapeRef`s; `poly`/`h` become parameters and feature specs, and geometry comes from the exact backend.

**Ephemeral interaction state (never a transaction)**
`camera {target, dist, az, el, fov}`, `pending` (rect/line/measure in progress), `drag` (orbit/pan/push/move with its start snapshot), `hoverFace`, `snap`, `lastOp` (for post-release exact entry).

**UI state**
`theme`, `tool`, `sel: id[]`, `menu` (open menu id), `showGrid`/`showAxes`/`showDims`, `showOutliner`/`showTags`/`showAI`, `snapOn`, `digest`, `hoverText`, `hint`, `camText`, `vcb`, `vcbLabel`, `aiInput`, `aiMsgs`, `proposal`.

**History**: `undoStack`/`redoStack` of document snapshots.

Rendering is on-demand (`requestAnimationFrame` coalesced), never a continuous loop; canvas is DPR-scaled (capped at 2) and resized via `ResizeObserver`. Keep that discipline in the real renderer: redraw on invalidation, not on a timer.

## Design Tokens

### Colors — dark (default)
```
--bg      #0f1113   viewport + app background
--panel   #17191c   bars, rails, dock
--panel2  #1e2126   cards, inputs, wells
--line    #2b3036   all 1px borders and dividers
--text    #e9ebee
--dim     #98a0a9
--accent  #F04E23   brand / selection / commit
--sel     #4C8DFF   hover + measurement blue
```
### Colors — light (override on the root)
```
--bg #e9eaec · --panel #f6f7f8 · --panel2 #ffffff · --line #dde1e5 · --text #191b1d · --dim #6a727b
(accent and sel unchanged)
```
### Viewport-only colors
```
grid minor  #1b1f24 (dark) / #dfe2e6 (light)
grid major  #2a3138 (dark) / #c9ced5 (light)
face base   rgb(206,211,218) / rgb(252,252,253)
edge        #14171a / #4a525c
axis X/Y/Z  #c0453a / #3f8f57 / #3f6fbf
gizmo X/Y/Z #d4553f / #5fa66b / #5f86d0
snap endpoint #39d98a · midpoint #4C8DFF · origin #F04E23 · grid #8A93A0
overlay glass rgba(0,0,0,.42) (chips) / rgba(0,0,0,.55) (value box), blur 10px
accent washes rgba(240,78,35,.10 / .12 / .14 / .16) and border rgba(240,78,35,.28 / .35)
```
### Typography
- UI: **IBM Plex Sans** 400/500/600. Sizes: 10 / 10.5 / 11 / 11.5 / 12 / 12.5 / 13 / 14 px. Section labels 11px uppercase, letter-spacing .08em. Wordmark 14px/600, letter-spacing −0.01em.
- All numerics, coordinates, diffs, digests, status bar: **IBM Plex Mono** 400/500/600 at 10 / 10.5 / 11 / 15 px (15px only in the value box).
- Canvas labels: mono 500 11px; gizmo mono 600 10px.

### Spacing / radius / other
- Spacing steps used: 2, 4, 6, 8, 9, 10, 11, 12, 14, 16 px. Grid gutters: rail 56, dock 320, bars 46/26.
- Radius: 5 (diff row) · 6 (segment item) · 7 (buttons, tag row) · 8 (chips, small inputs) · 9 (cards, tool buttons, hint) · 10 (value box, proposal card) · 20 (pills) · 50% (dots).
- Borders: always 1px `--line`. Selected/hover strokes 1.6–1.8px.
- Shadow: only one — `0 10px 30px rgba(0,0,0,.35)` on the value box.
- Icons: 18px, stroke 1.6, round joins, `currentColor`; assistant send arrow 14px stroke 2.

## Assets
None external. All icons are inline stroke SVG authored in the prototype (select, line, rectangle, push/pull, move, measure, orbit, pan, trash, definition cube, send arrow) — reimplement with the codebase's icon system, keeping the 18px/1.6 stroke weight. Fonts are IBM Plex Sans + IBM Plex Mono (Google Fonts, SIL OFL) — self-host them in the desktop build.

## Files
- **`Ketchup Modeler (standalone, open this).html`** — the prototype as one self-contained offline file. **Open this one in a browser to try the design.** No server, no dependencies.
- `Ketchup Modeler.dc.html` + `support.js` — the readable source of the same prototype (markup and logic separated by the runtime in `support.js`). Keep the two files side by side; opening the `.dc.html` without `support.js` in the same folder shows raw `{{ placeholders }}` instead of values.
- `Ketchup Modeler.dc.html` — the full prototype (template markup + logic class in one file; open directly in a browser). Layout, copy and tokens live in the markup; camera math, picking, snapping, tools, history and the proposal planner live in the logic class.
- `EXECUTION_CONTRACT.md` — the frozen Architecture V3 contract this UI is bound to. Where the prototype and the contract disagree, **the contract wins**: preview is not a promise of exact geometry, only validated command batches mutate the document, and no user-facing string may be hard-coded in a widget.

## Notes for the implementer
1. The prototype mutates the document directly on drag; the real client must keep drags ephemeral and emit exactly one `CanonicalCommandBatch` on release.
2. Every string in this prototype is a placeholder for a localized resource key (ADR 0001).
3. The digest chip is not decoration — it is the contract's "localized action digest" and must state what will be committed *before* commit, and what was committed after.
4. Picking, snapping and inference belong to the interaction/spatial-query service, not the renderer; the renderer only supplies coarse candidates and highlighting.
