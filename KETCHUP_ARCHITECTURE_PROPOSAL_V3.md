# Kečup (Ketchup)

## Architektonický a technický návrh AI-native 2D/3D modelovacej platformy — verzia 3

**Stav dokumentu:** konsolidovaný a operatívny architektonický základ po troch oponentúrach verzie 2  
**Dátum:** 1. august 2026  
**Nahrádza na ďalšie rozhodovanie:** `KETCHUP_ARCHITECTURE_PROPOSAL_V2.md`  
**Historické podklady:** verzie 1 a 2 zostávajú zachované  
**Licenčný zámer:** open source a bezplatné používanie; konkrétna licencia vyžaduje rozhodnutie vlastníka projektu a právne overenie

---

# 0. Executive summary

## Čo je Kečup

Kečup má byť rýchly desktopový modelár pre architektúru, interiér a nábytok, ktorý spája jednoduchosť SketchUpu, presnosť parametrického CAD jadra a bezpečné AI ovládanie.

Používateľ musí vedieť modelovať ručne bez AI. AI má používať rovnaké kontrolované nástroje ako UI, pripravovať vysvetliteľné návrhy zmien a nikdy nesmie obísť validáciu, limity, náhľad ani transakčný commit.

## Prvý produkt

Prvý použiteľný produkt umožní:

- kresliť jednoduché presne kótované profily;
- vytiahnuť objemy a robiť základné otvory a booleany;
- používať Smart Push/Pull s okamžitým náhľadom;
- meniť parametre a jednotkovo bezpečné výrazy;
- používať skupiny, zdieľané komponenty, výskyty a tagy;
- vyberať, snapovať, zarovnávať, presúvať a kopírovať;
- bezpečne ukladať, načítať, undo/redo a exportovať základný výsledok;
- vykonať definovanú sadu modelovacích úloh cez UI aj AI Proposal workflow.

Prvý produkt ešte nebude plnohodnotný BIM, profesionálny drawing systém, mechanical assembly CAD, organický modelár, procedurálny les, plugin marketplace ani browserový CAD.

## Technické jadro

- Rust pre dokument, protokol, scheduler, renderer a aplikáciu;
- Open CASCADE Technology ako predvolený exact B-Rep backend za úzkym versionovaným C++ façade;
- `wgpu` pre renderer;
- nemenné revízie, single-writer/N-readers a asynchrónny scheduler;
- explicitné `ExactBody` a `MeshBody`, nie jedna nejasná geometria;
- jediná mutačná cesta cez `Canonical CommandBatch`;
- oddelené `Intent`, `Proposal`, `Canonical Commands` a efemérna interakcia;
- viacvrstvové topological naming s triedami `Guaranteed`, `BestEffort`, `Ephemeral`, `Ambiguous` a `Lost`;
- kanonické rozmery a význam ako zdroj pravdy; B-Rep, tessellácia a GPU dáta sú regenerovateľné výsledky.

## Najväčšie riziká

1. neúplná topologická história exact backendu;
2. stabilita referencií po zmene parametra alebo verzie backendu;
3. robustnosť booleanov a tolerancií;
4. C++ FFI, pády a procesná izolácia;
5. pamäť snapshotov a odvodených cache;
6. responzivita Smart Push/Pull a presného výberu;
7. príliš široký produktový rozsah;
8. nevhodná licencia alebo predčasne stabilizované API.

## Čo musí dokázať PoC

PoC nie je malé MVP. Je to séria zlyhateľných brán:

- **R0:** uzavrieť fakty, verzie, licenčné vstupy, korpusy a prahy;
- **A0:** overiť exact façade, topologickú evidenciu a prežitie referencií;
- **A1:** overiť kanonický dokument, presnosť, save/load a protokolovú ekvivalenciu;
- **B:** overiť snapshoty, scheduling, cancellation, crash recovery a worker režim;
- **C:** overiť viewport, presný výber a Smart Push/Pull na referenčnom hardvéri.

Po každej bráne sa buď pokračuje, vykoná vopred určená zmena, alebo sa vývoj zastaví. Prahy sa nesmú spätne zmäkčiť preto, že výsledok nevyšiel.

---

# 1. Register rozhodnutí

## 1.1 Rozhodnuté a záväzné pre PoC

| ID | Rozhodnutie | Dôvod |
|---|---|---|
| D-01 | Prvý segment je architektúra, interiér a nábytok. | Najlepšie zodpovedá jednoduchému priamemu modelovaniu a znižuje scope. |
| D-02 | Dlhodobé jadro zostane doménovo neutrálne. | Mechanical, BIM, drawing a nature nesmú vyžadovať druhé jadro. |
| D-03 | Hlavný core je v Ruste. | Bezpečnosť pamäte, concurrency a dlhodobá údržba. |
| D-04 | OCCT je predvolený exact backend PoC. | Potrebujeme priemyselný B-Rep baseline; alternatíva sa musí najprv preukázať. |
| D-05 | OCCT je izolovaný za vlastným úzkym C++ façade. | C++ typy, vlastníctvo, výnimky a thread-safety nesmú presiaknuť do core. |
| D-06 | Renderer používa `wgpu`. | Cross-platform GPU smer, instancing a moderný render pipeline. |
| D-07 | Dokument je revízny, single-writer/N-readers. | UI, Queries a renderer nesmú blokovať na dlhom recompute. |
| D-08 | Dokument mení iba validovaný `Canonical CommandBatch`. | Rovnaká auditovateľná cesta pre UI, CLI, plugin aj AI. |
| D-09 | Dragging/preview nie je dokumentová transakcia. | 60 Hz interakcia nesmie vytvárať revízie ani spúšťať plný exact commit. |
| D-10 | Kanonický dokument, nie command log, je zdroj pravdy. | Migrácie nesmú prehrávať celú históriu cez starý kernel. |
| D-11 | Exact a mesh telo sú odlišné kanonické typy. | Presnosť, strata a exportovateľnosť musia byť explicitné. |
| D-12 | TNP má úzky garantovaný rozsah a explicitné zlyhanie. | Tiché prepojenie na inú plochu je neprípustné. |
| D-13 | AI používa Intent/Proposal/Command/Query workflow. | LLM plánuje; deterministické jadro validuje a vykonáva. |
| D-14 | Undo krok je jeden úspešne commitnutý používateľský `CommandBatch`. | Intent ani Proposal nemusia existovať pri manuálnej, CLI alebo pluginovej operácii. |
| D-15 | BIM, drawing, mechanical a nature zostávajú vo vízii, nie v PoC. | Rozšíriteľnosť sa zachová bez paralelného stavania štyroch produktov. |

## 1.2 Hypotézy, ktoré rozhodne meranie

| ID | Hypotéza | Rozhoduje |
|---|---|---|
| H-01 | Exact backend bude v produkcii worker proces. | Brána B |
| H-02 | Zvolený tolerance profile a coordinate envelope sú bezpečné. | A0/A1 corpus |
| H-03 | `egui` postačí aj po PoC. | Brána C a UX spike |
| H-04 | Konkrétna serializácia `document.bin` je vhodná. | A1 migration/size benchmark |
| H-05 | Vybraný solver sa dá bezpečne integrovať. | Samostatný post-PoC spike po licenčnom filtri |
| H-06 | WASM Component Model je vhodný pre verejné pluginy. | Neskorší plugin pilot |
| H-07 | Mesh boolean cesta má hodnotu pre vybrané stavebné operácie. | Porovnávací benchmark po exact baseline |

## 1.3 Fakty a právne otázky — nie benchmarky

Pred príslušnou implementáciou sa musia z primárnych zdrojov alebo právnym posúdením uzavrieť:

- konkrétny OCCT release, build fingerprint a podporovaný toolchain;
- licencia každej závislosti a dôsledky distribučného/linkovacieho modelu;
- licenčná trieda Kečupu;
- aktuálne kompatibilné verzie `wgpu`, UI a WASM toolchainu;
- platformová dostupnosť vybraných knižníc.

Tieto body sa nesmú schovať za formuláciu „rozhodne PoC“. PoC meria technické správanie; nevytvára právne fakty.

## 1.4 Odložené

- profesionálny drawing a sadzba dokumentácie;
- IFC round-trip a plný BIM;
- mechanical assemblies;
- procedurálna vegetácia a veľké streamované svety;
- stabilné verejné plugin ABI a marketplace;
- browserový klient, cloud a simultánna spolupráca;
- široká import/export matica.

---

# 2. Produktový kontrakt: First Lovable Product

## 2.1 Jedna veta

Prvá verzia Kečupu je rýchly desktopový parametrický modelár pre architektúru, interiér a nábytok s priamym SketchUp-like ovládaním, presnými rozmermi a bezpečným AI asistentom.

## 2.2 Používateľ dokáže

1. založiť model v zvolených jednotkách;
2. nakresliť a zakótovať jednoduchý profil;
3. vytiahnuť ho na presný objem;
4. meniť rozmer bez kumulatívneho driftu;
5. vytvoriť otvor, jednoduchý cut alebo union;
6. používať Smart Push/Pull s vysvetlením výslednej operácie;
7. organizovať model do groups, definitions, occurrences, tags a collections;
8. používať presný move/copy, snapping, alignment a jednoduché patterns;
9. ukladať, načítať, undo/redo a zotaviť posledný commit po páde workera;
10. exportovať minimálne jeden exact formát a jeden mesh formát podľa výsledku PoC;
11. zadať vybrané úlohy textom a pred commitom vidieť Proposal, predpoklady a diff.

## 2.3 Používateľ zatiaľ nedokáže

- robiť plný BIM model s garantovaným IFC round-tripom;
- vyrábať profesionálne viaclistové výkresy;
- tvoriť komplexné mechanické zostavy a simulácie;
- organicky sculptovať;
- generovať produkčné lesy a filmové scény;
- inštalovať ľubovoľné verejné pluginy zo marketplace;
- simultánne editovať jeden dokument viacerými používateľmi.

## 2.4 Produktové pravidlo

AI nesmie byť podmienkou použiteľnosti. Ak je manuálne kreslenie, výber, Push/Pull alebo zmena rozmeru zlá, AI iba zrýchli zlý produkt.

---

# 3. Architektonický prehľad

```text
UI / CLI / Python / Voice / AI / trusted tools
                       │
                 Intent Layer
                       │
                Proposal Layer
 assumptions • read/write set • risk • preview • digest
                       │
          Canonical Command Gateway
 schema • capability • precondition • budget • transaction
                       │
        Revisioned Canonical Document
 entities • parameters • FeatureSpecs • stable references
                       │
             Evaluation Scheduler
 dirty DAG • generation • cancellation • stale rejection
             ┌─────────┴─────────┐
             │                   │
      Exact Geometry       Sketch / Mesh /
      façade or worker     Procedural services
             └─────────┬─────────┘
                       │
   Interaction and Spatial Query Service
 exact hit • snaps • inference • SubshapeRef resolution
                       │
      Derived Render Data / GPU Cache
                       │
                 wgpu Renderer
```

## 3.1 Závislostné pravidlá

- UI závisí od framework-agnostic view modelu a protokolu.
- Proposal vrstva plánuje, ale nedokáže mutovať dokument.
- Document Core nepozná OCCT, widgety ani GPU buffre.
- Geometry backend nepozná prirodzený jazyk ani používateľské oprávnenia.
- Renderer neposudzuje presnú CAD identitu vybranej plochy.
- AI, importér a plugin sú nedôveryhodní klienti s capabilities a budgetmi.
- Doménové balíky pridávajú sémantiku nad spoločné entity; nevytvárajú paralelný dokumentový model.

## 3.2 Počiatočné členenie

```text
ketchup/
├─ crates/
│  ├─ ketchup-core/
│  ├─ ketchup-protocol/
│  ├─ ketchup-geometry/
│  ├─ ketchup-render/
│  ├─ ketchup-io/
│  └─ ketchup-app/
├─ cpp/occt-facade/
├─ schemas/
├─ corpora/
└─ tests/
```

Nový crate vznikne až po preukázanej hranici závislostí, bezpečnosti, build time alebo distribúcie.

---

# 4. Kanonický dokument, revízie a pamäť

## 4.1 Zdroj pravdy

Kanonický dokument uchováva:

- stabilné ID, schémy a scope dokumentu/assetu;
- kanonické parametre, jednotky a výrazy;
- `FeatureSpec`, vstupné referencie a doménový význam;
- `Definition`, `Occurrence`, `Group`, `Tag`, `Collection` a `SavedView`;
- explicitné `ExactBody` alebo `MeshBody`;
- stav referencií a diagnostiku;
- odkazy na content-addressed blobs;
- determinism envelope a backend provenance.

`FeatureResult`, B-Rep cache, tessellácia, BVH, thumbnails a GPU buffre sú odvodené dáta. Môžu sa zahodiť a regenerovať.

## 4.2 Revízny model

- jeden writer vytvára novú nemennú revíziu;
- čitatelia držia konkrétny snapshot;
- snapshoty používajú structural sharing a malé delty;
- worker job nesie revision, generation token a input digest;
- výsledok sa vloží iba pri úplnej zhode tokenu a digestu;
- stale výsledok sa zahodí;
- renderer môže ukázať posledný dobrý výsledok iba ako označený stale obsah.

## 4.3 Undo/redo

Jeden používateľsky viditeľný undo krok je jeden úspešne commitnutý `CommandBatch`, bez ohľadu na to, či vznikol z UI, AI Proposalu, CLI alebo pluginu. Undo:

1. naviguje na predchádzajúcu kanonickú revíziu;
2. zruší rozbehnuté joby alebo invaliduje ich generation token;
3. nikdy neobnoví stale odvodený výsledok ako aktuálny.

## 4.4 Pamäťová politika

Immutable neznamená plnú kópiu geometrie pri každej revízii.

- kanonické stromy používajú structural sharing;
- odvodené cache majú revision tag, memory budget a LRU/priority eviction;
- GPU buffre starých revízií sa nedržia len kvôli undo;
- checkpointy obmedzia dĺžku delta reťazcov;
- audit log je orezateľný;
- undo retention má explicitnú používateľskú a systémovú politiku;
- memory benchmark sleduje plateau po opakovaných editáciách, nie iba jednorazové maximum.

---

# 5. Geometrické kontrakty

## 5.1 Autoritatívne typy

```rust
enum CanonicalBody {
    Exact(ExactBodySpec),
    Mesh(MeshBodySpec),
}
```

- `ExactBody` je autorita pre presné rozmery, B-Rep operácie, stabilné referencie a exact export.
- `MeshBody` je autorita pre importované/procedurálne polygonálne dáta alebo výslovne mesh workflow.
- tessellácia `ExactBody` je iba odvodený render výsledok, nie druhé kanonické telo.
- konverzia exact ↔ mesh je pomenovaná operácia s provenienciou a reportom straty.

Mesh boolean sa nesmie automaticky zameniť za exact boolean. Porovnávací benchmark môže podporiť explicitnú stavebnú mesh feature, iba ak sú jasné dôsledky pre kóty, referencie, export a BIM.

## 5.2 Exact backend

```rust
struct ExactOpOutput {
    shape: ShapeHandle,
    topology_history: TopologyHistory,
    tolerance_report: ToleranceReport,
    diagnostics: Vec<GeometryDiagnostic>,
    result_fingerprint: ResultFingerprint,
    history_confidence: HistoryConfidence,
}
```

Façade musí:

- zachytiť každú C++ výnimku;
- nevystaviť raw pointer ani OCCT typ;
- vlastniť handly a thread-safety pravidlá;
- podporovať rovnaký logický kontrakt in-process aj cez worker;
- vracať validáciu výsledného shape;
- priznať neúplnú topologickú históriu;
- doplniť backend history o post-operation topological diff/walker, kde je to potrebné;
- byť fuzzovaný a testovaný crash corpusom.

`Generated/Modified/Deleted` z backendu nie sú automaticky úplná pravda. `Guaranteed` referencia sa nesmie opierať o nedoloženú históriu.

## 5.3 Scheduler a cache key

Cache key zahŕňa minimálne:

```text
FeatureSpec
input result fingerprints
geometry backend identity/build
schema versions
tolerance profile
relevant platform envelope
```

Operácie sa plánujú paralelne iba tam, kde to povoľuje DAG a reálna thread-safety backendu. Cancellation je kooperatívna alebo procesná; vlákno sa násilne neukončuje.

---

# 6. Presnosť a determinizmus

## 6.1 Precision contract

Kečup rozlišuje:

1. autoritatívny vstup, napríklad `2400 mm`;
2. geometrickú reprezentáciu v `f64`;
3. toleranciu konkrétnej operácie;
4. aproximáciu render mesh a GPU `f32`.

Pravidlá:

- verejné rozmery majú explicitné jednotky;
- save/load nezmení kanonickú hodnotu;
- nameraná aproximácia nikdy neprepíše autoritatívny parameter;
- tolerance profile je centrálny, versionovaný a uložený v determinism envelope;
- lokálna degradácia kernelu sa reportuje;
- georeferencia je transformácia nad lokálnym modelom;
- pracovná jednotka a coordinate envelope sa uzavrú na mriežke `model unit × magnitude × smallest feature × operation`.

## 6.2 Determinism envelope

Dokument zaznamenáva core, backend, build, tolerance a schema verzie. Garantujeme dátový determinizmus kanonických údajov a geometrickú ekvivalenciu v deklarovanej tolerancii. Negarantujeme bitovo identické B-Rep alebo mesh bloby naprieč všetkými platformami a backendmi.

Golden testy porovnávajú ID, parametre, vzťahy, analytické typy, bounding box, objem, plochu a ďalšie invarianty, nie iba binárny blob.

---

# 7. Topological naming a migrácia referencií

## 7.1 SubshapeRef

Referencia obsahuje podľa potreby:

```text
document_or_asset_scope
reference_schema_version
producer_feature_id
output_port
semantic_role
source_element_id
genesis_or_lineage_path
expected_geometry_type
adjacency_signature
geometric_signature
expected_cardinality
stability_class
geometry_backend_provenance
```

Resolver používa v poradí sémantickú rolu, doloženú backend history, lineage, topologickú/susedskú signatúru a geometrický fingerprint. Fingerprint je dôkaz podobnosti, nie automatický dôkaz identity.

Výsledok je `Resolved`, `Ambiguous` alebo `Lost`. Tiché zvolenie inej plochy je chyba.

## 7.2 Triedy stability

- `Guaranteed`: platí iba pre presne pomenovaný a otestovaný feature kontrakt;
- `BestEffort`: resolver skúša bezpečné obnovenie bez garancie;
- `Ephemeral`: platí len pre aktuálny preview/result;
- `Ambiguous`: existuje viac rovnocenných kandidátov;
- `Lost`: cieľ už nemožno bezpečne identifikovať.

Počiatočný kandidát na `Guaranteed` zahŕňa top/bottom jednoduchej extrúzie a bočnú plochu odvodenú od konkrétnej hrany profilu. Brána A0 môže tento zoznam zúžiť, nie bez dôkazu rozšíriť.

## 7.3 Zmena backendu alebo jeho verzie

Pri otvorení dokumentu s iným geometry backend fingerprintom:

1. pôvodný súbor zostáva nedotknutý;
2. cache sa považuje za neplatnú;
3. spustí sa nedestruktívny reference audit;
4. všetky `Guaranteed` a používané `BestEffort` referencie sa znovu vyriešia;
5. výsledok sa porovná s uloženým lineage a fingerprintom;
6. report uvedie `Resolved/Ambiguous/Lost` a zmeny stability;
7. nesúlad nikdy nespôsobí tiché prepojenie;
8. migrácia sa commitne ako explicitná transakcia až po potvrdení.

Ak sa zhorší referencia používaná aktívnou downstream feature, dotknutá vetva sa otvorí v compatibility/quarantine režime. Celý dokument sa nemusí automaticky uzamknúť, ale žiadna poškodená vetva sa nesmie prepočítať ani exportovať ako platná bez vyriešenia. Migrácia medzi aspoň dvoma pripnutými backend buildmi sa stane povinným compatibility testom pred prvým verejným sľubom spätnej čitateľnosti.

---

# 8. Interaction a Smart Push/Pull

## 8.1 Interaction service

Samostatná služba poskytuje exact hit testing, prekryté kandidáty, snaps, inference, scoring, hysteréziu, hover lock, selection filters a prevod na `SubshapeRef`. Renderer poskytuje iba coarse GPU kandidáta a vizuálne zvýraznenie.

## 8.2 Preview a commit

```text
gesture
→ ephemeral interaction state
→ lacný transform/mesh preview
→ numerický HUD + snap + action digest
→ potvrdenie
→ jeden Canonical CommandBatch
→ exact recompute
```

Preview nie je sľub finálnej exact geometrie. Rozdiel alebo zlyhanie po commite musí UI ukázať.

## 8.3 Smart Push/Pull

Systém môže:

- zmeniť parameter pôvodnej extrúzie iba pri jednoznačnej proveniencii;
- pridať novú offset/extrude feature;
- vytvoriť cut/otvor;
- vytvoriť nové telo;
- pri nejednoznačnosti ponúknuť voľby.

Pred potvrdením UI zobrazí textový `action digest`, napríklad „Mení sa výška feature Extrude-12 z 2400 mm na 2700 mm“ alebo „Pridá sa nová Cut feature“. Farebné odlíšenie môže pomôcť, ale nesmie byť jediným vysvetlením. Pri nejednoznačnosti je bezpečný default nová feature, nie tichá zmena vzdialeného parametra.

---

# 9. Intent, Proposal, Commands a undo

## 9.1 Úrovne

- `Intent`: používateľský/doménový cieľ;
- `Proposal`: vysvetliteľný plán s predpokladmi, riskom, diffom a validity kontraktom;
- `Canonical CommandBatch`: presná atómová mutácia;
- `Query`: read-only pohľad na snapshot;
- `Ephemeral Interaction`: dočasný stav bez revízie.

## 9.2 Proposal validity

Proposal nesie `base_revision` ako provenance, ale neinvaliduje sa len preto, že sa zmenila nesúvisiaca kamera alebo vzdialená entita. Jadro autoritatívne vypočíta:

- explicitný read set a write set;
- transitívne vstupné fingerprinty affected setu;
- relevantné Query výsledky a výber;
- namespace/policy epochy a globálne invarianty;
- tolerance profile a schema verzie;
- digest plánovaného `CommandBatch`.

Klient môže poslať hint, nikdy nie autoritatívny scope.

Pred commitom sa dependency/read-set digest prepočíta. Ak sa relevantný vstup zmenil, Proposal je `Stale/Invalidated`; nesmie sa ticho rebasovať. Ak sa zmenila iba nerelevantná revízia, jadro môže Proposal znovu validovať bez nového LLM plánovania.

## 9.3 Commit pipeline

1. schema a capability validation;
2. dependency/read/write-set výpočet;
3. preconditions a resource budgets;
4. izolovaný dry-run;
5. geometrická a doménová validácia;
6. autoritatívny diff a digest;
7. potvrdenie rizikovej zmeny;
8. opätovná validácia dependency digestu;
9. atómový commit alebo rollback;
10. nová revízia, audit a scheduling.

Dry-run nesmie meniť dokument ani otráviť zdieľanú cache.

---

# 10. AI a bezpečnosť

AI dostáva malú kontextovú sadu doménových intents/tools, nie stovky interných Commands. Každý batch má limit Commands, entít, topologického rastu, wall time, CPU, RAM, I/O a paralelných jobov.

Threat model zahŕňa prompt/tool-output injection, geometry DoS, TOCTOU, preview mismatch, capability escalation, škodlivé importéry, cloud exfiltration, cache poisoning, path traversal, zip bombs a citlivé audit logy.

Metadáta dokumentu sú nedôveryhodné dáta, nie systémové inštrukcie. Cloudové odoslanie je opt-in a ukazuje rozsah dát. Importéry rizikových formátov bežia v procese/sandboxe s kvótami. Hromadné mazanie, stratová konverzia, prepis súboru, cloud upload a vysokorizikový commit vyžadujú potvrdenie.

---

# 11. Súborový formát a kompatibilita

```text
model.ketchup
├─ manifest.json
├─ document.bin
├─ audit/commands.log       # voliteľný
├─ blobs/<content-hash>
├─ cache/                   # zahoditeľná
├─ previews/
└─ extensions/<namespace>/
```

Požiadavky:

- atomické uloženie a obnova po páde;
- checksums, limity a bezpečné cesty;
- versionované schémy a explicitné migrácie;
- zachovanie neznámych namespaced dát, ak je to bezpečné;
- backend provenance pri každej backendovo citlivej referencii;
- žiadna migrácia priamo nad jedinou kópiou súboru;
- canonical round-trip bez driftu;
- cache nikdy nie je jediným nositeľom významu.

Stabilita formátu sa verejne sľúbi až po migration suite vrátane starších schém a aspoň jednej zmeny backend buildu.

---

# 12. Kritická cesta

Poradie sa nesmie meniť len preto, že UI alebo AI demo je atraktívnejšie:

1. **R0:** licenčné vstupy a vlastník/termín rozhodnutia, pripnutý backend/toolchain, corpora, hardvér a prahy;
2. **A0:** exception-safe façade, extrude/cut, topology evidence, TNP survival;
3. **A1:** canonical document, revision model, save/load, precision a UI/RPC ekvivalencia;
4. **B:** scheduler, memory policy, cancellation, crash recovery, worker/in-process rozhodnutie;
5. **C:** viewport, exact picking, snapping a Smart Push/Pull;
6. po výsledkoch C uzavrieť First Lovable Product proti pracovnému setu 20 úloh zmrazenému už v R0;
7. úzky manuálne použiteľný modelár;
8. AI Proposal workflow nad fungujúcim modelárom;
9. až potom solver, drawing pilot, BIM primitives a plugin pilot.

Ak A0 nepreukáže minimálny exact/TNP kontrakt, nezačína sa renderer produktu. Ak C nepreukáže jednoduchú interakciu, nezačína sa rozširovanie AI tool surface.

---

# 13. PoC charter: zlyhateľné brány

## 13.1 Pravidlá merania

Pred spustením každej brány sa verzujú a zmrazia:

- fixture corpus a jeho obtiažnostné triedy;
- očakávaný výsledok každej fixture;
- metrika, prah a spôsob merania;
- referenčný hardware/software envelope;
- následok úspechu a neúspechu.

Číselné prahy nižšie sú počiatočné rozhodnutia V3. Môžu sa zmeniť iba pred prvým meraním danej brány cez datovaný ADR so zdôvodnením. Zmena po zhliadnutí výsledku znamená neúspech pôvodnej brány a vytvorenie novej verzie testu.

## 13.2 R0 — research a preregistrácia

**Musí byť hotové pred A0:**

- pripnutý OCCT release, build fingerprint, compiler a build/link model;
- zoznam primárnych licenčných zdrojov a vlastník právneho posúdenia;
- termín rozhodnutia licenčnej triedy projektu;
- reprodukovateľný toolchain;
- A0/A1 corpus a mutation corpus v repozitári;
- zoznam interaktívnych operácií;
- definované referenčné stroje pre B/C;
- dvadsať kanonických používateľských úloh;
- vlastník každého otvoreného rozhodnutia.

**No-go:** bez pripnutého backendu, korpusu a threshold súboru sa A0 nezačne.

## 13.3 A0 — exact kill-risk spike

**Rozsah:** façade, jednoduchý profil, extrude, planar cut, parameter mutation, shape validation, topology evidence a reference resolver. Bez finálneho UI, save formátu a AI.

**Prahy:**

| Metrika | Prah |
|---|---|
| C++ výnimka prekročí FFI hranicu | 0 prípadov v celom corpuse a najmenej 10 000 fuzz volaniach |
| Baseline valid extrude/cut fixtures | 100 % validný očakávaný výsledok |
| Podporovaný adversarial corpus | najmenej 90 % validných očakávaných výsledkov; všetky zlyhania štruktúrovane diagnostikované |
| Tiché vrátenie geometricky/topologicky neplatného shape | 0 |
| `Guaranteed` TNP mutation tests | 100 % správna identita |
| Tiché nesprávne prepojenie v ľubovoľnej stability class | 0 |
| History evidence pre preregistrovaný `Guaranteed` subset | 100 % doložená; chýbajúca evidencia znamená neúspech daného A0 behu |

**Následky:**

- ak nedrží ani top/bottom/side jednoduchej extrúzie, zastaviť A1 a prehodnotiť backend alebo referenčný model;
- ak adversarial úspešnosť klesne pod prah, nezakrývať to retry slučkou: zúžiť podporovaný operation envelope a spustiť cielený exact-vs-mesh benchmark;
- ak façade prepustí výnimku, A0 zlyhala a hranica sa opraví pred opakovaním;
- ak prejde iba menší TNP subset, explicitne sa zúži `Guaranteed`; zvyšok je `BestEffort`.

## 13.4 A1 — canonical exact vertical slice

**Rozsah:** dokument, revisions, `CommandBatch`, save/load, precision round-trip, dirty DAG a UI/RPC adapter.

**Prahy:**

| Metrika | Prah |
|---|---|
| Zmena kanonických parametrov/ID po 100 save/load cykloch | 0 |
| UI a RPC nad rovnakým CommandBatch | 100 % rovnaký kanonický digest a invarianty |
| Atómový rollback po chybe | 100 % fixtures bez čiastočnej mutácie |
| Recompute nezávislého uzla po lokálnej zmene | 0 v zmrazenom DAG corpuse |
| Precision corpus | 100 % autoritatívnych hodnôt bez driftu; geometria v deklarovanej tolerancii |
| Stará schema migration fixtures | 100 % zachovaný deklarovaný význam alebo explicitný loss report |

**Následky:** neúspech save/load, rollbacku alebo protokolovej ekvivalencie blokuje B. Serializácia alebo revision model sa zmení skôr, než vznikne produktové UI.

## 13.5 B — konkurencia, izolácia a pamäť

**Rozsah:** snapshoty, scheduler, cancellation, stale races, worker/in-process experiment, crash recovery, Proposal digest a cache eviction.

**Prahy:**

| Metrika | Prah |
|---|---|
| Stale výsledok vložený ako aktuálny | 0 v najmenej 10 000 schedule permutations |
| Worker crash poškodí poslednú commitnutú revíziu | 0 v 100 crash-recovery behoch |
| C++ výnimka prekročí transport/FFI kontrakt | 0 |
| Commit Proposalu so zmeneným relevantným read-set digestom | 0 prijatých |
| Dlhý geometry job blokuje navigačný/query reader | 0 blokovaní nad 100 ms; p95 query do 16,7 ms na referenčnom scenári |
| Cancellation workera po žiadosti | p95 do 250 ms pre killable test job; žiadna strata commitu |
| Opakované editácie a eviction | po warm-up bez neobmedzeného rastu odvodených cache; plateau podľa zmrazeného memory budgetu |
| Worker transport overhead pre pomenovanú interaktívnu triedu | p95 najviac 15 ms a najviac 20 % end-to-end času |

**Následky:**

- stale insert alebo strata revízie blokuje C;
- ak worker overhead neprejde, vykoná sa jedna ohraničená optimalizácia transportu a brána sa zopakuje;
- ak stále neprejde, C nezačne bez ADR, ktoré na základe crash fuzzingu zvolí in-process, worker alebo explicitne rozdelené triedy operácií;
- memory rast bez plateau blokuje produktový revision/undo model.

## 13.6 C — interakcia a výkon

Referenčné scény a hardware sa zmrazia v R0. Minimálne jeden profil je bežný notebook s integrovanou GPU.

**Prahy:**

| Metrika | Prah |
|---|---|
| Navigácia a efemérny preview po warm-up | p95 frame time ≤ 16,7 ms; p99 ≤ 33,3 ms |
| Input-to-preview | p95 ≤ 50 ms |
| Exact parameter edit v pomenovanej interaktívnej triede | p95 výsledok ≤ 100 ms |
| Exact pick/snap | p95 ≤ 50 ms na referenčnej scéne |
| Dlhá operácia | progress/cancel bez zablokovania navigácie nad 100 ms |
| 10 000 occurrences jednej definície | jedna zdieľaná autoritatívna geometria; per-occurrence iba transform/override/index dáta |
| Preview/commit význam | 100 % fixtures buď zhodný action digest, alebo explicitne oznámený mismatch/error |

**Následky:**

- pri pomalom exact commite operácia vypadne z interaktívnej triedy a dostane async progress; ak ide o základný Push/Pull scenár, C zlyhala;
- pri pomalom pickingu sa opraví spatial query pipeline, nie maskuje rendererovým odhadom;
- C musí prejsť pred rozšírením modelovacích nástrojov alebo AI workflow.

---

# 14. Dvadsať kanonických používateľských úloh

Každá úloha dostane fixture, prirodzené zadanie, očakávaný Intent, tvar `CommandBatch` a deterministické invarianty. Toto je pracovný rozsah, nie tvrdenie o úplnosti.

1. vytvoriť presný obdĺžnikový profil;
2. extrudovať profil na zadanú výšku;
3. zmeniť výšku pôvodnej extrúzie;
4. vytvoriť obdĺžnikový otvor/cut;
5. Push/Pull jednoznačne zmeniť zdrojový parameter;
6. Push/Pull nejednoznačnej plochy a vyžiadať voľbu;
7. posunúť objekt o presný vektor;
8. kopírovať objekt so snapom;
9. vytvoriť zdieľanú component definition a viac occurrences;
10. upraviť definition a aktualizovať všetky occurrences;
11. urobiť jednu occurrence unikátnou;
12. vytvoriť group a vstúpiť do edit contextu;
13. priradiť tag a meniť viditeľnosť bez zmeny vlastníctva geometrie;
14. vytvoriť jednoduchý lineárny pattern;
15. nastaviť parameter výrazom, napríklad `šírka / 2`;
16. upraviť parameter a prepočítať iba závislú vetvu DAG;
17. uložiť, znovu otvoriť a overiť rozmery/referencie;
18. undo/redo celého viacpríkazového batchu ako jedného kroku;
19. exportovať podporovaný exact alebo mesh výsledok s loss reportom;
20. vykonať rizikovejšiu AI úpravu cez Proposal, preview, confirmation a verification.

Úloha sa nepovažuje za splnenú iba podľa textovej odpovede AI. Kontroluje sa výsledný kanonický dokument a geometrické invarianty.

---

# 15. Roadmapa po PoC

## Fáza 1 — úzky modelovací produkt

- desktopová aplikácia;
- kamera, selection, snapping a inference;
- groups, definitions, occurrences a tags;
- jednoduchá skica a základné constraints podľa licenčne filtrovaného solver spikeu;
- extrude, jednoduché cut/union a Smart Push/Pull;
- parametre, jednotky a výrazy;
- revision undo/redo a natívny save/load;
- jeden exact import/export a mesh export;
- lokálne Proposal/Command/Query API;
- AI asistent na kanonických úlohách.

## Fáza 2 — prvá doména

- architecture/interior/furniture primitives;
- základný asociatívny pohľad alebo technický export;
- Python SDK a obmedzený plugin pilot;
- migration a compatibility suite;
- používateľské testy vrátane vhodných FurniGen scenárov.

## Neskoršie míľniky

- profesionálny drawing;
- BIM/IFC a výkazy;
- mechanical assemblies;
- procedurálna príroda, instancing, LOD a streaming;
- WASM plugin ekosystém;
- collaboration, remote compute alebo web podľa budúceho rozhodnutia.

---

# 16. Vlastníci a termíny rozhodnutí

| Rozhodnutie | Vlastník roly | Najneskorší bod |
|---|---|---|
| Licenčná trieda projektu | vlastník projektu + právny poradca | pred solver shortlistom a verejnou distribúciou |
| OCCT release/build/link model | geometry lead | pred A0 |
| A0/A1 corpora a prahy | geometry lead + test lead | pred A0 |
| Referenčný hardware | product/graphics lead | v R0 pred B/C |
| Worker vs in-process | architecture lead | po B, pred C |
| Unit/tolerance/coordinate envelope | geometry lead | po A1, pred stabilizáciou formátu |
| `document.bin` serializácia | core/IO lead | v A1 |
| Solver shortlist | architecture lead + právny poradca | po licenčnej triede, po PoC |
| UI framework pre produkt | UI lead | po C |
| Verejný compatibility sľub | vlastník projektu + core lead | po migration suite |

Konkrétne osoby a kalendárne dátumy sa doplnia v projektovom pláne. Rozhodnutie bez vlastníka a najneskoršieho bodu nie je riadené rozhodnutie.

---

# 17. Stanovisko k trom recenziám V2

## 17.1 Prvá recenzia V2

**Verdikt:** veľmi dobré potvrdenie základnej architektúry a užitočný implementačný risk checklist. Jej tvrdenie, že dokument je pripravený rovno na Bránu A, bolo mierne optimistické, pretože chýbali prahy a R0 vstupy.

| Návrh | Stanovisko V3 | Dôvod |
|---|---|---|
| Testovať neúplnú OCCT topology history a doplniť walkery | **Prijať** | Bez evidencie nemožno sľúbiť `Guaranteed`. |
| Riešiť memory/VRAM snapshotov | **Prijať** | Structural sharing a eviction musia byť od začiatku kontrakt, nie neskoršia optimalizácia. |
| Smart Push/Pull nesmie neviditeľne meniť vzdialený parameter | **Prijať** | Proveniencia, action digest a bezpečný default novej feature sú povinné. |
| Starý AI Proposal nikdy ticho nerebasovať | **Prijať s úpravou** | Relevantná zmena invaliduje Proposal; nerelevantná revízia môže prejsť novou jadrovou validáciou bez LLM replanu. |
| Už iba kódovať a dokument zavrieť | **Prijať po V3 konsolidácii** | Recenzia odhalila reálne kontrakty hodné jedného krátkeho prepisu; ďalší široký papierový cyklus už nemá prednosť pred A0. |

## 17.2 Druhá recenzia V2

**Verdikt:** technicky najsilnejšia recenzia. Najpresnejšie odhalila, že V2 mala míľniky nazvané bránami bez formálneho spôsobu zlyhania.

| Návrh | Stanovisko V3 | Dôvod |
|---|---|---|
| Pridať prahy a vopred určené fail actions | **Prijať** | Bez nich možno každý výsledok vyhlásiť za úspech. |
| Migrácia `SubshapeRef` medzi backend verziami | **Prijať** | Ide o dlhodobé riziko používateľských dát, nie iba cache. |
| Jemný dependency/read-set Proposal digest | **Prijať s rozšírením** | Musí zahŕňať aj query výsledky, globálne epochy a policy/schema verzie. |
| Oddeliť právnu/release rešerš od benchmarkov | **Prijať** | Licencia a verzia sú overiteľné vstupy, nie výkonová hypotéza. |
| Exact-vs-mesh porovnanie | **Prijať čiastočne** | Je užitočné po exact baseline; mesh nie je automatický autoritatívny fallback. |
| Rozdeliť Bránu A na A0/A1 | **Prijať** | Najrizikovejšie otázky sa musia zodpovedať skôr než save/load a adaptéry. |
| Definovať 20 kanonických úloh vopred | **Prijať** | Úlohy definujú produktový rozsah aj Intent vocabulary. |
| Licencia → solver shortlist → technický spike | **Prijať** | Technický spike licenčne neprijateľného kandidáta je plytvanie. |
| Explicitné `ExactBody/MeshBody` | **Prijať** | Autorita a strata nesmú byť implicitné. |
| Undo viazať na Intent/Proposal | **Odmietnuť v tejto forme** | Správna jednotka je commitnutý používateľský `CommandBatch`. |
| Dokumentový scope a schema version v referencii | **Prijať** | Externé assets prídu neskôr, dátový kontrakt ich nesmie znemožniť. |
| Celý dokument read-only pri jednom zhoršení referencie | **Prijať s úpravou** | Bezpečný je selektívny quarantine dotknutých vetiev; nesmie sa však nič ticho prepočítať. |
| Konkrétne navrhnuté čísla brať ako fakty | **Odmietnuť ako fakty, prijať ako preregistrované V3 prahy** | Čísla sú počiatočné rozhodnutia testu, nie univerzálne vlastnosti OCCT. |

## 17.3 Tretia recenzia V2

**Verdikt:** najsilnejšia produktová a komunikačná recenzia. Neprináša nový geometrický kontrakt, ale správne ukazuje, že dlhý dokument bez manažérskej vrstvy sa ťažko používa.

| Návrh | Stanovisko V3 | Dôvod |
|---|---|---|
| Jednostranové executive summary | **Prijať** | Umožňuje rýchlo pochopiť produkt, stack, riziká a PoC. |
| Decision table `decided/open/postponed` | **Prijať** | Rozlišuje záväzok, hypotézu, fakt a odklad. |
| Ostrejšie oddeliť záväzné od smerovania | **Prijať** | `egui`, solver, WASM a serializácia nemajú rovnakú váhu ako Rust/OCCT façade/revisions. |
| Definovať First Lovable Product a „can/cannot“ | **Prijať** | Chráni scope lepšie než všeobecná vízia. |
| Pridať kritickú cestu | **Prijať** | Bráni odbočeniu k AI demu, BIM alebo pluginom pred jadrom. |
| Zhromaždiť hard acceptance metrics | **Prijať** | PoC má teraz jednotný charter a checklist. |
| Urobiť iba V3 light | **Prijať zámer** | V3 nemení základ V2; pridáva operačnú vrstvu a opravuje konkrétne kontrakty. |
| Investor/developer skrátené varianty | **Odložiť** | Executive summary postačí teraz; samostatné varianty majú vzniknúť až pre konkrétne publikum. |

## 17.4 Vedome odmietnuté skratky

- OCCT sa nenahrádza neovereným kernelom iba kvôli pohodlnejšiemu jazyku.
- Worker nie je dogma bez benchmarku, ale crash containment sa nesmie ignorovať.
- Mesh nie je univerzálna presná náhrada B-Repu.
- TNP sa nevyhlasuje za vyriešené.
- AI Commands neodstraňujú halucinácie; ohraničujú ich dôsledky.
- WASM alebo proces automaticky nerieši licenčné povinnosti.
- Konkrétne verzie, licencie a technické vlastnosti sa bez primárnych zdrojov nevydávajú za fakty.
- Drawing, BIM, mechanical ani nature sa nevyhadzujú z vízie; iba z PoC a prvého úzkeho produktu.

---

# 18. Otvorené rozhodnutia človeka

1. konkrétna open-source licencia;
2. dostupný tím, rozpočet a runway;
3. Windows-first verzus paralelná desktopová podpora;
4. osoby priradené k rolám v registri;
5. okamih prvého verejného compatibility sľubu;
6. cloud AI a predvolený privacy režim;
7. použitie FurniGen úloh a používateľov ako validačnej domény.

Tieto rozhodnutia nesmú byť potichu urobené prvým implementátorom v CMake, CI alebo závislostiach.

---

# 19. Záver

Verzia 3 nemení základnú tézu verzie 2. Kečup zostáva Rustová, presná a AI-native modelovacia platforma s OCCT za úzkou hranicou, `wgpu` rendererom, sémantickým parametrickým dokumentom a jedinou kanonickou mutačnou cestou.

V3 však mení návrh na vykonateľný kontrakt. Určuje First Lovable Product, rozlišuje pevné rozhodnutia od hypotéz, pridáva migráciu topologických referencií, dependency-scoped Proposal validity, explicitnú pamäťovú politiku, správnu undo jednotku a zlyhateľné brány R0/A0/A1/B/C.

Najbližším technickým krokom po schválení V3 je R0 a následne A0. Nie je ním budovanie celej aplikácie, BIM, marketplace ani efektné AI demo. Ak A0 nedokáže exception-safe exact hranicu a minimálnu stabilitu referencií, Kečup musí zmeniť geometrický základ skôr, než na ňom postaví produkt.
