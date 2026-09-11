# flatland for Python

The `pcb` engine as a Python package: a project held in memory, every command
available as a method, nothing written until you say so.

```python
from flatland import Pcb

pcb = Pcb.new("blinky", layers=["F.Cu", "B.Cu"],
              indexes=["../library-jlcpcb/index.json", "../library/index.json"],
              path="boards/blinky/pcb.json")      # where save() puts it; libraries resolve from here
pcb.rules(trace_width=0.25, clearance=0.15)
pcb.outline_rect(20, 16, radius=1)
for i, (x, y) in enumerate([(4, 4), (8, 4), (12, 4)], start=1):
    pcb.add(f"R{i}", "jlcpcb:resistor-0603", value="10k", lcsc="C25804", at=(x, y))
pcb.connect("R1.1", "R2.1", "R3.1", net="VCC")
pcb.pour("gnd", layer="B.Cu", net="GND", follow_outline=True)
for f in pcb.check():
    print(f["severity"], f["rule"], f["message"])
pcb.route()
pcb.gerbers()
pcb.save()
```

* `Pcb.new` / `Pcb.load` / `save` — the project lives in memory; `save(path)`
  writes it.
* `pcb.project` — the project as a dict, the same JSON as `pcb.json`; assign
  a modified dict back.
* `pcb.run("any", "pcb", "command", ...)` — every CLI command, in-process,
  returning what it prints; `run_json` for the `--json` forms.
* Typed helpers for the common commands: `add`, `connect`, `place`, `label`,
  `outline_rect`, `rules`, `rule_class`, `trace`, `via`, `pour`, `hole`,
  `check`, `route`, `visualize`, `gerbers`, `bom`, `pnp`, `sim_add`,
  `sim_run`, `drc_add`, `drc_waive`, `drc_rule`, `trim_traces`.
* A failing command raises `PcbError` carrying the CLI's diagnostic and help.

Outputs (renders, gerbers, simulations) go to `build/` next to the project
path, as with the CLI. `pcb docs <topic>` (or `pcb.run("docs", "copper")`) is
the reference for every command's options.

## Building

```
cd python
python3 -m venv .venv && . .venv/bin/activate
pip install maturin pytest
maturin develop            # builds the Rust extension into the venv
pytest
```
