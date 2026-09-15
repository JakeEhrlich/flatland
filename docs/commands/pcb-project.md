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
pcb undo
pcb hash [--short] [--canonical]
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

`pcb check [--strict] [--json] [--rule NAME] [--waived]` runs the design
rules: the built-in basic set (outline, placement, copper inside the
outline, clearance at the project's rule, routed nets, connected pins,
dangling traces, floating fill, overlapping courtyards) plus any fab rule
sets and project rules from `pcb drc`. See pcb-drc(1) for the grammar,
the bundled JLCPCB profiles and waivers. Exit status is non-zero on
errors, or on warnings with `--strict`; `pcb gerbers` refuses to write
while there are errors.

## pcb undo

`pcb undo` restores the project as it was before the last change. Every
command that changes the project first copies the previous `pcb.json` to
`build/undo/` (the last 20 are kept); undo pops the newest. In a Python
session the same stack lives in memory. It is the other half of the
try-look-keep-or-revert loop that `pcb route pin --png crop` is made for.

## pcb pads

`pcb pads [REFDES...] [--json]` lists every pad of the placed parts (or of
the named ones): pad name, pin(s), net, centre and copper extent in board
coordinates. Use it to get exact coordinates for `pcb trace add` and to
confirm which pad a pin lands on after rotation.

## pcb hash

Prints a semantic hash of the board, meant for revision names: blake3
over a canonical text of what the fab makes and the assembler places.
It covers the stackup, the design rules that shape copper, the outline,
holes, every non-virtual part (refdes, component, parameters, placement
and the resolved copper of its pads, so a library footprint change
counts), the nets, and traces, vias, pours and copper-layer text. It
does not cover silkscreen, solder mask or paste rules, labels, notes,
locks, the project name, rule sets, simulations, routing settings, or
the `routed` flag. Traces, vias, holes and pours describe a union, so
their order does not matter, nor the direction a trace was drawn in, nor
which vertex a closed ring starts from.

`--short` prints the first 12 hex digits; `--canonical` prints the text
the hash is taken over, to see exactly what a revision contains or why
two differ. From Python: `pcb.hash(short=True)`.

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
