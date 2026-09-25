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
| Agent skill | `KetchupProgram` with `action=check` or `action=build`; `KetchupDiscover section=program` returns the library |

Every run returns the same report:
- `issues`: severity, kind, parts, message, location in mm, and a hint;
- `params`: every parameter with its value, default and range;
- `bom.cut_list`: identical parts grouped by material and sorted dimensions;
- `bom.hardware`: fasteners from joints, e.g. `dowel 8x30: 8`;
- `bom.machining`: every hole and pocket per part, in face coordinates.

When the program itself is wrong, the interpreter error names the file, the
line and the cause and quotes the source.

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

Faces are `x-`, `x+`, `y-`, `y+`, `z-` and `z+` in the part's own frame. Face
coordinates `(u, v)` are measured from the part's minimum corner: z faces use
(x, y), x faces use (y, z) and y faces use (x, z).

## Library (Starlark, `crates/ketchup-program/library/prelude.star`)

`board`, `dowels`, `groove`, `rabbet`, `hole_row`, `spread`, `divide`, `sum`,
`round_to`, and the `DOWELS` table. New joinery, hardware or product types are
added here, never in Rust (see `AGENTS.md`).

## Checks

| Kind | Severity | Meaning |
|---|---|---|
| `collision` | error | two parts overlap, and the overlap is neither inside a pocket of one of them nor inside a declared joint volume |
| `hole_outside_face` | error | a hole does not fit on its face |
| `hole_breaks_through` | error | a hole is as deep as the part is thick |
| `joint_without_contact` | error | a joint connects parts that are further apart than its `max_gap` |
| `param_out_of_range` | error | a parameter is outside its `min`/`max` |
| `floating_part` | warning | a part touches nothing that rests on z = 0 |

Checks report; they never stop the program from being evaluated or built.

## Current limits

- Parts are axis-aligned cuboids. Rotated parts, profiles and curved shapes
  are the next generic builtins to add.
- A built program replaces the whole document. Keeping the program inside the
  document, and applying manual edits as overrides keyed by part name, are the
  next steps.
