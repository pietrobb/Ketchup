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

## P3 statics (§4.3, branch `r35-p3statics1109`)

| Finding | Status |
|---|---|
| R1109-PG-308: the tiny house returns 1 at 30° and 35°, the status says it passes | Fixed, a65b5430: the house is right to fail there, the north roof over the default 1000 mm knee wall comes down over the landing at the top of the stair. The `knee_wall` doc names 1300 mm at 30° and 1100 mm at 35°, which free the stair (test), and the R1108-PG-243 status says every member passes, not the program |
| R1109-PG-309: the joint catalogue reads a kind by its first word | Fixed, a65b5430: a kind is read as catalogue entries, the longest first, and qualifiers of size, product or side; any other word leaves it unknown. "slide in", "angle cut" and "plate bearing" are unknown, "loose tenon" and "contact adhesive" fasten |
| R1109-PG-310: one hint for every reason, `dedup` on an unsorted list, repeated `missing` lines | Fixed, a65b5430: each reason a declaration holds nothing carries its own hint (the edge to declare, the name to fix, the joint to add); a member's reasons and its braces without a strength class are each listed once, in the order found |
| R1109-PG-245, the rest: `Snow_SK` has no suggestion and a mistyped `--set` keeps the exit code | Fixed, a65b5430: the suggestion ignores case, and `check` with a `--set` the program has no parameter of ends with 2, the program not evaluated as asked |
