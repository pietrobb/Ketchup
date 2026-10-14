# Review 2026-10-14: stav opráv

Review (`docs/review-2026-10-14.md` na vetve `claude/optimistic-goodall-2aqwdt`) bol robený na
`b40f464` (0.1.0). Každý nález sa tu overuje na aktuálnom `main`: reprodukciou alebo čítaním kódu.
Pretrvávajúci nález sa opraví spolu s testom, ktorý bez opravy padne.

Stavy: **opravené** (test padne bez opravy), **neplatí** (na `main` sa nereprodukuje),
**otvorené** (overené, oprava čaká), **neoverené**.

Poradie podľa §8 review: Hneď → Tento týždeň → Potom.

## P1

| ID | Stav | Oprava | Test |
|---|---|---|---|
| FMT-1 | opravené | `snapshot_codec::decode` použije každú konverziu len pre formáty zapísané pred ňou: plech V1 len pre < 101, `tag` → `tags` len pre < 102. Premenovanie rolí validátorov (formát 100) sa robí len pre < 100. Bez opravy golden 102 dostal digest pisateľa `b276203dbda20d1d` namiesto `32004c2fb9d7739d` a plech vo formáte 101 padol na „missing field `width`“ | `snapshot_codec::tests::format_101_and_102_sheet_metal_on_tagged_parts_opens_under_its_writer_digest`, `format_golden::a_format_102_file_with_history_opens_with_every_revision_it_was_written_with` (digest pisateľa) |
| PRG-M1 | otvorené | R10-B: vyhodnocovanie programu v samostatnom procese | |
| DR-1 | neoverené | R10-B | |
| GE-N2 | neoverené | R10-B, sonda | |
| VW-1 | neoverené | R10-A | |
| VW-2 | neoverené | R10-B | |
| VW-3 | neoverené | R10-B | |
| ST-N1 | otvorené | R10-B (prenesené z `cbec147`) | |
| MF-N1 | opravené | Jednotlivý woodWOP export (`woodwop_mpr_4_0_setup_a_programs`) odmieta zrkadlenú kópiu rovnako ako BTLx a hromadný export: bez opravy prijal kópiu so zrkadlom a vydal obrábanie originálu | `fabrication_validation::btlx_and_woodwop_reject_a_mirrored_copy_that_would_reuse_its_original_machining` |

## P2 a P3

| ID | Stav | Oprava | Test |
|---|---|---|---|
| NEW-B | opravené | Kontrola „Scale za závitom“ číta feature a definíciu cez `.get()`; chýbajúci záznam nechá na kontrolu definícií, ktorá ho odmietne menom (`FeatureNotFound`) namiesto pádu procesu | `product_validation::…::a_thread_in_a_damaged_definition_is_refused_instead_of_crashing` |
| TH-1 | opravené | Nová tabuľka `METRIC_TAP_DRILL` v prelúde (M3–M24, de.wikipedia Kernlochbohrer + fullerfasteners): M8 6,8 a M12 10,2 namiesto D−P 6,75 a 10,25. Diera vŕtaná D−P závit stále prijme (pol kroku vrtáka) | `fasteners::metric_threads_give_the_coarse_pitch_and_the_tap_drill_of_the_shop`, `program::a_tapped_hole_drills_the_tap_drill_and_the_machining_report_names_the_thread` |
| GX-15 | opravené | Uhol (otočenie, úkos, kruhové pole) a mierka prijmú desatinnú čiarku (`length_input::parse_decimal`); bez opravy „5,0“ v úkose aj „1,5“ v mierke boli odmietnuté | `draft_faces::the_draft_tool_drafts_the_selected_faces_about_the_clicked_one_in_one_undo_step` („5,0“), `headless_shell::a_typed_scale_factor_grows_the_part_and_a_second_one_replaces_it` („1,5“) |
