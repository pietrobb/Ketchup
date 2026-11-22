# Review 2026-11-22 (ac): stav opráv

Review (`docs/archive/reviews/review-2026-11-22-ac.md`, PR #195) bol robený na `b66f5d12`
(0.3.4-dev). Opravy išli v poradí podľa §7: P1, potom P2, potom P3 po skupinách, každá v
samostatnom PR s testom, ktorý bez opravy padne. Záznam v `CHANGELOG.md` (Unreleased) je tentoraz
v tomto PR pre všetky opravy naraz.

Stavy: **opravené** (zlúčené na `main`), **odložené** (s dôvodom), **na rozhodnutie vlastníka**,
**už riešené** (oprava bola na `main` pred review alebo v inom PR).

## P1

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1122ac-PG-1100 | opravené | #196 | Prút otáčajúci sa okolo jednej skrutky na každom konci nesie silu pozdĺž priamky cez skrutky (`loads/anchorage/joint.rs`), nie pozdĺž osi obalu; smer sondy, kosínus aj vstup sily idú z nej. Sonda z review (doska 300 × 1 000 × 40, skrutky v (x − 135, 40) a (x + 5, 950), stĺp 20 × 400 × 2 400) drží ~24 N/mm (ručne ~24) a neprejde proti 56; predtým 214,9 a **pass**. So skrutkami na osi dosky ostáva ~211. |

## P2

| ID | Stav | PR | Oprava |
|---|---|---|---|
| R1122ac-PG-1101 | opravené | #201, #206 | Dva uzemnené diely, ktoré sa len dotýkajú, sa tlačia, ale ťah neprenášajú; cesta, ktorá cez nespojený dotyk tlačí, sa overí aj v opačnom smere a berie sa slabšia. Stĺp ťahaný od bloku drží ako sám (0,3–1 N/mm), nie 247, a nosník neprejde; priskrutkovaný k bloku drží viac ako 10 N/mm. #206 (capability review 1023): rovnako pri šikmom dotyku — pri 46° v pôdoryse blok predtým držal 235,6 a 185,2 N/mm (stĺp sám 1,374 a 1,216), teraz nič navyše. Spevnenie, ktorého sila nenájde cestu k zemi, drží 0 N/mm (predtým bez údaja o ukotvení a nosník sa kontroloval ako spevnený). Základ výpočtu hovorí, že nespojený dotyk nenesie ťah (#208). |
| R1122ac-PS-1100 | opravené | #199 | Dokument, v ktorom je skupina, výskyt, definícia, značka alebo `classification_dimensions` pod kľúčom iným než svoje `id`, sa odmietne s `canonical.entry_under_other_id` skôr, ako sa v mapách niečo hľadá. Test prepisuje uložené `document.bin` formátov 98, 108, 112, `release-0.3.0` a súboru asistenta a kontajner znovu zapečatí; bez opravy skupiny a definície spadnú a dimenzie sa otvoria. |
| R1122ac-VW-1100 | opravené | #198 | Validátor GLB balíka `.ketchup-view` sčíta indexy siete pre každý uzol, ktorý ju kreslí, a nad 48 M kreslených indexov (trojnásobok uloženého stropu) balík odmietne (`glb.nodes`). Balík z review (50 001 uzlov na sieť s 5 M trojuholníkmi) sa odmietne pri čítaní. Dáta pre výber podľa siete namiesto inštancie: odložené, pozri nižšie. |
| R1122ac-PG-1102 | opravené | #204 | Prekrývajúce sa dvojice a vypísané kolízie sa zastavia na limite a správa povie, že zoznam je skrátený; `ketchup-program check` odpovie do svojho termínu. 5 000 prekrývajúcich sa dosiek predtým bežalo viac ako 300 s. Dvojice sa hľadajú podľa blízkosti vo všetkých troch osiach (`near_pairs.rs`), nie len v x: 5 000 dosiek nad sebou trvalo 20,7 s, teraz 0,19 s (capability review 1026). Dvojice, ktorých obaly sa prekrývajú, ale telesá rozhodnú až presné tvary, majú vlastný limit 2 000 a nezapočítavajú sa do 100 kolízií; limit prekrývajúcich sa dvojíc je 20 000, takže príklad domu si zachová všetkých 2 650. Testy termínu workera v `ketchup-scheduler` a `ketchup-application` potrebujú pomalý model, preto používajú hustý blok kvádrov. |
| R1122ac-PG-1103 | opravené | #200, #205 | Vnorenie sa počíta z tokenov pred akoukoľvek rekurziou a nad 200 úrovní sa odmietne ako `syntax_error` s riadkom; vyhodnotenie beží na vlastnom zásobníku, ktorý unesie každú prepustenú úroveň. Komentáre sa do vnorenia nepočítajú. #205: worker v okne odpovie na vnorenie 3 000 chybou na riadku 2 a zostane pre ďalší program. |

## P3 (§4)

| Skupina | Stav | PR | Čo |
|---|---|---|---|
| Statika | opravené | #208 | Nad pomernou štíhlosťou 0,3 diel cesty drží k_c f_c,0,k A (EN 1995-1-1 (6.25)–(6.29), β_c 0,2), nie Eulerovu silu: pri 0,49 predtým 1 412,6 kN proti drveniu 240 kN. Základ výpočtu uvádza, že priamka cesty ide dielom bez momentu z jej vzdialenosti od ťažiska, že sa berie len najtuhšia z ciest k zemi, že `diaphragm()` je tuhá doska na pružinách spojov a že nespojený dotyk dvoch uzemnených dielov nenesie ťah. |
| Tepelný výpočet | opravené | #208, #209 | Strop `r` vrstvy vzduchu podľa smeru toku tepla prvkom (EN ISO 6946 tab. 8: vodorovný pre stenu, nahor pre strechu), ako `surfaces()` pre R_si; stena predtým prešla s 0,19–0,23, kde tabuľka dáva 0,18. Pri kondenzácii a chýbajúcom mieste stavby rada menuje aj chýbajúce miesto. Tepelná validácia (~207 B na štvorsten) sa počíta do rozpočtu pamäte FEA (#209). |
| CAM, balík a FEA | opravené | #209 | Odmietnutie balíka prebaleného iným nástrojom to povie a žiada balík, ako ho zapísal Kečup. Rozpočet pod 1 MiB sa vypíše v KiB, nie „the 0 MiB it may use“. |
| Ukladanie a MCP | opravené | #203, #202 | PS-1101: uloženie zachová práva nahrádzaného súboru, ide cez symbolický odkaz k súboru, na ktorý ukazuje, a `.recovery` dostane práva dokumentu a leží pri cieli odkazu (`persistence/staged.rs`). MCP-1100: dávka JSON-RPC, holé číslo alebo reťazec a id nad `u64` dostanú odpoveď (pole pre dávku, inak Invalid Request). MCP-1101: limit riadka sleduje požiadavku okna (8 MiB, predtým 32 MiB) a čakajúce volania majú okrem počtu aj rozpočet bajtov. |
| Opravy nestabilných testov | opravené | #205 | Štart workera dostane len čas, ktorý termínu kontroly zostáva (najviac 30 s), a požiadavka čas, ktorý štart nechal; s termínom 1 s kontrola skončí pod 10 s (predtým 30 s; test `perf_*` beží v noci). `worker_client.rs` kontroluje `memory_limit_exceeded` pred `RequestTimedOut`, takže skutočný nedostatok pamäte sa neukáže ako timeout. |
| Bezpečnosť a CI | opravené | #207 | Zlyhaný export zapíše každý list a pridružený súbor, ktorý nevrátil, aj kópiu `.work-recovery`. Odmietnutie pre cudziu zmenu (`TargetChanged`) sa nezapíše ako zápis AI. `test_ci_root_tests.py` neráta `--list`, `--help`, `&`, `continue-on-error` a `set +e` ako beh testov pod rootom. ENOENT pri `openat` a EINVAL pri `readlinkat` neradia „set HOME“. Nečitateľné `/proc/self/cgroup` sa číta ako koreň v1. |

## Stav nálezov z review 11-21 (§3)

Čiastočné body z §3 review sú dokončené v P3 vyššie: statika nad štíhlosťou 0,3 (#208), smer toku
tepla pri strope `r` (#208), tepelná validácia v rozpočte (#209), zlyhaný export a
`test_ci_root_tests.py` (#207). Odložené body R1121ac (hľadanie najväčšej veľkosti, loft U, limity
riadkov, `room_areas`) ostávajú odložené s dôvodmi v `review-2026-11-21-ac-status.md`.

## Štrukturálne návrhy (§6)

| Bod | Stav | PR | Čo |
|---|---|---|---|
| 1. Statika: priamka sily zo spojov a smer dotyku | opravené | #196, #201, #206 | Obe vstupné chyby sú opravené tam, kde vznikajú: priamku sily dáva skrutka na každom konci (`joint.rs`), tlak alebo ťah cez nespojený dotyk rozhoduje `Footing::bears` (`anchorage.rs`). Zvyšok výpočtu ostal, ako navrhuje review. Jedna spoločná funkcia „spoj“, ktorá by vracala body prenosu aj to, či spoj nesie ťah, postavená nie je; zatiaľ ju nič ďalšie nepotrebuje. |
| 2. Celistvosť dokumentu ako kontrola | čiastočne | #199 | Validácia pri načítaní overuje kľúč mapy proti `id` pre každú kolekciu, na ktorú sa spolieha indexovanie. Mutačná sonda ako nočný test: odložené, pozri nižšie. |
| 3. Limity podľa práce, nie veľkosti vstupu | opravené | #198, #204, #200, #202 | Trojuholníky × inštancie v balíku Viewera, limit dvojíc a termín kontroly programu, hĺbka vnorenia programu a limit riadka MCP podľa požiadavky okna; každý limit sa nahlási s príčinou. |

### Odložené a na rozhodnutie

| Nález | Stav | Dôvod |
|---|---|---|
| VW-1100: dáta pre výber zdieľať podľa siete, nie podľa inštancie | odložené | Strop 48 M kreslených indexov ohraničuje aj prácu výberu a `BodyPoints::of`, takže balík z cudzieho zdroja už Viewer nezamrazí. Zdieľanie podľa siete je úspora pamäte pri veľkých platných balíkoch; bez merania na takom balíku by zmena nemala test, ktorý bez nej padne (§7). |
| §6 bod 2: mutačná sonda (150 mutácií CBOR na súbor, so znovu zapečatením) ako nočný test | odložené | Fuzzer `fuzz/fuzz_targets/load.rs` beží na každom PR, ale jeho mutácie neprejdú kontrolným súčtom, takže k validácii, ktorú PS-1100 obišiel, sa nedostanú. Sonda, ktorá kontajner po mutácii znovu zapečatí, je nový test harness, nie oprava nálezu; triedu chýb PS-1100 pokrýva test v #199 pre všetky staré formáty. |
| `loads.rs` (2 985) a `member_check.rs` (2 981) pri limite 3 000 | odložené | Pozorovanie z metrík (§5), nie chyba; `check_file_sizes.py` limit drží. |
