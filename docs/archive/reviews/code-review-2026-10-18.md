# Podrobný code review Kečupu: celý strom na `dde47a4` (0.2.0)

Stav `main` 2026-10-18, vo verzii 0.2.0. Nadväzuje na `code-review-2026-10-17-full.md`
(`9163252`). Odvtedy pribudlo 79 commitov. Takmer každý z nich rieši niektorý nález z minulého
review.

## 1. Metóda

- Sedem nezávislých čitateľov nad celým stromom. Každý najprv overil opravy vo svojej
  oblasti a potom urobil čerstvý review:
  1. model a perzistencia;
  2. program, EC5 a gravitácia;
  3. exact a workery;
  4. GUI;
  5. MCP, pluginy a headless;
  6. výroba a výkresy;
  7. viewer, CI, testy a vydanie.
- SEC-5 je rozhodnutie vlastníka a nehlási sa.
- **Overenie:**
  - **K**: mechanizmus som sám overil čítaním kódu;
  - **P**: čitateľ ho reprodukoval sondou nad skutočným crate, mimo repozitára;
  - **S**: statický nález čitateľa so stopou od volajúceho.
- Spustené na `dde47a4`:
  - všetkých 11 `scripts/check_*.py` vrátane nového `check_test_timing.py`: prešli;
  - lokálne Linux job z CI (`fmt`, `clippy`, testy 15 crate): prešiel, 1 370 testov,
    0 zlyhaní.
- Priorita:
  - **P1**: zlý statický alebo výrobný výsledok, strata dát, zamrznutie pri bežnej práci;
  - **P2**: konkrétna chyba s menším dopadom;
  - **P3**: dlh alebo hygiena.

## 2. Záver v skratke

**Opravy z minulého review:** všetkých päť P1 (PG-1, PG-2, M-1, MF-5, EX-2) má opravu a
test. Z P2:

- **úplne opravené:** M-4 až M-9, MF-6, MF-7, EX-3, EX-5, EX-7, PG-3, PG-5, PG-6, UI-8,
  UI-9, MCP-1, MCP-2, VW-12, TST-2, TST-4, DOC-1;
- **čiastočne opravené:** M-2, M-3, PG-4, PG-7, EX-2, EX-6, SEC-7, VW-13, TST-3, PS-11.

Kontrola gravitácie teraz správne rozlišuje uloženie zhora od bočného kontaktu. Overené
prípady: sedlo krokvy, doska na rampe 30° aj diel zavesený cez spoj. Nová cesta kolízie cez
konvexné kúsky nikde neprehliadla prienik.

Nízke body z minulého review v GUI, výrobe a dokumentácii zostali väčšinou otvorené.

**Nové vážne nálezy (P1):**

1. **Statika:**
   - nosník nakreslený z boku, so zárezom v profile, sa posudzuje na najväčšej výške
     (PG-8);
   - nosník pootočený okolo osi cez `place()` sa posudzuje, akoby zaťaženie šlo pozdĺž jeho
     výšky (PG-9).
2. **Model:**
   - rez „po plochu“ sa po úprave telesa, na ktoré ide, už nikdy nezostaví; oprava M-2 je
     neúplná (M-10);
   - Make Unique stratí závity dier (M-11).
3. **Výroba:** diel bez výrobnej role alebo s preklepom v role vypadne z kusovníka a zo
   všetkých výrobných formátov bez hlásenia (MF-8).
4. **Program:** `struct(**D)` obíde limit 1 GiB (PG-10).
5. **Exact:** každé vyhodnotenie importovaného dielu parsuje zdrojový STEP trikrát (EX-8).

## 3. Overenie opráv z minulého review

| ID | Stav | Poznámka |
|---|---|---|
| PG-1 | opravené pre operácie | Rez, kapsa, boolean aj veľká diera dajú `not_verified`. Zárez nakreslený priamo v bočnom profile pozri PG-8. |
| PG-2 | opravené | Plochy sa držia podľa vonkajšej normály, do 60° od gravitácie. Zvyšok: aj svah 59° sa berie ako uloženie (P3). |
| PG-3, PG-5 | opravené | Text BASIS ešte opisuje staré vynechanie koncov. |
| PG-4 | čiastočne | `material_along` sleduje len os diery. Pozri PG-11. |
| PG-6 | opravené | V·Q/(I·b) a nesymetrický ohyb sú správne. Pozri PG-9 a P3 k stredu šmyku. |
| PG-7 | čiastočne | Slovníky a množiny sa počítajú. `struct(**D)` pozri PG-10. |
| P3-S (zaokrúhlenie, dve diery, šikmý stĺp) | opravené | |
| M-1, M-4–M-9 | opravené | M-7: uloženie cez požadovanú cestu používa `Replace`, nie `IfUnchanged`. M-9: pri zápise sa meria kompaktný JSON, pri čítaní surové bajty. |
| M-2 | čiastočne | Premenovanie dôkaz plochy prežije, úprava telesa nie (M-10). |
| M-3 | čiastočne | **K**: súbor, rozšírenia aj zámok ostanú, ale `*document = replacement.document` (`session.rs:525-528`) vloží históriu `[prázdny, program]`. Undo po nahradení vedie do prázdneho dokumentu, nie späť k otvorenému súboru. |
| PS-11 | čiastočne | Každá dávka ďalej klonuje celý `ProductModel` (`store.rs:1194`). `CreateDrawingSheet` a `UpdateDrawingSheet` klonujú raz na príkaz. |
| MF-5, MF-6, MF-7 | opravené | MF-5: logický kolík na nakupovanom diele zlyhá vo výrobnom jobe všeobecnou chybou `ExportBlocked`. MF-6 pozri MF-10 a MF-11. |
| EX-2 | čiastočne | Rozpočet podľa bajtov a dielov je rozumný (strop 15 min). Problémy:<br>• inšpekcia STEP a IGES má `parts = 0`;<br>• `ConvertStepXdeToIges` sa ráta ako jeden diel (EX-9);<br>• graf s importovanými zdrojmi má pevných 60 s;<br>• inšpekcia jedného dielu parsuje dvakrát;<br>• počítadlo v teste nevidí opakované čítania v `native_io.cc:723`. |
| EX-3, EX-5, EX-7 | opravené | EX-7: `PR_SET_PDEATHSIG` správne nie je na workeroch z poolu (spúšťajú sa z krátko žijúcich vlákien). Tie namiesto toho sledujú rodiča. |
| EX-4 | opravené | Výstup OCCT ide na `cerr`. Priamy zápis na fd 1 by rámce ešte poškodil. Stderr exact workera ide do `Stdio::null`, takže diagnostika OCCT sa zahodí. |
| EX-6 | čiastočne | V GUI zrušenie funguje a neostane rozpísaný súbor. Export cez live bridge a záložná cesta v `dispatch_command` bežia stále synchrónne na UI vlákne (EX-10). |
| P3-E | opravené | Výnimka: identita workera kryje len Windows `TK*.dll`. Na Unixe ide OCCT cez rpath, takže kontrola nič nerobí. |
| UI-8 | väčšinou opravené | Náhľad sa ukáže, ale píše sa s bodkou („= 1.2 mm“) aj na čiarkovom systéme. Dialógové polia náhľad nemajú, takže `1.200` tam ďalej potichu znamená 1,2 mm. |
| UI-9, exponent v FEA | opravené | |
| Nízke body GUI | väčšinou otvorené | Otvorené ostali:<br>• tiché odmietanie čiarky v dialógoch;<br>• Revolve číta uhol ako dĺžku;<br>• `×` a `x` znamenajú rôzne veci;<br>• `str::parse` v počtoch;<br>• `hole_spec` delí podľa medzier;<br>• lokálne nastavenie na macOS;<br>• skratka pri pomalej snímke;<br>• `GESTURES` má len 6 nástrojov. |
| SEC-7 | čiastočne | Všetky tvary z review sa odmietnu, prostredie je prázdne a stage je mimo `%TEMP%`. Zvyšok pozri SEC-8. |
| MCP-1, MCP-2, `QUERY INPUTS` | opravené | |
| Nízke body MCP | otvorené | Otvorené ostali:<br>• zaplnenie endpointu pripojenia;<br>• `list` bez dôkazu prezradí názov dokumentu;<br>• export s dialógom vydania pokračuje aj po odpojení klienta;<br>• veľkosť vstupov pluginu sa nekontroluje pred súhlasom;<br>• `docs/plugins.md` a schéma sa nezhodujú s kódom (teraz ešte viac);<br>• chýbajúci obrázok sa vráti ako úspech. |
| VW-12, TST-2, TST-4, DOC-1 | opravené | TST-2: `test_mcp_live_measurement.py` (14 testov) stále nebeží nikde. |
| VW-13 | čiastočne, s regresiou | Pozri VW-14. |
| TST-3 | väčšinou | Pozri TST-5. `check_test_timing.py` sa dá obísť viacerými spôsobmi (P3). |
| Nízke body výroby a výkresov | otvorené | Žiadny P3 commit v tejto oblasti. Otvorené ostali:<br>• kóta na čiastočne skrytej hrane;<br>• priechodné diery v BTLx a woodWOP;<br>• názvy vrstiev a text v R12;<br>• čiary po valcoch pri 20°;<br>• diel, ktorý môže z projektového výkresu potichu vypadnúť;<br>• DXF v `ANSI_1250`;<br>• ToUnicode v PDF. |
| Nízke body CI a dokumentácie | väčšinou otvorené | Otvorené ostali:<br>• `check_ci_crates` uzná komentár v YAML;<br>• `check_test_sleeps` neskenuje `viewer/*`;<br>• fuzzing má 2 ciele bez korpusu;<br>• štart Androidu beží na UI vlákne;<br>• README (Undo, formáty);<br>• `program-language.md`;<br>• ADR 0002 a 0004;<br>• `viewer/MAC_*.md`.<br>Opravené: popis bielej listiny v `plugins.md`. |

## 4. Nové nálezy

### P1

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| PG-8 | K, P | `ketchup-program/src/member_check.rs:821-845`, `:1316-1340` | **Nosník z bočného pohľadu sa posudzuje na najväčšej výške.**<br>• Vetva bočného pohľadu nastaví `h = thickest`, `area = b·h`, `modulus = b·h²/6` a `inertia = b·h³/12`. `depth_at` sa používa len pre `shear_notch` pri uložení.<br>• Nosník `beam()` 100×200 so zárezom hlbokým 100 mm v profile v strede rozpätia vyjde `pass` s využitím 0,348, rovnako ako bez zárezu. Skutočné využitie je ~1,39.<br>• To isté platí pre sedlá krokiev nad vnútornou podperou; `tiny-house.star` ich robí do R/3. | W, I a šmykovú plochu brať v každej stanici z `depth_at` a priehyb integrovať s premenlivým I. Alebo `not_verified`, keď sa výška mení mimo uložení. |
| PG-9 | P | `member_check.rs:735-761` | **Pootočený nosník sa posudzuje, akoby zaťaženie šlo pozdĺž jeho výšky.**<br>• `depth` je len „zvislejšia“ lokálna os. Zvislé zaťaženie sa nikdy nerozloží do skutočnej orientácie prierezu.<br>• Väznica 100×200 pootočená o 30° cez `place()`: `pass`, ohyb 0,555, w_inst 0,59.<br>• To isté teleso nakreslené otočené priamo v profile: `fail`, 0,975 a 1,085. | Prierez stavať vo svetových zvislých a vodorovných osiach (profil otočiť podľa umiestnenia), aby ho riešil voľný ohyb. Alebo `not_verified`, keď `1 − |world[depth][2]|` prekročí toleranciu. |
| M-10 | S | `ketchup-model/src/document/planar_face.rs:102-140`, `store.rs:620-640`, `:723-741`, `snapshot.rs:280-287`, `compiler.rs:973-976` | **Rez „po plochu“ sa po úprave telesa už nezostaví.**<br>• `rebind_planar_face_reference` prepája len podpory Workplane a `CutStart::Support`; `FeatureExtent::UpToFace` nie.<br>• Pad A, rez B `UpToFace(A.top)`, zmena hĺbky A: línia sa označí za zastaranú a nová registrácia má nové digesty.<br>• B drží starú štruktúru a porovnanie `evidence == reference` cez odvodené `PartialEq` zlyhá. Výsledok je natrvalo `UnresolvedExtent`.<br>• Test M-2 len premenúva, takže to nezachytí. | Jeden návštevník odkazov nad všetkými `FeatureKind`, použitý pri prepájaní, kontrole konfliktov aj označovaní zastaraných. Test: úprava producenta, nová registrácia, rez sa zostaví. |
| M-11 | K | `ketchup-model/src/document/solid_tool.rs:219-660` | **Make Unique stratí závity.**<br>• `clone_definition_and_repoint` premapuje prvky, telesá, potlačenie aj väzby parametrov. `hole_threads`, `shaft_threads` ani `feature_parameter_provenance` nespomína.<br>• Diel so závitovými dierami M8 po Make Unique má obyčajné diery. Zmení sa výkaz závitov, plán vŕtania aj modelovaný závit, bez hlásenia.<br>• Cesta mazania tieto tabuľky pozná (`store.rs:142-149`), cesta klonovania nie. | Všetky tabuľky viazané na prvok kopírovať cez mapu prvkov. Test triedenia tabuliek ako `part_references.rs`. |
| MF-8 | K | `ketchup-manufacturing/src/fabrication.rs:2568-2582`, `:2918-2924` | **Diel bez platnej výrobnej role vypadne zo všetkých formátov bez hlásenia.**<br>• Diel bez klasifikácie `ketchup.fabrication-role.v1` aj diel s neznámym názvom kategórie (`_ => continue`) sa preskočí.<br>• `made_items` potom filtruje podľa `bom_metadata`, takže diel nie je v BOM, BTLx, woodWOP, jobe ani na dielenskom výkrese.<br>• Ručne dokreslená polica v skrini z programu sa tak nikdy nevyreže. | Neznámy názov roly je `InvalidBomMetadata`. Každý zobrazený diel s telesom bez roly export odmietne, alebo ho aspoň vymenuje. |
| PG-10 | P | `third_party/starlark/src/values/types/structs/value.rs:105`, `ketchup-program/src/eval.rs:2385` | **`struct(**D)` obíde limit 1 GiB.**<br>• `Struct.fields` je `SmallMap` mimo arény a `OutsideBytes` ho nenavštívi.<br>• `S = [struct(**D) for _ in range(300)]` pri 200 000 kľúčoch prejde s RSS 1,9 GB a rastie lineárne. Ten istý riadok s `dict(**D)` sa odmietne.<br>• `partial(f, **D)` dosiahol 1,85 GB, kým sa odmietol. | Rátať mapy `struct` a `partial` v `OutsideBytes`. Lepšie: počítajúci alokátor na vlákno (návrh 3). |
| EX-8 | S | `ketchup-application/src/evaluation.rs:1005,1068`, `ketchup-scheduler/src/lib.rs:933,1283`, `exact_worker.rs:3438,3497` | **Vyhodnotenie importovaného dielu parsuje celý zdroj trikrát.**<br>Opätovné otvorenie dokumentu so 100-dielnym STEP s 30 MiB znamená 300 úplných behov `ReadFile` a `Transfer`. Každá požiadavka sa zmestí do rozpočtu, takže nič nevyprší, ale trvá to desiatky minút. Je to pôvodný problém EX-2, presunutý do vyhodnocovania. | LRU dokumentov XDE vo workeri podľa sha256 zdroja, alebo jedna požiadavka na zdroj, ktorá vráti všetky diely. |

### P2

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| EX-9 | S | `ketchup-scheduler/src/request_budget.rs:52`, `exact_worker.rs:3335-3385`, `native_io.cc:723`, `exchange_export.rs:76` | **Export IGES veľkého modelu má malý rozpočet.**<br>`ConvertStepXdeToIges` sa ráta ako jeden diel, ale worker spúšťa `export_step` pre každý výskyt a výstup potom dvakrát parsuje. Dom so 750 výskytmi (~10 MiB) dostane ~57 s a skončí `RequestTimedOut`. | Posielať počet výskytov a každý jedinečný diel exportovať raz. |
| EX-10 | S | `live_bridge/interchange.rs:189`, `app/document.rs:1017-1025`, `exchange_export.rs:178` | **Export cez live bridge a z menu zamrazí okno.**<br>Volajú `export_exchange_to`, ktorá úlohu spúšťa na UI vlákne s `AtomicBool::new(false)`. Bridge má príznak zrušenia, ale exportéru ho neodovzdá. Rozpočet EX-2 (až 15 min) tento čas ešte predĺžil. | Cez `queue_exchange_export` s príznakom bridge. |
| EX-11 | S | `native_common.cc:64-98`, `native_finish.cc:125-185` | **Mapovanie histórie v natívnom kóde je O(H·F).**<br>Mapa tvarov sa stavia nanovo pre každý záznam histórie. Shell alebo fillet na telese s tisíckami plôch tak robí milióny vložení. | Mapy plôch a hrán postaviť raz na operáciu. |
| EX-12 | S | `ketchup-scheduler/src/lib.rs:472`, `child_process.rs:11`, `exchange_export.rs:48` | **Pevný rozpočet grafu a zvyšky po zrušení exportu.**<br>Graf s importovanými zdrojmi (až 128 MiB) má pevných 60 s. Na Windows zrušený export môže nechať `.ketchup-prepared-export-*` vedľa súboru používateľa. | Pridať bajty zdrojov do rozpočtu. Dočasný priečinok dať do systémového temp. |
| M-12 | S | `ketchup-model/src/persistence.rs:2626-2649`, `:1754-1758`, `:1016` | **Blob, ktorý používa už len história, sa po otvorení ukladá navždy.**<br>Import 30 MB STEP, zmazanie dielu, uloženie, otvorenie: odvtedy každé uloženie a checkpoint zapíše 30 MB. Niekoľko takých importov dosiahne `MAX_CONTAINER_BYTES` (64 MB) a potom každé uloženie zlyhá na `TooLarge`. | Označiť bloby odkazované každou načítanou revíziou, alebo počítať odkazy z revízií, ktoré sa zapisujú. |
| M-13 | S | `ketchup-application/src/session.rs:437`, `app/document.rs:1322`, `persistence.rs:1012-1043`, `pipeline.rs:58-64` | **Každá úprava znova hashuje a kopíruje všetky bloby.**<br>Checkpoint beží v každej mutácii. Pri 30 MB zdroji je to ~60 MB SHA-256 a ~90 MB kopírovania na každú úpravu, na UI vlákne. Pri `KeepIfFits` sa história, ktorá sa nezmestí, kóduje dvakrát. | Bloby sú adresované obsahom a overené pri vložení, takže ich netreba hashovať znova. Písať ich zo zdieľaného `Arc`. Checkpoint presunúť na pozadie (návrh 4). |
| PG-11 | P | `ketchup-program/src/material.rs:23-104`, `validate.rs:527-535`, `prelude.star` (`_drill_room`) | **Kontrola materiálu pri diere sleduje len os.**<br>Diera Ø12 hĺbky 12 mm na x = 96 v 18 mm doske s drážkou 6×10 mm na x = 100–106 zarezáva 2 mm do drážky, ale nedá žiadne varovanie. Na x = 103 dá `hole_breaks_through`. | Testovať celý valec vrtáka: lúče na obvode s polomerom r a hrot, alebo obrysy rezov a kapsy posunuté o r. |
| PG-12 | P | `ketchup-program/library/prelude.star:203-204` | **`member()` pri sklone ~25,8° potichu otočí prierez o 90°.**<br>Predvolené `across` sa prepne zo svetovej y na x, keď |dx| ≤ 0,9·L. Krokva 60×200 pri 25° stojí, pri 27° leží naplocho. Parameter sklonu nad 26° tak otočí všetky krokvy. | Predvolene `across = normalize(z × d)` a na svetovú x prejsť len pri zvislom prvku. |
| MF-9 | S | `ketchup-manufacturing/src/fabrication.rs:4384`, `:736`, `:1277`, `:2466` | **„Dĺžka“ v BOM je iná os než „Length“ v BTLx.**<br>`length_mm` je rozmer v X, BTLx berie dĺžku v Z. Pre prvok 60×140×3000 vytiahnutý v Z je v BOM dĺžka 60, šírka 140 a výška 3000, kým BTLx má Length 3000. | Rozmery BOM brať zo skladového rámca (zoradené ako `job_stock_frame`), alebo stĺpce pomenovať `x_mm`, `y_mm`, `z_mm`. |
| MF-10 | S | `ketchup-model/src/drawing.rs:3094-3131` | **Kóta na detailnom pohľade meria orezaný kus.**<br>`clip_lines_to_detail` skráti čiaru a nechá jej `stable_line_id`. Hrana 500 mm cez detailový kruh 40 mm dostane kótu ~80 mm. | Merať z neorezaných koncov, alebo kótu na orezanej čiare odmietnuť (návrh 6). |
| MF-11 | S | `drawing.rs:4093-4108` | **Listy zostavy spred MF-6 zlyhajú všeobecnou chybou.**<br>List uložený pred MF-6, alebo kópia zrkadlená až po vytvorení listu, má ľavú aj pravú kópiu na pozícii 1. Vznikne `InvalidAnnotation` bez nápovedy. | Pri načítaní pozície prečíslovať, alebo pomenované odmietnutie s ponukou prečíslovania. |
| MF-12 | S | `drawing.rs` (kľúč pozície) | **Kusovník výkresu zlúči diely, ktoré výrobný BOM rozlišuje.**<br>Dve kópie police z duba a borovice sú na výkrese pozícia 1, QTY 2, v BOM dva riadky. | Jeden spoločný kľúč položky (návrh 5). |
| MF-13 | S | `ketchup-model/src/import/dxf.rs:604`, `:2838-2841`, `ketchup-dwg/src/lib.rs:32` | **Import DXF a vrstvy a kóty.**<br>• Vrstvy vypnuté alebo zmrazené (farba < 0, 70 bit 1) sa importujú.<br>• Jedna kóta DIMENSION s chýbajúcim alebo prázdnym blokom `*D` zhodí celý súbor (`InvalidBlock`). | Čítať LAYER 62/70 a vynechať s počtom v diagnostike. Zlú kótu preskočiť s varovaním. |
| UI-10 | K | `ketchup-model/src/persistence/work_recovery.rs:11-25`, `ketchup-app/src/app/document.rs:200-215` | **Dokument v priečinku len na čítanie sa neotvorí.**<br>`WorkRecoveryLock::acquire` vytvára zámkový súbor vedľa dokumentu. Keď sa to nepodarí, otvorenie skončí ešte pred čítaním. Platí to pre zdieľaný priečinok len na čítanie, chránený USB kľúč aj pripojený DMG. | Bez zámku otvoriť bez work-recovery (alebo len na čítanie) a povedať to. Zámok držať v priečinku používateľa podľa absolútnej cesty. |
| UI-11 | S | `ketchup-app/src/app/project_drawings.rs:556-568`, `drawing_sheet_export.rs:171` | **Export projektového PDF a DXF/DWG listu beží na UI vlákne.**<br>Projekcia, odstránenie skrytých čiar a zápis bežia synchrónne, takže pri dome okno zamrzne bez možnosti zrušiť. | Cez `FileActivity` ako `exchange_export`. |
| UI-12 | S | `project_drawings.rs:253-293`, `:543-546` | **Predvolené hodnoty rohového razítka zamrznú v dokumente.**<br>Úprava akéhokoľvek poľa (napr. Autor) uloží aj názov projektu, názov listu v jazyku UI a *dnešný dátum*. Každé ďalšie PDF potom nesie ten dátum. | Ukladať len zmenené polia, predvolené dopĺňať pri exporte. |
| UI-13 | S | `ketchup-app/src/main.rs:101-107`, `app/document.rs:181`, `:313` | **Otvorenie zo spúšťacieho argumentu.**<br>• Relatívna cesta sa nikdy nezmení na absolútnu. Neskoršie Otvoriť toho istého súboru potom hlási „iné okno môže vlastniť neuloženú obnovu“.<br>• Neexistujúci súbor, priečinok alebo argument ako `--help` najprv vytvorí `<arg>.work-recovery-lock` a ten ostane.<br>• `.ketchup-view` dá všeobecnú chybu dekódovania bez odkazu na Viewer.<br>• Viac argumentov skončí `eprintln!` bez konzoly, teda bez viditeľnej reakcie.<br>• dde47a4 pridal len test, ktorý cestu cez argv neprechádza. | `std::path::absolute` v `main.rs`. Odmietnuť priečinky, prepínače a cudzie formáty s nápovedou v okne. |
| SEC-8 | S | `live_bridge/plugin_manifest.rs:299-327`, `plugin_consent.rs:122`, `:486` | **Ochrana príkazového riadku pluginu je stále čierna listina.**<br>• Pevný zoznam 11 dlhých a 8 krátkych prepínačov rovnaký pre každý interpret. Iné spúšťacie voľby, ktoré načítajú kód, prejdú.<br>• „Menuje skript“ znamená akýkoľvek hashovaný súbor v argv, nie súbor, ktorý interpret spustí.<br>• Hashuje sa len executable, nie interpret z `#!`.<br>• Ostáva len prípad toho istého používateľa a používateľ musí príkaz schváliť. | Štruktúrna biela listina pre každý interpret: `<interpret> [neškodné voľby]* <hashovaný skript> [slová]*`. Odmietnuť executable začínajúci `#!`. Alebo MCP formu príkazového riadku zrušiť. |
| MCP-3 | K | `live_bridge/drawings.rs:172`, `app/project_drawings.rs:390-402`, `drawing_sheet_export.rs:164` | **Export výkresov môže prepísať súbor, ktorý sľúbil neprepísať.**<br>`refuse_existing` sa skontroluje pred niekoľkosekundovou projekciou. Zápis potom berie `expected` z toho, čo je na ceste v tej chvíli, takže súbor uložený medzitým sa potichu prepíše. | Zapisovať s „nesmie existovať“ (`persist_noclobber`) ako `program_variant_export.rs`. |
| MCP-4 | S | `ketchup-headless/src/protocol.rs:158,173,212,1056,1427-1570` | **Headless zahodí obsah chýb (AGENTS §2).**<br>`Error::new(code, error.to_string())` a `Error::invalid(e.to_string())` zahodia príčinu. Ratchet `STRINGIFIED_VARIANT` chytá len konštruktory s veľkým písmenom. | `Error` s `Rejection` a `caused_by`. Rozšíriť regex. |
| MCP-5 | S | `plugin_consent.rs:104-121`, `ketchup-scheduler/src/plugin.rs:268-273` | **Forma príkazového riadku nemá strop pri hashovaní.**<br>Bez stropu na počet argumentov a veľkosť súborov. Čítajúci worker nekontroluje zrušenie. | Stropy ako pri priečinku. Kontrola zrušenia medzi súbormi. |
| VW-14 | S | `viewer/android/src/replace.rs:364-370`, `files.rs:339,382` | **Regresia VW-13: vzdanie obnovy pre dokumenty skoro nikdy nenastane.**<br>Každá chyba otvorenia cez SAF sa zabalí ako `io::Error::other`, nikdy nie `InvalidData`, a teda sa nepočíta. Zmazaný dokument alebo odobraté povolenie potom pri každom štarte hlási chybu a drží povolenie zápisu. Skorší obsah v `replacing-*.bak` „Uložiť skorší obsah ako…“ neponúkne. | `FileNotFoundException` mapovať na `NotFound`, `SecurityException` na `PermissionDenied` a počítať ich. Ponúknuť aj `replacing-*.bak`. |
| VW-15 | S | `scripts/windows/build-android-viewer.ps1` | **Aktualizácia na Androide môže zlyhať pre podpisový kľúč.**<br>Keystore je v `target\viewer-android\`. Po `cargo clean` alebo zostavení na inom stroji sa APK s `versionCode 2` nenainštaluje cez verziu 1. Odinštalovanie zmaže súkromné úložisko vrátane uloženej session a `kept-*.bak`. | Keystore mimo `target/` a kontrola odtlačku certifikátu v skripte. |
| REL-1 | S | `persistence.rs` (`CONTAINER_SCHEMA`), 0.1.0 `c56d771:persistence.rs:1427-1428` | **Riziko pri otvorení v staršej verzii: 0.1.0 a súbor z 0.2.0.**<br>0.2.0 píše `program.json` bez zvýšenia `CONTAINER_SCHEMA`. 0.1.0 berie neznámy záznam ako poškodenie a potichu otvorí `<súbor>.recovery`, teda obsah spred aktualizácie. Uloženie tam potom môže prepísať novší súbor. | V poznámkach k vydaniu: bez prechodu späť na 0.1.0. `CONTAINER_SCHEMA` zvyšovať s každým novým druhom záznamu. Golden test so súborom predošlého vydania. |
| TST-5 | K | `ketchup-scheduler/tests/scheduler_reliability.rs:41`, `:125-128` | **Rozpočet času stále beží pri každom PR.**<br>p95 zrušenia ≤ 250 ms cez 300 spustení workera v debugu na Windows runneri. K tomu `sleep` 2 ms a zápis metrík „gate B“. `check_test_timing.py` ho nezachytí, lebo čas prichádza z `worker.cancel()` ako `f64`. | Overiť len, že zrušenie nastalo. p95 presunúť do `perf_scheduler_cancellation` a `write_metrics` zmazať. |

### P3

- **Model:**
  - Dokument sa po zahodení najstaršieho Undo kroku (nad 200) ukazuje ako zmenený, lebo
    `history_digest` hashuje celú históriu.
  - Výrobné kódy sa prepočítavajú po každom príkaze s cenou O(hĺbka²), pri dome ~1,5 M
    rozlíšení na dávku (`store/commands.rs:171-178`).
  - Rozšírenia po M-3 môžu odkazovať na staré id definícií.
  - `SetFeatureDimension` ostal vedľa `SetFeatureParameter` (§6).
- **Statika:**
  - `Section::twists` porovnáva s absolútnym 1e-9 mm, takže symetrický I prierez pri
    súradniciach ~12 m vyjde `not_verified`.
  - Okraj dôkazu pri pohybe (`sweep_clear`) je väčší než kontaktná tolerancia v produkcii,
    takže sa nepoužije (konzervatívne).
  - Dokumentácia `member_check` v `prelude.star` hovorí len o stĺpoch.
  - `BEARS_ALONG_GRAVITY = 0.5` uzná svah 59°.
- **Exact:**
  - Ukončenie pri smrti rodiča volá `process::exit` (atexit, statické deštruktory OCCT), malo
    by `_exit`.
  - Po `prctl` chýba kontrola `getppid()`.
  - Chyby IGES hovoria „STEP XDE reader“.
  - Spustenie vyhodnotenia histórie prvkov zruší vyhodnotenie scény.
  - Identita workera na Unixe nekryje knižnice OCCT.
- **GUI:**
  - Zoznam vrstiev stavia 6–7 plánov na riadok pri každej snímke (`outliner.rs:263-350`).
  - Polia spojov a priestorov sa predvypĺňajú zaokrúhlené na 0,01 mm a pri každej revízii sa
    prepíšu (`joints_ui.rs:130-133`).
  - Nástroj Diera číta `M8x1.25` ako hĺbku 1,25.
  - Pravidlo predĺženia Pad cez Push/Pull sa líši medzi `transform.rs:3318` a crate
    interakcie.
  - Nastavenia listu obchádzajú `apply_batch`, takže Ctrl+Z ich nevráti.
  - Rozpis materiálu počíta len viditeľné diely, kovanie celý model.
  - Odmietnutia exact sú v slovenskom UI po anglicky.
- **MCP:**
  - `export_drawings` môže vrátiť `exported: true` bez uloženia rohového razítka
    (`project_drawings.rs:280-290`).
  - Výpis okien MCP môže zamrznúť na FIFO (`discovery.rs:181-183`).
  - Na Unixe politika ciest pustí `/proc` a `/dev/fd`.
- **Výroba:**
  - Projektový výkres v príliš malom formáte pretečie bez hlásenia.
  - Diel bez prvkov (nakupovaná položka len s metadátami) chýba v BOM bez hlásenia.
- **Viewer a web:**
  - Kontrola pôvodu odkazu na webe ignoruje presmerovania.
  - Neskoro dokončené stiahnutie prepíše lokálne otvorený súbor.
  - Komentáre sa berú ako uložené aj po zrušenom dialógu.
  - Štart na Androide beží na UI vlákne.
- **CI:**
  - `check_test_timing.py` obíde malé písmeno (`elapsed < budget`), obrátené porovnanie,
    pole štruktúry, `recv_timeout(D).expect`, pomocná funkcia `perf_*` alebo `mod` s
    odsadením.
  - c6f7c01 mení zlyhania na zaseknutie až do 180-minútového timeoutu jobu.
  - `build-*-viewer.ps1` nepoužíva `--locked`.
  - Neignorované testy používajú 750-dielny dom mimo `large_house_*`: `program_evaluation`,
    `takeoff_house`, `free_space`, `member_design`, `member_loads`.
  - `build-installer.ps1 -Version` sa neporovnáva s `Cargo.toml` a žiadny skript nevyrába
    Examples.zip.
- **Kvalita testov:**
  - Z 30 vzoriek 26 overuje skutočné výsledky.
  - Tri testy v `view-format/tests/packages.rs:383-508` overujú len `is_err()`.

## 5. Metriky

| | dnes |
|---|---:|
| `ketchup-app` / `ketchup-model` / `ketchup-application` | 166,9k / 140,5k / 59,1k riadkov |
| najväčší súbor GUI | `app/viewport.rs` 4 760 (limit 5 000) |
| produkčné funkcie GUI nad 150 / nad 300 riadkov | 30 / 3 (minule 36 / 4) |
| najdlhšie | `show_assembly_editor_content` 756, `show_feature_history_content` 630, `dispatch_command` 357 |
| ratchety `check_*.py` | 11, všetky zelené |
| lokálny Linux job | 1 370 testov, 0 zlyhaní |

## 6. Štrukturálne návrhy

1. **Jeden model prierezu pozdĺž vlákna vo svetových osiach** (PG-8, PG-9, PG-11 a zvyšky
   PG-1 až PG-6).
   - V každej stanici: čistý polygón vo svetových zvislých a vodorovných osiach, plocha,
     tenzor I, W, tabuľka Q/b a výška.
   - Ohyb, priehyb, šmyk, zárez aj vzper čítajú z neho; jediná náhrada je `not_verified`.
   - Pripojiť dotaz „ostane valec vrtáka v diele“, analyticky alebo v OCCT.
2. **Jeden návštevník odkazov a tabuľky na prvku** (M-10, M-11).
   - `FeatureKind::references_mut()` pre ukotvenie, prepájanie, označovanie zastaraných,
     konflikty aj klonovanie.
   - Závity, väzby a pôvod parametrov presunúť do prvku, aby sa klonovali a mazali spolu s
     ním.
3. **Počítať pamäť v alokátore, nie podľa typu** (PG-10). Počítajúci alokátor na vlákno,
   zapnutý počas vyhodnotenia a kontrolovaný v háku príkazu. Uzavrie `struct`, `partial`,
   `kwargs` aj budúce typy, a záplaty v starlarku sa môžu zmenšiť.
4. **Úložisko blobov s počítaním odkazov a checkpoint mimo mutácie** (M-12, M-13, UI-10).
   - Zapisovať len bloby odkazované zapisovanými revíziami a neprehashovať ich.
   - Checkpoint písať na pozadí s revíziou.
   - Zámok work-recovery držať v priečinku používateľa.
5. **Jeden kľúč položky BOM** (MF-9, MF-12, MF-6): `(zdroj, ruka, materiál, druh, rozmery
   skladového rámca)` v `ketchup-model`, spoločný pre BOM, pozície na výkrese aj rozpis
   materiálu.
6. **Kóta meria 3D hranu, nie nakreslenú čiaru** (MF-10, skryté a orezané hrany, rezy).
7. **Jeden dokument XDE na zdroj vo workeri a protokol mimo fd 1** (EX-8, EX-9, EX-4).
   - LRU podľa sha256 pre inšpekciu, tesseláciu, export dielu aj konverziu na IGES.
   - Rámce na vlastnom deskriptore, `dup2(2, 1)`, a stderr cez obmedzený buffer do
     `WorkerError`.
8. **Štruktúrna biela listina pre príkaz pluginu** (SEC-8), alebo zrušenie MCP formy
   príkazového riadku.
9. **Politika kompatibility formátu a jeden manifest vydania** (REL-1, VW-15).
   - Každý nový druh záznamu zvýši `CONTAINER_SCHEMA`, s golden súborom predošlého vydania
     v CI.
   - Verzia sa číta raz z `Cargo.toml` pre Android, inštalátor aj Examples.zip.
   - Keystore mimo `target/`.

## 7. Odporúčané poradie

1. **Hneď:**
   - PG-8, PG-9 (statika nosníkov);
   - MF-8 (diel, ktorý vypadne z výroby);
   - M-11 (Make Unique stratí závity);
   - M-10 (rez „po plochu“).
2. **Tento týždeň:**
   - PG-10, EX-8, EX-9, EX-10;
   - M-3 (Undo po nahradení), M-12;
   - UI-10 (priečinok len na čítanie);
   - MCP-3;
   - PG-11, PG-12;
   - MF-9;
   - REL-1 do poznámok k vydaniu;
   - VW-14, VW-15.
3. **Potom:** zvyšok P2 a nízke body. Štrukturálne body 1, 2 a 4 ako samostatné bloky
   podľa AGENTS §6.
