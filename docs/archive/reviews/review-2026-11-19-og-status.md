# Review 2026-11-19 (og): status of the findings

Every point of `review-2026-11-19-og-todo.md` (the fix list of `review-2026-11-19-og.md`, of `main`
`86b606ba`, prefix `R1119og-` and `og:R1110-`) is listed here as fixed, with its pull request, or as
deferred or kept, with the reason. The points are fixed in the order of the list, one pull request
for each area. Each code fix begins with a test that fails before it.

## The owner's decisions

They overrule every older finding they touch.

| Decision | Status |
|---|---|
| R1: a connected AI creates a new file anywhere the system allows without asking; only overwriting asks; only autostart and system folders are refused; a write log in the Connect tab replaces the questions; allowed folders are removed | Fixed (point 6) |
| R2: the last AI to connect wins without asking; the status line and the Connect tab say who connected | Fixed (point 7) |
| R3: rules for an AI on one computer are not tightened unless the user's work is at risk; statics, FEA and CAM always say what a check does not cover | Applies to every point below |

## 1. Numbers and programs people trust

| Point | Status |
|---|---|
| 1. R1119og-MF-801, MF-804: G-code without tool length compensation | Fixed in #127: every dialect writes the safe header (G40 G49 G80 G94) and G43 H with the tool's length offset before the first cut |
| 2. R1119og-FEA-800: a face joint in an assembly is a rigid plate | Fixed in #128: a bonded joint ties only the part where the faces overlap (node to node or node to face), a rigid spider only for an explicit bolt, faces that do not touch are refused; two 1000×50×50 layers bonded by a face bend as one piece of the same mesh (0.197 mm at the end on 20 cells, where the beam formula gives 0.381 mm: the coarse mesh is stiffer, not the joint) and a spider does not (corrected 2026-11-24, R1124og-FEA-900: the 0.342 mm stated here came from no test). Faces that cover each other only in part are refused when their meshes would meet only along a line (R1124og-FEA-900) |
| 3. R1119og-PG-800, PG-801: bracing anchorage without buckling and without a diaphragm | Fixed in #129: the anchorage path starts at the member (EA/L of the brace, buckling over its length, the stiffness of both connections) and a diaphragm is checked for every anchor |
| 4. og:R1110-PG-402, R1119og-PG-802: the structural calculation for the authority | Fixed in #130: the calculation opens with the overall verdict and the exit code of the command line and a warnings section listing every error and warning of the report; a lost load no longer leaves "Vyhovuje" alone; only the braces the check counted are listed, each with C and F_cap of its path to the ground |
| 5. R1119og-MF-800, MF-803: drilling does not pass through the part | Fixed in #131: a drill has a point angle (saved in format 120); a through hole is drilled past its exit until the full diameter is out, limited by the flute length alone, without the axial allowance (corrected 2026-11-24, R1124og-CAM-901: the planner does not check the stock or workholding under the exit; the simulation checks only declared fixtures there, and a CAM result says so in `not_checked`); the exact CAM simulation sweeps a drill's cone and a ball end mill's ball instead of a flat cylinder, so the stock left in a blind hole's cone is counted; tests compare residual and gouge volumes with the analytic values |

## 2. Policy for an AI

| Point | Status |
|---|---|
| 6. File writes by R1 (closes og:R1110-SEC-400, R1119og-SEC-800) | Fixed: allowed folders are gone; a connected AI writes a new file anywhere the user may write, unasked; only the system drive's root, system folders and autostart folders are refused, with the reason; replacing a file the AI did not write, or one changed since, asks in the window on every write path (export, drawings, program variants, CAM), returns a decline code when refused and is not written if the file changes while asking; the Connect tab lists every AI-written file with its path, time and size |
| 7. Connection by R2 (closes R1108-MCP-240, R1109-MCP-305, R1119og-SEC-804) | Fixed: the AI that connects last takes the window over without a question; the status line and the Connect tab name the new AI and the one it replaced |

## 3. Release and data

| Point | Status |
|---|---|
| 8. og:R1110-REL-404, R1119og-DOC-801: an older version drops the north arrow, revisions and storeys | Fixed in #133: a document with sheet language, ±0,000 level or altitude, storey table, north or revisions saves as container schema 3, which released versions refuse as newer instead of dropping the settings; `scripts/release_reads.py` also checks persisted struct fields |
| 9. R1119og-REL-800: the structural calculation command is not in the packages | Fixed in #133: the Windows installer and portable archive, `Ketchup.app` and the Linux archive carry `ketchup-program`, and each package build checks it writes a calculation |
| 10. R1119og-DOC-802: CHANGELOG 0.3.3 lists three entries not in the tag | Fixed in #133: the three entries moved to Unreleased |

## 4. CI

| Point | Status |
|---|---|
| 11. R1119og-EXA-801, CI-807: the nightly Windows run is red | Fixed in #136: the cause was runner load, not code (the panel test took 19 s on the last green nightly and 35 s on the red one beside three other house tests); the panel check runs as its own nightly step on one thread, `perf_*` budgets scale with the runner's measured speed, and a red scheduled run opens an issue |
| 12. CI-407, CI-808: rerun and a flaky Connect test | Fixed in #136: GitHub refuses to rerun run 42661783930 (too old, HTTP 403); all its jobs and the checks of its commit are green, and the next nightly covers it. The Connect test gives one fixed AI set-up row and waits for it before the click, so rows arriving between frames no longer move the button |

## 5. Drawings for the authority

| Point | Status |
|---|---|
| 13. og:R1110r2-MF-500, MF-501: storeys | Fixed in #135: without a storey table the plans and level marks use the same floors as `ModelFloors::table`, found on the export's thread; a lining under a floor structure is no longer a storey, so the test house with its ceiling 2700–2954 gets its upper plan on 2954 cut at 4154 and no +2,512 level; a storey of the model no table row covers is reported (`not_in_table`, a warning in the window) |
| 14. og:R1110-CD-404, UO-406: rooms in DXF | Fixed in #139: room fills are a light grey of their own (ACI 254), written first so the architect's lines and the room names lie over them, and from DXF 2010 on 60 % transparent (group 440); names and areas keep the colour of the rooms' layer; a room point on a line of its region (within 1 mm) is refused |
| 15. og:R1110-MF-404: DXF 2013 sheets | Fixed in #138: a section's cutting line uses the `CENTER` linetype; black fills and pens of the marks are ByLayer instead of RGB 0, so they show on a dark screen; the `Standard` text style is Arial (TrueType); dimensions and level marks as DIMENSION entities stay for later, as the fix list says |
| 16. og:R1110-PG-403 to PG-410: texts of the structural and thermal calculation | Fixed in #145: only the highest row of the governing check says it governs; glulam cites k_h by 3.3 (3.2) and u_inst no longer cites (2.2); a column has no lateral torsional buckling text and `brace()` on it says a column's bracing is not used; the site line is in Slovak or Czech; a thermal failure prints "<" only where f_Rsi fails; the Slovak requirement is U_N (R_N on the ground); a site below sea level names the altitude 0 its s_k uses |
| 17. R1119og-MF-805: hole callouts | Fixed in #137: a tapped hole whose bottom the part does not show keeps its drilled depth (`M8 THREAD DEPTH 12 mm; BORE Ø6.8 mm CYLINDRICAL DEPTH 20 mm`), a coarse thread is named by its size alone, and callouts are written in the sheets' language (English, Czech, Slovak) with its decimal sign |

## 6. Workshop, import, modelling

| Point | Status |
|---|---|
| 18. og:R1110-MF-400, MF-401, MF-406, R1119og-MF-807: losses in the workshop package | Fixed in #140: the package carries `losses.csv` (each thread, hole in a bought part and round hole milled instead of drilled, by part) and the BTLx `support.txt`; a package that makes nothing names the reason of each output; a CAM program lists the threads no operation cuts in its release prompt and in the AI result |
| 19. og:R1110-MF-402, MF-403: checking the package | Fixed in #141: the check compares each stored header with the central directory, refuses data no entry accounts for, names Windows cannot write and names equal by case or NFC; the package marks a non-ASCII name as UTF-8 (bit 11) and refuses such names when it is made |
| 20. R1119og-SKP-800: a SketchUp component changed by Scale | Fixed in #142: a component stretched with the Scale tool imports with the stretch baked into its own part definitions, and group placements stay rigid instead of carrying a scaling transform |
| 21. R1119og-IMG-800: the presentation image | Fixed in #143: a picture the renderer cannot read, such as one over 64 MP, shows as its part's colour, as in the viewport; the saved image names the part, the picture and why (`lost` for a client, the status line in the window) |
| 22. R1119og-EXA-800: a control point of a freeform surface | Fixed in #144: a control point is asked for with where it lies (`from_mm`) beside its row and column, stored in the document; when an earlier edit changed the net so that row and column lie elsewhere or no longer exist, the move is refused with both positions and the size of the net instead of moving another point |

## 7. DXF revisions from the architect

| Point | Status |
|---|---|
| 23. og:R1110-UL-400 to UL-403 | Fixed in #147: a revision that only changes an entity's color, linetype or lineweight, a text's color or its layer's color reports it changed (`Style`) (UL-400); entities the later revision draws alike in the same place under a new handle, as ArchiCAD, Revit and Allplan export them, count as renumbered, not removed and added, and drawings of which one has no handles are refused with the reason (UL-401); the comparison runs on the reading thread, not the window's (UL-402); the MCP answer carries the counts and the first 100 entities of each list, the rest is paged through `inspect underlay_objects` of the overlay by layer (UL-403) |

## 8. FEA, the rest

| Point | Status |
|---|---|
| 24. R1119og-FEA-801 to FEA-809 | Fixed in #146, #148 and #149: a frequency study checks the count of its modes by a Sturm sequence, so no mode is skipped (FEA-801), and starts from M-orthonormal vectors, so a slender solid converges (FEA-802); a mesh study is converged only when its finest mesh has at least twice the elements of the one before and the peak stress has settled too (FEA-803); a frame's largest move includes the sag inside each element, so one simply supported element under a uniform load reports its mid-span sag instead of 0 mm (FEA-804); cancelling stops the factorization, every iteration of the frequency search and the heat solve, and the AI answer is `cancelled` (FEA-806); the excluded physics says natural frequencies come only from a modes study and that the parts' own weight is not added (FEA-809). FEA-807 (the window's FEA outside the memory budget) is fixed by R1120ac-FEA-902 in #159 and #161; FEA-805 and FEA-808 still stand (review 2026-11-20 ac §3) |

## 9. Process

| Point | Status |
|---|---|
| 25. The review texts on `main`, IDs with the reviewer's suffix, one status document for each review | Fixed in the pull request adding this file: `review-2026-11-10-og.md`, `review-2026-11-10-og-r2.md`, `review-2026-11-19-og.md`, `review-2026-11-19-og-todo.md` and the other reviewer's `review-2026-11-09.md` (11-03 and 11-08 were already on `main`); this document is the status of the 11-19 review |

## When there is time (P3)

The P3 items at the end of the todo list stay open until the points above are done.
