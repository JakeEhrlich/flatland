# pcb-drc(1)

## NAME

pcb drc, pcb check — design rules: what the fab can make and what the
design must satisfy, as importable rule sets, project rules and waivers.

## SYNOPSIS

```
pcb check [--strict] [--json] [--rule NAME] [--waived] [--info]
pcb drc profiles
pcb drc add NAME|PATH
pcb drc remove NAME|PATH
pcb drc list
pcb drc explain RULE
pcb drc rule NAME --for FEATURE --check JSON [--where JSON] [--severity S] [--description TEXT]
pcb drc unrule NAME
pcb drc waive RULE [FEATURE...] --reason TEXT
pcb drc unwaive RULE
pcb drc new PATH [--name NAME]
```

## DESCRIPTION

`pcb check` runs every active rule against the board and prints the
findings, most severe first (at most six per rule unless `--rule NAME`;
info-level ones only with `--info`), then a summary line `N error(s), M
warning(s)`. It exits non-zero on errors (or on warnings with `--strict`).
`pcb gerbers` runs the same check and refuses to write fab files while
there are errors (`--force` overrides).

Three sources of rules are active at once:

1. **basic**, built into `pcb` and always on: the board is complete and
   self-consistent (outline present, parts placed, copper inside the
   outline, different nets keep the project's `clearance`, nets routed,
   pins connected, no dangling traces, no floating fill, part bodies do
   not overlap).
2. **Rule sets** referenced from `pcb.json` by URL and blake3 hash, exactly
   like component indexes: one file per fabrication process. `pcb drc
   profiles` lists the ones bundled with `pcb`; `pcb drc add NAME` copies
   one into `drc/` in the project and references it, `pcb drc add PATH`
   references any file. A set can say which layer counts it is for; a
   mismatch is reported as a warning.
3. **Project rules** added with `pcb drc rule`, for constraints specific to
   this design (a wider ground return, a keep-out).

Findings can be **waived**: `pcb drc waive RULE FEATURE... --reason` silences
that rule for the named features (feature ids as printed, reference
designators, or net names; none means all) and records why. Waived findings
are counted separately and shown with `--waived`. Waive what you have judged
acceptable; loosen a project rule instead if the rule is wrong for the design.

## THE GRAMMAR

A rule set is a `pcb-drc/1` JSON file (`pcb schema drc`). Each rule
**selects** a feature kind, optionally **filters** it, and applies one
**check** at a **severity**:

```json
{ "name": "hole-to-copper", "severity": "error",
  "for": "hole", "where": { "plated": false },
  "check": { "clearance": { "to": "copper", "relation": "any", "min": 0.3 } },
  "description": "Unplated holes keep 0.3 mm from every copper feature." }
```

**Features** (`for`, and `to` in clearance / `inside`):

| feature | what it is | attributes for `min`/`max` |
| --- | --- | --- |
| `trace` | a drawn or routed trace | `width` |
| `via` | a via's copper | `drill`, `diameter`, `annular_ring` |
| `pad` | a footprint pad or a plated hole's ring | `width`, `height`, `drill`, `annular_ring` |
| `hole` | any drilled hole: pad drills, via drills, free holes | `drill` |
| `pour` | a whole filled pour | `area` |
| `pour_piece` | one connected piece of a pour | `area` |
| `copper` | trace, via, pad and pour together | as each |
| `mask_opening` | solder-mask opening of a pad, via or ring, per side | |
| `silk` | a footprint silkscreen stroke | `width` |
| `silk_text` | a reference designator | `height`, `width` (stroke) |
| `courtyard` | a part's courtyard, per side | |
| `outline` | the board outline | |
| `board` | the board as a whole | `width`, `height`, `layers` |
| `net` | a net | `pins` |
| `pin` | a component pin | |
| `part` | a component instance | `placed`, `virtual` |

**Filters** (`where`, every given field must match): `layer` (a copper
layer, or `top`/`bottom` for silk, mask and courtyards), `net`, `class`
(net class), `plated`, `kind` (`smd` or `through_hole`), `refdes`,
`footprint`, `component`.

**Checks** (exactly one per rule):

`clearance: { to, relation, min }`
: Grown by `min`, the feature must not touch any `to` feature on a shared
  layer. `relation` is `different_net` (default; copper without a net is
  different from everything), `any`, `different_part` (courtyards, mask
  openings) or `same_net`. `to: outline` measures to the board edge.
  `min: 0` means "must not overlap".

`min_width: W`
: Nothing narrower than `W`. Traces, pads and silk use their width;
  pours and other geometry are tested by a morphological opening (erode by
  `W/2`, dilate back): whatever vanished was too thin. Reported per sliver
  with its location and area; corner chips from arc approximation are
  ignored.

`min_gap: G`
: No gap narrower than `G` inside a feature or between same-net copper on
  a layer (acid traps), or between mask openings on a side (mask dams).
  The dual operation: dilate by `G/2`, erode back, and whatever got filled
  in was a too-narrow gap.

`min: { attr: value }` / `max: { attr: value }`
: Attribute bounds, millimetres or counts (see the table).

`inside: FEATURE`
: The feature must lie entirely within the union of that feature kind
  (`copper` inside `outline`).

`connected: true`
: Nets: one island. Pins: on a net. Pour pieces: touching copper of their
  net. Traces: both ends on copper of their net. Pads: touched by copper of
  their net when the net has other pins.

`exists: true|false`
: Whether the feature kind must exist (an outline) or must not (vias on an
  aluminium board).

`placed: true`
: Non-virtual parts have a placement.

`design_rules: { rule: value }`
: The project's own design rules (`pcb rules`) are at least these values —
  catches a project set up looser than the process before any geometry is.

**Distances** are millimetres, or the string `"design"` for the project's
own value (`clearance`, `edge_clearance` for `to: outline`, `trace_width`
for `min_width`).

## BUNDLED PROFILES

`jlcpcb-fr4-2layer`, `jlcpcb-fr4-4layer` and `jlcpcb-aluminium-1layer`
carry JLCPCB's published limits (trace/spacing, via and hole sizes,
annular rings, hole-to-hole and hole-to-copper, copper-to-edge, mask dams,
silkscreen, board size), read from jlcpcb.com/capabilities on the date
in each file's description. Treat them as a starting point: capabilities
change, and 2 oz copper or special finishes have their own numbers. `pcb
drc new` writes a template to derive a profile for another fab.

## OUTPUT

Each finding is `severity: rule: message`; messages name the features
involved (`R1.2`, `trace VIN at 4,6.5`, `pour gnd piece 2`) and, where it
helps, a location and a size. `--json` gives an array of
`{rule, severity, message, at, features, waived}`. `pcb drc explain RULE`
prints a rule's description, selector and check.

## EXAMPLES

```
pcb drc add jlcpcb-fr4-2layer
pcb check
pcb drc explain mask-dam
pcb drc rule gnd-return --for pour --where '{"net":"GND"}' --check '{"min_width": 1.0}' --severity warning
pcb drc waive courtyards J1 --reason "connector housing overhangs the cap; checked with the parts"
pcb check --json | jq '.[] | select(.severity=="error")'
```

## SEE ALSO

pcb(1), pcb-board(1), pcb-copper(1), pcb-gerbers(1), pcb-index(1).
