# Review 2026-11-01: status of the P3 findings

Branch `r17-review1101`. Every P3 finding of section 4.3 of `review-2026-11-01.md` is listed
here as fixed (with its commit), deferred (with the reason), or left to the macOS instance.
P1 and P2 findings are fixed in the commits that cite their IDs.

## CD (DXF/DWG write-back)

| Finding | Status |
|---|---|
| CD-101: an R12 drawing without handles and with one zero-length LINE refuses every edit as `NoHandle` | Fixed, 23a5a6c2 |
| CD-102: a layer name is encoded as text (`%%` → `%%%%%%`) | Fixed, 23a5a6c2 |
| CD-103: removing an MTEXT leader leaves `73 0` | Fixed, 23a5a6c2 |
| CD-104: `write()` and `mapped_record` have 207 lines each | Fixed, bccd9a24 and 22da006d: split into parts under 150 lines, without a new clippy allow |
| CD-105: a hatch moved with its boundary loses its association | Fixed, 23a5a6c2 |
| CD-106: an underlay refuses a DXF with a BOM, a trailing Ctrl-Z or blank lines after EOF | Fixed, 23a5a6c2 |
| CD-107: the export's publish (write, `sync_all`, rename) runs on the UI thread | Fixed, 23a5a6c2 |

## UO and UL (areas and inserts from drawings)

| Finding | Status |
|---|---|
| UO-101: a hatch whose hole touches the outer boundary is refused as two outlines | Fixed, de2d10ea |
| UO-102: skipped holes are stored once, so a shaft added later is neither cut nor named | Fixed, de2d10ea |
| UO-103: arcs another CAD wrote as a polyline of chords stay polygons | Deferred: new feature. Recovering arcs from chords needs a fitting rule and a tolerance decision (which runs of chords are an arc, how far a chord may lie from it); today the outline is exact to the drawn lines, never a wrong arc |
| UL-101: a door block with non-uniform scale and an arc is dropped whole | Fixed, de2d10ea |
| The placing budget of an insert refused later is not given back | Fixed, de2d10ea |

## PG and WB (program results, writing to the program)

| Finding | Status |
|---|---|
| PG-104: `production(tags=["detailed"])` drops every untagged part silently | Fixed, 42aef990 |
| PG-106: kmod of solid timber used for OSB/3 | Fixed, 42aef990 |
| PG-107: a refused connector is counted without its nails | Fixed, 42aef990 |
| PG-108: the cut list and the cutting plan order sizes differently | Fixed, 42aef990 |
| WB-42: a whole-number parameter accepts 2.5 | Fixed, 42aef990 |
| WB-43: Push/Pull −20 on a 10 mm cube leaves a broken part | Fixed, 42aef990 |
| WB-45: refusing `coloured_at_once` names no line | Fixed, 42aef990: the refusal names the line that makes the first part coloured otherwise (test `program_part_change_tests.rs`, review 2026-10-26 WB-45) |

## EX, MCP and SEC

| Finding | Status |
|---|---|
| EX-102: the per-shape "not a closed volume" check changes a stored part's solid, so it opens with `EvidenceMismatch` | Fixed, 160b0fd1 |
| EX-103: multi-shape parts are validated twice | Fixed, 160b0fd1 |
| EX-104: salvage report details (reason for wires only, empty groups, list without `show_rows`, IGES numbers as STEP) | Fixed, 160b0fd1 |
| MCP-101: the cancellation ratchet is a text match only | Fixed, 309479dc |
| SEC-101: `/net/<host>/…` and `/Volumes/<smb>` pass the path check on macOS | macOS instance |

## MD, UI and MF

| Finding | Status |
|---|---|
| MD-101: N copies of one insert take the tree N times | Fixed, 872a6f54 |
| UI-101: a `.pdf` given at start creates a lock and ends with a generic error | Fixed, 2388a0a6 |
| MF-81: the tests check the drawing's source, not the hand drawn | Fixed, 93eee9a8: the test compares the edges the mirrored part's sheet draws |
| MF-104: `SawContour` and `MillContour` keep `DepthBounded="yes"` (MF-82 rest) | Fixed, ab5a7787 |
| MF-105: a through cut outside the blank leaves the part undescribed | Fixed, ab5a7787 |
| MF-106: bare `ExportBlocked` in `row_stock`, `UnsupportedOrUnavailableGeometry` without the part | Fixed, ab5a7787 |
| MF-107: a missing glyph (∅, ′) also blocks the SVG and DXF | Fixed, 191e2912 |

## CI, tests, Viewer and documentation

| Finding | Status |
|---|---|
| TST-100: ratchet bypasses (`let Ok(x) = f() else`, `.or_else(\|_\| Err(..))`, `Err(_) => None`, `SystemTime` as the clock, checked subtraction, `pub async fn headless_*`) | Fixed in the commit that adds this file: each form is counted by its ratchet, and the cited `consent.rs` attach says in the window why the bridge could not open. Clippy `disallowed_methods` cannot express these (they are patterns, not methods); a type-aware lint stays deferred as new tooling, as in review 2026-10-31 |
| TST-101: `#[cfg(test)]` functions and impls in production files are not scanned | Fixed, 93845296 |
| TST-102: code behind the `saved-files` feature is never linted | Fixed, d9398fb5 |
| TST-44: two lock tests fail under root and `release_reads.py` stops before reading | Fixed, a80e6376 |
| CI-100: the macOS OCCT cache key lacks the compiler version | macOS instance |
| CI-101: pull requests run the large-house tests only for `tiny-house.star`; a documentation push costs a full CI | Fixed, 7dbaeeab |
| VW-100: a comment on an unreachable part drops the open import review | Fixed, b2f5e0c0 |
| DOC-101: README lacks cutting plans, quotes, CNC DXF, drawing sets and STEP salvage; no CHANGELOG | Fixed, 09d315a9 |
| DOC-41: `docs/plugins.md:136` still names an image underlay | Fixed, 6bfa8329 |
| The Android release still reacts to `benchmark.ketchup-view` | Fixed, 6bfa8329: only a `-Benchmark` build measures it |
