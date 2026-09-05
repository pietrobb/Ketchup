# Supervisor priamo v Kečupe: od celku k detailu

Dátum auditu: 2026-09-05. Zadanie operátora: najprv umožniť plnému Supervisorovi pracovať v Kečupe cez vlastné skills; obmedzenejší vstavaný asistent vznikne až z overeného riešenia. Hlavný cieľ HMOS: #1826, postup #1827–#1833. Stav po prvom prírastku: S1/S2 dokončené; application/headless/Python poskytujú summary/query/detail a kompaktné odpovede. Reálny protokolový test vytvoril 10 000 opakovaných výskytov, prešiel 100 stránok bez duplicít, prehľad 718 B a najväčšia odpoveď 15 880 B; vytvorenie približne 0,91 s, čítanie 0,38 s. Šesť skills v `skills/ketchup_model.py` prešlo tool.call cyklom create/query/edit/Undo/Redo/exact/Save/Open; celkovo 27 Python testov + 8 podtestov a 23 Rust testov. S3 runtime overený po reštarte: všetkých šesť registrovaných tools vykonalo izolovaný create/search/detail/set_dimension/Undo/Redo/Save/Open/exact cyklus; Discover hlási plan_guard_bound=true. Dôkaz `artifacts/supervisor-runtime-4e8d737f.ketchup`, dokument 1788634074155950701, revízia 2, digest f0124a39ae3a5a20: rozmery 120×80×55 mm, OCCT BRep objem 528000 mm³, complete aj topology_complete=true. Zastaraný zápis odmietnutý; oba vlastnené procesy bezpečne zatvorené. Kolízie pre jediné teleso passed; gravity_support not_evaluated pre chýbajúce podpery, teda nie dôkaz statiky. Aktuálne bezpečnostné testy 12 passed/1 skipped; aktívny plan-mode zákaz overený testom, v živom engine potvrdené viazanie (bez prepínania režimu). S4a / #1835 je implementované a nezávisle overené: opt-in autentifikovaný TCP most mení ten istý GUI DocumentStore cez ui(), nie druhú session (`crates/ketchup-app/src/live_bridge.rs:185`, `:222`). Protokol v1 používa 4-bajtovú BE dĺžku a najviac 32768 B JSON; token na každej správe, explicitný stamp document/revision/digest/mutation_epoch, návrh→commit, ohraničené query/detail, GUI Undo/Redo, výber/pohľad a bezpečné odpojenie. Epoch v `crates/ketchup-core/src/document.rs:3859` zachytáva aj ABA cez Undo/Redo. Opravené a testované busy/read-only ochrany, skupiny/skryté výskyty, idle/partial-frame/write deadlines a zrušenie pipelined zápisu. Nezávisle prešlo 7 bridge unit + 4 TCP/headless GUI testy, 7 spoločných query testov, 172 Python testov + 8 podtestov (6 integračných testov preskočených) a cargo check bežnej aplikácie s default OAuth. Python `LiveSession` v `sdk/python/ketchup/live.py:179` je nevlastniaci klient s explicitnými stamps, bez retry a bez ukončovania aplikácie; jeho sieťové testy používajú testovací server, nie Rust–Python end-to-end dôkaz. S4b / #1836 implementované a kontraktovo overené: `crates/ketchup-app/src/main.rs:10` podporuje explicitné `--supervisor-live-stdin` s voliteľným absolútnym dokumentom; `live_bridge/bootstrap.rs:44` prijíma najviac 1024 B a má dvojsekundové deadlines vstupu/readiness. Náhodný token ide iba súkromnou stdin rúrou, nie argv/env/logom/súborom; stdout obsahuje len verziu a loopback adresu. `skills/ketchup_live.py:303` registruje štyri samostatné live tools (Session/Inspect/Edit/View), pôvodných šesť offline tools ostalo nezmenených. Launch otvorí NOVÉ okno bežnej aplikácie a LiveSession nevlastní jeho životný cyklus; nepripája už otvorené okno. Explicitný stamp zahŕňa document_id/revision/digest/mutation_epoch, zápisy majú plan guard a žiadne automatické opakovanie. Nezávislá finálna kontrola: 7 bootstrap + 4 existujúce bridge integration + 7 bridge unit + 1 Rust–Python tool E2E = 19 Rust testov; Python 143 passed, 6 skipped, 8 subtests passed; default OAuth cargo check a git diff --check prešli. `tests/live_bridge_skill_client.py:63` volá skutočné registrované beta tool.call cez reálny LiveSession/TCP do toho istého offscreen GUI Shell (`crates/ketchup-app/tests/live_bridge_python.rs:53`): create/propose/commit, AccessKit Undo/Redo ABA, odmietnutie stale, plan guard a použiteľnosť GUI po disconnect/Python exit. Toto je trusted-host attachment E2E; Python launcher je v tomto teste nahradený pripojením k Shell, preto nejde o neprerušený dôkaz produkčného launcher→native-window toku ani o render/geometry proof. Bootstrap a Python launcher majú osobitné kontraktové testy, Windows pipe polling je reálne testovaný. ToolSearch KetchupLiveSession v tejto bežiacej inštancii zatiaľ vracia no tools found: aktivácia po znovunačítaní a vlastné runtime volania zostávajú pred uzavretím S4 povinné. S4c / #1837 je implementované a nezávisle overené; S4 / #1830 NIE JE dokončené a runtime prijatie pokračuje v #1838. Pôvodné orezávanie GUI Screenshot bolo po bezpečnostnej kontrole nahradené samostatnou GPU textúrou vytvorenou iba z CAD vstupov cez súkromný egui kontext a nezávislý renderer/atlas/scénové resources. Žiadny GUI Screenshot event nie je autoritou obrazu. Súkromný readback koreluje náhodný nonce, capture pass, dokument/revíziu/digest/epoch, pohľad, výber a exact/topology contents stamps; skryté/stale snímky odmieta, zrušenie/odpojenie ruší autoritu. PNG má najviac 64 px na dlhšej strane, render.source=isolated_cad_target, gui_overlays_included=false a geometry_complete=false. Neskoré end-pass, transformované a rovnakovrstvové farebné prekrytia boli reálne viditeľné v GUI renderi, no nezmenili ani bajt CAD PNG. Offscreen natívny ScenePaintCallback vykreslil známy box: 25 projektovaných bodov plochy sa líšilo od prázdneho vonkajšieho rendereru, všetkých 3968 vzoriek náhľadu 64×62 sedelo s nezávislým GPU readbackom pri 1× aj 2× DPI (zdroje 1200×800 a 2400×1600). Výmena exact registry pri nezmenenom dokumente/pohľade odmietla pending obraz ako stale_image; fixture test nie je dôkaz OCCT workeru. Python ukladá PNG iba na explicitnú novú cestu pod artifacts/live-view, overuje stamp/epoch, base64/PNG a vracia cestu/hash/rozmery bez pixelov v texte. Reálny Rust→TCP→Python LiveSession→registrovaný beta.call→PNG test prešiel; uložené PNG sa bajtovo zhodovalo s raw Rust výstupom na tom istom stabilnom CAD stave. Dôkaz artifacts/live-view/s4c-isolated-29604-1788640842049944100.png: 64×51, 9911 B, 99 farieb, SHA-256 230c8a5516a8017cc35fb4b72358320f7fb413a429a1003748c5b0f97c2776ce. Nezávisle 31 Rust testov (14 unit + 7 bootstrap + 4 TCP + 1 Python store + 5 image), Python 214 passed/6 skipped/8 subtests, default OAuth cargo check a git diff --check prešli. Image testy používajú sériový prístup ku GPU, nie zvýšený produkčný timeout. Limity: iba náhľad; 3200×2000 zdroj prekročil existujúci 1500 ms timeout; discarded pass sa neopakuje automaticky. Natívne okno/produkčný launcher ani vlastné načítané live tools Supervisora ešte nie sú runtime overené. Dva skutočné Read(PNG) pokusy v aktuálnom engine vrátili iba [image returned by Read], teda žiadne pixely modelu; visual_delivery zostáva unverified. Pred jedným spoločným reloadom treba vyriešiť odovzdávanie obrazov v Supervisore, nie predstierať vizuálnu kontrolu. Cron #14 je dočasne pozastavený pri tejto runtime prekážke; #1831–#1833 sa neuzatvárajú ani nepreskakujú. Plná hierarchia/priestor a veľkorozsahové geometrické overenie tiež zostávajú budúce kroky. Žiadne fyzické UI vstupy, úpravy existujúcich modelov ani commit/push.

## 1. Ako funguje Supervisor dnes

Supervisor nie je jeden generátor skriptov pre konkrétnu aplikáciu. Model rozhoduje, ktorý pomenovaný nástroj zavolá s typovanými argumentmi. Host nástroj vykoná, vráti výsledok a model podľa neho zvolí ďalší krok. Externé aplikácie robia skutočnú prácu; text asistenta nie je dôkaz jej vykonania.

| Mechanizmus | Overená implementácia | Poučenie pre CAD |
|---|---|---|
| Rozšíriteľné skills | `C:/Sources8/Supervisor/src/tools.py:2590`: načítanie `skills/*.py`, `register_tools()` | Tenký skill sprístupní autoritatívne API; nepotrebuje druhé geometrické jadro. |
| Vytvorenie skillu | `C:/Sources8/Supervisor/skills/create_skill.py:26`: zápis Python súboru, iba minimálna kontrola registračnej funkcie | Vytvorenie súboru nie je test ani dôkaz aktívnej registrácie. Loader používa adresár konkrétnej inštancie. |
| Vykonanie a spätná väzba | `C:/Sources8/Supervisor/src/claude_engine.py:2888`: dispatch, sledovanie zrušenia, výsledok alebo chyba | Nástroj musí vracať jasný úspech/neúplnosť/chybu a identitu dokumentu, nie iba slovné uistenie. |
| Postupné načítanie pamäte | `C:/Sources8/Supervisor/src/hmos.py:870`: predkovia, súhrn, deti, hlbší detail a dimenzionálne odkazy | CAD potrebuje mapu celku, vyhľadanie a rozbalenie konkrétnej zostavy/oblasti. |
| Kontext | `C:/Sources8/Supervisor/src/context_engine/flat_fade.py:98`: staršie výmeny sa zmenšujú, aktuálna ostáva úplná | Veľký výstup aktuálneho nástroja kompakcia nezachráni. Rozpočet musí dodržať už CAD API. |
| Ciele a priebežný stav | `C:/Sources8/Supervisor/src/scheduling/cron_scheduler.py:450`; `src/hmos.py:1388` | Pamäť uchová zámer a ďalší krok; po obnovení treba znovu overiť aktuálny dokument. |

Nekopírovať slepo všetky detaily Supervisora: dnešný ToolSearch podľa auditu môže aktivovať všetky odložené schémy, chyby dispatchu sú často text a podpora obrazov sa líši podľa providera. CAD API má ponúknuť malý stály katalóg a detail schopností na požiadanie. Zmeny Supervisora mimo CAD integrácie nie sú súčasťou tejto úlohy.

## 2. Čo v Kečupe existuje a čo chýba

Stav pred začatím tejto implementácie:

- Python `Session` vytvára vlastný proces `ketchup-headless --stdio`, NIE spojenie s dokumentom otvoreného okna (`sdk/python/ketchup/client.py:87`). Otvorenie rovnakého súboru v dvoch procesoch neznamená spoločný živý stav.
- Headless `state` zostavuje celý zoznam definícií, koreňových výskytov a prvkov; používa sa aj v odpovediach na zmeny (`crates/ketchup-headless/src/protocol.rs:106`). Limit riadku je 4 MiB. Zmena sa môže vykonať skôr, než serializácia zistí príliš veľkú odpoveď. Toto treba odstrániť z cesty nových skills.
- Spoločný `DocumentSession` už plánuje a aplikuje návrhy a poskytuje Undo/Redo (`crates/ketchup-application/src/session.rs:162`). Revízne kontroly nesmú byť obídené.
- Jadro pozná hierarchiu, `InstancePath`, transformácie, tags, klasifikácie, kolekcie, závislosti a väzby (`crates/ketchup-core/src/document.rs:3135`, `:3386`). Tieto dáta netreba znovu vymýšľať v HMOS.
- Existuje priestorový index, ale proxy geometria nemá úplné pokrytie všetkých telies (`crates/ketchup-interaction/src/spatial.rs:12`; `src/projection.rs:15`). Neznáme bounds nesmú znamenať „objekt sa v oblasti nenachádza“.
- Kolízny validátor preveruje kandidátov podľa BRep, ale limituje celý rozsah na 512 výskytov/tiel; iné validátory majú vlastné limity (`crates/ketchup-application/src/collision.rs:29`; `src/validation.rs:10`). Dôkaz navigácie na 10 000 objektoch NIE JE dôkaz úplnej kontroly 10 000 telies.
- Verejné pripojenie k živému GUI dokumentu a obrazový endpoint v súčasnom headless protokole chýbajú. UI testy musia používať headless harness, nie fyzickú myš.

Odkazy sú kotvy auditu; riadky sa pri implementácii môžu posunúť.

## 3. Cieľová architektúra — návrh

Supervisor (plán, HMOS, voľba nástroja)
→ CAD skills (malé schémy, správa spojenia, ohraničené výsledky)
→ spoločná modelová vrstva Kečupu (dotazy, pracovné množiny, návrhy, revízie)
→ živý CAD dokument + existujúci planner, Undo/Redo, OCCT a renderer.

Dva adaptéry toho istého kontraktu:

1. Vlastnená headless session na bezpečný začiatok, automatické testy a samostatné dokumenty.
2. Výslovne zapnuté zabezpečené lokálne spojenie s otvoreným Kečupom. Príkazy spracuje vlastník dokumentu; nesmie sa potichu načítať jeho súbor do iného procesu alebo nahradiť obsah okna.

### Typický pracovný cyklus

1. **Prehľad:** počty, identita/revízia, jednotky, hlavné zostavy a dostupné dimenzie. Nie 10 000 objektov.
2. **Zúženie:** vyhľadám zostavu, oblasť, typ/vlastnosť alebo väzbu. Dostanem súhrn a stránku výsledkov či referenciu pracovnej množiny.
3. **Detail:** rozbalím potrebnú vetvu, prvok, hranu alebo susedstvo; identita vnoreného výskytu používa úplnú cestu.
4. **Návrh zmeny:** malý CAD program, odhad dotknutého rozsahu, explicitné zdieľaná definícia verzus konkrétny výskyt.
5. **Vykonanie:** kontrola pozorovanej revízie; stručný výsledok s ID a stavom Undo. Väčšia makroúloha má jasné čiastkové transakcie, nie sľub jednej atómovej akcie cez tisíce objektov.
6. **Overenie:** geometria, lokálny rozsah aj potrebné okolie/závislosti, obraz s revíziou. „Incomplete“ nie je úspech.
7. **Pamäť:** uložiť zámer, rozhodnutie, stabilný odkaz a ďalší krok. Pri ďalšom volaní znova zistiť aktuálny stav.

### Dimenzie navigácie

- Hierarchia: projekt → zostava → podzostava → výskyt → prvok/geometria.
- Priestor: oblasť, susedstvo, vrstva/podlažie, poloha vo svete.
- Vlastnosti: názov, tag, klasifikácia, materiál a iné už podporované údaje.
- Vzťahy: závislosti, spoje, podopretie, spoločné definície, dotknuté okolie.
- Pracovný zámer: úlohy, rozhodnutia a odkazy v HMOS; nie neoverená kópia geometrie.

### Kontrakt a ochrany

- Každé pozorovanie identifikuje dokument, revíziu/digest, rozsah a úplnosť. Každý zápis vyžaduje pozorovanú revíziu.
- Kurzor je viazaný na dokument, revíziu aj dotaz. Otvorenie iného dokumentu, zmena či Undo zneplatnia starú pracovnú množinu; nespoliehať sa iba na rovnaké číselné ID objektu.
- Počiatočný cieľ rozpočtu: prehľad do 8 KiB; jedna stránka/detail/receipt do 32 KiB UTF-8 a najviac 100 riadkov. Dlžku názvov a zoznamov tiež ohraničiť, s explicitnou informáciou o skrátení. Hodnoty sú návrh akceptačných limitov, nie už zmerané výsledky.
- Úplný export zostáva explicitná operácia/súbor, nie predvolená odpoveď nástroja. Hromadné ID a geometria nesmú zaplavovať kontext.
- Bounds uvádzajú svetový/lokálny priestor, mm, pôvod a platnosť. Obalový kandidát nie je potvrdený prienik.
- Žiadne automatické opakovanie zápisu s neistým výsledkom. Po prerušení najprv zistiť výsledok alebo ohlásiť neistotu.
- Python skill je dôveryhodný kód hosta, nie sandbox. Používať deklarované operácie, nie nový ľubovoľný exec endpoint v CAD aplikácii.
- Zachovať existujúce domčeky, rozpracované súbory, OAuth a všeobecnosť operácií. Žiadne nové špeciálne „house/roof“ vetvy.

## 4. Poradie a dôkazy

| Cieľ | Dodávka | Čo musí preukázať dokončenie |
|---|---|---|
| S1 / #1827 | Tento audit a kontrakt | Zdrojové odkazy, rozdiel live/headless, explicitné medzery a limity. |
| S2 / #1828 | Ohraničené dotazy a kompaktné výsledky cez application/headless/Python | 10k položiek bez strát/duplicít v stránkovaní, merané bajty, stale kurzory, čítanie nemení model; prvý koreňový rozsah jasne označený. |
| S3 / #1829 | Registrované skills Supervisora, trvalá session | Skutočné volania nástrojov: prehľad → detail → vytvorenie/úprava → Undo → Save/Open → overenie. Nie iba import testu ani externý obchádzkový skript. |
| S4 / #1830 | Živý most a vizuálna spätná väzba | Rovnaký otvorený dokument, odmietnutý súbežný zastaraný zápis, GUI Undo, obraz viazaný na revíziu, bezpečné odpojenie. |
| S5 / #1831 | Plná viacrozmerná navigácia a makroúlohy | Vnorené identity, revízne indexy, priestor/vlastnosti/väzby, pracovné množiny a explicitné hranice transakcií. |
| S6 / #1832 | Škálovateľné a poctivé overenie | Kandidáti indexom, BRep detail, závislosti a prieniky cez hranice rozsahu; stale/missing/cancel nikdy nevydajú falošné potvrdenie. |
| S7 / #1833 | Koncový dôkaz vlastnými skills | Nový reálny model a osobitný 10k experiment; veľkosti odpovedí, časy, správnosť zmien, obraz/geometria, Save/Open/Undo a obnovenie pracovného zámeru. |

Pri meraní rozlíšiť 10 000 výskytov zdieľanej definície od 10 000 unikátnych zložitých telies. Zaznamenať studený aj teplý dotaz, náklady na obnovu indexu a veľkosť dát prenesenú do kontextu. Časové výsledky sa musia zmerať na konkrétnom stroji, nie odhadnúť a označiť za splnené.

Starý cieľ A2 (vstavaný asistent stavia drevodom) zostáva zachovaný, ale nie je aktuálnym smerom. Žiadna časť tohto plánu neoprávňuje označiť jeho dôkaz za splnený Supervisorovým modelovaním.
