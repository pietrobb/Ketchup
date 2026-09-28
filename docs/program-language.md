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
| Headless protocol | `program_check` (report only), `program_apply` (replace the document, one undo step) |
| Python SDK | `session.check_program(source, params={...})`, `session.program_document(source, ...)` |
| Agent skill | `KetchupProgram` with `action=check` or `action=build`; `KetchupDiscover section=program` lists the library topics, `name=<topic>` returns one |

Every run returns the same report:
- `issues`: severity, kind, parts, message, location in mm, and a hint;
- `relations`: how parts sit against each other, one entry per pair that
  touches, overlaps, is jointed, or is within 20 mm: `kind` `contact` (the
  touching `faces` of each part in its own frame and `area_mm2`), `touch`
  (edge or corner only), `overlap` (`depth_mm` and `status`: `subtracted`
  with `cut_in` naming the part that holds the socket and `depth_mm` how far
  the other reaches in at its deepest; `removed`, `pocket`, `joint`,
  `collision`, `unverified`, `boxes_only`) or `gap` (`gap_mm`); `direction`
  is the world unit vector from the first part towards the second; `joint`
  names a declared joint; `approx` marks a pair measured on the box of a
  profile body or an intersected part. The Kečup window lists at most 40.
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
| `box(name, size, at=, material=, grain=, color=)` | an axis-aligned part; `at` is its minimum corner |
| `part_info(part)` | current `name`, `size`, `at`, `max` of a part |
| `hole(part, face, at=(u, v) or world=(x, y, z), diameter=, depth=, id=)` | a drilled hole |
| `pocket(part, face, rect=(u0, v0, u1, v1), depth=, id=)` | a rectangular pocket; may run off the face edges |
| `contact(a, b)` | where two parts touch: `axis`, `face_a`, `face_b`, `min`, `max`, or `None` |
| `joint(a, b, kind=, fasteners=, fastener=, volume=, max_gap=, name=)` | a declared connection |
| `expect(name, terms=, op=, value=, tolerance=, unit=, hint=)` | a condition on the final model: a sum of `reach`/`distance`/`contact_area` measures compared with a value; the library's `expect_contact`, `expect_gap`, `expect_flush`, `expect_symmetric` and `expect_inside` are built on it |

Faces are `x-`, `x+`, `y-`, `y+`, `z-` and `z+` in the part's own frame. Face
coordinates `(u, v)` are measured from the part's minimum corner: z faces use
(x, y), x faces use (y, z) and y faces use (x, z).

## Library (Starlark, `crates/ketchup-program/library/prelude.star`)

`board`, `dowels`, `groove`, `rabbet`, `hole_row`, `spread`, `divide`, `sum`,
`round_to`, and the `DOWELS` table. New joinery, hardware or product types are
added here, never in Rust (see `AGENTS.md`).

The file is split into topics by `#@topic id: title` lines (`basics`,
`placement`, `profiles`, `machining`, `joinery`, `intent`, `report`). Each
topic holds the comments that document its builtins and the helpers that
belong to it, so an AI reads the index and then only the topics it needs;
every answer stays under the 32 KiB tool limit. A new helper goes into the
topic it belongs to.

## Checks

| Kind | Severity | Meaning |
|---|---|---|
| `collision` | error | two parts overlap, and the overlap is neither inside a pocket of one of them nor inside a declared joint volume |
| `hole_outside_face` | error | a hole does not fit on its face |
| `hole_breaks_through` | error | a hole is as deep as the part is thick |
| `joint_without_contact` | error | a joint connects parts that are further apart than its `max_gap` |
| `param_out_of_range` | error | a parameter is outside its `min`/`max` |
| `floating_part` | warning | a part touches nothing that rests on z = 0 |
| `expectation_failed` | error | a stated condition (`expect*`) does not hold; the message gives the measured and the required value |
| `collision_unverified` | warning | the boxes of two parts overlap, a part is not its box, and the exact solids were not measured |
| `expectation_unverified` | warning | a condition needs the distance of two solids that are apart, but that distance was not measured |

The checks measure boxes. Where a part is not exactly its box (profile body,
push_pull, subtract/intersect) and its box touches or overlaps another,
KetchupProgram `check` (in a scratch document), `build` and the window measure
that pair on the exact solids with the same native pair query, and collisions,
floating parts, joints, `expect*` distances/contact areas and the relation map
use those answers. The standalone `ketchup-program` binary has no OCCT and
keeps the box answers.

Checks report; they never stop the program from being evaluated or built.

## Current limits

- Parts are cuboids, extruded or revolved profiles (lines and exact circular
  arcs; `round_corners` rounds a point loop), profiles swept along a smooth
  path of lines and arcs (`sweep`, corners rounded with `bend=`) or lofted
  through stacked sections (`loft`), and their booleans; any part can be
  rotated. Fillet, chamfer, cut and push_pull apply to extruded and revolved
  parts only; paths are planar or spatial lines and arcs, not splines.
- A built program replaces the whole document. Keeping the program inside the
  document, and applying manual edits as overrides keyed by part name, are the
  next steps.
