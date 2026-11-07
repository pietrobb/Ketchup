# Review 2026-11-07: status of the findings

Branch `r26-review1107` (`origin/main` `ca8bc99f`). Every finding of `review-2026-11-07.md` is
listed here as fixed (with its commits) or deferred (with the reason). Each code fix began with
a test that failed before it.

## P1

None were reported.

## P2

| Finding | Status |
|---|---|
| UL-220: escaping can make a line longer than the reader allows | Fixed, 02b02426: a long MTEXT is written in pieces of group 3 under 250 characters, cut before an escape, as AutoCAD writes one; any other text longer than the 2 049 a DXF line holds is refused with `drawing_text_too_long`, naming its entity (test: 2 000 Cyrillic characters written and read back). 3a7d7a3a sorts the new code in the catalog; 67323810 lists the new refusal in `release_reads.py`, as it is never saved in a document. The capability review found the check ran only where the whole file is escaped, while a text drawn into a drawing read in its code page escapes too: 29c9830c holds every drawing written to the limit, counted in characters as it is read, so 300 Cyrillic characters drawn into an ANSI_1252 drawing are refused and 1 500 in a DXF 2007 still read back |
| UL-221: block names and attribute tags measured escaped | Fixed, 02b02426: names of layers and blocks and tags of attributes are measured in characters as read (`chars().count() > 255`); a 43-character Cyrillic block name and tag stay readable through two saves |
| UI-220: a dangling link to a folder counts as gone | Fixed, 966b475e: a file below a dangling link (or a junction to a share that is out) is Unknown, so Open Recent and `--mcp-setup` keep it (tests with a dangling symlink and a junction made with `mklink /J`). 158f3a14 (capability finding 546 on 60471ae5): a folder missing outside the home folder is Unknown again for Open Recent and `drawings.txt` (test: a gvfs share not mounted outside home) |

## P3

| Area | Item | Status |
|---|---|---|
| Program | the five-letter limit is applied to the kind written: `lose`, `rest`, `frei` fasten unsaid | Fixed, 0f2a5809: a kind under five letters is looked at for slips of every kind but "none" |
| | an override out of range and no whole number gets only the whole-number message | Fixed, fbe147aa: it is told both, and to set a whole number inside the range |
| | the rule for pairs without a bearing (7222050e) does not look at the contact | Fixed, 10536501: the fastener passes only over where the parts touch, as one of a looked-at pair |
| | `depth()` leaves out the axes of the box under 45° to the member | Fixed, a537fad1: an anchorage is measured across the member along the two directions square to it |
| File presence, MCP and settings | a program in a folder deleted outside home is Unknown, so `--mcp-setup` never replaces its entry | Fixed, 60471ae5 and 158f3a14: only `--mcp-setup` also trusts the folders programs are installed in (`/opt`, `/usr`, `/Applications`, `/tmp` for an AppImage's `/tmp/.mount_*`). **Not changed:** asking once on Unknown instead (structural proposal 2) |
| | a client reusing the id of a running or refused call loses a waiting call | Fixed, 48a1bc40: calls are known by the number of their turn; a cancel of an id stops every call under it, as the server cannot tell which the client meant |
| | `written_owner` resolves the file on the UI thread on every `written_drawings()` | Fixed, ea6640c0: the document's file is resolved once per file opened or saved |
| | lines of deleted documents stay in `drawings.txt` | Fixed, ea6640c0: a deleted document's gone drawings leave it |
| | "still saving" by mtime and length fails on 1–2 s mtime file systems; windows saving over and over hold the UI thread without a limit | Fixed, cf96b2e3: a window waits while others take turns, counted beside the lock, and 10 s at most in all |
| | `create_dir_all` failing midway leaves the folders it made | Fixed, bf937821 |
| | a file ID on ReFS or Dev Drive is not guaranteed unique (S) | **Deferred:** the 64-bit index of `GetFileInformationByHandle` would have to give way to the 128-bit `FILE_ID_INFO`; there is no ReFS or Dev Drive volume here to make the failing test on |
| DXF | a UTF-8 file under ANSI_1252 with Western characters only is written as `Caf\U+00E9` (S) | **Deferred** to structural proposal 1: writing such a file in its header's code page is a second escaping decision beside the one c2b67f1e made by how the file reads, the class UL-18x to UL-22x came from; it belongs in the one DXF value writer, with a check in a simple importer |
| | `dxf.block-unreadable` shows the undecoded name | Fixed, 02b02426 |
| | "and K more" of `write_listed` is fixed English in the Slovak and Czech digest | **Deferred:** the whole refusal it ends is English (`DxfEditRefusal`'s `Display`, passed to `digest-underlay-export-failed` as `reason`); translating those few words alone makes a sentence of two languages. It goes with moving the refusal texts into the catalog |
| | "C:\Ü" is written as `C:\\U+00DC`, which MTEXT-style readers show as "C:\U+00DC" | **Not changed, unverified:** in a TEXT the backslash is no escape, so AutoCAD reads `C:\` and `\U+00DC`; in an MTEXT a backslash of the text is already `\\` in the file. Which readers the review means and what AutoCAD does with a TEXT written `C:\\\U+00DC` needs a real CAD reader, not available here |
| Drawings and manufacturing | a table 800 mm over the edge of a gallery, a bed over the stair opening, are marked as levels | Fixed, 393e6de6: a top is measured against the highest marked level under at least a quarter of it |
| | a floor laid twice counts twice in `area_over` | Fixed, 393e6de6: each triangle is counted once, at most its own area |
| | a platform outside the plan is compared with a landing elsewhere (`or_else(marked.last())`) | Fixed, 393e6de6: with no level under it, it is measured against the highest floor below |
| | the overlap of triangles is quadratic and cannot be stopped; `section_levels` ignores `cancelled` | Fixed, 393e6de6: a grid of the triangles' boxes; the overlap stops when the set is cancelled (tests: tiled levels overlapped and stopped, and a timed one under a second) |
| | a file without a token and its copy take a set by name | Fixed, 35247f0d |
| | "0: mill every hole" is wrong for declared drillings | Fixed, 0e402103 |
| | `BtlxSettings` are not kept | Fixed, 0e402103: kept between starts in `btlx.txt` and through File > New |
| | `drills` compares without a tolerance | Fixed, d86fa3d9 |
| | an empty sheet list says "It has 0 sheets, :" | Fixed, fa16daef |
| | sheet names from a list file on a share reach MCP without a length limit | Fixed, fa16daef: cut to 100 characters |
| Ratchets | `OR_DEFAULT` misses `String::default`, `<_>::default`, `unwrap_or_else(drop)`, `map_or_else(\|_\| (), …)`, `save(p).is_ok();`, `if let Ok(()) = save(p) {}` | Fixed, 3cf8fba4: all six are counted; the one found in production (`plugin_menu.rs`) names the two length refusals it sends on as text |
| | `window_eprintlns` misses a helper passed by name, `self.fail()`, helpers of another module; `block_end` reads braces in strings; `log_window_to` in an `if` ends at the block | Fixed, 3cf8fba4, but for helpers of another module: **deferred** to structural proposal 4, as following a call into another file needs the module tree |
| | `expect(` is not ratcheted | Decided, 3cf8fba4: AGENTS.md §2 says new production code has no `.expect(` either; the 528 older ones are counted per file and may only go down |
| | `fabrication.rs` and `import/dxf.rs` grow past 3 000 lines | Fixed, 3cf8fba4: both are in `FROZEN_MODULES` (4 787 and 4 599 lines) and named in AGENTS.md §6 |

## Structural proposals (section 6)

| Proposal | Status |
|---|---|
| 1. DXF writing with the format's limits | Done for line length, MTEXT pieces and name lengths (02b02426). **Deferred:** one value writer deciding the escaping, and the generated round trip over lengths 1–3 000 |
| 2. `presence()` three-state throughout | Done for links and junctions (966b475e) and for the folders a program is installed in (158f3a14). **Deferred:** the installer asking once on Unknown |
| 3. Floors as data | **Deferred** as in r19, r23 and 11-06; the rule was repaired (393e6de6) |
| 4. Ratchets on `syn`, not regexes | **Deferred:** the regexes were extended once more (3cf8fba4) |

## Carried over

- MCP-160 (a cancellable FEA solver) stays deferred as in `review-2026-11-06-status.md`.
- `replace: true` stays a flag the client sets (SEC-182).
- `plugin_sandbox::host_max_response_honors_timeout_when_plugin_stops_reading` times out on this
  Windows machine, alone too; `ketchup-scheduler` is untouched since `ca8bc99f`, so CI decides.
