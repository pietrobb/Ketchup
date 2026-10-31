# Podrobný code review Kečupu: celý strom na `3ebe47c`

Stav `main` 2026-10-21. Nadväzuje na `code-review-2026-10-19.md` (`69bb694`). Odvtedy
pribudlo 78 commitov. Opravujú P1 z 10-19 a prinášajú dve veľké zmeny:

- **podklady (underlay) z DXF a DWG:** vloženie, štýly čiar, texty, prichytávanie,
  prispôsobenie dvomi bodmi, posun a otočenie, súradnice S-JTSK;
- **Undo len v pamäti a bez pevných limitov veľkosti:** Save zapíše len aktuálnu revíziu a
  zrušené sú limity zdrojov STEP, IGES, GLB, STL, DXF a DWG, `document.bin`,
  `history.bin`, blobov aj rámcov worker požiadaviek.

Pribudol aj export DAE (COLLADA) pre SketchUp, prírastkové kroky obnovy a rýchla kontrola
kolízií cez obaly.

## 1. Metóda

- Sedem nezávislých čitateľov:
  1. limity a Undo v pamäti;
  2. podklady;
  3. zapisovanie do programu, program a EC5;
  4. exact, workery a MCP;
  5. GUI a DAE;
  6. výroba a výkresy;
  7. viewer, CI a dokumentácia.
- Každý najprv overil body z 10-19 a potom urobil čerstvý review. SEC-5 je rozhodnutie
  vlastníka a nehlási sa.
- **Overenie** v registri:
  - **K**: overené mnou čítaním kódu;
  - **P**: čitateľ ho reprodukoval sondou nad skutočným crate (mimo repozitára);
  - **S**: statický nález čitateľa.
- Spustené na `3ebe47c`:
  - všetkých 12 `scripts/check_*.py`: prešli;
  - lokálne Linux job z CI: `fmt` a `clippy` prešli, testy dali 1 425 prešlo a **1
    zlyhalo** (TST-7). Samostatne ten test prešiel za 101 s.

## 2. Záver v skratke

**Opravené z 10-19 (s testami):**

- WB-1, WB-2, WB-3 v priamych prípadoch;
- PG-13 (zrkadlený nosník) a dosah dielu pri zrkadlení;
- MF-14 čiastočne (role v knižnici);
- PS-1 drží aj bez histórie (`program.json`).

Prírastkové kroky obnovy sú navrhnuté správne:

- roztrhnutý krok sa preskočí s pomenovaním;
- identity revízií sa kontrolujú;
- checkpoint je viazaný na súbor.

Kontrola kolízií cez obaly neprehliadla prienik.

Pri odstránených limitoch nedôveryhodný vstup znovu nespôsobí pád:

- malý upravený súbor s obrovskou deklarovanou dĺžkou sa odmietne a pamäť stúpne o menej
  ako 2 MB;
- odpovede workerov ostávajú obmedzené na 16 MiB;
- program worker má stále limit pamäte.

Export DAE nemá chyby správnosti.

**Väčšina ostatných bodov z 10-19 ostala otvorená.** Nemajú commit a v kóde sa nezmenili:

- GUI: UI-10 až UI-15 a WB-7;
- výroba: MF-9 až MF-13, MF-15, MF-16, MF-17;
- exact: EX-9 až EX-13;
- MCP a pluginy: SEC-8, SEC-9, MCP-3 až MCP-5;
- model: M-3, M-14 až M-16, PS-11;
- program: PG-11, PG-12, PG-14 až PG-17, WB-4 až WB-6;
- viewer, vydanie a testy: VW-14 až VW-16, REL-1, TST-5, TST-6.

Kód viewera sa od 4d8debd vôbec nezmenil.

**Nové P1:**

1. **Zapisovanie do programu:**
   - WB-1 sa vráti cez `param()`: dĺžka 4 mm namiesto 12, alebo záporná (WB-8);
   - posun dielu cez literál `at=` nechá nástroje booleanov na mieste, takže diera alebo čap
     sa posunie voči dielu, bez hlásenia (WB-9).
2. **Podklady:**
   - entita s akýmkoľvek Z, výškou (elevation) alebo hrúbkou sa z podkladu vyhodí, takže
     geodetické zamerania v S-JTSK prídu takmer prázdne (UL-1).

## 3. Overenie opráv z 10-19

| ID | Stav | Poznámka |
|---|---|---|
| WB-1 | opravené pre literálnu dĺžku (P) | 18 → 10 → 12 dá jeden `push_pull(..., name="Length")` a 12 mm. 0 a −5 sa odmietnu. Cez `param()` sa chyba vracia (WB-8). |
| WB-2 | opravené pre iné diely (P) | Zdieľaný `W` sa odmietne a vymenuje ďalšie diely aj nástroj. Zmeny toho istého dielu a polí mimo dielov pozri WB-10. |
| WB-3 | opravené (P) | `material="oak"`, `param("a")` aj `mk("a")` ostanú bajt po bajte. |
| WB-4 | **otvorené, širšie** (P) | Pozri WB-11. |
| WB-5, WB-6, WB-7 | **otvorené** | WB-6: posun 10 dielov domu trvá 555 ms a pridá 9 riadkov. |
| P3 zapisovania | 1 zo 6 opravený | Opravené sú dve premenovania rôznych dielov v jednej dávke. Ostatné ostali (riadok s dvoma volaniami, farba za komentárom, šum v číslach, prevzaté meno, `if`). Zreťazené premenovanie a→p→q program odpojí. |
| PG-13 | opravené (P) | `mirror(axis="x")` aj `mirrored()` dajú `shear_notch` 1,106 fail, rovnako ako nakreslený nosník. |
| MF-14 | čiastočne | `member`, `board`, `bolt`, `nut`, `washer` a `threaded_rod` majú rolu. Zvyšok pozri MF-18. |
| PG-11, PG-12, PG-14–PG-17, P3 statiky | **otvorené** (P) | Sondy dávajú rovnaké výsledky ako 10-19. |
| M-3, M-14, M-15, M-16, PS-11 | **otvorené** | |
| M-12 | prakticky bezpredmetné | Nové súbory nemajú históriu. Pri starom súbore sa blob, ktorý má len história, ukladá ďalej. |
| M-13 | **otvorené a teraz bez stropu** | Pozri LIM-1. |
| EX-9–EX-13, P3 exact | **otvorené** | EX-10 sa zhoršil (MCP-6). |
| SEC-8, SEC-9 | **otvorené** | `pwsh -C …` stále prejde. |
| MCP-3, MCP-4, MCP-5, overenie odpovede `list` | **otvorené** | |
| UI-10–UI-15, WB-7, P3 GUI | **otvorené** | UI-14 sa zhoršil: potvrdenie importu STEP spustí celú inšpekciu XDE druhýkrát, takže okno zamrzne dvakrát. Zoznam vrstiev sa zlepšil len čiastočne. 967c898 funguje bez súbehu, ale zamrazí okno (UI-17). |
| MF-9–MF-13, MF-15–MF-17, P3 výroby | **otvorené** | MF-9 sa rozšíril na dosky (od 9135fcf). |
| VW-14, VW-15, VW-16, TST-5, TST-6, P3 viewera a CI | **otvorené** | `check_ignored_tests` už zvláda `--skip`. Zhoda `union` v sprievodcovi je čiastočná. |
| REL-1 | čiastočne | 0.2.0 súbor vo formáte 104 čisto odmietne. 0.1.0 potichu otvorí `.recovery`, čo je teraz horšie, lebo 104 sa zapisuje vždy. Zásada pre prechod na staršiu verziu nikde nie je. |

## 4. Nové nálezy

### P1

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| WB-8 | P | `ketchup-application/src/program_part_change.rs:855-861` | **WB-1 sa vráti cez `param()`.**<br>• Keď `param()` riadi Pad sám, `Driven::Alone` ho prepíše a `Expected::Reads` skontroluje len `extent.distance`. Riadky `Length` a vlastné `push_pull` na koncoch ostanú navrch.<br>• Kontrola ≤ 0 testuje len zadanú hodnotu, nie výslednú dĺžku.<br>• (a) `H = param("h", 18)` pre diely a, b. Zadanie 10 pre a zapíše `Length -8`, lebo H je zdieľané. Po zmazaní b a zadaní 12 sa prepíše h = 12 a **a má 4 mm**.<br>• (b) `H = param("h", 30)` a vlastný `push_pull("a", face="end", distance=-15)`. Zadanie 10 prejde a **dĺžka je −5**, teda obrátený diel. | • Pri `extent.distance` cieliť vždy na `Length = hodnota + ostatné` a odmietnuť ≤ 0 ešte pred `driven`.<br>• Ak riadok `Length` existuje, prepísať ten riadok namiesto prepisu parametra. |
| WB-9 | K, P | `ketchup-application/src/program_part_edit.rs:130-138`, `program_part_change.rs:261-273` | **Posun cez literál `at=` nechá nástroje booleanov na mieste.**<br>• `Expected::Placed` porovná pri posunutom diele len meno, rozmer, polohu a otočenie, nie operácie ani nástroje.<br>• `a = box("a", (100,100,18), at=(0,0,0))` a `subtract(a, box("h", (10,10,30), at=(45,45,-5), tool=True))`. Posun a o +20 v x zapíše `at=(20,0,0)`, ale nástroj ostane na x = 45 vo svete. Diera sa voči dielu posunie z 45 na 25 a kontrola prejde.<br>• `move()` nástroje berie so sebou (`eval.rs:821`), úprava literálu nie. To isté platí pre `union()` so svetovo umiestneným telesom (čap).<br>• Diera môže z dielu úplne vypadnúť bez hlásenia. | • `Placed` nech porovná celý posunutý diel s dielom pred úpravou, posunutým cez `move_part` (vrátane operácií a nástrojov).<br>• Alebo vyhodnotiť úpravu literálu aj pridaný `move()` a literál ponechať len pri zhode. |
| UL-1 | K, P | `ketchup-model/src/import/dxf.rs:4049-4061`, `UNDERLAY_READING` `:61-70` | **Entita s akýmkoľvek Z, výškou alebo hrúbkou sa z podkladu vyhodí.**<br>• Čítanie podkladu používa pravidlo rovinnosti importu profilov a len zapína `skip_unreadable`.<br>• Čiara v S-JTSK s `30=251.3` alebo LWPOLYLINE s `38=250` sa preskočí, v reporte ostane len `dxf.entity-unreadable … non-planar Z`.<br>• Geodetické zameranie prakticky vždy nesie Z, takže príde takmer prázdne a podpora S-JTSK stráca zmysel. | • V režime podkladu premietnuť do XY: ignorovať skupiny 30/31/38/39. Odmietnuť len extrúziu 210/220/230, ktorá nie je ±Z.<br>• Jedna informatívna diagnostika „Z vynechané“. |

### P2

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| WB-10 | P | `program_part_change.rs:731`, `:744-757` | **`Driven::Alone` kontroluje len iné diely.**<br>• Pri `box("a", (W, W, 10))` zadanie šírky 150 zmení aj **hĺbku na 150**.<br>• `area_load(kn_m2=W/200)` sa zmení z 2,0 na 4,0 bez slova.<br>• `expect_gap(..., W+100)` sa posunie.<br>• `joints`, `load_paths`, `free_spaces`, `motions` ani `material_weights` sa neporovnávajú. | „Sám“ má znamenať, že upravený diel sa líši len v jednej hodnote a všetko ostatné v `ProgramModel` je rovnaké. Inak vymenovať zmeny ako pri `Shared`. |
| WB-11 | P | `renamed()` `program_part_change.rs:318-326` | **Premenovanie berie každý diel „<meno> …“ ako odvodený.**<br>`box("shelf")` a `box("shelf 2")`: premenovanie shelf na board sa odmietne bez užitočnej správy. Zrkadlenie s následným premenovaním aj literál `"door/handle"` sa odmietnu stále. | Premenovať len mená, ktoré program naozaj odvodzuje (`part_sources`), alebo premenovať aj odvodený literál. |
| WB-12 | P | `program_part_change.rs:705`, `:864` | **Zadaná dĺžka obíde `min` a `max` parametra.**<br>Pri `H = param("h", 18, min=12, max=40)` zadanie 10 zapíše `distance=-8` a zadanie 60 zapíše `+42`. Rozsah, ktorý program deklaruje, sa potichu obíde. | Keď je jediný ovládač mimo rozsahu, odmietnuť s uvedením rozsahu. |
| MF-18 | S | `ketchup-program/library/prelude.star:1646`, `:1995`, `fabrication.rs:3000` | **Medzery v rolách MF-14.**<br>• `rebar()` a vrstvy `buildup()` (stĺpiky, prahy, dosky) nemajú rolu. Stena z `buildup()` sa bez `production()` nevyexportuje.<br>• `board`, `member` a spojovacie prvky neberú `attributes=` ani `role=`, takže nakúpená doska z `board()` ostane „vyrábaná“ a oceľový stĺp z `member()` „drevo“.<br>• Slovník rolí nie je zdokumentovaný (§2). | `role=` s predvolenou hodnotou v každom tvorcovi dielov, rola podľa druhu vrstvy v `buildup()` a nakupovaný `rebar`. Zdokumentovať. |
| MF-19 | K | `ketchup-manufacturing/src/fabrication.rs:2312-2315` | **WoodWOP odmietne korpus aj stôl s nesprávnou hláškou.**<br>• Dieru vŕtanú smerom k +Z stroja (zospodu) woodWOP odmietne (`axis == 2 && sign > 0 → None`).<br>• Od 9135fcf idú dosky do woodWOP. `cabinet.star` má kolíky a rady dier pre policové kolíky z plochy x−, ktorá je v zoradenom skladovom rámci +Z. `table.star` má kolíky zdola.<br>• Oba hlavné príklady sa tak odmietnu celé a hláška tvrdí „sklad nie je obdĺžnikový / rez mimo skladu“. | Prvok schopnosti `UndersideMachining` s vlastným dôvodom („otočte diel alebo exportujte výrobný job“), alebo generovať setup B s otočeným obrobkom. |
| MF-20 | S | `fabrication.rs:3233-3253`, `:812` | **Dielenský výkres stratí obrábanie na druhom riadku toho istého telesa.**<br>Dve kópie jednej vŕtanej definície s rôznym materiálom dajú dva riadky BOM. Výkres druhého riadku nemá diery ani rezy a ani neodkazuje na prvý, takže dielňa ho vyrobí bez dier. | Kresliť operácie na každom riadku, ktorý ich vlastní, a kontrolu robiť po riadkoch. |
| MF-21 | S | `ketchup-application/src/planner.rs:2021-2034`, `drawing.rs:2471-2487` | **Zrkadlený koreňový diel sa kreslí v opačnej ruke (regresia z 74e6164).**<br>Jediná koreňová cesta sa stane `DrawingSource::Definition` a kreslí sa s identitou. `create_drawing` na `"side mirrored"` dá pôvodnú ruku bez značky „(mirrored)“. Ignoruje sa aj otočenie výskytu a zahodia sa požadované pozície. | `Definition` len pri identite (alebo vlastnej rotácii). Inak ponechať cestu inštancie alebo odmietnuť. |
| MF-22 | S | `fabrication.rs:1298-1304` | **BTLx zapíše prvok kreslený pozdĺž X alebo Y s Length = hĺbka.**<br>`box("beam", (4000,120,200))` s rolou dreva sa zapíše ako `Length=200 Width=4000 Height=120` a obrábanie sa umiestni v tom rámci. | Preorientovať cez zoradený skladový rámec, alebo pomenovane odmietnuť. |
| UL-2 | P | `ketchup-model/src/import/dxf/block.rs:54` | **Blok so Z bázy ≠ 0 zhodí celý import podkladu.**<br>`ensure_planar(header)?` beží v `parse_blocks` mimo vynechávania po entitách. To isté platí pre ďalšie tvrdé `InvalidBlock` (príznaky, alternatívne meno, duplicita, hĺbka). | V režime podkladu blok preskočiť a nahlásiť. |
| UL-3 | P | `ketchup-model/src/underlay.rs:692` (`MAX_CIRCLE_CHORDS` 1024) | **Oblúky nad ~21 m porušia deklarovanú chybu tetivy 0,1 mm.**<br>Pri R = 100 m je chyba 0,47 mm, pri R = 500 m 2,35 mm. Odporuje to `UNDERLAY_ARC_CHORD_MM`, dokumentácii prichytávania aj schéme MCP („within 0.1 mm“). Prichytenie na osi ciest sa mýli o milimetre. | Strop tetív podľa dĺžky oblúka s globálnym rozpočtom, alebo zdokumentovať a hlásiť skutočnú chybu. |
| UL-4 | P | `underlay.rs:244` (`move_near_origin`) | **Čiary ďalej než ±1 km od mediánu sa potichu orežú.**<br>Výkres S-JTSK s rozsahom 3 km stratil 11 z 31 čiar a ostal len počet vo varovaní. Katastrálna mapa celej obce tak príde bez okrajov. | Podklad je v lokálnom f64 blobe, teda nepotrebuje obálku modelu. Povoliť väčšiu lokálnu obálku, alebo pri rozsahu nad 2 km odmietnuť. |
| UL-5 / LIM-2 | P | `ketchup-model/src/underlay/snap.rs:443-485`, `ketchup-app/src/underlay_snapping.rs:72-93` | **Index prichytávania podkladu rastie superlineárne.**<br>• Dlhá úsečka sa zapíše do každej bunky, ktorú pretína, takže pamäť rastie ako N·√N.<br>• 150k dlhých úsečiek: 2,2 s a 512 MB. 10,8 MB blob: 2,0 GB za 9,8 s.<br>• Beží v procese okna, mimo limitu workera. Spustí ho aj cudzí `.ketchup` s blobom podkladu alebo skutočný výkres s dlhými šrafami. | Dlhé úsečky v samostatnom zozname alebo R-strome, strop záznamov c·N, alebo stavať index vo workeri. |
| UL-6 | S | `ketchup-app/src/renderer/underlay.rs:89`, `:194`, `app/viewport.rs:4332` | **Podklad ďaleko od počiatku stráca presnosť na GPU.**<br>Lokálne súradnice dosahujú 1e6 mm. Pri priblížení 100 px/mm a x ≈ 900 m je krok f32 v klip-priestore ~8 px, takže čiary sa trasú a odchádzajú od značiek prichytenia práve tam, kde nástroj Prispôsobiť potrebuje presný klik. Vypočítané, nie sondované. | Vrcholy relatívne k stredu blobu a stred aj cieľ kamery zložiť v f64 pred pretypovaním (vykresľovanie relatívne ku kamere). |
| UL-7 | P | `ketchup-model/src/import/dxf/style.rs:96` (`within`) | **ByBlock sa rieši na nesprávnej vrstve, keď je samotný INSERT ByLayer.**<br>Entita bloku na vrstve WALLS s farbou ByBlock, vložená INSERT-om ByLayer na vrstve DOORS: správne je farba DOORS (modrá), Kečup dá červenú z WALLS. To isté platí pre typ čiary aj hrúbku. | Prenášať `ByLayerOf(insert_layer)`. |
| LIM-1 | S | `ketchup-model/src/persistence/work_recovery.rs:45-104`, `persistence.rs:1011-1017`, `:1161-1168`, `work_recovery_steps.rs:62`, `ketchup-application/src/evaluation.rs:606-616` | **Každá úprava číta a hashuje celý súbor, checkpoint aj všetky bloby na UI vlákne, teraz bez stropu.**<br>• `write_work_recovery` beží v každej mutácii. Dvakrát `fs::read` a SHA-256 celého `.ketchup`, dvakrát celého checkpointu, opätovné hashovanie a kópia každého blobu.<br>• Každé exact vyhodnotenie hashuje a kopíruje každý importovaný zdroj; strop 128 MiB zmizol.<br>• Po importe 500 MB STEP (teraz povolený) je to pri každej úprave ~2 GB čítania a SHA plus ~1 GB kópií. | • Bloby ako `Arc<[u8]>` s hashom spočítaným raz.<br>• Identitu toho, čo session naposledy zapísala, držať v pamäti namiesto čítania súborov.<br>• Checkpoint mimo UI vlákna. |
| LIM-3 | S | `ketchup-scheduler/src/worker_client.rs:40`, `third_party/process-wrap/src/std/job_memory_limit.rs:24-33` | **Exact worker nemá limit pamäte, program worker na macOS fakticky tiež nie.**<br>• Exact worker dostáva STEP a IGES ľubovoľnej veľkosti a beží s `None`. Nepriateľský alebo obrovský STEP vyčerpá RAM systému; na Windows pri vyčerpaní commitu môže spadnúť aj okno.<br>• Na Unixe je limit `RLIMIT_AS`, ktorý macOS prijme, ale nevynucuje (overiť na Macu). Predpoklad z 805b2bb („job limit drží, čo worker drží“) tam neplatí. | Limit pre exact worker (napr. 50 % fyzickej pamäte) s pomenovaným `memory_limit`. Na macOS sledovať RSS (`proc_pid_rusage`). |
| UNDO-1 | K | `crates/ketchup-app/src/feature_history_ui.rs:2050`, `locales/en-US.ftl:388`, `README.md:172` | **Pomenované checkpointy a tvrdenie v README klamú.**<br>• „Named checkpoint created; save the document to persist it“, ale Save históriu už nezapisuje. Checkpoint sa po otvorení stratí a pri `UNDO_REVISIONS` = 30 sa zahodí už po 30 úpravách.<br>• README: „A saved document keeps its Undo/Redo history up to the Undo limit“, čo od e5685da neplatí.<br>• Nový limit 30 krokov (predtým 200) nikde nie je uvedený. | Opraviť texty a README, alebo checkpointy ukladať ako malý záznam aktuálnej revízie. |
| MCP-6 | S | `ketchup-app/src/exchange_export.rs:178`, `live_bridge/interchange.rs:214` | **EX-10 má nový dôsledok: export sa zapíše, aj keď klient dostal timeout.**<br>MCP čaká 120 s, rozpočet výmeny dovolí 15 min. Klient dostane `response_timeout`, okno je zamrznuté a súbor sa o minúty neskôr aj tak zapíše. | Odovzdať príznak zrušenia z bridge do `job.run`, spúšťať cez `queue_exchange_export` a po zrušení nič nezverejňovať. |
| UI-16 | S | `ketchup-app/src/live_bridge/interchange.rs:29,47` | **Export DAE je len cez MCP.**<br>Nie je `AppCommand` ani položka v menu Súbor, takže používateľ SketchUpu ho nenájde. Komentár pritom tvrdí, že ide o „every export format of the File menu“. | `ExportSketchupDae` vedľa GLB, cez `FileActivity`. |
| UI-17 | S | `ketchup-app/src/app/exact.rs` (`finish_exact_evaluation_now`), `tool_gesture.rs:571` | **Enter pri napísanej hodnote počas vyhodnotenia zamrazí okno.**<br>`500⏎` na dieli domu počas ~30 s vyhodnotenia zablokuje okno bez prekreslenia aj zrušenia, OS hlási „neodpovedá“. Po 60 s sa vyhodnotenie zruší a začne od nuly. | Modálna aktivita s indikátorom a Zrušiť, alebo hodnotu odložiť naviazanú na `AwaitedValue`. |
| UI-18 | S | `ketchup-interaction/src/push_pull_gesture.rs` (509 r.), `rectangle_face_authoring.rs` (444 r.) | **Mŕtvy kód.**<br>Oba moduly používajú len ich vlastné testy. Duplikujú pravidlo Push/Pull z aplikácie, čo je presne „rozdelenie pravidla“ z 10-18, a porušujú §6 (žiadne paralelné cesty). | Zmazať aj s testami a nechať jednu plánovaciu funkciu. |
| VW-17 | S | `viewer/web/src/lib.rs:189-192` | **Webový viewer berie komentáre ako uložené po zrušenom dialógu.**<br>`download()` len klikne na odkaz a `comments_written` ich potom označí za uložené. Keď prehliadač ukáže dialóg a používateľ ho zruší, varovanie pri odchode zmizne a zatvorenie karty komentáre stratí. | Nechať neuložené do potvrdenia, alebo `showSaveFilePicker`. |
| VW-18 | S | `viewer/android/src/lib.rs:111-113` | **Android po zlyhanej obnove session stratí neuložené komentáre.**<br>Zlyhanie sa len zapíše do logu. Ďalší otvorený balík `SessionKeeper` prepíše `session.*`, takže komentáre z neúspešnej obnovy sa potichu stratia. | Pri zlyhanej obnove blokovať prepis session, kým používateľ nerozhodne. |
| TST-7 | K | `ketchup-program/tests/example_programs.rs:37`, `ketchup-program/src/execution_budget.rs:10` | **Test príkladov je nestabilný: časový rozpočet programu je 20 s aj v debugu.**<br>V plnom behu pri záťaži `modular-doweled-cabinet.star` skončil na „program execution budget exceeded“ (`dowels` → `_contact_row`). Samostatne test prešiel za 101 s. Výsledok tak závisí od rýchlosti stroja. | V testoch rozhodovať podľa rozpočtu príkazov (`MAX_STATEMENTS`), nie času. Alebo v debugu násobiť `MAX_TIME`. |

### P3

- **Zapisovanie do programu:**
  - MCP `program_edit_source` používa snímku spred úpravy, takže dve zrkadlenia toho istého
    dielu v jednom programe dostanú rovnaké meno.
  - Premenovanie na meno skupiny, komponentu, nástroja alebo prevzaté meno dá surovú chybu
    vyhodnotenia.
- **Podklady:**
  - Prispôsobenie prijme ľubovoľnú mierku a posun (1e9, 1e12 mm), takže metre namiesto mm
    sa potichu uložia.
  - Body vybrané v nástroji Prispôsobiť prežijú Undo aj úpravu cez MCP.
  - Posun podkladu na bod modelu so z > 0 sa odmietne.
  - Vypnuté a zmrazené vrstvy sú viditeľné a dá sa na ne prichytiť.
  - `%%nnn` sa číta ako bod Unicode, nie podľa kódovej stránky.
  - Počet xref je na blok, nie na vloženie.
  - Chyby sa splošťujú (`error.to_string()`, len `io::ErrorKind`).
  - Pri pohľade šikmo k obzoru je dotaz na prichytenie pomalý.
  - Blob je nekomprimovaný (16 B na bod).
  - `placement` pribudol do formátu 104 bez zvýšenia verzie.
- **Limity a Undo:**
  - Starý súbor s históriou ju pri otvorení potichu zahodí, bez upozornenia
    (`digest-opened-without-history` existuje).
  - GLB stojí v okne ~11× svojej veľkosti (`serde_json::Value`).
  - Prevod DWG prečíta celý výstupný DXF do okna, až potom sa uplatní limit párov.
  - Do formátu 104 sa zapisuje vždy, aj bez podkladu, takže 0.1.0 neotvorí žiadny nový súbor.
  - MCP import DWG nad 120 s nikdy neuspeje.
  - `encode_frame` alokuje celé CBOR bez možnosti zlyhať a `read_frame` robí
    `with_capacity` z hlavičky.
  - `read_regular_file` číta za rezerváciu.
- **Exact a MCP:**
  - Skorá odpoveď po 5 s (9aa98e0) nedá klientovi pokyn, ako sa pýtať ďalej. Kontrola sa
    potom nedá zrušiť a drží jediný slot úloh (~25 s), takže ostatní dostanú `busy`.
  - Náhľad histórie prvkov nevyhodnotí importované diely (`&[]` zdroje).
  - Konverzia STEP na IGES berie odtlačok len z `part(0)` a chyby splošťuje.
- **GUI:**
  - DAE: `is_alphanumeric` pustí `²` a `½`, ktoré XML NameChar nedovoľuje, a nerozložené
    diakritické znaky sa zmenia na `_`.
  - DAE: zrkadlený výskyt zapíše maticu so záporným determinantom bez otočenia poradia
    vrcholov; overiť import do SketchUpu.
  - DAE: až 512 MiB XML na UI vlákne a súradnice v plnej presnosti f64.
  - Dátum na výkrese je v UTC, takže v Bratislave medzi 00:00 a 02:00 ukáže včerajšok.
  - Šum f64 v tokenoch prvkov (`0.30000000000000004`).
  - Uloženie alebo export vo fronte sa potichu zahodí pri úprave cez MCP.
  - Dve okná nad tým istým súborom hlásia pevnú anglickú vetu.
- **Výroba:**
  - Rozmery BOM sa berú z vrcholov siete, takže oblé telesá sú menšie o chybu tetivy.
  - Kontrola vŕtania BTLx neodhalí bočný výstup diery a odmieta bez dôvodu.
  - Prázdny INSERT alebo ELLIPSE s koncovým parametrom pod začiatkom zhodí celý import
    geometrie.
  - Knižnica kolíkov vŕta z plochy, ktorá sa práve dotýka, čo je príčina MF-19.
  - Matica DAE sa nekontroluje na konečné čísla.
- **Viewer, CI a dokumentácia:**
  - Kroky v jobe `checks` bežia za sebou, takže prvý zlyhaný ratchet schová ostatné; stálo
    to tri opravné commity.
  - Fuzzing nepokrýva nové čítanie podkladu DXF.
  - `check_test_timing` neskenuje `viewer/*`; `check_test_sleeps` neskenuje Python
    (17× `time.sleep`).
  - `afe1325` teraz testuje cestu ukladania, ktorú používateľ nepoužíva.
  - `AGENTS.md:103` spomína „history past the file limit“.
  - Komentáre `app/underlay.rs` a `dwg_import.rs` spomínajú zrušený deadline.
  - README nepopisuje podklady.
  - `en-US.ftl:1497` „program je v Undo histórii“ už po zatvorení neplatí.

## 5. Metriky

| | dnes |
|---|---:|
| ratchety `check_*.py` | 12, zelené |
| lokálny Linux job | 1 425 prešlo, 1 zlyhalo (TST-7, nestabilné) |
| najväčší súbor GUI | `app/viewport.rs` 4 770 |
| produkčné funkcie GUI nad 300 / nad 150 | 5 / 35 (nová nad 300: `dispatch_file_command` 302) |
| mŕtvy kód v `ketchup-interaction` | ~950 riadkov (UI-18) |

## 6. Štrukturálne návrhy

1. **Zapisovanie do programu: jedna kontrola celého modelu** (WB-8 až WB-12). Každá úprava
   postaví očakávaný `ProgramModel`: model pred úpravou s explicitnou záplatou.
   - Diel sa posúva tuhý aj s nástrojmi.
   - Mení sa jedna hodnota.
   - Meno sa mení len tam, kde ho program odvodzuje.

   Porovná sa celý model. `Reads`, `Placed` bez operácií a `Renamed` podľa prefixu sa zrušia.
   Dĺžka extrúzie je jeden pojem: Pad plus všetky offsety koncov, a kontroly ≤ 0 a rozsahu
   platia pre výsledok.
2. **Skutočný režim čítania podkladu** (UL-1, UL-2, UL-4, UL-7) namiesto importu profilov
   so zapnutým vynechávaním.
   - Premieta Z, nikdy neskončí na bloku a má vlastnú politiku súradníc.
   - Lokálny rámec od začiatku po koniec: čiary relatívne k stredu, vykresľovanie relatívne
     ku kamere a kontrola umiestnených hraníc pri zápise (UL-6, fit).
3. **Rozpočet namiesto počtu pri odvodených štruktúrach** (UL-3, UL-5, LIM-1).
   - Tetivy podľa dĺžky oblúka.
   - Záznamy mriežky na úsečku.
   - Bloby ako `Arc<[u8]>` s hashom spočítaným raz.
   - Checkpoint mimo UI vlákna.
4. **Nedôveryhodné parsovanie len v obmedzených workeroch alebo prúdovo** (LIM-3, GLB,
   index podkladu).
   - Exact worker dostane limit pamäte.
   - Na macOS sledovanie RSS.
   - „Bez pevného limitu bajtov“ potom znamená, že proces je ohraničený, nie „neohraničený a
     dúfajme lineárny“.
5. **„Undo len v session“ ako vlastnosť formátu** (UNDO-1, REL-1).
   - `history.bin` zo súborov už neprijímať, okrem čítania programu zo starých súborov, a
     pri vynechaní upozorniť.
   - Zapisovať najnižší formát, ktorý dokument vyjadrí (103 bez podkladov).
   - Jeden test, ktorý drží README, lokalizácie a SDK v zhode s týmto správaním.
6. **Jeden skladový rámec a jeden kľúč položky** (MF-9, MF-19 až MF-22). Zoradený pravotočivý
   rámec polotovaru raz, pre BOM, BTLx, woodWOP, job aj výkres. Obrábanie vlastnené po
   riadkoch.

## 7. Odporúčané poradie

1. **Hneď:**
   - WB-8, WB-9 (zapisovanie do programu potichu mení model);
   - UL-1 (podklad bez Z);
   - MF-19 (hlavné príklady sa nevyexportujú do woodWOP).
2. **Tento týždeň:**
   - WB-10 až WB-12;
   - UL-2 až UL-5;
   - LIM-1, LIM-3;
   - UNDO-1 (texty a README);
   - MF-20, MF-21;
   - MCP-6;
   - TST-7.
3. **Otvorené P2 z 10-19 bez commitu:** UI-10, UI-14, M-14, MF-16, MF-17, EX-13, SEC-9 a
   VW-16. Väčšina sú malé zmeny; SEC-9 je jeden riadok.
4. **Potom:** zvyšok P2 a nízke body. Štrukturálne body 1 a 2 ako samostatné bloky podľa
   AGENTS §6.
