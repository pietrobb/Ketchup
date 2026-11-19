# Review 2026-11-19 (og): status of the findings

Every point of `review-2026-11-19-og-todo.md` (the fix list of `review-2026-11-19-og.md`, of `main`
`86b606ba`, prefix `R1119og-` and `og:R1110-`) is listed here as fixed, with its pull request, or as
deferred or kept, with the reason. The points are fixed in the order of the list, one pull request
for each area. Each code fix begins with a test that fails before it.

## The owner's decisions

They overrule every older finding they touch.

| Decision | Status |
|---|---|
| R1: a connected AI creates a new file anywhere the system allows without asking; only overwriting asks; only autostart and system folders are refused; a write log in the Connect tab replaces the questions; allowed folders are removed | Open (point 6) |
| R2: the last AI to connect wins without asking; the status line and the Connect tab say who connected | Open (point 7) |
| R3: rules for an AI on one computer are not tightened unless the user's work is at risk; statics, FEA and CAM always say what a check does not cover | Applies to every point below |

## 1. Numbers and programs people trust

| Point | Status |
|---|---|
| 1. R1119og-MF-801, MF-804: G-code without tool length compensation | Open |
| 2. R1119og-FEA-800: a face joint in an assembly is a rigid plate | Open |
| 3. R1119og-PG-800, PG-801: bracing anchorage without buckling and without a diaphragm | Open |
| 4. og:R1110-PG-402, R1119og-PG-802: the structural calculation for the authority | Open |
| 5. R1119og-MF-800, MF-803: drilling does not pass through the part | Open |

## 2. Policy for an AI

| Point | Status |
|---|---|
| 6. File writes by R1 (closes og:R1110-SEC-400, R1119og-SEC-800) | Open |
| 7. Connection by R2 (closes R1108-MCP-240, R1109-MCP-305, R1119og-SEC-804) | Open |

## 3. Release and data

| Point | Status |
|---|---|
| 8. og:R1110-REL-404, R1119og-DOC-801: an older version drops the north arrow, revisions and storeys | Fixed in pull request `fix/og-release-data`: sheet language, ±0,000 level and altitude, storeys, north and revisions save as container schema 3, which every released reader refuses as newer; `release_reads.py` also checks persisted struct fields |
| 9. R1119og-REL-800: the structural calculation command is not in the packages | Fixed in pull request `fix/og-release-data`: the Windows, Linux and macOS packages carry `ketchup-program`, and each package build checks it writes a calculation |
| 10. R1119og-DOC-802: CHANGELOG 0.3.3 lists three entries not in the tag | Fixed in pull request `fix/og-release-data`: the three entries moved to Unreleased |

## 4. CI

| Point | Status |
|---|---|
| 11. R1119og-EXA-801, CI-807: the nightly Windows run is red | Open |
| 12. CI-407, CI-808: rerun and a flaky Connect test | Open |

## 5. Drawings for the authority

| Point | Status |
|---|---|
| 13. og:R1110r2-MF-500, MF-501: storeys | Open |
| 14. og:R1110-CD-404, UO-406: rooms in DXF | Open |
| 15. og:R1110-MF-404: DXF 2013 sheets | Open |
| 16. og:R1110-PG-403 to PG-410: texts of the structural and thermal calculation | Open |
| 17. R1119og-MF-805: hole callouts | Open |

## 6. Workshop, import, modelling

| Point | Status |
|---|---|
| 18. og:R1110-MF-400, MF-401, MF-406, R1119og-MF-807: losses in the workshop package | Open |
| 19. og:R1110-MF-402, MF-403: checking the package | Open |
| 20. R1119og-SKP-800: a SketchUp component changed by Scale | Open |
| 21. R1119og-IMG-800: the presentation image | Open |
| 22. R1119og-EXA-800: a control point of a freeform surface | Open |

## 7. DXF revisions from the architect

| Point | Status |
|---|---|
| 23. og:R1110-UL-400 to UL-403 | Open |

## 8. FEA, the rest

| Point | Status |
|---|---|
| 24. R1119og-FEA-801 to FEA-809 | Open |

## 9. Process

| Point | Status |
|---|---|
| 25. The review texts on `main`, IDs with the reviewer's suffix, one status document for each review | Fixed in the pull request adding this file: `review-2026-11-10-og.md`, `review-2026-11-10-og-r2.md`, `review-2026-11-19-og.md`, `review-2026-11-19-og-todo.md` and the other reviewer's `review-2026-11-09.md` (11-03 and 11-08 were already on `main`); this document is the status of the 11-19 review |

## When there is time (P3)

The P3 items at the end of the todo list stay open until the points above are done.
