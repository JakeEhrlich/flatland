# pcb-copper(1)

## NAME

pcb pour, pcb trace, pcb via — copper you draw by hand: filled zones,
traces and vias.

## SYNOPSIS

```
pcb pour new NAME --layer L [--net NET] [--follow-outline | --rect x,y --size w,h]
                  [--clearance C] [--priority N] [--solid]
pcb pour add NAME line FROM TO
pcb pour add NAME arc FROM TO (--center x,y [--cw] | --through x,y)
pcb pour remove NAME
pcb pour list                                  # fill pieces, area, pads reached

pcb pads [REFDES...] [--json]
pcb trace add --layer L [--net NET] [--width W] [--chamfer C] POINT POINT [POINT...]
pcb trace clear [--routed-only] [--net NET]
pcb trace list

pcb via add AT [--net NET] [--drill D] [--diameter D]
pcb via remove INDEX
pcb via list
```

## POURS

A pour is a region on one copper layer, optionally assigned to a net, that
is filled with copper wherever nothing else needs the space. The fill is
computed from the current board every time it is loaded:

1. start with the pour's boundary polygon;
2. clip it to the outline shrunk by `edge_clearance`;
3. subtract all copper of *other* nets (and unassigned copper) on that
   layer — pads, traces, vias, plated hole rings — grown by the clearance;
4. subtract non-plated holes and every drill not on the pour's net;
5. subtract the boundaries of higher-priority pours on the same layer;
6. remove every sliver of fill narrower than `pour_min_width` (default:
   `trace_width`) — a trace squeezed between two pads would otherwise leave
   hair-thin threads of copper along both sides; the check is repeated after
   the thermal-relief rings are cut;
7. for a pour with a net, drop every remaining island that touches no
   copper of that net (pads, traces, vias, plated holes) — such fragments
   would be floating copper.

Traces, vias and plated holes on the pour's own net are joined solidly.
Pads on the pour's net get a **thermal relief**: a gap ring of
`thermal_gap` (default: the pour clearance) bridged by four spokes of
`thermal_spoke_width` (default 0.3 mm) aligned with the pad, so the pad
stays solderable without the pour sinking the heat. Spokes are clipped to
the fill, so they never enter another net's clearance. Opt out per pour
with `--solid`, or per pad with `"thermal_relief": false` in the
footprint (wire pads, heat-sink tabs). The fill is what `check`,
connectivity, `visualize`, gerbers and (as a `plane`) the router see.

`new NAME --layer L`
: Create a pour. `--net` assigns it (must exist). Shape: `--follow-outline`
  copies the board outline; `--rect x,y --size w,h` makes a rectangle from
  its bottom-left corner; otherwise the pour starts with no edges and you
  add them with `pour add` (the pour is ignored until its chain closes).
  `--clearance` overrides `pour_clearance` (and the thermal gap);
  `--priority` orders overlapping pours (higher wins the overlap);
  `--solid` floods same-net pads without thermal reliefs.

`add NAME line|arc …`
: Append an edge, with the same syntax and chaining rule as `pcb outline
  add`.

`remove NAME`, `list`.

## FINDING COORDINATES

`pcb pads [REFDES...] [--json]` lists every pad of the placed parts with its
pin, net, centre and copper extent in board coordinates — the numbers to
feed `pcb trace add`. `pcb visualize pcb --grid` shows the same frame.

## TRACES

`add --layer L POINT POINT...`
: A polyline (at least two points) of copper on layer `L`.

`--net NET`
: Net assignment. If omitted, the net is taken from a pad on layer `L`
  under the first or last point; with no such pad the trace has no net
  (and `check` will flag it against everything nearby).

Traces have round joins and **flat ends**: the copper stops exactly at the
last point. End a trace at or inside a pad; a round cap would poke out of
any pad narrower than the trace.

`--chamfer C`
: Replace every right-angle corner of the polyline with a 45° cut `C` long
  (shortened where a leg is too short), so hand-drawn buses match the
  router's 45° wiring. Gentle bends are left alone.

`--width W`
: Defaults to the net class width, else `trace_width`.

Hand-drawn traces are stored with `routed: false`. `pcb route` keeps them,
exports them to freerouting as protected wiring, and only replaces traces it
created itself.

`clear`
: With no flags deletes **all** traces and vias. `--routed-only` deletes
  only autorouted ones; `--net NET` restricts to one net.

`list`
: `#i: layer net width points (routed)`.

## VIAS

`add AT [--net] [--drill] [--diameter]`
: A plated through-hole via spanning all copper layers (refused on a
  single-sided board). Sizes default to `via_drill`/`via_diameter`. Vias
  join same-net copper on every layer for connectivity purposes.

`remove INDEX`, `list`.

## CONNECTIVITY

After drawing, `pcb status` shows whether each net is one copper island.
Traces must actually reach into a pad's copper (end on the pad centre, or
anywhere inside the pad) to count; `pcb visualize pcb` draws magenta
airwires for what is still missing.

## EXAMPLES

```
pcb pour new gnd --layer B.Cu --net GND --follow-outline
pcb pour new vcc_island --layer F.Cu --net VCC --rect 10,10 --size 15,8 --priority 1
pcb trace add --layer F.Cu 5.875,5 9.125,7                   # net inferred from R1.2
pcb trace add --layer F.Cu --net GND --width 0.5 10.875,7 12,10
pcb via add 12,10 --net GND
pcb trace add --layer B.Cu --net GND 12,10 5,9.125
pcb trace clear --routed-only                                # undo the autorouter only
```

## SEE ALSO

pcb(1), pcb-board(1), pcb-route(1), pcb-visualize(1).
