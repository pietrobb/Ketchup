# Kečup — odpoveď na zadanie z 3. 8. 2026

**Reaguje na:** `ketchup-zadanie-2026-08-03.md`
**Autor:** implementačná strana (prístup do repozitára)
**Dátum:** 3. 8. 2026

Kontext domény prijímam celý a mení mi váhy rovnako, ako píšeš. Nižšie sú štyri veci: jeden fakt
z kódu, ktorý mení tvoj nález §3 aj zadanie §6.3, odpoveď na rozhodnutie §5 (mám dáta, ktoré si
nemal), tri drobné korekcie k §4 a §6, a prepracované poradie.

Všetko, čo tvrdím o kóde, je overiteľné jedným grepom — uvádzam súbor a riadok.

---

## 1. Rozhodujúci fakt: interakčná cesta nie je „druhá geometrická pravda". Je to kváder.

Toto je jediný fakt v celej výmene, ktorý naozaj mení plán, a nemohol si ho vedieť.

`ketchup-interaction` nemá analytický B-Rep. Má **jeden typ telesa**:

```rust
// crates/ketchup-interaction/src/lib.rs:206
pub struct SharedBoxGeometry {
    size_mm: Vec3,
    endpoints: [Vec3; 8],
    edge_midpoints: [Vec3; 12],
}
```

Osovo orientovaný kváder. Osem rohov, dvanásť stredov hrán. To je celý repertoár. Picking, snapping,
Smart Push/Pull aj Gate C bežia nad týmto.

Ďalej: **`SubshapeRef` v `ketchup-interaction` neexistuje.** Vyskytuje sa len v `ketchup-exact`
(6×), v jeho `native.cc` (7×) a v teste A0 (5×). Interakčná vrstva má vlastný, nesúvisiaci slovník:
`ElementId`, `SelectionId`, `ExactHit`, `PickResult`.

### Čo to robí s tvojím nálezom §3

Nález je **reálny, ale opačne, než je formulovaný**. Nie sú to dve nezávislé implementácie
geometrickej pravdy, medzi ktorými chýba kontrakt ekvivalencie. Je to **jedna pravda (`ExactBody`
nad OCCT) a jeden kvádrový proxy**, ktorý väčšinu tvarov z A0 nevie ani reprezentovať — rez cez
viac plôch, union, plocha rozdelená hranou nemajú v `SharedBoxGeometry` obraz.

Preto **bránu C1 v tvojom znení dnes nie je nad čím napísať.** „Ten istý `SubshapeRef`, nula
nezhôd" by na kvádroch prešlo triviálne a mimo kvádrov je nedefinované. Bola by to zelená brána bez
informácie — presne ten typ, ktorý sme si zakázali.

### Čo namiesto toho

Rozdelené na to, čo sa dá spraviť dnes, a to, čo má zmysel až potom:

**C1a — kontrakt autority (dnes, malé).** ADR + jeden test: `ketchup-interaction` je **odvodený
pohľad**, nie zdroj pravdy. Scéna sa smie plniť výhradne projekciou z kanonického dokumentu; test
tvrdí, že každý `Occurrence` v interakčnej scéne má `node_id` a rozmer zhodný s kanonickým
`ProductModel`, a že neexistuje cesta, ktorá interakčnú scénu mutuje bez `apply_batch`. To chráni
presne ten nulový prah, o ktorý ti ide, a nečaká na OCCT.

**C1b — ekvivalencia resolverov (až keď OCCT teleso vojde do aplikácie).** Vtedy je to naozaj to,
čo popisuješ, a vtedy to bude aj merateľné. Preregistrovať sa dá už teraz.

To je vecné zlepšenie tvojho návrhu, nie jeho odmietnutie: chránený je ten istý invariant, ale
testom, ktorý dnes môže zlyhať.

### Čo to robí so zadaním §6.3 — a toto je dôležitejšie

**Drážky sa dnes do modelu nedajú dostať.** Nosník A bez drážok je množina kvádrov a celý cyklus
(pravidlo → kusy → zmena → reťazec kót → kolízia) je nad kvádrami spustiteľný. S drážkami to nie je
„o kúsok viac práce" — je to zapojenie OCCT do aplikácie, teda presne C1b, teda tá najdrahšia
položka celého zoznamu.

Preto navrhujem **rozdeliť 6.3 presne na tejto hrane**:

- **6.3a — nosník A bez drážok.** Rozpon, počet polí, prierez, rozhodenie zvyšku, výpis kót
  `415 × 6`, `408 × 5`, `400`, zmena jedného čísla → prepočet, kolízny test. Všetko kvádre.
  Toto je tých „pár dní" a meria presne to, čo chceš merať: či sa jadro dá použiť na produkt.
- **6.3b — drážky.** Vyžaduje OCCT v aplikácii + C1b. Je to samostatná položka s vlastnou cenou,
  nie dovetok.

Tvoj argument, že polodrážka 20/20 je jeden vzťah a nie dva výrezy, tým nestráca platnosť —
naopak, v 6.3a sa dá zapísať ako vzťah bez geometrie („tieto dva kusy sa krížia polodrážkou")
a v 6.3b sa mu dorobí telo. Vzťah je to podstatné; výrez je jeho dôsledok.

---

## 2. Rozhodnutie §5: kde bývajú pravidlá

Navrhuješ **(A) s ústupkom** a necháš si otvorené (B), ak z kódu vyplynie, že je lacnejšie.
**Z kódu to vyplýva.** Odpoveď je (B), a to z dôvodu, ktorý si nemohol vidieť: *kanonický dokument
už dnes obsahuje parametrickú vrstvu.*

```rust
// crates/ketchup-core/src/document.rs:312
pub struct CanonicalNode {
    id: NodeId,
    name: String,
    dimension: Dimension,
    dependencies: Vec<NodeId>,   // validované, kanonicky zoradené, bez cyklov
}
```

A k tomu:

- `CanonicalCommand::CreateNode { dependencies, .. }`, `SetDimension`, `RenameNode` —
  vlastná trieda príkazov, odlíšená od produktových cez `is_product_command()` (`document.rs:476`);
- `dependent_closure(&nodes, &changed)` pri každom `apply_batch` (`document.rs:1165`) — skutočná
  propagácia zmeny po DAG-u, nie len uloženie hrán;
- `Revision::recomputed_nodes()` — každá revízia nesie množinu prepočítaných uzlov;
- uzly sú **perzistované** vrátane závislostí (`persistence.rs:49–56`, čítanie `153–184`);
- idú tou istou `apply_batch` bránou, tým istým undo/redo, tým istým `canonical_digest`.

Inými slovami: **kostra (B) je postavená a zaplatená.** DAG, undo, referencie aj perzistencia už
s parametrickou vrstvou počítajú. Tvoj odhad ceny (B) je odhad práce, ktorá je hotová.

### Čo v (B) skutočne chýba

Dve veci, obe malé a obe konkrétne:

1. **Uzol nemá výraz.** `Dimension` je hodnota so `source_token` (pôvodný zápis používateľa), nie
   výraz. `CanonicalNode` teda vie, *od čoho závisí*, ale nie *ako sa z toho počíta*. Chýba
   evaluátor a výraz na uzle.
2. **Neexistuje hrana uzol → produktová entita.** Príkazové triedy sú disjunktné. Uzol dnes nevie
   povedať „tieto occurrence sú môj výstup".

To je celý rozdiel medzi dneškom a (B). Nie architektúra — dve polia a evaluátor.

### Prečo (A) nie je lacnejšie, len odloží účet

Zlyhanie, ktoré sám menuješ pri (A) — *„niekto posunie jeden kus ručne, pravidlo o tom nevie a pri
prepočte ho prepíše"* — nie je vlastnosť (A). Je to **nezaplatená cena, ktorá je rovnaká v (A) aj
v (B)**: akonáhle sú kusy odvodené, musí existovať záznam „táto entita je odvodená z uzla N, slot
K" a explicitný override. (B) toto miesto má kam zapísať. (A) ho nemá, takže sa zapíše mimo
dokumentu a rozíde sa s ním pri prvom Open.

Preto je moje odporúčanie:

> **(B) s hranicou tam, kde už je.** Pravidlá sú uzly kanonického dokumentu (`CanonicalNode`
> rozšírený o výraz a väzbu na výstup). Produktové entity dostanú `derived_from: Option<(NodeId,
> SlotKey)>` a explicitný `override`. Vyhodnocovač pravidla produkuje `CommandBatch` — teda tvoj
> ústupok z (A) zostáva v platnosti a je to zároveň mechanizmus (B).

Formálne je to (B), prakticky sa to implementuje ako tvoj návrh (A) plus dve polia. Rozdiel je, že
provenience žije v dokumente a nie v batchi, ktorý po commite zanikne.

ADR napíšem s týmto odôvodnením. Ak z prvého nosníka vyplynie opak, ADR sa prepíše — ale písať ho
ako (A) proti tomu, čo je v `document.rs`, by bolo písanie od stola.

### Identita odvodenej inštancie

Súhlas, že je to topological naming o poschodie vyššie, a mám naň hotový tvar — nemusíme vymýšľať
nový. Architektúra V3 §7 už definuje päť tried stability s explicitným `Ambiguous` a `Lost`
a zásadou *„tiché prepojenie na inú plochu je chyba"*. To isté platí doslova pre sloty:

- identita = `(NodeId pravidla, SlotKey)`, kde `SlotKey` je **sémantický kľúč vyrobený pravidlom**
  (`pole_7` počítané od pomenovaného konca), nie index v poli a nie poradie vzniku;
- ak zmena parametra slot zruší, override sa stane `Lost` a **musí to byť viditeľné**; nikdy sa
  nepremapuje na susedný slot;
- ak sa raster zahustí a slot 7 už nie je ten istý fyzický stĺpik, je to `Ambiguous`, nie tichý
  presun.

Rovnaký resolver, rovnaké triedy, rovnaké pravidlo hlásenia. To je lacná konzistencia.

---

## 3. Tri korekcie k §4 a §6

### 3.1 Príznak smeru zmien v R0 (§4.2) — beriem, s jednou podmienkou

Požiadavka je správna a lacná. Podmienka: príznaky sa musia **rekonštruovať z histórie
repozitára**, nie doplniť spätne podľa toho, ako to má vyzerať. Ak sa pri niektorom prechode
nedá spoľahlivo určiť, či zmena vznikla pred alebo po meraní, zapíše sa `unknown` a nie `neutral`.
Auditovateľnosť, o ktorú ti ide, zabije práve dopísanie chýbajúceho údaja.

### 3.2 Envelope (§4.3) — tvoj výpočet ULP sedí, ale tvoja alternatíva je „loosen"

Aritmetika je správna: pri 1e6 mm je `1e-6 mm` rádovo 4500 ULP a rezerva na akumuláciu v
booleanoch je tenká. Adversarial korpus na hornom konci envelope doplníme — to je jednoznačné
sprísnenie a nemá žiadnu cenu okrem času.

Ale pozor na druhú vetu: **zúženie envelope je podľa tvojho vlastného pravidla z §4.2 `loosen`,
nie `tighten`.** Zmenšuje sľub, aby test prešiel ľahšie. Ak by sme zúžili envelope *po* tom, čo
korpus na hornom konci padne, je to učebnicový príklad zmäkčenia po meraní.

Poradie preto musí byť: (1) doplniť korpus na hornom konci, (2) zmerať, (3) až potom rozhodovať
o envelope — a ak sa zúži, zapísať to ako `loosen` s odôvodnením. Tvoj vlastný nástroj má zuby
hneď pri prvom použití, čo je dobrá správa o ňom.

### 3.3 Kolízny validátor (§6.2) — súhlas, ale narrow phase bez tesselácie

Celý §6.2 beriem vrátane odôvodnenia cez FurniGen. Jedna technická korekcia, ktorá ti odoberie
problém, ktorý si sám pomenoval:

Pre kvádre a všeobecnejšie konvexné mnohosteny **nie je potrebná tesselácia**. Narrow phase sa robí
exaktne cez SAT (separating axis theorem) nad hranami a normálami, v `f64`, deterministicky.
Tvoja obava, že tesselácia použitá na validáciu musí byť súčasťou determinism envelope, je správna
— a v prvom kole ju vieme celú obísť tým, že tam žiadna tesselácia nebude. Do envelope sa dostane
až s oblúkmi a šikmými rezmi, teda spolu s OCCT, teda vo fáze 6.3b.

Broad phase (AABB strom) → OBB medzistupeň → SAT. Prah nekolízneho prieniku ako pomenovaná
konštanta v tolerančnom profile, presne ako píšeš.

**Prosba, ktorá je na vlastníkovi, nie na nás dvoch:** korpus prípadov z FurniGenu, na ktorých
kolízia najprv nefungovala. Súhlasím, že je to najcennejšia prenosná vec, a je to jediná položka
celého zadania, ktorú si nevieme vyrobiť sami.

### 3.4 `StateView` ako dve projekcie (§4.4) — beriem bez výhrad

Zdieľaný enkodér, dva výstupy, verzionované zvlášť, golden fixtures viazané na úplný výpis.
Nemám čo dodať; je to lepšie než moja formulácia.

---

## 4. Prepracované poradie

Tvoje poradie mením na troch miestach a všetky tri vyplývajú z bodu 1 tohto dokumentu.

| # | Položka | Zmena oproti tvojmu §7 |
|---|---------|------------------------|
| 1 | **CI + dva architektonické testy** | bez zmeny — najvyššia páka, a je to moja priznaná diera |
| 2 | **C1a — kontrakt autority + ADR „`ExactBody` je autoritatívny"** | posunuté dopredu, prepísané |
| 3 | **`StateView` v1 (dve projekcie) + golden fixtures** | bez zmeny okrem 4.4 |
| 4 | **Kolízny validátor (SAT, bez tesselácie) + FurniGen korpus** | bez zmeny, upresnená narrow phase |
| 5 | **ADR: pravidlá = uzly dokumentu (B) + identita `(NodeId, SlotKey)`** | odporúčanie (B), nie (A) |
| 6 | **6.3a — nosník A bez drážok, koniec-koniec** | rozdelené |
| 7 | **Príznak smeru v R0 reporte + adversarial korpus na hornom konci envelope** | spojené, oboje lacné |
| 8 | **C1b — ekvivalencia resolverov, OCCT teleso v aplikácii** | nová položka, dedí z tvojho C1 |
| 9 | **6.3b — drážky** | závisí na 8 |
| 10 | **`Intent` slovník a brána D** | bez zmeny — až po validátoroch a pravidlách |

Prečo je C1a hneď druhé a nie tretie: je to jediná položka, ktorá stojí pár hodín a chráni pred
tichou chybou, ktorá sa inak prejaví až o pol roka ako „záhadný bug v pickingu" — tvoja vlastná
veta, a je presná.

Prečo je odloženie brány D za 4–6 správne: tvoje odôvodnenie beriem celé. Merať generovanie
geometrie bez validátorov = merať známy neúspech FurniGenu. Nemám čo doplniť.

---

## 5. Kde sme sa nezhodli a prečo to nevadí

Zostáva jediný otvorený rozdiel: ty navrhuješ **(A) s provenienciou v batchi**, ja **(B) s
provenienciou v dokumente**. Rozdiel v práci je dve polia a evaluátor; rozdiel vo výsledku je, či
odvodenosť prežije `Save`/`Open`. Píšem to ako ADR s obidvoma variantmi a s odôvodnením, prečo
odporúčam (B) — nie ako implementáciu, ktorá sa stane. Ak ti odôvodnenie nebude sedieť, prepíše sa
ADR a nie kód, čo je presne to, čo si žiadal.

Zvyšok zadania beriem tak, ako je napísaný. Kontext domény bol to, čo v celej diskusii chýbalo
najviac — bez neho bola „validátory sú krok 5 commit pipeline" obhájiteľná veta, a s ním je zjavne
nesprávna.

---

## Príloha: overiteľné tvrdenia z tohto dokumentu

| Tvrdenie | Kde |
|---|---|
| interakčná geometria je len osovo orientovaný kváder | `crates/ketchup-interaction/src/lib.rs:206–210` |
| `SubshapeRef` v `ketchup-interaction` neexistuje | grep: len `ketchup-exact` (6), `native.cc` (7), `gate_a0.rs` (5) |
| kanonický dokument má DAG uzlov so závislosťami | `crates/ketchup-core/src/document.rs:312–344` |
| propagácia zmeny po DAG-u pri každom batchi | `document.rs:1165` (`dependent_closure`) |
| revízia nesie množinu prepočítaných uzlov | `document.rs:680`, `700` |
| parametrická vrstva je perzistovaná vrátane hrán | `crates/ketchup-core/src/persistence.rs:49–56`, `153–184` |
| parametrické a produktové príkazy sú oddelené triedy | `document.rs:476` (`is_product_command`) |
| jediný verejný mutačný vstup | `document.rs:783` (`apply_batch`) |
