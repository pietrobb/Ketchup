# Kečup (Ketchup)

## Ideový a technický návrh AI-native 2D/3D modelovacej platformy

**Stav dokumentu:** počiatočný architektonický návrh na odbornú a AI oponentúru  
**Dátum:** 1. august 2026  
**Licenčný zámer projektu:** open source a bezplatné používanie  
**Pracovný názov repozitára:** `ketchup`  
**Zobrazovaný názov:** Kečup

---

## 1. Vízia

Kečup má byť rýchla, otvorená a rozšíriteľná platforma na 2D a 3D modelovanie, ktorú možno používať klasicky aj prostredníctvom AI. Dlhodobým cieľom je pokryť veľmi odlišné typy práce:

- 2D technické výkresy;
- architektonické a stavebné modely;
- presné výrobné a strojárske diely;
- interiéry a nábytok;
- voľné a organické tvary;
- rozsiahle procedurálne objekty, napríklad borovicu s kmeňom, kôrou, konármi a ihličím;
- automatizované generovanie, úpravy a kontrolu modelov pomocou AI.

Kečup nemá byť iba ďalší editor trojuholníkov. Dokument má uchovávať význam objektov, ich parametre, vzťahy a históriu vytvorenia. Stena má zostať stenou, otvor otvorom a procedurálny strom stromom, nie iba anonymnou množinou plôch.

### 1.1 Hlavné zásady

1. **AI-native, nie AI doplnok:** všetky operácie musia byť bezpečne a deterministicky dostupné rovnakým príkazovým rozhraním pre používateľské UI, automatizáciu aj AI.
2. **Sémantický parametrický dokument:** model uchováva objekty, parametre, väzby a postup tvorby.
3. **Viac geometrií pre rôzne úlohy:** presná CAD geometria, procedurálna/mesh geometria a 2D geometria sa nemajú násilne zlúčiť do jednej reprezentácie.
4. **Výkon od začiatku:** viacvláknový Rust core, inkrementálne prepočty, GPU rendering, instancing, LOD a priestorové indexy.
5. **Rozšíriteľnosť:** malé všeobecné jadro a samostatné doménové balíky pre architektúru, výrobu, prírodu, nábytok a ďalšie oblasti.
6. **Otvorený a migrovateľný formát:** versionovaná schéma, dokumentované rozhrania a žiadne uzamknutie používateľských dát v proprietárnom formáte.
7. **Bezpečná automatizácia:** AI ani nedôveryhodný plugin nesmú automaticky dostať neobmedzené spúšťanie natívneho kódu.
8. **Deterministické výsledky:** ten istý dokument, verzia jadra a príkazy majú viesť k reprodukovateľnému výsledku.

---

## 2. Čím Kečup nemá byť v prvej verzii

Prvý produkt nemôže naraz nahradiť Blender, Revit, AutoCAD, SketchUp a FreeCAD. Univerzálnosť má vzniknúť vhodnou architektúrou a rozšíreniami, nie implementáciou všetkých domén v MVP.

Prvá verzia preto nemá obsahovať:

- kompletný BIM ekosystém;
- plnohodnotnú animáciu a filmový rendering;
- simulácie všetkých fyzikálnych odborov;
- kompletný CAM systém;
- všetky profesionálne importné a exportné formáty;
- vlastný CAD kernel napísaný od nuly;
- všeobecné spúšťanie kódu vytvoreného AI.

---

## 3. Odporúčaný technologický základ

| Oblasť | Odporúčanie | Dôvod |
|---|---|---|
| Hlavný jazyk | Rust | výkon, pamäťová bezpečnosť, paralelizácia, vhodné knižničné jadro |
| Presná CAD geometria | Open CASCADE Technology za úzkym adaptérom | vyspelé B-Rep operácie, booleany, krivky, plochy a priemyselné formáty |
| Procedurálna a mesh geometria | vlastné Rust moduly + vybrané knižnice | kontrola nad dátovým layoutom, LOD, instancingom a GPU cestou |
| Rendering | `wgpu` | moderné GPU API nad DirectX 12, Vulkan a Metal |
| MVP používateľské rozhranie | `egui` | rýchly vývoj natívnych nástrojov v Ruste a jednoduchá integrácia |
| Pluginy | WebAssembly Component Model + WIT | prenositeľné, capability-based a bezpečnejšie rozšírenia |
| Externá automatizácia | versionované RPC/Command API | rovnaké funkcie pre Python, CLI, MCP aj vzdialené AI |
| Python | externé SDK, nie autoritatívne jadro | jednoduché experimenty bez oslabenia stability core |
| Dáta CPU | `f64` + explicitné jednotky | presnosť veľkých aj malých modelov |
| Dáta GPU | camera-relative `f32` | výkon bežných GPU bez viditeľnej straty presnosti |

### 3.1 Prečo Rust

Rust je vhodný na výkonné a dlhodobo udržiavateľné jadro:

- nevyžaduje garbage collector;
- chráni pred veľkou časťou chýb práce s pamäťou;
- podporuje bezpečnú paralelizáciu;
- dobre sa hodí na dátovo orientované algoritmy;
- umožňuje zostaviť natívne aplikácie pre hlavné desktopové platformy;
- je vhodný na knižničné aj serverové komponenty.

Rust sám o sebe však nezaručuje rýchlosť. Výkon bude závisieť najmä od dátového modelu, minimalizovania úplných prepočtov, batchovania GPU operácií, instancingu, LOD, profilovania a kvality geometrických algoritmov.

### 3.2 Čo je Open CASCADE

Open CASCADE Technology (OCCT) je otvorené priemyselné geometrické jadro napísané prevažne v C++. Nie je to hotový modelovací program ani renderer. Poskytuje matematické a topologické operácie potrebné pre presný CAD, napríklad:

- body, vektory, osi a transformácie;
- analytické aj NURBS krivky a plochy;
- hrany, drôty, plochy, škrupiny a telesá v B-Rep reprezentácii;
- extrúzie, rotácie, zaoblenia a skosenia;
- boolean operácie union, cut a intersect;
- trianguláciu presných telies na zobrazenie;
- čítanie a zápis vybraných CAD formátov, napríklad STEP a IGES.

OCCT je odporúčaný preto, že vytvorenie porovnateľne robustného CAD kernelu od nuly by bolo samostatným viacročným projektom. Zároveň ide o veľkú C++ knižnicu s komplikovaným API a operáciami, ktoré môžu na degenerovaných vstupoch zlyhať. Kečup preto nesmie rozptýliť OCCT typy po celom programe. Majú zostať za malým rozhraním `ExactGeometryBackend`, aby bolo možné:

- izolovať C++ FFI a pády;
- testovať a neskôr prípadne vymeniť backend;
- držať dokumentový model nezávislý od interných OCCT typov;
- vykonávať náročné alebo rizikové operácie aj v samostatnom procese.

Pred distribúciou je potrebné právne overiť presné licenčné podmienky každej použitej verzie a modulu OCCT. Architektonický návrh počíta s jeho LGPL licenciou s OCCT exception, ale toto nie je právne stanovisko.

### 3.3 Prečo `wgpu`

`wgpu` poskytuje jednotnú Rust abstrakciu nad modernými grafickými API. Umožní:

- natívny rendering cez DirectX 12, Vulkan a Metal;
- compute shadery pre vybrané geometrické a vizualizačné úlohy;
- GPU instancing opakovaných objektov;
- nepriame vykresľovanie a redukciu CPU draw-call overheadu;
- viacúrovňové LOD;
- moderný výber objektov a pomocné CAD zobrazenia.

Renderer musí byť samostatný konzument odvodených dát. Nesmie sa stať vlastníkom autoritatívneho dokumentu.

### 3.4 Prečo `egui` iba ako pragmatický začiatok

`egui` umožní rýchlo vytvoriť desktopový MVP editor, panely parametrov, strom objektov, konzolu príkazov a diagnostické nástroje. UI však musí komunikovať s jadrom iba cez Commands a Queries. Ak neskôr limity `egui` zabránia profesionálnemu desktopovému UX, bude možné UI vymeniť bez prepisovania modelovacieho jadra.

---

## 4. Navrhovaná architektúra

```text
┌──────────────────────────────────────────────────────────────┐
│ UI / CLI / Python SDK / MCP / Voice / WASM plugins          │
└───────────────────────┬──────────────────────────────────────┘
                        │ typed Commands and Queries
┌───────────────────────▼──────────────────────────────────────┐
│ Command Gateway                                              │
│ schema • permissions • validation • dry-run • transactions  │
└───────────────────────┬──────────────────────────────────────┘
                        │
┌───────────────────────▼──────────────────────────────────────┐
│ Document Core                                                │
│ entities • parameters • feature graph • scene graph • undo  │
└──────────────┬────────────────────────────┬──────────────────┘
               │                            │
┌──────────────▼──────────────┐  ┌─────────▼──────────────────┐
│ Geometry Services          │  │ Domain packages            │
│ exact • mesh • 2D • solver │  │ architecture • nature ... │
└──────────────┬──────────────┘  └────────────────────────────┘
               │ derived render data
┌──────────────▼───────────────────────────────────────────────┐
│ Renderer / Selection / LOD / GPU caches                     │
└──────────────────────────────────────────────────────────────┘
```

### 4.1 Pravidlo závislostí

- UI závisí od verejného aplikačného API, nie od geometrických detailov.
- AI závisí od Command/Query schémy, nie od interných Rust funkcií.
- Dokumentové jadro pozná abstrakcie geometrie, nie OCCT triedy.
- Renderer pracuje s odvodenými render packetmi, nie priamo s B-Rep objektmi.
- Doménové balíky skladajú všeobecné funkcie jadra, ale jadro nepozná pojmy konkrétnej domény.
- Pluginy komunikujú cez versionovaný kontrakt a deklarované capabilities.

### 4.2 Orientačný Rust workspace

```text
ketchup/
├─ crates/
│  ├─ ketchup-core/          # dokument, entity, ID, jednotky, transakcie
│  ├─ ketchup-commands/      # Commands, Queries, schémy, validácia
│  ├─ ketchup-feature-graph/ # závislosti a inkrementálny prepočet
│  ├─ ketchup-geometry-api/  # backend-neutral geometrické kontrakty
│  ├─ ketchup-geometry-occt/ # jediná vrstva poznajúca OCCT
│  ├─ ketchup-mesh/          # mesh a procedurálna geometria
│  ├─ ketchup-sketch/        # 2D geometria a constraint solver
│  ├─ ketchup-render/        # wgpu renderer, picking, LOD, cache
│  ├─ ketchup-io/            # natívny formát, migrácie, import/export
│  ├─ ketchup-plugin-host/   # WASM Component Model, capabilities
│  ├─ ketchup-ai-gateway/    # AI tool definitions a bezpečnostná politika
│  ├─ ketchup-server/        # voliteľný lokálny RPC proces
│  └─ ketchup-app/           # desktopová aplikácia
├─ wit/                      # kontrakty WASM komponentov
├─ schemas/                  # versionované command/document schémy
├─ sdk/
│  └─ python/                # externý klient rovnakého API
├─ domains/
│  ├─ architecture/
│  ├─ furniture/
│  └─ nature/
└─ tests/
   ├─ golden/
   ├─ roundtrip/
   ├─ geometry/
   └─ benchmarks/
```

Toto je cieľové členenie, nie požiadavka vytvoriť všetky balíky hneď v MVP.

---

## 5. Dokumentový model

### 5.1 Tri odlišné grafy

Kečup potrebuje odlíšiť minimálne tri pohľady:

1. **Feature/dependency graph** – čo bolo z čoho vytvorené a čo sa musí po zmene prepočítať.
2. **Scene graph** – hierarchia umiestnenia, transformácií, viditeľnosti a zoskupenia.
3. **Render graph** – poradie a závislosti GPU renderovacích passov.

Tieto grafy spolu súvisia, ale nie sú totožné. Zmiešanie do jednej stromovej štruktúry by neskôr obmedzilo parametrické modelovanie aj rendering.

### 5.2 Entity a stabilné identifikátory

Každá entita má stabilné ID nezávislé od názvu a poradia v UI. Entita môže obsahovať:

- typ a versionovanú schému;
- používateľský názov a metadata;
- lokálnu transformáciu;
- parametre s jednotkami;
- referencie na vstupné entity alebo subelementy;
- materiál a zobrazovacie vlastnosti;
- doménové komponenty;
- odvodený stav geometrie a diagnostiku.

ID nesmú závisieť od pozície v poli ani od dočasného indexu v OCCT alebo GPU buffri.

### 5.3 Parametrické features

Príklady všeobecných features:

- sketch;
- extrude;
- revolve;
- sweep;
- loft;
- boolean union/cut/intersect;
- fillet/chamfer;
- pattern/array;
- transform;
- procedural generator;
- imported reference.

Zmena parametra označí iba závislé uzly ako neaktuálne. Scheduler prepočíta minimálnu potrebnú časť grafu a nezmenené výsledky ostanú v cache.

### 5.4 Jednotky a súradnice

Autoritatívne geometrické hodnoty majú byť `f64`. Každá verejná hodnota dĺžky, uhla, plochy alebo objemu musí mať explicitnú fyzikálnu jednotku alebo typ, ktorý zabráni náhodnému miešaniu jednotiek.

Dokument môže mať prezentačnú jednotku, ale interné príkazy nemajú obsahovať nejasné číslo typu `10` bez informácie, či ide o milimetre alebo metre.

Pre rozsiahle modely renderer používa camera-relative origin:

- CPU uchováva svetové pozície v `f64`;
- pred odovzdaním GPU sa odčíta blízky lokálny počiatok;
- GPU spracúva relatívne hodnoty v efektívnom `f32` rozsahu.

#### 5.4.1 Precision contract

Kečup nesmie zamieňať presne zadaný rozmer s približným výsledkom numerického geometrického výpočtu. Používateľský zámer, napríklad `2400 mm`, sa má uložiť ako kanonická rozmerová hodnota bez zaokrúhľovania pri každom načítaní a uložení. Geometrický backend môže pri prienikoch, NURBS a boolean operáciách používať `f64` a riadené tolerancie, ale nesmie potichu meniť autoritatívny parameter.

Projekt preto potrebuje jednotnú politiku presnosti:

- kanonické rozmerové parametre s explicitnými jednotkami a stabilným round-trip zápisom;
- `f64` pre CPU geometriu, lokálne súradnicové systémy a camera-relative rendering;
- modelový rozsah a odporúčanú pracovnú mierku definované a testované od začiatku;
- centrálne spravované absolútne a relatívne tolerancie namiesto náhodných `epsilon` konštánt v moduloch;
- toleranciu priradenú geometrickým výsledkom a diagnostiku, keď ju operácia zhorší;
- adaptívne alebo exaktné predikáty tam, kde rozhodujú o orientácii, incidencii či topológii;
- zachovanie parametrického zdroja namiesto opakovaného deštruktívneho prepisovania už aproximovanej geometrie;
- bezstratový save/load round trip rozmerov, transformácií, jednotiek a constraintov;
- testy veľmi malých detailov, veľkých súradníc, zmiešaných mierok a opakovaných úprav.

Žiadny všeobecný CAD kernel negarantuje nekonečnú matematickú presnosť všetkých prienikov. Cieľom je, aby zadané rozmery zostali autoritatívne a reprodukovateľné, analytická geometria zostala analytická, aproximácie boli kontrolované a každá strata presnosti bola merateľná a oznámená. Georeferencované BIM modely majú používať oddelenie globálneho umiestnenia od lokálnych súradníc budovy, aby veľké mapové súradnice neznižovali lokálnu presnosť.

### 5.5 Topological naming

Toto je jedno z najväčších rizík parametrického CAD. Neskorší feature alebo AI nesmie trvalo odkazovať na `face[7]`, pretože po boolean operácii či zmene rozmeru môže mať tá istá logická plocha iný interný index.

Navrhovaná referencia má kombinovať:

- pôvod vo feature grafe;
- sémantickú rolu, napríklad „horná plocha extrúzie“;
- históriu mapovania topológie z backendu;
- geometrické vlastnosti a tolerančný fallback;
- jednoznačnú diagnostiku pri strate alebo nejednoznačnosti referencie.

Topological naming treba testovať už v prvom proof-of-concepte. Nemožno ho odložiť ako detail na koniec vývoja.

### 5.6 Skupiny, komponenty, tagy a klasifikácia

Tieto pojmy nesmú byť zlúčené do jednej nejasnej „layer“ vlastnosti:

- **skupina** vytvára hierarchiu, lokálny súradnicový systém a izolačný kontext editovania;
- **component definition** uchováva jednu zdieľanú definíciu geometrie a sémantiky;
- **component instance** odkazuje na definíciu a má vlastnú transformáciu, prípadne povolené parametrické alebo materiálové override hodnoty;
- **tag/layer** riadi najmä viditeľnosť, filtrovanie, výber a štýl vo viewporte či výkrese, nie vlastníctvo geometrie;
- **collection** predstavuje používateľský alebo pracovný výber objektov bez zmeny ich hierarchie;
- **classification/category** vyjadruje sémantický typ, napríklad stena, nábytok alebo nosný prvok;
- **saved view/visibility set** uchováva kameru, rezy, viditeľnosť, štýl a účel konkrétneho pohľadu.

Scene graph už s hierarchiou a instancingom počíta, ale tieto objekty musia byť explicitnou súčasťou verejného dokumentového modelu a Command/Query API. Úprava zdieľanej definície má aktualizovať všetky inštancie bez duplikovania geometrie; používateľ zároveň musí vedieť inštanciu vedome odpojiť alebo vytvoriť variant.

---

## 6. Geometrická architektúra

### 6.1 Presná B-Rep geometria

Vhodná pre:

- výrobné diely;
- presné stavebné prvky;
- analytické krivky a plochy;
- booleany a presné rozmery;
- STEP/IGES výmenu.

Autoritatívnym výsledkom môže byť B-Rep, z ktorého sa podľa potrebnej tolerancie vytvorí triangulácia pre viewport.

### 6.2 Procedurálna a mesh geometria

Vhodná pre:

- stromy, porasty a terén;
- detailnú kôru a ihličie;
- organické modelovanie;
- scan dáta;
- rozsiahle opakované objekty;
- vizualizačné detaily, ktoré by v B-Rep boli neúmerne drahé.

Borovica nemá byť milión samostatných CAD telies. Má sa skladať zo sémantického procedurálneho receptu, kompaktnej kostry a GPU inštancií vetiev či ihličia. Detail sa generuje podľa vzdialenosti kamery, účelu exportu a nastavenia kvality.

### 6.3 2D geometria a constraint solver

2D skica potrebuje vlastné primitíva a väzby:

- bod, úsečka, oblúk, kružnica, spline;
- horizontála, vertikála, rovnobežnosť, kolmosť;
- dotyk, súososť, rovnaká dĺžka alebo polomer;
- rozmerové väzby;
- diagnostiku stupňov voľnosti, preurčenia a konfliktov.

Solver je samostatné riziko. Pre proof-of-concept treba porovnať integráciu existujúceho solvera alebo algoritmov so samostatnou implementáciou. Rozhranie solvera nemá byť previazané s UI.

### 6.4 Prepojenie reprezentácií

Reprezentácie sa môžu odvodiť jedna z druhej, ale každá musí mať jasne označený autoritatívny zdroj:

- B-Rep → render mesh;
- procedural recipe → LOD meshes alebo GPU instances;
- sketch → profil pre exact feature;
- importovaný mesh → mesh objekt, nie predstierané presné CAD teleso;
- voliteľná rekonštrukcia mesh → B-Rep je explicitná a potenciálne stratová operácia.

### 6.5 Priame modelovanie a lepší Push/Pull

Jednoduchosť SketchUpu treba zachovať, ale operácia nemá byť iba deštruktívnym posúvaním anonymných plôch. Základná interakcia má fungovať takto:

1. používateľ ukáže na plochu alebo uzavretý 2D región;
2. systém zobrazí okamžitý náhľad a dočasný presný rozmer;
3. vzdialenosť možno určiť ťahaním, snapom, klávesnicou, parametrom alebo hlasom;
4. inference engine ponúka rovnobežnosť, kolmosť, zarovnanie, symetriu, rovnakú výšku a väzbu na existujúcu geometriu;
5. jadro vytvorí alebo upraví parametrický feature cez ten istý Command protokol;
6. výsledok zostane spätne editovateľný a zobrazí, či ide o pridanie, odobratie, offset alebo nové teleso.

„Smart Push/Pull“ má podľa kontextu zvoliť bezpečné správanie:

- na koncovej ploche jednoduchej extrúzie zmeniť jej pôvodný rozmer, ak je zámer jednoznačný;
- na inej planárnej ploche vytvoriť nový offset/extrude feature;
- pri zatlačení do telesa ponúknuť parametrický cut alebo otvor;
- pri zložitej či nejednoznačnej histórii ukázať voľby namiesto tichej deštrukcie modelu.

Push/Pull má dopĺňať priame manipulátory, kontextové rukoväti, skicu priamo na ploche, numerický HUD pri kurzore, property panel, command palette a AI/hlasové pokyny. Všetky vstupy majú vytvoriť rovnaké typované Commands, takže jednoduché ručné ovládanie a presné parametrické modelovanie nebudú dva oddelené systémy.

---

## 7. Command a Query protokol

### 7.1 Jediná cesta pre všetky zmeny

Každá editácia dokumentu prechádza typovaným príkazom. UI nesmie potajme meniť interné štruktúry inak ako AI alebo plugin. Rovnaký protokol používajú:

- natívne UI;
- klávesové skratky a makrá;
- CLI;
- WASM pluginy;
- Python SDK;
- MCP adaptér;
- lokálny alebo vzdialený AI model;
- hlasový vstup po prevode reči na zámer.

### 7.2 Vlastnosti príkazu

Každý Command má mať:

- versionovaný názov a schému;
- stabilné ID príkazu a korelačné ID;
- explicitné jednotky;
- vstupné entity a preconditions;
- deklarovaný alebo vypočítaný zoznam ovplyvnených entít;
- oprávnenia/capabilities;
- validáciu bez modifikácie dokumentu;
- deterministický výsledok alebo štruktúrovanú chybu;
- auditovateľný záznam.

Ilustračný príklad, nie finálna schéma:

```json
{
  "schema": "ketchup.command/v1",
  "command": "feature.extrude.create",
  "id": "cmd_01J...",
  "document_revision": 42,
  "input": {
    "profile": { "entity_id": "ent_sketch_7", "region": "closed_region_1" },
    "distance": { "value": 2400.0, "unit": "mm" },
    "direction": "profile_normal",
    "operation": "new_body"
  },
  "preconditions": [
    { "kind": "entity_exists", "entity_id": "ent_sketch_7" },
    { "kind": "profile_is_closed", "region": "closed_region_1" }
  ]
}
```

### 7.3 Transakcie, undo a redo

`CommandBatch` je atómová transakcia:

1. overenie schémy a oprávnení;
2. vyhodnotenie preconditions;
3. dry-run na pracovnom stave;
4. geometrická a doménová validácia;
5. commit celej dávky alebo rollback;
6. vytvorenie záznamu pre undo/redo a audit.

Undo/redo nemá byť doplnok UI. Má byť základná vlastnosť dokumentových transakcií. Pri veľkých dátach môže byť implementované kombináciou inverzných príkazov, persistentných dátových štruktúr, snapshotov a content-addressed blobov.

### 7.4 Queries

Queries sú read-only a nevyžadujú undo. AI aj UI potrebujú štruktúrovane zisťovať napríklad:

- výber a viditeľné objekty;
- strom a závislosti dokumentu;
- parametre a jednotky;
- bounding box, plochu, objem a ťažisko;
- typ a sémantické vlastnosti objektu;
- chyby feature grafu;
- možné referencie na hrany a plochy;
- náhľad z kamery;
- rozdiel medzi aktuálnou a navrhovanou revíziou.

---

## 8. AI-native workflow

### 8.1 Základný cyklus

AI nemá z textu okamžite vytvárať neoverený model. Odporúčaný cyklus:

1. používateľ slovom alebo textom opíše cieľ;
2. AI cez Queries preskúma dokument, výber, jednotky a dostupné nástroje;
3. pri nejasnostiach položí cielenú otázku alebo uvedie predpoklady;
4. zostaví plán z typovaných Commands;
5. vykoná dry-run;
6. systém skontroluje schému, jednotky, topológiu, kolízie a doménové pravidlá;
7. AI dostane štruktúrovaný výsledok a podľa potreby aj obrázkový náhľad;
8. pri rizikovej, deštruktívnej alebo nákladnej zmene si vyžiada potvrdenie;
9. potvrdí transakciu;
10. overí výsledný dokument a vysvetlí zmenu používateľovi.

### 8.2 Bezpečnostné hranice

AI štandardne nesmie:

- spúšťať ľubovoľný Rust, Python, Ruby alebo shell kód;
- čítať a zapisovať ľubovoľné súbory mimo povoleného priestoru;
- inštalovať pluginy bez vedomia používateľa;
- automaticky potvrdiť deštruktívne externé operácie;
- obísť transakcie, validáciu alebo audit log.

AI má dostať menovanú množinu nástrojov s JSON Schema/WIT kontraktom a minimálnymi potrebnými oprávneniami.

### 8.3 AI nie je autoritatívny geometrický engine

LLM rozhoduje o zámere a skladá operácie. Rozmery, transformácie, prieniky, topológiu a fyzikálne vlastnosti má počítať deterministické jadro. Výsledok nemá závisieť od toho, či LLM správne mentálne vypočítal trigonometrický vzťah.

### 8.4 Vizuálna spätná väzba

AI môže popri štruktúrovaných Queries dostať:

- PNG náhľad z definovaných kamier;
- depth/normal/object-ID buffer;
- zvýraznenie zmenených objektov;
- diagnostické prekrytie kolízií a chýb;
- jednoduchý scene summary.

Obrázok je doplnok, nie náhrada presných dát a validácie.

### 8.5 Hlasové ovládanie

Hlas je iba vstupný adaptér:

```text
reč → prepis → interpretácia zámeru → Queries/Commands → dry-run → potvrdenie
```

Všetky bezpečnostné a transakčné pravidlá zostávajú rovnaké ako pri textovom AI ovládaní.

---

## 9. Pluginový systém

### 9.1 Dve úrovne rozšírení

**Bezpečné prenositeľné pluginy:**

- WebAssembly Component Model;
- WIT kontrakty;
- deklarované capabilities;
- obmedzený prístup k súborom, sieti, GPU a dokumentu;
- versionované rozhranie a kontrolované zdroje.

**Dôveryhodné natívne backendy:**

- geometrické jadrá, ovládače alebo výkonné importéry;
- jasne oddelené ABI alebo procesné RPC;
- používateľ musí vedieť, že ide o natívny dôveryhodný komponent;
- pri rizikových knižniciach preferovať izolovaný worker proces.

### 9.2 Doménové balíky

Univerzálne jadro nemá priamo implementovať všetky stavebné a prírodné objekty. Doménový balík môže pridávať:

- nové entity a ich schémy;
- parametrické generators/features;
- validátory a výpočty;
- UI panely;
- AI tools a odborný slovník;
- importéry/exportéry;
- knižnice materiálov a komponentov.

Príklady:

- `architecture`: steny, miestnosti, otvory, podlažia;
- `furniture`: skrinky, kovania, rezné zoznamy;
- `nature`: stromy, porasty, terén, L-systems;
- `mechanical`: diely, zostavy, tolerancie;
- `drawing`: výkresové pohľady, kóty, značky.

### 9.3 Modul projektovej dokumentácie

Profesionálny výstup má byť samostatný doménový modul nad tým istým dokumentom, nie export screenshotu. Musí podporovať:

- výkresové listy, formáty papiera, rámčeky, rohové pečiatky a používateľské šablóny;
- asociatívne pôdorysy, rezy, pohľady, axonometrie, detaily a výrezy odvodené z modelu;
- mierku pohľadu, skryté hrany, rezové čiary, hĺbkové zoslabovanie a vektorový hidden-line rendering;
- presné kóty, výškové značky, osi, popisy, symboly, legendy, šrafy a materiálové skladby;
- pravidlá hrúbok, typov a farieb čiar podľa kategórie, rezu, mierky a fázy projektu;
- visibility sets, grafické override pravidlá a samostatné anotácie konkrétneho výkresu;
- tabuľky, výkazy prvkov, plôch, objemov a množstiev napojené na model;
- revízie, issue značky a kontrolu neaktuálnych pohľadov;
- kvalitný vektorový PDF/SVG výstup a podľa možností DXF; rastrový výstup iba tam, kde je potrebný.

Pohľady, kóty a výkazy majú byť asociatívne: po zmene modelu sa prepočítajú alebo jasne označia ako neaktuálne. Vzhľad musí byť prispôsobiteľný cez versionované šablóny a štýlové pravidlá, aby kancelárie nemuseli upravovať každý list ručne.

### 9.4 BIM podpora

BIM nemá byť vlastnosť každého objektu v základnom geometrickom jadre, ale prvotriedny doménový balík postavený na všeobecnom entity/component modeli. Má obsahovať:

- stavebné elementy ako stena, doska, strecha, nosník, stĺp, dvere, okno a schodisko;
- podlažia, priestory, zóny, systémy, typy a inštancie;
- property sets, klasifikácie, fázy, materiálové skladby a stabilné externé GUID;
- explicitné vzťahy hostiteľ–otvor–výplň, priestorové hranice a napojenia prvkov;
- výpočty množstiev, plôch, objemov a kontrolné pravidlá;
- georeferenciu oddelenú od presných lokálnych súradníc budovy;
- IFC import/export s jasným reportom strát a round-trip testami;
- neskôr BCF/issues, federované referenčné modely a ďalšie odborové výmeny.

Najspoľahlivejší BIM vzniká priamo zo sémantických prvkov. AI môže rozpoznať steny, miestnosti či otvory vo všeobecnej geometrii a navrhnúť ich konverziu, ale taký „automatický BIM“ musí ukázať neistoty a vyžiadať potvrdenie. Samotný tvar nestačí na spoľahlivé odvodenie všetkých stavebných vlastností.

---

## 10. Rendering a výkon

### 10.1 Výkonnostné princípy

- oddeliť autoritatívny model od odvodených GPU cache;
- prepočítavať iba neaktuálne uzly feature grafu;
- geometrické úlohy plánovať paralelne podľa závislostí;
- opakované objekty vykresľovať cez instancing;
- používať LOD a culling;
- veľké scény deliť priestorovým indexom/BVH;
- tesselláciu cacheovať podľa geometrie a tolerancie;
- minimalizovať synchronizáciu CPU ↔ GPU;
- používať content-addressed cache pre veľké odvodené dáta;
- náročné exact operácie spúšťať asynchrónne s možnosťou zrušenia;
- pravidelne profilovať reálne scény, nie iba mikrobenchmarky.

### 10.2 Očakávania

Navrhnutý stack môže byť veľmi rýchly, ale samotná voľba Rustu, OCCT a `wgpu` to negarantuje. OCCT boolean alebo fillet na zložitej geometrii môže byť pomalý či neúspešný. Naopak, milión ihličiek sa dá zobrazovať efektívne, ak nie sú miliónom plných dokumentových a B-Rep objektov, ale GPU inštanciami s LOD.

Preto treba oddelene benchmarkovať:

- latenciu bežných Commands;
- inkrementálny prepočet feature grafu;
- exact booleany a fillets;
- tesselláciu B-Rep;
- načítanie a uloženie dokumentu;
- viewport FPS a frame time;
- výber objektov;
- spotrebu RAM a VRAM;
- procedurálny strom pri rôznych úrovniach detailu;
- režijné náklady WASM a externého RPC.

### 10.3 Predbežné výkonnostné ciele pre proof-of-concept

Tieto čísla sú pracovné ciele, nie sľub produktu:

- plynulý viewport pri 60 FPS na bežnej pracovnej stanici pre testovaciu scénu;
- interakcie kamery bez čakania na prepočet presnej geometrie;
- viditeľná odozva jednoduchého lokálneho editovania približne do 100 ms;
- progres a zrušenie pri dlhších operáciách;
- žiadny úplný prepočet dokumentu pri lokálnej zmene bez závislostí;
- procedurálny strom s veľmi veľkým počtom vizuálnych prvkov bez rovnako veľkého počtu dokumentových entít.

Konkrétne testovacie hardvéry a veľkosti scén musí určiť benchmark plán.

---

## 11. Súborový formát a interoperabilita

### 11.1 Natívny formát

Natívny dokument má byť kontajner, nie jeden nekontrolovaný JSON súbor. Možná štruktúra:

```text
model.ketchup
├─ manifest.json
├─ document.bin
├─ commands.log
├─ blobs/<content-hash>
├─ previews/
└─ extensions/<namespace>/
```

Požiadavky:

- jasná verzia formátu a použitých schém;
- migračná cesta medzi verziami;
- oddelenie malých metadát od veľkých binárnych blobov;
- checksums a detekcia poškodenia;
- atomické uloženie;
- možnosť obnovy po páde;
- zdokumentovaný minimálny interoperabilný formát;
- ignorovanie neznámych voliteľných rozšírení bez straty ich dát, ak je to bezpečné.

Výber konkrétnej serializácie (`serde`, MessagePack, CBOR, FlatBuffers, Cap'n Proto alebo iná) má nasledovať až po prototype, meraniach a analýze migrácií. Verejná Command schéma môže byť JSON-compatible aj vtedy, keď je interný dokument binárny.

### 11.2 Import a export

Prvé priority by mali byť malé a realistické:

- OBJ alebo glTF pre mesh výmenu;
- STEP cez OCCT pre presnú CAD výmenu;
- SVG/DXF podmnožina pre 2D podľa potrieb MVP;
- PNG pre náhľady.

Importované cudzie formáty nemusia zachovať celý parametrický feature graph. Straty sa majú používateľovi jasne oznámiť.

---

## 12. Použiteľné lekcie z FurniGenu

Preskúmaný bol existujúci projekt FurniGen v `C:\Users\peter\PycharmProjects\FurniGen`, ktorý AI používa na generovanie nábytku v SketchUpe.

### 12.1 Čo prevziať ako princíp

- parametrické building blocks namiesto ručného kreslenia každého detailu;
- explicitné rozmery, materiály a sémantické objekty;
- tool calling, pri ktorom výpočty robia deterministické funkcie, nie LLM;
- uzavretú slučku vytvorenie → náhľad → diagnostika → oprava;
- queryable scénu, ktorú AI vie preskúmať;
- perzistentný bridge s atómovými operáciami a rollbackom;
- oddelenie používateľského zámeru od nízkoúrovňových príkazov hostiteľskej aplikácie.

### 12.2 Čo nepreberať do univerzálneho jadra

- závislosť od SketchUpu;
- generovanie Ruby kódu ako hlavný AI protokol;
- YAML ako jediný kanonický editovateľný model;
- nábytkové pojmy zabudované do všeobecného core;
- axis-aligned bounding box ako náhradu skutočnej geometrie;
- neobmedzené vykonávanie kódu vytvoreného modelom.

FurniGen sa neskôr môže zmeniť na doménový balík `furniture` nad Kečupom.

---

## 13. Hlavné technické riziká

| Riziko | Dôsledok | Navrhované zmiernenie |
|---|---|---|
| Topological naming | features sa po úprave odpoja od správnych plôch a hrán | sémantické referencie, história mapovania, fallback a regresné testy od PoC |
| Robustnosť CAD operácií | booleany, fillets alebo importy zlyhajú | izolovaný backend, tolerančná politika, diagnostika, corpus reálnych modelov |
| Constraint solver | nestabilné alebo nevysvetliteľné skice | samostatné rozhranie, porovnanie existujúcich riešení, diagnostika DoF/konfliktov |
| Príliš široký rozsah | projekt nikdy nedokončí použiteľný editor | malé vertikálne MVP a doménové pluginy |
| Kombinovanie B-Rep a mesh | strata presnosti alebo extrémne náklady | jasný autoritatívny typ a explicitné konverzie |
| Výkon veľkých scén | nízke FPS a vysoká RAM | instancing, LOD, BVH, streaming a cache |
| C++ FFI | pády alebo prenikanie OCCT detailov | úzky adapter, safe wrapper, voliteľný worker proces |
| Nestabilné plugin ABI | rozšírenia sa pri každej verzii rozbijú | WIT kontrakty, semver, compatibility test suite |
| Nedôveryhodná AI | strata dát alebo nebezpečné akcie | capabilities, dry-run, transakcie, potvrdenia a audit |
| Formát dát | nečitateľné staré projekty | versionovanie, migrácie, golden a round-trip testy |
| Kompatibilita CAD formátov | strata sémantiky pri výmene | realisticky dokumentované úrovne kompatibility |
| Licencie závislostí | problém s open-source distribúciou | licenčný audit pred prijatím kľúčových knižníc |

---

## 14. Realistický postup vývoja

### Fáza 0 – technický proof-of-concept

Cieľom nie je pekný editor, ale zníženie najväčších rizík.

Minimálny demonštrátor:

1. Rust workspace a základné rozhranie dokumentu;
2. vytvorenie skice alebo jednoduchého profilu;
3. exact extrude cez OCCT adapter;
4. tessellácia a zobrazenie cez `wgpu`;
5. výber entity vo viewporte;
6. priame Push/Pull upravujúce rozmer cez rovnaký typovaný Command ako numerický panel;
7. inkrementálny prepočet závislého feature;
8. undo/redo transakcie;
9. uloženie a znovunačítanie dokumentu;
10. externý JSON/RPC príkaz vykonávajúci tú istú operáciu ako UI;
11. základný test stabilnej referencie na plochu po zmene rozmeru;
12. jednoduchý procedurálny objekt zobrazený instancingom;
13. precision corpus dokazujúci bezstratové zachovanie rozmerov po save/load a stabilné správanie na malých detailoch i veľkých súradniciach.

**Go/no-go otázky:**

- Dá sa OCCT spoľahlivo a udržiavateľne izolovať za Rust API?
- Funguje prepočet a topological naming aspoň na definovanej sade príkladov?
- Je renderer responzívny aj počas geometrických výpočtov?
- Je Command protokol dostatočný pre UI aj externého AI klienta?
- Je možné uložiť a reprodukovať rovnaký dokument deterministicky?

### Fáza 1 – modelovacie MVP

- desktopová aplikácia;
- výber, kamera, transformácie a základný strom dokumentu;
- skupiny, zdieľané komponenty/inštancie a tagy viditeľnosti;
- 2D sketch s malou sadou constraints;
- Smart Push/Pull, extrude, revolve a základné booleany;
- parametre a jednotky;
- undo/redo;
- natívne uloženie;
- jeden exact import/export a jeden mesh export;
- lokálny Command/Query endpoint;
- textový AI asistent s inspect → plan → dry-run → commit slučkou.

### Fáza 2 – rozšíriteľnosť

- stabilizované verejné schémy;
- WASM plugin host a WIT SDK;
- Python SDK;
- MCP adaptér;
- capabilities a podpisovanie/distribúcia pluginov;
- prvý doménový balík `architecture` so základnými stenami, otvormi, podlažiami a priestormi;
- základ modulu `drawing`: asociatívny pôdorys/rez, kóty, šablóna listu a vektorový PDF/SVG výstup;
- benchmark a compatibility suite.

### Fáza 3 – veľké a procedurálne scény

- pokročilé instancing a LOD;
- streaming a priestorové delenie;
- procedurálne grafy;
- doména nature;
- demonštračná borovica s parametrickou kostrou, kôrou a GPU ihličím;
- export detailu podľa cieľového použitia.

### Fáza 4 – profesionálne pracovné postupy

Až podľa reálneho používania:

- pokročilý BIM vrátane IFC, klasifikácií, validácie a výkazov;
- spolupráca a verzovanie;
- profesionálne viaclistové výkresy, kancelárske šablóny a revízie;
- rozšírená výmena dát;
- zostavy, varianty a konfigurácie;
- cloudové výpočty bez povinnej cloudovej závislosti desktopu.

---

## 15. Testovacia stratégia

### 15.1 Determinizmus a Command protokol

- schema validation testy;
- rovnaká dávka príkazov → rovnaký výsledok;
- transaction rollback pri chybe uprostred dávky;
- undo/redo round trip;
- precondition conflict pri zastaranej revízii;
- audit log bez citlivých údajov.

### 15.2 Geometria

- golden modely jednoduchých aj degenerovaných vstupov;
- property-based testy transformácií a jednotiek;
- kanonický round trip používateľských rozmerov bez driftu;
- malé detaily, veľké súradnice, zmiešané mierky a opakované parametrické zmeny;
- kontrola centrálnej tolerančnej politiky a diagnostiky degradovanej presnosti;
- volume/area sanity checks;
- boolean corpus z reálnych modelov;
- topological naming regresie pri zmene parametrov;
- tessellation tolerance testy;
- import → export → import round trips, kde je to zmysluplné.

### 15.3 Rendering

- screenshot/regression testy na podporovaných backendov;
- picking a object-ID testy;
- veľké súradnice a camera-relative rendering;
- instancing a LOD pre rozsiahle scény;
- frame-time benchmarky, nie iba priemerné FPS.

### 15.4 Pluginy a bezpečnosť

- capability denial testy;
- obmedzenia času, pamäte a počtu operácií;
- nekompatibilná verzia kontraktu;
- škodlivé alebo poškodené WASM moduly;
- fuzzing parserov a FFI hraníc;
- izolácia pádu geometrického worker procesu.

### 15.5 AI

- AI musí používať iba deklarované nástroje;
- nejednoznačné jednotky musia viesť k otázke alebo explicitnému predpokladu;
- deštruktívny plán nesmie byť automaticky commitnutý;
- dry-run a finálny commit musia odhaliť konflikt revízie;
- testovacia sada prirodzených požiadaviek s očakávaným Command plánom;
- hodnotenie správnosti výsledného dokumentu deterministickými kontrolami, nie iba textom LLM.

---

## 16. Otvorené rozhodnutia

Nasledujúce body návrh zámerne ešte neuzatvára:

1. Presný Rust ↔ OCCT binding a či exact backend bude in-process alebo worker process.
2. Konkrétny 2D constraint solver.
3. Interná serializácia natívneho dokumentu.
4. Dlhodobý UI framework po MVP.
5. Presný model persistentných dát a undo/redo pre veľmi veľké dokumenty.
6. Topological naming algoritmus a hranice garantovanej stability.
7. Presný rozsah prvého balíka `architecture` a hranica medzi základnou dokumentáciou vo Fáze 2 a profesionálnym BIM vo Fáze 4.
8. Konkrétne licencie samotného projektu a plugin marketplace pravidlá.
9. Minimálne podporované GPU, operačné systémy a benchmark hardvér.
10. Miera kompatibility so STEP, DXF, glTF a ďalšími formátmi.
11. Či AI gateway bude súčasť desktopového procesu alebo samostatná lokálna služba.
12. Model spolupráce viacerých používateľov a konfliktov dokumentu v budúcnosti.

---

## 17. Kritériá dobrého základu

Architektúra je vhodná, ak proof-of-concept preukáže, že:

- rovnaký príkaz možno vyvolať z UI aj externého AI klienta;
- príkaz možno bezpečne validovať, simulovať, commitnúť a vrátiť späť;
- zmena jedného parametra nevyžaduje prepočet nezávislého zvyšku modelu;
- presná CAD geometria a procedurálne GPU dáta môžu žiť v jednom dokumente bez predstierania, že sú rovnakým typom;
- OCCT možno nahradiť alebo izolovať bez zmeny celého dokumentového modelu;
- veľký procedurálny objekt ostáva interaktívny vďaka instancingu a LOD;
- starší testovací dokument sa po migrácii načíta bez tichej straty dát;
- plugin bez oprávnenia nedokáže zapisovať súbory ani meniť dokument mimo transakcie;
- AI nedokáže obísť Commands, validáciu a potvrdenie rizikových operácií.

---

## 18. Otázky pre nezávislú AI a odbornú oponentúru

Prosíme posúdiť návrh kriticky, nie iba potvrdiť jeho správnosť.

### Architektúra

1. Ktoré hranice modulov sú nesprávne alebo príliš komplikované?
2. Je oddelenie document core, geometry backendov, rendereru a domain packages dostatočné?
3. Existuje jednoduchší návrh, ktorý zachová AI bezpečnosť aj rozšíriteľnosť?
4. Kde hrozí zbytočné kopírovanie dát alebo vysoké režijné náklady?

### Geometria

5. Je Open CASCADE najvhodnejší exact kernel pre tento cieľ? Porovnajte ho s realistickými alternatívami podľa robustnosti, licencie, integrácie s Rustom a formátov.
6. Má byť OCCT v procese alebo za RPC workerom?
7. Ako konkrétne navrhnúť topological naming tak, aby bol uskutočniteľný v malom tíme?
8. Ktorý constraint solver alebo algoritmus má najlepší pomer licencie, kvality a integračného rizika?
9. Ktoré konverzie medzi B-Rep, mesh a procedural reprezentáciou majú byť podporované v MVP?

### Výkon

10. Sú camera-relative `f32`, instancing, LOD, BVH a inkrementálne výpočty dostatočný základ?
11. Kde budú pravdepodobné najväčšie úzke miesta?
12. Aké konkrétne benchmark scény, hardvér a limity majú rozhodnúť go/no-go?
13. Je Rust + `wgpu` vhodný aj pre veľmi veľké technické a prírodné scény?

### AI a bezpečnosť

14. Je jednotný Command/Query protokol vhodný pre UI, pluginy aj AI, alebo treba oddeliť verejný intent protokol od nízkoúrovňových Commands?
15. Ako navrhnúť dry-run, diff a potvrdenie, aby boli bezpečné a zároveň používateľsky rýchle?
16. Aké nové útoky prinášajú AI tool calling, MCP a pluginy pracujúce s nedôveryhodnými dokumentmi?
17. Ktoré operácie musia vždy vyžadovať potvrdenie používateľa?

### Formát a ekosystém

18. Aký natívny kontajner a serializáciu zvoliť pre dlhodobú migráciu a veľké dáta?
19. Je WASM Component Model dostatočne zrelý ako hlavné pluginové API?
20. Aký minimálny verejný kontrakt treba stabilizovať pred otvorením pluginového ekosystému?
21. Aká open-source licencia najlepšie podporí komunitu a zároveň zabráni proprietárnemu uzamknutiu ekosystému?

### Rozsah a realizácia

22. Je navrhnutý proof-of-concept stále príliš široký?
23. Čo treba z PoC odstrániť a čo naopak musí byť overené skôr, než sa začne UI produktu?
24. Ktoré rozhodnutie je najpravdepodobnejšie slepá ulička?
25. Navrhnite alternatívne etapy pre malý tím a označte kritickú cestu.

---

## 19. Požadovaný formát oponentúry

Pre ľahké porovnanie odpovedí odporúčame, aby každý hodnotiaci AI model dodal:

1. **Stručný verdikt** – či je architektúra životaschopná a prečo.
2. **Päť najlepších rozhodnutí** – čo zachovať.
3. **Päť najväčších problémov** – zoradených podľa rizika.
4. **Navrhované zmeny** – konkrétne, nie všeobecné odporúčania.
5. **Alternatívny stack** – iba ak je preukázateľne vhodnejší.
6. **Zúžený proof-of-concept** – funkcie, akceptačné kritériá a benchmarky.
7. **Bezpečnostná analýza AI a pluginov.**
8. **Otázky, ktoré treba rozhodnúť s človekom.**
9. Pri tvrdeniach o aktuálnych knižniciach uviesť **verziu, dátum a overiteľný zdroj**.
10. Jasne oddeliť **fakty, odhady a názory**.

---

## 20. Záver

Odporúčaný základ Kečupu je Rust core, Open CASCADE za úzkym exact-geometry adaptérom, `wgpu` renderer, pragmatické `egui` UI pre MVP, oddelené B-Rep/mesh/2D reprezentácie, parametrický feature graph a jednotný transakčný Command/Query protokol. WASM pluginy, externé Python/MCP rozhranie a AI gateway majú používať tie isté bezpečné a versionované operácie.

Tento návrh je architektonická hypotéza, nie potvrdenie, že celý systém bude automaticky rýchly a robustný. Najbližší technický krok má byť malý proof-of-concept, ktorý zmeria integráciu OCCT s Rustom, topological naming, inkrementálny prepočet, rendering, uloženie a Command protokol. Až výsledky týchto skúšok majú rozhodnúť, ktoré technológie a hranice modulov sa stanú stabilným základom projektu.
