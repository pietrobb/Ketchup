# Overenie nálezov CR-2026-10-02 pred opravami

Baseline overenia `e92f4a7`. Každý nález som znova overil v aktuálnom kóde a doplnil, **kto sa k chybe reálne dostane**. Prvý review dosiahnuteľnosť nehodnotil. Crony #68/#69 boli počas overenia vypnuté; po súhlase používateľa 2026-10-02 16:50 s opravou všetkých nálezov sú znovu zapnuté. Nasledujúce odporúčania sú historický výsledok triáže, nie zúženie autorizovaného rozsahu. Priebeh opráv je v hlavnom review Markdownu.

| ID | Reálne? | Dosiahnuteľné bežným používaním? | Nová priorita | Odporúčanie |
|---|---|---|---|---|
| CR-01 | áno (spustené kontroly) | CI je červené | P1 | opraviť |
| CR-02 | áno, `result_history` sa čistí iba pri New/Open/reštarte workera (`app/document.rs:111`, `app/exact.rs:388`) | áno, každá revízia ostane v RAM počas celej relácie | P2 | opraviť (držať iba zdroje, ktoré sú v Undo/Redo) |
| CR-03 | áno, `exact.rs:134–142` vráti Err pri 2 vyhovujúcich DX12 adaptéroch aj s `requirement=None` | áno, notebook s iGPU + dGPU alebo PC so zapnutou integrovanou kartou sa nespustí | **P1** | opraviť ako prvé, oprava je malá |
| CR-04 | áno (runtime R1), žiadna ochrana pred otvorením toho istého súboru v dvoch oknách | iba pri dvoch oknách na tom istom súbore (napr. používateľ + MCP `open_window`) a následnom páde | P2 | opraviť jednoduchšie: druhé okno na ten istý súbor upozorniť/odmietnuť, bez vetvenia recovery |
| CR-05 | áno (runtime R2) | **nie**: jediný produkčný volajúci (`persistence.rs:1346`) pri Err zahodí celý kontajner | P3 | 3-riadková oprava cez `entry`, len popri iných opravách |
| CR-06 | áno (runtime R7) | **nie**: cyklus vznikne iba v dávke `CreateGroup/SetGroupParent + ConvertGroupToComponent`. Taká dávka nevzniká v GUI ani v asistentovi či MCP (1 intent = 1 príkaz, Ungroup prevesí deti na starého rodiča). Samostatný cyklický `SetGroupParent` zachytí záverečná validácia | P3 (bolo P1) | lacná poistka `visited` v `group_is_descendant`, nie urgentné |
| CR-07 | áno | **nie na Windows**: `sync_parent_directory` je tu no-op | P4 | odložiť |
| CR-08 | áno (runtime R6) | áno, ale zriedkavo: GUI Revision history → rollback na revíziu líšiacu sa iba zdrojom programu, nie geometriou → chyba „NoOp“ | P3 | malá oprava (porovnať aj `rule_program`) |
| CR-09 | áno (runtime R3), `model.rs:993–1001` predpokladá os Y cez 0 | áno pri `revolve(..., axis=...)` mimo predvolenej osi: zlý `part_info`/`reach` → zlé umiestnenie a vynechaný kolízny kandidát | P2 (bolo P1) | opraviť, oprava je lokálna (bounds z profilu a osi) |
| CR-10 | áno (runtime R4), `faces.rs:337–341` berie obdĺžnik profilu | iba pri neobdĺžnikových profiloch (rohová polica, skosenie) | P2 | opraviť spolu s CR-11 (skutočný polygón kontaktu) |
| CR-11 | áno (runtime R5) | iba pri doske otočenej o iný uhol než násobok 90°. Validátor to **hlási** (`hole_outside_face`), takže chyba nie je tichá | P3 | spolu s CR-10 |
| CR-12 | áno (runtime R8), `rule_program.rs:136–161` nikdy neoverí výsledný rozmer | áno pri nelineárnom vzťahu, napr. zaokrúhlenie na 32 mm raster, `max/min` | P2 | opraviť: po výpočte overiť rozmer, inak použiť `push_pull` riadok (fallback už existuje) |
| CR-13 | áno, `consent.rs:185–216` automaticky vydá token každému loopback klientovi | **iba na počítači s viacerými súčasne prihlásenými používateľmi** (RDS, prepínanie používateľov); webová stránka sa cez HTTP nepripojí (rámec nie je JSON riadok). Pohodlný attach bez tokenu bolo tvoje rozhodnutie | P4 (bolo P1) | odložiť, kým nebude cieľom terminal server |
| CR-14 | áno, Starlark eval beží synchrónne na UI vlákne (`program_check.rs:89`) a nemá limit krokov (`eval.rs:2394`) | áno: chybný program s obrovským `range` zamrazí okno natrvalo. Neskorý commit po zrušení je okrajový, transport hlási „unknown outcome“ | P2 (bolo P1) | opraviť: limit krokov a vyhodnotenie mimo UI |
| CR-15 | áno, Query/Detail/EditContext/WorksetStatus/BatchJobStatus ignorujú `expected` | málo: `expected` je voliteľný a AI ho takmer nepoužíva | P3 | jednoriadková oprava na vetvu (`Self::guard`) |
| CR-16 | áno, `tools.rs:382–386` berie prvé nové okno, discovery nemá PID | iba ak sa počas ~sekundy štartu otvorí aj iné okno | P4 | odložiť |
| CR-17 | áno, `tools.rs:177` číta celý súbor | iba pri omylom zadanom obrovskom súbore; rámec ho aj tak odmietne | P3 | lacná kontrola veľkosti pred čítaním |

## Súhrn

- **Všetkých 17 nálezov je technicky reálnych.** Žiadny nie je falošný: runtime reprodukcie aj kód sa zhodujú.
- **Štyri P1 boli nadhodnotené:** CR-06 je v aplikácii nedosiahnuteľné, CR-13 platí iba pre multi-user stroj, CR-09 a CR-14 sú P2.
- **Naozaj riešiť (8):** CR-01, CR-03, CR-02, CR-12, CR-09, CR-14, CR-04 (zjednodušene), CR-10+CR-11.
- **Lacné poistky popri tom (5):** CR-05, CR-06, CR-08, CR-15, CR-17. Každá je na niekoľko riadkov.
- **Odložiť (3):** CR-07 (iba Unix), CR-13 (multi-user), CR-16 (zriedkavý race).
