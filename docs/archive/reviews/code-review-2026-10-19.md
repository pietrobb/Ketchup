# Podrobný code review Kečupu: celý strom na `69bb694`

Stav `main` 2026-10-19. Nadväzuje na `code-review-2026-10-18.md` (`dde47a4`, 0.2.0), ktorý
je v tomto commite pridaný do archívu, lebo na `main` chýbal. Odvtedy pribudlo 70 commitov.
Riešia hlavne P1 z 10-18, nízke body GUI (P3-G), MCP (P3-C), výkresy (P3-V), CI a
dokumentáciu. Pribudla nová funkcia: **úprava dielu, ktorý vyrába program, sa zapíše späť do
zdrojového kódu programu** (posun, otočenie, farba, premenovanie, zrkadlenie, zmazanie,
hodnota parametra).

## 1. Metóda

- Sedem nezávislých čitateľov:
  1. zapisovanie do programu;
  2. model a perzistencia;
  3. program, EC5 a zaťaženia;
  4. exact, workery a MCP;
  5. GUI;
  6. výroba a výkresy;
  7. viewer, CI, testy a dokumentácia.
- Každý najprv overil opravy z 10-18 vo svojej oblasti a potom urobil čerstvý review. SEC-5 je
  rozhodnutie vlastníka a nehlási sa.
- **Overenie** v registri:
  - **K**: overené mnou čítaním kódu;
  - **P**: čitateľ ho reprodukoval sondou nad skutočným crate (mimo repozitára);
  - **S**: statický nález čitateľa so stopou od volajúceho.
- Spustené na `69bb694`:
  - všetkých 12 `scripts/check_*.py` vrátane nového `check_ignored_tests.py`: prešli;
  - lokálne Linux job z CI (`fmt`, `clippy`, testy 15 crate): prešiel, 1 398 testov,
    0 zlyhaní.

## 2. Záver v skratke

**Opravené z 10-18 (s testami):**

- P1: PG-8, PG-9, PG-10, M-10, M-11 a MF-8 (s regresiou MF-14).
- Väčšina nízkych bodov GUI, MCP, výkresov, CI a dokumentácie:
  - jedna gramatika hodnôt pre všetky polia;
  - `GESTURES` pre všetky nástroje;
  - zámerne poslaná anotácia `list`;
  - názvy vrstiev a text v R12;
  - kódové stránky DXF;
  - ToUnicode;
  - fuzzing so 6 cieľmi a korpusom;
  - `check_ignored_tests.py`;
  - README, sprievodca jazykom, `plugins.md`, ADR.

**Čiastočne opravené alebo bez commitu:**

- EX-8: cache funguje pre inšpekciu, ale nie pre graf (EX-13).
- Z 10-18 ostali otvorené:
  - model: M-3, M-12, M-13, PS-11;
  - exact: EX-9 až EX-12;
  - GUI: UI-10 až UI-13;
  - výroba: MF-9 až MF-13;
  - program: PG-11, PG-12;
  - viewer a vydanie: VW-14, VW-15, REL-1;
  - testy: TST-5;
  - MCP: SEC-8, MCP-3 až MCP-5.

**Nové P1:**

1. **Zapisovanie do programu:**
   - hodnota zadaná dvakrát sa sčíta, nula a záporná hodnota prejdú (WB-1);
   - zmena hodnoty riadenej `param()` potichu zmení iné diely aj počet dielov (WB-2);
   - premenovanie prepíše každý rovnaký reťazec v programe vrátane materiálu a názvu
     parametra (WB-3).
2. **Statika:** nosník zrkadlený pozdĺž vlákna (`mirrored()`) sa posudzuje so zárezom na
   nesprávnom konci a vyjde „pass“ namiesto „fail“ (PG-13).

## 3. Overenie opráv z 10-18

| ID | Stav | Poznámka |
|---|---|---|
| PG-8, PG-9 | opravené (P) | 64 staníc plus rohy profilu; priehyb sa integruje s premenlivým I; voľný ohyb. Zvyšky sú v PG-15 a PG-16. |
| PG-10 | opravené pre `struct` a `partial` (P) | Všetko mimo arény sa počíta cez `Allocative`. Pamäť na strane Rustu sa nepočíta (PG-17). |
| PG-11, PG-12 | **otvorené** (P, K) | Diera Ø12 zarezaná 2 mm do drážky stále nedá varovanie. Krokva sa nad ~25,8° stále otočí naplocho. |
| P3 statiky | čiastočne | `twists`: pri 12 m prejde, pri 120 m stále `not_verified`. `BEARS_ALONG_GRAVITY` = 0,5 a dokumentácia `member_check` sú bez zmeny. |
| M-10, M-11 | opravené | M-11: kontrola tabuliek nepokryje `body_feature_suppression` ani ciele trvalých kót. |
| M-3, M-12, M-13, PS-11, P3 modelu | **otvorené** | M-3: Undo po nahradení programom vedie do prázdneho dokumentu (headless/MCP). |
| EX-8 | čiastočne | Pozri EX-13. Aplikácia ďalej zapisuje a hashuje celý zdroj raz na diel (`evaluation.rs:1005-1015`), pri 100 dieloch × 30 MiB ~3 GB. Cache má jednu položku a nepočíta sa do rozpočtu. Výnimka OCCT pri parsovaní sa hlási ako „could not be allocated“. |
| EX-9 až EX-12, P3 exact | **otvorené** | |
| 69bb694, a235704, fb5a2f8 | v poriadku | 69bb694: `BackendException` zahŕňa aj `std::bad_alloc`, takže pri nedostatku pamäte dávka pokračuje. |
| SEC-8 | **otvorené** | Pozri SEC-9. |
| MCP-3, MCP-4 | **otvorené** | MCP-3 platí aj pre DXF a DWG listy (`drawings.rs:90`). MCP-4: v headless je stále 31× `to_string()`. |
| MCP-5 | čiastočne | Hashovanie je mimo UI vlákna, ale bez stropu veľkosti a bez zrušenia. |
| P3-C (437ce93, bfe162c, 2874502, e02d28c, 6d18fb3, 00b6189) | opravené | Odpoveď na `list` nie je overená dôkazom, takže cudzí proces na zastaranej adrese môže podvrhnúť názov dokumentu. |
| UI-10 až UI-13 | **otvorené** | UI-10: aj keby sa otvorenie opravilo, prvá úprava si zámok vezme znova. UI-12 sa zhoršil: aj polnoc, Uložiť ako a zmena jazyka uložia staré predvolené hodnoty. |
| P3-G (11 commitov) | opravené | Zvyšky:<br>• `3×200` v jednom poli sa teraz odmietne bez nápovedy, že `*` násobí;<br>• `5x` (zvyk zo SketchUpu) nefunguje;<br>• cfbb39b pri pomalej snímke potlačí aj Delete a Ctrl+Z a Enter potvrdí náhľad, nie napísanú hodnotu. |
| P3 GUI z 10-18 | **otvorené** | Otvorené ostali:<br>• zoznam vrstiev;<br>• predvypĺňanie spojov;<br>• `M8x1.25` v nástroji Diera;<br>• pravidlo Push/Pull;<br>• nastavenia listu mimo Undo;<br>• filter rozpisu materiálu;<br>• anglické odmietnutia exact. |
| MF-8 | čiastočne, s regresiou | Pozri MF-14 a MF-15. Ostatné príklady (`garden-studio`, `wardrobe`, `table`…) rolu nemajú, ale padali už predtým. |
| MF-9 až MF-13 | **otvorené** | |
| P3-V (e7b384e, 062ab1b, 208072c, 4b4d87f, 99b259a, 89105bf, ca2bc81, 0c7c426) | opravené | 208072c: prípona `_2` závisí od poradia v scéne. 0c7c426: príčina (`ConflictingBodyTerminals`) sa zahodí. |
| 8362a0f | čiastočne, **regresia** | Pozri MF-16. |
| a6deebd | čiastočne | DOS stránky (850, 852, 866) nie sú. Zlá stránka v hlavičke dá tichú zmenu znakov („Stěna“ → „Stìna“). |
| aab2f34 | čiastočne | Šírka riadku kusovníka sa nekontroluje (MF-17). |
| VW-14, VW-15, REL-1, TST-5 | **otvorené** | |
| P3 viewer a CI | väčšinou | Opravené:<br>• fuzzing;<br>• komentáre v `check_ci_crates`;<br>• `check_test_sleeps` pre viewer;<br>• `check_ignored_tests`;<br>• dokumentácia;<br>• čítanie na štarte Androidu.<br>Otvorené:<br>• obchádzanie `check_test_timing`;<br>• zaseknutie namiesto zlyhania;<br>• `--locked` vo viewer skriptoch;<br>• pravidlo veľkého domu;<br>• `-Version` a Examples.zip;<br>• stav webového viewera;<br>• `recover_interrupted_saves` na UI vlákne. |

## 4. Nové nálezy

### P1

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| WB-1 | K, P | `ketchup-application/src/program_part_change.rs:715-724` | **Dĺžka zadaná v paneli parametrov sa pri opakovaní sčíta, nula a záporná hodnota prejdú.**<br>• Rozdiel sa ráta ako `value - current`, kde `current` je stále `extent.distance` pôvodného Pad, napr. 18.<br>• Zadanie 10 zapíše `push_pull(..., distance=-8)`, ďalšie zadanie 12 pridá `push_pull(..., distance=-6)`. Koniec je na 18−8−6 = **4 mm, nie 12**.<br>• `Expected::Reads` overí len to, že posledný offset sa rovná rozdielu, takže kontrola prejde.<br>• Zadanie 0 zapíše `-18` (nulová hrúbka), −5 zapíše `-23` (obrátený diel). Obíde to pravidlo z bcf5252, že nulová vzdialenosť sa odmietne. | • `current` = vzdialenosť Pad plus súčet koncových offsetov. Lepšie: prepísať literál posledného `push_pull` z GUI.<br>• Overiť výslednú polohu konca a dĺžku ≤ 0 odmietnuť s nápovedou. |
| WB-2 | P | `program_part_change.rs:591-634`, kontrola `:228-234` | **Zmena hodnoty riadenej `param()` potichu zmení iné diely.**<br>• Pri `H = param("h", 18)` a dvoch extrúziách s `distance=H` posun konca `a` o 5 nastaví `{"h": 23}`, takže **narastie aj `b`**.<br>• Pri `if W > 110: box("extra", ...)` zadanie 120 pre šírku `a` zmení aj `b` a **vytvorí diel „extra“**.<br>• V `cabinet.star` zmena výšky `carcass/left` zmení celý korpus a všetky police, bez hlásenia.<br>• Prepísaná hodnota sa nezaokrúhli (250,3 sa uloží ako `w = 750.5999999999299`). Celočíselný parameter môže dostať desatinnú hodnotu. | • Porovnať celý model a prepísanie prijať, len ak je zmena jediná; inak ukázať, ktoré ďalšie diely sa zmenia, a pýtať sa.<br>• Zmenu počtu dielov odmietnuť.<br>• Hodnotu zaokrúhliť cez `written()` a celé čísla nechať celé. |
| WB-3 | K, P | `program_part_change.rs:498-503`, `:236-241`, `renamed()` `:257-286` | **Premenovanie prepíše každý reťazcový literál rovný starému menu.**<br>• `box("oak", ...)` a `box("b", ..., material="oak")`: premenovanie `oak` na `lid` zmení aj **`material="lid"` dielu `b`**, čím pokazí kusovník.<br>• `w = param("a", 10)` a `box("a", ...)`: premenuje sa aj **parameter**, takže prepisy a varianty s kľúčom `"a"` ostanú visieť.<br>• Prepíše sa aj `if n == "a"` vo funkcii.<br>• Kontrola prejde, lebo na serializovaný model použije to isté slepé premenovanie. | Prepisovať len literály na pozíciách odkazu na diel (meno vo vytvárajúcom volaní, prvý argument `move`, `push_pull`, `continue_part` …). Kontrolu obmedziť na mená dielov. Ak je meno inde, odmietnuť s riadkom. |
| PG-13 | K, P | `ketchup-program/src/member_check.rs:833`, `:682`, `:950` | **Nosník z bočného profilu zrkadlený pozdĺž vlákna sa posudzuje ako nezrkadlený.**<br>• `ProgramOperation::Mirror(_) => {}` s komentárom „A mirror leaves the section the same“. Neplatí, keď os zrkadlenia leží pozdĺž vlákna.<br>• Zaťaženie a reakcie idú zo skutočného zrkadleného telesa, takže zárez sa kontroluje na nesprávnom konci.<br>• Ten istý nosník nakreslený priamo vyjde **fail** (`shear_notch` 1,106 pri stĺpiku a). Zrkadlený do toho istého tvaru vyjde **pass** (0,153).<br>• `mirrored()` je prirodzený spôsob, ako urobiť protiľahlú krokvu. | • Zrkadlenie preniesť do profilu pred rezaním (stanica `u → 2c − u`), alebo pri zrkadlení `not_verified`.<br>• Golden test: zrkadlený = nakreslený. |

### P2

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| WB-4 | P | `program_part_change.rs` (`renamed()`), `program_transform.rs:164-168` | **Premenovanie dielu so zrkadlenou kópiou alebo dieťaťom sa odmietne.**<br>`renamed()` premenuje odvodené mená (`"a mirrored"`, `"a/x"`), ale literál v zdroji ostane. Zrkadlenie cez MCP pomenuje kópiu `"{name} mirrored"`, takže **zrkadlenie a potom premenovanie vždy zlyhá**. | Odvodené mená premenovať len vtedy, keď ich zdroj naozaj odvodzuje, alebo premenovať aj literál kópie. |
| WB-5 | S | `program_part_edit.rs:105`, `live_bridge.rs:2053-2058`, `:2806-2811` | **Odmietnutia zapisovania do programu nespĺňajú AGENTS §2.**<br>• `refused()` stavia text bez `fix_hint` a `target` a kódy nie sú v `rejections`.<br>• Okno všetko zabalí do `program.part_change`, MCP všetko zredukuje na `planning_rejected`.<br>• Pri nezhode chýba riadok aj príčina. `program_parameter_not_driven` cituje riadok vytvárajúceho volania, nie `push_pull` alebo `fillet`. | Štruktúrované `Rejection` z katalógu s `target` = riadok programu a skutočným `fix_hint`. |
| WB-6 | P (čiastočne meranie) | `program_part_change.rs`, `program_part_edit.rs` | **Cena zapisovania a rast programu.**<br>• Každá zmena dielu znamená aspoň dve úplné vyhodnotenia vo workeri, synchrónne na UI vlákne; `driven()` pridá jedno vyhodnotenie na parameter.<br>• `tiny-house` sa vyhodnotí za 56 ms a posun 10 dielov trvá 460 ms. Označiť všetko a posunúť 750 dielov je ~1 500 vyhodnotení, teda desiatky sekúnd zamrznutia.<br>• Do programu pribudne ~750 riadkov `move()`. | Jedno vyhodnotenie na dávku, jeden `move` nad zoznamom, mimo UI vlákna s kontrolou revízie. |
| WB-7 | S | `ketchup-app/src/app/commands.rs:545-549`, `lib.rs:2305` | **Upozornenie na odpojenie programu sa zobrazí len raz.**<br>`program_detach_warned` sa nikdy nenuluje: ani pri Nový, Otvoriť, Undo ani po „Vrátiť úpravu a ponechať program“. Ďalšie odpojenie v tom istom okne prebehne potichu. | Nulovať v `reset_document_presentation` a po reakcii na upozornenie. |
| PG-14 | P | `ketchup-program/src/loads.rs:848-851`, `:860-861` | **Diel, ktorý sa nosiča len zboku dotýka, mu odovzdá svoju váhu (6a9b186).**<br>Blok 52 kg vedľa 20 mm hrany OSB dosky označenej ako nosič, bez spoja a bez uloženia: `unassigned` je prázdne a zaťaženie stĺpika a stúpne z 90,6 na 605,7 N bez varovania. Predtým to bolo „rests on nothing“. Odporuje to vlastnému pravidlu `load_path.rs:346`. | Bočný alebo spodný kontakt prijať ako `fixed_to` alebo `leans` len pri spoji. Inak váhu ponechať a pridať `missing` s pomenovaným kontaktom. |
| PG-14b | P (otvorené) | `ketchup-program/src/loads.rs` (`leans`), `prelude.star` (`buildup()`) | **Opieranie o nosný prvok bez spoja stále prenáša váhu.**<br>Oprava PG-14 vyžaduje spoj len pri bočnom alebo spodnom dotyku s nosnou doskou (`fixed_to`). Diel, ktorý sa opiera o nosný prvok (`leans`), mu váhu odovzdá aj bez spoja. Zámerne ostáva: vrstvy stien v `tiny-house` sa opierajú o stĺpiky bez deklarovaných spojov, takže zmena by veľkú časť domu nechala nenesenú. | `buildup()` nech vrstvy sám deklaruje ako spojené so stĺpikmi; až potom vyžadovať `joint()` aj pri opieraní. |
| PG-15 | P | `member_check.rs:1493`, BASIS r. 167 | **Pravouhlý zárez na ťahanej strane mimo podpery prejde na čistom priereze.**<br>EC5 6.5.1(2) dovoľuje koncentráciu napätia ignorovať len pri sklone 1:10 alebo zárezu na tlačenej strane. Zárez 50 % výšky dolu v strede rozpätia prejde s využitím 0,641 bez poznámky. | `not_verified` s dôvodom pre zmenu výšky strmšiu než 1:10 na ťahanej strane mimo uložení. |
| PG-16 | S | `member_check.rs:416-421`, `:1515-1530` | **Pri pootočenom priereze sa kontroluje len zvislý priehyb.**<br>Pri 100×200 pootočenom o 30° je vodorovná zložka 0,742 zvislej, takže výsledný priehyb je 1,245× kontrolovanej hodnoty. BASIS to neuvádza. | Integrovať aj bočné zakrivenie a kontrolovať výslednicu, alebo to uviesť v BASIS. |
| M-14 | S | `ketchup-model/src/document/product_validation.rs:290-298`, `drawing.rs:2396-2431`, `tests/occurrence_fork_impact.rs:1187-1198` | **Kontrola tuhosti výkresového listu blokuje bežné úpravy (AGENTS §3).**<br>List so zdrojom v tuhej zostave: odzemnenie výskytu alebo nový spoj von zo zostavy odmietne celú dávku `SourceNotRigid`, bez id ani názvu listu. Test toto odmietnutie priamo vyžaduje. | Pri zápise ponechať len `SourceLost`. `SourceNotRigid` hlásiť pri projekcii alebo exporte, s `DrawingSheetId`. |
| M-15 | S | `ketchup-application/src/evaluation_task.rs:271-291`, `ketchup-model/src/document/store.rs:706-776`, `planar_face.rs:155-181` | **Každé exact vyhodnotenie znova publikuje revíziu a zahodí cache.**<br>Pri akomkoľvek ukotvení na plochu `register_derived_result` skopíruje produkt, prepojí každý odkaz (aj rovnaký) a validuje všetko bez `before`. Klon zahodí `ExactGraphCache`, digest aj cachované kódovanie. Inkrementálne vyhodnotenie tak v modeli s náčrtom na ploche prestáva byť inkrementálne. | Preskočiť rovnaké odkazy, pri žiadnej zmene skončiť, validovať zmenu (`validate_product_change`), grafy preniesť. |
| M-16 | S | `product_validation.rs:168-393` | **Každá dávka validuje celý dokument (§3).**<br>Každá dávka znova kontroluje všetky výrobné kódy, spoje, väzby, listy, kolíky, recept zostavy aj voľné cesty. Na 750-dielnom dome to platí aj zmena farby alebo posun. | `ProductChange` rozšíriť o množiny zmenených kľúčov a kontrolovať len dotknuté položky. |
| EX-13 | S | `ketchup-model/src/exact_brep_graph/compiler.rs:638`, `exact_worker.rs:265`, `:1463-1480` | **Graf zahodí index dielu a formát importovaného telesa.**<br>`ImportedExact {sha, len, fingerprint}` nemá `source_part_index` ani formát, takže worker importuje celý súbor ako STEP. Diel z viacdielneho STEP s dierou, posunom alebo v kolíznej dávke zlyhá na „result fingerprint does not match“. IGES sa nedá prečítať vôbec. Tým sa obíde aj cache z EX-8. Žiadny test nepokrýva diel z XDE v grafe; pred opravou ho treba potvrdiť testom. | `format` a `part_index` do `ImportedExact` (schéma aj digest) a rozlíšenie cez `SourceDocument`. |
| SEC-9 | K | `live_bridge/plugin_manifest.rs:312-314`, `:332` | **Čierna listina krátkych prepínačov rozlišuje veľkosť písmen, PowerShell nie.**<br>`CODE_SHORT_OPTIONS = "ceEmMpri"`, takže `-C` (`-Command` v PowerShelli) prejde. `pwsh.exe -C <cmdlet> C:\any\hashed.txt`: hashovaný súbor splní podmienku „menuje skript“ a cmdlet z obyčajných slov sa spustí, ak používateľ schváli bežne vyzerajúci prompt. | Krátkodobo porovnávať bez ohľadu na veľkosť. Systémovo: štruktúrna biela listina pre každý interpret, alebo zrušiť MCP formu príkazového riadku. |
| MF-14 | K | `ketchup-program/library/prelude.star:1498-1518` | **Regresia z MF-8: knižničné spojovacie prvky zablokujú výrobný export.**<br>`_fastener_part` (`bolt`, `nut`, `washer`, `threaded_rod`) nastaví len `designation`, nie `ketchup.fabrication-role.v1`, a `bolt()` neberie `attributes=`. Rolu nenastaví ani `member()` a `board()`. Program s jedným `bolt()` tak odmietne BTLx, woodWOP aj job. Používateľ to nevie opraviť inak, než že skrutku skryje, a tým ju stratí aj z výkazu kovania. Slovník rolí nie je zdokumentovaný (§2). | Rolu nech nastaví knižnica: diel s `designation` je nakupovaný, `member()` je drevený prvok, `board()` je vyrábaný. Alebo nech výroba číta tie isté atribúty ako rozpis materiálu. |
| MF-15 | S | `ketchup-manufacturing/src/capabilities.rs:39`, `fabrication/production.rs:131-134` | **Betón ide na CNC.**<br>Po 90e3a48 idú pätky (600×1100×1100 betón) a komín do woodWOP a výrobného jobu. Bez výrobného kódu job padne na všeobecné `ExportBlocked`. | Druh položky „na stavbe / neobrába sa“: v BOM a rozpise, v žiadnom strojovom formáte. |
| MF-16 | S | `ketchup-manufacturing/src/project_drawings.rs:160-183` | **Projektové výkresy nekreslia obrysy valcov (regresia z 8362a0f).**<br>Kreslia sa len hrany medzi dvoma plochami a okraje. Bok valca je jedna plocha so skrytým švom, takže okrúhly stĺp v pohľade má len horný a dolný okraj, bez zvislých obrysov. To isté platí pre vodorovnú rúru. Na čiernobielej tlači tvar takmer zmizne. | Pridať obrysové hrany závislé od pohľadu (zmena znamienka `dot(n, depth)` medzi susednými trojuholníkmi), alebo HLR z OCCT. |
| MF-17 | K | `ketchup-manufacturing/src/fabrication.rs:1439`, `:1673-1716` | **BTLx nevie vyvŕtať priechodnú dieru.**<br>`btlx_drilling` odmietne `through: true` aj dieru, ktorá dosiahne protiľahlú plochu, a šablóna má napevno `<DepthLimited>yes`. Diera pre skrutku cez trám, najbežnejšie obrábanie dreva, tak export BTLx odmietne. Tabuľka schopností to teraz aspoň pomenuje (062ab1b). | `DepthLimited` = `no` s hĺbkou výstupu zo skladu. |
| UI-14 | S | `ketchup-app/src/app/document.rs:932-980`, `app/import.rs:239-257` | **Import STEP a IGES blokuje okno.**<br>Inšpekcia XDE beží synchrónne na UI vlákne a jej príznak zrušenia sa nikdy nenastaví. 200 MB súbor zamrazí okno bez možnosti zrušiť. | Cez `FileActivity` ako `dwg_reading`. |
| UI-15 | S | `ketchup-app/src/file_activity.rs:108-111` | **Otvoriť môže potichu nič neurobiť.**<br>Dokončené čítanie sa zahodí, ak sa medzitým zmenila revízia (napr. úprava cez MCP), bez správy. | Povedať prečo, alebo sa spýtať znova. |
| VW-16 | S | `viewer/android/src/files.rs:233-240`, `viewer/web/src/lib.rs:107-112` | **Viewer prepíše neuložené komentáre.**<br>• Android: súbor odovzdaný inou aplikáciou otvorí `app.open` bez `may_discard_comments`. Obnovené neuložené komentáre tak ťuknutie na `.ketchup-view` v pošte potichu nahradí.<br>• Web: neskoro dokončené sťahovanie zdieľaného odkazu prepíše lokálne otvorený a okomentovaný balík a vypne varovanie pri odchode. | Jedna brána `request_open(source, generation)` v `ViewerApp` s `may_discard` a zahodením zastaraných výsledkov. |
| TST-6 | S | `ketchup-headless/src/protocol.rs:1900` | **Ďalší časový rozpočet v testoch pre každý PR.**<br>`elapsed < total + 1s` po kalibračnej slučke beží na Windows runneri s 2 vláknami a `check_test_timing` ho nezachytí. | Overiť len `time_limit`; rozpočet presunúť do `perf_*`. |

### P3

- **Zapisovanie do programu:**
  - Dve volania na jednom riadku: úprava `b` cieli na `a`. Posun sa zachráni, farba sa
    odmietne. Treba porovnávať podľa riadku aj stĺpca.
  - Farba za komentárom v poslednom argumente sa vloží do komentára a úprava sa odmietne.
  - Šum v číslach: `distance=0.30000000000000004` a zaoblenie alebo skosenie bez `written()`.
  - Premenovanie na meno, ktoré už prevzal `continue_part`, dá surovú chybu vyhodnotenia.
  - Zmazanie dielu v `if` sa odmietne zavádzajúcou správou.
  - Dve premenovania v jednej dávke odpoja program.
  - Po zmazaní posledného dielu s ponechaným komentárom alebo `param()` vlastní program
    prázdny dokument, ale upozornenie `emptied` sa neukáže.
- **Statika a program:**
  - kh sa berie zo zrezanej výšky (+8 % pri záreze, EC5 3.2(3) chce výšku prvku).
  - Kontrola 6.5.2 sa pri uložení dlhšom ako 400 mm vynechá.
  - Limit 1 GiB nepočíta pamäť na strane Rustu: 250× `print` po 8 MB dá 6,2 GB, atribúty
    dielov 3,5 GB. Worker to zastaví, CLI nie, a hláška smeruje inam. **PG-17: opravené
    okrem kópie v `face_cache`.** `print()` riadky a text dielov (meno, materiál, atribúty,
    tagy, aj pri `copy()`) sa počítajú do limitu cez `Heap::hold_outside`, chyba je
    `memory_limit` s nápovedou a zastaví aj CLI. **Otvorený zvyšok:** diel, na ktorého plochy
    sa program pýta, má v `eval/face_cache.rs` ešte jednu kópiu vrátane atribútov, ktorá sa
    nepočíta. Najhoršie tak tento text zaberie ~2× limit (nie neobmedzene).
  - Pri 96a6222 hrebenáč dostane sneh len z plôch nad krytinou.
- **Model:**
  - `register_exact_references` stojí odkazy × prvky (~4,5k × 2k na dome).
  - Pri importe sa každý blob hlboko kopíruje (`Arc::make_mut` po klonovaní), pri STEP 3 × 30 MB.
- **Exact:**
  - Na Unixe „odpojené“ MCP okno ostane v skupine procesov klienta, takže Ctrl+C ho zabije.
  - `killpg` po zobratí skupiny môže zasiahnuť znova použité PGID.
  - Headless berie relatívny `--worker`, a každé vyhodnotenie potom zlyhá.
  - Chyby v cachovanom dokumente nesú cestu k zmazanému dočasnému súboru.
  - Mapovaný sieťový disk (`Z:\`) prejde politikou ciest.
  - Ukončenie cez `process::exit` namiesto `_exit`.
  - Po smrti rodiča pred štartom vlákna worker nikdy neskončí.
  - Správy IGES hovoria „STEP“.
  - Náhľad histórie prvkov zahodí vyhodnotenie scény.
- **GUI:**
  - Zrušené Otvoriť drží zámok, takže okamžité opätovné otvorenie hlási konflikt.
  - Dve okná nad tým istým súborom: úprava v druhom okne hlási „chyba uloženia“.
  - Na MacBooku chýba kláves Delete (posiela Backspace).
  - Výraz na plátne nemôže začať `(` ani `+`.
  - Správy v `ketchup-interaction` sa nahrádzajú postupne a bez pluralít.
  - „Face Offset“ a „Pad“ sú po anglicky v slovenskej histórii prvkov.
  - Príliš veľa kópií hlási „nedostupné“ namiesto „najviac N“.
  - Okno listu číta nastavenia pri každej snímke.
  - Najväčšie súbory a funkcie sú bez zmeny (`viewport.rs` 4 760; 4 funkcie nad 300
    riadkov).
- **Výroba a výkresy:**
  - Riadok kusovníka prekročí stĺpec 70 mm a „(mirrored)“ je po anglicky.
  - Rohové razítko projektového listu pretečie.
  - Diel iba s náčrtom sa odmietne s hláškou, že má teleso.
  - Výrobný job odmieta všeobecným `ExportBlocked` a headless chyby sploští.
  - DXF neinformuje o použitej kódovej stránke.
  - `?` na výrobnom liste sa tlačí bez varovania.
- **Viewer, CI a testy:**
  - `exercise_crash_recovery` nemôže zlyhať (worker dokument nikdy nemení).
  - Testy `atomic_save` (Windows a macOS) ani `browser_smoke.py` nebežia v CI.
  - `startup.rs` zahodí správu panic.
  - Android pri zlyhanej obnove session len zapíše log.
  - Fuzz job nenahráva artefakty, nefuzzuje DWG ani `Comments::read_unbound` a nájdený
    pád zablokuje nesúvisiaci PR.
  - Verzia je na troch miestach.
  - Tri testy v `packages.rs` overujú len `is_err()`.
  - `check_ci_crates` uzná `-p` v `cargo build` aj `if: false`.
  - `check_test_sleeps` neuvidí `sleep (` ani `park_timeout`.
  - `check_ignored_tests` vynechá test s textom „subprocess“ v dôvode a nevidí `--skip`.
  - Sprievodca jazykom nepopisuje obmedzenia `shell` (`open`) a `union`.

## 5. Metriky

| | dnes |
|---|---:|
| ratchety `check_*.py` | 12, všetky zelené |
| lokálny Linux job | 1 398 testov, 0 zlyhaní (10-18: 1 370) |
| najväčší súbor GUI | `app/viewport.rs` 4 760 |
| produkčné funkcie GUI nad 300 riadkov | 4 (`show_assembly_editor_content` 746, `show_feature_history_content` 628, `dispatch_command` 357, `paint_viewport_scene` 347) |

## 6. Štrukturálne návrhy

1. **Zapisovanie do programu: jedna prísna kontrola a presné rozsahy volaní** (WB-1 až
   WB-6).
   - Každá úprava je „model pred“ plus explicitná záplata a porovná sa celý model. `Reads`
     a slepé `Renamed` sa zrušia.
   - Úprava cez `param()` vymenuje ďalšie diely, ktoré zmení, a pýta sa.
   - `part_sources` zaznamená presný rozsah volania (riadok a stĺpec) a pozície argumentov,
     ktoré odkazujú na diel.
   - Zapisovač dávok mimo UI vlákna: jedno vyhodnotenie na dávku a úprava literálu
     posledného `push_pull`, `move` alebo `rotate` z GUI namiesto pridávania riadkov.
2. **Jeden model prierezu pozdĺž vlákna z hotového telesa** (PG-13, PG-15, PG-16, PG-11):
   zrkadlenia, rezy a diery sa aplikujú vo svetových osiach. `member_check`,
   `material_along` aj kontroly stĺpov čítajú z neho.
3. **Pravidlá cesty zaťaženia na jednom mieste** (PG-14): jeden druh kontaktu (leží, visí,
   upevnený spojom, dotýka sa) pre `load_path.rs` aj `loads.rs`.
4. **Validácia v dvoch triedach** (M-14, M-16, M-15).
   - Integrita (neznáme id, cykly, nekonečné čísla) blokuje, ale len nad kľúčmi, ktoré dávka
     zmenila.
   - Všetko ostatné je zoznam issues.
   - Registrácia odvodených dôkazov zachová zdieľanie: bez zmeny nevznikne nová revízia.
5. **Jedna služba importovaných zdrojov vo workeri** (EX-13, EX-8, EX-9). Uzly grafu nesú
   formát a index dielu. LRU vážené bajtmi a započítané do rozpočtu cache slúži inšpekcii,
   tessellácii, grafu, párom aj konverzii na IGES.
6. **Jeden zdroj druhu položky** (MF-14, MF-15): rola z atribútov, ktoré nastaví knižnica,
   plus druh „neobrába sa“, spoločná pre výrobu aj rozpis materiálu.
7. **Jedna cesta úloh na pozadí pre každú akciu so súborom** (UI-11, UI-14, UI-15, EX-10,
   MCP-3). Import, export, otvorenie a list idú cez `FileActivity` so zrušením, hlásením
   zastaraného výsledku a zápisom „nesmie existovať“.

## 7. Odporúčané poradie

1. **Hneď:**
   - WB-1, WB-2, WB-3 (zapisovanie do programu mení model alebo program potichu);
   - PG-13 (zrkadlený nosník „pass“);
   - MF-14 (skrutka zablokuje výrobu).
2. **Tento týždeň:**
   - WB-4, WB-7;
   - SEC-9;
   - PG-14, PG-15;
   - EX-13;
   - M-14;
   - MF-16, MF-17;
   - UI-14;
   - VW-16;
   - a otvorené P2 z 10-18, ktoré nemajú commit: M-3, UI-10, MCP-3, EX-9, EX-10, VW-14,
     VW-15, REL-1, TST-5.
3. **Potom:** zvyšok P2 a nízke body. Štrukturálne body 1, 2 a 4 ako samostatné bloky podľa
   AGENTS §6.
