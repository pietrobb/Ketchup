# Code review Kečup — 2026-10-01 (druhé kolo)

Stav stromu: `769cacb` (main, čistý). Nadväzuje na `docs/code-review-2026-09-29.md`; všetkých 11 bodov (CR1–CR11) z neho je uzavretých, tento review hľadá, čo ostalo alebo pribudlo.

Súbor sa dopĺňa priebežne — každá sekcia má nálezy s cestou a riadkom, dôvod a návrh. Závažnosť: **A** = brzdí ďalší vývoj alebo porušuje AGENTS.md, **B** = dlh, ktorý treba splatiť pred ďalším rastom modulu, **C** = kozmetika / rýchla výhra.

## 0. Metóda

- Statická analýza nad `git ls-files` (455 zdrojových súborov, 428 135 riadkov Rust + C++ + Python + Starlark).
- Skripty: dĺžky súborov a funkcií, numerické literály mimo `const`, `unwrap/expect/panic` v produkčnom kóde, `#[allow]`, `TODO/FIXME`, doménové slová, duplicitné bloky.
- Čítanie kódu v miestach, ktoré metriky označili.
- Nič sa nespúšťalo okrem existujúcich kontrol (`scripts/check_*.py`), takže funkčné nálezy sú z čítania, nie z reprodukcie — pri každom je to uvedené.

## 1. Metriky veľkosti

| Crate | spolu | src | testy |
|---|---:|---:|---:|
| ketchup-app | 138 681 | 89 331 | 49 350 |
| ketchup-model | 126 029 | 79 046 | 46 983 |
| ketchup-application | 38 814 | 23 226 | 15 588 |
| ketchup-scheduler | 22 866 | 11 884 | 10 982 |
| python (sdk, scripts, tests) | 19 746 | 19 746 | 0 |
| ketchup-assistant | 16 922 | 6 860 | 10 062 |
| ketchup-exact (Rust + C++) | 16 107 | 13 838 | 2 269 |
| ketchup-manufacturing | 10 920 | 6 957 | 3 963 |
| ketchup-program | 10 165 | 7 401 | 2 764 |
| ketchup-interaction | 8 271 | 5 635 | 2 636 |
| ketchup-geometry | 6 957 | 6 957 | **0** |
| ketchup-headless | 5 800 | 4 233 | 1 567 |
| ketchup-analysis | 2 865 | 2 040 | 825 |
| ketchup-pdm | 1 938 | 1 179 | 759 |
| ketchup-rejection | 339 | 339 | 0 |
| ketchup-tolerance | 169 | 169 | 0 |

Najväčšie súbory (produkčný kód):

| riadkov | súbor |
|---:|---|
| **36 480** | `crates/ketchup-app/src/lib.rs` |
| 5 000 | `crates/ketchup-model/src/drawing.rs` |
| 4 750 | `crates/ketchup-manufacturing/src/fabrication.rs` |
| 4 743 | `crates/ketchup-model/src/persistence/legacy.rs` |
| 4 650 | `crates/ketchup-model/src/shared_change.rs` |
| 4 513 | `crates/ketchup-assistant/src/sidecar.rs` |
| 4 398 | `crates/ketchup-model/src/import/dxf.rs` |
| 4 320 | `crates/ketchup-model/src/exact_product.rs` |
| 4 291 | `crates/ketchup-application/src/planner.rs` |
| 4 030 | `crates/ketchup-application/src/validation.rs` |
| 3 888 | `crates/ketchup-scheduler/src/exact_worker.rs` |
| 3 871 | `crates/ketchup-model/src/document/store.rs` |

Najväčšie testové súbory: `ketchup-app/src/tests.rs` 17 975, `ketchup-app/tests/headless_shell.rs` 14 532, `ketchup-assistant/tests/intent_review_and_commit.rs` 7 438, `ketchup-scheduler/tests/exact_brep_graph.rs` 7 190.

Funkcie: 6 055 v produkčnom Rust kóde, **95 má cez 200 riadkov, 324 cez 100**. Najdlhšie:

| riadkov | funkcia |
|---:|---|
| 2 365 | `ketchup-application/src/planner.rs:1756` `plan_assistant_cad_edit_program_with_outputs` |
| 2 141 | `ketchup-model/src/document/store.rs:798` `apply_batch_with_origin_and_validation` |
| 1 587 | `ketchup-app/src/lib.rs:26228` `viewport` |
| 1 424 | `ketchup-model/src/persistence/legacy.rs:2928` `read_product` |
| 1 132 | `ketchup-model/src/document/product_validation.rs:27` `validate_product_with_drawing_sources` |
| 1 078 | `ketchup-program/src/eval.rs:1035` `builtins` |
| 1 011 | `ketchup-assistant/src/intent.rs:474` `propose_intent` |
| 969 | `ketchup-model/src/document/proposal_analysis.rs:763` `authoritative_dependencies` |
| 935 | `ketchup-assistant/src/sidecar.rs:2940` `validate` |
| 853 | `ketchup-application/src/collision.rs:446` `collision_report` |
| 823 | `ketchup-app/src/lib.rs:30726` `show_assistant` |
| 815 | `ketchup-application/src/append_feature.rs:24` `plan_feature_kind` |

Čo sa od minulého reviewu zlepšilo: `native.cc` (6 599 r.) je rozdelený na 10 súborov po 436–878 r.; `ketchup-core` neexistuje, je rozdelený do model/geometry/manufacturing/assistant/analysis/pdm; `persistence/legacy.rs` je zamrznutá hranica.

Čo sa nezlepšilo: `ketchup-app/src/lib.rs` **narástol** (36 480 r.) — CR7 zmenšil počet polí stavu, ale nerozdelil súbor. Pozri §2.

## 2. `ketchup-app/src/lib.rs` — jeden `impl` s 29 470 riadkami (závažnosť A)

### 2.1 Nález

- Súbor má 36 480 riadkov, 977 funkcií, 148 štruktúr, 29 enumov, 46 `mod` deklarácií.
- `impl KetchupApp` začína na r. 5029 a končí na r. 34 499 — **jeden impl blok, 860 metód**. Prefixy metód ukazujú najmenej desať nezávislých zodpovedností: `show_*` (39), `apply_*` (37), `assistant_*` (31), `occurrence_*` (26), `confirm_*` (24), `tag_*` (24), `select_*`/`selected_*` (42), `preview_*` (18), `export_*` (11), `rotate_*`/`move_*` (22), `paint_*` (10), `exact_*` (11).
- Najdlhšie metódy: `viewport` 1 587 r. (r. 26 228), `show_assistant` 823 r. (r. 30 726), `prepare_assistant_from_inputs` 654 r. (r. 29 900), `assistant_proposal_value_label` 508 r. (r. 8 406).
- CR7 zmenšil `KetchupApp` na 43 polí, ale všetky metódy ostali v jednom súbore. 46 modulov vedľa je správny smer, ale `lib.rs` je stále 7× väčší ako druhý najväčší produkčný súbor repozitára.
- `egui::` sa v súbore vyskytuje 426×, `ketchup_model::` 84× — UI a doménová logika sú prepletené v tých istých metódach (`viewport` obsahuje hit-testing, gestá, kreslenie aj commit príkazov).

### 2.2 Prečo je to problém

- rust-analyzer na 36k-riadkovom súbore je pomalý; každá zmena rekompiluje celý crate (`ketchup-app` je zároveň najväčší test crate: 49 350 r. testov).
- Nedá sa reviewovať po častiach; každý PR do UI mení ten istý súbor → konflikty medzi paralelnými úlohami (napr. #2258 chat okno vs. ďalšie UI práce).
- Metódy sa nedajú testovať bez celej `KetchupApp` — preto má `tests.rs` 17 975 riadkov a `headless_shell.rs` 14 532.

### 2.3 Návrh

Rozdeliť podľa už existujúcich prefixov na `impl KetchupApp` bloky v samostatných súboroch (bez zmeny API):
`app/viewport.rs` (viewport, paint_*, hit-test), `app/selection.rs` (select_*, selected_*, occurrence_*, tag_*), `app/assistant.rs` (assistant_*, prepare_assistant_*, show_assistant), `app/transform.rs` (move_*, rotate_*, push_*), `app/commands.rs` (apply_*, commit_*, confirm_*), `app/export.rs`, `app/exact.rs`.
Pravidlo do `check_crate_layers.py`: ratchet na počet `.rs` v `crates/*/src` nad 5 000 riadkov (dnes 1) a počet funkcií nad 400 riadkov (dnes 20, zoznam v §1). Počet sa nesmie zvýšiť.

## 3. Lineárna algebra napísaná 24× (závažnosť A — porušuje „jeden zdroj pravdy“)

### 3.1 Nález

Vlastné `fn dot`, `fn cross`, `fn normalize`, `fn sub`, `fn transform_point`, inverzia afinnej matice — každý crate (a často každý súbor) má svoju kópiu:

| operácia | počet súborov s vlastnou implementáciou |
|---|---:|
| `dot` | 24 |
| `cross` | 20 |
| `subtract`/`sub` | 16 |
| `transform_point` | 9 |
| `distance` | 9 |
| `scale` | 8 |
| `add` | 7 |
| `length` | 7 |
| inverzia 3×3 / 3×4 / 4×4 matice (`let determinant = m[0] * (m[5] * m[10] …`) | 12 |

Inverzia afinnej transformácie konkrétne: `ketchup-application/src/transforms.rs:16`, `ketchup-model/src/assembly_joint.rs:2307`, `ketchup-model/src/shared_change.rs:545`, `ketchup-model/src/exact_product.rs:1229` a `:1316` (dvakrát v jednom súbore), `ketchup-model/src/import/glb.rs:1186`, `ketchup-model/src/import/sketchup_scene.rs:449`, `ketchup-manufacturing/src/fabrication.rs:3818`, `three_mf_export.rs:587`, `blender_export.rs:574`, `ketchup-app/src/lib.rs:35677`, `ketchup-model/src/document/entities.rs:63` (`Transform::rigid_inverse`). Dvanásť implementácií toho istého vzorca, každá so svojím prahom singularity.

Kubická Bézierova krivka (vyhodnotenie + derivácia) je napísaná dvakrát v `exact_product.rs` (r. 2572 a r. 3644) a dvakrát v `exact_brep_graph/bounds.rs` (r. 1191 a r. 1286).

Rámec (frame) je stále plochý `[f64; 12]` indexovaný `frame[3 + axis]`, `frame[6 + axis]`, `frame[9 + axis]` v `exact_brep_graph/bounds.rs:237-971`, `exact_worker.rs:2031-2934`, `exact_validation.rs:1630`. CR9 typizoval segmenty, rámec nie.

### 3.2 Prečo je to problém

- 12 inverzií = 12 rôznych prahov singularity a 12 miest, kde sa `Option`/`Result` správa inak (niektoré vracajú identitu, niektoré `None`).
- Nedá sa jedným testom overiť správnosť; chyba v jednej kópii (napr. transponovaná rotácia) sa prejaví len v jednom exporte.
- `[f64; 12]` rámec bez typu znamená, že `frame[9]` sa dá omylom zameniť s `frame[6]` bez kompilačnej chyby.

### 3.3 Návrh

- `ketchup-geometry::linalg`: `Vec3` (`[f64;3]` newtype s `dot/cross/norm/sub/add/scale`), `Affine3` (`[[f64;3];4]` s `invert() -> Result<_, Singular>` cez `TolerancePolicy`), `Frame { origin, x, y, z }` s `From<[f64;12]>`, `CubicBezier::eval/derivative`.
- Ratchet v `check_crate_layers.py`: zákaz nových `fn dot(`/`fn cross(`/`let determinant =` mimo `ketchup-geometry`; dnešný počet (24/20/12) je strop a musí klesať.

## 4. Limity a bulharské konštanty (závažnosť B)

### 4.1 Čo je dobre

842 pomenovaných `const`, z toho 320 limitov `MAX_*`/`MIN_*`. Tolerancie sú po CR4 v `ketchup-tolerance` (`TolerancePolicy`) a ratchet `check_tolerance_literals.py` drží počet literálov tolerancií na nule.

### 4.2 Nález — ten istý limit má 4–7 mien

| hodnota | mená |
|---|---|
| 64 segmentov cesty | `MAX_SWEEP_PATH_SEGMENTS` (exact), `MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS` (model), `MAX_PATH_SEGMENTS` (program), `MAX_SLOT_PATH_SEGMENTS` (geometry), `MAX_ASSISTANT_SPATIAL_PATH_SEGMENTS` (assistant), `MAX_PLANAR_LOOP_SEGMENTS` + `MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS` |
| 0,01 mm minimálna dĺžka | `MIN_LENGTH_MM` (exact), `EXACT_MIN_LENGTH_MM` (model), `MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM`, `MIN_EXTENT_MM` (interaction), `MIN_RECTANGLE_SIZE_MM` (interaction) |
| 100 000 mm maximum | `MAX_LENGTH_MM`, `MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM`, `MAX_EXACT_PLANAR_OFFSET_LENGTH_MM`, `MAX_SHEET_METAL_LENGTH_MM` — **ale** `ketchup-tolerance::MAX_COORDINATE_MM = 1 000 000`. Dve rôzne „maximálne súradnice“ v jednom modeli. |
| 100 000 vrcholov / 200 000 trojuholníkov | `MAX_MESH_VERTICES/TRIANGLES` (feature_validation), `MAX_STL_VERTICES/TRIANGLES` (import), `MAX_VERTICES_PER_PRIMITIVE/…` (glb), `MAX_VERTICES_PER_DEFINITION/…` (sketchup_scene), `MAX_EXACT_BREP_GRAPH_MESH_TRIANGLES` (scheduler) |
| 256 krokov cesty inštancie | `MAX_COLLISION_SCENE_PATH_STEPS`, `MAX_INSTANCE_PATH_STEPS`, `MAX_AXIS_INSTANCE_PATH_STEPS`, `MAX_ASSISTANT_VALIDATION_PATH_STEPS`, `MAX_ASSISTANT_INSTANCE_PATH_STEPS`, `MAX_RELEASE_LINEAGE_DEPTH` |
| 8 000 inštancií exportu | `MAX_STL_EXPORT_INSTANCES`, `MAX_GLB_EXPORT_INSTANCES`, `MAX_THREE_MF_EXPORT_INSTANCES`, `MAX_DRAWING_INSTANCES` |
| 4 MiB text | `MAX_*_TEXT_BYTES` v collision, model_query, planner, validation (×4 v jednom crate) |
| 100 výskytov/issues validácie | `MAX_ASSISTANT_VALIDATION_OCCURRENCES` a `MAX_ASSISTANT_VALIDATION_ISSUES` sú definované **dvakrát pod rovnakým menom**: `ketchup-app/src/lib.rs:267` a `ketchup-application/src/validation.rs:16` |
| 32 MiB súbor | `MAX_FILE_BYTES` v `persistence.rs:30` a `persistence/legacy.rs:483` |
| 16 batch jobov | `MAX_BATCH_JOBS` v `live_bridge.rs` aj `headless/protocol.rs` |

### 4.3 Nález — literály bez mena v produkčnom kóde

Skript našiel 2 492 numerických literálov mimo `const`/testov/assertov; po odpočítaní DXF group kódov a indexov matíc ostáva rádovo 400 relevantných. Najhoršie miesta:

- `ketchup-assistant/src/sidecar.rs` — limity inline vo `validate` (935 r.): `(2..=16).contains(&sections.len())` r. 1382/1414, `(2..=256)` r. 1460, `<= 64` r. 1496/1518/1555, `<= 32` r. 1527, `> 128` r. 3113/3461/3508/3584, `(DEFAULT_LINEAR_TOLERANCE_MM..=10.0)` r. 1467, `> 0.1 && < 89.9` r. 1577-1578, `< 0.01` uhol r. 2379/3829/4025, `64 * 1024`/`128 * 1024` r. 4350-4353, `len() == 64` (hex SHA) 6×. 28 `MAX_ASSISTANT_*` konštánt na vrchu súboru existuje, ale polovica limitov v tej istej funkcii ich nepoužíva.
- `sidecar.rs:624-634` — tvar profilu závitu: V = `radius * 0.72`, trapéz = `0.65`, `0.42`, `0.72`. Tri geometrické faktory bez odvodenia (ISO 60° profil má pomer `tan 30° ≈ 0.577`, nie 0,72). Pozri §5.
- `ketchup-manufacturing/src/fabrication.rs:624-836` — SVG rozloženie výkresu: `45`, `35`, `25.0`, `400.0`, `55.0`, `90.0`, `80.0`, `30.0`, `20.0` ako inline rozostupy (14 výskytov). Jedna `struct SheetLayout { margin, row_gap, … }` by to vyriešila.
- `fabrication.rs:1469` `if intermediate_saw_cuts > 32`, `:1601` `PI / 18.0` (10° krok), `:2494` `(5.0..=175.0)` uhol.
- `exact_product.rs:2195` `PI / 16.0`, `:2596` a `:4030` `(2..=64).contains(&segments.len())` (64 je `MAX_PLANAR_LOOP_SEGMENTS`, nepoužité), `:3142` `depth >= 16`, `:4068` `radius < 0.01` (je to `EXACT_MIN_LENGTH_MM`, nepoužité).
- `exact_validation.rs:792` a `:839` `16 * 1024 * 1024` dvakrát inline; `:1585` `[1, 2, 4, 6, 8, 9, 12, 13, 14]` — indexy matice bez komentára.
- `exact_worker.rs:722/752/761` `value.abs() <= 1.0e6` (je to `MAX_COORDINATE_MM`, ale literál), `:734` `fixtures.len() > 64`.
- `fea.rs:85-88` `maximum_nodes: 256`, `0.05`, `0.02` inline v `Default`; `:342` `> 256` znovu literál; `:363` `1.0e9`.
- `ketchup-app/src/theme.rs` — 171 literálov (farby, rozmery). Téma je legitímna, ale nie je v dátovom súbore — zmena témy = rekompilácia.
- `ketchup-model/src/import/dxf.rs` — 661 literálov, prevažne DXF group kódy (10/20/30/70/72/210…). Legitímne ako protokol, ale `const GROUP_X: u16 = 10` by 4 400-riadkový parser zčitateľnilo.

### 4.4 Podozrivé hodnoty

- `ketchup-model/src/import.rs:13` `MAX_IMPORT_SOURCE_BYTES = 1024⁴` (1 TiB) — fakticky žiadny limit; každý formát má vlastný `MAX_*_SOURCE_BYTES` (8–32 MiB), tento je mŕtvy strop.
- `ketchup-model/src/sheet_metal.rs:6` `MAX_SHEET_METAL_FLANGES = 4` — nie bezpečnostný limit, ale dôsledok modelu „obdĺžnik so 4 hranami“ (§5).
- `sidecar.rs:23-24` `MAX_ASSISTANT_PROFILE_TRANSLATIONS = 1`, `MAX_ASSISTANT_PARAMETER_EDITS = 1` — limit 1 je zakódovaná neschopnosť dávky, nie ochrana.
- `persistence.rs:30` `MAX_FILE_BYTES = 32 MiB` vs `:33` `MAX_NATIVE_DOCUMENT_BYTES = 64 MiB` — dve rôzne maximá pre ten istý dokument v jednom súbore.

### 4.5 Návrh

- `ketchup-tolerance` dostane `pub mod limits` s jedným menom pre každú veličinu: `PATH_SEGMENTS`, `MIN_LENGTH_MM`, `MAX_LENGTH_MM` (= `MAX_COORDINATE_MM`, jedna hodnota), `MESH_VERTICES`, `MESH_TRIANGLES`, `INSTANCE_PATH_STEPS`, `EXPORT_INSTANCES`, `TEXT_BYTES`. Ostatné crates ich re-exportujú alebo odvodzujú (`MAX_DXF_PROFILES = HOST_MAX…` je dobrý vzor).
- Rozšíriť `check_tolerance_literals.py` o limity: `.contains(&x.len())`, `len() > N`, `len() <= N` s literálom N ≥ 8 v `crates/*/src` je nález; ratchet od dnešných 79.
- `sidecar.rs::validate` rozbiť na `impl Validate for <každý request typ>` — potom má každý limit prirodzené miesto.

## 5. Neuniverzálne operácie — kde AGENTS.md §1 ešte neplatí (závažnosť A)

### 5.1 Programový jazyk: `box()` je stále výnimka, nie špeciálny prípad `extrude()`

- `ketchup-program/src/model.rs:452` `ProgramPartBody::Panel` je samostatný variant vedľa `Extrusion/Revolve/Sweep/Loft`. Na `Panel` sa vetví 21 miest: `model.rs` 6×, `faces.rs` 3×, `validate.rs` 3×, `eval.rs` 2× (jedno z nich `unreachable!()` r. 778), `face_at.rs`, `cad.rs`, `rule_operations.rs` 2×, `rule_program.rs` 3×.
- `validate.rs:200-206` „solid je presne jeho kváder: panel“ — booleany majú inú cestu pre `Panel` ako pre `Extrusion` s obdĺžnikovým profilom (`validate.rs:121,147`), hoci geometria je tá istá.
- `eval.rs:1078` `box()` prijíma `material`, `grain`, `color`; `extrude()` (r. 1135), `revolve`, `sweep`, `loft` **nie**. Používateľ, ktorý nakreslí dosku ako profil, nemôže zadať materiál. To je presne „vlastnosť dostupná len pre pomenovaný tvar“.
- `Part.grain_axis` (`model.rs:513`) je drevársky pojem v jadre jazyka — ratchet ho eviduje (9+2+1 výskytov), ale správne miesto je `prelude.star` (`board()` už tam je a `grain` môže byť voľný atribút `attributes: BTreeMap<String, Value>`).
- `Part` má `operations: Vec<ProgramOperation>` (v poradí zápisu) **a zároveň** `holes: Vec<Hole>` a `pockets: Vec<Pocket>` mimo poradia (`model.rs:519-523`). Diera je operácia ako každá iná; dve paralelné mechaniky znamenajú, že `hole` po `cut` sa nevyhodnotí v poradí zápisu.

Návrh: `Panel` zrušiť — `box()` vytvorí `Extrusion { segments: rectangle, distance }` a nastaví `size_mm` rovnako. `material/color/attributes` presunúť do `Part` ako voliteľné polia pre každé telo. `Hole`/`Pocket` zaradiť do `ProgramOperation`. Zvyšný `Panel`-špecifický kód (`faces::PANEL_FACES`) je už po CR6 odvodený z tela, takže je nadbytočný.

### 5.2 Plech: obdĺžnik so štyrmi pomenovanými hranami

`ketchup-model/src/sheet_metal.rs:17` `SheetMetalEdge { MinX, MaxX, MinY, MaxY }`, `SheetMetalSpec { width, depth, thickness, flanges: Vec<_> }`, `MAX_SHEET_METAL_FLANGES = 4`. Základ je vždy obdĺžnik, ohyb len na jednej zo štyroch hrán, žiadny ohyb na ohybe (druhá úroveň), žiadny lem, žiadny výrez po ohybe. Enum `SheetMetalEdge` je skopírovaný ešte dvakrát: `exact_brep_graph.rs:372` `ExactBRepSheetMetalEdge` a `sidecar.rs:1105` `AssistantCadSheetMetalEdge`, s konverziami medzi nimi.

Návrh: základ = ľubovoľný planárny profil (ten istý `PlanarRegion` ako pre `Pad`), ohyb = `Bend { edge: EdgeReference, angle, radius, length }` na ktorejkoľvek hrane ktorejkoľvek steny (po CR6/CR9 máme topologické referencie hrán). Flat pattern je potom rozvinutie stromu ohybov, nie zoznam 4 hrán.

### 5.3 Závit: tri pomenované profily s magickými faktormi

`sidecar.rs:609-635` `AssistantThreadProfile::{Round, V, Trapezoid}` s tvarmi `radius * 0.72`, `0.65`, `0.42`. Používateľ nemôže zadať vlastný profil závitu (ISO 60°, lichobežník 30°, pílový, oblý DIN 405 majú každý iné pomery). Navyše `CreateHelixPath`, `CreateHelix` a `CreateThread` sú tri samostatné operácie Asistenta pre jeden koncept „profil vedený po skrutkovici“.

Návrh: jedna operácia `sweep` s `path = helix(...)` a ľubovoľným `profile` (segmenty ako všade inde); `Round/V/Trapezoid` sú tri funkcie v `prelude.star`, ktoré vrátia profil.

### 5.4 Rozpoznanie meshu: kváder, valec, výtlačok

`ketchup-model/src/mesh_recognition.rs` a `ketchup-application/src/mesh_conversion.rs` rozpoznajú len `Box | Cylinder | LinearExtrusion`. Je to v poriadku ako prvé tri triedy, ale štruktúra je `enum` s tromi vetvami a `unreachable!("cylinder candidates return before generic extrusion planning")` (`mesh_conversion.rs:678`) — pridanie štvrtého tvaru (revolve, loft) znamená ďalšiu vetvu, nie ďalší „rozpoznávač“ v zozname.

Návrh: `trait Recognizer { fn recognize(&Mesh) -> Option<Candidate> }` a zoznam rozpoznávačov; valec je výtlačok kruhu, kváder je výtlačok obdĺžnika, takže tri dnešné triedy sú jedna (planárny profil + výtlačok) s rôznymi profilmi.

### 5.5 Asistent: 29 operácií, z toho 9 sú dvojice „dokument / program“

`AssistantCadEditOperation` (`sidecar.rs`): `CreateSketch` + `CreateProgramSketch`, `CreatePart` + `CreatePanel`, `CreatePinJoint` + `CreateProgramPinJoint` + `CreatePhysicalPinJoint`, `AppendFeature` + `AppendProgramPocket`, `CreateHelixPath` + `CreateHelix` + `CreateThread`. Každá dvojica je tá istá vec s iným cieľom (priamy dokument vs. program vlastniaci dokument). AI musí vedieť, ktorý z dvoch použiť, a `planner.rs::plan_assistant_cad_edit_program_with_outputs` (2 365 r.) rieši oba.

Návrh: jedna operácia s poľom `target: Document | Program`, alebo lepšie — ak dokument vlastní program, **každá** operácia sa prepíše do programu (`rule_program.rs::rewrite_rule_program_push_pull` je vzor, ktorý už existuje pre Push/Pull). Potom `*Program*` varianty zaniknú.

### 5.6 Doménové slová, ktoré ratchet nevidí

`scripts/check_no_named_products.py` sleduje 24 slov. Nesleduje: `panel` (38× ketchup-application, 23× ketchup-program, 15× ketchup-assistant, 29× ketchup-app), `board` (15× application), `beam` (13× application), `timber` (13× manufacturing: `GeneralMachiningGeometry::TimberStock`), `weldment` (12× app, 7× application, 7× manufacturing), `cup`/`cup-bore` (validation.rs), `apron`, `rail`, `leg`. `RoleFunction::{Panel, Beam}` v `part_role.rs:22-24` sú zabudované roly. Odporúčam pridať `panel, board, beam, timber, lumber, weldment, cup_bore, apron` do `WORDS` a zafixovať dnešný baseline (ratchet nedovolí rast).

## 6. Chyby, panics a slepé miesta ratchetov (závažnosť B)

### 6.1 Čo je dobre

- `check_error_hygiene.py` drží `Err("…".to_owned())` a `map_err(|_| …)` na nule; `ketchup-rejection` dáva typovaný dôvod odmietnutia pre AI aj GUI.
- `clippy::all = deny` a `unsafe_code = forbid` v celom workspace; len 43 `#[allow]` (30× `too_many_arguments`).
- Žiadne `TODO/FIXME/HACK` v zdrojákoch.

### 6.2 Ratchet meria užší vzor, než CR10 sľuboval

`check_error_hygiene.py` počíta len dva regexy. Mimo nich ostáva:

- **58 funkcií s `Result<_, String>`** v produkčnom kóde: `ketchup-app/src/lib.rs` 14, `assembly_ui.rs` 13, `scheduler/assistant.rs` 4, `scheduler/plugin.rs` 3, `program/path.rs` 3, `validation_rules.rs` 3, `drawn_shape.rs` 3, `evaluation.rs` 2, `program/model.rs` 2, ďalších 9 súborov po 1.
- **69× `map_err(|error| error.to_string())`** — typovaná chyba sa splošti na text pri prechode do UI vrstvy (`lib.rs` 23, `assembly_ui.rs` 14, `feature_history_ui.rs` 6, `planar_push_pull.rs` 5, `assistant_runtime.rs` 4, `face_workflow_ui.rs` 4). Používateľ vidí správny text, ale GUI nemôže na druh chyby reagovať (napr. ponúknuť „otvoriť len na čítanie“ pri `ReviewOnly`).
- `ketchup-application/src/session.rs:61` `SessionError::Persistence(String)` — jediný variant `SessionError`, ktorý nenesie typ; `session.rs:274` `.map_err(|error| SessionError::Persistence(error.to_string()))` zahadzuje `persistence::Error` so všetkými jeho poľami (cesta, offset, verzia).

Návrh: rozšíriť `check_error_hygiene.py` o tretí vzor `Result<[^,]+, String>` v signatúrach a štvrtý `map_err\(\|\w+\| \w+\.to_string\(\)\)`; baseline = dnešných 58 + 69, len klesať. `SessionError::Persistence(persistence::Error)`.

### 6.3 Panic body v produkčnom kóde — 369

`expect` 260, `unreachable!` 66, `unwrap` 40, `panic!` 3. Väčšina `expect` má správu opisujúcu invariant („CreatePart always creates a definition“ — `planner.rs:553-582` 4×, „writing to a String cannot fail“ — `exact_product.rs:1245-1365` 5×). Dva vzory sú ale varovné:

- **Dvojfázový „collect then match“** s `unreachable!("collected feature is a workplane")` — `document/planar_face.rs:88,91,134,232`, `:167` („…is a pad“). Prvá fáza vyberie ID, druhá znova matchuje enum a padne, ak sa medzitým zmenil. Správny tvar je vybrať **referenciu na variant** (`&WorkplaneSpec`) v prvej fáze.
- `unreachable!()` bez správy: `exact_product.rs:3266,3321,3379,3388`, `feature_history_ui.rs:695,722,753,1495`, `proposal_analysis.rs:323,433`, `headless/protocol.rs:1167`, `sketch/solver.rs:137`, `region.rs:245`, `geometry.rs:206`. Pri páde v produkcii používateľ dostane „internal error: entered unreachable code“ bez kontextu.
- `scheduler/child_process.rs:41,44` `panic!("cancellation must not wait for Windows kernel teardown")` — správne ako invariant, ale v procese, ktorý beží v GUI vlákne? Overiť, že je to vždy worker-side.

Návrh: ratchet `check_error_hygiene.py` na `unreachable!()` bez správy (dnes 14) a `.unwrap()` v `crates/*/src` (dnes 40); nič nové, existujúce klesajú.

### 6.4 Pretypovania `as` — 669

`digest_v3.rs` 90 (legitímne — zamrznutý formát), `exact_worker.rs` 58, `import.rs` 41, `renderer.rs` 31, `fabrication.rs` 26, `iges.rs` 22. `clippy::pedantic::cast_possible_truncation` nie je zapnutý (2 miesta ho explicitne `allow`-ujú, teda niekto ho skúšal). `u32 as usize` je neškodné; `f64 as u32`/`usize as u32` v importoch (počet trojuholníkov, indexy) sú miesta, kde zlý súbor môže ticho pretiecť. Návrh: zapnúť `cast_possible_truncation`, `cast_sign_loss`, `cast_precision_loss` ako `warn` a vyčistiť import/export cesty (≈150 miest), zvyšok `allow` s dôvodom.

### 6.5 Dve pravdy o Assistant protokole

`sdk/python/ketchup_assistant_protocol.py` (3 219 r.) nesie `MAX_CAD_EDIT_OPERATIONS = 64`, `MAX_CAD_SELECTOR_TARGETS = 100`, `MAX_CAD_GENERATED_OCCURRENCES = 512`, `MAX_LINE_BYTES`, `PROTOCOL_VERSION = 3` — tie isté hodnoty, ktoré má `sidecar.rs:18-43` a `scheduler/assistant.rs:24-27`. Žiadny test neoveruje, že sedia. `catalog.rs::cad_operation_catalog` už vie vyexportovať JSON schému operácií; Python ju nečíta.

Návrh: Rust je zdroj; `cargo test` v `ketchup-assistant` zapíše `sdk/python/ketchup_assistant_protocol_limits.json`, Python ho načíta, a pytest porovná konštanty. Rovnaký vzor ako golden reporty z CR11.

### 6.6 Dodatok k 6.2 — „typované“ varianty s `format!` vnútri

81× `Variant(format!(…))` v produkčnom kóde, z toho 63 v `ketchup-model/src/shared_change.rs`: `OccurrenceForkPropagationError::Dependency` 14, `ComponentReplacementImpactError::Incompatible` 14, `SharedChangePropagationError::Dependency` 12, `ComponentReplacementImpactError::Unsupported` 10, `ComponentReplacementCommitError::InvalidImpact` 6, `OccurrenceForkImpactError::Unsupported` 5. Ďalej `WorkerError::Protocol/Transport` 5, `PluginHostError::MalformedProtocol` 3, `PersistenceError::InvalidPayload` 2. Enum je typovaný, ale obsah je text — GUI ani AI z neho nevyčíta, **ktorá** závislosť (ID, meno, druh) bránila zmene. `shared_change.rs` je 4 650 r. s 823-riadkovou `project_component_replacement_impact_for_principal`; návrh: `Dependency { blocker: DependencyKind, source: FeatureId, target: OccurrenceId }` namiesto `Dependency(String)`; ratchet na `\(format!\(` v `Err(`/variantoch.

## 7. Testy (závažnosť B/C)

### 7.1 Čísla

2 218 `#[test]` v `tests/` + 239 inline = 2 457 testov; 8 `#[ignore]`; **157× `thread::sleep` v testoch** (polling na UI/worker stav — zdroj nestabilít; `file_workflow.rs:71` `wait_for_visible_label` spí 300×10 ms).

Testy sú 43 % riadkov repozitára. Pomer je zdravý, ale rozdelený nerovnomerne: `ketchup-geometry` (6 957 r. src, sketch solver 2 030 r.) má **0 súborov v `tests/` a 2 inline moduly**; solver je testovaný len nepriamo cez `ketchup-model/tests/workplane_sketch.rs`. `ketchup-tolerance` a `ketchup-rejection` nemajú testy (malé, ale sú to základy).

### 7.2 Nálezy

- 22 testových funkcií nad 300 riadkov; `assistant_workflows.rs::scripted_create_part_program_round_trips_state_view_and_one_step_undo_redo` (r. 1878) a `product_document.rs::persistent_associative_dimensions_…` (r. 1655, ~2 700 r.) sú scenáre „všetko v jednom“ — pri páde jedna assert, žiadna informácia, čo ešte funguje.
- `ketchup-app/src/tests.rs` 17 975 r. a `tests/headless_shell.rs` 14 532 r. — dva najväčšie súbory po `lib.rs`. Rovnaký liek: rozdeliť podľa toho, čo testujú (CR11 premenovalo súbory, ale nerozdelilo).
- 13 rôznych `env::var` v testoch (`KETCHUP_LIVE_PYTHON`, `PYTHON`, `KETCHUP_PYTHON`, `KETCHUP_TEST_EXACT_WORKER`, `KETCHUP_UPDATE_GOLDEN`, `UPDATE_STATE_VIEW_FIXTURES`, …) — tri rôzne mená pre „kde je Python“. Jeden `tests/common/env.rs` s jedným menom na vec.
- Živé testy s OAuth (`timber_frame_house.rs::live_oauth_assistant_builds_a_roofed_house_frame_across_turns`, 710 r.) — pomenovaný produkt v názve testu je v poriadku (fixture), ale test závisí od siete a modelu; mal by byť v samostatnom `--features live` alebo `#[ignore]` s jasným dôvodom, inak je CI nedeterministické.

### 7.3 Návrh

- `ketchup-geometry/tests/sketch_solver.rs`: priame testy riešiča (konvergencia, over-constrained, DOF) — dnes je jediná cesta cez dokument.
- Ratchet na `thread::sleep` v testoch (157 → klesá) — nahradiť `settle()`/`wait_until(predicate, deadline)` v harness-e, ktorý už existuje (`shell.settle()`).

## 8. Funkčnosť a výkon — nálezy z čítania (závažnosť B)

Nič z tohto som nereprodukoval spustením; každý bod je z kódu a treba ho potvrdiť testom.

### 8.1 Undo je 10 krokov pre dokument vlastnený programom, neobmedzené pre ručný

`ketchup-model/src/document/store.rs:153-159` `limit_rule_program_history` pri každom `bind_rule_program` (r. 172) a `replace_rule_program_source` (r. 214) **zahodí celé revízie dokumentu** nad `RULE_PROGRAM_UNDO_LIMIT = 10` (`revision.rs:89`). `self.revisions` je história celého dokumentu, nie len programu. Ručne editovaný dokument má históriu neobmedzenú (`revisions` len `truncate` pri novej vetve, r. 211/730/2933; uloží sa až 4 096 — `persistence.rs:27`). Dva rôzne režimy undo bez toho, aby to používateľ videl; po desiatom `apply` programu zmizne možnosť vrátiť sa k ručným krokom pred ním. `ketchup-program/src/document.rs:9` má druhý `UNDO_LIMIT = 10` pre vlastnú históriu zdroja.

Návrh: jeden limit v `ketchup-model` (napr. 200 revízií, pamäť je `BTreeMap<Id, Arc<T>>` so zdieľaním, takže revízia stojí O(počet ID) ukazovateľov), rovnaký pre oba režimy; program-undo nech nemá vlastný limit.

### 8.2 Každá zmena validuje celý model

`store.rs:2852-2858` po každom `apply_batch` beží `refresh_sketch_projections`, `validate_graph`, `validate_overrides`, `validate_product_with_drawing_sources` (1 132 r., prechádza všetky definície, výskyty, features, väzby, výkresy), `validate_sketch_projections`, `validate_assembly_joint_motion_publication`. Je to O(veľkosť modelu) na jedno kliknutie. Limity hovoria o 10 000 výskytoch (`MAX_SCOPED_COLLISION_OCCURRENCES`, `MAX_INSTANCE_INDEX_ITEMS`, `MAX_STRUCTURAL_SCOPE_OCCURRENCES`), ale výkonnostný test (`ketchup-app/tests/production_performance.rs`) meria korpus s **590 výskytmi** a rozpočty len na otvorenie, exact rebuild, 20 UI snímkov a 2 GiB pamäte — žiadny rozpočet na latenciu jednej editácie.

Návrh: (1) test „1 presun v modeli s 10 000 výskytmi < 50 ms“ ako ratchet; (2) validácia nad `dirty_features`/dotknutými ID (štruktúra `dirty_features` v `Revision` už existuje — r. 114), nie nad celým `ProductModel`.

### 8.3 GUI kreslí len čiaru, obdĺžnik, kružnicu a oblúk

`ActiveTool` (`lib.rs:1708`): `Line, Rectangle, Circle, Arc` + 3D nástroje. Model (`entities.rs`, 22 výskytov `Cubic/Bezier/Spline`), program (`polygon()`, `ellipse()`, `round_corners()`, `mirrored()`) aj Asistent (`Mirror`, `ProfileSegment::Spline`) vedia polygón, elipsu, spline a zrkadlenie — ale **ručne v okne sa nedajú nakresliť**. `Mirror` sa v `lib.rs` vyskytuje raz (len ako Assistant operácia), `Polygon`/`Ellipse` nikdy. Používateľ bez AI alebo programu má menej ako SketchUp Free.

Návrh: `Polygon`, `Ellipse`, `Spline` (kliknutím body, Esc koniec), `Mirror` (vybrať rovinu = existujúca stena), `Offset2D`. Všetky štyri majú model aj Asistent hotové; chýba len nástroj v `ActiveTool` a `KEYMAP`.

### 8.4 `Helix` a `Thread` ako dva nástroje v GUI

`AppCommand::Helix` a `AppCommand::Thread` (+ `helix_thread_ui.rs`) sú dve cesty k „profil po skrutkovici“. Po zovšeobecnení z §5.3 je to jeden `Sweep` s cestou `helix`.

### 8.5 Ostatné

- `assistant_runtime`/`lib.rs:3700-3706` — `AssistantRequestSnapshot::build` aktívne čaká (`sleep(1 ms)` v slučke) na `preparation_delay`, ktorý je nenulový len v testoch (`headless_set_assistant_context_preparation_delay`). Testovací hák v produkčnej ceste; patrí za `#[cfg(feature = "testing")]` alebo do harnessu.
- `ketchup-app/src/live_bridge/consent.rs` — 10× `let _ = handler.join()` a `sleep(5–10 ms)` slučky (r. 428, 735, 777); vlákna sa ukončujú pollingom namiesto `Condvar`/kanála. Funguje, ale je to zdroj 157 `sleep` v testoch (§7.1).
- `mesh_conversion.rs:249-388` — vlákno + watchdog vlákno + `let _ = worker.join()` 6×: ručná orchestrácia životného cyklu, ktorú `ketchup-scheduler` už rieši pre exact worker. Konverzia meshu by mala ísť cez ten istý scheduler.
- `collision.rs:1105` `tx.send(Err(format!("exact_worker_unavailable: {error}")))` — kanál s `Result<_, String>`; typ `ExactWorkerUnavailable { cause }` existuje v scheduleri.

## 9. Návrhy a smer — čo by malo byť ďalej

Zoradené podľa páky (koľko ďalších vecí odblokuje), nie podľa ľahkosti.

1. **Rozdeliť `ketchup-app/src/lib.rs`** (§2). Odblokuje paralelnú prácu na GUI (#2258 chat okno, nástroje z §8.3) bez konfliktov a skráti kompiláciu testov. Mechanické, bez zmeny správania, dá sa overiť existujúcimi 2 457 testami.
2. **`ketchup-geometry::linalg`** (§3). Jedna `Vec3`, `Affine3`, `Frame`, `CubicBezier`; 24+20+12 kópií ide na nulu; ratchet. Odblokuje typovaný rámec v exact worker-i a zníži riziko transponovaných matíc v exportoch.
3. **Univerzálny program** (§5.1): `Panel` → `Extrusion`, `material/color` pre každé telo, `holes/pockets` → `operations`. Odblokuje jednotnú cestu program ↔ dokument (§5.5) a zrušenie 9 duplicitných Assistant operácií.
4. **Plech ako profil + strom ohybov** (§5.2) a **závit ako sweep po helixe** (§5.3). Dve posledné „pomenované tvary“ v jadre.
5. **Jeden limit = jedno meno** (§4.5) + rozšírené ratchety (§4.5, §6.2, §6.3, §7.3). Lacné, ale bez toho sa duplicity vrátia.
6. **Undo a validácia** (§8.1, §8.2). Jeden undo limit; inkrementálna validácia s testom latencie na 10k výskytov.
7. **GUI nástroje** (§8.3): polygón, elipsa, spline, zrkadlenie. Používateľská hodnota za málo kódu, keď je (1) hotové.
8. **Python protokol z Rust schémy** (§6.5). Odstráni druhú pravdu o limitoch Asistenta.

## 10. Čo je dobré (aby review nebol len zoznam dlhov)

- Všetkých 11 bodov z 29. 9. je skutočne uzavretých a väčšina z nich s ratchetom v CI — regresie sú zablokované, nie len opravené.
- `ketchup-tolerance` + `ketchup-rejection` ako najnižšie vrstvy sú správny tvar: malé, bez závislostí, jeden zdroj pravdy pre tolerancie a odmietnutia.
- `prelude.star` je presne to miesto, kam patria `board`, `dowels`, `hinge`, `groove` — doménová knižnica ako dáta, jadro bez nej.
- Typované odmietnutia s dôvodom a kauzou (CR10) sú viditeľne lepšie pre AI aj GUI; `check_error_hygiene.py` ich drží.
- Golden reporty vzorových programov (CR11) sú lacný a presný regresný test evaluátora.
- Žiadny `TODO`, žiadny `unsafe` mimo FFI, `clippy::all = deny` všade, 43 % riadkov sú testy.

## 11. Metriky na sledovanie (návrh ratchetov)

| metrika | dnes | smer |
|---|---:|---|
| `.rs` v `crates/*/src` nad 5 000 r. | 1 | 0 |
| funkcie nad 400 r. (produkčný kód) | 20 | ↓ |
| vlastné `fn dot/cross/determinant` mimo `ketchup-geometry` | 24 / 20 / 12 | 0 |
| `Result<_, String>` v signatúrach | 58 | 0 |
| `map_err(\|e\| e.to_string())` | 69 | 0 |
| `Variant(format!(…))` | 81 | ↓ |
| `unreachable!()` bez správy | 14 | 0 |
| `.unwrap()` v produkčnom kóde | 40 | ↓ |
| limity s literálom (`len() > N`, `.contains(&len)`) | 79 | ↓ |
| `thread::sleep` v testoch | 157 | ↓ |
| `ProgramPartBody::Panel` match ramien | 21 | 0 |
| doménové slová mimo ratchetu (`panel/board/beam/timber/weldment/lumber/apron`) | 281 | pridať do `WORDS`, len klesať |

## 12. Záver — prioritné poradie

Review z 29. 9. je uzavretý a nič z neho sa nevrátilo. Dnešné nálezy sú inej povahy: nie „slabé miesta v návrhu“, ale **tri veľké dlhy, ktoré brzdia ďalšiu prácu**, a rad menších, ktoré ratchety ešte nevidia.

**Závažnosť A (blokuje ďalší vývoj):**

| # | nález | § | prečo A |
|---|---|---|---|
| A1 | `ketchup-app/src/lib.rs` — jeden `impl` s 29 470 r. | 2 | každá GUI zmena (#2258, nové nástroje) ide cez jeden súbor; konflikty, pomalá kompilácia testov |
| A2 | lineárna algebra napísaná 24× (`dot`), 20× (`cross`), 12× (`determinant`) | 3 | riziko transponovaných matíc v exportoch; nemožno zaviesť typovaný `Frame` |
| A3 | `ProgramPartBody::Panel` ako výnimka (21 match ramien), plech, závit, 9 dvojíc Assistant operácií | 5 | porušuje AGENTS.md §1 — každý nový tvar = nová vetva |

**Závažnosť B (dlh, ktorý rastie, kým ho ratchet nedrží):**

| # | nález | § |
|---|---|---|
| B1 | rovnaký limit má 4–7 mien; 79 limitov s literálom | 4 |
| B2 | 58 `Result<_, String>`, 69 `to_string()` splošťovaní, 81 `Variant(format!)`, 14 `unreachable!()` bez správy | 6 |
| B3 | undo 10 krokov pri programe vs. neobmedzené ručne; validácia celého modelu pri každej zmene | 8.1, 8.2 |
| B4 | GUI nekreslí polygón, elipsu, spline, zrkadlenie — model aj Asistent to vedia | 8.3 |
| B5 | Python protokol nesie druhú kópiu limitov Asistenta | 6.5 |

**Závažnosť C (hygiena):** 157 `sleep` v testoch, 22 testov nad 300 r., `tests.rs` 17 975 r., `ketchup-geometry` bez `tests/`, 13 `env::var` mien, testovací hák v produkčnej ceste (§7, §8.5).

**Odporúčané poradie krokov** (§9, zoradené podľa páky):

1. A1 rozdeliť `lib.rs` — mechanické, 2 457 testov ako sieť, nič iné na to nečaká, ale všetko GUI na to čaká.
2. A2 `ketchup-geometry::linalg` + ratchet na vlastné `dot/cross/determinant`.
3. A3 univerzálny program (`Panel` → `Extrusion`, `operations`), potom plech a závit ako profil + cesta, potom zrušenie `*Program*` dvojíc Asistenta.
4. B1 + B2 jeden limit = jedno meno, rozšírené ratchety (§11) — lacné, treba urobiť hneď po 1–3, inak sa duplicity vrátia.
5. B3 jeden undo limit; inkrementálna validácia s testom latencie na 10 000 výskytov.
6. B4 GUI nástroje — používateľská hodnota za málo kódu, keď je 1 hotové.
7. B5 Python protokol z Rust schémy.
8. C hygiena testov priebežne, ratchet na `sleep`.

Pravidlá rovnaké ako pri CR-2026-09-29: univerzálne riešenia namiesto špeciálnych prípadov, žiadna nová pomenovaná konštanta bez jedného zdroja pravdy, každý krok = zelené existujúce testy + vlastné regresné testy + ratchet, bez automatického commitu/pushu.
