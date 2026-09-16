import json
import os
from pathlib import Path

import pytest

from flatland import Pcb, PcbError

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "library" / "index.json"
JLC = ROOT / "library-jlcpcb" / "index.json"


def test_in_memory_flow(tmp_path):
    pcb = Pcb.new("t", path=tmp_path / "pcb.json", layers=["F.Cu", "B.Cu"], indexes=[LIB])
    assert not (tmp_path / "pcb.json").exists()          # nothing written yet
    pcb.outline_rect(20, 16)
    pcb.add("R1", "resistor-0603", value="10k", at=(5, 5))
    pcb.add("R2", "resistor-0603", value="10k", at=(10, 5))
    pcb.connect("R1.1", "R2.1", net="A")
    pcb.connect("R1.2", "R2.2", net="GND")
    st = pcb.status()
    assert st["nets"] and len(st["components"]) == 2
    pcb.trace("A", [(4.125, 5), (4.125, 6.5), (9.125, 6.5), (9.125, 5)], layer="F.Cu", width=0.3)
    pcb.trace("GND", [(5.875, 5), (5.875, 3.5), (10.875, 3.5), (10.875, 5)], layer="F.Cu", width=0.3)
    findings = pcb.check()
    assert findings == []
    # The dict view is the pcb.json contents.
    proj = pcb.project
    assert proj["name"] == "t" and len(proj["traces"]) == 2
    # Round-trip an edit through the dict.
    proj["description"] = "edited in python"
    pcb.project = proj
    assert pcb.project["description"] == "edited in python"
    # Errors carry the tool's diagnostics.
    with pytest.raises(PcbError) as e:
        pcb.add("R3", "no-such-part")
    assert "no-such-part" in str(e.value)
    # Save writes the file; load reads it back into a fresh session.
    out = pcb.save()
    assert Path(out) == tmp_path / "pcb.json"
    again = Pcb.load(tmp_path / "pcb.json")
    assert again.project == pcb.project
    # Outputs land in build/ next to the project.
    again.visualize("pcb")
    assert (tmp_path / "build" / "pcb.png").exists()


def test_check_findings_and_run(tmp_path):
    pcb = Pcb.new("c", path=tmp_path / "pcb.json", indexes=[LIB])
    pcb.outline_rect(20, 16)
    pcb.add("R1", "resistor-0603", value="1k", at=(5, 5))
    pcb.add("R2", "resistor-0603", value="1k", at=(10, 5))
    pcb.connect("R1.1", "R2.1", net="A")
    pcb.connect("R1.2", "R2.2", net="GND")
    pcb.trace("A", [(4.125, 5), (4.125, 6.5), (9.125, 6.5), (9.125, 5)], layer="F.Cu", width=0.3)
    pcb.trace("GND", [(5.875, 5), (5.875, 6.2), (10.875, 6.2), (10.875, 5)], layer="F.Cu", width=0.3)
    findings = pcb.check()
    assert any(f["rule"] == "copper-clearance" and f["severity"] == "error" for f in findings)
    text = pcb.run("status")
    assert "components: 2" in text
    pcb.drc_add("jlcpcb-fr4-2layer")
    assert any(f["rule"] == "copper-spacing" for f in pcb.check())


def test_jlc_board(tmp_path):
    pcb = Pcb.new("j", path=tmp_path / "pcb.json", indexes=[JLC, LIB])
    pcb.outline_rect(20, 16, radius=1)
    pcb.add("U1", "ne555dr", at=(10, 8))
    pcb.add("R1", "jlcpcb:resistor-0603", value="10k", lcsc="C25804", at=(15, 8), rotation=90)
    pcb.connect("U1.VCC", "R1.1", net="VCC")
    pads = pcb.pads("R1")
    assert len(pads) == 2
    assert "2 part(s)" in pcb.bom()


def test_check_returns_findings_on_errors_and_drops_waived(tmp_path):
    pcb = Pcb.new("w", path=tmp_path / "pcb.json", layers=["F.Cu", "B.Cu"], indexes=[LIB])
    pcb.outline_rect(20, 16)
    pcb.add("R1", "resistor-0603", value="10k", at=(5, 5))
    pcb.add("R2", "resistor-0603", value="10k", at=(10, 5))
    pcb.connect("R1.1", "R2.1", net="A")
    pcb.connect("R1.2", "R2.2", net="GND")
    # Unrouted nets are errors: check() must still return the list, not raise.
    findings = pcb.check()
    assert any(f["rule"] == "nets-routed" and f["severity"] == "error" for f in findings)
    # A waived finding is left out unless asked for.
    pcb.drc_waive("nets-routed", "net GND", reason="not yet")
    names = [f["message"] for f in pcb.check()]
    assert not any("GND" in m for m in names), names
    allf = pcb.check(include_waived=True)
    assert any(f.get("waived") for f in allf)
