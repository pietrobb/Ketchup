# Code review Kečupu — slabé miesta, magické konštanty, špeciálne prípady

Dátum: 2026-09-29. Stav stromu: `783662b` (main).
Súbor je zapisovaný priebežne; každá sekcia je uzavretá vo chvíli, keď je zapísaná.

## 0. Rozsah a metóda

- Rust workspace: `ketchup-core`, `ketchup-exact`, `ketchup-scheduler`, `ketchup-application`,
  `ketchup-interaction`, `ketchup-program`, `ketchup-headless`, `ketchup-app` (~10 MB zdrojov).
- Metóda: (1) metriky veľkosti a štruktúry, (2) grep vzorov (číselné literály, tolerancie,
  pomenované produkty, `map_err(|_|`, `unwrap`, `todo`), (3) čítanie kľúčových modulov,
  (4) návrhy smerom k všeobecnému, profesionálnemu riešeniu.
- Hodnotenie závažnosti: **A** = architektonické / blokuje všeobecnosť, **B** = lokálne, ale
  opakujúce sa, **C** = kozmetika / hygiena.

## 1. Metriky veľkosti (kontext pre všetko ďalšie)

| Súbor | Veľkosť | Poznámka |
|---|---|---|
| `crates/ketchup-app/src/lib.rs` | 1 521 KB | AGENTS.md §6 zakazuje rast, ale je to stále monolit |
| `crates/ketchup-core/src/document.rs` | 769 KB | dtto |
| `crates/ketchup-app/src/tests.rs` | 684 KB | jeden testovací súbor |
| `crates/ketchup-app/tests/headless_shell.rs` | 598 KB | jeden testovací súbor |
| `crates/ketchup-core/src/persistence.rs` | 375 KB | ručný binárny formát so schémami 1..N |
| `crates/ketchup-core/src/exact_product.rs` | 247 KB | |
| `crates/ketchup-core/src/exact_brep_graph.rs` | 234 KB | |
| `crates/ketchup-exact/src/lib.rs` | 232 KB | celý OCCT façade v jednom súbore |
| `crates/ketchup-scheduler/src/lib.rs` + `exact_worker.rs` | 190 + 184 KB | |
| `crates/ketchup-application/src/planner.rs` | 191 KB | |
| `crates/ketchup-core/src/sketch.rs` | 206 KB | |

Nálezy sa dopĺňajú nižšie.

## 2. Tolerancie a limity — „bulharské konštanty“ (závažnosť A)

### 2.1 Nález

Grep `const *(TOL|EPS|MAX|MIN)*: f64` v `src/` nachádza **~95 definícií v 35 súboroch**. Tá istá
fyzikálna veličina má v každom module inú hodnotu a iné meno:

| Veličina | Hodnoty v strome | Kde |
|---|---|---|
| Geometrická epsilon (mm) | `1e-12`, `1e-10`, `1e-9`, `1e-8`, `1e-7`, `1e-6` | `drawing.rs:21-22`, `mechanical_contract.rs:15-17`, `joinery.rs:6`, `sketch.rs:12`, `exact_brep_graph.rs:1016,3165`, `dxf_export.rs:16`, `cam.rs:20`, `prismatic.rs:6`, `program/path.rs:16`, `program/frame.rs:11` |
| Max. absolútna súradnica | `1_000_000.0` ×15, ale `1e12` v `drawing.rs:23` a `prismatic.rs:5` | `document.rs:78`, `sketch.rs:10`, `cam.rs:14`, `dxf_export.rs:15`, `import/*.rs`, `assembly*.rs`, `mechanical_*.rs`, `program/eval.rs:36`, `app/lib.rs:242` |
| Min. dĺžka hrany | `0.01` (`exact/lib.rs:9`, `exact_product.rs:34`, `push_pull_gesture.rs:15`, `rectangle_face_authoring.rs:17`), ale `1e-7` (`exact_brep_graph.rs:57`), `1e-4` (`sheet_metal.rs:7`) | |
| Max. dĺžka | `100_000.0` v `exact/lib.rs:10`, `exact_product.rs:35`, `exact_brep_graph.rs:48`, `sheet_metal.rs:8` | |
| Ray epsilon | `1e-12` definovaná 3× samostatne | `interaction/lib.rs:26`, `exact_projection.rs:22`, `spatial.rs:13`, `mesh_projection.rs:15` |
| Mesh area/volume eps | `1e-18` / `1e-12` definované 2× identicky | `document.rs:12308-12309`, `import.rs:1040-1041` |

Ďalšie lokálne "magické" hodnoty bez odvodenia: `ELLIPSE_CUBIC_MAX_NORMALIZED_RADIAL_DEVIATION = 0.000_273`
(`assistant_sidecar.rs:38`, nie je uvedené, že ide o známu chybu 4-segmentovej Bézier aproximácie
kružnice 0,0273 %), `MAX_ASSISTANT_HELIX_TURNS = 16.0` (`assistant_sidecar.rs:32` – prečo 16?),
`GAP_LIMIT_MM = 20.0` (`program/relations.rs:14`), `MESH_CONVERSION_TOLERANCE_MM = 0.25`
(`app/lib.rs:194`), `PRECISE_SNAP_WORLD_TOLERANCE_MM = 0.5`, `ROTATION_MIN_ARM_MM = 0.5`,
`MAX_CAMERA_ZOOM = 8.0` (`app/lib.rs:195-220`).

### 2.2 Prečo je to problém

- Rovnaký model prejde validáciou v `sketch.rs` (eps 1e-7) a zlyhá v `joinery.rs` (eps 1e-8) alebo
  `drawing.rs` (1e-12). Používateľ dostane "faces do not mate" bez toho, aby vedel, že problém je
  rozdiel tolerancií medzi modulmi, nie jeho model.
- `MAX_ABS_MM` sa nedá zmeniť na jednom mieste (napr. pre architektúru v km alebo jemnú mechaniku
  v µm). V profesionálnom CAD je "model tolerance" / "linear precision" nastavením dokumentu.
- Tolerancie v mm sú absolútne, nie relatívne k veľkosti modelu; 1e-9 mm pri modeli 100 m je pod
  rozlíšením f64 (ULP pri 1e5 je ~1.5e-11, po pár operáciách je 1e-9 šum).

### 2.3 Návrh

1. Zaviesť jeden modul `ketchup-core::tolerance` s typom `Tolerance { linear_mm, angular_rad, model_extent_mm }`
   a odvodenými funkciami (`is_coincident(a, b)`, `is_parallel(u, v)`, `is_zero_length(l)`),
   uloženým v `DocumentStore` (persistovaný, editovateľný v nastaveniach dokumentu).
   OCCT to robí cez `Precision::Confusion()`; Kečup by mal odovzdať tú istú hodnotu do
   `ketchup-exact` (dnes `exact/lib.rs:9-14` má vlastnú sadu).
2. Konštanty typu `MAX_*` zlúčiť do `DocumentLimits` (jedna definícia, ostatné moduly ju importujú).
   Rozdiel `1e6` vs `1e12` (drawing/prismatic) je pravdepodobne chyba, nie zámer — overiť.
3. Každá zostávajúca lokálna konštanta musí mať komentár s odvodením (napr. "4× ULP pri max.
   súradnici", "0.0273 % = známa chyba Bézier aproximácie štvrťkružnice").
4. Doménové limity (`GAP_LIMIT_MM`, `MESH_CONVERSION_TOLERANCE_MM`) presunúť do parametrov
   operácie s defaultom, nie do `const`.

## 3. Doménové (nábytkárske) koncepty v Rust jadre (závažnosť A — priamo porušuje AGENTS.md §1)

### 3.1 Nález

AGENTS.md §1 hovorí: „board, dowel, groove, shelf, cabinet, drawer, … live only in the program
language library“. Skutočnosť v `src/`:

| Miesto | Čo | Rozsah |
|---|---|---|
| `ketchup-core/src/joinery.rs` | `DowelSpec`, `DowelJointContract`, `DOWEL_JOINERY_PROJECTION_V1`, 12 chybových variantov o kolíkoch | 37 KB, celý modul |
| `ketchup-core/src/document.rs:9905-9910` | `DowelJoint` chybové varianty v jadre dokumentu | |
| `ketchup-core/src/document/stable_digest.rs:50,477-481` | `canonical-dowel-joints.v1` v digest-e dokumentu → formát súboru je naviazaný na kolíky | |
| `ketchup-core/src/assistant_sidecar.rs:2029-2056, 3381-3506` | `AssistantStandardDowel`, `create_physical_dowel_joint` ako AI tool | |
| `ketchup-core/src/cad_catalog.rs:198` | `create_physical_dowel_joint` v katalógu operácií | |
| `ketchup-application/src/validation.rs:43-101` | `shelf_deflection`, `anchoring`, `hardware_manufacturing`, `room_placement`, `passage_clearance` validátory + 17 nábytkárskych konštánt (`MINIMUM_HINGE_CUP_DIAMETER_MM = 35`, `MINIMUM_PASSAGE_WIDTH_MM = 900`, `SHELF_ELASTIC_MODULUS_N_MM2 = 2500`) | 167 KB súbor |
| `ketchup-application/src/validation.rs:535-551, 1987-1989` | Stringové role `"furniture.shelf.xy"`, `"furniture.case.z"`, `"manufacturing.hinge-cup.x"` matchované `match role { "…" => }` | |
| `ketchup-application/src/planner.rs:742-979, 2733-3164` | plánovanie `dowel` operácií, hlášky "Move the dowel row clear of existing joinery or hardware." | |
| `ketchup-core/src/exact_validation.rs:1866-1868` | komentáre "A shelf hanging on the side panel" — kód napísaný pre jeden scenár | |

### 3.2 Prečo je to problém

- **Kolík (dowel) je špeciálny prípad generického „cylindrical pin joint + two coaxial holes“.**
  Ten istý mechanizmus potrebuje čap, skrutka, konfirmát, rúrka v otvore, kolík v ráme. Dnes je
  celý reťazec dowel-specific: typ → validácia → digest → persistencia → AI tool → planner →
  chybové hlášky.
- `SHELF_ELASTIC_MODULUS_N_MM2 = 2500` je materiálová konštanta (DTD/MDF) zakódovaná do
  validátora. Profesionálne riešenie: materiál je vlastnosť dielu (`Material { e_modulus, density, … }`),
  validátor je generický „beam deflection under load“ a berie E z dielu.
- `MINIMUM_PASSAGE_WIDTH_MM = 900` je norma (stavebný predpis), líši sa podľa krajiny. Patrí do
  konfigurovateľnej sady pravidiel (program library / JSON), nie do Rust `const`.
- Stringové role `"furniture.shelf.xy"` sú kódovanie troch informácií (doména, funkcia, orientácia)
  do jedného reťazca. Role by mala byť štruktúra `{ load_bearing: bool, thickness_axis: Axis }`
  a "shelf" je len jej pomenovanie v knižnici.

### 3.3 Návrh

1. `joinery.rs` → premenovať a zovšeobecniť na `pin_joint.rs`: `PinJoint { diameter, length,
   insertion: [f64; 2], clearance }` + `PinRow { direction, count, spacing }`. "Dowel 8x30" je
   potom funkcia v `crates/ketchup-program/library/`.
2. Validátory v `validation.rs` rozdeliť: generické fyzikálne (`collision`, `gravity_support`,
   `static_load`, `tipping`, `beam_deflection`) ostávajú v Ruste ale bez doménových konštánt;
   normové (`passage_clearance`, `anchoring`, `hardware_manufacturing`) odísť do dátovo
   riadených pravidiel (`rules/*.star` alebo JSON), ktoré Rust len vyhodnocuje.
3. Role: nahradiť `&str` typom `PartRole { function: RoleFunction, frame: SourceFrameHint }`
   so serde-serializáciou; string ostáva len na hranici (JSON pre AI).
4. Vytvoriť `scripts/check_no_named_products.py` (AGENTS.md §8 ho spomína ako "once it exists")
   s baseline zo súčasného stavu — inak ratchet nefunguje.

## 4. `FeatureKind` — päť extrúzií, tri profily, dva shelly (závažnosť A)

### 4.1 Nález (`ketchup-core/src/document.rs:741-865`)

Enum `FeatureKind` má 37 variantov. Viaceré sú tá istá operácia s inou kombináciou parametrov:

| Skupina | Varianty | Generický ekvivalent |
|---|---|---|
| Profil | `Profile { points_mm }`, `SegmentProfile { segments, closed }`, `SplineProfile { control_points_mm }` | jeden `Profile { segments: Vec<Segment> }` kde `Segment ∈ {Line, Arc, Bezier, Spline}` — `Profile` je špeciálny prípad `SegmentProfile` so samými `Line` |
| Extrúzia | `Extrusion { profile, height }`, `Pad(PadSpec)`, `SketchPocket(PocketSpec)`, `Pocket { target, profile, depth }`, `ThroughCut { target, profile }` | jeden `Extrude { profile, extent: Blind(d) \| ThroughAll \| ToFace(ref) \| Symmetric(d), operation: NewBody \| Join(target) \| Cut(target) \| Intersect(target) }` — presne ako Fusion/Onshape/SolidWorks |
| Shell | `Shell { removed_faces: Vec<StableFaceRole> }`, `TopologyShell { removed_faces: Vec<TopologicalElementRef>, profile_faces: Vec<ProfileFaceReference>, … }` | jeden `Shell` s jedným referenčným typom (`FaceRef` = enum nad všetkými spôsobmi odkazu). `Shell` je legacy paralelná cesta — AGENTS.md §6 |
| Odkaz na plochu | `TopologyFaceOffset { face: Option<TopologicalElementRef>, profile_face: Option<ProfileFaceReference> }` | dve `Option` polia = neplatné stavy (obe `None`, obe `Some`) reprezentovateľné v type; má byť `face: FaceRef` |
| Hrany | `TopologyEdgeFinish { kind: EdgeFinishKind, amount, fillet_radius_stations, chamfer_mode, chamfer_edge_sides }` | jeden variant nesie zjednotenie parametrov fillet-u a chamfer-u; polovica polí je vždy ignorovaná. Má byť `EdgeFinish { edges, spec: Fillet{…} \| Chamfer{…} }` |
| Doména | `WeldmentMember`, `WeldmentJoint`, `SheetMetal` | "beam" je v AGENTS.md §1 zakázaný pojem; weldment je `Sweep` profilu po ceste + `Boolean`. Sheet-metal je legitímna trieda operácií (bend/unfold) |

Dôsledok: každý z týchto variantov je matchovaný v **~15 funkciách** (`stable_digest.rs::feature_kind`
768 riadkov, `persistence.rs::write_features` 613 r. + `read_product` 1316 r.,
`document.rs::validate_feature_kind` 453 r., `authoritative_dependencies` 971 r.,
`apply_batch_with_origin_and_validation` 2193 r., `state_view.rs::encode_semantic_state_with_results`
2737 r., `append_feature.rs::plan_feature_kind` 791 r., `exact_brep_graph.rs::compile_node` 615 r.,
`exact_worker.rs::evaluate_exact_brep_graph` 547 r., `feature_history_ui.rs`, `planner.rs`…).
Grep `FeatureKind::(Extrusion|Pad|SketchPocket|Pocket|ThroughCut)` → 81 výskytov len v `document.rs`.
Pridanie jedného varianta = zásah do 10+ súborov (shotgun surgery); zlúčenie piatich extrúzií do
jednej by odstránilo odhadom 3–5 tisíc riadkov.

### 4.2 Návrh

1. Zaviesť trait-based dispatch: `trait Feature { fn dependencies(&self) -> …; fn validate(&self, ctx) -> …;
   fn digest(&self, d: &mut Digest); fn compile(&self, g: &mut BrepGraph) -> …; fn descriptors(&self) -> … }`
   a každý variant implementovať v **jednom** súbore `features/extrude.rs`, `features/shell.rs`…
   Persistencia cez `serde` + tagované varianty namiesto ručného `write_features/read_product`.
2. Migrácia: `Extrusion/Pad/SketchPocket/Pocket/ThroughCut → Extrude`, `Profile/SplineProfile →
   SegmentProfile`, `Shell → TopologyShell` (potom premenovať na `Shell`). Loader starých súborov
   konvertuje na nové varianty pri čítaní (jednorazovo), nie paralelné cesty.
3. `FaceRef`/`EdgeRef` ako enum `{ Stable(StableFaceRole), Topological(TopologicalElementRef),
   Named(ProfileFaceReference) }` — jeden typ, jeden resolver.

## 5. Monolitické funkcie a súbory (závažnosť A/B)

### 5.1 Nález

Skript s párovaním zátvoriek nachádza **159 funkcií nad 150 riadkov** v `src/` (AGENTS.md §6
požaduje ~150). Najhoršie:

| Riadky | Funkcia | Súbor |
|---|---|---|
| 2 737 | `encode_semantic_state_with_results` | `ketchup-core/src/state_view.rs:119` |
| 2 363 | `plan_assistant_cad_edit_program_with_outputs` | `ketchup-application/src/planner.rs:1737` |
| 2 193 | `apply_batch_with_origin_and_validation` | `ketchup-core/src/document.rs:5480` |
| 1 557 | `viewport` | `ketchup-app/src/lib.rs:27759` |
| 1 316 | `read_product` | `ketchup-core/src/persistence.rs:6634` |
| 1 306 | `validate_product_with_drawing_sources` | `ketchup-core/src/document.rs:16222` |
| 1 010 | `builtins` | `ketchup-program/src/eval.rs:1093` |
| 971 | `authoritative_dependencies` | `ketchup-core/src/document.rs:18451` |
| 959 | `propose_intent` | `ketchup-core/src/intent.rs:474` |
| 901 | `validate` | `ketchup-core/src/assistant_sidecar.rs:2874` |
| 841 | `collision_report` | `ketchup-application/src/collision.rs:396` |

`KetchupApp` (`app/lib.rs`) má **204 polí** a 1 027 funkcií v jednom súbore (38 747 riadkov).
Predchádzajúci audit (v pamäti: "Architectural isolation audit") už identifikoval, že Move/Rotate/Scale
stav sú paralelné `Option` + `bool` polia bez typu, ktorý by vynútil exkluzivitu gesta;
`clear_ephemeral_edit_state`, `cancel_transform_gesture`, `reset_document_presentation`, `undo/redo`
a Escape každý ručne nuluje inú podmnožinu polí.

Ďalšie: `assistant_subtracted_box_mesh` (`app/lib.rs:1570`) je `#[allow(dead_code)]` voxelový
generátor meshu pre „box mínus boxy“ — ručne písaný mesh pre konkrétnu kombináciu tvarov
(AGENTS.md §1 zakázané) a navyše mŕtvy kód. Zmazať.

### 5.2 Prečo je to problém

- Funkcie s 1–3 tisíc riadkami sa nedajú testovať po častiach; testy sú preto end-to-end
  (`headless_shell.rs` 598 KB, `tests.rs` 684 KB), pomalé a krehké.
- 204 polí v `KetchupApp` = stav aplikácie nie je modelovaný, iba akumulovaný. Každý nový nástroj
  pridá 3–5 polí a `if` do `viewport`.

### 5.3 Návrh

1. `KetchupApp` rozdeliť na `Document + Session` (model), `ToolState` (enum s exkluzívnym
   aktívnym gestom: `enum Gesture { None, Move(MoveState), Rotate(RotateState), PushPull(…) }`),
   `ViewportState`, `AssistantState`, `Panels`. Egui `App::update` len deleguje.
2. `viewport()` rozložiť na `handle_input → update_tool → render` pipeline; každý nástroj
   implementuje `trait Tool { fn on_pointer(…); fn overlay(…); fn cancel(…) }`.
3. `state_view.rs::encode_semantic_state_with_results` (2 737 r.) je jedna ručná serializácia;
   nahradiť `serde::Serialize` na typoch + jednu `to_value()`.
4. `persistence.rs` ručný binárny formát (schémy 1..N, `read_product` 1 316 r.): nahradiť
   `postcard`/`bincode` + `serde` s verzionovaním cez `#[serde(default)]`; checksum ostáva.

## 6. Persistencia — 97 verzií schémy v jednom čítači (závažnosť A)

### 6.1 Nález (`ketchup-core/src/persistence.rs:96-199`)

- 97 konštánt `*_SCHEMA: u16` (0..97) a **~97 porovnaní `schema >= X_SCHEMA`** v čítacej ceste.
  Každá nová vlastnosť pridala vetvu; žiadna stará sa neodstránila. Produkt nemá verziu 1.0,
  ale nesie spätnú kompatibilitu s 97 medzistavmi formátu.
- Názvy schém sú doménové (`DOWEL_JOINERY_SCHEMA = 90`, `DOWEL_PHYSICAL_HOLE_BINDING_SCHEMA = 92`,
  `DOWEL_PAIR_OFFSET_SCHEMA = 94`, `WELDMENT_*`, `SHEET_METAL_SCHEMA`) — formát súboru je
  naviazaný na nábytkárske koncepty.
- 150 `unwrap/expect/panic` v `persistence.rs`, 53 `map_err(|_| PersistenceError::Truncated)` —
  chyba čítania nepovie, ktorý záznam/offset zlyhal (AGENTS.md §2).
- Ručné `read_*`/`write_*` páry pre každý typ; `state_view.rs` má paralelnú ručnú textovú
  serializáciu (2 737 riadkov, 370 `unwrap` na `writeln!`).

### 6.2 Návrh

1. Jedna "freeze" migrácia: načítať všetky existujúce fixtures/examples starým čítačom, zapísať
   novým formátom, starý čítač zmazať. Pred 1.0 nie je dôvod držať 97 verzií.
2. Nový formát: `serde` + `postcard` (kompaktný, deterministický) alebo CBOR; verzia
   dokumentu jedno číslo + `#[serde(default)]` pre nové polia. Zachovať `KETCHUPDOC` magic +
   SHA-256 checksum (AGENTS.md §4).
3. `state_view.rs` textový encoder nahradiť `serde_json`/`ron` `Serialize` derive na tých istých
   typoch → −2 500 riadkov.

## 7. Chyby bez kódu, cesty a nápovedy (závažnosť B — AGENTS.md §2)

### 7.1 Nález

- `assistant_sidecar.rs`: **118** `Err("… is invalid".to_owned())` — plain `String`, bez `code`,
  `path`, `hint`. Príklady: `"assistant CAD part feature is invalid"` (r. 801 a 812 – dve rôzne
  príčiny, jedna hláška), `"assistant axis is invalid"` (r. 383, 394).
- `map_err(|_| …)` ~**430×** v `src/` (53 v `persistence.rs`, 41 v `import/dxf.rs`, 31 v
  `three_mf_export.rs`, 19 v `planner.rs`, 16 v `exact_brep_graph.rs`). Napr.
  `planner.rs:1248` zahodí `body_values()` chybu a nahradí ju generickým
  „exact topology evidence is unavailable“.
- `joinery.rs:191-206` — 12 chybových variantov ako `&'static str` bez lokalizácie v mm
  (napr. `FacesDoNotMate` nepovie, ktoré plochy ani akú medzeru).
- `program/eval.rs`: `anyhow::bail!` s dobrým textom (pozitívny príklad), ale bez kódu — AI
  klient nevie rozlíšiť triedy chýb strojovo.

### 7.2 Návrh

1. Jeden typ `Rejection { code: &'static str, path: String, message: String, hint: String,
   location_mm: Option<[f64;3]>, parts: Vec<PartRef>, source: Option<Box<dyn Error>> }` v
   `ketchup-core::error`, `From<…>` impls pre všetky doménové chyby.
2. Lint: `clippy::disallowed_methods` pre `map_err(|_|` → vynútiť `.map_err(|e| Rejection::from(e).at(path))`.
3. `assistant_sidecar` validácia: každý `bail` má vlastný kód (`assistant.axis.zero_length`, …).

## 8. Ratchet pomenovaných produktov má slepú škvrnu (závažnosť B)

`scripts/check_no_named_products.py:24-28` kontroluje 17 slov (`bottle`, `teapot`, `nightstand`,
`capsule`, `drawer`, `cabinet`, `wardrobe`…), ale **nie** `dowel`, `shelf`, `hinge`, `board`,
`groove`, `beam`, `weldment`, `furniture`, `passage`, `room` — čo sú práve najčastejšie doménové
slová v jadre (viď §3). Baseline má preto len 8 riadkov, hoci `joinery.rs` je celý o kolíkoch.
Návrh: rozšíriť `WORDS` o slová z AGENTS.md §1 a `--update` baseline; potom ratchet reálne tlačí.

## 9. Programový jazyk — dobrý základ, ale model dielu je „kváder s prílohami“ (závažnosť A)

### 9.1 Čo je dobre

- `ketchup-program/src/eval.rs` má generické builtiny (`box`, `extrude`, `revolve`, `sweep`,
  `loft`, `fillet`, `chamfer`, `cut`, `push_pull`, `shell`, `mirror`, `hole`, `pocket`, `joint`,
  `contact`, `expect`) a doménové helpery (`board`, `dowels`, `member`) žijú v
  `library/prelude.star`. Test `every_builtin_is_documented_in_the_library_comments` je pekný
  mechanizmus. Toto je správny smer podľa AGENTS.md §1.

### 9.2 Nález

| Miesto | Problém |
|---|---|
| `program/model.rs:11-26` `enum Face { XMin … ZMax }` | Plocha je **jedna zo šiestich stien kvádra**. `hole()`, `pocket()`, `contact()`, `push_pull(face=&str)` prijímajú len tieto. Na diele z `revolve`/`sweep`/`loft` nemá „z+“ zmysel; diera do valca alebo šikmej plochy nie je vyjadriteľná. |
| `model.rs:528` `ProgramPartBody::Panel` ako prvý variant, `Extrusion/Revolve/Sweep` dodatočne | Model vznikol pre panely; ostatné telesá sú "tiež-diely" bez plnohodnotných operácií. |
| `eval.rs:1003` `reject_mirrored` | Po `mirror()` nie je dovolená žiadna ďalšia operácia, lebo názvy plôch sa neprepočítajú. Obmedzenie vyplývajúce zo stringových názvov plôch, nie z geometrie. |
| `eval.rs:1136-1166` `box(grain=…)` | `grain` (smer vlákna dreva) je doménový parameter v Rust builtine. Má byť generický `metadata` dict alebo `anisotropy_axis`. |
| `eval.rs:1149`, `TOLERANCE_MM = 0.01` | Vlastná tolerancia jazyka, iná než v jadre (§2). |
| `relations.rs:14` `GAP_LIMIT_MM = 20.0` | „Diely bližšie než 20 mm sú vo vzťahu" — nábytkárska heuristika ako `const`. |

### 9.3 Návrh

1. Plocha ako **topologický dotaz**, nie enum: `face_at(part, point)`, `face(part, normal=(0,0,1))`,
   `faces(part, filter=lambda f: f.area > …)` — vráti `FaceRef` (stabilné meno z jadra
   `ProfileFaceReference`/`StableFaceRole`). Šesť stien kvádra sú potom len pomenované skratky v
   `prelude.star`: `def z_plus(part): return face(part, normal=(0,0,1))`.
2. `hole(part, face_ref, at=(u,v) | world=…)` funguje na ľubovoľnej rovinnej ploche; pre valcové
   plochy `hole(part, axis=…)`.
3. `mirror` prepočíta `FaceRef` (jadro už má `TopologicalElementRef` s lineage) → obmedzenie
   „mirror last“ zmizne.
4. `GAP_LIMIT_MM`, `TOLERANCE_MM` prevziať z dokumentu (§2.3).

## 10. OCCT façade — profily ako ploché `Vec<f64>` so stride 10/14 (závažnosť B)

### 10.1 Nález (`ketchup-exact/src/lib.rs`)

- `PLANAR_SEGMENT_STRIDE = 10`, `SPATIAL_SEGMENT_STRIDE = 14` (r. 12-13): každý segment profilu sa
  cez FFI posiela ako 10 resp. 14 `f64`, kde **prvý float je kód druhu** (`0.0` = line, `…`,
  `3.0` = circle, r. 745-756, 1372-1383) a nepoužité sloty sú `0.0`. Posledný slot `f64::from(loop_index != 0)`
  je bool ako float. Na strane C++ (`native.cc`, 295 KB) sa to dekóduje rovnakými indexmi.
  Pridanie nového druhu segmentu (napr. elipsa, NURBS) = zmena stride na oboch stranách a
  všetkých indexov.
- Pevné limity `MAX_PLANAR_LOOP_SEGMENTS = 64`, `MAX_SWEEP_PATH_SEGMENTS = 64`,
  `MAX_PLANAR_REGION_HOLES = 64`, `MAX_PLANAR_REGION_SEGMENTS = 4096` (r. 15-18): profil
  s 65 segmentmi (polygonizovaná krivka, obrys písma, importovaný DXF) je odmietnutý. Dôvod
  limitu (ochrana pamäte? čas?) nie je dokumentovaný; nemá žiadny vzťah k skutočnej zložitosti
  pre OCCT.
- `sweep_path_self_intersects` (r. 710-739) považuje **prekryv AABB dvoch nesusedných segmentov**
  za samopretnutie. Cesta tvaru U s blízkymi ramenami alebo špirála je falošne odmietnutá.
  Skutočný test pretnutia (segment–segment, arc–segment) je O(n²) pri n ≤ 64 triviálny.
- `TOLERANCE_PROFILE = "r0-v1:bbox=1e-6mm:volume_abs=1e-6mm3:volume_rel=1e-10"` (r. 8) je
  tolerancia zakódovaná ako reťazec, ktorý sa nikde neparsuje — len porovnáva. Neprepojené s §2.
- `surface_kind: String`, `curve_kind: String` v `NativeFaceEvidence`/`NativeEdgeEvidence`
  (r. 59, 1548, 1563) a potom `matches!(evidence.curve_kind.as_str(), "line" | "circle")` v
  `planner.rs:1273`, `exact_product.rs:291,302`, `scheduler/lib.rs:418,429` — stringly typed
  enum prechádzajúci štyrmi crate-mi.

### 10.2 Návrh

1. Cez `cxx::bridge` posielať `Vec<NativeSegment>` so štruktúrou `{ kind: u8, p: [f64; 12] }`
   alebo ešte lepšie samostatné `Vec<NativeLine>`, `Vec<NativeArc>`, `Vec<NativeBezier>` s indexovým
   poradím. `cxx` podporuje shared structs a `Vec<T>`.
2. Limity odvodiť od rozpočtu (čas/pamäť) nie od počtu; alebo aspoň zdvihnúť na rádovo 10⁴ a
   pomenovať dôvod.
3. `SurfaceKind`/`CurveKind` ako `#[repr(u8)] enum` v `ketchup-core`, mapovanie na OCCT
   `GeomAbs_SurfaceType` v C++; string len v JSON pre AI.
4. Samopretnutie: skutočný test, s toleranciou z dokumentu.

## 11. Worker protokol — textové riadky, pozičné polia, 23 verzií (závažnosť A)

### 11.1 Nález (`ketchup-scheduler/src/lib.rs:4660-4771`, `exact_worker.rs`)

- Odpoveď workera je riadok `OK_BREP_GRAPH_V23 <sha256> <sha256> <u64> <fnv> <fnv> <f64 ako hex bits>×8 <u32>×5 …`
  parsovaný **pozičnými indexmi** (`fields[1]`, `parse_f64(6)`, `parse_u32(16 + topology_offset)`,
  `fields[19 + topology_offset]`). JSON pre plochy/hrany je **hex-enkódovaný vnútri poľa**.
- Scheduler akceptuje `OK_BREP_GRAPH_V8` … `V23` (16 verzií) a podľa verzie posúva indexy
  (`topology_offset`, `wire_count_index`, `faces_index`). Grep `"OK_BREP_GRAPH_V\d+"` → 58 výskytov
  v `lib.rs`, 17 v `exact_worker.rs`. Worker a scheduler sú v jednom repozitári a jednom builde —
  neexistuje scenár, kde by bežali rôzne verzie.
- `MAX_WORKER_RESPONSE_LINE_BYTES = 64 KiB` + `response_transport.rs` chunkovanie (`CHUNK_BYTES =
  MAX − 1`) — vlastný framing nad riadkovým protokolom, lebo riadok je krátky.
- Timeouty ako konštanty: `DEFAULT_WORKER_REQUEST_TIMEOUT = 5 s`, `EXACT_BREP_GRAPH_REQUEST_TIMEOUT
  = 60 s` (r. 830-833). Veľký model (1000+ dielov) nedostane viac času; malý nedostane menej.
  V pamäti je záznam, že live-bridge timeouty už raz museli ísť z malého SLA na 60 s → 120 s.
- Limity `MAX_EXACT_BREP_GRAPH_MESH_TRIANGLES = 200_000`, `MAX_CACHED_GRAPH_OUTPUTS = 512`,
  `MAX_EXACT_PAIR_CANDIDATES = 10_000` bez odvodenia.

### 11.2 Návrh

1. Jeden `#[derive(Serialize, Deserialize)] enum WorkerResponse { BrepGraph(BrepGraphResult),
   Error(GeometryError), … }` + length-prefixed framing (`u32 len + postcard/bincode bytes`).
   Verzia protokolu jedna (handshake pri štarte workera); staré varianty zmazať. Odhad −4 000 riadkov.
2. Timeouty ako funkcia veľkosti požiadavky (`base + per_node × node_count`) alebo progres-based
   (worker posiela heartbeat; timeout je „bez heartbeatu N s“).
3. Limity: pomenovať ako `ResourceBudget` s hodnotami z konfigurácie, nie `const`.

## 12. `native.cc` — inline tolerancie a hľadanie hrán podľa geometrie (závažnosť A)

### 12.1 Nález (`ketchup-exact/src/native.cc`, 6 599 riadkov, jeden súbor)

- Inline literály: `1.0e-9` ×41, `1.0e-7` ×24, `1.0e-6` ×7, `1.0e-12` ×5; `Precision::Confusion()`
  len ×11. Tolerancie nie sú konzistentné ani s Rust stranou (§2), ani s OCCT.
- `kind == 1.0` / `kind == 2.0` / `kind == 3.0` ×45 — dekódovanie druhu segmentu z floatu (§10).
- `offset += 10` ×7 — stride duplikovaný ako literál, nie ako konštanta zdieľaná s Rustom.
- Posledný commit `783662b` je typický príklad špeciálneho prípadu: `extrude_mixed_profile_native`
  hľadá referenčnú hranu profilu **porovnávaním koncových bodov** (`Distance(...) <= 1.0e-9`); kružnica
  z dvoch polkruhov má obe hrany s rovnakými koncami → pridala sa ďalšia podmienka na stred oblúka
  (`<= 1.0e-6`). Ďalší tvar (dva oblúky rovnakého stredu a rôznych polomerov? uzavretý Bézier?)
  bude ďalší patch. Správne riešenie: hrany vytvára tento istý kód zo segmentov, takže
  `segment_index → TopoDS_Edge` je známy priamo; nemusí sa nič hľadať podľa geometrie.
- 8 stavových kódov `STATUS_*` v 366 použitiach; diagnostika ide ako voľný text
  (`"OCCT segmented profile reference edge is ambiguous"`) bez lokality v mm.

### 12.2 Návrh

1. `native.cc` rozdeliť podľa operácií (`extrude.cc`, `revolve.cc`, `sweep.cc`, `boolean.cc`,
   `evidence.cc`, `profile.cc`) s jedným `tolerance.hxx` (`kLinear`, `kAngular`) nastavovaným z Rustu
   pri každom volaní (alebo raz per worker session).
2. Hrany/plochy vytvorené zo vstupu sledovať cez `TopTools_IndexedMapOfShape` alebo
   `BRepTools_History` — OCCT to priamo poskytuje (`Generated()`, `Modified()`); Kečup už history
   používa pri Loft-e (viď pamäť: `FirstShape/LastShape/GeneratedFace`), stačí ju použiť aj tu.
3. Chybové výsledky ako `struct { code, message, location_mm[3], shape_ids[] }`.

## 13. Stav aplikácie — 204 polí, paralelné `Option`-y (závažnosť A pre udržateľnosť UI)

### 13.1 Nález (`ketchup-app/src/lib.rs:5143-5360`)

| Vzor | Počet | Príklad |
|---|---|---|
| `pending_*: Option<Pending…>` (modálne dialógy) | 16 | `pending_tag_creation`, `pending_tag_deletion`, `pending_tag_clear`, `pending_tag_rename`, `pending_tag_assignment` … |
| `pending_*_import` per formát | 6 | `pending_stl_import`, `pending_dxf_import`, `pending_step_import`, `pending_iges_import`, `pending_sketchup_scene_import`, `pending_glb_import` |
| `*_preview: Option<…>` per nástroj | 11 | `preview`, `preview_box`, `revolve_preview`, `sweep_preview`, `loft_preview`, `planar_offset_preview`, `general_finish_preview`, `drawn_shape_preview`, `occurrence_operation_preview`, `smart_push_pull_proposal`, `face_offset_evaluation` |
| `*_visible: bool` (zobrazenie) | 22 | `shadows_visible`, `fog_visible`, `xray_visible`, `jitter_visible`, `dashes_visible` … |
| stav nástroja ako ploché polia | ~25 | `sketch_start/end/cursor`, `line_chain_origin/points/items`, `measure_start/cursor/end`, `move_copy_mode`, `rotate_copy_mode`, `move_axis_lock`, `rotate_axis_lock`, `scale_axis_lock`, `line_axis_lock` |
| konfigurácia exportéra v App | 2 | `btlx_profile_strategy`, `btlx_intermediate_saw_cuts` |
| vstupné reťazce per dialóg | 8 | `push_pull_distance_input`, `pocket_depth_input`, `parameter_expression_input`, `assistant_target_input`, `assistant_value_input`, `classification_*_input` … |

Každé pole je nulované ručne v inej funkcii (`clear_ephemeral_edit_state`, `cancel_transform_gesture`,
Escape handler, undo/redo…), takže je ľahké zabudnúť jedno a mať „duch“ preview po Undo.
`handle_shortcuts` (367 r.) má 42 `Key::*` literálov — klávesové skratky nie sú dáta, nedajú sa
zobraziť v jednom zozname ani prekonfigurovať; `shortcuts_open` dialóg musí byť ručne udržiavaný.

### 13.2 Návrh

1. `enum Modal { None, TagCreate(…), TagDelete(…), Import(ImportFormat, PendingImport), Rename(…), … }`
   — jedno pole `modal: Option<Modal>`; neplatný stav „dva modály naraz“ prestane existovať.
2. `enum ToolPreview { Box(…), Revolve(…), Sweep(…), … }` — jedno pole; `cancel()` je jedno
   priradenie `None`.
3. `struct ViewSettings { flags: EnumSet<ViewFlag> }` s `ViewFlag::{Shadows, Fog, XRay, …}` —
   menu View sa generuje iteráciou, persistuje sa ako bitset.
4. `enum Gesture { Idle, Sketch(SketchState), LineChain(…), Measure(…), Move(MoveState), … }` —
   exkluzívny stav nástroja (už identifikované v predchádzajúcom audite, stále neriešené).
5. Skratky ako tabuľka `[(Command, KeyChord)]` v `keymap.rs`, z ktorej sa generuje aj dialóg
   Shortcuts aj menu tooltipy; používateľ ju môže neskôr prepísať (professional CAD štandard).
6. `ImportFormat` enum + jeden `PendingImport` — DXF/STL/STEP/IGES/GLB/SKP majú rovnaký životný
   cyklus (vyber súbor → náhľad → potvrď → vlož).

## 14. `ketchup-core` je „kitchen sink“ a každý typ má 4–5 ručných serializácií (závažnosť A)

### 14.1 Nález

- `ketchup-core/src/lib.rs` exportuje 40 modulov, medzi nimi `cam` (72 KB), `fea` (51 KB),
  `local_pdm` (40 KB), `blender_export`, `three_mf_export`, `dxf_export`, `drawing` (172 KB),
  `drawing_export`, `sheet_metal`, `assistant_sidecar` (163 KB — protokol AI asistenta),
  `fabrication` (179 KB, „BEAM_FABRICATION_EVALUATOR“), `joinery`, `mechanical_*`,
  `validator_hosting`. „Core“ tým pádom nie je geometrické jadro, ale celá aplikačná vrstva;
  nedá sa použiť samostatne (napr. headless bez CAM/FEA/PDM/AI).
- Každý dokumentový typ prechádza **piatimi ručnými traverzami**: `persistence.rs` write +
  read (375 KB), `document/stable_digest.rs` (143 KB; `command` 804 r., `feature_kind` 768 r.),
  `state_view.rs` text encoder (117 KB, 2 737-riadková funkcia), JSON pre AI (`model_query.rs`
  89 KB, `assistant_sidecar.rs`). Ak niekto pridá pole do `FeatureKind::Loft`, musí ho pridať na
  5 miestach; ak zabudne v `stable_digest`, dva rôzne dokumenty majú rovnaký digest (tichá chyba).
- ~115 verzionovaných string ID `pub const X_V1: &str = "ketchup.…-v1"` naprieč crate-mi
  (18 v `exact_brep_graph.rs`, 18 vo `fabrication.rs`, 10 v `exact_validation.rs`). Slová
  `digest|fingerprint|receipt|envelope|evidence` majú ~900 výskytov v `ketchup-core/src`.
  AGENTS.md §4 zakazuje pridávať nové, ale existujúce tvoria väčšinu kódu okolo každej
  operácie (`FabricationProjectionEnvelope { projection_schema, evaluator_id, source_digest,
  result_digest, … }`).
- `prismatic.rs::TolerancePolicy` (default `1e-7`) **existuje**, ale používa sa len vo
  validácii/kolíziách; `sketch.rs`, `joinery.rs`, `drawing.rs`, `exact/*` majú vlastné
  konštanty (§2). Typ je správny, len nie je prepojený.

### 14.2 Návrh

1. Rozdeliť workspace: `ketchup-geometry` (sketch, profile, brep graph, tolerance, topology),
   `ketchup-document` (store, commands, undo, persistence), `ketchup-exact` (OCCT), `ketchup-export`
   (dxf/3mf/glb/blender/drawing), `ketchup-cam`, `ketchup-fea`, `ketchup-pdm`, `ketchup-assistant`.
   Závislosti len smerom dole. Headless a program môžu závisieť len od geometry+document.
2. Jedna reprezentácia = jeden `#[derive(Serialize, Deserialize)]`; digest = SHA-256 nad
   kanonickou serializáciou (`postcard` je deterministický), `state_view` = `serde_json`,
   AI JSON = `serde_json` s `#[serde(rename)]`. Odhad −8 000 riadkov ručných traverzov.
3. String ID `*_V1` nahradiť jednou verziou dokumentu; evaluator/projection envelope zrušiť tam,
   kde neexistuje reprodukovaný bug, ktorý chytá (AGENTS.md §4 dáva na to mandát).
4. `TolerancePolicy` povýšiť na dokumentovú vlastnosť a pretiahnuť do všetkých modulov (§2.3).

## 15. Testy (závažnosť B)

- `ketchup-app/src/tests.rs` 684 KB, `tests/headless_shell.rs` 598 KB, `tests/assistant_m7.rs`
  277 KB, `ketchup-scheduler/tests/exact_brep_graph.rs` 272 KB, `ketchup-core/tests/gate_d.rs`
  252 KB. Jeden test file = desiatky minút kompilácie pri zmene jedného testu.
- Názvy `gate_a1`, `gate_b`, `gate_c1a_projection_authority`, `gate_d`, `validation_m17`,
  `renderer_m16`, `plugin_m7b`, `assistant_m7`, `spatial_m16` — testy pomenované podľa
  míľnikov/brán, nie podľa správania. Nový vývojár nevie, čo `gate_d.rs` testuje, bez otvorenia.
- `manual_bottle_documentation.rs`, `timber_frame_house.rs`, `garden_studio_source_parity.rs` —
  produktové testy sú v poriadku (AGENTS.md §5 ich chce), ale mali by byť golden testy programu
  (`examples/*.star` → očakávaný BOM/objem), nie 100 KB Rust kódu.
- Návrh: rozdeliť podľa správania (`push_pull.rs`, `save_reopen.rs`, `undo.rs`…), premenovať
  gate/milestone súbory, golden testy ako dátové fixtures.

## 16. Zhrnutie a poradie prác

### 16.1 Čo je dobré (aby review nebol len negatívny)

- `ketchup-program` má správny tvar: malé generické builtiny, doména v `prelude.star`, test na
  dokumentáciu každého builtinu.
- `ketchup-interaction` je rozumne malý a modulárny.
- AGENTS.md formuluje presne správne pravidlá; problém je, že kód pred ním vznikol a ratchet
  (§8) nepokrýva kľúčové slová.
- `FeatureExtent { Blind, ThroughAll, UpToFace }` a `PadSpec` už sú správna generalizácia —
  treba len zrušiť staršie varianty.
- `TolerancePolicy` a `TopologicalElementRef` s lineage existujú — treba ich len použiť všade.

### 16.2 Priority (od najväčšej páky)

| # | Práca | Efekt | Odhad |
|---|---|---|---|
| 1 | **Jedna serializácia** (serde) namiesto persistence/digest/state_view/JSON traverzov (§6, §14) | −8–10 k riadkov, koniec shotgun surgery, koniec 97 schém | 2–3 týždne |
| 2 | **Zlúčiť `FeatureKind`** varianty: 5 extrúzií → 1, 3 profily → 1, 2 shelly → 1, `FaceRef` enum (§4) | −3–5 k riadkov, jednoduchší planner/UI/worker | 1–2 týždne |
| 3 | **Worker protokol** cez serde framing, jedna verzia (§11) | −4 k riadkov, žiadne pozičné indexy | 3–5 dní |
| 4 | **Tolerancie**: `TolerancePolicy` v dokumente, prepojiť Rust ↔ C++ (§2, §12) | konzistentné správanie, konfigurovateľnosť | 3–5 dní |
| 5 | **Doména von z jadra**: `joinery` → generický pin joint, nábytkárske validátory → dátové pravidlá, `grain` → metadata (§3, §9) | súlad s AGENTS.md §1, otvára iné domény | 1–2 týždne |
| 6 | **Program `Face`** ako topologický dotaz namiesto šiestich stien (§9) | diery/kontakty na ľubovoľných dieloch, mirror bez obmedzenia | 1 týždeň |
| 7 | **`KetchupApp` stav**: `Modal`, `ToolPreview`, `Gesture`, `ViewSettings`, keymap tabuľka (§13) | menej „duch“ stavov, konfigurovateľné skratky | 1–2 týždne |
| 8 | Rozdelenie `ketchup-core` na crate-y (§14.2.1) | rýchlejšie buildy, jasné hranice | 1 týždeň po 1–2 |
| 9 | `native.cc` rozdeliť, `segment_index → Edge` mapa namiesto geometrického hľadania (§12) | koniec patchov typu `783662b` | 3–5 dní |
| 10 | Chyby: `Rejection` typ + lint na `map_err(\|_\|` (§7) | AGENTS.md §2 vynútené kompilátorom | priebežne |
| 11 | Ratchet: rozšíriť slová, rozdeliť testy (§8, §15) | hygiena | 1 deň |

Body 1–3 sú prerekvizita pre všetko ostatné: kým každý typ žije v piatich ručných traverzoch
a worker parsuje pozičné polia, každé zjednodušenie `FeatureKind` stojí päťnásobok práce.
