# Kečup (Ketchup)

## Architektonický a technický návrh AI-native 2D/3D modelovacej platformy — verzia 2

**Stav dokumentu:** konsolidovaný architektonický návrh po troch externých AI oponentúrach  
**Dátum:** 1. august 2026  
**Nahrádza na ďalšie rozhodovanie:** `KETCHUP_ARCHITECTURE_PROPOSAL.md` (verzia 1 zostáva zachovaná ako historický podklad)  
**Licenčný zámer:** open source a bezplatné používanie; konkrétna licencia ešte vyžaduje rozhodnutie človeka a právny audit  
**Pracovný názov:** Kečup / `ketchup`

---

## 0. Čo sa vo verzii 2 zmenilo

Verzia 2 zachováva základ pôvodného návrhu, ale mení ho z katalógu možností na súbor konkrétnejších rozhodnutí.

Najdôležitejšie zmeny:

1. prvý produkt a prvých približne 24 mesiacov sú zúžené na rýchly, presný, AI-native, SketchUp-like parametrický modelár pre architektúru, interiéry a nábytok;
2. dlhodobé jadro zostáva univerzálne a musí umožniť neskoršie mechanical, drawing/BIM a nature balíky;
3. nad jedinou kanonickou mutačnou cestou sa oddeľujú `Intent`, `Proposal` a `Canonical Command`;
4. 60 Hz interakčný náhľad nie je auditovaná dokumentová transakcia;
5. dokument používa nemenné revízne snapshoty, single-writer/N-readers a asynchrónny evaluation scheduler;
6. exact backend musí vracať aj topologickú históriu, tolerančný report a diagnostiku;
7. topological naming dostáva merateľné triedy stability a explicitné zlyhanie pri nejednoznačnosti;
8. definuje sa realistický determinism envelope namiesto sľubu bitovo identickej geometrie;
9. renderer sa oddeľuje od presného výberu, snappingu a inference;
10. PoC sa delí na tri samostatné go/no-go brány;
11. drawing, BIM, pluginový ekosystém a procedurálna príroda zostávajú cieľmi, ale nie sú súčasťou prvého PoC;
12. záver obsahuje samostatné stanovisko ku všetkým podstatným návrhom troch recenzentov.

---

## 1. Vízia a produktový smer

Kečup má byť otvorená platforma, v ktorej sa dá modelovať jednoducho ako v SketchUpe, presne ako v CAD systéme a automatizovane prostredníctvom bezpečného AI rozhrania.

Dlhodobá vízia zahŕňa:

- architektonické a stavebné modely;
- interiéry a nábytok;
- 2D technické výkresy a projektovú dokumentáciu;
- presné výrobné a strojárske diely;
- parametrické komponenty a zostavy;
- rozsiahle procedurálne objekty, terén a vegetáciu;
- BIM klasifikáciu, vlastnosti, výkazy a IFC výmenu;
- textové, hlasové a agentné AI ovládanie.

Univerzálnosť je vlastnosť architektúry, nie rozsahu prvej verzie.

### 1.1 Prvý cieľový používateľ

Počas prvých približne 24 mesiacov je primárnym cieľom človek, ktorý dnes používa SketchUp alebo podobný priamy modelár na:

- architektúru a stavebné koncepty;
- interiér;
- nábytok a zákazkovú výrobu;
- rýchle objemové a parametrické modelovanie.

Prvý produkt musí byť užitočný ručne aj bez AI. AI má zrýchliť tvorbu a úpravy, nie zakrývať slabé modelovacie jadro.

### 1.2 Architektúra verzus strojárstvo

Obe domény zdieľajú presné rozmery, skice, extrúzie, booleany, komponenty, push/pull a výkresy. Líšia sa hlavne sémantikou, toleranciami výroby, zostavovými väzbami, normami, dokumentáciou a výmennými formátmi.

Kečup preto nebude mať dve nesúvisiace jadrá. Bude mať:

- spoločný presný modeler a dokumentový model;
- prvé pracovné prostredie zamerané na architektúru/interiér/nábytok;
- neskorší `mechanical` balík pre zostavy, výrobné tolerancie, konfigurácie a strojárske výkresy;
- neskorší `architecture/BIM` balík pre stavebné elementy, priestory, klasifikácie, výkazy a IFC.

### 1.3 Čím Kečup nebude v prvom produkte

Prvý produkt nebude plnou náhradou Revitu, FreeCADu, SolidWorksu, Blenderu ani AutoCADu. Nebude obsahovať:

- kompletný BIM a IFC round-trip;
- profesionálny viaclistový drawing systém;
- plný mechanical assembly systém;
- plnohodnotné organické sculpting nástroje;
- filmový renderer;
- kompletný CAM alebo simulácie;
- otvorený marketplace so stabilným verejným ABI;
- browserovú verziu;
- vlastný CAD kernel alebo nový všeobecný constraint solver od nuly.

---

## 2. Záväzné architektonické zásady

1. **Jediná kanonická mutačná cesta:** dokument menia iba validované `Canonical Commands`.
2. **AI nie je geometrický engine:** AI interpretuje zámer; jadro počíta a kontroluje geometriu.
3. **Autoritatívny sémantický dokument:** parametre, význam, väzby a stabilné ID sú dôležitejšie než aktuálna triangulácia.
4. **Viac geometrických reprezentácií:** B-Rep, sketch/2D a mesh/procedural majú oddelené kontrakty a explicitné konverzie.
5. **Presný vstup nesmie potichu driftovať:** zadaný rozmer zostáva autoritatívny aj pri aproximovaných geometrických výsledkoch.
6. **Žiadne tiché prepojenie referencie:** nejednoznačný subshape je chyba viditeľná používateľovi.
7. **Revízne snapshoty:** čitatelia nikdy neblokujú na dlhom zápise do mutovateľného globálneho dokumentu.
8. **Odvodené dáta sú zahoditeľné:** B-Rep výsledky, tessellácia, spatial indexy a GPU buffre sa dajú regenerovať.
9. **Bezpečnosť na hraniciach:** AI, pluginy a importéry sú nedôveryhodné vstupy.
10. **Meranie pred sľubmi:** výkon, tolerancie, FFI režim a knižnice sa uzatvoria podľa PoC a benchmarkov.

---

## 3. Technologický základ

| Oblasť | Rozhodnutie V2 | Poznámka |
|---|---|---|
| Hlavný jazyk | Rust | dokumentové jadro, scheduler, protokol, mesh, renderer a aplikácia |
| Exact geometria | Open CASCADE Technology | predvolený backend za vlastným úzkym C++ façade |
| FFI/proces | kontrakt podporujúci worker aj in-process | PoC porovná odolnosť, latenciu, prenos dát a debugovanie |
| Renderer | `wgpu` | zachovať; presné verzie a minimálne GPU určí implementácia |
| UI PoC/MVP | `egui` | view model nesmie byť závislý od konkrétnych widgetov |
| 2D solver | integračný spike | nevytvárať plný solver od nuly; preveriť kvalitu a licencie kandidátov |
| Pluginy | smer WASM Component Model + WIT | v prvých fázach nestabilizovať verejný ekosystém |
| Externá automatizácia | versionované lokálne RPC + SDK | rovnaké Proposals/Commands/Queries ako UI |
| CPU čísla | `f64` + jednotkovo bezpečné typy | kanonické hodnoty majú stabilný round-trip |
| GPU čísla | camera-relative `f32` | lokálne počiatky a oddelená georeferencia |

### 3.1 Prečo zostáva Rust

Rust je vhodný pre pamäťovo bezpečné, viacvláknové a dlhodobo udržiavateľné jadro. Neznamená automatický výkon; rozhodujú dátové štruktúry, inkrementálnosť, cache, scheduling a profilovanie.

### 3.2 Prečo zostáva OCCT

OCCT poskytuje B-Rep, analytické/NURBS krivky a plochy, booleany, fillets, tesselláciu a priemyselné formáty. Čisto Rust alternatívy zatiaľ nepovažujeme bez porovnávacieho PoC za rovnocennú náhradu univerzálneho exact backendu.

Rust nesmie vidieť všeobecné OCCT triedy ani raw ukazovatele. C++ façade:

- zachytí všetky C++ výnimky;
- vlastní backendové handly;
- vracia iba versionované hrubozrnné dátové kontrakty;
- izoluje správu pamäte a thread-safety pravidlá;
- umožní rovnaké rozhranie použiť in-process aj cez worker transport;
- bude mať fuzzing a crash corpus.

Worker proces zvyšuje odolnosť proti pádu a umožňuje timeout/reštart. Súčasne pridáva IPC, serializáciu, kopírovanie, scheduling a komplikovanejší debugging. V2 preto worker preferuje ako produkčný smer, ale definitívny režim podmieňuje bránou PoC B.

### 3.3 Čo zatiaľ neprijímame ako fakt

Bez primárneho overenia nefixujeme:

- konkrétnu verziu OCCT, `wgpu`, `egui`, WASI alebo wrappera;
- tvrdenie, že konkrétny Rust wrapper je produkčne pripravený alebo mŕtvy;
- konkrétny výkon alternatívneho kernelu;
- licenčné dôsledky statického/dynamického linkovania;
- konkrétnu internú jednotku a absolútny limit súradníc ako univerzálne pravidlo.

Tieto body sú predmetom technického alebo právneho overenia pred implementačným záväzkom.

### 3.4 Počiatočné členenie workspace

Namiesto štrnástich predčasných crates začne projekt približne so šiestimi:

```text
ketchup/
├─ crates/
│  ├─ ketchup-core/       # dokument, revízie, entities, features, units
│  ├─ ketchup-protocol/   # intents, proposals, commands, queries, schemas
│  ├─ ketchup-geometry/   # neutral API + dočasne OCCT bridge/workers
│  ├─ ketchup-render/     # wgpu + render data + viewport
│  ├─ ketchup-io/         # formát, migrácie, import/export
│  └─ ketchup-app/        # view model, UI, composition root
├─ cpp/occt-facade/
├─ schemas/
└─ tests/
```

Crate sa rozdelí až vtedy, keď hranicu preukážu závislosti, build time, bezpečnosť alebo samostatná distribúcia.

---

## 4. Vrstvená architektúra V2

```text
UI / CLI / Python / Voice / AI / Plugins
                    │
          Intent and Proposal Layer
 assumptions • base revision • plan • risk • preview digest
                    │
        Canonical Command Gateway
 schema • capabilities • preconditions • budget • transaction
                    │
       Revisioned Canonical Document
 entities • definitions • occurrences • bodies • FeatureSpecs
                    │
          Evaluation Scheduler
 dirty DAG • jobs • generation • cancellation • stale rejection
          ┌─────────┴──────────┐
          │                    │
 Exact Geometry Backend   Sketch / Mesh / Procedural
 OCCT façade/worker       services
          └─────────┬──────────┘
                    │
     Interaction and Spatial Query Service
 coarse pick • exact hit • snaps • inference • SubshapeRef
                    │
        Derived Render Data / GPU Cache
                    │
                 Renderer
```

### 4.1 Závislostné pravidlá

- UI závisí od framework-agnostic view modelu a protokolu.
- Proposal layer môže plánovať, ale nesmie obísť Canonical Command Gateway.
- Document Core nepozná OCCT triedy, widgety ani GPU buffre.
- Geometry backend neinterpretuje prirodzený jazyk.
- Renderer nevlastní dokument a nerozhoduje o sémantike presného výberu.
- Doménové balíky rozširujú všeobecný entity/component model.
- Importéry, AI a pluginy sú klienti s explicitnými capabilities.

---

## 5. Kanonický dokument a revízny model

### 5.1 Zdroj pravdy

Autoritatívny je aktuálny kanonický dokument, nie replay celého command logu. Dokument uchováva:

- stabilné entity ID a schémy;
- kanonické parametre a jednotky;
- `FeatureSpec` a jeho vstupné referencie;
- definície, výskyty, telá a sémantické vlastnosti;
- stav referencií a diagnostiku;
- odkazy na content-addressed blobs;
- verzie schém a determinism envelope.

`commands.log` je voliteľný a orezateľný audit, zdroj makier, debugovania alebo spolupráce. Migrácia mení dokumentovú schému; nesmie vyžadovať prehranie každej historickej operácie cez starú verziu kernelu.

### 5.2 Nemenné snapshoty

Model konkurencie:

- jeden zapisovateľ vytvára nové revízie;
- N čitateľov drží nemenný `Snapshot` konkrétnej revízie;
- snapshoty používajú štrukturálne zdieľanie;
- renderer kreslí posledný kompletný dostupný výsledok;
- worker dostane revíziu, generation token a hash vstupov;
- výsledok sa vloží iba vtedy, ak revízia/generation/input hash stále sedia;
- zastaraný výsledok sa zahodí, nikdy sa potichu nezlúči.

Undo/redo je navigácia medzi revíziami doplnená content-addressed blobs a podľa potreby inverznými príkazmi pre veľké externé dáta.

### 5.3 Základné objekty

- `Definition` — zdieľaná definícia komponentu;
- `Occurrence` — výskyt/instance definície s transformáciou a povolenými overrides;
- `Body` — logické presné alebo mesh teleso;
- `FeatureSpec` — autoritatívna operácia, parametre a referencie;
- `FeatureResult` — odvodený výsledok výpočtu;
- `GeometryResource` — handle/blob/cache key geometrie;
- `Reference` — stabilný odkaz na entitu alebo subshape;
- `PropertySet` — versionované sémantické a doménové vlastnosti;
- `Group` — hierarchia a editovací kontext;
- `Tag/Layer` — viditeľnosť, filtrovanie a štýl, nie vlastníctvo geometrie;
- `Collection` — nehierarchická používateľská množina;
- `Classification` — význam objektu, napríklad stena alebo nábytok;
- `SavedView` — kamera, rezy, visibility set a štýl.

### 5.4 Feature stavový automat

Minimálne stavy:

- `Clean` — výsledok zodpovedá vstupom;
- `Dirty` — vyžaduje prepočet;
- `Queued`;
- `Computing`;
- `Failed`;
- `Cancelled`;
- `Stale` — výpočet dobehol pre starú generáciu;
- `Suppressed`;
- `BrokenRef` — referencia je stratená alebo nejednoznačná.

Posledný dobrý výsledok môže zostať vo viewporte ako vizuálne označený stale náhľad. Nesmie byť vydávaný za aktuálny výsledok ani použitý na export bez upozornenia.

### 5.5 Výrazy parametrov

Fáza 1 potrebuje malý jednotkovo bezpečný výrazový jazyk pre vzťahy typu `šírka/2`, `výška_podlažia - 150 mm` a odkazy na pomenované parametre. Má byť deterministický, sandboxovaný, s detekciou cyklov a bez všeobecnej Turingovej úplnosti.

---

## 6. Geometrické kontrakty

### 6.1 Oddelené reprezentácie

1. **Exact B-Rep:** presné diely, stavebné elementy, analytické plochy, STEP.
2. **Sketch/2D:** profily, constraints, výkresová geometria.
3. **Mesh/procedural:** terén, vegetácia, scan dáta, veľké vizualizačné scény.

Každá entita označuje autoritatívny zdroj. Konverzie sú explicitné a oznamujú stratu:

- B-Rep → render mesh je odvodenie;
- sketch → exact feature je parametrické odvodenie;
- procedural recipe → LOD/instances je odvodenie;
- mesh → B-Rep je potenciálne stratová rekonštrukcia;
- mesh boolean nie je automaticky presný B-Rep boolean.

### 6.2 Exact backend API

Každá modelovacia operácia musí vrátiť viac než shape:

```rust
struct ExactOpOutput {
    shape: ShapeHandle,
    topology_history: TopologyHistory,
    tolerance_report: ToleranceReport,
    diagnostics: Vec<GeometryDiagnostic>,
    result_fingerprint: ResultFingerprint,
}
```

`TopologyHistory` obsahuje generated/modified/deleted mapovanie vstupných subshapes. `ToleranceReport` uvádza použitý profil, lokálne tolerancie výsledku a degradáciu. Diagnostika rozlišuje neplatný vstup, degeneráciu, neúspešnú operáciu, timeout, cancellation a backend crash.

### 6.3 Scheduler a cache

Výsledok feature je cacheovaný približne podľa:

```text
hash(
  FeatureSpec,
  input result fingerprints,
  geometry backend identity/version,
  tolerance profile,
  schema versions,
  relevant platform envelope
)
```

Nezávislé uzly sa môžu počítať paralelne, ale operácie konkrétneho backendu musia rešpektovať jeho reálnu thread-safety. Cancellation musí byť kooperatívna alebo procesná; nebezpečné násilné ukončenie vlákna sa nepoužíva.

---

## 7. Presnosť a determinizmus

### 7.1 Precision contract

Kečup oddeľuje:

- **autoritatívny zámer:** napríklad presne zadaných `2400 mm`;
- **matematickú/geometrickú reprezentáciu:** analytická alebo NURBS geometria v `f64`;
- **toleranciu operácie:** prípustná numerická odchýlka a lokálna degradácia;
- **vizualizačnú aproximáciu:** tessellácia a GPU `f32`.

Pravidlá:

- verejné rozmery majú explicitné jednotky;
- kanonický zápis prežije save/load bez driftu;
- autoritatívny parameter sa neprepisuje nameranou aproximovanou hodnotou;
- tolerančná politika je centrálna, versionovaná a súčasť determinism envelope;
- kernelom vytvorené lokálne tolerancie sa reportujú, nie ručne nastavujú ľubovoľne na každej entite;
- veľmi malé detaily, veľké rozsahy a zmiešané mierky majú corpus testov;
- georeferencia je transformácia nad lokálnym modelom, nie veľká mapová súradnica každého vrcholu;
- prekročenie overeného pracovného rozsahu je štruktúrovaná chyba alebo varovanie.

Interná pracovná jednotka exact backendu a konkrétny bezpečný rozsah súradníc budú rozhodnuté meraním v PoC. Kandidát „milimetre a lokálny rozsah približne stavby/dielu“ je rozumný, ale nie je zatiaľ vydávaný za univerzálny fakt OCCT.

### 7.2 Determinism envelope

Dokument zaznamená minimálne:

```text
core_version
geometry_backend_id
geometry_backend_version
geometry_backend_build_fingerprint
tolerance_profile_version
document_schema_versions
command_schema_versions
relevant_target_platform
```

Garantujeme:

1. dátový determinizmus kanonických parametrov, ID, jednotiek a väzieb;
2. sémantickú geometrickú ekvivalenciu v deklarovanej tolerancii;
3. reprodukovateľnú diagnostiku v podporovanom envelope podľa možností testov.

Negarantujeme bitovo identické B-Rep ani mesh dáta na všetkých OS, CPU, kompilátoroch a verziách kernelu.

Golden testy porovnávajú parametre, stabilné ID, počty a vzťahy telies, bounding box, objem, plochu, analytické typy a geometrické invarianty v tolerancii — nie iba binárny blob.

---

## 8. Topological naming a stabilné referencie

### 8.1 Cieľ

Topological naming nebude prezentovaný ako úplne vyriešený problém. Cieľom MVP je:

- garantovať malú presne definovanú podmnožinu;
- pri ďalších prípadoch použiť best effort;
- spoľahlivo detegovať stratu alebo nejednoznačnosť;
- nikdy potichu nevybrať inú plochu.

### 8.2 SubshapeRef

Referencia môže obsahovať:

```text
producer_feature_id
output_port
semantic_role
source_element_id
genesis/lineage path
expected_geometry_type
adjacency_signature
geometric_signature
expected_cardinality
stability_class
```

Poradie riešenia:

1. feature-specific sémantická rola;
2. povinná topologická história backendu;
3. genesis/lineage path;
4. topologická a susedská signatúra;
5. geometrický odtlačok uložený v dokumente;
6. explicitné `Ambiguous` alebo `Lost`, ak výsledok nie je jednoznačný.

### 8.3 Triedy stability

- `Guaranteed` — stabilita je súčasťou otestovaného kontraktu konkrétnej feature;
- `BestEffort` — resolver sa pokúsi referenciu obnoviť, bez garancie;
- `Ephemeral` — platí iba pre aktuálny výsledok/preview;
- `Ambiguous` — existuje viac kandidátov;
- `Lost` — referencia už nemá platný cieľ.

Počiatočná garantovaná podmnožina:

- začiatok a koniec jednoduchej extrúzie;
- bočná plocha odvodená od konkrétnej hrany skice;
- základné sémantické plochy jednoduchej revolve;
- jednoduché pattern occurrences.

Všeobecné booleany a fillety sú na začiatku `BestEffort`. PoC ich úmyselne testuje ako negatívny aj pozitívny corpus, ale úspech projektu neznamená falošný sľub ich stopercentnej stability.

---

## 9. Interaction, presný výber a Smart Push/Pull

### 9.1 Samostatná Interaction and Spatial Query Service

Renderer poskytne GPU coarse picking a vizuálne zvýraznenie. Samostatná služba zabezpečí:

- CPU exact hit testing;
- výber hrán, plôch, telies a prekrytých kandidátov;
- snap kandidátov a scoring;
- inference pravidlá;
- hysteréziu a hover lock proti blikaniu kandidátov;
- selection filters;
- prevod kandidáta na stabilný `SubshapeRef`;
- rovnaké read-only Query rozhranie pre UI aj AI.

### 9.2 Efemérny náhľad verzus commit

Počas draggingu sa nevytvára 60 auditovaných transakcií za sekundu.

```text
pointer/gesture
  → efemérny interaction state
  → lacný mesh/transform preview
  → priebežný numerický HUD a snap
  → potvrdenie
  → jeden Canonical Command alebo CommandBatch
  → exact recompute
```

Preview nesmie predstierať finálnu validovanú geometriu. Ak exact commit dopadne inak, UI ukáže rozdiel alebo chybu.

### 9.3 Smart Push/Pull

Používateľ môže vzdialenosť zadať myšou, klávesnicou, snapom, parametrom alebo hlasom. Po potvrdení systém podľa kontextu:

- zmení parameter pôvodnej jednoduchej extrúzie;
- vytvorí offset/extrude feature;
- vytvorí parametrický cut alebo otvor;
- vytvorí nové teleso;
- pri nejednoznačnosti ponúkne voľby.

Jednoduchosť SketchUpu sa zachová na interakčnej vrstve, ale výsledok ostane parametrický, presný a auditovateľný.

---

## 10. Intent, Proposal, Commands a Queries

### 10.1 Tri úrovne

1. **Intent:** používateľský alebo doménový cieľ, napríklad „zväčši miestnosť o 800 mm na sever“.
2. **Proposal:** konkrétny plán viazaný na revíziu, s predpokladmi, rizikom, diffom a digestom.
3. **Canonical Commands:** malá, versionovaná a presná mutačná abeceda dokumentu.

UI môže jednoduchý Command vytvoriť priamo. AI, hlas, makrá a doménové nástroje typicky vytvárajú Proposal, ktorý sa skompiluje na Commands.

### 10.2 Proposal kontrakt

Proposal obsahuje:

- `base_revision`;
- používateľský zámer a explicitné predpoklady;
- plánovaný `CommandBatch`;
- odhadované dôsledky a náročnosť;
- rizikovú triedu;
- štruktúrovaný diff;
- preview odkazy;
- digest presného plánu;
- požadované capabilities;
- expiration/validity pravidlá.

Affected-set autoritatívne vypočíta jadro. Klient ho môže poslať iba ako hint.

### 10.3 Canonical Command transaction

Commit pipeline:

1. schema a capability validation;
2. kontrola `base_revision` a preconditions;
3. kontrola budgetov;
4. dry-run nad izolovaným snapshotom;
5. geometrická a doménová validácia;
6. vytvorenie autoritatívneho diffu a digestu;
7. pri rizikovej zmene potvrdenie používateľom;
8. opätovná kontrola revízie a digestu;
9. atómový commit alebo rollback;
10. nová revízia, audit a naplánovanie odvodených výpočtov.

Dry-run nesmie meniť kanonický dokument. Cache použitá pri dry-rune musí byť oddelená alebo bezpečne content-addressed, aby nedôveryhodný vstup nemohol otráviť výsledky.

### 10.4 Queries

Queries sú read-only a vracajú:

- dokumentový strom a výber;
- parametre, jednotky a expressions;
- feature DAG a stavy;
- geometrické vlastnosti a diagnostiku;
- snap/inference kandidátov;
- dostupné stabilné referencie;
- diff revízií;
- odhad náročnosti plánu;
- bezpečne filtrované metadáta.

---

## 11. AI-native workflow a bezpečnosť

### 11.1 Workflow

```text
inspect → interpret intent → clarify assumptions → proposal
→ budget check → isolated dry-run → deterministic validation
→ preview/diff → optional confirmation → revision-bound commit
→ verify result
```

AI dostane menšiu doménovú sadu intents/tools, nie stovky interných nízkoúrovňových Commands naraz. Canonical Commands zostávajú stabilným jadrovým kontraktom, ale tool surface sa skladá podľa kontextu a capability.

### 11.2 Resource budgets

Gas score je iba odhad, nie záruka. Každý AI/plugin batch má kombináciu:

- maximálneho počtu Commands a vytvorených entít;
- odhadu a limitu topologického rastu;
- wall-clock timeoutu;
- CPU a memory budgetu;
- limitu veľkosti vstupov/výstupov;
- cancellation tokenu;
- obmedzenia paralelných jobov;
- potvrdenia pri prekročení bežnej náročnosti.

### 11.3 Hrozby

Threat model zahŕňa:

- prompt injection v názve, metadátach, BIM vlastnostiach alebo importovanom dokumente;
- tool-output injection;
- geometry DoS cez obrovský boolean/pattern/mesh;
- TOCTOU medzi dry-runom a commitom;
- preview/commit mismatch;
- capability escalation pluginu;
- škodlivé importéry a parsery;
- exfiltráciu modelu cloudovým AI providerom;
- cache poisoning;
- path traversal a zip bombs v natívnom kontajneri;
- únik citlivých údajov v audit logu alebo telemetry.

Metadáta dokumentu sa modelu poskytujú ako nedôveryhodné dáta oddelené od systémových inštrukcií. Importéry rizikových formátov majú bežať v sandboxovanom procese s kvótami. Externý cloudový AI prístup musí byť opt-in a transparentne ukázať, ktoré dáta opúšťajú zariadenie.

### 11.4 Operácie vyžadujúce potvrdenie

Minimálne:

- rozsiahle alebo nevratné mazanie;
- prepis externého súboru;
- inštalácia alebo zvýšenie capability pluginu;
- odoslanie modelu do cloudu;
- hromadná stratová konverzia reprezentácií;
- commit s vysokým resource/risk score;
- automatická BIM reinterpretácia s neistotou;
- zmena, ktorej Proposal digest už nezodpovedá potvrdenému preview.

---

## 12. Renderer a výkon

Renderer je konzument odvodených render packets. Používa:

- `wgpu`;
- camera-relative `f32`;
- instancing;
- LOD a culling;
- spatial index/BVH;
- cache tessellácie podľa geometrie a tolerancie;
- oddelené interaktívne a finálne quality profiles;
- frame-time a p95/p99 latencie namiesto samotného priemerného FPS.

Renderer nesmie čakať na exact recompute. Kreslí posledný kompletný snapshot a neaktuálny výsledok označí.

PoC C musí merať aspoň:

- frame time pri navigácii;
- latenciu coarse a exact pickingu;
- čas tessellácie;
- RAM a VRAM;
- počet inštancií;
- režijné náklady snapshotov a transportu;
- odozvu pri súbežnom geometrickom výpočte.

Procedurálna borovica je dôležitý dlhodobý demonštrátor, ale nie je kritické riziko brány PoC A.

---

## 13. 2D sketch a constraint solver

Plný solver od nuly nie je cieľ MVP. Samostatný spike porovná kandidátov podľa:

- kvality a stability riešenia;
- diagnostiky DoF, konfliktov a preurčenia;
- podpory potrebných constraints;
- možnosti cancellation a izolácie;
- integračnej náročnosti;
- licencie a distribučných povinností.

PlaneGCS, SolveSpace/libslvs a iné riešenia sú kandidáti na overenie, nie schválené závislosti. Spustenie GPL knižnice vo WASM alebo inom procese automaticky neruší licenčné povinnosti. Konečný výber nasleduje po právnom a technickom spikeu.

Prvá skica môže mať malú sadu constraints: coincident, horizontal, vertical, parallel, perpendicular a základné rozmery.

---

## 14. Pluginy a externé SDK

WASM Component Model + WIT zostáva preferovaný smer pre bezpečné prenositeľné pluginy, ale:

- verejné API sa nebude stabilizovať pred stabilitou Document/Command schém;
- prvé pluginy budú najmä orchestration, validátory a doménové generátory;
- ťažké exact backendy a importéry môžu byť dôveryhodné natívne/procesné komponenty;
- capabilities budú explicitné pre dokument, súbory, sieť, UI a výpočtové zdroje;
- chýbajúci plugin sa zobrazí ako placeholder so zachovaním namespaced dát;
- plugin nesmie dostať všeobecný mutable pointer do dokumentu;
- Python, CLI a MCP používajú rovnaké versionované Proposal/Command/Query API.

Web nie je cieľ prvého produktu. Procesná a transportná hranica však nemá znemožniť budúci remote exact worker alebo browser klienta.

---

## 15. Súborový formát

Natívny formát je versionovaný kontajner, napríklad:

```text
model.ketchup
├─ manifest.json
├─ document.bin
├─ audit/commands.log          # voliteľný a orezateľný
├─ blobs/<content-hash>
├─ cache/                      # zahoditeľná
├─ previews/
└─ extensions/<namespace>/
```

Požiadavky:

- kanonický dokument je zdroj pravdy;
- atomické ukladanie a obnova po páde;
- checksums a limity proti zip bomb/path traversal;
- versionované schémy a explicitné migrácie;
- preservovanie neznámych namespaced extension dát, ak je to bezpečné;
- odvodené B-Rep/mesh/GPU dáta môžu byť vymazané a regenerované;
- kernel fingerprint určí, či je cache znovu použiteľná;
- presná serializácia sa vyberie po prototype migrácií a benchmarku.

Importovaný STEP alebo mesh nemusí rekonštruovať pôvodnú parametrickú históriu. Strata sémantiky musí byť používateľovi oznámená.

---

## 16. Projektová dokumentácia, BIM a doménové balíky

### 16.1 Drawing

Profesionálny drawing modul zostáva povinnou dlhodobou súčasťou vízie:

- asociatívne pôdorysy, rezy, pohľady a detaily;
- kóty, symboly, text, osi, šrafy a štýly čiar;
- listy, pečiatky a kancelárske šablóny;
- výkazy a revízie;
- vektorový PDF/SVG a podľa potrieb DXF;
- označenie neaktuálnych pohľadov.

V2 prijíma, že hidden-line rendering, sadzba textu, fonty, orezávanie šráf, mierky čiar a asociativita sú veľký samostatný produkt. Preto nie sú vo Fáze 2 jednou odrážkou. Najprv vznikne iba jednoduchý technický export alebo základný asociatívny pohľad; profesionálny drawing je samostatný neskorší míľnik.

### 16.2 BIM

BIM je doménový balík nad spoločným entity/component modelom. Dlhodobo obsahuje:

- steny, dosky, strechy, nosníky, stĺpy, dvere, okná a schodiská;
- podlažia, priestory, zóny, typy a instances;
- property sets, klasifikácie, fázy a externé GUID;
- hostiteľ–otvor–výplň a priestorové vzťahy;
- výkazy, kontroly a georeferenciu;
- IFC import/export s reportom strát a testami.

AI môže navrhnúť konverziu všeobecnej geometrie na BIM, ale neistý význam musí ukázať a vyžiadať potvrdenie.

### 16.3 Ďalšie balíky

- `furniture`: skrinky, kovania, materiály, rezné zoznamy;
- `mechanical`: zostavy, konfigurácie, výrobné tolerancie a výkresy;
- `nature`: procedurálne stromy, porasty, terén, LOD;
- `drawing`: dokumentácia spoločná viacerým doménam.

---

## 17. Roadmapa

### Fáza 0 — tri technické go/no-go brány

Bez marketplace, BIM, profesionálnych výkresov, plnej skice, borovice a finálneho UX.

### Fáza 1 — úzky modelovací produkt

- desktopová aplikácia;
- kamera, výber, snapping a inference;
- skupiny, definície, occurrences a tagy;
- jednoduchá skica;
- extrude, jednoduchý cut/union a Smart Push/Pull;
- parametre, jednotky a výrazy;
- revízne undo/redo a natívny save/load;
- jeden exact import/export a mesh export;
- lokálny Proposal/Command/Query endpoint;
- textový AI asistent na kanonickej sade úloh.

### Fáza 2 — stabilizácia a prvá doména

- stabilnejšie verejné schémy;
- constraint solver podľa výsledku spikeu;
- architecture/interior/furniture primitives;
- základný asociatívny pohľad alebo jednoduchý technický export;
- Python SDK a obmedzený plugin pilot;
- benchmark, migration a compatibility suite.

### Fáza 3 — rozšíriteľnosť a veľké scény

- WASM host po overení zrelosti;
- procedurálny graf, instancing, LOD a streaming;
- `nature` demonštrátor vrátane borovice;
- rozšírené importéry v sandboxe;
- pokročilejšie doménové balíky.

### Fáza 4 — profesionálne workflow

- plný drawing míľnik;
- BIM/IFC, výkazy, klasifikácie a validácia;
- mechanical assemblies a konfigurácie;
- collaboration/versioning podľa budúceho rozhodnutia;
- voliteľné remote/cloud výpočty bez povinnej cloudovej závislosti.

---

## 18. Trojbránový proof-of-concept

Každá brána má samostatné rozhodnutie. Neúspech brány sa najprv opraví alebo vedie k zmene architektúry; automaticky sa nepokračuje budovaním vyšších vrstiev.

### Brána A — exact vertical slice

**Rozsah:**

- kanonický profil/sketch subset;
- extrude;
- jednoduchý cut;
- zmena parametra a inkrementálny recompute;
- povinná topologická história;
- save/load kanonického dokumentu;
- rovnaká transakcia z UI adaptéra a JSON/RPC;
- základný view model, nie finálne UI.

**Corpus:**

- zmena šírky/výšky extrúzie;
- odkazy na top/bottom/side faces;
- jednoduchý cut;
- fillet ako `BestEffort` negatívny/pozitívny test;
- malé hrany, dotyky, takmer koplanárne plochy;
- precision round-trip.

**Akceptačné kritériá:**

- autoritatívne rozmery sa po najmenej 100 save/load cykloch nezmenia;
- garantované referencie zostanú správne vo všetkých definovaných mutation testoch;
- nejednoznačný alebo stratený best-effort odkaz skončí `Ambiguous/Lost`, nikdy inou ticho zvolenou plochou;
- UI a RPC vytvoria ekvivalentný kanonický dokument;
- lokálna zmena neprepočíta nezávislý uzol;
- geometrické zlyhanie vráti štruktúrovanú diagnostiku.

### Brána B — robustnosť, konkurencia a izolácia

**Rozsah:**

- nemenné snapshoty;
- scheduler a generation tokens;
- cancellation;
- stale-result race;
- timeout/resource budget;
- worker aj in-process experiment;
- simulovaný backend crash;
- isolated dry-run a revision/digest konflikt.

**Akceptačné kritériá:**

- renderer/query čitateľ nie je blokovaný dlhým writer/geometry jobom;
- starý výsledok sa po zmene vstupov nikdy nevloží ako aktuálny;
- cancellation ukončí alebo izoluje testovaný dlhý job v definovanom limite;
- worker crash nestratí poslednú commitnutú revíziu a worker možno obnoviť;
- C++ výnimka nikdy neprekročí FFI hranicu;
- commit zastaranej alebo zmenenej Proposal revízie/digestu sa odmietne;
- porovnanie worker/in-process zdokumentuje p50/p95 latenciu, pamäť, crash containment a zložitosť.

### Brána C — interakcia a výkon

**Rozsah:**

- `wgpu` viewport;
- derived render data;
- coarse pick + CPU exact hit;
- efemérny Smart Push/Pull preview;
- instancing a jednoduché LOD;
- väčší feature DAG a component occurrences.

**Akceptačné kritériá:**

- kamera a efemérny preview smerujú k 60 FPS na definovanom referenčnom hardvéri;
- jednoduchý lokálny commit má viditeľnú odozvu približne do 100 ms, ak exact operácia patrí do interaktívnej triedy;
- dlhšia operácia okamžite zobrazí stav/progress a neblokuje navigáciu;
- exact pick a snap majú zmeranú p95 latenciu a stabilné scoring/hysteresis správanie;
- 10 000+ occurrences zdieľanej jednoduchej definície neduplikuje autoritatívnu geometriu;
- benchmark report obsahuje p50/p95/p99, RAM, VRAM, frame time, tesselláciu, save/load a transport overhead.

**Referenčný hardvér:** minimálne bežný notebook s integrovaným GPU a výkonnejší desktop. Presné modely sa zapíšu do benchmark plánu, nie do architektonického sľubu.

### Čo nie je v PoC

- profesionálny drawing;
- BIM/IFC;
- marketplace a stabilné WIT SDK;
- plný constraint solver;
- procedurálna borovica;
- komplexné mechanical assemblies;
- cloud a simultánna spolupráca;
- široký import/export matrix.

---

## 19. Testovacia stratégia

### 19.1 Dokument a protokol

- schema a migration testy;
- snapshot/undo/redo round-trip;
- transakčný rollback;
- stale revision a TOCTOU;
- Proposal preview/digest/commit zhoda;
- UI/RPC/AI ekvivalencia kanonického výsledku;
- determinism-envelope testy.

### 19.2 Geometria a presnosť

- mutation corpus pre TNP;
- generated/modified/deleted history testy;
- `Resolved/Ambiguous/Lost` regresie;
- boolean/fillet degenerované vstupy;
- 100+ save/load round-trip rozmerov;
- malé detaily, veľké lokálne rozsahy a zmiešané mierky;
- area/volume/bounding-box invarianty v tolerancii;
- fuzzing C++ façade a importérov.

### 19.3 Scheduler a výkon

- cancellation a stale-result race testy;
- worker crash recovery;
- DAG invalidation a cache keys;
- p50/p95/p99 latencie;
- frame-time, RAM a VRAM;
- component instancing;
- testy na integrovanom aj samostatnom GPU.

### 19.4 AI a bezpečnosť

- približne 20 kanonických používateľských úloh s deterministickými kontrolami výsledku;
- nejednoznačné jednotky a predpoklady;
- prompt injection v názvoch a metadátach;
- geometry DoS a budget denial;
- capability denial;
- dry-run cache isolation;
- cloud exfiltration consent;
- škodlivý kontajner/import.

---

## 20. Hlavné riziká

| Riziko | Stav V2 | Zmiernenie |
|---|---|---|
| Topological naming | kritické | povinná backend history, stability classes, úzky guaranteed subset, corpus |
| OCCT FFI a pády | kritické | vlastný façade, exception boundary, worker experiment, fuzzing |
| Príliš široký rozsah | kritické | jeden prvý segment, trojbránový PoC, odsun drawing/BIM/nature |
| Snapshot/scheduler chyby | vysoké | immutable revisions, generation tokens, race testy |
| Robustnosť booleanov/filletov | vysoké | corpus, diagnostika, tolerančný report, best-effort hranice |
| Constraint solver | vysoké | technický/licenčný spike, malý MVP subset |
| Presnosť a rozsah súradníc | vysoké | canonical dimensions, local coordinates, tolerance profile, corpus |
| AI DoS/prompt injection | vysoké | budgety, izolácia, revision/digest binding, capabilities |
| Dlhodobý formát | vysoké | canonical document, migrácie, namespaced extensions, round-trip testy |
| Drawing/BIM náročnosť | vysoké | samostatné míľniky a realistické akceptačné kritériá |
| Plugin ABI | stredné | nestabilizovať skoro, WIT pilot, compatibility suite |
| Licencie | vysoké | rozhodnutie človeka a právny audit pred záväznými závislosťami |

---

## 21. Stanovisko k trom oponentúram

Nasledujúce tabuľky sú rozhodovacou maticou. „Prijať“ znamená zapracovať do architektúry V2. „Prijať s úpravou“ znamená prijať problém alebo smer, nie neoverenú konkrétnu implementáciu. „PoC/právne overiť“ znamená nemať ešte falošný záväzok. „Odmietnuť“ znamená, že návrh je v rozpore s cieľom alebo zamieňa hypotézu za fakt.

### 21.1 Prvá recenzia

| Položka recenzenta | Stanovisko V2 | Zdôvodnenie |
|---|---|---|
| Jednotné Commands/Queries pre UI a AI | **Prijať s úpravou** | Jedna mutačná cesta zostáva, ale intent a proposal nesmú byť zamieňané s kanonickou operáciou. |
| Izolácia OCCT | **Prijať** | Zabraňuje presakovaniu C++ typov a umožní worker/in-process implementáciu. |
| WASM Component Model ako bezpečný pluginový základ | **Prijať ako smer** | Capability sandbox je správny; zrelosť a verejná stabilizácia sa overia neskôr. |
| Explicitné jednotky, CPU `f64`, camera-relative GPU `f32` | **Prijať** | Je to základ precision contractu; samotné `f64` však nerieši tolerancie a topológiu. |
| Transakcie a dry-run | **Prijať s posilnením** | Pridávame izolovaný snapshot, revision/digest binding a ochranu proti TOCTOU. |
| TNP je riziko číslo jeden | **Prijať** | Dopĺňame povinnú topology history, stability classes a explicitné Ambiguous/Lost. |
| C++ exception cez FFI je kritické riziko | **Prijať** | Všetky výnimky musí zachytiť façade; hranica bude fuzzovaná a crash-testovaná. |
| Command validácia pri každom frame zničí UX | **Prijať** | Dragging používa efemérny preview; až potvrdenie vytvorí jednu transakciu. |
| AI môže spôsobiť geometry DoS | **Prijať** | Gas score dopĺňame timeoutom, pamäťou, topologickými a entitnými limitmi a cancellation. |
| Nevytvárať plný solver od nuly | **Prijať** | Solver je samostatný integračný a licenčný spike. |
| OCCT musí byť vždy worker proces | **Prijať s úpravou / PoC** | Worker je preferovaný produkčný smer, ale režijné náklady a transport treba porovnať s in-process. |
| Tolerancia priamo na každej entite | **Odmietnuť v tejto forme** | Preferujeme versionovaný document tolerance profile a report lokálnych tolerancií kernelu; ľubovoľné per-entity epsilon by bolo nekonzistentné. |
| Gas metering ako hlavná ochrana | **Prijať s úpravou** | Odhad náročnosti nie je spoľahlivý limit; musí byť kombinovaný s reálnymi resource controls. |
| SolveSpace vo WASM kvôli GPL/open-core | **Právne overiť; argument izolácie odmietnuť** | Proces/WASM automaticky nemení licenčné povinnosti. |
| Truck ako náhrada OCCT a OCCT iba import plugin | **Odmietnuť ako aktuálne rozhodnutie** | Bez dôkazov nie je rovnocenným exact backendom; môže byť porovnávací spike pre úzky rozsah. |
| Zúžiť PoC na box/fillet/change/crash | **Prijať princíp, upraviť scenár** | TNP a crash isolation sú jadro, ale potrebujeme aj sketch/extrude/cut/save-load/protocol vertical slice. |
| Oddeliť nedôveryhodné metadata od AI inštrukcií | **Prijať** | Je to nutná obrana proti prompt injection. |
| Dry-run nesmie otráviť cache | **Prijať** | Použije izolovanú alebo bezpečne content-addressed cache. |
| Licencia, web a prvá doména sú ľudské rozhodnutia | **Prijať** | Prvú doménu V2 uzatvára; licencia zostáva človeku; web nie je prvý produkt. |
| Commands eliminujú halucinácie | **Odmietnuť absolútnu formuláciu** | Commands halucinácie ohraničia a validujú, ale nezabránia zlému plánu alebo nesprávnemu zámeru. |

### 21.2 Druhá recenzia

| Položka recenzenta | Stanovisko V2 | Zdôvodnenie |
|---|---|---|
| Architektúra je dobrá, ale scope je najväčšie riziko | **Prijať** | V2 vyberá prvý segment a presúva BIM/drawing/nature. |
| PoC je malé MVP a treba tri brány | **Prijať** | V2 definuje A exact, B robustnosť a C interaction/performance s metrikami. |
| Rozlíšiť dátový, sémantický a bitový determinizmus | **Prijať** | Garantujeme prvé dva v envelope; bitovú identitu nesľubujeme. |
| Pridať determinism envelope | **Prijať** | Je súčasťou dokumentu, cache a testov. |
| Oddeliť Intent/Proposal od Canonical Commands | **Prijať** | Zachová sa jedna mutačná cesta bez neurčitých príkazov v core API. |
| Proposal: revízia, predpoklady, dôsledky, riziko, digest, preview | **Prijať** | Dopĺňame aj capability a expiration pravidlá. |
| Affected-set počíta jadro | **Prijať** | Klient je nedôveryhodný a nemusí poznať transitive dependencies. |
| Oddeliť Interaction/Spatial Queries od rendereru | **Prijať** | GPU picking je iba kandidát; CAD potrebuje exact hit, snaps, inference a stabilnú referenciu. |
| Zaviesť TNP stability classes | **Prijať** | Guaranteed subset je úzky; boolean/fillet začínajú BestEffort. |
| Explicitný `FeatureSpec` a `FeatureResult` | **Prijať** | Oddeľuje kanonický dokument od cache a backendu. |
| Explicitné Definition/Occurrence/Body/GeometryResource/Reference/PropertySet | **Prijať** | Je nutné pre components, assemblies, BIM aj rendering. |
| Evaluation scheduler so stavmi a stale rejection | **Prijať** | Dopĺňame nemenné snapshoty a generation tokens. |
| Canonical document je autorita, command log audit | **Prijať** | Replay starej geometrickej histórie nie je udržateľná migrácia. |
| Vlastný úzky C++ façade | **Prijať** | Je to stabilná hranica; konkrétne wrappery nie sú záväzné. |
| Worker preferovať pre distribúciu | **Prijať s PoC podmienkou** | Crash containment je silný argument; zmeriame overhead. |
| `wgpu` zachovať | **Prijať** | Vyhovuje cross-platform GPU a instancingu. |
| `egui` pre PoC, view model oddeliť | **Prijať** | Znižuje riziko budúcej zmeny UI frameworku. |
| WASM skúšať, nie skoro stabilizovať | **Prijať** | Verejný ekosystém nesmie zmraziť nezrelý dokumentový model. |
| Solver porovnať technicky a licenčne | **Prijať** | Žiadny kandidát sa neprijíma bez overenia. |
| Začať približne so šiestimi crates | **Prijať** | Pôvodných štrnásť hraníc bolo predčasných. |
| Bezpečnostné hrozby TOCTOU, mismatch, DoS, importéry, exfiltrácia | **Prijať** | Sú explicitnou súčasťou threat modelu V2. |
| Prvý produkt pre architektúru/interiér/nábytok | **Prijať** | Zodpovedá SketchUp-like UX a existujúcim lekciám FurniGenu bez uzavretia mechanického smeru. |
| Konkrétne verzie a licenčné tvrdenia | **Overiť** | Recenzentove odkazy nie sú týmto dokumentom nezávisle potvrdené. |

### 21.3 Tretia recenzia

| Položka recenzenta | Stanovisko V2 | Zdôvodnenie |
|---|---|---|
| V1 je katalóg princípov s odloženými jadrovými rozhodnutiami | **Prijať** | V2 uzatvára scope, snapshot model, protokol a TNP kontrakt; licenciu ponecháva explicitne človeku. |
| Jeden cieľový používateľ prvých 24 mesiacov | **Prijať** | Definovaný je architekt/interiérový a nábytkový modelár s presným jadrom. |
| OCCT binding bude trvalá údržbová réžia | **Prijať problém** | Vlastný façade znamená vedomé vlastníctvo hranice; konkrétne FTE/verzie treba overiť. |
| Immutable persistent document, single writer/N readers | **Prijať** | Rieši responzivitu, undo/redo, workery a konzistentné Queries. |
| Worker výsledky viazať na revision/hash a stale zahodiť | **Prijať** | Je to povinné pravidlo scheduleru. |
| Renderer číta posledný kompletný snapshot | **Prijať** | Viewport neblokuje a stale stav je vizuálne priznaný. |
| Exact backend musí vracať shape/history/tolerance/diagnostics | **Prijať** | Bez history sa TNP nedá realizovať bez neskoršieho zlomu API. |
| Genesis path + semantic role + geometric fingerprint | **Prijať s úpravou** | Používame viacvrstvový resolver; fingerprint je fallback, nie dôkaz identity. |
| `Resolved/Ambiguous/Lost` ako viditeľný výsledok | **Prijať** | Tiché hádanie je neprípustné. |
| Interná jednotka vždy mm a limit ±1e6 mm | **PoC overiť, zatiaľ neprijať ako univerzálny fakt** | Lokálne súradnice sú správne, konkrétny rozsah musí vzniknúť z corpus testov a aktuálnej dokumentácie. |
| Georeferencia nad lokálnym modelom | **Prijať** | Chráni lokálnu presnosť a zodpovedá BIM potrebám. |
| Rozdeliť interaction preview, canonical Commands a AI intents | **Prijať** | V2 má tri úrovne; preview nie je trvalá mutačná vrstva. |
| AI nesmie dostať 200–400 príkazov naraz | **Prijať** | Tool surface sa skladá z menších doménových intents; canonical API zostáva interným stabilným základom. |
| Command log nie je zdroj pravdy | **Prijať** | Autoritou je migrovateľný canonical document. |
| Výrazový jazyk parametrov | **Prijať** | Je potrebný skoro, ale musí byť malý, jednotkovo bezpečný a neturingovský. |
| Samostatný inference engine, scoring, hysterézia a hover lock | **Prijať** | Je to jadro jednoduchého SketchUp-like UX a nesmie byť skryté v renderer widgetoch. |
| Framework-agnostic view model | **Prijať** | Umožní vymeniť `egui` bez prepisu workflow. |
| Drawing je podhodnotený | **Prijať** | Profesionálny drawing sa stáva samostatným neskorším míľnikom. |
| Manifold ako druhý mesh backend | **PoC/neskôr** | Môže byť užitočný pre massing a mesh, ale nesmie byť vydávaný za exact B-Rep náhradu. |
| Feature stavy vrátane BrokenRef/Suppressed a last-good policy | **Prijať** | V2 formalizuje stavový automat a vizuálne označený stale výsledok. |
| Formalizovať taxonómiu chýb, crash/telemetry policy | **Prijať ako implementačnú požiadavku** | Konkrétna telemetry musí byť privacy-preserving a opt-in podľa typu dát. |
| Chýbajúci plugin placeholder a preservovanie dát | **Prijať** | Dokument sa nesmie poškodiť len preto, že rozšírenie nie je nainštalované. |
| Externé referencie, materiály, GPU minimum, lokalizácia | **Prijať ako neskoršie špecifikácie** | Sú potrebné, ale neblokujú bránu A; nesmú sa stratiť z roadmapy. |
| Samostatná trieda nevratných operácií | **Prijať** | Vyžaduje potvrdenie a jasný diff/export dopad. |
| 20 kanonických AI úloh | **Prijať** | Kvalita AI sa musí hodnotiť deterministickým výsledkom, nie dojmom z konverzácie. |
| Importéry sandboxovať | **Prijať** | Parsovanie CAD/BIM je bezpečnostná hranica. |
| MPL-2.0 core + Apache/MIT SDK | **Prijať do shortlistu, nie ako rozhodnutie** | Je to rozumný kompromis, ale licencia je produktové a právne rozhodnutie človeka. |
| Web, spolupráca, platformy a stabilita formátu | **Človek/neskôr** | Web nie je prvý produkt; kontrakty ho nemajú blokovať. Collaboration model sa nesmie predčasne zabudovať bez potreby. |
| Konkrétne verzie, licencie, dátumy a FTE odhady | **Neoverené** | V2 ich nepoužíva ako projektové fakty bez primárnych zdrojov a právneho auditu. |

### 21.4 Kde sa s recenzentmi vedome nezhodujeme

1. **Nezahadzujeme OCCT v prospech Trucku.** Riziko FFI je reálne, ale schopnosti exact kernelu sú pre dlhodobú univerzálnosť zásadné. Alternatívu najprv meriame.
2. **Neprikazujeme worker bez benchmarku.** Worker je pravdepodobný produkčný smer, nie dogma; rovnaký kontrakt musí umožniť oba režimy.
3. **Nedávame ľubovoľné tolerancie na každú entitu.** Centrálna politika a report výsledku sú konzistentnejšie.
4. **Nesľubujeme vyriešenie TNP.** Sľubujeme úzke garancie, best effort a bezpečné explicitné zlyhanie.
5. **Nevyhlasujeme konkrétnu internú jednotku, limit súradníc, verziu knižnice ani licenciu za uzavretú bez merania alebo právneho rozhodnutia.**
6. **Nevyhadzujeme drawing, BIM ani procedurálnu prírodu z vízie.** Odmietame iba ich súbežnú implementáciu v prvom produkte.

---

## 22. Stále otvorené rozhodnutia pre človeka

1. **Licencia projektu:**
   - Apache-2.0/MIT maximalizuje adopciu, ale umožňuje uzavreté forky;
   - MPL-2.0 poskytuje file-level copyleft a povoľuje širšie linkovanie;
   - GPL/AGPL silnejšie chráni otvorenosť odvodeného produktu, ale obmedzuje adopciu a kompatibilitu.
2. **Tím a runway:** koľko ľudí a času je reálne dostupných na core, UI, geometry a testy.
3. **Platformy:** Windows-first verzus súbežná Windows/Linux/macOS podpora.
4. **Stabilita formátu:** odkedy projekt verejne sľúbi dlhodobú spätnú čitateľnosť.
5. **Cloud a súkromie:** ktoré AI modely a remote služby budú podporované a aký je default opt-in režim.
6. **Web:** nie je cieľom prvého produktu; neskôr rozhodnúť browser klient verzus remote compute.
7. **Spolupráca:** či bude budúci cieľ simultánny multi-user editing alebo iba verzovanie/review.
8. **FurniGen ako validačná doména:** či jeho reálni používatelia a nábytkové úlohy budú prvým používateľským testom.

Technické rozhodnutia, ktoré uzavrie PoC:

- worker verzus in-process exact backend;
- pracovná jednotka a podporovaný lokálny coordinate envelope;
- konkrétny OCCT release a build/distribution model;
- solver;
- serializácia `document.bin`;
- minimálny GPU/OS benchmark profil;
- presný guaranteed TNP subset;
- pripravenosť `egui` pre prvý produkt a WASM pre plugin pilot.

---

## 23. Záver

Verzia 2 potvrdzuje základnú tézu: Kečup má byť Rustová, presná a AI-native modelovacia platforma s OCCT za úzkou hranicou, `wgpu` rendererom, sémantickým parametrickým dokumentom a jedinou kanonickou mutačnou cestou.

Zároveň odstraňuje najväčšie slabiny verzie 1. Prvý produkt je zúžený, interakčný preview je oddelený od transakcií, dokument má jasný snapshot model, scheduler odmieta zastarané výsledky, exact backend povinne vracia topologickú históriu a TNP má realistické garancie. AI pracuje cez Intent/Proposal vrstvu, dostáva resource budgety a nemôže obísť revision-bound dry-run a commit.

Najbližší krok nie je budovanie celej aplikácie. Je ním Brána A trojbránového PoC. Až merania z brán A–C rozhodnú o worker režime, tolerančnom profile, konkrétnych knižniciach a tom, či je tento základ dostatočne robustný na prvý použiteľný modelovací produkt.
