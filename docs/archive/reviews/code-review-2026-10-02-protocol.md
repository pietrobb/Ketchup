# Nezávislý review protokolu – 2026-10-02

## Mandát a priebeh
- Overený HEAD: `e92f4a73844b185570dfbb49a807ab2001ecd845`. Na začiatku bez modifikovaných tracked súborov; existujú cudzie untracked review dokumenty.
- Rozsah: ketchup-mcp, app live_bridge, assistant/auth, Python SDK, skills; lifecycle IPC/transport, hranice a súbory, timeout atomicita, schema drift.
- Najprv prečítaná HMOS SQLite pamäť (read-only) a koreňový AGENTS.md. Pamäť je historický kontext, nie dôkaz nálezu. Pod crates/sdk/skills nebol nájdený vnorený AGENTS.md.
- Bez opráv zdrojov, zmien hlavného review dokumentu, commit/push/stash operácií, Cargo build/test a živých OAuth/provider volaní.
- Dokument bol založený pred kontrolou implementácie a priebežne dopĺňaný. Nižšie sú staticky potvrdené nálezy; runtime reprodukcie ani Cargo testy sa nespúšťali.
- P1 = vysoká priorita (bezpečnostná hranica, neobmedzené zablokovanie GUI); P2 = stredná priorita (nesprávne smerovanie, porušenie kontraktu, dostupnosť pri konkrétnom vstupe).

## Súhrn

| ID | Priorita | Nález |
|---|---|---|
| PR-01 | P1 | Automatický TCP attach nerozlišuje lokálnych OS používateľov |
| PR-02 | P1 | ApplyProgram beží neobmedzene na UI vlákne a po zrušení môže stále publikovať |
| PR-03 | P2 | Niektoré live metódy prijímajú, ale ignorujú `expected` stamp |
| PR-04 | P2 | MCP open_window sa môže pripojiť k inému súbežne otvorenému oknu |
| PR-05 | P2 | MCP source_path načítava celý súbor pred uplatnením limitu rámca |

## Potvrdené nálezy

### PR-01 – P1: Automatický TCP attach nerozlišuje lokálnych používateľov
- **Miesto:** `C:/Sources8/Ketchup/crates/ketchup-app/src/live_bridge/consent.rs:405-426`, `:184-215`, `:550-590`; aktivácia `C:/Sources8/Ketchup/crates/ketchup-app/src/main.rs:152-157`.
- **Scenár:** Na počítači s viacerými lokálnymi účtami má používateľ A otvorený Kečup. Proces používateľa B nájde loopback port (registry nemusí vedieť čítať), pošle `{"version":1,"action":"attach","nonce":"<64 hex>"}` a dostane bridge adresu a token. Nie je potrebné poznať instance ID; server ho sám vráti. Nový attach zároveň odpojí doterajšieho klienta.
- **Dopad:** Iný OS používateľ získa čítanie aktuálneho modelu a mutačnú autoritu v procese A; môže tiež opakovane preberať spojenie. Ide o cross-user hranicu, nie o tvrdenie, že ľubovoľný proces rovnakého používateľa musí byť sandboxovaný.
- **Dôkaz a guardy volajúceho:** Native main zapína broker aj pri obyčajnom štarte. Prijatie spojenia kontroluje iba `peer.ip().is_loopback()`. AttachRequest obsahuje iba version/action/nonce; validácia nonce overuje tvar, nie dôkaz identity. UI `poll_live_consent` udeľuje prístup bez potvrdenia a `allow_live_consent` pošle platný token. Per-user discovery adresár chráni súbor, nie TCP port. Autentifikácia každého následného bridge rámca existuje (`C:/Sources8/Ketchup/crates/ketchup-app/src/live_bridge/transport.rs:306-315`), ale token vydá tento nechránený bootstrap. Potvrdenie Open v `consent.rs:226-240` nechráni query ani bežné mutácie. Klientská kontrola instance ID v `C:/Sources8/Ketchup/crates/ketchup-mcp/src/discovery.rs:153-157` neautentifikuje žiadateľa na serveri.
- **Odporúčanie:** Použiť lokálny transport s OS autentifikáciou a ACL (named pipe/Unix socket), prípadne vyžadovať pre attach tajomstvo prenesené zabezpečeným kanálom alebo skutočné explicitné potvrdenie. Same-user automatický reconnect môže zostať zachovaný.
- **Overenie:** Statický end-to-end trace; žiadne živé pripojenie k používateľovmu GUI ani cross-user exploit nebol spustený.

### PR-02 – P1: ApplyProgram nemá zrušiteľné plánovanie a pred neskorým commitom neobnovuje autoritu
- **Miesto:** `C:/Sources8/Ketchup/crates/ketchup-app/src/live_bridge/program_check.rs:85-97`; `C:/Sources8/Ketchup/crates/ketchup-app/src/program_edit.rs:73-136`; `C:/Sources8/Ketchup/crates/ketchup-program/src/eval.rs:2394-2399`.
- **Scenár:** Klient odošle malý platný Starlark zdroj, ktorý pred normálnym vytvorením geometrie vykoná dlhý výpočet (napr. vnorené slučky nad `range(1000000000)` vo funkcii). Rámec je pod 32 KiB. Počas výpočtu sa klient odpojí alebo vyprší 30-sekundové čakanie transportu. Výpočet pokračuje na UI vlákne; keď skončí, publikovanie prebehne aj so zrušeným requestom.
- **Dopad:** Krátky vstup môže na neobmedzene dlhý čas zablokovať GUI. Transportové zrušenie nezastaví plánovanie a nie je prekážkou neskoršej mutácie, hoci pri queued autorite ho implementácia používa ako revokáciu. Nejde iba o bežnú stratu odpovede po už dokončenom commite.
- **Dôkaz a guardy volajúceho:** `C:/Sources8/Ketchup/crates/ketchup-app/src/live_bridge.rs:919-923` volá `start_queued_apply_program` priamo z UI pollera; tá na `program_check.rs:143` synchrónne volá `apply_program`. Jediný cancellation check je pred `apply_program_source`. Tá nemá cancellation parameter; po `plan_rule_program` priamo vykonáva SourceOnly alebo commit vetvu. `C:/Sources8/Ketchup/crates/ketchup-application/src/rule_program.rs:194-207` najprv vyhodnocuje program; interpreter má iba print handler, bez execution budgetu/cancellation callbacku. Limit `MAX_PARTS = 20000` (`eval.rs:39-40`) neobmedzuje slučku, ktorá negeneruje diely. Transport na `C:/Sources8/Ketchup/crates/ketchup-app/src/live_bridge/transport.rs:145-150,277-281,355-365` nastaví cancellation po timeoute/EOF, ale plánovač ho už nečíta. Asynchrónny worker a 20 s exact timeout v `program_check.rs:163-174` začínajú až PO publikovaní; tento problém nechránia. MCP 45 s wait (`C:/Sources8/Ketchup/crates/ketchup-mcp/src/tools.rs:18-20,184`) tiež GUI výpočet nezruší.
- **Odporúčanie:** Vyhodnocovať a plánovať kandidáta mimo UI so skutočným časovým/inštrukčným budgetom (aj pre program bez dielov); tesne pred publikovaním na UI znovu overiť cancellation a aktuálny dokument. Do mutačnej fázy nevstúpiť po zrušení. Test má deterministicky pozastaviť plánovanie, zrušiť request a overiť nezmenený dokument aj Undo históriu.
- **Overenie:** Statická cesta od socketu po interpreter a commit; zámerne nebol spustený dlhý program, ktorý by zablokoval GUI.

### PR-03 – P2: `expected` je v niektorých metódach potichu ignorované
- **Miesto:** `C:/Sources8/Ketchup/crates/ketchup-app/src/live_bridge.rs:2019-2042` (EditContext/Query/Detail), `:2049-2052` (WorksetStatus), `:2088-2094` (BatchJobStatus), `:2327-2343` (View).
- **Scenár:** Klient si uloží stamp a entity ID z dokumentu A. Používateľ medzitým otvorí dokument B alebo zmení revíziu. Klient pošle napr. `detail` alebo `edit_context` s pôvodným `expected`. Server namiesto `stale_document` vykoná query nad aktuálnym snapshotom; pri zhodných číselných ID môže vrátiť inú entitu. `view` s rovnakým stale guardom tiež zmení kameru.
- **Dopad:** Porušenie výslovne ponúkaného guardu a strata konzistencie viac-krokovej inšpekcie. Nález netvrdí obídenie následného commit guardu: mutačné commit cesty stamp overujú.
- **Dôkaz a guardy volajúceho:** Request DTO má `expected` (`live_bridge.rs:124-138,150-159,248-252`), vetvy ho však zahodia cez `..`. `execute_authorized` na `:2001-2003` globálne overuje iba cancellation. UI poller na `:833-928` invaliduje cache pri zmene stampu, ale neporovnáva klientovo `expected`. ModelQuery dostáva aktuálny snapshot, nie klientov stamp, preto túto kontrolu nemôže doplniť. Workset/job majú vlastné kontroly handle/stavu, ale tie nie sú všeobecným porovnaním `expected`. MCP ho iba forwarduje (`C:/Sources8/Ketchup/crates/ketchup-mcp/src/tools.rs:194-233,244-247,471-488`) a schéma sľubuje stale rejection (`C:/Sources8/Ketchup/crates/ketchup-mcp/src/schema.rs:22-23,87,106,144,155`).
- **Odporúčanie:** Aplikovať rovnaký `Self::guard(app, &expected)?` pre každú metódu, ktorá parameter prijíma, ideálne spoločnou dispatch kontrolou; pridať test query/detail/edit_context/view s pôvodným stampom po výmene dokumentu. Alternatívou je odstrániť parameter a sľub z príslušných schém, nie ho ticho ignorovať.
- **Overenie:** Statické porovnanie publikovanej MCP schémy, DTO, routera a dispatchera; ide o potvrdený kontraktový drift.

### PR-04 – P2: open_window identifikuje nové okno iba rozdielom zoznamov
- **Miesto:** `C:/Sources8/Ketchup/crates/ketchup-mcp/src/tools.rs:361-388`.
- **Scenár:** MCP spustí okno pre dokument A. Po prvom zozname `before` používateľ alebo druhý MCP klient spustí aj okno B. Ak B zverejní registry skôr než A, tento MCP sa pripojí k B a vráti úspech `open_window`. Pri súčasnej dostupnosti oboch vyberie prvé podľa zoznamu, nie podľa spusteného procesu.
- **Dopad:** Nasledujúce implicitne smerované editácie môžu zasiahnuť nesprávne používateľovo okno; attach navyše môže odpojiť jeho existujúceho klienta. Otvorenie požadovanej cesty sa nesprávne prezentuje ako úspešné pripojenie k novému oknu.
- **Dôkaz a guardy volajúceho:** Jediná podmienka je `!before.contains(&window.instance_id)`. Child handle sa používa len na `try_wait`, nie na koreláciu identity. `C:/Sources8/Ketchup/crates/ketchup-mcp/src/discovery.rs:24-31,74,103-117` má instance ID, dokument a adresu, ale nie PID/launch identity. `attach` správne overí ID vybraného okna, čím však iba potvrdí nesprávny výber; status následne rovnako odpovie z B. Kontrola absolútnej existujúcej document_path na `tools.rs:353-359` nevie korelovať výsledné okno.
- **Odporúčanie:** Pre open_window použiť readiness kanál priamo od spusteného child procesu s jeho konkrétnou adresou/instance ID (existujúci trusted launcher bootstrap poskytuje vhodný smer), nie rozdiel globálnych discovery zoznamov. Testovať dva súbežné štarty s opačným poradím registrácie.
- **Overenie:** Statický prechod oboma procesovými a discovery cestami; konkurenčné GUI štarty neboli spustené.

### PR-05 – P2: source_path obchádza bounded vstup pred načítaním súboru
- **Miesto:** `C:/Sources8/Ketchup/crates/ketchup-mcp/src/tools.rs:169-182`.
- **Scenár:** `program action=apply source_path=...` omylom odkáže na veľký textový log/export namiesto malého .star súboru. MCP načíta celý súbor do pamäte; až po vytvorení zdroja a serializovaného envelope ho bridge môže odmietnuť ako väčší než 32 KiB. Na Unix platforme cesta k nekonečnému/čakajúcemu špeciálnemu súboru môže blokovať ešte pred socketovým timeoutom.
- **Dopad:** Nepotrebná veľká alokácia/OOM alebo zablokovanie jediného synchrónneho MCP servera; transportový limit ani timeout nechráni fázu čítania súboru.
- **Dôkaz a guardy volajúceho:** `take_text` (`tools.rs:550-558`) overí iba typ/prázdny string; `server.rs:49-54` iba objekt argumentov. Následné `std::fs::read_to_string` nemá veľkostný bound ani kontrolu regular file. `C:/Sources8/Ketchup/crates/ketchup-mcp/src/bridge.rs:61-66` limit kontroluje až po serializácii načítaného obsahu. Schéma uvádza absolútny .star súbor a približný 32 KB limit (`C:/Sources8/Ketchup/crates/ketchup-mcp/src/schema.rs:58-59`), ale server JSON schému argumentov nevaliduje a tento opis nie je guard.
- **Odporúčanie:** Pred čítaním vyžadovať bežný súbor; čítať najviac limit+1 bajtov a odmietnuť nadlimitný obsah bez jeho ďalšej serializácie. Zachovať finálnu kontrolu envelope vrátane tokenu a JSON escaping overheadu. Pri podporovaných platformách riešiť aj otvorenie špeciálnych súborov tak, aby samotný open nemohol neobmedzene blokovať.
- **Overenie:** Statické sledovanie vstupu; veľký súbor ani špeciálne zariadenie neboli otvárané.

## Pokrytie a negatívne overenia

### MCP a live bridge
- Prečítané MCP server/bridge/discovery/tools/schema/build; relevantné testové prípady vyhľadané, nie spustené. Discovery obmedzuje počet registry záznamov a paralelizuje probe; kontroluje loopback reply adresy, nonce, instance ID a tvar tokenu.
- App transport/bootstrap/consent prečítané vrátane shutdown/disconnect a request queue. Hlavné live_bridge vetvy pre query, proposals, batch, ApplyAndVerify, ApplyProgram, Save/Open/View boli sledované cez volajúcich.
- Transport má autentifikáciu každého requestu, obmedzený počet spojení/frontu/rámce, spoločný absolútny deadline header+body a zápisu odpovede, odvolanie queued autority pri EOF/pipeliningu. Tieto existujúce guardy nie sú hlásené ako chýbajúce.
- MCP po transportovej chybe zahodí spojenie a vracia výslovné unknown-outcome upozornenie; automatický retry mutácie som nenašiel.
- ApplyAndVerify kontroluje deadline, cancellation, mutation epoch a selection pred publish; save po commite priznáva `committed_but_unsaved`, preto samotnú neatomicitu dokument+save nepovažujem za zatajený rollback bug.
- Open má consent/discard potvrdenia a cancellation check po dialógoch. SaveAs má kontrolu absolútnej cesty a deleguje cancellation do save workflow. Detailný audit nízkoúrovňovej persistence patrí mimo tento protokolový review.

### Assistant/auth
- Preskúmané `C:/Sources8/Ketchup/crates/ketchup-app/src/assistant_runtime.rs`, procesový lifecycle v `C:/Sources8/Ketchup/crates/ketchup-scheduler/src/assistant.rs`, časti typed sidecar DTO a Python provider/protocol implementácie.
- Public launch čistí environment, povoľuje iba vybraný API kľúč a explicitné proxy/CA nastavenia; runtime skripty overuje proti embedded obsahu, pri Python bootstrap opäť kontroluje bounded obsah/hash, používa `-I -B`. Na Windows guarded executable file zakazuje zdieľaný zápis/delete počas spawnu.
- Procesový klient overuje handshake a request ID, používa bounded response reader, deadline/cancellation a Windows job object. Python provider odpovede majú 8 MiB limit, reject redirect handler a strict JSON parser. Hoci urllib timeout nie je celkový deadline celej konverzácie, Rust host má vlastný timeout a proces ukončí; chýbajúci host timeout preto nehlásim.
- Aktuálny `C:/Sources8/Ketchup/crates/ketchup-app/Cargo.toml:13-15` má private-oauth v default features. Historický HMOS výrok o API-only defaulte teda nie je aktuálny dôkaz chyby; bez aktuálneho distribučného kontraktu ho nehlásim ako nález.
- V repozitári nie je samostatný ketchup-auth crate. Privátny OAuth provider je externý spustiteľný sidecar; jeho login/token refresh implementáciu tento review nemohol overiť proti HEAD Ketchup. Žiadne credentials ani živé OAuth/provider volania sa nepoužili.

### Python SDK a schema drift
- Prečítané `C:/Sources8/Ketchup/sdk/python/ketchup/client.py`, `C:/Sources8/Ketchup/sdk/python/ketchup_sdk/__init__.py`, `C:/Sources8/Ketchup/sdk/python/ketchup/manufacturing.py`; kontrolované hlavné väzby na headless batch receipt, typed CAD vstupy a assistant schémy.
- Headless klient má bounded stdout/stderr, bounded writer wait, kontrolu envelope/id/duplicitných JSON polí a zavretie session pri transportovej chybe. Document generation oddeľuje staré handles po new/open; batch receipt after obsahuje skutočne polia stampu, ktoré klient aktualizuje.
- Manufacturing export má explicitné confirmed=True, flat filenames vrátane ADS/device-name ochrany, kontrolu symlink/junction rodičov, stage a atomic no-replace publish; trusted parent concurrency je výslovne uvedený predpoklad. JAF bunky sú zapisované ako text, nie vzorce.
- CAD operation catalog sa generuje z Rust serde typov (`C:/Sources8/Ketchup/crates/ketchup-assistant/build.rs`), preto tvrdenie o úplne ručne duplikovanom katalógu by bolo nesprávne. MCP schéma má potvrdený sémantický drift `expected` (PR-03). Celá mnohotisícriadková Python/Rust CAD validácia nebola exhaustívne porovnaná po každej variante; ďalší konkrétny SDK schema mismatch nie je tvrdený.

### Skills – dôležité oddelenie pôvodu
- `git ls-files skills sdk/python` neuvádza žiadny skills súbor. `git check-ignore` potvrdil ignorovanie `C:/Sources8/Ketchup/skills/mcp_bridge.py`, `C:/Sources8/Ketchup/skills/spawn_instance.py` a `C:/Sources8/Ketchup/skills/create_skill.py`.
- Tieto tri lokálne súbory boli prečítané, ale nie sú súčasťou reviewovaného HEAD. MCP wrapper importuje externý `C:/Sources8/Supervisor/skills/mcp_bridge.py`; SpawnInstance závisí od externého Supervisor loadera/HMOS. Externý loader a jeho guardy nie sú v tomto repozitári, preto z týchto skills nevyvodzujem bezpečnostný nález o Ketchup HEAD a žiadny skill som nespúšťal/importom neaktivoval.

## CR-13 implementačný checkpoint (2026-10-02)

Používateľ schválil opravu aj po triáži. Prečítané HMOS, AGENTS.md, review a verification; pracovný strom obsahuje súbežné cudzie zmeny. Návrh: náhodný bootstrap v registry súbore, pred zápisom chránený explicitnou user-only Windows DACL alebo Unix 0600, pri čítaní overiť vlastníka a práva cez otvorený handle. Attach overí bootstrap pred frontou a revokáciou; bez manuálneho tokenu a bez nového dialógu. MCP discovery/tools a program_check/program_edit neupravovať; klientsky patch sa odovzdá, ak prekročí izolovanú connect funkciu. Testy zatiaľ nespustené; multiuser runtime sa netvrdí.

### CR-13 checkpoint implementácie a odovzdania

- `consent.rs`: 256-bit náhodný bootstrap uložený v registry, kontrolovaný pred vložením do UI fronty; chýbajúci/nesprávny bootstrap vracia `rejected` bez vydania tokenu a bez revokácie. Metadata `list` zostávajú bez bootstrapu (neudeľujú autoritu). Rovnaký používateľ stále nevidí nový dialóg ani manuálny token.
- Nový zdieľaný `ketchup-mcp/src/local_auth.rs` + `local_auth_windows.rs`: atomické create-new. Windows explicitný owner=current SID, protected DACL s jediným allow ACE pre tento SID; Unix 0600/O_NOFOLLOW. Reader overuje otvorený handle (regular, vlastník, práva, Windows reparse/Unix hardlink odmietnutie), maximálne 513 bytes. Nepoužíva sa dodatočné sprísnenie verejne vytvoreného súboru — pre-open race by prezradil bootstrap. Prvý pokus s dodatočnou DACL zlyhal na AccessDenied a bol nahradený atomickým vytvorením.
- MCP discovery/tools NIE sú prepísané. Presný integračný patch je `C:/Sources8/Ketchup/docs/cr-13-discovery.patch`; aplikovať pomocou `git apply --recount`. Prenesie registry_path cez Window, pred attach načíta private bootstrap a overí instance/address, pošle bootstrap v exchange. `git apply --recount --check` PASS. Hlavný agent musí patch integrovať so súbežným CR-16 a aktualizovať jeho mock registry fixtures; bez patchu existujúci MCP attach správne fail-closed odmietne server. Python SDK podľa vyhľadávania tento broker protokol neimplementuje.
- Regresie v `live_bridge_bootstrap.rs` používajú private bootstrap, kontrolujú missing/wrong bootstrap a pokračovanie pôvodného aktívneho bridge spojenia, aj autorizovaný takeover/reconnect. Broker unit testy aktualizované na nový bootstrap.
- Runtime Windows OS hranica: samostatný offline manifest `C:/Sources8/Ketchup/.claude/review-2026-10-02/cr13-auth/Cargo.toml` priamo kompiluje PRODUKČNÝ `local_auth.rs` (nie kópiu): **3 passed, 0 failed**, Clippy all-targets `-D warnings` PASS. Test skutočne impersonuje restricted token s Everyone restricted SID a volá File::open: PermissionDenied; vlastník pred/po číta úspešne. Ďalšie testy overujú odmietnutie inherited ACL, prijatie private ACL, roundtrip a bound. Toto nie je druhý prihlásený OS účet ani multiuser exploit.
- Pokusy o workspace MCP/app testy a MCP Clippy blokovali súbežné nedokončené `ketchup-program/src/contact.rs`/`faces.rs` (najprv planar_rings, následne profile_outline). Logy `bg45-18dabf88d108d4fc`, `bg47-18dabfa00664e9d0`, `bg49-18dabfb14f697518`, `bg50-18dabfb7e08830b0`. Tieto behy NIE sú PASS a app wrong-bootstrap runtime regresia zatiaľ nie je potvrdená.
- Unix implementácia a cfg testy sú prítomné, na tomto Windows hoste NEBOLI spustené ani cross-kompilované. Unix musí ešte overiť 0600/owner/no-follow a reálny druhý UID; Windows tiež chýba test druhého skutočného účtu. Administrátor/root a škodlivý proces rovnakého používateľa nie sú touto hranicou izolované. Nepodporované OS fail closed.
- Bez GUI/OAuth, zásahov do cudzích procesov, commit/push/stash. Cargo.lock dopĺňa dve už existujúce dependency väzby (libc, windows-sys). Konečný stav CR-13 je **čiastočný**, čaká klientsku integráciu a app/platformové regresie, nie deklarácia úplného vyriešenia.

## Validácie a hranice záverov
- Overené presné HEAD, čistota tracked diffu a tracked/ignored pôvod SDK/skills. Súborové odkazy sú absolútne a riadky pochádzajú z aktuálneho checkoutu.
- Všetkých päť nálezov je založených na aktuálnom control/data flow vrátane volajúceho; navrhnuté runtime scenáre sú odporúčania na regresné testy, nie predstierané výsledky vykonaných testov.
- Nespustené Cargo build/test/clippy ani provider/OAuth sieťové volania. Hlavný reviewer vykonáva testy. GUI exploity ani zámerne blokujúce programy sa nespúšťali.
- Bez zmien zdrojov, commitu/pushu/stash operácií a bez úprav `C:/Sources8/Ketchup/docs/code-review-2026-10-02.md`.
