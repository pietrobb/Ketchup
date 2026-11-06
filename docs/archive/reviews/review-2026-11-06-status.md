# Review 2026-11-06: status of the findings

Branch `r23-review1106` (`origin/main` `b79afb18` = 0.3.2 + the review, `e9b106fd`). Every
finding of `review-2026-11-06.md` is listed here as fixed (with its commits) or deferred
(with the reason). Each code fix began with a test that failed before it. SEC-5 is the
owner's decision and is not taken up here.

## P1

| Finding | Status |
|---|---|
| PG-180: `bearing=True` bypasses the PG-160 check | Fixed, 847025b6: the points of a joint are no contact of their own; a point holds only over a real contact face of the two parts, and a bearing joint with no face contact carries load but braces nothing. Gathering the supports was taken out of `loads_with` first |

## P2

| Finding | Status |
|---|---|
| PG-181: false `fastener_off_contact` on correct points | Fixed, 847025b6: a point is decided once for the pair (member, other part) against all of its contacts together; the text names the part by its role (`member_design.rs`, the beam on one sill with two end blocks) |
| PG-182: NaN passes `param()` | Fixed, 9c24d54e: `param()` and `--set` refuse a value that is not finite by the parameter's name |
| MCP-180: a full queue stops reading | Fixed, a72ada62 and 0ae54f4a: a call beyond the queue is answered `busy` at once and the reader goes on, so cancellation and the end of input are still seen |
| SEC-180: the installer replaces an entry it could not check | Fixed, a87ff4c0 and 84f2830f: a program is gone only where the system says there is no such file; an entry turned off ranks with a gone one; each gone entry is replaced on its own, and Connect replaces a project's entry only when it is gone |
| SEC-181: Connect keeps the network fields of an `http`/`sse` entry | Fixed, a87ff4c0: address, token, headers and sign-in are dropped |
| SEC-182: the SHA-256 register of drawings | Fixed, a25f2e40, d2dbada4 and 1cf2d340 (the live test expects an export to leave the document unchanged): the register is kept with the user's settings, so an export leaves a saved document saved and a declined save loses nothing; a file that is gone leaves the list; a moved document keeps its drawings by a token kept in its file and made anew on Save As and on a template's first save, so a copy asks before replacing the original's drawing even once the original is renamed (capability review run 529). **Not changed:** `replace: true` is still a flag the client sets; the refusal tells the AI to ask the user first. A consent given in the window rather than by the client is a change of the AI consent model, not a repair |
| UL-180: a deletion still switches the encoding for UTF-8 | Fixed, a0c473ff and b2b14c6c: where UTF-8 decided over the header, text beyond ASCII is written as `\U+XXXX` |
| CD-180: an object deleted in Kečup and in the file is refused | Fixed, a0c473ff |
| MF-180: a gallery loses its level mark once a landing is marked | Fixed, 216e8675: a raised level is measured against the highest marked level under it in plan |
| MF-181: a large round cut-out became a full-diameter drilling | Fixed, 9014f868 and f97f0907: a round opening is its two half arcs cut to the part's depth, drilled only where the largest drill a format is set up with is as large (BTLx `largest_drill_mm`); under the app's default BTLx strategy it is milled along the arcs. Note: BTLx writes timber parts only, so a worktop modelled as a manufactured item is not in BTLx at all |
| MF-182: a project moved with its drawings folder refuses its own set | Fixed, 216e8675 |
| UI-180: a late answer of Open Recent opens a document nobody wants | Fixed, 8434dbc7: New, Open and a client's open cancel the pick |
| UI-181: window start errors are lost in a Windows release | Fixed, 8434dbc7: once a window logs to a file its start failures go to the log, and the `eprintln` ratchet counts what an entry point writes after `log_window_to` |

## Fixes of PR #37 the review found partly done (section 3)

| Finding | Status |
|---|---|
| PG-160 (`bearing=True`, false `fastener_off_contact`), WB-42 (NaN) | See PG-180, PG-181, PG-182 |
| MCP-161, SEC-161, UL-161, c61ac188, MF-161, MF-162, ddaaa5be, UI-161, TST-160 | See MCP-180, SEC-182, UL-180, CD-180, MF-180, MF-182, MF-181, UI-180, UI-181 |
| MCP-160: `model.solve` cannot be cancelled; a dropped run keeps computing and another can start | **Deferred:** the review gives it no new ID and no place in its order; a cancellable solver is new work in the FEA worker, not a repair. The stale result itself is still dropped (MCP-160 holds there) |

## P3

| Area | Item | Status |
|---|---|---|
| Program | points checked in plan only, a point 2.4 m below the contact anchors | Fixed, 847025b6: a point must lie within the part's thickness of the contact plane |
| | `continue` skips the member side's report | Fixed, 847025b6 |
| | `fastens()` is a deny list | Fixed, 847025b6 (an empty kind) and 3900f25e (a kind that reads like a slip of `none`/`contact`/`friction` is reported) |
| | a tiny grounded block braces the member | Fixed, 3900f25e: still braces, but is reported |
| | `min=5, max=1` is not refused | Fixed, 9c24d54e |
| | a stored override outside a narrowed range stops the program | Fixed, 9c24d54e: the program builds with the default and the model reports it |
| AI connection and MCP | a new file written under `sudo` belongs to root | Fixed, 98372196 |
| | `ReplaceFileW` 1176 may lose the settings and the staged copy | Fixed, 98372196 |
| | `vboxsf` and `prl_fs` missing | Fixed, 98372196 |
| | `export_sheet` hashes the target on the UI thread | Fixed, a25f2e40: hashing, drawing and writing run on a job |
| | `zero_level_mm: "auto"` does not say which height it used | Fixed, a25f2e40: `zero_level_used_mm` |
| DXF | CD-161 compares `f64` exactly | Fixed, a0c473ff and b2b14c6c: compared within the drawing tolerance, the place reported apart from the content |
| | grammar "its lines reads otherwise" | Fixed, a0c473ff |
| | `refused_edit` survives New and Open | Fixed, a0c473ff: a refused edit goes with its document |
| | only the first refused object is reported | Fixed, a0c473ff |
| | an R12 refusal names another entity's place | Fixed, a0c473ff |
| | layer names do not vote for the code page | Fixed, a0c473ff |
| Drawings and manufacturing | planks 5 mm apart are not merged | Fixed, 216e8675 |
| | ±0 has no tolerance around z = 0 | Fixed, 216e8675 |
| | a dimension with a decimal comma goes back to a point | Fixed, 216e8675: the value is the dimension's text |
| | `same_file` and `found_zero` on the UI thread | Fixed, a25f2e40 and 216e8675 |
| | `project_drawings.rs` drops errors with `unwrap_or_default()` | Fixed, 9014f868 |
| | `ValueOutOfRange` names neither value nor limit; the barcode is not tied to the part | Fixed, 9014f868 |
| | taking a set over deletes sheets without warning | Fixed, 216e8675: the question says which sheets go |
| | unticking ±0 before the value is found starts from 0 | Fixed, 216e8675 |
| GUI, release and CI | the log: 0644, rotated only on open, the window does not say where | Fixed, 2bdc42b1 and 29e99d2a (a log folder that already exists is closed to its user too; CI on macOS and Linux) |
| | `check_error_hygiene.py` misses idioms | Fixed, 92724bfa |
| | settings `lock()` without timeout, NFS without lockd, `fchown` EPERM, the Viewer's fixed `.part` | Fixed, 0297532a |
| | `save_again_with_history` skips the comparison | Fixed, 5afe72b2 |
| | an sshfs share not mounted drops recent documents | Fixed, 0297532a |
| | PowerShell `-match` ignores case for `-RC.1` | Fixed, 7907f24b |

## Structural proposals (section 6)

| Proposal | Status |
|---|---|
| 1. Contact as the single source of where parts meet | Done for PG-180/PG-181 and the depth check (847025b6) |
| 2. MCP back-pressure without blocking the reader | Done, a72ada62 |
| 3. Selection and document state as a generation | Done for Open Recent (8434dbc7) and the refused DXF edit (a0c473ff); SEC-182 keys the register by the document's file and its token instead; FEA as in MCP-160 above |
| 4. Unambiguous DXF output for UTF-8 against the header | Done for writing (a0c473ff). **Deferred:** keeping the decided encoding with the underlay (r19 proposal 2) |
| 5. Manufacturing by tool, not by shape | Done, 9014f868 |
| 6. Floors as data | **Deferred** as in r19 (proposal 3); the heuristics were repaired (216e8675) |
