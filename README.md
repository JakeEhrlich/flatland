# flatland — PCB design from the command line, for agents

`pcb` is a single binary that builds a printed circuit board out of a JSON
project file: add parts from shareable component libraries, wire pins into
nets, draw the outline, place parts, pour copper, autoroute with
[freerouting](https://github.com/freerouting/freerouting), simulate with
[ngspice](https://ngspice.sourceforge.io/), render PNGs you can look at, and
emit gerbers. Every command validates the whole project before saving and
fails with a specific, actionable message.

```
cargo build --release            # binary at target/release/pcb
brew install ngspice             # for `pcb sim run` (libngspice, loaded at runtime)
# freerouting: install the app/jar; `pcb route` finds /Applications/freerouting.app,
# $FREEROUTING, or --freerouting <path>
```

## Tour

```sh
pcb init my-board --layers F.Cu,B.Cu --index path/to/library/index.json
pcb component list                       # what the indexes offer
pcb component show npn-2n2222            # pins, parameters, footprint, model
pcb add R1 resistor-axial --param value=10k
pcb add Q1 npn-2n2222
pcb add V1 vsource --param value="PULSE(0 5 0 1u 1u 1m 2m)"   # virtual part, sim only
pcb connect R1.2 Q1.B --net BASE
pcb connect Q1.E V1.- --net GND
pcb outline rect 40 30 --radius 2        # or: pcb outline add line 0,0 40,0 / add arc ... --through x,y
pcb hole add 3,3 --drill 2.2
pcb place R1 17,7 --rotation 90
pcb pour new gnd --layer B.Cu --net GND --follow-outline
pcb status                               # parts, nets, what is unrouted
pcb drc add jlcpcb-fr4-2layer            # the fab's limits as a rule set
pcb check                                # basic rules + fab profile + project rules; waivable
pcb visualize pcb                        # build/pcb.png (+ .svg); --from-bottom, --layers, --grid
pcb visualize netlist                    # build/netlist.png
pcb route                                # DSN -> freerouting (headless) -> SES -> traces
pcb trace add --layer F.Cu 1,1 5,1 5,5   # hand routing; `pcb via add`
pcb sim add blink tran 10u 6m --probe "v(BASE)" --probe "@q1[ic]" --temperature 85
pcb sim run blink                        # build/sim/blink/{results.csv,plot.png,manifest.json}
pcb sim list                             # up to date / stale
pcb gerbers                              # build/gerbers/*.g* + Excellon + zip
pcb bom; pcb pnp                         # JLCPCB BOM + pick-and-place CSVs
pcb schema [project|index|component|footprint|simulation]
```

`pcb status --json` gives an agent the whole state in one call.

## Documentation

Manual pages, one per command family, live in [`docs/commands/`](docs/commands/pcb.md)
and are built into the binary: `pcb docs` lists them, `pcb docs route` (or
`pcb docs connect`, `pcb docs sim`, …) prints one. File formats are in
[`docs/SCHEMA.md`](docs/SCHEMA.md) (`pcb schema`). Start with
[pcb(1)](docs/commands/pcb.md).

## Files

* `pcb.json` — the board (see `docs/SCHEMA.md`). Found by walking up from
  the current directory, or given with `--project`/`$PCB_PROJECT`.
* Component indexes — `library/index.json` is a starter set (passives,
  LED, 1N4007, 2N2222, LM7805, headers, virtual sources);
  `library-jlcpcb/index.json` holds LCSC-numbered SMD parts for JLCPCB
  assembly (see `pcb docs assembly`). Make your own with
  `pcb index new <dir>`, write component/footprint JSON (`pcb component
  template` gives a starting point), then `pcb index register <index>
  <component.json>`; `pcb index verify`/`update` maintain the blake3 hashes
  so indexes can be pinned and mirrored.
* `build/` — generated: PNG/SVG renders, `.dsn`/`.ses`, gerbers, sim results.

## Conventions

* Units are millimetres (with `mil`/`in`/`um` accepted anywhere); geometry is
  exact integer nanometres internally, booleans via Clipper2.
* Board frame is y-up. Rotations are counter-clockwise degrees. Placing on
  the bottom mirrors the footprint about its y axis.
* `stackup.copper_layers` lists layers top to bottom; one entry gives a
  single-sided board.
* SPICE: a net named `GND` (or `0`) is ground. Components carry their model
  as a template (`R{ref} {pin:1} {pin:2} {value} tc1={tc1}`); temperature
  studies use `.temp`/`tnom` and ngspice's device temperature models.
* Autorouted traces are marked `routed` and replaced on the next `pcb route`;
  hand-drawn traces are kept and protected.

## Examples

* `sh examples/led-driver.sh` — a single-sided through-hole LED driver:
  three studies (operating point, transient blink, −40…100 °C sweep of LED
  current), routing, renders, gerbers.
* `sh examples/led-panel.sh` — a 60×60 mm single-layer **aluminium** LED
  panel: 32 OSRAM DURIS E 2835 CRI-90 LEDs in two 16-LED strings off a
  54 V Mean Well supply, AL5809 constant-current regulators, PWM-dimmed
  from a Raspberry Pi GPIO; hand-routed planar bus (`pcb pads` + `pcb
  trace add`), JLCPCB BOM/CPL, WAGO push-in terminals.
* `sh examples/blinky-555.sh` — an all-SMD 555 blinker (~0.7 Hz) built
  from JLCPCB basic parts (`library-jlcpcb/`, LCSC numbers verified): two
  layers, autorouted, simulated with a behavioural NE555 model, and emitted
  as the three files JLCPCB's assembly service takes — gerber zip, BOM
  (`pcb bom`) and pick-and-place (`pcb pnp`).

## Layout of the code

| module | role |
| --- | --- |
| `schema/` | serde types for project / index / component / footprint |
| `store` | URL resolution, hashing, loading libraries with good errors |
| `model` | resolved board: placed pads in board frame, nets, pours, connectivity |
| `geom`, `units` | nanometre lengths, arcs, pad shapes, Clipper2 wrappers |
| `viz/` | SVG board and netlist renderers, PNG via resvg |
| `dsn/` | Specctra DSN writer, SES reader, s-expression parser |
| `route` | freerouting driver |
| `gerber/` | RS-274X, Excellon, stroke font, zip |
| `sim/` | SPICE netlist generation, ngspice FFI, plots |
| `cli/` | clap commands |
