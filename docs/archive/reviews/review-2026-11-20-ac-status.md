# Review 2026-11-20 (ac): stav opráv

Review (`docs/archive/reviews/review-2026-11-20-ac.md`, PR #151) bol robený na `6169322c`
(0.3.4-dev). Opravy išli v poradí podľa §7: P1, potom P2, potom P3 po skupinách, každá v
samostatnom PR s testom, ktorý bez opravy padne. Záznamy v `CHANGELOG.md` (Unreleased) sú v tomto PR,
nie v PR s opravami, aby sa PR s kódom nehádali o jeho horné riadky.

Stavy: **opravené** (zlúčené na `main`), **odložené** (s dôvodom), **na rozhodnutie vlastníka**,
**už riešené** (oprava bola na `main` pred review).

## P1

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1120ac-CAM-900 | opravené | #153 | Oblúk, ktorého konce po zaokrúhlení v dialekte splynú pri uhle pod 2π, sa nikdy nezapíše ako G2/G3 s I/J (celý kruh na FANUC, Grbl, LinuxCNC). |
| R1120ac-PG-900 | opravené | #154 | Jeden model tuhosti spoja pre `search` aj `through` (`loads/anchorage/stiffness.rs`): kĺb jednej skrutky nie je votknutie. |
| R1120ac-PG-901 | opravené | #154 | Ukotvenia diafragmy sa sčítajú len po cestách, ktoré nezdieľajú diel. |
| R1120ac-PG-902 | opravené | #154 | Vzperná dĺžka spevnenia pokračuje cez spoj do ukotvenia. |

## P2

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1120ac-PG-903 | opravené | #154 | Šikmé kĺbové spevnenie s tuhosťou a únosnosťou podľa osi prútu. |
| R1120ac-TST-900 | opravené | #154 | Test PG-701 má stĺp priskrutkovaný, takže ho odmietne len pravidlo plochy dotyku. |
| R1120ac-TH-900 | opravené | #157 | Neoverená tepelná kontrola končí exit 3 ako statika. |
| R1120ac-CAM-901 | opravené | #158 | Slepý vrt plným priemerom do hĺbky (s hrotom, pri kužeľovom dne do hrany valca); ploché dno so špicatým vrtákom sa odmietne. |
| R1120ac-CAM-902 | opravené | #158 | Obdĺžniková kapsa má stenový prechod, hrebene 0,32 mm zmizli. |
| R1120ac-ZIP-900 | opravené | #156 | Kontrola balíka prijme len to, čo zapisuje Kečup; extra polia (Unicode Path 0x7075 a ďalšie) sa odmietnu. |
| R1120ac-ZIP-901 | opravené | #156 | Položka „made by Unix“ s módom, ktorý nie je obyčajný súbor (symbolický odkaz), sa odmietne. |
| R1120ac-SKP-900 | opravené | #155 | Umiestnené trojuholníky a natiahnuté kópie sa spočítajú a odmietnu pred kopírovaním sietí. |
| R1120ac-FEA-900 | opravené | #159 | Módy aj na predošlej úrovni siete s relatívnou zmenou a `converged`; `not_checked` (sieť, tuhé podpory, bez tlmenia, bez predpätia). |
| R1120ac-FEA-901 | opravené | #159 | Diel zlepený na dvoch susedných plochách: uzol, ktorý už je viazaný, sa preskočí. |
| R1120ac-FEA-902 | opravené | #159, #161 | Rozpočet tuhosti pred zostavením; výpočet aj módy v rozpočte pamäte (účtovanie alokácií). |
| R1120ac-EXA-900 | opravené | #160 | `inspect surface_control_net` (MCP, SDK); `from_mm` sa porovnáva s toleranciou viazanou na sieť. |
| R1120ac-EXA-901 | opravené | #162 | Odmietnuté vstupy voľných plôch a loftu menujú vlastné pole (`weight`, `keep[i]`, `rail`) s radou. |
| R1120ac-EXA-902 | opravené | #164 | Pomocník AI menuje pole a pravidlo pri každej kontrole plôch, loftu, zaoblení a skosení. |
| R1120ac-EXA-903 | opravené | #163 | Loft s vodiacou krivkou tvaru U: berie sa priesečník najbližší bodu vedenia. |

## P3 (§4)

| Skupina | Stav | PR | Čo |
|---|---|---|---|
| Statika | opravené | #165 | Vzperná sila podľa osi dielu (prút len ohýbaný nevybočí), `--summary` počíta spoje, horná hranica `r` vrstvy vzduchu (0,23 m²K/W), chyby čísiel bez „mm“ pri λ, μ, r, základ výpočtu (R3: spoločné ukotvenie). |
| CAM a výkresy | opravené | #166 | `NOT_CHECKED` o chladení a zápichu bez rampy, zdroje dialektu ISO o `when_fractional`, hlboké vrty z 1 mm nad polotovarom, ISO 1101 znaky (Ø, (M), (L), bez RFS), `losses.csv` bez vzorcov. Príznaky hlavičky ZIP (len bit 11) v #156. |
| FEA | opravené | #167 | Chyby FEA ako rada namiesto Debug textu, výsledok mimo rozsahu čísel (1e300 N) sa odmietne, lepenie len cez plochu (nie hranu), text limitov (vlastná tiaž, votknutá hrana). Zle podmienené modely: text menuje aj zlú podmienenosť. Pamäť módov v rozpočte: #161. |
| Modelovanie a balík | opravené | #168, #171 | `ketchup-program check` odmietne dokončenia, ktoré OCCT odmietne; rada rozpočtu pri jedinom procese; limit cgroup; `build-package.sh` a `CARGO_TARGET_DIR`; `check-package.sh` pod `pipefail`; balík pre Linux pri každom push na `main`. #171: váhy NURBS 1e-6 až 1e6, odmietnuté zaoblenie či skosenie menuje najväčší rozmer, ktorý sa zmestí. |
| Bezpečnosť a R1 | čaká na zlúčenie | #170 | Denník zápisov AI zaznamená sady DXF/DWG, správy o stratách a podpore, `companions`, `takeoff`, `Request::Save {}`, úpravu `apply_and_verify` uloženú na mieste (`save: current`) aj prepis s rovnakou veľkosťou; priečinky Po spustení cez `SHGetKnownFolderPath`; odstránenie záznamu MCP ako vlastník; `HomeError::ChangedWhileRead`; komentár R1. |
| CI a ratchety | opravené | #169, #172 | `test_ci_root_tests.py` číta YAML po jobe, `check_test_timing.py` uvoľní len chyba samotného čakania, `check_user_folder_writes.py` vidí ďalšie formy zápisu, `release_reads.py` celý vzor. #172: `LINT_ALLOW` počíta aj `#[expect(`, `check_no_named_products.py` už nevynecháva `anchorage.rs` a `stiffness.rs`, komentár `persistence.rs` o 0.3.3. |

### Odložené a na rozhodnutie

| Nález | Stav | Dôvod |
|---|---|---|
| `from_mm` bez `serde(default)` (formát 119) | odložené | Formát 119 a riadiaci bod neboli v žiadnom vydaní (0.3.3 ich nemá), takže žiadny uložený súbor pole nemôže postrádať. |
| `overwrite: true` od AI nad súborom exportovaným z ponuky | na rozhodnutie vlastníka | `written_by_ketchup` znamená „zapísal Kečup a nikto ho nezmenil“, nie „zapísala AI“. Review to posiela vlastníkovi (R1). |
| Plochy miestností: bod v hrúbke steny dostane plochu steny | odložené | Bod na čiare steny sa odmieta (R1110 UO-406). Oblasť medzi dvoma čiarami steny sa od úzkej miestnosti nedá odlíšiť bez významu vrstiev; odmietnutie podľa šírky by odmietlo aj komory a chodby. |
| Premenovanie `Kečup.app` na `Ketchup.app` a konfigurácie AI | už riešené | `d6542595`: okno pri otvorení pomenuje klienta AI nastaveného na `/Applications/Kečup.app` a **Use this Ketchup** ho nastaví na novú aplikáciu. |
| Stav „og“ pri R1119og-FEA-801 až 809 | opravené | Riadok 24 stavu „og“ uvádza 805 a 808 ako platné a 807 ako opravené cez R1120ac-FEA-902. |
