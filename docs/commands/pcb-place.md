# pcb-place(1)

## NAME

pcb place, pcb unplace, pcb label — position components and their
silkscreen labels on the board.

## SYNOPSIS

```
pcb place REFDES [x,y] [-r DEG] [-s top|bottom] [--lock | --unlock]
pcb unplace REFDES
pcb label REFDES... [--at x,y [--absolute]] [--size MM] [--hide | --show] [--reset]
```

## DESCRIPTION

`pcb place` sets or updates an instance's placement. The position is that of
the footprint origin (normally the part's centre). On the first placement
`x,y` is required; afterwards any subset of position, rotation, side and
lock may be changed, keeping the rest.

`-r, --rotation DEG`
: Counter-clockwise rotation in degrees (default 0 on first placement).

`-s, --side top|bottom`
: Mounting side. Bottom-side parts are mirrored about their own y axis
  *before* rotation, so a pad at footprint `(+0.875, 0)` on a part placed
  at `5,10` with `--side bottom --rotation 90` ends up at `(5, 9.125)`.
  SMD pads move to the copper layer of that side (or the only layer of a
  single-sided board); through-hole pads are on every layer regardless.

`--lock`, `--unlock`
: Locked parts are exported to freerouting with `(lock_type position)`.
  (The router does not move parts anyway; the flag is kept for future
  placement passes.)

`pcb unplace` removes the placement; the part stays in the netlist and
`check` reports it as unplaced.

## LABELS

Every placed part gets its reference designator on the silkscreen at the
footprint's `label_at` (usually just above the body), in the
`silk_text_size` rule's height. Where that collides with a neighbour, a
trace you would rather keep readable, or another label, `pcb label`
overrides it per part:

`--at x,y`
: Label centre as an offset from the footprint origin, in the footprint
  frame — it rotates and mirrors with the part, so the same offset works
  for a row of identically rotated parts. With `--absolute`, `x,y` are
  board coordinates; the offset is computed and stored, so later moves of
  the part carry the label along.

`--size MM`
: Text height for this part only. Stroke width stays `silk_width`; below
  about 0.6 mm the text stops being legible on a fabricated board.

`--hide` / `--show`
: Omit the label from the silkscreen (the render still shows the note).

`--reset`
: Drop all overrides and return to the footprint default.

Several parts may be given at once (`pcb label CC1 CC2 CC3 --at 0,1.5`).
Labels are clipped clear of every mask opening on the way out, so a label
that overlaps a pad is silently cut rather than printed on it — check the
render.

Placement is validated: the component must have a footprint (virtual parts
cannot be placed), and the whole board must still build. Placement does not
check overlap or outline containment — run `pcb check`, and look at
`pcb visualize pcb` (courtyards are drawn dashed).

## EXAMPLES

```
pcb place U1 25,15 --rotation 90
pcb place U1 --side bottom            # keep x,y and rotation, flip
pcb place J1 2,10 --lock
pcb label Q1 --at 19.5,52 --absolute --size 0.7   # move the label into free space
pcb label CC1 CC2 CC3 --at 0,1.5                   # same offset for a row of parts
pcb label J1 --hide
pcb visualize pcb --grid
```

## SEE ALSO

pcb(1), pcb-netlist(1), pcb-visualize(1), pcb-route(1).
