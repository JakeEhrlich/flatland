# pcb-serve(1)

## NAME

pcb serve, pcb changes, pcb diff — look at a board in the browser, record
incremental changes for the agent, and check that a rebuilt board matches
them.

## SYNOPSIS

```
pcb serve [--port PORT] [--open]
pcb changes [list [--json] | apply [-o FILE] [--force] | clear]
pcb diff [A] [B]
```

## WHY

Agents build boards from a generator script (usually through the Python
API) and keep that script as the source of truth. Telling an agent what
to move is slow and imprecise; showing it is not. `pcb serve` opens the
board in a local page where a person moves parts, reshapes or draws
traces, moves vias and pins notes to places. Nothing is written to the
project file. Every edit is recorded in a **change list**
(`build/changes.json`) against the hash of the `pcb.json` it was made on,
and the page shows the result, with live check findings, from an
in-memory copy. The agent reads the list (`pcb changes`), folds it into
its script, rebuilds, and compares (`pcb diff`) until the board matches.

## pcb serve

Serves the project on `http://127.0.0.1:PORT/` (default port 7350;
`--port 0` picks a free one; `--open` launches the browser). Single user,
local only. The page:

* draws every copper layer, pours, pads, vias, holes, silkscreen,
  courtyards, labels, the outline, airwires for unrouted connections and
  markers for check findings (info findings are listed, not drawn);
* pans (drag empty space or right button), zooms (wheel), fits (F),
  finds a part or net (search box, Enter), toggles layers;
* selects a part, pad, trace or via: the side panel shows its facts,
  clicking a net name highlights every piece of copper on that net;
* **moves a part** by dragging (grid-snapped), **rotates** it with R;
* **reshapes a trace**: drag a vertex (snaps to pads, vias and other
  trace ends), or drag the trace body to shift that segment sideways;
  Delete removes it;
* **draws a trace** (T, or the button): click points, 45° and pad
  snapping (Shift for free angles), Enter or double click to finish; the
  net comes from the pad the first point snapped to, the layer and width
  from the toolbar;
* **adds or moves a via** (V, or drag);
* **pins a note** (N): text attached to a point, and to the part, trace
  or net under the click; **sketch note** draws a path first, for "route
  it like this";
* lists the pending changes, undoes (Ctrl/⌘-Z) and redoes
  (Ctrl/⌘-Shift-Z) them, clears them, copies them as commands.

The page polls the project file: when the agent rebuilds `pcb.json` the
board reloads. A change list recorded against an older file is shown as
stale and no longer applied; clear it, or apply it with `--force`.

## pcb changes

`pcb changes` (or `pcb changes list`) prints each pending change with what
it replaced, as a `pcb` command line and as the Python call, plus notes:

```
1. move C3 to 15.5,2.6 rot 90
   was: C3 at 14.5,2.6 rot 0 top
   pcb:    pcb place C3 15.5,2.6 --rotation 90
   python: pcb.place("C3", (15.5, 2.6), rotation=90)
2. reshape trace #4: 9.5625,2.6 -> 9.5625,7.09 -> 7.3,8.635
   was: #4: THR trace on F.Cu w 0.25mm 9.5625,2.6 -> ...
3. note: route THR around here instead (along 9,4 -> 9,6 -> 12,6)
```

Trace and via edits name the object by its index in the base board and
describe it, so the agent can find the call in its script that created
it. `--json` prints the file itself.

`pcb changes apply` applies the list to the project and writes the
result as `pcb-target.json` (`-o` for another name). The project file is
not touched. It refuses when the project file's hash differs from the one
the changes were recorded against (`--force` applies anyway). Notes are
not applied to anything; they are for the agent to read.

`pcb changes clear` deletes the list. Do this once the changes are folded
into the script.

The file (`pcb-changes/1`):

```json
{ "schema": "pcb-changes/1",
  "base_blake3": "69d7…", "base": "pcb.json",
  "changes": [
    { "op": "move_part", "refdes": "C3", "at": [15.5, 2.6], "rotation": 90,
      "was": "C3 at 14.5,2.6 rot 0 top", "when": "2026-09-15T15:33:03Z" },
    { "op": "set_trace_points", "trace": 4, "points": [[9.5625, 2.6], [9.5625, 7.09]] },
    { "op": "add_trace", "layer": "F.Cu", "net": "GND", "width": 0.3, "points": [[3, 3], [3, 6]] },
    { "op": "delete_trace", "trace": 7 },
    { "op": "move_via", "via": 0, "to": [2, 3] },
    { "op": "add_via", "at": [12, 4], "net": "GND" },
    { "op": "delete_via", "via": 1 },
    { "op": "note", "text": "move the cap closer to U1", "refdes": "C1", "at": [10.5, 2.6] },
    { "op": "note", "text": "route THR like this", "path": [[9, 4], [9, 6], [12, 6]] } ] }
```

Changes apply in order; an index refers to the board after the changes
before it.

## pcb diff

`pcb diff` compares two project files as boards. With no arguments it
compares the project against `pcb-target.json`; with one, the project
against that file; with two, those files. The comparison is semantic:
parts by refdes (component, parameters, placement), nets by name (pin
sets, class), traces and vias as multisets independent of order, of the
direction a trace was drawn in and of the `routed` flag; stackup, design
rules, outline, holes, texts, pours, rule sets and simulations as values.
Each difference is one line; the exit status is non-zero when there are
any, so a script can loop on it:

```
part C3: 14.5,2.6 rot 0 top vs 15.5,2.6 rot 90 top
trace only in pcb-target.json: GND trace on F.Cu w 0.3mm: 3,3 -> 3,6 -> 6,6
```

## THE LOOP

```
pcb serve --open                 # person: move, draw, annotate
pcb changes                      # agent: read what changed and why
# ...edit the generator script, rebuild pcb.json...
pcb changes apply && pcb diff    # until "describe the same board"
pcb changes clear
```

From Python, `pcb.serve()` saves the in-memory board and serves it in a
background thread; `pcb.changes()` returns the list.

## SEE ALSO

pcb-python(1), pcb-place(1), pcb-copper(1), pcb-drc(1)
