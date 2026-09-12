# pcb-board(1)

## NAME

pcb outline, pcb hole, pcb stackup, pcb rules — board shape, mounting
holes, layer stack and design rules.

## SYNOPSIS

```
pcb text add TEXT --at x,y [--layer L] [--size MM] [--rotation DEG] [--width MM]
pcb text list | pcb text remove INDEX
pcb outline rect WIDTH HEIGHT [--at x,y] [--radius R]
pcb outline circle DIAMETER [--center x,y]
pcb outline add line FROM TO
pcb outline add arc FROM TO (--center x,y [--cw] | --through x,y)
pcb outline pop | clear | show

pcb hole add AT --drill D [--plated] [--diameter D2] [--net NET]
pcb hole remove INDEX
pcb hole list

pcb stackup set LAYER[,LAYER...] [--board-thickness T] [--copper-thickness T]
pcb stackup show

pcb rules set KEY=LENGTH...
pcb rules class NAME [--width W] [--clearance C] [--via-drill D] [--via-diameter D]
pcb rules show
```

## OUTLINE

The outline is a closed chain of edges — straight lines and circular arcs
— stored in order. It must be closed (each edge starts where the previous
ends, and the last ends at the first) before anything that needs board
geometry (`check`, `route`, `gerbers`, pours) will work; `pcb outline add`
prints `open`/`closed` after each edge, and errors if the new edge does not
start at the previous edge's end.

`rect WIDTH HEIGHT [--at x,y] [--radius R]`
: Replace the outline with a rectangle whose bottom-left corner is `--at`
  (default `0,0`), optionally with rounded corners of radius `R`.

`circle DIAMETER [--center x,y]`
: Replace the outline with a circle (two 180° arcs).

`add line FROM TO`
: Append a straight edge.

`add arc FROM TO --center x,y [--cw]`
: Append an arc about `--center`, counter-clockwise unless `--cw`. `FROM`
  and `TO` must be equidistant from the centre (within 10 µm).

`add arc FROM TO --through x,y`
: Append the arc through a third point; the centre and direction are
  computed (error if the three points are collinear).

`pop`, `clear`, `show`
: Remove the last edge / all edges / print them with indices.

Arcs are discretised (5 µm chord tolerance) for geometry and outputs; the
gerber outline is emitted as the discretised polyline.

## HOLES

Holes here are drills that belong to the board rather than to a footprint:
mounting holes, plated holes used as test points or connections.

`add AT --drill D`
: Non-plated hole of diameter `D` at `AT`. Cut out of pours (with
  clearance) and exported as a keepout to the router and to the NPTH drill
  file.

`--plated`, `--diameter D2`, `--net NET`
: Plated hole with a copper ring on every copper layer. Ring outer
  diameter is `--diameter` or `drill + 2 × hole_annular_ring`. `--net`
  connects the ring to a net (it then behaves like a through-hole pad:
  joined by pours of that net, offered to the router as a pin, counted in
  connectivity). `--diameter` or `--net` imply `--plated`.

`remove INDEX`, `list`
: Holes are addressed by their index in the list.

## STACKUP

`set LAYER,...`
: Copper layers, top to bottom. Names are free but conventional (`F.Cu`,
  `In1.Cu`, `In2.Cu`, `B.Cu`); they are used in `--layer` arguments,
  gerber file names and DSN. A single layer makes a single-sided board:
  SMD parts on either side land on that layer, vias are refused, and
  gerber file naming uses `Bot` if the layer name starts with `B`, `Top`
  otherwise. Changing the stackup fails if existing traces/pours/vias
  reference a layer that would disappear.

`--board-thickness`, `--copper-thickness`
: Informational for now (written to the gerber README).

## DESIGN RULES

`set KEY=LENGTH...` accepts:

| key | meaning | default |
| --- | --- | --- |
| `trace_width` | default trace width (routing, `trace add`) | 0.25 mm |
| `clearance` | minimum copper-to-copper distance between different nets | 0.2 mm |
| `via_drill`, `via_diameter` | default via | 0.4 / 0.8 mm |
| `pour_clearance` | pour-to-copper distance (defaults to `clearance`) | — |
| `mask_expansion` | solder-mask opening beyond copper | 0.05 mm |
| `paste_shrink` | paste aperture inset from SMD copper | 0 |
| `silk_width` | default silkscreen stroke | 0.15 mm |
| `silk_text_size` | reference designator height on silkscreen | 1 mm |
| `edge_clearance` | pour keep-in distance from the outline | 0.3 mm |
| `hole_annular_ring` | ring width for plated holes without `--diameter` | 0.5 mm |
| `thermal_gap` | pad-to-pour gap of a thermal relief (defaults to `pour_clearance`) | — |
| `thermal_spoke_width` | width of the four relief spokes | 0.3 mm |
| `pour_min_width` | pour fill narrower than this is removed (defaults to `trace_width`) | — |
| `hole_clearance` | pour distance from unplated holes and other-net drills (default: the larger of `pour_clearance` and 0.3 mm) | — |

`class NAME --width W --clearance C --via-drill D --via-diameter D`
: Define or update a net class; omitted fields fall back to the global
  rules. Assign nets with `pcb net class`. Classes drive the router's
  per-class rules, the default width of `pcb trace add`, and via sizes.

`show`
: Print all rules and classes.

## EXAMPLES

```
pcb outline rect 60 40 --radius 3
pcb outline clear
pcb outline add line 0,0 50,0
pcb outline add arc 50,0 50,30 --through 60,15
pcb outline add line 50,30 0,30
pcb outline add line 0,30 0,0
pcb hole add 4,4 --drill 3.2
pcb hole add 30,20 --drill 1 --net GND --diameter 2.4
pcb stackup set F.Cu,In1.Cu,In2.Cu,B.Cu
pcb rules set trace_width=0.3 clearance=0.25 via_drill=0.3 via_diameter=0.6
pcb rules class power --width 1 --clearance 0.3
pcb net class VIN power
```

## TEXT

`pcb text add "54V IN" --at 8,9` strokes a string with the built-in font
(upper case, digits, `- _ + . : / ( ) # *`; lower case is upper-cased),
centred at the point, `silk_text_size` high unless `--size` says otherwise,
turned by `--rotation`. `--layer F.Silkscreen` (default) or `B.Silkscreen`
puts it on the legend, where it is clipped clear of pads like every other
silkscreen and mirrored on the bottom side; a copper layer name etches it
into copper, where it belongs to no net: pours keep their clearance from
it, the router treats it as a keepout, and the design rules check it like
any other copper. `pcb text list` and `pcb text remove INDEX` manage them.

## SEE ALSO

pcb(1), pcb-copper(1), pcb-route(1), pcb-gerbers(1).
