# Čo opraviť po review 2026-11-19 (og), s rozhodnutiami vlastníka

Pracovný zoznam pre implementáciu. Vychádza z `review-2026-11-19-og.md` (main `86b606ba`)
a z rozhovoru s vlastníkom 2026-11-19. Podrobnosti, sondy a dôkazy ku každému bodu sú v review
pod uvedeným ID. ID majú príponu recenzenta `og` (`R1119og-…`, `og:R1110-…`), aby sa nezrazili
s ID druhého recenzenta.

**Kde to je:** vetva `claude/optimistic-goodall-2aqwdt`, priečinok `docs/archive/reviews/`:

- `review-2026-11-19-og.md`: celé review;
- `review-2026-11-19-og-todo.md`: tento zoznam;
- `review-2026-11-10-og.md`, `review-2026-11-10-og-r2.md`: staršie nálezy `og:R1110-…`.

Ak tie súbory na `main` nie sú, prečítaj ich príkazom
`git show origin/claude/optimistic-goodall-2aqwdt:docs/archive/reviews/<súbor>`. Odporúčame ich
pridať na `main` v prvom PR, ktoré začne tieto opravy (bod 25).

## Rozhodnutia vlastníka (platia pred akýmkoľvek starším nálezom)

### R1. Zápis súborov pripojenou AI

Kečup beží na jednom počítači pod jedným používateľom. Zapisuje len vlastné typy súborov, ktoré
sa samy nespustia, a existujúci súbor bez súhlasu neprepíše. Povolené priečinky preto
neochraňujú nič, len otravujú, a dajú sa obísť (R1119og-SEC-800).

1. AI smie vytvoriť **nový** súbor kdekoľvek, kam systém dovolí, a to dokument aj každý export
   (DXF, DWG, PDF, PNG, ZIP, BTLx, woodWOP, NC, …), **bez otázky**.
2. **Prepis existujúceho súboru** sa pýta v okne (`authorize_overwrite`, ako dnes).
3. Odmietnu sa len systémové miesta, kam používateľ neukladá:
   - priečinky automatického spustenia (Po spustení / Startup, `~/.config/autostart`,
     `~/Library/LaunchAgents`);
   - priečinky Windows a Program Files;
   - koreň systémového disku.

   Odmietnutie musí vysvetliť, prečo.
4. Namiesto otázok **denník v karte Connect**: čo AI kam zapísala (cesta, čas, veľkosť) a tlačidlo
   „Otvoriť priečinok“.
5. Zrušiť celý systém povolených priečinkov podľa pravidla 6 AGENTS:
   - `live_bridge/write_targets.rs` (in_sandbox, roots, Open Recent ako koreň);
   - `write_target_declined`;
   - povolenia priečinkov v nastaveniach dokumentu a ich testy.

   Odpadnú tým og:R1110-SEC-400 a R1119og-SEC-800.

### R2. Pripojenie AI k oknu

1. **Posledná pripojená AI vyhráva, bez otázky**, ako pred R1108-MCP-240.
2. Okno to oznámi len v stavovom riadku a v karte Connect, napríklad „Pripojený Claude Code
   (predtým Codex)“, bez dialógu.
3. Zrušiť dialóg prevzatia:
   - `consent.rs` (takeover, `takeover_description`, odpoveď `declined`);
   - príslušné texty v `locales`;
   - príslušné testy.

   Odpadnú R1109-MCP-305 a R1119og-SEC-804. Dialóg vznikol z nálezu R1108-MCP-240, ktorý sa
   týmto ruší.

### R3. Smer pre ďalšie review

Nesprísňovať pravidlá pre AI na jednom počítači pod jedným používateľom, ak nehrozí strata práce
používateľa. Pri statike, FEA a CAM naopak vždy povedať, čo kontrola **nepokrýva**.

## 1. Hneď: čísla a programy, ktorým ľudia uveria

1. **R1119og-MF-801 — G-kód bez kompenzácie dĺžky nástroja (bezpečnosť stroja).**
   - **Kde:** `crates/ketchup-model/library/cam-gcode-dialects.json`, dialekt `iso_metric_gcode`
     (predvolený aj pre MCP `cam_nc`, `ketchup-mcp/src/schema.rs:219`).
   - **Úloha:**
     - v hlavičke reset `G40 G49 G80 G94` a `G00 G43 H{tool} Z{safe}`;
     - prvý pohyb najprv v Z, až potom XY (MF-804);
     - komentáre v zátvorkách, nie `;`;
     - alebo export bez výberu riadenia odmietnuť.
   - **Test:** výstup každého dialektu obsahuje kompenzáciu dĺžky pred prvým rezom.
2. **R1119og-FEA-800 — spoj plôch v zostave je tuhá doska.**
   - **Kde:** `crates/ketchup-scheduler/src/fea_assembly.rs:313-371` (`joints`).
   - **Úloha:**
     - viazať len prekrývajúcu sa časť plôch (uzol na uzol alebo uzol na plochu);
     - tuhý „pavúk“ len ako výslovná voľba pre skrutku;
     - plochy bez prekrytia odmietnuť.
   - **Test:** dve vrstvy 1000×50×50 spojené plochou prehnú koniec ako jeden kus (0,342 mm, nie
     0,077 mm).
3. **R1119og-PG-800 a PG-801 — ukotvenie zaistenia bez vzpery a bez diafragmy.**
   - **Kde:** `crates/ketchup-program/src/loads.rs:1460`, `:2093-2098`; `loads/anchorage.rs`.
   - **Úloha:**
     - cestu zaistenia začať pri prvku: EA/L vzpery, vzper na dĺžke vzpery a poddajnosť oboch
       spojov;
     - pre každú kotvu diafragmy volať `Footing::anchorage` a pridať šmykovú tuhosť dosiek;
     - pri nesplnení „not verified“.
   - **Testy:**
     - tri pásiky 3×3 mm nezaistia nosník GL24h 80×400 na 5,8 m;
     - dosky na voľných stĺpikoch 60×40×2400 nezaistia nosník.
4. **og:R1110-PG-402 a R1119og-PG-802 — statický výpočet pre úrad.**
   - **Kde:** `crates/ketchup-program/src/calculation.rs` (`document`, `held`).
   - **Úloha:**
     - sekcia „Upozornenia / Upozornění“ so všetkými chybami a varovaniami (najmenej
       `load_unassigned`, `member_not_carried`, `declared_bracing_ignored`, `joint_kind_unknown`,
       `thermal_*`, nehodnotené závesy);
     - celkový verdikt a návratový kód v hlavičke;
     - medzi zaisteniami len tie, ktoré kontrola použila, s C a F_cap pre každú vzperu;
     - opraviť zastaranú poznámku „Výstuhy … posúdené nie sú“.
   - **Test:** pri strate zaťaženia (rc 3) HTML neprečíta „Vyhovuje“ bez upozornenia.
5. **R1119og-MF-800 — vŕtanie neprejde cez diel.**
   - **Kde:** `crates/ketchup-manufacturing/src/cam_region.rs:299-313`, `cam.rs:1006-1040`,
     `:1780`, `cam_simulation.rs:263`.
   - **Úloha:**
     - vrcholový uhol nástroja v `CamTool`;
     - prevŕtanie pod výstup priechodnej diery (obmedzené polotovarom a upínaním);
     - pri vŕtaní žiadny osový prídavok;
     - simulácia vrtáka a guľovej frézy v skutočnom tvare (MF-803).

## 2. Politika pre AI

6. **Zápis súborov podľa R1.**
7. **Pripojenie podľa R2.**

## 3. Vydanie a dáta

8. **og:R1110-REL-404 a R1119og-DOC-801 — staršia verzia zahodí severku, revízie a podlažia.**
   - **Kde:** `ketchup-manufacturing/src/title_block.rs:81-121` (`SheetSettings`),
     `ketchup-app/src/app/project_drawings.rs:1479-1484` (rozšírenie `sheet-v1.json`
     s `required=false`).
   - **Úloha:**
     - keď sú použité `storeys`, `zero_altitude_mm`, `north_tenths_deg` alebo `revisions`, uložiť
       vyšší formát alebo schému, ktorú 0.3.2 a 0.3.3 odmietnu;
     - `scripts/release_reads.py` nech prečíta a znova uloží `sheet-v1.json` cez `SheetSettings`
       každého tagu a zlyhá pri strate poľa;
     - opraviť sľub v README („No stable public API…“) a v poznámkach k vydaniu.
9. **R1119og-REL-800 — príkaz pre statický výpočet nie je v balíkoch.**
   - **Kde:** `scripts/linux/build-package.sh:28,34`, `scripts/macos/build-app.sh:35`, balík pre
     Windows.
   - **Úloha:** pribaliť `ketchup-program`, alebo pridať export statického výpočtu v okne a cez
     MCP.
10. **R1119og-DOC-802 — CHANGELOG 0.3.3** uvádza tri položky z `887d1ec1`, ktorý v tagu nie je.
    Presunúť ich do Unreleased.

## 4. CI

11. **R1119og-EXA-801 a CI-807 — nočný beh na Windows je červený.**
    - **Kde:** `house_panel_validation.rs:51`: `exact_collision_timeout` 30 s, 163 dvojíc, od
      11-17; ešte 11-16 prešlo za 453 s.
    - **Úloha:**
      - bisect `f9cb5142..e1187366` (PR #109 a #111; podozrivé sú predohra +928 riadkov,
        `PLANAR_PROFILE_SEGMENTS` 64 → 256 a `arcs_intersect`);
      - issue pre každý červený nočný beh;
      - limity `perf_*` voči rýchlosti runnera.
12. **CI-407 a CI-808:**
    - znova spustiť beh 42661783930;
    - stabilizovať `connect_ai::the_section_shows_the_status_and_folds_away_three_set_ups_to_copy`
      (`connect_ai.rs:452-462`).

## 5. Výkresy na úrad

13. **og:R1110r2-MF-500 a MF-501 — podlažia.**
    - **Kde:** `ketchup-app/src/app/project_drawings.rs:1808-1809` (`storeys(&solids.solids, …)`).
    - **Úloha:**
      - bez tabuľky použiť tie isté podlažia ako `ModelFloors::table`, aj pre výškové kóty;
      - varovať, keď v tabuľke chýba podlažie, ktoré model má.
    - **Test:** dom so stropom 2700–2954 bez tabuľky má horné podlažie na 2954 (rez 4154) a nemá
      „+2,512“.
14. **og:R1110-CD-404 a UO-406 — miestnosti v DXF.**
    - **Kde:** `ketchup-model/src/…/rewrite.rs:2590-2620`, `live_bridge/room_areas.rs`.
    - **Úloha:**
      - výplň s priehľadnosťou (440) alebo svetlou farbou;
      - texty vo vlastnej farbe alebo hladine;
      - výplň v poradí kreslenia pod čiarami (SORTENTSTABLE alebo poradie entít);
      - bod na čiare steny odmietnuť.
15. **og:R1110-MF-404 — hárky DXF 2013** (`ketchup-manufacturing/src/sheet_cad.rs:288-317`,
    `ketchup-dwg/src/sheet.rs`):
    - čiara rezu typom `CENTER`;
    - výplne značiek ByLayer, nie RGB 0;
    - štýl písma TTF (ISOCPEUR alebo Arial);
    - neskôr entity DIMENSION;
    - zmazať zastaraný riadok `sheet_dxf.rs` z baseline panikov (TST-404).
16. **og:R1110-PG-403 až PG-410 — texty statického výpočtu a tepelnej kontroly:**
    - „rozhoduje“ len pri najvyššom využití (`calculation.rs:429`);
    - k_h pre lepené drevo 3.3 (3.2), priehyb w_inst nie rovnica (2.2) (`:70`, `:86`);
    - stĺp bez textu o klopení;
    - riadok o stavenisku v SK/CZ (`site.rs`);
    - `thermal.rs:150-158` vypíše „<“ len pri nesplnenom kritériu;
    - SK 0,22 označiť ako U_N, nie U_r2;
    - zápornú nadmorskú výšku vypísať orezanú (`site.rs:101-106`);
    - `brace()` na stĺpe: hláška „zaistenie stĺpa sa nepoužíva“.
17. **R1119og-MF-805 — popisy dier** (`hole_callouts.rs:392`):
    - hĺbka vŕtania aj pri závite;
    - jazyk a desatinná čiarka hárku;
    - „M8“ namiesto „M8 x 1.25“ pri hrubom závite.

## 6. Dielňa, import, modelovanie

18. **og:R1110-MF-400, MF-401, MF-406 a R1119og-MF-807 — straty v dielenskom balíku.**
    - **Kde:** `workshop_package.rs:136-198`, `woodwop_part.rs:244`, `app/workshop_export.rs:68,84`.
    - **Úloha:**
      - `btlx/model.btlx.support.txt` a strojovo čitateľný `losses.csv` (závity, kolíky
        v nevyrábaných dieloch, frézované diery);
      - dôvody `not_made`, keď sa nevyrobí nič;
      - pri `cam_nc` riadok `lost` pre závity.
19. **og:R1110-MF-402 a MF-403 — kontrola balíka** (`workshop_package.rs:228-338`,
    `three_mf_export.rs:665`):
    - porovnať lokálne hlavičky s centrálnymi;
    - odmietnuť skryté záznamy, duplicity podľa veľkosti písmen a NFC a mená zakázané vo Windows;
    - príznak UTF-8 (bit 11).
20. **R1119og-SKP-800 — SketchUp, komponent zmenený nástrojom Scale** (`import/glb.rs:1726-1810`):
    mierku zapiecť do vlastnej definície, umiestnenie nechať tuhé, a dať poznámku.
21. **R1119og-IMG-800 — prezentačný obrázok** (`presentation.rs:313-323`): priveľkú textúru
    nahradiť farbou s poznámkou, nezhodiť celý obrázok.
22. **R1119og-EXA-800 — riadiaci bod voľnej plochy** (`document/entities.rs:925-940`,
    `native_control_points.cc:183-195`): ukladať polohu v mm alebo (u, v), nie index pólu; pri
    zmene siete odmietnuť s oboma polohami.
    - **Stav:** opravené vo vetve `fix/og-control-point` — `SurfaceBodySpec::ControlPoint` nesie aj
      `from_mm` (poloha bodu v mm) popri indexe; `control_points.rs` porovná pól siete s `from_mm`
      a pri inej polohe alebo bode mimo siete odmietne s oboma polohami a rozmermi siete. Testy
      `ketchup-exact/tests/surface_control_points.rs`, `ketchup-scheduler/tests/surface_control_point.rs`.

## 7. Revízie DXF od architekta

23. **og:R1110-UL-400 až UL-403** (`rewrite/compare.rs`, `rewrite/revisions.rs`,
    `app/underlay/revision.rs`, `app/underlay.rs:327`, `live_bridge/dwg_import.rs:332-342`):
    - rozdiel štýlu (farba, typ a hrúbka čiary, farba hladiny, štýl textu);
    - párovanie nanovo očíslovaných entít podľa geometrie a odmietnutie, keď jedna strana nemá
      handle;
    - prekryv cez index a mimo vlákna okna;
    - odpoveď MCP ako počty plus stránkovaný zoznam.

## 8. FEA, zvyšok

24. **R1119og-FEA-801 až FEA-809:**
    - príznak zrušenia do faktorizácie a do každej iterácie (FEA-806);
    - M-ortonormálne štartovacie vektory (FEA-802);
    - minimálne zjemnenie siete a napätie v konvergencii (FEA-803);
    - Sturmova kontrola frekvencií (FEA-801);
    - priehyb vnútri prvku (FEA-804);
    - text `excluded_physics` a vlastná tiaž (FEA-809).

## 9. Proces

25. **Review:**
    - texty review `-og` (11-10, 11-10-r2, 11-19, tento zoznam) a review druhého recenzenta pridať
      na `main`;
    - ID s príponou recenzenta;
    - jeden dokument stavu na každé review: `review-2026-11-19-og-status.md` so všetkými bodmi
      tohto zoznamu.

## Keď bude čas (P3)

- **Ratchety:**
  - makrá `( )` a `[ ]` (TST-400);
  - turbofish `unwrap` (TST-401);
  - logika `cfg` v `check_ci_crates` (CI-402);
  - kód len pre release a za feature (TST-403);
  - baseline proti základu (TST-804);
  - `check_user_folder_writes` (TST-805);
  - ďalšie diery (TST-806);
  - AGENTS §2 a §6 (DOC-809);
  - „vypratie“ ratchetu červeným commitom (CI-803).
- **Presné modelovanie:**
  - odmietnutia zaoblení a loftu s dielom a polohou v mm (EXA-802, EXA-805);
  - plné zaoblenie bez histórie plôch (EXA-803);
  - zlúčené príčiny (EXA-804);
  - samoprekrytie voľných plôch (EXA-806, EXA-807).
- **SketchUp:**
  - skryté plochy (SKP-801);
  - zbytočné varovania (SKP-802);
  - poškodený `model.dat` (SKP-803);
  - skorý limit vnorení (SKP-804).
- **Platforma:**
  - rozpočet pamäte: `MemoryBudget` namiesto `WorkerExited`, podprocesy pluginov, FEA v okne
    (PL-801, PL-802, EXA-808);
  - balík pre Linux: test s oknom, `libx11-xcb1`, podpis, `GH_TOKEN` (PL-803).
- **MCP:** horné limity `revisions`, `storeys` a `change` (og:R1110-MCP-402).
- **Výroba:**
  - `plan_overlap` pri dlhých pásoch (og:R1110-MF-405);
  - CAM: obvodový prejazd obdĺžnikovej kapsy (MF-802), ostrov pri stene (MF-806).
- **Statika:**
  - natočenie skupiny spojovacích prostriedkov a K_u (PG-803);
  - vzper konzolového stĺpika 2L (PG-804).
- **Podklady:**
  - plocha miestnosti a mierka (UO-407);
  - revízia s jednotkou podkladu (UL-408);
  - DXF 12 a výplne bez `object_ids` (CD-405).
