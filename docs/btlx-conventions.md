# BTLx 2.3.1: súradnicové konvencie Kečupu

Stav: konvencia je pripnutá testami, **v konkrétnom importéri zatiaľ nepotvrdená**
(support report hovorí `concrete_importer_verified=false`). Overenie: otvoriť
`crates/ketchup-manufacturing/tests/fixtures/btlx/centred-60x140-drilling-2.3.1.btlx`
v BTLx prehliadači alebo v importéri stroja a porovnať s očakávaním nižšie.

## Polotovar (stock frame)

Prvok je priamy hranol: obdĺžnikový profil v rovine definície x/y, vytiahnutý
pozdĺž z. Rám polotovaru má počiatok v **minimálnom rohu** profilu, nie v
počiatku definície. Ten istý roh používajú BTLx, woodWOP aj `production_job`
(`StockBlank` v `fabrication.rs`).

| BTLx       | Os dielca | Os definície | Rozmer           |
|------------|-----------|--------------|------------------|
| `Length`   | X         | z            | dĺžka vytiahnutia |
| `Width`    | Y         | x            | šírka profilu    |
| `Height`   | Z         | y            | výška profilu    |

Bod definície `(x, y, z)` je v dielci `(z, x − x_min, y − y_min)`.

## Referenčná rovina opracovania

`UserReferencePlane` má `ReferencePoint`, `XVector` a `YVector` v súradniciach
dielca. Normála roviny je `XVector × YVector` a smeruje **do materiálu**:
vŕtanie začína v bode `ReferencePoint + StartX·XVector + StartY·YVector` a ide
o `Depth` pozdĺž normály.

## Overený príklad

Profil `(−30, −70)…(30, 70)`, dĺžka 2000 mm, otvor Ø10 hĺbky 50 mm do čela
v bode profilu `(15, 40)`:

- `Length="2000" Width="60" Height="140"`,
- vstup otvoru v dielci `(2000, 45, 110)`, koniec `(1950, 45, 110)`.

Ak importér ukáže otvor 45 mm od hrany na 60 mm strane a 110 mm od spodku na
140 mm strane, Width/Height aj smer normály sedia.

## Testy

- `btlx_of_a_60_by_140_member_centred_on_its_axis_maps_width_height_and_corner`:
  golden súbor vyššie.
- Každý BTLx golden prechádza `assert_btlx_machining_lies_in_its_blank`: každý
  bod opracovania na rovine aj v hĺbke leží v `[0, Length] × [0, Width] × [0, Height]`.
- `a_profile_centred_on_its_axis_is_machined_from_its_minimum_corner`: BTLx aj
  woodWOP merajú od rovnakého rohu.
