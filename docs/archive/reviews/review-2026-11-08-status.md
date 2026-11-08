# Review 2026-11-08: status of the findings

Branch `r28-review1108` (from `origin/main` `40e15e55`). Every finding of `review-2026-11-08.md`
(prefix `R1108-`) and every open finding of `review-2026-11-03.md` (prefix `R1103-`, §3.1 of the
11-08 review) is listed here as fixed, with its commits, or as deferred or kept, with the reason.
Each code fix began with a test that failed before it. Both reviews land on `main` with this file
(§0 of the review). AGENTS.md §7 now says how findings are named, so the IDs of reviews on
different branches no longer collide.

## P1

| Finding | Status |
|---|---|
| R1108-PG-240: a lath on the side of a beam just above mid-depth holds its compressed top | Fixed, 62057e94: a brace holds an edge only in the outer quarter of the depth from it, read in the member's own bar frame. 5975df08 adds `brace()` and `diaphragm()` (§5.1), so what the geometry cannot say can be declared |
| R1103-PG-141(b): a rafter extruded at a slope swaps its top and bottom | Fixed, 62057e94: the edge is read in the true bar frame (`LENGTH_AXIS`, else the profile's smallest enclosing rectangle), not along the longest local axis. 6f60b93c (found while closing this review): from about 40°, where the rise and plumb cut outgrow the run, the loads still ran up the extrusion's vertical axis, so nothing braced it. A side view now runs along the profile axis its length follows most nearly level while it bends, and most nearly plumb while it stands (test: 35–60° drawn as box and as extrusion) |
| R1108-PG-241: a joint kind that is not one of 7 English words fastens | Fixed, 1882d995: a closed catalogue of kinds in English, Slovak, Czech and German. `voľne`, `kontakt`, `volně`, `lose`, `frei` and `nichts` fasten nothing; an unknown kind is `joint_kind_unknown`, and every check standing on it is not verified |
| R1108-SEC-241: `--mcp-setup` under `sudo` writes as root through planted links | Fixed, ed5c0e4c: it writes only into a file and folder of the user the configuration is named for; a link to a root file or folder is refused and named |
| R1108-CD-240: Discard of a refused edit revives deleted objects and deletes others | Fixed, b83662c7: the copies of the discarded object are taken out where they are, so objects added after them keep their ids (the KEEP-1/DELETED/KEEP-2 scenario is the test) |
| R1103-MF-140: one part's `.mpr` has no pin holes | Fixed, 5815b7dc: it drills them through the writer the production package uses (`woodwop_part_macros`) |

## P2

| Finding | Status |
|---|---|
| R1108-PG-242: the material and stiffness of a brace are never checked | Fixed, 64b83239: a brace without a timber strength class holds nothing, and the not-verified reason names it; each braced edge reports the force and stiffness its braces must take (EN 1995-1-1 9.2.5.2), and the basis says the braces are not checked |
| R1108-PG-243: the tiny house fails at 30° with snow | Fixed, f0f733b3: loads and grain run along `length_axis`, else the longest axis, so a short gable stud piece is a column; the house passes at 30, 40 and 50° with snow |
| R1108-PG-244 = R1103-PG-143: the cutting plan opens the largest stock size first | Fixed, 559d61ac (b76418d3 for the named-product check): a piece spanning a stock size in both axes opens the smallest such sheet (test: two 2 500 × 1 250 panels on stock 2 500 and 2 800) |
| R1108-MF-240: a round hole cut through bypasses the largest-drill rule | Fixed, 2fe6f1e6: it is milled past the largest drill as a blind one is, and the BTLx support report names the drill and every hole it mills |
| R1108-MF-241: a layered floor gives wrong levels and storeys | Fixed, 7d2f09fc (§5.1): a table of storeys in the drawing settings (`finished_floor_mm`, `structural_mm`, ±0,000 altitude Bpv) cuts the plans and marks the sections; without a table the storeys are found as before |
| R1108-UL-241 = R1103-CD-140: whole arcs are lost both ways | Fixed, cf372186: an ARC of a whole turn reads as a circle, and one drawn over a whole turn is written 0/360 (ezdxf 1.4.4 reads its span as 360) |
| R1108-SEC-240: MCP `save_as` writes any name into any folder | Fixed, 2dfd33f2: only a `.ketchup` document, never a hidden file, and outside the document's folder or one from Open Recent it asks in the window first |
| R1108-UI-240: the Connect tab checks presence on the window's thread | Fixed, d879518e: the client rows are read on a thread of their own, with no stat of a share on the window's thread |
| R1108-CI-250: Windows-only code of `ketchup-mcp` is never linted or tested | Fixed, a2f9f057: the Windows job runs clippy and the tests of `ketchup-mcp`. b1608869 (capability finding 591/592): the Windows job also counts its panics and stderr writes against a Windows baseline, and `check_ci_crates.py` fails on a crate with Windows-only code it does not count |
| R1108-REL-250: 0.3.2 writes woodWOP programs with LF line ends | Fixed: PR #33 merged on `main` (40e15e55); 52f82f32 names the LF line ends of 0.3.2 as a known issue in CHANGELOG, with the way around them. **Not done:** a 0.3.3 release; the owner decides when to publish one |

## P3

| Area | Item | Status |
|---|---|---|
| Program and statics | R1108-PG-245: a misspelled `--set` name gives rc 3 unsaid | Fixed, 5e912003: stderr names it and the parameter it likely means |
| | R1108-PG-246: static FEA compares one isotropic material with one yield strength | Fixed, 35e00e1e: it says so and warns that it does not fit timber; the AI's review lists anisotropy among the excluded physics |
| | Nit: `w=1e-320` is told back in 378 characters | Fixed, 8cd53928 and ec3adcaf: numbers in program messages too small or too large for a decimal are written with an exponent |
| Manufacturing | R1108-MF-242: the BTLx support report is out of date | Fixed, e595192f |
| | R1108-MF-243: 0.3.2 drawing lists are taken by file name | Fixed, 2558af37: a list is the project's only when it names the project where it lies |
| | R1108-MF-244: CHANGELOG says a round hole is a drilling | Fixed, ab7d4e3b |
| | R1108-MF-245: a rounded groove refuses the whole BTLx under saw-then-mill | Fixed, 84f5cd80: a contour with an arc is milled whole |
| | R1108-MF-246: a part with only pin holes is `NothingToMachine` | Fixed, 5815b7dc (with R1103-MF-140) |
| | R1108-MF-247: an `.expect` taken into the error baseline | Fixed, 8a1c3c7b |
| | `BtlxSettings`: two windows overwrite each other, an unreadable value silently becomes the default | Fixed, 8a1c3c7b: each window keeps only what it changed, and an unreadable value is named in the status line. "20,5" was already read as 20.5 mm (r26, `the_options_are_kept_at_one_turn_and_a_decimal_comma_is_read`) |
| | `plan_overlap.rs:46` covers only a full double overlap | Fixed, 409e3f5f |
| | every refusal reports `export_failed` | Fixed, 0f865625: `export_refused` and `export_declined` with their hints; `export_failed` only for a file that could not be written |
| Underlays | R1108-UL-240: 1250 text under a 1252 header becomes "Dåžka" unsaid | Fixed, 2dc02bec |
| | R1108-CD-241: `TextTooLong` names a handle only in the output | Fixed, b83662c7: the text is named, without the DXF 2007 advice |
| | R1108-CD-242: no Discard for `NoHandle` | Fixed, b83662c7 |
| | R1108-CD-243: the 7-byte window cuts a font code in two | Fixed, aaf0323d: pieces are cut before the code, found by walking the codes |
| Security and MCP | R1108-SEC-242: `.ketchup.bak` under `sudo` is root's, through a link | Fixed, ed5c0e4c: a new file with the original's owner and permissions, not written through a link |
| | R1108-SEC-243: home taken from `$HOME` unfiltered | Fixed, ed5c0e4c: a home of `/` or a relative one trusts no missing folder |
| | R1108-MCP-240: a second client silently takes over the window | Fixed, 1177fbda: the user agrees in the window, which names the connected client |
| | `/proc/<pid>/root` in another mount namespace | Fixed, 6254dd4e: a link on a proc file system that reads absolute or names no path makes a path not local |
| | `eprintln!` in `server.rs:492` | Fixed, ddbe98c2 |
| CI, tests, docs | R1108-TST-250: new bypasses of the ratchets | Fixed, 5f2ff90c: clippy counts `unwrap_used`, `expect_used`, `panic` and `print_stderr` per file; b1608869 per system |
| | R1108-TST-251: EMPTY_BLOCK regression | Fixed, d5f5bf71 |
| | R1108-TST-252: short deadlines in asserts | Fixed, 58c931e0: 60 s, returning as soon as the condition holds |
| | R1108-TST-253: frozen modules listed only after the fact | Fixed, a12be19c: a module past 3 000 lines fails the layer check until split or listed |
| | R1108-CI-251: Windows hangs in `production_performance.rs` | Fixed, 9c8cc9b7: Win32 instead of PowerShell and WMI, output to files, the child ended once measured |
| | R1108-DOC-250: Unreleased stale, line 21 contradicts a25f2e40 | Fixed, 52f82f32 |
| | R1108-DOC-251: wrong menu paths in README and START_HERE | Fixed, 6764bad6: every menu path a document names is walked in the real menus, in English and Slovak |
| Release | R1108-REL-251: SHA256SUMS names a file that is no asset | Fixed, 6764bad6: `write-checksums.ps1` refuses a file not beside the sums |
| | R1108-REL-252: the notes say Cancel stops FEA | Fixed, 52f82f32: Cancel stops the meshing, not a running solve |
| Viewer and GUI | R1108-VW-250: Viewer settings saved without `sync`, replacing a link | Fixed, 8644df56 |
| | R1108-UI-250: accessibility of the dock tabs | Fixed, ab34bc98 |

## Findings of 2026-11-03 (§3.1)

Those the 11-03 review lists as fixed are not repeated.

| Finding | Status |
|---|---|
| R1103-PG-141(a): an edge from a notional middle of the contact | Fixed, 62057e94 (`Frame::edge` rewritten with R1108-PG-240) |
| R1103-PG-145: under hogging, the free bottom runs support to support and ignores the point of zero moment | **Kept, on the safe side:** EN 1995-1-1 table 6.1 gives no effective length up to a point of zero moment. Taking that point as a brace would be unsafe without checking a restraint there, so a continuous beam stays not verified rather than wrongly verified |
| R1103-PG-146: rafters steeper than ~72.5° get no edge; `braced_by` lists every board | Fixed, 62057e94: an edge is read in the section plane, with no 0.3 threshold; 6f60b93c tests a board under a rafter at 35–80°. **Kept:** `braced_by` has one entry per bracing part, merged along the member; 36 boards are 36 parts that each hold it |
| R1103-PG-147: no example reaches `lateral_torsional` | Fixed, f0f733b3: the tiny house's steep roof checks it |
| R1103-MF-141, R1103-MF-142 | Fixed, 209bbb0f and 9c67d162 |
| R1103-CD-141: source identity of a drawing | **Deferred** to structural proposal 2 of 11-03 (revision by hash and handle): the changed entities are already listed was/now with counts; storing the source's hash and reader version with the underlay is a change of the document format, not a fix of the export |
| R1103-SEC-141: an overlay on a network layer passes as local | Fixed, 3798b74b |
| R1103-SEC-142: NFC/NFD on macOS | Fixed, 3e73c8a2 |
| R1103-PL-141 and the plugin memory cap | Fixed, dc003692: a plugin has the exact worker's memory cap on every platform, and the worker on Unix dies with its handle |
| R1103-REL-140: the 0.3.1 release text on GitHub | **Not done:** editing a published release is the owner's call. README says that only 0.3.1 needs the Visual C++ Redistributable; 0.3.2 carries the runtime |
| R1103-TST-122 | Fixed, b87893ec |
| R1103-CI-141 | Fixed, a2f9f057 |
| R1103-DOC-60: cargo-about | Fixed, 80a3dee3 |
| R1103-DOC-121: regression | Fixed, 5cfc4a30 |

## Deferrals of 2026-11-07 the review did not accept (§3.2)

| Finding | Status |
|---|---|
| SEC-182: `replace:true` and `take_over` left to the AI | Fixed, a2e0c0f4: they ask the user in the window through `authorize_overwrite` (`HighRiskClass::Overwrite`) |
| Storeys as data | Fixed, 7d2f09fc (R1108-MF-241) |
| ReFS (weak deferral) | Fixed, 501014d1: a file is known by its 128-bit `FileIdInfo`; on NTFS it reads as the index did (tested on NTFS) |
| Version 0.3.2 on `main` after the tag | Fixed, 52f82f32: `0.3.3-dev` |
