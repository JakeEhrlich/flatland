#!/usr/bin/env python3
"""60 x 60 mm single-layer ALUMINIUM LED panel for a Mean Well HLG-240H-54A.

The same board as examples/led-panel.sh, built through the Python bindings: the
project is assembled in memory and written once at the end. Run from the repo
root with the python/ venv active (see python/README.md):

    python examples/led-panel.py            # writes examples/led-panel/pcb.json and build/
    python examples/led-panel.py --no-sim   # skip the ngspice studies (they take minutes)

32 x OSRAM DURIS E 2835 CRI-90 LEDs in two strings of 16 (~47 V each), each string
regulated to 180 mA by three parallel AL5809-60 two-terminal constant-current
regulators, PWM-dimmed by one low-side MOSFET driven straight from a 3.3 V
Raspberry Pi GPIO (high = on, 100-200 Hz). Peak ~17 W of LED power; average set by
duty cycle. WAGO 2060 push-in terminals for the 54 V supply (left) and the Pi (right).
"""

import random
import shutil
import sys
from pathlib import Path

from flatland import Pcb

ROOT = Path(__file__).resolve().parents[1]
DIR = ROOT / "examples" / "led-panel"
SIM = "--no-sim" not in sys.argv

shutil.rmtree(DIR, ignore_errors=True)
DIR.mkdir(parents=True)

# ---- Project: one copper layer, ordered as an aluminium-core board at JLCPCB.
pcb = Pcb.new("led-panel", path=DIR / "pcb.json", layers=["F.Cu"],
              indexes=[ROOT / "library-jlcpcb" / "index.json", ROOT / "library" / "index.json"])
pcb.rules(trace_width=0.4, clearance=0.3, pour_clearance=0.35, edge_clearance=0.5,
          silk_text_size=0.8, thermal_spoke_width=0.4)
pcb.drc_add("jlcpcb-aluminium-1layer")            # metal-core process limits; check() runs them
pcb.drc_waive("design-rules-silk", reason="0.8 mm reference designators are a deliberate choice "
              "on this small board; JLCPCB's 1 mm figure is legibility guidance")
# LED-to-LED links as wide as the LED pads; the supply/return buses 1.2 mm. Electrically
# 0.4 mm would do at 180 mA -- the width is for looks and a little extra copper for heat.
pcb.rule_class("led", width=2.0, clearance=0.3)
pcb.rule_class("bus", width=1.2, clearance=0.3)
pcb.stackup("F.Cu", board_thickness=1.6)

# ---- Parts
LEDS = [f"D{i}" for i in range(1, 33)]
for d in LEDS:
    pcb.add(d, "led-duris-e-2835-cri90")
pcb.add("J1", "wago-2060-452", note="54V IN")
pcb.add("J2", "wago-2060-452", note="Pi PWM")
pcb.add("C1", "capacitor-1206", value="1u", lcsc="C13832", note="54V")
CCS = [f"CC{i}" for i in range(1, 7)]
for cc in CCS:
    pcb.add(cc, "al5809-60")
pcb.add("Q1", "mosfet-aod482", note="PWM switch")
pcb.add("RIN", "jlcpcb:resistor-0603", value="1k", lcsc="C21190")
pcb.add("RPD", "jlcpcb:resistor-0603", value="100k", lcsc="C25803")
pcb.add("VS", "vsource", value="DC 54")                                  # virtual: the supply
pcb.add("VP", "vsource", value="PULSE(0 3.3 0 100n 100n 1.5m 5m)")        # virtual: Pi GPIO, 200 Hz 30 %

# ---- Netlist. String A = D1..D16, string B = D17..D32, each a series chain.
pcb.connect("J1.1", "C1.1", "D1.A", "D17.A", "VS.+", net="VIN")
pcb.connect("J1.2", "C1.2", "Q1.S", "RPD.2", "J2.2", "VS.-", "VP.-", net="GND")
for i in range(1, 16):
    pcb.connect(f"D{i}.K", f"D{i + 1}.A", net=f"LA{i}")
pcb.connect("D16.K", "CC1.IN", "CC2.IN", "CC3.IN", net="STR_A")
for i in range(17, 32):
    pcb.connect(f"D{i}.K", f"D{i + 1}.A", net=f"LB{i - 16}")
pcb.connect("D32.K", "CC4.IN", "CC5.IN", "CC6.IN", net="STR_B")
pcb.connect(*[f"{cc}.OUT" for cc in CCS], "Q1.D", net="SW")
for i in range(1, 16):
    pcb.net_class(f"LA{i}", "led")
    pcb.net_class(f"LB{i}", "led")
for n in ("VIN", "STR_A", "STR_B", "SW"):
    pcb.net_class(n, "bus")
pcb.connect("J2.1", "RIN.1", "VP.+", net="PWM")
pcb.connect("RIN.2", "RPD.1", "Q1.G", net="PWM_G")

# ---- Board. LEDs in 4 rows of 8 on a 7 mm pitch; the rows sit at y = 12/21/30/39 so the
# bottom row clears the 8.2 mm-deep WAGO housings. Row direction alternates so each
# string snakes: A right->left along row 1 then left->right along row 2 (ending at D16,
# right), B left->right along row 3 then right->left along row 4 (ending at D32, left).
pcb.outline_rect(60, 60, radius=2)
ROWS = [(range(1, 9), 12, 0, True), (range(9, 17), 21, 180, False),
        (range(17, 25), 30, 180, False), (range(25, 33), 39, 0, True)]
for leds, y, rot, right_to_left in ROWS:
    for k, i in enumerate(leds):
        x = 6 + 7 * (7 - k if right_to_left else k)
        pcb.place(f"D{i}", (x, y), rotation=rot)
pcb.place("J1", (9.5, 4.5))              # 1.5 mm in from the edge: leaves the far-left lane free
pcb.place("J2", (46, 4.5), rotation=180)  # Pi, wires from the right edge; pole 1 (PWM) lower row
pcb.place("C1", (18.5, 8.0), rotation=270)  # pin 1 (VIN) north on the feed trace, pin 2 (GND) south
for cc, y in zip(("CC1", "CC2", "CC3"), (52, 49, 46)):
    pcb.place(cc, (54, y))                # IN (pin 1) toward the right margin
for cc, y in zip(("CC4", "CC5", "CC6"), (52, 49, 46)):
    pcb.place(cc, (6, y), rotation=180)
pcb.place("Q1", (30, 51), rotation=180)   # leads up: S left, G right; drain tab toward the LEDs
pcb.place("RIN", (35, 57.5), rotation=270)  # pad 1 (PWM) at the top, pad 2 (PWM_G) below
pcb.place("RPD", (35, 53.5), rotation=270)  # pad 1 (PWM_G) top, pad 2 (GND) bottom
pcb.pour("gnd", layer="F.Cu", net="GND", follow_outline=True)

# ---- Hand-routed copper (coordinates from pcb.pads()). Chamfers give the buses 45° corners.
L = "F.Cu"
# 54 V feed: J1 pole 1 -> left-margin riser -> D17.A (string B), and J1 -> C1.1 -> along the
# bottom LED row -> D1.A (string A). Running the feed just under row 1 instead of through J2
# leaves J2's pole gap to the ground fill, which ties the fill inside the string-A loop to
# J2's ground pads.
pcb.trace("VIN", [(4, 6.5), (2.5, 6.5), (2.5, 30), (4.7, 30)], layer=L, width=1.0, chamfer=1)
pcb.trace("VIN", [(16, 6.5), (16.7, 6.5), (16.7, 10.2), (57.5, 10.2), (57.5, 12), (56.3, 12)],
          layer=L, width=0.8, chamfer=1)
# LED-to-LED links, pad centre to pad centre, 2 mm (the `led` class width). Rows at rotation 0
# have K at x-0.71 and A at x+1.31; rotation 180 swaps the signs.
for leds, y, rot, right_to_left in ROWS:
    ids = list(leds)
    for k, i in enumerate(ids[:-1]):
        x = 6 + 7 * (7 - k if right_to_left else k)
        x_next = x - 7 if right_to_left else x + 7
        k_pad = x - 0.71 if rot == 0 else x + 0.71
        a_pad = x_next + 1.31 if rot == 0 else x_next - 1.31
        net = f"LA{i}" if i < 16 else f"LB{i - 16}"
        pcb.trace(net, [(k_pad, y), (a_pad, y)], layer=L)
# Row turns (D8->D9, D24->D25): straight 0.8 mm verticals centred in the anode pads. The
# left one leaves a 0.65 mm ground channel beside it, the only way into the fill between rows.
pcb.trace("LA8", [(4.75, 12), (4.75, 21)], layer=L, width=0.8)
pcb.trace("LB8", [(56.31, 30), (56.31, 39)], layer=L, width=0.8)
# String returns: D16.K -> CC1-3.IN down the right margin, D32.K -> CC4-6.IN down the left.
pcb.trace("STR_A", [(55.7, 21), (57.8, 21), (57.8, 52), (55.525, 52)], layer=L, chamfer=1)
pcb.trace("STR_A", [(57.8, 49), (55.525, 49)], layer=L)
pcb.trace("STR_A", [(57.8, 46), (55.525, 46)], layer=L)
pcb.trace("STR_B", [(5.3, 39), (3.7, 39), (3.7, 52), (4.475, 52)], layer=L, chamfer=1)
pcb.trace("STR_B", [(3.7, 49), (4.475, 49)], layer=L)
pcb.trace("STR_B", [(3.7, 46), (4.475, 46)], layer=L)
# Switch node: each regulator bank's OUT pins onto a short bus, then a 2 mm run straight into
# the sides of Q1's drain tab.
pcb.trace("SW", [(52.475, 46), (50.5, 46), (50.5, 52), (52.475, 52)], layer=L, chamfer=1)
pcb.trace("SW", [(52.475, 49), (50.5, 49)], layer=L)
pcb.trace("SW", [(50.5, 49), (32.5, 49)], layer=L, width=2.0)
pcb.trace("SW", [(7.525, 46), (9.5, 46), (9.5, 52), (7.525, 52)], layer=L, chamfer=1)
pcb.trace("SW", [(7.525, 49), (9.5, 49)], layer=L)
pcb.trace("SW", [(9.5, 49), (27.5, 49)], layer=L, width=2.0)
# Pi signal: J2 pole 1 -> up the right edge -> along the top -> RIN.
pcb.trace("PWM", [(50, 2.5), (59.3, 2.5), (59.3, 59.1), (35, 59.1), (35, 58.275)], layer=L, chamfer=1.5)
pcb.trace("PWM_G", [(35, 56.725), (35, 54.275)], layer=L)
pcb.trace("PWM_G", [(35, 56), (33.2, 56), (32.28, 55.7)], layer=L)

# ---- Simulations
ON = {"VP.value": "DC 3.3"}
pcb.sim_add("op", "op", probes=["v(STR_A)", "v(STR_B)", "v(SW)", "i(VS)"], params=ON)
pcb.sim_add("line", "dc", "VS", 45, 58, 0.25, probes=["i(VS)", "v(STR_A)", "v(SW)"], params=ON)
pcb.sim_add("pwm", "tran", "20u", "20m", probes=["v(PWM)", "i(VS)", "v(SW)", "@d1[id]"])
pcb.sim_add("thermal", "temp", -20, 85, 5, probes=["i(VS)", "v(STR_A)", "v(SW)"], params=ON)
# Mismatch: every LED gets its own Vf (a reel is one 0.1 V bin; parts scatter +-0.05 V inside
# it) and the two regulator banks sit at opposite ends of their 57-63 mA tolerance.
rng = random.Random(20260905)
mismatch = dict(ON)
mismatch.update({f"{cc}.current": "63m" for cc in ("CC1", "CC2", "CC3")})
mismatch.update({f"{cc}.current": "57m" for cc in ("CC4", "CC5", "CC6")})
mismatch.update({f"{d}.vf_trim": round(rng.uniform(-0.05, 0.05), 3) for d in LEDS})
pcb.sim_add("mismatch", "op", probes=["i(VS)", "i(VD1_trim)", "i(VD17_trim)", "v(STR_A)", "v(STR_B)",
                                      "v(SW)", "@d1[p]", "@d17[p]"], params=mismatch)
# Worst forward-voltage bin (N1, +0.36 V per LED): what supply keeps the strings regulated?
worst = dict(ON, **{f"{d}.vf_trim": 0.36 for d in LEDS})
pcb.sim_add("worst-bin", "dc", "VS", 50, 58, 0.25,
            probes=["i(VD1_trim)", "i(VD17_trim)", "v(STR_A)", "v(STR_B)"], params=worst)
# Same worst bin, hot: the -1.7 mV/K tempco buys back headroom at temperature.
pcb.sim_add("worst-bin-hot", "dc", "VS", 50, 58, 0.25, temperature=85,
            probes=["i(VD1_trim)", "i(VD17_trim)", "v(STR_A)", "v(STR_B)"], params=worst)

# ---- Labels: out of the traces and off neighbouring pads.
pcb.label(*CCS, at=(0, 1.5), size=0.7)                 # in the 1.5 mm gap between regulators
pcb.label("RIN", at=(38, 57.5), absolute=True, size=0.7)  # right of the resistors
pcb.label("RPD", at=(38.2, 53.5), absolute=True, size=0.7)
pcb.label("Q1", at=(19.5, 52), absolute=True)           # free space left of the MOSFET
pcb.label("C1", at=(21.6, 8.0), absolute=True, size=0.7)
pcb.label("J1", at=(1.25, 0), size=0.7)                # between the solder tabs of each pole
pcb.label("J2", at=(1.25, 0), size=0.7)

# ---- Build: the project is written once, then checked and turned into fab files.
pcb.save()
print(pcb.run("status").splitlines()[0])
print(pcb.visualize("netlist").strip())
print(pcb.route().strip())
findings = [f for f in pcb.check() if f["severity"] in ("error", "warning") and not f.get("waived")]
for f in findings:
    print(f["severity"], f["rule"], f["message"])
print(pcb.run("check").strip().splitlines()[-1])
print(pcb.visualize("pcb").strip())
if SIM:
    print(pcb.sim_run().strip().splitlines()[-1])
print(pcb.gerbers().strip().splitlines()[-1])
print(pcb.bom().strip().splitlines()[-1])
print(pcb.pnp().strip().splitlines()[-1])
pcb.save()
