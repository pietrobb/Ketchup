# Kečup — plán upratovania a prechodu na pravidlový jazyk

**Dátum:** 24. 9. 2026
**Nadväzuje na:** `docs/analyza-fundamentov-2026-09-24.md`
**Pravidlá pre agentov:** `AGENTS.md` v koreni repozitára (platia od teraz, pre GPT/Codex aj Claude)

---

## 1. Cieľ

1. **Z jadra zmizne všetko, čo je robené na mieru.** Fľaša, čajník, balónové písmená, strechy,
   schodiská, kapsuly, ručne písané siete pre kombinácie tvarov a moduly pre jednotlivé testy.
   V Ruste zostanú len **všeobecné geometrické operácie**, ktoré dávajú zmysel pre ľubovoľný tvar.
2. **Model opisuje pravidlový program** (Starlark), ktorý píše AI alebo človek. Doska, kolík,
   drážka, polica aj skrinka sú **funkcie v knižnici tohto jazyka**, nie typy v jadre.
3. **Dôkazy sa obmedzia na to, čo chráni reálne dáta.** Všetko ostatné ide preč (zoznam v kap. 3).
4. **Postup, ktorý to udrží:** pravidlá v `AGENTS.md`, jedna mechanická poistka proti menám
   produktov v jadre a definícia „hotového“ kroku.

Čo sa **nemení**: jediná mutačná cesta s undo, OCCT v samostatnom procese, `ExactBRepGraph`
ako všeobecná geometrická IR, riešič skíc, GUI, importy/exporty, BTLx/HOMAG, výkresy a kolízie.

---

## 2. Hranica: čo smie byť v jadre

Jednoduchý test: **„Použil by to niekto, kto modeluje niečo úplne iné?“**

| Smie byť v jadre (Rust) | Patrí do knižnice jazyka | Nesmie byť nikde |
|---|---|---|
| skica, väzby, profil (čiara, oblúk, kruh, spline) | `board`, `panel`, `beam` | `Bottle*`, `Teapot*`, `Balloon*` |
| extrude, revolve, sweep, loft | `hole` so štandardnými rozmermi, `dowels`, `groove`, `lap_joint` | ručne písaná sieť pre konkrétnu kombináciu tvarov |
| cut / union / intersect / split | `shelf`, `cabinet`, `drawer`, `roof`, `stair` | evaluátor, ktorý vie iba jeden tvar |
| shell, fillet, chamfer, offset | katalógy kovania (Hettich, …) ako dáta | featura s pevným počtom bodov („šesť bodov profilu“) |
| transform, instancia, pole, zrkadlenie | doménové validátory s parametrami (prevrátenie, priehyb police) | moduly pre jeden test (`beam_m5`, `release_capstone`) |
| všeobecný spoj: diely + objem + spojovacie prvky | | |
| kolízia, kontakt, podpora — nad ľubovoľnými telesami | | |
| exportéry formátov (STEP, BTLx, HOMAG, DXF) | | |

Exportéry do konkrétnych strojových formátov sú v poriadku. Sú to formáty, nie produkty.

---

## 3. Dôkazy a kontroly: čo zostane a čo ide preč

### Zostáva (chráni dáta alebo používateľa)

| Kontrola | Prečo |
|---|---|
| jediná mutačná cesta, atomická dávka, jeden undo krok | bez nej sa dokument rozsype |
| výsledok z OCCT nesie číslo revízie; ak je starší, zahodí sa | asynchrónny výpočet nesmie prepísať novší stav — **jedno číslo, nie päť digestov** |
| izolácia OCCT procesu a timeout | pád OCCT nesmie zhodiť aplikáciu |
| mesh nikdy potichu nenahradí presné teleso; export odmietne zastaranú geometriu | výroba nesmie dostať zlé dáta |
| validácia **pred exportom do výroby** blokuje | tu je sito, ktoré chránilo FurniGen |
| lokálne invarianty príkazu (neexistujúce ID, cyklus) | lacné a zmysluplné |
| kontrolný súčet pri uložení súboru | odhalí poškodený súbor |

### Ide preč

| Čo | Kde |
|---|---|
| povinné pečiatky `expected_revision/digest/mutation_epoch`, receipty, replay | skills, headless, živý most |
| `plan_guard` hľadaný prechodom zásobníka | `skills/*.py` |
| ed25519 podpisy súhlasu s ochranou proti replay | `document.rs` (proposal approval) |
| 5 digestov návrhu (diff, provenance, command, dependency, intended result) a trojitá aplikácia | `document.rs:8240-8560` |
| digesty a odtlačky v odkazoch na plochy (`BodySubshapeRef` má 18 polí, z toho 5 digestov) | `exact_product.rs:415` |
| SHA-256 celého binárneho súboru workera pri každom spustení | `scheduler/src/lib.rs:1362` |
| 23 verzií schémy BRep grafu a handshake `CAPS` pri každom výpočte → **jedna verzia = build aplikácie** | `exact_brep_graph.rs`, `exact_worker.rs` |
| celomodelová validácia v každom `apply_batch` → lokálna k príkazu | `document.rs:16331` |
| preregistrácie, freeze ID, `contract-changes.json`, SHA zámky vstupov, kontrola historického NO-GO | `governance/`, `thresholds/`, `scripts/windows/validate-*`, CI |
| ručný digest pre každý typ (3,6k riadkov) → odvodený zo serde | `document/stable_digest.rs` |
| ručný binárny formát (97 schém) → textový formát | `persistence.rs` |

---

## 4. Postup po krokoch

Každý krok = **jeden PR**, po ktorom aplikácia funguje. Poradie je zvolené tak, aby sa najprv
mazalo to, na čom nič nezávisí, a nové veci vznikali pred zmazaním starých ciest.

Odhady sú hrubé a predpokladajú prácu s AI agentom na Windows stroji, kde sa dá zostaviť a testovať.

### Krok 0 — Pravidlá a poistka (1 deň)

- [x] `AGENTS.md` (+ `CLAUDE.md`, ktorý naň odkazuje) s pravidlami. **Hotové týmto commitom.**
- [ ] Skript `scripts/check_no_named_products.py`: hľadá zakázané mená (`bottle`, `teapot`,
      `balloon`, `capsule`, `gable_roof`, `staircase`, `nightstand`, `hettich`, …) v `crates/*/src`.
      Funguje ako **rohatka**: súbor so súčasnými výskytmi (`scripts/named_products_baseline.txt`)
      sa smie len zmenšovať. Nový výskyt = červené CI. Po kroku 5 bude baseline prázdny.
- [ ] ADR 0008 „Retire gate governance“. Ruší §11, §12 a §16 z `EXECUTION_CONTRACT.md`
      (preregistrácie, pevné poradie brán, commit iba pri „validovaných míľnikoch“). Nahrádza ich
      tabuľkou z kap. 3.

### Krok 1 — Agent prestane zlyhávať naslepo (2–4 dni)

Toto je Fáza 0 z analýzy:
- všetky `map_err(|_| "…")` v `live_bridge.rs` preč, chyba ide celá (`code`, `path`, `message`, `hint`),
- kolízia vracia zoznam kolízií a `apply_and_verify` zapíše aj pri kolízii (voliteľné `strict`),
- pečiatky a `plan_guard` preč zo skills, headless aj mosta,
- kolíkový spoj zadaný ako `(doska_a, doska_b, kolík, počet, okraj/rozostup)`; styk a rámy si
  dopočíta sám.

**Hotové, keď:** úloha z `fast_assembly/manifest.json` prejde z čistého kontextu. Pri zlyhaní
agent z odpovede vie, čo opraviť.

### Krok 2 — Vyhodiť showcase (1–2 dni, čisté mazanie)

- legacy `model_intent` celý: boxy, fľaše, čajníky, `balloon_texts`, `gable_roofs`, `staircases`,
  `oriented_beams`
  - v `ketchup-app/src/lib.rs` (vrátane ručného fontu `assistant_balloon_glyph_paths`),
  - v `ketchup-core/src/assistant_sidecar.rs` (`AssistantTeapotIntent` atď.),
  - v `sdk/python/ketchup_assistant_protocol.py` (systémový prompt aj validátor);
- `examples/assistant-*.ketchup` (3 súbory) a ich zmienka v README;
- moduly `beam_m4ae.rs`, `beam_m5.rs`, `release_capstone.rs`, `reference_examples.rs`,
  `linear_hardware.rs` (Hettich konštanta), feature `named-product-fixtures`;
- binárky brán (`ketchup-gate-c-nav`, `ketchup-gate-c-core`, `ketchup-a0-*`) a feature `a0-certification`;
- testy, ktoré testujú iba zmazané veci (`manual_bottle_documentation.rs`, `beam_m4ae.rs`,
  `m5_product.rs`, `release_capstone_contract*.rs`, časti `assistant_m7.rs`, …).

**Hotové, keď:** `cargo test --workspace` je zelený, aplikácia sa spustí, otvorí sa
`hettich-quadro-v6-drawer.ketchup` aj `grooved-beam-array.ketchup` a počet riadkov klesne.

### Krok 3 — Dôkazová diéta, 1. časť (2–3 dni)

- odstrániť z CI governance kroky, `governance/`, `thresholds/`, `corpora/r0`, preregistračné
  skripty, `test-architecture-guards.ps1` a pravidlá v CODEOWNERS pre tieto cesty. Nahradiť ich
  tromi jednoduchými kontrolami: `cargo test`, `clippy`, `check_no_named_products.py`;
- `artifacts/` (50 MB) a staré R0 reporty preč z pracovného stromu. História zostáva v gite;
- `KETCHUP_ARCHITECTURE_SPECIFICATION_V4c.md` (344 KB) a `docs/gates/` presunúť do `docs/archive/`
  a README už na ne neodkazuje ako na autoritu;
- odstrániť SHA-256 kontrolu binárky workera, ed25519 podpisy súhlasu a proposal digesty.
  Návrh sa aplikuje raz na klon kvôli náhľadu a raz naostro.

### Krok 4 — Pravidlový jazyk, MVP (2–3 týždne)

Nový crate `ketchup-program`:

1. **Starlark evaluátor** (`starlark-rust`), deterministický, bez I/O, s limitom krokov.
2. **Primitíva v Ruste** (tenké, mapujú sa na existujúce featury):
   `param`, `sketch`/`rect`/`circle`/`polyline`/`arc`, `extrude`, `revolve`, `cut`, `union`,
   `hole`, `place` (transform), `array`, `part` (diel = definícia + výskyt), `joint`
   (všeobecný: diely + objem + spojovacie prvky).
3. **Výber plôch podľa konštrukcie, nie podľa digestu.** Extrúzia pozná `start`, `end` a
   `side[i]` podľa hrany profilu. Doska z knižnice ich pomenuje `top/bottom/left/right/front/back`.
4. **Knižnica v Starlarku** (`crates/ketchup-program/library/*.star`): `board`, `dowels`, `groove`,
   `shelf`, … Každá funkcia má komentár s príkladom. **Tu, a iba tu, smú byť doménové mená.**
5. **Vyhodnotenie → `CommandBatch`** cez existujúci `apply_batch`, jeden undo krok. Stabilná
   identita dielu = cesta v programe (`skrinka/bok_lavy`, `skrinka/kolik[bok_lavy,dno]/2`),
   párovanie pri prepočte podľa cesty.
6. **Program sa ukladá v dokumente** ako text + verzia evaluátora.
7. **Ručné úpravy odvodených dielov:** zapíšu sa ako override s cestou do programu. Nestratia sa.
8. **Analytická geometria pre prizmatické diely** (kváder mínus konvexné výrezy): objem,
   hranice, styky a kolízie bez OCCT. OCCT len na zaoblenia, lofty, sweepy a STEP.

**Golden testy**, ktoré tvoria jediné akceptačné kritérium:
- skrinka (program → kusovník, vŕtací plán, 0 issues; `sirka += 100` → prepočet sedí),
- nočný stolík z `fast_assembly` prepísaný ako program,
- nosník A z drevodomu (`415 × 6`, `408 × 5`, `400`).

### Krok 5 — Fľaša a špeciálne evaluátory von (1–2 týždne)

Najprv **test zhody**: všetky existujúce fixture telesá sa vypočítajú iba cez `ExactBRepGraph`
a porovnajú so súčasným výsledkom (objem, hranice, počet plôch). Medzery sa opravia v grafe.
Potom zmazať:

- 16 špecializovaných evaluátorov v `exact_product.rs:40-61` (Rectangle, Circle, ArcProfile,
  LinearProfile, ThroughCut, CircularCut, Pocket, BooleanUnion/Intersect/Split, BoxShell,
  BoxFinish, …) a kaskádu v `ExactProducerPlan::plan`;
- ~6 700 riadkov ručne písaných sietí (`render_*_mesh`, `*_clipped_*_overlap`, `capsule`, `d_profile`);
- fľašu: `FeatureKind::BottleProfileControl`, `BottleEdgeFinish`, `CanonicalCommand::SetBottle*`,
  `exact_revolve.rs`, fľašové funkcie v `native.cc` („six profile points“), roly
  `Revolve{Shoulder,Neck,Mouth}` a `ShellInner*/Outer*`;
- `ExactFaceRole` s 50 menami → všeobecné adresovanie plôch z kroku 4.3;
- baseline v `check_no_named_products.py` = prázdny.

Staré súbory s fľašou sa prestanú otvárať. README stabilitu formátu nesľubuje, takže je to prijateľné.

### Krok 6 — Jedno rozhranie pre AI (1–2 týždne)

- 4 nástroje `model_read`, `model_write`, `model_view`, `model_export` nad programom;
  headless a živé okno majú ten istý protokol;
- vstavaný asistent používa tie isté nástroje;
- zmazať `AssistantCadEditOperation` (41 operácií), `WorkflowIntent` (49), duplicitný validátor
  v Pythone, staré skills a `ketchup_sdk` plugin protokol, pokiaľ ho nič nepoužíva.

### Krok 7 — Zlúčiť duplicity (1–2 týždne)

- `Extrusion` + `Pad` → `Extrude`; `Pocket` + `SketchPocket` + `ThroughCut` → `Cut { extent }`;
  `Shell` + `TopologyShell` → `Shell`; `Profile` + `SegmentProfile` + `SplineProfile` → `Sketch`;
- sedem pojmov spoja → `Joint` (všeobecný) + `AssemblyJoint` (kinematika);
- doménové validátory (priehyb police 500 N, prevrátenie 15°, ukotvenie, miestnosť, priechod)
  z jadra do knižnice jazyka s explicitnými parametrami. Kolízia, kontakt a podpora zostávajú v jadre.

### Krok 8 — Formát súboru a digesty (1–2 týždne)

- `.ketchup` = čitateľný JSON: program, parametre, override-y, ručne modelované diely.
  Odvodené výsledky idú do cache s kľúčom `(digest programu, verzia evaluátora, build)`, ktorú
  je možné zahodiť;
- `stable_digest.rs` → digest zo serde serializácie;
- `persistence.rs` sa zmenší z 9,7k na zlomok.

### Krok 9 — Priebežne

- rozdeliť `ketchup-app/src/lib.rs` (43k), `document.rs` (20k), `apply_batch` (2,3k riadkov jednej
  funkcie) a `state_view::encode_semantic_state_with_results` (2,8k). Nové veci idú do nových modulov;
- predčasné funkcie (FEA, pohybové štúdie, ozubenie, PDM) zmraziť. Ak sa do pol roka nepoužijú, zmazať.

---

## 5. Definícia „hotového“ kroku

PR je hotový, keď:

1. `cargo test --workspace`, `cargo clippy -- -D warnings` a `check_no_named_products.py` sú zelené;
2. prejde krátky ručný smoke test:
   - otvoriť príklady,
   - nakresliť obdĺžnik a Push/Pull,
   - uložiť a znovu otvoriť,
   - od kroku 4 aj spustiť program skrinky a exportovať kusovník;
3. popis PR obsahuje **čistý rozdiel riadkov**. Okrem kroku 4 má byť záporný;
4. popis PR je normálna ľudská reč: čo sa zmenilo a prečo. Žiadne hashe, nonce ani počty
   testov v odseku.

---

## 6. Čo tým získate

- Nová schopnosť sa pridá ako funkcia v knižnici (desiatky riadkov Starlarku), nie ako
  10 `match` vetiev v Ruste.
- AI mení pravidlá, nie súradnice. „Posuň kolíky“ a „rozšír skrinku“ sú zmeny jedného čísla.
- Kód sa zmenší zhruba na 150 tisíc riadkov a dá sa v ňom zorientovať.
- Pravidlo „nič na mieru“ nestráži disciplína, ale skript v CI.
