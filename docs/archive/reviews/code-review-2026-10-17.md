# Code review 2026-10-17: `4b43046`

Nadväzuje na `code-review-2026-10-16-status.md`, overený na `cd90cb8`. Od toho stavu
pribudlo 25 commitov. Review má dve časti:

1. overenie otvorených bodov a nového kódu;
2. čerstvý review oblastí, ktoré predošlé review nečítali: importéry DXF, DWG, STEP, STL,
   GLB a kscene, kontroly EC5, výkresy a ich exporty, `ketchup-interaction`, `ketchup-pdm`,
   `ketchup-assistant` a exporty STEP, STL a 3MF.

## Metóda

- Dvaja nezávislí čitatelia. Nálezy so stĺpcom **K** som sám overil čítaním kódu.
- Spustené na `4b43046`:
  - všetky `scripts/check_*.py`: prešli;
  - lokálne Linux job z CI (`fmt`, `clippy` 15 crate vrátane `ketchup-model`, testy): prešiel, 1 328 testov, 0 zlyhaní.
- CI na GitHube:
  - beh 510 (push `4b43046`) je zelený, nočný beh 511 bežal;
  - od `cd90cb8` zlyhal jeden beh (507) a jeden sa zrušil.
  - S pre-push hookom je to zreteľne lepšie než predtým (9 z 30).

## Súhrn

- **Otvorené body:** z bodov, ktoré `-10-16-status` viedol ako otvorené alebo čiastočné, je
  opravených 13 a k tomu nízke body. Čiastočne sú 2 (R-1, R-6) a neopravený 1 (PS-11). SEC-5 je zámer (rozhodnutie vlastníka, pozri tabuľku).
- **Opravy s testom:**
  - MF-1 (závit sa dá v histórii prvkov zmeniť aj zrušiť a `SetHoleThread` ho kontroluje);
  - N-12 (report už neblokuje apply, nesie varovanie `checks_unverified`);
  - VW-10 a R-3 na Androide;
  - R-4 (validátor dokumentu má pravidlo hrúbky);
  - PRG-1 (skrátený report so zoznamom prvých issues);
  - Scale s jedným parserom;
  - nočný beh `large_house` a desktop viewer v CI.
- **Nové nálezy:**
  - 2 vysoké: kontrola stĺpa podľa EC5 pri profilovanom priereze (F-1) a náhľad Move, ktorý
    nesedí s vykonaním (D-1);
  - 7 stredných;
  - rad nízkych.
- **Čerstvé oblasti sú v dobrom stave.** Importéry sú ohraničené a nepadajú, jednotky STEP a
  IGES sa neškálujú dvakrát a STL správne otáča trojuholníky pri zrkadlení. Tabuľky EC5
  (kmod, kdef, γM, kh, kcr, kv) sú správne.

## Stav otvorených bodov

| ID | Stav | Commit | Poznámka |
|---|---|---|---|
| SEC-5 | zámer, uzavreté | – | Rozhodnutie vlastníka 2026-10-17: okno spustené cez MCP `open_window` otvára a importuje súbory bez potvrdenia. Ďalšie review to nehlásia. |
| N-12 | opravené, so zvyškom D-4 | f252e31 | |
| MF-1 | opravené, so zvyškom D-10 | 706d8dc, 4635582 | |
| VW-10 | opravené | fabc319 | Zápis balíka beží na vlákne. Pozri D-7. |
| VW-11, R-3 | opravené | e8b0c45, e549793 | |
| PS-11 | **neopravené** | – | Každá dávka klonuje celý `ProductModel` (`store.rs:1196`). |
| PS-12, PS-4 | opravené | 96eec3e, 2efc085 | |
| PRG-1 | opravené, so zvyškom D-3 | 9ebc832 | |
| PRG-5, R-4 | opravené, so zvyškom D-11 | 838f07d | |
| UI-6 (Scale) | opravené | d2221d9 | Pozri D-9. |
| R-1 | **čiastočne** | 66ba6d8, 3847bc8, 4e7ea7b | Pozri D-1 a D-2. |
| R-2 | opravené, so zvyškom D-6 | 6cae929, 2cc2fb7 | |
| R-5 | opravené | 7ed221b | |
| R-6 | čiastočne | 3ed968d, 3d02ff1 | Pozri D-8. |
| nízke body | opravené | 4b43046, 89621e7, b3337dd, 6ad0eea | Patrí sem nonce, Mirror v registri gest, verzia Visual Studia v kľúči cache a spoločný deadline. |

## Nové nálezy

### Vysoké

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| F-1 | K | `ketchup-program/src/member_check.rs:385-398`, `:1049-1054` | **Kontrola stĺpa podľa EC5 nadhodnocuje prierez a môže povedať „vyhovuje“.**<br>• Pri prvku vytiahnutom z profilu (`long == 2`) sú `b` a `h` rozmery obalového kvádra profilu.<br>• Vzper počíta `force / (b*h)` a štíhlosť z `min(b,h)`, teda ako pre plný obdĺžnik.<br>• Príklad: stĺp L 200×200 s ramenami 50 mm. Skutočná plocha je 17 500 mm², kontrola ráta so 40 000 mm². Polomer zotrvačnosti k slabej osi je tiež oveľa menší než 200/√12.<br>• Využitie na vzper vyjde viac ako 2× nižšie, než v skutočnosti je. Člen `net` síce používa skutočnú plochu, ale bez kc, takže prvok môže dostať „pass“, hoci podľa 6.3.2 nevyhovuje. | • Brať `shape.area` a √(I_min/A) zo `section()` k obom hlavným osiam.<br>• Kým to nie je, neobdĺžnikový prierez stĺpa hlásiť ako `not_verified`.<br>• Test so stĺpom L a s dutým stĺpom. |
| D-1 | K | `ketchup-app/src/app/tool_gesture.rs:593-605`, `app/shell.rs:612` (`length_preview_mm`) | **Náhľad Move ukazuje inú hodnotu, než sa vykoná.**<br>• Pri jednej `*` vedľa čiarky ide text do parsera vektora, ale náhľad nad poľom ho vyhodnotí ako jednu dĺžku.<br>• Bez zamknutej osi:<br>• `2*1,5`: náhľad „= 3 mm“, posun (2, 5, 0);<br>• `12,5*3`: náhľad 37,5 mm, posun (12, 15, 0);<br>• `1,5*2m`: náhľad 3 000 mm, posun (1, 10 000, 0).<br>• Existujúci test prejde len preto, že zamyká os Z. | Viacero hodnôt deliť len cez `;` alebo `x`. Ak text dáva dve platné čítania, odmietnuť ho s nápovedou `80;60`, ako to 66ba6d8 zaviedol pre obdĺžnik. Náhľad a vykonanie musia ísť cez tú istú funkciu. |

### Stredné

| ID | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|
| D-2 | S | `ketchup-app/src/length_input.rs:73-75` | **Obdĺžnik ďalej potichu číta nesprávne a polia sa nezhodnú.**<br>• Čiarka vyhrá, kedykoľvek sú obe časti dĺžky: `2*1,5` dá 2 × 5 a `1,5*2m` dá 1 mm × 10 m.<br>• Tie isté texty sa v poliach čítajú rôzne:<br>• `10+5,20`: obdĺžnik 15 × 20, Move 15,2 mm;<br>• `80*60`: obdĺžnik 80 × 60, Move 4 800 mm;<br>• `1+0,5`: obdĺžnik 1 × 5, Move 1,5 mm, Scale faktor 1,5.<br>• Po troch opravách (R-1, N-10, D-1) je jasné, že čiarka ako oddeľovač aj ako desatinná čiarka sa pravidlom zachrániť nedá. | Jedno pravidlo pre všetky polia: čiarka je vždy desatinná a hodnoty sa oddeľujú `;` alebo `x`. Starý zápis `80,60` odmietnuť s nápovedou `80;60`. |
| D-3 | S | `live_bridge/program_validation_summary.rs:76-84`, `live_bridge.rs:791-795`, `program_worker.rs` | **Skrátený report sa berie ako úplný.**<br>• Celkový `state` a `declared_intent` ignorujú `report_truncated`. Ak sa skrátením stratia varovania `collision_unverified`, výsledok môže byť „passed“.<br>• `issues_total` je počet po skrátení.<br>• Skracovanie polí issues aj relácie spolu, takže chyby sa zahodia, aj keď je príliš veľká len časť relácií. | Ak je report skrátený, `state` je najviac `incomplete`. Uviesť aj pôvodný počet. Najprv skracovať relácie. |
| D-4 | S | `ketchup-program/src/lib.rs` (`Report::unfinished`), `ketchup-headless/src/protocol.rs:1086-1110`, `rule_exact_collisions.rs:76-77` | **Varovanie `checks_unverified` je príliš tiché.**<br>• `Report::unfinished` má `ok: true`.<br>• Headless `program_apply` vráti `ok: true` bez súhrnu.<br>• `declared_intent` hlási „passed“ pre očakávania, ktoré sa vôbec nekontrolovali.<br>• Úspešný refine po zastavení môže varovanie zahodiť (nepotvrdené). | `ok: false`, alebo samostatný stav `unverified`. Varovanie nesmie zmiznúť pri refine. Zvážiť, či výrobný export nemá na nekontrolovaný model upozorniť (AGENTS §3 blokovanie exportu dovoľuje). |
| D-6 | S | `live_bridge/plugin_manifest.rs:268` (`embeds_rooted_path`) | **Ochrana argumentov pluginu (R-2) sa dá ďalej obísť.**<br>Pomocou URL (`--import=file:///tmp/x.mjs`, `--loader=file:///…`) alebo response súboru (`@/tmp/args` pre java a dotnet). Nič z toho sa nestaguje ani nehashuje. | Zoznam zakázaných tvarov nikdy nebude úplný. Odmietať každý argument, ktorý obsahuje `=` alebo začína na `@`, okrem bielej listiny tvarov voľieb. Alternatíva: argumenty povoliť len ako samostatné slová, ktoré sú buď súbor v priečinku, alebo nemajú tvar cesty. |
| D-7 | S | `ketchup-viewer/src/app/session.rs` (`keep`), `viewer/android` | **Android: zápis balíka na pozadí môže stratiť komentáre.**<br>• Úpravy komentárov počas zápisu balíka na pozadí čakajú. Ak sa proces ukončí, stratia sa a reštart otvorí predošlý `session.ketchup-view` so zmazanými komentármi.<br>• Poškodený súbor `.new` dá pri každom štarte `Kept` a drží povolenie zápisu do dokumentu navždy.<br>• `(_, Whole)` hlási „Restored“, hoci sa nič nezapísalo. | Komentáre zapisovať hneď vedľa značky nového balíka. Poškodený `.new` po N štartoch zahodiť s hlásením. |
| F-2 | S | `ketchup-model/src/import/dxf.rs:881-1040` | **DXF import berie aj papierový priestor a neviditeľné entity.**<br>• Skupiny 67 (paper space) a 60 (invisible) vyvolajú len všeobecné varovanie a entita sa importuje.<br>• Rám alebo rohové razítko z layoutu sa tak stane profilom v modeli.<br>• Ak zdieľa vrstvu a koncové body s modelom, celý súbor padne na `AmbiguousGeometry`. To isté platí pre DWG. | Preskočiť `67=1` a `60=1` s počtom v diagnostike. |
| F-3 | K | `ketchup-model/src/drawing_export.rs:914`, `:922`, `:955` | **DXF výkresu porušuje špecifikáciu.**<br>• LINE a TEXT majú XDATA (`1001 KETCHUP`) pred súradnicami, hoci podľa špecifikácie DXF musí byť na konci entity. Prísny čítač stratí súradnice alebo súbor odmietne.<br>• Hlavička hlási `AC1027`, ale súbor nemá handles, `BLOCK_RECORD` ani `OBJECTS`.<br>• Test DWG prejde len preto, že acadrust je tolerantný. | XDATA presunúť na koniec (ARC a CIRCLE to už majú). Písať R12 (`AC1009`) ako `manufacturing/dxf_export.rs`, alebo cez `DxfWriter` z acadrust. Export overovať prísnym čítačom (ezdxf) v CI. |

### Nízke

- **D-5:** deadline požiadavky sa ráta od vybratia z frontu, nie od začiatku 45 s spojenia.
  Okno variantov v GUI dostalo MCP rozpočet 40 s.
- **D-8:** pre-push hook kontroluje pracovný strom, nie pushované commity. Predvolene volá
  `python`, ktorý na Debiane chýba. `clippy --workspace` lokálne potrebuje OCCT. Je opt-in
  cez `core.hooksPath`.
- **D-9:** Scale berie holé `150` ako faktor 150 (strop je 1000), kým každé iné pole ako mm.
  Pri faktore nad ~10 by sa hodilo potvrdenie.
- **D-10:** hláška o odmietnutom závite v GUI ukazuje anglický text `CanonicalError` s
  interným id prvku. Závit sa nedá pridať, len meniť. Úpravy diery po pridaní závitu sa
  znova nekontrolujú, odmietne ich až export.
- **D-11:** analytická cesta používa `max(CONTACT_FIT_MM, tolerance)`, worker len
  `CONTACT_FIT_MM`. Pri tolerancii dokumentu nad 0,01 mm sa znova rozídu.
- **F-4:** DXF ELLIPSE s pomerom < 1 sa zahodí len s varovaním, takže obrys sa importuje
  ako otvorený. Treba ju previesť na Bézierove krivky (`ProfileSegment::CubicBezier`
  existuje) alebo súbor odmietnuť.
- **F-5:** text v PDF výkresu (`drawing_export.rs:999`) je o ~28 % menší než v DXF a SVG.
  Chýba delenie výškou verzálok Helvetice 0,718, ako to robí `sheet_pdf.rs`. Kóty 3,5 mm sa
  tlačia 2,5 mm, pod minimom ISO 3098.
- **F-6:** keď sa projektový výkres nezmestí v žiadnej mierke, kreslí sa v najhrubšej mierke
  cez rám aj rohové razítko a hlási úspech (`project_drawings.rs:629,640`).
- **F-7:** profilový DXF export zlúči vrstvy „Plán“ a „Plan“ po odstránení diakritiky
  (`manufacturing/dxf_export.rs:136`).

## Štrukturálne návrhy

1. **Jeden zápis viacerých hodnôt** (D-1, D-2, R-1, N-10). Čiarka je desatinná, hodnoty sa
   oddeľujú `;` alebo `x`, a to v každom poli. Náhľad a vykonanie idú cez jednu funkciu.
2. **Jedny vlastnosti prierezu** (F-1). Jedna štruktúra `SectionProperties` (A, I_y, I_z,
   i_min, W, `rectangular`) pre všetky kontroly EC5. Kontroly platné len pre obdĺžnik
   vrátia pri inom priereze `not_verified`.
3. **Jeden zapisovač DXF** (F-3, F-7). Dnes existujú tri ručne písané (`drawing_export.rs`,
   `manufacturing/dxf_export.rs`, DWG cez acadrust). Spoločný zapisovač vynúti poradie
   skupín, handles a tabuľky pre verziu, a každý export overí prísny čítač v CI.
4. **Strata geometrie pri importe je chyba, nie varovanie** (F-2, F-4). Diagnostiku rozdeliť
   na „geometria sa zmenila alebo stratila“, ktorá v náhľade vyžaduje potvrdenie, a
   „metadáta ignorované“, ktorá ostáva informatívna.
5. **Stav „nekontrolované“ ako prvotriedny výsledok** (D-3, D-4). Skrátený alebo nedokončený
   report nesmie dať `ok: true` ani „passed“.

## Odporúčané poradie

1. **F-1:** nesprávne „vyhovuje“ v statike.
2. **D-1 a D-2** spolu so štrukturálnym návrhom 1.
3. **D-3, D-4.**
4. **F-3, F-2.**
5. **D-6, D-7.**
6. Nízke body.
