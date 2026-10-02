# Review GUI, exportov a validácie — 2026-10-02

Baseline `e92f4a7`. Priebežný checkpoint hlavného reviewera; bez zmien produkčného kódu.

## Checkpoint opráv GUI/cache/MCP — 2026-10-02

Autorizovaný nezávislý workstream CR-02/03/15/16/17. HMOS overený read-only, AGENTS a hlavný report/protocol/verification prečítané. Implementácia rozpracovaná: cache prune cez verejnú revision_history (bez store.rs), preferovaný adaptér pre normálny štart, optional expected guard, bootstrap vlastného child a bounded regular-file čítanie. Doplnené outcome regresie GPU 0/1/2, Arc uvoľnenia po Undo limite/odstránení redo, stale expected po Open, opačného poradia readiness dvoch streamov, bounded limit+1, Unicode/escaping a špeciálnych súborov. Prvé validačné pokusy bg34/bg35 sa zastavili pred testami: súbežne pridaná duplicitná Unix dependency sekcia (zlúčená) a dočasný Windows mapped-file lock pri rustfmt. Nie sú PASS. Opakované scoped rustfmt prešlo. Cargo potom skončilo E0583: súbežná autentifikačná zmena v ketchup-mcp/src/lib.rs deklaruje local_auth, ale src/local_auth.rs zatiaľ neexistoval; ani dodatočné bounded čakanie 120 s neprinieslo súbor. Žiadna z nových regresií sa zatiaľ nevykonala, kompilácia app testov nie je potvrdená. Nevytváral som náhradný cudzí auth modul.

### Odovzdanie implementácie a validačný blocker

Zmeny tohto workstreamu (všetko pod C:/Sources8/Ketchup):
- crates/ketchup-app/src/app/exact.rs: preferencia discrete → integrated → ostatné fyzické DX12 adaptéry pri normálnom štarte; explicitná požiadavka zostáva unique. Oba exact archívy retain podľa zdrojov všetkých reachable revision snapshots vrátane redo.
- crates/ketchup-app/src/live_bridge.rs: len expected guard pre EditContext/Query/Detail/WorksetStatus/BatchJobStatus/View; ApplyProgram vetva nedotknutá.
- crates/ketchup-app/src/main.rs a src/live_bridge/bootstrap.rs: broker vzniká pred bootstrap readiness; readiness môže obsahovať vlastné instance_id a consent_address, nikdy token. Pôvodný bootstrap bez brokera stále vracia pôvodné dve polia.
- crates/ketchup-mcp/src/tools.rs a src/discovery.rs: open_window číta bounded readiness svojho child stdout a používa jeho bridge s tokenom zaslaným iba privátnym stdin. Žiadny výber prvého nového discovery. Pri zlyhaní ukončí iba vlastný child. Readiness limit 512 B, štart 30 s, stdout priebežne drainovaný. source_path vyžaduje absolútny regular file, precheck aj kontrolu otvoreného handle, Unix O_NONBLOCK/O_NOFOLLOW, Windows OPEN_REPARSE_POINT, read najviac 32769 B pre 32768 B limit; existujúci finálny escaped-envelope guard v bridge.rs zostal zachovaný.
- crates/ketchup-mcp/Cargo.toml/Cargo.lock: Unix libc zdieľané so súbežným auth workstreamom; duplicitná sekcia odstránená, cudzie windows-sys/lint zmeny zachované.
- Testy: app/src/app/exact.rs (adapter_tests), app/src/tests/exact_modeling.rs (exact_history_releases_evicted_and_abandoned_packages_but_keeps_redo), app/src/live_bridge/tests.rs (optional_expected_rejects_replaced_document_without_camera_or_query_effects), app/tests/live_bridge_bootstrap.rs (readiness_identifies_the_actual_child_window_when_broker_is_enabled), mcp/src/tests.rs (bounded_program_read_rejects_oversize_utf8_errors_and_special_files; escaped_source_envelope_is_refused_before_send_and_connection_remains_usable; concurrent_launch_readiness_is_bound_to_each_private_stream_not_registration_order).

Po dokončení cudzieho local_auth spustiť znovu: cargo test --manifest-path C:/Sources8/Ketchup/Cargo.toml --locked -p ketchup-mcp --lib; cargo test --manifest-path C:/Sources8/Ketchup/Cargo.toml --locked -p ketchup-app --lib adapter_tests; rovnaký app --lib príkaz s filtrami exact_history_releases a optional_expected; cargo test --manifest-path C:/Sources8/Ketchup/Cargo.toml --locked -p ketchup-app --test integration live_bridge_bootstrap:: -- --test-threads=2. Následne scoped clippy a fmt check.

Limity: implementácia je odovzdaná ako NEOVERENÁ regresiami, nie ako uzavreté CR. GPU testy sú čistý selector, nie fyzický multi-GPU run. Concurrent test používa dva privátne readiness streamy, nie dve živé GUI okná; app bootstrap má offscreen harness test. Windows special-file fixture je NUL, Unix fixtures /dev/zero, socket a symlink sú cfg(unix) a na tomto hoste nebežia. Bez živého GUI/OAuth, bez commit/push/stash, bez store.rs/consent.rs zmien týmto agentom a bez ukončenia cudzích procesov. Bez živého GUI/OAuth, commit/push/stash a zásahov do cudzích procesov; ApplyProgram a consent.rs vlastní hlavný agent.

## G1 — P1: Bežný štart odmietne viac než jednu vhodnú GPU

**Staticky potvrdené od entry pointu po chybovú vetvu; bez runtime skúšky na multi-GPU HW.**

- `crates/ketchup-app/src/main.rs:137–140` používa obyčajné `KetchupApp::native_options()`.
- `crates/ketchup-app/src/app/exact.rs:109–110` odovzdá `requirement=None`.
- `app/exact.rs:119–140` obmedzí backend na DX12, vyfiltruje fyzické adaptéry kompatibilné so surface a po nájdení druhého vráti `Err("multiple Direct3D 12 physical adapters matched the frozen requirement")`.
- Ak sa v zozname nachádza integrovaná aj dedikovaná GPU a obe podporujú surface, okno sa nespustí. Nie je to nejednoznačná požiadavka používateľa: používateľ žiadny konkrétny adaptér nepožadoval.

**Odporúčanie:** v normálnom štarte vybrať preferovanú dostupnú GPU; pravidlo presnej jedinečnej identity uplatniť len pri explicitnom diagnostickom requirement. Testovať čistú voľbu nad 0/1/2 kandidátmi vrátane integrated+discrete. DX12-only rozsah je samostatné platformové obmedzenie, nie druhý nález.

## G2 — P2: Neobmedzená história exact registrov

**Staticky potvrdené, spotreba RAM nemeraná.**

- `app_state.rs:187–188`: render aj topology história sú BTreeMap bez rozpočtu.
- `app/exact.rs:173–214`: archív udržiava každú zdrojovú revíziu; vloženie `history.insert(source, registry.clone())`.
- `app/exact.rs:244–256`: archivácia pri refresh cykle.
- Všetky referencie polí boli vyhľadané. Čistenie je iba pri otvorení/resetovaní dokumentu (`app/document.rs:109–112`) alebo pripojení workera (`app/exact.rs:387–389`), nie pri skrátení Undo histórie.

**Dopad:** neaktívne geometrické verzie zostávajú v pamäti cez Arc, aj keď už nie sú obnoviteľné cez Undo. Nezmenené balíky môžu byť zdieľané, preto sa tu netvrdí úplná kópia modelu pri každej zmene. Dlhá relácia s veľkými zmenami môže spotrebovať zbytočne veľa RAM.

**Odporúčanie:** previazať životnosť cache s dostupnými snapshotmi alebo bounded LRU/memory budget; otestovať uvoľnenie po viac než maximálnom počte Undo krokov.

## G3 — P2: Reprodukovateľne červené CI kontroly

- `cargo fmt --all -- --check`: PASS.
- `python scripts/check_crate_layers.py`: FAIL. `sidecar.rs::validate_with_part_cuts` 733 riadkov, limit 400; stará výnimka `::validate` už nie je oprávnená.
- `python scripts/check_no_named_products.py`: FAIL. Šesť prekročení: `scene_snapping.rs:panel`, `sidecar.rs:dowel/panel/shelf`, `document/command.rs:cabinet/dowel`.
- `python -m pytest -q tests sdk/python/tests`: 296 passed, 2 failed, 24 skipped, 71 subtests passed. Zlyhali `test_app_shell_has_no_oversized_module_or_function` a `test_core_and_application_crates_name_no_product_domain`.
- `check_error_hygiene`, `check_test_env`, `check_test_hooks`, `check_test_sleeps`, `check_tolerance_literals`, `check_tool_rail_docs`: všetky PASS.
- CI `.github/workflows/ci.yml:28–47` tieto skripty/testy vyžaduje.

**Dôležité rozlíšenie:** `document/command.rs:540–541` obsahuje cabinet/dowel iba v dokumentačnom komentári k budgetu; nie sú to doménové enumy ani algoritmus. Ide o nekonzistenciu zdrojového komentára s mechanickým ratchetom, nie dôkaz porušenia univerzálnej geometrie. Rovnako treba posúdiť zvyšné komentáre. Validátor treba skutočne rozdeliť, nie zvýšiť limit.

## Aktuálne metriky architektúry

Merané z `git ls-files`, prípony `.rs/.cc/.h/.py/.star`, iba crates/sdk/skills/scripts/tests. Celkom **505 súborov, 429 777 riadkov**. Test/production rozdelenie je podľa cesty; inline `#[test]` sa nezapočítava samostatne, nejde o coverage.

| Oblasť | Riadky spolu | Produkčné cesty | Testové cesty |
|---|---:|---:|---:|
| ketchup-app | 140112 | 73159 | 66953 |
| ketchup-model | 127726 | 80589 | 47137 |
| ketchup-application | 38982 | 22829 | 16153 |
| ketchup-scheduler | 23060 | 11932 | 11128 |
| ketchup-assistant | 17303 | 7030 | 10273 |
| ketchup-exact | 16201 | 12687 | 3514 |
| ketchup-program | 12189 | 9114 | 3075 |
| ketchup-manufacturing | 10936 | 6973 | 3963 |
| ketchup-geometry | 8337 | 7945 | 392 |
| ketchup-interaction | 8169 | 5533 | 2636 |
| ketchup-mcp | 1540 | 1299 | 241 |

Pozitívny posun: `ketchup-app/src/lib.rs` má 4754 riadkov oproti 36480 uvádzaným v minulom review. Zdrojové moduly sú pod 5000 riadkov. Zostávajú veľké koncentrácie zodpovedností: model/drawing.rs 4955, manufacturing/fabrication.rs 4741, app/viewport.rs 4482, assistant/sidecar.rs 4252, application/validation.rs 4026, application/planner.rs 3911, model/document/store.rs 3877. Samotná veľkosť sa tu nepočíta ako funkčná chyba; rozdeľovanie má sledovať životný cyklus a kontrakty, nie iba číselný limit.

## Prečítané cesty a zamietnuté podozrenia

- GUI document lifecycle: reset ruší presné tasky, vymaže exact registre/históriu a obnoví interakčný stav.
- `application/evaluation_task.rs:163` má Drop, ktorý volá cancel; samotné `self.exact.task.take()` pri stale revízii preto nie je leak zabudnutého cancel flagu.
- Scale nástroj je implementovaný: preview, numerický faktor, world-axis lock, commit, Undo a korekcia v `app/transform.rs:2320–2485`; testy `headless_shell.rs:13281–13425`. Starý otvorený goal nie je dôkaz chýbajúceho Scale.
- Podozrenie na stratu nested instance cesty v numerickej korekcii sa **nepotvrdilo ako dosiahnuteľná chyba**: `transform_operation.rs:40–46` povoľuje iba root InstancePath; definition-edit context má ďalší guard. Nested group a definition-local component nie sú to isté. Obmedzenie treba zohľadniť pri budúcich komponentoch, nie hlásiť ako súčasné poškodzovanie.
- Exporty: prečítané `fabrication/production.rs:59–220`, `fabrication.rs:1016–1127`, `1841–1949`, `2025–2135`. Existuje gate exact/current výsledkov, kontrola rigid right-handed instance transformu, vstupnej strany a hĺbky dier, explicitné číslo nástroja pre pocket a odmietnutie nepodporovaných operácií. Nenašiel som v tomto priechode dôkaz, že export potichu nahrádza exact meshom. Skutočný HOMAG/woodWOP runtime nebol testovaný.
- Snap optimalizácia: prečítané build scene geometry a nearest point/circle culling, vrátane perspektívnej korekcie world bodu. Bez nového potvrdeného nálezu zatiaľ.

## Testovacie obmedzenie a pokračovanie

Plný príkaz `cargo build --locked -p ketchup-scheduler --bin ketchup-exact-worker && cargo test --locked --workspace --all-targets --no-fail-fast -- --test-threads=2` sa zastavil už na builde workera: Windows Access denied pri odstránení `target/debug/ketchup-exact-worker.exe`. Proces je používaný existujúcou headless reláciou (worker PID 138496, parent ketchup-headless PID 27580). Žiadny cudzí proces nebol ukončený. Nie je to zlyhaný Rust test ani dokázaná chyba kompilácie.

**Záverečný výsledok:** workspace lib testy (`bg23-18dab7208f984068`) 686 passed/1 ignored; Clippy workspace all-targets all-features PASS. Integration bez scheduler/headless (`bg25-18dab77b301c428c`) 1664 passed/1 failed/4 ignored. Jediné zlyhanie je `legacy_migration.rs:79–85`: nesprávny predpoklad odlišného digestu pri každom uloženom príklade, vrátane nového `wardrobe-chair-table.ketchup`. App integration: 500 passed/0 failed/4 ignored. Osem funkčných nálezov bolo dodatočne izolovane reprodukovaných; finálny register, priority a limity sú v `code-review-2026-10-02.md`. Review je dokončený, bez opráv produkčného kódu.
