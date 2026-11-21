# Review 2026-11-21 (ac): stav opráv

Review (`docs/archive/reviews/review-2026-11-21-ac.md`, PR #174) bol robený na `b1f9e29f`
(0.3.4-dev). Opravy išli v poradí podľa §7: P1, potom P2, potom P3 po skupinách, každá v
samostatnom PR s testom, ktorý bez opravy padne. Záznam v `CHANGELOG.md` (Unreleased) je tentoraz
v každom PR s opravou; kolízie horných riadkov sa riešili zlúčením `main` do vetvy PR.

Stavy: **opravené** (zlúčené na `main`), **odložené** (s dôvodom), **na rozhodnutie vlastníka**,
**už riešené** (oprava bola na `main` pred review alebo v inom PR).

## P1

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1121ac-PG-1000 | opravené | #175 | Ukotvenie sa skúša pod silou, ktorú spevnenie nesie: kĺbový prút tlačí pozdĺž svojej osi (stĺp 20 mm pri prúte pod 40° drží 0,94 N/mm, nie 124,8), votknuté spevnenie otáča ukotvenie okolo bodu prvku. Diafragma na ukotveniach ako tuhá doska na pružinách. |
| R1121ac-PG-1001 | opravené | #175 | Skupina skrutiek drží proti otočeniu len okolo normály dotyku; pri osi momentu v rovine dotyku je spoj kĺb (EN 1995-1-1 tab. 7.1 nedáva modul preklzu). Dve skrutky pozdĺž osi už nevotknú vešiak. |

## P2

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1121ac-PG-1002 | opravené | #179 | Každý diel cesty tlačený pozdĺž svojej dĺžky vybočí cez celú dĺžku dielov koncom ku koncu po diel, ktorý silu prenesie naprieč: spevnenie 15 × 15 na ukotvení 30 × 30 drží 69,4 N (na strane bezpečnosti), nie 277,6. |
| R1121ac-FEA-1000 | opravené | #176 | Jemnejšia zlepená plocha nasleduje hrubšiu, počet väzieb rozhoduje len pri zhode; police 0,5 a 0,67 mm medzi bočnicami 1 a 2 mm sú do 0,2 % od monolitického telesa. |
| R1121ac-ZIP-1000 | opravené | #177 | Kontrola balíka prijme len `made_by = 20` a `external = 0`; komentár archívu aj položky sa odmietne. |
| R1121ac-TST-1000 | opravené | #178 | Test `stretched_copies_over_what_one_import_stores_are_refused` zlyhá s vrátenou opravou SKP-900. |

## P3 (§4)

| Skupina | Stav | PR | Čo |
|---|---|---|---|
| Statika | opravené | #181 | Diel cesty s pomernou štíhlosťou ≤ 0,3 (EN 1995-1-1 6.3.2(3)) nemá vzpernú silu, takže blok ohýbaný vešiakom už neukáže F_cap 4 669 kN („bez vzperu“). |
| Tepelný výpočet | opravené | #180 | Horná hranica `r` vrstvy vzduchu podľa hrúbky (EN ISO 6946 tab. 8, 5 mm ~0,11); rada pri kondenzácii hovorí o parozábrane (EN ISO 13788), nie o chýbajúcich údajoch; `--summary` má blok `thermal` (verdikt, počty, neoverené). |
| CAM a balík | opravené | #182 | Oblúk s výškou ≤ pol kroku sa vždy zapíše ako G1 (oblúk r 2 500 s tetivou 0,0011 mm, 4,4e-7 rad, ktorý Grbl vykoná ako celý kruh); `UnwritableArc` a `DrillPointDoesNotFitFloor` menujú operáciu, pohyb a XY; `NOT_CHECKED` uvádza odchýlku až štyroch krokov; apostrof v `losses.csv` je zdokumentovaný (`WORKSHOP_LOSSES`, schéma exportu MCP). Komentár archívu: už riešené v #177. |
| FEA | opravené | #183 | CSR sa rezervuje podľa rôznych pozícií, nie tripletov (najmenší rozpočet statiky 32 → 22 MiB, módov 68 → 29 MiB); validácia (~214 B na prvok) a tepelný solver v rozpočte; `FactorTooLarge` „at least … the study“, `ModelTooLarge` menuje každý limit; `not_checked` módov hovorí, čo `converged` nesľubuje; úroveň zostavy hlási `untied_bonded_nodes`. |
| Modelovanie a SketchUp | opravené | #184 | `check` sčíta zaoblenia a skosenia na každej ploche (r = 6 na dvoch hranách 10 mm plochy sa odmietne, rada 9,99); príliš malá veľkosť menuje parameter; EXA-901: odmietnutie menuje prvok, v pomocníkovi AI operáciu a pole, `fix_hint` je pravidlo poľa; SKP-900 hovorí o natiahnutých kópiách; cgroup v1 číta skupinu procesu. |
| Bezpečnosť a CI | opravené | #185 | Prázdny alebo relatívny `XDG_CONFIG_HOME` sa ignoruje; denník zápisov drží 100 zápisov, nie súborov, a uvádza zmazané listy aj súbor, ktorý zlyhaný export nevrátil; hláška ELOOP/ENOTDIR radí skúsiť znova; `test_ci_root_tests.py`, `check_test_timing.py`, `check_user_folder_writes.py`, `release_reads.py` a `LINT_ALLOW` (aj `cfg_attr`) podľa review. Limity allow štyroch crate zvýšené o 16 existujúcich `cfg_attr`, baseline `user_folder_writes` +1 (`consent.rs`, existujúci zápis). |
| Stav 11-20 | už riešené | #173 | `review-2026-11-20-ac-status.md` je na `main` (zlúčené po commite review). |

### Odložené a na rozhodnutie

| Nález | Stav | Dôvod |
|---|---|---|
| Hľadanie najväčšej veľkosti po odmietnutom dokončení (`native_finish.cc:1143`) | odložené | Už je obmedzené na ~10 stavieb OCCT. Časový limit by sa dal overiť len nočným testom `perf_*` na veľkom telese (AGENTS §6); bez neho by bola zmena neoverená. |
| Loft s krivkou U (`native_loft_rail.cc:198-202`) | odložené | Nepodarilo sa zostaviť platný loft, ktorý by chybu vyvolal; oprava bez testu, ktorý bez nej padne, by porušila §7. |
| `loads.rs` (2 985) a `member_check.rs` (2 981) pri limite 3 000 | odložené | Pozorovanie, nie chyba. `check_file_sizes.py` limit drží; súbor sa rozdelí, keď ďalšia zmena potrebuje miesto (ako `planner.rs` a `sidecar.rs` v #184). |
| Premenovanie `Kečup.app` na `Ketchup.app` a konfigurácie AI | už riešené | Klienta AI nastaveného na zmazaný `/Applications/Kečup.app` opraví okno zastaraných klientov (**Use this Ketchup**, `d6542595`). Ak je stará 0.3.3 `Kečup.app` stále nainštalovaná, klient ju ďalej spúšťa a ukáže to len karta Connect AI; je to zámer, môže ísť o druhý Kečup ponechaný naschvál. |
| Plochy miestností: bod v hrúbke steny (`room_areas.rs:142`) | odložené | Bez zmeny oproti stavu 11-20: oblasť medzi dvoma čiarami steny sa od úzkej miestnosti nedá odlíšiť bez významu vrstiev; odmietnutie podľa šírky by odmietlo aj komory a chodby. |
