//! ngspice via its shared library (`libngspice`), loaded at runtime with
//! `libloading` so the binary builds and runs without ngspice installed —
//! only `pcb sim run` needs it.

use crate::error::{Error, Result};
use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CANDIDATES: &[&str] = &[
    "/opt/homebrew/lib/libngspice.dylib",
    "/opt/homebrew/opt/ngspice/lib/libngspice.dylib",
    "/usr/local/lib/libngspice.dylib",
    "/usr/local/opt/ngspice/lib/libngspice.dylib",
    "/opt/local/lib/libngspice.dylib",
    "libngspice.dylib",
    "libngspice.so.0",
    "libngspice.so",
    "/usr/lib/x86_64-linux-gnu/libngspice.so.0",
    "/usr/lib/libngspice.so.0",
    "ngspice.dll",
];

#[repr(C)]
struct VectorInfo {
    v_name: *mut c_char,
    v_type: c_int,
    v_flags: i16,
    v_realdata: *mut f64,
    v_compdata: *mut f64, // ngcomplex_t { double cx_real; double cx_imag; }
    v_length: c_int,
}

type SendChar = extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type SendStat = extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type ControlledExit = extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type SendData = extern "C" fn(*mut c_void, c_int, c_int, *mut c_void) -> c_int;
type SendInitData = extern "C" fn(*mut c_void, c_int, *mut c_void) -> c_int;
type BgThreadRunning = extern "C" fn(bool, c_int, *mut c_void) -> c_int;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static EXITED: Mutex<Option<i32>> = Mutex::new(None);

extern "C" fn send_char(s: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    if !s.is_null() {
        let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().to_string();
        LOG.lock().unwrap().push(text);
    }
    0
}
extern "C" fn send_stat(_s: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    0
}
extern "C" fn controlled_exit(status: c_int, _immediate: bool, _quit: bool, _id: c_int, _user: *mut c_void) -> c_int {
    *EXITED.lock().unwrap() = Some(status);
    0
}
extern "C" fn send_data(_d: *mut c_void, _n: c_int, _id: c_int, _user: *mut c_void) -> c_int {
    0
}
extern "C" fn send_init_data(_d: *mut c_void, _id: c_int, _user: *mut c_void) -> c_int {
    0
}
extern "C" fn bg_running(_running: bool, _id: c_int, _user: *mut c_void) -> c_int {
    0
}

#[derive(Clone, Debug)]
pub struct Vector {
    pub name: String,
    pub values: Vec<f64>,
    pub imag: Vec<f64>,
    pub is_complex: bool,
    /// ngspice `v_type` (simulation_types enum).
    pub v_type: i32,
    /// Physical unit symbol: `V`, `A`, `s`, `Hz`, `W`, `°C`, `Ω`, `S`, `F`, `C`, `°`, `dB` or empty.
    pub unit: String,
}

/// Unit from ngspice's vector type, falling back to naming conventions.
pub fn unit_for(name: &str, v_type: i32) -> String {
    // enum simulation_types in ngspice's sim.h
    let by_type = match v_type {
        1 => "s",
        2 => "Hz",
        3 => "V",
        4 => "A",
        14 => "°C",
        15 | 16 => "Ω",
        17 => "S",
        18 => "W",
        19 => "°",
        20 => "dB",
        21 => "F",
        22 => "C",
        _ => "",
    };
    if !by_type.is_empty() {
        return by_type.into();
    }
    let n = name.to_lowercase();
    if n == "time" {
        return "s".into();
    }
    if n == "frequency" {
        return "Hz".into();
    }
    if n == "temp-sweep" {
        return "°C".into();
    }
    if n.ends_with("#branch") || n == "i-sweep" {
        return "A".into();
    }
    if let Some(param) = n.strip_prefix('@').and_then(|r| r.split_once('[')).map(|(_, p)| p.trim_end_matches(']')) {
        return match param {
            "p" | "power" => "W",
            "i" | "id" | "ic" | "ib" | "ie" | "is" | "ig" | "ids" => "A",
            "v" | "vd" | "vbe" | "vce" | "vgs" | "vds" | "vbs" | "vth" => "V",
            "gm" | "gds" | "gmb" | "gd" | "gpi" | "go" => "S",
            "r" | "rs" | "rd" | "req" | "resistance" => "Ω",
            "c" | "cd" | "cgs" | "cgd" | "cap" | "capacitance" => "F",
            "temp" | "dtemp" => "°C",
            _ => "",
        }
        .into();
    }
    // Plain node vectors are voltages.
    if !n.contains('#') && !n.starts_with('@') {
        return "V".into();
    }
    String::new()
}

pub struct RunResult {
    pub plot: String,
    pub scale: Option<String>,
    pub vectors: Vec<Vector>,
    pub log: String,
}

pub struct Ngspice {
    lib: Library,
}

impl Ngspice {
    pub fn load() -> Result<Ngspice> {
        let mut tried = Vec::new();
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("NGSPICE_LIB") {
            candidates.push(p);
        }
        candidates.extend(CANDIDATES.iter().map(|s| s.to_string()));
        for c in &candidates {
            match unsafe { Library::new(c) } {
                Ok(lib) => {
                    let ng = Ngspice { lib };
                    ng.init()?;
                    return Ok(ng);
                }
                Err(e) => tried.push(format!("{c}: {e}")),
            }
        }
        Err(Error::with_help(
            "libngspice (the ngspice shared library) was not found",
            format!(
                "install it with `brew install ngspice` (macOS) or `apt install libngspice0` (Debian/Ubuntu), \
                 or point $NGSPICE_LIB at the library. Tried:\n  {}",
                tried.join("\n  ")
            ),
        ))
    }

    fn init(&self) -> Result<()> {
        unsafe {
            let f: Symbol<unsafe extern "C" fn(SendChar, SendStat, ControlledExit, SendData, SendInitData, BgThreadRunning, *mut c_void) -> c_int> =
                self.lib.get(b"ngSpice_Init\0").map_err(|e| Error::msg(format!("libngspice has no ngSpice_Init: {e}")))?;
            let r = f(send_char, send_stat, controlled_exit, send_data, send_init_data, bg_running, std::ptr::null_mut());
            if r != 0 {
                return Err(Error::msg(format!("ngSpice_Init failed with code {r}")));
            }
        }
        Ok(())
    }

    fn command(&self, cmd: &str) -> Result<c_int> {
        let c = CString::new(cmd).map_err(|_| Error::msg("command contains a NUL byte"))?;
        unsafe {
            let f: Symbol<unsafe extern "C" fn(*const c_char) -> c_int> =
                self.lib.get(b"ngSpice_Command\0").map_err(|e| Error::msg(format!("libngspice has no ngSpice_Command: {e}")))?;
            Ok(f(c.as_ptr()))
        }
    }

    fn running(&self) -> bool {
        unsafe {
            match self.lib.get::<unsafe extern "C" fn() -> bool>(b"ngSpice_running\0") {
                Ok(f) => f(),
                Err(_) => false,
            }
        }
    }

    /// Run a netlist. `dc_op` says the analysis is a plain `.op`, for which a
    /// failed DC convergence is fatal (a transient can fall back to a
    /// transient-op and still be valid).
    pub fn run(&mut self, netlist: &str, dc_op: bool) -> Result<RunResult> {
        LOG.lock().unwrap().clear();
        *EXITED.lock().unwrap() = None;
        // Load the circuit line by line.
        let lines: Vec<CString> = netlist
            .lines()
            .map(|l| CString::new(l).unwrap_or_else(|_| CString::new("*").unwrap()))
            .collect();
        let mut ptrs: Vec<*const c_char> = lines.iter().map(|l| l.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let rc = unsafe {
            let f: Symbol<unsafe extern "C" fn(*mut *const c_char) -> c_int> =
                self.lib.get(b"ngSpice_Circ\0").map_err(|e| Error::msg(format!("libngspice has no ngSpice_Circ: {e}")))?;
            f(ptrs.as_mut_ptr())
        };
        let mut log = LOG.lock().unwrap().join("\n");
        if rc != 0 || log.lines().any(|l| l.contains("Error") && !l.contains("No error")) {
            return Err(self.spice_error("ngspice rejected the netlist", &log));
        }
        self.command("bg_run")?;
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(20));
        while self.running() {
            if start.elapsed() > Duration::from_secs(600) {
                let _ = self.command("bg_halt");
                return Err(Error::with_help("simulation did not finish within 10 minutes", "reduce the stop time or increase the step"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        log = LOG.lock().unwrap().join("\n");
        if let Some(code) = *EXITED.lock().unwrap() {
            if code != 0 {
                return Err(self.spice_error(&format!("ngspice exited with status {code}"), &log));
            }
        }
        let low = log.to_lowercase();
        // ngspice recovers from "singular matrix" warnings via gmin stepping; these it does not.
        let fatal = ["timestep too small", "too many iterations without convergence", "unable to find dc operating point"];
        if let Some(f) = fatal.iter().find(|f| low.contains(*f)) {
            return Err(self.spice_error(&format!("ngspice did not converge ({f})"), &log)
                .help("try a smaller step or a shorter stop time, add `.options reltol=1e-2` / `gmin=1e-10` with `-o`, or give initial conditions with `-x \".ic v(NODE)=0\"`"));
        }
        if dc_op && low.contains("source stepping failed") {
            return Err(Error::with_help(
                "no DC operating point: every convergence method failed (gmin and source stepping)",
                "the circuit probably has no stable DC state (an oscillator or astable has none by design); \
                 use a transient study instead, or `-x \".nodeset v(NODE)=…\"` to steer the solver",
            ));
        }
        let plot = self.cur_plot()?;
        let (vectors, scale) = self.vectors(&plot)?;
        if vectors.is_empty() {
            return Err(self.spice_error("ngspice produced no vectors", &log));
        }
        let _ = self.command("destroy all");
        Ok(RunResult { plot, scale, vectors, log })
    }

    fn spice_error(&self, what: &str, log: &str) -> Error {
        let interesting: Vec<&str> = log
            .lines()
            .filter(|l| {
                let low = l.to_lowercase();
                low.contains("error") || low.contains("warning") || low.contains("unknown") || low.contains("singular") || low.contains("timestep") || low.contains("could not")
            })
            .take(12)
            .collect();
        Error::with_help(
            format!("{what}:\n  {}", if interesting.is_empty() { log.lines().rev().take(8).collect::<Vec<_>>().join("\n  ") } else { interesting.join("\n  ") }),
            "inspect the generated netlist with `pcb sim netlist <name>`; the full ngspice log is saved next to the results",
        )
    }

    fn cur_plot(&self) -> Result<String> {
        unsafe {
            let f: Symbol<unsafe extern "C" fn() -> *mut c_char> = self.lib.get(b"ngSpice_CurPlot\0").map_err(|e| Error::msg(format!("{e}")))?;
            let p = f();
            if p.is_null() {
                return Err(Error::msg("ngspice has no current plot"));
            }
            Ok(CStr::from_ptr(p).to_string_lossy().to_string())
        }
    }

    fn vectors(&self, plot: &str) -> Result<(Vec<Vector>, Option<String>)> {
        let mut out = Vec::new();
        let mut scale = None;
        unsafe {
            let all: Symbol<unsafe extern "C" fn(*const c_char) -> *mut *mut c_char> = self.lib.get(b"ngSpice_AllVecs\0").map_err(|e| Error::msg(format!("{e}")))?;
            let info: Symbol<unsafe extern "C" fn(*const c_char) -> *mut VectorInfo> = self.lib.get(b"ngGet_Vec_Info\0").map_err(|e| Error::msg(format!("{e}")))?;
            let cplot = CString::new(plot).unwrap();
            let names = all(cplot.as_ptr());
            if names.is_null() {
                return Ok((out, None));
            }
            let mut i = 0;
            loop {
                let p = *names.add(i);
                if p.is_null() {
                    break;
                }
                let name = CStr::from_ptr(p).to_string_lossy().to_string();
                // Bare names resolve against the current plot; the
                // `plot.name` form mis-parses device vectors like `@d1[id]`.
                let bare = CString::new(name.clone()).unwrap();
                let mut vi = info(bare.as_ptr());
                if vi.is_null() || (*vi).v_length == 0 {
                    let full = CString::new(format!("{plot}.{name}")).unwrap();
                    let vi2 = info(full.as_ptr());
                    if !vi2.is_null() {
                        vi = vi2;
                    }
                }
                if !vi.is_null() {
                    let v = &*vi;
                    let n = v.v_length.max(0) as usize;
                    let (mut values, mut imag) = (Vec::with_capacity(n), Vec::new());
                    let is_complex = v.v_realdata.is_null() && !v.v_compdata.is_null();
                    if is_complex {
                        for k in 0..n {
                            values.push(*v.v_compdata.add(2 * k));
                            imag.push(*v.v_compdata.add(2 * k + 1));
                        }
                    } else if !v.v_realdata.is_null() {
                        for k in 0..n {
                            values.push(*v.v_realdata.add(k));
                        }
                    }
                    // VF_... flags: bit 0 real, 1 complex, ... ; scale vectors are
                    // identified by name instead (time, frequency, v-sweep, temp-sweep).
                    if matches!(name.as_str(), "time" | "frequency" | "v-sweep" | "i-sweep" | "temp-sweep" | "res-sweep") {
                        scale = Some(name.clone());
                    }
                    let unit = unit_for(&name, v.v_type);
                    if std::env::var_os("PCB_SIM_DEBUG").is_some() {
                        eprintln!("vector {name}: type {} unit {unit} len {n}", v.v_type);
                    }
                    out.push(Vector { name, values, imag, is_complex, v_type: v.v_type, unit });
                }
                i += 1;
            }
        }
        // Put the scale first, then nodes, then branch currents.
        out.sort_by_key(|v| (Some(&v.name) != scale.as_ref(), v.name.contains("#branch"), v.name.starts_with('@'), v.name.clone()));
        Ok((out, scale))
    }
}
