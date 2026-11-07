# Review 2026-11-07 r26: status of the findings

Branch `r27-review1107b` (`origin/main` `8d781633`). Every finding of `review-2026-11-07-r26.md`
(on `claude/adoring-carson-6poeki`) is listed here as fixed (with its commits) or deferred (with
the reason). Each code fix began with a test that failed before it.

## P1

None were reported.

## P2

| Finding | Status |
|---|---|
| TST-240: the flaky component-replacement test (issue #29) reads the digest before the worker answers | Fixed, f23b50bb: the window asks for no frame while a history change's bodies are evaluated, so the harness took it for idle. `run_idle`, which every click runs, now also waits for that evaluation and fails naming the digest when it does not end; the two `confirm` helpers lose their own unchecked wait. Eight parallel copies of the test with every core loaded: 31 of 32 failed with "Computing the exact bodies of the change..." before, 32 of 32 passed after; all 539 `ketchup-app` integration tests pass. Structural proposal 3 is met in the harness, where every click passes, instead of by a ratchet over `action_digest()` reads: a prototype of that ratchet flagged 73 reads, nearly all after work done on the UI thread |
| MF-240: a solid's winding is summed from world coordinates | Fixed, d1184abd: `winds_inward` sums the volume from a corner of the solid and `polygon_area` the area from the polygon's first corner, so neither loses its sign or precision far from the origin. Test: a room and 400 floor tiles moved to (−1.2e9, −5e8) and (5e9, 5e9) mm wind outward, and every view has the levels, cut solids and extents it has at the origin |
| UL-240: a text drawn in Kečup is always written as TEXT and refused under a code page | Fixed, 77316050: the writer counts a drawn text as it will be read (escaped beyond ASCII where the drawing is) and, past the 2 049 characters of a line, writes an MTEXT instead, with its backslashes, braces and `^` escaped and, before DXF 2007, `\U+XXXX` for what is beyond ASCII, so the line limit cuts it into pieces. Only a DXF 12 drawing, which has no MTEXT, refuses it, naming the text's start instead of a handle Kečup gave it. Test: 400 Cyrillic characters with `{\P}`, turned 30°, in R2000 under ANSI_1252 and with no code page, read back the same through two saves; 1 500 in DXF 2007 stay a TEXT. Before the fix the test fails on the refusal. Structural proposal 2 is met in the writer. Deferred: the warning at placement, as the window holds neither the drawing's version nor its bytes, and only DXF 12 is left to refuse |

## P3

| Area | Item | Status |
|---|---|---|
| Drawings | `polygon_area` from absolute coordinates | Fixed with MF-240, d1184abd |
| Statics and program | a grounded rod turned over about 3.8° counts as a full anchorage again | Fixed, ab647343: a side of the anchorage's box within acos 0.9 of the member's axis is its length and is left out of its depth. Test: the rod 5 900 × 10 × 10 turned 4.2° and 20° is reported `anchorage_small` (failed before); the 45° slab, and the tiny house in release with the ignored tests, are unchanged |
| | "or widen the range" does not help an override that is no whole number | Fixed, 1a9dc52a: the hint names the range and the whole number in it nearest the value ("Set a whole number inside [1, 4], as 4.") |
| DXF | the `drawing_text_too_long` advice ignores the cause | Fixed, 2a425876: the refusal says how the drawing writes the text (as read, in its code page, or escaped against a header naming another) and suggests DXF 2007 only where escaping is the cause; the catalog entry points to `caused_by` for the cause (test: the same Cyrillic texts write as read with no code page, refuse as escaped under ANSI_1252, and the as-read wording has no DXF 2007 or code page) |
| | names outside the BMP and unreadable blocks make a drawing unwritable | Fixed, 2a425876: a line other than a text that escaping would push past 2 049 characters keeps its bytes as read (test: a layer of 150 „𝔸“ and a block of 300 Cyrillic characters under ANSI_1252 read the same through two saves; it fails without the fix) |
| | the `mtext_pieces` comment promises more than it does | Fixed, 2a425876: it says only a `\U+XXXX` is never cut, and a longer format code may be, as readers join the pieces first |
| | several texts too long need several exports | Fixed, 2a425876: every text too long is refused at once in one `Several`, coded `drawing_text_too_long` (test: two too-long TEXTs give both; it fails when only the first is returned). ed98d53f: `release_reads.py` lists `DrawnTextTooLong`, which UL-240 added without |
| File presence and settings | the "below home" check compares path text | Fixed, 57ca2d28: the nearest existing folder and the trusted folders are compared as the system resolves them, so `~/NAS` linked to gvfs is not below home and a file below macOS `/tmp` is below `/private/tmp` (test `folders_are_compared_as_the_system_resolves_them`: a file under a home link to an unmounted share is Unknown, one deleted below a trusted folder named by a link is Gone; it fails without the fix) |
| | `/opt` and `/usr` from autofs or a network share (S) | Fixed, 57ca2d28: on Linux a folder whose deepest mount in `/proc/self/mountinfo` is autofs or a network file system does not answer, so it gives Unknown, not Gone (test `a_folder_on_a_server_or_mounted_on_demand_is_remote`: `/opt` from autofs, an nfs4 share with an escaped space in its mount point and fuse.sshfs are remote; a local `/home` mounted later over autofs and `/usr` on the root are not) |
| | a link to a file deleted in home stays Unknown | Fixed, 57ca2d28: a dangling link is judged by its target, followed through at most 40 links (test `a_dangling_link_to_a_file_is_as_its_target`: Gone when its target was deleted in home, Unknown on a volume not mounted; it fails without the fix) |
| | `forget_gone` counts Gone before taking the lock | Fixed, 57ca2d28: a drawing found gone before the lock is forgotten only if it is still missing under it (test `a_drawing_written_again_before_the_lock_keeps_its_line`; it fails without the fix) |
| | a failed write of `settings.txt.turn` stops the whole save | Fixed, 57ca2d28: the turn only tells other windows to wait, so a failed write of it is logged and the settings are saved (test `a_turn_not_counted_does_not_hold_the_save`; it fails without the fix) |
| Drawings and BTLx | one far triangle collapses the level grid | Open (R7) |
| | a fan of triangles is slower in the grid than by brute force | Open (R7) |
| | BTLx save runs `replace` three times on the UI thread; "20,5" falls back to 35 mm | Open (R7) |
| | a copy of a 0.3.2 file takes over the 0.3.2 list by name | Open (R7) |
| Ratchets and tests | idioms still unseen, `EXPECT` in comments, `EMPTY_IS_ERR` on `if {} else`, `window_eprintlns` keys | Open (R8) |
| | test 48a1bc40 checks inner structure | Open (R8) |
| | `project_drawings.rs` and `dxf/rewrite.rs` over 3 000 lines | Open (R8) |
| README | four inaccuracies | Open (R8) |
