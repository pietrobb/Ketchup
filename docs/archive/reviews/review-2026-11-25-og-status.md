# Review 2026-11-25 (og): stav opráv

Review (`docs/archive/reviews/review-2026-11-25-og.md`, PR #236) bol robený na `03e52dec`. K nemu
pribudli štyri nálezy R1125lo z vlastnej kontroly toho istého `03e52dec`, overené sondou
(`R1125lo-1` až `R1125lo-4`). Opravy išli v poradí podľa §6: P1, potom P2, potom P3 po skupinách,
každá v samostatnom PR s testom, ktorý bez opravy padne. Záznam v `CHANGELOG.md` (Unreleased)
priniesli PR #244, #245 a #246; záznamy pre #237 až #243 pridal PR s týmto stavom.

Stavy: **opravené** (zlúčené na `main`), **odložené** (s dôvodom), **bez kódu** (zistenie, nie
chyba v kóde).

Päť nálezov R1124og, ktoré review uviedol ako čiastočne opravené alebo s regresiou, má nový nález:
PG-900 → PG-1000, PG-902 → PG-1001, PG-905 → PG-1002, STEP-900 → STEP-1000 a STEP-1001,
DXF-902 → DXF-1001.

## P1

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1125og-PG-1000 | opravené | #237 | Spevnenie na jednej skrutke (alebo na skrutkách na jednom mieste) pozná normálu dotyku: je votknuté len vtedy, keď je moment okolo normály zanedbateľný a moment okolo osi v dotyku sa zmestí do hrúbky tenšieho dielu; inak sa okolo skrutky otočí. Sonda `t80_off75` je test: predtým 194,8 N/mm a pass, teraz 29 N/mm a nosník neprejde. Skrutka na priamke sily ostáva votknutá. |

## P2

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1125og-PG-1001 | opravené | #238 | Spevnenie sa vzpiera pre každú kombináciu MSÚ s jej vlastnou F_d a jej k_mod; kontrola aj výpočet menujú kombináciu a k_mod najbližšie k vzperu. Pár sond, kde pridané úžitkové zaťaženie menilo exit 3 na pass, je test (`brace_load_duration.rs`): 1,35 G sám vzperie pás pri 207,133 N proti F_d 228,454 N a s malým Q už neprejde. |
| R1125lo-1 | opravené | #238 | Ten istý nález ako PG-1001 (sondy `p07_s20_g540_q0` a `_q60`). |
| R1125og-PG-1002 | opravené | #243 | V pribalenom starlarku (`PATCH.md`): `str()`/`repr()` nad hĺbkou `MAX_JSON_DEPTH` napíše `...`, rovnako správa `fail()` a chýbajúceho kľúča; hash kľúča slovníka má rovnakú stráž zásobníka ako `==` a `<`, takže hlboká n-tica ako kľúč je chyba na riadku. Test `deep_values.rs`: zoznamy, n-tice, slovníky a štruktúry vnorené 10^6 končia `...`; na `main` pretiekol zásobník. Všetkých 40 sond recenzenta D skončí s exit 0 alebo 2 so správou a riadkom. |
| R1125lo-2 | opravené | #243 | Ten istý nález ako PG-1002 cez `fail()`, `{}[x]` a `str(struct(...))` pri hĺbke 10^6: predtým 0xC00000FD bez výstupu, teraz chyba na riadku 4. |
| R1125og-STEP-1000 | opravené | #240 | Teleso kúpeného dielu v `component()` sa nájde aj podľa lokálneho výskytu v komponente, nielen v koreni dokumentu, takže sa dá apply-nuť znova. Test `a_bought_part_in_a_component_can_be_applied_again` padal na `03e52dec` s `program_import_failed`. |
| R1125og-STEP-1001 | opravené | #240 | Teleso pomenované holým menom súboru (dokumenty spred #228) patrí programu; `./` a `\` menujú tú istú cestu a na Windows sa cesta porovnáva bez ohľadu na veľkosť písmen. Iný priečinok v mene telesa sa stále odmietne (test R1124og-STEP-900 bez zmeny). Test `a_body_named_by_the_bare_file_name_stays_the_programs` padal na `03e52dec`. |
| R1125og-DXF-1000 | opravené | #241 | Vlastník pridaných výplní a čiar je handle záznamu `*Model_Space`, inak prvé `330` prvej entity mimo jej skupín 102. Entita vlastnená niečím, čo nie je záznam bloku (tak písal export pred opravou), sa v tabuľke poradia ráta do model space. Test `entities_added_are_owned_by_model_space_when_the_first_entity_is_in_a_group` (AC1015, AC1032) padal s vlastníkom `3C` (GROUP); jeho druhá polovica (opätovný export) padne, ak sa opraví len vlastník. |
| R1125og-DXF-1001 | opravené | #239 | Súbor s blobom podkladu verzie, ktorú žiadne vydanie nekreslí (`KUNDRLY7`), sa zapíše s príznakom `LostByReleasedReaders`, teda ako kontajner schémy 3, ktorý každé vydanie odmietne ako novší (mechanizmus REL-404). Súbor len s čiarami ostáva schéma 1. `scripts/release_reads.py` dáva každému vydaniu verzie blobov, ktoré kreslí, a čítanie vydaním zlyhá na súbore s novším blobom. Test `underlay_released_readers.rs` (výkres z ezdxf so štýlom Standard na `txt` a jedným TEXT) padal so schémou 1. |
| R1125og-DXF-1002 | opravené | #242 | `ketchup-dwg` (`draw_order.rs`) pred zápisom DXF z DWG dá do tabuľky poradia každú entitu priestoru, ktorý tabuľku má, podľa jej starého triediaceho alebo vlastného handle; nové handle zapisovača acadrust potom zachovajú celé poradie. Test: šrafa poslaná dozadu po DXF → DWG → DXF bola na `main` navrchu (`LWPOLYLINE, LINE, TEXT, HATCH`), teraz `HATCH` prvá ako v zdroji. |

## P3 (§4) po skupinách

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1125og-PG-1003 | opravené | #244 | Cesta, ktorá tlačí cez holý dotyk šikmo k sile, sa na ňom rozdelí (`loads/anchorage/bare.rs`): zložka pozdĺž normály ide cez dotyk, zvyšok tak, ako diely držia bez neho. Test pri β = 0,2°, 2°, 30° a 60° dáva k / cos²β; predtým 3,708 N/mm proti 0,588. |
| R1125og-PG-1004 | opravené | #238 | Spevnenie z triedy pevnosti bez produktu nechá prvok neoverený s tým istým dôvodom ako prvok z nej, namiesto k_mod / γ_M rastlého dreva. |
| R1125og-PG-1005 | opravené | #244 | Odmietnutie steny stojacej bez spoja na doske menuje dvojice, ktoré sa len opierajú („slab and wall only bear on each other … fasten them with joint()“), nie „through nothing besides beam“. S `joint(wall, slab)` spevnenie drží. |
| R1125og-PG-1006 | opravené | #246 | Dva kvádre jednej orientácie zdieľajú aspoň objem prekrytia mínus všetko, čo môžu odobrať ich otvory, kapsy a odčítané kvádre; nad 1 mm³ je to `collision` (exit 1) „overlap by at least N mm³“ (`certain_overlap.rs`). Dosky, ktorých otvory môžu spoločný objem odobrať, ostávajú neoverené. Exit 3 pri každej neoverenej kolízii sme nezaviedli: istú kolíziu teraz rozhodnú kvádre a zvyšok naozaj potrebuje presné telesá. |
| R1125og-PG-1007 | opravené | #246 | Dvojica z `alternatives()` má nový stav `alternatives`, polovice rozdeleného dielu `removed` (obe predtým `boxes_only`); `docs/program-language.md` stav uvádza. |
| R1125lo-3 | opravené | #243 | `enumerate`, `sorted` a `reversed` nad `range(10**9)` overia budovaný zoznam voči hranici haldy ako `list(iterable)` a odmietnu ho s „program memory limit exceeded“ na riadku 1, nie panikou (exit 101, `AlignedSize`). |
| R1125lo-4 | opravené | #246 | Test `turned_and_swept_parts_are_measured_only_against_the_parts_near_them` (a test kolízií kúpených dielov) kontroloval odpoveď apply pred koncom presnej kontroly; teraz ju sleduje cez `settled_exact_check` ako klient. Pod záťažou sa chyba naschvál vyvolať nedá, overené je len, že test prejde. |
| R1125og-STEP-1002 | opravené | #240 | Apply, ktoré nahradí dokument bez priečinka, novému dokumentu priečinok predchádzajúceho neodovzdá (`ProgramFolder::given_by_request`). |
| R1125og-STEP-1003 | opravené | #240 | Súbor STEP sa pri jednej požiadavke prečíta a zahashuje raz, nech program umiestni koľkokoľvek jeho mien `part=`. Test so zostavou troch dielov z jedného súboru čakal jeden hash; na `03e52dec` boli tri. |
| R1125og-DXF-1003 | opravené | #241 | Export pôdorysu (R12) píše `$MEASUREMENT 1`; `ketchup-dwg::read_dxf` nastaví `measurement = 1`, keď je `$INSUNITS` metrické a výkres `$MEASUREMENT` neuvádza. Palce a výslovné `$MEASUREMENT 0` ostávajú 0. |
| R1125og-DXF-1004 | opravené | #241 | Nová `SORTENTSTABLE` vo výkrese, ktorý ju nemal, dostane záznam `CLASS` (`SORTENTSTABLE / AcDbSortentsTable / ObjectDBX Classes`) v sekcii CLASSES výkresu alebo v novej pred TABLES, raz, nie znova pri druhom exporte. AutoCAD neoverený (S). |
| R1125og-CAM-1000 | opravené | #245 | Dialekt ISO sa volá „ISO 6983 metric G-code with FANUC-style G43 and G28“ a jeho zdroj hovorí, že z ISO 6983 je len formát slov a na Heidenhain ISO (G28 = zrkadlenie) treba vlastný dialekt; FANUC a LinuxCNC uvádzajú referenčný bod Z hore (`G53 G0 Z0` predpokladá Z navádzané do 0 hore). Popis MCP `cam_dialect` hovorí to isté. Test: každé `G28`, `G53` a `G43`, ktoré dialekt knižnice píše, má zdroj s „Assumes“; padal na `iso_metric_gcode`. |

## §5 Čo review nekontroloval

Body §5 (OCCT, AutoCAD/BricsCAD, nočný beh) nie sú nálezy; nič sa nemenilo. Toky STEP, ktoré review
uviedol len ako K, majú po #240 testy v `ketchup-application`.
