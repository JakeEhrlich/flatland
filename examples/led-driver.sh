#!/bin/sh
# Builds a small single-sided LED driver board from scratch with the CLI,
# then simulates and routes it. Run from the repo root: sh examples/led-driver.sh
set -e
PCB=${PCB:-"cargo run -q --"}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR=${1:-"$ROOT/examples/led-driver"}
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

$PCB init led-driver --layers B.Cu --index "$ROOT/library/index.json"

# Parts
$PCB add J1 battery-9v
$PCB add J2 header-1x02 --note "SIG/GND in"
$PCB add R1 resistor-axial --param value=10k
$PCB add R2 resistor-axial --param value=330 --param tc1=200e-6
$PCB add Q1 npn-2n2222
$PCB add D1 led-5mm
$PCB add C1 capacitor-electrolytic --param value=100u
$PCB add V1 vsource --param value="PULSE(0 5 0 1u 1u 1m 2m)"   # virtual: drives SIG in simulation

# Netlist
$PCB connect J1.+ R2.1 C1.+ --net VIN
$PCB connect J1.- Q1.E C1.- J2.2 V1.- --net GND
$PCB connect R2.2 D1.A --net LED_A
$PCB connect D1.K Q1.C --net LED_K
$PCB connect J2.1 R1.1 V1.+ --net SIG
$PCB connect R1.2 Q1.B --net BASE

# Board
$PCB outline rect 40 30 --radius 2
$PCB hole add 3,3 --drill 2.2
$PCB hole add 37,27 --drill 2.2
$PCB place J1 4,20 --rotation 0
$PCB place J2 4,10
$PCB place C1 11,22 --rotation 90
$PCB place R2 21,25
$PCB place R1 17,7
$PCB place Q1 27,9 --rotation 180
$PCB place D1 31,20 --rotation 90
$PCB pour new gnd --layer B.Cu --net GND --follow-outline

# Simulations
$PCB sim add op op --probe "v(LED_A)" --probe "v(LED_K)" --probe "i(VJ1)"
$PCB sim add blink tran 10u 6m --probe "v(SIG)" --probe "v(LED_K)" --probe "@d1[id]"
$PCB sim add thermal temp -40 100 5 --probe "@d1[id]" --probe "@r2[p]" --param V1.value="DC 5"

$PCB status
$PCB visualize netlist
$PCB visualize pcb

# Autoroute (freerouting runs headless via -de/-do; needs the app or jar installed)
$PCB route
$PCB check
$PCB visualize pcb
$PCB gerbers
