# Podrobný code review Kečupu — 2026-10-15

Stav stromu: `52a0828` (vetva `claude/adoring-carson-6poeki`, zhodná s `main`). Nadväzuje na
`docs/review-2026-10-14-status.md`. Nálezy, ktoré tam už sú opravené, sa tu neopakujú.

## 1. Metóda

- Šesť nezávislých čitateľov po oblastiach:
  - model a perzistencia;
  - hranice a bezpečnosť (MCP, bridge, pluginy, workery);
  - GUI a nástroje;
  - výroba a exporty;
  - geometria, exact a programový jazyk;
  - viewer, CI a hygiena repozitára.
- Posledných 50 commitov (všetky z 15. 10.) sa čítalo po diffoch.
- Spustené na Linuxe:
  - všetky `scripts/check_*.py`: prešli, tri ratchety majú rezervu na `--update`;
  - `cargo fmt --all -- --check`: prešlo;
  - `cargo clippy` nedobehne, lebo `ketchup-exact/build.rs` vyžaduje
    `third_party/occt-install-r0-v1` (pozri CI-1).
- Stĺpec **Overenie**:
  - **K**: mechanizmus som sám overil čítaním kódu.
  - **S**: zatiaľ len statický nález pomocného čitateľa, s cestou od volajúceho, nepotvrdený
    mnou.
  - Nič nebolo reprodukované spustením; produkčný kód som nemenil.
- Priorita:
  - **P1**: strata dát, nebezpečný alebo vyrobiteľný zlý výsledok, zamrznutie okna.
  - **P2**: konkrétna chyba s menším dopadom.
  - **P3**: dlh alebo hygiena.

## 2. Záver v skratke

**Čo je dobré:**

- Hranice voči nedôveryhodnému vstupu sú poctivo ohraničené: dĺžky rámcov, CBOR, JSON, GLB
  accessory, strom uzlov vo viewer balíku. Webový viewer nepoužíva `innerHTML`.
- Kľúče cache v exact grafe pokrývajú všetky vstupy a výstup je deterministický.
- Kontrola projekčných tabuliek prešla:
  - stúpania ISO 261 a výbehy DIN 76-1;
  - koeficienty EC5 (kmod, kdef, γM, kh, kv, kc, kcr);
  - parita zrkadlenia v BTLx a woodWOP.
- Ratchety v CI sú zelené a klesajú.

**Čo je zlé:**

- **Program dokumentu žije len v Undo histórii.** Keď sa história pri otvorení zahodí, súbor
  sa otvorí ako „kompletný“ bez programu. Ďalšie uloženie ho stratí natrvalo (PS-1).
- **Validačný report programu beží v procese okna bez limitu.** Izolácia z PRG-M1 kryje len
  vyhodnotenie, nie drahšiu polovicu (PRG-1).
- **Box-level kolízie ignorujú `mirror()` pri booleanoch** a otočené drážky posudzujú cez
  svetový AABB. Reálny prienik sa tak môže ohlásiť ako zámer (PRG-2, PRG-4).
- **Move s napísanou vzdialenosťou počas gesta** posunie diel po svetovej X, nie smerom
  ukazovateľa (UI-1).
- **Rust sa v CI prekladá len na jednom self-hosted Windows runneri.** Android, web a macOS
  viewer sa v CI nikdy neprekladajú. PR z forku má zelenú bez buildu (CI-1 až CI-3).

## 3. Register nálezov

### P1

| ID | Overenie | Kde | Nález | Návrh opravy |
|---|---|---|---|---|
| PS-1 | K | `ketchup-model/src/persistence.rs:1684-1704`, `:534-554`, `:641-663` | **Strata programu pri zahodenej histórii.**<br>• Program dokumentu (`RuleProgramSource`) sa zapisuje len do `history.bin`; `document.bin` (snapshot codec) ho nenesie.<br>• Ak `decode_revision_history` zlyhá inak než `InvalidRuleProgram`, `load_container` iba vyplní `audit.history_discarded` a vráti store zo snapshotu bez programu.<br>• Príčiny zlyhania: staršia revízia sa po sprísnení validácie (napr. b31f9c4) už neotvorí, poškodený chunk, rozpočet položiek čítača (PS-9).<br>• `DocumentSession::open` audit neukáže. Používateľ vidí „model je kompletný“ a ďalšie Save program zahodí.<br>• `migration_review.rs:171` ukladá cez `save_container`, takže program zahodí rovnako. | • Program aktuálnej revízie uložiť mimo histórie: povinný záznam `program.json` alebo pole v `document.bin` pod novým číslom formátu.<br>• Krátkodobo: pri zlyhaní histórie aspoň dekódovať program z cursor záznamu, ktorý predchádza snapshotom.<br>• Zahodenie histórie vždy ukázať ako varovanie.<br>• Test: história so zlou starou revíziou sa otvorí s programom a Save ho zachová. |
| PRG-1 | K | `ketchup-application/src/program_isolation.rs:45-53`, `ketchup-program/src/validate.rs:374-476` | **Report programu bez limitu v procese okna.**<br>• `run_program` vyhodnotí program vo workeri, ale `ketchup_program::report()` (kolízie, relácie) volá v procese okna.<br>• `collisions` nemá strop na páry ani issues.<br>• Príklad: 20 000 boxov so spoločným min-x obíde sweep-and-prune, čo dáva rádovo 10⁸ párov. Každý pár rátajú polytopové enumerácie a vznikne formátovaný Issue.<br>• Výsledok sú hodiny CPU a GB pamäte v okne. Limity workera z PRG-M1 sa neuplatnia. | • Presunúť `report()` do program workera pod rovnaký deadline a pamäťový limit.<br>• Strop issues na druh: prvých N plus počet.<br>• Test: veľký program skončí pomenovaným odmietnutím a okno žije. |
| PRG-2 | K | `ketchup-program/src/validate.rs:233-262`, `faces.rs:746-758`, `relations.rs:120-195` | **`mirror()` sa neuplatní na boolean nástroje v box-level úvahách.**<br>• `after_operation` zrkadlí len diery a kapsy.<br>• `booleans_leave_overlap`, `trims_separate` a `shared_volume` čítajú `tool.obb()` v polohe pred zrkadlením.<br>• Príklad: diel 100×20×20, `subtract` nástroja na x 0–20, potom `mirror(axis="x")`. Zárez je reálne na x 80–100, ale diel `q` na x 0–20 sa vyhodnotí ako „odobraný nástrojom“.<br>• `collisions` dá `continue` pred `needs_exact_shapes`, takže sa presná odpoveď ani nespýta. Prienik sa nenahlási.<br>• Opačne: diel sediaci v skutočnom záreze dostane falošnú kolíziu. | • Jedna funkcia „poloha na hotovom diele po neskorších operáciách“, používaná pre diery, kapsy, boolean nástroje aj spoje.<br>• Kým nie je: boolean nasledovaný mirrorom je na boxoch nerozhodnuteľný a ide na exact.<br>• Test so zrkadleným zárezom v oboch smeroch. |
| UI-1 | K | `ketchup-app/src/app/commands.rs:771-791`, `:832-880` | **Move: napísaná vzdialenosť počas gesta ide po svetovej X.**<br>• Bez zamknutej osi dá `150` hodnotu `axis_travel = None` a `parse_move_vector` chce 2–3 zložky, preto `typed = None`.<br>• Vetva so vzdialenosťou nevolá `take_move_session` a skončí v `move_selected(Vec3::new(distance_mm, 0, 0))`.<br>• Príklad: M, klik na bázu, ukazovateľ smerom +Y, `150`, Enter. Diel sa posunie o 150 mm po +X.<br>• Session ostane visieť so starou revíziou, ďalší klik zahlási `error-preview-stale`.<br>• Testy ukazujú len pozdĺž +X (`manual_workflow_ui.rs:101-105`), preto chybu nezachytia. | • Vo vetve so vzdialenosťou najprv `take_move_session(None)`, potom `delta = normalize(drag.delta_mm) * distance` a commit.<br>• Test so smerom +Y aj šikmo. |

### P2

| ID | Overenie | Kde | Nález | Návrh opravy |
|---|---|---|---|---|
| SEC-1 | K | `live_bridge/plugin_manifest.rs:265-275`, `:444-453` | **Argument pluginu s `..` beží mimo stage.**<br>• Argument sa kontroluje lexikálne (`lexical(folder.join(arg)).starts_with(folder)`), ale do procesu ide surový text.<br>• `in_stage` cez `strip_prefix` nechá `..` a proces beží s `cwd = stage`.<br>• Príklad: `"../plug/run.py"` sa pri súhlase hashuje ako súbor priečinka, ale spustí sa `%TEMP%\plug\run.py`, ktorý nikto nehashoval ani neukázal. | • Odmietnuť argument s `ParentDir`.<br>• Do príkazu dávať `stage.join(candidate.strip_prefix(folder))` z lexikálneho kandidáta, nie zo surového textu. |
| SEC-2 | S | `plugin_consent.rs:76-79`, `plugin_manifest.rs:286`, `ketchup-scheduler/src/plugin.rs:28` | **Windows `Command` doplní `.exe`, takže hashovaný súbor nie je spúšťaný súbor.**<br>• `C:\Py\python` sa hashuje, ale spustí sa `python.exe`.<br>• Absolútny executable, ktorý nie je súbor, sa ani nehashuje, ani neodmietne. | Na Windows vyžadovať príponu `.exe`, alebo vyriešiť cestu ako std a hashovať výsledok. Neexistujúci executable odmietnuť. |
| SEC-3 | S | `plugin_manifest.rs:119-179`, `:350-374`, `plugin_menu.rs:75` | **Staging pluginu blokuje UI vlákno.**<br>• Beží synchrónne na UI vlákne pred čítaním `plugin.json`.<br>• Kopíruje až 256 MiB z ľubovoľného priečinka.<br>• Otvára aj ne-regulárne súbory: FIFO na Unixe zablokuje `open(2)` navždy. | • Najprv manifest.<br>• Kopírovať len regulárne súbory, s kontrolou na otvorenom handle.<br>• Staging a hash presunúť do plugin workera. |
| SEC-4 | S | `live_bridge/consent.rs:252-267`, `app/document.rs:362-367`, `export*.rs` | **Sanitizácia SEC-3 z 10-14 kryje len prompt pluginu.**<br>Cesty od klienta idú do ostatných high-risk dialógov (open, save-as, HOMAG, bundle) surové. Bidi znaky a `\n` v nich môžu podvrhnúť riadok `Path:`. | Escapovať v `HighRiskConfirmationRequest::new`, aby to platilo pre každý prompt. |
| SEC-5 | S | `live_bridge/interchange.rs:95-118`, `consent.rs:253`, `bootstrap.rs:255` | **Súhlas s otvorením súboru sa dá obísť.**<br>• `Request::Import` načíta ľubovoľný lokálny STEP/STL/DXF/DWG/GLB bez promptu.<br>• V okne spustenom cez MCP `open_window` sa súhlas s otvorením vôbec nepýta. | Jedna politika „klient číta lokálny súbor“ pred open aj import, v oboch režimoch pripojenia. |
| PS-2 | S | `persistence.rs:1413-1480`, `:1532`, `app/document.rs:1222-1260` | **Neplatný `.work-recovery` zablokuje úpravy.**<br>• Zastaraný alebo neplatný recovery súbor sa pri otvorení potichu preskočí (`Ok(None) \| Err(_)`).<br>• Prvá úprava potom narazí na `ExternalConflict` a vráti sa späť. Dokument sa nedá upravovať ani uložiť a obnovenú prácu nikto neponúkne. | • Vrátiť `SkippedRecovery { reason, identity }`.<br>• Ponúknuť Zahodiť alebo Odložiť (premenovať na `.stale`). |
| PS-3 | S | `persistence.rs:1540-1545` | **Súbor z novšej verzie sa otvorí ako starší `.recovery`.**<br>• `UnknownFields` a `UnsupportedSchema` spadnú na `.recovery` fallback.<br>• Súbor z novšej verzie sa tak otvorí ako jeho staršia záloha a ďalšie Save prepíše novšiu prácu. To popiera 0d09dbe. | Tieto chyby (aj `UnsupportedHistorySchema` a `UnsupportedContainerSchema`) brať ako konečné. |
| PS-4 | S | `persistence/snapshot_codec.rs:210` | **`unknown_fields` je kvadratické.**<br>• Pre každý kľúč robí lineárne `find`.<br>• Upravený súbor s mapou s ~500k kľúčmi potrebuje ~10¹¹ porovnaní, takže otvorenie zamrzne. | Merge prechod dvoch kanonicky zoradených máp, alebo `HashSet` známych kľúčov. |
| PS-5 | S | `persistence.rs:448-463`, `store.rs:364,1019` | **Cache kódovania sa pri zdieľanom produkte nepoužije.**<br>• Cache je jeden `OnceLock` na zdieľanom `ProductModel`.<br>• Revízie, ktoré produkt zdieľajú (úprava zdrojového textu programu, rollback), sa kódujú vždy nanovo.<br>• Každý zápis work-recovery je preto O(N × veľkosť dokumentu). | Cache na `Revision`, alebo kľúčovať podľa revision id. |
| PS-6 | S | `persistence.rs:1178,1193`, `session.rs:205-250` | **Save robí dvojnásobnú prácu.**<br>• Každé Save dvakrát úplne dekóduje celú históriu: kontrola nového súboru a rozhodnutie o `.recovery`.<br>• Headless/MCP session navyše kóduje dvakrát.<br>• Na `TooLarge` nemá fallback ako GUI (`save_activity.rs:10-18`). | Jedna pipeline `prepare` → `publish` pre app, session aj recovery. Spätne overiť len checksum kontajnera a `document.bin`. |
| PS-7 | S | `document/store.rs:797-833` | `history_digest()` serializuje do JSON program každej revízie, 2–3× na mutáciu. | Digest programu počítať raz pri naviazaní a cachovať ho na revízii. |
| PRG-3 | S | `ketchup-program/src/eval.rs:1250-1256`, `ketchup-scheduler/src/lib.rs:1051` | **Exponenciálny rast boolean stromu.**<br>• `boolean()` hlboko klonuje celý strom nástroja.<br>• Striedavé `union(a,b); union(b,a)` rastú Fibonacciho radom, mimo rozpočtu starlark heapu.<br>• Na Unixe `spawn_limited_worker_command` pamäťový limit zahodí. | • Nástroje cez `Rc` alebo index.<br>• Strop na uzly a hĺbku.<br>• Rátať veľkosť modelu do rozpočtu.<br>• `setrlimit` na Unixe. |
| PRG-4 | S | `ketchup-program/src/validate.rs:216-231`, `:395-397`, `relations.rs:197` | **Výnimka „sedí v drážke“ ráta so svetovým AABB.**<br>Pri diele otočenom o 45° je AABB drážky ~78×78. Blok v plnom dreve vedľa drážky sa tak uzná ako osadený a kolízia sa nenahlási. | Test obsahu v lokálnom rámci dielu: vrcholy prekryvu premietnuť do rámca a porovnať s lokálnou kapsou. |
| PRG-5 | S | `program/src/exact.rs:29-31`, `eval.rs:39`, `exact_worker.rs:2184` | **Tri rôzne prahy pre „prienik“.**<br>• Box cesta: 0,01 mm tolerancia.<br>• Exact cesta: `common_volume > 0`.<br>• Worker: tretí prah.<br>Dotyk s prekryvom 1e-5 mm na 10⁴ mm² je teda v exact chyba, v boxoch kontakt. | Jeden predikát kontakt/prienik podľa odhadnutej hĺbky (objem / prierez), so spoločným prahom z `ketchup-tolerance`. |
| MF-1 | S | `ketchup-manufacturing/src/fabrication.rs:3074-3086`, `product_validation.rs:188-192` | **Závit sa neoveruje voči diere, do ktorej patrí.**<br>• Pri exporte sa závit nekontroluje: hĺbka minus výbeh, vrták = jadrový, stúpanie k veľkosti.<br>• Príklady: súbor spred d64ebd2 (M8 12 mm v 12 mm slepej diere), neskoršia zmena hĺbky alebo priemeru, surový `SetHoleThread`.<br>• BTLx, výrobný job aj výkres napíšu „M8×1,25 hĺbka 12“ bez varovania. | • Jedno pravidlo `MetricThread::admits(diameter, depth, through)`, volané pri tvorbe, pri exporte aj vo validácii.<br>• Pri exporte pomenovaná chyba `TappedHoleInvalid`. |
| MF-2 | K | `fabrication.rs:4038`, `:4090-4110` | **SVG oblúky na výrobnom výkrese sú doplnkové.**<br>• `point()` neotáča os y.<br>• Kód nútí kladný sweep pre `clockwise` a zapíše `sweep-flag = clockwise`.<br>• Prehodené sú tak oba flagy (large-arc aj sweep) a SVG nakreslí doplnkový oblúk tej istej kružnice: štvrťkruh ako 270°.<br>• BTLx (`btlx_arc_points`) je správne. | Sweep počítať ako `btlx_arc_points` (clockwise → záporný), zapísať `sweep-flag = !clockwise`. Test, ktorý porovná vzorky SVG oblúka s BTLx bodmi. |
| MF-3 | S | `fabrication.rs:1373-1396` | **BTLx nesie závit len ako metadáta.**<br>• Závit je len v `Comment` a `UserAttributes`. Stroj vyvŕta jadrový otvor bez závitu, teda rovnaká tichá strata, pre ktorú sa woodWOP odmieta.<br>• Nie je overené, že tvar `UserAttribute` a `Comment` na processingu prejde XSD 2.3.1. | • Golden súbory validovať proti oficiálnemu XSD.<br>• Stratu závitu uviesť v loss reporte, alebo ju odmietnuť ako vo woodWOP. |
| VW-10 | S | `viewer/android/src/lib.rs:198`, `ketchup-viewer/src/app/session.rs:62-71`, `view-format/src/container.rs:105` | **Android session prepisuje celý balík na UI vlákne.**<br>• `keep_session` pri každej zmene komentárov prepíše celý balík s `fsync`.<br>• Pri 120 MB balíku (VW-6) to znamená +120 MB alokácie a blokovanie UI, čím hrozí ANR.<br>• SHA balíka sa pri validácii ráta dvakrát. | • Balík zapísať raz, komentáre do malého samostatného súboru.<br>• Streamovať a mimo UI vlákna.<br>• `payload_sha256` cachovať na `Package`. |
| UI-2 | S | `ketchup-app/src/app/drawing.rs:3152-3160` | **Ctrl+Shift+Z v poli hodnoty zruší operáciu namiesto Redo.**<br>`consume_shortcut(COMMAND+Z)` cez `matches_logically` ignoruje Shift a beží pred keymapou. | Presná zhoda modifikátorov, alebo testovať `CTRL_SHIFT+Z` ako prvé. |
| UI-3 | S | `app/commands.rs:627-633`, `app/transform.rs:748-758` | **Odmietnutá korekcia Scale ukončí korekciu.**<br>• Príklad: ×2, potom `0` (odmietnuté), potom `3` dá ×6 ako nový krok namiesto ×3.<br>• `1` sa ako korekcia nedá zadať. Rotate a Push/Pull sa správajú opačne. | Pri odmietnutej hodnote nevolať `end_transform_correction`. Identitu povoliť ako korekciu. |
| UI-4 | S | `app/shell.rs:512-536`, `app/document.rs:1373-1383` | **Ctrl+Z počas rozkresleného tvaru vráti skorší krok.**<br>Ak počas rozkresleného tvaru nie je napísaná hodnota, Ctrl+Z vráti skorší potvrdený krok dokumentu a tvar zahodí. `ephemeral_edit_active` nepočíta `sketch.start`. | Pridať `gesture.sketch.start.is_some()` do `ephemeral_edit_active`. Test bez napísanej hodnoty. |
| CI-1 | K | `.github/workflows/ci.yml:55`, `ketchup-exact/build.rs:60` | **PR z forku má zelenú bez buildu.**<br>• Rust job beží len na self-hosted Windows a pre PR z forku sa preskočí.<br>• Preskočený povinný check sa v GitHube počíta ako úspešný.<br>• Mimo runnera sa nedá spustiť ani clippy, lebo build skript `ketchup-exact` panikuje bez OCCT. | • GitHub-hosted Linux job s `fmt`, `clippy` a testami crate, ktoré OCCT nepotrebujú (`ketchup-view-format`, `ketchup-viewer`, `ketchup-rejection`, `ketchup-tolerance`, `ketchup-program` …).<br>• `ketchup-exact` za cfg/feature „bez OCCT“ alebo za stub.<br>• Pre fork explicitný gating job. |
| CI-2 | S | `viewer/android`, `viewer/web`, `viewer/macos`, `rust-toolchain.toml` | **Android, wasm a macOS kód sa v CI nikdy neprekladá.**<br>Týka sa to aj JNI a fd kódu z VW-6. | Pridať targety `aarch64-linux-android` a `wasm32-unknown-unknown` a spúšťať `cargo clippy --target …`. Pre macOS job `macos-latest`. |
| CI-3 | S | `scripts/check_error_hygiene.py`, `.github/CODEOWNERS` | **Ratchet chybovej hygieny má diery.**<br>• Neskenuje `viewer/*/src` (`viewer/android/src/files.rs` má 5× `to_string()`).<br>• Nechytá `format!("{error:?}")` ani `map_err(ToString::to_string)`.<br>• `--allow-growth` existuje.<br>• Baseliny okrem named products nie sú v CODEOWNERS. | • Rozšíriť rozsah a vzory.<br>• Odstrániť `--allow-growth`.<br>• Dať `scripts/*baseline*` a `scripts/check_*.py` pod CODEOWNERS. |

### P3

| ID | Overenie | Kde | Nález a návrh |
|---|---|---|---|
| PRG-6 | S | `third_party/starlark/src/values/types/string/methods.rs:766-797` | **Neúplné obmedzenie alokácií v starlarku.**<br>• Nález: `str.replace`, `format`, `%`, `repr`/`str` a `json.encode` obchádzajú `check_allocation`. BigInt `x*x` nemá strop.<br>• Dopad: na Windows to zastaví job limit workera z PRG-M1. Na Unixe a v CLI `ketchup-program check` limit nie je.<br>• Návrh: kontrolu dať do alokátora heapu, nie do jednotlivých operácií. |
| PRG-7 | S | `program/src/eval.rs:1279-1293` | **Pridanie operácie je O(n).**<br>• Nález: `add_operation` pri každej diere prestavia celý strom prvkov, takže n dier je O(n²). Čas sa kontroluje len každých 256 príkazov.<br>• Návrh: inkrementálny strom. |
| PRG-8 | S | `program/src/eval.rs:39`, `sketch.rs:22,541` | **Vlastný epsilon v programe.**<br>• Nález: `TOLERANCE_MM = 0,01` ignoruje `TolerancePolicy` dokumentu, proti pravidlu crate `ketchup-tolerance`.<br>• Návrh: odovzdať politiku do programu. |
| EX-1 | S | `exact_worker.rs:936-972` | **Cache výstupov bez bajtového rozpočtu.**<br>• Nález: cache má 512 B-repov bez obmedzenia veľkosti a worker nemá pamäťový limit.<br>• Návrh: pridať bajtový rozpočet. |
| PS-8 | S | `persistence.rs:539,2216,2258`, `snapshot_codec.rs:142` | **Chyby so stratenou príčinou.**<br>• Nález: `InvalidPayload(format!(…))` a pomocník `invalid_payload` schovávajú `to_string()` pred ratchetom. `try_load_recovery` zahodí chybu zálohy.<br>• Návrh: typované polia namiesto textu (AGENTS §2). |
| PS-9 | S | `persistence.rs` (`encode_history`, `Reader::count_with_limit`) | **Zapisovač nedrží rozpočet čítača.**<br>• Nález: zápis nekontroluje rozpočet 500k položiek, ktorý drží čítač. Uložená história sa tak nemusí dať otvoriť, a cez PS-1 zoberie so sebou program.<br>• Návrh: rovnaké počítadlo pri zápise. |
| PS-10 | S | `persistence.rs:573-603,690-693,868-878` | **Nekontrolované konverzie.**<br>• Nález: `as u32` a `as u64` v zapisovači histórie.<br>• Návrh: `try_from` ako vo zvyšku modulu. |
| PS-11 | S | `document/store.rs:1116-3182` | **Príliš veľká funkcia a klonovanie modelu.**<br>• Nález: `apply_batch_with_origin_and_validation` má ~2 070 riadkov, AGENTS §6 hovorí ~150. Každá dávka klonuje celý `ProductModel`, aj prepnutie viditeľnosti.<br>• Návrh: dispatch po rodinách príkazov do vlastných modulov a copy-on-write produktu. |
| PS-12 | S | `persistence.rs` (`decode_revision_history`) | **Zbytočne načítané revízie.**<br>• Nález: staré súbory s až 4 096 revíziami sa načítajú celé, hoci sa ponechá 200.<br>• Návrh: dekódovať len ponechané okno. |
| UI-5 | S | `app/gesture.rs:56`, `viewport.rs:2473-2478` | **`shown_reading` sa nenuluje.**<br>• Nález: po Esc a novom tvare na tom istom bode ostane pole hodnoty prázdne.<br>• Návrh: nulovať spolu so `start`. |
| UI-6 | S | `drawing.rs:3525`, `commands.rs:1503`, `scale_grips.rs:25` | **Každý nástroj číta hodnotu inak.**<br>• Nález:<br>• `80*60` sa už neprijme.<br>• Move nečíta jednotky, aritmetiku ani desatinnú čiarku.<br>• V Scale je holé `150` faktor, inde mm.<br>• Návrh: jeden parser nad `length_input` s nastavením pre nástroj. |
| UI-7 | S | `crates/ketchup-app/src/lib.rs` | **`lib.rs` rastie.**<br>• Nález: za 25 commitov 2 893 → 2 906 riadkov (ff5d905), proti AGENTS §6.<br>• Návrh: typy drag a session presunúť do `gesture.rs`. |
| MF-4 | S | `prelude.star:1424`, `entities.rs:1775`, `project_drawings.rs:720,735` | **Drobnosti v exportoch.**<br>• Komentár výbehu („6,3 až 4 P“) nesedí s tabuľkou (5,6 až 4,37 P), hodnoty sú správne.<br>• `HoleThread::label` formátuje f64 surovo, treba `format_number`.<br>• `{:.0}` na projektových rozmeroch zaokrúhľuje k párnemu číslu. |
| SEC-6 | S | `plugin_manifest.rs:138,179,286`, `scheduler/src/plugin.rs:43`, `ketchup-mcp/src/discovery.rs:127-171`, `transport.rs:11` | **Menšie bezpečnostné diery.**<br>• Neexistujúci súbor mimo priečinka sa nehashuje, môže vzniknúť neskôr.<br>• `fs::copy` nemá strop.<br>• Adresáre stage nie sú zamknuté.<br>• Na Unixe chýba skupina procesov pluginu.<br>• Okno sa pri pripojení neautentifikuje: cudzí proces môže obsadiť port po páde.<br>• 4 bridge sloty sa dajú obsadiť. |
| SDK-1 | S | `sdk/python/ketchup_sdk/__init__.py:121-123` | **Python SDK a kódovanie a limity.**<br>• `_send` píše cez `sys.stdout` v kódovej stránke lokálu (bez `PYTHONUTF8`). Treba `sys.stdout.buffer`.<br>• Limit riadku SDK (128 KiB) je nižší ako limit hostiteľa. |
| VW-11 | S | `viewer/web/src/lib.rs:257`, `viewer/android/src/files.rs:288` | **Viewer bez stropu a neatomický zápis.**<br>• Web: sťahuje model bez stropu veľkosti.<br>• Android: zápis s `"wt"` nie je atomický, chyba v polovici zničí pôvodný súbor. |
| CI-4 | S | `ci.yml:25,48`, `deny.toml` | **Nepripnuté závislosti CI a obmedzený cargo-deny.**<br>• `setup-python@v5` nie je pripnuté SHA.<br>• `pip install` je bez verzií a hashov.<br>• `cargo deny` nebeží v CI a `deny.toml` má len Windows target. |
| REPO-1 | K | koreň repozitára | **Veľké a generované súbory v Gite.**<br>• ~90 MB sledovaných binárnych súborov bez LFS: `examples/tiny-house.ketchup` 24 MB, `house-project.ketchup` 9 MB, generované `*.report.json` ~7 MB, `pic/`, `newdesign/`, `artifacts/*.blend`.<br>• Súbor s medzerami v názve `docs/design/Ketchup Modeler (standalone, open this).html` rozbije `xargs`/`wc` v skriptoch. |
| REPO-2 | K | `docs/` | **Neporiadok v `docs/` a jazyk dokumentov.**<br>• 12 datovaných review a status súborov leží vedľa normatívnych dokumentov.<br>• Status súbor mení skoro každý commit, takže je to horúce miesto konfliktov.<br>• Slovenské dokumenty, na ktoré sa odvoláva AGENTS.md, sú v rozpore s ADR 0001.<br>• Návrh:<br>• Archív `docs/archive/reviews/` alebo issues.<br>• Pravidlá preniesť do anglického ADR. |
| TST-1 | K | `ketchup-app/tests/headless_shell.rs` (14 879 r.), `file_workflow.rs` (5 382 r.) | **Testy bez stropu veľkosti.**<br>• Nález: strop modulov sa vzťahuje len na `src`. 22 testovacích funkcií má viac ako 300 riadkov.<br>• Návrh: rozdeliť po nástrojoch (ako `scale_grips_ui.rs`) a zaviesť strop aj na testy. |

## 4. Metriky

| | dnes |
|---|---:|
| Rust riadkov spolu (bez `third_party`) | ~530 000 |
| `ketchup-app` / `ketchup-model` / `ketchup-application` | 161,6k / 136,9k / 57,4k |
| najväčší zdrojový modul | `fabrication.rs` 4 979 (limit 5 000) |
| moduly nad 4 000 riadkov | 9 |
| produkčné funkcie nad 150 riadkov (`ketchup-app`) | 39 |
| najdlhšie | `show_assembly_editor_content` 756, `show_feature_history_content` 672, `execute_authorized` 394, `dispatch_command` 357 |
| najdlhšia funkcia v `ketchup-model` | `apply_batch_with_origin_and_validation` ~2 070 |
| ratchety (`check_*.py`) | všetky zelené, 3 s rezervou na `--update` |

## 5. Návrhy štrukturálnych vylepšení

1. **Dokument nezávislý od histórie.**
   - Všetko, čo potrebuje aktuálna revízia (snapshot, program, bloby), je v povinných
     záznamoch.
   - História sa dá vždy zahodiť bez straty dát, a to len s viditeľným varovaním (PS-1, PS-9).
   - Každé preskočenie alebo náhrada pri načítaní (história, recovery, záloha) je typovaný
     záznam v `LoadAudit` a app aj session ho ukážu (PS-2, PS-3).
2. **Jedna save pipeline.**
   - `prepare` → `publish` pre GUI, headless session aj work-recovery.
   - Jedno kódovanie a kontrola len checksumu.
   - Rovnaký fallback na `TooLarge` všade (PS-5, PS-6, PS-7).
3. **Celé vyhodnotenie programu vo workeri.**
   - Report, validácia aj relácie bežia pod rovnakým deadlinom a pamäťovým limitom ako
     vyhodnotenie.
   - Explicitné stropy na páry, issues a uzly boolean stromu.
   - Na Unixe `setrlimit` a skupina procesov, alebo pomenované odmietnutie (PRG-1, PRG-3,
     PRG-6).
4. **Jedna geometrická pravda pre box úvahy.**
   - Jedna funkcia „poloha na hotovom diele“ pre všetkých spotrebiteľov.
   - OBB a polytopové predikáty namiesto svetového AABB.
   - Spoločný predikát kontakt/prienik pre box aj exact cestu, s prahom z `ketchup-tolerance`
     (PRG-2, PRG-4, PRG-5, PRG-8).
5. **Jeden model relácie nástroja v GUI.**
   - Dnes má Move, Rotate a Scale `ToolSession`, Sketch `SketchGesture` a Push/Pull vlastný
     drag, takže „zrušiť“, „ponechať odmietnutú hodnotu“ a „Ctrl+Z“ sú postavené štyrikrát.
   - Návrh: spoločný trait (`is_current`, `cancel`, `apply_typed`) s jedným registrom a jeden
     parser hodnôt.
   - `apply_value_input` (293 r.) rozdeliť na `apply_typed_<nástroj>`.
   - Testy s nenulovým smerom vo všetkých osiach (UI-1 až UI-6).
6. **Pravidlá výroby na jednom mieste.**
   - Jedno pravidlo závitovej diery, volané pri tvorbe, exporte aj validácii.
   - Tabuľka schopností exportéra (čo BTLx, woodWOP a výrobný job vedia niesť) s jednou
     bránou, ktorá pomenuje diel a stratený prvok.
   - Krížové testy: SVG, BTLx, woodWOP a job dávajú rovnaké lokálne body pre všetkých 6
     permutácií osí a zrkadlenú kópiu.
   - Golden súbory validovať proti XSD (MF-1 až MF-3).
7. **Plugin: jedna vyriešená hodnota spustenia.**
   - Každé slovo príkazu sa raz vyrieši na kanonickú cestu bez `..`, ktorá rešpektuje
     Windows `.exe`.
   - Súhlas, hash aj `PluginCommand` sa berú z tejto hodnoty.
   - Príprava mimo UI vlákna.
   - Escapovanie v dialógovej vrstve pre všetky prompty (SEC-1 až SEC-4).
8. **Prenosné CI.**
   - GitHub-hosted Linux a macOS: fmt, clippy pre všetky targety (vrátane android a wasm32),
     cargo-deny a testy crate bez OCCT.
   - Self-hosted Windows ostane pre OCCT a plnú sadu.
   - `cargo-fuzz` ciele pre `load()` a `Package::read_with_comments` s limitom času a pamäte
     (CI-1 až CI-4, PS-4).
9. **Hygiena repozitára.**
   - Binárky do LFS alebo release assets, generované reporty počítať v testoch.
   - `docs/` len normatívne anglické dokumenty a ADR.
   - Strop veľkosti aj pre testovacie súbory (REPO-1, REPO-2, TST-1).

## 6. Odporúčané poradie

- **Hneď:**
  - PS-1 (strata programu);
  - UI-1 (Move po X);
  - PRG-2 (mirror a kolízie);
  - MF-2 (SVG oblúky na výrobnom výkrese);
  - SEC-1 (`..` v argumente pluginu).
- **Tento týždeň:**
  - PRG-1 (report vo workeri);
  - PS-2 a PS-3 (recovery a novší formát);
  - MF-1 (závit voči diere);
  - SEC-2 až SEC-5;
  - CI-1 (Linux job, aspoň fmt a clippy crate bez OCCT).
- **Potom:**
  - zvyšok P2 a P3;
  - štrukturálne body 1, 3 a 5 ako samostatné bloky podľa AGENTS §6.
