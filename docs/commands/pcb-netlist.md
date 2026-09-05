# pcb-netlist(1)

## NAME

pcb add, pcb remove, pcb set, pcb connect, pcb disconnect, pcb net —
build the netlist: component instances and the nets joining their pins.

## SYNOPSIS

```
pcb add REFDES COMPONENT [-P key=value]... [--at x,y] [-r DEG] [-s top|bottom] [--note TEXT]
pcb remove REFDES
pcb set REFDES key=value...
pcb connect PIN [PIN...] [-n NET] [--merge]
pcb disconnect PIN...
pcb net list
pcb net show NET
pcb net rename OLD NEW
pcb net class NET CLASS|-
pcb net remove NET
```

## pcb add REFDES COMPONENT

Creates an instance of `COMPONENT` named `REFDES`. `COMPONENT` is a name
from `pcb component list`, qualified as `index:name` when ambiguous.
`REFDES` must be unique and contain no `.`, `:` or whitespace.

`-P, --param KEY=VALUE` (repeatable)
: Instance parameters. Only keys declared by the component are accepted;
  parameters declared without a default are required. Values are strings
  and are substituted verbatim into the SPICE template (`value=4.7k`,
  `value="PULSE(0 5 0 1u 1u 1m 2m)"`).

`--at x,y`, `-r, --rotation DEG`, `-s, --side top|bottom`
: Place the part immediately (same semantics as `pcb place`). Without
  `--at` the part is added unplaced. Virtual components cannot be placed.

`--note TEXT`
: Free text shown next to the reference designator in `visualize pcb`
  (when absent, the `value` parameter is shown instead).

On success prints the resolved component, footprint and pin names. The new
instance has no connections; use `pcb connect`.

## pcb remove REFDES

Deletes the instance and removes its pins from every net; nets left empty
are deleted. Traces, vias and pours keep their net names (they may now
belong to a net with no pins — `pcb check` will warn).

## pcb set REFDES key=value...

Changes an instance. Keys are the component's parameter names, plus:

`note=TEXT`
: Display note (`note=` clears it).

`component=NAME`
: Swap the instance to another component. Pins are *not* remapped: nets
  referencing pins the new component lacks fail validation, so disconnect
  first if the pin names differ.

## pcb connect PIN [PIN...] [--net NAME] [--merge]

Joins the given pins into one net. Every `PIN` must be `<refdes>.<pin>`
with an existing instance and a pin the component declares.

Net naming and merging rules, in order:

1. If `--net NAME` is given, the result is called `NAME` (created if new,
   joined if it exists).
2. Otherwise, if any of the pins already belongs to a *named* net (not
   `N$…`), that name is kept.
3. Otherwise, if any pin belongs to an auto-named net, that net is reused.
4. Otherwise a new net `N$<n>` is created.

If the pins span two or more differently *named* nets (`VCC` and `VIN`,
say) the command refuses unless `--merge` is given, because that usually
means a typo. With `--merge` all the nets involved are folded into the
result and traces/vias/pours/holes referencing the old names are
relabelled. Auto-named nets merge silently. A single pin with `--net` is
allowed (adds it to that net).

A pin can belong to only one net; connecting it again moves it (merging its
old net per the rules above).

## pcb disconnect PIN...

Removes each pin from its net; a net that becomes empty is deleted.

## pcb net list

`NAME [class]: pin pin …` for every net.

## pcb net show NET

Pins of the net plus counts of traces, vias and pours assigned to it.

## pcb net rename OLD NEW

Renames a net everywhere (pins, traces, vias, pours, holes). Use this to
turn `N$3` into `GND` — which also makes it SPICE ground.

## pcb net class NET CLASS|-

Assigns a net class defined with `pcb rules class` (`-` clears). Classes set
trace width/clearance/via size for routing and hand traces.

## pcb net remove NET

Deletes the net: its pins become unconnected, its traces and vias are
deleted, pours that referenced it become unassigned.

## VALIDATION

Every command above rebuilds the whole board before saving, so mistakes
surface immediately with the object named: unknown instance (with
suggestions), unknown pin (listing the component's pins), unknown
parameter, missing required parameter, duplicate pin in two nets, etc.

## EXAMPLES

```
pcb add U1 regulator-lm7805
pcb add C1 capacitor-electrolytic --param value=100u --note "input cap"
pcb add V1 vsource --param value="DC 12"          # virtual, simulation only
pcb connect V1.+ U1.IN C1.+ --net VIN
pcb connect V1.- U1.GND C1.- --net GND
pcb connect U1.OUT R1.1                           # becomes N$1
pcb net rename N$1 VOUT
pcb set C1 value=220u
pcb connect R1.2 GND_PIN.1 --net GND              # one pin into an existing net
```

## SEE ALSO

pcb(1), pcb-index(1), pcb-place(1), pcb-copper(1), pcb-sim(1).
