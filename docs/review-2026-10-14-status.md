# Review 2026-10-14: stav opráv

Review (`docs/review-2026-10-14.md` na vetve `claude/optimistic-goodall-2aqwdt`) bol robený na
`b40f464` (0.1.0). Každý nález sa tu overuje na aktuálnom `main`: reprodukciou alebo čítaním kódu.
Pretrvávajúci nález sa opraví spolu s testom, ktorý bez opravy padne.

Stavy: **opravené** (test padne bez opravy), **neplatí** (na `main` sa nereprodukuje),
**otvorené** (overené, oprava čaká), **neoverené**.

Poradie podľa §8 review: Hneď → Tento týždeň → Potom.

## P1

| ID | Stav | Oprava | Test |
|---|---|---|---|
| FMT-1 | opravené | `snapshot_codec::decode` použije každú konverziu len pre formáty zapísané pred ňou: plech V1 len pre < 101, `tag` → `tags` len pre < 102. Premenovanie rolí validátorov (formát 100) sa robí len pre < 100. Bez opravy golden 102 dostal digest pisateľa `b276203dbda20d1d` namiesto `32004c2fb9d7739d` a plech vo formáte 101 padol na „missing field `width`“ | `snapshot_codec::tests::format_101_and_102_sheet_metal_on_tagged_parts_opens_under_its_writer_digest`, `format_golden::a_format_102_file_with_history_opens_with_every_revision_it_was_written_with` (digest pisateľa) |
| PRG-M1 | otvorené | R10-B: vyhodnocovanie programu v samostatnom procese | |
| DR-1 | neoverené | R10-B | |
| GE-N2 | neoverené | R10-B, sonda | |
| VW-1 | neoverené | R10-A | |
| VW-2 | neoverené | R10-B | |
| VW-3 | neoverené | R10-B | |
| ST-N1 | otvorené | R10-B (prenesené z `cbec147`) | |
| MF-N1 | neoverené | R10-A (prenesené z `cbec147`) | |
