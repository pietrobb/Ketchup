# Podrobný code review Kečupu: celý strom na `9163252`

Stav `main` 2026-10-17. Nadväzuje na `code-review-2026-10-17.md`. Od neho pribudlo 19
commitov. Tie opravili F-1 (stĺp na skutočnom priereze), D-1 až D-4, D-6, D-7, F-2, F-3 a
zaviedli jedno pravidlo pre desatinný oddeľovač vo všetkých poliach.

## 1. Metóda

- Sedem nezávislých čitateľov nad celým stromom, nielen nad posledným diffom:
  1. model a perzistencia;
  2. program, geometria a EC5;
  3. exact a workery;
  4. GUI;
  5. MCP, pluginy a headless;
  6. výroba, výkresy a importy;
  7. viewer, CI, testy a dokumentácia.
- Nálezy už opravené alebo vedené v predošlých review sa neopakujú. SEC-5 je rozhodnutie
  vlastníka a nehlási sa.
- **Overenie** v registri:
  - **K**: mechanizmus som sám overil čítaním kódu;
  - **P**: čitateľ ho potvrdil sondou proti skutočnému crate, mimo repozitára;
  - **S**: statický nález čitateľa so stopou od volajúceho.
- Spustené na `9163252`:
  - všetky `scripts/check_*.py`: prešli;
  - lokálne Linux job z CI (`fmt`, `clippy` a testy 15 crate bez OCCT): prešiel, 1 342 testov,
    0 zlyhaní.
- CI na GitHube je na `9163252` zelené: push 516 aj ručný beh 515 vrátane nočných testov.
  Nočný beh 511 a ručný 514 boli červené pre stopky v testoch; tie opravili 4778ea4
  a 86bfd71.
- Priorita:
  - **P1**: zlý výrobný alebo statický výsledok, strata dát, zamrznutie alebo pád pri
    bežnej práci;
  - **P2**: konkrétna chyba s menším dopadom;
  - **P3**: dlh alebo hygiena.

## 2. Záver v skratke

**Čo je dobré:**

- Hranica C++/Rust je robustná: každé `*_native` volanie je `noexcept` v `guarded()` a
  výsledky OCCT prechádzajú `BRepCheck`.
- Zastarané výsledky exact sa zahodia podľa `ExactSource`.
- Na bridge sa token kontroluje pri každom rámci a HMAC pri pripojení je viazaný na okno a
  nonce.
- Ostatné časti, ktoré čitatelia prešli a sú v poriadku:
  - tabuľky EC5 a integrály prierezu;
  - parita zrkadlenia v BTLx a woodWOP;
  - escapovanie XML;
  - mapovanie `$INSUNITS`;
  - jednotky STEP a IGES;
  - lokalizácia: žiadna cesta mimo UI nezávisí od lokálneho nastavenia systému.
- Z 40 vzoriek testov 34 overuje skutočné výsledky (objemy, rozmery, BTLx, 3MF, obnovené
  bajty), ako to chce AGENTS §5.

**Čo je zlé, päť P1 a jeden blízky P2:**

1. **Statika:**
   - nosník so zárezom, kapsou alebo dierou sa posudzuje na plnom priereze a dostane „pass“;
     oprava F-1 pokryla len stĺpy (PG-1);
   - gravitácia berie za podporu aj bočný dotyk (PG-2).
2. **Model:** `SetFeatureDimension` potichu zmení symetrický alebo „cez všetko“ extent na
   jednostranný (M-1).
3. **Výroba:** kolíkové diery sa vo woodWOP a výrobnom jobe vŕtajú dvakrát, alebo v BTLx a
   na výkrese chýbajú (MF-5).
4. **Exact:** import a export STEP majú timeout 5 s, hoci súbor sa parsuje N+2-krát (EX-2).
5. **Model:** dôkazy plôch pre UpToFace sa zahodia pri každej ďalšej úprave, takže rez
   prestane fungovať (M-2; vediem ho ako P2, lebo chyba je hlásená).

## 3. Register nálezov

### P1

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| PG-1 | K | `ketchup-program/src/member_check.rs:1257-1271`, `:955-1003` | **Nosník so zárezom alebo dierou sa posudzuje na plnom priereze.**<br>• `shape.unmodelled` sa číta len vo vetve `role == "column"`. `beam_checks` počíta ohyb, priehyb a šmyk z `shape.modulus`, `shape.area` a plného `I`.<br>• Príklad: stropnica `member()` so `subtract()` zárezom do polovice výšky v strede rozpätia. W klesne 4× a I 8×, napriek tomu výsledok `status: "pass"`.<br>• To isté pre `pocket()` a dieru Ø60 cez výšku 200.<br>• Zárez nad uložením urobený booleanom alebo kapsou nedostane kontrolu `shear_notch` vôbec. `depth_at` existuje len pre profily vytiahnuté z bočného pohľadu. | • Rovnaká brána ako pri stĺpoch: `unmodelled` dať do `missing`. Výnimka 5 % pre malé diery môže ostať.<br>• Testy: nosník `member()` so `subtract` a s `pocket` musia byť `not_verified`. |
| PG-2 | K | `ketchup-model/src/exact_validation.rs:1317-1331`, `ketchup-application/src/collision.rs:837-855` | **Gravitácia berie bočný dotyk ako podporu (regresia z bed3811).**<br>• Pri `common_contact_area_mm2 > tol²` vráti `ProvenExact` ešte pred kontrolou smeru.<br>• Plocha sa sčíta zo všetkých orientácií kontaktu.<br>• Voľná polica bez spoja, ktorá sa čelom dotýka boku skrine, je „podopretá“. Rovnako doska pritlačená zospodu k doske stola.<br>• Pred bed3811 boli oba prípady `gravity.unsupported`. | • Worker a hull nech vracajú plochu len pre dvojice plôch, ktorých normála je blízko smeru gravitácie (ako `load_path::RESTS_ON = -0.5`).<br>• Aspoň: najnižší bod dielu na alebo nad najnižším bodom podpery a ťažisko nad kontaktom.<br>• Test: blok dotýkajúci sa zboku ostane nepodopretý. |
| M-1 | K, P | `ketchup-model/src/document/store/commands/features.rs:97-101` | **`SetFeatureDimension` na Pad potichu zmení extent na jednostranný.**<br>• `updated.extent = FeatureExtent::Blind(dimension)` sa vykoná bez ohľadu na predošlý extent.<br>• Volajú to planner asistenta (`planner.rs:2479`), `UiAdapter` (`adapters.rs:22`) aj `feature_history.rs:276`. Len vlastný picker v GUI sa obmedzí na Blind.<br>• Príklad: Pad `Symmetric(18)` (z −9…9) po zmene na 20 je `Blind(20)` (z 0…20). ThroughAll, UpToFace aj Bidirectional sa zmenia na pevnú hĺbku. | • Variant extentu zachovať; meniť len vzdialenosť pri Blind a Symmetric.<br>• Ostatné odmietnuť typovanou chybou s odkazom na `SetFeatureParameter`.<br>• Podľa §6 starý príkaz nahradiť `SetFeatureParameter`. |
| MF-5 | K (čiastočne) | `ketchup-manufacturing/src/fabrication.rs:1099-1107`, `:1183-1190`, `fabrication/production.rs:81-95`, `ketchup-application/src/planner.rs:2859-2893` | **Kolíkové diery sa vŕtajú dvakrát, alebo úplne chýbajú.**<br>• woodWOP aj výrobný job pridajú diery každého `pin_joint` a nekontrolujú `physical_hole_pairs`.<br>• V režime `Drill` sa tie isté diery vyrežú ako kapsy a z nich vznikne aj `CircularDrill` operácia. Stroj tak dostane dva `BohrVert` na tom istom mieste.<br>• V režime `Logical` (bez kapsy) ich vyvŕta woodWOP a job, ale BTLx a dielenský výkres ich nemajú, lebo čítajú len `operations`.<br>• Brána schopností kolíkové diery nevidí. | • Dvojice spojov s `physical_hole_pairs.is_some()` preskočiť.<br>• Logické diery urobiť operáciami (alebo `ShopElement::PinHole`), ktoré BTLx a výkres buď nesú, alebo pomenovane odmietnu.<br>• Krížový test: rovnaký počet dier vo všetkých formátoch. |
| EX-2 | K | `ketchup-scheduler/src/lib.rs:818`, `worker_client.rs:366,431,494,519,576,605,644`, `exact_worker.rs:3076,3366,3473-3606`, `native_io.cc:588-592,752-760` | **Import a export STEP a IGES majú len 5 s.**<br>• Inšpekcia, tessellácia, konverzia aj export assembly bežia pod `DEFAULT_WORKER_REQUEST_TIMEOUT` (5 s).<br>• Inšpekcia parsuje súbor N+2-krát: `import_step_xde_part` pre každý diel urobí celý `ReadFile` a `Transfer`.<br>• Export assembly každý diel znova importuje a výstup dvakrát parsuje s `BRepCheck`.<br>• Súbor s desiatkami dielov alebo export 750-dielneho domu skončí na `RequestTimedOut(5000 ms)`. `permits_restart` timeout zopakuje.<br>• Export beží synchrónne s `NEVER_CANCELLED`, okno zamrzne. | • Rozpočet podľa bajtov a počtu dielov.<br>• Dokument XDE načítať raz na požiadavku a vrátiť z neho všetky diely.<br>• Deterministický timeout neopakovať (EX-5).<br>• Export na pozadí s možnosťou zrušiť (EX-6). |

### P2

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| M-2 | P | `ketchup-model/src/document/store.rs:1197-1208`, `:48-92`, `:606-622`, `snapshot.rs:275`, `compiler.rs:969` | **Dôkazy plôch pre UpToFace sa zahodia pri ďalšej úprave.**<br>• Pred príkazmi sa zahodia všetky dôkazy okrem tých pre Workplane, `retain_anchored_evidence`. Odkazy Pad sa počítajú za ukotvené až potom.<br>• Rez `UpToFace(top)` stratí dôkaz už v dávke, v ktorej vznikol. Po novej registrácii ho stratí pri najbližšom `RenameDefinition`.<br>• Graf potom hlási „exact feature extent target is missing“. Súbor sa otvorí, ale rez sa nikdy nezostaví. | Jedna funkcia `anchored_lineages(product)` vrátane `PadSpec::references()`, použitá na všetkých troch miestach. |
| M-3 | K | `ketchup-application/src/session.rs:484-495` | **Nahradenie dokumentu programom zahodí väzbu session na súbor.**<br>• `replace_with_rule_model` urobí `*self = Self::new(...)`, čím stratí `path`, `file_identity`, `container_data` (bloby, `extensions/*`) aj zámok work-recovery.<br>• Headless: `open`, `apply`, `program_apply` s `discard_unsaved`, `save` s `overwrite`. Výsledok je `ExternalConflict` pri každom uložení. Pri ďalšom otvorení sa obnoví starý checkpoint.<br>• Bez `apply` sa potichu stratia `extensions/*` (nastavenia viewera, projektové výkresy). | Nahradiť dokument vnútri existujúcej session ako jednu dávku s Undo, alebo preniesť cestu, identitu, zámok a `container_data`. |
| MF-6 | S | `ketchup-model/src/drawing.rs:4080-4110` | **Kusovník výkresu zostavy zlúči zrkadlené kópie.**<br>Pozície sú zoskupené len podľa `definition_id`. Ľavý bok a jeho zrkadlený pravý dostanú jedno číslo a „QTY 2“, kým výrobný BOM (od 3850d5c) ich rozlišuje. Dielňa podľa výkresu vyrobí dva kusy rovnakej ruky. | Kľúč `(definícia, determinant < 0)` ako v `project_general_fabrication`. |
| MF-7 | S | `ketchup-model/src/drawing.rs:3634-3636`, `:453` | **Kóty na izometrii ukazujú skrátené dĺžky.**<br>`value_mm` je 2D dĺžka premietnutej čiary. Hrana 1 000 mm na izometrii má kótu 816,497 mm a pravý uhol 60° alebo 120°. To isté platí pre šikmý `Auxiliary` pohľad. | Odmietnuť kótu na hrane, ktorá nie je rovnobežná s rovinou pohľadu, alebo merať 3D hranu. Aspoň na izometrii odmietnuť. |
| EX-3 | K | `ketchup-scheduler/src/plugin.rs:186-187`, `child_process.rs:4-14`, `third_party/process-wrap/src/std/process_group.rs:164-205` | **Na Unixe každý úspešný beh pluginu skončí chybou.**<br>• `try_wait` zoberie vodcu skupiny cez `waitpid(-pgid)` a `terminate` potom volá `killpg` na neexistujúcu skupinu.<br>• `ESRCH` a `?` urobia z úspechu `PluginHostError::Transport`.<br>• Testy pluginov bežia len na Windows. | `ESRCH` v `terminate` brať ako úspech, alebo nezabíjať, ak je exit status známy. Pustiť testy pluginov aj na Linuxe. |
| EX-4 | S | `ketchup-exact` (`run_stdio`, `native_*.cc`) | **OCCT môže písať do binárneho kanála workera.**<br>• Rámce idú na stdout. Predvolený `Message_PrinterOStream` OCCT píše varovania na `std::cout`.<br>• Varovanie čítača STEP sa tak dostane do rámca. Klient ho prečíta ako dĺžku, hlási `TooLarge` a worker reštartuje.<br>• Používateľ vidí „exceeded the 16777216-byte limit“ namiesto diagnostiky čítača. Nereprodukované. | Pri štarte `Message::DefaultMessenger()->RemovePrinters(...)`, alebo protokol presunúť na vlastný deskriptor (fd 1 → stderr). |
| EX-5 | S | `ketchup-scheduler/src/lib.rs:796`, `:2008-2027`, `:2096-2114` | **Deterministické chyby sa opakujú naplno.**<br>• `permits_restart` zahŕňa timeout, `ResponseTooLarge` aj `Protocol`, vrátane vlastného `resource_limit` workera.<br>• Graf alebo tessellácia tak môže trvať 2 × 60 s, kým príde chyba.<br>• Veľké importované teleso s dôkazmi nad 16 MiB sa nikdy nezobrazí a hláška nepovie prečo. | Reštart len pri `WorkerExited`, `Transport` a `MalformedTransport`. `resource_limit` hlásiť ako odmietnutie s počtom plôch. |
| EX-6 | S | `ketchup-app/src/feature_history_ui.rs:1326-1350`, `app/export.rs:676-690`, `:763-800` | **Volania exact workera blokujú UI vlákno bez zrušenia.**<br>História prvkov vyhodnocuje telesá synchrónne (až 60 s na teleso plus opakovanie). Export STEP a IGES beží s `NEVER_CANCELLED`. Pomalý fillet tak zamrazí okno až na 2 minúty. | `BackgroundTask` so zrušením, ako `mesh_conversion`. |
| EX-7 | S | `ketchup-scheduler/src/lib.rs:1055-1062`, `plugin.rs:47` | **Na Unixe worker a plugin prežijú pád editora.**<br>Chýba `PR_SET_PDEATHSIG` a worker si všíma EOF na stdin len medzi požiadavkami. Test je `#[cfg(windows)]`. | `PR_SET_PDEATHSIG` v `pre_exec` na Linuxe. Inde strážne vlákno vo workeri. |
| PG-3 | S | `member_check.rs:1063` | **EC5 6.5.2: `x` pri zárezе nad uložením je polovica dĺžky uloženia.**<br>• Podľa obr. 6.11 má byť `x` vzdialenosť reakcie od rohu zárezu.<br>• Vlastný test (`member_design.rs:250-282`): zárez 0–150 mm, stred stĺpika na 50 mm, teda skutočné x = 100 mm. Kód ráta s x = 50.<br>• kv vyjde 0,636 namiesto 0,497, takže `shear_notch` je o ~28 % nižší a test potvrdzuje nesprávnu hodnotu. | `x` merať od stredu reakcie po roh zárezu (z `depth_at`). Opraviť test. |
| PG-4 | S | `ketchup-program/src/validate.rs:527`, `model.rs:1244-1269`, `prelude.star:972-977` | **Prevŕtanie slepej diery sa na neprizmatickom diele neodhalí.**<br>• Hrúbka sa počíta z `Part::reach`, teda z konvexného obalu profilu pred rezmi a kapsami.<br>• 12 mm kolík v 18 mm doske na línii drážky hĺbky 10 mm na zadnú stenu prerazí do drážky bez varovania.<br>• `_drill_room` v knižnici má ten istý problém: v L profile s ramenom 18 mm navrhne hĺbku 19 mm, teda cez rameno. | Všeobecný dotaz „materiál pozdĺž lúča od vstupného bodu“ (profil mínus rezy, kapsy, shell, alebo exact). Použiť vo validátore aj v `_drill_room`. |
| PG-5 | S | `member_check.rs:548-563`, `:582-587` | **Stĺp z bočného pohľadu ignoruje zárezy pri koncoch.**<br>• `net_area` vynechá stanice do 2× najväčšej šírky od konca.<br>• `area` a `radius` berú najhrubší prierez.<br>• Preplátovanie v hornej 100 mm časti stĺpika 60×140 sa nekontroluje, tlak je podhodnotený 2×. | Pre `column` zahrnúť koncové stanice. Na vzper brať najtenší prierez, alebo `not_verified` pri premenlivom priereze. |
| PG-6 | S | `member_check.rs:1003`, `:350` | **Profilovaný nosník používa vzorce pre obdĺžnik.**<br>• Šmyk `τ = 1,5·V/A`: dutý 200×200 s otvorom 160×160 je podhodnotený o 49 %.<br>• Pri L a nesymetrickom priereze je `I` k geometrickej osi, nie hlavnej, takže ohyb aj priehyb sú podhodnotené. | `τ = V·Q/(I·b)` z tetív profilu. Hlavné osi, alebo `not_verified`. |
| PG-7 | S | `third_party/starlark/src/values/types/dict/value.rs:93`, `heap_type.rs:680` | **Slovníky a množiny v Starlarku obchádzajú limit 1 GiB.**<br>`SmallMap` je mimo arény. `[dict(zip(L, L)) for _ in range(20)]` nad 10 M prvkov postaví >10 GB za ~22 počítaných krokov. Na Unixe a v CLI `ketchup-program check` to nič nezastaví. | Rátať kapacitu `SmallMap` cez `note_external_bytes`, alebo počítajúci alokátor na vlákno. |
| UI-8 | K | `ketchup-app/src/length_input.rs:193-216`, `:362-374` | **Na systéme s desatinnou čiarkou sa bodka berie ako desatinná, aj keď je oddeľovač tisícov.**<br>• Nemecký používateľ napíše `1.200` pre Push/Pull a dostane 1,2 mm. Text je „čisté číslo“, takže sa nezobrazí ani náhľad.<br>• `1.200;600` dá obdĺžnik 1,2 × 600. Starý zápis `80,60` v Move dá 80,6 mm bez náhľadu. | Na čiarkovom systéme bodka a presne tri číslice: odmietnuť s nápovedou, alebo vždy ukázať náhľad. Lacnejšie: náhľad vždy, keď napísaný oddeľovač nie je systémový. |
| UI-9 | S | `ketchup-app/src/app/takeoff.rs:186-189`, `live_bridge/program_variant_export.rs:200-203` | **CSV berie desatinný oddeľovač z jazyka UI, polia hodnôt zo systému.**<br>Anglické UI na slovenskom Windows dá CSV s `.` a `,`, takže Excel tam všetko dá do stĺpca A. MCP export variantov tak závisí od jazyka okna. | V GUI použiť `value_box.decimal`. MCP export s explicitným argumentom `decimal`. |
| SEC-7 | P | `live_bridge/plugin_manifest.rs:295-333`, `:378-397`, `plugin_consent.rs:178` | **Biela listina argumentov pluginu prepustí kód ako obyčajné slovo.**<br>• Slovo bez `=` a bez úvodného `-` alebo `@` prejde ako text: `-c "exec(open('C:/Users/u/x.py').read())"`, `-e require(...)`, `-m evil`, `-` (stdin).<br>• Uložený súhlas (`covers`) potom pokryje aj neskoršiu zmenu `x.py`, ktorú nikto nehashoval.<br>• Rovnaká trieda: `python -m mod` zo zapisovateľného site-packages a `require` v Node hľadaný od stage v `%TEMP%` alebo `/tmp`. | • Slovo, ktoré nie je súbor, musí prejsť `plain_value` (bez medzier, úvodzoviek, zátvoriek, `;`).<br>• Príkazový riadok musí menovať aspoň jeden hashovaný skript.<br>• Najlepšie: MCP formu príkazového riadku zrušiť, `docs/plugins.md` aj tak popisuje len priečinky.<br>• Stage v súkromnom priečinku namiesto `%TEMP%`. |
| MCP-1 | K | `live_bridge/interchange.rs:200-208`, `app/export.rs:704,800` | **Export cez MCP do `.stp` a `.igs` hlási „bez strát“.**<br>• Odpoveď hľadá `<cesta>.{prípona}.loss.txt`, ale exportér píše `step.loss.txt` a `iges.loss.txt`.<br>• Pri `x.stp` tak MCP vráti `loss_report: null` a klient usúdi, že sa nič nestratilo.<br>• Sprievodné súbory (`x.drawings.svg`, `x.bends.csv` …) sa nehlásia nikdy. | Exportér vráti cesty, ktoré zapísal, a MCP ich odovzdá namiesto hádania mien. |
| MCP-2 | S | `live_bridge/drawings.rs:262`, `:279` | **`export_drawings` zahodí obsah chyby (AGENTS §2).**<br>`ProjectDrawingsError::Drawing(_)` a `SettingsUnreadable(_)` sa zmenia na holý kód. Ratchet vzor `Variant(_) => "kód"` nechytá. | `failed_because(...)` v oboch vetvách a pravidlo pre `(_) => "` v `check_error_hygiene.py`. |
| VW-12 | K (čiastočne) | `ketchup-viewer/src/app/session.rs:160-178`, `:46-70`, `view-format/src/comments.rs:260` | **Oprava D-7 nepomôže, keď proces naozaj skončí.**<br>• Komentáre sa zapíšu hneď, ale menujú balík B, ktorý sa ešte zapisuje do `.part`. Po ukončení procesu sa B nikdy nepremenuje.<br>• Pri štarte sa otvorí A a komentáre k B neprejdú kontrolou digestu.<br>• Pri ďalšom otvorení B sa komentáre zmažú (`:168`). `session.unsaved` sa navyše použije na A.<br>• Test nechá vlákno zapisovača dobehnúť, takže skutočné ukončenie procesu neskúša. | Nemazať komentáre, ktorých `package_sha256` sedí s otváraným balíkom. UNSAVED použiť len vtedy, keď sa komentáre otvorili. Test so skutočne zabitým zapisovačom. |
| VW-13 | S | `viewer/android/src/replace.rs:338-357`, `:403-411`, `files.rs:358-367` | **Po vzdaní obnovy na Androide je dobrá kópia nedostupná.**<br>• Po 3 štartoch sa staré bajty presunú do `kept-*.bak` v súkromnom úložisku, kam sa používateľ nedostane.<br>• Počítajú sa aj dočasné chyby (cloud offline, odpojená SD karta). | Počítať len `InvalidData`. Pri vzdaní ponúknuť „Uložiť skorší obsah ako…“ alebo zdieľanie. |
| TST-2 | S | `tests/test_headless_acceptance.py` a ďalších 8 súborov, `scripts/run_production_tests.py` | **~34 natívnych Python testov nebeží v žiadnom CI.**<br>Bez `KETCHUP_HEADLESS` sa preskočia a job „Python tests“ je zelený. | V jobe `rust-with-occt` zostaviť `ketchup-headless` a pustiť všetky natívne súbory. |
| TST-3 | S | `ketchup-app/tests/performance_ai_house.rs:56-74`, `~290`, `ketchup-scheduler/tests/validator_sandbox.rs:431`, `background_task.rs:126`, `program_worker.rs:113,243-276` a ďalšie | **V testoch pre každý PR ostali stopky.**<br>Napríklad „20 snímok < 1 s“ v debugu na WARP. `program_worker.rs:243` kalibruje deadline na 1,6× jedného merania. Je to rovnaká trieda ako 86bfd71, 4778ea4 a bcd4a63. | Overovať výsledok (zrušené, nedokončené). Časové rozpočty len v `perf_*` testoch nočne v release. Ratchet na `elapsed() <`. |
| TST-4 | K | `ketchup-application/tests/house_collision.rs:11-53` | **Celý dom sa kontroluje pri každom behu testov.**<br>Exact kolízia 750-dielneho domu beží pri každom behu (debug rozpočet 600 s), nie je `large_house_*` ani `#[ignore]`, proti AGENTS §6. `assert!(elapsed < BUDGET)` nič nepridáva. | Premenovať na `large_house_…` a pridať `#[ignore]`. |
| DOC-1 | S | `README.md` (Build and run) | **README dáva príkazy v rozpore s AGENTS §6.**<br>`cargo clippy --workspace --all-targets --all-features` bez `--locked` a `--test-threads=1` sú v rozpore s §6 a §8. | Kanonické príkazy, `check_*.py` a hook. |

### P3

- **Model a perzistencia:**
  - **M-4:** definícia komponentu sa nedá zmazať. `DeleteDefinition` nemaže
    `local_groups` ani `local_occurrences` a vráti `InvalidLocalGraph` bez cieľa. (P)
  - **M-5:** otvorenie histórie kopíruje všetky bloby raz na revíziu
    (`persistence.rs:2350`). Pri 30 MB STEP a 400 revíziách je to ~12 GB kopírovania.
  - **M-6:** každá dávka, aj prepnutie viditeľnosti, klonuje produkt pre každý výkresový
    list (`assembly_validation.rs:371`). Znova kontroluje všetky spoje a priestory a po
    každom príkaze prepočíta výrobné kódy. To je proti §3 („lokálne“).
  - **M-7:** session otvorená z checkpointu work-recovery prestane písať nové checkpointy
    (`session.rs:165`).
  - **M-8:** chyby ako text: `io::Error::new(.., "…")` (`work_recovery.rs:39`,
    `persistence.rs:1364`), `format!` (`persistence.rs:1118`),
    `MigrationNotConfirmable(&'static str)`.
  - **M-9:** `program.json` nemá pri načítaní limit `MAX_RULE_PROGRAM_BYTES`, pri zápise
    áno. Upravený súbor sa otvorí a potom zlyhá každý zápis checkpointu.
  - **PS-11:** stále otvorené (klon celého produktu na dávku).
- **Statika a program:**
  - `settle` zaokrúhľuje na 3 desatinné miesta pred porovnaním, takže 1,0004 prejde.
  - Dve malé diery v jednom priereze sa výnimkou 5 % posudzujú každá zvlášť.
  - Šikmý stĺp berie zvislé zaťaženie ako osovú silu, takže pri sklone 26° je osová sila
    až o 11 % nižšia.
- **Exact:**
  - Zber výstupu je kvadratický (`output.rs:110-118,166-171`).
  - Každé požičanie a vrátenie workera hashuje spustiteľný súbor (až 128 MiB) a hash
    nezahŕňa knižnice OCCT.
  - Pád úlohy na pozadí sa hlási ako `Cancelled` (`background.rs:139-146`).
  - `QUERY INPUTS` pluginu sa nepočíta do `max_requests`.
  - Session párov drží až 20 000 kópií B-Rep mimo rozpočtu cache.
  - `EvaluationScheduler` a `general.rs` sa používajú len v testoch, teda paralelná cesta
    proti §6.
- **GUI:**
  - Väčšina dialógových polí odmietne čiarku bez dôvodu. Assembly a FEA dôvod uvedú, ale v
    angličtine v slovenskom UI.
  - Revolve číta uhol ako dĺžku (`9 cm` je 90°, `90°` sa odmietne).
  - FEA polia neberú `2.1e5` (regresia z 57f7ce7).
  - `×` násobí, `x` oddeľuje: `80×60` v Move je 4 800 mm.
  - Počet strán polygónu, počty vzorov a celé čísla v pluginoch ďalej používajú
    `str::parse`.
  - `hole_spec` delí podľa medzier, takže `M8 1 cm` sa odmietne.
  - Na macOS sa systém lokálne zistí z jazyka.
  - Pri pomalej snímke môže písmeno z `1m` prepnúť nástroj.
  - Register `GESTURES` nepokrýva Revolve, Sweep, Loft, Helix, offset ani finish.
- **MCP:**
  - Endpoint pripojenia sa dá zaplniť 8 nečinnými spojeniami a `list` bez dôkazu prezradí
    názov dokumentu.
  - Export s dialógom vydania pokračuje aj po odpojení klienta.
  - Veľkosť vstupov pluginu sa nekontroluje pred súhlasom.
  - `docs/plugins.md` a schéma sa nezhodujú s kódom (príkazový riadok, DWG).
  - Chýbajúci obrázok sa vráti ako úspech s `data: null`.
- **Výroba a výkresy:**
  - Kóta na čiastočne skrytej hrane meria len viditeľný kus.
  - Priechodná diera blokuje BTLx a woodWOP všeobecnou chybou, hoci tabuľka schopností
    tvrdí opak.
  - Názvy vrstiev v profilovom DXF porušujú R12 (31 znakov, `[A-Z0-9$_-]`, bez ohľadu na
    veľkosť písmen).
  - Text DXF môže po escapovaní presiahnuť 255 znakov a emoji sa zapíšu ako surogáty.
  - Projektové výkresy kreslia náhodné čiary po valcoch: prah 20° je rovný kroku siete 20°.
  - Projektový výkres môže diel vynechať bez slova (`unwrap_or_default`,
    `app/project_drawings.rs:131-136`).
  - Staré slovenské DXF v `ANSI_1250` sa odmietnu celé.
  - Kopírovaný text z PDF môže byť nesprávny (ToUnicode pre `?`).
- **Viewer a CI:**
  - `check_ci_crates.py` uzná aj komentár v YAML.
  - `check_test_sleeps.py` neskenuje `viewer/*`.
  - Fuzzing má len 2 ciele, bez uloženého korpusu a bez `--locked`.
  - Štart na Androide načíta a overí až ~264 MB na UI vlákne.
  - Štyri `#[ignore]` testy nebežia nikde.
- **Dokumentácia:**
  - README tvrdí, že Undo je len v session, a nespomína DWG, IGES, 3MF, BTLx ani viewer.
  - `program-language.md` nepopisuje `helix`, `shell`, `union`, `intersect`, `nearest`
    ani funkcie EC5.
  - `plugins.md` nepopisuje bielu listinu argumentov.
  - ADR 0002 a 0004 odkazujú na neexistujúce súbory.
  - `viewer/MAC_TASK.md` a `MAC_RESULT.md` sú slovenské pracovné poznámky mimo archívu.

## 4. Metriky

| | dnes |
|---|---:|
| Rust riadkov v `crates/` (777 súborov) | ~520 000 |
| `ketchup-app` / `ketchup-model` / `ketchup-application` | 165,8k / 139,7k / 58,1k |
| najväčší súbor GUI | `app/viewport.rs` 4 760 (limit 5 000) |
| produkčné funkcie GUI nad 150 riadkov / nad 300 | 36 / 4 |
| najdlhšie | `show_assembly_editor_content` 756, `show_feature_history_content` 655, `dispatch_command` 357, `paint_viewport_scene` 347 |
| ratchety `check_*.py` | všetky zelené |
| lokálny Linux job | 1 342 testov, 0 zlyhaní |

## 5. Štrukturálne návrhy

1. **Jeden model prierezu pozdĺž vlákna pre všetky úlohy prvku** (PG-1, PG-3, PG-5, PG-6).
   - A, hlavné I₁ a I₂, W a Q/(I·b) sa počítajú v staniciach pozdĺž vlákna z hotového dielu:
     tetiva profilu mínus rezy, kapsy, booleany a diery, alebo exact rez OCCT.
   - Ohyb, šmyk, zárez, čistý prierez aj vzper sa počítajú z týchto staníc.
   - Čo model nevie reprezentovať, je `not_verified` pre nosníky aj stĺpy.
2. **Smerový kontakt v exact workeri** (PG-2).
   - Worker vráti plochu kontaktu rozdelenú podľa normály vzhľadom na zadaný smer.
   - Gravitácia, `load_path` a `floating_part` v programe zdieľajú jeden predikát
     uloženia: počíta sa plocha smerom nadol, bočný kontakt len cez deklarovaný spoj.
3. **Jeden zoznam obrábania** (MF-5, priechodné diery).
   - Kolíkové diery, fyzické aj logické, sú operácie v `GeneralManufacturingProjection`.
   - BTLx, woodWOP, job, výkres aj brána schopností čítajú ten istý zoznam.
   - Krížový test overí rovnaký počet a polohu dier vo všetkých formátoch.
4. **Úpravy zachovávajú variant** (M-1, M-2, M-3).
   - `SetFeatureDimension` nahradí `SetFeatureParameter` nad deskriptorom.
   - Jeden modul politiky dôkazov rozhoduje, ktoré prvky ukotvujú ktoré plochy.
   - Operácie session (nahradenie dokumentu, migrácia, obnova) nikdy nestavajú `Self`
     nanovo.
5. **Rozpočet na typ požiadavky v jednej tabuľke** (EX-2, EX-5, EX-6, EX-7).
   - Výmenné požiadavky podľa bajtov × dielov, grafy podľa počtu uzlov.
   - Opakovať len pri chybe transportu.
   - Worker má vlastný protokolový kanál, vlastný deadline a zomrie s rodičom na všetkých
     platformách.
   - `ketchup-scheduler` sa testuje aj na Linuxe.
6. **Súhlas s pluginom kryje to, čo proces spustí, nie text argv** (SEC-7).
   - MCP obmedziť na priečinky pluginov; `plugin.json` menuje svoj skript, ktorý sa
     hashuje.
   - Interpretery spúšťať izolovane (`python -I -S`), stage v súkromnom priečinku.
7. **Jedno pole hodnoty `ValueField`** (UI-8, nízke body GUI).
   - Nesie text, druh (dĺžka, číslo, uhol, počet, zoznam) a oddeľovač a vracia
     `Result<Value, Refusal>` s náhľadom a lokalizovaným odmietnutím.
   - Používajú ho všetky polia, Revolve, polygón, počty aj `hole_spec`.
   - CSV berie oddeľovač z toho istého nastavenia.
8. **Exportér vráti, čo zapísal** (MCP-1). `Written { primary, reports }` pre stavový riadok
   okna aj odpoveď MCP.
9. **Jedna politika času v testoch** (TST-3, TST-4).
   - Ratchet na `elapsed() <` v bežných testoch.
   - Časové rozpočty len v ignorovaných `perf_*` testoch nočne v release.
   - Natívne Python testy v OCCT jobe.

## 6. Odporúčané poradie

1. **Hneď:**
   - PG-1 (nosník „pass“ na plnom priereze);
   - PG-2 (bočný dotyk ako podpora);
   - M-1 (tichá zmena extentu);
   - MF-5 (dvojité alebo chýbajúce kolíkové diery).
2. **Tento týždeň:**
   - EX-2 (timeout STEP);
   - M-2, M-3;
   - MF-6, MF-7;
   - EX-3, EX-4;
   - SEC-7;
   - MCP-1;
   - UI-8;
   - VW-12.
3. **Potom:** zvyšok P2 a P3 a štrukturálne body 1, 3 a 4 ako samostatné bloky podľa
   AGENTS §6.
