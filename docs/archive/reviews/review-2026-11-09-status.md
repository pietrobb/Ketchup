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

## P3 statics (§4.3, branch `r35-p3statics1109`)

| Finding | Status |
|---|---|
| R1109-PG-308: the tiny house returns 1 at 30° and 35°, the status says it passes | Fixed, a65b5430: the house is right to fail there, the north roof over the default 1000 mm knee wall comes down over the landing at the top of the stair. The `knee_wall` doc names 1300 mm at 30° and 1100 mm at 35°, which free the stair (test), and the R1108-PG-243 status says every member passes, not the program |
| R1109-PG-309: the joint catalogue reads a kind by its first word | Fixed, a65b5430: a kind is read as catalogue entries, the longest first, and qualifiers of size, product or side; any other word leaves it unknown. "slide in", "angle cut" and "plate bearing" are unknown, "loose tenon" and "contact adhesive" fasten |
| R1109-PG-310: one hint for every reason, `dedup` on an unsorted list, repeated `missing` lines | Fixed, a65b5430: each reason a declaration holds nothing carries its own hint (the edge to declare, the name to fix, the joint to add); a member's reasons and its braces without a strength class are each listed once, in the order found |
| R1109-PG-245, the rest: `Snow_SK` has no suggestion and a mistyped `--set` keeps the exit code | Fixed, a65b5430: the suggestion ignores case, and `check` with a `--set` the program has no parameter of ends with 2, the program not evaluated as asked |
