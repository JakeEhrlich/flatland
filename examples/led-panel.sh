#!/bin/sh
# 60 x 60 mm single-layer ALUMINIUM LED panel for a Mean Well HLG-240H-54A:
# 32 x OSRAM DURIS E 2835 CRI-90 LEDs in two strings of 16 (~47 V each), each
# string regulated to 180 mA by three parallel AL5809-60 two-terminal
# constant-current regulators, PWM-dimmed by one low-side MOSFET driven
# straight from a 3.3 V Raspberry Pi GPIO (high = on, 100-200 Hz).
# Peak ~17 W of LED power; average set by duty cycle. WAGO 2060 push-in
# terminals for the 54 V supply (left edge) and the Pi (right edge).
# Run from the repo root: sh examples/led-panel.sh
set -e
PCB=${PCB:-"cargo run -q --"}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR=${1:-"$ROOT/examples/led-panel"}
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

# One copper layer (order as an aluminium-core board at JLCPCB).
$PCB init led-panel --layers F.Cu --index "$ROOT/library-jlcpcb/index.json" --index "$ROOT/library/index.json"
$PCB rules set trace_width=0.4 clearance=0.3 pour_clearance=0.35 edge_clearance=0.5 silk_text_size=0.8 thermal_spoke_width=0.4
# LED-to-LED links as wide as the LED pads; the supply/return buses 1.2 mm. Electrically 0.4 mm
# would do at 180 mA — the width is for looks and a little extra copper for heat spreading.
$PCB drc add jlcpcb-aluminium-1layer   # metal-core process limits; `pcb check` runs them
$PCB drc waive design-rules-silk --reason "0.8 mm reference designators are a deliberate choice on this small board; JLCPCB's 1 mm figure is legibility guidance"
$PCB rules class led --width 2.0 --clearance 0.3
$PCB rules class bus --width 1.2 --clearance 0.3
$PCB stackup set F.Cu --board-thickness 1.6

# ---- Parts
for i in $(seq 1 32); do $PCB add D$i led-duris-e-2835-cri90 >/dev/null; done
$PCB add J1 wago-2060-452 --note "54V IN"
$PCB add J2 wago-2060-452 --note "Pi PWM"
$PCB add C1 capacitor-1206 --param value=1u --param lcsc=C13832 --note "54V"
for i in 1 2 3 4 5 6; do $PCB add CC$i al5809-60 >/dev/null; done
$PCB add Q1 mosfet-aod482 --note "PWM switch"
$PCB add RIN jlcpcb:resistor-0603 --param value=1k --param lcsc=C21190
$PCB add RPD jlcpcb:resistor-0603 --param value=100k --param lcsc=C25803
$PCB add VS vsource --param value="DC 54"                                   # virtual: the supply
$PCB add VP vsource --param value="PULSE(0 3.3 0 100n 100n 1.5m 5m)"         # virtual: Pi GPIO, 200 Hz 30 %

# ---- Netlist
$PCB connect J1.1 C1.1 D1.A D17.A VS.+ --net VIN
$PCB connect J1.2 C1.2 Q1.S RPD.2 J2.2 VS.- VP.- --net GND
i=1; while [ $i -lt 16 ]; do $PCB connect D$i.K D$((i+1)).A --net LA$i >/dev/null; i=$((i+1)); done
$PCB connect D16.K CC1.IN CC2.IN CC3.IN --net STR_A
i=17; while [ $i -lt 32 ]; do $PCB connect D$i.K D$((i+1)).A --net LB$((i-16)) >/dev/null; i=$((i+1)); done
$PCB connect D32.K CC4.IN CC5.IN CC6.IN --net STR_B
$PCB connect CC1.OUT CC2.OUT CC3.OUT CC4.OUT CC5.OUT CC6.OUT Q1.D --net SW
for i in $(seq 1 15); do $PCB net class LA$i led >/dev/null; $PCB net class LB$i led >/dev/null; done
for n in VIN STR_A STR_B SW; do $PCB net class $n bus >/dev/null; done
$PCB connect J2.1 RIN.1 VP.+ --net PWM
$PCB connect RIN.2 RPD.1 Q1.G --net PWM_G

# ---- Board: LEDs in 4 rows of 8 (7 mm pitch, y = 10..40); connectors on the
# bottom corners; regulators beside their string ends at the top corners; the
# PWM switch top-centre. One copper layer, so the bus is hand-routed planar:
#   VIN: J1 -> up the left margin to row 3, and along the bottom edge / right
#        margin to row 1. String A ends at the right (D16) -> right margin ->
#        CC1-3; string B ends at the left (D32) -> left margin -> CC4-6.
#   SW joins all regulator outputs to Q1's drain; PWM runs up the right edge
#   and along the top edge to RIN. GND is the pour.
$PCB outline rect 60 60 --radius 2
k=7; for i in $(seq 1 8);   do $PCB place D$i  $((6+7*k)),12 --rotation 0   >/dev/null; k=$((k-1)); done   # A: right -> left
k=0; for i in $(seq 9 16);  do $PCB place D$i  $((6+7*k)),21 --rotation 180 >/dev/null; k=$((k+1)); done   # A: left -> right, ends D16 (right)
k=0; for i in $(seq 17 24); do $PCB place D$i  $((6+7*k)),30 --rotation 180 >/dev/null; k=$((k+1)); done   # B: left -> right
k=7; for i in $(seq 25 32); do $PCB place D$i  $((6+7*k)),39 --rotation 0   >/dev/null; k=$((k-1)); done   # B: right -> left, ends D32 (left)
$PCB place J1 9.5,4.5          # 1.5 mm in from the edge: leaves the far-left lane to the GND return
$PCB place J2 46,4.5 --rotation 180     # Pi, wires from the right edge; pole 1 (PWM) lower row
$PCB place C1 18.5,8.0 --rotation 270   # pin 1 (VIN) north on the feed trace, pin 2 (GND) south
$PCB place CC1 54,52 && $PCB place CC2 54,49 && $PCB place CC3 54,46          # IN (pin 1) toward the right margin
$PCB place CC4 6,52 --rotation 180 && $PCB place CC5 6,49 --rotation 180 && $PCB place CC6 6,46 --rotation 180
$PCB place Q1 30,51 --rotation 180      # leads up: S left, G right; drain tab toward the LEDs
$PCB place RIN 35,57.5 --rotation 270   # pad 1 (PWM) at the top, pad 2 (PWM_G) below
$PCB place RPD 35,53.5 --rotation 270   # pad 1 (PWM_G) top, pad 2 (GND) bottom
$PCB pour new gnd --layer F.Cu --net GND --follow-outline

# LED rows at y = 12/21/30/39: the bottom row clears the 8.2 mm-deep WAGO housings by 2.3 mm.
# ---- Hand-routed traces (see `pcb pads` for the coordinates). --chamfer gives the buses
# 45° corners to match the router's wiring.
# 54 V feed: J1 pole 1 -> left-margin riser -> D17.A (string B), and J1 -> C1.1 -> along the
# bottom LED row -> D1.A (string A). Running the feed just under row 1 instead of through J2
# leaves J2's pole gap to the ground fill, which is what ties the fill inside the string-A
# loop (between rows 1 and 2, and the right-hand corridor) to J2's ground pads.
$PCB trace add --layer F.Cu --net VIN --width 1.0 --chamfer 1 4,6.5 2.5,6.5 2.5,30 4.7,30
$PCB trace add --layer F.Cu --net VIN --width 0.8 --chamfer 1 16,6.5 16.7,6.5 16.7,10.2 57.5,10.2 57.5,12 56.3,12
# LED-to-LED links: straight, pad centre to pad centre, 2 mm (the `led` class width). Drawn
# here rather than left to freerouting, whose round-cap model stops a wide wire short of a
# narrow anode pad. Rows 1 and 4 read right to left (rotation 0: K at x-0.71, A at x+1.31),
# rows 2 and 3 left to right (rotation 180: K at x+0.71, A at x-1.31).
link() { $PCB trace add --layer F.Cu --net $1 $2,$3 $4,$3 >/dev/null; }
k=7; for i in 1 2 3 4 5 6 7;       do x=$((6+7*k)); link LA$i  $(echo "$x-0.71" | bc) 12 $(echo "$x-7+1.31" | bc); k=$((k-1)); done
k=0; for i in 9 10 11 12 13 14 15; do x=$((6+7*k)); link LA$i  $(echo "$x+0.71" | bc) 21 $(echo "$x+7-1.31" | bc); k=$((k+1)); done
k=0; for i in 1 2 3 4 5 6 7;       do x=$((6+7*k)); link LB$i  $(echo "$x+0.71" | bc) 30 $(echo "$x+7-1.31" | bc); k=$((k+1)); done
k=7; for i in 9 10 11 12 13 14 15; do x=$((6+7*k)); link LB$i  $(echo "$x-0.71" | bc) 39 $(echo "$x-7+1.31" | bc); k=$((k-1)); done
# Row turns (D8->D9, D24->D25): straight 0.8 mm verticals centred in the anode pads
# (a 2 mm link cannot enter a 0.9 mm anode pad, so the router gives up on them).
$PCB trace add --layer F.Cu --net LA8 --width 0.8 4.75,12 4.75,21    # leaves a 0.65 mm ground channel beside it: the only way into the fill between rows 2 and 3
$PCB trace add --layer F.Cu --net LB8 --width 0.8 56.31,30 56.31,39
# String returns: D16.K -> CC1-3.IN down the right margin, D32.K -> CC4-6.IN down the left.
$PCB trace add --layer F.Cu --net STR_A --chamfer 1 55.7,21 57.8,21 57.8,52 55.525,52
$PCB trace add --layer F.Cu --net STR_A 57.8,49 55.525,49
$PCB trace add --layer F.Cu --net STR_A 57.8,46 55.525,46
$PCB trace add --layer F.Cu --net STR_B --chamfer 1 5.3,39 3.7,39 3.7,52 4.475,52
$PCB trace add --layer F.Cu --net STR_B 3.7,49 4.475,49
$PCB trace add --layer F.Cu --net STR_B 3.7,46 4.475,46
# Switch node: each regulator bank's OUT pins onto a short bus, then a 2 mm run straight into
# the sides of Q1's drain tab.
$PCB trace add --layer F.Cu --net SW --chamfer 1 52.475,46 50.5,46 50.5,52 52.475,52
$PCB trace add --layer F.Cu --net SW 52.475,49 50.5,49
$PCB trace add --layer F.Cu --net SW --width 2.0 50.5,49 32.5,49
$PCB trace add --layer F.Cu --net SW --chamfer 1 7.525,46 9.5,46 9.5,52 7.525,52
$PCB trace add --layer F.Cu --net SW 7.525,49 9.5,49
$PCB trace add --layer F.Cu --net SW --width 2.0 9.5,49 27.5,49
# Pi signal: J2 pole 1 -> up the right edge -> along the top -> RIN.
$PCB trace add --layer F.Cu --net PWM --chamfer 1.5 50,2.5 59.3,2.5 59.3,59.1 35,59.1 35,58.275
$PCB trace add --layer F.Cu --net PWM_G 35,56.725 35,54.275
$PCB trace add --layer F.Cu --net PWM_G 35,56 33.2,56 32.28,55.7

# ---- Simulations
$PCB sim add op op --probe "v(STR_A)" --probe "v(STR_B)" --probe "v(SW)" --probe "i(VS)" -P VP.value="DC 3.3"
$PCB sim add line dc VS 45 58 0.25 --probe "i(VS)" --probe "v(STR_A)" --probe "v(SW)" -P VP.value="DC 3.3"
$PCB sim add pwm tran 20u 20m --probe "v(PWM)" --probe "i(VS)" --probe "v(SW)" --probe "@d1[id]"
$PCB sim add thermal temp -20 85 5 --probe "i(VS)" --probe "v(STR_A)" --probe "v(SW)" -P VP.value="DC 3.3"
# Mismatch: every LED gets its own Vf (a reel is one 0.1 V bin, parts scatter +-0.05 V
# inside it) and the two regulator banks sit at opposite ends of their 57-63 mA tolerance.
$PCB sim add mismatch op --probe "i(VS)" --probe "i(VD1_trim)" --probe "i(VD17_trim)" --probe "v(STR_A)" --probe "v(STR_B)" --probe "v(SW)" \
  --probe "@d1[p]" --probe "@d17[p]" -P VP.value="DC 3.3" \
  -P CC1.current=63m -P CC2.current=63m -P CC3.current=63m -P CC4.current=57m -P CC5.current=57m -P CC6.current=57m \
  -P D1.vf_trim=0.009 -P D2.vf_trim=-0.023 -P D3.vf_trim=0.022 -P D4.vf_trim=0.001 -P D5.vf_trim=0.04 -P D6.vf_trim=-0.039 -P D7.vf_trim=0.023 -P D8.vf_trim=-0.027 -P D9.vf_trim=-0.02 -P D10.vf_trim=0.042 -P D11.vf_trim=-0.037 -P D12.vf_trim=0.006 -P D13.vf_trim=-0.046 -P D14.vf_trim=-0.049 -P D15.vf_trim=0.041 -P D16.vf_trim=-0.028 -P D17.vf_trim=0.019 -P D18.vf_trim=-0.034 -P D19.vf_trim=0.049 -P D20.vf_trim=0.014 -P D21.vf_trim=0.035 -P D22.vf_trim=0.043 -P D23.vf_trim=0.008 -P D24.vf_trim=-0.04 -P D25.vf_trim=-0.01 -P D26.vf_trim=0.013 -P D27.vf_trim=0.023 -P D28.vf_trim=0.017 -P D29.vf_trim=-0.038 -P D30.vf_trim=0.009 -P D31.vf_trim=0.007 -P D32.vf_trim=-0.045
# Worst forward-voltage bin (N1, +0.36 V per LED): what supply voltage keeps the strings regulated?
$PCB sim add worst-bin dc VS 50 58 0.25 --probe "i(VD1_trim)" --probe "i(VD17_trim)" --probe "v(STR_A)" --probe "v(STR_B)" -P VP.value="DC 3.3" \
  $(for i in $(seq 1 32); do printf -- "-P D%d.vf_trim=0.36 " $i; done)
# Same worst bin, hot: the -1.7 mV/K tempco buys back headroom once the panel is at temperature.
$PCB sim add worst-bin-hot dc VS 50 58 0.25 --temperature 85 --probe "i(VD1_trim)" --probe "i(VD17_trim)" --probe "v(STR_A)" --probe "v(STR_B)" -P VP.value="DC 3.3" \
  $(for i in $(seq 1 32); do printf -- "-P D%d.vf_trim=0.36 " $i; done)

# Labels: keep them out of the traces and off neighbouring pads.
$PCB label CC1 CC2 CC3 CC4 CC5 CC6 --at 0,1.5 --size 0.7   # in the 1.5 mm gap between regulators
$PCB label RIN --at 38,57.5 --absolute --size 0.7          # right of the resistors, clear of the gate trace
$PCB label RPD --at 38.2,53.5 --absolute --size 0.7
$PCB label Q1 --at 19.5,52 --absolute                      # free space left of the MOSFET
$PCB label C1 --at 21.6,8.0 --absolute --size 0.7
$PCB label J1 --at 1.25,0 --size 0.7                       # between the solder tabs of each pole
$PCB label J2 --at 1.25,0 --size 0.7

$PCB status
$PCB visualize netlist
$PCB route
$PCB check
$PCB visualize pcb
$PCB sim run
$PCB gerbers
$PCB bom
$PCB pnp
