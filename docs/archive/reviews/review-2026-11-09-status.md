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
| R1109-PG-304: a sloping member's vertical cut taken for its depth (regression of `f0f733b3`) | Fixed, d7494ea3: the named `length_axis` chooses only which profile edge the grain follows, the one reaching farthest along it; a rafter 80 × 400 at 30° and 50° naming `"x"` is checked as 80 × 400, as one naming none. The large-house test at 30, 40 and 50° passes |
| R1109-PG-305: `length_axis` is not checked for sense | Fixed, d7494ea3: a value other than `x`, `y` or `z` is refused when the part is made; one naming an axis ten times shorter than the part's longest side leaves the member not verified, the reason naming both lengths |
| R1109-PG-306: k_f,1 = 60 for solid timber | Kept: 60 is the recommended value of EN 1995-1-1 table 9.2, as the review notes; a national annex value belongs to `site()` data with its source (§5.3) |
| R1109-PG-307: `spacing` has no lower bound | Fixed, d7494ea3: `brace()` refuses a spacing under 10 mm, which no fastening is that close |

## Ratchets and CI (§7 point 4, branch `r34-ratchets1109`)

| Finding | Status |
|---|---|
| R1109-TST-300: the forms clippy does not report are tested on made-up JSON | Fixed, 4d79ba03: `check_error_hygiene.py` counts a panic in a `macro_rules!`, `.map(Result::unwrap)` / `.map(Option::unwrap)`; a test runs the real clippy on a small crate and asserts that clippy and the text check between them count every form once |
| R1109-TST-301: `#[allow(clippy::unwrap_used)]` hides from the count | Fixed, 4d79ba03: clippy runs with `--force-warn` for the four lints, and the text check counts `allow`/`expect(clippy::` attributes |
| R1109-TST-302: code compiled only elsewhere is counted by no run | Fixed, 04f63bc3: the workspace packages a run compiles are counted too; counted on Linux with OCCT (`linux-occt`), for Android and the browser by `--target`, in the macOS Viewer job and the macOS app job; `check_ci_crates.py` fails on a crate with code for one system that no step there counts. A new `.expect` in Linux-only code of `ketchup-app` fails the Linux OCCT count, one in `ketchup-macos-open` the macOS count |
| R1109-TST-306: `check_clippy_panics.py` without `-p` ends 0 having run nothing | Fixed, ef81ce3f: without `-p` it checks the packages the system's baseline names, and fails when there are none |
| R1109-CI-300: `WINDOWS_CODE` misses `cfg(not(unix))` and a cfg over several lines | Fixed, 1f51ed37 |
| R1109-CI-301: the junction tests of `ketchup-pdm` run nowhere | Fixed, 1f51ed37: they run in the Windows job |
