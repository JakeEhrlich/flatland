# Fixes forward from the flint board (mips32, 2026-09-16)

Status (2026-09-16): all six are in the tool; see the notes under each.

flint is a 100 x 100 mm, 4-layer board (ground on In1.Cu, 5 V on In2.Cu
as pours that follow the outline; everything else routed): a PLCC-84
socket, a Raspberry Pi Pico, twelve headers, a barrel-jack supply.  It
is built by `boards/flint/pcb/build.py` in the mips32 repository through
the in-process Python API (`from flatland import Pcb`), the way an agent
should be able to build a board.  These are the places where that did
not work and the build script had to carry a workaround, each with the
fix we want in flatland instead.  Items 1 and 2 are one feature and
block routing a plane board through `Pcb.route()`; 3 and 4 broke the
route or the check; 5 and 6 are quality of life.

## 1. Plane layers: keep the router off them

`dsn::emit` writes `(layer X (type signal) ...)` for every layer, with or
without `--planes`.  freerouting then routes signals across the layers
that carry the outline-following pours: on flint 167 of 267 traces were
put on In1.Cu and In2.Cu, the ground pour came back as 19 islands and
every via and pad on those layers reported a pour overlap.  grit only
routed because its build.py rewrote the DSN between `pcb route
--dsn-only` and its own freerouting launch:

    (layer In1.Cu (type signal)  ->  (layer In1.Cu (type power)

Fix: a layer whose only copper is one outline-following pour is a plane
layer.  Emit it as `(type power)` with the `(plane <net> ...)` entry, and
do not offer that net to the router on any layer.  Make this the default
rather than something behind `--planes`; the 2-layer behaviour of
6b43f16 (a pour's net is routed like any other, the fill cuts away what
it covers) is unchanged, because a pour that shares a layer with traces
is not a plane.  Verify: after `pcb route` on a board with such layers,
no routed trace lies on a plane layer and the plane's net is one island.

**Done.** `Board::plane_layers()` (one netted outline-following pour, no
hand traces or copper text on the layer) drives `dsn::emit`: `(type power)`
plus `(plane ...)`, always; `--planes` now only adds planes for pours that
share a layer with traces. See pcb-route(1) PLANE LAYERS.

## 2. Reaching a plane from an SMD pad

freerouting never drops a via from a pad to a plane, so grit and flint
each carry a hand-written `draw_power_vias`: for every SMD pad on a
plane net, a short stub away from the part's centre, a via, the stub
lengths alternated so neighbouring vias' mask openings do not merge,
a wide stub and a row of vias on the supply-entry pads.  Fix: flatland
draws these itself before writing the DSN, as protected wiring, for
every SMD pad whose net is a plane net (through-hole pads reach the
plane through the fill already).  It knows the pad size, the via size,
clearances and the mask-dam rule, so it can place the via where the
check will pass.  With 1 and 2 a plane board routes with one
`Pcb.route()` call and no hand wiring in the build script.  A `pcb via
fanout --net GND` command with the same code would also let a script do
it explicitly.

**Done.** `pcb route` draws the fanout before writing the DSN
(`route::fanout_plane_pads`: outward stub, alternating distances, checked
against other nets' copper, other vias, mask dams and the edge); `pcb via
fanout` / `Pcb.via_fanout()` do it on demand.

## 3. Every via size in the DSN

Hand-drawn vias of a size other than the rules' via (flint's supply
entry used 0.4 mm drill / 0.8 mm vias next to the 0.2 / 0.5 mm rule)
reference a padstack the DSN's library section never declares:

    WARN  Wiring: via padstack 'Via[0-3]_800:400_um' not found at 'GND'
    ERROR There was a parse error while reading DSN file at '(pcb': DSN structure parsing failed

freerouting 2.2.4 then throws a NullPointerException in
`RouterSettings.applyBoardSpecificOptimizations` and exits 0 without a
session file; `pcb route` reports "freerouting exited with exit status:
0 without writing ...ses".  Fix: emit one padstack per distinct (drill,
diameter) among the project's vias, name them all in the `(via ...)`
list, and reference each wire's via by its own padstack.  flint's
workaround is every via at the project size.

**Done** (e8c11ec).

## 4. Waived findings through the Python API

`Pcb.check()` returns waived findings at their original severity with a
`waived` field carrying the reason, while the CLI hides them.  A script
that counts errors from the Python result sees 46 phantom
`pins-connected` errors (flint's open header positions, all waived) and
stops before routing.  Fix: `check()` leaves waived findings out, the
same as the CLI, or takes `include_waived=False`.  While here: the
error path (`PcbError` carrying the findings JSON after "output before
the error") should not be how a script gets findings when there are
errors; return the list either way and let the caller decide.

**Done.** `check(include_waived=False)` drops waived findings; the list is
returned whether or not there are errors.

## 5. Rotation for reference labels

Per-pin names on headers are free text placed by the build script, and
`pcb text add --at --rotation` is enough for that: a name rotated 90
degrees fits any header pitch, and X/Y is the right interface.  The one
gap is `pcb label`, which takes `--at` (offset from the footprint
origin, rotating with the part) but no rotation of its own: a reference
designator along a standing header reads best rotated.  Fix: `pcb label
--rotation DEG`, counter-clockwise, relative to the part's rotation, and
the same in `Pcb.label()`.

## 6. Version drift between the CLI and the Python module

`pcb hash --short` existed in `target/release/pcb` but not in the
`_native` module built two hours earlier, and the failure inside
`Pcb.run("hash", ...)` was a plain "unrecognized subcommand 'hash'".
Fix: both report the engine's version (git describe or the crate
version plus the commit); `Pcb` warns, or refuses, when the module is
older than the binary next to it, and `maturin develop` is part of the
documented build so the two are rebuilt together.

**Done.** `build.rs` stamps `pcb --version` and `flatland.__version__` with
the crate version and `git describe`; creating a `Pcb` warns when the
`pcb` on PATH differs; pcb-python(1) BUILDING lists both builds.

## Where flint stands

`boards/flint/pcb/build.py` in mips32 is a clean port of the grit flow
onto the Python API: library, placement, hand power vias, silk, pours,
pre-route check (zero errors), `Pcb.route(planes=True)`, check, render,
outputs.  With 1 and 2 done, its `draw_power_vias` and `planes=True`
can go; with 3, its `HEAVY_DRILL, HEAVY_DIA = VIA_DRILL, VIA_DIA` line
can go back to the 0.4 / 0.8 mm supply vias; with 4, the `waived`
filter in its `_check_text` can go.
