# Review 2026-11-26 (og): stav opráv

Review (`docs/archive/reviews/review-2026-11-26-og.md`, PR #248) bol robený na `c6b576ce`. Opravy
išli v poradí podľa §6: najprv PG-1100, potom ostatné P2, potom P3 po skupinách, každá v samostatnom
PR s testom, ktorý bez opravy padne. Každý PR priniesol svoj záznam v `CHANGELOG.md` (Unreleased);
PR s týmto stavom nepridáva žiadny ďalší.

Stavy: **opravené** (zlúčené na `main`), **odložené** (s dôvodom), **bez kódu** (zistenie, nie
chyba v kóde).

## P2

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1126og-PG-1100 | opravené | #249 | Sila rozdelená na holom dotyku dá poddajnosť najviac (c √g_n + s √g_t)², čím je nahradený chýbajúci krížový člen 2cs·g_nt, a cesta cez holý dotyk drží aspoň tak tuho ako tá istá cesta bez neho (`loads/anchorage/bare.rs`). Test `bare_blocks_never_weaken_the_column_they_bear_on` s geometriou review (skrutky stĺpa ±15) a so stĺpom `c100pin` 400 × 100: predtým 0,533 N/mm s blokmi proti 0,885 bez nich, teraz rovnako (0,885 / 1,324 / 2,629 a 98,0 / 135,4 / 218,9 N/mm pri 30° / 45° / 60°). Veta CHANGELOG k PG-1003 už netvrdí k / cos² β všeobecne. |
| R1126og-PG-1101 | opravené | #250 | Dvojica za hranicou `MAX_UNVERIFIED_COLLISIONS` sa preskočí len vtedy, keď `certain_overlap_mm3` nič nedá; kvádre, ktoré zdieľajú viac, než môžu odobrať ich otvory, kapsy a nástroje, sú kolízia aj tam. Test `a_certain_overlap_past_the_listed_unverified_pairs_is_still_a_collision` (`past_limit.star` z review): predtým p–q bez kolízie, teraz `collision` a exit 1. |
| R1126og-PG-1102 | opravené | #253 | Každý otvor (obal jeho valca), kapsa (jej rohy) a nástroj (jeho obal v súradniciach dielu) sa ráta najviac v tom, čo jeho obal zasiahne zo spoločného obalu, a nikdy viac ako jeho objem. Hranica ostáva bezpečná: čo prvok zo spoločného obalu odoberie, leží v oboch. Sondy `beams.star`, `throughpocket.star` a `trimmed.star` sú testy v `certain_overlaps.rs` a bez zmeny padnú. Dve staršie očakávania sa stali kolíziami, obe skutočnými (doska vedľa drážky, nie v nej, 4 × 4 × 600 mm; dve orezané kópie 30 × 60 × 40 mm). |
| R1126og-PG-1103 | opravené | #252 | V pribalenom starlarku (`PATCH.md`): `Range::count()` ráta dĺžku v `i64`, takže iterátor ju udá presne; `Heap::collect_bounded` overuje hranicu haldy pri každom zväčšení vektora, aj pri iterátore bez dĺžky. Cez ňu zbierajú `list()`, `tuple()`, `enumerate`, `reversed`, `sorted`, `list.extend` a `+=`; `zip` overí každú položku pred zostavením. Testy v `deep_values.rs` (rozsah dlhší ako i32::MAX; `extend`, `+=`, `zip` nad `range(10**9)`) padali panikou `AlignedSize`, teraz „program memory limit exceeded“ na riadku 2. |
| R1126og-STEP-1100 | opravené | #251 | Súbor v koreni programu sa v mene telesa píše cez `./` (`one from ./model.step`); holé meno súboru bez priečinka (dokumenty spred #228) stojí za súbor toho mena v ľubovoľnom priečinku len vtedy, keď cesta prvku nemá priečinok vôbec. Telesá z vývojových zostáv medzi #228 a #251 so súborom v koreni sa čítajú ako staré; žiadne vydanie kúpené diely nemá. Test `a_file_at_the_root_does_not_stand_for_one_of_the_same_name_in_a_folder` (celý súbor aj `part=`) padal s „hettich/hinge.step took the root file's body“ a nahradil test, ktorého „starý dokument“ vyrobil dnešný kód. |

## P3 (§4) po skupinách

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1126og-PG-1104 | opravené | #254 | Tabuľka spevnení vo výpočte má stĺpec „F_d (tej kombinácie)“ so silou kombinácie najbližšej k vzperu; poznámka pod tabuľkou nárokov hovorí, že N_d a F_d tam sú najväčšie zo všetkých kombinácií a každá kombinácia sa overuje s vlastným k_mod. Základ v `member_check.rs`: „k_mod of each ultimate combination, held against the F_d of that combination“. Test `the_calculation_prints_the_force_of_the_combination_a_brace_buckles_in` (`kmod_g400_q106`: 0,169 kN pri k_mod 0,60) padal. |
| R1126og-PG-1105 | opravené | #255 | `str()`, `repr()`, `%`, `format` a f-reťazce hodnoty vnorenej hlbšie ako 200 sa odmietnu s riadkom programu (strážca rekurzie v starlarku udá hĺbku, `PATCH.md`); `...` píšu už len `fail()`, chyba chýbajúceho kľúča a `print`. `docs/program-language.md` hranicu 200 uvádza. Test `text_of_a_deeply_nested_value_is_refused_with_its_line` padal. |
| R1126og-PG-1106 | opravené | #256 | Za tromi rozdeleniami šikmých holých dotykov sa cesta ďalšiemu takému dotyku vyhne: drží popri ceste bez neho, nikdy namiesto nej, takže už nie je potichu tuhá. Cesta bez dotyku sa ráta prvá a keď čiastková cesta chýba, vráti sa ona, takže hranica z PG-1100 (s dotykom aspoň tak tuho ako bez neho) platí na každej úrovni delenia. Test `blocks_cut_into_pieces_hold_no_more_than_whole`: stĺp otočený o 30° / 60° medzi uzemnenými blokmi drží spevnenie 4,263 / 9,768 N/mm; tie isté bloky rozrezané na štyri dávali 4,429 / 10,844 (viac ako celé), teraz 4,174 / 9,591; test overuje oba smery: nie menej ako bez blokov, nie viac ako celé bloky. |
| R1126og-STEP-1101 | opravené | #251 | `\` sa mení na `/` a počíta sa ako priečinok len na Windows; inde je `a\b.step` súbor v koreni, tak ako ho otvára `step_files.rs`. Jednotkové testy v `program_imports.rs`. |
| R1126og-DXF-1100 | opravené | #257 | Každý export dá výkresu, ktorý má `SORTENTSTABLE`, jej CLASS (s novými výplňami aj bez nich) a entitu model space v sekcii ENTITIES, ktorej vlastník nie je záznam bloku, zapíše s vlastníkom `*Model_Space` (`Drawing::misowned_entities`). Test `an_earlier_exports_sort_table_gets_its_class_and_its_entities_model_space` (AC1015/1018/1032, s výplňou aj s čiarou) padal. |
| R1126og-DXF-1101 | opravené | #257 | CLASS tabuľky poradia má počet inštancií (91), keď je `$ACADVER` AC1018 alebo novší, nie len keď ho majú ostatné triedy výkresu. Test `a_sort_tables_class_is_counted_from_dxf_2004`. AutoCAD neoverený (S). |
| R1126og-DXF-1102 | opravené | #257 | Export pôdorysu R12 ďalej píše `$INSUNITS 4` a `$MEASUREMENT 1` (Kečup a export DWG ich čítajú), ale jeho správa hovorí, že DXF R12 ich nedefinuje a iný program ho môže otvoriť v jednotkách svojej šablóny. CHANGELOG už netvrdí, že export DXF je metrický pre každého čitateľa. Export v AC1015 sme nezaviedli. AutoCAD neoverený (S). |
| R1126og-DXF-1103 | opravené | #257 | `scripts/release_reads.py` berie najvyššie `KUNDRLY<n>`, ktoré vydanie menuje, nie počet výskytov. Test `test_the_underlay_versions_a_release_draws_come_from_its_source`. |

## §5 Čo review nekontroloval

Body §5 (OCCT, AutoCAD/BricsCAD, zrkadlené inštancie pod pravidlom istého prekrytia, staršie DWG)
nie sú nálezy; okrem testov vyššie sa nič nemenilo.
