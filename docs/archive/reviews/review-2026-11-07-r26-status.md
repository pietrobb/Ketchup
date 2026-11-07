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
| DXF | the `drawing_text_too_long` advice ignores the cause | Open (R5) |
| | names outside the BMP and unreadable blocks make a drawing unwritable | Open (R5) |
| | the `mtext_pieces` comment promises more than it does | Open (R5) |
| | several texts too long need several exports | Open (R5) |
| File presence and settings | the "below home" check compares path text | Open (R6) |
| | `/opt` and `/usr` from autofs or a network share (S) | Open (R6) |
| | a link to a file deleted in home stays Unknown | Open (R6) |
| | `forget_gone` counts Gone before taking the lock | Open (R6) |
| | a failed write of `settings.txt.turn` stops the whole save | Open (R6) |
| Drawings and BTLx | one far triangle collapses the level grid | Open (R7) |
| | a fan of triangles is slower in the grid than by brute force | Open (R7) |
| | BTLx save runs `replace` three times on the UI thread; "20,5" falls back to 35 mm | Open (R7) |
| | a copy of a 0.3.2 file takes over the 0.3.2 list by name | Open (R7) |
| Ratchets and tests | idioms still unseen, `EXPECT` in comments, `EMPTY_IS_ERR` on `if {} else`, `window_eprintlns` keys | Open (R8) |
| | test 48a1bc40 checks inner structure | Open (R8) |
| | `project_drawings.rs` and `dxf/rewrite.rs` over 3 000 lines | Open (R8) |
| README | four inaccuracies | Open (R8) |
