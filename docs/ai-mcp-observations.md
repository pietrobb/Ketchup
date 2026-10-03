# Poznatky z používania Kečupu cez AI/MCP

Priebežný denník reálnych skúšok. Ďalšie poznatky pridávame na koniec tohto súboru; staršie záznamy zachovávame. Pozorovania nie sú automaticky potvrdené chyby ani schválený plán implementácie. Prípadné vyriešenie doplníme pri príslušnom bode s dátumom a dôkazom.

Pri každej skúške zaznamenávame zadanie, výsledok, konkrétnu prekážku, použitý workaround a návrh zlepšenia. Rozlišujeme prijatie programu, rozmerové kontroly a presnú geometrickú validáciu. Implementačné pokračovanie 2026-10-03: autorizovaná vetva #2389, kroky #2390–2396 a crony #72 (10 min) / #73 (1 h). Opravené presné kandidáty navŕtaných/vyfrézovaných dielov a vynútené meranie deklarovaných vzdialeností aj pri oddelených obálkach; geometrické testy potvrdili 650 mm a odhalili nesprávnu požiadavku 649 mm. Pridané read-only `program validate` (3 testy zachovania dokumentu/histórie a odmietnutia starého výsledku), konkrétne busy dôvody (4 testy) a vysvetlenie vybraných vrtov/párov podľa deklarovaných polôh kovania (2 testy; nejde o explicitný ownership). Programová sada 143/143 a MCP 11/11 prešli. Test existujúcich podzostáv prešiel vrátane presunu, Undo/Redo a Save/Open, ale jeho 650 mm je šírka variantu, nie požadovaná výška; tento scenár ešte treba opraviť/doplniť. Priame meranie vybraných plôch, zvyšok A1–A4, úplná akceptácia A6 a plná fresh-build skúška A7 zostávajú otvorené; pôvodný používateľov model sa nemenil.

## 2026-10-03 — Implementačné pokračovanie po autorizácii commit/push

Doplnené a cielene overené: natívne meranie orezaných plôch a samostatná smerová vzdialenosť podporných rovín; read-only host test potvrdil 650 mm po vŕtaní a odmietnutie zastaraného výsledku. Explicitné väzby spojov na operácie a kovanie prešli 5 programovými a 3 aplikačnými testami vrátane oboch poradí mazania/náhrady, Undo/Redo a Save/Open. Odsadenie kolíkov a priechodný zámer prešli 6 testami. Lokálne opravy zdroja a úplné stránkovanie prešli 4 testami, MCP sada 13 testami. Nová 40-dielová zostava 4×2 overila svetlú VÝŠKU 650 mm, rovnaké šírky 875 mm, presun modulu a persistence natívne. Chýbajúci exact výsledok už nevracia verified; explicitná validácia aktualizuje aj stránkovaný report.

Celé riešenie ešte NIE JE hotové: plná programová sada má 149/150 úspešných testov, zostáva skontrolovať/regenerovať tri referenčné reporty príkladov. Clippy hlási 8 argumentov program_edit_result; kontrola doménového kódu odmieta nábytkový príklad v Rust docs/schema. Architektúra, formátovanie, error hygiene a git diff --check prešli. Čerstvá GUI/MCP skúška, úplná workspace sada, export a meranie komunikácie pred/po ešte neprebehli. Žiadny commit ani push, pôvodná skriňa nezmenená. Vetva #2389 a crony zostávajú aktívne; podrobný ďalší krok je uložený v HMOS.

## 2026-10-03 — Kolíkovaná policová skriňa

### Zadanie a výsledok

- Otvorená skriňa 3500 × 2600 × 520 mm, rozdelená na štyri rovnako široké úseky.
- Dosky hrúbky 18 mm; svetlá šírka úseku 852,5 mm.
- Štyri pevné police v každom úseku, všetky kolíkované. Kolíkované aj segmentované dná a stropy.
- Pravý dolný otvor má svetlú výšku 650 mm. V prvých troch úsekoch sú otvory vysoké 498,4 mm; v pravom úseku sú štyri horné otvory vysoké 460,5 mm.
- Model obsahuje 29 dosiek, 48 spojov a 192 kolíkov Ø8 × 35 mm s 384 vŕtaniami podľa programu.
- Program `policova_skrina.star` bol prijatý prvým `program apply`, v jednom Undo kroku. Report: `ok=true`, `errors=0`, `warnings=0`.
- Model zostal otvorený v čelnom pohľade; nebol uložený do samostatného súboru.

### AI-MCP-001 — OK report neznamená úplnú presnú geometrickú kontrolu

**Pozorovanie:** `program apply` hlásil úspech, ale súčasne `geometry_evaluated=false` a `exact_collisions=null`. Prešli programové kontroly rozmerov, kontaktov a svetlých výšok, nie preukázaná úplná exact kontrola zostavy.

**Doplňujúca skúška:** `inspect query faces` s `limit=1` vrátil OCCT topológiu a `geometry_evaluated=true`. Tento výsledok sám osebe nedokazuje kontrolu kolízií celej zostavy ani nezávislé potvrdenie všetkých vrtov. Vyhľadanie features s `search=dowel` vrátilo 1152 features (384 skupín po troch), čo potvrdzuje štruktúru prvkov, nie ich úplnú geometrickú validáciu.

**Dopad:** AI môže zameniť prijatie programu za výrobnú správnosť modelu. Musí dodatočne zisťovať, čo bolo skutočne overené.

**Návrh:** Jednoznačne oddeliť stav prijatia programu, rozmerových kontrol a exact geometrie. Poskytnúť explicitnú presnú validáciu programu bez odpojenia jeho parametrického zdroja, s jasným rozsahom a informáciou o nevyhodnotených kontrolách.

**Stav (2026-10-03):** Opravené rozlíšením prijatia, deklarovaného zámeru a exact výsledku; `program validate` je read-only. Výsledkové testy `program_validation_is_read_only_and_keeps_exact_coverage_after_removing_holes` a `missing_exact_result_and_worker_failure_never_claim_verified_or_revert_the_edit` prešli; verejný MCP scenár potvrdil aj zachovanie histórie. Úplná workspace kontrola je evidovaná osobitne na konci denníka.

### AI-MCP-002 — Protistojace kolíky na spoločnej priečke vyžadujú ručný výpočet

**Pozorovanie:** Bezpečné odsadenie vrtov z opačných strán spoločných priečok musela AI navrhnúť sama.

**Workaround:** Štyri kolíky na spoj; na ľavom konci vodorovnej dosky `margin=90`, na pravom `margin=50`. Rozdiel polôh radov v smere hĺbky je najmenej približne 13,333 mm pri priemere kolíka 8 mm. Ide o výpočtové odsadenie, nie o dôkaz úplnej exact validácie.

**Dopad:** Pri opakovaných priečkach musí AI sledovať susedné spoje a riziko stretu vrtov, nie iba zadávať spoj medzi dvoma doskami.

**Návrh:** Podporiť explicitný offset radu kolíkov alebo automatické bezpečné rozmiestnenie protistojacich vrtov s kontrolou kolízií a vysvetlením výsledného odsadenia.

**Stav (2026-10-03):** Opravené explicitným `dowels(offset=...)` pre oba párové rady, bez automatického vnucovania konštrukcie. Programové `opposing_holes` a natívny `plinth_bottom_and_brace_keep_blind_paired_bores_clear_after_rotation` prešli: bezpečné odsadenie, stret, zostávajúca stena, rozmery vrtov a rotácie. Nemožný spoj zostáva editovateľný bez fiktívnych vrtov/kovania (`an_impossible_join_remains_editable_without_fabricated_holes_or_hardware`). Montážna vhodnosť zostáva návrhovým rozhodnutím, nie tvrdením validátora.

### AI-MCP-003 — Dokumentácia vyžaduje veľa volaní a vracia príliš podrobné implementácie

**Pozorovanie:** Bolo potrebné načítať index, `basics`, `placement`, `joinery`, príklad `cabinet.star`, `intent` a `report`. Dokumentácia vracia aj celé implementácie helperov. Príklad skrine používa nastaviteľné police na podperných pinoch, nie pevné kolíkované police.

**Dopad:** AI spotrebuje viac volaní a kontextu na získanie jednoduchého pracovného postupu.

**Návrh:** Ponúknuť stručné podpisy helperov, význam parametrov, obmedzenia a krátke príklady. Detailnú implementáciu načítavať na vyžiadanie. Doplniť príklad pevných kolíkovaných políc vrátane spoločnej priečky.

**Stav (2026-10-03):** Opravené stručnou dokumentáciou; implementácia sa vracia až cez `detail=implementation`. Test `concise_topics_keep_public_signatures_without_function_bodies` prešiel. Príklady `shared-partition-shelves.star` a `modular-doweled-cabinet.star` sú spustiteľné a majú golden reporty; oba prešli aj vo verejnej MCP skúške. Namerané dáta a obmedzenia porovnania sú na konci denníka.

### AI-MCP-004 — Chýbajúci kusovník a skrátený zoznam vzťahov

**Pozorovanie:** Odpoveď `program apply` neobsahovala kusovník napriek tomu, že ho dokumentácia témy `report` opisovala. Z 62 vzťahov sa vrátilo 40 a odpoveď uvádzala `truncated=true`.

**Dopad:** AI nemá v jednej odpovedi úplný prehľad dielov, spojov a vzťahov. Musí používať ďalšie dotazy a nemôže považovať skrátený zoznam za úplný.

**Návrh:** Zjednotiť dokumentáciu so skutočným kontraktom odpovede. Vždy vracať spoľahlivé súhrnné počty a sprístupniť úplný kusovník a vzťahy cez stránkovanie alebo samostatný dotaz.

**Stav (2026-10-03):** Opravené súhrnným BOM a úplným stránkovaným `program report` pre diely, kovanie, obrábanie, vzťahy a problémy. `report_pages_reconstruct_complete_model_bom_and_relations` a verejný `test_modular_cabinet_local_edit_and_complete_reports` prešli; druhý uložil celý kusovník do JSON a porovnal počty riadkov. Čiastočná odpoveď sa nevydáva za kompletný zoznam ani za nový exact výsledok.

### Čo fungovalo dobre

- Model vznikol jedným zápisom celého Starlark programu, bez opravného apply.
- Parametrické zadanie a `expect`, `expect_gap`, `expect_contact` umožnili vyjadriť rozmery a požadované kontakty.
- Jediný Undo krok umožňuje jednoducho vrátiť celú skúšku.
- Prieskum modelu cez `inspect` sprístupnil počty prvkov a OCCT topológiu bez potreby obrázka.

### Hranice tejto skúšky

Únosnosť políc, zavetrovanie a kotvenie proti prevráteniu neboli posúdené. Skriňa je bez chrbta a dvierok. Úspešné modelovanie nie je potvrdením pripravenosti na výrobu.

## 2026-10-03 — Modulárne korpusy a sokel

### Zadanie a čiastočný výsledok

- Používateľ odmietol konštrukciu s dvoma protiľahlými policami kolíkovanými do spoločnej dosky a požiadal menšie transportovateľné segmenty, zavetrovanie a sokel 80 mm.
- Model bol prestavaný na osem samostatných korpusov 875 × 1260 × 520 mm, usporiadaných 4 × 2. Každý má vlastné bočnice, dno, strop a dve kolíkované police. Svetlá šírka je 839 mm; celková výška vrátane sokla zostala 2600 mm.
- Štyri samostatné soklové rámy majú výšku 80 mm, predné odsadenie 50 mm a strednú podperu. Spojenie korpusov je navrhnuté rozoberateľnými M6 nábytkovými spojkami s Ø8 priechodnými vrtmi; konkrétne kovanie a jeho tolerancie treba potvrdiť pred výrobou. Skrutky a spojky sú evidované ako hardware a majú pripravené otvory, nie samostatné modelované telá.
- Program eviduje 68 dosiek, 16 políc, 304 kolíkov (256 Ø8 × 35 a 48 Ø6 × 30), 40 spojok a 16 skrutiek pre sokel.
- Zadná doska zatiaľ nepridaná: požiadavka „pätnásť centimetrovú“ čaká na upresnenie, či ide o celoplošný chrbát hrúbky 15 mm alebo pás vysoký 150 mm. Zavetrovanie a kotvenie tým zostávajú nedokončené.

### AI-MCP-005 — Zámerný priechodný vrt sa hlási ako chyba

**Pozorovanie:** Prvý apply tejto úpravy vytvoril model, ale hlásil 96 chýb `hole_breaks_through`. Všetky zodpovedali zámerným priechodným vrtom: 80 vrtov pre 40 spojok a 16 otvorov pre skrutky cez dno. Dokumentované `hole()` nemá uvedený spôsob deklarovania zámerného priechodného otvoru. Hlásenie odporúča kratší spoj alebo hrubší materiál, čo pri priechodnej nábytkovej spojke nerieši požiadavku.

**Workaround:** Priechodné otvory sa vytvorili explicitným odčítaním valca: `extrude(..., profile=round_thread(radius), tool=True)`, umiestnenie kolmo k doske a `subtract()`. Slepé kolíkové vrty a pilotné vrty zostali cez pôvodné helpery. Nešlo o skrátenie vrtov ani vypnutie validátora. Výrobný export týchto boolean otvorov nebol overený.

**Výsledok:** Opravný apply vrátil `errors=0`, `ok=true`, `geometry_evaluated=true`, `exact_collisions.state=verified`, `collisions=0`, `unresolved=0`, `not_evaluated=[]` (1332 kontrolovaných párov, 136 exact párov). Report súčasne obsahuje 12 varovaní uvedených nižšie.

**Návrh:** Zdokumentovať alebo doplniť explicitné priechodné vŕtanie, aby sa odlíšilo od nechceného prerazenia slepého vrtu a zachovalo výrobný význam otvoru.

**Stav (2026-10-03):** Opravené explicitným `hole(through=True)` a zachovaním zámeru v kanonickej geometrii aj neutrálnych výrobných údajoch. `rule_program_through` prešlo vrátane šiestich vstupných smerov, objemov, príliš krátkeho vrtu, odpojenia programu a Save/Open. `blind_depth_btlx_export_stays_fail_closed_after_detach_and_reopen` overil platný slepý export aj odmietnutie nechceného prerazenia. Nepodporované priechodné BTLx/HOMAG cykly zostávajú výslovne odmietnuté, nie predstierané ako overený export.

### AI-MCP-006 — Presná kontrola kolízií neznamená overenie svetlých vzdialeností

**Pozorovanie:** Po cylindrických odčítaniach sa spustila exact kontrola, ktorá nenašla kolízie. Napriek tomu 12 `expect_gap` kontrol vracia `expectation_unverified`: „the exact solids are apart, but their distance was not measured“. Patrí sem aj požadovaný pravý dolný otvor 650 mm. Programové polohy medzeru určujú, ale report ju nepreukázal nezávislým exact meraním.

**Dopad:** Prechod od jednoduchých dosiek k boolean opracovaniu mení rozsah overenia. AI musí rozlišovať nulové kolízie od zmeraných vzdialeností; varovanie nie je dôkaz chybného rozmeru.

**Návrh:** Doplniť presné meranie požadovaných vzdialeností alebo výslovne reportovať samostatné výsledky smerového rozmeru a najkratšej vzdialenosti opracovaných telies.

**Stav (2026-10-03):** Opravené vynúteným exact meraním deklarovaných medzier aj mimo kolíznej obálky a priamym read-only meraním dvoch plôch. `separated_parts_keep_the_650_mm_measurement_after_unrelated_drilling_is_removed` overil 650 mm pred/po odstránení vrtu a odmietol 649 mm; `equal_width_modules_keep_the_lower_right_650_mm_height_after_motion_and_reopen` preukázal svetlú VÝŠKU, nie šírku. Šesť host testov merania a verejný MCP scenár prešli vrátane vnorených dielov a odmietnutia neplatných/nepodporovaných referencií. Najkratšia vzdialenosť orezaných plôch a smerová vzdialenosť podporných rovín sú dva explicitné režimy.

### Doplnenie AI-MCP-002 — Konštrukcia nie je iba problém odsadenia vrtov

Používateľ odmietol spoločnú priečku pre dve protiľahlé kolíkované police aj po výpočtovom odsadení vrtov. V tejto úprave sa preto prešlo na samostatné bočnice každého korpusu. Geometricky nekolízne vrty samy osebe neznamenajú vhodné výrobné alebo montážne riešenie. Zmena nepreukazuje automatické vyriešenie pôvodného problému rozhraním; je to zmena konštrukcie podľa požiadavky používateľa.

## 2026-10-03 — Zadné zavetrovacie pásy tvoriace L s policami

### Upresnenie a výsledok

- Používateľ upresnil zadné dosky: hrúbka 18 mm, výška 150 mm, dĺžka medzi bočnicami na celú šírku police; spojiť s policou aj bočnicami do L.
- Pridaných 16 pásov 839 × 150 × 18 mm, po jednom ku každej vnútornej polici. Sú zvislé pri zadnej hrane, v rámci hĺbky 520 mm, a stoja na hornej ploche police. Preto nezasahujú do otvoru pod policou a pravý dolný otvor zostáva podľa polôh dosiek vysoký 650 mm. V oblasti pásu je využiteľná hĺbka nad policou 502 mm.
- Každý pás má dva kolíky Ø8 × 35 do každej bočnice a štyri do police, spolu 128 nových kolíkov s párovými vrtmi. Model má 84 dosiek a podľa programu 432 kolíkov.
- Inkrementálny `program apply` prijatý na prvý pokus: 16 pridaných dielov, žiadny odstránený, zachované pôvodné diely aj parametrický zdroj; úprava je jeden nový Undo krok (celkový stav histórie `undo_steps=4`).
- Report: `ok=true`, `errors=0`, `warnings=12`, `geometry_evaluated=true`; exact kolízie `state=verified`, `collisions=0`, `unresolved=0`, `not_evaluated=[]`, 1716 kontrolovaných párov a 160 exact párov.
- Nebola zistená nová prekážka rozhrania. Zostáva AI-MCP-006: tých istých 12 varovaní o nezmeranej exact vzdialenosti vrátane 650 mm otvoru. Kontakty nových pásov nemajú hlásenú chybu.
- Ide o vymodelované zavetrovacie spoje, nie statický dôkaz tuhosti alebo únosnosti. Pevné kolíkové spoje predpokladajú vhodné lepenie; kotvenie k stene, únosnosť a výrobný export stále neboli overené.

## 2026-10-03 — Zavetrovanie aj nad štyrmi spodnými dnami

- Podľa šípok v používateľovom obrázku pridané 4 pásy 839 × 150 × 18 mm nad dná dolných korpusov pri zadnej hrane; každý s 2 kolíkmi Ø8 × 35 do každej bočnice a 4 do dna. Existujúce pásy a naložené stropy zachované.
- AI musela zohľadniť existujúce vrty: kolíky do dna majú okrajový odstup 150 mm namiesto 100 mm, aby sa neprekrývali so zadnými soklovými skrutkami v tej istej línii y=511 mm. Kontrola kolízií dosiek sama o sebe nedokazuje neprítomnosť kolízie vrtov/kovania; rozmiestnenie bolo posúdené z programu.
- Inkrementálny apply prijatý na prvý pokus, revision 6, jeden Undo krok, 4 nové diely (ID 158–161), 88 dielov a 20 pásov celkom. Exact report: geometry_evaluated=true, state=verified, 1812 párov, 176 exact párov, collisions=0, unresolved=0, not_evaluated=[]; errors=0, pôvodných 12 expectation_unverified upozornení zostáva.
- Svetlá výška pravého dolného otvoru zostáva podľa polôh dosiek 650 mm; nový pás zaberá zadných 18 mm hĺbky v spodných 150 mm otvoru. Statické posúdenie ani kotvenie k stene nie sú týmto overené.

## 2026-10-03 — Úprava označených vrchných dosiek na naložené

- `inspect status` úspešne sprístupnil výber `[57, 63, 69, 75]`; `program read` umožnil priradiť ID ku štyrom dielom `Usek 1–4 / Horny korpus / Strop`. AI teda vie identifikovať dosky označené používateľom aj bez obrázka.
- Pripravená úprava: šírka stropov 875 namiesto 839 mm, skrátenie horných bočníc o 18 mm, zachovaná celková výška 2600 mm a zvislé kolíkovanie stropov do horných hrán bočníc.
- Prvý zápis bol odmietnutý s `code=busy`, `phase=validation`; chybová správa tvrdila, že je otvorený nástroj alebo editor parametrov. Používateľ následne výslovne uviedol, že mal iba označené dosky a nič otvorené. Príčina `busy` teda nie je potvrdená; model sa odmietnutým pokusom nezmenil.
- Pozorovanie: čítanie výberu a programu fungovalo aj pri `busy=true`, zápis nie. Rozhranie by malo uvádzať konkrétny blokujúci stav/nástroj namiesto všeobecného vysvetlenia; AI nesmie vydávať text chyby za nezávisle potvrdený stav UI.
- Stav: pri ďalšom pokuse `busy=false`, výber už prázdny; pôvodné štyri stropy identifikované podľa zachovaných ID a názvov. Úprava úspešne vykonaná inkrementálnym `program apply` (revision 5), jeden Undo krok: stropy 875 × 520 × 18 mm, horné bočnice vysoké 1242 mm, zvislé párové kolíkové vrty Ø8 pre kolíky 8×35. Celková výška 2600 mm, police, zavetrovacie pásy a sokel zachované. Exact kolízny report: `geometry_evaluated=true`, `state=verified`, 0 kolízií, 0 unresolved, report 0 chýb a 12 pôvodných `expectation_unverified` upozornení.

## 2026-10-03 — Skryté upevnenie sokla zospodu

- Používateľ správne namietol viditeľné priechodné vrty v dnách. Pôvodný návrh AI skrutkoval zhora do horných hrán predného a zadného soklového pásu; nejde o nedostatok MCP, ale nevhodne zvolený spoj.
- Revision 8: odstránených 16 priechodných vrtov Ø4,5 mm v dnách a 16 horných hranových predvrtov Ø3 × 35 mm v sokle. Nahradené 16 slepými predvrtmi Ø3 × 12 mm zo spodnej strany dna a 16 rovnakými vrtmi z vnútorných plôch soklových pásov. Dno nad predvrtom zostáva hrubé 6 mm; vrty sú mimo línie zadných zavetrovacích kolíkov.
- Spoje evidujú 16 vnútorných uholníkov 40 × 40 × 2 mm, každý s dvomi skrutkami 4 × 16 mm. Kovanie je záznam spoja, nie fyzicky vymodelovaný uholník ani skrutka; konkrétne kovanie a jeho únosnosť neboli overené. Montáž prebieha pred položením zostavy na podlahu.
- Apply prijatý na prvý pokus, jeden Undo krok, zachovaných 88 dosiek a rozmery. Report `ok=true`, errors=0, warnings=0, ale `geometry_evaluated=false`, `exact_collisions=null`. Po odstránení posledných explicitných boolean priechodných vrtov sa report opäť vrátil k slabšiemu overeniu; nula upozornení neznamená silnejšiu exact validáciu. Rozhranie musí tieto úrovne jasne odlišovať.
- Následná oprava podľa používateľa (revision 9): bez uholníkov a skrutiek. Odstránených všetkých 32 predvrtov pre uholníky aj ich záznamy kovania; namiesto nich zvislé kolíkové spoje 8×35 z horných hrán predného a zadného soklového pásu do spodnej strany dna. Štyri kolíky na pás, spolu 32; párové slepé vrty Ø8, hĺbka 26 mm v sokle a 12 mm v dne, nad vrtom zostáva 6 mm materiálu. Odstup 100 mm od koncov odlišuje zadné spodné vrty od horných zavetrovacích vrtov s odstupom 150 mm. Jeden Undo krok, 88 dosiek zachovaných, report errors=0, warnings=0; exact kontrola opäť neposkytnutá (`geometry_evaluated=false`, `exact_collisions=null`). Poučenie pre AI: zbytočne nenahrádzať jednoduché používateľom preferované kolíkovanie komplikovanejším kovaním.

## 2026-10-03 — A1: natívne meranie a verejná MCP regresia

- Opravená dodatočne reprodukovaná chyba: odstránenie posledného nesúvisiaceho vrtu vyradilo obyčajné kvádre z požadovaného exact merania. Explicitné vzdialenostné podmienky teraz vyberajú oba diely bez ohľadu na opracovanie; neklesnú potichu na slabší report.
- `rule_exact_measurements` prešlo 3/3: rovnaká svetlosť 4 mm v otvore/kapse/boolean výreze; plytká drážka namiesto 4 mm správne dá 0 mm a kolíziu; svetlosť 650 mm sa zmeria s vrtom aj po jeho odstránení a požiadavka 649 mm zlyhá.
- `ketchup-mcp` prešlo 14/14 vrátane kontraktu oboch režimov merania, vnorených ciest a explicitného validačného volania. Povinný `expected`, dve plochy, režim a smer pre roviny sú súčasťou verejnej schémy.
- Čerstvý normálny build `cargo build --workspace --bins` a celá sada `ketchup-program` 150/150 prešli. Pre existujúce golden reporty pribudlo iba `through=false`; rozmery, počty a ostatné údaje sa nezmenili. Nový príklad spoločnej priečky má samostatný report.
- `tests/test_mcp_live_measurement.py` reálne prešlo 1/1 bez preskočenia cez Python → verejné MCP → nové izolované GUI. Oba read-only dotazy namerali 650 mm, explicitná validácia prešla, zdroj/výber/história sa nezmenili, neplatná a stale referencia neboli pass, Undo a Save/Open zachovali model. Build: `target/assemblies-20261003/debug/ketchup-app.exe`; log `bg24-18db09ee90b53fc0`, metriky v `target/mcp-measure-20261003-a1c/`. Pôvodná skriňa nebola otvorená ani menená.
- Samotná požiadavka `limit=100` nezaručuje úplný zoznam plôch: treba sledovať `next_cursor`, lebo platí aj bajtový limit. Názvy v model query sú štruktúry `name.text`, nie holé reťazce. Verejný test rešpektuje obidva kontrakty.
- Celá vetva ešte nie je hotová: prebiehajú rozšírené nested/negative testy a workspace kontroly. `check_test_sleeps.py` našiel v nezmenenom `live_bridge/image_privacy_tests.rs` 5 sleep oproti baseline 4; nejde o nový vrt/measurement test ani o zelenú kontrolu.

## 2026-10-03 — A2/A3: vlastníctvo spojov a bezpečné vrty

- A2: päť natívnych testov provenance prešlo vrátane oboch poradí odstránenia/náhrady susedných spojov, nezávislého servisného otvoru, pántových vrtov, Undo/Redo a Save/Open. Nepodporované rozpoznanie boolean obrábania sa nevydáva za explicitného vlastníka.
- AI-MCP-002: `dowels(offset=...)` posunie oba párové rady a zverejní stredy. Existujúce kontroly zostávajúcej steny, okrajov a protiľahlých vrtov zostali aktívne. Nemožný vzor hlási problém, ale model zostáva editovateľný; nevytvorí vymyslené vrty ani kovanie.
- Natívny `plinth_bottom_and_brace_keep_blind_paired_bores_clear_after_rotation` overil 12 párov Ø8 s hĺbkami 12/26 mm na dne/sokli/zavetrovaní, objem odobratého materiálu, rotácie 0/31° a Undo. Odsadenie 11 mm je bezpečné, nulové odsadenie dá presne štyri strety. Test odhalil ďalšiu chybu: po rotácii dielov zostávali stredy kovania na starých súradniciach. Opravené ukotvením na prvý diel spoja; samostatný test overil obe poradia presunu bez dvojitej transformácie aj hlásenie oddeleného druhého dielu.
- AI-MCP-005: `hole(through=True)` sa odlišuje od nechceného prerazenia slepého vrtu. Štyri natívne testy overili všetkých šesť vstupných smerov, objemy, intent-only zmeny, zmenu hrúbky, Undo/Redo a Save/Open. Príliš krátky priechodný vrt sa neprehĺbi potichu a jeho boolean geometria nemôže byť exportovaná ako schválené slepé obrábanie.
- Výrobná hranica: neutrálne údaje zachovávajú priechodný zámer; BTLx/HOMAG priechodné cykly zostávajú nepodporované a bezpečne odmietnuté. Nejde o dokončenie strojovo špecifického exportu. Všetkých 20 `fabrication_validation` testov prešlo vrátane zachovania existujúceho slepého BTLx exportu.
- `bg34` prešiel natívny soklový test a workspace Clippy. `bg36` prešlo 152 programových testov, 4 through-cut testy a 20 výrobných testov. A3 uzavreté, A4 a finálna akceptácia pokračujú. Žiadny commit/push ani zmena pôvodnej skrine.

## 2026-10-03 — A4: veľká verejná skúška odhalila časový limit

- Nový príklad `modular-doweled-cabinet.star` má 88 dielov, osem korpusov, svetlú výšku 650 mm, naložené horné dosky, L zavetrovanie a iba slepé soklové kolíky. Samostatné programové kontroly nemajú chyby; skupinové kontakty sú bez natívneho jadra výslovne neoverené.
- Prvý debug MCP beh stratil spojenie po 30 s. Potvrdený rozpor: host čakal 30 s, MCP klient 45 s. Programový transport teraz zdieľa 45 s budget, klient má navyše 5 s na doručenie; dve transportné regresie prešli. Presný validačný limit nebol zmenený.
- Debug veľký model prekročil aj zjednotený transportný limit. Čerstvý normálny release build vznikol úspešne v izolovanom `target/assemblies-20261003/release`. Read-only 650 mm scenár aj natívny príklad spoločnej priečky prešli cez verejné MCP. Veľká skriňa už vrátila pravdivý report namiesto straty spojenia, ale NIE pass.
- Konkrétny nedokončený výsledok: `exact_collision_timeout`, `timeout_ms=20000`, `exact_pair_count=0`, `unchecked_pair_count=249`, osem nezmeraných vzdialeností, 11 neoverených skupín. Úplný report: `target/mcp-cabinet-20261003-a4diagnostic/test_modular_cabinet_local_edi0/last-program-result.json`. Test nebol oslabený; neoverené nie je úspech a fixture sa nezmenšila.
- Porovnanie plného zdroja oproti lokálnemu patchu, kompletné stránkovanie a ďalšie prechody sú implementované v Python teste, ale na veľkej fixture sa ešte nevykonali: zastaví ho vyššie uvedená asercia. Žiadny číselný výkonový prínos zatiaľ netvrdíme.
- `bg39` prešli všetky tri scenáre podzostáv vrátane svetlej výšky, lokálneho presunu, neplatného členstva, Undo/Redo a Save/Open. Architektúra, named-products, error-hygiene, test-env, test-sleeps, formátovanie a diff-check prešli. Pevný sleep v existujúcom obrazovom teste bol nahradený čakaním na jeho kanál.
- Plná workspace sada beží ako `bg44-18db0f3e35e8def8`; nezávislý read-only audit beží ako `20adbf73`. A4/#2397 zostáva otvorené; pôvodná skriňa nebola menená, nič nebolo commitnuté ani pushnuté.

## 2026-10-03 — Profilovanie veľkej zostavy a opravy regresií

- Profil tej istej 88-dielovej zostavy bez GUI: načítanie a vytvorenie telies 15,61 s, celá worker fáza 19,65 s, spolu s prípravou programu 34,33 s. Všetkých 249 potrebných exact párov a osem svetlostí bolo overených; sedem je 400 mm, pravý dolný otvor 650 mm. Diagnostický test používal existujúci 120 s testovací budget, nie zmenu verejného 20 s limitu.
- Väčšie dávky sa teraz rozdelia podľa rozsahu ľavých grafov medzi najviac dva izolované workery; malé kontroly zostávajú na jednom. Žiadny pár sa nevynechá a výsledky sa odovzdávajú v poradí rozsahov. Rovnaká fixture následne prešla s worker fázou 10,50 s a celkovým časom 24,05 s. Ide o jednotlivé merania, nie SLA. Dočasné profilovacie výpisy boli odstránené. Negatívna 40-dielová regresia odhalí presne jednu kolíziu v druhej polovici a správne zmeria všetkých 20 vzdialeností.
- Audit našiel dieru v BTLx ochrane slepého vŕtania po odpojení programu. Export teraz kontroluje vstup a celé dno vrtu vrátane polomeru pri šikmom smere proti lokálnemu materiálu. Hĺbka dosahujúca alebo prekračujúca výstup sa odmietne; editovaná geometria sa nemení. Prešlo päť BTLx unit testov, 21 výrobných testov a päť natívnych scenárov vrátane odpojenia, rotácie a Save/Open. Platný slepý BTLx zostal zhodný s golden súborom.
- `bg44` skončil s dvomi regresiami: príliš veľký editovací kontext po sprístupnení nepomenovaných plôch. Krátky kontext teraz výslovne uvádza `scope=named_faces`; úplný stránkovaný zoznam vrátane nepomenovaných plôch sa nemení. Dve aplikačné a jeden headless výsledkový test prešli (`bg49`). Celá workspace sada sa musí po opravách zopakovať; tento výsledok ju nenahrádza.
- `bg49` úspešne zostavil čerstvé normálne release binárky. Následný verejný beh odhalil, že explicitný výber odmietal aj viditeľné koreňové diely v obyčajných skupinách. Opravený iba výber; ochrany nízkoúrovňových zápisov zostali nezmenené. Prešlo 40 testov výberu a päť program-access testov vrátane označenia štyroch skupinových dielov, zdrojového kontextu a zrušenia výberu bez zmeny modelu/histórie.
- Čerstvý release MCP beh `target/mcp-cabinet-20261003-group-selection/` prešiel 2/2 bez skipov (222,38 s za celé oba scenáre): 650 mm read-only, plná skriňa, spoločná priečka, štyri vybrané stropy, naložené/vložené stropy, odstránenie fyzických spojkových vrtov, slepé sokly, 650→660, stale/nejednoznačný patch, Undo/Redo, úplné stránkovanie kusovníka/vrtov/vzťahov a Save/Open. Pôvodná skriňa zostala nedotknutá.
- Komunikácia na rovnakej úlohe: full read/apply 27 619 B, source/patch 9 173 B (o 66,8 % menej), obe dve volania. Stručné joinery docs 5 584 B oproti 18 565 B implementácie (o 69,9 % menej). Časy jednotlivých behov boli 19,06 s full a 31,22 s patch; zrýchlenie patchu ani SLA z toho netvrdíme. Baseline je plný režim aktuálnej binárky, nie historická binárka. Metriky a celý kusovník sú uložené vedľa testového dokumentu.
- A3/A4/A6 sú výsledkovo pokryté. Záverečná opakovaná workspace sada, Clippy a doručenie zostávajú otvorené; cielené výsledky ich nenahrádzajú.

## 2026-10-03 — Záverečné pokrytie pozorovaní mimo AI-MCP-001–006

- **Busy a štyri označené dosky:** `busy_diagnostics_*` rozlišujú nástroj, editor, preview, pointer a klávesnicový fokus bez zrušenia ľudskej práce. `grouped_root_parts_can_be_selected_read_and_cleared_without_an_edit` a verejný MCP scenár prešli pre štyri stropy v skupinách; následná úprava zachovala naloženie aj rozmery. Pôvodný dôvod historického busy spätne nevyhlasujeme za dokázaný.
- **Účel otvorov a ownership:** `neighboring_join_change_delete_undo_redo_and_save_open_preserve_only_owned_holes` a `reverse_order_adjacent_join_patch_preserves_neighbor_and_reopens` prešli pre oba poradia úprav/mazania a nezávislý servisný otvor. `generic_rotated_joint_picks_do_not_invent_solid_hardware_links` rozlišuje metadáta od explicitne priradeného telesa; pántový test overuje prepojenie misky a dvoch predvrtov. Boolean plocha bez podporovaného priradenia zostáva `not_identified`, nie vymyslené vlastníctvo.
- **Podzostavy a svetlá výška:** `cabinet_4x2_variant_move_reapply_history_persistence_and_invalid_membership` a `equal_width_modules_keep_the_lower_right_650_mm_height_after_motion_and_reopen` prešli; skupiny používajú skutočné členstvo, nie iba lomky v názvoch. Samostatný soklový test overuje párové slepé vrty, L pásy, rotáciu a Undo. Verejná 88-dielová fixture overila naložené stropy aj odstránenie spojkových vrtov a uloženie/opätovné otvorenie.
- **Lokálne úpravy:** `local_patch_retains_source_overrides_geometry_and_one_undo` a `ambiguous_missing_overlapping_or_invalid_patch_publishes_nothing` prešli. Verejný test mení tú istú výšku plným zdrojom aj patchom, meria prenos a čas, overuje Undo/Redo a stale odmietnutie; úspora bajtov nie je vydávaná za zrýchlenie.
- **AI návrhový postup:** pokyny MCP a programová dokumentácia uprednostňujú požadovaný jednoduchý spoj, skoré zváženie transportu/montáže a stručné pravdivé hlásenia. Príklady obsahujú pevné kolíkované police aj modulárnu alternatívu bez vnútených uholníkov; verejný test asertuje iba kolíky v konečnom kusovníku a slepé vrty. Testy dokazujú tieto konkrétne výstupy, nie záruku správneho rozhodnutia ľubovoľného LLM alebo statickej únosnosti.
- **Dodatočný audit:** read-only audit `3a31c1bc` našiel pokračovanie druhej paralelnej dávky po zlyhaní prvej. Doplnené zrušenie po zachovaní pôvodnej chyby a pri strate príjemcu; `parallel_worker_failure_cancels_remaining_work_without_masking_the_cause` a natívny `lost_parallel_receiver_cancels_workers_and_joins` prešli v `bg52` (1 + 1 test bez skipov). Druhý používa 20 skutočných opracovaných telies a čaká na ukončenie koordinátora, nie iba na vydanie chyby. `bg51` medzitým skončil úspešne: celý workspace beh pred poslednou opravou, Clippy nad aktuálnym zdrojom a päť Python headless testov bez skipov. Päť existujúcich workspace opt-in/pomocných testov zostalo ignored; ich výsledok netvrdíme. `bg52` teraz opakuje celú sadu po poslednej oprave, potom Clippy, fmt, release build a všetky tri Python headless/MCP súbory. Commit/push čaká na tento výsledok.
- **Regresia vlastníctva zrušenia:** následný `bg52` odhalil dve chyby Assistant workflowov (planárny offset a kontext importovanej XDE zostavy). Interné zastavenie workerov zapisovalo do príznaku celej požiadavky, takže neúplná kontrola skončila ako `request_cancelled`. Worker má teraz vlastný príznak; používateľovo zrušenie sa odovzdáva jednosmerne. Regresia chyby workeru výslovne kontroluje, že rodičovská požiadavka zostáva aktívna, a existujúci natívny test naďalej vyžaduje ukončenie workerov. Nadväzujúci `bg53` po konci `bg52` overil obe zlyhané úlohy (1,32 s a 4,43 s), zachovanie rodičovskej požiadavky (0,14 s), natívne ukončenie workerov (47,49 s) a explicitné používateľovo zrušenie; všetkých päť cielených testov prešlo bez skipov. Následne prešla aj celá doručovacia sada, uvedená nižšie.

## 2026-10-03 — Finálne overenie pred doručením

- `bg53-18db1639b2b43778` skončil s exit code 0: `cargo test --workspace --no-fail-fast -- --test-threads=1`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` a čerstvý normálny `cargo build --release --workspace --bins` prešli nad konečnou opravou. Päť existujúcich opt-in/pomocných workspace testov zostalo ignored (dva live OAuth, CPU benchmark, Blender parity a subprocess helper); nevydávame ich za vykonané dôkazy.
- Všetkých sedem Python headless/MCP testov prešlo bez skipov za 254,27 s. Verejný MCP vytvoril vlastné okno a zopakoval read-only meranie aj celý 88-dielový modelovací postup vrátane úprav, zmazania spojok, všetkých reportových stránok, Undo/Redo a Save/Open. Artefakty sú v `target/final-mcp-20261003-isolated-cancel/`; výsledný dokument, úplný JSON kusovník a metriky sú v `test_modular_cabinet_local_edi0/`. Pôvodná skriňa nebola menená.
- Finálne porovnanie rovnakej úpravy v tej istej binárke: full 2 volania, 27 619 B, 26,656 s; patch 2 volania, 9 173 B, 31,000 s. Prenos klesol o 66,8 %, počet volaní sa nezmenil a zrýchlenie patchu sa nepotvrdilo. Stručné docs majú 5 584 B oproti 18 565 B implementácie (o 69,9 % menej). Ide o jednotlivé merania, nie SLA ani historický benchmark.
- Architektonická kontrola, named-products, error-hygiene, test-env, test-sleeps a kontrola diffu prešli. Finálny audit aj následná regresia sú opravené a výsledkovo overené. Na doručenie sú vybrané iba súbory tejto vetvy; cudzie modely, patch a recovery locks zostávajú mimo commitu.
