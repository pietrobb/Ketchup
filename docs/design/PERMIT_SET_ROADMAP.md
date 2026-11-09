# Roadmap: from a model to a building-permit submission

**Goal.** One person goes from a Ketchup model of a house (timber frame first, then any building)
to a set of documents a Slovak or Czech building office accepts. Partners must be able to work on
the drawings in AutoCAD, BricsCAD or DraftSight without redrawing them. That partner may be an
architect, a structural engineer, or a specialist in fire safety or building services. In short:
model it, export the set, send it to the office, done.

Ketchup does not replace the authorised designer. It produces the documents, and the authorised
designer checks, stamps and signs them. Every report it writes says which checks it made and which
it did not.

This plan is ordered. Each work package (WP) states:
- where the code goes;
- what "done" means;
- its rough size: S is up to a few days, M about a week, L several weeks.

The rules of `AGENTS.md` apply throughout:
- The core gets only generic operations. Hatch patterns, a labelled region with an area, layer
  mapping and level marks are generic.
- Domain content lives in the program library or in data files, not in Rust. That covers material
  names, a room-name list, the office title-block template and default layer names.
- Validators against a standard are allowed. A thermal check after STN/CSN 73 0540-2 is one.

## 0. State at `8d781633` (0.3.2), checked against the code

The drawing set (`ketchup-manufacturing/src/project_drawings*`) already has these sheets:
- one plan per storey;
- a longitudinal and a cross section;
- four elevations;
- a window and door schedule;
- shop drawings of walls and floors;
- cutting plans.

What the sheets carry today:
- **Frame and title block.** An ISO A frame and a title block with 12 user fields
  (`title_block.rs`).
- **Language.** Sheet language `cs`, `sk` or `en`.
- **Zero level.** A zero level (`zero_level_mm`).
- **Program annotations.** Texts, dimensions with ticks, and leaders (`project_drawings/annotations.rs`).

Gaps that matter for a permit set:

| Area | Today | Needed |
|---|---|---|
| Parts cut by the plane | Solid fill plus heavy outline (`sheet_pdf::Mark::Fill`) | Material hatching per ISO 128-50 / STN 01 3420 |
| Sheet DXF | AutoCAD R12 (`sheet_dxf.rs`) | DXF 2013 or later, so that AutoCAD users can work on it |
| Sheet DXF content | LINE per pen layer, fill outlines on CUT, TEXT; no HATCH, DIMENSION, blocks or paper space | All of these (section 2) |
| Sheet DWG | Converted from that DXF | The same content |
| Dimensions | Overall dimensions per view, plus what the program annotates | Chains of openings, walls and overall size; parapet heights |
| Levels | Derived from floor areas (heuristic; wrong for layered floors, review 2026-11-08 MF-241) | A storey table entered as data |
| Rooms | None | Number, name, area, floor finish, clear height; a room table |
| Plan symbols | No section markers, north arrow or material legend | All three |
| Site plan | None | A site plan on the cadastral map in S-JTSK |
| Reports | EC5 member checks in the program output | A printable statics report, a thermal assessment and area statements |
| PDF | Vector PDF | PDF/A, ready for a qualified electronic signature |

The underlay editor can already delete, move, rotate, mirror, trim, extend and offset. It draws
lines, rectangles, circles, arcs and text. It writes a DXF/DWG back with every unchanged object as
it was. Section 3 builds on it.

## Phase 0. Correctness first (blocks everything below)

A permit set with a wrong level or an unearned "pass" is worse than no set. These come from
review 2026-11-08 and must be fixed before the drawings are promoted for permits.

- **WP-0.1 (M).** Make the statics edge rule and joint kinds explicit:
  - PG-240: a side ledger counts as holding the top edge;
  - PG-141(b): sloped extrusions;
  - PG-241: any non-English joint kind fastens;
  - PG-243.

  Done when:
  - an unknown joint kind is `not_verified`;
  - the probes of that review end on the safe side;
  - the house golden runs at pitches 30/40/50 with snow.
- **WP-0.2 (M).** Storeys as data (MF-241); this replaces the level heuristic. For each storey the
  sheet settings hold:
  - name;
  - finished-floor level;
  - structural level;
  - the absolute height of +/-0.000 (Bpv).

  Done when tiny-house shows +/-0.000 at the finished floor and the upper storey cut at its own
  finished floor plus the cut height.
- **WP-0.3 (S).** Merge the woodWOP CRLF fix (PR #33). Add a known-issue note to 0.3.2.
- **WP-0.4 (S).** Fix CD-240 (Discard shifts drawn-object ids) and SEC-241 (`--mcp-setup` under
  sudo).

## Phase 1. Drawings that follow the drawing standards (PDF)

Target conventions:
- STN 01 3420 / CSN 01 3420 (drawings of buildings);
- ISO 128 (lines, cuts, hatching);
- ISO 5457 (sheets, folding);
- ISO 7200 (title block).

The national standard governs where it differs. The implementer reads the current edition before
coding the conventions in.

- **WP-1.1 (S). Pen table.**
  - Named pens with a width and a dash: 0.13, 0.18, 0.25, 0.35, 0.5, 0.7 and 1.0 mm.
  - Line types: continuous, dashed (hidden or above the cut), dash-dot (axes, cut planes) and
    dash-dot-dot.
  - A drawing-set setting maps each role to a pen. The roles are cut outline, visible edge, hidden
    edge, axis, dimension, text and underlay.
  - Done when the defaults match the national standard and a user file can override them.
- **WP-1.2 (M). Material hatching in cuts.**
  - A hatch is a generic property of a material: families of parallel lines (angle, spacing,
    offset, dashes), solid, or a band pattern drawn along the band's midline. The band pattern is
    the zigzag of soft insulation, laid across the thickness.
  - Spacing is in paper millimetres, so it reads the same at 1:50 and 1:100.
  - The library gives defaults per material: solid timber, glulam, CLT, OSB, gypsum and fibre
    board, mineral wool, wood fibre, EPS/XPS, concrete, reinforced concrete, masonry, gravel, soil
    and membranes. Membranes are a single thick line, not a hatch.
  - Done when every section and plan of tiny-house and house-project shows each material with its
    own hatch, and the legend (WP-1.7) lists them.
- **WP-1.3 (M). Dimension chains on plans.**
  - Three chains outside each facade: openings, walls and offsets, overall.
  - Room dimensions inside.
  - Opening labels with width/height and parapet height.
  - Ticks, not arrows, where the standard asks for them.
  - The chains are generated from the model on every export, and user dimensions (WP-3.2) are
    kept.
  - Done when the tiny-house plan carries all three chains and they add up to the overall
    dimension.
- **WP-1.4 (S). Level marks.**
  - A level mark symbol per STN 01 3420 in sections and elevations, with relative and absolute
    height (+/-0.000 = 245.300 Bpv).
  - Plans get floor-level marks.
  - Built on WP-0.2.
- **WP-1.5 (M). Rooms (generic labelled regions).**
  - A region is a closed area on a storey with these fields:
    - number;
    - name;
    - floor finish;
    - clear height;
    - free fields.
  - Its area and perimeter come from the plan cut, to the finished inner face of the walls.
  - Plans get a label for each region.
  - The room table gives per-storey and total areas and is written to the sheet and to CSV/XLSX.
  - Names and finishes come from a library list, not from code.
  - Done when tiny-house has its rooms labelled and the table's sum equals the usable floor area
    within 0.01 m2.
- **WP-1.6 (S). Plan symbols.**
  - A north arrow; the model holds the angle of north.
  - Section markers (A-A, B-B) with view direction, placed where the sections are cut.
  - A graphic scale bar.
  - Elevations named by the side they face (south, north, ...) from the north angle, in the
    sheet language.
- **WP-1.7 (M). Legend and build-up callouts.**
  - A legend of the hatches and line types used on the sheet.
  - A callout for every build-up a section cuts, such as a wall, floor or roof: a leader to the
    section and a table of its layers outside-in, with material and thickness.
  - The build-ups come from the model, because the program library already describes them.
- **WP-1.8 (S). Title block template as data.**
  - The title block layout is a user file: its fields, their sizes and an office logo.
  - It has free fields such as parcel number, cadastral area, client address and design stage.
  - It keeps an empty frame for the authorised designer's stamp and signature.
  - The sheets carry a revision table: index, date, description, by whom.
  - Default templates for SK and CZ are in the library.
- **WP-1.9 (M). Sheet composition.**
  - Several views per sheet, at their own scales, placed by the user and kept between exports.
  - Detail views: a crop of a section at 1:5 to 1:20.
  - Folding marks for folding to A4 (ISO 5457).
  - A drawing register sheet listing every sheet of the set.
- **WP-1.10 (S). PDF/A.**
  - Write PDF/A-2b with embedded fonts, an output intent and document metadata.
  - Both the set as one file and one file per sheet.
  - Done when veraPDF validates the output and a qualified electronic signature (made outside
    Ketchup) can be applied without invalidating it.

## Phase 2. DXF/DWG that an AutoCAD user can keep working on

Today's sheet DXF is a picture made of lines. Partners need a drawing they can edit. Keep the R12
writer for CNC routers, and add a DXF 2013 (AC1027) writer for drawings. The underlay writer
already writes DXF 2007 and keeps code pages, so share its encoding.

- **WP-2.1 (L). Model space and paper space.**
  - The views go into model space at 1:1 in millimetres. Plans sit in real coordinates when the
    site is placed in S-JTSK (EPSG:5514), so that a site-plan underlay lines up.
  - Each sheet becomes a paper-space layout with viewports at the sheet scale.
  - The title block is a block with attributes.
  - Done when the tiny-house set opens in AutoCAD or BricsCAD (checked by hand once) and in
    `ezdxf` with zero audit errors.
  - **Status:** written (`ketchup_dwg::sheet`, `ketchup_manufacturing::sheet_cad`).
    - Plans, sections and elevations are in model space at 1:1. A plan the sheet turns lies
      unturned and its viewport turns it. A plan's north arrow and section marks go with it
      on `MARK`; the revision table stays on paper beside the title block.
    - One layout per sheet, with viewports at the set's scale.
    - Test sheets pass the `ezdxf` audit with zero errors and render like the PDF.
    - Still open:
      - the title block as a block with attributes;
      - shop drawings and cutting plans in model space (they are on paper only);
      - S-JTSK placement from the site (WP-4.1);
      - the manual check in AutoCAD or BricsCAD.
- **WP-2.2 (M). Layers by content.**
  - Each layer stands for what it holds, not for a pen: walls cut, walls beyond, openings,
    hatching per material, dimensions, texts, levels, rooms, axes, underlay and so on.
  - Each layer has a colour, a lineweight and a line type.
  - A mapping file sets the layer names, in an office scheme or an ISO 13567-style one. A default
    ships in the library.
  - Write the LTYPE table (DASHED, CENTER, HIDDEN, PHANTOM) with `$LTSCALE`, `$PSLTSCALE` and
    `$MSLTSCALE` set, so that dashes look right in model space and in layouts.
  - **Status:** content layers written.
    - The fixed default names are `CUT`, `CUT-FILL`, `VISIBLE`, `UNDERLAY`, `ON-CUT`,
      `DIMENSION`, `LEVEL`, `MARK`, `ANNOTATION`, `VIEW-TITLE`, `SHEET` and `SHEET-TEXT`.
    - Each entity keeps its pen as a lineweight. DASHED and CENTER are written, with
      `$PSLTSCALE` on.
    - Still open:
      - the mapping file for office or ISO 13567 names;
      - a split of `VISIBLE` by building element, which needs the model's tags on the marks.
- **WP-2.3 (M). HATCH entities.**
  - Every hatch of WP-1.2 becomes a HATCH.
  - The library maps each material to a standard pattern name with its definition: ANSI31,
    ANSI37, AR-CONC, EARTH, INSUL, or SOLID. A pattern that a stock AutoCAD has not got goes into
    the file as a user-defined pattern.
  - The boundary is the cut polygon, so the hatch moves with it when edited.
  - Done when `ezdxf` and the ODA converter read every hatch back with the same pattern and
    boundary.
- **WP-2.4 (M). DIMENSION, TEXT and STYLE.**
  - Dimensions are true DIMENSION entities with a DIMSTYLE: ticks, text height by scale, decimal
    comma. Editing a dimension in AutoCAD then works.
  - Texts are TEXT/MTEXT with a STYLE on a TrueType font that has Central European characters.
  - Room labels and level marks are blocks with attributes.
- **WP-2.5 (S). Blocks for symbols.**
  - Doors with swing, windows, north arrow, section marker and level mark become blocks, each
    inserted where it is used.
- **WP-2.6 (M). Office template.**
  - Read layers, line types, dimension and text styles, and blocks from the office's own `.dwt`
    or `.dxf` template.
  - Write the set using them, so that it looks like the office's own drawings.
- **WP-2.7 (S). Round-trip tests.**
  - A golden DXF of the tiny-house set.
  - The `ezdxf` audit and a DWG round trip through the ODA converter in CI.
  - A test that a partner's edit can come back. Open the DXF as an underlay in Ketchup and
    compare it with the model, in the spirit of the architect-DXF revision overlay proposed in
    review 2026-11-08.

## Phase 3. 2D drafting

The generated views never cover a whole permit set. Details, notes and corrections have to be
drawn, and they must survive the next regeneration.

- **WP-3.1 (L). Drafting tools on sheets and on 2D drawings.**
  - Drawing: polyline with arcs, spline, rectangle, polygon, circle, arc, ellipse.
  - Hatch: pick a closed region, set pattern, scale and angle.
  - Dimensions: linear, aligned, chain, baseline, angular, radius, diameter, arc length.
  - Annotation: leader and multileader; text with styles.
  - Editing: offset, trim, extend, 2D fillet and chamfer, array, scale, stretch, match
    properties.
  - Blocks: define, insert, attributes. Layers with lineweight and line type.
  - Area measurement.
  - Existing underlay tools are reused, not duplicated (`AGENTS.md` section 6).
- **WP-3.2 (M). Annotations anchored to the model.**
  - A dimension or note on a generated view attaches to model edges or points.
  - On regeneration it follows them.
  - When its reference is gone, it is shown as orphaned and listed, never silently dropped.
- **WP-3.3 (M). Construction details.**
  - A detail sheet from a section crop at 1:5 or 1:10, with hatches, build-up callouts and the
    drafting tools.
  - Typical junctions: foundation and sill, wall and floor, wall and roof, window jamb, sill and
    head.
  - Stored with the document and regenerated from the model, as in WP-3.2.

## Phase 4. Site plan

- **WP-4.1 (M). Cadastral map and survey as underlays.**
  - Import the cadastral map as DXF/DWG: SK from the cadastral portal or ZBGIS export, CZ from
    the CUZK export. Import the surveyor's DWG too.
  - Place them in S-JTSK; the underlay keeps national-grid precision already.
  - Import spot heights as points (CSV or DXF) and build a terrain surface from them.
- **WP-4.2 (M). Site plan sheet.** At 1:200 or 1:500, it shows:
  - the building footprint on the map;
  - distances from the footprint to the parcel boundaries, as generated dimension chains to
    picked boundary lines;
  - parcel numbers and neighbouring buildings from the underlay;
  - utility connections drawn by the user;
  - north arrow, legend, and a table of the building's corner coordinates in S-JTSK.
- **WP-4.3 (S). Area statement.**
  - Built-up area, paved area, green area and parcel area, with their ratios to the parcel.
  - Computed from the footprint and from regions drawn on the site plan (WP-1.5 regions, on the
    site).
  - Written to the sheet and to CSV.
- **WP-4.4 (S). Terrain in sections and elevations.**
  - The original and finished terrain line, cut from the surface of WP-4.1.

## Phase 5. Reports for the annexes

- **WP-5.1 (L). Printable statics report** in SK/CZ, as an annex for the structural engineer.
  - For each member:
    - inputs;
    - load cases and the governing combination;
    - formula references (EN 1995-1-1 clause numbers);
    - utilisation.
  - Braced stretches name their braces.
  - It lists what was not verified.
  - National-annex factors and site snow and wind loads come from a `site(country, altitude,
    zone)` record. The zone data lives in the library with its source.
  - It requires WP-0.1.
- **WP-5.2 (M). Thermal assessment of build-ups** after STN 73 0540-2 / CSN 73 0540-2. A validator
  that, for every build-up, computes:
  - U against the required and the recommended value;
  - the internal surface temperature;
  - the annual condensation balance (Glaser method).

  Material lambda, mu and density come from the library with their sources. The result goes to a
  report and to the build-up callouts of WP-1.7.
- **WP-5.3 (S). Areas and volumes.**
  - Usable floor area per storey and in total, from the WP-1.5 regions.
  - Built-up area and enclosed volume.
  - Rules configurable per country.
- **WP-5.4 (S). Data for the specialists.** An export of what the fire-safety, energy and
  building-services specialists need:
  - rooms with areas and heights;
  - build-ups with their materials and reaction-to-fire classes, as material properties;
  - openings.

  Fire separation distances and the fire-safety design itself are out of scope for now.

## Phase 6. The submission package

- **WP-6.1 (M). A permit-set export in the File menu.** In one step it writes:
  - every sheet as PDF/A plus the combined set;
  - the drawing register;
  - the reports of phase 5;
  - the DWG/DXF set;
  - a ZIP named by the office's numbering scheme.

  Exports refuse stale or failed geometry, as production exports do.
- **WP-6.2 (S). Revisions.**
  - Sheets that changed since the last issued revision get a new revision index and a cloud or
    mark.
  - The revision table fills itself.
  - This uses the named versions proposed in the 2026-11-08 discussion: an automatic version
    before every AI task, a manual "save version" with a note, and comparison of two versions.
- **WP-6.3 (S). Submission checklist.**
  - A configurable list (library data, per country and procedure) of the parts a submission
    needs.
  - It is ticked automatically from the package, so the user sees what is missing before
    sending.
  - The content requirements follow the building act and its implementing decree:
    - SK: Act No. 25/2025 Coll. and its decree;
    - CZ: Act No. 283/2021 Coll. and Decree No. 499/2006 Coll. on construction documentation.

    The implementer confirms the current wording; it changes.

## Phase 7. IFC

- **WP-7.1 (L). IFC4 export (Reference View).**
  - Content:
    - the site, building and storeys (from WP-0.2);
    - walls, slabs, roofs, members, openings and spaces (from WP-1.5);
    - material layer sets from the build-ups;
    - properties.
  - The IFC class of a part comes from its program-library tag, not from code.
  - Done when the tiny-house IFC opens in a free viewer such as BIMvision or the BlenderBIM add-on
    with storeys, spaces and materials, and validates against the IFC4 schema.

## Order and parallel work

1. Phase 0, all of it.
2. In parallel:
   - WP-1.1, WP-1.2, WP-1.4, WP-1.6 and WP-1.8 (visible on paper, small);
   - WP-2.1 and WP-2.2 (the DXF 2013 writer, which the rest of phase 2 needs).
3. WP-1.3, WP-1.5, WP-1.7 and WP-2.3 to WP-2.5. After this the set is usable for a permit in
   practice.
4. WP-1.9, WP-1.10, WP-4.x and WP-6.1. After this a whole submission can be exported.
5. WP-3.x, WP-5.x and WP-2.6.
6. WP-6.2, WP-6.3 and WP-7.1.

**Acceptance of the whole roadmap.** `tiny-house.star` exports a permit set that an authorised
designer in SK and one in CZ review and would submit with only their stamp, signature and the
specialist annexes added. Every finding of that review becomes a test.
