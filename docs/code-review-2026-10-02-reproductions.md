# Runtime dôkazy review — 2026-10-02

Baseline `e92f4a7`. Toto sú overenia hlavného reviewera, ktoré dopĺňajú statické správy pomocných reviewerov. Produkčné zdrojové súbory sa nemenili. Izolované reproduktory sú v `.claude/review-2026-10-02/`; tento Markdown uchováva vstupy a výsledky aj keby sa dočasné binárky zmazali.

## Overené buildy a testy

- `cargo test --locked --workspace --lib --no-fail-fast -- --test-threads=2`: PASS. App 474 testov; súhrn všetkých crates doplní hlavný report. Jeden ignorovaný test je `request_timeout_tests::sleeping_worker`, subprocess fixture iných testov.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: PASS.
- Integračná sada bez scheduler/headless skončila (`bg25-18dab77b301c428c`): 1664 passed, 1 failed (legacy_migration predpoklad pre nový príklad), 4 ignored; GUI 500 pass/0 fail. Knižničná sada spolu 686 passed/1 ignored. Finálne výsledky a obmedzenia sú v hlavnom reporte.
- Plná all-targets sada nebola spustená, pretože úvodný build workera zastavil OS lock existujúcej headless relácie. Procesy používateľa ostali nedotknuté.

## R1: Recovery dvoch vetiev — POTVRDENÉ

Izolovaný Rust program linkovaný na aktuálnu `ketchup_model` knižnicu:

1. Vytvoril dokument s definíciou `base`, uložil primárny súbor.
2. Načítal ho dvakrát ešte pred editmi.
3. Vo vetve A pridal definíciu ID 2 s menom `branch_A`, vo vetve B rovnaké ID s menom `branch_B`.
4. Pre oba DocumentStore zavolal `save_work_recovery_document_store_with_container` s rovnakou platnou identitou primárneho súboru.
5. Opäť otvoril dokument cez `load_file_with_source`.

Výstup:

```text
both_recovery_writes_succeeded=true; checkpoint_changed=true; recovered_branch=branch_B
PROBES_CONFIRMED
```

Oba zápisy uspejú, ale existuje iba druhá recovery vetva. Primárny súbor zostal nezmenený (asertované). Testovací adresár bol samostatný `%TEMP%/ketchup-review-recovery-142912`, nie používateľský model. Potvrdzuje M1; nespúšťal crash používateľského procesu, iba priamo overil obsah dostupný pre recovery.

## R2: Duplicate extension mutuje napriek Err — POTVRDENÉ

`ContainerData::insert_extension` najprv vložil `(org.example.audit, data.bin, required=false, bytes=A)`. Druhé vloženie rovnakého kľúča `(required=true, bytes=B)`:

```text
duplicate_insert_error=Err(DuplicateContainerEntry); retained_payload=[66]; required=true
```

Byte 66 je B. Asersion overila Err aj skutočne prepísaný payload. Potvrdzuje M2 vrátane zmeny required flagu.

## R3: Revolúcia a exact kandidáti — POTVRDENÉ

Verejné `ketchup_program::run` + `exact_candidates`:

```python
p = revolve("tube", profile=[(0,10),(100,10),(100,20),(0,20)], axis=[(0,0),(1,0)])
b = box("inside_wall", (5,2,2), at=(40,-19,-1))
print(part_info(p).local_min, part_info(p).local_max)
print(reach(p, (0,-1,0)))
```

```text
log=["(-100.0, 10.0, -100.0) (100.0, 20.0, 100.0)", "-10.0"]
issues=[]
exact_candidates={}
```

Analyticky správna obálka plnej revolúcie okolo X je `(0,-20,-20)..(100,20,20)`. Druhý box leží v materiáli rúry. Evaluator aj explicitná asercia potvrdili chybnú obálku a prázdnu množinu kandidátov. **OCCT intersection nebol v tomto reproduktore spustený**; tvrdenie sa týka nesprávneho broad-phase a vynechania exact kandidáta. Potvrdzuje PRG-01.

## R4: Falošný contact trojuholníka — POTVRDENÉ

```python
a = extrude("triangle", profile=[(0,0),(100,0),(0,100)], distance=30)
b = box("corner", (10,10,30), at=(80,80,30))
print(contact(a,b))
```

Výsledok je contact s `face_a=end`, `face_b=z-`, `origin=(80,80,30)`, `size=(10,10)`, points `(80,90,30),(80,80,30),(90,80,30),(90,90,30)`. Report `issues=[]`, kandidát pre exact je triangle. Všetky body patch majú x+y>=160, hoci horná plocha trojuholníka má x+y<=100. Asertované cez skutočné public `contact`. Neskoršie exact refine reportu neruší nesprávny výsledok už vrátený builtin funkciou v programe. Potvrdzuje PRG-02.

## R5: Kolíky mimo otočenej dosky — POTVRDENÉ

```python
a = box("base", (300,300,30))
b = box("top", (200,20,30), at=(50,100,30))
rotate(b, axis=(0,0,1), angle=45, pivot=(150,110,0))
print(dowels(a,b,dowel="6x30",count=2,margin=10))
```

Výsledné svetové body sú `(82.21825406947978,110,30)` a `(217.78174593052023,110,30)`. Report obsahuje dve chyby `hole_outside_face` na `top`, s lokálnymi súradnicami `(52.071,57.929)` a `(147.929,-37.929)` pri platnej ploche 200x20. Evaluátor aj validátor chybu potvrdili; runtime tu nevydal falošný Pass, ale helper vyrobil neplatné otvory z platného kontaktu. Potvrdzuje PRG-03.

## Reprodukčné príkazy a obmedzenia

`rustc --edition=2024 <probe.rs> -L dependency=target/debug/deps --extern ketchup_model=<aktuálny rlib> -o <probe.exe>`; programový probe navyše `--extern ketchup_program=<aktuálny rlib>` a native cesta k `windows_x86_64_msvc-0.52.6/lib` z Cargo cache. Prvý link programu bez tejto native cesty zlyhal LNK1181, po doplnení prešiel. Nešlo o chybu projektu v Cargo builde.

Reproduktory linkujú knižnice z buildov aktuálneho nezmeneného stromu. Nie sú to nové regresné testy integrované do repozitára; odporúčanie je previesť ich pri oprave na trvalé testy správneho výsledku, nie asertovať dnešné chybné správanie.

**Doplnené runtime dôkazy R6–R8:** `history_cycle_probe.exe` potvrdil source-only rollback: `rollback_result=Err(NoOpRollback); source="# B\n"` namiesto návratu k A (M5). Ten istý reproduktor s argumentom `cycle` vytvoril root 1 a cyklus 2↔3, potom ConvertGroupToComponent(1); vypísal `ENTERING_CYCLE_BATCH` a nedokončil sa do 3 s. `subprocess.run(timeout=3)` ukončil iba tento vlastný proces; statická slučka bez visited potvrdzuje príčinu (M3). `push_pull_probe.exe` zavolal skutočný `rewrite_rule_program_push_pull` pre `H=10`, `distance=H*H`, požadovaný posun +21: `H=11.049895010499; requested_height=121; produced_height=122.100179743051; error_mm=1.100179743051` (PRG-04). Probe bol zlinkovaný na aktuálnu ketchup-application; potreboval OCCT link cestu `third_party/occt-install-r0-v1/win64/vc14/lib` a DLL cestu `.../bin` v PATH. Prvý launch bez DLL cesty nedokončil štart do 15 s a vlastný probe bol ukončený deadline; s DLL cestou prešiel exit 0. Nešlo o timeout samotného Push/Pull. Všetky štyri probe zdroje ostávajú v `.claude/review-2026-10-02/`. Protokolový DoS sa zámerne nespúšťal v používateľovom GUI.
