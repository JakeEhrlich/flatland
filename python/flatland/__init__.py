"""flatland: build a PCB in memory with the same engine as the `pcb` CLI.

    from flatland import Pcb
    pcb = Pcb.new("blinky", layers=["F.Cu", "B.Cu"], indexes=["../library-jlcpcb/index.json"],
                  path="boards/blinky/pcb.json")
    pcb.add("R1", "jlcpcb:resistor-0603", value="10k", lcsc="C25804", at=(15, 8), rotation=90)
    pcb.connect("R1.1", "U1.VCC", net="VCC")
    ...
    pcb.check()          # findings as dicts
    pcb.save()           # only now does pcb.json get written

Every method is a thin, typed front for one `pcb` command; `run()` takes any
command as words. Output the CLI would print is returned as a string, and a
failing command raises `PcbError` with the CLI's diagnostic and help text.
"""

from __future__ import annotations

import json
import os
from typing import Any, Iterable, Sequence

from ._native import PcbError, Session

__all__ = ["Pcb", "PcbError", "Session"]

Point = tuple[float, float]


def _pt(p: Point | str) -> str:
    if isinstance(p, str):
        return p
    x, y = p
    return f"{_num(x)},{_num(y)}"


def _num(v: Any) -> str:
    if isinstance(v, bool):
        raise TypeError("expected a number")
    if isinstance(v, float) and v.is_integer():
        return str(int(v))
    return str(v)


def _flag(name: str, value: Any) -> list[str]:
    """`--name value`, `--name` for True, nothing for None/False."""
    if value is None or value is False:
        return []
    if value is True:
        return [f"--{name}"]
    return [f"--{name}", _num(value) if isinstance(value, (int, float)) else str(value)]


class Pcb:
    """A project held in memory. Nothing is written until `save()`."""

    def __init__(self, session: Session):
        self._s = session

    # ---- lifecycle -----------------------------------------------------------------

    @classmethod
    def new(
        cls,
        name: str,
        *,
        path: str | os.PathLike = "pcb.json",
        layers: Sequence[str] | None = None,
        indexes: Iterable[str | os.PathLike] = (),
    ) -> "Pcb":
        """Create a project (like `pcb init`). `path` is where `save()` will put it and
        what relative library paths resolve against; it need not exist."""
        pcb = cls(Session(os.fspath(path)))
        argv = ["init", name]
        if layers:
            argv += ["--layers", ",".join(layers)]
        for ix in indexes:
            argv += ["--index", os.fspath(ix)]
        pcb.run(*argv)
        return pcb

    @classmethod
    def load(cls, path: str | os.PathLike) -> "Pcb":
        """Open an existing `pcb.json` (or its directory) into memory."""
        return cls(Session.open(os.fspath(path)))

    def save(self, path: str | os.PathLike | None = None) -> str:
        """Write the project; `path` (file or directory) becomes the project's home."""
        return os.fspath(self._s.save(None if path is None else os.fspath(path)))

    @property
    def path(self) -> str:
        return os.fspath(self._s.path)

    @property
    def project(self) -> dict:
        """The project as a plain dict (a copy; assign back with `project = ...`)."""
        return json.loads(self._s.project_json())

    @project.setter
    def project(self, value: dict) -> None:
        self._s.set_project_json(json.dumps(value))

    # ---- the general escape hatch ---------------------------------------------------

    def run(self, *args: Any) -> str:
        """Run any `pcb` command given as words: `pcb.run("place", "R1", "5,5", "--rotation", 90)`."""
        argv = [a if isinstance(a, str) else _num(a) for a in args]
        return self._s.run(argv)

    def run_json(self, *args: Any) -> Any:
        return json.loads(self.run(*args, "--json"))

    # ---- netlist ---------------------------------------------------------------------

    def add(self, refdes: str, component: str, *, at: Point | None = None, rotation: float | None = None,
            side: str | None = None, note: str | None = None, **params: Any) -> str:
        argv = ["add", refdes, component]
        for k, v in params.items():
            argv += ["--param", f"{k}={v}"]
        if at is not None:
            argv += ["--at", _pt(at)]
        argv += _flag("rotation", rotation) + _flag("side", side) + _flag("note", note)
        return self.run(*argv)

    def remove(self, refdes: str) -> str:
        return self.run("remove", refdes)

    def set(self, refdes: str, **params: Any) -> str:
        argv = ["set", refdes]
        for k, v in params.items():
            argv += ["--param", f"{k}={v}"]
        return self.run(*argv)

    def connect(self, *pins: str, net: str | None = None) -> str:
        return self.run("connect", *pins, *_flag("net", net))

    def disconnect(self, *pins: str) -> str:
        return self.run("disconnect", *pins)

    def net_class(self, net: str, cls: str) -> str:
        return self.run("net", "class", net, cls)

    # ---- board -----------------------------------------------------------------------

    def outline_rect(self, width: float, height: float, *, radius: float | None = None) -> str:
        return self.run("outline", "rect", width, height, *_flag("radius", radius))

    def outline_circle(self, diameter: float) -> str:
        return self.run("outline", "circle", diameter)

    def rules(self, **rules: Any) -> str:
        """`pcb rules set key=value ...`"""
        return self.run("rules", "set", *[f"{k}={_num(v)}" for k, v in rules.items()])

    def rule_class(self, name: str, *, width: float | None = None, clearance: float | None = None,
                   via_drill: float | None = None, via_diameter: float | None = None) -> str:
        return self.run("rules", "class", name, *_flag("width", width), *_flag("clearance", clearance),
                        *_flag("via-drill", via_drill), *_flag("via-diameter", via_diameter))

    def stackup(self, *layers: str, board_thickness: float | None = None) -> str:
        return self.run("stackup", "set", *layers, *_flag("board-thickness", board_thickness))

    def hole(self, at: Point, drill: float, *, plated: bool = False, net: str | None = None,
             diameter: float | None = None) -> str:
        return self.run("hole", "add", _pt(at), "--drill", drill, *_flag("plated", plated),
                        *_flag("net", net), *_flag("diameter", diameter))

    # ---- placement -------------------------------------------------------------------

    def place(self, refdes: str, at: Point | None = None, *, rotation: float | None = None,
              side: str | None = None, lock: bool = False) -> str:
        argv = ["place", refdes]
        if at is not None:
            argv.append(_pt(at))
        argv += _flag("rotation", rotation) + _flag("side", side) + _flag("lock", lock)
        return self.run(*argv)

    def label(self, *refdes: str, at: Point | None = None, absolute: bool = False,
              size: float | None = None, hide: bool = False, show: bool = False, reset: bool = False) -> str:
        argv = ["label", *refdes]
        if at is not None:
            argv += ["--at", _pt(at)]
        argv += _flag("absolute", absolute) + _flag("size", size) + _flag("hide", hide) + _flag("show", show) + _flag("reset", reset)
        return self.run(*argv)

    # ---- copper ----------------------------------------------------------------------

    def pour(self, name: str, *, layer: str, net: str | None = None, follow_outline: bool = False,
             rect: Point | None = None, size: Point | None = None, priority: int | None = None,
             solid: bool = False, clearance: float | None = None) -> str:
        argv = ["pour", "new", name, "--layer", layer, *_flag("net", net), *_flag("follow-outline", follow_outline)]
        if rect is not None:
            argv += ["--rect", _pt(rect), "--size", _pt(size if size is not None else (10, 10))]
        argv += _flag("priority", priority) + _flag("solid", solid) + _flag("clearance", clearance)
        return self.run(*argv)

    def trace(self, net: str | None, points: Sequence[Point], *, layer: str, width: float | None = None,
              chamfer: float | None = None) -> str:
        argv = ["trace", "add", "--layer", layer, *_flag("net", net), *_flag("width", width), *_flag("chamfer", chamfer)]
        argv += [_pt(p) for p in points]
        return self.run(*argv)

    def via(self, at: Point, *, net: str | None = None, drill: float | None = None, diameter: float | None = None) -> str:
        return self.run("via", "add", _pt(at), *_flag("net", net), *_flag("drill", drill), *_flag("diameter", diameter))

    def text(self, text: str, at: Point, *, layer: str = "F.Silkscreen", size: float | None = None,
             rotation: float | None = None, width: float | None = None) -> str:
        """Free text on silkscreen (default) or a copper layer, stroked with the built-in font."""
        return self.run("text", "add", text, "--at", _pt(at), "--layer", layer, *_flag("size", size),
                        *_flag("rotation", rotation), *_flag("width", width))

    def trim_traces(self, *, net: str | None = None, dry_run: bool = False) -> str:
        return self.run("trace", "trim", *_flag("net", net), *_flag("dry-run", dry_run))

    # ---- checks, routing, outputs ----------------------------------------------------

    def serve(self, port: int = 7350, *, open: bool = True) -> str:
        """Save the board and open it in the local UI (background thread). Returns the URL.

        Edits made there are recorded in build/changes.json next to the project;
        read them with ``changes()`` or ``pcb changes``."""
        return self._s.serve(port, open)

    def changes(self) -> list[dict]:
        """Pending changes recorded by the UI (see ``pcb changes``), as dicts."""
        try:
            return json.loads(self.run("changes", "list", "--json"))["changes"]
        except Exception:
            return []

    def hash(self, *, short: bool = False) -> str:
        """Semantic hash of the board (see ``pcb hash``): copper, drills, outline and
        parts, independent of silkscreen, mask, notes and the order things were drawn in."""
        return self.run("hash", *(["--short"] if short else [])).strip()

    def status(self) -> dict:
        return self.run_json("status")

    def pads(self, *refdes: str) -> list[dict]:
        return self.run_json("pads", *refdes)

    def check(self, *, strict: bool = False, rule: str | None = None) -> list[dict]:
        """Design-check findings (never raises on findings; look at `severity`)."""
        try:
            return self.run_json("check", *_flag("strict", strict), *_flag("rule", rule))
        except PcbError as e:
            # A failing check still prints its findings; the JSON is the message body.
            text = str(e)
            start = text.find("[")
            if start >= 0:
                try:
                    return json.loads(text[start:text.rfind("]") + 1])
                except ValueError:
                    pass
            raise

    def drc_add(self, ruleset: str | os.PathLike) -> str:
        return self.run("drc", "add", os.fspath(ruleset))

    def drc_waive(self, rule: str, *features: str, reason: str) -> str:
        return self.run("drc", "waive", rule, *features, "--reason", reason)

    def drc_rule(self, name: str, *, feature: str, check: dict, where: dict | None = None,
                 severity: str | None = None, description: str | None = None) -> str:
        argv = ["drc", "rule", name, "--for", feature, "--check", json.dumps(check)]
        if where is not None:
            argv += ["--where", json.dumps(where)]
        argv += _flag("severity", severity) + _flag("description", description)
        return self.run(*argv)

    def route(self, *, passes: int | None = None, keep: bool = False, dsn_only: bool = False,
              keep_redundant: bool = False) -> str:
        return self.run("route", *_flag("passes", passes), *_flag("keep", keep), *_flag("dsn-only", dsn_only),
                        *_flag("keep-redundant", keep_redundant))

    def visualize(self, what: str = "pcb", *, output: str | os.PathLike | None = None, width: int | None = None,
                  crop: tuple[Point, Point] | None = None) -> str:
        argv = ["visualize", what, *_flag("output", None if output is None else os.fspath(output)), *_flag("width", width)]
        if crop is not None:
            argv += ["--crop", _pt(crop[0]), _pt(crop[1])]
        return self.run(*argv)

    def route_pin(self, from_pin: str, to_pin: str, *, layer: str | None = None, width: float | None = None,
                  via: Sequence[Point] = (), via_cost: float | None = None, png: str | None = None,
                  dry_run: bool = False) -> str:
        """One fast trace between two pins of a net (`pcb route pin`); `png="crop"` writes build/pcb.png
        showing the new trace. Raises PcbError with the closest approach and blockers when no route exists."""
        argv = ["route", "pin", from_pin, to_pin, *_flag("layer", layer), *_flag("width", width)]
        for p in via:
            argv += ["--via", _pt(p)]
        argv += _flag("via-cost", via_cost) + _flag("png", png) + _flag("dry-run", dry_run)
        return self.run(*argv)

    def export_kicad(self, *, output: str | os.PathLike | None = None, drc: bool = False) -> str:
        """Write a .kicad_pcb/.kicad_pro pair (and with drc=True run KiCad's DRC on it)."""
        return self.run("export", "kicad", *_flag("output", None if output is None else os.fspath(output)), *_flag("drc", drc))

    def undo(self) -> str:
        """Restore the project as it was before the last change."""
        return self.run("undo")

    def gerbers(self, *, output: str | os.PathLike | None = None, force: bool = False) -> str:
        return self.run("gerbers", *_flag("output", None if output is None else os.fspath(output)), *_flag("force", force))

    def bom(self, *, all: bool = False, output: str | os.PathLike | None = None) -> str:
        return self.run("bom", *_flag("all", all), *_flag("output", None if output is None else os.fspath(output)))

    def pnp(self, *, all: bool = False, output: str | os.PathLike | None = None) -> str:
        return self.run("pnp", *_flag("all", all), *_flag("output", None if output is None else os.fspath(output)))

    def sim_add(self, name: str, analysis: str, *args: Any, probes: Iterable[str] = (),
                params: dict[str, Any] | None = None, temperature: float | None = None) -> str:
        argv = ["sim", "add", name, analysis, *[_num(a) if isinstance(a, (int, float)) else str(a) for a in args]]
        for p in probes:
            argv += ["--probe", p]
        for k, v in (params or {}).items():
            argv += ["-P", f"{k}={v}"]
        argv += _flag("temperature", temperature)
        return self.run(*argv)

    def sim_run(self, *names: str) -> str:
        return self.run("sim", "run", *names)

    def __repr__(self) -> str:
        return f"Pcb({self.path!r})"
