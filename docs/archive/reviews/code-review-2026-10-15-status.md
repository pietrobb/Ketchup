# Review 2026-10-15: stav opráv k 2026-10-16

Stav overený na `992c0c8` (`main`), 61 commitov po `52a0828`, na ktorom bol robený review
`code-review-2026-10-15.md`.

## Metóda

- Štyria nezávislí čitatelia po oblastiach. Pri každom náleze si prečítali pôvodný text,
  `git show` opravného commitu a výsledný kód. Pýtali sa, či je príčina odstránená, či
  scenár kryje test a či oprava nepriniesla novú chybu.
- Overenie stavov:
  - Nové nálezy so stĺpcom **K** som sám overil čítaním kódu.
  - Nálezy označené *meraním* pomocný čitateľ spustil na kópii stromu.
  - Ostatné nálezy sú statické.
- Spustené na Linuxe na `992c0c8`:
  - všetky `scripts/check_*.py`: prešli;
  - lokálne celý job `rust-without-occt` z CI (`fmt`, `clippy` 14 crate a `ketchup-model
    --lib`, testy): prešiel, 601 testov, 0 zlyhaní.

## Súhrn

| | počet |
|---|---:|
| opravené | 30 |
| čiastočne | 16 |
| neopravené | 2 (UI-5, EX-1) |
| nové nálezy | 2 vysoké, 10 stredných, niekoľko nízkych |

Tempo a disciplína opráv sú výborné. Takmer každý nález má vlastný commit s testom, ktorý
scenár naozaj prechádza. Napríklad:

- Move po +Y a šikmo;
- Scale ×2 → 0 → 3 dá ×3;
- SVG oblúk podľa bodov BTLx;
- zahodená história s programom cez Open a Save.

Upratovanie repozitára je urobené. `docs/` obsahuje len normatívne dokumenty, binárky sú v
release `sample-files` a žiadny súbor nie je väčší ako 5 000 riadkov. Ratchety majú
CODEOWNERS a nedá sa ich obísť.

Dve veci sa však zhoršili:

1. **Oprava PRG-5 zaviedla falošné „dotýka sa“** pri skutočných prienikoch v rade milimetrov
   (N-1).
2. **Commit bf8f1ef zrušil OCCT job.** Asi 82 % testov (~2 700 z ~3 300) dnes nebeží v
   žiadnom CI jobe (N-2).

## Stav pôvodných nálezov

### Perzistencia

| ID | Stav | Poznámka |
|---|---|---|
| PS-1 | čiastočne | **Hotové:**<br>• Povinný záznam `program.json` s vlastným SHA-256.<br>• Pri nezhode s históriou vyhrá `program.json`.<br>• GUI aj headless varujú.<br>• Test `a_discarded_history_keeps_the_current_program_through_open_and_save`.<br>**Ostáva:**<br>• **K**: `migration_review.rs:171` ukladá cez `save_container`, ktorý posiela program `None` (`persistence.rs:847-858`). Migrovaný súbor s programom ho stále stratí.<br>• Súbor spred 1d52a1f (bez `program.json`) so zlou históriou program stráca, krátkodobý fallback nie je urobený. |
| PS-2 | opravené | Nepoužiteľný checkpoint sa presunie na `.work-recovery.stale[-N]` s hlásením. |
| PS-3 | čiastočne | Novší formát kontajnera, entry aj polí sa odmietne. Ostáva `UnsupportedHistorySchema`: novšia história sa potichu zahodí a Save ju prepíše starším formátom. |
| PS-4 | čiastočne | Textové a celočíselné kľúče idú cez hash. Štruktúrne a tuple kľúče (`local_occurrences`, `instance_transform_overrides`, …) sa stále prehľadávajú lineárne. |
| PS-5, PS-6, PS-7, PS-8, PS-9, PS-10 | opravené | |
| PS-11 | čiastočne | Rozdelenie dispatchu je verné a zhoda je úplná. Každá dávka ďalej klonuje celý `ProductModel`. Rodinné moduly končia `_ => {}`, takže zle smerovaný príkaz ticho nič neurobí. |
| PS-12 | čiastočne | Načítajú sa len ponechané snapshoty. Všetky záznamy (až 4 096) sa ale stále parsujú a každý ich program sa validuje (pozri N-5). |

### Program a geometria

| ID | Stav | Poznámka |
|---|---|---|
| PRG-1 | čiastočne | `report()` beží vo workeri s deadlinom 60 s. Release na `tiny-house` zbehne za 1,2 s a na `house-project` za 2,2 s, takže reálne modely deadline neodmietne. Ostáva:<br>• strop „prvých N issues“ nie je; veľký report sa odmietne celý ako `report_too_large`;<br>• `Report::refine` beží v okne bez limitu (N-4);<br>• na Unixe nie je pamäťový limit (`scheduler/src/lib.rs:1051`). |
| PRG-2 | opravené | Booleany pred posledným mirrorom sa z box úvah vynechajú a pár ide na exact. Testy pokrývajú oba smery, ale len na boxoch. |
| PRG-3 | čiastočne | Strop 10 000 telies platí na jeden diel, nie na celý model (N-6). Hĺbka vnorenia nie je obmedzená (N-7). |
| PRG-4, PRG-7 | opravené | |
| PRG-5 | čiastočne, **s regresiou** | Jedno pravidlo pre box a exact v `validate` a `relations`. `connectivity.rs:10`, `group_connectivity.rs:116` a `contact_joints.rs:54` majú ďalej vlastné pravidlo. Pozri N-1. |
| PRG-6 | čiastočne | Pokryté sú `replace`, `format`, `%`, `str`/`repr`, `json` aj BigInt. Ostáva pamäť modelu mimo heapu a Unix bez limitu. Počítadlo BigInt bajtov len rastie, takže dlhý program môže dostať falošný `memory_limit`. |
| PRG-8 | opravené (rozhodnutím) | `CONTACT_FIT_MM` je v `ketchup-tolerance`. Ratchet obíde konštanta s iným menom (`faces.rs:55` `TOUCH_MM`, `program_pick/provenance.rs:6` `MATCH_MM`). |
| EX-1 | neopravené | `exact_worker.rs:936` má stále len strop počtu (512), bez bajtového rozpočtu. |

### GUI

| ID | Stav | Poznámka |
|---|---|---|
| UI-1, UI-2, UI-3, UI-4, UI-7 | opravené | Každý s testom presne podľa scenára review. |
| UI-5 | neopravené | `shown_reading` (`gesture.rs:62`) sa zapisuje len vo `viewport.rs:2467-2469` a nikde sa nenuluje. |
| UI-6 | čiastočne | Move a obdĺžnik zdieľajú `parse_length_list`, `80*60` opäť funguje. Scale má vlastný parser (holé `150` je faktor). Nový parser je nejednoznačný (N-10). |

### Bezpečnosť a SDK

| ID | Stav | Poznámka |
|---|---|---|
| SEC-1 | opravené pre tvar z review | Samostatný argument s `..`, relatívny aj absolútny, ide do procesu ako stage kópia hashovaného súboru. Obchádza ho `--script=../…` (N-8). |
| SEC-2 | opravené | Test je `#[cfg(windows)]`, takže po bf8f1ef ho žiadne CI nespustí. |
| SEC-3, SEC-4 | opravené | SEC-4: neescapujú sa ešte U+00AD, U+034F, U+FE0F a tagy U+E00xx. |
| SEC-5 | čiastočne | **K**: import sa už pýta, ale `confirm_live_read_path` vráti `true`, keď okno nie je pripojené cez consent (`consent.rs:274`). Okno spustené cez MCP `open_window` teda otvorí aj importuje ľubovoľný súbor bez opýtania. |
| SEC-6 | väčšinou opravené | Hotové sú strop kópie, skupina procesov, HMAC pri pripojení a vytláčanie slotov. Ostávajú N-9 a nízke body nižšie. |
| SDK-1 | opravené | |

### Výroba

| ID | Stav | Poznámka |
|---|---|---|
| MF-1 | čiastočne | Každý export (BTLx, job, výkres) pravidlo kontroluje. Ostáva:<br>• `product_validation.rs` a aplikácia `SetHoleThread` ho nekontrolujú;<br>• starý súbor s nevyhovujúcim závitom sa nedá vyexportovať a GUI nemá úpravu ani zrušenie závitu, takže jediná cesta je zmazať dieru a nakresliť ju znova;<br>• N-3. |
| MF-2, MF-4 | opravené | MF-2: test porovnáva flagy oblúka so smerom bodov BTLx, SVG nevykresľuje. |
| MF-3 | čiastočne | Loss report je pridaný. Golden súbory sa proti XSD stále nevalidujú. |

### Viewer, CI, repozitár

| ID | Stav | Poznámka |
|---|---|---|
| VW-10 | opravené | Balík sa zapíše raz, komentáre do malého súboru. Prvý zápis balíka (až ~264 MB s fsync) ostáva na UI vlákne. |
| VW-11 | čiastočne | Web: opravené. Android: `"wt"` stále nie je atomický, kill procesu počas zápisu nechá súbor skrátený a `.bak` sa nikdy neobnoví. |
| CI-1 | čiastočne | Linux job a brána „CI passed“ fungujú, fork už nedostane zelenú bez buildu. Pozri N-2. |
| CI-2, CI-3, CI-4 | opravené | CI-3: `--allow-growth` ostal v `check_tolerance_literals.py`, `check_test_sleeps.py` a `check_no_named_products.py`. |
| REPO-1, REPO-2, TST-1 | opravené | Strom má 50 MB a odkazy v dokumentácii sedia. Testy pod `viewer/*` limit veľkosti súboru nesledujú. |

## Nové nálezy

Zoradené podľa závažnosti. **K** = overené mnou čítaním kódu, *meranie* = spustené
pomocným čitateľom.

| ID | Závažnosť | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|---|
| N-1 | **vysoká** | K | `ketchup-program/src/exact.rs:110-135` | **Oprava PRG-5 hlási skutočné prieniky ako dotyk.**<br>• Povolený spoločný objem je `CONTACT_FIT_MM × plocha prekryvu boxov`, nie skutočná dotyková plocha.<br>• Keď box jedného dielu obsahuje druhý diel (`shell()` zásuvka, L, U, rám, prstenec), je plocha prekryvu veľká.<br>• Príklad: doska 300×300×10 otočená o 45° v boxe zásuvky. Povolené je až 900 mm³, takže roh zarezaný 9,5 mm do 12 mm steny sa hlási ako dotyk.<br>• Tyč 6×6 dlhšia o 0,4 mm na každom konci tiež prejde. | • Hĺbku odhadnúť ako spoločný objem / skutočná plocha kontaktu (z OCCT), alebo porovnať s prierezom menšieho dielu pozdĺž tenkej osi prekryvu.<br>• Test s doskou v zásuvke cez skutočné exact teleso, nie syntetický `ExactPair`. |
| N-2 | **vysoká** (proces) | K | `.github/workflows/ci.yml`, bf8f1ef, `ketchup-model/Cargo.toml:30` | **Väčšina testov nebeží v žiadnom CI.**<br>• Bez OCCT jobu nebežia testy `ketchup-app`, `application`, `exact`, `scheduler`, `headless`, `model` ani desktop `viewer`. To je ~2 700 testov a ~162 tisíc riadkov testov.<br>• Rovnako nebeží clippy týchto crate a žiadny `cfg(windows)` kód (job objekty, bridge, SEC-2).<br>• Pravidlo §8 AGENTS.md dnes stojí len na disciplíne.<br>• `ketchup-model` má `ketchup-exact` (a tým OCCT) len ako dev-dependency, ktorú žiadny test nepoužíva. | Od najlacnejšieho:<br>1. Zmazať nepoužitú dev-dependency `ketchup-exact` v `ketchup-model` a pridať jeho testy (~700) do Linux jobu.<br>2. Pridať testy `ketchup-viewer` cez lavapipe.<br>3. GitHub-hosted job, ktorý OCCT 8.0.1 zostaví raz a cachuje (`actions/cache`), pustený nočne alebo na push do `main`, aj s `large_house`.<br>4. Windows hosted job aspoň pre `cargo clippy --workspace`.<br>5. AGENTS.md §6 doplniť o výnimku pre `-p` v CI: dnes CI porušuje pravidlo, ktoré dokument zakazuje. |
| N-3 | stredná | K | `ketchup-geometry/src/tapping.rs:128-129`, `:151-165` | **Nástroj na diery vytvorí dieru, ktorú export odmietne.**<br>• `blind_hole_depth_mm` zaokrúhľuje hĺbku diery na 1 µm, `check` porovnáva s toleranciou modelu ~1e-7 mm.<br>• Príklad: M8, závit 10/3 mm. Diera má 9,533 mm a najhlbší závit 3,333 mm < 3,3333 mm, takže každý export dieru odmietne. | Zaokrúhľovať nahor (`ceil`), alebo kontrolovať s toleranciou ½ µm. |
| N-4 | stredná | S | `ketchup-program/src/rule_exact_collisions.rs:73`, `lib.rs:121-122` | **`Report::refine` opakuje celú kontrolu v okne.**<br>`validate_with` aj `relations_with` bežia na vlákne v procese okna bez deadlinu a pamäťového limitu. Vlákno beží ďalej aj po timeoute klienta. | Refine robiť vo workeri, alebo len nad pármi, ktoré exact rozhodne. |
| N-5 | stredná | S | `ketchup-model/src/persistence.rs:1858`, `:1990-2025` | **Zlý program v starom Undo kroku odmietne celý súbor.**<br>• Po PS-1 už toto odmietnutie nie je potrebné, aktuálny program je v `program.json`.<br>• Program v revízii mimo 200-krokového okna, ktorý by PS-12 aj tak zahodil, odmietne súbor.<br>• Chyba nie je konečná, takže sa padne na `.recovery`, teda na staršiu prácu. | S `program.json` históriu zahodiť s varovaním a validovať len ponechané záznamy. |
| N-6 | stredná | *meranie* | `ketchup-program/src/eval.rs:1269` | **Strop telies je na diel, nie na model.**<br>• Reťaz `subtract(p_i, p_{i-1})` rastie kvadraticky: 1 000 dielov dá 380 MB za 3,6 s.<br>• Na Unixe a v `ketchup-program check` to nič nezastaví. | Strop na celý model a veľkosť modelu počítať do rozpočtu. |
| N-7 | stredná | *meranie* | ciborium 0.2.2 (CBOR medzi workerom a oknom) | **Hlboko vnorený model sa z workera nevráti.**<br>• CBOR dekóduje do hĺbky 256. Reťaz booleanov s 80 úrovňami prejde, s 90 zlyhá (`RecursionLimitExceeded`), hoci model má len ~3 MB.<br>• Používateľ vidí „worker frame does not decode“. | Strop hĺbky v `apply_boolean` pod ~80 s pomenovanou chybou, alebo dekódovať s vyšším limitom. |
| N-8 | stredná | S | `live_bridge/plugin_manifest.rs:271-282` | **SEC-1 sa dá obísť.**<br>• Argument `--script=../plug/run.py` nemá zložku `ParentDir`, preto prejde nezmenený.<br>• Plugin si ho vyrieši voči stage a otvorí nehashovaný súbor. | Odmietnuť `..` kdekoľvek v argumente, alebo rozoznať `=`-pripojené cesty. |
| N-9 | stredná | S | `plugin_files.rs:103`, `plugin.rs:174,358`, `plugin_consent.rs:118` | **Neexistujúce cesty pluginu.**<br>• „Neexistujúca“ cesta mimo priečinka sa overí len okamihom kontroly, potom sa spustí. Iný proces ju medzitým môže vytvoriť (TOCTOU).<br>• Relatívny neexistujúci argument sa nezaznamená ani neodmietne a vyrieši sa voči pracovnému adresáru aplikácie. | Neexistujúce cesty, ktoré by sa dali spustiť, odmietnuť. Relatívne argumenty riešiť voči stage. |
| N-10 | stredná | S | `ketchup-app/src/length_input.rs:42-53` | **Spoločný parser dĺžok číta ten istý text rôzne.**<br>• V Move bez zamknutej osi je `12,5` vektor (12, 5, 0), aj keď gesto ukazuje smer.<br>• `2*20` je vektor, nie 40 mm.<br>• `12,5*3` sa potichu zmení na [12, 15], kým `12,5x3` dá [12,5; 3]. | Vektor len cez `;`. Čiarka je desatinná, keď je zložka jedna. `*` je násobenie, nie oddeľovač. |
| N-11 | stredná | S | `viewer/android/src/lib.rs:205`, `ketchup-viewer/src/app/session.rs:94-96` | **Android môže stratiť balík aj neuložené komentáre.**<br>• Po zlyhanom `keep_session` (napr. ENOSPC) sa značka posunie a komentáre nového balíka sa zapíšu vedľa starého `session.ketchup-view`.<br>• Pri štarte `Review::open` nezhodu odmietne, celé obnovenie zlyhá a balík aj komentáre sa stratia bez hlásenia. | Pri chybe značku neposúvať. Zlý súbor komentárov pri obnove brať ako „žiadne komentáre“. |
| N-12 | stredná | S | `ketchup-application/src/program_access.rs:478-497`, `rule_program.rs:295`, `program_check.rs:21` | **Pomalý report blokuje apply cez MCP a headless.**<br>`plan.report()?` je tvrdá brána pred zápisom: model s platnou geometriou, ktorého report trvá viac ako 60 s alebo je väčší ako 16 MiB, sa cez MCP ani headless nedá aplikovať. GUI cestu to neblokuje. | Report má hlásiť, nie blokovať (AGENTS §3). Pri timeoute aplikovať a report označiť ako nedokončený. |

**Nízke:**

- **Perzistencia:**
  - `program.json` pribudol bez zvýšenia `CONTAINER_SCHEMA`, takže starší build otvorí
    `.recovery`; treba to uviesť v poznámkach k vydaniu.
  - `verify()` pri Save nechytí súbor, ktorý `load()` odmietne pre program v histórii.
  - Headless open zlyhá, keď sa checkpoint nedá presunúť.
  - GUI pri otvorení ukáže len jednu správu (`app/document.rs:321-352`).
- **Pluginy a pripojenie:**
  - Na Linuxe a macOS sa súbory mimo priečinka pred spustením neoverujú znova.
  - Okno si nepamätá použité HMAC nonce.
  - Menu pluginov prechádza adresáre na UI vlákne (`plugin_manifest.rs:135`).
  - `poll_program_report` pred odpoveďou neoverí revíziu.
- **Android:** chýba fsync značky a adresára (`session.rs:70,129`) a obnovené bajty v
  `replace.rs:50-53`.
- **Dokumentácia:**
  - `README.md:50` tvrdí, že všetky príklady sú v `examples/`.
  - `artifacts/blender/import_garden_studio.py:5` odkazuje na odstránený `.blend`.
  - Mŕtve cesty v `docs/adr/0006`, `docs/adr/0007` a `viewer/web/HOSTING.md`.

## Odporúčané poradie

1. **N-1:** nesprávne „dotýka sa“ je presne trieda chyby, ktorej sa projekt chce vyhnúť.
2. **N-2, krok 1:** jedna zmazaná riadka v `Cargo.toml` vráti do CI ~700 testov modelu.
3. **SEC-5 a N-8:** dokončenie bezpečnostných opráv.
4. **PS-1:** cesta migračného review.
5. **N-3, N-5, N-11:** strata dát alebo odmietnutie vlastnej práce.
6. **N-2, kroky 3–4:** OCCT build v CI.
7. Zvyšok podľa tabuliek. UI-5 a EX-1 sú malé zmeny.
