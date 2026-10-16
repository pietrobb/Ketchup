# Nezávislý review ketchup-program — 2026-10-02

## Zadanie a stav

- Baseline aj overený HEAD: `e92f4a73844b185570dfbb49a807ab2001ecd845`.
- Rozsah: ketchup-program (evaluator, inkrementálny model, identita, operácie/joints, library), s nevyhnutným trasovaním jeho planner/reconcile integrácie v ketchup-application. Jedno volanie v ketchup-app bolo dohľadané iba na potvrdenie dosiahnuteľnosti programového rewrite.
- HMOS skontrolovaný read-only cez `C:/Sources8/Ketchup/.claude/hmos.db`; koreňový `C:/Sources8/Ketchup/AGENTS.md` prečítaný. V crates sa nenašiel ďalší AGENTS.md.
- Tento MD vznikol na začiatku a priebežne sa aktualizoval. Hlavný review MD ani zdrojové súbory sa nemenia.
- Bez opráv, Cargo test/build, commit/push/stash alebo GUI vstupu.
- Výsledok: **4 staticky potvrdené nálezy; 0 runtime reprodukcií Rust/Starlark/OCCT**. Číselné príklady pre PRG-01/03/04 sú matematické dôkazy; nezamieňať ich s vykonaním produktu.

## PRG-01 — P1: Obálka revolúcie ignoruje jej os a môže vyradiť skutočnú kolíziu z exact kandidátov

**Miesto:** `C:/Sources8/Ketchup/crates/ketchup-program/src/model.rs:992-999`, najmä `994-997`; tvorba rozmerov `C:/Sources8/Ketchup/crates/ketchup-program/src/eval.rs:854-857`.

**Scenár:** verejne podporovaný `revolve` okolo osi X, nie okolo Y cez nulu:

```python
p = revolve("tube", profile=[(0,10),(100,10),(100,20),(0,20)], axis=[(0,0),(1,0)])
b = box("inside_wall", (5,2,2), at=(40,-19,-1))
print(part_info(p).local_min, part_info(p).local_max)
print(reach(p, (0,-1,0)))
```

**Dôkaz:** evaluator nastaví radius z absolútnej X súradnice profilu, teda 100. `body_bounds` vždy vyrobí `[-radius,min_y,-radius] .. [radius,max_y,radius]`, v tomto prípade **(-100,10,-100)..(100,20,100)**. Zadanú os nepoužije. Skutočná plná revolúcia je rúra dĺžky 100 s polomermi 10 a 20 okolo X: **(0,-20,-20)..(100,20,20)**. Obálka teda nie je len voľná aproximácia; časť telesa vôbec neobsahuje. `reach` pre túto os použije `bounds_reach` (`model.rs:1124-1143`) a smer -Y vráti -10 namiesto 20.

`inside_wall` leží v materiáli rúry (radiálna vzdialenosť 17 až približne 19.03 mm), ale jeho Y interval -19..-17 je mimo vypočítanej obálky 10..20. `C:/Sources8/Ketchup/crates/ketchup-program/src/exact.rs:104-126` filtruje kandidátov cez `world_bounds` a OBB separation, takže tento pár v samotnom `exact_candidates` vyradí. Rovnako sú nesprávne placement helpery používajúce `reach`.

**Prečo nejde o chýbajúcu feature:** `revolve` prijíma všeobecnú os v rovine (`eval.rs:1353-1398`), `cad.rs:92-105` ju odovzdá kanonickej revolúcii a `faces.rs:393-488` pre ňu dokonca počíta face frames. Chybná je súčasná obálka podporovaného telesa.

**Overenie:** statické trasovanie a analytické rozmery; produkt nespustený. Netvrdím runtime výsledok celého GUI/native collision workflow.

## PRG-02 — P2: contact() vydáva prázdny priestor nad profilom za kontaktnú plochu

**Miesto:** `C:/Sources8/Ketchup/crates/ketchup-program/src/contact.rs:63-69`; zdroj chybnej hranice `C:/Sources8/Ketchup/crates/ketchup-program/src/faces.rs:336-341`.

**Scenár:** trojuholníkový hranol a kváder nad prázdnym rohom jeho obálky:

```python
a = extrude("triangle", profile=[(0,0),(100,0),(0,100)], distance=30)
b = box("corner", (10,10,30), at=(80,80,30))
print(contact(a,b))
```

**Dôkaz:** uzáver `end` trojuholníka dostane rozsah `(0,0)..(100,100)` z `profile_bounds`; jeho skutočný obrys x+y<=100 sa do `FaceFrame` neprenesie. `contact` zoberie rohy tohto obdĺžnika a oreže ich iba obdĺžnikom druhej face a oboma OBB. Výsledok je štvorcová patch **(80,80,30)..(90,90,30), plocha 100 mm²**, s protiľahlými normálami uzáverov. Všetky jej body však majú x+y>=160 a neležia na prvom telese. Najbližší bod hranola je od rohu (80,80,30) vzdialený približne 42.43 mm.

**Dopad:** builtin v `eval.rs:2089-2129` bez ďalšieho overenia vráti kontakt ako skutočný; library `dowels` na ňom priamo zakladá obrábanie (`prelude.star:820-837`). Neskoršie exact spresnenie reportu nemení už vyhodnotené vetvenie programu ani umiestnené otvory. Rovnaký nesúlad zasahuje výber face podľa bodu, ktorý tiež pracuje s týmito rámami; nález však stojí na konkrétnom `contact` scenári.

**Kontrakt:** `C:/Sources8/Ketchup/crates/ketchup-program/library/prelude.star:164-169` sľubuje patch skutočne priliehajúcich plochých faces pre všeobecné extrusions/revolves. Nejde o dokumentovanú OBB aproximáciu funkcie `distance`.

**Overenie:** staticky potvrdené; uvedený program je pripravený reproduktor, nie vykonaný test.

## PRG-03 — P2: dowels() ignoruje kontaktný polygón a vŕta mimo otočenej dosky

**Miesto:** `C:/Sources8/Ketchup/crates/ketchup-program/library/prelude.star:824-836`, najmä `832-833`.

**Scenár:** aj pri dvoch obyčajných kvádroch, kde je samotný kontakt správny:

```python
a = box("base", (300,300,30))
b = box("top", (200,20,30), at=(50,100,30))
rotate(b, axis=(0,0,1), angle=45, pivot=(150,110,0))
dowels(a,b,dowel="6x30",count=2,margin=10)
```

**Dôkaz:** patch je otočený obdĺžnik 200x20. `contact` poskytuje jeho správne rohy `points` a obalový obdĺžnik `origin/size` (`contact.rs:95-106`). Library však `points` vôbec nepoužije: zoberie dlhšiu stranu obalu a jeho stredovú čiaru. Obal má rozmer približne 155.563x155.563; pri voľbe radu podľa X vzniknú svetové body **(82.218254,110,30)** a **(217.781746,110,30)**. V lokálnom rámci hornej dosky sú to **(52.071068,57.928932,0)** a **(147.928932,-37.928932,0)**. Platný lokálny Y interval je len 0..20. Ak zaokrúhlenie rozhodne remízu opačne a vyberie Y rad, zrkadlovo nastane rovnaký problém.

Dĺžka kolíka ani hrúbka nie sú prekážkou: obe dosky majú 30 mm, `6x30` potrebuje po 16.5 mm s predvolenou vôľou, čo sa zmestí. Existuje dostatok miesta pre dva správne otvory pozdĺž reálnej dlhej osi patch. `hole` body iba prevedie do face súradníc (`eval.rs:2003-2025`); validator až dodatočne oznámi `hole_outside_face` (`validate.rs:354-387`). Helper teda sám vyrobí zlú geometriu z platnej zostavy, namiesto deklarovaného páru zhodných otvorov.

**Odlišnosť od PRG-02:** tu sú dve box faces a kontakt je presný; chyba vzniká až stratou polygonálnej hranice v library. Oprava všeobecného contact clippingu sama tento prípad nevyrieši.

**Overenie:** staticky potvrdené; nezávislá aritmetika rotácie v Python REPL potvrdila uvedené súradnice. Starlark/Rust produkt nespustený.

## PRG-04 — P2: Programový Push/Pull pri nelineárnom parametri potichu použije nesprávnu vzdialenosť

**Miesto integrácie programu:** `C:/Sources8/Ketchup/crates/ketchup-application/src/rule_program.rs:135-148`, následné overenie iba vyhodnotením na `160`.

**Scenár:** program so štvorcovou závislosťou výšky:

```python
H = param("H", 10.0)
p = extrude("part", profile=[(0,0),(20,0),(20,20),(0,20)], distance=H*H)
```

Zavolať `rewrite_rule_program_push_pull(source, "part", "end", 21.0)` — pôvodná výška je 100 mm, požadovaná výška 121 mm.

**Dôkaz:** `controlled_value` číta extrúznu vzdialenosť. Kód urobí jednu sondu parametra o 0.002, dostane lokálny sklon približne 20.002 a rovno nastaví `H = 10 + 21/20.002 = 11.049895...`. Nová výška je **122.100180 mm**, t. j. pohyb **22.100180 mm namiesto 21 mm**. Jediné záverečné `evaluate(&rewritten)?` overí, že program možno vyhodnotiť; vôbec neporovná výslednú vzdialenosť s požadovanou. Nedochádza k ďalšiemu riešeniu ani k part-local fallbacku, pretože existuje práve jeden driver.

**Dopad/dosiahnuteľnosť:** túto verejnú aplikačnú cestu používa programový Push/Pull (`C:/Sources8/Ketchup/crates/ketchup-app/src/planar_push_pull.rs:213-218`). Nelineárne výrazy sú normálne platný Starlark, nie nepodporovaná geometrická feature. Rovnaký mechanizmus navyše nerozlišuje, či driver používa aj iný diel, ale tento nález sa neopiera o potenciálne historicky meniaci sa UX kontrakt: merateľne zlá cieľová vzdialenosť stačí sama osebe.

**Overenie:** staticky potvrdené a číselne prepočítané v Python REPL; bez Rust testu alebo GUI reprodukcie.

## Pokrytie a hranice

### Preskúmané

- Evaluator: vstupné čísla/profily, pomenovanie dielov a operácií, tvorba a kopírovanie partov/tools, transformácie, holes/pockets, boolean a union, joints, parametre, vyhodnotenie a source mapping.
- `document.rs`: diff podľa názvu a celého `Part`, no-op/failure pred publikovaním, truncation redo a limit undo, opätovná evaluácia pri undo/redo.
- `model.rs` a `cad.rs`: feature parametre, štruktúrne identity, telo/operácie a ich lowering, obálky/reach, face labels.
- Programová integrácia v `rule_program.rs`: source-only/incremental/replacement rozhodovanie, name lookup a transform guards, feature-param guards, rebuild/repoint existujúcej occurrence, odstránenie/pridanie partov. `planner.rs:3821-3896` a relevantné časti `rule_operations.rs`: lowering bázy a poradie operácií.
- Manuálne zmeny: replacement confirmation gate a existujúci test detachment/Save/Open v `C:/Sources8/Ketchup/crates/ketchup-program/tests/document.rs:238-288` iba prečítané. Nepotvrdzujem tým kompletnú ochranu vo všetkých aplikačných cestách.
- Geometrické kontrakty library: placement, profily, machining helpery, dowels/hinge a expect helpers; súvisiace `faces.rs`, `contact.rs`, výbery v `validate.rs` a `exact.rs`.
- Existujúce testy boli použité na pochopenie kontraktov, nie ako novo namerané zelené výsledky. Napríklad `tests/program.rs:825-865` kontroluje polygonálny kontakt otočených boxov, ale uvedený problém dowels tým nie je pokrytý.

### Limity a vylúčené témy

- Žiadny Cargo test/build, OCCT worker ani GUI sa nespúšťal. Existujúci `target/debug/ketchup-program.exe` bol len lokalizovaný, **nebol vykonaný** a jeho príslušnosť k baseline sa nepredpokladá.
- Analytické/Python číselné kontroly nie sú runtime reprodukciou produktu. Všetky štyri nálezy ostávajú označené ako **staticky potvrdené**.
- Bez úplného auditu sweep transportu, numerickej robustnosti všetkých profilov a topologického kernelu. Bez všeobecného auditu ketchup-model/persistence/protocol mimo nevyhnutných kontraktov programu.
- Ako nálezy sa neuvádzajú explicitné obmedzenia named faces pre sweep/loft, známe obmedzenia operácií na boolean tooloch, odmietnutie nepodporovaného inkrementálneho update, ani len štýlové či error-hygiene návrhy.
- V preverenej inkrementálnej identite/undo ceste nebol izolovaný ďalší dostatočne preukázaný bug; nie je to tvrdenie o jej bezchybnosti.

## Checkpoint opráv CR-09 až CR-12

2026-10-02: Samostatný implementačný workstream začal. HMOS overený read-only; AGENTS.md a všetky štyri review prílohy prečítané. Potvrdené aktuálne mechanizmy: os ignorovaná v revolve bounds, obdĺžniky namiesto polygonálneho kontaktu, AABB rad vrtov, neoverená linearizácia Push/Pull. Rozsah iba ketchup-program (bez eval.rs), library/prelude.star, ketchup-application/rule_program.rs a ich testy. Cudzie zmeny ostávajú nedotknuté. Nasleduje implementácia a cielené outcome regresie; zatiaľ žiadny nový PASS.

### Implementácia a prvý validačný pokus

- CR-09: `crates/ketchup-program/src/model.rs` teraz počíta analytický support revolúcie z profilu, skutočnej osi, jej posunu a uhla; bounds, reach aj rozmery revolúcie používajú tento support. Nové `tests/revolve_bounds.rs`: pôvodný X-axis reproduktor vrátane collision candidate, neobdĺžnikový profil okolo posunutej šikmej osi, čiastočný uhol, rotácia a posun, nezávislé vzorkované maximum.
- CR-12: `crates/ketchup-application/src/rule_program.rs` prijíma parametrický návrh iba pri dosiahnutí cieľového rozmeru v ACCUMULATED_ROUNDING; inak obnoví pôvodné overrides a použije existujúci part-local push_pull. Neplatná skúšobná parameter sonda neblokuje fallback.
- Nové `crates/ketchup-application/tests/rule_program_review.rs`: lineárny driver, kvadratická reprodukcia 121 mm, opakovaný lokálny posun, clamp/piecewise a hranice parametra; zachovanie druhého dielu pri fallbacku. Tiež exact kolízny oracle pre dolnú stenu revolúcie pred/po spoločnej rotácii a posune. Oba nové testové moduly registrované v príslušných integration.rs.
- CR-10 a CR-11 **zatiaľ neimplementované**; polygonálny kontakt s konkávnosťou/otvormi a bezpečný rad vrtov ostávajú ďalšiemu pokračovaniu. Bez zmien faces.rs, contact.rs, prelude.star alebo eval.rs.
- Cielené rustfmt a git diff --check PASS. Prvý Cargo pokus `bg30-18dabee424e3604c` exit 101 pred kompiláciou: duplicitná `[target.'cfg(unix)'.dependencies]` v cudzom `crates/ketchup-mcp/Cargo.toml:21`. Žiadny test týmto behom neprešiel. Manifest sa v tomto workstreame nemení; nasleduje opakovanie cielených testov.

### Odovzdanie implementačného workstreamu — čiastočné, neoverené Cargo

Opakovaný testovací beh `bg32-18dabef5fe118914` skončil rovnakým exit 101 na duplicitnej MCP manifest sekcii, pred kompiláciou. Cielený Clippy skončil na tom istom blockeri. `rustfmt --edition 2024 --config skip_children=true --check` pre všetkých šesť zmenených/nových Rust súborov a `git diff --check` PASS. Žiadny Cargo test sa nespustil: **CR-09/12 nie sú označené za overene uzavreté; CR-10/11 ostávajú otvorené.**

Po oprave manifestu jeho vlastníkom spustiť z C:/Sources8/Ketchup s KETCHUP_OCCT_ROOT a OCCT bin v PATH:

1. `cargo test --locked -p ketchup-program --test integration revolve_bounds:: -- --test-threads=1`
2. `cargo test --locked -p ketchup-application --test integration rule_program_review:: -- --test-threads=1`
3. `cargo clippy --locked -p ketchup-program -p ketchup-application --tests -- -D warnings`
4. Existujúce program/application geometrické regresie; potom pokračovať CR-10/11.

Presné súbory tohto workstreamu: `crates/ketchup-program/src/model.rs`, `crates/ketchup-program/tests/revolve_bounds.rs`, `crates/ketchup-program/tests/integration.rs`, `crates/ketchup-application/src/rule_program.rs`, `crates/ketchup-application/tests/rule_program_review.rs`, `crates/ketchup-application/tests/integration.rs` a tento checkpoint. Žiadny commit/push/stash, zásah do cudzích procesov či GUI. Bez zmeny eval.rs, persistence a GUI/MCP. Posledné dve kapitoly pôvodného review nižšie opisujú historický review, nie stav následnej implementácie.

### Pokračovanie geometrického workstreamu

- CR-09: `cargo test --locked -p ketchup-program --test integration revolve_bounds:: -- --test-threads=1` PASS, 2 testy (bg41).
- CR-10/11: implementované boundary rings, polygonálny prienik/difference, rezy extrúznych nástrojov, a library rad podľa skutočných hrán s kontrolou celého priemeru vrtu. Bez zmien eval.rs či budget súborov. Nové contact_review regresie pokrývajú trojuholník, konkávnosť, oddelené oblasti, dieru, rotácie a margin. Validácia ešte prebieha.
- Prvý application pokus bg44 zasiahol rozpracovanú zmenu kontaktu (chýbajúci module polygon v okamihu kompilácie); nejde o cudzí blocker, po dokončení súborov sa opakuje.
- Nasledujúci pokus celej program integration, application `rule_program` a Clippy zablokovaný E0428 mimo rozsahu: `crates/ketchup-geometry/src/linalg.rs` duplicitné dot2 (184/259) a cross2 (189/266). Súbor nemením, validáciu opakujem. Vlastná skoršia chyba distance2 je opravená delegovaním na existujúcu 3D distance.

### Odovzdanie pokračovania — implementované CR-10/11, Cargo blokované závislosťou

**Nie je to zelené uzavretie CR-09–12.** Jediný nový vykonaný PASS v tomto pokračovaní je `revolve_bounds::` (2/2, bg41), ešte pred doplnením kontaktov. Päť nových `contact_review::` testov a application `rule_program_review::` zatiaľ neprešli kompiláciou/vykonaním. Celý program integration, application filter `rule_program` aj targeted Clippy sa opakovane zastavili pred testami na E0428: duplicitné `dot2` a `cross2` v cudzom `C:/Sources8/Ketchup/crates/ketchup-geometry/src/linalg.rs` (184/259 a 189/266). Tento súbor ostal nedotknutý. MCP manifest ani local_auth už neboli blockerom. Posledná rozšírená rustfmt kontrola odhalila len formátovanie súbežne upraveného model.rs; nasleduje cielený rustfmt a diff check.

Implementácia CR-10: face boundary rings, polygonálny intersection/difference bez konvexného obalu; konkávnosť, otvory a oddelené komponenty; geometrické rezy extrúznych nástrojov pre holes/pockets a subtract; prenesenie hranice rotáciou/zrkadlením a pri sledovaní push/pull. `contact().points` zachováva API pomocou obojsmerných spojníc s nulovou plochou medzi prstencami. Curves majú chord sampling 0.001 mm; nejde o exact OCCT kontakt. Sweep/loft sa už nevydávajú za plné OBB kontakty. Existujúce obmedzenia modelovania orezaných faces po všeobecných CSG/shell/cut/finish operáciách nie sú týmto workstreamom univerzálne vyriešené; rezy nástrojov sú implementované pre extrúzne telá, nie všetky druhy CSG.

Implementácia CR-11: rad sa vyberá podľa reálnych hrán/polygonálnych intervalov, margin je vzdialenosť stredu vrtu od konca radu, celý priemer musí ostať mimo skutočných hrán vrátane otvorov. Nevyhovujúca geometria/count/margin sa odmietne pred vrtmi. Výber radu je bezpečný kandidátny algoritmus, nie optimalizátor garantujúci nájdenie každého možného rozmiestnenia. Regresie: triangle empty corner a plocha 5000; konkávny obrys a dve oddelené oblasti po rigid transform; priechodný kruhový otvor a odpočítaná plocha; otočené paired bores lokálne (10,10)/(190,10) aj po 3D transform; vyhnutie sa konkávnosti/otvoru a odmietnutie nemožného marginu.

Nové/zmenené v pokračovaní (absolútne cesty):
- `C:/Sources8/Ketchup/crates/ketchup-program/src/contact.rs`
- `C:/Sources8/Ketchup/crates/ketchup-program/src/contact_polygon.rs` (nový)
- `C:/Sources8/Ketchup/crates/ketchup-program/src/faces.rs`
- `C:/Sources8/Ketchup/crates/ketchup-program/src/faces_boundary.rs` (nový)
- `C:/Sources8/Ketchup/crates/ketchup-program/library/prelude.star`
- `C:/Sources8/Ketchup/crates/ketchup-program/tests/contact_review.rs` (nový)
- `C:/Sources8/Ketchup/crates/ketchup-program/tests/integration.rs`
- `C:/Sources8/Ketchup/crates/ketchup-program/src/model.rs` (iba záverečné formátovanie zdedenej opravy)
- `C:/Sources8/Ketchup/docs/code-review-2026-10-02-program.md`

Zdedene pripravené a zachované: `C:/Sources8/Ketchup/crates/ketchup-program/tests/revolve_bounds.rs`, `C:/Sources8/Ketchup/crates/ketchup-application/src/rule_program.rs`, `C:/Sources8/Ketchup/crates/ketchup-application/tests/rule_program_review.rs`, `C:/Sources8/Ketchup/crates/ketchup-application/tests/integration.rs`.

Po odstránení E0428 vlastníkom spustiť s OCCT env z tohto checkpointu:
1. `cargo test --locked -p ketchup-program --test integration -- --test-threads=1` (zahŕňa contact_review, revolve_bounds aj existujúce faces/program/examples).
2. `cargo test --locked -p ketchup-application --test integration rule_program -- --test-threads=1` (zahŕňa review exact oracle aj existujúce rule_program geometrické testy).
3. `cargo clippy --locked -p ketchup-program -p ketchup-application --tests -- -D warnings`.

Bez commit/push/stash, nového targetu, GUI, zmien MCP/persistence/eval.rs/execution_budget.rs/lib.rs alebo zásahu do cudzích procesov. Plná workspace validácia ani manuálny smoke test neboli vykonané.

## Odovzdanie

Jediný súbor vytvorený/menený týmto review: `C:/Sources8/Ketchup/docs/code-review-2026-10-02-program.md`. Hlavný reviewer môže použiť štyri scenáre vyššie na cielené runtime regresie vo svojej riadenej Cargo validácii. Review neobsahuje opravy.
