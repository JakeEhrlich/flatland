# pcb-route(1)

## NAME

pcb route — autoroute the board with freerouting.

## SYNOPSIS

```
pcb route [--passes N] [--timeout SECONDS] [--keep]
          [--freerouting PATH] [--java PATH]
pcb route --dsn-only
pcb route --import FILE.ses
```

## DESCRIPTION

`pcb route` performs, in order:

1. Deletes previously autorouted traces and vias (`routed: true`), unless
   `--keep`. Hand-drawn copper is kept.
2. Writes a Specctra design file `build/<name>.dsn` describing layers,
   outline, keepouts (non-plated holes), every placed footprint with its
   pads, nets and net classes with widths / clearances / via, and
   hand-drawn traces and vias as *protected* wiring. A pour's net is an
   ordinary net to the router (see `--planes`). Stops here with
   `--dsn-only`.
3. If every net is already one island, stops ("nothing to route").
4. Runs freerouting in batch mode:
   `freerouting -de build/<name>.dsn -do build/<name>.ses -mp N -dct 0 -da -dl`
   with `JAVA_TOOL_OPTIONS=-Djava.awt.headless=true`, output captured to
   `build/freerouting.log`. No window should appear; it exits by itself.
5. Parses the session file `build/<name>.ses`: wires become traces
   (`routed: true`, with the net and width freerouting used) and vias
   become vias, converted from the session's `resolution`/`unit`.
6. Rebuilds the board, saves, and reports imported counts and which nets
   remain unrouted.

`--import FILE.ses` skips steps 2–4 and imports a session you produced
yourself (for example after routing interactively in freerouting from the
`--dsn-only` file). Existing routed traces are kept in this mode.

## PIN TO PIN

```
pcb route pin FROM TO [--layer L] [--width W] [--via x,y]... [--via-cost MM] [--png full|crop] [--dry-run]
pcb undo
```

`pcb route pin D8.K D9.A` draws one trace between two pins of a net and
stops. It is built for an agent's edit loop: it takes tens of milliseconds,
prints exactly what it drew, and with `--png crop` writes `build/pcb.png`
showing the new trace and its surroundings so the result can be looked at
before the next step. `pcb undo` puts the project back as it was.

The route may start on any copper already joined to FROM (its pad, or a
trace or via touching it) and end on any copper joined to TO, so a bus is
extended rather than duplicated. Copper of other nets, keepouts and the
board edge are avoided at the net's clearance; pours are ignored (they
refill around the trace). On multi-layer boards the route may change layer
through a via, charged `--via-cost` millimetres of trace; `--layer` pins it
to one layer. `--via x,y` (repeatable) forces the route through waypoints,
which is how to say "go round the other side": the router picks the
shortest path, and you pick the topology.

The result is a polyline of 0/45/90° segments, chamfered corners included,
in the net class's width, checked against real geometry before it is
added; the trace is stored as hand-drawn (`pcb route` and `pcb trace
clear --routed-only` leave it alone). When no route exists the error says
on which layer the search got closest, how far from the target, and which
features were in the way there.

Under the hood: an A* search over a grid (half the smaller of width and
clearance per cell) with obstacles inflated by a distance transform, bend
and hugging penalties for tidy results, then the cell path pulled into the
fewest 45°-multiple segments whose corridor is free.

## OPTIONS

`--passes N`
: Maximum optimisation passes (`-mp`), default 20 or `routing.max_passes`
  from `pcb.json`. Small boards finish in seconds; more passes mostly
  shorten traces.

`--timeout SECONDS`
: Kill freerouting after this long (default 600) and fail with a hint.

`--keep`
: Do not delete previously autorouted copper first (freerouting then sees
  it as protected wiring and only routes what is missing).

`--keep-redundant`
: After import every router-drawn segment is removed in turn and kept only
  if its net would fall apart without it. That strips the links freerouting
  draws between the pads of a pour's net, the pieces of protected hand
  wiring it echoes back, and links between the solder tabs of one pin.
  What survives is then cut back to the stretches the fill does not cover:
  a ground trace that crosses a clearance channel into a pocket keeps that
  crossing (extended a trace width into the fill on each side) and loses
  the rest. This flag keeps everything as routed. Hand-drawn traces are
  never touched here; `pcb trace trim` applies the same cut to them.

`--planes`
: Hand every pour with a net to the router as a `plane`, and declare a
  layer whose netted pour covers the outline as a power layer so the
  router keeps signal traces off it. The router then
  treats that net as solid copper across the layer, never draws it, and
  keeps other nets clear of nothing but the pads, so it can wall a pad of
  the plane's net in with other traces; the real fill cannot reach such a
  pad and `pcb check` fails with the net in two islands. Routing the net
  like any other (the default) makes the router reach every pad itself,
  with a trace or a via, and the fill swallows whatever it covers. Planes
  remain useful on a dense multi-layer board where routing the ground net
  costs too much.

`--freerouting PATH`
: A `.jar` (run with Java) or the app's launcher executable. Precedence:
  this flag, `routing.freerouting` in `pcb.json`, `$FREEROUTING`, then
  `/Applications/freerouting.app/Contents/MacOS/freerouting`,
  `~/freerouting/freerouting.jar`, `~/freerouting.jar`,
  `/usr/local/share/freerouting/freerouting.jar`,
  `/opt/freerouting/freerouting.jar`.

`--java PATH`
: Java executable for a jar. Precedence: flag, `routing.java`,
  `$JAVA_HOME/bin/java`, Homebrew `openjdk` locations, `java` on `PATH`.
  Recent freerouting jars need a recent JDK (2.2.x needs Java 25); the
  `.app` bundle carries its own runtime, which is why it is preferred.

## WHAT THE ROUTER SEES

The DSN asks for 45° routing (`snap_angle fortyfive_degree`), so router
wiring runs horizontally, vertically or diagonally; hand-drawn traces can
match with `pcb trace add --chamfer`.

* Layers: all `copper_layers`, in order. A single-layer board is routed on
  its one layer (a via padstack is still declared because freerouting
  requires one, but it cannot be used).
* Rules: `trace_width` and `clearance` globally; per net class width,
  clearance and via; a `default` class for unclassed nets.
* Pads: exact polygons (SMD pads on the top layer, mirrored by the router
  for bottom-side parts; through-hole pads on all layers). Plated holes
  with a net are exported as one-pin parts named `HOLE<n>`.
* Pours are not exported unless `--planes`: the router draws a pour's net
  as traces and vias like any other, and the pour then merges with them
  (the covered stretches are cut away on import). With `--planes` a pour
  with a net is a `plane` on a multi-layer board, which the router treats
  as connecting its own net without checking that the fill can reach each
  pad.
* A pin with several solder tabs is one router pin whose padstack holds
  every tab.
* Keepouts: non-plated holes and unassigned plated holes, grown by
  `clearance`.

## OUTPUT AND STATE

Traces/vias imported from the router carry `routed: true` and are shown
like any other copper. Re-running `pcb route` regenerates them; `pcb trace
clear --routed-only` removes them. Because pours are recomputed from the
current copper, a pour adjusts itself around routed traces automatically.

After routing, run `pcb check` (clearance/short detection is independent of
freerouting's own rules) and `pcb visualize pcb`.

## TROUBLESHOOTING

* *freerouting was not found* — install it and pass `--freerouting`, or
  set `$FREEROUTING`, or add `"routing": {"freerouting": "…"}` to
  `pcb.json`.
* *needs a newer Java* — the jar's class version exceeds your JDK; use the
  app bundle or `brew install openjdk` and `--java`.
* *exited without writing …ses* — see the last log lines in the error and
  `build/freerouting.log`; common causes are an unclosed outline (caught
  earlier) or parts placed outside the outline.
* *still unrouted* — the router gave up on some connections; add layers,
  move parts, widen the board, relax clearances, or draw the remaining
  traces with `pcb trace add`.
* Do not launch freerouting by hand without `-de/-do`: it opens a GUI.

## EXAMPLES

```
pcb route
pcb route --passes 50 --timeout 1200
pcb route --dsn-only            # then route interactively, save build/<name>.ses
pcb route --import build/<name>.ses
```

## SEE ALSO

pcb(1), pcb-copper(1), pcb-board(1), pcb-project(1).
