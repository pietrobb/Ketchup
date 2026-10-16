# Podrobný code review Kečupu — 2026-10-07

Stav stromu: `2bf43c7` (`main`). Nadväzuje na `docs/code-review-2026-10-01.md` a `docs/code-review-2026-10-02*.md`. Od 1. 10. pribudlo 107 commitov: statika podľa EC5, nosná cesta, vrstvy a uložené pohľady, výkresy do PDF, demo dom pre Hundegger, rýchla presná kolízia celého domu, MCP server.

Projekt sa za týždeň posunul od „sady overených operácií“ k nástroju, ktorým sa dá navrhnúť a vyrobiť dom. Tento review preto hľadá hlavne chyby, ktoré by pri reálnom použití dali **zlý výsledok bez varovania**: zlá statika, zlé rezy na stroji, stratené dáta.

## 1. Metóda

- Sedem nezávislých čitateľov po oblastiach: program a statika, model a perzistencia, GUI, protokol a bezpečnosť, geometria a OCCT, výroba a exporty, metriky. Každý nález som potom sám prečítal v aktuálnom kóde.
- Spustené: všetky `scripts/check_*.py`, `cargo fmt --check`, `cargo clippy` a celá testovacia sada na Linuxe (výsledky v §9).
- Stĺpec **Overenie** v registri:
  - **K**: mechanizmus som overil čítaním kódu.
  - **D**: navyše ho potvrdzujú čísla z commitnutých dát (golden report).
  - **S**: zatiaľ len statický nález pomocného čitateľa.
  - Žiadny nález nie je overený spustením upraveného kódu; nič v produkčnom kóde som nemenil.
- Priorita:
  - **P1**: môže dať nebezpečný alebo vyrobiteľný zlý výsledok, alebo stratiť dáta.
  - **P2**: konkrétna chyba s menším dopadom.
  - **P3**: dlh alebo hygiena.

## 2. Záver v skratke

**Čo je dobré:** architektúra drží. Dlhy A1 a A2 z review 1. 10. sú splatené:

| | 1. 10. | dnes |
|---|---:|---:|
| `ketchup-app/src/lib.rs` | 36 480 r. | 4 782 r. |
| vlastné kópie `dot` / `cross` / `determinant` | 24 / 20 / 12 | 0 / 0 / 0 |
| `Result<_, String>` | 58 | 2 |
| `.unwrap()` v produkčnom kóde | 40 | 0 |
| modul nad 5 000 riadkov | 1 | 0 |

Presná kolízia nikde nehlási „bez prieniku“, keď sa diely prekrývajú. Každá rýchla cesta, ktorú som prešiel, buď platí, alebo vráti „nevyhodnotené“. Ratchety v CI fungujú a čísla klesajú.

**Čo je zlé:** nové doménové výstupy (statika a výrobné exporty) majú chyby, pri ktorých softvér povie „vyhovuje“ alebo vyrobí súbor, hoci výsledok je zlý:

- **Statika podceňuje zaťaženie stropníc.** Commitnutý report malého domu dáva vnútornej stropnici 0,71 N/mm namiesto približne 1,25 N/mm. Okrajové prahy dostávajú rovnako ako stropnice, hoci majú dostať asi štvrtinu.
- **Za istých podmienok sa ohyb vôbec nekontroluje.** Prvok s dvoma uloženiami na tom istom podpernom diele hlási „pass“, hoci sa skontroloval len otlak.
- **Výrobné exporty majú tri spôsoby, ako vyrobiť zrkadlový alebo posunutý dielec:**
  - zrkadlená kópia v BTLx,
  - profil, ktorého roh nie je v počiatku,
  - nepárna permutácia osí vo woodWOP.
- **CI na `main` je červené** od 4. 10. (`check_test_sleeps.py`). Na Linuxe padá aj clippy. Rust CI beží iba na Windows, takže si to nikto nevšimol.

**Počty:** 9 × P1, 26 × P2 a približne 30 × P3. Bez P0: nenašiel som vzdialený exploit ani bezpodmienečnú stratu dát pri bežnej práci.

## 3. Register nálezov

| ID | P | Oblasť | Nález | Overenie |
|---|---|---|---|---|
| ST-1 | P1 | statika | Zaťaženie dosky sa delí podľa plochy kontaktu, nie zaťažovacej šírky: stropnice dostanú o 40 % menej, prahy 4× viac | K, D |
| ST-2 | P1 | statika | Dve uloženia na tom istom dieli sa zlúčia do jedného lôžka; ohyb, šmyk ani priehyb sa nekontrolujú a výsledok je „pass“ | S |
| ST-3 | P1 | statika | Zaťaženie dielu, ktorý leží len na „nenosných“ dieloch, potichu zmizne | S |
| ST-4 | P1 | statika | Previsy a spojité nosníky: reakcie sú podhodnotené, odťah (uplift) sa nehlási | K |
| MF-1 | P1 | výroba | BTLx exportuje zrkadlenú kópiu ako bežnú kópiu (`Count=2`, jedna sada opracovaní) | K |
| MF-2 | P1 | výroba | BTLx a woodWOP predpokladajú, že profil začína v [0,0]; centrovaný profil posunie rezy mimo dielec | K |
| MF-3 | P1 | výroba | Rámec woodWOP môže byť ľavotočivý (nepárna permutácia osí): vŕtanie je zrkadlené | K |
| MF-4 | P1? | výroba | Konvencie referenčných strán a osí Width/Height v BTLx nie sú overené v reálnom importéri; testy majú len štvorcové prierezy | S, treba potvrdiť |
| CI-1 | P1 | proces | CI na `main` je červené (`check_test_sleeps.py`); Rust CI beží iba na Windows a clippy na Linuxe padá | K |
| ST-5 | P2 | statika | Nosný spoj nikdy neporovná zaťaženie s únosnosťou (`status` je vždy „not_verified“) | K |
| ST-6 | P2 | statika | Spoj bez kontaktu a bez `fasteners` dáva reakciu do ťažiska prvku, nie na jeho koniec | S |
| ST-7 | P2 | statika | Vietor a vodorovné zaťaženia nie sú v rozsahu a výstup to nehovorí; stĺpiky „pass“ bez kombinácie 6.23/6.24 | K |
| GUI-1 | P2 | GUI | Shift-klik odznačí diel zo skupiny, ale Move/Rotate stále pohne celou skupinou | K |
| GUI-2 | P2 | GUI | Súhrn programu hlási `state: passed`, aj keď `load_capacity.state` je `failed` | K |
| GUI-3 | P2 | GUI | Fillet/Chamfer na programovom diele: k+1 synchrónnych vyhodnotení programu na UI vlákne a chyba vyhodnotenia sa skryje | S |
| GUI-4 | P2 | GUI | Push/Pull na programovom modeli vyhodnotí celý program synchrónne pri prvom ťahu aj pri pustení | S |
| GUI-5 | P2 | GUI | Náhľad Face Offset serializuje a znova načíta celý dokument na UI vlákne pri každej zmene vzdialenosti | S |
| GUI-6 | P2 | GUI | Skryté diely ostanú vybrané po skrytí vrstvy alebo prepnutí scény; Delete zmaže neviditeľné diely | S |
| MD-1 | P2 | model | Uložiť sa dá dokument, ktorý sa nedá otvoriť (zápis do 64 MB, čítanie do 32 MB); história drží plnú kópiu každej revízie | K |
| MD-2 | P2 | model | Schválený návrh AI môže potichu zmazať kontaktné spoje a uzemnenie pridané po schválení | S |
| MD-3 | P2 | model | Neznáme polia sa v dokumente ignorujú, ale história potom neprejde (`InvalidRevisionHistory`); serde zmena bez zvýšenia formátu | S |
| GE-1 | P2 | geometria | Kontaktný `BRepAlgoAPI_Common` beží dvakrát a prvý beh je deštruktívny nad zdieľanými cache tvarmi | K |
| GE-2 | P2 | geometria | Validátor celého modelu má limit 512 výskytov vrátane skrytých: na dome kolízia, gravitácia a súvislosť nikdy nebežia | K |
| GE-3 | P2 | geometria | „Kontakt pred vzdialenosťou“ prijme rovnobežné plochy vzdialené až o súčet tolerancií plôch ako dotyk s nulovou vzdialenosťou | K |
| PR-1 | P2 | protokol | Neautentifikovaný klient pošle 8 MiB rámec, ktorý sa celý rozparsuje do `serde_json::Value` (stovky MiB na spojenie) | K |
| PR-2 | P2 | protokol | `export_drawings` prepíše ľubovoľný `.pdf` bez súhlasu a zmení dokument mimo `apply_batch` | K |
| PR-3 | P2 | protokol | MCP staging zmaže každý cudzí podpriečinok v `KETCHUP_MCP_STAGE_DIR` | S |
| PR-4 | P2 | protokol | Znovu použitý staged build sa spustí bez overenia (predvídateľný kľúč, predvolené práva) | S |
| MF-5 | P2 | výroba | Produkčný BTLx závisí od viditeľnosti vrstiev: skrytá strecha v súbore potichu chýba | K |
| MF-6 | P2 | výroba | Predvolená stratégia BTLx v aplikácii na demo dome zlyhá a chyba nemenuje prvok | S |
| MF-7 | P2 | výroba | Booleovský rez hranolom nemá normalizáciu vstupnej plochy ako profilové rezy | S |
| MF-8 | P2 | výroba | „Pôdorys“ domu je rez −112 mm, teda základy; výšku rezu nemožno nastaviť | S |
| MF-9 | P2 | výroba | Výkres potichu vynechá diely bez aktuálneho presného telesa | S |
| MF-10 | P2 | výroba | Výkaz materiálu domu má riadok na každý prvok (~1 850 riadkov) | S |
| MF-11 | P2 | výroba | `ExportBlocked` má 56 príčin bez dielu a návodu; poškodené nastavenia výkresu sa potichu prepíšu predvolenými | S |

P3 nálezy sú v §4 pri jednotlivých oblastiach.

## 4. Nálezy podrobne

### 4.1 Statika a nosná cesta (`crates/ketchup-program`)

Kontrola EC5 je napísaná poctivo. Sedí `kmod`, `kdef`, `γM`, `kh`, `kcr = 0,67`, `kv`, `kc` s `λrel` a `βc`, `wfin` aj kombinácie a ψ podľa EN 1990. Chýbajúce vstupy sa šíria ako „not_verified“, nikdy ako „pass“. **Problém je v tom, čo do týchto vzorcov vstupuje.**

**ST-1 (P1): rozdelenie zaťaženia dosky** — `loads.rs:415-428` (`sheet_shares`, vetva `own`)

Vlastná tiaž dosky a všetky plošné zaťaženia (úžitkové, sneh, strecha) idú na podpery v pomere `polygon_area(contact)`. Plocha kontaktu nehovorí nič o tom, aký pás dosky podpera nesie. Prah pozdĺž okraja sa dotýka celou dĺžkou, takže dostane veľa, hoci nesie len pás nad sebou.

Dôkaz z commitnutého `examples/programs/tiny-house.report.json` (strop, úžitkové 2,0 kN/m², stropnice á 625 mm):

| prvok | smer | úžitkové N/mm | očakávané |
|---|---|---:|---:|
| `stropnice/stud 20…23` (vnútorné) | y | 0,71 | ≈ 2,0 × 0,625 = **1,25** |
| `stropnice/bottom plate`, `top plate` (okraj) | x | 0,62–0,65 | ≈ 2,0 × 0,08 = **0,16** |

Vnútorná stropnica je podhodnotená o približne 43 %. Teraz je stav „not_verified“ len preto, že demo nemá zadaný sneh (`snow_sk = 0`). Po jeho zadaní by stropnice prešli s polovičným zaťažením.

**Oprava:** rozdeliť zaťaženie podľa zaťažovacej plochy v pôdoryse (napr. raster 25–50 mm, každá bunka k najbližšiemu kontaktnému polygónu), alebo podľa smeru nosnosti dosky. Pridať test s nerovnakou osovou vzdialenosťou a okrajovým prahom.

**ST-2 (P1, S): zlúčené uloženia** — `loads.rs:296-312` (`grouped`), `member_check.rs:642-653`

Všetky kontakty prvku s jedným podperným dielom sa spoja do jednej reakcie od prvého po posledný bod. Ak je táto zóna dlhšia ako `BED_MM = 400`, prvok sa berie ako spojito podopretý: nevznikne žiadny úsek `Span`, takže sa nekontroluje ohyb, šmyk ani priehyb. Ostane len otlak a výsledok je „pass“.

Scenár: 4 m trám na obvodovom základe alebo prstencovom prahu modelovanom ako jeden diel s otvorom. Pri 45×145 a 2 kN/m² je skutočné využitie ≈ 1,5, softvér hlási „pass“.

Oprava: zhlukovať kontakty po súvislých plôškach, nie po podpernom diele. Pre nosník s dvoma oddelenými uloženiami vyžadovať aspoň jednu kontrolu rozpätia.

**ST-3 (P1, S): stratené zaťaženie** — `loads.rs:624-635`, `:699-702`

Nenosný diel (napr. krytina), ktorý leží len na dieloch, ktoré nie sú `only=` prvky ani `carriers=` (napr. laty), nemá podpery. Dôvod sa zapíše do `missing`, ale nikam sa neodovzdá, lebo nie je cieľ. Sneh a krytina zmiznú a krokvy hlásia „pass“.

Oprava: dôvod pripísať všetkým prvkom, ktorých sa diel dotýka, alebo ho vrátiť ako `loads.unassigned`, ktorý prepne `load_capacity` na `incomplete`.

**ST-4 (P1): previsy a spojitosť** — `loads.rs:355-358` (`beam_shares`)

Zaťaženie za poslednou podperou ide celé na ňu. Vnútorné podpery dostanú podiel ako pri prostom nosníku:

| prípad | softvér | statika | odchýlka |
|---|---|---|---|
| podpery 0 a 3000, bremeno P v 4500 | R₃₀₀₀ = P, R₀ = 0 | R₃₀₀₀ = 1,5 P, R₀ = −0,5 P | −33 %, odťah sa nehlási |
| dve rovnaké spojité polia | stredná reakcia 1,0 wL | 1,25 wL | −20 % |
| šmyk pri dvoch spojitých poliach | 0,5 wL | 0,625 wL | −20 % |

Na krokvách demo domu (previs 500 mm) to robí asi 5 %. Pri balkónoch a konzolách je to podstatné.

Oprava: jeden 1D riešič spojitého nosníka na prvok, zdieľaný pre odovzdanie zaťaženia aj pre momenty a šmyk v `member_check`. Zápornú reakciu hlásiť ako „uplift“.

**ST-5 (P2): únosnosť spoja** — `joint_check.rs:37-61`

`status` je napevno „not_verified“, aj keď je známa únosnosť aj zaťaženie. Závesný kus s únosnosťou 5 kN pri zaťažení 20 kN nikdy nezlyhá. Navyše `load_capacity` modelu s akýmkoľvek nosným spojom nemôže byť nikdy „passed“.

Oprava: porovnať podľa `basis`:
- `allowable` porovnať s charakteristickým súčtom,
- `characteristic` porovnať s `kmod·Rk/1,3`,
- `design` porovnať priamo.

**ST-6 (P2, S): reakcia spoja bez kontaktu**

Pre spoj bez kontaktu a bez `fasteners=` dáva `loads.rs:531-535` reakciu do ťažiska prvku, kým `load_path.rs:258-268` použije prekrytie obálok. Trám so závesom pri 2 mm medzere sa tak počíta ako 2 m pole plus 2 m konzola.

**ST-7 (P2): rozsah výpočtu**

`BASIS` (`member_check.rs:135-144`) nehovorí, že vietor a vodorovná stabilita sú mimo rozsahu. Obvodové stĺpiky sa kontrolujú len na osovú silu, bez kombinácie 6.23/6.24. Kým sa to nedoplní, `load_capacity` treba označiť ako „len gravitačné“.

**P3:**
- NaN využitie vedie na „pass“ (`member_check.rs:579-591`).
- Smer nosného spoja je v `load_path.rs:270-277` symetrický, ale v `loads.rs:539` orientovaný.
- `validate.rs:566-590` (`support`) je O(n²) v každom priechode.
- Kontakty sa počítajú dvakrát (`loads.rs:480`).
- `model.part(name)` je lineárne vyhľadávanie (`model.rs:1646`).
- Testy: jediný ručne prepočítaný nosník. Chýba `kv`, `kc`, `kh < 150`, sneh spolu s úžitkovým, trieda použitia 3, konzola, spojitý nosník a doska na nerovnakých podperách.
- Test domu tvrdí, že všetky prvky sú „pass“, čo by prešlo, aj keby sa kontroly potichu preskočili.

### 4.2 Výroba a exporty (`crates/ketchup-manufacturing`, výkresy)

Exporty sú inak napísané opatrne:
- každá operácia musí byť spotrebovaná,
- nevyriešené zdroje blokujú export,
- čísla sa formátujú deterministicky (`{:.9}`, bez exponentu, nezávisle od lokálu),
- `sheet_pdf` vkladá TrueType font s ToUnicode, takže slovenčina sa tlačí aj vyhľadáva.

Problémy sú v **rámcoch a v tom, čo sa do exportu dostane**.

**MF-1 (P1): zrkadlená kópia v BTLx**
- `project_general_fabrication` (`fabrication.rs:2752`) kontroluje len `is_rigid_transform`, ktorá prijme aj determinant −1.
- BOM spojí originál a zrkadlo do jedného riadku.
- `btlx_2_3_1_export_with_options` (`:1143`) na rozdiel od woodWOP (`:1080`) nevolá `is_production_transform` pre každú inštanciu.

Krokva s výrezom na jednej strane a jej zrkadlová kópia (Mirror používa tú istú definíciu) tak vyjde ako `Count="2"` s jednou sadou opracovaní. Stroj vyrobí dve ľavé krokvy.

Oprava: v BTLx overiť každú inštanciu, alebo v BOM rozdeliť riadky podľa orientácie (ľavá/pravá).

**MF-2 (P1): roh profilu v počiatku**

`rectangular_stock_profile_dimensions` (`fabrication.rs:2299-2372`) overí obdĺžnik zarovnaný s osami, ale nie to, že jeho minimum je [0,0]. `btlx_part_coordinate` (`:1830`) potom kopíruje súradnice definície rovno do súradníc dielca. `production_job` to robí správne: odpočíta minimálny roh.

Profil `[(-30,-70)…(30,70)]` s kapsou v lokálnom y 0..70 dá v BTLx kapsu v Z 0..70, teda v dolnej polovici namiesto hornej. Kapsa v y −70..−60 skončí mimo polotovaru. Demo domu sa to netýka, lebo `box()` začína v nule.

Oprava: jeden spoločný „stock frame“ s minimom v počiatku pre BTLx, woodWOP aj `production_job`.

**MF-3 (P1): ľavotočivý rámec woodWOP**

`woodwop_stock_frame` (`fabrication.rs:1877-1897`) zoradí osi podľa rozmeru a súradnice len poprehadzuje. Nepárna permutácia je zrkadlenie. Bočnica skrinky `box((19,400,600))` dá poradie osí [z, y, x], teda nepárne, a vŕtanie vyjde zrkadlovo spredu dozadu. Test pokrýva len 100×50×1000, čo je párna permutácia.

Oprava: pri nepárnej permutácii otočiť jednu os (y → rozmer − y).

**MF-4 (P1, treba potvrdiť): konvencie BTLx**

Vlastný support report hovorí `concrete_importer_verified=false`. Podľa konvencií iných zapisovačov BTLx (compas_timber) môžu byť dve veci obrátene:
- smer normály referenčnej roviny (Ketchup: X×Y do materiálu),
- priradenie Width/Height k osiam dielca.

Fixtures sú kocka 10×10×10 a prierez 100×50, takže chybu by neodhalili.

Odporúčanie: skontrolovať jeden prvok 60×140×2000 v BTLx prehliadači alebo importéri, konvenciu zapísať do `docs/` a pripnúť golden testom s neštvorcovým prierezom. Pridať test, že každé opracovanie leží v polotovare.

**MF-5 (P2): viditeľnosť vrstiev**

`fabrication.rs:2736-2744` filtruje `occurrence.visible` vrátane viditeľnosti vrstiev. Používateľ si skryje strechu, aby videl stropy, vyexportuje BTLx a strecha v súbore chýba bez varovania. Produkčný export musí brať všetky prvky, alebo zablokovať s počtom skrytých.

**MF-6 až MF-11 (P2, S):**
- Predvolená stratégia `EdgeSawCutsThenMillContour` (`app/shell.rs:84`) neprijme trojuholníkové šikmé zarezanie domu a chyba nepovie, ktorý prvok to je. Release test prechádza len preto, že používa `PortableFreeContour`.
- Booleovský rez hranolom nemá normalizáciu vstupnej plochy (`fabrication.rs:3099`).
- Rez pôdorysu je napevno v najnižšom bode + 1 200 mm (`project_drawings.rs:720`). Pri dome so základmi od −1 312 mm to dá −112 mm, teda základy.
- Výkres potichu vynechá diely bez aktuálneho presného telesa (`app/project_drawings.rs:52`). Export pritom smie blokovať.
- Kategória vo výkaze materiálu je text do prvého `/` (`takeoff.rs:84`). Názvy domu `/` nemajú, preto vznikne ~1 850 riadkov.
- `GeneralFabricationError::ExportBlocked` má 56 miest volania bez dielu a návodu.
- `stored_sheet_settings` (`app/project_drawings.rs:107`) zmení poškodený JSON na predvolené hodnoty a ďalšie uloženie ho prepíše.

**P3:**
- Neescapovaný `material_key` v SVG (`fabrication.rs:789`).
- `cut_at_mm` stráca znamienko (`project_drawings.rs:482`).
- BTLx `Designation="definition-N"` a konštantný `Material`: operátor nespáruje dielec s „krokva 012“ a stratí sa trieda C24.
- Dva PDF zapisovače. `drawing_export.rs` používa nevložený Helvetica so zoznamom 36 diakritických znakov; zlúčiť na `sheet_pdf.rs`.
- Mierka 1:50 a výška rezu 1,2 m sú napevno v Ruste; patria do nastavení.
- Release test domu kontroluje 1 813 `<Part>` bez geometrickej asercie (commit hovorí o 1 731 prvkoch).

### 4.3 GUI (`crates/ketchup-app`)

**GUI-1 (P2): odznačenie dielu zo skupiny** — `lib.rs:2086-2099`

Vetva `select_exact`, ktorá diel odoberá, vráti skôr, než vynuluje `selected_group`. Vynuluje ho len vetva, ktorá pridáva (`:2111`). `transform_request` (`app/transform.rs:1663`) potom pri nastavenom `selected_group` hýbe celou skupinou.

Scenár: klik na stĺpik vyberie skupinu „Stena A“, Shift-klik stĺpik odznačí, potom Move. Pohne sa celá skupina vrátane odznačeného stĺpika a Delete neurobí nič.

Oprava: jeden riadok `self.selected_group = None;` v odoberajúcej vetve, plus headless test.

**GUI-2 (P2): súhrn programu** — `live_bridge/program_validation_summary.rs:14` vs `:44`

Hlavný `state` sa počíta len z `report.errors`, presnosti a očakávaní. Zlyhanie únosnosti je v `report.design` a do `errors` sa nedostane. AI, ktorá číta prvý riadok, ohlási úspech pri preťaženom preklade. Oprava: `load_capacity` zahrnúť do hlavného stavu.

**GUI-3 až GUI-5 (P2, S): synchrónne vyhodnotenia na UI vlákne**
- Fillet alebo Chamfer na k hranách programového dielu urobí k+1 celých vyhodnotení programu (`program_edit.rs:163` → `program_pick.rs:125`). Chyba vyhodnotenia sa cez `.ok()?` zmení na „hrana nemá meno“.
- Push/Pull nakresleného tvaru na programovom modeli vyhodnotí program pri prvom ťahu aj pri pustení (`drawn_shape.rs:444-460`, `:887`). Testy na to majú rozpočet 1 s na snímok.
- Náhľad Face Offset robí `save_container → load → into_editable` celého dokumentu pri každej zmene vzdialenosti (`planar_push_pull.rs:471-475`). Štyri príčiny chýb zahodí `.ok()` a digest je napevno po anglicky.
- Spoločná oprava: **jedna služba vyhodnocovania programu** s cache podľa zdrojáku a override hodnôt, ktorá beží na plánovacom vlákne s revíziou, ako to už robí `ProgramPlanJob` pre live bridge.

**GUI-6 (P2, S): skryté diely vo výbere**

`reconcile_selection` (`selection.rs:977`) nefiltruje podľa viditeľnosti. Výber strechy, prepnutie na scénu „Prízemie“ a Delete zmaže neviditeľné krokvy. Oprava: pri zmene viditeľnosti skryté diely z výberu vyradiť, alebo zobraziť „N skrytých vybraných“ a deštruktívne príkazy odmietnuť.

**P3:**
- `has_preview()` stavia celý `PreviewCheckKey` (klony náhľadu, príkazov a výberu) pri každom volaní, teda na box a na plochu v každom snímku (`drawing.rs:1972`).
- Move klonuje celý render plan a v každom snímku znova nahrá inštančný buffer (`renderer.rs:232`, `:976`).
- Dock vrstiev volá `scene_query()` dvakrát za snímok (`assistant.rs:3263`, `:3271`).
- Cache `push_pull.program_evaluation` sa pri New/Open nevyprázdni.
- Predĺženie zapíše `Dimension::new(length.to_string())` (`transform.rs:3054`). Parametrická dĺžka sa zmení na literál a v histórii sa objaví `133.29999999999998`.
- Záložky scén:
  - dvojklik na premenovanie najprv aktivuje scénu (kamera plus krok Undo),
  - Escape v poli zároveň zruší výber,
  - duplicitný názov sa zahodí bez správy,
  - Undo vráti vrstvy, ale záložka ostane aktívna.
- Tlačidlá `↻` a `+` nemajú hover nápovedu.
- `DrawnShapeRefusal` je anglický text v digeste, nie `Rejection`.
- `viewport.rs` (4 583 r.) a `drawing.rs` (3 470 r.) rastú k limitu 5 000.

### 4.4 Model a perzistencia (`crates/ketchup-model`, `ketchup-pdm`)

Atomicita batchov a nálezy CR-04 až CR-08 z 2. 10. ostali opravené: kandidát sa klonuje a pri chybe zahodí, cykly skupín sa odhalia a recovery používa compare-and-swap.

**MD-1 (P2): veľké dokumenty** — `persistence.rs:648` vs `:1648`
- Uloženie povolí `document.bin` až po limit kontajnera (64 MB), načítanie odmietne čokoľvek nad `MAX_SNAPSHOT_BYTES` (32 MB).
- História ukladá plnú kópiu každej revízie, aj tej aktuálnej. Pri dome je `history.bin` 9,4 MB vedľa 8,7 MB `document.bin`.
- Odvodené dáta (face-reference evidence) tvoria 3,9 MB z 8,7 MB snapshotu.

Dôsledky:
- PDM `create_release` uspeje a `open_release` zlyhá s `ResourceLimit`; release sa už nedá otvoriť.
- Pri dome už po troch úpravách (aj prepnutiach vrstiev) Save ponúkne „uložiť len aktuálny stav“ a zahodí Undo.

Oprava:
- kontrolovať limit už pri zápise, s veľkosťou v chybe,
- aktuálnu revíziu uložiť odkazom,
- odvodené dáta neukladať,
- revízie, ktoré menia len zobrazenie, zlučovať.

**MD-2 (P2, S): schválený návrh AI** — `proposal_analysis.rs:722-734`, `:810-813`; `store.rs:3030-3031`

Na konci batchu sa prerežú kontaktné spoje a uzemnenia, ktoré už neukazujú na existujúce diely. Tieto sekcie však nie sú v zozname toho, čo príkaz mení. Postup: návrh `DeleteOccurrence(7)` sa overí v R0, používateľ v R1 pridá kontaktný spoj 7↔8, potom sa návrh potvrdí. Spoj zmizne bez hlásenia.

Oprava: register všetkých kolekcií, ktoré držia cestu k inštancii, s politikou (prerezať alebo blokovať), a z neho odvodiť závislosti.

**MD-3 (P2, S): vývoj schémy** — `snapshot_codec.rs:85`, `snapshot_encoding.rs:3-17`, `persistence.rs:1610`

`ProductModel` neznáme kľúče ticho zahodí (`flatten` vylučuje `deny_unknown_fields`), ale história musí po prekódovaní dať identické CBOR. Ak niekto pridá `#[serde(default)]` pole bez `skip_serializing_if`, každý súbor formátu 102 s históriou sa prestane otvárať s chybou „revision history is invalid“.

Commit 09bf063 navyše presunul `grounded_instances` bez zvýšenia formátu, čím sa zmenil digest súborov zapísaných 97ae1dd.

Oprava: politika vývoja schémy (pri každej serde zmene zvýšiť formát), golden testy starých súborov formátu 102, a pri nezhode histórie otvoriť dokument bez histórie s upozornením namiesto odmietnutia.

**P3:**
- Chyby bez cieľa:
  - `InvalidSavedView` zbiera 7 príčin pod jeden kód,
  - kontaktné spoje a uzemnenie nemenujú cestu,
  - `TagInUse` nemenuje držiteľa,
  - `InvalidPayload(error.to_string())` ratchet nevidí.
- Neexistuje príkaz na zmenu vrstiev vnoreného dielu, takže vrstvu priradenú programom nemožno zmazať.
- Celý model sa spracuje pri každom príkaze, aj pri prepnutí vrstvy:
  - porovnanie všetkých features (`Feature` nie je `Eq`, chýba `Arc::ptr_eq`),
  - dvakrát graf závislostí,
  - plná validácia,
  - CBOR celého modelu pre digest a recovery.

  Na dome je to 1 854 výskytov a 15 950 features.
- Načítanie skompiluje presný graf pre každý z 5 193 záznamov evidence (`persistence.rs:1753`) a každú revíziu validuje dvakrát.
- `apply_batch_with_origin_and_validation` má 2 103 riadkov a `store.rs` narástol na 4 052. `saved_view_commands.rs` ukazuje správny vzor delenia.

### 4.5 Geometria a OCCT (`ketchup-application`, `ketchup-scheduler`, `ketchup-exact`)

Kolíziu som prešiel celú:
- certifikované obálky (oblúky, kubiky, otvory),
- BVH v svetových súradniciach s vnorenými inštanciami,
- SAT na obálke hranolu,
- vynechanie natívneho Common pri tenkej doske,
- cache výsledkov párov.

Žiadna z týchto ciest nevie povedať „bez prieniku“ pri skutočnom prekrytí. Pokrytie je fail-closed: „pass“ len ak `checked == total_pairs`. To je výborná práca.

**GE-1 (P2): dvojitý a deštruktívny Boolean** — `native_booleans.cc:448-450`

`BRepAlgoAPI_Common face_common(a, b);` už v konštruktore spustí `Build()` v predvolenom **deštruktívnom** režime. Až potom kód nastaví `SetNonDestructive(true)` a buduje znova. Prvý beh môže zväčšiť tolerancie podtvarov zdieľaných telies z `GRAPH_OUTPUT_CACHE`. Výsledok páru potom závisí od toho, čo worker počítal predtým, čo porušuje predpoklad `PAIR_RESULT_CACHE`. Na horúcej ceste to navyše stojí dvojnásobok času.

To isté sa týka `query_face_pair_native` (`:250-251`, `Perform()` dvakrát) a ďalších dvojargumentových konštruktorov (`native_booleans.cc:110/155/166/181`, `native_naming.cc:373-380`, `native_finish.cc:428/445`).

Oprava: predvolený konštruktor, potom `SetArguments`, `SetTools`, `SetNonDestructive(true)` a jeden `Build()`, ako to už robí `:333-341`. K tomu jeden pomocník v C++ a test, ktorý zamieša poradie párov.

**GE-2 (P2): validátor nad celým modelom** — `collision.rs:63`, `:920-940`

`MAX_COLLISION_BODIES = 512` počíta všetky výskyty vrátane skrytých. Panel validátorov, session aj Asistent preto na dome vždy dostanú pre kolíziu, gravitáciu a súvislosť `not_evaluated: collision_scene_resource_limit`. Dom skontroluje iba výrobný export (scoped, limit 10 000).

Je to v rozpore s AGENTS §3 („validácia beží lokálne nad zmenenými dielmi a susedmi“). Oprava: všetky vstupné body cez `CollisionScope` zo zmenených dielov a ich susedov, celý model len na výslovnú požiadavku, a limit len na viditeľné.

**GE-3 (P2): kontakt pred vzdialenosťou** — `native_booleans.cc:439-470`

Brána rovnobežnosti pustí roviny vzdialené až o `tol_L + tol_R + 1e-7` mm. Fuzzy Common potom nájde plochu a vzdialenosť sa nastaví na 0 bez toho, aby sa zmerala. Pri importovaných alebo opravovaných telesách (tolerancie plôch 1e-3 až 1e-2 mm) sa tak plochy vzdialené 0,005 mm hlásia ako „Touching, 0 mm, verified“ a gravitácia ich počíta ako uloženie.

Oprava: bránu viazať len na `linear_mm`, a pri väčšej tolerancii plochy najprv zmerať vzdialenosť.

**P3:**
- Ratchet hygieny nevidí `Variant(error.to_string())`; takých miest je 139 (37 len v `ketchup-scheduler/src/lib.rs`).
- Tolerancia aproximácie kubík je relatívna (1e-6 × dĺžka), ale stráž priesečníka je absolútna 3e-6 mm (`region.rs:394` vs `:180`).
- Zlúčenie švov Face Offset hádže výnimku na singulárnom bode namiesto návratu k nezlúčenému tvaru (`native_finish.cc`, `Normalized()`).
- `collision_report` má 786 riadkov a 8 kópií bloku zrušenia.
- Z 12 workerov párovej kontroly ostanú teplé len 2 (`worker_pool.rs:9`).

### 4.6 Protokol, MCP a bezpečnosť

PR-01 až PR-05 z 2. 10. ostali opravené:
- bootstrap cez súkromný register,
- zrušenie a limit vyhodnocovania,
- `expected` stráže,
- výber okna z vlastného stdout,
- ohraničené čítanie zdrojáku.

**PR-1 (P2): rámec pred autentifikáciou** — `live_bridge/transport.rs:132-137`, `:230`, `:352-359`

`RawEnvelope.request` je `serde_json::Value`, takže celý rámec sa rozparsuje do stromu ešte pred kontrolou tokenu. Od ce9d7ec má rámec až 8 MiB.

Ľubovoľný lokálny proces, aj iného používateľa, otvorí 4 spojenia (`MAX_CONNECTIONS`) a pošle `[0,0,0,…]`. To sú približne 4 milióny `Value` po 32 B na spojenie, spolu 0,5–1 GiB. Zároveň obsadí všetky spojenia pre skutočného MCP klienta.

Oprava: `Box<RawValue>` (funkcia `raw_value`) a do prvej autentifikácie limit rámca približne 64 KiB.

**PR-2 (P2): `export_drawings`** — `live_bridge/drawings.rs:53-96`, `app/project_drawings.rs:234`
- Cesta musí byť len absolútna a končiť na `.pdf`. Potom `std::fs::write` nasleduje symlinky, prepíše cieľ a nikoho sa nepýta.
- `SaveAs` naproti tomu pýta súhlas a zapisuje atomicky.
- `store_sheet_settings` mení `container_data` pred zápisom, mimo `apply_batch`, bez Undo, a zmena ostane aj pri zlyhaní zápisu.

Oprava: jedna spoločná politika ciest a zápisov:
- lokálna absolútna cesta, bez UNC, zariadení a ADS,
- súhlas pred prepísaním,
- zápis cez dočasný súbor a premenovanie,
- nastavenia uložiť až po úspechu.

**PR-3, PR-4 (P2, S): MCP staging** — `ketchup-mcp/src/stage.rs`
- `remove_stale` (`:120-135`) zmaže každý podpriečinok koreňa okrem aktuálneho kľúča. Ak si používateľ nastaví `KETCHUP_MCP_STAGE_DIR` na `%TEMP%` alebo priečinok projektov, prvé `open_window` zmaže všetko vedľa. Mazať treba len priečinky s vlastnou značkou.
- Znovu použitý build sa spustí bez overenia. Kľúč (FNV z cesty, času a veľkosti) je predvídateľný a koreň má predvolené práva. Pri dodanom `.cmd` je to priečinok toho istého používateľa, takže to dnes nie je zneužiteľné.

**P3:**
- Opustená žiadosť o pripojenie neskôr odpojí aktuálneho klienta (`consent.rs:532-546`).
- `await_response` pred `TimedOut` naposledy neskontroluje kanál (`transport.rs:320`).
- MCP čaká `timeout + 20 s`, okno `max(timeout + 15 s, 30 s)`, takže pri `timeout_ms` pod 10 s sa MCP vzdá skôr.
- Štart okna má limit 30 s, kým `open` povolí 120 s.
- `.cmd` porovnáva build len podľa veľkosti a minúty.
- Python SDK má vlastné limity: kategória 128 vs 1 024 v Ruste (`ketchup_assistant_protocol.py:1907`) a vlastné texty chýb.
- `open_rejected` a `save_rejected` zlučujú tri rôzne príčiny.
- UNC cesty: `Open` volá `is_file()` pred dialógom súhlasu, takže SMB autentifikácia odíde na cudzí server bez interakcie.

### 4.7 Build, CI a repozitár

**CI-1 (P1, proces): CI je červené a Linux sa netestuje**
- `python3 scripts/check_test_sleeps.py` padá od 4. 10.:

  | súbor | sleep | povolené |
  |---|---:|---:|
  | `interactive_house_tests.rs` | 1 | 0 |
  | `project_drawings_tests.rs` | 1 | 0 |
  | `push_pull_house_tests.rs` | 2 | 0 |
  | `tag_toggle_tests.rs` | 1 | 0 |
  | `tests/harness/mod.rs` | 3 | 2 |

  Unit testy v `src/app/*_tests.rs` nevidia `wait_until` z harnessu, preto každý píše vlastnú slučku `step(); sleep()`.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` padá na Linuxe: `path_with_suffix` a `release_descendant` v `ketchup-scheduler/tests/assistant_process.rs` sa používajú len pod `#[cfg(windows)]`.
- Rust job CI beží len na self-hosted Windows runneri, takže Linux (a vývojár bez Windows) nemá žiadnu kontrolu.

Oprava:
- `#[cfg(test)]` pomocník `wait_for` v `ketchup-app`, ktorý odstráni sleepy,
- `run_idle` v harnesse cez `wait_until`,
- `#[cfg(windows)]` na dvoch funkciách,
- lacný Linux job s `cargo check` a `clippy` (stačí bez OCCT testov, `build.rs` už berie `KETCHUP_OCCT_ROOT`).

**P3:**
- `.gitignore` ignoruje `*.work-recovery`, ale nie `*.work-recovery-lock`. Jeden už bol commitnutý omylom (c4b7b5f).
- `examples/tiny-house.ketchup` (24,8 MB) a `house-project.ketchup` (18 MB) sú v gite ako binárky.
- Golden `*.report.json` tvoria asi 70 % všetkých zmenených riadkov za týždeň: `tiny-house.report.json` bol prepísaný 11×, 373 000 riadkov.

  Návrh: report rozdeliť na stabilnú súhrnnú časť (golden) a detail, ktorý sa generuje na požiadanie. Veľké dokumenty generovať z programu, alebo použiť Git LFS.

## 5. Metriky (1. 10. → dnes)

| metrika | 1. 10. | dnes |
|---|---:|---:|
| sledované riadky (Rust, C++, Python, Starlark) | 428 135 | 475 852 |
| Starlark (doménová knižnica a programy) | 1 070 | 7 850 |
| `ketchup-app/src/lib.rs` | 36 480 | 4 782 |
| moduly nad 5 000 r. | 1 | 0 |
| funkcie nad 400 r. | 20 | 22 (zamrznuté v ratchete) |
| najdlhšia funkcia | 2 365 (`plan_assistant_cad_edit_program_with_outputs`) | 2 103 (`apply_batch_with_origin_and_validation`) |
| `fn dot` / `cross` / `determinant` mimo `ketchup-geometry` | 24 / 20 / 12 | 0 / 0 / 0 (ostalo 7× `normalize`, 4× `sub`, 4× `transform_point`) |
| `Result<_, String>` | 58 | 2 |
| `map_err(\|e\| e.to_string())` | 69 | 0 (ale `Variant(e.to_string())` 139) |
| `Variant(format!(…))` | 81 | 10 |
| holé `unreachable!()` / `.unwrap()` | 14 / 40 | 0 / 0 |
| `.expect(` / pretypovania `as` / `#[allow(` | 260 / 669 / 43 | 332 / 955 / 67 |
| `ProgramPartBody::Panel` match ramien | 21 | 0 |
| testy `#[test]` | 2 457 | 2 900 |
| `sleep` v testoch | 157 | 138 (ale ratchet padá) |
| doménové slová mimo ratchetu (7 pôvodných) | 279 | 244 |

Nové doménové slová, ktoré ratchet zatiaľ nevidí:
- `stud`: 9, z toho 7 v `program/takeoff.rs`
- `rafter`: 3
- `joist`: 3
- `house`: 6
- `roof`: 6
- `stair`: 1

Výnimka pre validátory podľa normy (EC5) je oprávnená, ale `takeoff.rs` žiadny validátor nie je.

## 6. Čo je dobré

- **Splatené dlhy:** A1 a A2 z 1. 10. sú preč a B2 je takmer preč, všetko pod ratchetmi. `ProgramPartBody::Panel` zmizol, takže program je univerzálny.
- **Presná kolízia celého domu za ~18 s namiesto > 3 min,** bez novej paralelnej cesty a bez straty fail-closed vlastnosti (Cuthill-McKee poradie, zdieľaná fronta, kanonické steny hranolov).
- **Statika má poctivý základ:**
  - normové hodnoty a zdroje sú v `prelude.star`, nie v Ruste,
  - `BASIS` otvorene píše predpoklady,
  - chýbajúce vstupy dajú „not_verified“, nie „pass“,
  - reakcie aj zaťaženia sú dohľadateľné po zdrojoch.
- **Model:** batchy sú atomické už konštrukciou (klon kandidáta, `Arc`). Migrácia 101→102 zachováva digest. Rýchla cesta pre zmeny len zobrazenia je strážená revíziou a testom.
- **Protokol:** bootstrap z privátneho registra bez manuálneho tokenu, poctivé `response_timeout` s návodom „najprv prečítaj stav“, každá požiadavka má vlastný kanál odpovede.
- **PDF:** vložený TrueType font s ToUnicode, takže slovenčina sa tlačí aj vyhľadáva.

## 7. Návrhy vylepšení a smer

Zoradené podľa páky:

1. **Statika ako riešič, nie heuristika** (ST-1 až ST-4).
   - Jeden 1D riešič spojitého nosníka na prvok, zdieľaný pre reakcie aj pre kontroly EC5.
   - Zaťaženie dosiek podľa zaťažovacej plochy alebo smeru nosnosti.
   - Typovaný stav kontroly so zoznamom vykonaných kontrol; nosník s dvoma uloženiami musí mať kontrolu rozpätia.
   - Kontrola spojov podľa `basis`.
   - Sada ručne prepočítaných príkladov:
     - konzola, spojitý nosník, zárez `kv`, vzper `kc`;
     - doska na nerovnakých podperách;
     - spoj nad únosnosťou.

   Kým to nie je hotové, `load_capacity` označovať „predbežné, len gravitačné“.
2. **Jeden výrobný rámec a úplnosť exportu** (MF-1 až MF-5).
   - Kanonický pravotočivý rámec polotovaru s minimom v počiatku, zdieľaný pre BTLx, woodWOP a `production_job`.
   - Pred produkčným exportom report úplnosti, ktorý blokuje export a menuje diely:
     - skryté,
     - zrkadlené,
     - nevyriešené.
   - V CI: XSD validácia BTLx, test „každé opracovanie leží v polotovare“, golden test s neštvorcovým prierezom.
   - Konvencie raz overiť v reálnom importéri a zapísať do `docs/`.
3. **Zelené CI aj na Linuxe** (CI-1). Lacné a bez toho ratchety strácajú zmysel.
4. **Jedna služba vyhodnocovania programu mimo UI vlákna** (GUI-3, GUI-4, GUI-5): cache podľa zdrojáku a override hodnôt, revízia, „plánujem…“ náhľad. Odstráni zamŕzanie na veľkom dome.
5. **Lokálna validácia ako jediná cesta** (GE-2, AGENTS §3): `CollisionScope` zo zmenených dielov a susedov všade, celý model len na výslovnú požiadavku.
6. **Perzistencia pre veľké modely** (MD-1, MD-3):
   - história s entitami uloženými raz,
   - bez odvodených dát,
   - zlučovanie revízií, ktoré menia len zobrazenie,
   - rovnaké limity pri zápise aj čítaní,
   - politika vývoja schémy s golden súbormi starých formátov.
7. **Jedna politika ciest pre protokol** (PR-1, PR-2, P3 UNC): RawValue pred autentifikáciou, spoločná kontrola a atomický zápis pre `save_as`, `export_drawings` a ďalšie exporty. Dlhodobo prechod na OS-autentifikovaný transport (pomenovaná rúra alebo Unix socket).
8. **Ratchety:**
   - `Variant(e.to_string())`,
   - `normalize`, `sub` a `transform_point` mimo `ketchup-geometry`,
   - nové doménové slová (`stud`, `rafter`, `joist`, `house`, `roof`, `stair`),
   - počet `as` a `#[allow(` (nesmie rásť),
   - `*.work-recovery-lock` do `.gitignore`.

## 8. Odporúčané poradie

1. **Hneď (malé, vysoký dopad):**
   - CI-1,
   - GUI-1 (jeden riadok),
   - GUI-2,
   - MF-1 a MF-5 (kontrola determinantu a viditeľnosti v BTLx),
   - MF-3 (otočenie osi pri nepárnej permutácii),
   - PR-2 (politika zápisu),
   - GE-1 (jeden `Build()`).
2. **Tento týždeň:**
   - ST-1 až ST-5 s ručne prepočítanými testami,
   - MF-2 a overenie MF-4 v importéri,
   - PR-1,
   - GE-3.
3. **Potom:** služba vyhodnocovania programu (GUI-3 až GUI-5), lokálna validácia (GE-2), perzistencia (MD-1 až MD-3), MCP staging (PR-3, PR-4) a zvyšok P2.
4. **Priebežne:** P3 a nové ratchety.

Každý krok: zelené existujúce testy, vlastný regresný test, ktorý tvrdí výsledok (nie kód chyby), a ratchet tam, kde sa chyba môže vrátiť.

## 9. Výsledky spustených kontrol (Linux, OCCT 8.0.1)

- `scripts/check_*.py`: všetky OK okrem `check_test_sleeps.py` (CI-1).
- `cargo fmt --all -- --check`: OK.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **zlyhá** na Linuxe (CI-1).
- `cargo test --workspace`: doplním po dobehnutí.
