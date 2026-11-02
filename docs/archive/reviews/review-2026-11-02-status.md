# Review 2026-11-02: status of the findings

Branch `r18-review1102`. Every finding of `review-2026-11-02.md` is listed here as fixed
(with its commit), deferred (with the reason), or left to the macOS instance.

## Release and CI

| Finding | Status |
|---|---|
| REL-120: the Windows Setup and Portable ship without the Visual C++ runtime | Fixed, 99e22efb: the four runtime DLLs go into the payload and every EXE/DLL's imports are checked against the payload and Windows itself |
| CI-120: the published-release guard always skips in CI (no tags in the clone) | Fixed, 02dae261 |
| TST-123: `cargo clippy` without features fails on `report_path` in the Android Viewer | Fixed, 80fe7a0a and acdd9266 |

## PG (statics)

| Finding | Status |
|---|---|
| PG-120: any touch of a part on two members counts as lateral bracing | Fixed, c1f7ea27: only the compressed edge (top under sagging, bottom under hogging), only from a part screwed to the member by `joint()` (or of the `fixed` tags) or also bearing on a grounded part; `braced_by` in the loads report |
| PG-122: with kcrit below 1 the bending row shows the value without kcrit; no `lateral_torsional` row | Fixed, c1f7ea27 |
| PG-105 (rest): `load_path` + `timber_design` without loads gives statics "none", rc 0 | Fixed, 666470b8 |
| PG-106: OSB without `product=` takes the kmod of solid timber | Fixed, 2511142e |
| PG-124: rafters under counter-battens and battens are not braced (`not_verified`) | Deferred: new feature. Bracing passed on through a part that rides on one member (a counter-batten screwed along a rafter, battens across the counter-battens) is proposal 5.1 (explicit bracing, transitive holding); without it the result stays on the safe side, `not_verified` with the stretch named |

## Blank and program

| Finding | Status |
|---|---|
| PG-121: above 45 degrees the turned blank swaps length and width | Fixed, 18437c2a: the blank's long side is the length the part names (LENGTH_AXIS), at any pitch |
| PG-123, PG-108 (rest): the shop drawing, CSV/XLSX cut list and CNC DXF size the blank each their own way | Fixed, f7e63559: all take the cut list's blank |
| PG-102 (rest): two stock sizes give two boards instead of one | Fixed, 5c8faee5 |
| PG-125: the blank ignores the grain direction | Fixed, 3365a31a and 9dc3b494 |
| WB-42 (rest): `whole=True` only shows when the evaluation survives the value; the window does not read `Param::whole` | Fixed, 99866798: `param()` refuses the fraction, naming the parameter; the dock field refuses it |
