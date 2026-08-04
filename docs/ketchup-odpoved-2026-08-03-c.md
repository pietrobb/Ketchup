# Kečup — odpoveď na tri dopovedané body

**Dátum:** 3. 8. 2026
**Nadväzuje na:** `ketchup-zadanie-2026-08-03.md`, `ketchup-odpoved-2026-08-03-b.md`

Všetky tri body beriem. Sú vecné a dva z nich menia poradie práce, nie iba jej obsah. Ku každému
mám ale jednu korekciu z kódu alebo z domény, ktorá mení tvar implementácie — nie záver.

---

## 1. Sémantika Open — súhlas s treťou možnosťou, ale polovica z nej už existuje

Trichotómia je správne postavená a tretia možnosť je správna voľba. Doplnenie z repozitára:

**„Uložiť oboje" nie je nová práca — je to súčasný formát.** `crates/ketchup-core/src/persistence.rs`
zapisuje pri `PRODUCT_SCHEMA = 2` obe vrstvy do jedného súboru: najprv celý uzlový graf (id, meno,
`source_token` kóty, binárnu hodnotu, zoznam závislostí), potom celý produktový model (definície,
featury, výskyty, skupiny, jednotky, `document_id`). Súbor teda **už dnes drží pravidlá aj odvodené
kusy**. Chýba jediná vec z tvojho návrhu — digest vstupov.

**Čo chýba presne.** Digest je dnes `digest_snapshot` — jeden rolujúci hash cez celý snapshot
(`document.rs:1786`, `Digest::node`). Vie povedať „dokument sa líši", nevie povedať „ktorý kus je
zastaraný". Na to, čo navrhuješ, treba **per-uzol Merkle digest**: `digest(uzol) = H(source_token,
výraz, zoradené digesty závislostí)`. Dokumentový digest z neho potom vypadne ako koreň, čiže sa
nezavádza druhá pravda o identite — len sa sprístupní medzistupeň, ktorý sa dnes zahadzuje.

**Dve korekcie k tvojmu tvaru cache key.**

**(a) Digest vstupov nestačí. Musí obsahovať identitu vyhodnocovača.** Zhoda digestov vstupov pri
zmenenej verzii evaluátora alebo backend buildu znamená „vstupy sedia, výsledok je iný" — a to je
presne zlyhanie, kvôli ktorému §7.3 vôbec existuje. Kľúč teda musí byť `(digest vstupov, verzia
evaluátora pravidiel, backend build id)`, rovnako ako to má cache o poschodie nižšie. Bez toho
tretia možnosť ticho degraduje na druhú.

**(b) Treba explicitne určiť, kto pri nezhode vyhráva — inak Open poruší D-08.** Ak sa pri nezhode
prepočíta a zapíše, Open sa stáva mutáciou, ktorá obchádza `apply_batch`. Návrh: **uložené kusy
zostávajú autoritatívne**, otvorenie prebehne, audit ohlási zoznam uzlov s nezhodou a prepočet je
**samostatný používateľský príkaz**, ktorý ide cez `apply_batch` a je jedným undo krokom. Tým sa
jediná mutačná cesta zachová a používateľ vidí rozdiel skôr, než ho prijme.

**(c) Nezhoda nemá len jednu príčinu.** Okrem zmeny backendu: starší alebo novší zapisovateľ,
ručne upravený súbor, neúplná sada pravidiel. Audit teda musí príčinu **pomenovať**, nie len
ohlásiť rozdiel — inak je hlásenie nepoužiteľné presne vtedy, keď je najpotrebnejšie.

Prakticky: schéma sa dvíha na 3 (uzol dostane výraz a digest), precedens `LEGACY/RESEARCH/PRODUCT`
existuje vrátane roundtrip testov na každú schému, takže je to prírastok, nie prepis.

**Súhlasím so záverom:** bez tohto rozhodnutia vznikne odpoveď mlčky v perzistencii. Ide do toho
istého ADR ako samostatná sekcia.

---

## 2. Deklarovaný spoj — beriem ako podmienku, nie ako možnosť

Máš pravdu a moja formulácia bola príliš voľná. Bez neho 6.3a buď hlási desiatky falošných kolízií,
alebo má kolízny test vypnutý — a potom nemeria nič. Poradie `pravidlo → kusy → deklarovaný spoj →
kolízny test` beriem.

**Jedna korekcia, ktorá rozhoduje o tom, či je to sito alebo diera:**

**Výnimka musí byť ohraničená objemom, nie párom kusov.** Ak deklarovaný spoj vyníma dvojicu
`(nosník, profil)` z kolízneho testu plošne, potom dva kusy, ktoré prejdú **cez seba naskrz** —
lebo sa po zmene rozostupu posunuli o 300 mm — prejdú tiež. Spoj preto musí niesť vlastný objem
(kváder polodrážky 20/20) a platí toto: prienik **vnútri** deklarovaného objemu je očakávaný,
prienik **mimo** neho je naďalej chyba. To je jednoriadková zmena v sémantike a rozdiel medzi
validátorom a alibi.

**Tri verdikty, nie dva:**

| stav | verdikt |
|---|---|
| prienik bez deklarovaného spoja | chyba |
| prienik vnútri deklarovaného objemu spoja | v poriadku |
| prienik mimo objemu spoja | chyba |
| deklarovaný spoj s prázdnym prienikom | **chyba** |

Posledný riadok je tvoj „zadarmo tretí" a súhlasím, že je v doméne najcennejší — patrí medzi
kritériá brány, nie medzi varovania. „Dva kusy sa mali stretnúť a minuli sa" je presne tá chyba,
ktorá sa inak zistí až na CNC linke.

**A jedna hranica, ktorú si treba ustrážiť pri implementácii:** spoj je kanonická entita — vlastný
`CanonicalCommand`, jeden undo krok, súčasť digestu, číta ho validátor **z dokumentu**. Ak by žil
vo vedľajšej tabuľke validátora, vznikne druhá autorita nad modelom, čo je presne to, čo invariant
misie zakazuje.

---

## 3. SAT s rozkladom — súhlas, a ide to ešte ďalej, než píšeš

Rozklad hranola s pravouhlými drážkami na kvádre je exaktný a súhlasím, že tým 6.3b nespadne späť
na tesseláciu. Doplním, že hranica je ďalej, než ju kladieš:

**Šikmé rezy rozklad vôbec nepotrebujú.** Skrátenie krokvy pod uhlom je rez polrovinou, a rez
polrovinou konvexného telesa je konvexné teleso. SAT ide priamo, bez rozkladu, bez aproximácie.
Takže **celá prizmatická tesárska doména vrátane sklonov strechy** ostáva exaktná; tesselácia
prichádza na rad až pri oblúkoch, kruhových otvoroch a oblých profiloch.

**Tri veci, ktoré rozklad musí splniť, aby nezaviedol nový problém:**

1. **Rozklad musí byť deterministický a kanonický.** Počet a poradie zložiek nesmie závisieť od
   poradia iterácie — musí vypadnúť z poradia featur na definícii. Inak sa nestabilita presunie
   z tesselácie do rozkladu a digest začne skákať bez zmeny modelu.
2. **Na kolíziu stačí konvexné *pokrytie*, nie disjunktný rozklad.** Postačuje, aby zjednotenie
   zložiek bolo teleso; prekryv medzi zložkami kolízii nevadí a je lacnejší a stabilnejší.
   Ale **nesmie sa recyklovať na objem a hmotnosť** — tam prekryv počíta dvakrát. Nech je to
   v kóde pomenované tak, aby si to nikto nepomýlil.
3. **Cena je kvadratická v zložkách.** Broad phase preto zostáva na AABB celých kusov, narrow phase
   beží zložka × zložka až po prieniku obalov, a výnimka spoja sa vyhodnocuje **na úrovni dvojice
   kusov s ohraničeným objemom** — nie na úrovni zložiek. Inak sa explózia zložiek presype do
   sémantiky spoja.

**A ešte jedna poznámka k tomu, čo príde po oblúkoch:** prvý krok tam nie je „presnejšia
tesselácia", ale **konzervatívny obal** — aproximácia, ktorá smie hlásiť falošnú kolíziu, ale nikdy
nesmie kolíziu zmeškať. Tým sa vstup do determinism envelope odsúva ešte raz, lebo konzervatívny
obal nepotrebuje reprodukovateľnosť na ULP, potrebuje len stranu chyby.

---

## Čo z toho mení plán

- **ADR (A/B)** dostáva druhú povinnú sekciu: *sémantika Open* — uložiť oboje + per-uzol digest
  vstupov vrátane identity evaluátora, uložené kusy autoritatívne, prepočet ako explicitný príkaz.
- **6.3a** dostáva štvrtú položku pred kolízny test: *deklarovaný spoj* ako kanonická entita
  s vlastným objemom, a štvrtý verdikt *spoj bez prieniku = chyba* medzi kritériá brány.
- **6.3b** nemení nástroj: SAT nad konvexným pokrytím, exaktne. Tesselácia až s oblúkmi, a aj
  potom najprv ako konzervatívny obal.
- **C1a** zostáva na druhom mieste, ako si potvrdil.

Nezhoda po tejto výmene nezostáva žiadna. Zvyšok je implementácia a jedna vec, ktorá sa nedá
odargumentovať ani z jednej strany: **projekt stále nemá CI**, takže všetky tieto invarianty drží
disciplína, nie stroj. To je v poradí prvá položka, ktorá nie je názorová.
