# pcb-python(7)

## NAME

flatland (Python) — the `pcb` engine as a Python package: a project held in
memory, every command a method, nothing written until `save()`.

## SYNOPSIS

```python
from flatland import Pcb, PcbError

pcb = Pcb.new("name", path="boards/name/pcb.json", layers=["F.Cu", "B.Cu"],
              indexes=["library-jlcpcb/index.json"])
pcb = Pcb.load("boards/name/pcb.json")
pcb.run("any", "pcb", "command", "--flag", value)   # -> printed text
pcb.run_json("status")                               # -> parsed --json output
pcb.project                                          # -> dict (assign back to replace)
pcb.save(path=None)
```

## DESCRIPTION

The package wraps the same Rust engine as the CLI (`python/` in the
repository, built with maturin). A `Pcb` owns one project in memory. Each
method runs one `pcb` command in-process against that project: no process
is spawned, the project file is not read or written, and the component
library is loaded once and cached for the session. What the command would
print is returned as a string; `--json` forms come back parsed through
`run_json`. A failing command raises `PcbError` whose message is the CLI's
diagnostic including its `help:` line.

`path` is where `save()` writes the project and what relative library and
rule-set references resolve against; it need not exist until then. Generated
files (renders, gerbers, BOM/CPL, simulation results) go to `build/` next
to that path, as with the CLI.

Typed helpers cover the everyday commands; anything else goes through
`run`. Numbers are formatted for you, points are `(x, y)` tuples, and
keyword arguments map to the command's `--flags`:

| helper | command |
| --- | --- |
| `add(ref, component, at=, rotation=, side=, note=, **params)` | `add` |
| `remove`, `set(ref, **params)` | `remove`, `set` |
| `connect(*pins, net=)`, `disconnect`, `net_class(net, cls)` | `connect`, `disconnect`, `net class` |
| `outline_rect(w, h, radius=)`, `outline_circle(d)` | `outline rect`, `outline circle` |
| `rules(**kv)`, `rule_class(name, width=, clearance=, ...)`, `stackup(*layers, board_thickness=)` | `rules set`, `rules class`, `stackup set` |
| `place(ref, at, rotation=, side=, lock=)`, `label(*refs, at=, absolute=, size=, hide=)` | `place`, `label` |
| `pour(name, layer=, net=, follow_outline=, rect=, size=, priority=, solid=)` | `pour new` |
| `trace(net, points, layer=, width=, chamfer=)`, `via(at, net=)`, `hole(at, drill, ...)`, `trim_traces()` | `trace add`, `via add`, `hole add`, `trace trim` |
| `text(text, at, layer=, size=, rotation=, width=)` | `text add` |
| `status()`, `pads(*refs)`, `check(strict=, rule=)` | `status --json`, `pads --json`, `check --json` |
| `drc_add(set)`, `drc_waive(rule, *features, reason=)`, `drc_rule(name, feature=, check=, where=, severity=)` | `drc add`, `drc waive`, `drc rule` |
| `route(passes=, keep=)`, `visualize(what, output=, width=, crop=)` | `route`, `visualize` |
| `serve(port=7350, open=True)`, `changes()` | `serve`, `changes list --json` (see pcb-serve(1)): saves the board, opens the UI in a background thread, returns the URL; `changes()` is the pending change list as dicts |
| `route_pin(from, to, layer=, width=, via=[(x,y),...], png=, dry_run=)`, `undo()` | `route pin`, `undo` |
| `export_kicad(output=, drc=)` | `export kicad` |
| `gerbers(force=)`, `bom(all=)`, `pnp(all=)` | `gerbers`, `bom`, `pnp` |
| `sim_add(name, analysis, *args, probes=, params=, temperature=)`, `sim_run(*names)` | `sim add`, `sim run` |

`check()` returns the findings as a list of dicts (`rule`, `severity`,
`message`, `at`, `features`, `waived`) whether or not there are errors, so
a script can decide for itself; `run("check")` raises like the CLI exits
non-zero.

## WHY

A board is a program: rows of parts, chains of nets and buses of traces
are loops, and the numbers that matter (pitches, widths, offsets) want to
be variables. A generator script that rebuilds the whole board from
scratch is the natural way to keep a design, and building it in memory
makes that fast enough to run on every edit. `examples/led-panel.py` is
the reference: the same board as `examples/led-panel.sh`, a third the
length, and it builds in a few seconds before simulations.

## BUILDING

```
cd python && python3 -m venv .venv && . .venv/bin/activate
pip install maturin pytest
maturin develop && pytest
```

## SEE ALSO

pcb(1) and every other page here: the helpers take the same options as the
commands they wrap.
