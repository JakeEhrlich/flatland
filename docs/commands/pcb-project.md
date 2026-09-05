# pcb-project(1)

## NAME

pcb init, pcb status, pcb check, pcb pads, pcb schema, pcb docs — create a
project, inspect its state, verify the design, read the documentation.

## SYNOPSIS

```
pcb init NAME [-d DIR] [-l LAYERS] [-i INDEX]... [--force]
pcb status [--json]
pcb check [--strict]
pcb pads [REFDES...] [--json]
pcb schema [project|index|component|footprint|simulation]
pcb docs [TOPIC]
```

## pcb init

Creates `DIR/pcb.json` (default `./pcb.json`) describing an empty board
called `NAME`. Refuses to overwrite an existing file unless `--force`.

`-d, --dir DIR`
: Directory to create (made if missing).

`-l, --layers LAYERS`
: Comma-separated copper layers, top to bottom. Default `F.Cu,B.Cu`.
  A single name (`-l B.Cu`) makes a single-sided board: parts on either
  side put their SMD pads on that one layer, vias are refused, and
  freerouting routes on one layer.

`-i, --index PATH` (repeatable)
: Component index file (or its directory) to use. Stored in `pcb.json` as a
  URL relative to the project file. Indexes can also be added later with
  `pcb index add`.

Defaults written into the new file: 1.6 mm board, 35 µm copper, design rules
`trace_width 0.25`, `clearance 0.2`, `via_drill 0.4`, `via_diameter 0.8`,
`mask_expansion 0.05`, `paste_shrink 0`, `silk_width 0.15`,
`silk_text_size 1`, `edge_clearance 0.3`, `hole_annular_ring 0.5` (all mm).
Change them with `pcb rules set`.

## pcb status

Prints a summary: project name and path, layers, outline size and area,
every component instance (component name, parameters, placement or
`UNPLACED`/`virtual`), every net with its pins and routing state, counts of
traces/vias/pours/holes, simulations, and a list of unplaced parts.

A net's routing state is derived from actual copper: the pads of the net,
plus traces, vias, plated holes and pours assigned to it, are unioned per
layer; pads that end up in the same copper island (joined across layers by
vias) are connected. `routed` means one island; `N islands` means N−1
connections are still missing. Virtual parts (no footprint) are ignored, so
a net whose only other member is a virtual source counts as routed.

`--json`
: Machine-readable form:

```json
{
  "name": "...", "path": "...", "layers": ["B.Cu"],
  "outline": {"width_mm": 40, "height_mm": 30, "min": [0,0], "max": [40,30]} | null,
  "components": [{"refdes": "R1", "component": "resistor-axial", "placed": true, "virtual": false,
                  "placement": {"at": [17,7], "rotation": 0, "side": "top"} | null,
                  "parameters": {"value": "10k"}}],
  "nets": [{"name": "GND", "pins": 5, "islands": [["J1.-","Q1.E"], ["C1.-"]], "unrouted": 1}],
  "unrouted_connections": 4,
  "traces": 0, "vias": 0, "pours": 1, "holes": 2,
  "simulations": ["op", "blink"]
}
```

## pcb check

Runs the design checks and prints every finding as `error:` or `warning:`
lines followed by a count. Exit status is 1 if there is any error, or, with
`--strict`, any warning.

Errors:

* no board outline;
* a non-virtual component is not placed;
* a pad centre or hole centre lies outside the outline;
* on any copper layer, two copper features on different nets (or with no
  net) are closer than `design_rules.clearance`; features that actually
  overlap are reported as `overlap (short circuit)`. Features are pads,
  traces, vias, plated hole rings and pours (a pour counts as one feature
  including its clearance holes). Features that sit exactly at the
  clearance distance — pour edges, for example — are accepted (2 µm
  tolerance).

Warnings:

* a pin is not on any net;
* a net has fewer than two pins;
* a net is not fully routed (islands are listed, e.g.
  `2 islands (R2.2 | D1.A)`).

Not checked (yet): trace-to-outline distance beyond what pours already
respect, annular ring sizes, silkscreen over pads, courtyard overlap.

## pcb pads

`pcb pads [REFDES...] [--json]` lists every pad of the placed parts (or of
the named ones): pad name, pin(s), net, centre and copper extent in board
coordinates. Use it to get exact coordinates for `pcb trace add` and to
confirm which pad a pin lands on after rotation.

## pcb schema

Prints `docs/SCHEMA.md`, the reference for the JSON files, or just the
section for one file kind.

## pcb docs

Prints these manual pages. Without a topic, lists them; with a topic (a page
name such as `route`, `sim`, `netlist`, or a command name such as `connect`
or `pour`), prints the matching page.

## EXAMPLES

```
pcb init blinky --layers B.Cu --index ~/libs/basic/index.json
pcb status --json | jq '.nets[] | select(.unrouted > 0) | .name'
pcb check --strict && pcb gerbers
```

## SEE ALSO

pcb(1), pcb-index(1), pcb-netlist(1).
