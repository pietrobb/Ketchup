# Review 2026-11-06, second round: status of the findings

Branch `r25-review1106b` (`origin/main` `b867c085` = PR #38 + the review, `d878bd32`). Every
finding of `review-2026-11-06-r23.md` is listed here as fixed (with its commits) or deferred
(with the reason). Each code fix began with a test that failed before it. SEC-5 is the
owner's decision and is not taken up here.

## P1

None were reported.

## P2

| Finding | Status |
|---|---|
| UL-200: one character of three bytes (EUR sign, en dash) turns the `\U+XXXX` escaping off | Fixed, c2b67f1e: a drawing read as UTF-8 against a code page header before AC1021 is written with everything beyond ASCII escaped, whichever characters it holds (test with "€ 120", "Ø 20 – 30", "Δh", "Кухня" after a deletion). f06c0259 (capability review runs 534/535): block names, attribute tags and values, hatch patterns and dimension texts read `\U+XXXX` back as their characters, so a drawing saved twice reads back alike |
| SEC-200: the owner fix under `sudo` broke writing for a plain user (EPERM) | Fixed, 857b9503: a new file is chowned only when `geteuid() == 0`; folders use `lchown`; folders made for a write that failed are removed |
| SEC-201: a drive or share that is out counts as gone and the installer replaces its entry | Fixed, 0ccb163c: one `presence(path)` of Here, Gone or Unknown walks to the nearest existing folder and says Unknown for a drive, share or volume that does not answer (Windows `GetDriveTypeW`/`GetVolumeInformationW`); Open Recent, the installer and the written drawings use it |
| SEC-202: cleaning the register freezes the window and forgets offline drawings | Fixed, 0ccb163c: an export replaces only this document's lines; its gone drawings are forgotten on a worker; lines of other documents, offline or unreadable, are kept |
| MF-200: "under it in plan" measured by bounding boxes | Fixed, 59903719: measured on the triangles of the top faces, against the marked level under most of the level's area (L gallery around a stair, a landing under the edge) |
| MF-201: a copy saved as another file of the same name takes the original's drawing set | Fixed, ec2de414: a set's list names the token of the project's file, not the document id a copy shares |
| CD-200: an object moved, then deleted in Kečup and in the file is refused | Fixed, 74c4472b: removal wins over a move |
| CD-201: the `Several` message has no length limit | Fixed, 74c4472b: one helper (`ketchup_rejection::write_listed`) names the first 5 and counts the rest, for the status line and MCP alike; the whole list stays in `refused_edits()` |
| CD-202: a fixed 1e-7 mm tolerance refuses geodetic drawings | Fixed, 74c4472b: `ketchup_tolerance::same_coordinate_mm` compares within the linear tolerance or 1e-12 of the value's size |

## P3

| Area | Item | Status |
|---|---|---|
| Program | `anchorage_small` depends on declaration order | Fixed, bb7e369b: the largest anchorage of the point is taken |
| | a long thin member 5 900 × 10 × 10 is not reported | Fixed, bb7e369b: the size across the member is compared with its height |
| | kind slips report "cone", "node"; "unfastened", "loose", "free", "rests" fasten | Fixed, bb7e369b: words under five letters are no slips; those four fasten nothing |
| | a fraction for a `whole` param is a hard error, an override out of range a warning | Fixed, bb7e369b: both are refused alike, the model builds with the default and reports it |
| | MCP `program apply`/`check` store overrides without the range check | Fixed, bb7e369b: overrides the program refuses are refused |
| | a bad `--set` prints the whole usage | Fixed, bb7e369b |
| | joint points are not checked when the contact makes no bearing | Fixed, bb7e369b |
| AI connection and MCP | cancelled queued calls still take the queue | Fixed, 4808d3e2: a cancelled queued call frees its place |
| | a copy in a file manager shares the token (SEC-203) | Fixed, ec2de414: a written drawing's line keeps the identity of the document's file (inode, or volume serial and file index); a copy gets a token of its own on its first save |
| | a register line is taken by its path though its token differs (`Owner::holds`) | Fixed, ec2de414 |
| | Save As sets the token before the save succeeds | Fixed, ec2de414 |
| | `chown` under root follows symlinks | Fixed, 857b9503 (`lchown`) |
| | `headersHelper`, `scopes`, `oauth_resource` stay on the converted entry | Fixed, 4808d3e2 |
| DXF | one 1250 layer name has two votes and outvotes the texts (UL-201) | Fixed, c2b67f1e: it votes twice only where no text reads as words in both 1252 and 1250 |
| | characters beyond the BMP are written as a surrogate pair | **Not changed:** `\U+` carries four hex digits, so such a character has no single code; Kečup reads the pair back as one character (c2b67f1e, f06c0259). Whether AutoCAD shows it stays unverified (S in the review) |
| | block names are not decoded in diagnostics | Fixed, c2b67f1e and f06c0259 |
| | a doubtful reading can flip 1252 → UTF-8 without a warning | Fixed, c2b67f1e: a drawing read in the code page that is valid UTF-8 too is escaped as well, so what is left after an edit cannot flip |
| Drawings and manufacturing | ±0 takes the terrace −150 under a floor +450 (MF-203) | Fixed, 59903719: the lowest floor at or above z = 0 by less than a storey comes first |
| | two tables beside the floor get "+0,740" | Fixed, 59903719 |
| | the app never sets `largest_drill_mm` (MF-202) | Fixed, 59903719: the File menu sets the largest drill, 35 mm by default |
| | woodWOP refuses a round opening as "drillings and vertical pockets only" | Fixed, 59903719: the message names a through opening |
| | a dimension with a decimal comma is fixed text (`DIMDSEP`) | **Deferred:** R12 has no `DIMDSEP`, and carrying it into DWG needs a check in a real CAD reader |
| | the takeover question names only a count and says "1 sheets" | Fixed, 59903719: it names the first five sheets in the plural of the language |
| | `ValueOutOfRange` does not name the part | Fixed, 59903719 |
| GUI, release and CI | opening a file for its migration review does not raise the generation | Fixed, 4808d3e2 |
| | log rotation from two windows loses the log or writes to `.old` | Fixed, df39c677: the file's own length says when it is full; a window whose log another moved aside writes to the new one; a log moved aside by the other first is no error |
| | the error ratchet misses `.unwrap_or(())`, `.err();`, `mem::forget(call())`, `unwrap_or_else(Default::default)`, `map_or((), …)`; `gpu.rs:130`; false `drop(found.remove(oldest))` | Fixed, d645a06e: the five forms are counted; `gpu.rs` drops its send through `dropped()`; the regions cache keeps its newest by `retain`, so the false entry left the baseline |
| | `window_eprintlns` skips `main.rs` helpers called after `log_window_to` | Fixed, d645a06e |
| | 5afe72b2 passes when some release restores the history | Fixed, d645a06e: every release read must restore it |
| | the settings lock timeout leaves a thread and an open file | Fixed, 74c27bcd: a save tries the lock without waiting, so one that gives up leaves nothing behind; it gives up only on a window that holds the lock two seconds without saving |

## Structural proposals (section 6)

| Proposal | Status |
|---|---|
| 1. DXF encoding decided once and kept | Done for writing: the escaping is decided by how the file reads against its header, not by its characters (c2b67f1e). **Deferred:** keeping the decision with the underlay (r19 proposal 2); it is worked out again from the source file on save |
| 2. One identity of the project file for sheets and sets | Done, ec2de414 |
| 3. No disk on the UI thread, Unknown as a third state | Done, 0ccb163c |
| 4. Tolerances relative to size | Done for DXF coordinates, 74c4472b |
| 5. Length limits for user and MCP texts | Done, 74c4472b (`write_listed`) |
| 6. Floors as data | **Deferred** as in r19 and r23; the heuristic was repaired (59903719) |

## Carried over

- MCP-160 (a cancellable FEA solver) stays deferred as in `review-2026-11-06-status.md`.
- `replace: true` stays a flag the client sets (SEC-182).
