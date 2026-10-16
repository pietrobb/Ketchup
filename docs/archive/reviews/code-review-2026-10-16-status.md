# Review 2026-10-15: stav opráv k `cd90cb8`

Nadväzuje na `code-review-2026-10-15-status.md`, ktorý bol overený na `992c0c8`. Odvtedy
pribudlo 59 commitov.

## Metóda

- Traja nezávislí čitatelia po oblastiach. Každý prešiel `git show` opravných commitov aj
  výsledný kód.
- Nálezy so stĺpcom **K** som sám overil čítaním kódu.
- Spustené na `cd90cb8`:
  - všetky `scripts/check_*.py` (vrátane nového `check_ci_crates.py`): prešli;
  - Python testy: 221 prešlo; 11 chýb je len sieťových (proxy prostredia blokuje sťahovanie
    XSD), nesúvisia s kódom.
- CI na `cd90cb8` je zelené (beh 506). Z posledných 30 behov na `main` však 13 prešlo,
  9 zlyhalo a 8 sa zrušilo.

## Súhrn

Zo 16 čiastočných a 2 otvorených bodov z minulého stavu a 12 nových nálezov (N-1 až N-12),
spolu 30 bodov:

| | počet |
|---|---:|
| opravené | 21 |
| čiastočne | 7 |
| neopravené | 2 (SEC-5, N-12) |

K tomu ostáva neopravený zvyšok VW-10, ktorý minulý stav viedol ako opravený.

Pribudlo 6 nových stredných nálezov (R-1 až R-6). Žiadny nový vysoký.

**Najväčšie zlepšenia:**

- **N-1 je opravený správne.** Dotyk znamená, že spoločné teleso nie je hrubšie ako
  kontaktná tolerancia: 2V/A, kde A je plocha z OCCT. Test na skutočnom exact telese: roh
  zarezaný 9,5 mm do steny zásuvky aj tyč dlhšia o 0,4 mm kolidujú, 0,005 mm je dotyk.
  - Zostatková medza je malá: klin 90° prejde do ~0,024 mm, roh kocky do ~0,04 mm a ihla do
    polomeru 0,01 mm.
- **N-2 je opravený.** CI má znova Windows job s OCCT na `windows-latest` s cache buildu
  (20–35 min s cache, 70–83 min bez nej). `ketchup-model` stratil nepotrebnú závislosť na
  OCCT. Pribudli testy na macOS, fuzz pre `load()` a `Package::read_with_comments` a
  kontrola, že každý crate pokrýva nejaký job.
- **Štrukturálne body review sú urobené:**
  - jedna save pipeline (`prepare_document` → `publish_bytes`);
  - typovaný `LoadNotice`, ktorý okno aj session ukážu celý;
  - jeden model gesta `ToolGesture` pre Move, Rotate, Scale, kreslenie a Push/Pull;
  - jedna brána schopností exportérov;
  - BTLx golden súbory validované proti XSD 2.3.1;
  - jedno pravidlo `TappedHole::check`;
  - jeden deadline pre vyhodnotenie, report a refine;
  - strop telies (100k) a hĺbky booleanov (32) na celý model;
  - pamäťový limit workera aj na Unixe (`RLIMIT_AS`).

## Stav otvorených bodov

| ID | Stav | Poznámka |
|---|---|---|
| PS-1 | opravené | Migračný review už program zachová (3318e82). Súbor spred `program.json` si s poškodenou históriou nechá program aktuálnej revízie (06b5367). Nízky zvyšok: ak sa ani ten nedá prečítať, ukáže sa len „história zahodená“, nie že sa stratil program. |
| PS-3, PS-4, N-5 | opravené | PS-4: miss pri textovom kľúči teraz padá na lineárne hľadanie, takže mapa s mnohými neznámymi kľúčmi je znova O(n²). Nízke, len pri úmyselne upravenom súbore. |
| PS-11 | čiastočne | Príkaz mimo svojej rodiny sa odmietne menom. Každá dávka ďalej klonuje celý `ProductModel` (`store.rs:1196`). |
| PS-12 | čiastočne | Programy sa dekódujú len pre ponechané kroky. Všetky záznamy sa stále parsujú. |
| PRG-1 | čiastočne | Refine beží vo workeri. Stále chýba strop „prvých N issues“, takže veľký report sa odmietne celý. |
| PRG-2, PRG-3, PRG-6, PRG-8, EX-1, N-4, N-6, N-7 | opravené | EX-1: rozpočet 256 MiB ráta 16 KiB na plochu, takže veľké voľné plochy z importu podhodnotí. PRG-8: ratchet nechytí konštantu bez prípony `_MM`. |
| PRG-5 | čiastočne | Pozri R-4. |
| N-12 | **neopravené** | **K**: `rule_program.rs:336` a `live_bridge/program_check.rs:22` volajú `plan.report(deadline)?` pred zápisom. Pomalý alebo veľký report tak cez MCP a headless stále zablokuje apply, proti AGENTS §3. Spoločný deadline to ešte zhoršil: report dostane 60 s mínus čas vyhodnotenia. |
| UI-5, UI-6 (Move) | opravené | |
| UI-6 (Scale) | čiastočne | Scale má ďalej vlastný parser: holé `150` je faktor a nepočíta aritmetiku. |
| N-10 | opravené, **s regresiou** R-1 | |
| SEC-5 | **neopravené** | **K**: `consent.rs:273-276` sa nezmenil. Okno spustené cez MCP `open_window` (`launch_attached`, nie `consent_attached`) otvorí aj importuje ľubovoľný súbor bez opýtania. |
| N-8, N-9 | opravené | `--script=../x` sa odmietne pre priečinok aj príkazový riadok. Neexistujúca cesta mimo priečinka sa odmietne. Príkazový riadok beží v prázdnom súkromnom adresári. Pozri R-2. |
| SEC-6 (nízke) | opravené | Súbory mimo priečinka sa pred štartom hashujú znova. Opakovaný nonce sa odmietne. Menu pluginov sa načíta mimo UI vlákna. |
| N-3 | opravené | Hĺbka sa zaokrúhľuje nahor (`ceil`). |
| MF-1 | čiastočne | Aplikácia `SetHoleThread` (`document/threads.rs:185-195`) a `product_validation.rs:188-196` stále kontrolujú len `is_valid()`. Kanonický príkaz tak uloží závit, ktorý každý export odmietne. GUI nemá úpravu ani zrušenie závitu, hoci hláška `digest-scale-refused-thread` radí závit odstrániť. |
| MF-3 | opravené | XSD sa sťahuje v teste, overený SHA-256 (pozri nízke body). |
| VW-10 | **neopravené** | Prvý zápis balíka (až ~264 MB s fsync) ostáva v `update()` na UI vlákne (`viewer/android/src/lib.rs:~201`). |
| VW-11 (Android) | čiastočne | Zápis prerušený ukončením aplikácie sa pri ďalšom štarte vráti späť (81a35b3). Pozri R-3. |
| N-11, fsync na Androide | opravené | |
| N-2 | opravené, zvyšky v R-5 | |

## Nové nálezy

| ID | Závažnosť | Overenie | Kde | Nález | Návrh |
|---|---|---|---|---|---|
| R-1 | stredná (regresia a7c4e47) | K | `ketchup-app/src/length_input.rs:48`, `drawing.rs:3531` | **Rozmery s aritmetikou sa potichu prečítajú zle.**<br>• Pri jednej `*` sa vstup delí najprv podľa nej, až potom podľa čiarky.<br>• Obdĺžnik `2*20,30` (predtým 40 × 30) je teraz 2 × 20,3.<br>• `600+2*18,400` (predtým 636 × 400) je teraz 602 × 18,4. | Ak sa vstup dá rozdeliť čiarkou na platné dĺžky, deliť čiarkou. `*` brať ako oddeľovač len bez čiarky. Test na oba príklady. |
| R-2 | stredná | K | `live_bridge/plugin_manifest.rs:283-293`, `:313-319` | **Absolútna cesta pripojená k voľbe obíde hashovanie.**<br>• `--require=/tmp/evil.js`, `-r/tmp/x` alebo `--x=C:\evil.js` nemajú zložku `RootDir` ani `ParentDir`.<br>• V priečinku `folder.join(arg)` ostane „vnútri“ a argument ide do procesu nezmenený. Na príkazovom riadku sú všetky zložky `Normal`.<br>• Interpret (node, python) otvorí súbor mimo stage, ktorý nikto nehashoval ani neukázal. | Odmietnuť argument, v ktorom za `=` alebo za prefixom voľby nasleduje absolútna cesta, alebo ho rozdeliť a cestu vyriešiť cez `resolve_argument`. Test pre `=` aj pripojený tvar. |
| R-3 | stredná | S | `viewer/android/src/replace.rs` (`recover_interrupted`) | **Obnova prerušeného zápisu na Androide sa môže pokaziť.**<br>• Hocijaký dokument kratší než zaznamenaná nová dĺžka dostane späť staré bajty. Novší a kratší obsah, ktorý tam medzitým uložila iná aplikácia, sa tak prepíše.<br>• Ak je prerušená aj samotná obnova a staré bajty sú dlhšie, záloha sa zmaže, hoci dokument ostal skrátený.<br>• Záznam sa nezapisuje atomicky: roztrhnutý záznam s orezaným URI môže prepísať iný povolený dokument (`…/4` namiesto `…/42`). | • Do záznamu dať aj starú dĺžku a checksum.<br>• Obnoviť len vtedy, keď je dokument prefixom nových alebo starých bajtov, inak zálohu ponechať a nahlásiť.<br>• Záznam zapisovať cez temp súbor a rename, s koncovou značkou. |
| R-4 | stredná | S | `ketchup-model/.../collision.rs:1603`, `pair_query.rs` | **Validátor dokumentu a report programu sa nezhodnú na kolízii.**<br>Validátor hlási kolíziu pri každom objeme > 0, hrúbku ignoruje. Lisovaný spoj 0,005 mm je v reporte programu dotyk, ale vo validátore dokumentu kolízia. To je zvyšok PRG-5. | Použiť ten istý predikát hrúbky (`CONTACT_FIT_MM`) aj tu. |
| R-5 | stredná | S | `.github/workflows/ci.yml`, `scripts/check_ci_crates.py` | **Časť testov nebeží v žiadnom CI jobe.**<br>• `check_ci_crates.py` počíta za pokrytý aj crate, ktorý je len v clippy jobe. Tak 39 testov desktop `ketchup-viewer` nebeží nikde.<br>• Nikde nebežia ani `large_house_*` testy (`--ignored large_house`) ani `house_*` testy len pre release.<br>• AGENTS.md §6 stále zakazuje `-p`, ktoré používa každý CI job. | • Kontrola má vyžadovať job s testami.<br>• Pridať `ketchup-viewer` do testov na Linuxe (lavapipe).<br>• Nočný beh `large_house` v OCCT jobe.<br>• Doplniť výnimku pre CI do AGENTS.md. |
| R-6 | stredná (proces) | S | história CI | **Pravidlo §8 sa pred pushom nedodržiava.**<br>• Polovica opravných commitov sú opravy fmt, clippy, ratchetov alebo limitov riadkov, ktoré sa dajú lokálne overiť za sekundy až minúty: 9493904, 0dfa396, 36eeacb, 06b5367, c109563, 23dbfe9, bc28f8b, cd90cb8.<br>• Pushe idú každých 5–30 minút, teda rýchlejšie, než CI dobehne (~30 min), preto sa veľa behov zruší. | • `scripts/pre-push` (fmt, `check_*.py`, clippy) ako git hook.<br>• Pushovať po blokoch, nie po commitoch, ako to AGENTS §6 aj tak predpisuje. |

**Nízke:**

- **Program a geometria:**
  - Box a exact kontrola sa ďalej líšia: dva čisté boxy s rohom zarezaným 0,03 mm sú
    kolízia, s dierou v jednom z nich je to dotyk.
  - MCP dry-run má deadline 20 s, apply 60 s (`program_dry_run.rs:57-60`), takže môžu dať
    rôzny verdikt.
  - Živý MCP apply má po pláne a reporte (60 s) ďalších 20 s na exact (`program_check.rs:355`).
  - `RLIMIT_AS` 2 GiB ráta aj virtuálne rezervácie. Worker linkuje OCCT, takže ak by OCCT
    alebo TBB raz spustili vlákna, limit sa môže míňať naprázdno.
- **Mirror v GUI:** nie je v registri `ToolGesture`. Po Undo alebo zmene cez MCP napísaný
  offset zrkadlí podľa starej roviny.
- **Pluginy:**
  - `--s=a/../b` v priečinku stratí názov voľby.
  - Na Unixe sa súbor mimo priečinka dá vymeniť medzi kontrolou a `exec`.
  - `used_nonces` rastie počas života okna.
- **Android:**
  - Záloha bez záznamu sa nikdy nezmaže.
  - Povolenia URI sa berú pri každom uložení a neuvoľňujú sa (`ViewerActivity.java:61`).
  - Jeden zlyhaný zápis komentárov označí balík ako neuložený, a ďalšie úpravy potom
    prepisujú celý balík.
- **CI:**
  - Test XSD závisí od troch externých webov bez cache medzi behmi. `w3.org` a `web3d.org`
    cez http sú overené SHA-256.
  - Kľúč cache OCCT neobsahuje verziu Visual Studia.
  - Nie je Rust target cache.
  - Nájdený fuzz pád zafarbí na červeno aj nesúvisiaci PR.

## Odporúčané poradie

1. **SEC-5 a R-2:** posledné dve diery v súhlase so súbormi.
2. **R-1:** tichá zlá hodnota v kreslení.
3. **N-12:** report má hlásiť, nie blokovať zápis.
4. **MF-1:** kontrola v `SetHoleThread` a validácii, a GUI na úpravu alebo zrušenie závitu.
5. **R-3, R-4.**
6. **R-5, R-6:** pre-push hook a testy viewera v CI.
7. **VW-10, PS-11, PRG-1** (strop issues) a nízke body.
