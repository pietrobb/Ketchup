# Review 2026-10-31: status of the P3 findings and structural proposals

Branch `r15-review1031`. Every P3 finding of section 4 of `review-2026-10-31.md` is listed
here as fixed (with its commit), disproved or explained (with the evidence), deferred (with
the reason), or left to the macOS instance. P1 and P2 findings are fixed in the commits
that cite their IDs.

## CD (DXF/DWG write-back)

| Finding | Status |
|---|---|
| HATCH of extrusion −Z still refuses the whole export (CD-69 rest) | Fixed, d76ab1ce |
| References from layout blocks dangle (`without_removed` scans only ENTITIES) | Fixed, d76ab1ce |
| DXF → DWG through acadrust adds MTEXT wrap width 10 | Fixed, d76ab1ce |
| DWG output is always AC1032 without notice | Fixed, d76ab1ce: the export names the version it wrote |
| Characters above U+FFFF become `?` before DXF 2007 | Fixed, d76ab1ce |
| LWPOLYLINE written into an R13 drawing | Fixed, d76ab1ce: written as POLYLINE |
| Copied LEADER keeps the original's `340` | Fixed, d76ab1ce |
| Group 12 of a copied dimension is mapped as a vector | Explained, and a related defect fixed in the commit that adds this file. The importer adds group 12 as an offset to the dimension block, and write-back maps the block's entities whole, translation included, so group 12 must turn with the map's linear part only, as a vector. The block written for a moved or copied dimension did keep the original's base point, though, so a turned dimension whose block has a base point read back elsewhere. The base point now turns as well. Test: `turned_dimensions_whose_block_has_a_base_point_are_written_where_they_now_lie` |
| Stretch rescales extended data 1041/1042 | Fixed, d76ab1ce |
| `TooLarge` and `Malformed` reported as `drawing_source_unavailable` with a wrong hint | Fixed, d76ab1ce: `drawing_source_unwritable`, naming the repair |
| Bridge returns `Result<Value, &'static str>` | Deferred: a signature change across every live-bridge handler, part of the cause-chain refactor of review section 2. It changes no answer a client sees |
| CD-64: export needs an exact line match, so any importer change blocks the export of every edited drawing | Deferred: new feature (record the importer version with the underlay, or match lines within tolerance). The refusal is explicit, never a silent wrong file |

## UO and UL (areas from drawings)

| Finding | Status |
|---|---|
| `walled` and `ring_layers` cost O(boundary × all segments) on the UI thread | Deferred: needs a spatial index shared with the region search (structural proposal 5). It affects speed only, never the result |
| Section heading states the polygon's area | Fixed, 36b26f07: it measures the arcs |
| `MAX_ITEMS` counts chord corners before they merge into arcs | Disproved: the data always writes `NAME_POINTS` and `NAME_HOLES_POINTS` with every chord corner (`underlay_program.rs` `outline_data`), so the limit counts what is written |
| A pending Pull area click is still used after Undo | Fixed, 36b26f07 |
| `solid_from_views` measures its box from segment starts only | Disproved: an outline is closed, so every segment's end is the next one's start. Arcs are split at their circle's quarter points (`underlay_arcs.rs`), so no arc bulges past its ends along the axes of a view, and views stand on axis planes |
| UL-41: beyond 10 km it falls back to identity without a diagnostic | Fixed, d76ab1ce |

## PG (program results)

| Finding | Status |
|---|---|
| Lath chosen greedily: 2×1500 + 3×1000 on [3000, 5000] buys 9 m instead of 8 m | Fixed, b6308755 (that very case is tested) |
| Cutting plan leaves a part out without an issue while `ok` stays true | Fixed, b6308755 |
| A second `stock()` of a material silently replaces the first | Fixed, b6308755: refused |
| Prices keyed by name only | Fixed, b6308755: a name matching two kinds of counted item prices neither |
| Hole key in `making()` ignores the thread | Fixed, b6308755 |
| PG-40: a block hanging 0.5 mm under a beam passes; `ok` and the CLI ignore `not_verified` | Fixed, b6308755 |

## PL, EX and MCP

| Finding | Status |
|---|---|
| EINTR from `poll` ends a frame halfway | Fixed, eae4f30e |
| The MCP client still reads with SO_RCVTIMEO | Explained: a timed-out read is a transport error, and the server then drops the connection (`tools.rs`, `self.attached = None`) and answers `connection_lost`, telling the client to read the status first. No later frame is ever read from a stream that may have lost bytes. The next call opens a new connection |
| The window reads worker outputs whole without a limit; `reserve_whole` guards only on Windows | Deferred: structural proposal 2 (one memory guard in the scheduler for all platforms) |
| Worker identity leaves out OCCT on macOS and Linux and hashes the whole executable at every start | Deferred: new feature (an identity of the OCCT libraries each platform loads, cached by file identity). The macOS part belongs to the macOS instance |
| On macOS a plugin outlives a window crash | macOS instance |
| Stdin lines of the MCP server and headless are unbounded; one invalid UTF-8 byte ends the MCP server | Fixed, eae4f30e (headless was already bounded) |
| `WorkerError::*(String)` and others drop the cause (section 2) | Deferred: the typed cause-chain refactor of review section 2, across the scheduler and its callers |
| Every export leaves `<file>.ketchup-export-lock` | Fixed, eae4f30e |
| The memory budget counts host RAM inside a container | Deferred: structural proposal 2 (read the cgroup limit in the one scheduler guard) |
| `find hashed -exec …` passes the plugin whitelist | Fixed, 7fb3847a: no word may follow the script of an executable that is no listed interpreter |
| The OCCT cache key on Linux and macOS leaves out the clang or Xcode version | Linux fixed, 7fb3847a; macOS (Xcode) belongs to the macOS instance |

## MD and UI

| Finding | Status |
|---|---|
| MD-44: two unprivileged users on NFS share the shared lock | Deferred: needs a lock protocol that holds on NFS (an owner file with a lease), a new feature. Local file systems are covered |
| UI-62 rest: every edit clones the whole decoded drawing, ~2× RAM at 400k lines | Deferred: needs shared (copy-on-write) underlay lines, a refactor of the underlay edit path |
| The detach warning stays off after the program is applied again | Fixed, 36b26f07 |
| Dead public API in `ketchup-interaction` (`face_intent.rs`, `ActionDigest`…) | Fixed, 7fb3847a: removed with its tests |
| `viewport.rs` has 4 854 lines against a ceiling of 5 000 | Deferred: under the ceiling, which the size ratchet holds. Splitting the file is a refactor, not a defect |
| `Ketchup plan.dxf` as an argument creates a lock beside the DXF and ends with a generic error | Fixed, eae4f30e |

## MF (manufacturing outputs)

| Finding | Status |
|---|---|
| woodWOP does not name a blind hole whose depth reaches the opposite face | Fixed, d79ab607 |
| Identical bytes on every system rest on rounding only (platform libm in CNC, BTLx and PDF; 3MF writes `{:.17}`) | Deferred: new feature (correctly rounded maths and number formatting for these outputs, extending ADR 0010) |
| CNC outlines keep the profile's direction, so a part laid upside down has clockwise outlines | Fixed, d79ab607 |
| MF-13 rest: a dimension block with base ≠ 0 fails the whole file with a bare `InvalidBlock` | Fixed, d79ab607; write-back of such a dimension when turned, in the commit that adds this file (see CD) |

## CI, VW, DOC and tests

| Finding | Status |
|---|---|
| CI-62: macOS lacks clippy `--all-targets` and the `-app`, `-application`, `-scheduler` tests | macOS instance |
| VW-40: author dimensions write a point, measurements a comma by language | Fixed, d76ee845 |
| `README.md:213` and other files cite ADR 0009 instead of 0010 | Fixed, d76ee845 |
| README says "341 profiles", the code allows 1 024 | Fixed, d76ee845 |
| The Viewer error banner hides `caused_by` | Fixed, d76ee845 |
| `open_contexts_to` loses the author's context even when the target is not found | Fixed, d76ee845 |
| Web/Android round trip in CI uses fixtures refreshed by hand | Deferred: generating them in CI needs the web and Android builds in the release pipeline (structural proposal 1) |
| `pre-push` calls `python`, which macOS lacks | macOS instance |
| `comments.rs:61` checks a SHA from fixed bytes | Fixed, 8fed1080: hashed apart from the writer |
| `format_golden.rs:72-78` digests | Explained: for current formats the opened document's `source_canonical_digest` is recomputed from the model read (`persistence.rs`), so the constants pin the model a fixture opens to; a reader change that alters it fails the test (FMT-1) |
| `benchmark.rs:203` cannot fail | Fixed, 8fed1080: the median-under-worst tautology is gone |
| 24× `yield_now` as a busy-wait | Fixed, 8fed1080: they sleep a millisecond and count in the sleep ratchet |

## Structural proposals (section 6)

| Proposal | Status |
|---|---|
| 1. Release gate and one manifest | Partly done by CI-80 (8d483d7f) and VW-81 (3ca995a1); the single manifest and generated notes are deferred as a new feature |
| 2. One memory guard in the scheduler for all platforms | Deferred: large refactor across the scheduler, plugins and worker outputs; the macOS part belongs to the macOS instance |
| 3. One "machining described" state | Done by MF-80, MF-81, MF-85 (3843fcec) |
| 4. One "counted parts" function and one blank model | Deferred: refactor; its findings PG-80 to PG-86 are fixed one by one (00435146, 5a707d0b) |
| 5. Topology before tolerance for drawing areas | Deferred: refactor; UO-80, UO-81 and UO-84 are fixed (3d75ae52) |
| 6. One reference pass in DXF write-back | Deferred: refactor; CD-80, CD-81 and the CD rest are fixed (27fc4e76, d76ab1ce) |
| 7. A type-aware lint instead of regex ratchets | Deferred: new tooling; TST-80 closed the reported bypasses and shares one list of roots (23892cd4, 4f9b063b) |
