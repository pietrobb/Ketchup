# Review 2026-11-24 (og): stav opráv

Review (`docs/archive/reviews/review-2026-11-24-og.md`, PR #223) bol robený na `739df631`.
Opravy išli v poradí podľa §7: P1, potom P2, potom P3 po skupinách, každá v samostatnom PR s
testom, ktorý bez opravy padne. Záznam v `CHANGELOG.md` (Unreleased) priniesol každý PR okrem #224
a #225; ich záznamy (PG-900, PG-901, PG-902, PG-903 + PG-904) pridal PR s týmto stavom.

Stavy: **opravené** (zlúčené na `main`), **odložené** (s dôvodom), **bez kódu** (zistenie, nie
chyba v kóde).

## P1

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1124og-PG-900 | opravené | #224 | Kĺb spevnenia s ukotvením sa meria od spojovacích prvkov, ktoré ich spájajú, nie od stredu dotyku. Doska na jednej skrutke 140 mm od priamky skrutky prúta predtým platila za votknutú a držala nosník 215 N/mm namiesto asi 13–28; pásik so skrutkou na každom konci sa vzpieral len po stred dotyku (283 N namiesto 186). |
| R1124og-PG-901 | opravené | #224 | Holý dotyk dvoch uzemnených dielov drží len tlak naprieč dotykom, nie posun pozdĺž neho; odmietnutie menuje dvojicu a radí `joint()`. Kváder pri čele stĺpa ťahaný pozdĺž neho predtým držal 184 N/mm namiesto 0,665. Sondy z review `d_side_block` a `h_stack` sú testy (capability review 1031): predtým 183,985 / 196,868 a 273,067 N/mm, teraz stĺp sám 0,665 N/mm a nosník neprejde. |

## P2

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1124og-PG-902 | opravené | #224 | Vzper cesty spevnenia k zemi sa počíta s návrhovou pevnosťou f_c,0,d = k_mod · f_c,0,k / γ_M dielu, ktorý sa vzpiera, pri triede trvania kombinácie, ktorá určuje F_d. `held_by`, dôvody aj základ výpočtu píšu k_c f_c,0,d A. `e_strip14` a `e_strip15` už neprejdú. |
| R1124og-PG-903 | opravené | #225 | Za 2 000 neoverenými dvojicami hľadanie kolízií pokračuje bez ich vypisovania a preskočí len dvojice, ktoré by rozhodli až presné telesá. `cap_hides.star` nájde kolíziu `p` a `q` a skončí s exit 1, nie 0. |
| R1124og-PG-904 | opravené | #225 | Dvojica, ktorú kontrola nevypísala, má vzťah `unchecked`, nie `boxes_only`; vzťahy nad 20 000 prekrytí pridajú varovanie `relations_truncated`. |
| R1124og-PG-905 | opravené | #230 | V pribalenom starlarku (`PATCH.md`): `json.encode` odmietne kontajnery vnorené hlbšie ako 200 s chybou na riadku volania; trasovanie haldy pri kopírovaní nad hĺbku 256 hodnoty odloží a dotrasuje ich na mieste, takže každé vnorenie sa skopíruje na ohraničenom zásobníku. JSON hĺbky 1 000 je chyba na riadku 4, reťaze zoznamov, n-tíc a slovníkov hĺbky 600 000 a 300 000 vnorených štruktúr sa vyhodnotia; worker v okne odpovie chybou a zostane bežať. Testy bez opravy padnú na `STATUS_STACK_OVERFLOW`. |
| R1124og-STEP-900 | opravené | #228 | Prvok, ktorý drží teleso kúpeného dielu, je pomenovaný cestou z programu a `part=`; bez priečinka programu dostane diel teleso dokumentu len pri presnej zhode, náhrada podľa holého mena súboru je zrušená a inak sa apply odmietne s „apply the program from its file“. Staré dokumenty so súbormi pri programe fungujú ďalej. |
| R1124og-STEP-901 | opravené | #228 | Priečinok, ktorý dalo apply, okno zabudne pri otvorení alebo začatí iného dokumentu; apply, ktoré samo dokument nahradí, ho nechá pre dokument, ktorý vytvorilo. |
| R1124og-STEP-902 | opravené | #227, #229 | Kúpený diel, ktorého súbor ešte nebol načítaný, je v statike v `loads.unassigned` a v `missing` každého prvku (`not_verified`), bez falošného `floating_part` (#227). Nemá vzťah ani kontrolu spoja, varovanie `import_unread` menuje diel a súbor, CLI skončí s exit 3; headless `program_check` postaví zvyšok a diel uvedie v `exact.not_evaluated` (`import_unread`); MCP `program check` so `source_path` odovzdá priečinok `.star` súboru (#229). Dokumentácia kúpených dielov je spresnená. |
| R1124og-DXF-900 | opravené | #231 | Export zapíše poradie kreslenia model space (`ACAD_SORTENTS`, alebo prepíše tabuľku výkresu): výplne miestností dostanú najnižšie triediace handle a výkres si zachová svoje poradie. Výplne berú handle pred kópiami a čiarami pridanými tým istým exportom. Nerobí sa: výkres pred DXF 14 alebo bez sekcie OBJECTS či záznamu `*Model_Space` má výplne len prvé v súbore. |
| R1124og-CAM-900 | opravené | #226 | ISO a FANUC končia `G91 G28 Z0` / `G90` pred `G49`, LinuxCNC `G53 G0 Z0`. Test: každý dialekt s `G43` po poslednom pohybe dôjde v Z na referenčný bod stroja pred akýmkoľvek `G49` a nekončí v `G91`. |

## P3 (§5) po skupinách

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1124og-PG-906 | opravené | #232 | Každý diel na ceste ukotvenia sa počíta so šmykovou deformáciou L / (5/6 · G · A) popri ohybe, ako spevnenie. |
| R1124og-PG-907 | opravené | #234 | Sily a zaťaženia sa vo výpočte tlačia na newton: spevnenie s 259 N proti 263 N ukáže „0,259 kN“ / „0,263 kN“, nie „0,26 / 0,26“. Zaokrúhľovanie únosnosti nadol a sily nahor sme zamietli, lebo spevnenie, ktoré prejde o zlomok newtonu, by vyzeralo ako neprejdené. |
| R1124og-PG-908 | opravené | #232 | `time_limit` v CLI menuje termín 60 s, bežné príčiny (napr. tisíce dielov blízko seba) a čo zmeniť; `docs/program-language.md` ho uvádza medzi kódmi exit 2. |
| R1124og-PG-909 | opravené | #232 | Worker, ktorý zastaví spoločný termín kontroly, menuje ten termín, nie vlastný čas. Štart workera, ktorý neodpovie do svojich 30 s, je `worker_failed` „did not answer within 30 s of starting; no program ran“; za termín kontroly sa hlási len vtedy, keď štart skrátil práve on (capability review, vyhodnotenie aj report/refine). |
| R1124og-STEP-903 | opravené | #228 | Vyrovnávacia pamäť súborov STEP v okne hashuje bajty pri každom čítaní; súbor s inými bajtmi, ale rovnakou dĺžkou a časom sa načíta znova. |
| R1124og-STEP-904 | opravené | #227 | Načítaný kúpený diel váži `evidence.volume_mm3`, nie obalový kváder. |
| R1124og-VW-900 | opravené | #232 | Body pre prichytenie nástroja Measure vo Viewri sa držia raz pre sieť, nie pre každú kópiu. |
| R1124og-DXF-901 | opravené | #231 | Listy majú `$MEASUREMENT` = 1 v DXF aj DWG, takže neskôr načítané typy čiar a šrafy idú z metrických súborov. |
| R1124og-DXF-902 | opravené | #233 | Porovnanie revízií hlási text s iným písmom a hrubšie nakreslenú LWPOLYLINE ako zmenu štýlu (`Style`). Formát uloženého podkladu prejde na verziu 7 len pri výkresoch, ktoré to potrebujú. |
| R1124og-FEA-900 | opravené | #232 | Spoj, ktorého siete sa stretnú len pozdĺž čiary, sa odmietne vlastnou chybou; pribudol test čiastočného prekrytia (dva bloky posunuté o polovicu dĺžky). Stav bodu 2 v `review-2026-11-19-og-status.md` už neuvádza „0.342 mm“ (nevychádza z testu), ale čo test overuje: zlepené vrstvy sa ohnú ako jeden kus tej istej siete (0,197 mm na 20 bunkách, vzorec nosníka 0,381 mm; tuhšia je hrubá sieť, nie spoj). |
| R1124og-CAM-901 | opravené | #232 | Výsledok CAM v `not_checked` hovorí, že priestor pod priechodným otvorom sa kontroluje len voči upnutiu deklarovanému v pláne (R3). Stav bodu 5 v `review-2026-11-19-og-status.md` je opravený: prevŕtanie obmedzuje len dĺžka britov. |
| R1124og-CI-900 | bez kódu | — | Joby na macOS pri pushi `739df631` nezrušil človek ani padajúci krok: GitHub pre ne nenašiel runner („job was not acquired by Runner“) a po 15 min ich zrušil. Rovnako čakali joby na macOS PR #226 v tejto sérii. |

## Poznámka bez nálezu

`busy` pre druhého klienta počas kontroly na pozadí až po `PROGRAM_BACKGROUND_CHECK_TIME` (300 s)
review neoznačil ako nález; nič sa nemenilo.
