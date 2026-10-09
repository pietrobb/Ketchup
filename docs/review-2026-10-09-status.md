# Review 2026-10-09: stav opráv

Review (`docs/review-2026-10-09.md` na vetve `claude/optimistic-goodall-2aqwdt`) bol robený na
`2bf43c7`. Každý nález sa tu overuje na aktuálnom `main`: reprodukciou alebo čítaním kódu.
Pretrvávajúci nález sa opraví spolu s testom, ktorý bez opravy padne.

Stavy: **opravené** (test padne bez opravy), **neplatí** (na `main` sa nereprodukuje),
**otvorené** (overené, oprava čaká), **neoverené**.

## P1

| ID | Stav | Oprava | Test |
|---|---|---|---|
| PRG-1 | opravené | `starlark` 0.13.0 s jednou zmenou (`third_party/starlark/PATCH.md`): každá iterácia cyklu a komprehenzie prejde háčikom `before_stmt`, takže limit 5 M krokov zastaví aj komprehenziu, `lambda` a cyklus s `pass` | `execution_budget::tests::iteration_without_statements_counts_against_the_budget`, `a_loop_costs_one_step_per_iteration_plus_its_statements` |
| PRG-2 | opravené | `ProgramProfileSegment::doubled_signed_area` počíta plochu so znamienkom vrátane výdutí oblúkov a Bézierov; používa ju `faces.rs` aj `model.rs` | `tests/faces.rs::faces_point_outward_for_a_clockwise_profile_of_arcs` |
| GEO-1 | opravené | `arc_winding`: oblúk sa rozdelí na úseky monotónne v y a každý započíta dolný koniec, nie horný (rovnako ako úsečka), takže lúč cez spoj oblúk–úsečka sa počíta raz | `tests/sketch_solver.rs::a_circle_level_with_a_rounded_corner_joint_is_a_hole_only_inside_the_board` |
| DXF-1 | opravené | limit profilov sa kontroluje priebežne po každom INSERT a pre celok ešte pred hľadaním duplicít | `tests/dxf_import.rs::many_inserts_of_a_full_block_are_refused_before_comparing_their_profiles` |

Vyhodnocovanie programu mimo UI vlákna s termínom a zrušením (druhá časť návrhu k PRG-1) je
krok míľnika §5.2 (#2452), nie oprava správnosti.

## P2

| ID | Stav | Poznámka |
|---|---|---|
| DXF-2 | opravené | `rem_euclid` malého záporného uhla dá presne 360°, čo viedlo na `unreachable!`; uhol sa normalizuje na 0 a vetva je úplná. Test: `tests/dxf_import.rs::an_insert_turned_a_hair_below_zero_is_not_turned` |
| AI-1 | opravené | `view action=section` zahodil `offset_mm = 0` ako „nevyplnenú“ hodnotu; s normálou sa nula teraz posiela. Test: `ketchup-mcp` `a_section_through_the_origin_keeps_its_zero_offset` |
| UI-4 | opravené | pri zamknutej osi je jedna čiarka desatinná („12,5“ = 12,5 mm po osi), nie dvojzložkový vektor. Test: `a_decimal_comma_on_a_pinned_axis_is_one_distance_not_a_vector` |
| GEO-2 | opravené | `extend_entity` oblúka, ktorý by prešiel celú otáčku (270° × 1,5), sa odmietne ako neplatný parameter namiesto potichu skráteného oblúka. Test: `workplane_sketch.rs::canonical_extend_is_reviewed_persistent_and_fails_closed` |
| UI-1 | opravené | medzerník (skratka Výber) s textovou udalosťou „ “ otvoril pole hodnoty, ktoré potom držalo klávesnicu a ďalšia skratka nezabrala; samotné medzery pole neotvoria. Test: `drawing_plane_tests::space_selects_without_opening_the_value_field` |
| INT-1 | opravené | snap Priesečník vracal polohu kurzora so vzdialenosťou 0, takže prebil bližší koncový bod; teraz je to skutočný priesečník hrán (hrany, ktoré sa míňajú, priesečník nemajú) so skutočnou vzdialenosťou. Test: `an_intersection_snap_lands_on_the_crossing_and_yields_to_a_nearer_endpoint` |
| SKP-1 | opravené | SketchUp zapisuje rovnomernú mierku do homogénneho deliteľa m[15] (mierka 2 = 0,5); import ho odmietol. Deliteľ sa teraz vydelí; nenulová perspektívna časť ostáva chybou. Test: `a_uniform_scale_in_the_homogeneous_divisor_is_imported_as_scale` |
| PRG-3 | opravené | `expect` s mierou na neexistujúcu plochu alebo diel sa potichu preskočil, takže zámer vyzeral splnený; teraz je to chyba `expectation_unmeasurable`. Test: `intent_along_a_face_the_part_lacks_is_an_error_not_a_pass` |
| ST-1 | neplatí | review bežal na `2bf43c7`, pred opravami CR7 (`7e10b5e`). Na `main` rozloženie z review (doska 1 250×2 500, 2 kN/m², stropnice v osiach 0/625/1 250, prah pod okrajom dosky) dáva strednej stropnici 3 125 N (pás 625 mm) a prahu 125 N (pás 25 mm). Test `member_loads.rs::a_deck_shares_its_area_load_by_the_strip_each_joist_is_nearest_to` |
| ST-2 | neplatí | rovnaký prípad (U-prah, trám 45×145, 4 m, doska 625 mm) je test `member_design.rs::a_beam_across_both_legs_of_one_sill_is_checked_over_the_span_between_them`: dve uloženia 3 900 mm od seba, ohyb ručne prepočítaný, `fail` |
| ST-4 | neplatí | `member_loads.rs::a_load_on_an_overhang_lifts_the_far_end_off_its_post_and_says_so`: bez spoja sa vzdialená podpera nadvihne a prvok hlási preklopenie; so skrutkou je reakcia −P·1 500/2 950 (z review −3 103 N) |
| MF-1, MF-2, MF-3 | neplatí | opravené v CR7 (`2500dcc` a ďalšie) po `2bf43c7`. Testy: `btlx_rejects_a_mirrored_copy_that_would_reuse_its_original_machining` (zrkadlená kópia sa odmietne), `a_profile_centred_on_its_axis_is_machined_from_its_minimum_corner` (meria sa od minimálneho rohu polotovaru), `woodwop_frame_of_an_odd_axis_order_stays_right_handed` (pravotočivý rám pri otočení) |
| BOM-2 | opravené | spoj s `fastener=` bez polôh `fasteners=` dával v kusovníku 0 kusov; teraz je to jeden kus na spoj. Test: `program.rs::a_named_fastener_without_positions_counts_one_piece_per_joint` |
| LIB-1 | opravené | vrstva stropu v `buildup()` prijme `"hanger_rating": connector_rating(...)`, takže závesy pri otvore nesú publikovanú únosnosť výrobcu a nie sú „not_verified“. Test: `buildup.rs::a_floor_layer_gives_its_hangers_the_makers_rating` |
| AI-2 | opravené | `program apply` bez `overrides` ponechá hodnoty uložené pre ten istý program (napr. Push/Pull z okna); `{}` ich výslovne zmaže. Test: `a_whole_program_sent_again_keeps_the_stored_overrides_unless_it_gives_its_own` |
| AI-3 | opravené | odpoveď na publikovanú zmenu skracuje riadky logu (1 000 znakov) a zoznam parametrov (200), takže sa vždy zmestí do rámca a agent ju nepošle znova. Test: `a_published_change_with_a_huge_log_still_fits_one_response` |
| UI-3 | opravené | rezová rovina teraz obmedzuje aj výber a snapovanie: zásahy a body na skrytej strane roviny sa preskočia, takže odrezanú časť nemožno vybrať, ťahať ani na ňu snapovať. Test: `a_section_keeps_what_it_hides_out_of_picking_and_snapping` |
| DXF-3 | opravené | konce oblúkov s rôznymi stredmi sa takmer nikdy nezhodujú bitovo; konce do tolerancie sa zjednotia na jeden bod (pri viac ako dvoch kandidátoch je to nejednoznačná geometria), takže uzavretý obrys ostane uzavretý. Test: `arcs_with_different_centres_join_where_their_ends_meet` |
| DXF-4 | opravené | UTF-8 názvy (DXF 2007+), komentáre `999` a oblúky, kružnice a polylínie zrkadlené cez OCS (0,0,−1) sa načítajú; diagnostiky nad 1 024 sa zrátajú do `dxf.diagnostics-omitted` namiesto odmietnutia súboru. Test: `utf8_names_comments_mirrored_arcs_and_many_layers_import` |
| GLB-1 | opravené | otvorené primitívy jednej siete (Blender: jeden na materiál) sa spoja; čo ani potom nie je teleso (napr. sklo), sa vynechá s upozornením `glb_non_solid_pieces_skipped` namiesto odmietnutia scény. Primitív môže mať od 1 trojuholníka. Test: `glb_joins_material_pieces_and_leaves_out_an_open_surface` |
| DXF-5 | opravené | export profilov je AutoCAD R12 (AC1009), ktorý handles ani subclass značky nepotrebuje; má tabuľky LTYPE a LAYER a každý profil je jedna 2D `POLYLINE` (oblúky ako bulge), bez BLOCK/INSERT. Názov vrstvy sa zapíše v ASCII (`Pôdorys: Štít` → `Podorys_ Stit`), počet premenovaných je v správe o stratách. Test: `dxf_export.rs::profiles_export_as_r12_polylines_and_a_slovak_name_becomes_an_ascii_layer` |
| BOM-1 | otvorené → R9-C | pretrváva (kusovník berie všetky viditeľné diely). Oprava je krok 10 míľnika §5.2, jeden výrobný rozsah `production(tags=…)` (#2452) |
| UI-2 | otvorené → R9-B | pretrváva (`SetOccurrenceVisibility` nie je view-only). Oprava je rýchla výhra §5.1 (2) (#2451) |
| CSV-1 | otvorené → R9-B | pretrváva. Oprava je rýchla výhra §5.1 (8), skutočný kusovník CSV/XLSX (#2451) |

## P3

| Nález | Stav | Poznámka |
|---|---|---|
| Ctrl+Shift+Z robí Undo | opravené | Ctrl+Shift+Z je Redo (popri Ctrl+Y); kontroluje sa pred Ctrl+Z. Test: `every_shortcut_in_the_keymap_runs_its_command_and_prints_the_same_chord` |
| chyba programu orezaná na prvých 4 000 znakov | opravené | dlhá správa si nechá začiatok aj koniec (príčina chyby je za tracebackom). Test: `ai_reads_and_edits_the_window_program_in_one_call_each` |
| `patch`: rovnaká správa pre 0 aj ≥ 2 zhody | opravené | prázdny, nenájdený a viacnásobný text majú vlastnú správu a radu. Test: `ambiguous_missing_overlapping_or_invalid_patch_publishes_nothing` |
| `.star` nie sú `eol=lf` | opravené | `.gitattributes`: `*.star text eol=lf` (v indexe už všetky LF) |
| `material_weight`, `weight_scope`, `timber_strength` nezdokumentované | opravené | opísané v témach `loads` a `member_check`; test dokumentácie builtinov teraz zahŕňa `conditions::builtins` (`every_builtin_is_documented_in_the_library_comments`) |
| `program-language.md` uvádza 7 z 12 tém | opravené | uvádza všetkých 12; test `the_language_guide_names_every_library_topic` |
| GLB odmietne `alphaMode` BLEND a neznáme chunky | opravené | MASK/BLEND sa importuje ako nepriehľadná farba s upozornením `glb_material_transparency_ignored`; chunky neznámeho typu za JSON sa preskočia (podľa špecifikácie), duplicitný JSON/BIN ostáva chybou. Test: `glb_imports_transparent_materials_and_skips_unknown_chunks` |
| CAM: `ceil` dá o prechod viac | opravené | podiel o zaokrúhľovaciu chybu nad celým číslom (20 − 18,9)/0,1 = 11,000000000000014 je 11 prechodov; posledný prechod a dráha ležia presne na dne a na okraji. Pozn.: samotné 1,1/0,1 dáva v f64 presne 11. Test: `cam::step_tests::a_depth_a_rounding_error_above_whole_passes_takes_no_extra_pass` |
| ostatné P3 | neoverené | |
