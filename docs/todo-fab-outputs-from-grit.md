# Fab-output consistency to move into flatland (from the grit board, 2026-09-15)

Status (2026-09-16): items 1, 2, 3 (the check), 4, 6, 7 and 8 are in the
tool; see the notes under each. Open: `pcb rename` (item 3).

The grit generator (`/Users/jake/mips32/boards/grit/pcb/build.py`,
`outputs()`) post-processes `pcb bom` / `pcb pnp` output because JLCPCB
rejected the files as written.  Each item below is a measure that belongs
in the tool so every board gets it.  The grit build.py functions named
are the working reference implementations.

## 1. The placement list must list exactly what the BOM lists (`pnp`)

**What went wrong.**  `pcb bom` leaves out parts with no LCSC number
(hand-placed parts, and parts not sourced yet); `pcb pnp` writes every
placed part.  JLCPCB refuses the upload: "U24 designator don't exist in
the BOM file".

**Wanted.**  `pnp` applies the same filter as `bom` (or both take one
`--assembled-only` rule): a part with no LCSC number, or with
`assembly: false`, is in neither file.  Print the parts left out, so a
missing number is visible at build time, and optionally write them to a
side file (grit writes `reference/hand-placed.txt`).  A `pcb bom --check`
that diffs the two designator sets would also do.

Reference: `consistent_cpl()` in grit's build.py.

**Done.** `pcb pnp` applies the BOM's filter, prints what it left out and
writes `build/assembly/reference/hand-placed.txt` with their positions;
it ends with "BOM and placement list name the same N part(s)".

## 2. Mid X / Mid Y is the part's centre, not the footprint origin (`pnp`)

**What went wrong.**  JLCPCB places at Mid X/Y.  The tool writes the
footprint origin, which for the pin headers and the DB9 is pin 1: up to
9 mm off the part's centre.

**Wanted.**  The centroid of the pad field (plated pads only: no NPTH
holes, pegs or mounting holes), or a footprint-declared centre if one
exists.  Reference: `centre_cpl()` in grit's build.py (parses `pcb pads`
and rewrites the CSV).

**Done** (2026-09-12): Mid X/Y is the centre of the plated pads' bounding box.

## 3. Reference designators with one prefix per part class (BOM review)

**What went wrong.**  JLCPCB's BOM review flags a line whose designators
do not share a prefix ("there may be multiple types of parts, but one
type of part has been matched").  Design names such as `cd_seq0`,
`cst0`, `c1` on one 100 nF line trip it on every passive line.

**What grit does.**  The board file carries a `designator` per part
(`U1`, `C7`, `R12`, `D3`, `J2`, `SW1`, `Y1`, `Q1`, `FID1`), assigned by
class in design order; the PCB's components are created under those
names, and the design names are kept in the generator and on the silk
(the sockets show both).  `reference/designators.csv` is the
cross-reference.

**Wanted in flatland.**  Nothing is required if the project uses proper
designators, but a `pcb bom --check` warning when a BOM line's
designators do not share an alphabetic prefix would catch it before
upload.  A `pcb rename` (bulk designator remap with a mapping file) would
make adopting the scheme on an existing project one command.

**Check done** (2026-09-13): the JLCPCB profiles carry `bom-prefixes` and
`designator-format` (warnings). `pcb rename` is still open.

## 4. The upload set should be obvious

Only two files go to JLCPCB.  Grit keeps `grit-bom.csv` and
`grit-cpl.csv` alone in `build/assembly` and moves everything else
(`grit-bom-all.csv`, `designators.csv`, `hand-placed.txt`) to
`build/assembly/reference`.  `pcb bom --all` could default to a
different directory or name pattern so the upload pair stands alone.

**Done:** `--all` outputs go to `build/assembly/reference/<name>-bom-all.csv`
and `<name>-cpl-all.csv`; `hand-placed.txt` lives there too.

## 5. Related, already written up

`docs/checks-from-grit-audit.md` (E2 there is item 2 here; E3 covers
inert waivers; G1 the netlist compare).

## 6. Rotation offsets in the library (`pnp`)

JLCPCB's placement preview (2026-09-15) showed `dcjack-pj-002ah-smt`
from `library-jlcpcb` 90 degrees clockwise of its pads: its
`metadata.rotation_offset` wants +90.  Grit corrects it in a CPL
post-process (`ROTATION_FIX` in build.py) because a board generator
should not edit the shared library.  Every library part's offset needs
the same check against the preview once; the offsets grit verified for
its own parts (DIP sockets, SOIC, TSSOP, SOT-23, SOT-23-5, pin headers
270; DB9 270; through-hole DIP switch 0; LEDs, 0603/0805 passives 0)
could seed a table.

**Done for what the library has:** `dcjack-pj-002ah-smt` carries
`rotation_offset: 90` with the preview noted; `cj431-sot23` now carries
the SOT-23 270 that `mosfet-2n7002` (same footprint) had verified. SOIC
270 and passive/LED 0 were already in place. `library-jlcpcb` has no
DIP, TSSOP, SOT-23-5, pin-header or DB9 parts yet; seed those with the
values above when they are added.

## 7. `pcb route`: declare every via size in the DSN (found on flint, 2026-09-16)

A board whose hand wiring uses a via other than the project's default
(flint's supply-entry pads had 0.4/0.8 mm vias next to the 0.2/0.5 mm
rule) makes freerouting 2.2.4 reject the DSN outright:

    WARN  Wiring: via padstack 'Via[0-3]_800:400_um' not found at 'GND'
    ERROR There was a parse error while reading DSN file at '(pcb': DSN structure parsing failed

then a NullPointerException in RouterSettings.applyBoardSpecificOptimizations
and exit status 0 with no .ses (`pcb route` reports "freerouting exited
... without writing ...ses").  The wiring section references a padstack
the library section never defines.  `dsn::emit` should emit one padstack
per distinct (drill, diameter) among the project's vias (and the
`(via ...)` list should name them all), not only the rules' via.
Workaround on flint: all vias at the project size.

**Done** (2026-09-16): `dsn::emit` declares a padstack for every distinct
(diameter, drill) among the project's vias and lists them all in `(via ...)`.

## 8. `pcb route --planes`: mark plane layers `(type power)` (found on flint, 2026-09-16)

`dsn::emit` writes `(layer X (type signal))` for every layer whatever
`planes` is, so freerouting routes signals across the layers that carry
the outline-following pours: on flint (4 layers, GND on In1.Cu, VCC on
In2.Cu) it put 167 of 267 traces on the two plane layers, the ground
pour ended up as 19 islands and every plane-layer via/pad reported
pour overlaps.  grit's build.py worked around it by rewriting the DSN
(`(layer In1.Cu (type signal)` -> `(type power)`) between `--dsn-only`
and its own freerouting launch.  With `planes`, a layer whose pour
follows the outline should be emitted as `(type power)` (freerouting
then keeps signal traces off it), and `Pcb.route(planes=True)` becomes
usable for a plane board without a hand-launched router.

**Done** (2026-09-16): with `--planes` (or `Pcb.route(planes=True)`) a layer
whose netted pour covers at least 90 % of the outline is written as
`(type power)`; other layers stay `(type signal)`.
