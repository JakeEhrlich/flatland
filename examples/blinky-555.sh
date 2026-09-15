#!/bin/sh
# An all-SMD 555 LED blinker (~0.7 Hz) built entirely from JLCPCB basic
# parts, ready for their SMT assembly service: gerbers + BOM + CPL.
# Run from the repo root: sh examples/blinky-555.sh
set -e
PCB=${PCB:-"cargo run -q --"}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR=${1:-"$ROOT/examples/blinky-555"}
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

# Two-layer board; JLCPCB parts plus the basic library for simulation sources.
$PCB init blinky-555 --layers F.Cu,B.Cu --index "$ROOT/library-jlcpcb/index.json" --index "$ROOT/library/index.json"
$PCB rules set trace_width=0.25 clearance=0.15 via_drill=0.3 via_diameter=0.6 silk_text_size=0.8
$PCB drc add jlcpcb-fr4-2layer   # the fab's limits; `pcb check` runs them
$PCB drc waive design-rules-silk --reason "0.8 mm reference designators are a deliberate choice on this small board; JLCPCB's 1 mm figure is legibility guidance"

# Parts (LCSC numbers are JLCPCB basic parts; passives carry theirs per value)
$PCB add U1 ne555dr
$PCB add R1 jlcpcb:resistor-0603 --param value=10k  --param lcsc=C25804
$PCB add R2 jlcpcb:resistor-0603 --param value=100k --param lcsc=C25803
$PCB add R3 jlcpcb:resistor-0603 --param value=330  --param lcsc=C23138
$PCB add C1 capacitor-0805       --param value=10u  --param lcsc=C15850
$PCB add C2 jlcpcb:capacitor-0603 --param value=100n --param lcsc=C14663 --note "VCC decoupling"
$PCB add C3 jlcpcb:capacitor-0603 --param value=100n --param lcsc=C14663 --note "CTRL"
$PCB add D1 led-0603-red
$PCB add J1 solder-pads-2 --note "5V in"
$PCB add V1 vsource --param value="DC 5"        # virtual supply for simulation

# Netlist: classic astable, f = 1.44 / ((R1 + 2 R2) C1) ~= 0.69 Hz, ~52 % duty
$PCB connect J1.+ U1.VCC U1.RESET R1.1 C2.1 V1.+ --net VCC
$PCB connect J1.- U1.GND C1.2 C2.2 C3.2 D1.K V1.- --net GND
$PCB connect R1.2 R2.1 U1.DIS --net DIS
$PCB connect R2.2 U1.THR U1.TRIG C1.1 --net THR
$PCB connect U1.CTRL C3.1 --net CTRL
$PCB connect U1.OUT R3.1 --net OUT
$PCB connect R3.2 D1.A --net LED_A

# Board: 20 x 16 mm, everything on top, ground pours on both sides
$PCB outline rect 20 16 --radius 1.5
$PCB place U1 10,8
$PCB place J1 4,13.2
$PCB place C2 15.6,11.6 --rotation 90
$PCB place R1 15.6,8.6  --rotation 90
$PCB place R2 17.6,6.6  --rotation 90
$PCB place C1 10.5,2.6
$PCB place C3 14.5,2.6
$PCB place R3 4.4,8.2   --rotation 90
$PCB place D1 4.4,3.6
$PCB pour new gnd_top --layer F.Cu --net GND --follow-outline
$PCB pour new gnd_bottom --layer B.Cu --net GND --follow-outline
# One stitching via so the bottom fill is actually ground (without it the fill is floating copper).
$PCB via add --net GND 2,2
# C1's ground pad ends up walled in by the THR and CTRL traces, a pocket of
# top fill that reaches nothing else; this via ties the pocket to the bottom fill.
$PCB via add --net GND 11.4375,3.95

# Simulations
# (no `op` study: an astable has no DC operating point, ngspice would not converge)
$PCB sim add blink tran 2m 4 --probe "v(OUT)" --probe "v(THR)" --probe "@d1[id]" --probe "i(V1)"

$PCB status
$PCB visualize netlist
$PCB route
$PCB check
$PCB visualize pcb
$PCB sim run
$PCB gerbers
$PCB bom
$PCB pnp
