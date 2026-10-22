# BTLx 2.3.1: Ketchup coordinate conventions

Status: the convention is pinned by tests, **not yet confirmed in a concrete importer**
(the support report says `concrete_importer_verified=false`). To confirm it, open
`crates/ketchup-manufacturing/tests/fixtures/btlx/centred-60x140-drilling-2.3.1.btlx`
in a BTLx viewer or a machine importer and compare it with the expectation below.

## Stock frame

A member is a straight prism: a rectangular profile in the definition x/y plane,
extruded along z. The stock frame has its origin at the **minimum corner** of the
profile, not at the definition origin. BTLx, woodWOP and `production_job` use the
same corner (`StockBlank` in `fabrication.rs`).

| BTLx       | Part axis | Definition axis | Dimension         |
|------------|-----------|-----------------|-------------------|
| `Length`   | X         | z               | extrusion length  |
| `Width`    | Y         | x               | profile width     |
| `Height`   | Z         | y               | profile height    |

The definition point `(x, y, z)` is `(z, x − x_min, y − y_min)` in the part.

A stock longer across its profile than along z (a box drawn lying along x or
y) is measured along its longest axis instead: part X is that axis and Y, Z the
next definition axes in cyclic order, so the part frame stays a rotation. A
`(4000, 120, 200)` box gives `Length="4000" Width="120" Height="200"` and the
definition point `(x, y, z)` is `(x − x_min, y − y_min, z)`; along y it is
`(y − y_min, z, x − x_min)` (review 2026-10-21 MF-22).

## Machining reference plane

`UserReferencePlane` has `ReferencePoint`, `XVector` and `YVector` in part
coordinates. The plane normal is `XVector × YVector` and points **into the
material**: a drilling starts at `ReferencePoint + StartX·XVector + StartY·YVector`
and goes `Depth` along the normal.

## Worked example

Profile `(−30, −70)…(30, 70)`, length 2000 mm, a Ø10 hole 50 mm deep into the end
face at profile point `(15, 40)`:

- `Length="2000" Width="60" Height="140"`,
- hole entry in the part `(2000, 45, 110)`, end `(1950, 45, 110)`.

If the importer shows the hole 45 mm from the edge on the 60 mm side and 110 mm
from the bottom on the 140 mm side, Width/Height and the normal direction match.

## Tests

- `btlx_of_a_60_by_140_member_centred_on_its_axis_maps_width_height_and_corner`:
  the golden file above.
- Every BTLx golden passes `assert_btlx_machining_lies_in_its_blank`: every
  machining point on the plane and at depth lies in
  `[0, Length] × [0, Width] × [0, Height]`.
- `a_profile_centred_on_its_axis_is_machined_from_its_minimum_corner`: BTLx and
  woodWOP measure from the same corner.
