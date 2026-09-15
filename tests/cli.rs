//! End-to-end tests driving the `pcb` binary.

use std::path::{Path, PathBuf};
use std::process::Command;

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("library/index.json")
}

struct Proj {
    dir: PathBuf,
}

impl Proj {
    fn new(name: &str) -> Proj {
        let dir = std::env::temp_dir().join(format!("flatland-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Proj { dir }
    }
    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_pcb")).args(args).current_dir(&self.dir).output().expect("run pcb");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        (out.status.success(), text)
    }
    fn ok(&self, args: &[&str]) -> String {
        let (ok, text) = self.run(args);
        assert!(ok, "pcb {} failed:\n{text}", args.join(" "));
        text
    }
    fn fails(&self, args: &[&str]) -> String {
        let (ok, text) = self.run(args);
        assert!(!ok, "pcb {} unexpectedly succeeded:\n{text}", args.join(" "));
        text
    }
    fn build(&self, rel: &str) -> PathBuf {
        self.dir.join("build").join(rel)
    }
}

impl Drop for Proj {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ngspice_available() -> bool {
    std::env::var_os("NGSPICE_LIB").is_some()
        || ["/opt/homebrew/lib/libngspice.dylib", "/usr/local/lib/libngspice.dylib", "/usr/lib/x86_64-linux-gnu/libngspice.so.0"]
            .iter()
            .any(|p| Path::new(p).exists())
}

fn freerouting_available() -> bool {
    std::env::var_os("FREEROUTING").is_some() || Path::new("/Applications/freerouting.app/Contents/MacOS/freerouting").exists()
}

fn build_led_driver(p: &Proj) {
    let lib = library();
    p.ok(&["init", "led", "--layers", "B.Cu", "--index", lib.to_str().unwrap()]);
    // TO-92 leads on a 1.27 mm pitch leave 0.17 mm between pads.
    p.ok(&["rules", "set", "clearance=0.15"]);
    p.ok(&["add", "J1", "battery-9v"]);
    p.ok(&["add", "R1", "resistor-axial", "--param", "value=10k"]);
    p.ok(&["add", "R2", "resistor-axial", "--param", "value=330"]);
    p.ok(&["add", "Q1", "npn-2n2222"]);
    p.ok(&["add", "D1", "led-5mm"]);
    p.ok(&["add", "V1", "vsource", "--param", "value=PULSE(0 5 0 1u 1u 1m 2m)"]);
    p.ok(&["connect", "J1.+", "R2.1", "--net", "VIN"]);
    p.ok(&["connect", "J1.-", "Q1.E", "V1.-", "--net", "GND"]);
    p.ok(&["connect", "R2.2", "D1.A"]);
    p.ok(&["connect", "D1.K", "Q1.C", "--net", "LED_K"]);
    p.ok(&["connect", "R1.1", "V1.+", "--net", "SIG"]);
    p.ok(&["connect", "R1.2", "Q1.B", "--net", "BASE"]);
    p.ok(&["outline", "rect", "40", "30", "--radius", "2"]);
    p.ok(&["hole", "add", "3,3", "--drill", "2.2"]);
    p.ok(&["place", "J1", "4,20"]);
    p.ok(&["place", "R2", "21,25"]);
    p.ok(&["place", "R1", "17,7"]);
    p.ok(&["place", "Q1", "27,9", "--rotation", "180"]);
    p.ok(&["place", "D1", "31,20", "--rotation", "90"]);
    p.ok(&["pour", "new", "gnd", "--layer", "B.Cu", "--net", "GND", "--follow-outline"]);
}

#[test]
fn single_sided_board_end_to_end() {
    let p = Proj::new("single");
    build_led_driver(&p);

    let status = p.ok(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert_eq!(v["layers"], serde_json::json!(["B.Cu"]));
    assert_eq!(v["unrouted_connections"], 4); // GND is joined by the pour; SIG only has one real pad
    assert_eq!(v["outline"]["width_mm"], 40.0);

    // Auto-named net and merge behaviour.
    let text = p.ok(&["net", "list"]);
    assert!(text.contains("N$1: R2.2 D1.A"), "{text}");
    let text = p.fails(&["connect", "VIN_dummy.1", "R1.1"]);
    assert!(text.contains("no instance named `VIN_dummy`"), "{text}");
    let text = p.fails(&["connect", "R1.1", "J1.+"]);
    assert!(text.contains("--merge"), "{text}");

    // Before routing the only errors are the unrouted nets.
    let check = p.run(&["check"]).1;
    assert!(check.contains("error: nets-routed") && check.lines().filter(|l| l.starts_with("error:")).all(|l| l.contains("nets-routed")), "{check}");

    p.ok(&["visualize", "pcb"]);
    p.ok(&["visualize", "pcb", "--from-bottom", "--grid", "-o", p.build("bottom.png").to_str().unwrap()]);
    p.ok(&["visualize", "netlist"]);
    assert!(p.build("pcb.png").exists() && p.build("pcb.svg").exists() && p.build("bottom.png").exists() && p.build("netlist.png").exists());

    let files = p.ok(&["gerbers", "--force"]);
    for f in ["B_Cu.gbl", "F_Mask.gts", "B_Mask.gbs", "F_Silkscreen.gto", "Edge_Cuts.gko", "PTH.drl", "NPTH.drl", "gerbers.zip"] {
        assert!(files.contains(f), "missing {f} in:\n{files}");
    }
    let gbl = std::fs::read_to_string(p.build("gerbers/led-B_Cu.gbl")).unwrap();
    assert!(gbl.starts_with("%TF.GenerationSoftware") && gbl.ends_with("M02*\n") && gbl.contains("G36*"));

    p.ok(&["route", "--dsn-only"]);
    let dsn = std::fs::read_to_string(p.build("led.dsn")).unwrap();
    assert!(dsn.contains("(place Q1 27000 9000 front 180)"), "{dsn}");
    assert!(dsn.contains("(net GND (pins"), "{dsn}");
    // Single-layer boards export no planes: the router must draw the pour net's
    // links itself, the pour then swallows them.
    assert!(!dsn.contains("(plane GND"), "{dsn}");

    // Autorouting keeps GND on the pour: router-drawn GND links are pruned.
    if std::path::Path::new("/Applications/freerouting.app").exists() {
        let out = p.ok(&["route"]);
        assert!(out.contains("all nets routed"), "{out}");
        let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
        let gnd_routed = proj["traces"].as_array().unwrap().iter().filter(|t| t["routed"] == true && t["net"] == "GND").count();
        assert_eq!(gnd_routed, 0, "router GND segments should be pruned:\n{out}");
    }

    // Simulations: definition always works; running needs libngspice.
    p.ok(&["sim", "add", "op", "op", "--probe", "v(LED_K)", "--probe", "@d1[id]"]);
    p.ok(&["sim", "add", "hot", "temp", "-40", "100", "10", "--probe", "@d1[id]", "--param", "V1.value=DC 5"]);
    let net = p.ok(&["sim", "netlist", "hot"]);
    assert!(net.contains("D1 N$1 LED_K LED_RED"), "{net}");
    assert!(net.contains("V1 SIG 0 DC 5"), "{net}");
    assert!(net.contains(".dc temp -40 100 10"), "{net}");
    let text = p.fails(&["sim", "add", "bad", "op", "--param", "R1.resistance=1"]);
    assert!(text.contains("no parameter `resistance`"), "{text}");
    if ngspice_available() {
        let out = p.ok(&["sim", "run", "hot"]);
        assert!(out.contains("@d1[id]"), "{out}");
        assert!(p.build("sim/hot/results.csv").exists() && p.build("sim/hot/plot.png").exists());
        let out = p.ok(&["sim", "run", "hot"]);
        assert!(out.contains("up to date"), "{out}");
        let list = p.ok(&["sim", "list"]);
        assert!(list.contains("hot: .dc temp -40 100 10 — up to date"), "{list}");
        // Changing an input makes the study stale.
        p.ok(&["set", "R2", "value=470"]);
        let list = p.ok(&["sim", "list"]);
        assert!(list.contains("stale"), "{list}");
        let op = p.ok(&["sim", "run", "op"]);
        assert!(op.contains("operating point"), "{op}");
    } else {
        eprintln!("libngspice not found; skipping simulation run");
    }

    if freerouting_available() {
        let out = p.ok(&["route", "--timeout", "180"]);
        assert!(out.contains("all nets routed"), "{out}");
        let check = p.run(&["check"]).1;
        assert!(check.contains("0 error(s), 0 warning(s)"), "{check}");
        // Re-routing replaces routed traces; hand traces would be kept.
        let out = p.ok(&["route", "--timeout", "180"]);
        assert!(out.contains("all nets routed"), "{out}");
    } else {
        eprintln!("freerouting not found; skipping autorouting");
    }
}

#[test]
fn two_layer_smd_board() {
    let p = Proj::new("two");
    let lib = library();
    p.ok(&["init", "smd", "--index", lib.to_str().unwrap()]);
    p.ok(&["add", "R1", "resistor-0603", "--param", "value=1k", "--at", "5,5"]);
    p.ok(&["add", "R2", "resistor-0603", "--param", "value=2k", "--at", "5,10", "--side", "bottom", "--rotation", "90"]);
    p.ok(&["add", "C1", "capacitor-0603", "--at", "10,7"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "A"]);
    p.ok(&["connect", "R1.2", "C1.1", "--net", "B"]);
    p.ok(&["connect", "R2.2", "C1.2", "--net", "GND"]);
    p.ok(&["outline", "rect", "15", "15"]);

    // A hand trace on F.Cu connects R1.2 to C1.1; net is inferred from the pad.
    p.ok(&["trace", "add", "--layer", "F.Cu", "5.875,5", "9.125,7"]);
    let list = p.ok(&["trace", "list"]);
    assert!(list.contains("net B"), "{list}");
    // A via plus a bottom trace joins R2.2 (bottom) to C1.2 (top).
    p.ok(&["via", "add", "12,10", "--net", "GND"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "GND", "10.875,7", "12,10"]);
    // R2 is on the bottom, rotated 90°: pad 2 is mirrored then rotated to (5, 9.125).
    p.ok(&["trace", "add", "--layer", "B.Cu", "--net", "GND", "12,10", "5,9.125"]);
    let status = p.ok(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&status).unwrap();
    let nets = v["nets"].as_array().unwrap();
    let unrouted = |n: &str| nets.iter().find(|x| x["name"] == n).unwrap()["unrouted"].as_u64().unwrap();
    assert_eq!(unrouted("B"), 0);
    assert_eq!(unrouted("GND"), 0);
    assert_eq!(unrouted("A"), 1);

    let text = p.fails(&["trace", "add", "--layer", "In1.Cu", "1,1", "2,2"]);
    assert!(text.contains("stackup only has"), "{text}");
    let text = p.fails(&["place", "R9", "1,1"]);
    assert!(text.contains("no instance named `R9`"), "{text}");
    let text = p.fails(&["add", "R3", "resistor-060"]);
    assert!(text.contains("did you mean `resistor-0603`"), "{text}");
    let text = p.fails(&["connect", "R1.3", "C1.1"]);
    assert!(text.contains("has no pin `3`"), "{text}");

    let files = p.ok(&["gerbers", "--force"]);
    for f in ["F_Cu.gtl", "B_Cu.gbl", "F_Paste.gtp", "B_Paste.gbp", "F_Mask.gts", "B_Mask.gbs", "PTH.drl"] {
        assert!(files.contains(f), "missing {f} in:\n{files}");
    }
    p.ok(&["route", "--dsn-only"]);
    let dsn = std::fs::read_to_string(p.build("smd.dsn")).unwrap();
    assert!(dsn.contains("(place R2 5000 10000 back 90"), "{dsn}");
    assert!(dsn.contains("(wire (path F.Cu 250"), "{dsn}");
    assert!(dsn.contains("(via Via[0-1]_800:400_um 12000 10000 (net GND) (type protect))"), "{dsn}");

    // Import a hand-written session for net A.
    let ses = r#"(session "smd.ses" (routes (resolution um 10) (library_out (padstack "Via[0-1]_800:400_um" (shape (circle F.Cu 8000 0 0)) (shape (circle B.Cu 8000 0 0)) (attach off)))
      (network_out (net A (wire (path F.Cu 2500 41250 50000 30000 80000)) (via "Via[0-1]_800:400_um" 30000 80000) (wire (path B.Cu 2500 30000 80000 50000 108750))))))"#;
    std::fs::write(p.dir.join("hand.ses"), ses).unwrap();
    let out = p.ok(&["route", "--import", "hand.ses"]);
    assert!(out.contains("imported 2 trace segment(s) and 1 via(s)"), "{out}");
    assert!(out.contains("all nets routed"), "{out}");
    p.ok(&["visualize", "pcb", "--from-bottom"]);
    let check = p.run(&["check"]).1;
    assert!(check.contains("0 error(s)"), "{check}");
}

#[test]
fn index_management() {
    let p = Proj::new("index");
    p.ok(&["init", "ix"]);
    let text = p.fails(&["add", "R1", "resistor-0603"]);
    assert!(text.contains("no component indexes"), "{text}");
    p.ok(&["index", "new", "mylib", "--name", "mine"]);
    p.ok(&["component", "template", "mylib/components/widget.json", "--name", "widget"]);
    p.ok(&["index", "register", "mylib", "mylib/components/widget.json"]);
    p.ok(&["index", "add", "mylib", "--pin"]);
    let list = p.ok(&["component", "list"]);
    assert!(list.contains("mine:widget"), "{list}");
    p.ok(&["add", "W1", "widget", "--at", "1,1"]);
    p.ok(&["index", "verify"]);
    // Editing a component file breaks the pinned hash until `index update`.
    let path = p.dir.join("mylib/components/widget.json");
    let mut s = std::fs::read_to_string(&path).unwrap();
    s = s.replace("Template component; edit me", "changed");
    std::fs::write(&path, s).unwrap();
    let text = p.fails(&["status"]);
    assert!(text.contains("hash mismatch"), "{text}");
    p.fails(&["index", "verify"]);
    // The index file itself changed hash too (it is pinned), so update then re-pin.
    p.ok(&["index", "update", "mylib"]);
    let text = p.fails(&["status"]);
    assert!(text.contains("hash mismatch for component index"), "{text}");
    p.ok(&["index", "remove", "mine"]);
    p.ok(&["index", "add", "mylib", "--pin"]);
    p.ok(&["status"]);
}

#[test]
fn jlcpcb_assembly_outputs() {
    let p = Proj::new("jlc");
    let jlc = Path::new(env!("CARGO_MANIFEST_DIR")).join("library-jlcpcb/index.json");
    let basic = library();
    p.ok(&["init", "blinky", "--index", jlc.to_str().unwrap(), "--index", basic.to_str().unwrap()]);
    // Ambiguous name needs qualification.
    let text = p.fails(&["add", "R1", "resistor-0603", "--param", "value=10k"]);
    assert!(text.contains("several indexes"), "{text}");
    p.ok(&["add", "U1", "ne555dr", "--at", "10,8"]);
    p.ok(&["add", "R1", "jlcpcb:resistor-0603", "--param", "value=10k", "--param", "lcsc=C25804", "--at", "15,8", "--rotation", "90"]);
    p.ok(&["add", "R2", "jlcpcb:resistor-0603", "--param", "value=10k", "--param", "lcsc=C25804", "--at", "17,6"]);
    p.ok(&["add", "R9", "jlcpcb:resistor-0603", "--param", "value=1meg", "--at", "3,3"]); // no LCSC number
    p.ok(&["add", "D1", "led-0603-red", "--at", "4,4", "--side", "bottom"]);
    p.ok(&["add", "J1", "solder-pads-2", "--at", "4,13"]);
    p.ok(&["add", "V1", "vsource"]);
    p.ok(&["outline", "rect", "20", "16"]);
    p.ok(&["connect", "U1.VCC", "R1.1", "J1.+", "V1.+", "--net", "VCC"]);
    p.ok(&["connect", "U1.GND", "D1.K", "J1.-", "V1.-", "--net", "GND"]);

    let out = p.ok(&["bom"]);
    assert!(out.contains("not assembled (assembly: false): J1"), "{out}");
    assert!(out.contains("no LCSC part number for R9"), "{out}");
    let bom = std::fs::read_to_string(p.build("assembly/blinky-bom.csv")).unwrap();
    assert!(bom.starts_with("Comment,Designator,Footprint,LCSC Part #\n"), "{bom}");
    assert!(bom.contains("NE555DR,U1,SOIC-8_3.9x4.9mm_P1.27mm,C7593"), "{bom}");
    assert!(bom.contains("10k,\"R1,R2\",0603,C25804"), "{bom}");
    assert!(bom.contains("KT-0603R,D1,LED_0603,C2286"), "{bom}");
    assert!(!bom.contains("R9") && !bom.contains("J1") && !bom.contains("V1"), "{bom}");
    let full = p.ok(&["bom", "-f", "csv", "--all", "-o", "full.csv"]);
    assert!(full.contains("wrote"), "{full}");
    let full = std::fs::read_to_string(p.dir.join("full.csv")).unwrap();
    assert!(full.contains("R9,1,resistor-0603,1meg,0603,UNI-ROYAL,,,yes"), "{full}");
    assert!(full.contains("J1,1,solder-pads-2,solder-pads-2,SolderWirePads_2,,,,no"), "{full}");

    let cpl = p.ok(&["pnp"]);
    assert!(cpl.contains("placement(s)"), "{cpl}");
    let cpl = std::fs::read_to_string(p.build("assembly/blinky-cpl.csv")).unwrap();
    assert!(cpl.starts_with("Designator,Mid X,Mid Y,Layer,Rotation\n"), "{cpl}");
    assert!(cpl.contains("U1,10.0000,8.0000,Top,270\n"), "{cpl}"); // SOIC-8 rotation_offset for JLCPCB
    assert!(cpl.contains("R1,15.0000,8.0000,Top,90\n"), "{cpl}");
    assert!(cpl.contains("D1,4.0000,4.0000,Bottom,0\n"), "{cpl}");
    assert!(!cpl.contains("J1") && !cpl.contains("V1"), "{cpl}");
    // An unplaced assembled part is an error (file still written).
    p.ok(&["unplace", "R2"]);
    let text = p.fails(&["pnp"]);
    assert!(text.contains("not placed: R2"), "{text}");

    // A pin with several solder tabs: every tab carries the net, `pads` lists them,
    // and the tabs count as connected to each other.
    p.ok(&["add", "J9", "wago-2060-452", "--at", "10,13"]);
    p.ok(&["connect", "J9.1", "--net", "VCC"]);
    p.ok(&["connect", "J9.2", "--net", "GND"]);
    let pads = p.ok(&["pads", "J9"]);
    assert!(pads.contains("1A") && pads.contains("1B") && pads.contains("2A") && pads.contains("2B"), "{pads}");
    assert_eq!(pads.matches("VCC").count(), 2, "{pads}");
    let status = p.ok(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&status).unwrap();
    let vcc = v["nets"].as_array().unwrap().iter().find(|n| n["name"] == "VCC").unwrap();
    // J9.1 is one pin (two tabs) plus U1.VCC, R1.1: three separate islands, not four.
    assert_eq!(vcc["islands"].as_array().unwrap().len(), 3, "{status}");
    p.ok(&["route", "--dsn-only"]);
    let dsn = std::fs::read_to_string(p.build("blinky.dsn")).unwrap();
    assert!(dsn.contains("(pins U1-8 R1-1 J9-1A)") || dsn.contains("J9-1A"), "{dsn}");
    assert!(!dsn.contains("J9-1B"), "secondary tab must not be a separate DSN pin:\n{dsn}");
    // Label overrides live on the placement, in the footprint frame.
    p.ok(&["label", "J9", "R1", "--at", "0,-2", "--size", "0.6"]);
    let out = p.ok(&["label", "U1", "--at", "12,3", "--absolute"]);
    assert!(out.contains("U1 label at 12,3"), "{out}");
    let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
    assert_eq!(proj["components"]["R1"]["placement"]["label_size"], 0.6);
    assert!(proj["components"]["U1"]["placement"]["label_at"].is_array());
    p.ok(&["label", "R1", "--hide"]);
    assert!(p.ok(&["label", "R1", "--show"]).contains("R1 label at"));
    p.ok(&["label", "R1", "U1", "--reset"]);
    assert!(p.fails(&["label", "R1"]).contains("nothing to change"));
    p.ok(&["remove", "J9"]);

    // The 555 model simulates and oscillates.
    if ngspice_available() {
        p.ok(&["add", "R3", "jlcpcb:resistor-0603", "--param", "value=100k", "--param", "lcsc=C25803"]);
        p.ok(&["add", "C1", "capacitor-0805", "--param", "value=10u", "--param", "lcsc=C15850"]);
        p.ok(&["connect", "U1.RESET", "--net", "VCC"]);
        p.ok(&["connect", "R1.2", "R3.1", "U1.DIS", "--net", "DIS"]);
        p.ok(&["connect", "R3.2", "U1.THR", "U1.TRIG", "C1.1", "--net", "THR"]);
        p.ok(&["connect", "C1.2", "--net", "GND"]);
        // U1.OUT is left unconnected here; probe the threshold ramp instead.
        p.ok(&["sim", "add", "blink", "tran", "5m", "3", "--probe", "v(THR)"]);
        let out = p.ok(&["sim", "run", "blink"]);
        assert!(out.contains("thr"), "{out}");
        let csv = std::fs::read_to_string(p.build("sim/blink/results.csv")).unwrap();
        let vals: Vec<f64> = csv.lines().skip(1).filter_map(|l| l.split(',').nth(1)?.parse().ok()).collect();
        let max = vals.iter().cloned().fold(0.0, f64::max);
        let min_late = vals.iter().skip(vals.len() / 2).cloned().fold(9.0, f64::min);
        assert!(max > 3.2 && max < 3.5, "threshold should reach 2/3 VCC, max {max}");
        assert!(min_late < 1.8, "threshold should fall back to 1/3 VCC, min {min_late}");
    }
}

#[test]
fn design_rules() {
    let p = Proj::new("drc");
    let basic = library();
    p.ok(&["init", "drc", "--index", basic.to_str().unwrap()]);
    p.ok(&["outline", "rect", "20", "16"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,5"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "10,5"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "A"]);
    p.ok(&["connect", "R1.2", "R2.2", "--net", "GND"]);
    // Basic rules: an unrouted net is an error and blocks gerbers (a built board
    // shipped with an open ground this way); `gerbers --force` is the escape.
    let out = p.fails(&["check"]);
    assert!(out.contains("error: nets-routed"), "{out}");
    let out = p.fails(&["gerbers"]);
    assert!(out.contains("not writing gerbers"), "{out}");
    // A GND trace hugging the A trace closer than the clearance is an error and blocks gerbers.
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "4.125,5", "4.125,6.5", "9.125,6.5", "9.125,5"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "GND", "--width", "0.3", "5.875,5", "5.875,6.2", "10.875,6.2", "10.875,5"]);
    let out = p.fails(&["check"]);
    assert!(out.contains("error: copper-clearance"), "{out}");
    let out = p.fails(&["gerbers"]);
    assert!(out.contains("not writing gerbers"), "{out}");
    p.ok(&["gerbers", "--force"]);
    p.ok(&["trace", "clear"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "4.125,5", "4.125,6.5", "9.125,6.5", "9.125,5"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "GND", "--width", "0.3", "5.875,5", "5.875,3.5", "10.875,3.5", "10.875,5"]);
    let out = p.ok(&["check"]);
    assert!(out.contains("0 error(s), 0 warning(s)"), "{out}");
    // A fab profile: bundled, copied into the project, hash-pinned.
    let out = p.ok(&["drc", "add", "jlcpcb-fr4-2layer"]);
    assert!(out.contains("added rule set jlcpcb-fr4-2layer"), "{out}");
    assert!(p.dir.join("drc/jlcpcb-fr4-2layer.json").exists());
    let out = p.ok(&["check"]);
    assert!(out.contains("from basic, jlcpcb-fr4-2layer"), "{out}");
    // A 0.05 mm trace violates the process minimum and the meta-check on design rules.
    p.ok(&["rules", "set", "trace_width=0.05"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.05", "4.125,4", "4.125,5"]);
    let out = p.fails(&["check"]);
    assert!(out.contains("error: trace-width") && out.contains("error: design-rules"), "{out}");
    // Waive one, fix the other.
    p.ok(&["drc", "waive", "trace-width", "A", "--reason", "test"]);
    p.ok(&["rules", "set", "trace_width=0.25"]);
    let out = p.ok(&["check"]);
    assert!(out.contains("1 waived"), "{out}");
    let out = p.ok(&["check", "--waived"]);
    assert!(out.contains("waived: trace-width"), "{out}");
    // Project rule, explain, list, JSON.
    p.ok(&["drc", "rule", "wide-gnd", "--for", "trace", "--where", "{\"net\":\"GND\"}", "--check", "{\"min_width\": 0.5}", "--severity", "warning"]);
    let out = p.ok(&["check"]);
    assert!(out.contains("warning: wide-gnd"), "{out}");
    let out = p.ok(&["drc", "explain", "wide-gnd"]);
    assert!(out.contains("min_width"), "{out}");
    let out = p.ok(&["drc", "list"]);
    assert!(out.contains("waiver: trace-width"), "{out}");
    let out = p.ok(&["check", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.as_array().unwrap().iter().any(|f| f["rule"] == "wide-gnd"), "{out}");
    // Slivers: a pour squeezed between two traces leaves a thread narrower than the process
    // minimum unless pour_min_width trims it; the check names it either way.
    p.ok(&["drc", "unrule", "wide-gnd"]);
    p.ok(&["drc", "unwaive", "trace-width"]);
    p.ok(&["trace", "clear"]);
    p.ok(&["pour", "new", "gnd", "--layer", "F.Cu", "--net", "GND", "--follow-outline"]);
    p.ok(&["rules", "set", "pour_min_width=0.01", "pour_clearance=0.2"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "4.125,5", "4.125,6.5", "9.125,6.5", "9.125,5"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "3,7.25", "11,7.25"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("trace-width: pour gnd has a sliver"), "{out}");
    p.ok(&["rules", "set", "pour_min_width=0.3"]);
    let out = p.run(&["check"]).1;
    assert!(!out.contains("sliver"), "{out}");
}

#[test]
fn designator_rules() {
    let p = Proj::new("designators");
    p.ok(&["init", "des", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "40", "20"]);
    p.ok(&["drc", "add", "jlcpcb-fr4-2layer"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,10", "-P", "value=10k"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "15,10", "-P", "value=10k"]);
    let out = p.run(&["check"]).1;
    assert!(!out.contains("designator") && !out.contains("BOM row"), "{out}");
    // The same part under a second prefix: JLCPCB reads two component types in one row.
    p.ok(&["add", "RT1", "resistor-0603", "--at", "25,10", "-P", "value=10k"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("warning: bom-prefixes") && out.contains("R (R1, R2) and RT (RT1)"), "{out}");
    // A different value is a different row: no complaint.
    p.ok(&["remove", "RT1"]);
    p.ok(&["add", "RT1", "resistor-0603", "--at", "25,10", "-P", "value=1k"]);
    let out = p.run(&["check"]).1;
    assert!(!out.contains("bom-prefixes"), "{out}");
    // Format: lower case, digits first, a suffix after the number, a case collision.
    p.ok(&["add", "r3", "resistor-0603", "--at", "35,10", "-P", "value=1k"]);
    p.ok(&["add", "R_4", "resistor-0603", "--at", "35,5", "-P", "value=1k"]);
    p.ok(&["add", "1R", "resistor-0603", "--at", "35,15", "-P", "value=1k"]);
    p.ok(&["add", "R1A", "resistor-0603", "--at", "25,5", "-P", "value=1k"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("designator r3 has lower-case"), "{out}");
    assert!(out.contains("designator R_4 has `_4`"), "{out}");
    assert!(out.contains("designator 1R starts with a digit"), "{out}");
    assert!(out.contains("designator R1A has `1A`"), "{out}");
    // `r3` is also the same row as `R3` would be; a case collision is named explicitly.
    p.ok(&["add", "R3", "resistor-0603", "--at", "25,15", "-P", "value=1k"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("differ only in case"), "{out}");
}

#[test]
fn gerber_pours_emit_in_priority_order() {
    // Two overlapping pours on one layer: a board-wide GND at priority 0 and a VCC
    // island at priority 1 cut out of it.  In RS-274X an LPC clear erases everything
    // drawn before it, so the GND pour (whose holes include the island) has to be
    // emitted before the island's fill or the island vanishes from the fab file.
    let p = Proj::new("pourorder");
    p.ok(&["init", "pours", "--layers", "F.Cu", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "40", "30"]);
    p.ok(&["add", "R1", "resistor-0603"]);
    p.ok(&["add", "R2", "resistor-0603"]);
    p.ok(&["place", "R1", "10,15"]);
    p.ok(&["place", "R2", "30,15"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "VCC"]);
    p.ok(&["connect", "R1.2", "R2.2", "--net", "GND"]);
    // The island is declared first on purpose: emission order must not follow project order.
    p.ok(&["pour", "new", "vcc", "--layer", "F.Cu", "--net", "VCC", "--rect", "5,5", "--size", "12,20", "--priority", "1"]);
    p.ok(&["pour", "new", "gnd", "--layer", "F.Cu", "--net", "GND", "--follow-outline"]);
    p.ok(&["gerbers", "--force"]);
    let gtl = std::fs::read_to_string(p.build("gerbers/pours-F_Cu.gtl")).unwrap();
    // The first region drawn is the GND pour: it reaches the far edge of the board.
    let first = gtl.split("G36*").nth(1).unwrap();
    let first = &first[..first.find("G37*").unwrap()];
    let max_x = first
        .lines()
        .filter_map(|l| l.strip_prefix('X').and_then(|r| r.split('Y').next()).and_then(|x| x.parse::<i64>().ok()))
        .max()
        .unwrap();
    assert!(max_x > 30_000_000, "first region should be the board-wide GND pour, max x = {max_x}:\n{first}");
    // Holes are keyholed into their outers: no polarity clears anywhere, so nothing
    // drawn earlier can be erased; the island's fill still comes after the GND pour.
    assert!(!gtl.contains("%LPC*%"), "regions must not use polarity clears");
    let island_fill = gtl.find("X5000000Y").or_else(|| gtl.find("X17000000Y")).expect("island region");
    assert!(island_fill > gtl.find("G36*").unwrap(), "VCC island must be drawn after the GND pour");
}

#[test]
fn trim_traces() {
    let p = Proj::new("trim");
    p.ok(&["init", "trim", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "20", "16"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,5"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "10,5"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "A"]);
    p.ok(&["connect", "R1.2", "R2.2", "--net", "GND"]);
    // A trace that overshoots R2.1 by nearly 5 mm, and a stub touching nothing.
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "4.125,5", "4.125,7", "9.125,7", "9.125,5", "14,5"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "GND", "--width", "0.3", "3,12", "8,12"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("dangling-trace"), "{out}");
    let out = p.ok(&["trace", "trim", "--dry-run"]);
    assert!(out.contains("dry run") && out.contains("removed GND trace"), "{out}");
    let out = p.ok(&["trace", "trim"]);
    assert!(out.contains("trimmed 1 end(s), removed 1 trace(s)"), "{out}");
    let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
    let traces = proj["traces"].as_array().unwrap();
    assert_eq!(traces.len(), 1);
    let last = traces[0]["points"].as_array().unwrap().last().unwrap();
    let x = last[0].as_f64().unwrap();
    // Cut back to where the body still touches R2.1 (pad spans 8.6..9.65 in x).
    assert!(x > 9.0 && x < 9.9, "trimmed end x = {x}");
    let out = p.run(&["check"]).1;
    assert!(!out.contains("dangling-trace"), "{out}");
    let out = p.ok(&["trace", "trim"]);
    assert!(out.contains("nothing to trim"), "{out}");
}

#[test]
fn covered_traces() {
    // A trace running inside its own net's fill adds nothing: `trace trim` cuts
    // it back to the stretches outside the fill (plus a trace width of overlap)
    // and removes it entirely when the fill covers all of it.
    let p = Proj::new("covered");
    p.ok(&["init", "cov", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "30", "20"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,10"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "25,10"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "GND"]);
    p.ok(&["connect", "R1.2", "R2.2", "--net", "A"]);
    // A hand trace joins the GND pads; the fill covers only the left half.
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "GND", "--width", "0.3", "4.125,10", "4.125,13", "24.125,13", "24.125,10"]);
    p.ok(&["pour", "new", "gnd", "--layer", "F.Cu", "--net", "GND", "--rect", "0,0", "--size", "15,20"]);
    let out = p.ok(&["trace", "trim"]);
    assert!(out.contains("stretches of 1 more"), "{out}");
    let list = p.ok(&["trace", "list"]);
    // The stretch inside the fill (x < 15) is gone except for one trace width
    // of overlap; the stretch outside stays whole and still reaches R2.1.
    let gnd: Vec<&str> = list.lines().filter(|l| l.contains("net GND")).collect();
    assert_eq!(gnd.len(), 1, "the stub left in R1.1's thermal ring parallels its spokes and goes too:\n{list}");
    let line = gnd[0].to_string();
    assert!(line.contains("24.125,10"), "{line}");
    let first = line.split("w 0.3mm ").nth(1).unwrap().split(" ->").next().unwrap();
    let x: f64 = first.split(',').next().unwrap().parse().unwrap();
    assert!(x > 14.0 && x < 15.0, "cut should land a trace width inside the fill: {line}");
    let status = p.ok(&["status"]);
    assert!(status.lines().any(|l| l.contains("GND") && l.contains("routed")), "{status}");
    // Whole trace inside the fill: removed.
    p.ok(&["pour", "remove", "gnd"]);
    p.ok(&["pour", "new", "gnd", "--layer", "F.Cu", "--net", "GND", "--follow-outline"]);
    // (The end inside R2.1's thermal ring is uncovered, so the trace is cut
    // there first and the stub then dropped as redundant.)
    let out = p.ok(&["trace", "trim"]);
    assert!(out.contains("fill covers"), "{out}");
    let list = p.ok(&["trace", "list"]);
    assert!(!list.contains("net GND"), "{list}");
    // GND is whole through the fill alone; only the never-routed net A remains open.
    let out = p.run(&["check"]).1;
    assert!(!out.contains("net GND"), "{out}");
    assert!(out.lines().filter(|l| l.starts_with("error:")).all(|l| l.contains("net A")), "{out}");
}

#[test]
fn gerber_verify() {
    // The written gerbers are read back and their copper traced to pads by
    // contact alone; tampering with the files must show up as an open or a short.
    let p = Proj::new("gverify");
    p.ok(&["init", "gv", "--layers", "F.Cu,B.Cu", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "30", "20"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,10"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "25,10"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "A"]);
    p.ok(&["connect", "R1.2", "R2.2", "--net", "GND"]);
    // Net A crosses to the bottom and back through two vias; GND is a top trace.
    p.ok(&["via", "add", "--net", "A", "15,15"]);
    p.ok(&["via", "add", "--net", "A", "24.125,15"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "4.125,10", "4.125,15", "15,15"]);
    p.ok(&["trace", "add", "--layer", "B.Cu", "--net", "A", "--width", "0.3", "15,15", "24.125,15"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "24.125,15", "24.125,10"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "GND", "--width", "0.3", "5.875,10", "5.875,5", "25.875,5", "25.875,10"]);
    let out = p.ok(&["gerbers"]);
    assert!(out.contains("gerbers verified against the netlist: 4 pad(s) on 2 copper layer(s)") && out.contains("2 plated drill(s)"), "{out}");
    // Drop the via's drill: the top and bottom halves of net A no longer meet.
    let drl = p.build("gerbers/gv-PTH.drl");
    let original = std::fs::read_to_string(&drl).unwrap();
    let tampered: String = original.lines().filter(|l| !l.starts_with("X15.000Y15.000")).map(|l| format!("{l}\n")).collect();
    std::fs::write(&drl, tampered).unwrap();
    let out = p.fails(&["gerbers", "--verify-only"]);
    assert!(out.contains("open: net A is 2 separate pieces") && out.contains("1 open net(s), 0 short(s)"), "{out}");
    std::fs::write(&drl, original).unwrap();
    p.ok(&["gerbers", "--verify-only"]);
    // A stray region on top from R1.1 across to the GND trace at x = 5.875 is a short.
    let gtl = p.build("gerbers/gv-F_Cu.gtl");
    let original = std::fs::read_to_string(&gtl).unwrap();
    let bridge = "G36*\nX3900000Y9600000D02*\nX6000000Y9600000D01*\nX6000000Y9900000D01*\nX3900000Y9900000D01*\nX3900000Y9600000D01*\nG37*\n";
    let tampered = original.replace("M02*", &format!("{bridge}M02*"));
    std::fs::write(&gtl, tampered).unwrap();
    let out = p.fails(&["gerbers", "--verify-only"]);
    assert!(out.contains("short: one piece of copper in the gerbers joins A (") && out.contains("GND ("), "{out}");
}

#[test]
fn free_text() {
    let p = Proj::new("text");
    p.ok(&["init", "text", "--layers", "F.Cu", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "30", "20"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,5"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "25,5"]);
    p.ok(&["connect", "R1.2", "R2.2", "--net", "GND"]);
    p.ok(&["connect", "R1.1", "R2.1", "--net", "A"]);
    p.ok(&["pour", "new", "gnd", "--layer", "F.Cu", "--net", "GND", "--follow-outline"]);
    let before = p.ok(&["pour", "list"]);
    p.ok(&["gerbers", "--force"]);
    let silk_before = std::fs::read_to_string(p.build("gerbers/text-F_Silkscreen.gto")).unwrap().matches("G36*").count();
    let cu_before = std::fs::read_to_string(p.build("gerbers/text-F_Cu.gtl")).unwrap().matches("G36*").count();
    let out = p.ok(&["text", "add", "hello 54v", "--at", "15,14", "--size", "2"]);
    assert!(out.contains("F.Silkscreen"), "{out}");
    p.ok(&["text", "add", "REV A", "--at", "15,10", "--layer", "F.Cu", "--size", "3", "--rotation", "-90"]);
    let bad = p.fails(&["text", "add", "héllo", "--at", "1,1"]);
    assert!(bad.contains("no glyph"), "{bad}");
    let bad = p.fails(&["text", "add", "x", "--at", "1,1", "--layer", "B.Cu"]);
    assert!(bad.contains("no layer"), "{bad}");
    let list = p.ok(&["text", "list"]);
    assert!(list.contains("#0") && list.contains("#1") && list.contains("rot -90"), "{list}");
    // Copper text takes fill away from the pour; silk text does not.
    let after = p.ok(&["pour", "list"]);
    let area = |s: &str| s.lines().next().unwrap().split("—").nth(1).unwrap().trim().split(' ').next().unwrap().parse::<f64>().unwrap();
    assert!(area(&after) < area(&before) - 10.0, "before {before} after {after}");
    // Copper text must not raise anything beyond the (deliberately) unrouted net.
    let out = p.run(&["check"]).1;
    assert!(out.lines().filter(|l| l.starts_with("error:")).all(|l| l.contains("nets-routed")), "{out}");
    p.ok(&["gerbers", "--force"]);
    let silk = std::fs::read_to_string(p.build("gerbers/text-F_Silkscreen.gto")).unwrap().matches("G36*").count();
    assert!(silk > silk_before, "silk should carry the text strokes ({silk_before} -> {silk})");
    let cu = std::fs::read_to_string(p.build("gerbers/text-F_Cu.gtl")).unwrap().matches("G36*").count();
    assert!(cu > cu_before, "copper should carry the text strokes ({cu_before} -> {cu})");
    p.ok(&["route", "--dsn-only"]);
    let dsn = std::fs::read_to_string(p.build("text.dsn")).unwrap();
    assert!(dsn.matches("(keepout").count() >= 4, "copper text must be a router keepout:\n{dsn}");
    p.ok(&["text", "remove", "1"]);
    assert!(p.ok(&["text", "list"]).contains("HELLO 54V") || p.ok(&["text", "list"]).contains("hello 54v"));
}

#[test]
fn route_pin_and_undo() {
    let p = Proj::new("routepin");
    p.ok(&["init", "rp", "--layers", "F.Cu", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "30", "20"]);
    p.ok(&["rules", "set", "trace_width=0.3", "clearance=0.2"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,10"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "25,10"]);
    p.ok(&["add", "R3", "resistor-0603", "--at", "15,10", "--rotation", "90"]); // in the way
    p.ok(&["connect", "R1.2", "R2.1", "--net", "A"]);
    p.ok(&["connect", "R3.1", "R3.2", "--net", "B"]);
    p.ok(&["connect", "R1.1", "R2.2", "--net", "C"]);
    // Dry run leaves the project alone.
    let out = p.ok(&["route", "pin", "R1.2", "R2.1", "--dry-run"]);
    assert!(out.contains("dry run: routed R1.2 -> R2.1 on F.Cu"), "{out}");
    let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
    assert!(proj.get("traces").map_or(true, |t| t.as_array().unwrap().is_empty()));
    // The real thing: goes round R3, lands on the pads, is DRC clean, and writes a crop.
    let out = p.ok(&["route", "pin", "R1.2", "R2.1", "--png", "crop"]);
    assert!(out.contains("routed R1.2 -> R2.1 on F.Cu") && out.contains("cropped"), "{out}");
    assert!(p.build("pcb.png").exists());
    let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
    let traces = proj["traces"].as_array().unwrap();
    assert_eq!(traces.len(), 1);
    let pts = traces[0]["points"].as_array().unwrap();
    assert!(pts.len() >= 3, "should bend round R3: {pts:?}");
    assert_eq!(pts[0][0].as_f64().unwrap(), 5.875); // R1.2 centre
    let last = pts.last().unwrap();
    assert_eq!(last[0].as_f64().unwrap(), 24.125); // R2.1 centre
    let check = p.run(&["check"]).1;
    assert!(!check.contains("net A is not fully routed"), "{check}");
    assert!(check.lines().filter(|l| l.starts_with("error:")).all(|l| l.contains("nets-routed")), "{check}");
    // Already connected now.
    let out = p.fails(&["route", "pin", "R1.2", "R2.1"]);
    assert!(out.contains("already connected"), "{out}");
    // Waypoint: force the other way round.
    p.ok(&["undo"]);
    let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
    assert!(proj.get("traces").map_or(true, |t| t.as_array().unwrap().is_empty()), "undo should remove the trace");
    let out = p.ok(&["route", "pin", "R1.2", "R2.1", "--via", "15,4"]);
    assert!(out.contains("routed"), "{out}");
    let proj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dir.join("pcb.json")).unwrap()).unwrap();
    let pts = proj["traces"][0]["points"].as_array().unwrap();
    assert!(pts.iter().any(|q| q[1].as_f64().unwrap() < 6.0), "should pass below R3 via the waypoint: {pts:?}");
    // Blocked: a wall of another net from edge to edge, then ask for net C.
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "B", "--width", "1", "20,0.5", "20,19.5"]);
    let out = p.fails(&["route", "pin", "R1.1", "R2.2"]);
    assert!(out.contains("no route from R1.1 to R2.2") && out.contains("nearest approach"), "{out}");
    let out = p.ok(&["undo"]);
    assert!(out.contains("restored"), "{out}");
}

#[test]
fn kicad_export() {
    let p = Proj::new("kicad");
    let jlc = Path::new(env!("CARGO_MANIFEST_DIR")).join("library-jlcpcb/index.json");
    p.ok(&["init", "kc", "--index", jlc.to_str().unwrap(), "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "30", "20", "--radius", "1"]);
    p.ok(&["rules", "set", "clearance=0.15"]);
    p.ok(&["add", "J1", "wago-2060-452", "--at", "9.5,5"]); // wire-entry ticks inside the outline
    p.ok(&["add", "R1", "jlcpcb:resistor-0603", "--param", "value=1k", "--at", "20,12"]);
    p.ok(&["add", "D1", "led-0603-red", "--at", "25,12", "--side", "bottom"]);
    p.ok(&["connect", "J1.1", "R1.1", "--net", "VIN"]);
    p.ok(&["connect", "J1.2", "D1.K", "--net", "GND"]);
    p.ok(&["connect", "R1.2", "D1.A", "--net", "LED"]);
    p.ok(&["pour", "new", "gnd", "--layer", "B.Cu", "--net", "GND", "--follow-outline"]);
    p.ok(&["text", "add", "KC", "--at", "15,17", "--size", "1"]);
    p.ok(&["route", "pin", "J1.1", "R1.1"]);
    // D1 is on the back: the LED route must change layer, and J1's ground tab needs a via
    // down to the pour. Both routes are left to the router (the via is placed by hand).
    let out = p.ok(&["route", "pin", "R1.2", "D1.A"]);
    assert!(out.contains("1 via(s)"), "route to a back-side pad must use a via:\n{out}");
    p.ok(&["via", "add", "--net", "GND", "12,9"]);
    p.ok(&["route", "pin", "J1.2", "D1.K"]);
    let check = p.run(&["check"]).1;
    assert!(check.contains("0 error(s), 0 warning(s)"), "{check}");
    let out = p.ok(&["export", "kicad"]);
    assert!(out.contains("kc.kicad_pcb") && out.contains("kc.kicad_pro"), "{out}");
    let pcb = std::fs::read_to_string(p.build("kicad/kc.kicad_pcb")).unwrap();
    assert!(pcb.starts_with("(kicad_pcb"));
    assert!(pcb.contains("(duplicate_pad_numbers_are_jumpers yes)"), "WAGO tabs share a pad number");
    assert!(pcb.contains("(zone (net") && pcb.contains("(gr_text \"KC\"") && pcb.contains("(via (at"));
    assert!(pcb.matches("(footprint ").count() == 3);
    // With KiCad installed, its DRC must accept the file and find no errors.
    if flatland::kicad::find_kicad_cli().is_some() {
        let out = p.run(&["export", "kicad", "--drc"]).1;
        assert!(out.contains("kicad drc: 0 error(s)"), "{out}");
    }
}

#[test]
fn audit_checks() {
    let p = Proj::new("audit");
    p.ok(&["init", "au", "--index", library().to_str().unwrap()]);
    p.ok(&["outline", "rect", "30", "20"]);
    p.ok(&["add", "R1", "resistor-0603", "--at", "5,10"]);
    p.ok(&["add", "R2", "resistor-0603", "--at", "25,10"]);
    p.ok(&["connect", "R1.2", "R2.1", "--net", "A"]);
    p.ok(&["connect", "R1.1", "R2.2", "--net", "B"]);
    // Two traces meeting end to end (a width change): connectivity must not accept the
    // shared edge as a joint, and trace-junctions must name the pair.
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "5.875,10", "12,10"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.5", "12,10", "24.125,10"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("error: trace-junctions") && out.contains("meet end to end"), "{out}");
    assert!(out.contains("net A is not fully routed"), "connectivity must not count an edge as contact:\n{out}");
    // Overlapping by a hair fixes both.
    p.ok(&["trace", "clear"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.3", "5.875,10", "12.25,10"]);
    p.ok(&["trace", "add", "--layer", "F.Cu", "--net", "A", "--width", "0.5", "12,10", "24.125,10"]);
    let out = p.run(&["check"]).1;
    assert!(!out.contains("trace-junctions") && !out.contains("net A is not"), "{out}");
    // Coincident vias are an error even at distance zero.
    p.ok(&["via", "add", "--net", "B", "15,15"]);
    p.ok(&["via", "add", "--net", "B", "15,15"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("error: holes-overlap"), "{out}");
    p.ok(&["undo"]);
    // Silk outside the outline and over an opening.
    p.ok(&["text", "add", "OUT", "--at", "40,10"]);
    p.ok(&["text", "add", "ON", "--at", "5,10", "--size", "1"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("silk-text-outside-outline") && out.contains("silk-text-over-opening"), "{out}");
    // A stale waiver is reported.
    p.ok(&["drc", "waive", "courtyards", "R9", "--reason", "nothing"]);
    let out = p.run(&["check"]).1;
    assert!(out.contains("matched no finding"), "{out}");
    // Netlist compare: a swapped pin, a missing pin, a board-only net.
    std::fs::write(p.dir.join("ext.txt"), "A: R1.2 R2.2\nB: R1.1\nC: R2.1\n").unwrap();
    let out = p.fails(&["net", "compare", "ext.txt"]);
    assert!(out.contains("R2.2: on net A in the file, on net B on the board"), "{out}");
    assert!(out.contains("R2.1: on net C in the file, on net A on the board"), "{out}");
    std::fs::write(p.dir.join("same.json"), "{\"A\": [\"R1.2\", \"R2.1\"], \"B\": [\"R1.1\", \"R2.2\"]}").unwrap();
    let out = p.ok(&["net", "compare", "same.json"]);
    assert!(out.contains("0 difference(s)"), "{out}");
}
