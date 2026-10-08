# Stav nálezov code review 2026-10-07

Ku každému nálezu z `docs/code-review-2026-10-07.md`: overené v aktuálnom kóde,
potom oprava s testom, ktorý tvrdí výsledok, alebo dôkaz, prečo nález neplatí.

## CR7-A Hneď

| nález | stav | oprava a test |
|---|---|---|
| CI-1 sleepy | opravené | Unit testy čakajú cez `tests::shell::step_until`, `run_idle` v harnesse cez `wait_until`; `check_test_sleeps.py` zelený, baseline znížený o 5. |
| CI-1 `#[cfg(windows)]` | opravené | `path_with_suffix` a `release_descendant` v `assistant_process.rs` sú len pod Windows. |
| CI-1 Linux job | **čaká na operátora** | Clippy na Linuxe potrebuje hlavičky OCCT 8.0.1 (`ketchup-exact` kompiluje natívny kód cez `cc`). Na `ubuntu-latest` sú v apt len OCCT 7.x. Treba rozhodnúť: zostaviť OCCT 8.0.1 v CI s cache, alebo self-hosted Linux runner. Zmena CI sa robí až po súhlase. |
| GUI-1 | opravené | Shift-odznačenie zruší `selected_group`. Test `shift_deselecting_a_group_member_moves_only_the_parts_still_selected` (`tests/transform.rs`): presun posunie len diely, ktoré ostali vybrané. |
| GUI-2 | opravené | Preťažený prút (únosnosť `fail`) zhodí celkový stav súhrnu. Test `an_overloaded_member_fails_the_whole_summary`. |
| MF-1 | opravené | BTLx odmietne zrkadlenú inštanciu (`MirroredInstance`), čisté otočenie ostáva `Count="2"`. Test `btlx_rejects_a_mirrored_copy_that_would_reuse_its_original_machining` pre všetky tri osi, priamo aj zdedené. |
| MF-5 | opravené | Produkčná projekcia s výrobným dielom na skrytej vrstve alebo skrytým dielom skončí chybou `HiddenProductionParts { count }`. Test `production_projection_refuses_to_leave_out_hidden_parts`. |
| MF-3 | opravené | Nepárna permutácia osí vo woodWOP meria strojové Y od vzdialenej hrany, smery sa preklápajú zvlášť. Test `woodwop_frame_of_an_odd_axis_order_stays_right_handed`: konkrétne `XA/YA/ZA` bočnice skrinky a determinant +1 pre tri poradia osí. |
| PR-2 | opravené | `export_drawings` cez MCP neprepíše existujúci súbor (`drawings_target_exists`), odmietne sieťové cesty, zariadenia a ADS, zapisuje atomicky cez dočasný súbor a pri chybe vráti predchádzajúce nastavenia hárku. Testy v `live_bridge/drawings.rs`. |
| GE-1 | opravené | Všetky booleany OCCT začínajú prázdnou operáciou, `configure_boolean` nastaví argumenty a nedeštruktívny režim, `Build()` beží raz. Test `native_pair_results_do_not_depend_on_query_order` (`ketchup-exact/tests/pair_query.rs`): každá dvojica dá rovnaký výsledok bez ohľadu na to, čo sa dopytovalo pred ňou. |

## CR7-B Statika ako riešič

| nález | stav | oprava a test |
|---|---|---|
| ST-1 | opravené | Doska delí plošné zaťaženie podľa pásu v pôdoryse, ktorý je najbližšie ku každej podpere, nie podľa plochy kontaktu. Test `a_deck_shares_its_area_load_by_the_strip_each_joist_is_nearest_to`: nerovnaké osové vzdialenosti 0/500/1520, okrajový trám dostane len svoj pás (±3 %). |
| ST-2 | opravené | Kontakty s jedným podperným dielom sa delia na súvislé úseky (`bearing_runs`), trám cez obe ramená jedného prahu má dve uloženia a kontrolu rozpätia. Test `a_beam_across_both_legs_of_one_sill_is_checked_over_the_span_between_them`. |
| ST-3 | opravené | Zaťaženie, ktoré nedôjde k žiadnemu prvku, ide do `loads.unassigned` a prepne `load_capacity` na `incomplete`. Test `a_load_that_reaches_no_member_leaves_the_load_check_incomplete`. |
| ST-4 | opravené | Jeden riešič spojitého nosníka (`continuous_span.rs`, rovnica troch momentov) pre odovzdanie zaťaženia aj pre momenty a šmyk v kontrole prútu. Podpera, ktorá by musela ťahať nadol a nedrží ju spoj, sa uvoľní a zaťaženie sa prerozdelí. Keď ostane jediná podpera, prvok sa preklopí a hlási sa to v `missing`. Spoj ťahá nadol zápornou reakciou. Dlhé lôžko drží susedné pole votknuté; okraj lôžka, ktorý by musel prenášať kladný moment, sa nadvihne a pôsobí ako kĺb. Uloženia bližšie ako výška prierezu sú jedno uloženie. Testy: `two_equal_continuous_spans_put_five_quarters_on_the_middle_support`, `a_load_on_an_overhang_lifts_the_far_support`, `a_long_bed_clamps_the_span_next_to_it_instead_of_spanning_itself`, `a_bed_edge_that_would_have_to_hold_sagging_lets_go_and_acts_as_a_pivot` (riešič), `a_beam_over_three_posts_is_continuous_and_its_middle_post_takes_five_quarters`, `a_load_on_an_overhang_lifts_the_far_end_off_its_post_and_says_so` (odovzdanie, aj so skrutkou), `a_beam_continuous_over_a_middle_post_takes_five_eighths_shear_there`, `bearings_closer_than_the_beam_is_deep_hold_it_as_one` (kontrola prútu; bez zlúčenia padá so šmykom 0,647). Dom so snehom prejde. |
| ST-5 | otvorené | |
| ST-6 | otvorené | |
| ST-7 | otvorené | |
| NaN ≠ pass | otvorené | |

Vedľajšia úprava kvôli ratchetu `check_crate_layers.py`: inline testy
`fabrication.rs` sú presunuté do `fabrication/tests.rs` (súbor prekročil 5000
riadkov), determinant v teste ide cez `linalg::Mat3`.
