# pcb(1)

## NAME

pcb — design a printed circuit board from the command line: parts, nets,
outline, placement, pours, autorouting, simulation, visualisation, gerbers.

## SYNOPSIS

```
pcb [-p PATH] <command> [<subcommand>] [options] [args]
pcb <command> --help
pcb docs [TOPIC]
```

## DESCRIPTION

`pcb` edits and builds one board described by a JSON project file
(`pcb.json`). Each invocation loads the project, applies one change or
produces one output, and exits. Mutating commands validate the *entire*
project (every component resolves, every pin exists, every layer exists,
every outline closes, …) before writing; if validation fails the project
file is left untouched and the command exits non-zero with a diagnostic that
names the offending object and usually suggests a fix (`help:` line,
"did you mean `…`?").

The command set is organised in families, each documented on its own page:

| page | commands |
| --- | --- |
| [pcb-project](pcb-project.md) | `init`, `status`, `check`, `schema`, `docs` |
| [pcb-index](pcb-index.md) | `index …`, `component …` — component libraries |
| [pcb-netlist](pcb-netlist.md) | `add`, `remove`, `set`, `connect`, `disconnect`, `net …` |
| [pcb-board](pcb-board.md) | `outline …`, `hole …`, `stackup …`, `rules …` |
| [pcb-place](pcb-place.md) | `place`, `unplace` |
| [pcb-copper](pcb-copper.md) | `pour …`, `trace …`, `via …` |
| [pcb-visualize](pcb-visualize.md) | `visualize pcb`, `visualize netlist` |
| [pcb-route](pcb-route.md) | `route` — freerouting autorouter |
| [pcb-drc](pcb-drc.md) | `check`, `drc …` — design rules, fab profiles, waivers |
| [pcb-gerbers](pcb-gerbers.md) | `gerbers` — fabrication outputs |
| [pcb-assembly](pcb-assembly.md) | `bom`, `pnp` — BOM and pick-and-place for SMT assembly |
| [pcb-python](pcb-python.md) | the engine from Python: `Pcb.new`, methods per command, in memory |
| [pcb-sim](pcb-sim.md) | `sim …` — ngspice simulation studies |

The file formats are described in [SCHEMA.md](../SCHEMA.md) (`pcb schema`).

## GLOBAL OPTIONS

`-p, --project PATH`
: The project file to operate on. `PATH` may be the `pcb.json` itself or a
  directory containing one. Without it, `$PCB_PROJECT` is used if set;
  otherwise the nearest `pcb.json` in the current directory or any parent
  is used. `pcb init` is the only command that does not need a project.

`-h, --help`, `-V, --version`
: Usual meanings. `--help` works at every level (`pcb pour add --help`).

## PROJECT LAYOUT

```
my-board/
  pcb.json            the board (see SCHEMA.md)
  build/              everything generated; safe to delete
    pcb.png, pcb.svg, netlist.png, netlist.svg
    <name>.dsn, <name>.ses, freerouting.log
    gerbers/           *.gtl *.gbl *.gts … *.drl README.txt <name>-gerbers.zip
    sim/<study>/       netlist.cir ngspice.log results.csv plot.png manifest.json
```

Component libraries live outside the project (a directory with an
`index.json`, `components/*.json`, `footprints/*.json`) and are referenced by
relative URL from `pcb.json`, so a project directory plus its libraries can
be a git repository, with libraries as submodules or mirrored downloads.

## UNITS AND COORDINATES

Every length on the command line and in JSON is millimetres unless a unit
suffix is given: `1.5`, `1.5mm`, `10mil`, `0.1in`, `100um`, `2cm`.
Points are `x,y` on the command line (`12.5,3`, `0.5in,10mm`) and `[x, y]`
in JSON. Angles are degrees, counter-clockwise, e.g. `--rotation 90`
(negative values are accepted: `--rotation -45`).

The board frame is **y-up** with the origin wherever you put it — `pcb
outline rect W H` places the bottom-left corner at `0,0` by default, so
coordinates run 0…W left→right and 0…H bottom→top. Rendered PNGs draw rulers
in this frame. Internally all geometry is exact integer nanometres.

A footprint's own frame has its origin at the component centre, y-up, as
seen from the top. Placing a part on the bottom side mirrors it about its
own y axis before rotating; `pcb status`/`visualize` show the resulting pad
positions.

## NAMES

* **Reference designator** (instance name): `R1`, `U3`, `J_PWR` — no `.`,
  `:` or whitespace.
* **Pin reference**: `<refdes>.<pin>` — `R1.1`, `U1.VCC`, `C1.+`.
* **Net name**: `GND`, `VCC_3V3`, `N$4` — no whitespace, quotes or
  parentheses. Auto-generated nets are `N$<n>`. A net named `GND` (or `0`)
  is SPICE ground.
* **Component name**: as listed by `pcb component list`; qualify with the
  index when two indexes define the same name: `basic:resistor-0603`.
* **Layer name**: whatever `stackup.copper_layers` says, typically `F.Cu`,
  `B.Cu`, `In1.Cu`. The first entry is the top layer, the last the bottom.

## TYPICAL WORKFLOW

```
pcb init board --layers F.Cu,B.Cu --index ../library/index.json
pcb component list; pcb component show <name>
pcb add …; pcb connect …                    # netlist
pcb outline rect 50 30; pcb hole add …      # shape
pcb place …; pcb visualize pcb              # look, iterate
pcb pour new gnd --layer B.Cu --net GND --follow-outline
pcb route; pcb check; pcb visualize pcb
pcb sim add …; pcb sim run …
pcb gerbers
```

`pcb status --json` returns the whole state (components, placements, nets
with their connectivity islands, counts) in one call and is the intended way
for an agent to orient itself.

## EXIT STATUS

`0` on success. `1` on any error (bad arguments, validation failure, missing
tool, failed check); the project file is never modified on a non-zero exit
except by commands that explicitly report partial success (none currently).

## ENVIRONMENT

`PCB_PROJECT`
: Default project path when `--project` is not given.

`FREEROUTING`
: Path to the freerouting jar or app launcher (see pcb-route).

`JAVA_HOME`
: Used to find `java` when freerouting is a jar.

`NGSPICE_LIB`
: Path to `libngspice` (see pcb-sim).

`HOME`
: Expands `~/` in URLs and tool paths.

## FILES

`pcb.json` — project. `index.json` — component index. `*.json` component and
footprint files. `build/` — outputs. See SCHEMA.md.

## SEE ALSO

pcb-project(1), pcb-index(1), pcb-netlist(1), pcb-board(1), pcb-place(1), pcb-drc(1),
pcb-copper(1), pcb-visualize(1), pcb-route(1), pcb-gerbers(1), pcb-assembly(1), pcb-sim(1).
