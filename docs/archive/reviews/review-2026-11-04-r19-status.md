# Review 2026-11-04, second round: status of the findings

Branch `r22-review1104b` (`origin/main` `3978ad8a` = 0.3.2 + the review, `d5802735`). Every
finding of `review-2026-11-04-r19.md` is listed here as fixed (with its commits) or deferred
(with the reason). Each code fix began with a test that failed before it.

## P1

| Finding | Status |
|---|---|
| PG-160: fastener points anywhere in space brace the member | Fixed, c97eb439: only points in the contact of the two parts hold (`CONTACT_FIT_MM`), and at the anchor only those in its contact; a point away from it is reported (`fastener_off_contact`) |
| PG-161: a lath on both arms of one grounded U braces again | Fixed, c97eb439: anchorage is taken from each contact apart, and the lever must reach beyond the member's side |
| PG-162: any `joint()` fastens | Fixed, c97eb439: kinds `none`, `contact` and `friction` fasten nothing |
| SEC-160: `--mcp-setup` replaces working entries whose command is no full path | Fixed, 0e7df0da: the command is found as the client finds it (PATH, PATHEXT, `~`); one Ketchup cannot tell about counts as existing and is kept |

## P2

| Finding | Status |
|---|---|
| PG-163: grounded is taken as laterally stiff | Fixed, c97eb439: only a declared grounded part anchors, the docs say it must be laterally stiff; a free part standing on the floor and a `fixed` tag no longer anchor |
| PG-164: a common floor is always `not_verified` | Documented, c97eb439: `member_check` and the program docs say a deck fastened only to the members leaves them `not_verified`. **Deferred:** a declared diaphragm anchored at two non-collinear points (structural proposal 1) is new modelling, not a repair; until then the result stays on the safe side |
| SEC-161: the MCP-140 guard needs no consent and does not survive a restart | Fixed, a87a1820: the document keeps the SHA-256 of each drawing file it wrote by canonical path; the refusal tells the AI to ask the user; `schema.rs` corrected |
| MCP-160: a stale FEA result is shown | Fixed, 92fdb454: a run carries the document id and revision, a run or result of a revision the document left is dropped and its worker stopped, Open and New close the dialog |
| MCP-161: unbounded MCP queue, calls run for a client that left | Fixed, d050389a: 16 waiting calls at most; end of input or a failed reply write cancels the running call and drops the queue; `open_window` can be cancelled |
| UL-160: UTF-8 Greek or Cyrillic in a Latin drawing breaks silently | Fixed, 69081b29 and 03872707: a valid 2-byte UTF-8 sequence that a code page reads as a capital with a diacritic and a sign is UTF-8 evidence, and a drawing both readings fit is always doubted |
| UL-161: a deletion switches the encoding on the next import | Fixed, 60dafd0b: the code page actually used is written to `$DWGCODEPAGE` |
| CD-160: a zero import offset is not kept | Fixed, f029b639 |
| CD-161: CD-140 ignores texts and fills | Fixed, 5a849e29: one comparison of layer, lines, texts and fills |
| CD-162: the refusal path is quadratic | Fixed, 799ee4b0: one handle index; `perf_` test refuses as fast as it writes |
| CD-163: the refusal does not say what to do | Fixed, c68efe9d: object, entity, handle, what differs and where; **Tools ▸ Discard the refused drawing edit** drops that one edit. 15a049e3 lists its two new `DxfEditRefusal` variants for the release-reads gate |
| MF-160: ±0 in a georeferenced model is the basement again; ±200 m cap | Fixed, 116013af and 110893d7: no cap, the window shows the found value and starts from it |
| MF-161: level heuristics | Fixed, 69665d04, 9cf75363, 0e40f4e9 and 70be13f0: only drawn bodies and those hidden one by one decide ±0, an unreadable body is an error, bodies indexed per definition and found off the UI thread, touching pieces measured as one, beds and tables no longer marked. **Deferred:** a floor table as data in the sheet settings (structural proposal 3) |
| MF-162: a renamed or moved project refuses its own set | Fixed, 37575bca and 29f9fa86: typed refusal naming the other project, **Take the set over and export**, the list names the file only, a 0.3.2 list is matched by file identity |
| UI-160: views left out by 0.3.2 are shared by 0.3.1 | Fixed, 50cef17f: the old `scenes` list is written beside `excluded_scenes` |
| UI-161: a click on Open Recent still blocks | Fixed, 65a9abb9 |
| UI-162: a damaged settings file is written over | Fixed, 3d662f0e; the Viewer's settings file d36ba7c8 and fa4a3de1 |
| TST-160: the error ratchet is bypassed by a new idiom | Fixed, 3ddff6e9: `dropped(result, Reason)` is the one way, the ratchet counts empty `if … .is_err() {}` and `let (Ok(()) \| Err(..))`, the lines go to a real log file |

## Fixes of PR #34 the review found partly done (section 3)

| Finding | Status |
|---|---|
| PG-141, PG-142, `kind="none"` | See PG-161, PG-160, PG-162 |
| WB-42: `whole=True` with default `3.0` gives a float; `max=` reported after the build | Fixed, 03850087: an override outside the range stops the program at `param()` and the Variants field refuses it; a `whole` param is an int |
| SEC-142 (`--mcp-setup`), MCP-140, MCP-101 (stale result) | See SEC-160, SEC-161, MCP-160 |
| SEC-101: `9p`/`drvfs`, `virtiofs`, `gpfs` missing | Fixed, ca8e8bb7 |
| UL-141, CD-140 (texts and fills), MF-141 | See UL-160, CD-161, MF-160 and MF-161 |
| UO-51: `unwrap_or_default` skips the check | Fixed, 0b971b95 |
| MF-123: ~50 bare `ExportBlocked` | Fixed, 40025291 and 3707f465 |
| UI-141, UI-142, TST-140, 7b1788e3, e4ea558a | See UI-161, UI-162, TST-160 and P3 below |
| UI-101 rest, underlay without texts and fills, dock accessibility | Deferred as before (`review-2026-11-04-status.md`) |

## P3

| Area | Item | Status |
|---|---|---|
| Program | `fixed` counted as fastened to everything | Fixed, c97eb439 |
| | a ceiling also on a wall loses its bottom-edge bracing under a negative moment | **Deferred:** conservative (it can only report more utilisation, never a false pass); it belongs to the explicit bracing path of structural proposal 1 |
| | `stock(along=)` takes any attribute name | Fixed, c97eb439: documented as the declared direction attribute |
| | a pad narrower than high carries load on its side face | Fixed, c97eb439: reported as `area_load_edge_on` |
| AI connection | Connect reports success for disabled, `enabled = false`, `http`/`sse`/`url` entries | Fixed, 0e7df0da: shown as starting nothing, Connect turns it on |
| | dangling symlink replaced, Windows rewrite in place, owner lost under `sudo`, nested error positions | Fixed, 0e7df0da |
| DXF | votes per occurrence | Fixed, 40ecc90f |
| | `\|c238` and STYLE font charset ignored | Fixed, 3d17e483 (with 05c52b4e) |
| | a short drawing with one Czech word stays 1252 without an offer to read it as 1250 | Partly: with 3d17e483 a charset-238 font turns it, and the doubt is warned of. **Deferred:** an offer to read an imported drawing again in another code page needs the decision stored with the underlay and a re-import action (structural proposal 2) |
| | an entity deleted in AutoCAD and not edited refuses the export; `changed_in_file` counts reader differences | Fixed, c61ac188 |
| | UO-51 | Fixed, 0b971b95 |
| Drawings and manufacturing | 900 mm landing not marked | Fixed, 0e40f4e9 |
| | `today()` writes d. m. yyyy on English sheets | Fixed, cd0580d2 |
| | plan DXF writes a point; the separator goes three ways | Fixed, f21fe83b |
| | worktop opening bounds and round hole as a two-arc through cut | Fixed, ddaaa5be |
| | ±0 cannot be set over MCP | Fixed, 5e8a77d7 |
| | ~50 bare `ExportBlocked`; `NoSupportedGeometry` text false | Fixed, 40025291 and 3707f465 |
| GUI, release and CI | New does not keep the theme | Fixed, 0f404401 |
| | settings file: fsync, permissions, two windows, symlink, `.part`, a locally deleted folder Unavailable for ever | Fixed, 9edcf798 and f3f48fa2 |
| | Program page lays out every line while the dock edge is dragged; a screen reader sees only visible lines | Fixed, 4538d136 |
| | TST-140 changed behaviour without tests | Fixed, 2f18be0a |
| | REL-143 compares only the current snapshot, `extensions` as a `Vec` | Fixed, 49df8258 |
| | REL-142 accepts a catalog signature | Fixed, e801c028 |
| | e4ea558a: `-rc.1`, `-rc.2` and `-dev` share a `versionCode` | Fixed, 410ba210 |
| | 7b1788e3 finds only helpers of the same file with a `Duration` parameter | Fixed, b2f32ad2 |

## Structural proposals

1. Bracing path as an explicit model: the points, anchors and kinds are done (c97eb439). A
   declared diaphragm and an explicit path object are not added; members braced only by a
   deck stay `not_verified`, on the safe side.
2. Keep at import what the export must repeat: the offset is always kept (f029b639), one
   object comparison serves bindings, counts and refusals (5a849e29, c68efe9d), one handle
   index (799ee4b0). The decided encoding is not stored with the underlay yet; the written
   code page follows the one used (60dafd0b).
3. Floors as data: not done; the found ±0,000 is shown and can be set (116013af, 5e8a77d7).
4. One output register in the document: done for PDF, sheets and DXF/DWG sets (a87a1820,
   37575bca).
5. One model of an AI client registration: done (0e7df0da).
6. `WindowSettings` apart from the document: done (0f404401). The app and the Viewer keep two
   copies of the same settings-file rules (3d662f0e, 9edcf798, d36ba7c8), as the Viewer does
   not depend on the app; one shared module needs a crate of its own. `DocumentSession` is
   not split out.
7. Dropping an error as a typed, counted operation with a real log: done (3ddff6e9).
