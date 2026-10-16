# Kečup — analýza fundamentov a návrh nápravy

**Dátum:** 24. 9. 2026
**Rozsah:** celý repozitár (`crates/*`, `sdk/python`, `skills`, `docs`, `governance`, CI) na vetve
`claude/optimistic-goodall-2aqwdt` (HEAD `0802aa5`). Nič sa nekompilovalo ani nespúšťalo, všetko
je zo zdrojového kódu. Odkazy `súbor:riadok` sú kotvy a pri ďalšej práci sa môžu posunúť.

---

## 0. Zhrnutie na jednu stranu

Nie je pravda, že sa „veľa nerobilo“. Repozitár má **~280 000 riadkov Rustu v `src` a ~180 000
riadkov testov**, 1 800+ integračných testov, 110 kanonických príkazov a 36 typov featur. Problém
nie je v objeme práce. Problém je v tom, **kam tá práca išla**:

1. **Model je z kusov, nie z pravidiel.** AI musí zadať každú dosku, každú dieru a každý kolík ako
   absolútne čísla. Pravidlová vrstva (výrazy, uzly) v jadre existuje, ale je to kalkulačka
   `+ − × ÷` bez mien. **Nevie vyrobiť kus** a **AI sa k nej vôbec nedostane**. Presne toto bolo
   jadro zadania z 3. 8. („AI píše pravidlá, nie geometriu“) a nepostavilo sa.
2. **Chyby sa pred AI zamlčujú.** Živý most zahodí dôvod zamietnutia a vráti holé
   `planning_rejected` / `validation_failed` (`crates/ketchup-app/src/live_bridge.rs:1655`,
   `:1054`). Skill to potom prepíše na „Live bridge rejected the request.“ (`skills/ketchup_live.py:299`).
   Ďalších ~130 rôznych validačných hlášok padá do jediného kódu `intent.cad_edit_program_invalid`
   bez cesty k chybnému poľu. **Agent teda nevie, čo má opraviť.** To je z väčšej časti ten pocit
   „vždy zlyhá na nejakej pečiatke“.
3. **Každá úprava prejde validáciou celého modelu 3× a viac** (príprava návrhu, overenie, commit,
   plus každé obnovenie plánovača). Validácia je brána, nie správa. Stačí jedna stará
   nekonzistencia kdekoľvek v modeli a zablokuje aj úplne nesúvisiacu úpravu.
4. **Osem paralelných rozhraní pre AI**, ktoré sa navzájom nezhodujú: iné pečiatky, iné kódy chýb,
   iné limity veľkosti, iný tvar programu. Vstavaný asistent má vlastný validátor v Pythone, ktorý
   pozná len 32 zo 41 operácií. **Dosky a kolíky nevie vôbec.**
5. **Uzavretý slovník plný špeciálnych prípadov.** Fľaša, čajník, nafukovacie písmená, Hettich
   a štyri pevné rozmery kolíkov sú zašité priamo v jadre. Pridať nový pojem znamená zasiahnuť
   zhruba 10 vyčerpávajúcich `match` miest. Preto namiesto zovšeobecnenia vzniká ďalšia vetva.
6. **Byrokracia dôkazov.** Slovo `digest` sa v `src` vyskytuje ~4 700-krát a `evidence` ~2 000-krát.
   Približne 25–35 % kódu jadra dokazuje, podpisuje alebo kontroluje, a nemodeluje. Asi 40 %
   assertov v testoch overuje proces (revízie, digesty, zamietnutia), len ~15 % geometrický výsledok.
   R0 sa preregistroval 13-krát aj kvôli koncom riadkov a CI kontroluje, že historická brána A0 v1
   stále hlási NO-GO.

**Čo je zdravé a treba to zachovať:** jediná mutačná cesta (`apply_batch`) s atomickou dávkou
a undo, nemenné revízie, OCCT izolované v samostatnom procese, `ExactBRepGraph` ako všeobecná
geometrická IR, DAG vyhodnocovača, riešič skíc, myšlienka `DerivedIdentity` (identita z pravidla
a slotu) a triedy referencií Resolved/Ambiguous/Lost. Kolíkový spoj, ktorý vyvŕta diery v oboch
doskách a overí ich, je skutočný úspech a dá sa naň nadviazať.

**Návrh v jednej vete:** nad existujúcim jadrom postaviť **pravidlový program ako jediný
autoritatívny opis modelu, ktorý píše AI**. Kusy, diery a spoje z neho vypadnú deterministicky
cez existujúci `apply_batch`. Validátory budú vracať správu namiesto zamietnutia. Rozhranie pre
AI sa zúži na **jeden protokol so štyrmi nástrojmi a úplnými chybovými hláškami**. Zvyšok
(legacy vetvy, governance brány, binárny formát) sa bude postupne odoberať. Podrobnosti sú v kap. 3 a 4.

---

## 1. Čo v projekte je — mapa

| Crate | `src` | testy | Rola |
|---|---:|---:|---|
| `ketchup-core` | 125k | 66k | dokument, príkazy, validácia, perzistencia, digesty, joinery, BTLx/HOMAG, výkresy |
| `ketchup-app` | 92k | 54k | GUI (egui/wgpu), 125 `AppCommand`, asistent, živý most; `lib.rs` má sám **43 575 riadkov** |
| `ketchup-scheduler` | 23k | 38k | správa OCCT workera, protokol, handshake |
| `ketchup-application` | 19k | 14k | plánovač AI programov, validátory, dotazy, session |
| `ketchup-exact` | 12k | 5k | OCCT FFI (+ 7,6k riadkov C++) |
| `ketchup-interaction` | 6k | 3k | analytický picking, priestorový index |
| `ketchup-headless` | 4k | 0,4k | JSON-RPC nad stdio |

K tomu patria `sdk/python` (~7k), `skills/` (2 súbory), 344 KB architektonickej špecifikácie
a 50 MB `artifacts/` s dôkazmi brán.

### Štyri slovníky príkazov

| Slovník | Variantov | Kde |
|---|---:|---|
| `CanonicalCommand` | 110 | `crates/ketchup-core/src/document.rs:2397` |
| `AppCommand` | 125 | `crates/ketchup-app/src/lib.rs:3385` |
| `WorkflowIntent` | 49 | `crates/ketchup-core/src/intent.rs:169` |
| `AssistantCadEditOperation` (AI) | 41 | `crates/ketchup-core/src/assistant_sidecar.rs:1855` |

K tomu `CanonicalError` so 142 variantmi a spolu ~350 rôznych chybových kódov naprieč vrstvami.

### Sedem reprezentácií geometrie

1. história featur (autoritatívna),
2. vyriešené regióny skíc,
3. legacy `ExactFeatureChainRequest` („Rectangle“) — plochá štruktúra so slotmi na *jeden* kruh,
   *jeden* pocket a *jeden* boolean (`exact_product.rs:8456`),
4. legacy `ExactRevolveRequest` (fľaša),
5. `ExactBRepGraph`, všeobecná cesta so schémami **V8 až V23**, z ktorých každá má vlastný handshake,
6. importované exaktné telesá a `MeshBody`,
7. analytické proxy (AABB v `ketchup-interaction`, `collision_hull`, OBB pri gravitácii).

Vyhodnotenie sa rozhoduje kaskádou „skús Rectangle → Revolve → Import → Graph → inak nič“
(`exact_product.rs:8380`). Obyčajná doska môže stáť dve volania workera: jedno na tvar, druhé
na topológiu (`crates/ketchup-application/src/evaluation.rs:813`).

---

## 2. Fundamentálne problémy (podľa toho, ako veľmi brzdia)

### P1 — Model z kusov; pravidlá sú kalkulačka, ktorú AI nevidí

**Ako vyzerá doska 600×400×18 s dvoma dierami** (`planner.rs:440-602`): 1 definícia, 1 teleso,
**9 featur** (workplane + skica zo 4 čiar + pad, a za každú dieru ďalší workplane, skica kruhu
a pocket) a 1 výskyt. Všetky čísla sú literály. Workplane diery je absolútny rám, takže **pri
zmene šírky dosky diera nejde s ňou**.

**Pravidlová vrstva, ktorá existuje:**
- `ExpressionAst` pozná len `+ − × ÷` a odkazy `$<číselné id>` (`graph.rs:50`). Nemá mená, `min/max`,
  podmienky ani jednotky.
- `FeatureParameterBinding` je *push*: príkaz `RecomputeFeatureParameters` prepíše číslo vo featúre
  (`document.rs:10568`). Väzba nie je živá.
- Pravidlo vie vyrobiť iba číslo. **Nevie vyrobiť kus**, nevie riadiť polohu výskytu (cieľ väzby je
  iba featúra, `document.rs:382`) a nevie vytvoriť spoj.
- AI program (`AssistantCadEditOperation`) vie vytvoriť len konštantný vstupný uzol
  (`planner.rs:3465`). Výrazy a pravidlá sú dostupné iba cez `WorkflowIntent` a fixture.
- Rozhodnutie A/B „kde bývajú pravidlá“, ktoré zadanie z 3. 8. žiadalo ako ADR, **v `docs/adr`
  nie je**.

**Dôsledok:** „skrinka o 100 mm širšia“ dnes znamená, že AI pošle úpravu každej súradnice skice,
každej transformácie a každého rámu diery. Pri tom musí v hlave skladať rotácie. Požiadavka vo
fixture `fast_assembly/manifest.json` („predĺž zadnú dosku… pridaj tri kolíky…“) má rozpočet 3 kolá
nástrojov a 60 s. S pravidlami by to bola **jedna zmena parametra**.

Mimochodom, aj mená dielov nesú rozmery („Bočnica 350x432x18 - 9 reálnych otvorov“). To je typický
príznak modelu bez parametrov: rozmer žije na dvoch miestach a rozíde sa.

### P2 — Chyby sa pred agentom strácajú

Toto je najpriamejšia príčina opakovaných zlyhaní:

- Živý most: `derive_assistant_cad_edit_proposal(&program).map_err(|_| "planning_rejected")`
  (`live_bridge.rs:1655`). Pri `apply_and_verify` rovnako vráti `invalid_program` alebo
  `planning_rejected` (`:1126`, `:1138`, `:1154`). Pri kolízii vráti holé `validation_failed`
  **bez zoznamu kolízií** (`:1054`).
- Skill potom text prepíše na „Live bridge rejected the request.“ a každú `ValueError` zmení na
  všeobecné `invalid_arguments` (`skills/ketchup_live.py:297-305`).
- ~130 rôznych správ „assistant … is invalid“ sa mapuje na jediný kód
  `intent.cad_edit_program_invalid` (`planner.rs:1623`) bez JSON cesty k poľu.
- Schéma operácií nemá **žiadne popisy polí** (`crates/ketchup-headless/build.rs:215`). Skryté
  pravidlá, napríklad hĺbka diery = polovica kolíka + 1 mm (`joinery.rs:61-69`) alebo zhoda diery
  s radom kolíkov na 1e-8 mm (`joinery.rs:6`), sa agent dozvie až zamietnutím.
- Názov spoja „Row (left)“ zlyhá s hláškou „dowel joint ID is invalid“, lebo zátvorky nie sú
  povolené (`joinery.rs:218`).
- `output_too_large` môže vrátiť `ok:false`, hoci sa mutácia **vykonala** (`skills/ketchup_model.py:66`).
- Skills hľadajú „plan state“ prechádzaním zásobníka volaní na rámec `claude_engine.ClaudeEngine`
  (`skills/ketchup_model.py:36-50`). V akomkoľvek inom hostiteľovi je každý zápis zamietnutý
  s `plan_guard_unavailable`.

Pravidlo, ktoré tu chýba: **LLM sa opraví len podľa toho, čo mu povieš.** Kód, ktorý zamietne
a nepovie prečo, je pre agenta horší než kód, ktorý nič nekontroluje.

### P3 — Validácia ako brána celého modelu, opakovane

`apply_batch` je jedna funkcia s **2 279 riadkami** (`document.rs:5341`). Po každom príkaze
kontroluje kotvy výrobných kódov. Nakoniec obnoví rámy plôch, skicové projekcie a graf, prevalidovať
override-y, a hlavne spustí `validate_product_with_drawing_sources` (1 390 riadkov,
`document.rs:16331`) **nad celým modelom**. Úprava od AI prejde:

1. plánovačom, ktorý pri staging-u opakovane volá `preview_batch` (`planner.rs:121`),
2. `prepare_proposal_with_context`: aplikácia na klon a päť digestov (`document.rs:8240`),
3. `verify_proposal_candidate`: znova aplikácia na klon (`:8505`),
4. `commit_verified_proposal_inner`: ostrá aplikácia a porovnanie digestov (`:8528`).

Rizikové úpravy navyše vyžadujú ed25519-podpísaný súhlas človeka s ochranou proti replay.

Pritom:
- Validátory skrinky (police, prevrátenie, ukotvenie) sú riadené reťazcami rolí s osou v názve
  (`furniture.shelf.xy`, `validation.rs:534`) a natvrdo zadanými konštantami (500 N, 15°).
- Gravitácia uznáva len stretnutie spodku OBB s vrchom OBB (`exact_validation.rs:1972`).
- Kolízia **nepozná deklarovanú výnimku spoja**: „no collision is exempted merely because it belongs
  to a dowel joint“ (`validation.rs:1043`). Zadanie z 3. 8. pritom navrhlo presne opak: spoj
  s vlastným ohraničeným objemom a štyrmi verdiktmi.

### P4 — Osem rozhraní pre to isté

1. headless JSON-RPC (35 metód),
2. Python `client.py`,
3. `skills/ketchup_model.py` (7 nástrojov),
4. živý TCP most (23 požiadaviek, rámec 32 KiB),
5. Python `live.py`,
6. `skills/ketchup_live.py` (7 nástrojov),
7. vstavaný asistent: `ketchup_assistant_protocol.py` s vlastným **~1 070-riadkovým validátorom
   v Pythone**, ktorý duplikuje ten v Ruste, a legacy `model_intent`
   (boxy / fľaše / čajníky / balónové písmená / strechy),
8. pluginový `ketchup_sdk` (max. 1 zápis).

Nezhody medzi nimi:

| | headless | živý most |
|---|---|---|
| pečiatka | 3 samostatné `expected_*` | objekt `expected` so 4 poľami |
| kód „zastarané“ | `stale_state` | `stale_precondition` / `stale_document` |
| program | zoznam alebo dict | iba `{"operations": [...]}` |
| vytvorené ID v odpovedi | áno | nie (`live_bridge.rs:1691`) |
| kolízia | nespúšťa sa automaticky | vždy |
| limit požiadavky | 128 KiB (skill) / 4 MiB | 32 KiB |

Docstring SDK tvrdí, že na novovytvorený objekt sa v tom istom programe nedá odkázať
(`client.py:343`). Text schopností tvrdí opak (`protocol.rs:832`).

### P5 — Uzavretý slovník → špeciálne prípady v jadre

Počty výskytov v `src` (core + application): dowel 628, beam 343, weldment 267, bottle 219,
btlx 130, drawer 111, hettich 99, teapot 26. `FeatureKind` obsahuje `BottleProfileControl`
a `BottleEdgeFinish`, `CanonicalCommand` obsahuje `SetBottleControlDimension`, `ExactFaceRole` má
plochy Shoulder/Neck/Mouth a `AssistantTeapotIntent` žije v jadre. Kolíky sú len štyri pevné
varianty `D6x30 … D10x40` (`joinery.rs:40`).

Duplicitné cesty k tomu istému: tri spôsoby 2D profilu, `Extrusion` a `Pad`, tri druhy výrezu
(`Pocket`, `SketchPocket`, `ThroughCut`), `Shell` a `TopologyShell`, tri systémy referencií na
plochy (`StableFaceRole(String)`, enum `ExactFaceRole` s 50 menami, `TopologicalElementRef`
s 15 poľami) a sedem rôznych pojmov „spoj“ (`CanonicalJoint`, `AssemblyMate`, `AssemblyJoint`,
`DowelJointContract`, `WeldmentJoint`, `MechanicalInterface`, recipe relation).

Nový pojem si vyžaduje zmenu v ~10 vyčerpávajúcich `match` miestach: `ProductModel`, príkaz,
vetva `apply_batch`, `validate_product`, ručný digest (`stable_digest.rs`, 3,6k riadkov), ručná
binárna perzistencia (97 konštánt `_SCHEMA`), `AuthoritativeDependency` (37 variantov),
`ProposalGoal` (50), `ProposalValue` (29) a `state_view`. Je to drahé, a tak je lacnejšie pridať
špeciálny prípad než zovšeobecniť.

### P6 — Nadbytočné údaje, ktoré musí AI zadať konzistentne

`create_physical_dowel_joint` potrebuje pre *každú* stranu počiatok plochy, vnútornú normálu,
min/max hranice, stred prvého kolíka a smer radu, všetko v lokálnom rámci dosky
(`cad_program.rs:3466-3494`, ~30 riadkov JSON). Ak má byť diera aj v `create_panel`, musí sa
s radom kolíkov zhodovať na 1e-8 mm. Pritom **stykovú plochu dvoch dosiek vie jadro zistiť samo**:
`collision_hull` aj gravitácia už kontakty plôch analyticky počítajú. Agent teda ručne opisuje
niečo, čo je z geometrie odvoditeľné, a každá jeho odchýlka je chyba.

Navyše platí:
- ID prideľuje systém, takže na kolík medzi existujúcimi doskami treba spätné čítanie.
- `create_panel` vystavuje len posledný pocket (`planner.rs:593`).
- Jednoprogramová varianta preto funguje len pre jeden kolík na dosku.

### P7 — Geometrické potrubie je ťažšie, než treba

Na dosku s dierou sa volá OCCT, hoci jadro už **analyticky pozná** očakávaný objem aj hranice
a výsledok OCCT dokonca zamieta, ak sa nezhoduje (`exact_product.rs:7705`, `:8007`). Každé
spustenie workera zahŕňa SHA-256 celého binárneho súboru, Job Object, `PING`, `CAPS M3_V1` a pri
každom vyhodnotení ďalší `CAPS EXACT_BREP_GRAPH_Vn`. Výkon je v poriadku (p95 ~3 ms na box),
problém je **zložitosť**. 23 verzií schémy internej IR, ktorú ADR 0002 sám nazýva neverejnou, je
čistá záťaž.

### P8 — Proces a dôkazy namiesto produktu

- `EXECUTION_CONTRACT.md` §12: po zmene po meraní sa brána musí preregistrovať. R0 prešiel 13
  verziami, často kvôli pinu `cxx`, koncom riadkov či poradiu atribútov
  (`governance/r0-transitions-v1-v13.json`). K tomu patrí 12 takmer rovnakých skriptov
  `validate-r0-vN-preregistration.ps1`.
- `test-architecture-guards.ps1` (695 riadkov) obsahuje 33 kontrol „jedinej mutácie“,
  21 „anti-loosening“ a 7 SHA-256 zámkov na vstupy. Každá úprava chráneného súboru potrebuje
  záznam v `governance/contract-changes.json`.
- CI beží iba na Windows a plný build potrebuje self-hosted runner `ketchup-occt-r0-v1`.
  `build.rs` má natvrdo cestu `win64/vc14` a toolchain je pinnutý na `x86_64-pc-windows-msvc`.
  **Žiadny AI agent v cloudovom kontajneri (Linux) nevie projekt zostaviť ani otestovať.** Pre
  projekt, ktorý má byť „AI-ready“, je to podstatné: AI ho nevie ani vyvíjať, ani overiť svoju prácu.
- Dokumenty v štýle `docs/architecture/supervisor-model-tools.md` majú ako prvý odsek jednu stenu
  SHA-256, nonce, počtov testov a epoch. Čítať sa to nedá a rozhodovať podľa toho tiež nie.
- Súbor `.ketchup` je vlastný binárny kontajner. Nočný stolík s 15 dielmi a 144 featurami má
  **1,3 MB**. Nedá sa porovnať (diff), AI ho nevie prečítať a nevie ho ani napísať.

Nie je to celé zlé. Disciplína „mesh nikdy potichu nenahradí exaktné teleso“, „stale výsledok
sa nezapíše“ a „jedna mutačná cesta“ je správna. Zlé je, že **rovnaká váha dôkazu sa kladie na
všetko**, aj na veci, ktoré nie sú rizikové, a že sa dokazuje proces namiesto výsledku.

---

## 3. Navrhovaný systém

### 3.1 Princíp

```
   AI / človek / skript
          │  píše a upravuje
          ▼
  ┌──────────────────────┐
  │  PROGRAM (pravidlá)  │   jediný autoritatívny opis: parametre, diely, spoje
  └──────────┬───────────┘   text, diffovateľný, uložený v dokumente
             │  deterministické vyhodnotenie (rýchle, bez OCCT pre prizmatické diely)
             ▼
  ┌──────────────────────┐
  │  KUSY + SPOJE        │   odvodené; stabilná identita = cesta v programe
  │  (cez apply_batch)   │   (skrinka/bok_lavy, skrinka/kolik[bok_lavy,dno]/2)
  └──────────┬───────────┘
             │
     ┌───────┴─────────┬─────────────────┐
     ▼                 ▼                 ▼
  VALIDÁTORY        GEOMETRIA         VÝSTUPY
  (správa, nie      analytická pre    kusovník, zoznam vŕtaní,
   brána)           dosky; OCCT len   BTLx/HOMAG, STEP, výkres
                    na zložité veci
```

Toto je variant **A+** zo zadania z 3. 8.: pravidlo sa vyhodnotí do `CommandBatch` a ten prejde
existujúcou jedinou mutačnou cestou, takže undo, perzistencia a GUI fungujú bez zmeny. Doplnky:

- Program je **uložený v dokumente** ako jedna kanonická entita (text a verzia evaluátora).
- Každý vygenerovaný objekt nesie `DerivedIdentity` = (cesta pravidla, kľúč). Ten pojem v jadre
  už existuje. Pri prepočte sa objekty párujú podľa kľúča, nie podľa poradia, takže ID výskytov
  zostávajú stabilné.
- Podstrom, ktorý program vygeneroval, **patrí programu**. Ručná úprava v GUI sa buď preloží na
  zmenu parametra (ak ide o kótu naviazanú na parameter), alebo sa zapíše ako explicitný
  **override** s kľúčom v programe (`override("skrinka/dno", posun=(0,0,5))`). Nikdy sa
  nestratí potichu.

### 3.2 Jazyk programu

Odporúčanie: **Starlark** (crate `starlark-rust`). Je to podmnožina Pythonu, deterministická
a hermetická, bez I/O a bez nekonečných cyklov, navrhnutá presne na „konfigurácia ako kód“.
LLM ho píše rovnako dobre ako Python. Alternatíva je Rhai. **Neodporúčam** ďalší JSON slovník
operácií: práve ten dnes nesie všetku nadbytočnosť z P6.

Ukážka toho, ako by mala vyzerať celá skrinka s kolíkmi:

```python
W = param("sirka", 600)
H = param("vyska", 720)
D = param("hlbka", 350)
T = param("hrubka", 18, material="DTD")

bok_l = board("bok_lavy",  size=(T, D, H),         at=(0, 0, 0))
bok_p = board("bok_pravy", size=(T, D, H),         at=(W - T, 0, 0))
dno   = board("dno",       size=(W - 2*T, D, T),   at=(T, 0, 0))
vrch  = board("vrch",      size=(W - 2*T, D, T),   at=(T, 0, H - T))
polica_n = param("pocet_polic", 2)
for i in range(polica_n):
    z = T + (H - 2*T) * (i + 1) / (polica_n + 1)
    board("polica/%d" % i, size=(W - 2*T, D - 20, T), at=(T, 0, z))

for bok in (bok_l, bok_p):
    for kus in (dno, vrch):
        dowels(kus, bok, dowel="8x30", count=3, margin=50)   # diery v OBOCH doskách
```

Podstatné vlastnosti:
- `dowels(a, b, …)` si **stykovú plochu nájde sám** z geometrie. Ak sa dosky nedotýkajú, vráti
  zrozumiteľnú chybu s menami oboch dosiek a vzdialenosťou. Hĺbky dier vyplynú z kolíka a hrúbok.
  Agent nič nezadáva dvakrát.
- „Posuň kolíkovú dieru“ znamená zmeniť `margin`, a diery sa posunú na oboch doskách aj na
  všetkých ostatných spojoch.
- „Skrinka o 100 mm širšia“ znamená `sirka = 700`. Dno, vrch, police, kolíky aj kusovník sa
  prepočítajú.
- Rotácie a všeobecné telesá: `board(..., rotate=...)` a nižšie primitíva `extrude(profile, ...)`,
  `cut(...)`, `hole(...)`, ktoré sa priamo mapujú na existujúce featury a `ExactBRepGraph`.
  Knižnica funkcií ako `board`, `dowels`, `groove`, `lap_joint` je **napísaná v tom istom jazyku**,
  nie v Ruste. Nový typ spoja teda znamená novú funkciu v knižnici, nie 10 `match` vetiev v jadre.

### 3.3 Validátory ako správa

- Vyhodnotenie vždy **uspeje, ak program beží**, a vráti zoznam `issues`:
  `{severity, kind, parts:[cesty], where_mm, message, hint}`.
- Brána (blokovanie) sa zapína až pri **exporte do výroby**. Úprava, ktorá vytvorí kolíziu, sa
  zapíše a agent dostane presný zoznam, čo má opraviť. Tak sa to správa aj pri FurniGene, kde
  validátor funguje ako sito, nie ako zámok.
- Kolízia so **spojmi, ktoré nesú objem** (návrh z 3. 8., tabuľka so štyrmi verdiktmi): prienik
  vnútri objemu spoja je v poriadku, mimo neho je chyba a deklarovaný spoj bez prieniku je tiež
  chyba.
- Validácia beží **inkrementálne nad zmenenými kusmi a ich susedmi**, nie nad celým modelom 3×.
  Konzistenčné invarianty jadra (neplatné ID, cyklus v grafe) zostávajú v `apply_batch`, ale len
  lokálne k príkazu.

### 3.4 Jedno rozhranie pre AI

Jeden JSON-RPC protokol, rovnaký pre headless aj pre živé okno (líši sa len transport). K nemu
jeden Python klient a **štyri nástroje**:

| Nástroj | Čo robí |
|---|---|
| `model_read` | vráti program (text), parametre, súhrn kusov, otvorené `issues`; voliteľne detail jedného kusu |
| `model_write` | nahradí program alebo aplikuje textový patch; vyhodnotí; vráti diff kusov, `issues` a nové `etag` |
| `model_view` | obrázok (pohľad, výber, zvýraznenie problémových kusov) |
| `model_export` | kusovník, vŕtací plán, BTLx/HOMAG/STEP; tu je brána validácie |

Pravidlá rozhrania:
- **Žiadne povinné pečiatky.** Voliteľný `etag` slúži na optimistickú kontrolu súbežnosti.
  Nesúlad pritom nie je chyba, vráti sa aktuálny stav a agent sa rozhodne sám.
- **Každá chyba má:** `code`, `path` (napr. `line 14` alebo `operations[3].dowel`), `message`
  v ľudskej reči, `hint` s konkrétnou opravou a pri geometrii aj `parts` a `where_mm`. Nijaká vrstva
  nesmie chybu zúžiť. Na to stačí jeden test „každá chyba z jadra prejde až do skillu nezmenená“.
- Plan guard sa rieši v hostiteľovi (Supervisor), nie prechádzaním zásobníka v skille.
- Vstavaný asistent používa **ten istý** nástroj a nemá vlastný validátor v Pythone ani legacy
  `model_intent`.

### 3.5 Geometria

- Prizmatické diely (doska, drážka, diera, polodrážka, šikmý rez) sa počítajú **analyticky**:
  kváder mínus konvexné výrezy dáva objem, hranice, tesseláciu, stykové plochy a SAT kolízie.
  Analytiku v jadre už máte (`collision_hull`, očakávané objemy v `exact_product.rs`).
- OCCT sa volá iba na zaoblenia, lofty, sweepy a STEP import/export, a to cez jednu verziu IR.
  Handshake verzií sa zjednoduší na „build worker = build aplikácie“.
- Legacy evaluátory Rectangle a Revolve a featury Bottle* sa odstránia, keď ich pokryje graf.

### 3.6 Súbor a proces

- **Súbor:** program (text) + parametre + override-y ako čitateľný JSON/TOML. Odvodené kusy a
  exaktné výsledky idú ako cache s kľúčom `(digest programu, verzia evaluátora, build backendu)`,
  presne podľa odpovede z 3. 8. Cache sa dá kedykoľvek zahodiť a prepočítať.
- **Proces:** jadro sa chráni tromi vecami:
  1. jedna mutačná cesta,
  2. golden testy produktu: „program skrinky → očakávaný kusovník, vŕtací plán, 0 issues“,
  3. CI na Linuxe, kde beží všetko okrem GUI.

  Preregistrácie, `contract-changes.json`, frozen SHA zámky, 23 schém a kontrola historického
  NO-GO sa zmrazia a odstránia z CI. Históriu drží git.

---

## 4. Postup — po krokoch, každý sa dá odovzdať samostatne

### Fáza 0 — Rýchle opravy bez prestavby (2–4 dni)

Cieľ: agent prestane „zlyhávať na pečiatke“ už nad súčasným API.

1. **Priechod chýb.** Odstrániť všetky `map_err(|_| "...")` v `live_bridge.rs` a posielať celý
   `CanonicalError` / `PlanningError` so správou. Skills nesmú prepisovať `message`.
   `intent.cad_edit_program_invalid` doplniť o `path` (index operácie a pole). Kolízia vracia zoznam
   kolízií.
2. **Validácia nezamieta.** `apply_and_verify` zapíše zmenu aj pri kolízii a vráti `issues`.
   Tvrdý režim sa zapína voliteľne (`strict: true`).
3. **Pečiatky preč** (dnes sú voliteľné, stačí ich odstrániť zo schém a dokumentácie). Plan guard
   má pri neznámom hostiteľovi povoliť zápis, nie ho zakázať.
4. **Kolíky:** `create_physical_dowel_joint` bude prijímať iba `(doska_a, doska_b, dowel, count,
   margin | spacing)`. Stykovú plochu a rámy si dopočíta sám. Hĺbka diery sa odvodí. Pravidlá
   zapísať do popisov schémy.
5. **Zjednotiť** `skills/ketchup_model.py` a `skills/ketchup_live.py` na rovnaké názvy nástrojov
   a rovnaký tvar odpovedí.

Meradlo: fixture `fast_assembly` (predĺženie zadnej dosky a 9 nových kolíkov) prejde z čistého
kontextu na prvý pokus, alebo pri zlyhaní agent z odpovede vie, čo opraviť.

### Fáza 1 — Pravidlový program (2–3 týždne)

1. ADR „Program ako autoritatívny opis“ (A+, identita z cesty, override-y, sémantika Open podľa
   odpovede z 3. 8.).
2. `ketchup-program` crate: Starlark evaluátor, knižnica `param/board/hole/groove/dowels/extrude/cut`,
   výstup `CommandBatch` a mapovanie `DerivedIdentity` na výskyty.
3. Nástroje `model_read` a `model_write` nad headless aj živým oknom.
4. Analytická geometria a stykové plochy pre prizmatické diely. OCCT sa v tomto kroku nevolá.
5. Golden testy: skrinka, nočný stolík z fixture a nosník A z drevodomu (`415 × 6`, `408 × 5`,
   `400`). To je bod 6.3 zo zadania z 3. 8., ktorý stále čaká.

Meradlo: „skrinka o 100 mm širšia a o jednu policu viac“ je **jedna** zmena, 0 issues, a kusovník
aj vŕtací plán sedia.

### Fáza 2 — Jedno rozhranie (1–2 týždne)

1. Jeden protokol (headless = živé okno), jeden Python klient, štyri nástroje.
2. Vstavaný asistent prejde na tie isté nástroje. Zmaže sa `ketchup_assistant_protocol.py`
   validátor a `model_intent` (boxy, fľaše, čajníky, balóny).
3. Kolízia so spojmi nesúcimi objem a verdikt „spoj bez prieniku = chyba“.

### Fáza 3 — Odchudzovanie (priebežne)

1. **Linux build a CI:** prenosný `build.rs` pre OCCT, wgpu s Vulkan/GL. Crates `core`,
   `interaction` a `program` sa dajú testovať bez OCCT. Potom môže projekt vyvíjať a overovať aj
   cloudový AI agent.
2. Odstrániť z CI governance (preregistrácie, frozen hash, historické NO-GO, `contract-changes.json`).
3. Legacy evaluátory Rectangle/Revolve, featury `Bottle*`, `AssistantTeapotIntent`,
   `ExactFaceRole` s 50 menami → jeden systém referencií.
4. Zlúčiť duplicitné featury (Extrusion/Pad, tri výrezy, dva shelly) a pojmy spoja.
5. Čitateľný formát súboru.
6. Rozdeliť `ketchup-app/src/lib.rs` (43k), `document.rs` (20k) a `apply_batch` (2,3k riadkov jednej
   funkcie).
7. Testy: postupne nahrádzať procesné asserty (`digest == …`, `revision == …`) scenármi produktu.

### Čo NErobiť

- **Neprepisovať od nuly.** Jadro (mutačná cesta, undo, OCCT izolácia, BRep graf, skicový riešič,
  kolíkový verifikátor) je použiteľné a Fáza 1 ho priamo využíva.
- Nepridávať ďalšie operácie do JSON slovníka `AssistantCadEditOperation`. Každá nová schopnosť
  má byť funkcia v knižnici programu.
- Nepridávať nové „dôkazové“ vrstvy (digest, receipt, epoch) bez konkrétneho zlyhania, ktoré
  chytajú.

---

## 5. Kontrolné otázky, ktoré treba rozhodnúť (patria vlastníkovi)

1. **Starlark vs. Rhai vs. Python cez SDK.** Starlark odporúčam kvôli determinizmu
   a „pythonovitosti“. Python cez SDK je rýchlejší štart, ale program potom nežije v dokumente
   a nedá sa prepočítať bez externého interpreteru.
2. **Ručné úpravy odvodených kusov v GUI:** override (odporúčam), alebo zákaz a presmerovanie na
   parameter?
3. **Kedy validácia blokuje:** iba pri exporte do výroby (odporúčam), alebo aj pri uložení?
4. **Linux:** stačí headless jadro a testy na Linuxe (odporúčam ako minimum), alebo aj GUI?

---

## Príloha — ako vznikla táto analýza

Kód sa čítal priamo. Tri samostatné prechody pokryli (a) rozhranie pre AI, (b) dátový model
a mutačnú cestu, (c) geometrické potrubie, validátory, proces a testy. Kľúčové tvrdenia (zahadzovanie
chýb v živom moste, rozsah výrazov, pevné kolíky, 110/36/41 variantov, dĺžka `apply_batch`) boli
overené druhýkrát. Percentá podielu „kontrolného“ kódu a procesných assertov sú **heuristické
odhady** podľa kľúčových slov, nie presné meranie. Nič sa nezostavovalo ani nespúšťalo.
