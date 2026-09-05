# pcb-visualize(1)

## NAME

pcb visualize — render the board or the netlist to PNG (and SVG).

## SYNOPSIS

```
pcb visualize pcb [-o FILE.png] [--width PX] [--layers L,...] [--from-bottom]
                  [--no-ratsnest] [--no-labels] [--grid]
pcb visualize netlist [-o FILE.png] [--width PX]
```

## DESCRIPTION

Both commands write a PNG (default `build/pcb.png` / `build/netlist.png`,
1600 px wide) and an SVG with the same stem. The PNG is meant to be looked
at by an agent after every few edits; the SVG scales for humans.

## pcb visualize pcb

A top-down view in the board frame (y up), with:

* **rulers** along the bottom and left in millimetres, so positions read
  off the picture map directly to `pcb place`/`trace add` coordinates;
* the **outline** in yellow, board substrate dark green, background dark
  grey; non-plated holes as dark discs;
* **copper** coloured per layer: top red, bottom blue, inner layers green /
  orange / purple / cyan (a single-layer board uses red). Pours are drawn
  translucent inside a dashed boundary; traces solid; SMD pads in their
  layer colour; through-hole pads and plated holes gold with the drill
  shown; vias grey;
* **silkscreen** exactly as it will be fabricated — footprint graphics and
  the reference designator in the stroke font, clipped clear of every
  solder-mask opening — white for top-side parts, pink for bottom-side;
  courtyards dashed grey; pad names inside pads when there is room;
* **labels**: the `value` parameter (or the `--note`) in a readable font
  just below the silkscreen reference designator;
* **airwires** in dashed magenta between copper islands that still need
  connecting (one line per missing connection, minimum spanning tree per
  net);
* a **legend** and title (`name — top view` / `bottom (mirrored)`), and a
  line listing unplaced parts.

Draw order is far side first, so on a top view bottom copper is underneath
top copper.

`--from-bottom`
: Mirror the image left-right and draw bottom copper on top, i.e. the board
  as you would see it flipped over. Ruler x values still show board
  coordinates.

`--layers L,...`
: Draw only these copper layers (pads, traces, pours). Silkscreen, holes,
  outline and airwires are always drawn.

`--no-ratsnest`, `--no-labels`
: Hide airwires / reference designators.

`--grid`
: Light grid lines at a round millimetre step chosen from the board size.

`-o FILE.png`, `--width PX`
: Output path (directories are created; the SVG goes beside it) and pixel
  width (height follows the aspect ratio).

## pcb visualize netlist

A schematic-style drawing. Every instance is a box headed by its
reference designator and `component value/note`, with named pin stubs on
its left and right sides (each pin faces the side its wire leaves from).
Nets are drawn the way a schematic would:

* the ground net (`GND`/`0`) is a ground symbol at every pin;
* nets with four or more pins are shown as net-label flags at every pin
  (like global labels) instead of wires, to avoid spaghetti;
* other nets are blue wires — two-pin nets as a labelled orthogonal wire,
  three-pin nets meeting at a junction dot carrying the net name.

Box positions come from a layered graph layout (left to right); wires
route around boxes. Virtual parts (sources) appear like any other. If the
layout engine fails, boxes are placed on a grid instead.

## EXAMPLES

```
pcb visualize pcb
pcb visualize pcb --from-bottom --layers B.Cu -o build/bottom.png
pcb visualize pcb --grid --width 2400
pcb visualize netlist -o build/schematic.png
```

## SEE ALSO

pcb(1), pcb-place(1), pcb-copper(1).
