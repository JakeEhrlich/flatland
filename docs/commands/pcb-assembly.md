# pcb-assembly(1)

## NAME

pcb bom, pcb pnp — bill of materials and pick-and-place files for SMT
assembly (JLCPCB/LCSC format by default).

## SYNOPSIS

```
pcb bom [-f jlcpcb|csv] [-o FILE] [--all]
pcb pnp [-f jlcpcb|csv] [-o FILE] [--all]
```

## DESCRIPTION

Together with `pcb gerbers` these produce everything an assembly service
needs: the gerber zip, a BOM that maps designators to orderable parts, and a
placement (CPL) file with the centre, side and rotation of each part.

Which part to order comes from the component library:

* `metadata.lcsc` in the component file — for specific parts (`ne555dr`
  → `C7593`);
* the instance parameter `lcsc` — for generic passives whose part number
  depends on the value (`pcb add R1 resistor-0603 --param value=10k
  --param lcsc=C25804`); it overrides the component metadata;
* `metadata.mpn` / `metadata.manufacturer` — informational, shown in the
  generic CSV and used as the BOM comment when there is no `value`;
* `metadata.assembly: false` — the part is not placed by the assembler
  (solder pads, test points, hand-soldered connectors); excluded from both
  files unless `--all`;
* `metadata.rotation_offset` — degrees added to the placement rotation in
  the CPL, for parts whose reel orientation differs from the footprint's
  zero orientation.

Virtual components (no footprint) never appear.

## pcb bom

Groups instances by (component, value, LCSC number) into line items.

`jlcpcb` format (default), the columns JLCPCB's upload expects:

```
Comment,Designator,Footprint,LCSC Part #
10k,"R1,R2",0603,C25804
NE555DR,U1,SOIC-8_3.9x4.9mm_P1.27mm,C7593
```

`Comment` is the `value` parameter, else the MPN, else the component
name. Parts without an LCSC number are left out with a warning (JLCPCB
cannot source them) unless `--all`; `pcb pnp` leaves out the same parts.

`csv` format: `Designators,Quantity,Component,Value,Footprint,
Manufacturer,MPN,LCSC,Assemble,Description` — everything known, one line
per line item, including non-assembled and number-less parts.

Default output: `build/assembly/<name>-bom.csv`; with `--all`,
`build/assembly/reference/<name>-bom-all.csv`, so the two files JLCPCB
takes stand alone in `build/assembly`.

## pcb pnp

One row per placed part that is in the BOM: the two files must name
exactly the same designators, or JLCPCB refuses the upload ("U24
designator don't exist in the BOM file"). Parts the BOM leaves out
(`assembly: false`, or no LCSC number) are left out here too, listed on
the console, and written with their positions to
`build/assembly/reference/hand-placed.txt` for whoever fits them.

`jlcpcb` format:

```
Designator,Mid X,Mid Y,Layer,Rotation
U1,10.0000,8.0000,Top,0
```

* `Mid X/Y` are the centre of the part's plated pads in millimetres in
  the board frame — the same origin and y-up orientation as the gerbers,
  which is what the assembler aligns to. JLCPCB places at Mid X/Y, and a
  footprint whose origin is pin 1 (headers, a DB9) would otherwise land
  up to its half length off.
* `Layer` is `Top` or `Bottom`.
* `Rotation` is counter-clockwise degrees (0–360) of the footprint plus
  the component's `rotation_offset`. JLCPCB's zero orientation is defined
  per part in their library and often differs from IPC/KiCad-style
  footprints: e.g. their SOIC/SOP parts sit body-horizontal with pin 1 at
  the bottom-left, while our SOIC-8 footprint has pin 1 top-left with pins
  running down the left side — that is their orientation rotated 270°
  CCW, so `ne555dr` carries `"rotation_offset": 270`. Symmetric passives
  need none. Check the placement preview when ordering (polarised parts:
  ICs, LEDs, diodes, electrolytics) and record any correction in the
  component's `rotation_offset` so every later board gets it right.

`csv` format adds `Footprint,Value,LCSC` columns and uses `Side` instead
of `Layer`.

Unplaced assembled parts make the command fail (after writing the file
without them). With `--all` every placed part is written, to
`build/assembly/reference/<name>-cpl-all.csv`.

## ORDERING FROM JLCPCB

1. `pcb check --strict`, `pcb gerbers`, `pcb bom`, `pcb pnp`.
2. Upload `build/gerbers/<name>-gerbers.zip` as the PCB.
3. Choose SMT assembly, upload `build/assembly/<name>-bom.csv` and
   `<name>-cpl.csv` (the only two files in that directory; anything
   else is under `reference/`).
4. In the part-confirmation step check every line matched the intended
   LCSC part, and in the placement preview check orientations (polarised
   parts: LEDs, ICs, electrolytics).

Basic parts (no per-part loading fee) are worth preferring; the `jlcpcb`
library index in this repository lists which parts are basic.

## EXAMPLES

```
pcb add R1 resistor-0603 --param value=10k --param lcsc=C25804
pcb add U1 ne555dr
pcb bom && pcb pnp
pcb bom -f csv -o build/full-bom.csv --all
```

## SEE ALSO

pcb-gerbers(1), pcb-index(1), pcb-place(1).

Mid X/Y in the placement list is the centre of the part's plated pads, not
the footprint origin: JLCPCB places at the part centre, and footprints
whose origin is pin 1 (pin headers, connectors with mounting holes) would
otherwise land up to a body length away.
