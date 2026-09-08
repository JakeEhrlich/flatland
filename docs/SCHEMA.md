# File formats

All files are JSON. Lengths are millimetres: a bare number (`1.5`) or a string
with a unit (`"1.5mm"`, `"10mil"`, `"0.1in"`, `"100um"`). Points are `[x, y]`.
The board frame is y-up, origin at the bottom-left of the outline by
convention. Every cross-file link is a `{ "url": ..., "blake3": ... }` pair:
the URL is a path relative to the referencing file (or `file:`, `~/`, an
absolute path; `http(s)://` and `s3://` are reserved for mirrored libraries),
and the optional hash pins the referenced file's contents.

## project (`pcb.json`) — one board

```json
{
  "schema": "pcb-project/1",
  "name": "led-driver",
  "component_indexes": [ { "url": "../library/index.json", "blake3": null } ],
  "stackup": { "copper_layers": ["F.Cu", "B.Cu"], "board_thickness": 1.6, "copper_thickness": 0.035 },
  "design_rules": {
    "trace_width": 0.25, "clearance": 0.2, "via_drill": 0.4, "via_diameter": 0.8,
    "pour_clearance": null, "mask_expansion": 0.05, "paste_shrink": 0, "silk_width": 0.15,
    "silk_text_size": 1, "edge_clearance": 0.3, "hole_annular_ring": 0.5,
    "thermal_gap": null, "thermal_spoke_width": 0.3, "pour_min_width": null, "hole_clearance": null,
    "net_classes": { "power": { "trace_width": 0.5, "clearance": 0.3 } }
  },
  "outline": [
    { "line": { "from": [0, 0], "to": [40, 0] } },
    { "arc": { "from": [40, 0], "to": [40, 30], "center": [40, 15], "clockwise": false } }
  ],
  "components": {
    "R1": {
      "component": "resistor-0603",
      "parameters": { "value": "10k" },
      "placement": { "at": [10, 5], "rotation": 90, "side": "top", "locked": false,
                     "label_at": [0, 1.5], "label_size": 0.7, "label_hidden": false },
      "note": "pull-up"
    }
  },
  "nets": { "GND": { "pins": ["R1.2", "U1.GND"], "class": "power" } },
  "holes": [ { "at": [3, 3], "drill": 3.2, "plated": false, "diameter": null, "net": null } ],
  "pours": [ { "name": "gnd", "layer": "B.Cu", "net": "GND", "edges": [ ...outline edges... ], "clearance": null, "priority": 0, "thermal": true } ],
  "traces": [ { "layer": "F.Cu", "net": "GND", "width": 0.25, "points": [[1, 1], [5, 1]], "routed": false } ],
  "vias": [ { "at": [5, 1], "net": "GND", "drill": 0.4, "diameter": 0.8, "layers": [] } ],
  "simulations": { "...": "see simulation below" },
  "drc": { "rulesets": [{ "url": "drc/jlcpcb-fr4-2layer.json", "blake3": "…" }],
           "rules": [], "waivers": [{ "rule": "courtyards", "features": ["J1"], "reason": "…" }] },
  "routing": { "freerouting": "/Applications/freerouting.app/Contents/MacOS/freerouting", "java": null, "max_passes": 20 }
}
```

* `stackup.copper_layers` is listed top to bottom; a single entry makes a
  single-sided board (SMD parts on either side land on that one layer).
* `components.<refdes>.component` is a component name from any index, or
  `index:name` when two indexes define the same name.
* A pin reference is `<refdes>.<pin>`; a pin belongs to at most one net.
* `traces`/`vias` with `"routed": true` were produced by `pcb route` and are
  replaced on the next run; hand-drawn ones are kept and protected.
* `pours` are clipped to the outline (minus `edge_clearance`) and cleared
  around other-net copper by `clearance`; same-net traces/vias connect
  solidly, same-net pads through thermal reliefs unless `thermal` is false
  on the pour or `thermal_relief` is false on the pad.

## index — a shareable component catalogue

```json
{
  "schema": "pcb-component-index/1",
  "name": "basic",
  "description": "Starter parts",
  "components": {
    "resistor-0603": { "url": "components/resistor-0603.json", "blake3": "…", "description": "Generic chip resistor" }
  }
}
```

An index directory is meant to be shared as a zip, git repo/submodule, or a
mirror of a remote bucket. `pcb index register`, `verify` and `update`
maintain the hashes.

## component

```json
{
  "schema": "pcb-component/1",
  "name": "npn-2n2222",
  "description": "2N2222A NPN transistor, TO-92",
  "datasheet": { "url": "https://…/p2n2222a-d.pdf" },
  "footprint": { "url": "../footprints/TO-92_Inline.json", "blake3": "…" },
  "pins": [
    { "name": "E", "pad": "1", "description": "emitter" },
    { "name": "B", "pad": "2" },
    { "name": "C", "pad": "3" }
  ],
  "parameters": { "value": { "default": "10k", "description": "resistance" } },
  "spice": {
    "template": "Q{ref} {pin:C} {pin:B} {pin:E} Q2N2222",
    "model": ".model Q2N2222 NPN(IS=1e-14 BF=200 …)",
    "includes": [ { "url": "../models/foo.lib", "blake3": "…" } ]
  },
  "metadata": { "manufacturer": "onsemi", "mpn": "P2N2222A", "lcsc": "C8545", "assembly": true, "rotation_offset": 0 }
}
```

`metadata` is free-form; the assembly commands (`pcb bom`, `pcb pnp`)
read `lcsc` (orderable part number; may also be given per instance as the
`lcsc` parameter), `mpn`, `manufacturer`, `assembly` (false = do not
place, e.g. solder pads) and `rotation_offset` (degrees added in the
pick-and-place file).

* `footprint` may be omitted for *virtual* parts (voltage sources, probes)
  that exist only in the netlist and simulations.
* `pins[].pad` defaults to the pin name; it may also be a list of pad names
  when one pin has several solder tabs (terminal blocks, tabs) — all of
  them join the pin's net. Pin names may not contain `.`.
* `parameters` without a `default` must be given when the instance is added.
* SPICE `template` placeholders: `{ref}` (instance name; a leading type
  letter in the template that matches the refdes is collapsed, so
  `R{ref}` with `R1` gives `R1`), `{pin:NAME}` (the SPICE node of the net on
  that pin; nets named `GND`/`0` become node `0`), `{param}` (a parameter
  value). Multi-line templates are allowed. `model` is emitted once per
  component type; `includes` become `.include` lines.

## footprint

```json
{
  "schema": "pcb-footprint/1",
  "name": "0603",
  "pads": [
    { "name": "1", "type": "smd", "shape": "round_rect", "at": [-0.875, 0], "size": [1.05, 0.95], "corner_radius": 0.2 },
    { "name": "2", "type": "through_hole", "shape": "circle", "at": [3.81, 0], "size": [1.6, 1.6], "drill": 0.8,
      "drill_length": null, "plated": true, "paste": null, "mask_expansion": null, "rotation": 0,
      "thermal_relief": true }
  ],
  "silkscreen": [
    { "line": { "from": [-0.25, 0.75], "to": [0.25, 0.75], "width": 0.15 } },
    { "arc": { "from": [2.1, 1.7], "to": [2.1, -1.7], "center": [0, 0], "clockwise": false } },
    { "circle": { "center": [0, 0], "diameter": 5.2 } },
    { "polyline": { "points": [[-1, -1], [1, -1], [1, 1], [-1, 1]] } },
    { "polygon": { "points": [[0, 0], [1, 0], [0, 1]] } }
  ],
  "courtyard": [ { "polyline": { "points": [[-1.65, -0.75], [1.65, -0.75], [1.65, 0.75], [-1.65, 0.75]] } } ],
  "label_at": [0, 1.5]
}
```

* Library convention: `silkscreen` carries only polarity and pin-1 marks
  (cathode bars, a pin-1 dot, `+` signs, wire-entry ticks). Body outlines
  are deliberately omitted — they clutter the board and add nothing the
  refdes label does not; `courtyard` is what DRC uses for spacing.
* Footprints are drawn as seen from the top with y up; placing on the bottom
  mirrors them. `type` is `smd` or `through_hole`; shapes are `circle`,
  `rect`, `round_rect`, `oval`. `size` is `[width, height]` (circles use the
  width). Through-hole pads need `drill`; `drill_length` makes a slot.

## simulation — an entry of `project.simulations`

```json
{
  "analysis": { "type": "tran", "step": "10u", "stop": "6m", "start": null },
  "description": "blink",
  "temperature": 85,
  "nominal_temperature": 27,
  "probes": ["v(LED_K)", "i(V1)", "@d1[id]"],
  "parameters": { "V1.value": "DC 5" },
  "extra": [".ic v(OUT)=0"],
  "options": { "reltol": "1e-4" }
}
```

Analyses: `{"type": "op"}`, `tran` (`step`, `stop`, optional `start`),
`dc` (`source`, `start`, `stop`, `step`), `ac` (`sweep` dec|oct|lin,
`points`, `start`, `stop`), `temp_sweep` (`start`, `stop`, `step` in °C —
an operating-point sweep over temperature). `temperature` sets `.temp`,
`nominal_temperature` sets `tnom`. Results land in
`build/sim/<name>/` (`netlist.cir`, `results.csv`, `plot.png`,
`manifest.json` with the input hash used to detect stale results).

## drc — a `pcb-drc/1` rule set

```json
{
  "schema": "pcb-drc/1",
  "name": "jlcpcb-fr4-2layer",
  "description": "where the numbers come from and when they were read",
  "applies": { "layers": [1, 2], "material": "fr4" },
  "rules": [
    { "name": "copper-spacing", "severity": "error", "for": "copper",
      "check": { "clearance": { "to": "copper", "relation": "different_net", "min": 0.10 } } },
    { "name": "trace-width", "for": "copper", "check": { "min_width": 0.10 } },
    { "name": "npth-size", "for": "hole", "where": { "plated": false }, "check": { "min": { "drill": 0.5 } } },
    { "name": "mask-dam", "for": "mask_opening", "severity": "warning", "check": { "min_gap": 0.10 } },
    { "name": "no-vias", "for": "via", "check": { "exists": false } }
  ]
}
```

* `severity` is `error` (default), `warning` or `info`. `for` is one of
  `trace via pad hole pour pour_piece copper mask_opening silk silk_text
  courtyard outline board net pin part`. `where` filters on `layer`, `net`,
  `class`, `plated`, `kind`, `refdes`, `footprint`, `component`.
* `check` is exactly one of `clearance`, `min_width`, `min_gap`, `min`,
  `max`, `inside`, `connected`, `exists`, `placed`, `design_rules`. Distances
  are mm or `"design"` (the project's own rule). See `pcb docs drc`.
* The project's `drc` section holds `rulesets` (file references with
  hashes), `rules` (same shape as above) and `waivers`
  (`{rule, features, reason}`).
