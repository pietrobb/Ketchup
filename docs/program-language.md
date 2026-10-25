# Rule programs

A rule program describes a model as parameters and rules instead of a list of
finished pieces. It is written in [Starlark](https://github.com/bazelbuild/starlark/blob/master/spec.md),
a small deterministic subset of Python. Change one parameter and every part,
hole and groove that depends on it follows.

```python
W = param("width", 600, min = 300, max = 1200)
T = param("thickness", 18)

left   = board("carcass/left",   (T, 350, 720))
right  = board("carcass/right",  (T, 350, 720), at = (W - T, 0, 0))
bottom = board("carcass/bottom", (W - 2 * T, 350, T), at = (T, 0, 0))

for side in (left, right):
    dowels(bottom, side, dowel = "8x30", margin = 50)   # holes in both boards
```

The full example is [`examples/programs/cabinet.star`](../examples/programs/cabinet.star).

## Running a program

| Way | Command |
|---|---|
| Command line, no OCCT or GUI needed | `ketchup-program check cabinet.star --set width=700` |
| Quick check of a large program or an edited prelude, no build | `ketchup-program check house.star --summary --prelude crates/ketchup-program/library/prelude.star` (parts per layer, statics, issues grouped by kind); `--diff saved.json` lists what changed against a summary saved earlier |
| Headless protocol | `program_check` (report only), `program_apply` (replace the document, one undo step) |
| Python SDK | `session.check_program(source, params={...})`, `session.program_document(source, ...)` |
| AI agent (MCP, `ketchup-app --mcp`) | `program` with `action=apply` in the open window; `action=docs` lists the library topics, `name=<topic>` returns one |

The CLI/headless run returns the report below. MCP apply/patch returns a concise
source diff, summary counts and bounded issue/relation previews. Retrieve complete
model-derived details with `program action=report expected=<stamp> section=...`:
`cut_list`, `hardware`, `machining`, `relations`, or `issues`. Use `limit` (1–100,
default 50) and follow `next_offset` until null with the same stamp and section.
Cut-list pages have one row per part with `group`, `group_count`, material and
sorted dimensions; regroup by `group`. Machining pages have one operation per
row; regroup by `part`. This avoids truncating large nested lists. `basis=apply_report`
reuses the last apply report; `program_evaluation` after opening/undo is a fresh
program evaluation, not a native geometry check. Unverified is neither wrong nor passed.

For a local edit, read `program action=read` (source only, no full part map), or
`mode=selection` for selected part lines and pick context. Selection lines include
two neighbours, not a dependency closure. Reuse already loaded context; request
`mode=source` only for missing dependencies (`mode=full` explicitly requests the old map).
Then send one `action=patch expected=<stamp> edits=[{"old":"...","new":"..."}]`.
All nonempty old texts must match once in the original source and must not overlap.
Stale, ambiguous or invalid patches leave the document unchanged. Patches retain
overrides, filename and program ownership, use normal validation, and publish one Undo step.

In a large program read only what you need: `mode=outline` lists the top-level `def`s,
`param(...)` lines and comment sections with their line numbers (a section reaches to `last`);
`lines=[first, last]`, `search=<text>` (ignoring case) or `part=<name>` return just those
numbered lines (at most 400 per answer; `truncated` says to read on). To change parameter
values only, send `action=set_params expected=<stamp> params={"width": 900}`: the current
source is applied again with those values, the other stored values stay, and an unknown
name is rejected with the list of declared parameters.

To try a change first, send the same `source`/`source_path` or `expected`+`edits` with
`action=check`. It plans the program on a copy of the document, builds the changed parts there
and runs the same exact check as apply, then answers like apply (`change`, `added`, `removed`,
report, `exact_collisions`) with `check_only: true`. Nothing is published: the source, the
model, the selection and the Undo history stay as they were.

A program that does not evaluate is rejected with `details.location`: `file`, `line` and
`column` (1-based) of the innermost place in your own program that led to the failure, an
`excerpt` of that line and two lines on each side from the evaluated text (after a patch,
the patched lines), and `call_stack` (each called `function` and where it was called).

Full report fields:
- `issues`: severity, kind, parts, message, location in mm, a hint, and
  `source_lines` (`first`/`last`) of the program lines that define the parts;
- `relations`: how parts sit against each other, one entry per pair that
  touches, overlaps, is jointed, or is within 20 mm: `kind` `contact` (the
  touching `faces` of each part in its own frame and `area_mm2`), `touch`
  (edge or corner only), `overlap` (`depth_mm` and `status`: `subtracted`
  with `cut_in` naming the part that holds the socket and `depth_mm` how far
  the other reaches in at its deepest; `removed`, `pocket`, `joint`,
  `collision`, `unverified`, `boxes_only`) or `gap` (`gap_mm`); `direction`
  is the world unit vector from the first part towards the second; `joint`
  names a declared joint; `approx` marks a pair measured on the box of a
  profile body or an intersected part. MCP previews at most 8; report pages retrieve every relation.
- `params`: every parameter with its value, default and range;
- `bom.cut_list`: identical parts grouped by material and sorted dimensions;
- `bom.hardware`: fasteners from joints, e.g. `dowel 8x30: 8`;
- `bom.machining`: every hole and pocket per part, in face coordinates.

When the program itself is wrong, the interpreter error names the file, the
line and the cause and quotes the source.

A face or edge the user picks in the window on a program-owned part is
reported by the live `status` under `selected_context.program` in program
terms: `part`, `face` (`x-` ... `z+` on a box, `start`, `end` or a segment
name on a profile part, `null` for a face the program cannot name, e.g. a hole
wall), `on_face` (what `on(part, target, face=...)` takes), and the picked
point and outward normal in the part's frame and in the world. An edge gives
`edge`: the two faces that meet there, as `fillet`/`chamfer` `edges=` take.

## Builtins (Rust, generic)

| Builtin | Purpose |
|---|---|
| `param(name, default, min=, max=, doc=)` | a number the caller can override |
| `box(name, size, at=, material=, color=, attributes=)` | an axis-aligned part; `at` is its minimum corner. Every body (`box`, `extrude`, `revolve`, `sweep`, `loft`) takes `material=`, `color=` and `attributes=` (a dict of strings kept on the part; the library's `board(grain=)` stores `"grain"` there) |
| `part_info(part)` | current `name`, `size`, `at`, `max` of a part |
| `face(part, name)`, `faces(part)`, `face_at(part, point)` | a flat or round face of any part in world: `kind`, `origin`, `normal`, `u`, `v`, `min`, `max`, `radius`, `center` |
| `hole(part, face, at=(u, v) or world=(x, y, z), diameter=, depth=, through=False, id=)` | a hole drilled along the face's inward normal; returns an operation reference for `joint_link`; `face` is a name or a `face()` value; on a round face `at=(angle, v)` |
| `pocket(part, face, rect=(u0, v0, u1, v1), depth=, id=)` | a rectangular pocket in a flat face; may run off the face edges. Holes and pockets are operations like `cut` and `push_pull`: each is machined where the part is when the program writes it, in that order, and its `id` shares the part's operation names |
| `contact(a, b)` | where two parts touch: `axis`, `face_a`, `face_b`, `min`, `max`, or `None` |
| `joint(a, b, kind=, fasteners=, fastener=, volume=, max_gap=, name=, links=[])` | a declared connection; links explicitly associate operations and optional physical hardware |
| `expect(name, terms=, op=, value=, tolerance=, unit=, hint=)` | a condition on the final model: a sum of `reach`/`distance`/`contact_area` measures compared with a value; the library's `expect_contact`, `expect_gap`, `expect_flush`, `expect_symmetric` and `expect_inside` are built on it |
| `extrude(name, profile=, distance=, at=, tool=)`, `revolve(name, profile=, axis=, angle=, at=, tool=)` | a closed profile in the local XY plane padded along local +z or turned about `axis` |
| `sweep(name, profile=, path=, bend=None, up=None, at=, tool=)`, `loft(name, sections=[(profile, z), ...], at=, tool=)` | a closed profile carried along a smooth path; a solid through 2 to 16 stacked sections |
| `helix(radius=, pitch=, turns=, at=(0, 0, 0), axis=(0, 0, 1), start_angle=0, left=False)` | a sweep path; swept with `up=axis` the profile keeps its axial section on every turn, so a thread profile (`v_thread`, `trapezoid_thread`, `round_thread`) makes a thread |
| `fillet(part, edges=[[face_a, face_b], ...], radius=, name=)`, `chamfer(part, edges=, distance=, name=)`, `cut(part, profile=, depth=, name=, through=False)`, `push_pull(part, face=, distance=, name=)` | operations on extruded and revolved parts |
| `shell(part, thickness=, open=[faces], name=)` | the part hollowed to walls `thickness` thick, open at the listed faces; the inner wall along face F is `<name>.F`. `open` lists at least one face, each once (a closed hollow is refused); `thickness` is under half the part's thinnest size; a swept or lofted part has no named faces and is refused; `name` has no `#`, `,`, `(`, `)`, `.` or `:` |
| `subtract(part, tool, name=)`, `intersect(part, tool, name=)`, `union(part, other, name=)` | booleans in the order written. `union` needs the two to touch or overlap, refuses an `other` that has holes or pockets (drill and mill the joined part afterwards) and leaves one part: `other` is no longer a part of its own. A part holds at most 10 000 bodies through its booleans and its tools' booleans nest at most 32 deep, so cut from tool bodies rather than from parts that cut each other in turn |
| `copy(part, name)`, `mirror(part, axis="x", name=)` | an identical new part; the solid reflected across its own middle plane, faces keeping their names |
| `move(part, by=)`, `rotate(part, axis=, angle=, pivot=)`, `place(part, origin=, z=, x=)` | a shift by a world vector, a turn in degrees, a new frame of the part |
| `reach(part, direction)`, `material_along(part, point, direction, radius=0)`, `distance(a, b)`, `nearest(part, among=None)` | how far a part reaches along a world direction; mm of material from a point, cuts and pockets taken out, with `radius` the least on the axis and rim of a drill that wide (`None` if unknown); `distance`, `overlap`, `touching` and `direction` to another part or to the closest one (`None` without one), on each part's box in its own frame |
| `group(name, members, grounded=False)`, `component(name, members)`, `instance(name, component, at=, x=, z=)`, `continue_part(part, was=)` | groups in the document tree, a shared assembly and its instances, a renamed part keeping its ID |
| `floor(z)`, `alternatives(tags)` | the support plane (z = 0 without it); tags that show the same thing in different detail, whose parts may overlap |
| `slide(axis, min, max)`, `rotate_motion(axis, min, max, pivot=)`, `assembly_step(motion, start=, end=)`, `tool_access(name, envelope=, motion=, start=, end=)` | a motion for `joint(kind="motion")` in mm or degrees; an insertion in source order; a tool envelope's approach, checked by the `assembly_path` and `tool_access` validators |
| `keep_clear(zone, name=, only=, ignore=[], hint=)`, `load_path(only=, carriers=[], fixed=[], name=, ignore=[], hint=)` | a zone no part may reach into; members that must pass their weight down (a part leaning on or hanging from them needs a `joint()` or a `fixed` tag) |
| `material_weight(material, source=, kg_m3=, kg_m2=)`, `weight_scope(tags)`, `area_load(name, kind=, kn_m2=, on=[tags], projected=True, source=)` | self weight and area loads in kN/m² on the load-path members |
| `text(name, value, at=, height_mm=2.5, tags=)`, `dim(name, a, b, offset_mm=0, height_mm=2.5, tags=)`, `leader(name, value, point=, at=, height_mm=2.5, tags=)`, `polyline(name, points, closed=False, tags=)` | drawing annotations, no parts (never in the cut list, collisions, mass or takeoff): a text at a point, a dimension whose value Kečup measures between two points, an arrow from a point to a text, drawn lines; text height in mm on paper, the dimension offset in model mm; the library's `chain_dim(name, points, offset_mm=)` dimensions neighbouring points |
| `timber_strength(material, strength_class=, service_class=, glulam=, fm_k=, fv_k=, fc0_k=, fc90_k=, e0_mean=, e0_05=, source=)` | the EC5 characteristic values in N/mm² of one material, behind the library's `timber_design` |

A box's faces are `x-`, `x+`, `y-`, `y+`, `z-` and `z+` in the part's own
frame; their coordinates `(u, v)` are measured from the part's minimum corner:
z faces use (x, y), x faces use (y, z) and y faces use (x, z). A box is an
extrusion like any other: the rectangle `y-`, `x+`, `y+`, `x-` padded from cap
`z-` to cap `z+`. An extruded or revolved profile has `start`, `end` and its
segment names; a flat side's u runs forward along its dominant axis, which is
what measures a box's sides from its minimum corner. A boolean leaves
`<operation>.<tool face>`. `face()` gives each face's frame, and faces follow
`push_pull`, `mirror`, `rotate` and `place`, so holes and pockets work in any
order with them.

## Library (Starlark, `crates/ketchup-program/library/prelude.star`)

`board`, `dowels`, `groove`, `rabbet`, `hole_row`, `spread`, `divide`, `sum`,
`round_to`, and the `DOWELS` table. New joinery, hardware or product types are
added here, never in Rust (see `AGENTS.md`).

`dowels(a, b, dowel="8x35", clearance=1.5, rest=6)` sizes each hole by the
part it enters: a hole never comes nearer the far side than `rest` or a third
of the thickness, so the face of an 18 mm board takes 12 mm. When one part
cannot take half the dowel, it gets what it can and the other part (drilled
into its edge) the rest, every hole `clearance` deeper than its dowel end
(8x35 side to shelf: 12 mm + 26 mm). Parts that cannot hold the dowel report
both parts' limits without aborting the model. A missing contact or impossible base row returns no centres, holes or hardware; correct the reported problem rather than treating the missing join as successful. An explicit row `offset` retains the requested centres and reports unsafe clearance instead of clamping them.

`timber_design(materials, service_class=1, classes=TIMBER_CLASSES)` checks every
`load_path()` member of the named materials after EN 1995-1-1 (EC5), e.g.
`{"KVH C24": "C24"}`; `TIMBER_CLASSES` holds C16, C24 (EN 338:2016), GL24h and
GL28h (EN 14080:2013). Report section `design.members` gives each member's checks
(bending, shear, shear_notch, deflection_inst, deflection_fin, bearing and
buckling of columns) with their utilization and `pass`, `fail` or `not_verified`
when an input is missing. It is a computed check, not an authorized structural design.

`distribute(parts, a, b, face=None)` spaces several parts between `a` and `b`
with equal clear gaps (two shelves between bottom and top make three equal
compartments) and returns the gap; parts that do not fit fail with the numbers.

`hinge(door, side, count=None, ...)` hangs a door (inset or overlay) on
concealed 35 mm cup hinges: cups in the door's inner face 4 mm from the hinge
edge, two mounting-plate holes per hinge in the side's inner face 37 mm behind
the door and 32 mm apart, and a `hinge` joint whose `max_gap` covers the
reveal, so the door is carried and not floating. The count follows the door
height (2 up to 900 mm, 3 to 1600, 4 to 2000, then 5); a door further than
`max_gap` (4 mm) from the side is refused with the distance. Each cup and its two plate holes are explicitly linked to their hinge; `dowels()` likewise links both mating holes. Generic `joint(..., links=[joint_link([operation_refs], hardware=[parts])])` records ownership without inventing it from proximity. Delete or replace the generating helper through a source patch to regenerate only its holes; deleting a metadata-only declaration does not erase independent machining. Pick context distinguishes linked solid hardware from catalog metadata. A surface not matched to supported drilling reports `machining_provenance.state=not_identified`, not an inferred owner or a claim that no cut exists.

`nut(name, size, norm="ISO 4032")` (also `DIN 934`, `ISO 7040`, `DIN 6334`),
`washer(name, size)` (ISO 7089), `bolt(name, size, length, norm="ISO 4017")`
(also `ISO 4014`, `ISO 4762`) and `threaded_rod(name, size, length)` (DIN 975)
place a simplified steel body, seated at `at` along `axis`, with `designation`
(e.g. `ISO 4017 M8x40`), `standard`, `mass_basis` and `source`; they are bought,
so report section `hardware` lists them and the cut list does not.
A size or bolt length the norm does not have is refused with the valid ones;
`bolt_lengths(size, norm)` lists the lengths. An ISO 4014 bolt is threaded only
`bolt_thread_length(size, length, norm)` from its tip (b = 2d + 6, 2d + 12 or
2d + 25 by length; M8x80: 22 mm), the rest is a plain shank of diameter d; the
view, the drawing, the modelled thread and the tapped-hole check take only that
length. ISO 4017 is threaded to the head. The dimensions come from public
catalogues, not from the text of the standards; a value of one size that only
one catalogue gives is named in that part's `source`. `rebar(name, diameter,
length)` places a straight B500B bar (EN 10080, Ø6–Ø40) as a plain cylinder of
the nominal diameter, with `grade`, `diameter_mm`, `shape` and `cut_length_mm`.
Bars are ordered as pieces of a length and shape, not by weight: any part with
`cut_length_mm` is in report section `bar_schedule` (the cutting and bending
schedule: position, diameter, grade, shape, cut length, count, total length and
the mass derived from `mass_kg`, Ø12 × 1000 mm = 0.888 kg, which the window's
mass of the solid equals), not in the hardware list or the cut list.
The parts get no holes by themselves: drill them a little wider than the shank
and subtract a bar or rod cast into concrete from the concrete.
[`examples/programs/anchored-beam.star`](../examples/programs/anchored-beam.star)
anchors a timber beam to a reinforced footing with threaded rods, washers, nuts
and bolts.

Every made part needs a fabrication role (attribute `FABRICATION_ROLE`); the
production export refuses a part without one and names it. The roles are
`TIMBER_MEMBER` (a bar cut and machined from timber stock: BTLx and the bar
list), `MANUFACTURED_ITEM` (a part made in the shop: a panel, a steel or cut
piece; the cut list), `PURCHASED_ITEM` (bought finished: fasteners, bars,
hardware) and `SITE_BUILT_ITEM` (built on site: a cast footing, masonry; in
the BOM and on the drawings, in no machine file). `member` sets a timber member, `board` a manufactured item, the
fasteners and `rebar` a purchased item; each takes `role=` to change it (a
steel post from `member(..., role=MANUFACTURED_ITEM)`, a bought panel from
`board(..., role=PURCHASED_ITEM)`). `buildup` makes the plates, studs,
noggins, headers and sills of a framed layer timber members and sheet and
infill pieces manufactured items; a layer's `"role"` and `"infill_role"`
change them. A part from `box`, `extrude`... sets it in
`attributes={FABRICATION_ROLE: MANUFACTURED_ITEM}`. `role=` refuses any other
value at once; another value in `attributes=` stops the export, which names it.

The file is split into topics by `#@topic id: title` lines (`basics`,
`placement`, `profiles`, `machining`, `joinery`, `fasteners`, `intent`, `annotations`, `space`, `buildup`,
`openings`, `loads`, `member_check`, `validation`, `report`). Each
topic holds the comments that document its builtins and the helpers that
belong to it, so an AI reads the index and then only the topics it needs;
concise topic answers contain signatures, rules and a short example; helper code is
returned only with `detail=implementation`. Named example files return runnable source. A new helper goes into the
topic it belongs to.

## Checks

| Kind | Severity | Meaning |
|---|---|---|
| `collision` | error | two parts overlap, and the overlap is neither inside a pocket of one of them nor inside a declared joint volume |
| `hole_outside_face` | error | a hole does not fit on its face |
| `hole_breaks_through` | error | a blind hole is as deep as the part is thick; `through=True` explicitly declares intentional penetration |
| `through_hole_too_shallow` | error | the requested through depth does not span the part; numeric geometry is retained, never silently deepened |
| `hole_wall_too_thin` | warning | a blind hole leaves less than 3 mm of the part behind it |
| `joint_without_contact` | error | a joint connects parts that are further apart than its `max_gap` |
| `param_out_of_range` | error | a parameter is outside its `min`/`max` |
| `program_condition_failed` | error | `check(condition, message, parts=[...], hint=...)` found a library design problem; evaluation continues and the condition is returned |
| `floating_part` | warning | a part touches nothing that rests on z = 0 |
| `production_tag_unused` | error / warning | no part carries a tag of `production(tags=…)`; an error when therefore nothing is made, a warning when another tag still makes parts |
| `production_leaves_out_ordered_parts` | warning | bought parts (`designation`) or bars (`cut_length_mm`) carry none of the `production()` tags, so they reach no hardware list or bar schedule; `tag()` them to order them |
| `expectation_failed` | error | a stated condition (`expect*`) does not hold; the message gives the measured and the required value |
| `collision_unverified` | warning | the boxes of two parts overlap, a part is not its box, and the exact solids were not measured |
| `expectation_unverified` | warning | a condition needs the distance of two solids that are apart, but that distance was not measured |
| `checks_unverified` | warning | the checks did not finish within the program worker's time, memory or size limit; the edit is still applied, and no collision, relation, joint or load was checked |
| `report_truncated` | error or warning | the whole report did not fit one reply of the program worker, so it lists only its first issues (errors first) and relations; an error when an error was left out |

Automatic checks use native solids where machining (holes, pockets, booleans)
or another shape change makes a part differ from its box near another part.
Declared minimum distances are measured even when bounds are far apart.
The headless commands and window use these answers for collisions, contacts,
joints, expectations and relations. `program action=validate expected=<stamp>`
checks all program parts without editing; its summary distinguishes incomplete
checks from passes. Standalone `ketchup-program` has no OCCT and reports that limit.

Checks report; they never stop the program from being evaluated or built.

`hole(..., through=True)` retains the explicit intent in the program and machining
report. Valid cuboid through cuts use canonical ThroughAll geometry and retain the
intent in neutral manufacturing output after Save/Open. Too-short through cuts keep
the requested depth as boolean geometry and report an issue; they cannot masquerade
as approved blind machining. Later/non-cuboid bores remain boolean operations, not
inferred machine drilling. BTLx drills a through hole, or a blind one that reaches the
far face, with `DepthLimited` `no` and the depth at which its axis leaves the stock,
never the padded cutter. woodWOP (HOMAG) export of through cuts is unsupported and
rejected rather than emitting an unverified machine cycle.

Joint fastener centres are anchored to the first endpoint when that part is moved or
rotated after the joint declaration. Moving the second endpoint does not move them
again or repair a separated joint; contact validation still reports that separation.

## Current limits

- Parts are cuboids, extruded or revolved profiles (lines and exact circular
  arcs; `round_corners` rounds a point loop), profiles swept along a smooth
  path of lines and arcs (`sweep`, corners rounded with `bend=`) or lofted
  through stacked sections (`loft`), and their booleans; any part can be
  rotated. Fillet, chamfer, cut and push_pull apply to extruded and revolved
  parts only; paths are planar or spatial lines and arcs, not splines.
- Program apply/patch retains the parametrically owned source and reconciles surviving
  parts. Editing a part the program makes, in the window or by a typed edit, is written
  into the source: a move edits a literal `at=` or adds `move()`, a turn adds `rotate()`,
  delete removes the creating call, a colour sets `color=` in it, rename changes the name
  and adds `continue_part()`, mirror adds `mirrored()`, Push/Pull, fillet and chamfer add
  their call, and a panel value one `param()` drives becomes that parameter's override.
  A part made in a loop or by the program's own function, a part another line uses, or a
  value written in the source is refused with the line to change. Other edits (scaling,
  components and their copies, new geometry) detach the program, the window says why, and
  Undo restores ownership.
- Preserve the user's requested simple joints: do not replace dowels with screws or
  brackets without approval. Check transport envelope, access and assembly order early;
  offer a modular alternative, do not impose it. Contact/support checks do not establish
  structural load capacity or provide FEA verification.
- Distinguish a changed requirement from a defect, and an unmeasured result from an
  impossible measurement. Busy diagnostics describe backend evidence, not confirmed
  visible window state. Report changes and uncertainty briefly; do not cancel human work.
