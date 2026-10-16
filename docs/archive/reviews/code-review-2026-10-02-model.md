# Nezávislé code review — model/application — 2026-10-02

## Výsledok a metodika

**5 staticky doložených nálezov: 2× P1, 3× P2. Žiadna vykonaná reprodukcia.** P1 označuje vysokú prioritu (strata recovery vetvy alebo neukončiteľná kanonická operácia); P2 strednú prioritu (porušenie failure atomicity alebo nefunkčný konkrétny workflow).

- Overený HEAD: `e92f4a73844b185570dfbb49a807ab2001ecd845`.
- Strom pri začiatku: iba nové review MD (model, protocol, hlavný review). Iné dokumenty nemením.
- Rozsah iba `crates/ketchup-model` a `crates/ketchup-application`; bez opráv, commitov, pushov, stash operácií a cargo build/test.
- HMOS skontrolovaný read-only cez SQLite; koreňový AGENTS.md prečítaný. Pod crates ani priamo v docs nebol nájdený ďalší AGENTS.md. Historické zoznamy sa nepoužívajú ako dôkaz.
- Dôkazy sú aktuálne zdroje baseline a čítané existujúce testy. Citácia testu **neznamená jeho spustenie ani potvrdenie, že prešiel**. Reprodukčné postupy nižšie sú návrhy pre hlavného reviewera.
- Nejde o tvrdenie úplného line-by-line auditu oboch rozsiahlych crates; konkrétne pokrytie a limity sú na konci.

## Checklist

- [x] HMOS, AGENTS.md, HEAD a pracovný strom
- [x] Hlavná atomicita apply_batch, transaction rollback, session mutácie
- [x] Undo/redo, truncation, persistent history a source-only revízie
- [x] Save/open, checksum, recovery, identity a container
- [x] Nested/shared occurrences: group conversion, instance paths, projekcia a replacement hranice
- [x] Async publish, cancel, source/revision kontrola a batch mutation epoch
- [x] Statické overenie mechanizmov a relevantných existujúcich testov
- [x] Záverečné pokrytie a limity

## Nálezy

### M1 — P1: Druhá session prepíše jediný recovery checkpoint prvej session

- **Primárne miesto:** `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs:1018-1037`.
- **Volajúci:** `C:/Sources8/Ketchup/crates/ketchup-application/src/session.rs:303-327`.
- **Mechanizmus:** zápis checkpointu overuje iba identitu primárneho súboru. Neskontroluje existujúci `.work-recovery` ani identitu predchádzajúceho checkpointu. Lock serializuje zápisy, ale nezabráni ich vzájomnému prepísaniu. Session síce pozná `work_recovery_identity`, pri zápise ju neposiela; používa sa len pri čistení.
- **Konkrétny scenár:** A a B otvoria rovnaký uložený dokument pred prvým editom. A vykoná edit, potom B vykoná odlišný edit. Obe operácie uspejú, lebo primárny súbor zostáva nezmenený. B atomicky nahradí recovery A. Po páde oboch procesov zostane iba vetva B; neuložená vetva A nie je obnoviteľná. Nevyžaduje sa súbežnosť jednotlivých systémových volaní, stačia sekvenčné edity dvoch session.
- **Dôkaz:** priamy tok `read_native_document_identity(path) == base_identity` → `write_atomic(work_recovery_path(path), bytes)` bez compare existujúceho checkpointu. Existujúci test `C:/Sources8/Ketchup/crates/ketchup-application/tests/document_session.rs:545-573` vytvára dve session a druhý checkpoint, ale overuje iba ochranu novšieho checkpointu pri Save As cleanup, nie zachovanie prvej vetvy. Loader na `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs:1213-1221` preferuje jediný aktuálny work-recovery.
- **Závažnosť:** P1 — tichá strata už checkpointovaných neuložených zmien pri podporovanom použití viacerých session.
- **Fix smer:** pod tým istým lockom vyžadovať očakávanú identitu checkpointu (vrátane očakávanej absencie), alebo uchovávať recovery vetvy oddelene pre session. Konflikt musí zachovať oba obsahy; nestačí zamykanie primárneho súboru.
- **Overenie:** statika a čítanie existujúceho testu, bez vykonanej reprodukcie.

### M2 — P2: Odmietnutý insert_extension už nahradil pôvodné dáta

- **Primárne miesto:** `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs:123-127`.
- **Mechanizmus:** `BTreeMap::insert` najprv nahradí hodnotu; až potom `is_some()` vyvolá `DuplicateContainerEntry`. Pôvodný extension payload aj `required` príznak sú stratené napriek výsledku Err.
- **Konkrétny scenár:** vložiť extension `(org.example.audit, data.bin)` s obsahom A; druhý insert rovnakého kľúča s obsahom B odmietnuť a pokračovať s pôvodným kontajnerom. Nasledujúci save zapíše B, nie A. Zmena required môže navyše zmeniť load disposition.
- **Dôkaz:** mutácia na riadku 125 pred chybou na 126; serializácia číta aktuálne hodnoty mapy na `persistence.rs:598-604`. `set_extension` je samostatná explicitná replacement operácia na `persistence.rs:131-134`, takže odmietnutý insert nemá byť replacement. Čítané extension roundtrip testy sú v `C:/Sources8/Ketchup/crates/ketchup-model/tests/native_persistence.rs`; vyhľadanie insert_extension neukázalo kontrolu nezmeneného kontajnera po duplicitnom inserte.
- **Závažnosť:** P2 — porušenie failure atomicity verejného ContainerData API s dopadom na následné uloženie. Netvrdím, že parser po Err publikuje čiastočne načítaný kontajner.
- **Fix smer:** použiť entry API alebo skontrolovať existenciu pred vložením; duplicitná vetva musí nechať pôvodný entry nedotknutý. Test má porovnať pôvodné bytes aj required po Err a po roundtripe.
- **Overenie:** statika, bez vykonanej reprodukcie.

### M3 — P1: Group conversion môže zacykliť apply_batch skôr, než sa vykoná kontrola cyklov

- **Primárne miesto:** `C:/Sources8/Ketchup/crates/ketchup-model/src/document/group_conversion.rs:10-20` a `:24-36`.
- **Vstupná cesta:** `C:/Sources8/Ketchup/crates/ketchup-model/src/document/store.rs:2732-2752`, `:2785-2802`; kontrola produktu až `:2864`.
- **Mechanizmus:** CreateGroup/SetGroupParent upravia kandidáta bez okamžitej kontroly cyklu. Neskorší ConvertGroupToComponent v tom istom batchi volá `descendant_groups` (`group_conversion.rs:187`), ktorý pre každú skupinu prechádza parent reťazec cez `group_is_descendant`. Táto slučka nemá visited množinu ani limit. Spolieha sa na už validný graf, hoci dostáva ešte nevalidovaný medzistav batchu.
- **Konkrétny scenár:** v novom DocumentStore poslať jeden batch: CreateGroup(1, parent=None), CreateGroup(2, parent=3), CreateGroup(3, parent=2), ConvertGroupToComponent(group_id=1, new_definition_id=1, new_occurrence_id=1, component_name="Converted"). Všetky mená sú neprázdne a transformy identity. Po prvej skupine prejde filter na group 2; cursor opakuje 2 → 3 → 2, nikdy sa nerovná konvertovanému root 1 ani None. Na definíciách a occurrences zatiaľ nie sú kolízie ID.
- **Dôkaz:** CreateGroup kontroluje ID, meno a transform, nie parent graf. `descendant_groups` iteruje všetky skupiny, teda aj cyklus mimo konvertovaného rootu. Konverzia beží vo vnútri command slučky; záverečný `validate_product_change` a jeho `validate_groups` (`C:/Sources8/Ketchup/crates/ketchup-model/src/document/product_validation.rs:55-56`) sa nedosiahnu. Aj `DocumentStore::validate_batch` používa vykonanie preview kandidáta (`store.rs:764-773`), takže názov „validate“ pred týmto hangom nechráni.
- **Závažnosť:** P1 — malý neplatný vstup nevráti rejection, ale neukončí synchrónnu mutáciu/validáciu; môže zablokovať volajúci thread. Nejde o zistený čiastočný commit, ale o chýbajúce ukončenie transakčnej cesty.
- **Fix smer:** traversal musí byť bezpečný aj na medzistave: visited/limit a štruktúrovaná chyba cyklu, alebo explicitné overenie group grafu pred konverziou. Zachovať možnosť validných forward references v batchi. Reprodukciu hlavný reviewer musí spustiť izolovane s timeoutom, nie ako nekonečný test v hlavnom procese.
- **Overenie:** presný statický trace, žiadny zámerne zavesený proces ani spustený test.

### M4 — P2: Chyba directory sync po publikovaní checkpointu spôsobí rollback iba pamäte

- **Primárne miesto:** `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs:962-965` v spojení s `:1037-1038`.
- **Volajúci:** `C:/Sources8/Ketchup/crates/ketchup-application/src/session.rs:383-396`; rollback `C:/Sources8/Ketchup/crates/ketchup-model/src/document/store.rs:33-39`.
- **Mechanizmus:** temporary.persist už nahradí cieľový súbor. Následný sync parent directory môže vrátiť Err. Persistence neodlíši „nepublikované“ od „publikované, nepotvrdená durability“. Session považuje akýkoľvek Err z checkpoint finalizeru za dôvod vrátiť celú históriu a canonical state späť, zatiaľ čo nový `.work-recovery` už zostal publikovaný a jeho identita sa session nepriradí.
- **Konkrétny scenár:** na Unix systéme uložený dokument v stave S0; edit na S1 vytvorí nový work-recovery, rename/persist uspeje, ale fsync adresára zlyhá. Edit vráti Err a živá session sa vráti na S0. Ukončiť proces bez ďalšieho úspešného checkpointu; pri novom Open loader uprednostní work-recovery S1 pred primárnym S0. Odmietnutá operácia sa tak po Open objaví ako vykonaná. Rovnaký mechanizmus sa týka undo/redo, ktoré používajú ten istý finalizer.
- **Dôkaz:** existujúce testy `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs:2040-2074` výslovne asertujú Err **a súčasne** nové bytes v cieľovom súbore po injektovanej sync chybe. Tieto testy dokazujú zamýšľané low-level správanie zo zdroja; chýbajúca väzba je rollback session oproti výsledku filesystemu. Open preferencia je `persistence.rs:1213-1221`.
- **Závažnosť:** P2 — podmienené I/O chybou na Unix; logická nekonzistencia live/disk a oživenie odmietnutej operácie. V aktuálnom Windows prostredí je sync_parent_directory no-op (`persistence.rs:974-978`), takže tento konkrétny trigger tam nenastáva.
- **Fix smer:** prenášať stav publikovania oddelene od durability chyby a zvoliť konzistentnú session politiku; nevracať slepo canonical state späť pri už publikovanom checkpointe. Alternatívne musí rollback checkpointu bezpečne obnoviť predchádzajúci obsah pod lockom. Zachovať hlásenie fsync chyby, neignorovať ju.
- **Overenie:** statický cross-layer dôkaz a čítanie testov s fault injection; bez vykonanej reprodukcie alebo I/O fault injection.

### M5 — P2: Rollback odmieta revízie s odlišným zdrojom programu, ale rovnakou geometriou

- **Primárne miesto:** `C:/Sources8/Ketchup/crates/ketchup-model/src/document/store.rs:710-711`.
- **Súvisiaci stav:** source-only revízia `store.rs:183-218`, kopírovanie source pri úspešnom rollbacku `store.rs:734`.
- **Mechanizmus:** NoOpRollback porovnáva iba snapshot canonical_digest. `replace_rule_program_source` vytvára plnohodnotnú revíziu a ukladá iný rule_program, ale zámerne zdieľa ten istý ProductModel Arc (`store.rs:203-205`), teda aj rovnaký canonical digest. Zdroj programu žije mimo snapshot product. Rozdiel v source/overrides sa v no-op porovnaní ignoruje.
- **Konkrétny scenár:** vytvoriť program-owned revíziu R1 so zdrojom A cez apply_batch + bind_rule_program (alebo session apply_rule_commands_with_source). Vykonať replace_rule_program_source so zdrojom B, napríklad s upraveným komentárom, čím vznikne R2 bez zmeny geometrie. Zavolať rollback_to_revision s aktuálnym revision/digest R2 a target R1. Dostane NoOpRollback namiesto obnovy A. Bežné Undo funguje; nefunkčná je explicitná rollback operácia, preto to nezamieňam za úplne pokazené Undo.
- **Dôkaz:** rovnaký produkt Arc v oboch revíziách, digest cache patrí produktu (`C:/Sources8/Ketchup/crates/ketchup-model/src/document/snapshot.rs:350-355`), no-op guard porovnáva iba tento digest. Históriový digest pritom source správne zahrňuje (`store.rs:514-516`) a úspešná rollback vetva source explicitne obnovuje. Zdroj je súčasťou perzistentnej histórie (`C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs:474-486`).
- **Závažnosť:** P2 — platný cieľ histórie nemožno obnoviť rollback API, hoci obsah programu je odlišný a trvalo uložený.
- **Fix smer:** no-op rozhodnutie musí okrem product obsahu porovnať rule_program, vrátane overrides; pridať source-only rollback test a save/open overenie obnoveného zdroja. Nepridávať nový digest.
- **Overenie:** statika, bez vykonanej reprodukcie.

## Pokrytie a negatívne zistenia

| Oblasť | Konkrétne skontrolované cesty | Výsledok / limit |
|---|---|---|
| Atomicita | `document/store.rs`: kandidát produktu, final validation, push_revision, try_canonical_transaction; `session.rs`: apply, source binding, recovery finalizer | Hlavná canonical mutácia publikuje až po validácii. M2 je kontajnerový vedľajší stav, M3 neukončenie, M4 cross-layer rollback. |
| Undo/redo | cursor, branch truncation, history limit, next_revision_id, rollback_to_revision, source-only revision | Bežné undo/redo obnovujú snapshot a menia mutation_epoch; nový edit odrezáva redo. M5 sa týka iba explicitného rollbacku. |
| Save/open | container encoding, history records/decoder, checksums, conditional/no-clobber save, recovery write/load/cleanup, session save commit point | Decoder kontroluje checksumy, monotónnosť revízií, cursor, next revision a súlad current/history. M1 a M4 sú reálne medzery mimo týchto kontrol. |
| Nested/shared | `document/group_conversion.rs`, `snapshot.rs:746-1166`, `scene_query.rs`, `instance_path.rs`, occurrence command vetvy, `shared_change/replacement.rs:414-638`, application transform helpers | Projekcia skladá parent transformy a per-instance overrides; replacement explicitne odmieta nested definície. M3 sa týka nevalidovaného medzistavu. Kompletné clone-feature/remapping a všetky assembly solver vetvy neboli auditované. |
| Async revision | `evaluation_task.rs:152-297`, `session.rs:489-625`, `exact_product.rs` ExactProducerEvidenceContext, occurrence batch task | Publication kontroluje document/revision/digest a cancel pred materializáciou. Registrácia evidence je obalená transakciou. Batch task porovnáva aj mutation_epoch. Žiadny ďalší samostatne doložený stale-result nález. Návrat na totožný snapshot cez Undo/Redo sám osebe neuvádzam ako chybu, keď produkty geometricky opäť zodpovedajú snapshotu. |

Relevantné testy čítané alebo cielene vyhľadané: `C:/Sources8/Ketchup/crates/ketchup-application/tests/document_session.rs`, `C:/Sources8/Ketchup/crates/ketchup-application/tests/batch_task.rs`, `C:/Sources8/Ketchup/crates/ketchup-model/tests/native_persistence.rs`, unit test exact publication v `C:/Sources8/Ketchup/crates/ketchup-application/src/evaluation_task.rs`, unit testy save/fsync v `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs`. Vyhľadanie cyklových testov nie je dôkaz úplnej absencie regresie M3; konkrétna kombinácia cyklického medzistavu a conversion nebola vykonaná.

### Limity a odporúčané overenie

- Žiadny cargo build/test, worker beh, nové fixture ani zdrojové zmeny. Päť scenárov má overiť hlavný reviewer; M3 iba s procesovým timeoutom, M4 s fault injection.
- Neauditované: implementácia native worker procesu, GUI, protocol, ostatné crates, všetky exportéry/importéry a kompletná legacy migrácia.
- Samotná existencia epoch bez jej použitia vo všetkých API nie je automaticky nález; vyžadovaný je konkrétny nesprávny výsledok.
- Nepredkladám historické závady ani iba hypotetické naming/style výhrady.

## Checkpoint implementácie CR-04/05/07/08

Implementácia prebieha v povolenom persistence/session/rollback workstreame. CR-05 používa entry bez mutácie pri duplicite; CR-08 porovnáva aj rule_program. CR-04 pridáva compare-and-swap recovery identity a OS lock živého zapisovača, vrátane ochrany reopen. CR-07 nesie Published stav s pôvodnou I/O príčinou a zachová publikovaný stav session. Fault injection je vo spoločnej sync_parent_directory ceste pod existujúcim feature testing; Windows beh nie je Unix fsync test.

**Integračný follow-up pre hlavného agenta:** GUI `crates/ketchup-app/src/app/document.rs` má dva volania pôvodného recovery writer API (približne 385 a 1118). Pôvodné API teraz bezpečne vyžaduje absenciu checkpointu; opakované zápisy treba prepojiť na `_if_unchanged` s existujúcou work_recovery_identity a vlastníctvom WorkRecoveryLock. GUI je mimo povoleného rozsahu tohto workstreamu. Zatiaľ žiadny testový PASS netvrdím. **Blocker testov:** cargo opakovane končí pred kompiláciou na duplicitnej `[target.'cfg(unix)'.dependencies]` sekcii v súbežne upravovanom `crates/ketchup-mcp/Cargo.toml:21`; mimo povoleného rozsahu, neopravené týmto agentom. Logy bg31-18dabee5d836c674 a bg33-18dabf006194b8e4. Regresie pridané do native_persistence.rs a document_session.rs; ďalší krok je odstrániť manifest blocker vlastníkom MCP a spustiť cielené testy.

### Výsledok workstreamu

Implementované CR-04/05/07/08 v model/session vrstve; integračne **čiastočne dokončené**, kým hlavný agent nezapojí vyššie uvedené GUI recovery volania. Checkpoint nesmie byť použitý ako tvrdenie úplného workspace PASS.

Presné súbory tohto workstreamu:
- `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence.rs`
- `C:/Sources8/Ketchup/crates/ketchup-model/src/persistence/work_recovery.rs` (nový modul: OS ownership, bounded identity check, testing-only fsync injection)
- `C:/Sources8/Ketchup/crates/ketchup-model/src/document/store.rs` (iba podmienka NoOpRollback; súbežný fork_for_planning patrí inému agentovi)
- `C:/Sources8/Ketchup/crates/ketchup-application/src/session.rs`
- `C:/Sources8/Ketchup/crates/ketchup-model/tests/native_persistence.rs`
- `C:/Sources8/Ketchup/crates/ketchup-application/tests/document_session.rs`
- tento Markdown.

Úspešné cielené testy, Windows, spoločný target-dir:
- `cargo test --locked -p ketchup-model --lib persistence:: -- --test-threads=2`: **12 PASS**.
- `cargo test --locked -p ketchup-model --test integration native_persistence:: -- --test-threads=2`: **33 PASS**, vrátane bytes/required roundtrip pri odmietnutej extension a source/overrides rollback, Undo/Redo, save/load, no-op a overrides-only variantu.
- `cargo test --locked -p ketchup-application --test integration document_session::recovery_conflict -- --test-threads=2`: **1 PASS**; konflikt nemení druhú session ani recovery, reopen a stale Save neukradnú vetvu, Save As uchová obe vetvy, po drop vlastníka sa recovery obnoví.
- Rovnaký session príkaz s filtrami `document_session::published_checkpoint`, `document_session::checkpoint_failures`, `document_session::stale_session`: každý **1 PASS**. Published fault test overuje apply/undo/redo a obsah po Open, pôvodné pre-publish chyby stále rollbacknú históriu.
- `git diff --check`, scoped rustfmt, `check_test_hooks.py`, `check_error_hygiene.py`: **PASS**.

Prechodné blokery: duplicitnú Unix dependencies sekciu MCP odstránil jej vlastník; potom sa Cargo dostalo ku kompilácii. Dve testové Debug-bound chyby boli opravené pred zelenými behmi. `cargo clippy --locked -p ketchup-model -p ketchup-application --all-targets -- -D warnings` zastavil **cudzí** `C:/Sources8/Ketchup/crates/ketchup-program/src/execution_budget.rs:33` (`manual_is_multiple_of`); tento súbor nebol upravený týmto workstreamom.

Limity a follow-up: plná workspace sada, GUI regresie a natívny Unix fsync neboli spustené. Fault injection ide cez skutočnú spoločnú sync_parent_directory vetvu, na Windows nedokazuje správanie Unix kernel/filesystem fsync. OS ownership lock súbor zostáva na disku zámerne, vlastníctvo drží otvorený OS handle a uvoľní ho Drop/pád procesu. API bez expected recovery už nemôže ticho nahradiť existujúci checkpoint; GUI musí migrovať na `_if_unchanged` a dodržať ownership lock, vrátane reopen a Save. Žiadny commit/push/stash, používateľské modely ani cudzie procesy/GUI neboli dotknuté.

## GUI integračný checkpoint CR-04/07

Oba GUI recovery writery migrované na CAS s identitou a FileState drží WorkRecoveryLock od prvého checkpointu po Save/drop. Open získava ownership pred načítaním a nevymazáva checkpoint pri odmietnutom reopen. Save kontroluje ownership a recovery identitu aj pre iný cieľ. Published chyba zachová model, container, identitu aj publication callback pred vrátením hlásenej chyby; Save As pri publikovanom cieli zachová binding. Pridané headless regresie dve okná/Save As/pád a spoločná sync fault injection pre edit/Undo/Redo/Save As. Recovery fault fixtures obnovujú pôvodné bytes pred retry, nie obchádzajú CAS. Whitespace, error-hygiene a test-hook check PASS. Dva prvé testové pokusy zastavené na súbežnej duplicite dot2/cross2 v ketchup-geometry/src/linalg.rs (184/259, 189/266); nejde zatiaľ o definitívny blocker a validácia pokračuje. Žiadny nový testový PASS netvrdím. Commands caller mení len propagáciu publication výsledku, nie readiness politiku.

### GUI odovzdanie a aktuálna validácia

Implementácia odovzdaná bez tvrdenia testového PASS: tri behy `cargo test --locked -p ketchup-app --test integration file_workflow:: -- --test-threads=2` aj `cargo clippy --locked -p ketchup-app --all-targets -- -D warnings` skončili pred kompiláciou GUI na stále prítomnej duplicite `dot2`/`cross2` v súbežne upravovanom `C:/Sources8/Ketchup/crates/ketchup-geometry/src/linalg.rs`. Geometria nebola týmto agentom upravovaná. Po stabilizácii treba zopakovať file_workflow a regression filtre `work_recovery_checkpoint_fails` (feature_history_ui/live_bridge_transactions), potom Clippy. Workspace testy a natívny Unix fsync neoverené.

Záverečné scoped rustfmt `--check`, `git diff --check`, `check_error_hygiene.py`, `check_test_hooks.py`: PASS. Sedem zdrojových/testových súborov, +260/-71 (net +189); Markdown checkpoint navyše. Presný rozsah: `C:/Sources8/Ketchup/crates/ketchup-app/src/app/document.rs`, `C:/Sources8/Ketchup/crates/ketchup-app/src/app_state.rs`, `C:/Sources8/Ketchup/crates/ketchup-app/src/app/shell.rs`, `C:/Sources8/Ketchup/crates/ketchup-app/src/app/commands.rs`, `C:/Sources8/Ketchup/crates/ketchup-app/tests/file_workflow.rs`, `C:/Sources8/Ketchup/crates/ketchup-app/tests/feature_history_ui.rs`, `C:/Sources8/Ketchup/crates/ketchup-app/tests/live_bridge_transactions.rs`. Test pádu používa drop celého vlastníka; žiadne ručné obchádzanie locku. Bez živého GUI, commit/push/stash či zásahov do cudzích procesov. Zostáva validačný follow-up, preto integráciu zatiaľ neoznačovať za plne overenú.

## Priebežný denník

1. Dokument vytvorený hneď po HMOS/AGENTS checku s checklistom.
2. Overený baseline a čistota sledovaných súborov. Prečítané transaction/session/save a async publish cesty, batch task a vybrané testy.
3. Samostatne priebežne uložené M1 a M2 pred ďalšou kontrolou.
4. Doplnené M3–M5, presné scenáre, cross-layer dôkazy, pokrytie a limity. Všetky nálezy explicitne označené ako statika.
