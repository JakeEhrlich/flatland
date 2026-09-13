# pcb-gerbers(1)

## NAME

pcb gerbers — write fabrication files: RS-274X gerbers, Excellon drills, zip.

## SYNOPSIS

```
pcb gerbers [-o DIR] [--no-zip]
```

## DESCRIPTION

Writes one file per layer into `DIR` (default `build/gerbers`), a
`README.txt` listing them, and `<name>-gerbers.zip` containing everything
(unless `--no-zip`). The board must have a closed outline. Nothing else is
required, but you almost certainly want `pcb check` to pass first; a
warning is printed if any connection is still unrouted, since the files
would describe an incomplete board. Gerbers are a snapshot: re-run after
routing or any other change.

Files (`<name>` is the project name with spaces replaced by `_`; layer
names have `.` replaced by `_`):

| file | X2 function | content |
| --- | --- | --- |
| `<name>-<layer>.gtl` / `.gbl` / `.g2`… | `Copper,L<n>,Top/Bot/Inr` | pours (with clearance holes), traces, pads on that layer, vias, plated hole rings |
| `<name>-F_Mask.gts`, `<name>-B_Mask.gbs` | `Soldermask,Top/Bot` (negative) | openings: every pad on that side grown by `mask_expansion` (through-hole pads on both sides), vias, holes |
| `<name>-F_Paste.gtp`, `<name>-B_Paste.gbp` | `Paste,Top/Bot` | SMD pads with `paste` enabled, shrunk by `paste_shrink`; only written if any exist |
| `<name>-F_Silkscreen.gto`, `<name>-B_Silkscreen.gbo` | `Legend,Top/Bot` | footprint silkscreen lines/polygons and the reference designator in a stroke font, per side, clipped 0.1 mm + half a stroke clear of every mask opening (pads, vias, holes); bottom text is mirrored; only written if non-empty |
| `<name>-Edge_Cuts.gko` | `Profile,NP` | the outline as a 0.1 mm line |
| `<name>-PTH.drl` | Excellon, plated | pad drills, via drills, plated holes; slots as `G85` |
| `<name>-NPTH.drl` | Excellon, non-plated | non-plated pads and holes |

Format details: RS-274X with X2 `TF.*` attributes, `%FSLAX46Y46*%` and
`%MOMM*%` — coordinates are exact nanometres, no rounding. Pads and pours
are regions (`G36/G37`); pour holes are cleared with `%LPC*%` before other
copper is drawn; traces are stroked with circular apertures; vias and round
holes are flashed. Drill files are metric, decimal coordinates, `FMAT,2`,
one tool per diameter.

Single-layer boards: the one copper layer is named `Bot` if its name starts
with `B`, else `Top`; both mask files are still written (through-hole pads
need openings on both sides).

Polygons with holes (pours, clipped silkscreen) are written as single
regions with each hole joined to its outer by a zero-width keyhole, never
as polarity clears: a clear erases whatever was drawn earlier inside it,
which once deleted a reference designator sitting inside a footprint's
silkscreen box. Traces are written as filled regions (the same flat-ended, round-joined
polygon the design rules see) rather than aperture draws; pads, pours and
plated rings are regions too, vias are flashes.

## LIMITATIONS

* Vias get mask openings (not tented).
* Board thickness and copper weight are only written into `README.txt`;
  tell the fab explicitly.

## EXAMPLES

```
pcb check --strict && pcb gerbers
pcb gerbers -o out/rev2 --no-zip
```

## SEE ALSO

pcb(1), pcb-board(1), pcb-copper(1), pcb-project(1).
