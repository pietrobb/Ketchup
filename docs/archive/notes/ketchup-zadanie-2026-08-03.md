# Kečup — reakcia na odpoveď a zadanie ďalšieho kroku

**Dátum:** 3. 8. 2026
**Nadväzuje na:** `ketchup-odpoved-inea-2026-08-03.md`

Tvoja odpoveď je vecná a štyri z piatich mojich nálezov korektne vyvracia alebo zmenšuje. Beriem to
bez výhrad — písal som ich bez prístupu k repozitáru.

Tento dokument obsahuje tri veci, ktoré v predchádzajúcej výmene nemohli byť: **kontext cieľovej
domény**, ktorý určuje, čo je vlastne produkt, **jeden nový nález**, ktorý recenzia minula, a
**konkrétne zadanie** vrátane jedného rozhodnutia, ktoré patrí tebe.

---

## 1. Kontext, ktorý ti chýba

Doterajšia diskusia sa točila okolo jadra. Medzitým vyšlo najavo, na akú úlohu má Kečup slúžiť, a
mení to váhy.

### Referenčný prípad: drevodom

Vlastník si postavil drevostavbu, ktorú si sám navrhol v SketchUpe. **1400 kusov reziva**, každý
s konkrétnou dĺžkou, prierezom a drážkami. Modelovanie trvalo týždne, nie hodiny. Výkresy išli do
firmy s CNC linkou Hundegger, ktorá kusy vyfrézovala; montáž prebehla ako skladanie stavebnice.

Z výkresov je vidieť podstatnú vec. Nosník A má rozostupy `415 × 6`, potom `408 × 5`, potom `400`.
Profil C22 má `132, 132, 133, 140, 141, 116, 132, 133, 149`. Žiadne z tých čísel nie je autorské
rozhodnutie — sú to výsledky delenia rozponu na N polí so zaokrúhlením na milimeter, vrátane
rozhodenia zvyšku.

**Dôsledok:** posun jedného okraja o 20 mm prepíše dvadsať kót na jednom kuse. Nie jednu. To je
dôvod, prečo je ručný postup neopraviteľný — nie počet kusov, ale to, že pravidlo existuje len
v hlave autora a v modeli sa prejaví ako dvadsať nezávislých čísel.

Druhá vec z tých istých výkresov: drážky sú všade `hĺbka 20 mm, šírka 160 mm` a profily vrstvy 03
sú presne `160 mm` široké. Drážka na nosníku a drážka na profile **nie sú dva údaje** — sú to dva
pohľady na jeden prienik. Ak je v modeli vzťah „tieto dva kusy sa krížia polodrážkou 20/20", obe
sady kót sú odvodené a nemôžu sa rozísť. Ak sú to nakreslené výrezy, rozísť sa môžu — a zistí sa to
až keď to CNC vyfrézuje.

### Druhý dôkaz: FurniGen

Vlastník už raz postavil systém tohto typu — generátor nábytku nad SketchUpom. Priebeh je pre nás
najdôležitejší dostupný údaj:

- **AI navrhujúca nábytok bez validátorov: nepoužiteľné.**
- **Tá istá AI s validátormi: použiteľné.** Vygenerovala skriňu vrátane kusovníka, dosky prišli
  narezané od dodávateľa, skriňa je postavená a sadla na dva centimetre pod strop.

Model sa nezmenil. Zmenilo sa sito. To je A/B test, ktorý má pre Kečup väčšiu výpovednú hodnotu než
čokoľvek v architektonickom dokumente.

Kolízny validátor tam fungoval takto: broad phase na prienikoch obalov hranolov, a tam, kde niečo
narazilo, exaktnejší test nad trojuholníkmi. Rýchle a spoľahlivé. Vedel rozlíšiť aj drážky
a netriviálne prieniky.

### Z toho plynúci cieľ

Kečup nemá byť „CAD s AI". Má byť nástroj, kde:

1. model je **súbor pravidiel**, nie súbor kusov — 200 riadkov namiesto 1400 položiek;
2. kusy, drážky a kóty sú **odvodené**;
3. AI píše a upravuje **pravidlá**, nie geometriu — pretože jazykový model píše pravidlá výborne
   a ťahá uzly zle, a pretože desať riadkov sa dá skontrolovať a dvesto kusov nie;
4. **jadro validuje** — a to je jediný dôvod, prečo sa celému postupu dá veriť;
5. výstup je kusovník a výrobné dáta, nie obrázok.

Cieľová veta vlastníka: *„predĺž obývačku o pol metra a zmeň všetko ostatné, aby sa to nerozpadlo."*
Prvá polovica je zmena parametra a deterministický prepočet — to je DAG, nie AI. Druhá polovica je
otvorený problém a rieši sa validátormi, nie lepším promptom.

### Čo z toho vyplýva pre existujúcu roadmapu

- **Validátory nie sú krok 5 commit pipeline.** Sú produkt. Zvyšok je infraštruktúra, ktorá im
  umožňuje bežať.
- **Kusovník a výrobné výkresy nie sú „neskorší míľnik".** Sú výstup. Nejde pritom o ten drahý
  drawing systém, ktorý je správne odložený — výkres jedného kusu s reťazcom kót a tabuľkou je
  deterministický výstup z kanonického modelu.
- **Zmena je hlavný prípad použitia**, nie okrajový. DAG, stabilné referencie a prepočet len
  závislej vetvy existujú kvôli nej. V dokumente sú popísané ako technické vlastnosti.
- **BTLx** (otvorený XML formát pre tesárske CNC linky, číta ho Hundegger a spol.) je pre túto
  doménu dôležitejší než STEP aj IFC dohromady a je rádovo jednoduchší — exportuje sa zoznam
  parametrických operácií, nie B-Rep. Nepatrí medzi odložené položky natrvalo, ale ani hneď.

---

## 2. Čo z tvojej odpovede beriem bez výhrad

Aby sme sa k tomu nevracali:

- gateway (`apply_batch` ako jediný verejný mutačný vstup) existuje — môj bod 4 bol z väčšej časti
  splnený;
- lokalizácia cez `LocaleCatalog` existuje, próza vo widgetoch nie je;
- coordinate envelope a tolerančný profil sú zmrazené v kóde;
- dynamické linkovanie OCCT je zvolené v `build.rs`;
- akceptačná suita beží headless cez AccessKit a asserty čítajú stav dokumentu, nie text — to je
  presne správne riešenie;
- R0 je skompilované, nie papierové; moja výhrada o „týždňoch bez kódu" bola mimo;
- prototyp je substrát, nie spike — súhlas, len nech je to ADR;
- poradie „čítacia strana → `Intent` slovník → prahy brány D" je správne a moje pôvodné poradie
  bolo horšie.

---

## 3. Nový nález: dve geometrické cesty bez kontraktu ekvivalencie

Toto vyplynulo z faktu, ktorý si uviedol ty: `ketchup-app` na OCCT nezávisí, picking a geometria
interakcie sú analytická exaktná cesta v `ketchup-interaction`, a `ketchup-exact` je linkovaný len
zo `ketchup-scheduler` a z A0 testu.

Znamená to, že v systéme sú **dve nezávislé implementácie geometrickej pravdy** a žiadna brána
nemeria, či súhlasia.

Konkrétne:

- **Gate C prešiel na analytickej ceste.** Nehovorí nič o pickingu nad telesom, ktoré vyrobilo
  OCCT — čo je to, čo FLP potrebuje.
- **Gate A0 testoval resolver nad OCCT topológiou.** Interakčná vrstva vyrába `SubshapeRef`
  z vlastnej reprezentácie. Ak sa tie dve nezhodujú v tom, čo je „tá istá plocha", vzniká tiché
  nesprávne prepojenie — a to má nulový prah na dvoch miestach dokumentu.
- **Nie je určené, ktorá reprezentácia je autoritatívna.** Dokument hovorí `ExactBody`. Bežiaca
  aplikácia hovorí analytická cesta. Dnes to nebolí, lebo sa nestretávajú.

Vedľajší efekt je pozitívny a potvrdzuje tvoj bod 5: existencia funkčnej planárnej exaktnej cesty
robí preregistrovaný fallback lacnejším, než vyzeral. Ale je to zároveň jediný dôvod, prečo tá diera
zatiaľ nebolí — a keď OCCT telesá vojdú do aplikácie, prejaví sa naraz a bude vyzerať ako záhadný
bug v pickingu.

---

## 4. Tri korekcie k tvojej odpovedi

### 4.1 A0 = 100 %: platí, ale je to vlastnosť dvojice *(OCCT, subset)*

Korekciu beriem — tvrdenie „OCCT ti tú evidenciu nedá" bolo v prítomnom čase vyvrátené meraním.
Formuluješ to sám správne („rekonštruovaná, nie konštrukčne daná").

Nech je len zapísané, že zmrazený subset je najpriaznivejší možný prípad: jedna operácia, jasná
provenience, žiadne delenie plochy. FLP bude subset tlačiť tam, kde to začne bolieť — cut cez viac
plôch, union dvoch telies, plocha rozdelená hranou, pattern. **Pri každom rozšírení subsetu sa A0
musí zopakovať a 100 % nie je prenosné.**

### 4.2 R0_V1…V13 — chýba smer zmien

Preregistračné pravidlo (§12) hovorí, že zmena po zhliadnutí výsledku znamená neúspech pôvodnej
brány a novú verziu testu. Trinásť verzií je s tým formálne zlučiteľné, ale je to presne tvar, ktorý
malo to pravidlo chytať.

Rozhodujúci je smer. **Sprísnenie po meraní je zdravý proces. Zmäkčenie po meraní je zlyhanie
brány, aj keď sa zapíše ako nová verzia.**

Požiadavka je jednoriadková: report nech ku každému prechodu V*n* → V*n+1* nesie príznak
`tighten` / `loosen` / `neutral` a či zmena vznikla pred alebo po meraní danej verzie. Ak sú všetky
sprísnenia, A0 je čistý a je to doložené. Bez tohto záznamu je „A0 prešiel" tvrdenie, ktoré nikto
neauditovateľne neoverí — vrátane teba o rok.

### 4.3 Envelope: tolerancia je absolútna, presnosť je relatívna

`MAX_COORDINATE_MM = 1 000 000` s `bbox=1e-6mm` je kombinácia, ktorá platí pri počiatku a je napnutá
na okraji. Pri súradnici 1e6 mm je ULP `f64` rádovo 2·10⁻¹⁰ mm, čiže 1e-6 mm je asi 4500 ULP —
vyjde to, ale rezerva na akumuláciu v booleanoch je tenká. OCCT navyše používa fixné
`Precision::Confusion` v modelových jednotkách, čo sa pri veľkých súradniciach správa horšie než
blízko počiatku.

Praktický dôsledok: **adversarial korpus musí obsahovať prípady na hornom konci envelope**, nie len
telesá pri počiatku. Ak tam dnes nie sú, tých 90 % meria ľahšiu úlohu, než akú envelope sľubuje.

Alternatíva je envelope zúžiť. 1 km je pre architektúru, interiér a nábytok nadmerné a georeferencia
je správne riešená ako transform nad lokálnym modelom.

### 4.4 `StateView`: dve projekcie, nie jeden artefakt

Tu spresňujem vlastnú formuláciu — „jedna implementácia, traja konzumenti" bolo príliš úsporné.
Zdieľaný má byť **enkodér**, nie výstup:

- **úplný kanonický výpis** — všetko, deterministicky usporiadané, pre golden testy a diffy;
  objemný, nečitateľný, presný;
- **agentský pohľad** — sémantický a zhrnutý; mená, vzťahy, rozmery, stav referencií, bez
  transformačných matíc.

Jeden artefakt pre oboje skončí kompromisom, ktorý je zlý pre oba účely. Verzionuj ich zvlášť;
golden fixtures viaž na úplný výpis, agentský pohľad nech sa smie meniť voľnejšie.

---

## 5. Rozhodnutie, ktoré patrí tebe: kde bývajú pravidlá

Od tohto závisí všetko ostatné a ty máš lepšie dáta, lebo vidíš do kódu.

`ketchup-core` má dnes dokument z **kusov**: Definition, Occurrence, Group, Feature, Transform.
Cieľová doména potrebuje dokument z **pravidiel**, z ktorých kusy padajú ako výsledok. Otázka je, či
je pravidlová vrstva:

**(A) Vrstva nad `ketchup-core`.** Pravidlo sa vyhodnotí a vyprodukuje `CommandBatch`. Jadro sa
nemení. Kusy sú v dokumente normálne entity.

**(B) Súčasť kanonického dokumentu.** `Rule` je entita vedľa `Definition`, kusy sú odvodené a
v súbore ako také neexistujú.

Praktický rozdiel: pri (A) sa dá pravidlo spustiť, ale keď niekto posunie jeden kus ručne, pravidlo
o tom nevie a pri ďalšom prepočte ho prepíše. Pri (B) to jadro rieši, ale DAG, undo, referencie
aj perzistencia musia počítať s odvodenými entitami.

**Môj návrh: (A) s jedným ústupkom** — `CommandBatch` vygenerovaný pravidlom si nesie odkaz na
pravidlo a jeho parametre, takže sa dá presne zistiť, čo je odvodené a z čoho. To dá väčšinu hodnoty
za zlomok práce a nechá dvere do (B) otvorené.

**Chcem to ako ADR s odôvodnením, nie ako implementáciu, ktorá sa stane.** Ak z kódu vyplýva, že
(B) je lacnejšie, než odhadujem, ustúpim.

Súvisiaci problém, ktorý treba v ADR pomenovať aj tak: **stabilná identita odvodenej inštancie.**
Ak sú kusy odvodené a chceš práve jednému stĺpiku pridať výrez na rozvod, na čo ukazuje tá výnimka
po zmene rastra? Identita musí vychádzať z pravidla a pozície v ňom (`rošt_strop_01/pole_7`), nie
z poradia v poli. Je to ten istý problém ako topological naming, len o poschodie vyššie.

---

## 6. Konkrétne zadanie

### 6.1 Brána C1 — ekvivalencia geometrických ciest

Malé, dá sa napísať rýchlo, chráni nulový prah, ktorý je deklarovaný na dvoch miestach dokumentu.

**Prah:** pre korpus telies vyrobených OCCT musí picking cez `ketchup-interaction` vrátiť **ten istý
`SubshapeRef`** ako priame rozlíšenie nad OCCT topológiou. Nula nezhôd.

**Ak sa to nedá dosiahnuť, výsledok je ADR, nie oprava:** buď je autoritatívny resolver len jeden
a druhá cesta je akcelerátor, ktorý sa musí potvrdiť, alebo sa cesty rozdelia podľa typu telesa a to
sa zapíše.

### 6.2 Kolízny validátor v jadre

Jediná kontrola, ktorá platí pre skriňu, dom aj čokoľvek ďalšie. Čisto geometrická, beží nad
`ketchup-interaction`, OCCT ju neblokuje. Bez nej sa nedá zodpovedne púšťať žiadny generátor —
a to je preukázané FurniGenom.

- broad phase (AABB strom) → medzistupeň pre neosovo orientované kusy (OBB alebo presnejší test;
  v nábytku bolo všetko osové, v dome sú krokvy a zavetrovanie) → exaktný test na kandidátoch;
- **prah, pod ktorým sa prienik nepovažuje za kolíziu, ako pomenovaná konštanta v tolerančnom
  profile**, nie zahrabaná v kóde;
- ak sa v narrow phase použijú trojuholníky, tesselácia použitá na validáciu **musí byť súčasťou
  determinism envelope** — inak dá tá istá kontrola iný výsledok na inom builde. Pri planárnych
  telesách je tesselácia presná, pri zaobleniach a šikmých rezoch už nie.

**Vstupný korpus: prípady z FurniGenu, na ktorých kolízia najprv nefungovala.** Toto je najcennejšia
prenosná vec z FurniGenu a je dostupná dnes. Ak sa dá získať aj zoznam vyňatých prípadov
a zdôvodnenie prahu, tým lepšie.

Opačná kontrola — *nič nechýba* (kus bez podpory, spoj s jedným koncom, diera v obvode) — počká, kým
existujú spoje. Je to horšia trieda chýb než kolízia, lebo sa nedá vidieť, ale bez stykov ju nie je
nad čím spustiť.

### 6.3 Jeden nosník koniec-koniec

**Toto je najdôležitejšia položka celého zadania.** Nie architektúra, nie návrh entít — jeden
funkčný výsek celého cyklu na reálnych dátach.

Vezmi **nosník A z drevodomu** (výkresy sú k dispozícii, sú referenčný výsledok) a sprav:

1. pravidlo v texte: rozpon, počet polí, prierez, hĺbka drážky;
2. spustenie → kusy v dokumente;
3. zmena jedného čísla → prepočet;
4. výpis kót od jedného konca — musí dať `415 × 6`, `408 × 5`, `400`;
5. kolízny test nad výsledkom.

**Zámerne bez AI, bez BTLx, bez systémov a stykov ako kanonických entít.** Z toho vytvrdne, čo tie
pojmy znamenajú v kóde, a je to prvý test, ktorý meria produkt a nie kernel.

Ak to zaberie viac než pár dní, znamená to, že rozhodnutie A/B bolo zlé — a to je užitočná
informácia, nie zlyhanie.

---

## 7. Navrhované poradie

1. **CI + dva architektonické testy** — tvoj bod 1, nemením. Najvyššia páka, chráni všetko ostatné.
2. **`StateView` v1 + golden fixtures** — tvoj bod 2, s korekciou 4.4 (dve projekcie).
3. **Brána C1** — malé, urgentné.
4. **Kolízny validátor + FurniGen regresný korpus.**
5. **ADR: kde bývajú pravidlá (A/B) + identita odvodenej inštancie** — rozhodnutie, nie kód.
6. **Jeden nosník koniec-koniec.**
7. **Príznak smeru zmien v R0 reporte** — jednoriadkové, kedykoľvek.
8. Až potom `Intent` slovník a **brána D**.

Zdôvodnenie posunu brány D za body 4–6: brána D meria, či AI navrhuje použiteľné veci. Bez
validátorov a bez pravidiel by merala generovanie geometrie — teda presne to, čo vo FurniGene
nefungovalo a čo sa opravilo až validátormi. Merali by sme známy neúspech.

Body 5–7 z tvojho zoznamu (ADR o substráte, ADR o planárnom fallbacku, licenčná položka R0) sú
zápisy, nie práca, a môžu ísť kedykoľvek.

---

## 8. Čo vedome odkladám

Aby to nebolo len pridávanie:

- **BTLx export** — až po jednom fungujúcom nosníku. Je to formát, nie architektúra, a bez správnych
  entít nemá čo exportovať.
- **Systém a styk ako kanonické entity** — až po bode 6.3. Návrh týchto entít od stola je presne ten
  papierový cyklus, ktorému sa chceme vyhnúť. Nech ich tvar určí fungujúci nosník.
- **Pomenovanie plôch a výber predikátom** — až keď bude existovať niečo, čo má zmysel adresovať
  menom.
- **Kontrola „nič nechýba"** — až po stykoch.
- **Statická validácia** — mimo rozsah. Nástroj, ktorý implicitne sľubuje, že konštrukcia vydrží,
  preberá zodpovednosť, ktorú open source projekt niesť nemôže. Rozhranie to musí hovoriť nahlas:
  *„prešlo geometrickou kontrolou, statiku neposudzuje."*

---

## 9. Zhrnutie

Jadro je postavené lepšie, než som predpokladal, a väčšina mojich pôvodných výhrad bola voči kódu
neplatná. Zostáva jeden reálny nález (C1) a tri korekcie k metodike.

Podstatnejšie je, že doterajšia práca je celá o jadre a produkt je inde: v pravidlách, validátoroch
a výrobných výstupoch. Bod 6.3 je najlacnejší spôsob, ako zistiť, či sa jadro dá na to použiť —
skôr, než sa navrhne pravidlová vrstva od stola.
