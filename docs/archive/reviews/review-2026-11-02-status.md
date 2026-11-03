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
| Function sizes (`loads_with`, `cut_sheets`, `eval.rs`) | `loads_with` shortened in ecdbffde (`fixed_by_tag()` moved out); `cut_sheets` and `eval.rs` deferred: splitting them changes no behaviour and is left to a refactoring branch that can be reviewed on its own |

## Manufacturing

| Finding | Status |
|---|---|
| MF-122: the shop drawing of a rounded board shows sharp corners, without a note | Fixed, 4a7b419e and 1ed1ecf5: a blank that is not a rectangle is drawn as its cross-section with its arcs, sized by the arcs that are cut |
| MF-121: a round table top is "not described" with the wrong reason and blocks the job | Fixed, 6f3d2639: a circle is two half arcs; a spline or a holed outline is refused as a free-form blank |
| MF-120 (MF-102 rest): the HOMAG ZIP never passes in the window or over MCP | Fixed, e3c95e57: the piece without a usable 12-character code is named; Tools > Assign production codes and `set_production_code` give the made pieces codes the program keeps |
| MF-123, MF-124: woodWOP and BTLx refuse a rounded blank, and a single `.mpr` ends in a bare `ExportBlocked` | Fixed, e6480ea9: refused by part with the way out; a part without machining says so |
| MF-125: a sheet exported to DXF/DWG also builds SVG and PDF | Fixed, 7a2f5570 |
| `cnc_dxf.rs` module doc lacks the TAP layer and the depth clipping | Fixed, 7aa6d86a |
| CNC DXF is still outside `admit()` | Deferred: the CNC DXF draws every outline the cut list knows and names what it cannot carry in `not_drawn` (MF-100 TAP layer, MF-122); moving it onto the capability table of the machine formats is a restructuring of its writer with no wrong output today |

## Import, MCP and window jobs

| Finding | Status |
|---|---|
| EX-120: a STEP with an empty subgroup inside a refused group crashes the window | Fixed, fd3820f7: kept nodes in one forward pass; a missing parent is refused evidence, not a panic |
| MCP-121: a drawings export over MCP is cancelled after 30 s | Fixed, 6860f034 and eff9eed6: 15 minutes on both sides of the connection |
| MCP-122: a job lands in another document | Fixed, f4cd962d: New and Open end a client's job; an import or drawings export answers `stale_document` in the next document; a cancelled export puts back only the settings the user left as it kept them |
| MCP-123: the busy reason and thread name say "DWG import" for STEP and drawings | Fixed, 52675b5b |
| Cancelling a drawing set does not stop the hidden-line projection | Fixed, 463994d8 (checked every 256 lines) |
| MCP-101 (rest): `CancelToken::default()` in `ketchup-application`; a cancelled `check_program` holds the worker 45 s | Fixed, f1c75a58, with a ratchet against an unkept `CancelToken::default()` there |
| MCP-101 (rest): the CAM, FEA and PDM dialogs block the window | Deferred: these dialogs wait for the user, not for a worker; running them as window jobs belongs to the job bar of proposal 5.7, which also binds jobs to the document |
| UI-101 (rest): the macOS open event and MCP `open` skip the check | Fixed, dab8349f (every way a file reaches the window) and dce7472c (a document an interrupted save damaged still opens from its `.recovery` backup) |
| 70d56910: a window that cannot open its bridge answers the client `rejected`, like a user's refusal | Fixed in this branch: the window answers `failed` and the MCP server says the window could not open its live connection and that its status line says why, not that the builds differ |
| EX-100 (rest): the window shows a partial import only in a passing status line; the receipt has no placement counts | Deferred: new feature. The persistent import report of proposal 5.8 (every receipt and left-out part with entity numbers and placement counts) is the place for it |
| EX-40: background jobs start more exact workers at once, each kind taking half the memory | Deferred: one memory budget for every worker of the window is proposal 2 of review 2026-10-31. Every worker is capped on every platform now (PL-61 on macOS), so a runaway worker fails with `MemoryLimit`; only several jobs of different kinds together can exceed |
| EX-42: `transport(error)` moves `to_string()` into a helper | Deferred: the message is kept, not dropped; giving the transport a typed error is a change of the bridge's error type across every tool and is left to its own branch |
| EX-62: face references under tolerance are held by order without checking the faces | Deferred: a check needs a face-matching rule (which faces of the healed shape are the same) that does not exist yet and is a design decision of its own |

## Underlays and DXF/DWG write-back

| Finding | Status |
|---|---|
| UL-120 (UL-100 regression): one "Nº", "1ª", "45º" or "G¾" turns a western drawing into CP1250 | Fixed, 0cfbda9e: ª and º are ignored, ¼ and ¾ count only between letters, the code page is chosen by a vote of words (1250 against 1252); the western cases are in the test |
| UL-100 (rest): "Predsieň", "Kuchyně", "ŘEZ A-A", "Střecha", "Izba č. 1" alone stay in 1252 | Fixed, 0cfbda9e |
| UL-121: a pre-2007 file is read as UTF-8 when its high bytes happen to be valid UTF-8 | Fixed, 0cfbda9e |
| CD-120 (CD-64): a drawing edited in 0.3.0 cannot be exported in 0.3.1 | Fixed, fc38e785 and a650921f: edits are bound by object ids across reader versions; CHANGELOG note bedab8cc |
| CD-121 (CD-101 rest): a hidden entity (`60 1`) in R12 refuses every edit | Fixed, fc38e785 |
| CD-100 (rest): a whole-turn ARC is written 30/390 after a turn | Fixed, fc38e785 |
| UO-120: crossing vertices depend on the order of the lines | Fixed, 284bebe9: 300 random drawings reversed, flipped and shuffled give the same regions |
| UO-121: the nesting tolerance is ~1e-5 mm at 10 m | Fixed, 284bebe9: tied to `UNDERLAY_JOIN_MM` |
| UO-48: a partition meeting a wall is a corner of the region | Fixed, 284bebe9 |
| UO-49: far longer lines are refused as "more than two million crossings" | Fixed, 284bebe9: their own refusal `TooSpread` |
| UO-51: `pass`, `def`, `None` are accepted as names | Fixed, 284bebe9 |
| UO-52: the cross drawn for a POINT closes off a region | Fixed, 284bebe9 |
| UO-54: lines to leave out are looked up per point | Fixed, 284bebe9: one pass |
| UO-103: arcs written as chords stay polygons | Deferred as in review-2026-11-01-p3-status.md (the review calls the deferral honest) |
| AG-120: `binding_data` 152 lines, `dxf.rs` 4 584, `parse_insert` 220, `parse_entities` 237 | Deferred: a split changes no behaviour and is left to a refactoring branch reviewed on its own. They are above the ~150 lines AGENTS.md asks of new code; `dxf.rs` is below the 5 000-line module cap `check_crate_layers.py` enforces |

## Tests, ratchets, platform and documentation

| Finding | Status |
|---|---|
| TST-120 (TST-101 regression): code under `#[cfg(any(test, X))]` escapes three ratchets | Fixed, f3111acf |
| TST-122: two tests race for one saved file name on Windows | Fixed, 44c12cc2: `persist_noclobber`, a file already there is kept, no panic inside Save |
| TST-121 (TST-100 rest): `check_error_hygiene` misses `Err(_)` arms, `unwrap_or_else(\|_\| ..)`, `let _ = call(..)` | Fixed, 8348ab90, with clippy `map_err_ignore` and `let_underscore_must_use` denied in every crate that holds none |
| CI-121: `third_party_notices.py` checks only registry crates | Fixed, 6d07523d: every crate but the workspace's own, vendored and git crates included |
| SEC-101: network paths on macOS and Linux pass the path check | Fixed, 8acc09d6 and e78d9a2f (macOS, PR #12), 811e1b2a (Linux: autofs, cifs, nfs, sshfs, ...) |
| SEC-121: the `-` check reads the original folder, `names_file` the copy | Fixed, 9e3e4289: both ask the staged copy that runs |
| PL-61: macOS has no memory cap for workers | Fixed, 1d40022c (PR #12, merged after the review's snapshot): the child caps its own address space and the parent samples its physical footprint; the scheduler comment is corrected there |
| TST-60: the memory-limit test was turned off on macOS | Fixed, 1d40022c: the asserts hold on macOS again |
| CI-100: the macOS OCCT cache key lacks the compiler | Fixed, 86ecbb79 (PR #12) |
| PL-62: the macOS `.app` and `.dmg` lack the notices | Fixed, 12cf8a02 and 41e263b1 (PR #12) |
| REL-121: the Android APK and the web Viewer lack the notices | Fixed, 7f6f6b79 |
| REL-80 (rest): `Cargo.toml` still 0.3.1 | Fixed, c680b71e: 0.3.2-dev |
| REL-80 (rest): the 0.3.1 notes say every release's reader is checked | 0.3.1 is checked since b8587564 (PR #13). 0.1.0 is not: it opens the `.recovery` for every format from 104 on, so no writer can keep it safe and the 0.1.0 promise was withdrawn (DOC-100, a5580c70); `release_reads.py` says why. Changing the published 0.3.1 notes waits for the operator's consent |
| DOC-121 (DOC-101 rest): CHANGELOG 0.3.0 still says Apple Silicon; 0.3.1 leaves out fixes | Fixed in the repository, bedab8cc; the published release notes are unchanged and wait for the operator's consent |
| DOC-60: README has no macOS build section | Fixed, 2689eae7 |
| CI-62: macOS lacks clippy `--all-targets` and the tests of the OCCT crates | macOS instance: the crates without OCCT are linted and tested on macOS (matrix job), the app bundle runs the production tests; the OCCT crates need the macOS runner's OCCT build |
| PL-66: Finder sends several files, only the first opens | macOS instance |

## Findings of 2026-11-01 the review confirms fixed

PG-103, PG-104, PG-107, PG-60, PG-63, WB-43, WB-45; UL-4, UL-101, UO-100, UO-101, UO-102, MD-100,
MD-101, CD-106, CD-107; MF-80, MF-81, MF-82, MF-85, MF-101, MF-104 to MF-107; SEC-100, MCP-100,
UI-100, UI-11, EX-101, EX-103, VW-100; REL-100, CI-80, CI-101, DOC-41, TST-44, TST-102, VW-81. The
partly fixed ones are listed above under their new IDs: PG-100 as PG-120, PG-101 as PG-121, MF-103
as MF-121 and MF-122, EX-104 as EX-120.

## Proposals (section 5)

Proposals 5.1 to 5.10 are new features and not part of this branch. 5.1 (explicit bracing) covers PG-124,
5.2 (drawing revision with kept edits) extends CD-120, 5.7 the dialogs of MCP-101 and 5.8 EX-100.
