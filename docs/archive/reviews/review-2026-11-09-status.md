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

## Release (§7 point 5, on GitHub)

| Finding | Status |
|---|---|
| R1109-REL-300: the 0.3.2 release on GitHub has wrong sums and notes | Fixed 2026-11-09 on the release itself, its files unchanged: `SHA256SUMS.txt` names only the four downloads with LF line ends (the first named `payload/package-manifest.json` and had CRLF), and `sha256sum -c SHA256SUMS.txt` over the four downloaded into an empty folder answers OK for each. The notes say Cancel stops only the meshing of the FEA review, not a running solve, and carry the known issue of woodWOP programs with LF line ends with its workarounds, as `CHANGELOG.md` does. Issue #29, fixed by f23b50bb, is closed |
