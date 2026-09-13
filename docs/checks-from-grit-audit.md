# Checks to add, from the grit pre-fab audit (2026-09-12)

What an independent KiCad DRC (`pcb export kicad` + `kicad-cli pcb drc`),
an independent gerber parse, and a datasheet review found on the grit
board that `pcb check` did not.  Each item: the defect, how to detect it,
the suggested rule name and severity, and the evidence on grit
(`/Users/jake/mips32/boards/grit/pcb`, commits up to `40a672d`).

## A. Copper connectivity

### A1. End-to-end trace junctions (`trace-junction`, error)

**Defect.**  Traces are flat-ended.  When two traces of one net meet end
to end (the router's necked-down entry into a fine-pitch pad, width
0.1874, continuing as a 0.25 mm trace), the copper touches only along
the shared end line (collinear) or in a wedge (at an angle): zero or
near-zero contact width.  `dangling-trace` warned on both ends,
`status` said the net was connected, KiCad said unconnected (its
segments are round-capped; the export pulls each end back by half its
width, so the two ends just touch at a point).  Physically it etches as
a hair.

**Detect.**  For every trace endpoint that lies on no pad, via or
plated hole of its net: if it coincides (≤ 0.01 mm) with another trace's
endpoint of the same net and layer, flag it.  Do not accept it as a
connection.

**Fix in the tool.**  `route --import` should merge consecutive session
wires of one net into a single polyline when the widths are equal, and
when they differ, extend the narrower one into the wider one by at least
the wider one's half-width (the grit build does this after import with
a bridging trace of the narrower width reaching 0.25 mm into both:
`weld_junctions` in build.py, 42 junctions on that board).

**Consistency.**  `status`/connectivity must use the same contact rule
as `dangling-trace`: a connection needs positive-area overlap, not a
shared edge.  Today `status` reports 0 unrouted while `dangling-trace`
reports the ends open; that hid the problem.

### A2. Coincident vias (`hole-to-hole`, error; and dedupe on import)

**Defect.**  The router's session repeated vias that the hand wiring
already had (the escape vias offered to it as one-pin parts): six pairs
of vias at exactly the same position, drilled and flashed twice.
`hole-to-hole` (min 0.45) did not fire at distance 0.

**Detect.**  Any two drills whose centres are closer than the sum of
their radii, same net or not, is an error.  `route --import` should
drop a session via that coincides with an existing via of the same net
(the grit build dedupes after import).

## B. Solder mask

### B1. Mask dam between any two openings (`mask-dam`, warning at 0.1, error below ~0.05)

**Defect.**  Two vias 0.65 mm apart (mask openings 0.6 mm) on different
nets left a 0.05 mm dam; the fab removes it and two bare rings of
different nets sit 0.15 mm apart under HASL.  `mask-dam` only considered
pad openings; via and hole openings were ignored.  Found at the SC-88
mux and the TSSOP transceiver (ground/supply escape vias of neighbouring
pads).

**Detect.**  Compute dams between every pair of mask openings: pads,
vias, plated and non-plated holes, each grown by its `mask_expansion`.
Different nets: warning below `mask_dam_min` (JLCPCB 0.1 mm green,
0.13 mm other colours; recommend 0.2).  Same net: info.

**Related.**  Via tenting is not supported ("vias get mask openings").
A `via_tented` rule/option (no mask opening on vias, optionally only for
vias under a size) would remove most via dams and the exposed rings.

### B2. Via in or against an SMD pad (`via-in-pad`, warning)

**Defect.**  Plane-escape vias were placed 1.0 mm from the pad centre
regardless of pad length, so on long pads (SOIC, SMA, electrolytic, the
jack) the untented via sat inside the pasted pad: paste wicks down the
hole.  Also a via 0.15 mm from a pad's edge leaves a merged mask opening.

**Detect.**  A via whose annular ring overlaps an SMD pad's mask
opening, or whose opening is within `mask_dam_min` of it.  (The grit
build now escapes by the pad's half-extent plus the via radius plus
0.2 mm, and staggers neighbouring escapes.)

## C. Same-net copper

### C1. Same-net sliver gap (`copper-gap`, warning at 0.25)

**Defect.**  A pad and a same-net via 0.075 mm apart (seq1 pin 19 and
the NEL anchor via) on three layers: a sliver the etch may bridge or
leave as an acid trap.  JLCPCB's rule is 0.25 mm same-net spacing.
`copper-gap` (0.09) did not fire.

**Detect.**  For copper of one net that does not overlap: gap below
`same_net_gap` (0.25 default) is a warning.  Apply to pad-via, pad-trace,
via-trace, trace-trace (non-touching).

## D. Silkscreen

### D1. Closed polylines filled by the gerber writer (writer bug, plus `silk-erased` check)

**Defect.**  A closed silk polyline is emitted as a filled polygon and
then an interior clear (`%LPC*%`), and every dark object drawn earlier
inside the box is erased: the oscillator's refdes vanished from the
gerber while the tool's own render still showed it.  (The grit
footprints now draw boxes as four separate lines.)

**Fix.**  Stroke polylines as lines, or emit a ring as a polygon with a
hole, or emit all clears before any dark object.  Then a check
`silk-erased`: any silk element wholly inside a later clear region.

### D2. Silk over mask openings (`silk-over-opening`, warning)

**Defect.**  The writer clips silk 0.1 mm clear of every mask opening,
silently.  Refdes over their own pads (q0, X1 over a neighbour's pad),
a header pin label 1.5 mm from a 1.7 mm pad, a legend over escape vias,
LED bit labels over LED outlines: all clipped to fragments.  KiCad
reports each as `silk_over_copper`/`silk_overlap`.

**Detect.**  Any silk stroke or text glyph intersecting a mask opening
(pad, via, hole, grown by expansion) or another silk element: warning
with the two items, before the writer clips.

### D3. Silk outside the outline (`silk-outside-outline`, warning)

**Defect.**  The DB9 footprint's face line sat 2.5 mm off the board.
Nothing flagged it; the fab clips it.

**Detect.**  Silk elements (footprint and free text) outside the outline
minus a silk edge clearance.

### D4. Text height and stroke against the fab (`silk-text-height`, keep, but per fab)

Present as info; JLCPCB wants ≥ 1.0 mm height and ≥ 0.15 mm stroke.
Consider tying it to the rule set with the fab's numbers, as an error for
text that will be illegible, not an info.

## E. Fabrication outputs

### E1. Drill file layer span

`TF.FileFunction,Plated,1,2,PTH` and `NonPlated,1,2,NPTH` on a 4-layer
board should say `1,4` (the outer layers).  A CAM honouring X2 would
read blind holes.

### E2. Placement list centroid (`pnp`)

Mid X/Y is the footprint origin; for footprints whose origin is pin 1
(the pin headers, the DB9 with its mounting holes) it is up to 9 mm from
the part's centre.  JLCPCB places at Mid X/Y.  Write the centroid of the
pad field (excluding NPTH and mounting holes), or the footprint's
declared centre if it has one.  (The grit build post-processes the CSV.)

### E3. Waivers that match nothing

`drc waive pth-annular-ring jpwr0 j1` matched zero findings because the
non-plated pads are written as copper discs equal to the hole and the
rule never fires on them.  Warn when a waiver matches no finding, so a
stale waiver is visible.  Also: NPTH pads should carry no copper
(or a real ring), not a disc equal to the drill.

## F. Semantics to decide, not defects

### F1. Net-class clearance and pads

`pcb check` applied a class clearance (power, 0.25) to the class's
traces only: a GND pad 0.225 mm from a signal trace passed.  KiCad
applies the larger of the two items' class clearances to everything,
pads included, so the exported project raised 94 errors until the class
was taken off GND and VCC.  Pick one semantics, document it, and make
the export honour it (export the class clearance only if it is meant for
pads, otherwise export the width alone).

### F2. Single-layer vias (`via-single-layer`, info)

Vias with copper on one layer only (anchor vias used as router pins,
test points).  KiCad warns; an info here would list them for review.

### F3. Degenerate segments (`trace-degenerate`, info; and drop on export)

A zero-length segment (a tap vertex) survived into the KiCad export as a
0.0 mm track.  Drop zero-length segments on export; optionally report
them.

## G. Not a check, but from the same audit

### G1. `pcb netlist compare FILE`

The board generator's own audit compared pin counts per net; the exact
per-net pin-set comparison (after the library pin-name mapping) had to
be scripted.  A command that reads an external netlist and reports every
pin on a different net, every missing net, and every board-only net
would make this routine.

## Priority

Must: A1 (with the connectivity consistency), B1, A2, D1.
Should: B2, C1, D2, E1, E2, F1.
Nice: D3, D4, E3, F2, F3, G1.
