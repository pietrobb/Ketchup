# Podrobný code review Kečupu — 2026-10-02

## 1. Stav, baseline a obnova

**OPRAVY R1–R7 OVERENÉ; R8 PREBIEHA.** Všetkých 17 nálezov má implementáciu a cielenú PASS regresiu; celá vetva ešte NIE JE hotová. Aktuálny fokus #2388: záverečný workspace Clippy/test a izolovaný native produkčný build. Historické výsledky analýzy baseline e92f4a7 nižšie sa nesmú zamieňať za výsledky opraveného stromu. Presné výsledky opráv a najnovší handoff sú v §6.

- Reviewovaný HEAD: `e92f4a73844b185570dfbb49a807ab2001ecd845`, vetva `main`.
- Pracovný strom bol na začiatku čistý. Pôvodný review menil iba správy/reproduktory; následné opravy produkčných zdrojov a testov boli používateľom schválené 2026-10-02 16:50 a sú necommitované v tom istom pracovnom strome.
- Bez commitu, pushu, aplikovania alebo mazania stashov. Z2 skupiny zo `stash@{0}` nie sú súčasťou tohto vydaného baseline; `stash@{1}` ostáva historickou zálohou.
- Žiadna fyzická myš, vstup do používateľovho GUI, živé OAuth volania ani pripojenie k jeho modelom. UI testovanie cez headless harness.
- Staré review a HMOS slúžili na orientáciu, nie ako dôkaz súčasných chýb. Každý nižšie uvedený mechanizmus bol skontrolovaný v aktuálnych zdrojoch.

### Trvalé prílohy

| Príloha | Obsah |
|---|---|
| [Model a persistence](code-review-2026-10-02-model.md) | M1–M5: atomicita, recovery, história, skupiny; presné scenáre a negatívne overenia |
| [Program a geometrické kontrakty](code-review-2026-10-02-program.md) | PRG-01–04: revolúcia, kontakt, kolíky, parametrický rewrite |
| [Protokol a bezpečnosť](code-review-2026-10-02-protocol.md) | PR-01–05: attach, cancellation, expected, výber okna, bounded súbory |
| [GUI, architektúra, validácia](code-review-2026-10-02-gui-validation.md) | G1–G3, metriky, exporty, vyradené podozrenia |
| [Runtime reprodukcie](code-review-2026-10-02-reproductions.md) | osem izolovane overených funkčných scenárov, vstupy a namerané výstupy |

Prvé tri prílohy sú nezávislé statické kontroly. Hlavný reviewer prečítal citované implementácie a dodatočne vykonal runtime reprodukcie; preto ich označenie „bez runtime“ opisuje činnosť konkrétneho pomocného reviewera, nie konečný stav overenia v tejto hlavnej správe.

## 2. Záver v skratke

**17 nálezov: 6 P1 a 11 P2.** Z toho osem funkčných nálezov bolo potvrdených izolovaným behom aktuálnych Rust knižníc, ďalší nález zoskupuje reprodukované zlyhania existujúcich kontrol. Zvyšných osem má konkrétny statický dôkaz, nie vykonaný exploit, hardware test alebo fault injection.

Najväčšie riziká sú ochrana neuloženej práce, zablokovanie synchronného vlákna, nesprávne geometrické predpoklady a lokálna autentifikácia. Nejde predovšetkým o kozmetiku ani o ďalšie rozdeľovanie veľkých súborov.

**Konečný stav po opravách:** všetkých 17 nálezov má opravu a cielené PASS regresie; celý workspace **2566 PASS / 0 FAIL / 5 explicitných ignored** (štyri externé/opt-in GUI skúšky a jedna subprocess fixture), Python/SDK **322 PASS / 0 skips / 71 subtests PASS**, povinný native runner **12 PASS / 0 skips**. Clippy všetky targets/features, fmt a všetkých osem statických kontrol PASS. Headless rectangle/PushPull, Save/Open, príklady a export BOM PASS. Podrobnosti finálneho behu a limity sú v záverečnom overení nižšie; pôvodné červené výsledky baseline zostávajú zdokumentované historicky v §4.

Priorita P1 znamená významné riziko integrity, dostupnosti alebo bezpečnostnej hranice; P2 je konkrétna chyba alebo vývojový problém nižšej naliehavosti. Bez P0: nebol preukázaný bezpodmienečný kolaps základnej práce ani vzdialený exploit cez internet.

### Register nálezov

| ID | Priorita | Nález | Dôkaz | Príloha |
|---|---|---|---|---|
| CR-01 | P2 | Červené CI ratchety a nesprávny predpoklad migračného testu | OPRAVENÉ/PASS: migration 13, assistant 136, ratchety a Python 298; R1 | G3 + §4 |
| CR-02 | P2 | Exact história GUI rastie bez limitu | OPRAVENÉ/PASS: exact_history_releases… Weak/Arc + Undo/Redo; bg83 | G2 |
| CR-03 | P1 | Bežný štart odmietne dve vhodné GPU | OPRAVENÉ/PASS: adapter_tests 2, 0/1/2 GPU a explicitný selector; bg83 | G1 |
| CR-04 | P1 | Druhá session prepíše recovery prvej | OPRAVENÉ/PASS: document_session + GUI file_workflow/recovery; bg63/65 | M1, R1 |
| CR-05 | P2 | Duplicate extension mutuje obsah napriek Err | OPRAVENÉ/PASS: native_persistence duplicate bytes/required/roundtrip; bg65 | M2, R2 |
| CR-06 | P1 | Cyklus skupín zavesí konverziu skôr než validáciu | OPRAVENÉ/PASS: group_ 3 vrátane cyklu/forward refs; bg66 | M3, R7 |
| CR-07 | P2 | Po úspešnom publish a zlyhaní fsync sa rollbackne iba pamäť | OPRAVENÉ/PASS: spoločná publish fault-injection session/GUI Undo/Redo/Save; natívny Unix nebežal | M4 |
| CR-08 | P2 | Rollback nedokáže obnoviť source-only revíziu | OPRAVENÉ/PASS: source/overrides Undo/Redo/Save/Open; bg63/65 | M5, R6 |
| CR-09 | P1 | Revolúcia ignoruje os v obálke; chýba kolízny kandidát | OPRAVENÉ/PASS: revolve_bounds + exact kolízia po rotácii/posune; bg82 | PRG-01, R3 |
| CR-10 | P2 | contact() vracia plochu v prázdnom rohu profilu | OPRAVENÉ/PASS: triangle/concavity/void/contact area + úplná program sada 115; bg82 | PRG-02, R4 |
| CR-11 | P2 | dowels() umiestni diery mimo otočenej dosky | OPRAVENÉ/PASS: contact_review 6, oba páry a materiál po rotácii/posune | PRG-03, R5 |
| CR-12 | P2 | Nelineárny parametrický Push/Pull nedosiahne požadovaný rozmer | OPRAVENÉ/PASS: lineárny, 121 mm kvadratický, piecewise a hranice; bg82 | PRG-04, R8 |
| CR-13 | P1 | Automatický attach nerozlišuje OS používateľov | OPRAVENÉ/PASS: Windows restricted-token/ACL + bootstrap/reconnect; bg65/84; nie druhý živý účet | PR-01 |
| CR-14 | P1 | ApplyProgram blokuje UI a môže commitnúť po zrušení requestu | OPRAVENÉ/PASS: budget + recovery, async cancel/EOF/document switch + Undo/Redo; bg66 a program_edit 8 | PR-02 |
| CR-15 | P2 | Niektoré metódy ignorujú prijaté expected | OPRAVENÉ/PASS: 6 stale metód, bez vedľajších účinkov, optional guard zachovaný; bg84 | PR-03 |
| CR-16 | P2 | open_window môže vybrať cudzie súbežne otvorené okno | OPRAVENÉ/PASS: private readiness streams, opačné poradie a vlastné endpointy; bg84 | PR-04 |
| CR-17 | P2 | source_path načíta celý súbor pred uplatnením limitu | OPRAVENÉ/PASS: limit+1, 1 GiB, Unicode/escaping, NUL a následný send; bg84; Unix cfg nevykonané | PR-05 |

## 3. Nálezy, dopad a odporúčané opravy

### CR-01 — Kontrolná sada nie je zelená

**Miesta:** `crates/ketchup-assistant/src/sidecar.rs` funkcia `validate_with_part_cuts`; `tests/test_check_crate_layers.py:137`; `tests/test_check_no_named_products.py:76`; `crates/ketchup-model/tests/legacy_migration.rs:75–85`; `.github/workflows/ci.yml:28–47,63–67`.

1. `validate_with_part_cuts` má 733 riadkov, limit 400. Zároveň ostala neaktuálna výnimka pre pôvodnú funkciu `validate`.
2. Named-product ratchet hlási šesť prekročení v troch súboroch: `scene_snapping.rs:panel`, `sidecar.rs:dowel/panel/shelf`, `document/command.rs:cabinet/dowel`. V `command.rs:540–541` ide iba o dokumentačný komentár, nie o doménovú implementáciu. Neuvádzam to ako dôkaz poškodenia všeobecnosti jadra.
3. Migračný test bezpodmienečne požaduje, aby digest po načítaní bol odlišný od pôvodného. Nový `examples/wardrobe-chair-table.ketchup` má zhodný digest `51ffc968bfab991f`; test preto padne. Samotné otvorenie pred aserciou uspelo. Zlyhanie nedokazuje pokazenú migráciu ani poškodený príklad, ale chybný predpoklad „všetky príklady sú staré“.

**Oprava:** validátor rozdeliť podľa operácií; bez zvýšenia ratchet limitu. U komentárov zosúladiť text a kontrolu bez pridávania nových architektonických výnimiek. Migračný test musí overovať zachovanie modelu a stabilný roundtrip aj pri aktuálnom formáte; zmenu digestu vyžadovať len tam, kde je naozaj súčasťou migrácie. Nevyhadzovať nový príklad ani test ako celok.

### CR-02 — Exact cache prežije dostupnú Undo históriu

**Miesta:** `crates/ketchup-app/src/app_state.rs:187–188`, `app/exact.rs:173–214,244–256`; čistenie iba `app/document.rs:109–112` a `app/exact.rs:387–389`.

Registre sa archivujú pre každú source revíziu, ale nikdy sa neprerezávajú podľa dostupných Undo/Redo snapshotov. Zdieľané Arc obmedzia duplicity nezmenených dielov, nie životnosť starých zmenených balíkov. Pri dlhej relácii tak rastie RAM aj po vypadaní starých revízií z Undo. Nameraný OOM ani konkrétne MB sa netvrdia.

**Oprava/test:** eviction podľa dostupných snapshotov alebo explicitného memory budgetu; po sérii zmien dlhšej než Undo limit overiť uvoľnenie starých produktov.

### CR-03 — Štart s viacerými GPU zlyhá

**Miesta:** `crates/ketchup-app/src/main.rs:137–140`, `app/exact.rs:109–149`.

Normálny entry point nastaví `requirement=None`, napriek tomu selector vyžaduje práve jeden vyhovujúci fyzický DX12 adaptér. Integrovaná aj dedikovaná GPU podporujúca surface znamenajú Err a žiadne okno. Je to staticky dosiahnuteľná chybová vetva; test na dvoch GPU nebol vykonaný.

**Oprava/test:** automaticky vybrať preferovanú dostupnú kartu, presnú jedinečnú identitu vyžadovať iba pri explicitnom diagnostickom režime. Testovať voľbu nad 0/1/2 kandidátmi bez nutnosti HW certifikácie.

### CR-04 — Recovery dvoch relácií má jediného víťaza

**Miesta:** `crates/ketchup-model/src/persistence.rs:1018–1039`; `crates/ketchup-application/src/session.rs:304–328`.

Kontrola porovnáva iba primárny uložený súbor. Druhá session preto môže nahradiť `.work-recovery` prvej, hoci primárny súbor ostáva nezmenený. Lock serializuje zápisy, neodlíši vlastníctvo vetvy. Runtime overenie: oba zápisy uspejú, reopen obnoví len `branch_B`; A už nemá samostatný checkpoint. Po páde sa stratia už checkpointované neuložené zmeny prvej relácie.

**Oprava/test:** compare-and-swap aj recovery identity pod rovnakým lockom, alebo samostatné vetvy relácií; pri konflikte zachovať obe verzie. Test dve otvorené sessions → odlišné zmeny → reopen/recovery bez straty vetvy.

### CR-05 — Odmietnutá extension sa napriek Err prepíše

**Miesto:** `crates/ketchup-model/src/persistence.rs:124–128`.

`BTreeMap::insert` nahradí obsah pred testom `is_some()`. Volajúci dostane DuplicateContainerEntry, ale pôvodné bytes aj required už neplatia. Reprodukcia vložila A/false, druhý insert B/true vrátil Err, v kontajneri zostalo B/true. Ide o verejné API; netvrdím, že parser po chybe publikuje rozrobený kontajner.

**Oprava/test:** entry API so zachovaním pôvodnej hodnoty na occupied vetve. Testovať bytes aj required pred/po Err a následný roundtrip.

### CR-06 — Konverzia skupiny môže uviaznuť na nevalidovanom cykle

**Miesta:** `crates/ketchup-model/src/document/group_conversion.rs:11–37`; `document/store.rs:2732–2802`, finálna validácia až za command slučkou.

Dávka vytvorí root 1, skupinu 2 s parent 3 a skupinu 3 s parent 2, potom konvertuje root 1. `descendant_groups` prechádza všetky skupiny; parent traversal na 2↔3 nemá visited ani limit a nikdy nedosiahne finálnu validáciu. Izolovaný reproduktor nedokončil štyri príkazy do 3 s; bol ukončený vlastným timeoutom. Kód dokladá nekonečnú slučku, nie iba pomalú geometriu.

**Oprava/test:** bezpečný traversal aj nad medzistavom alebo validácia grafu pred konverziou; zachovať podporované forward references. Regresný subprocess test musí skončiť rejection v bounded čase a bez publikácie.

### CR-07 — Publish checkpointu a jeho durability majú zle oddelené chyby

**Miesta:** `crates/ketchup-model/src/persistence.rs:945–978,1038`; `crates/ketchup-application/src/session.rs:374–398`; `document/store.rs:24–41`.

Na Unix môže persist/rename uspieť a následný fsync adresára zlyhať. Finalizer vráti Err, session obnoví starú pamäť, ale nový recovery súbor už zostáva na disku; po reopen sa odmietnutá operácia objaví. Low-level testy `persistence.rs:2040–2074` výslovne zachovávajú publikovaný obsah pri injektovanej sync chybe. Cross-layer session scenár nebol fault-injektovaný. Na Windows je parent sync no-op, tento konkrétny trigger tam neplatí.

**Oprava/test:** rozlíšiť „nepublikované“ od „publikované, nepotvrdená durability“ a zjednotiť session/disk výsledok. Test finalizeru s chybou po publish, následný Open. Neignorovať fsync chybu.

### CR-08 — Rollback nevidí rozdiel zdroja pri rovnakej geometrii

**Miesta:** `crates/ketchup-model/src/document/store.rs:184–219,687–735`.

NoOpRollback sa rozhoduje iba podľa product canonical digest. Source-only revízie však majú iný zdroj/overrides a rovnaký produkt. Reprodukcia A→B→rollback na A vrátila `Err(NoOpRollback)` a ponechala B. Bežné Undo funguje; nález sa týka explicitnej rollback operácie.

**Oprava/test:** porovnať aj existujúci rule_program obsah, nepridávať nový digest. Source-only rollback musí obnoviť zdroj, overrides a správny obsah po Save/Open.

### CR-09 — Obálka revolúcie nie je konzervatívna pre podporované osi

**Miesta:** `crates/ketchup-program/src/model.rs:993–1001`, `eval.rs:855–858`, `exact.rs:105–126`.

Telo okolo X z profilu `(0,10),(100,10),(100,20),(0,20)` má analytickú obálku `(0,-20,-20)..(100,20,20)`. Runtime vrátil `(-100,10,-100)..(100,20,100)` a `reach(-Y)=-10` namiesto 20. Malý box v materiáli dolnej steny rúry preto z broad-phase vypadol: `exact_candidates={}`, `issues=[]`. Os sa v geometrickej operácii podporuje, ale rozmery/obálka stále predpokladajú Y cez nulu. Toto nie je iba voľnejšia aproximácia: obálka časť telesa neobsahuje.

**Oprava/test:** odvodzovať bounds/reach z profilu a skutočnej osi, vrátane posunutej osi a uhla revolúcie. Výsledok musí byť konzervatívny; metamorfné testy rotácie a posunu + exact kolízny oracle. Runtime reproduktor overil evaluator a kandidátov, nie OCCT intersection.

### CR-10 — Obdĺžnik cap face sa vydáva za skutočný obrys

**Miesta:** `crates/ketchup-program/src/faces.rs:332–341`; `contact.rs:64–69`.

Trojuholníkový profil x+y<=100 s výškou 30 a box na `(80,80,30)` vrátia kontaktný štvorček 10×10. Runtime patch má body x+y>=160, teda všetky v prázdnom priestore. Funkcia pracuje s profile_bounds obdĺžnikom, nie s polygonálnym obrysom. Neskoršie exact spresnenie reportu neopraví už vykonané vetvenie programu ani machining založený na výsledku builtin.

**Oprava/test:** preniesť skutočný face boundary do kontaktu a orezávať ním; chýbajúci presný kontakt neprezentovať ako istý. Testy trojuholník, konkávny profil, otvor v ploche, natočenie a posun.

### CR-11 — Kolíkový helper ignoruje kontaktný polygón

**Miesto:** `crates/ketchup-program/library/prelude.star:824–837`.

Dve box faces majú správny kontakt, ale pri hornom diele 200×20 natočenom o 45° helper použije stredovú čiaru obalového obdĺžnika. Vzniknú body s lokálnym Y≈57.929 a -37.929 pri platnom intervale 0..20. Runtime validátor vrátil dve `hole_outside_face`; nie je to potichu zelený report, ale helper z platného kontaktu vytvorí chybnú požiadavku. Oprava samotného contact() tento prípad nevyrieši.

**Oprava/test:** row a offset určovať vnútri skutočného polygonálneho patch s okrajovou rezervou, všeobecne pre orientácie a profily. Test musí overiť oba páry otvorov v reálnom materiáli, nie iba počet dier.

### CR-12 — Jeden krok numerickej derivácie nie je riešenie parametra

**Miesto:** `crates/ketchup-application/src/rule_program.rs:106–161`; GUI volá túto cestu cez `planar_push_pull.rs:213–218`.

Pre `H=10`, `distance=H*H` a Push/Pull +21 mm má byť výsledok 121 mm. Skutočné API nastavilo `H=11.049895010499`, výšku `122.100179743051` — chyba **1.100179743051 mm**. Jedna finite-difference sonda sa používa ako presný lineárny driver; posledné evaluate kontroluje len vyhodnotiteľnosť, nie dosiahnutý rozmer.

**Oprava/test:** parametrický override prijať až po kontrole finálneho rozmeru; použiť bounded solver alebo bezpečný part-local operation fallback. Testovať lineárnu, kvadratickú, clamp/piecewise závislosť a hranice parametra. Žiadny tichý úspech s iným rozmerom.

### CR-13 — Loopback attach nie je autentifikácia OS používateľa

**Miesta:** `crates/ketchup-app/src/live_bridge/consent.rs:185–216,407–426,551–602`; aktivácia `main.rs:152–157`.

Broker overí loopback IP a tvar nonce, potom automaticky odovzdá platný bridge token. Iný lokálny OS používateľ môže nájsť port aj bez prístupu k per-user registry a získať čítanie/mutácie modelu používateľa A. Nový attach preberie staré spojenie. Následné requesty token kontrolujú správne, slabinou je jeho vydanie. Netvrdí sa prístup zo vzdialeného internetu ani potreba sandboxovať všetky procesy toho istého používateľa.

**Oprava/test:** OS autentifikovaný lokálny transport s ACL alebo zabezpečený bootstrap secret/explicitný consent. Zachovať pohodlný same-user reconnect. Testovať prijatie vlastného účtu a odmietnutie iného účtu; exploit nebol spustený.

### CR-14 — Program sa plánuje na UI bez budgetu a bez poslednej cancellation kontroly

**Miesta:** `crates/ketchup-app/src/live_bridge/program_check.rs:86–98,144`; `program_edit.rs:74–136`; `crates/ketchup-program/src/eval.rs:2394–2399`.

Krátky Starlark zdroj môže robiť veľmi dlhé slučky bez vytvárania dielov. Part limit ani frame limit to neobmedzí. Interpreter/planner beží synchrónne na UI; cancellation sa skontroluje pred ním, nie tesne pred commitom. Timeout/EOF transportu počas výpočtu preto nezastaví UI a nemusí zabrániť neskorej zmene dokumentu. Exact worker timeout začína až po publikácii, nie pri interpretácii.

**Oprava/test:** vyhodnotenie/plánovanie mimo UI s execution/time budgetom; na UI publikovať až po kontrole cancellation a pôvodného dokumentu. Deterministický test pozastavené plánovanie → cancel → dokončenie nesmie pridať revíziu ani Undo. Úmyselne dlhý program sa v používateľovom okne nespúšťal.

### CR-15 — expected sa prijíma, ale nie všade kontroluje

**Miesta:** `crates/ketchup-app/src/live_bridge.rs:2019–2042,2049–2052,2088–2094,2327–2343`; MCP kontrakt `crates/ketchup-mcp/src/schema.rs:22–23` a ďalšie method schemas.

Query/detail/edit_context/view a niektoré status metódy zahodia `expected` cez `..`. Po výmene dokumentu môžu so starým stampom vrátiť inú entitu s rovnakým ID alebo zmeniť kameru. Globálna cancellation kontrola nie je kontrolou klientovho stampu. Commit cesty majú vlastné guardy; nález netvrdí, že sa tým obídu.

**Oprava/test:** buď jednotne uplatniť voliteľný existujúci guard, alebo parameter aj jeho sľub odstrániť z konkrétnych schém. Nezavádzať povinnú stamp byrokraciu. Test starého expected po otvorení iného dokumentu.

### CR-16 — open_window môže pripojiť nesprávne nové okno

**Miesto:** `crates/ketchup-mcp/src/tools.rs:347–390`.

Po spustení child procesu vyberá prvé discovery ID, ktoré nebolo v zozname „before“. Pri dvoch súbežných štartoch môže vyhrať cudzie okno, pretože výber sa nekoreluje so spusteným child procesom. Následné editácie potom smerujú inde. Overenie zvoleného instance ID pri attach iba potvrdí nesprávnu voľbu.

**Oprava/test:** readiness/bootstrap kanál vlastného child procesu s jeho konkrétnou identitou; dva súbežné štarty s opačným poradím registrácie. GUI race experiment nebol spustený.

### CR-17 — Súborový vstup obíde limit pred načítaním

**Miesto:** `crates/ketchup-mcp/src/tools.rs:169–185`; finálna kontrola rámca `bridge.rs:61–66`.

`read_to_string` pre source_path načíta celý obsah bez limitu a kontroly regular file. Až potom serializácia/bridge uplatní frame budget. Náhodné zadanie veľkého logu môže spotrebovať RAM; špeciálny súbor na podporovanej platforme môže blokovať pred socket timeoutom.

**Oprava/test:** bounded read limit+1, regular-file kontrola a finálny envelope budget vrátane JSON escapingu. Test nadlimitný súbor sa musí odmietnuť bez načítania celého obsahu; osobitne Unicode/escaping a podporované špeciálne súbory.

## 4. Historické kontroly baseline pred opravami — čo bolo a nebolo overené

Build prostredie: Windows, `KETCHUP_OCCT_ROOT=C:\Sources8\Ketchup\third_party\occt-install-r0-v1`.

| Kontrola | Výsledok |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --locked --workspace --lib --no-fail-fast -- --test-threads=2` | PASS, 686 testov, 1 ignored subprocess fixture |
| `python -m pytest -q tests sdk/python/tests` | 296 passed, 2 failed, 24 skipped, 71 subtests passed |
| `check_crate_layers.py` | FAIL, veľkosť validátora/neaktuálna výnimka |
| `check_no_named_products.py` | FAIL, 6 prekročení |
| error hygiene, test env, test hooks, test sleeps, tolerance literals, tool rail docs | všetkých 6 PASS |
| plný worker build + workspace all-targets test | nespustená testová fáza: build workera blokoval Windows file lock |
| workspace integration bez scheduler/headless | 1664 passed, 1 failed, 4 ignored; app 500 pass, model 556 pass/1 fail; vnorené subprocess výsledky sa nepočítajú duplicitne |
| osem izolovaných funkčných reprodukcií | všetky potvrdili popísané mechanizmy |

Plný build sa zastavil pri odstraňovaní `target/debug/ketchup-exact-worker.exe`, ktorý používa existujúci `ketchup-headless` proces. Cudzie procesy neboli ukončené. Toto sa nepočíta ako zlyhaný Rust test ani chyba kompilácie.

Náhradný integračný príkaz: `cargo test --locked --workspace --exclude ketchup-scheduler --exclude ketchup-headless --test integration --no-fail-fast -- --test-threads=2`. Finálny log `.claude/bg/20261002_145221_bg25-18dab77b301c428c.log` hlási iba jeden failed target (`ketchup-model`, CR-01). Štyri ignored app testy potrebujú živý OAuth (2), externý 140-body benchmark (1) alebo lokálny Blender/parity model (1). Python native scenáre vyžadujú explicitný čerstvý `KETCHUP_HEADLESS`, ktorý v tomto behu nebol nastavený; skipped scenáre nie sú dokladom funkčnosti.

Ďalšie logy: `.claude/bg/20261002_144157_bg19-18dab6e9cb7486c8.log` (fmt/ratchet), `20261002_144218_bg20-18dab6eeb5ef9a7c.log` (worker lock), `20261002_144218_bg21-18dab6eeb9a1a46c.log` (Python), `20261002_144306_bg22-18dab6f9db9d4b88.log` (ostatné ratchety), `20261002_144552_bg23-18dab7208f984068.log` (lib tests), `20261002_145056_bg24-18dab7675814dc54.log` (Clippy).

## 5. Architektúra a pozitívne zistenia

- 17 workspace crates. Inventár sledovaných Rust/C++/Python/Starlark zdrojov v crates/sdk/skills/scripts/tests: **505 súborov, 429 777 riadkov**. Rozdelenie podľa crates je v GUI prílohe; nejde o line-by-line prečítanie celého inventára ani o code coverage metriku.
- `ketchup-app/src/lib.rs` klesol zo 36 480 riadkov v minulom reporte na **4 754**. Zdrojové moduly sú pod 5 000 riadkov. Stále veľké: model/drawing 4955, manufacturing/fabrication 4741, app/viewport 4482, assistant/sidecar 4252. Samotná veľkosť nie je samostatný bug.
- Canonical batch používa kandidáta a publikuje po validácii. Async exact má cancellation aj source/revision kontrolu; Drop tasku volá cancel. Nehlásime neexistujúci „zabudnutý cancel“ pri `take()`.
- Transport má autentifikáciu každého bridge rámca, bounded queue/frame, deadline čítania a unknown-outcome report pri strate spojenia. Slabina CR-13 je bootstrap, nie absencia všetkých týchto mechanizmov.
- Scale je implementovaný a má headless regresie. Otvorené HMOS ciele sa nepoužili ako dôkaz chýbajúcej funkcionality.
- Exporty kontrolujú current exact výsledok, výrobné transformácie, vstupné strany a hĺbky vrtov; pockets vyžadujú explicitný nástroj. Nepotvrdila sa tichá mesh-for-exact náhrada v kontrolovaných cestách. Konkrétna kompatibilita so strojom HOMAG/woodWOP nebola skúšaná.
- SDK manufacturing má staged no-replace export, ochrany ciest a textové bunky namiesto spreadsheet vzorcov. CAD catalog sa generuje z Rust serde typov; nejde o celý ručne duplikovaný protokol.

## 6. Odporúčané poradie opráv

1. **Ochrana práce a autority:** CR-04 recovery vetvy, CR-13 same-user attach, CR-14 budget/cancel pred publikovaním. Samostatné testy zlyhaní a zachovania používateľského stavu.
2. **Ukončenie a štart aplikácie:** CR-06 cycle-safe traversal, CR-03 normálny výber GPU. Obe chyby sa dajú izolovať bez veľkého refaktoru.
3. **Správnosť geometrie a parametrov:** CR-09, CR-10, CR-11, CR-12. Všeobecné bounds/face boundary/polygon/solver riešenia, nie výnimka pre konkrétnu rúru či skriňu. Reproduktory sú v prílohách.
4. **Transakčné okraje a pamäť:** CR-05, CR-07, CR-08, CR-02. Rozlišovať obsah programu, produkt a stav už publikovaného súboru; nepokrývať rozdiel ďalším digestom.
5. **Kontrakt MCP a resource bounds:** CR-15–17. Ochrana konzistencie má byť pravdivá a voliteľná, nie ďalšie nepotrebné kroky pri každej editácii.
6. **CI priebežne, nie až nakoniec:** CR-01 opraviť pred tým, než sa začne červená sada používať ako baseline. Reprodukcie previesť na regresné testy správnych výsledkov; do trvalej sady nekopírovať asercie dnešného chybného správania.

**Opravy autorizované používateľom 2026-10-02 po dokončení review. Checkpoint opráv: plán a automatické pokračovanie nastavené, implementácia ešte nezačala.** HMOS vetva #2380; poradie: R1 #2381 (CR-01, aktuálny fokus), R2 #2382 (CR-04/05/07/08), R3 #2383 (CR-13), R4 #2384 (CR-06/14), R5 #2385 (CR-09/10/11/12), R6 #2386 (CR-02/03), R7 #2387 (CR-15/16/17), R8 #2388 (úplné overenie a cleanup). Každý nález musí dostať opravu a úspešnú regresiu správneho výsledku; pri každom kroku sem zapisovať zmeny, vykonané testy/logy a ďalší krok. Cron #68: work_branch výhradne #2380, sekvenčne každých 15 minút. Cron #69: progress_check #2380 každú hodinu, po troch kontrolách bez pokroku pozastaví crony vetvy. Po overenom dokončení všetkých 17 opráv a R8 odstrániť #68 aj #69 cez cron_remove, overiť cron_list, uzavrieť #2380 a neprechádzať na nesúvisiace ciele. Skips, nulové behy a blockery nie sú PASS. Bez commitu/pushu, zásahov do stashov či používateľovho modelu. Nasledujúci krok: prečítať aktuálny git stav a dotknuté zdroje CR-01, opraviť červený CI baseline a vykonať jeho regresie.

**Checkpoint opráv 2026-10-02 16:56:** Používateľ po preverení schválil opravu všetkých 17 nálezov; crony #68/#69 znovu zapnuté a cron_list potvrdil ON. R1 #2381 rozpracované: šesť named-product prekročení odstránených z komentárov v sidecar.rs, scene_snapping.rs a document/command.rs, bez zmeny limitov. Named-product ratchet PASS, jeho Python regresie 5 PASS (bg27-18dabe3131b5b210). legacy_migration.rs už nevyžaduje zmenu digestu pri každom súbore: požaduje pokrytie zmenených aj nezmenených digestov, zachovanie occurrences/features a stabilný roundtrip; migračné regresie PASS: cargo test --locked -p ketchup-model --test integration migration -- --test-threads=2, 13 passed/0 failed/0 ignored (bg26-18dabe312eab8144). cargo fmt --all -- --check a git diff --check PASS. Ďalej: rozdeliť validate_with_part_cuts (sidecar.rs) bez novej výnimky alebo zvýšenia limitu, odstrániť neaktuálnu výnimku validate v scripts/check_crate_layers.py, spustiť príslušné ratchety/testy a až potom uzavrieť R1. R1 / CR-01 dokončený 17:06: validate_with_part_cuts rozdelený na krátke kontroly operácií; neaktuálna výnimka validate odstránená bez zvýšenia limitov. Oba ratchety PASS; Python regresie oboch kontrol 26 PASS (bg29-18dabe95ba5745d4); cargo test --locked -p ketchup-assistant --all-targets -- --test-threads=2 PASS (bg28-18dabe95b5fd60cc). Paralelné nezávislé opravy: persistence c8188797, geometria 12b1c2b1, GUI/MCP 0a898fc0, attach 1b07e833; checkpointy v príslušných prílohách. Hlavný agent: CR-06 cycle-safe prechody v group_conversion.rs, dve nové regresie (cykly bez partial publish a forward refs+Undo); cargo test -p ketchup-model --test integration group_ PASS 3 testy (bg40-18dabf4751874038). CR-14 execution_budget.rs s instrumentáciou aj prelude, runaway+ďalší program PASS (bg38-18dabf264f4e933c); asynchrónny program_check.rs používa fork_for_planning + publish_program_plan a guard/cancellation pred publikáciou, deterministic tests v program_plan_tests.rs. Beží bg43-18dabf750cdc010c: app program_check tests, GPU/cache/expected a MCP. Geometrický agent 12b1c2b1 skončil čiastočne CR09/12 bez testov, pokračuje 159a54a0 s CR10/11 a regresiami. Agent GUI 0a898fc0 dodal zmeny, testy čakali na local_auth.rs (už existuje). Persistence c8188797 a security 1b07e833 ešte bežia. Ostatné CR neuzatvárať bez kontroly diffs a skutočných regresií.

### Aktuálny checkpoint opráv — 2026-10-02 17:36

Všetky paralelné workstreamy skončili a majú čiastočné reporty v `.claude/tasks/{c8188797,12b1c2b1,0a898fc0,1b07e833,159a54a0,05a2ca65}_result.txt`. Zmeny pre všetky CR sú v pracovnom strome, ale okrem R1/CR01 sa ciele ešte NESMÚ uzavrieť. Crony #68/#69 ON; #2381 done, fokus #2382. Bez commitu/pushu/stash alebo zásahov do cudzích procesov.

- Hlavný agent priamo overil MCP `cargo test --locked -p ketchup-mcp --lib -- --test-threads=2`: **11 PASS**, vrátane Windows restricted-token denial, privátneho bootstrapu, bounded UTF8/escaping a korelácie readiness. Discovery patch z docs/cr-13-discovery.patch už manuálne integrovaný do discovery.rs; mock v tests.rs používa local_auth::create a overí bootstrap. Patch znovu NEAPLIKOVAŤ.
- App `--lib program_check::tests`: **2 PASS** (asynchrónne publikovanie + jeden Undo/Redo, cancellation a výmena dokumentu bez publish). Program runaway+ďalší program **1 PASS**; skupiny group_ **3 PASS**. Budget používa Starlark DAP fallible statement hook aj v prelude; generické native volania nie sú tvrdý procesový sandbox. Reconciliation beží vo vlákne, canonical commit ostáva UI transakcia.
- Persistence základ: podľa reportu agenta 12 unit +33 native +4 session testy PASS; GUI integrácia v document.rs/app_state.rs/commands.rs/shell.rs doplnená agentom 05a2ca65, ešte potrebuje runtime. Published-error a recovery ownership testy sú v existujúcich GUI súboroch.
- Najnovšie `cargo fmt --all`, check_crate_layers, check_error_hygiene, check_no_named_products, git diff --check a Python ratchet regresie **26 PASS**. Úplný Python beh mal 297 PASS/1 FAIL/24 skip; jediný fail bol odvtedy opravený linalg ratchet. Plný Python ešte zopakovať.
- Bg59-18dac051547b8b04 prešiel po consent testy (4 PASS), potom compile E0308 vo faces_boundary.rs (3D normal poslané dot2); tento výraz opravený na [normal[0], normal[1]]. Predchádzajúce contact_review/GPU/cache/expected výsledky vyčítať z celého logu, nie odhadovať z konca. Bg60-18dac051570ba778 GUI integration compile E0308 live_bridge_bootstrap.rs:304 (Response.stamp je Option); oprava má obaliť live_bridge_stamp do Some a následne beh zopakovať.
- Duplicitné dot2/cross2 po súbežnej integrácii odstránené; v linalg ostáva jedna implementácia, contact_polygon importuje spoločné funkcie. Dead-code warning `faces.rs::box_faces` stále treba odstrániť po kontrole volaní pred Clippy. Architektonické ručné súčiny vo faces/faces_boundary/model nahradené spoločným linalg.
- **Bezprostredne pokračovať:** opraviť/overiť dve vyššie uvedené typové integrácie; cargo test app --test integration file_workflow:: + work_recovery_checkpoint_fails + live_bridge_bootstrap::; application --test integration rule_program_review:: (CR12+exact oracle); program celá integration sada (contact_review, revolve_bounds a staré prípady); app program_edit tests. Prekontrolovať GUI diffy CR04/07 a polygon contact aproximácie, potom aktualizovať CR register a zatvárať len skutočne overené ciele.
- **Záverečné overenie ešte NEBOLO vykonané:** plný Clippy, workspace tests, native produkčné smoke cez scripts/run_production_tests.py a všetky required regresie. Používateľov headless PID27580 a worker PID138496 stále držia target/debug binárky; nezabíjať ich. Na úplný build použiť izolovaný CARGO_TARGET_DIR (cca170GB voľných v čase kontroly), so správnym OCCT_ROOT/DLL PATH. Crony odstrániť až po všetkých opravených/overených CR a R8.

### Pokračovanie opráv — 2026-10-02 17:43

R2 stále aktívny. Opravená Option<Stamp> asercia v live_bridge_bootstrap.rs. Bg61-18dac0984681b590: GUI file_workflow 67 PASS/1 FAIL; zlyhal nový fault-injection test, lebo Undo do čistého stavu checkpoint maže a sync nevolá. Scenár opravený na dve neuložené zmeny, takže Undo aj Redo skutočne publikujú checkpoint. Navyše kontrola session.rs odhalila Save As overwrite do cudzieho súboru s opusteným recovery: nový test v document_session::recovery_conflict túto stratu ochrany reprodukoval (bg62-18dac0aacdcbf0bc FAIL na očakávanom odmietnutí). Oprava kontroluje recovery identitu aj pre iný cieľ, kde musí byť checkpoint neprítomný; pôvodný súbor aj neuložené zmeny zostanú zachované. Beží bg63-18dac0ca4cddaee4: kompletné document_session → GUI file_workflow → work_recovery_checkpoint_fails → live_bridge_bootstrap → native_persistence. Bg63 výsledok: document_session 21 PASS (vrátane novej Save As regresie), GUI file_workflow 68 PASS, recovery failure 7 PASS. Bootstrap 20 PASS/1 FAIL odhalil vlastný potvrdený reopen blokovaný recovery lockom. Opravené v document.rs/live_bridge.rs: iba po potvrdení discard sa pod držaným zámkom skontroluje a odstráni vlastný checkpoint; pri načítaní sa zámok neuvoľňuje a cudzie/nepotvrdené otvorenie zostáva odmietnuté. Bg65-18dac10df6cbb19c už potvrdil bootstrap 21 PASS a pokračuje GUI file_workflow → native_persistence → MCP lib → consent lib. Najbližšie vyčítať jeho výsledok, až po PASS uzavrieť #2382 a pokračovať #2383. Žiadny commit/push ani cudzie procesy.

### Overené R2 a R3 — 2026-10-02 17:50

- **R2 / CR-04, CR-05, CR-07, CR-08 PASS.** Bg63: kompletné application document_session 21 PASS a GUI recovery failure scenáre 7 PASS. Bg65-18dac10df6cbb19c: GUI file_workflow 68 PASS, live_bridge_bootstrap 21 PASS, model native_persistence 33 PASS. Pokryté dve sessions/okná, aktívny a opustený checkpoint, ochrana Save As aj potvrdený vlastný reopen, original bytes/required pri duplicate extension, source+overrides rollback s Undo/Redo/Save/Open. Published-error injekcia reálne prešla spoločnou publish/sync cestou pri editácii, Undo/Redo aj GUI Save As; natívny Unix fsync sa na Windows nevykonal. Podrobnosti a dve nájdené integračné regresie vyššie.
- **R3 / CR-13 PASS na zvolenej Windows bezpečnostnej hranici.** Bg65: MCP lib 11 PASS, consent lib 4 PASS, bootstrap integration 21 PASS. Windows jadro odmietlo read pod impersonovaným restricted tokenom, owner read uspeje, inherited ACL odmietnuté, private file roundtrip/bounds PASS. Missing/wrong bootstrap odmietnuté bez revokácie aktívneho klienta; správny reconnect rotuje credential. Nie je to vykonaný test druhého prihláseného účtu ani Unix permission test; nebol použitý živý OAuth alebo používateľovo okno.
- Ďalej #2384: overiť CR-06/CR-14, doplniť explicitné EOF počas pozastaveného plánovania, ak chýba. Plný záverečný build/test a audit zostávajú R8. Crony #68/#69 ponechané aktívne.

### R4 priebežne — 2026-10-02 17:54

Bg66-18dac14db8a1d6f4: program_check 3 PASS (nový skutočný TCP EOF počas pozastaveného plánovania, cancel/výmena dokumentu, normálny apply + Undo/Redo); group_ 3 PASS (cykly odmietnuté bez čiastočnej zmeny, forward refs stále fungujú); runaway program 1 PASS za 20.47 s vrátane úspešného ďalšieho vyhodnotenia rozmerov dielu. Posledný filter live_bridge::tests::program_tests:: trafil 0 testov a NIE JE PASS dôkaz. Spustený opravený filter live_bridge::tests::program_edit:: v bg67-18dac1642f5a06e8. Po jeho výsledku dokončiť R4, potom R5: odstrániť už nepoužívanú box_faces (iba definícia, žiadne volania), vykonať celú program integration sadu a application rule_program_review s exact oracle. Rozsah budgetu zostáva interpretovaný Starlark vrátane prelude, nie tvrdý sandbox ľubovoľného native volania.

### AKTUÁLNY HANDOFF — 2026-10-02 18:02

**#2382 a #2383 sú DONE; fokus #2384 zostáva ACTIVE. Crony #68/#69 overené ON, progress_check green. Všetky testové tasky tohto ticku skončili; nespúšťať ich duplicitne.** Bez commitu/pushu/stash a bez zásahov do cudzích procesov. R4 špecifické outcome regresie PASS, ale širšia relevantná sada ešte nie; cieľ preto nezatvorený.

- Bg67-18dac1642f5a06e8 (správny filter `cargo test --locked -p ketchup-app --lib live_bridge::tests::program_edit:: -- --test-threads=2`): **7 PASS/1 FAIL**, 89.07 s. Zlyhal `applied_program_answers_box_overlaps_with_the_exact_solids`: TCP/harness deadline 4 s v tests.rs:2222/2233 už pri normálnom príklade TABLE. `ai_reads_and_edits_the_window_program_in_one_call_each` prešiel, ale trval nad 60 s. Nie je to dôkaz chybného kolízneho výsledku; je to reálna výkonnostná regresia. Časové limity NEBOLI zvýšené.
- V rámci odblokovania relevantnej regresie urobené všeobecné pruning zmeny v `faces_boundary.rs` (celý prierez medzi cap planes nevyžaduje polygon boolean), `contact_polygon.rs` (oddelené segmentové obálky vynechajú intersection; cached obálky ringov vynechajú nemožné point-in-ring). Odstránená nepoužívaná `faces.rs::box_faces` po kontrole, že nemá volania. **Zlepšenie latencie celého TABLE zatiaľ nedokázané.**
- Bg69-18dac1a3feb33c2c a posledný bg70-18dac1c24c44b3f8: contact_review **5 PASS** (posledný 1.36 s), následný samostatný live test stále **FAIL** na pôvodnom 4 s limite. Žiadny 0-test alebo zvýšený timeout sa nepočíta ako oprava.
- Bg68-18dac16ce9b6f4f8: `python -m pytest -q tests sdk/python/tests` **298 PASS, 24 skipped, 71 subtests PASS**. Bolo pred poslednými pruning zmenami; required natívna produkčná sada sa ešte nespustila.
- Bg71-18dac1c715234ccc: check_crate_layers, check_error_hygiene, check_no_named_products, check_test_hooks **PASS**; check_tolerance_literals **FAIL**: local_auth.rs jeden unnamed size limit (`bytes.len()>512`), contact_polygon.rs 2 nové tolerance literals, faces_boundary.rs 12. Použiť spoločný TolerancePolicy/ketchup-tolerance a pomenované limity; NEZVYŠOVAŤ baseline ani nepridávať výnimky. Za zlyhaním `&&` sa git diff --check v tomto behu už nespustil. Posledný fmt vykonaný v bg70.
- **Presný ďalší krok:** zmerať rozdelenie času TABLE evaluate/face_frames/contact/report/planning (izolovaný proces, nie používateľovo GUI) a odstrániť dominantnú prácu; neoptimalizovať ďalej naslepo ani neopakovať iba neúspešný test. Relevantné zdroje: faces.rs::face_frames/trim_faces (hranice sa opakovane rátajú pri vrtoch), faces_boundary.rs::extrusion_section, contact_polygon.rs::boolean/contains, contact.rs::contact. Potom pôvodný live timeout test a celý program_edit modul PASS, opraviť tolerance ratchet, uzavrieť #2384 a v tom istom ťahu pokračovať #2385. Pre R5 vykonať celú program integration sadu + application `rule_program_review::` (CR12 a exact collision oracle), ktoré ešte nemajú kompletný finálny beh. R6/R7/R8 ostávajú otvorené; finálne odstránenie cronov až po všetkých overeniach.

### NAJNOVŠÍ CHECKPOINT — pokračovanie po handoffe 18:02

Pokračoval som meraním a opravil som tolerance ratchet, nie iba stavovou správou. **#2382/#2383 DONE; #2384 ACTIVE; celá vetva nie je dokončená.** Crony zostávajú ON. Žiadny commit/push, cudzie procesy ani používateľovo GUI.

- Izolovaný profil `.claude/review-2026-10-02/program_timing.rs` skompilovaný proti aktuálnemu debug ketchup-program rlib (bg72-18dac1f30f325e80, exit 0): TABLE evaluate **1.069 s**, face_frames top **274.77 ms**, každá noha **8.6–8.8 ms**, validate **4.183 s**, relations **1.514 s**. Je to profil po predchádzajúcich pruning zmenách, pred presunom konštánt. Najväčšia nameraná fáza je validate, nie Starlark slučka.
- Konkrétna cesta na pokračovanie: `validate.rs::holes` volá `part.face_frame(&hole.face)` v slučke pre každý vrt, čím opakuje celé `face_frames`; `validate.rs` ďalšie `contact()` volania v joints/support a `relations.rs` opakujú tú istú geometriu. Najskôr merať alebo zdieľať výpočet plôch v rámci jedného vyhodnotenia; nevypínať validáciu a nezvyšovať 4 s testový timeout. Predchádzajúci širší program_edit beh ostáva **7 PASS/1 timeout FAIL**. Profil je v ignored .claude, nie produkčný modul.
- Nové tolerancie presunuté do ketchup-tolerance (`BOUNDARY_CHORD_MM`, `BOUNDARY_MIN_ANGLE_RAD`, `BOUNDARY_PROBE_FRACTION`), rovinné porovnania používajú DEFAULT_LINEAR_TOLERANCE_MM a numerické ROUNDING. Bootstrap bounded read používa pomenovaný MAX_BOOTSTRAP_BYTES=512, stále číta limit+1. Žiadna výnimka či zvýšenie ratchet baseline. Nepoužívaná box_faces odstránená.
- **Posledné skutočné overenie po všetkých týchto zmenách:** cargo fmt --all PASS; contact_review **5 PASS (1.31 s)**; ketchup-mcp --lib **11 PASS**; `cargo check --locked -p ketchup-app --all-targets` **PASS**, vrátane testových targetov; check_tolerance_literals **PASS**; git diff --check **PASS**. Predchádzajúci chýbajúci ROUNDING import bol opravený plne kvalifikovanou cestou a tento posledný build ho overil. Ostatné architektonické/error/named-product/test-hook ratchety prešli pri bezprostredne predchádzajúcom behu. Python zostáva naposledy 298 PASS/24 skipped; nejde o finálny required native beh.
- **Ďalej:** odstrániť meranú výkonnostnú regresiu, vykonať pôvodný live test a celý program_edit modul, až potom zatvoriť R4; pokračovať R5 exact/geometrické regresie, R6/R7 a R8. Úplný Clippy/workspace/native production audit ešte neprebehol. Žiadny teraz bežiaci test netreba reštartovať.

### Pokračovanie 18:30 — R4 výkon: validate.rs počíta face_frames raz na diel s vrtmi. Profil bg74: evaluate 1.106 s, validate 2.341 s (predtým 4.183), relations 1.606 s; bg73 stále 7 PASS/1 deadline FAIL, preto cieľ ostáva otvorený. Následne contact.rs dostal lokálny ContactFaces cache pre nemenný model; joints/support vo validate.rs a relations.rs zdieľajú geometrické plochy v rámci svojho behu, bez persistentného cache či zmeny polygonového algoritmu. Ratchety crate_layers/error_hygiene/no_named_products/tolerance_literals a diff check PASS. Bg75 zastavený compile chýbajúcim importom standalone contact pre exact_relation; import opravený. **Beží bg76-18dac35b1c10b9bc**: app live program_edit → celá program integration → application rule_program_review. **Bg76 app program_edit: 8 PASS / 0 FAIL / 0 ignored za 18.95 s; pôvodný 4 s deadline nezvýšený. R4 / CR-06, CR-14 DONE** spolu s doloženými group_ 3 PASS, program_check 3 PASS (cancel/EOF/výmena dokumentu/UndoRedo) a runaway 1 PASS z bg66. Bg76 ďalej beží program integration → application rule_program_review; to je ďalší krok R5. Po spustení bg76 odstránené iba zbytočné into_iter a borrow v contact_with_faces (táto malá úprava bude overená nasledujúcou kompiláciou). Bez commitu/pushu a cudzieho GUI.

### R5 checkpoint 18:39 — Bg76 širšia program sada po 427 s stále na dvoch testoch skrine; ukončený výhradne vlastný task cez KillShell. Tento nedokončený beh NIE JE PASS (app 8 PASS ostáva platný). Profil po cache: TABLE evaluate 1.113 s / validate 1.179 s / relations 0.743 s. Ďalšie opravy výkonu novej boundary cesty: hole/pocket používajú machining_frame bez nevyužitého trimovania hraníc, zatiaľ čo verejný face_frame/contact ponechávajú hranice; polygon::boolean má sweep kandidátov hrán so zachovanými pôvodnými priesečníkovými vzorcami. Nový contact_review test overuje plochu po 16 vrtoch a rotácii proti analytickému výsledku. **Beží bg77-18dac3d805d052d4**: contact_review → faces → application rule_program_review exact → plná program integration. Bg77: contact_review 6 PASS, faces 10 PASS/2 FAIL. Jedna zastaraná asercia čakala 4 vrcholy aj po dvoch vrtoch; nahradená analytickou materiálovou plochou. Druhý fail potvrdil reálny CR10 problém: nová plocha odrezania mala rozsah celého nástroja (300×200 kontakt namiesto 100×30/cos20). faces.rs teraz pretína planar boundary nástroja s prierezom základného telesa; regression PASS. **Bg78-18dac3f537983e40: faces 12 PASS (0.61 s), contact_review 6 PASS (1.70 s), application rule_program_review 4 PASS (1.66 s, vrátane exact kolízie revolúcie pred/po rotácii a lineárneho/kvadratického/piecewise Push/Pull). Plná program integration stále beží; changing_joinery aj širšia skriňa už PASS, golden report test zatiaľ FAIL — vyčítať finálne detaily, porovnať zmeny reportov a neopravovať expected naslepo.** Ratchety/diff opäť PASS. Prečítať bg78; neduplikovať build. R5 otvorený, R6/R7/R8 čakajú. Bez commitu/pushu a zásahu do používateľových procesov.

### Výsledky R5–R7 — 19:05, všetky tri ciele PASS

R4 #2384 uzavretý; R5 #2385 ostáva aktívny. Bg78-18dac3f537983e40 dokončený exit101: plná program integration **112 PASS / 2 FAIL / 0 ignored za 191.69 s**. Fail 1: golden reporty iba `examples/programs/cabinet.report.json` a `table.report.json` sa líšia; ešte NEBOLI prepísané. Fail 2: relations test čakal plný štvorec 4900 mm² namiesto materiálu po dvoch Ø8 vrtoch (report 4800 mm²). Táto asercia už opravená na analytických 4900−2π·4² s <1 mm² toleranciou pre celočíselne zaokrúhlený/tesselovaný report. **Následný skutočný beh proti finálnemu kódu: relations:: 7 PASS (0.95 s), app live_bridge::tests::program_edit:: 8 PASS (7.59 s), fmt a diff check PASS.** Predtým na rovnakých funkčných úpravách faces 12 PASS, contact_review 6 PASS (nový 16-hole rotated area oracle), application rule_program_review 4 PASS vrátane exact worker kolízie revolúcie a 121 mm Push/Pull. Všetky tasky tohto ticku skončili; nespúšťať duplicitne. Bez commitu/pushu/stash a bez cudzieho GUI/procesov; zastavený bol iba vlastný bg76.

**19:00 — audit goldenov:** izolované kandidáty `.claude/review-2026-10-02/{table,cabinet}.candidate.json` vytvorené bg81 exit0. `git diff --no-index` ukázal table iba 4× area 4900→4800 (4900−2π·4²) a dve float odchýlky ~1e−14 v polohách; cabinet iba 4× area 6300→6128 (6300−4·18−2π·4², <1 mm² report zaokrúhlenie/tesselácia). Kusovník, spoje, rozmery a ostatné obrábanie nezmenené. Tieto konkrétne polia aktualizované cez Edit, nie slepé regenerate. Nový `report_material.rs` obsahuje analytický oracle cabinet. **bg82-18dac4df7579c284 exit0**: plná program integration **115 PASS / 0 FAIL / 0 ignored** za 200.96 s (všetky goldeny aj analytický cabinet oracle), následne application rule_program_review **4 PASS** za 1.63 s vrátane exact worker kolízie a lineárneho/kvadratického/piecewise Push/Pull. Po skončení sprísnený contact_review proti finálnemu testu **6 PASS** za 1.98 s: triangle/concavity/void po rotácii a posune, počty a zhodné svetové polohy oboch párov vrtov + okrajová rezerva. **R5 / CR-09, CR-10, CR-11, CR-12 PASS**, bez oslabenia validácie alebo timeoutov. **R6 / CR-02 a CR-03 PASS:** bg83-18dac524fa295090 exit0, memory lifecycle **1 PASS** (0.46 s) s Weak/Arc dôkazom skutočného uvoľnenia evikovaných a opustených redo produktov, zachovanou exact geometriou Undo/Redo a uvoľnením po discard_history; adapter selector **2 PASS** (0/1/2 GPU, preferencia dostupnej discrete, surface filter a explicitná jedinečná diagnostická požiadavka). Testované headless, žiadna HW certifikácia ani používateľovo okno. **R7 / CR-15, CR-16, CR-17 PASS:** bg84-18dac53dc901a56c exit0: úplná MCP lib sada **11 PASS**, app optional_expected **1 PASS**. Stale guard po výmene dokumentu odmieta všetkých 6 dotknutých požiadaviek bez zmeny scény/histórie/kamery; absent/current expected naďalej funguje. Dve izolované readiness streams v opačnom poradí vracajú vlastnú identitu a endpoint, open_window nepoužíva discovery race. Čítanie overené limit+1, 1 GiB file, UTF-8, Unicode/escaping, finálny envelope limit a ďalší platný send po odmietnutí; Windows NUL odmietnutý. Unix device/socket/symlink vetvy sú cfg(unix), na Windows neboli vykonané (platformový limit, nie PASS). Nasleduje R8: plná sada, Clippy, native runner a finálny register. Bez commitu/pushu/stash/cudzieho GUI; crony #68/#69 ponechané ON do dokončenia vetvy.

### Záverečné overenie — 20:06, R8 PASS

Historický checkpoint pred záverečným behom: R1–R7 #2381–#2387 DONE; register CR-01–CR-17 aktualizovaný na cielené PASS s explicitnými platformovými limitmi. R8 následne našiel a opravil tri Clippy chyby: `ProgramCheckJob::Checking(Box<ExactCheckJob>)`, adapter_tests presunutý na koniec exact.rs a needless borrow v exact_modeling.rs. **Bg89-18dac590a44e4a38 exit0:** `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` PASS (17.56 s), program_check::tests **3 PASS**, adapter_tests **2 PASS**, exact_history_releases **1 PASS** proti finálnym zmenám. **Bg86-18dac551ef5e0494 exit0:** všetkých 8 skriptov check_*.py PASS, Python `tests sdk/python/tests` **298 PASS / 24 skipped / 71 subtests PASS**, fmt/diff PASS. Po Clippy opravách opäť všetkých 8 statických kontrol, fmt a diff PASS. Skipped Python sa NEPOČÍTA ako native produkčný PASS.

**R8 native chyba opravená a overená:** bg88 exit1 odhalil falošnú podporu dielu nad priechodným otvorom. `collision.rs` vydával plochu obalov vyrezaných dielov za exact kontakt. Nové `collision_hull::measure` ponecháva analytické odmietnutie oddelených obalov, ale pri dotyku vyrezaných dielov vyžiada OCCT. Funkčný native oracle ostal zachovaný; limit `collision_report` sprísnený 808→805. Ďalší beh odhalil staré očakávanie bez podlahy: git blame potvrdil úmyselnú zmenu 0fdfc322 (24.9.); test teraz overuje floor PASS a zdvihnutej neukotvenej zostave not_evaluated. Plná Python sada odhalila starý 16 mm vrt v objemovom oracle; ca229989 (28.9.) stanovuje 12 mm v 18 mm doske s 6 mm rest, nezávislý analytický výsledok 4504264.071534864 mm³ zhodný s OCCT 4504264.0715348. Test opravený na (18−6), tolerancia nezvýšená. **Bg92-18dac66c7ead8d48 exit0:** finálny `python scripts/run_production_tests.py` **12 PASS / 0 skips**, handshake a release manual-alpha startup/SaveOpen/OAuth capability PASS; následný `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` PASS. **Bg94-18dac6a8312524dc exit0:** `python -m pytest -q -ra tests sdk/python/tests` s explicitnými čerstvými KETCHUP_HEADLESS/KETCHUP_EXACT_WORKER **322 PASS / 0 skips / 71 subtests PASS** (95.37 s). Pôvodné 24 Python skips tým odstránené. Native warning iba explicitne pomenúva limit topology/volume oracle, netvrdí nezávislý mesh genus dôkaz. Cielená finálna `collision_brep::` **18 PASS**, všetkých 8 check skriptov/fmt/diff PASS. Prostredie všetkých native behov: OCCT `third_party/occt-install-r0-v1` + bin pred PATH, izolovaný **CARGO_TARGET_DIR=target/cr-20261002**, JOBS=4, DEV_DEBUG=0, RELEASE_DEBUG=0, INCREMENTAL=0. Žiadny cudzí proces ukončený.

**FINÁLNY PASS — bg95-18dac7738008a42c exit0**, log `.claude/bg/20261002_194500_bg95-18dac7738008a42c.log`: `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` → celý `cargo test --locked --workspace --all-targets --no-fail-fast -- --test-threads=2` proti finálnemu kódu v izolovanom targete bez súbežných build/test záťaží. Súčet top-level targetov **2566 PASS / 0 FAIL / 5 ignored**; tri vnorené subprocess úspechy sa NEPRIPOČÍTAVAJÚ druhýkrát. Ignored: dva live OAuth scenáre, voliteľný CPU benchmark s externým 140-body modelom, Blender source-parity fixture a `request_timeout_tests::sleeping_worker` (pomocný subprocess, nadradené timeout testy PASS). Žiadna požadovaná CR regresia nebola skipnutá; zero-test targety ani nulové doctesty sa nevydávajú za regresný dôkaz. App lib481, GUI503, application lib35+integration225, program lib5+integration115, model integration561, scheduler integration116 PASS. Predchádzajúci bg91 exit101 mal jediný 5 s timeout v `mesh_conversion::background_task_commits_on_the_monotonic_revision_after_undo_branch`, prekrytý so 140-body collision testom; izolovaná mesh sada8 PASS/5.22 s. Obom prekrývajúcim sa testom doplnený existujúci `integration_support::file_turn()` podľa AGENTS.md, žiadny produkčný timeout ani asercia nemená; celý finálny beh vrátane tohto testu PASS. Nezávislý read-only audit `.claude/tasks/da6b6608_result.txt`: bez blokera v contact fix/test contract/register audite, explicitné platformové limity zachované. Headless rectangle/PushPull/SaveOpen a príklady PASS v celej sade; cabinet native SaveOpen/BOM PASS v Python. Skutočný BOM export zo SDK uložený a spätne porovnaný v `target/cr-20261002/bom-smoke-er5rv4rk/cabinet.bom.json` (7 dielov,8 kolíkov). Bez commitu/pushu/stash/cudzích procesov či modelov; testový `examples/programs/table.ketchup.work-recovery-lock` necommitovaný, neodstraňovaný naslepo. Po tomto PASS sú #2388/#2380 DONE, crony #68/#69 odstránené a cron_list potvrdil „No cron jobs“. Systém automaticky nastavil iný focus, ale žiadna práca na nesúvisiacich cieľoch sa nezačala.

## 7. Rozsah a limity

Review prešiel kritické tokové cesty model/application/program/app/MCP/SDK a vybrané export/kernel časti, existujúce testy a CI. Presné pokrytie každého pomocného review je v prílohe. Native OCCT sweep/shell boli čítané vzorkovo, nie formálne verifikované. PDM, analysis, všetky import formáty, kompletné legacy migrácie, celá topologická numerika a všetky assembly solver vetvy neprešli úplným ručným auditom. Testy týchto crates nenahrádzajú taký audit.

Externý privátny OAuth provider a lokálne ignorované Supervisor skills nie sú súčasťou tohto HEAD; ich bezpečnosť sa tu nepotvrdzuje. Bez testu na multi-user/multi-GPU stroji, bez Unix fsync fault injection, bez CNC obrábania a bez neobmedzeného programu v živom GUI. Report rozlišuje tieto hranice pri jednotlivých nálezoch.

## 8. Priebežný denník

1. Čistý baseline, prečítané AGENTS.md, založený hlavný Markdown pred analýzou.
2. Tri nezávislé čítacie review s vlastnými checkpointmi; hlavný reviewer GUI/exporty/CI.
3. Zapísané prvé ratchet zlyhania a exact cache; zistený worker file lock bez zabíjania cudzích procesov.
4. Zelené knižničné testy a Clippy. Overené nálezy v konkrétnych zdrojoch, nie len prevzatý záver pomocných agentov.
5. Osem izolovaných reprodukcií: recovery, duplicate extension, revolúcia, kontakt, kolíky, source-only rollback, cycle hang, nelineárny Push/Pull.
6. Integračný beh dokončený: 1664 passed/1 failed/4 ignored, jediné zlyhanie vysvetlené v CR-01. Dokončený register 17 nálezov a návrh opráv. Záverečný `git diff --exit-code` prešiel, `git status` uvádza iba šesť nových review Markdownov; oba stashe zachované. Žiadna oprava kódu, commit ani push.
