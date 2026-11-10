# Review 2026-11-09: status of the findings

Every finding of `review-2026-11-09.md` (of `main` `54fdc748`, prefix `R1109-`) is listed here as
fixed, with its commits, or as deferred or kept, with the reason. The findings are fixed in the
order of its §7, one pull request for each area. Each code fix began with a test that failed before
it.

## Statics (§7 point 1, branch `r30-statics1109`)

| Finding | Status |
|---|---|
| R1109-PG-300: the edge a `brace()` declares is not held against the geometry | Fixed, d7494ea3: where the geometry reads an edge of the member at a contact, the declaration may not name the other one; laths under a beam declared on its top hold its bottom, and the declaration is reported (`declared_bracing_ignored`). Where the geometry cannot tell (a lath at mid-depth of the side), the declared edge holds as before |
| R1109-PG-301: a diaphragm holds though its boards are not joined | Fixed, d7494ea3: an anchor holds the boards it is fastened to where they touch; a board holds only when it reaches an anchor through boards touching each other, and one that does not is reported by name |
| R1109-PG-302: an unknown joint kind slips through a diaphragm | Fixed, d7494ea3: every check standing on a joint of an unknown kind between two parts of the chain bracing the member, its braces, their anchors and the diaphragm of a sheet among them, is not verified |
| R1109-PG-303: the anchor of a declaration bypasses the rules of found bracing | Fixed, d7494ea3: a brace fastened to a grounded part is anchored only at points that part holds it where they touch (a fastener beside the contact holds nothing), across the member and beyond its side; a block in line with the beam's end does not anchor it |
| R1109-PG-304: a sloping member's vertical cut taken for its depth (regression of `f0f733b3`) | Fixed, d7494ea3: the named `length_axis` chooses only which profile edge the grain follows, the one reaching farthest along it; a rafter 80 × 400 at 30° and 50° naming `"x"` is checked as 80 × 400, as one naming none. The large-house test at 50° now asserts the four sloping gable top plates are 140 × 60 ± 0.5. It passes on r30 and fails on main 54fdc748 with `top plate 1: [140.0, 93.343]`. Sloping sill (capability review run 611): in the tiny house at 50° with `snow_sk=1.5` the four sloping top plates of the gables are 140 × 60 (main: 140 × 93.343) and all 230 members pass. They report no `shear_notch`. Their profile is a plain parallelogram with plumb ends and no re-entrant corner, so there is no notch to check. The 11-08 values (0.281–0.5 at the end studs) came from horizontal cuts reaching the tip of a plumb end: 0.489 for a 134.5 N reaction means h_ef·kv ≈ 1.15 mm. Checking such an end square to the grain instead gave 0.014–0.016, and it turned rafters 2 and 9 not verified, so it was dropped. Test `a_sloping_top_plate_keeps_its_depth_and_its_notch_check`: a 140 × 60 plate at 30° and 50°, `length_axis` `"x"` or none. Plain, it is 140 × 60 with no `shear_notch`. With 20 mm cut from its underside over a stud, it gets `shear_notch` |
| R1109-PG-305: `length_axis` is not checked for sense | Fixed, d7494ea3: a value other than `x`, `y` or `z` is refused when the part is made; one naming an axis ten times shorter than the part's longest side leaves the member not verified, the reason naming both lengths |
| R1109-PG-306: k_f,1 = 60 for solid timber | Kept: 60 is the recommended value of EN 1995-1-1 table 9.2, as the review notes; a national annex value belongs to `site()` data with its source (§5.3) |
| R1109-PG-307: `spacing` has no lower bound | Fixed, d7494ea3: `brace()` refuses a spacing under 10 mm, which no fastening is that close |

## Manufacturing (§7 point 2, branch `r32-manuf1109`)

| Finding | Status |
|---|---|
| R1109-MF-303: the overlap of plan areas is about cubic and Cancel does not stop it | Fixed, 0549d3c8 (PR #48): a piece wholly outside the half-plane is kept whole instead of split, the subtraction runs only against pieces whose bounding box overlaps, and every clip operation counts toward Cancel. Test first: a slab over an uneven mesh is cut only where they overlap; an ignored release perf test of 3200 triangles |
| R1109-MF-300: the table of storeys is not checked | Fixed, branch `r38-storeys1109`: a table the model contradicts refuses the export of the drawings (PDF, DXF, DWG; window and MCP `storey_table_contradicts_model`), naming the row and why: its structure above its finished floor, its floor another row's, its floor under the model or not under its top, or no face of the model to stand on (looking up, not wholly under a solid lying on it) up to 50 mm under its floor (0/3500 against the floors at 200 and 2954; 2712, the lining under the joists, and 2932, the joists under the OSB). "Add a storey" in an empty table fills in the storeys found in the model, each structure at its floor, instead of one row of 0/0; measured by the area no solid lies on, it fills 200/2954 on the tiny house, not the lining at 2712 (capability review of PR #54) |

## Security (§7 point 3, branch `r33-sec1109`)

| Finding | Status |
|---|---|
| R1109-SEC-300: under `sudo`, `--mcp-setup` checks by path and writes by path | Fixed, 3d687611 and 463d6fbd (PR #49): the write runs with the effective user, group and groups of the owner of the folder the file is named in, then root again, so the system refuses root's folders whatever the checks saw. Probe as root in WSL: 3000 writes against a racing swap of the folder for a link leave 0 files in root's 0700 folder (the review found 14) |

## Ratchets and CI (§7 point 4, branch `r34-ratchets1109`)

| Finding | Status |
|---|---|
| R1109-TST-300: the forms clippy does not report are tested on made-up JSON | Fixed, 4d79ba03: `check_error_hygiene.py` counts a panic in a `macro_rules!`, `.map(Result::unwrap)` / `.map(Option::unwrap)`; a test runs the real clippy on a small crate and asserts that clippy and the text check between them count every form once |
| R1109-TST-301: `#[allow(clippy::unwrap_used)]` hides from the count | Fixed, 4d79ba03: clippy runs with `--force-warn` for the four lints, and the text check counts `allow`/`expect(clippy::` attributes |
| R1109-TST-302: code compiled only elsewhere is counted by no run | Fixed, 04f63bc3: the workspace packages a run compiles are counted too; counted on Linux with OCCT (`linux-occt`), for Android and the browser by `--target`, in the macOS Viewer job and the macOS app job; `check_ci_crates.py` fails on a crate with code for one system that no step there counts. A new `.expect` in Linux-only code of `ketchup-app` fails the Linux OCCT count, one in `ketchup-macos-open` the macOS count. The `macos-app` baseline was missing, so that job's first run allowed nothing and failed; de3e86ee records it from that run's own count, and `check_ci_crates.py` now fails on a step whose baseline file does not exist. On 63311b0a (run 42617687771) the macOS app job counts `OK; 488 in 18 packages` and its live tests then pass (18) |
| R1109-TST-306: `check_clippy_panics.py` without `-p` ends 0 having run nothing | Fixed, ef81ce3f: without `-p` it checks the packages the system's baseline names, and fails when there are none |
| R1109-CI-300: `WINDOWS_CODE` misses `cfg(not(unix))` and a cfg over several lines | Fixed, 1f51ed37 |
| R1109-CI-301: the junction tests of `ketchup-pdm` run nowhere | Fixed, 1f51ed37: they run in the Windows job |

## Release (§7 point 5, on GitHub)

| Finding | Status |
|---|---|
| R1109-REL-300: the 0.3.2 release on GitHub has wrong sums and notes | Fixed 2026-11-09 on the release itself, its files unchanged: `SHA256SUMS.txt` names only the four downloads with LF line ends (the first named `payload/package-manifest.json` and had CRLF), and `sha256sum -c SHA256SUMS.txt` over the four downloaded into an empty folder answers OK for each. The notes say Cancel stops only the meshing of the FEA review, not a running solve, and carry the known issue of woodWOP programs with LF line ends with its workarounds, as `CHANGELOG.md` does. Issue #29, fixed by f23b50bb, is closed |

## P3 manufacturing (§4.3, branch `r36-p3mf1109`)

| Finding | Status |
|---|---|
| R1109-MF-301: the table of storeys cannot say which storey is ±0 (a basement at the origin), and numbers its rows as entered while the sheets go by height | Fixed, bdb0fee5: each row is numbered as its plan sheet, by finished floor, and has a ±0,000 choice marking the storey at ±0,000 (the height set, else the ground floor found); choosing another storey sets ±0,000 to its floor |
| R1109-MF-302: a hole milled through a 60 mm part has Z = −1 and a depth of 62 | Fixed, 6764559a: a BTLx free contour is clamped to the part along its cut, so one through a 60 mm part starts on its face and is 60 deep; a cut wholly beyond the part is none. The woodWOP pocket already refuses a cut not starting on the face |
| R1109-MF-304: a 0.3.2 list naming its project by an 8.3 name (`PROJEK~1`) fits every folder of the same six letters | Fixed, d6f7ca54: an 8.3 name is the folder the file system gives it; only when a name has that form are the project's file and the list's folder asked for their short names (`GetShortPathNameW`), so no file is looked at otherwise. A copy in `Projekt beta` no longer takes the set of `Projekt alpha` (test on real 8.3 names), the original still does |
| R1109-MF-305: a pin into a bought part: the `.mpr`, the package and BTLx drill the made side without naming the loss, the production job refuses `UnmadePinHole` | Fixed, bdb0fee5: one loss in the capability gate for every format, the holes of a pin joint in a part the shop does not make joined to a made one. The production job lays the made part and lists the loss under `losses`; the BTLx support report has `unmade_pin_holes`; the `.mpr` and package exports, and BTLx, name it in the status line and in the MCP result (`lost`). Test with logical and drilled pins into a bought timber |

## Underlays (§4.3, branch `r37-p3docs1109`)

| Finding | Status |
|---|---|
| R1109-UL-300: an ARC of 360 / 0 or 180 / −180 is dropped | Fixed, bb4abe28: an arc whose end is a whole number of turns from its start reads and draws as its circle, as `ezdxf`, HATCH edges and the writer take it |
| R1109-UL-301: `InvalidShape` does not tell what an arc needs | Fixed, bb4abe28: the refusal says an arc needs an end that is not its start (an end whole turns from it draws a circle) and a fill a first ring with an area |
| R1109-UL-303: "Dĺžka steny, Hĺbka základu" stays in 1252 with only a warning; no choice of code page on import | Fixed (saved as format 110, which 0.3.2 refuses as newer): an underlay whose code page is doubtful opens a window with its layer names and texts as read, kept or read again in a chosen code page; MCP `file action=import as_underlay=true code_page=…` (the 17 `$DWGCODEPAGE` names); the underlay keeps it for `revision_of` imports and writes it into `$DWGCODEPAGE` on DXF export |
| R1109-CD-300: the hint tells the AI to have the user press Discard, but an export by the AI sets no Discard and MCP has none | Fixed: an export by the AI that refuses edits sets the window's Discard as one from the menu does, and `drawing action=discard_refused` (object ids from the refusal's `details.object_ids`, the default) discards them, keeping the other edits, as one Undo step; the hint names it |
| R1109-CD-301: texts too long of the file and drawn in Kečup together are `drawing_edit_not_writable` | Fixed, eb0142ef: the code of several refusals is the one each has when they share it, else `drawing_edit_not_writable` only when one of them is an edit Discard repairs; a text of the file and one drawn in Kečup, also nested, are `drawing_text_too_long`, whose hint tells to shorten them, not to press a Discard that is not there |

## P3 statics (§4.3, branch `r35-p3statics1109`)

| Finding | Status |
|---|---|
| R1109-PG-308: the tiny house returns 1 at 30° and 35°, the status says it passes | Fixed, a65b5430: the house is right to fail there, the north roof over the default 1000 mm knee wall comes down over the landing at the top of the stair. The `knee_wall` doc names 1300 mm at 30° and 1100 mm at 35°, which free the stair (test), and the R1108-PG-243 status says every member passes, not the program |
| R1109-PG-309: the joint catalogue reads a kind by its first word | Fixed, a65b5430: a kind is read as catalogue entries, the longest first, and qualifiers of size, product or side; any other word leaves it unknown. "slide in", "angle cut" and "plate bearing" are unknown, "loose tenon" and "contact adhesive" fasten |
| R1109-PG-310: one hint for every reason, `dedup` on an unsorted list, repeated `missing` lines | Fixed, a65b5430: each reason a declaration holds nothing carries its own hint (the edge to declare, the name to fix, the joint to add); a member's reasons and its braces without a strength class are each listed once, in the order found |
| R1109-PG-245, the rest: `Snow_SK` has no suggestion and a mistyped `--set` keeps the exit code | Fixed, a65b5430: the suggestion ignores case, and `check` with a `--set` the program has no parameter of ends with 2, the program not evaluated as asked |

## P3 security (§4.3, branch `r39-p3sec1109`)

| Finding | Status |
|---|---|
| R1109-SEC-302: an overlay's layer through a link onto a share, or a relative one, is local | Fixed, 950b8b64: a layer counts only as an absolute path that stays on local mounts once its links are followed, each directory checked before its link is read (test with a layer linked onto NFS and a relative one) |
| R1109-SEC-303: Linux mount points matched ignoring case and NFC (`/mnt/NAS` NFS against `/mnt/nas` ext4) | Fixed, 950b8b64: a Linux mount point is matched by its exact name; only macOS mount points fold case and Unicode form, as before (test with both pairs, in case and in NFC/NFD) |
| R1109-SEC-304: the `take_over` consent shows only `sheets.txt` | Fixed, 950b8b64: the question lists every file the take-over replaces, its sheets and their list, with their count, and the approval's payload is those files |
| R1109-MCP-305: the take-over dialog names the connected AI, not the one asking | Fixed, 950b8b64: the attach request carries the AI's name (at most 40 printable characters, unverified like `hello`'s) and the dialog names the AI asking and the one connected; a window older than the field hangs up on it and the MCP server asks it again without the name |
| R1109-SEC-301: a home that is a link of root's turns off the owner check | Fixed, branch `r39-p3sec1109`; SEC-300 alone did not remove it, as the review's §7 expected: the owner of the folder a file is named in was read without following links, so `~/.claude.json` under a home linked by root (`/home/u -> /data/u`) was root's, written as root with every check off. A link of root's now stands for the folder it leads to, and the write runs as that folder's owner; anyone else's link stays theirs (test of the rule; as root in WSL, with the home root's link to the user's folder and `.claude.json` the user's link to a file of root's, the write is refused and root's file kept, where without the fix root wrote it) |
| `FileIdInfo` has no fallback to the file index | Fixed, 950b8b64: where Windows gives no 128-bit id (FAT, SMB) the 64-bit file index stands for it, as NTFS's id reads, so the file keeps an identity |

## P3 CI and tests (§4.3, branch `r40-p3tests1109`)

| Finding | Status |
|---|---|
| R1109-TST-303: `_ = save(x);`, `writeln!(stderr())`, `todo!`, `unimplemented!` and `dbg!` are counted nowhere | Fixed, 06975b54: clippy's `todo`, `unimplemented` and `dbg_macro` join the forced lints of `check_clippy_panics.py`; `check_error_hygiene.py` counts `_ = call();` as `let_underscore`, `stderr()` outside an entry point as `stderr_write` (the one in `ketchup-rejection`'s logger recorded) and `dbg!` in a `macro_rules!`. A test runs the real clippy on a small crate holding each form |
| R1109-TST-304: the deadline check is narrow, 8 s in `live_bridge_bootstrap.rs` | Fixed, 06975b54: an `if` past a deadline, either way round, whose block fails the test after other statements (a child killed first) counts; the bootstrap test waits 60 s, only against a hang |
| R1109-TST-305: each change writes its module's new size into the frozen list | Fixed, 06975b54: `check_crate_layers.py` compares `FROZEN_MODULES` with the commit before (a PR's merge commit has `main` first): an entry may only go down, and one added may not be past 3 000 lines; the Python job checks out two commits |
| R1109-DOC-300: `R<MMDD>` collides for two reviews of one day | Fixed, 06975b54: AGENTS.md §7, a second review of a day names its round as its file does, `R1106r23-` |
