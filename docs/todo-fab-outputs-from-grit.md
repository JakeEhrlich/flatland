# Fab-output consistency to move into flatland (from the grit board, 2026-09-15)

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

## 2. Mid X / Mid Y is the part's centre, not the footprint origin (`pnp`)

**What went wrong.**  JLCPCB places at Mid X/Y.  The tool writes the
footprint origin, which for the pin headers and the DB9 is pin 1: up to
9 mm off the part's centre.

**Wanted.**  The centroid of the pad field (plated pads only: no NPTH
holes, pegs or mounting holes), or a footprint-declared centre if one
exists.  Reference: `centre_cpl()` in grit's build.py (parses `pcb pads`
and rewrites the CSV).

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

## 4. The upload set should be obvious

Only two files go to JLCPCB.  Grit keeps `grit-bom.csv` and
`grit-cpl.csv` alone in `build/assembly` and moves everything else
(`grit-bom-all.csv`, `designators.csv`, `hand-placed.txt`) to
`build/assembly/reference`.  `pcb bom --all` could default to a
different directory or name pattern so the upload pair stands alone.

## 5. Related, already written up

`docs/checks-from-grit-audit.md` (E2 there is item 2 here; E3 covers
inert waivers; G1 the netlist compare).
