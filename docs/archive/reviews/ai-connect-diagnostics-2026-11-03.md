# Pripojenie AI — diagnostika (#2687, krok 1), 2026-11-03

Stav kódu: `origin/main` 22f1534c (pred PR #15). Riadky sedia na tento commit.
Overené na tomto PC: Claude Code CLI `C:\Users\peter\.local\bin\claude.exe`, Codex CLI nie je,
Claude Desktop nie je, bežia 3 okná Kečupu, v registri okien je 48 súborov.

Formát: **súbor:riadok** — príznak → návrh opravy.

## Priorita a poradie opráv (po merge #15, worktree z main)

P1 = bez toho sa človek nepripojí alebo AI zamrzne; P2 = pripojí sa, ale nevie, čo sa deje / zlyhá
neskôr; P3 = okrajové. Poradie = poradie commitov.

| Poradie | Body | Priorita | Prečo práve tu |
|---|---|---|---|
| 1 | 12, 13 | P1 | MECH-1: server neodpovie na `ping` ani zrušenie počas čakania na okno. RED test už je; oprava je len v `ketchup-mcp`, nezávislá od UI. |
| 2 | 1, 2, 18 | P1 | Tlačidlo Pripojiť vždy a pre všetkých klientov (Claude Code, Codex, Claude Desktop, Supervisor) zápisom do ich súboru. Supervisor dostane cestu k **nainštalovanému** Kečupu a pri ďalšom Pripojiť sa prepíše, ak sa zmenila. |
| 3 | 3, 8, 9, 4, 10 | P1 | Očíslovaný zoznam krokov so ✓ podľa skutočného stavu; krok „zatvor a otvor AI“; posledný krok „AI je pripojená ✓“; stále viditeľné „Pripojená AI: <meno>“ + Odpojiť (meno z `clientInfo`); preč okienko „identita neoverená“. |
| 4 | 15, 16a | P1 | Nový klient prevezme okno po zaseknutom bez rady zvonku; starý dostane `applied: true/false`. |
| 5 | 16 | P2 | Pri viacerých oknách `connect` bez id vyberie okno, z ktorého sa klikalo Pripojiť. |
| 6 | 17 | P2 | Čistenie mŕtvych záznamov v registri okien (48 súborov, 3 živé). |
| 7 | 19 | P2 | Chýbajúci worker odmietnuť hneď pri štarte/attach s hláškou, nie až pri programe. |
| 8 | 14 | P2 | Heartbeat okna počas úlohy; mŕtve okno = `window_not_responding` do N s, nie 15 min. |
| 9 | 5 | P2 | Zápis konfigurácie mimo UI vlákna (po bode 2 je to väčšinou zápis súboru, CLI len doplnok). |
| 10 | 7 | P2 | Projektový záznam so starou cestou v `.claude.json` / `.mcp.json` hlásiť v paneli. |
| 11 | 11 | P2 | Nápadná výzva pri povolení pluginu / zahodení neuložených; jasná chyba klientovi. |
| 12 | 6 | P3 | Bez `USERPROFILE` napísať prečo chýbajú riadky klientov. |
| — | 20 | mimo | `mcp_bridge.py` je repo Supervisor, nie Kečup — podľa operátora sa nerieši. Strana Kečupu je bod 1 (12, 13). |

## A. Panel „Pripojiť AI“ (čo vidí používateľ)

1. **connect_ai.rs:62-66, mcp_setup.rs:151-153, 164-165** — keď `claude`/`codex` nie je na PATH
   okna, riadok je „na tomto počítači nie je“ a nemá tlačidlo; ostanú len tri „Kopírovať“.
   Presne prípad manželky. PATH okna je PATH z času prihlásenia (CLI nainštalované neskôr ho nemá).
   → Tlačidlo **Pripojiť** vždy; konfiguráciu zapisovať priamo do súboru klienta (JSON/TOML), CLI len ako
   doplnok; hľadať CLI aj v známych miestach (`%USERPROFILE%\.local\bin`, `%APPDATA%\npm`).
2. **mcp_setup.rs:20** — klienti sú len Claude Code a Codex. Chýba Claude Desktop
   (`%APPDATA%\Claude\claude_desktop_config.json`), ktorý má bežný používateľ najčastejšie,
   a náš Supervisor (`<Supervisor>\skills\mcp_servers.json`).
   → Pridať oboch ako riadky s tlačidlom Pripojiť (zápis JSON `{"mcpServers":{"ketchup":{command,args}}}`).
3. **sk-SK.ftl:10** — popis sľubuje „tlačidlo Nastaviť nižšie“, ktoré pri bode 1 neexistuje; zvyšok je
   jeden odsek o MCP, „novej relácii“ a „nástroji connect“. → Očíslované kroky obyčajnou rečou
   so ✓/✗ podľa skutočného stavu (zapísané / klient sa ozval / pripojený).
4. **connect_ai.rs:15-20** — stav má len dve hodnoty (pripojený / nepripojený). Nevidno, či je
   konfigurácia zapísaná, či sa klient vôbec spustil, ani ktorý klient je pripojený.
   → Stav po krokoch; meno klienta z `initialize.clientInfo` poslať pri attach a zobraziť.
5. **connect_ai.rs:86-87 → mcp_setup.rs:181-198** — Nastaviť spúšťa `claude mcp add` synchrónne na UI
   vlákne (Node CLI, sekundy); okno zamrzne bez náznaku. → Úloha mimo UI vlákna s „Zapisujem…“.
6. **connect_ai.rs:48-50** — bez `USERPROFILE` sa riadky klientov ticho nezobrazia. → Napísať prečo.
7. **mcp_setup.rs:74-94** — stav Claude Code číta len `mcpServers` používateľa v `.claude.json`;
   projektový záznam (`projects[..].mcpServers`, `.mcp.json`) so starou cestou má prednosť, panel
   ukáže „spúšťa tento Kečup“ a klient spustí starý. → Čítať aj projektové záznamy a hlásiť ich.
8. **Po zápise treba reštartovať klienta** (Claude Code, Codex, Supervisor načítajú MCP len pri štarte;
   Supervisor `mcp_bridge.py:148-167`). Dnes to hovorí len veta v popise. → Krok „2. Zatvor a znova
   otvor AI“ s ✗, kým sa klient neozve.

## B. Okno (povolenie, odpojenie)

9. **consent.rs:184-230** — lokálny attach sa povolí hneď, okno sa nepýta. Operátorov krok
   „4. Tu klikni Povoliť“ by dnes nebolo kde kliknúť; staré rozhodnutie v HMOS hovorí o súhlase v okne.
   → **Rozhodnuté operátorom 2026-11-03:** automatické povolenie ostáva, výzva sa nevracia, krok 4 sa
   ruší. Namiesto neho stále viditeľné „Pripojená AI: <meno klienta>“ + tlačidlo „Odpojiť“ (stavový
   riadok alebo karta AI) a posledný krok zoznamu „AI je pripojená ✓“ podľa skutočného spojenia.
   Rieši sa spolu s bodmi 4 a 10.
10. **consent.rs:292-309, sk-SK.ftl:3-5** — po pripojení malé okienko vpravo dole
    „Lokálna aplikácia je pripojená… Jej identita zostáva neoverená.“ Neznie ako AI a straší.
    → „Claude Code je pripojený k tomuto oknu“ + Odpojiť; to isté v karte AI.
11. **plugin_consent.rs, ketchup-mcp lib.rs:75** — povolenie pluginu a „zahodiť neuložené?“ pri open
    čakajú v okne 60 s; keď ich používateľ prehliadne, klient dostane chybu. → Nápadný pás hore
    + zvuk/blikanie v taskbare; chyba klienta má povedať „používateľ musí kliknúť v okne Kečupu“.

## C. MCP server a spojenie (MECH-1)

12. **server.rs:31-49** — server spracúva stdin po jednej správe. Kým `tools/call` čaká na okno
    (do 35 s, program 50 s, otvorenie 125 s, výkresy 15 min), neodpovie na nič, ani na `ping`.
    Klient vidí mŕtvy server (MECH-1: „tools/call bez odpovede > 120 s“).
    **RED test:** `server::tests::a_ping_is_answered_while_a_tool_call_waits_for_the_window`
    (žiadna odpoveď do 5 s). → Čítať stdin vo vlastnom vlákne; `ping`, `tools/list` a nové volanie
    počas behu odpovedať hneď (nové volanie aspoň `busy` s tým, čo beží).
13. **server.rs:84** — notifikácie sa zahadzujú, aj `notifications/cancelled`. Keď klient (Codex má
    predvolený limit nástroja 60 s, overiť) volanie vzdá, server ďalej čaká a okno ďalej počíta.
    → Pri `cancelled` prerušiť čakanie a poslať oknu zrušenie úlohy.
14. **bridge.rs:153-156** — čakanie na odpoveď okna je jeden `read_exact` s timeoutom celej úlohy,
    bez známky života. Mŕtve okno = server visí až 15 min. → Okno posiela počas úlohy „stále bežím“
    (heartbeat) a server po N s bez neho hlási `window_not_responding`.
15. **tools.rs:536** — `windows` ukáže `used_by_another_client: true` bez rady; agent v Měchenicích
    nepokračoval, kým sa starý proces nezabil. Okno pritom prevzatie vie (consent.rs:222-228, test
    `a_second_client_takes_over_a_window_its_launcher_still_holds`). → Do odpovede pridať
    „connect ho prevezme; starý klient sa odpojí“ a pri nečinnom starom klientovi prevziať bez otázky.
16. **tools.rs:611-631** — viac otvorených okien (tu 3) → `connect` bez id skončí `choose_window`;
    všetky sa volajú „Bez názvu“, agent ani používateľ nevedia, ktoré. → Pripojiť z karty AI označí
    „toto okno“ v registri a `connect` bez id si vyberie to; v okne ukázať krátke meno okna.
16a. **live_bridge.rs:1522-1527 → tools.rs:765-777** — pri prevzatí okno zahodí most aj s rozbehnutou
    úlohou starého klienta a pošle len `superseded_by_another_client`; starý klient dostane „may or may not
    have been applied“ a musí čítať status. Okno pritom vie, či úloha zapísala revíziu. → V poslednom
    rámci poslať `applied: true/false` + stamp, nech agent nehádá.

## D. Register okien a inštalácie

17. **discovery.rs:27, 68-73** — register sa nečistí: 48 súborov, živé 3 procesy. Najviac 64 záznamov
    sa skúša, ostatné sa vynechajú → pri ďalších pádoch sa živé okno nemusí nájsť („no_window“).
    Mŕtve záznamy predlžujú každé `windows` na ~1 s (odmietnuté spojenie na Windows). → Mazať záznamy, ktorých proces nežije
    (PID v zázname) alebo ktoré nepreukážu bootstrap; limit počítať len zo živých.
18. **%LOCALAPPDATA%\Programs\Supervisor\skills\mcp_servers.json** — ukazuje na
    `Supervisor\Kecup\ketchup-app.exe` z 9. 10. 2026 bez `ketchup-program-worker` a
    `ketchup-exact-worker`; okno je Kečup 0.3.0. Iná verzia servera a okna, programy padnú na chýbajúcom
    workeri. → Supervisor nemá mať vlastnú kópiu; Pripojiť pre Supervisor zapíše cestu k
    nainštalovanému Kečupu; server pri attach porovná verziu s oknom a povie, ktorý reštartovať.
19. **stage.rs:29-34** — workery sú „voliteľné“: okno bez nich naštartuje a program zlyhá až neskôr.
    → `open_window`/attach odmietnuť hneď s hláškou „chýba ketchup-program-worker vedľa …exe“.
20. **Supervisor `skills\mcp_bridge.py:52-56, 100-113`** — volanie drží zámok a `readline()` nemá
    timeout; jedno zaseknuté volanie zablokuje všetky ďalšie navždy. **:155-159** — keď server
    nenaštartuje, chyba ide len na konzolu, nástroje ketchup v Supervisore ticho chýbajú
    („AI nevie, že má zavolať connect“). → Timeout na každé volanie a reštart procesu; chybu načítania
    ukázať používateľovi (oprava v repo Supervisor, nie Kečup).

## Testy

- RED (dnes padá): `ketchup-mcp` `server::tests::a_ping_is_answered_while_a_tool_call_waits_for_the_window` (bod 12).
- Prevzatie okna od nečinného/zamrznutého klienta na strane okna už funguje
  (`live_bridge_bootstrap.rs:325`); zaseknutie v MECH-1 je na strane servera (body 12–15) a v Supervisore (20).
- Ďalej (po merge #15): E2E bez okna — čistý profil (dočasný HOME/APPDATA) → Pripojiť → klient zavolá
  `inspect status` → OK; a prevzatie, keď starý server visí na neodpovedanej úlohe.
