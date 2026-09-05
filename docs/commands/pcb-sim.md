# pcb-sim(1)

## NAME

pcb sim — define, run and inspect SPICE simulation studies with ngspice.

## SYNOPSIS

```
pcb sim add NAME ANALYSIS... [--probe V]... [-P REF.param=value]... [-x LINE]...
                             [-o key=value]... [--temperature C] [--tnom C] [--description TEXT]
pcb sim run [NAME] [--force] [--temperature C]
pcb sim list
pcb sim show NAME
pcb sim remove NAME
pcb sim netlist [NAME]
```

## DESCRIPTION

A simulation is a named, re-runnable study stored in `pcb.json` — an
analysis plus probes, parameter overrides and options. `pcb sim run`
generates a SPICE netlist from the board's netlist and component models,
runs it in ngspice (loaded as a shared library, in-process), and writes
results under `build/sim/<NAME>/`. A blake3 hash of the generated netlist
is stored with the results, so `pcb sim list` can tell whether a study is
`up to date` or `stale` after any change to parts, connections, parameters
or the study itself.

## ANALYSES (`pcb sim add NAME …`)

| words | SPICE | meaning |
| --- | --- | --- |
| `op` | `.op` | DC operating point |
| `tran STEP STOP [START]` | `.tran` | transient, e.g. `tran 10u 6m` |
| `dc SOURCE START STOP STEP` | `.dc` | sweep a source, e.g. `dc V1 0 12 0.1` |
| `ac dec\|oct\|lin POINTS FSTART FSTOP` | `.ac` | small-signal sweep, e.g. `ac dec 20 1 1meg` |
| `temp START STOP STEP` | `.dc temp` | operating point swept over temperature in °C, e.g. `temp -40 125 5` |

Values are SPICE numbers (`10u`, `1meg`, `4.7k`). Negative numbers are
accepted as arguments.

## OPTIONS OF `sim add`

`--probe VECTOR` (repeatable)
: What to save, report and plot. Forms: `v(NET)` (node voltage, net names
  case-insensitive), `i(VNAME)` (current through a voltage source, e.g.
  `i(V1)`, `i(VJ1)` for the battery part `J1`), and device parameters
  `@dev[param]` such as `@d1[id]`, `@q1[ic]`, `@r2[p]` (power), `@m1[gm]`
  (ngspice names; devices are the refdes in lower case). Without probes
  every node voltage and source current is reported.

`-P, --param REF.PARAM=VALUE` (repeatable)
: Override an instance parameter for this study only (e.g.
  `V1.value="DC 5"`, `R2.value=470`). The instance and parameter must
  exist.

`-x, --extra LINE` (repeatable)
: Verbatim SPICE lines added before the analysis: `.ic v(OUT)=0`,
  `.nodeset …`, extra sources, `.model` overrides.

`-o, --option KEY=VALUE` (repeatable)
: `.options` entries (`reltol=1e-4`, `abstol=1e-12`, `method=gear`).

`--temperature C`
: Circuit temperature (`.temp`); ngspice default is 27 °C.

`--tnom C`
: Temperature at which model parameters are nominal (`.options tnom=`).

`--description TEXT`
: Free text kept with the study.

Re-adding a name replaces the study. The netlist is generated immediately
to validate the definition (unknown probes are only detected at run time).

## THE GENERATED NETLIST (`pcb sim netlist [NAME]`)

* Each instance whose component has a `spice` section becomes its
  template expanded with `{ref}`, `{pin:…}` and parameter values
  (study override → instance parameter → component default). Instances
  without a model (connectors, mounting parts) are skipped and listed in a
  comment.
* Nets become nodes of the same name; `GND` (or `0`, `ground`) becomes
  node `0`. There must be a ground net — the error tells you to
  `pcb net rename <net> GND`.
* Unconnected pins that a template references become floating nodes
  `NC_<ref>_<pin>` with a warning comment; ngspice may then report a
  singular matrix.
* `.model`/`.subckt` text from components is emitted once per component
  type; `includes` become absolute `.include` paths (their hashes are
  verified).
* `.save all` plus `.save` for each `@…` probe, then `.end`.

Virtual components (no footprint) such as `vsource`/`isource` are the
normal way to inject stimuli; put them on the same nets as the real
connector pins.

## pcb sim run [NAME] [--force] [--temperature C]

Runs one study (or all). If results exist and the input hash is unchanged
the run is skipped and the previous summary printed; `--force` reruns.
`--temperature` overrides the study's temperature for this run only (never
cached as up to date).

Requires `libngspice`: `brew install ngspice` (macOS), `apt install
libngspice0` (Debian/Ubuntu), or set `$NGSPICE_LIB` to the library path.
The error lists every location tried.

Outputs in `build/sim/<NAME>/`:

* `netlist.cir` — what was simulated;
* `ngspice.log` — simulator output (consult it on convergence problems);
* `results.csv` — first column is the sweep variable (`time`, `frequency`,
  `v-sweep`, `temp-sweep`), then one column per probed vector; every header
  carries the unit, e.g. `time (s),led_k (V),@d1[id] (A)`; values are in
  base SI units (complex AC values as `re+imj`);
* `plot.png` / `plot.svg` — for sweeps: line plots; series whose
  magnitudes differ by more than 20× are split into stacked panels; each
  axis is labelled with its quantity and unit and scaled by an SI prefix
  (`time (ms)`, `@d1[id] (mA)`), legend entries show the unit; AC plots
  show magnitude in dB on a log frequency axis;
* `manifest.json` — input hash, timestamp, vector names, point count and
  the text summary.

The summary printed (and stored) is a table for `op` (`node  value`) and
`min / max / final` per vector for sweeps, with SI prefixes and units.

Units come from ngspice's vector type (voltage, current, time, frequency,
temperature, power, resistance, conductance, capacitance, charge, phase,
dB), with a fallback on naming conventions for device parameters
(`@r2[p]` → W, `@q1[ic]` → A, `@m1[gm]` → S, …). Vectors of unknown kind
are shown without a unit.

Convergence failures are errors, not results: a `.op` whose gmin and
source stepping both fail (typical for oscillators and astables, which
have no DC operating point — use a transient study), or a transient that
hits `timestep too small` / a singular matrix. The error suggests
`.options`, `.ic`/`.nodeset` lines and step changes to try.

## pcb sim list / show / remove

`list` prints every study with its analysis line and `not run` / `up to
date (time)` / `stale — inputs changed since …`. `show` prints the JSON
definition, the status and the last summary. `remove` deletes the
definition (results on disk are left alone).

## TEMPERATURE MODELLING

Use `--temperature` for the whole circuit, `temp` sweeps for behaviour over
ambient, resistor `tc1`/`tc2` parameters in component templates, and
ngspice's built-in temperature dependence of diodes/BJTs/MOSFETs (`XTI`,
`EG`, `XTB`, …) in models. Per-device temperature offsets can be added
through component parameters (`temp=` / `dtemp=` in the template).
Electro-thermal coupling (self-heating) must be modelled explicitly with
behavioural sources in `--extra` or a subcircuit.

## EXAMPLES

```
pcb sim add op op --probe "v(VOUT)" --probe "i(V1)"
pcb sim add startup tran 1u 5m --probe "v(VOUT)" -x ".ic v(VOUT)=0"
pcb sim add loadstep dc I1 0 1 0.01 --probe "v(VOUT)" -P I1.value="DC 0"
pcb sim add hot temp -40 125 5 --probe "@d1[id]" --probe "@q1[ic]" -P V1.value="DC 5"
pcb sim add bode ac dec 20 1 1meg --probe "v(OUT)" -P V1.value="DC 0 AC 1"
pcb sim run                       # all studies
pcb sim run hot --temperature 60  # ad-hoc, not cached
pcb sim netlist hot | less
```

## SEE ALSO

pcb(1), pcb-netlist(1), pcb-index(1), SCHEMA.md (component `spice`, simulation).
